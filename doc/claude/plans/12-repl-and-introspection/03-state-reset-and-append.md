<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 03 — State reset + bytecode append

**Status: open.**

## Revised design (2026-06-07) — supersedes the bytecode-append plan below

Two findings collapse this phase well below its original MH estimate:

1. **Appendability already exists.** `compile::byte_code_from(state, data,
   start_d_nr, …)` emits bytecode only for definitions `>= start_d_nr` and skips
   the one-time init — so a new statement's code is appended with no
   `Arc<Vec<u8>>` → `Vec<u8>` refactor.  The whole "Option A/B" section below is
   moot.

2. **Variables live on the stack; keep them there.**  The REPL session is one
   long-lived, growing frame.  Its variable table is shaped exactly like a
   normal function's (`vars: Function` — names + types at fixed slots from the
   frame base), so the **existing expression codegen + execution read the slots
   for free** — no session struct, no field-type inference, no `s.x` identifier
   rewriting.

   - A reference to a prior variable compiles to an ordinary load from its slot.
   - **Defining a new variable is just the normal expression result** landing in
     the next free slot; the parser allocates that slot and records the
     name→slot/type in the persistent `vars`.
   - The values persist on the stack across inputs; `reset_for_repl` resets only
     `code_pos` / `call_stack`, never the variable region.

So the work is: (a) a persistent function-shaped `vars` + preserved stack frame,
seeded into the parse of each input; (b) `byte_code_from` to emit the new
statement's code; (c) `reset_for_repl`; (d) execute from the new code offset.
The load-bearing claim to spike first: a statement compiled against a
pre-seeded, function-shaped `vars` reads/writes the right slots and the values
survive a reset-and-re-enter.

**Slice A — DONE (2026-06-07).** The parse → compile → execute pipeline composes
end-to-end: `parse_statement` → `compile::byte_code` → `execute_argv` on the
phase-02 wrapper.  A definition from one input is callable from a later input
(functions persist in `Data`); bare expressions and stdlib calls run.
`tests/repl_eval.rs`.  Slice A uses a fresh `State` + full compile per input
(correct, not yet optimized — a fresh State sidesteps the @P381 CONST_STORE
re-lock).

**Slice B — DONE (2026-06-07; persists any value type).** `ReplSession`
(`src/repl.rs`) gives cross-input *variable* persistence: a variable bound in one
input is visible to the next (`x = 1` then `x + 1`), depends on earlier ones
(`b = a * 2`), and rebinds (`n = n + 100`).  Built and first tested for integers,
but the mechanism is type-agnostic — text, struct, and vector bindings persist
too (verified in `tests/repl_session.rs`).

Mechanism: the session keeps the stdlib-loaded parser + the accumulated
*bindings* as source.  A binding is recorded but **not executed** — an unused
binding's slot is elided by the allocator, so compiling it would panic; its value
is realised when a later input *observes* it.  An observing input (expression /
call) is wrapped in one shared-scope fn over all bindings, run with a fresh
`State` per input (sidesteps the @P381 CONST_STORE re-lock).  `scopes::check` runs
between parse and `byte_code` (else locals get no slot).  Re-running the bindings
is correct as long as each RHS is deterministic and side-effect-free (any type):
re-running yields the same value.  A side effect in a binding's RHS would repeat
once per observation — the one real limitation, addressed by REPL.X.

**Error recovery — FIXED (2026-06-07).** Root cause: the lexer's `restart` /
`parse_string` reset the cursor but never cleared its `diagnostics`, and
`Diagnostics::level` is monotonic — so after a parse error, every later
`parse_str` re-`fill`ed the lexer's *stale* errors into the parser, and the
session rejected clean input (a typo poisoned the session).  Fix: `parse_str`
now clears the lexer diagnostics at its start (`Lexer::clear_diagnostics` +
`Diagnostics::clear`).  A standalone string parse no longer inherits prior
errors; benefits any repeated `parse_str` user, not just the REPL.
`tests/repl_session.rs::parse_error_leaves_session_usable` passes.

**Remaining toward the general model.** Eliminating the re-run via the true
stack-resident model (persistent `State` + `byte_code_from` + `reset_for_repl`
preserving `stack_pos` + resume-execution) — REPL.X.  (Result-for-display landed
in phase 04; cross-type persistence already works via the re-run model above.)

