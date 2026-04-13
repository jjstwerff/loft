
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

### C3 — WASM backend: `par()` runs sequentially
A Web Worker pool has real bundle-size and startup cost that most
browser loft programs don't want.  Revisit only when a concrete game
demands it (W1.18 / 1.1+).  **Workaround:** use native for CPU-bound
parallel work.

### C38 — Closure capture is copy-at-definition
Captured values are copied into the closure at definition time, like
Rust `move`.  Mutations to the outer variable after capture are not
visible inside the lambda.  Reference capture would require either GC
or borrow tracking, neither of which fits the "simple, fast, no
lifetime annotations" ethos.  **Test:**
`tests/scripts/56-closures.loft::test_capture_timing`.

---

## Scheduled — 0.8.5

### P137 — `loft --html` Brick Buster: runtime `unreachable` panic
The headline browser build wedges on the first call to `loft_start`;
native mode works.  Blocks the "share a link, anyone plays" story and
the Moros editor ships on the same WASM path.  Fix path: phase-C
bisection of `#native` functions (detailed in PROBLEMS.md #137).

### P135 / C58 — Canvas Y direction is not locked in
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

### C60 — Hash iteration in key order (designed 2026-04-13)

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

**Step 3 — Parser accepts `for e in hash`.**  Replace the
"Cannot iterate a hash directly" error at `src/parser/fields.rs:599`.

**Budget constraint confirmed 2026-04-13:** `src/fill.rs::OPERATORS`
is at 254/254 slots, so a dedicated opcode (original path 3b) is
ruled out without first retiring an existing opcode.  Path 3a —
a named native function — is the right vehicle:

- Register `n_hash_sorted` in `src/native.rs` beside
  `n_parallel_for_int` (line 60 pattern).  The Rust impl:
  1. Pop the hash's `DbRef` and type-id from the stack.
  2. Call `hash::records_sorted` with `stores.keys(tp)`.
  3. Allocate a fresh store via `stores.null()`, claim vector records
     (`stores.claim`), fill with each rec-nr as a `DbRef{store_nr,
     rec, pos:8}`, and write the count at offset 4 of the data
     record.  Same shape as `n_parallel_for_int`'s vector-building
     path at `src/native.rs:494-521`.
  4. Push the resulting `DbRef` back on the stack.
- Declare in `default/01_code.loft`:
  `pub fn hash_sorted(h: reference) -> reference;`
- Parser rewrite at `src/parser/fields.rs:599`: treat `for e in h`
  (where `h`'s type is `Type::Hash(content, _, _)`) as if the source
  had been `for e in hash_sorted(h)`, annotating the call result type
  as `Type::Vector(content, …)` at the call site so subsequent
  vector-iteration codegen proceeds unchanged.  The type annotation
  is a purely parser-local bookkeeping step — the runtime treats the
  returned reference as a vector regardless.

*Test:* `tests/issues.rs::c60_hash_iter_single_field_asc` (already
`#[ignore]`, acceptance criterion for this step).

**Step 3b parser desugar attempt (2026-04-13, reverted):** wrote the
desugar at the right site (`parse_for` just before the vector-temp
branch at `src/parser/collections.rs:929`), compiled, and ran — the
iteration loop body fired the correct number of times but the loop
variable's field reads returned garbage (`name=""` `count=8` for a
single-entry hash holding `{name:"apple",count:5}`).

Diagnostic: `Stores::build_hash_sorted_vec` writes each DbRef with
`pos=8` (matching the `hash::find` / `hash::validate` convention
for record bodies inside a hash), but vector-iteration field access
treats the element as a `reference<T>` pointing at `pos=0` with the
struct field offsets added on top.  The hash-record layout has 8
bytes of internal header (hash_val + next-ptr) before the struct
body, so field accesses land 8 bytes early.

**Next-session fix path (not started):** either
(a) make `build_hash_sorted_vec` write DbRefs with `pos=8` AND
    the vector codegen treat `reference<T>` elements with the
    correct pos offset — requires threading pos-awareness through
    the field-access machinery, or
(b) copy each hash-record's body bytes into a contiguous vector
    of struct records (not references), avoiding the pos=8 issue
    entirely but costing an extra copy per iteration.

Path (b) is simpler and matches how users already think of "iterate
a hash" — as iterating copies, not references.  Recommend (b) for
the next attempt.

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
The typeless API contradicts loft's type-system promise, but the full
`JsonValue` enum (Object / Array / String / Number / Boolean / Null)
is a large design surface for a problem that's mostly "distinguish
'valid JSON body' from 'garbage' at the type level".  **Decision
(revised):** introduce a typed newtype `JsonBody` that wraps `text`,
returned by `json_items` and accepted only by `MyStruct.parse`.  Adds
`.is_object() / .is_array() / .is_null()` for cheap shape checks.
Dynamic shape-unknown access (`v["users"][0]["name"]`) is deferred to
1.1+ when someone asks for it with a concrete use case.  80% of the
type-safety gain for 20% of the design surface.

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
| C58/P135 | 0.8.5   | Canonical `(0, 0) = screen-top-left`; lock in LOFT.md; re-bake brick-buster atlas |
| C60    | 0.9.0     | `for (k, v) in hash` returning `(K, V)` tuples in unspecified order |
| ~~C61.local~~ | — | **Done** — pass-1 reject via `was_loop_var`; stdlib docs cleaned up; unblocked by #139 |
| P54    | 0.9.0     | `JsonBody` newtype + `.is_object/array/null()`; full `JsonValue` deferred to 1.1+ |
| ~~P91~~ | — | **Done** — call-site substitution of `Var(arg_index)` in stored default tree; 4 regression tests |
| P137   | 0.8.5     | Browser WASM `unreachable` panic fix |

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
