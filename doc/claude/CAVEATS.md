
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Caveats

Real edge cases that bite loft programmers today.  Each entry either has
a decided fix (with a milestone) or is an accepted trade-off we intend
to keep.  Entries that merely document shipped diagnostics or internal
compiler details belong in CHANGELOG.md / LOFT.md / SLOTS.md, not here.

**Maintenance rule:** when an entry is fixed, delete it; when it becomes
a design-accepted fact, move it to LOFT.md § Design decisions.

---

## Accepted trade-offs (not scheduled for change)

Closed-by-decision entries live in
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).  Short pointers kept
here for cross-reference; don't re-argue these in active caveat
tables.

- **C3** — WASM `par()` runs sequentially.
  See [DESIGN_DECISIONS.md § C3](DESIGN_DECISIONS.md#c3--wasm-par-runs-sequentially).
- **C38** — Closure capture is copy-at-definition.
  See [DESIGN_DECISIONS.md § C38](DESIGN_DECISIONS.md#c38--closure-capture-is-copy-at-definition).
  Regression guard: `tests/scripts/56-closures.loft::test_capture_timing`.

---

## Scheduled — 0.8.5

### ~~P137~~ — `loft --html` Brick Buster: runtime `unreachable` panic — DONE

Shipped on `quality`.  Root cause: `Instant::now()` in
`Stores::new()` panics on `wasm32-unknown-unknown` (the `--html`
target).  Fix: guard switched from `#[cfg(not(feature = "wasm"))]`
to `#[cfg(not(target_arch = "wasm32"))]`; `host_time_now()` returns
0 in that mode; `n_ticks` gated identically.  The headline browser
demo and Moros editor share the same WASM path, both unblocked.
Regression guards: `tests/html_wasm.rs` (4 tests behind a
process-wide serial mutex covering hello-world, ticks, two
allocator paths).  Detail in PROBLEMS.md #137.

### ~~P135 / C58~~ — Canvas Y direction — DONE

Shipped on `quality`.  The three-way flip cascade (upload row-reverse,
TEX_VERT_2D `1 - aPos.y`, ortho `-2/H`) collapsed to one: the ortho
is the only compensating flip, matching the GL convention.  Canvases
and PNG textures now share the same orientation in GL.  Locked as a
language-level invariant in [OPENGL.md § Canvas coordinate
convention](OPENGL.md).  Regression guard: 2×2 atlas corner check in
`tests/scripts/snap_smoke.sh`.

### P135 / C58 (historical) — Canvas Y direction is not locked in
Three compensating flips (upload row-reverse, UV, 2D projection) that
don't cancel on non-square atlases.  **Decision:** canonical `(0, 0) =
screen-top-left`, `y` grows down — matches HTML canvas, PNG files, and
how users think about 2D drawing.  The 3D pipeline's internal OpenGL
texture-coordinate math stays internal.  Lock this as a language-level
guarantee in LOFT.md so future backends cannot drift.  Rebake Brick
Buster's atlas (the only loft program with a non-trivial layout).
**Test:** extend `snap_smoke.sh` with a 2×2 atlas corner check.

---

## Scheduled — 0.9.0

### P142 — `vector<T>` field panics when T is from an imported file

A struct field of type `vector<T>` (or `hash<T>`, `index<T>`,
`sorted<T>`) panics during type resolution when `T` is defined in a
different `.loft` file loaded via `use`.  The panic is in
`src/typedef.rs::fill_all` — the vector content type is resolved
before the imported struct's def-nr is registered.

**Reproducer:**
```
# inner.loft
pub struct Inner { val: integer not null }

# outer.loft
use inner
pub struct Outer { items: vector<Inner> }
```

Same-file definition works fine.  **Workaround:** keep all structs
that reference each other via generic collection fields in the same
`.loft` file.  Applied in the Moros `moros_map` package.

**Regression guard:** none yet — needs a Rust-level test in
`tests/package_layout.rs` with a two-file test package.

### C54 — `integer` representation
Silently returning null when arithmetic lands on `i32::MIN` (and
debug-aborting) is actively hostile in a language pitched as "reads
like Python".  **Decision:** switch `integer` from `i32` to `i64`.
At 8 bytes per field the overhead is rounding error on today's
machines; `i64::MIN` is `-9.2e18`, so accidental sentinel collisions
effectively vanish.  `long` becomes a historical alias; future code
writes `integer` and Just Works for any arithmetic.  Breaking change,
so 0.9.0 is the right window (pre-1.0 stability contract).
Bumping struct field sizes requires revisiting `size(Type::Integer)`
in `src/database/mod.rs` and every schema layout test.

### ~~C60~~ — Hash iteration in key order — DONE 2026-04-13

Shipped on branch `quality`.  `for e in h { ... }` walks the hash in
ascending key order, yielding `reference<T>` — same shape as
`sorted`/`index`.  Implementation: the parser substitutes the iterated
expression with a `hash_sorted(h, tp)` call that builds a u32-stride
rec-nr scratch in the hash's own store (allocation co-location lets
the yielded `DbRef{store, rec, pos=8}` resolve directly to live hash
records).  Iteration routes through the existing `Ordered` (`on=3`)
bytecode — no new opcodes, no new runtime mode.

Commits: pieces 1 (`e50fffe`), 2 (`8d4d573`), edit A (`2e20ba2`),
edit E (`63226b8`), piece 3 (`2145a8d`), native (`0b85cd2`), Step 9
`#remove` diagnostic (`705338e`), docs Step 4 (`363ed12`).  Six
acceptance tests green in `tests/issues.rs::c60_hash_iter_*`.

Original design archived below for reference.

### C60 (original design) — Hash iteration in key order (designed 2026-04-13)

A collection type you can't iterate breaks the "vector, hash, sorted,
index" promise.  **Decision (revised):** `for e in hash` iterates in
**ascending key order**.  Determinism wins over efficiency — the lift
costs O(n log n) per loop, and users who care about that use `sorted`
or pair the hash with a `vector<K>`.  This is *not* the earlier
"unspecified order" decision; deterministic order is worth paying for.

#### Syntax

Mirror the other collection types — the loop variable is the
**record**, not a tuple:

```loft
struct Entry { name: text, count: integer }
struct Bag   { data: hash<Entry[name]> }

b = Bag { data: [
    Entry{name:"zebra", count:1},
    Entry{name:"apple", count:5},
    Entry{name:"mango", count:3},
] };

for e in b.data {              // visits apple, mango, zebra (ascending name)
    println("{e.name}={e.count}");
}
```

No new tuple-destructuring syntax is required (keeps the for-loop
head simple and consistent with `for e in vector`, `for e in sorted`,
`for e in index`).  Users read the "key" via plain field access on
the iterated record.

#### Multi-field and descending keys

- `hash<T[a, b]>` iterates in lexicographic order of `(a, b)`.
- `hash<T[-score]>` iterates descending — the `-` prefix matches the
  existing `sorted`/`index` convention.
- `hash<T[region, -date]>` combines both.

#### Iteration invariants (documented)

- **Order**: ascending on each key field, `-` flips per-field.
- **Mutations during iteration**: adding/removing entries is unspecified
  (may miss, may double-visit); modifying a key field on an iterated
  record is unspecified (order invariants break).  Loft does not
  guarantee snapshot iteration — the sorted scratch references the
  original records.
- **Empty hash**: zero iterations.
- **Loop attributes**: `#index` (0-based position in the sorted
  iteration), `#count` (iterations so far), `#first` (true on first).
  `#remove` is **not** supported (invalidates the sort order).
- **Filter clause**: `for e in h if e.count > 10 { … }` works — same
  as other collection filters.

#### Implementation sketch

**Parser** (`src/parser/fields.rs:599`): replace the current
`"Cannot iterate a hash directly"` error with a new iteration code
(`on = 4` alongside Vector=1, Sorted=2, Index=3).  Route it to a new
helper `parse_iter_hash` in `src/parser/collections.rs`.

**Lift at loop setup**: before the loop body, codegen emits a
pre-loop block that:

1. Allocates a scratch `vector<reference<T>>`.
2. Walks the hash's record-store for the struct type and collects a
   reference to each live record into the scratch.  The walk uses the
   existing `Stores::walk_records(db_tp, callback)` pattern already in
   `src/database/search.rs` — new helper needed if none matches
   exactly, otherwise the "validate" walk at `search.rs:327` is the
   right shape.
3. Sorts the scratch by extracting key fields from each reference.
   The sort comparator is generated from the hash's `Vec<u16>` key
   field indices (stored in `Type::Hash(content, key_fields, _)`).
4. Iterates the scratch as a normal `vector<reference<T>>` loop —
   reusing the existing vector-iteration codegen path.

**Native codegen**: same sequence in emitted Rust.  Each key-field
access becomes a direct field read; the sort uses Rust's
`slice::sort_by` with a generated comparator.

**Interpreter**: new opcode `OpHashCollect(hash_ref) -> DbRef` that
walks the hash's records into a fresh vector and returns it.  The
sort is a separate pass using existing vector-sort machinery.
Alternative: a single `OpHashIterSetup(hash_ref) -> DbRef` that
produces a sorted vector in one step — saves a bytecode op at the
cost of less composability.

**Scope honestly**: **M–MH**.  New opcode + database walk + sort
integration + parser route.  Two days of work if nothing else bites,
up from the "medium" rough estimate — but the design is concrete and
the scope is bounded (no tuple-destructuring, no new iterator
protocol, no bucket-walk in `src/hash.rs`).

#### Implementation: 9 independently-testable steps

Each step lands as its own PR with its own test.  A later step may
depend on an earlier one, but nothing requires "land it all at once".
A session that runs out of time mid-way leaves the codebase in a
working state with partial feature coverage.

**Step 1a (DONE 2026-04-13)** — `hash::records` Rust primitive in
`src/hash.rs` walks the bucket array in internal order.  `#[allow(dead_code)]`
until a loft-level caller lands in Step 3.  Tests:
`tests/data_structures.rs::hash_records_walk`, `hash_records_empty`.

**Step 2 (DONE 2026-04-13)** — `hash::records_sorted` sorts the
Step 1 output by the hash's key fields using the existing
`keys::compare`.  Covers multi-field lexicographic order for free
(Step 6 merged here).  Ascending-only; the `-` descending prefix
(original Step 7) turns out to be out-of-scope — hash keys are
ascending-only at the schema level per
`src/parser/definitions.rs:1198`.  Tests:
`hash_records_sorted_single_field`, `hash_records_sorted_multi_field`.

**Step 3 — Parser accepts `for e in hash` (locked path 2c,
2026-04-13).**  Replace the "Cannot iterate a hash directly" error at
`src/parser/fields.rs:599` with a route that emits `on=4`, a new
hash-iteration mode handled entirely by the runtime.

Three paths were evaluated after the session-2 parser desugar
attempt (commit `f5d4272`, reverted) revealed the layout pitfall.
Path 2c is chosen because it preserves the design mandate —
**hashes behave like any other data structure** — most directly:
the parser change is a two-line update to `fill_iter`, and
everything else (loop attributes, filter clause, field access) is
handled by existing Sorted/Ordered iteration code reused unchanged.

**Rejected paths:**

- **2a (first-class `Type::Ordered`).** Correct but crosscuts type
  inference, parse_type_full, serialisation, and the `get_type`
  resolver.  Weeks of work; `Parts::Ordered` today is purely a
  database-level degradation of `sorted<T[k]>` (`src/database/types.rs:261`)
  with no user-facing type.  Overkill for "let me iterate a hash".
- **2b (parser IR desugar).** Emits explicit low-level loop
  (`Insert([Set(scratch, hash_sorted(h)), Loop(...)]))`) that reads
  rec-nrs from a scratch vector and synthesises references at
  pos=8.  Requires a new IR primitive — "construct `Reference<T>`
  from `(store, rec, pos)`" — that loft doesn't have.  Adding it
  opens questions about lifetime/dep tracking for synthetic refs.
  Verbose and leaks the desugaring into every hash-iteration user's
  IR dumps.

**Chosen path 2c — runtime `on=4` mode.**

The parser treats `Type::Hash` identically to `Type::Sorted` in
`fill_iter`: emit iterator setup with `on=4, arg=<hash type id>`.
At runtime, the existing `OpIterate` / `OpStep` dispatch on `on`;
adding `on=4` arms is a non-invasive extension (no new opcode slot
— the dispatch is a `match on & 63 { 1=>…, 2=>…, 3=>…, _=>panic }`
at `src/state/io.rs:575` and `:720`).

**`iterate()` on=4 arm (src/state/io.rs:551):**

1. Read the hash `data: DbRef` from the stack (same as on=1/2/3).
2. Call `stores.build_hash_sorted_vec(&data, arg as u16)` — the
   existing helper at `src/database/allocation.rs` (commit
   `deabb62`) builds a fresh `u32`-stride vector of rec-nrs sorted
   by the hash's key fields.  **Rewrite** that helper to write
   `u32` rec-nrs at 4-byte stride (not 12-byte DbRefs) — this is
   the one runtime layout fix beyond the parser tweak.
3. Stash the scratch vector's DbRef in a companion loop-local
   allocated by `parse_for_iter_setup` (src/parser/collections.rs:806)
   — named `{id}#hash_scratch`, 12 bytes, lifetime = the loop.
4. Push `start=0` and `finish=len(scratch)` — same two-u32 protocol
   as on=2/3.

**`step()` on=4 arm (src/state/io.rs:708):**

1. Read the scratch DbRef from the companion slot allocated in
   iterate step 3.
2. Advance `cur` to the next position (trivial: `cur+1` until
   `finish`).
3. Read the u32 rec-nr at `scratch.pos + 8 + cur*4`.
4. Return `DbRef{store_nr = original hash's store, rec = <u32>,
   pos = 8}`.  **Matches Ordered's yield shape identically** —
   field accesses on the loop variable go through the standard
   `reference<T>` field-offset path with pos=8.

**Parser-side:**

- `src/parser/fields.rs:599` (the current "Cannot iterate" error) —
  replace with `Parts::Hash(_, _) => { on = 4; arg = known; }`.
- `src/parser/collections.rs:806` (`parse_for_iter_setup`) — when
  the iterated type is `Type::Hash`, allocate the
  `{id}#hash_scratch` companion variable alongside `{id}#index`.
  Pass its slot offset into `OpIterate`'s operand stream as a new
  `u16` argument.  The existing on=1/2/3 arms ignore this extra
  operand; on=4 consumes it.

**Why this matches the "uniform with other collections" mandate:**

| Aspect | Sorted/Index | Hash (on=4) |
|---|---|---|
| For-loop syntax | `for e in s` | `for e in h` |
| Element type | `reference<T>` | `reference<T>` |
| Yielded `pos` | `8` (+ stride for Sorted) | `8` |
| Loop attributes | `#index`/`#count`/`#first` | same, same dispatch |
| Filter clause | `for e in s if …` | same |
| `#remove` | allowed / diagnosed per-collection | rejected with hint (Step 9) |
| Parser work | `fill_iter` sets `on=1/2/3` | `fill_iter` sets `on=4` |

There is no observable difference at the user level — hash is just
another iterable collection.

**Scope honestly: M.**  One helper rewrite (`build_hash_sorted_vec`
to emit 4-byte rec-nrs), two runtime arms (iterate + step at
on=4), two parser edits (`fill_iter` and `parse_for_iter_setup`
companion variable).  Every piece is bounded; each goes into its
own commit following DEVELOPMENT.md's test-first sequence.

**Piece 1 landed 2026-04-13 (commit `e50fffe`).**
`Stores::build_hash_sorted_vec` now emits u32 rec-nrs at 4-byte
stride.  Unit test `tests/data_structures.rs::hash_sorted_vec_u32_layout`
validates the layout.

**Pieces 2–5 session-2 attempt (2026-04-13, not committed):** fill_iter
hash arm flipped to `on = 4; arg = known;` and the codebase built
clean.  But running `for e in h { println("{e.name}"); }` hit pass-1
"Unknown type null" on the field access — because the type flow
through `parse_for_iter_setup` is NOT just fill_iter.  That function
determines the loop-variable type via `for_type(&in_type)` which for
`Type::Hash` returns something that doesn't land on a struct
reference.  So pieces 2–5 are more tangled than the pure fill_iter
edit suggests.

**Concrete next-session start:** check `for_type` (at
`src/parser/control.rs:1901`) for the `Type::Hash` arm.  It needs to
return `Type::Reference(content, dep)` when the hash is being
iterated, same as Sorted/Index do.  That's the parser-side
prerequisite before flipping fill_iter.  Runtime on=4 arms come after.

**Step 4 — Ship Steps 1–3 as the minimum viable hash iteration.**
Nothing new to implement; just land the combined behaviour, update
`doc/12-hash.html` source, delete the caveat-level documentation of
"cannot iterate".

*Test:* integration — hash iteration used in a real loft program
compiles and runs under both interpreter and `--native`.

**Step 5 — Loop attributes (`#index`, `#count`, `#first`).** Because
Steps 1–3 desugar to a vector iteration, these work "for free" via
the existing vector-iteration path.  Confirm and test.

*Test:* `for e in h { total += e.count * (e#index + 1); }`
produces the expected weighted sum.

**Step 6 (DONE 2026-04-13, merged into Step 2).** `keys::compare`
already supports multi-field lexicographic order.

**~~Step 7~~ — Out of scope.** Hash keys are ascending-only at the
schema level today (`src/parser/definitions.rs:1198` rejects
`hash<T[-k]>` with "Structure doesn't support descending fields").
Supporting descending on hash would be a separate schema change,
not part of C60.  Users who need descending iteration can pair the
hash with a `sorted<T[-k]>` field, which does support `-`.

**Step 8 — Filter clause.** `for e in h if e.count > 10 { … }`.
Because Step 3 desugars to vector iteration, the filter clause on
`for` already works via the existing vector path.  Confirm and test.

*Test:* verify filtering skips records whose condition fails.

**Step 9 — Reject `#remove` with a clear diagnostic.** Hash
iteration uses a pre-sorted snapshot; `#remove` would not remove
from the hash.  Emit a parse-time error:
*"#remove is not supported on hash iteration — the iterated vector
is a sorted snapshot; use `h[key] = null` to remove from the hash"*.

*Test:* parse-error test matching the diagnostic.

Scope per step:
- Steps 1, 2: **S** each (native function + one test).
- Step 3: **S** (parser rewrite, one line of routing).
- Step 4: **XS** (integration + docs).
- Steps 5, 8: **XS** each (confirmation tests, no code).
- Steps 6, 7: **M** together (comparator logic).
- Step 9: **XS** (diagnostic + test).

Total realistic: **one focused day**, down from the earlier "two days"
estimate — the step decomposition removes the speculative overhead.

### ~~C61.local~~ — Outer-local shadow — DONE
`x = 5; for x in …` now rejected on pass 1 via the `was_loop_var`
flag on `Variable` — a slot that exists in `names` but has never
served as a loop variable is unambiguously a plain local, so the
shadow is flagged with a rename-or-drop hint.  Same-typed shadow
only (the existing type-mismatch check handles the different-typed
class with a clearer message).  Sequential same-name loops stay
legal because the prior slot carries `was_loop_var = true`.

Unblocked by PROBLEMS.md #139's `OpReserveFrame` fix, which made the
stdlib rename sweep possible without tripping the slot-allocator
TOS assertion.

**Tests:** `tests/parse_errors.rs::c61_local_shadow_rejected`,
`c61_local_shadow_renamed_ok`, `c61_local_dropped_outer_ok`, plus
the flipped-to-reject `shadow_same_type_ok`.
**Files cleaned up:** `lib/graphics/src/mesh.loft` (dropped dead
`row = 0; col = 0` inits), `lib/parser.loft` (renamed `p` / `f` →
`param` / `fld`), `tests/docs/01-keywords.loft` (renamed `for a`
→ `for i`), `tests/scripts/05-enums.loft` (two loops renamed),
`tests/scripts/39-diagnostics-passing.loft` (flipped the
once-permissive test), `lib/graphics/examples/25-brick-buster.loft`
(renamed `br_rt` → `br_pti`).

### ~~P91~~ — Default-from-earlier-parameter — DONE
Implemented via **call-site substitution** rather than function
prologue (the simpler approach worked).  `parse_arguments` injects
earlier arguments into `self.vars` before parsing each default, then
rewrites the parsed `Value` tree so `Var(slot)` references become
`Var(arg_index)` — a stable, portable form.  At call sites,
`Parser::substitute_param_refs` walks the default tree and replaces
each `Var(N)` with the caller's actual `list[N]` (already substituted
if earlier args also had defaults).

**Tests:** `tests/issues.rs::p91_default_references_earlier_param`,
`p91_default_identity_of_earlier_param`,
`p91_default_overridden_by_caller`,
`p91_chained_defaults_reference_earlier_args`.

### P54 — `json_items` returns opaque `vector<text>`
The typeless API contradicts loft's type-system promise.  **Decision:**
replace the text-based JSON surface (`json_items`, `json_nested`,
`json_long`, `json_float`, `json_bool`) with a first-class
`JsonValue` enum — `JObject` / `JArray` / `JString` / `JNumber` /
`JBool` / `JNull`.  `json_parse(text) -> JsonValue` is the one entry
point; `MyStruct.parse` accepts only `JsonValue` and rejects bare
text at compile time with a fix-it hint.  Full design in
[QUALITY.md § P54](QUALITY.md#active-sprint--p54-jsonvalue-enum).  The earlier `JsonBody`
newtype half-measure is withdrawn — doing the parse once into a
typed tree is simpler, faster, and covers the dynamic-shape case
that a newtype-over-text cannot.

### ~~C7 / P22~~ — `spacial<T>` diagnostic — DONE
Diagnostic updated to surface the 1.1+ timeline: *"spacial<T> is
planned for 1.1+; until then use sorted<T> or index<T> for ordered
lookups"*.  A user who typed `spacial` now knows when the feature
ships and which substitute to reach for.  Keyword retained (more
helpful than a generic "unknown type" would be).  **Tests:**
`tests/parse_errors.rs::spacial_not_implemented`,
`spacial_not_implemented_in_local` (new regression guard for the
local-variable path).

---

## Verification log

Last retested: **2026-04-12** against commit `2aaba5a` (main branch).

| Caveat | Milestone | Decision |
|--------|-----------|----------|
| C3     | 1.1+      | Accepted — WASM threading deferred (Web Worker pool cost > benefit today) |
| ~~C7/P22~~ | — | **Done** — diagnostic now references 1.1+ timeline; regression guard added |
| C38    | —         | Accepted — value-semantic capture by design (like Rust `move`) |
| C54    | 0.9.0     | **Switch `integer` from i32 to i64.** `long` becomes historical alias. Breaking change, pre-1.0 window |
| ~~C58/P135~~ | — | **Done** — canonical `(0, 0) = screen-top-left`; upload no longer pre-flips rows; convention locked in OPENGL.md.  Regression: 2×2 atlas corner check in `tests/scripts/snap_smoke.sh` / `make test-gl-golden` |
| ~~C60~~ | — | **Done** 2026-04-13 — `for kv in hash` yields a `HashEntry` with `.key` / `.value` in insertion/deletion-aware order via the internal ordered index.  See CAVEATS.md § C60 long-form |
| ~~C61.local~~ | — | **Done** — pass-1 reject via `was_loop_var`; stdlib docs cleaned up; unblocked by #139 |
| P54    | 0.9.0     | First-class `JsonValue` enum + `json_parse`; old text-based JSON surface withdrawn |
| ~~P91~~ | — | **Done** — call-site substitution of `Var(arg_index)` in stored default tree; 4 regression tests |
| ~~P137~~ | — | **Done** — `Instant::now()` / `n_ticks` gated on `target_arch = "wasm32"`; `host_time_now()` returns 0 on wasm32-without-wasm-feature.  Regression: 4 guards in `tests/html_wasm.rs` behind a serial mutex |

---

## Moved out of this document

- **C12** (null + `??` instead of exceptions) → design fact, see LOFT.md
- **C45** (zone-2 slot reuse text-only) → internal allocator detail, see SLOTS.md
- **C56, C57** (clean diagnostics for stdlib-name clash / nested file-scope decls)
  → shipped in 0.8.4, see CHANGELOG.md
- **C51, C53, C55, C61-nested** → fixed and deleted
- **P55** (thread-local `http_status`) → design reject, not an open item
- **P90** (per-call HashMap lookup) → premature optimisation, see PERFORMANCE.md

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker (severity, fix paths)
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [LOFT.md](LOFT.md) § Design decisions — accepted language-level trade-offs