## REPL.X — eliminating the re-run (designed 2026-06-08, not yet built)

The re-run is structural: the session re-executes the accumulated bindings each
time it observes a value, so a binding whose RHS has a *side effect* repeats it.
A correct fix runs each line **once** and keeps the variable frame alive between
inputs.  Investigation found two viable approaches and one real hazard.

**Hazard (the reason this is not a quick edit).** A frame is not just bytes:
text/`DbRef` locals stored in it need lifetime handling.  The coroutine path
already proves this — `serialise_text_args` + `drop_text_locals_in_bytes` exist
to own text out of a saved frame and to free it without double-dropping.  Any
frame snapshot/restore for the REPL must reuse that handling, so the safe scope
to land *first* is integer-only locals (no text in the frame) — a constraint of
*this* preserved-frame approach, not of today's re-run model, which already
persists every type.

**Approach A — checkpoint / restore / resume (keeps all types).**  Persistent
`State`; the session is one growing `fn`.  After running through statement N,
checkpoint `(code_pos, stack-frame bytes, stack_pos, call_stack)`.  On input
N+1: append it, re-`byte_code` (codegen of the unchanged prefix 1..N is
deterministic, so the checkpoint `code_pos` stays valid), restore the frame, run
from the checkpoint → only N+1 executes.  Foundation: the coroutine stack-bytes
snapshot (`coroutine_create` copies from the stack store via `store.addr`).
Risk: prefix-stability + const-store/text-local corners.

**Approach B — function params + value capture (integer-scoped first).**  Each
input is a `fn f(<prior vars>) -> <new var> { … }`; the REPL stores variable
*values*, passes them as args, captures the return.  Each fn runs once → no
re-run.  Needs a typed-arg/return execute entry (push args, read the return) —
which also unblocks `:vars` and in-process result-as-`String`.  Marshalling
beyond integers (text `DbRef`, structs) is the follow-on.

**Recommendation.** Build B first for integers (bounded, reuses the
native-call arg/return marshalling, and dividends: `:vars` + result return),
then A for the all-types, no-recompile endpoint.  Either is a focused spike on
the execution core — land it deliberately, not bundled with unrelated work.

### Implementation-attempt findings (2026-06-08) — value-snapshot + the capture sub-problem

A third, lower-risk interim emerged when sizing the build: **value-snapshot.**
Keep today's fresh-State / re-run executor, but make the re-run *pure* — when a
binding is entered, execute its RHS **once**, capture the value, and rewrite the
body so the binding becomes a **literal** (`a = read_line()` → `a = "typed"`).
Later observes re-run a body of literals → effects happen once.  No persistent
State, no `reset_for_repl`, no frame snapshot, no CONST_STORE-lock corner — it
fixes the *correctness* bug (repeated effects) without the preserved-frame
hazards, for the cases whose value renders to a literal.  (It does **not**
eliminate the re-run *cost* — that stays Approach A / loft2.)

**But the load-bearing sub-problem is *value capture*, and it is more entangled
than "a small read helper."**  Verified mechanism:

- `get_var<T>(pos)` reads at `stack_cur.pos + stack_pos − pos` — **`stack_pos`-
  relative** (src/state/mod.rs:1644).  After `execute_argv` returns, the entry
  frame's *resting* `stack_pos` is gone (popped), so a post-hoc raw-slot read of
  a local needs the frame top captured **mid-run**, or the value **returned** by
  the entry fn and read at the stack top the way `execute_at_raw` does
  (src/state/mod.rs:2657, `get_stack` + `return_size`).  `execute_argv` is void,
  so neither exists yet — capture is net-new execution-core surface, not a helper.
- The frame's variable region *is* `[4, stack_pos)` at fixed absolute offsets
  while the frame is at rest — this is exactly what `parallel_join` snapshots for
  arm workers (src/state/mod.rs:1611-1632) — so a mid-run capture is feasible;
  the difficulty is purely *when* `stack_pos` is read.

**Decision (2026-06-08) — serialize/deserialize through loft's OWN format, not
JSON, and reuse existing machinery.**  Rather than invent value-capture surface
(frame reads, a returned-value entry, an output sink), round-trip the value
through loft's **own** serialization — render a value to loft-source text and
parse it back.  This is cleaner than JSON because loft's own format *is* loft
source: a struct/enum renders to its own constructor (which JSON's `{"a":1}`
cannot), so the **struct/enum residual disappears** — the snapshot is
semantically correct for *every* type, not just scalar+text+vector.

**Priority: semantic correctness over efficiency.**  This path may re-serialize
and re-parse on every binding; that is fine.  Re-running a body of own-format
literals is pure (effects happen once at capture), which is the whole point — the
re-run *cost* is explicitly not what REPL.X-interim optimises (that stays
Approach A / loft2).

**Investigation result (2026-06-08).** There is **no** own-format *serializer*
that round-trips: the display walk (`Stores::show`/`show_json`,
`src/database/format.rs:45`) drops type names and text quotes
(`{a:1,b:2}`, bare `Alice`), so its output does not re-parse.  The **only**
existing round-trip for runtime values is **JSON** — every struct gets
`value.to_json()` (`src/native.rs:3189`, via `show_json`) and `Type.parse(
json_parse(text))` (`src/parser/objects.rs:1025`), proven by
`tests/issues.rs::q3b_struct_to_json_round_trip`.  The **parser already accepts**
clean own-format RHS — struct constructors `Point{x:1,y:2}`
(`src/parser/objects.rs:817,1607`), text/vector/enum literals — so *deserialize*
of own-format is free; only the *serialize* side is missing.

**Sizing correction (2026-06-08).**  `show_loft` is **not** a trivial sibling.
`ShowDb::write` (`src/database/format.rs:638`) is one recursive traversal with an
`if self.json {…} else {…}` at every rendering point (text, struct, enum,
struct-enum variant, vector) and a `write_struct` that emits `{field: value}`
with no type name.  A native-loft mode is a **third output mode threaded through
the whole formatter** — moderate new code in a core file, not a contained
sibling.

**Decision (2026-06-08) — build the own-format serializer (`show_loft`), NOT
JSON.**  The deciding reason is strategic, not cleanliness: loft's own format is
**extensible with the language**, JSON is a frozen external schema.  The
formatter already carries language-specific shapes (enum-structs render their
variant + struct natively); an own-format serializer keeps growing alongside new
language constructs, where a JSON round-trip would force every future construct
through JSON's fixed grammar.  So `show_loft` is the format that *is* loft, and
it round-trips every type into clean re-parseable native source.  The moderate
formatter cost is accepted for this reason.

### Concrete design — own-format value-snapshot (the build spec)

**Invariant.** A binding's RHS side effects run **exactly once**; thereafter the
binding is own-format loft source in `body`, so every later re-run is pure.

**The three pieces.**

**① `show_loft` — own-format serializer (a third `ShowDb` mode).**
- Add `loft: bool` to `ShowDb` (`src/database/mod.rs:1353`).  Thread it through
  all **9** construction sites: entry `show`/`show_json` (`format.rs:47,71`) →
  `false`; runtime `io.rs:774` + `codegen_runtime.rs:372` → from a new format bit
  (below); the 5 recursive subs (`format.rs:917,981,1054,1144,1201`) → `self.loft`.
- Loft-mode deltas in `ShowDb::write` (`format.rs:638`) vs the JSON branch:
  | construct | JSON | **loft** |
  |---|---|---|
  | struct | `{"f":v}` | `TypeName{f: v}` — prefix `types[known_type].name`; keys unquoted (free: `if self.json` is false) |
  | text | `"esc"` | `"esc"` — same escaped+quoted form (verify loft accepts the escapes) |
  | enum-struct variant | `{"V":{…}}` | `V{…}` — variant name + struct |
  | simple enum | `V` | `Enum.V` — **qualified** so it re-parses unambiguously |
  | scalar / vector / null | bare / `[…]` / `null` | identical |
- Expose a runtime format bit `{x:l}` (`db_format & 4`) so loft code renders a
  value to loft-source text; `Stores::show_loft(s, db, tp)` for Rust callers.

**② Capture — read one `text` return (the only new execution-core surface).**
- Serialize *inside loft* so capture is single-typed: run
  `fn cap() -> text { <body> __v = <rhs>; "{__v:l}" }` — it returns the value's
  loft-source **text**.  (This sidesteps reading raw scalar/`DbRef`/`Str` slots
  by type — the program does the serialization; Rust only lifts out one `text`.)
- Add an execute entry that reads a **`text` return** into a Rust `String` (the
  return sits at the stack top after the run, like `execute_at_raw`'s return-read
  `src/state/mod.rs:2657`, but for the text representation).  One primitive,
  one type.

**③ `eval` binding branch (`src/repl.rs`).**
- Run the capture once.  On success, substitute `name = <captured-loft-src>;`
  into `body` (replacing `name = <rhs>;`).  On **any** failure (serialize error,
  unsupported shape) fall back to today's `name = <rhs>;` source — safe: it just
  re-runs, repeating an effect only in the rare unrenderable case.

**Edge risks → the falsification matrix (probe on `--interpret` BEFORE wiring ③).**
For each cell: value → `:l` text → re-parse as a binding → observe → **equal**.
- **float/single**: force a decimal point so `3.0` doesn't re-parse as `int`;
  large/small via exponent.
- **text**: loft string-escape vs the JSON escaper (`\n`, `"`, `\`, unicode);
  empty text; text containing `{`/`}`.
- **enum**: bare `V` is ambiguous → emit `Enum.V`; simple vs struct-enum variant.
- **text-return read**: the Str-at-stack-top representation (runtime `Str` vs
  store `text_nr`) — confirm the capture lifts the bytes correctly.
- **null / nested struct / vector-of-struct / DbRef indirection**.

**Resolution path (build order).**  Two halves — the *serialization format*
(reusable, independent of the REPL) then the *REPL.X consumer*.  Each step is
gated by a test that flips red→green.

**North star (the *why*).**  This serialization exists for **live data-structure
migration** — change a game's structs/enums while it runs, preserving as much
existing data as possible.  That is a key reason loft is a *language* (not a
store bolted onto an existing one); the purpose statement + the
migration-survival matrix are canonical in
[GOALS.md § Why a language, not a store bolted onto an existing one](../../GOALS.md#why-a-language-not-a-store-bolted-onto-an-existing-one).
Every fix in this section serves "maximize what survives a schema change":
leniency is the *feature* (it lets the schema change while old data reads); the
only thing forbidden is silent *wrong* data (a fail-soft), never silent
*graceful* data (default / null).

**Format design principles (2026-06-08).**  The database `show`/`parse` pair is
loft's *original* native serialization — it predates the language parser; the
value **is** its stored record (schema-driven, `DbRef`-position-independent).
`show_loft` is not a new format but the **type-explicit superset** of `show`
(adds the type info a parser needs without a schema).  Two principles govern
extension:

- **One monotonically-extending format, not multiple modes.**  The loft-native
  reader only ever *adds* accepted syntax (a strict superset each step), so a new
  reader reads every old dump.  Keep the two dialects you have (Strict = RFC-JSON
  interop; Lenient = loft-native); grow *Lenient*.  Modes would fork the format
  and re-create version skew.
- **Schema-evolution leniency is the feature; shape-mismatch fail-soft is the
  bug.**  Unknown-key-ignored + missing-field-defaulted are what let the schema
  evolve while old dumps read — **keep**.  But the type-tag swallowing a struct
  (→ `{}`) and an unknown enum tag → variant 1 are *wrong data*, not graceful
  degradation — **fix** (degrade to a null/default sentinel instead).  Leniency
  only fires on *mismatch*, so a same-schema round-trip is already exact once the
  bugs are gone — **no separate strict mode needed**.

**Part I — the symmetric own-format (no REPL code):**
1. ✅ **Fix B — dotted enum (DONE).**  `json.rs:parse_bare_identifier_value`
   consumes `.`-joined segments (`Category.Daily` → one `Ident`); the
   `walk_parsed_into` enum arm matches on the **last** segment (prefix
   informational — lenient, not validated).  `db_qualified_enum` passes (asserts
   `Hourly`, the 2nd variant, lands correctly — not the silent variant-1 default).
   Guards green: `data_structures`/`data_import`/`format` + 47 json units + 684
   `issues`.
2. **Fix A — struct type-tag via a `Constructor` AST node** (revised by the
   "read old dumps" requirement).  Today `json.rs:224` collapses `Tag{…}` into
   `Object([("Tag", obj)])` — **indistinguishable** from a plain `{Tag: obj}`, so
   a key==type-name unwrap heuristic would **misread an old dump** with a field
   named like its type.  Fix losslessly: emit `Parsed::Constructor(tag, body)`
   for `Tag{…}` (distinct from `Object`).  The walker then dispatches
   unambiguously — `Parts::Struct` validates/unwraps the tag, `Parts::Enum`
   reads it as the variant — and old dumps (plain `Object`) still read as fields.
   Purely additive (old readers can't read `Constructor`; new readers read both —
   the only direction that matters).  Also flip the enum no-match default from
   variant 1 → null sentinel (graceful degradation).  Un-ignore
   `db_struct_type_tag`; add an old-dump-ambiguity regression.
3. ✅ **`show_loft` serializer (DONE).**  A third `ShowDb` mode (`loft` flag,
   threaded through all construction sites) + `Stores::show_loft`, emitting
   `TypeName{…}`, qualified `Enum.Variant`, **forced-decimal** floats (`3.0` via
   `ensure_decimal`), JSON-escaped+quoted text, native enum-struct `V{…}`.
   `tests/own_format_alignment.rs::show_loft_round_trips_struct` proves value →
   `show_loft` → DB parser → equal; the language parser already accepts the same
   forms (the `lang_*` tests).  Debug/JSON output unchanged (loft branches gated
   on the flag); guards green.  (The runtime `:l` format bit — exposing
   `show_loft` to loft code — is deferred to step 4's capture, where it's used.)

**Part II — REPL.X consumes it:**
4. **Capture** — serialize in loft (`"{__v:l}"`) + a `text`-return read entry.
5. **Wire `eval`'s binding branch** — substitute `name = <own-format>;`; fall
   back to source on failure.
6. **Flip the regression** — `side_effecting_binding_reruns_per_observation`
   expects **once**; confirm text/struct/vector persistence still passes.

Correctness over efficiency — re-serialising per binding is fine.

**Out of scope.** The re-run *cost* (a long session still re-runs a body of
literals each observe) — that stays Approach A / loft2.  This fixes correctness
(no repeated effects), not cost.

### Aligning own-format across BOTH parsers (database `parse` + language parser)

own-format is loft's **symmetric native serialization** — it must round-trip
through *both* deserializers, not just the language parser:

- **Language parser** (`src/parser/`) — re-compiles `body` source; already
  accepts `TypeName{…}`, `Enum.V`, text/vector literals.  Free.
- **Database parser** (`Stores::parse`, `src/database/format.rs:225`) — routes
  through `crate::json::parse_with(Dialect::Lenient)` + `walk_parsed_into`
  (`src/database/structures.rs:475`); the **type is supplied out-of-band** (`tp`),
  not read from the text.  Lenient **already** accepts own-format's unquoted keys
  (`{x: 1}`) and bare-identifier enum tags (`Parsed::Ident("Red")`, dispatched to
  an enum field by the walker).  This is the `T.parse(...)` / data-literal path.

**Matrix-verified gaps (2026-06-08).**  `tests/own_format_alignment.rs` probes
each construct through both parsers and **overturned two of three predicted
gaps** — only **two** real fixes remain, and one is nastier than expected:

| own-format construct | matrix result | fix |
|---|---|---|
| **struct type tag** `Data{…}` | **SILENT FAIL-SOFT (data loss)** — lenient already parses `Tag{…}` to the single-key shape `{"Data":{fields}}` (`json.rs:224`); for a *struct* target the walker drops `Data` as an unknown key (@P366) → **all-default record**, real fields lost (`tagged={}` vs `untagged={n:42,…}`).  No error, so a `parses()`-only check misses it. | `walk_parsed_into` (`structures.rs`): a single-key object whose key equals the struct's **type name** unwraps to the struct body |
| **enum-struct** `V{…}` | **NOT A GAP** — `json.rs:224` already makes `Tag{…}` → `{"V":{…}}` and the `Parts::Enum` walker dispatches it to the variant.  ✓ | none |
| **qualified enum** `Enum.V` (dotted Ident) | **HARD ERROR** at the `.` (byte 27) — the lenient ident scanner stops at `.` | json.rs: lenient ident accepts `A.B` segments → `Ident("Enum.V")`; walker maps by the **last** segment against the known enum |

Confirmed **no-gap** by the matrix: unquoted keys, bare enum tags, escaped text
(`"a\"b\nc\\d"`), whole-number float on the DB parser; and every construct on the
language parser.  Open `show_loft`-side note (serializer, not parser): a float
must render **`3.0`** not Rust's `3`, or the language parser re-reads it as `int`.

**Enum qualifier — optional where inferable, reconciled.**  The serializer emits
**qualified** `Enum.V` because the REPL re-binds `a = <value>` in a context with
**no** inferable enum type (the language parser needs the qualifier there).  Both
deserializers accept the qualified form, **and** still accept the bare `V` where
the type *is* known (DB: from `tp`; language: from an inferable context) — so the
language's "bare variant where inferable" rule is preserved; the serializer just
always plays it safe with the qualified form.

**Scope note.**  This promotes own-format from a REPL-internal trick to **loft's
native serialization format** (`show_loft` + a Lenient-dialect deserialize),
sitting beside the JSON pair (`show_json` + `parse`) — extensible with the
language, the symmetric foundation for data literals + session persistence, and
consumed by REPL.X.  It is correspondingly **larger than the REPL.X interim
alone**: the `json.rs`/`walk_parsed_into` fixes land first (with a DB-parser
round-trip test), then `show_loft`, then the REPL wiring.

---

## Convergence — REPL.X, auto-resume, and persistence are one design

*(evaluated 2026-06-08)*

Three open REPL problems share a single fix: **make bindings store-resident
records instead of replayed source.**

- **REPL.X (no re-run):** if a binding's value lives in a store, observing it
  reads the store — the RHS never re-executes, so side effects don't repeat.
- **Auto-resume (REPL.S):** the session heap is then just stores, and stores
  already persist (below).
- **Exact restart:** because nothing re-executes on restore, every computed
  value returns *verbatim* — including non-deterministic ones (`random()`,
  `now()`).  Text-replay cannot do this: it re-runs the generators and draws new
  values.  (The generator's *forward* state is deliberately not restored — see
  "RNG" below.)

**Why not "mmap the stack".**  `State` (src/state/mod.rs:111) is not a flat
buffer: it holds `HashMap`s, `Arc`, `Vec<CallFrame>`, `coroutines: Vec<Box<…>>`,
and a raw `data_ptr: *const Data`.  Restoring that at a new base address would
dangle, and the stack is transient — nothing lives on it between inputs.

**Why the stores DO mmap.**  A `Store` is a word-addressed buffer whose pointers
are logical `DbRef{store_nr, rec, pos}` (src/keys.rs:202), not native addresses,
so the bytes are position-independent and survive mmap-restore at any base
(src/store.rs:23).  Stores are already mmap-backed with CRC + corruption
rejection (`file: Option<MmapStorage>`, src/store.rs:119).  And the save/load
already ships for the stdlib: `Bundle { data, types }` serialized into a store
(src/data_store.rs:406), "save the stdlib to a `.store` file, load it back
(mmap, no re-parse)" (src/ir_read.rs:1287), keyed + invalidated by a content
hash (src/cache.rs:181).  Session resume = that startup-cache mechanism applied
to the user's session store.

**RNG — values are stored, generator state is deliberately not.**  Drawn random
values sit in the session store like any value, so they restore exactly.  The
PCG generator state lives in the `random` cdylib (the single source of RNG state
for both backends — see the src/ops.rs comment), not in a store, and is **not**
snapshotted: restoring saved RNG state would make the stream reproducible from
the session image (predict/replay future `random()` outputs — a security
hazard).  On resume the generator continues fresh (re-seeded from entropy, as on
any launch); reproducible streams stay an explicit-seed opt-in (`random_seed`).
Declined — [DESIGN_DECISIONS.md § C72](../../DESIGN_DECISIONS.md#c72--repl-session-resume-does-not-persist-rng-generator-state).

**What the store-resident model still needs** (beyond the mmap, which is built):

1. A store-resident **binding environment** (name → `DbRef`/scalar); today the
   name→value map is regenerated by replaying `body`.
2. **Scalars at rest** (`x = 5`) boxed into the store (or a tiny text residue).
3. **Schema-version gating** — stamp the image with the cache key so a loft
   upgrade rejects a stale image and falls back to fresh (infra exists).
4. Not portable/shareable (endian + layout + binary specific) — fine for a local
   session, not for sharing.

Items 1–2 are exactly Approach A/B's hard part.  So **do not build the
store-resident model for resume alone — that is over-engineering.**  Ship
text-replay auto-resume first (portable, upgrade-proof, fault-tolerant).  Build
the store-resident image *when* you build REPL.X: then one design collapses all
three problems and reuses the startup cache.

---

## Original design (bytecode-append — NOT pursued; kept for context)

## Goal

Make `State` re-runnable across REPL inputs:

- Bytecode is **appendable**: each new statement adds bytes to the
  existing `bytecode` buffer; previously-emitted code remains
  valid (offsets and jumps stay correct).
- State has a **`reset_for_repl()`** method that clears
  per-execution state (stack, code-pos, call-stack) without
  losing `database`, `bytecode`, `const_refs`, or
  `string_from_const_store`.
- Worker / par execution paths keep working — the bytecode shared
  to par workers must still be safely accessible after appends.

## Design

### Bytecode appendability

Today: `state.bytecode: Arc<Vec<u8>>` is built once via
`compile::byte_code()` and is immutable.  Par workers
`Arc::clone()` it for read-only access.

REPL: each `parse_statement` produces an additional bytecode
segment for the new `__repl_N` synthetic fn (and possibly bytecode
for new top-level fns if the input defined any).

Two implementation options:

#### Option A — Single growable Vec (no Arc)

Replace `Arc<Vec<u8>>` with `Vec<u8>` (or `Arc<Mutex<Vec<u8>>>`).
`compile::byte_code()` becomes incremental: each call appends to
the existing buffer.

**Pro**: minimal touch; the existing bytecode emission machinery
already pushes to a `Vec<u8>` internally.
**Con**: par workers currently `Arc::clone()` the bytecode for
zero-copy sharing.  Switching to `Mutex<Vec<u8>>` adds lock
overhead per opcode read; switching to `Vec<u8>` requires the
worker to clone the whole buffer.  REPL is single-threaded, so
appears acceptable, but par calls inside REPL would need the
worker to snapshot the buffer at par-start.

#### Option B — Per-statement segments

Replace the single buffer with `Vec<Arc<[u8]>>` segments, indexed
by entry-point d_nr.  Each statement emits its own segment.
`code_pos` becomes `(segment_id, offset)` — invasive for every
opcode emitter.

**Pro**: zero-copy par sharing per segment; segments never grow.
**Con**: substantial refactor of every jump / call / opcode handler.

### Recommendation

**Option A with per-call snapshot for par.**  The REPL's primary
use-case is single-threaded interactive eval; par calls inside the
REPL are rare and can tolerate a buffer clone at par-start.

Concrete change:
- `State::bytecode: Vec<u8>` (no Arc).
- `compile::byte_code()` appends; existing callers (file mode)
  call once and never append.
- Par-worker dispatch (`src/parallel.rs::WorkerProgram`) clones
  the buffer at construction.  Single allocation per par call,
  bounded by program size — not per-row.

### `State::reset_for_repl()`

```rust
impl State {
    pub fn reset_for_repl(&mut self) {
        // Clear per-execution state.
        self.stack_pos = 0;
        self.code_pos = 0;
        self.def_pos = 0;
        self.call_stack.clear();
        self.eval_stack.clear();
        // Preserve: database, bytecode, const_refs,
        // string_from_const_store, fn_positions, types.
    }
}
```

The REPL calls `reset_for_repl()` after each input runs to
completion (success or runtime error).  Database isn't reset —
the user's defined values stay alive.

### Const-refs across statements

`State::const_refs` is a `Vec<DbRef>` (each entry an interned
literal).  Each new statement may add entries.  `reset_for_repl`
preserves the existing entries; `compile::byte_code()` appends
new ones.

`string_from_const_store` follows the same rule.

### Fn-position registry

`State::fn_positions: Vec<u32>` maps `d_nr → bytecode offset`.
Today filled once via `compile::byte_code()`.  REPL: extend with
new entries for each `__repl_N` synthetic and any user-defined fn
in the input.

### Worker bytecode safety

`WorkerProgram::bytecode` (`src/parallel.rs`) currently
`Arc::clone()`s the State's bytecode.  After option A, the worker
gets `state.bytecode.clone()` — a fresh `Vec<u8>` per par call.
Worker bytecode lives for the duration of the call; no append
during execution.

## Implementation outline

| Step | Files | Effort |
|------|-------|--------|
| 1. `State::bytecode` field type change `Arc<Vec<u8>>` → `Vec<u8>` | `src/state/mod.rs` | XS |
| 2. Update every read site (`*self.code::<T>()` etc.) | `src/state/mod.rs`, `src/state/text.rs`, `src/state/io.rs`, `src/codegen_runtime.rs` | S |
| 3. `compile::byte_code()` appends | `src/compile.rs` | XS |
| 4. `WorkerProgram::new` clones bytecode | `src/parallel.rs` | XS |
| 5. `State::reset_for_repl` + tests | `src/state/mod.rs` | XS |
| 6. Const-ref / fn-position append paths | `src/state/mod.rs`, `src/compile.rs` | S |
| 7. Round-trip test: parse statement, execute, parse another, execute, verify state | `tests/repl_state.rs` (new) | S |

## Tests

### Single statement → execute → reset → another statement

```rust
#[test]
fn repl_state_round_trip() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    let mut state = State::new(p.database.clone());

    // First statement: x = 42
    let r1 = p.parse_statement("x = 42");
    let entry1 = match r1 { ParseResult::Ready { entry_def_nr } => entry_def_nr, _ => panic!() };
    compile::byte_code(&mut state, &p.data);
    state.execute_at_def(entry1, &p.data);
    state.reset_for_repl();

    // Second statement: y = x + 1
    let r2 = p.parse_statement("y = x + 1");
    let entry2 = match r2 { ParseResult::Ready { entry_def_nr } => entry_def_nr, _ => panic!() };
    compile::byte_code(&mut state, &p.data);   // appends
    state.execute_at_def(entry2, &p.data);

    // Verify __repl_session.y == 43.
    let session = state.database.lookup_repl_session();
    assert_eq!(session.y, 43);
}
```

### Par-call inside REPL session

A regression test that defines a fn using `par(...)` then invokes
it from a REPL input, verifying the worker bytecode clone path
works.

## Acceptance criteria

1. Each `parse_statement` + `compile::byte_code` + `execute`
   cycle leaves `state.database` mutated as expected, with all
   prior state intact.
2. `state.reset_for_repl()` returns the State to a "ready for
   next call" shape with stack / code-pos / call-stack zeroed
   and database / bytecode / const-refs preserved.
3. Par calls from REPL inputs succeed (worker clones bytecode at
   par-start; no segfault from buffer-relocation under append).
4. File-mode execution (`cargo run --bin loft -- file.loft`)
   keeps working unchanged — `compile::byte_code` is called once
   and no `reset_for_repl` happens.
5. Full test suite green.

## Effort

**MH (~3–4 days).**  Step 2 (every bytecode read site) is the bulk —
changing `Arc<Vec<u8>>` to `Vec<u8>` ripples across the runtime.
Step 6 (const-ref append) needs care: the const-ref allocator
shouldn't reset between statements but new entries must register
correctly.

## Risk

- **Worker bytecode invalidation under append.**  If a par call
  is in flight and the main thread appends bytecode, the worker's
  `Arc<...>` was a snapshot — but if we switch to `Vec<u8>` the
  worker holds its own clone, immune to the append.  Verify via
  a stress test (par call in flight while another thread appends).
- **Memory growth** — never-resetting bytecode grows with every
  REPL input.  Mitigation: phase 03 doesn't garbage-collect
  bytecode; if memory becomes a problem, a `:reset` REPL command
  (phase 04) wipes the buffer.
- **Stack-position drift** — `reset_for_repl` zeros `stack_pos`
  but the database may still hold DbRefs into freed stack slots.
  Mitigation: after reset, verify no DbRef in the database has
  `store_nr == 0` (the stack store) before next call.  Same
  invariant the test runner uses today.

## Out of scope

- **GC of orphan bytecode segments** — when a `__repl_N`'s
  symbolic name is reassigned, the old segment is unreachable but
  the bytes stay.  Acceptable given session lifetimes.
- **Bytecode persistence across REPL launches** — start fresh each
  time.  Phase 06 may add session save/load.

## See also

- [00-baseline.md](00-baseline.md) — bytecode + state survey.
- [02-statement-parser.md](02-statement-parser.md) — produces the
  IR this phase compiles + executes.
- [04-repl-shell.md](04-repl-shell.md) — drives this phase from
  user input.
- `src/state/mod.rs` — `State` struct + bytecode field.
- `src/compile.rs::byte_code` — current single-shot compiler.
- `src/parallel.rs::WorkerProgram` — par worker bytecode handle.
