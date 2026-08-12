<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 10 — No materialised output vector

**Status: open** (strategic shift; supersedes phase 2's wall-clock
goal for the value-position case, makes phase 7's fused form the
canonical surface)

## Goal

Drop the materialised result vector from `par` entirely.  `par(...)`
becomes a stream-only primitive: every result is consumed exactly
once, in input order, by a single downstream consumer (a fused
for-loop body, a fold accumulator, or a queue reader).  Constructions
that require random access or multi-pass on the par result are
**rejected at compile time** with a diagnostic that points users at
the streaming alternative.

A future commit (phase 11, out of scope here) re-adds an explicit
`par_to_vec(input, fn, threads) -> vector<S>` helper for users who
genuinely need the materialised vector.  That helper will internally
use phase 2's rebase machinery — so the work landed in 2a + 2b-prep
isn't wasted, just deferred to the explicit-opt-in path.

## Why

### The primary motivation — collapse the par model's complexity

Today's par implementation carries enormous internal complexity for
a use case (random-access materialised vectors) that real programs
rarely need.  Concrete inventory of branching the materialised model
imposes:

- **3 native fns** — `n_parallel_for_native`, `n_parallel_for_text_native`,
  `n_parallel_for_ref_native` (one per output shape).
- **6 runtime variants** in `src/parallel.rs` — `run_parallel_direct`,
  `_raw`, `_text`, `_ref`, `_int`, `_light`.
- **3 dispatch arms** in `src/generation/dispatch.rs` (Text / Reference /
  Primitive) plus per-arm marshalling.
- **2 user surfaces** (`par(...)` vs `par_light(...)`) selected by
  whether the worker allocates.
- **4 getter primitives** (`parallel_get_int` / `_long` / `_float` /
  `_bool`) needed because the output isn't already in a Store.
- **`copy_block` + `copy_claims`** deep-copy infrastructure
  (~600 lines in `src/database/allocation.rs`) — exists almost solely
  to materialise par results.
- **Phase 2's rebase walk** (StoreRebase, rebase_walk_record,
  adopt_worker_excess) — needed to make the materialised path *fast*.
- **Result-vector layout questions** (Path A inline-struct vs Path B
  DbRef-indirection) that block phase 2b's wiring.

Every one of these branches exists because the runtime has to
**produce a `vector<S>` regardless of how the caller uses it** — the
type system doesn't distinguish "I want the vector" from "I'll
iterate it once and forget".

Stream-only par collapses this to **one runtime shape**: workers
write to per-worker output stores; main thread drains store 0, then
1, then 2…, feeding each result into the consumer body.  No copy_block,
no copy_claims, no result-vector allocation, no rebase walk, no
layout decision.  The 3 native fns merge to 1; the 6 runtime variants
merge to 3 (one per Stitch policy); the dispatch arm count drops to
1; the par/par_light split goes away.

Phase 2's rebase work doesn't disappear — it becomes the implementation
detail of phase 11's `par_to_vec` opt-in.  But it stops being the
critical path.

### The secondary motivation — wall-clock / memory

The biggest single allocation in a typical `par` call is the result
vector itself: `N × struct_size` bytes per call.  For 100 K elements
of 256-byte structs that's 25.6 MB allocated, written once by the
stitch pass, then often consumed by exactly one downstream `for r in
results { … }` loop and discarded.

The streaming model removes the allocation entirely:

```
Today (Concat — every par call):              After phase 10:
  workers → output stores                       workers → output stores
  ↓                                              ↓
  stitch into result vector (25.6 MB)           consumer body runs in
  ↓                                              parent thread per result
  for r in results { use(r) }                   (no vector allocated)
  ↓
  drop result vector
```

The wall-clock saving is bigger than phase 2's rebase: phase 2
eliminates the per-element `copy_claims` deep-copy but keeps the
`copy_block` memcpy and the result-vector allocation.  Phase 10
eliminates *all three*.

**For most user code**, the stream-only model is what people actually
write — `for x in input par(r=fn(x), 4) { use(r) }` is the
overwhelmingly common shape.  The materialised vector exists today
only because the runtime didn't have a streaming path; users worked
around its absence by writing `let r = parallel_for(...)` and then
iterating `r`.

## What gets disallowed

The compiler walks every `par`-result variable's downstream uses.
Any of the following triggers a compile error with a "did you mean"
suggestion:

| Disallowed pattern | Diagnostic | Suggested fix |
|---|---|---|
| `r[i]` random access | `par results are stream-only; use \`par_to_vec(...)\` if you need random access` | call `par_to_vec` |
| `len(r)` for filter-shaped par | (1:1 par_for keeps `len(r) == len(input)` and stays allowed) | use `len(input)` |
| `for x in r { … }; for x in r { … }` two passes | `par result already consumed; iterate twice with \`par_to_vec\` to materialise` | call `par_to_vec` |
| `r` passed as `vector<S>` arg | `par result is iterator<S>, fn expects vector<S>` | callee takes `iterator<S>` or caller `par_to_vec` |
| `r` returned from `fn() -> vector<S>` | `cannot return par result as vector<S>; declare return as iterator<S>` | change return type or `par_to_vec` |
| `r` stored in `vector<S>` field | `cannot store par result in vector<S> field` | field type → `iterator<S>` or call `par_to_vec` |
| `r.sort_by(…)` etc. | `sort requires a materialised vector; use \`par_to_vec\` first` | call `par_to_vec` |
| Aliasing `r` (`r2 = r`) | `par result is single-use; cannot alias` | call `par_to_vec` |

The check runs at the same place phase 5b' walks user-fn bodies for
par-safety — it has the IR + scope info to find every use site.

## What still works (no changes)

| Pattern | Stitch policy | Phase |
|---|---|---|
| `for x in input par(r=fn(x), 4) { body(r) }` | Discard / Queue | 7 (fused) |
| `total = par_fold(input, 0, |a,r| a+r, 4)` | Reduce | 7d |
| `par_for_each(input, |r| body(r), 4)` | Discard | 7 |
| `for r in parallel_for(input, fn, 4) { body(r) }` | Queue | 10 (auto-fused) |
| `let r = parallel_for(input, fn, 4); single_for_loop(r)` | Queue | 10 (auto-fused) |
| `total = sum(parallel_for(input, fn, 4))` | Reduce | 10 (auto-detected) |

The compiler's analysis at phase 10 reuses the per-result-variable
walk: if every use of `r` is single-pass and matches a Discard /
Reduce / Queue shape, codegen routes there directly without
materialisation.  The `let r = parallel_for(...)` form remains
syntactically valid; only its allowed downstream uses tighten.

## What gets retired

This phase retires several pieces from earlier @PLAN06 phases:

| Component | Why retired |
|---|---|
| `Stitch::Concat` runtime in `n_parallel_native` | Concat = materialise; no longer reachable |
| `parallel_execute_and_collect`'s `result_db` allocation + `copy_from_worker[_unowned]` calls | No result vector to fill |
| `run_parallel_ref` + `run_parallel_text` 's `Vec<(usize, DbRef)>` batched returns | Workers write into Stitch-specific channels, not batches |
| Phase 2's `copy_from_worker_rebase` (planned for 2b) | Materialise-only path; deferred to phase 11's `par_to_vec` |
| Phase 2's `StoreRebase` runtime wiring (the walk) | Deferred to phase 11 — but the helper code stays in `src/parallel.rs` since phase 11 will use it |
| Phase 4's `vector<S>` return-type signature | Replaced by `iterator<S>` |

What survives:

- **Phase 1's per-worker output stores** — workers still write to
  per-worker output stores; those stores feed Discard / Queue / Reduce
  consumers instead of being deep-copied to a result vector.
- **Phase 1.5's rayon pool** — unaffected.
- **Phase 2a's `StoreRebase` + `rebase_walk_record` + `adopt_worker_excess`** —
  retained as library code for phase 11's `par_to_vec` helper.
- **Phase 3's Stitch policy enum** — Discard/Reduce/Queue stay; Concat retires.
- **Phase 5's auto-light analyser** — extended to also do the
  no-materialise check (reuses the IR walk).
- **Phase 7's fused for-par** — promoted to the canonical par surface.
- **Phase 8's browser workers** — unaffected (Web Worker pool +
  postMessage transfer applies to streaming results too).
- **Phase 9's tuple support** — unaffected (tuple inputs/outputs).

## Per-commit landing plan

### 10a — Diagnose materialising patterns

Add a `Parser::check_par_result_singlepass(d_nr)` that walks every
`Set(v, par_call)` IR node where `par_call` is a `Value::Call(d, _)`
with `d == n_parallel_for` (or its variants).  For each such `v`,
walk the function body collecting `v`'s use sites; if any matches a
disallowed pattern (table above), emit a `Level::Error` diagnostic
at the use site with the "did you mean" suggestion.

Land as a **non-fatal warning** initially (`Level::Warning`) so
existing programs continue to compile while the test suite is
ported to the streaming form.  Promote to `Level::Error` in 10c.

### 10b — Compile streaming patterns to existing Stitch policies

The fused-for-par construct from phase 7 already lowers to
`Value::ParFor` with a Stitch policy.  Phase 10b extends the parser
to recognise the **value-position** par-result patterns and lower
them to the same IR:

| Source shape | Lowers to |
|---|---|
| `for r in parallel_for(input, fn, 4) { body }` | `Value::ParFor { input, fn, body, stitch: Discard if body never references r else Queue }` |
| `total = sum(parallel_for(input, fn, 4))` | `Value::ParFor { ..., stitch: Reduce { init: 0, fold: +} }` |
| `total = parallel_for(input, fn, 4).fold(0, |a,r| a+r)` | same |
| `vec_n = len(parallel_for(input, fn, 4))` | replace with `len(input)` (1:1 par_for) |

This is parser-level desugaring, not runtime work — Stitch::Discard /
Reduce / Queue runtimes already exist (lands in phase 3e + phase 7d).

### 10c — Promote diagnostic to error

Once the test suite is ported, flip `Level::Warning` to
`Level::Error` in 10a's check.  Materialising par results becomes a
hard compile error.  `parallel_for` keeps its existing signature
(`vector<S>` return type) — the type system never sees the
distinction; the par-result analyser is a parse-pass check.

### 10d — Retire Concat runtime

After 10c, Stitch::Concat has no callers.  Delete:

- `parallel_execute_and_collect` (`src/native.rs:898-1068`).
- The Stitch::Concat arm in any runtime dispatch.
- `run_parallel_ref`'s batch-return shape (replaced by per-stitch-policy variants).
- `copy_from_worker[_unowned]` calls in the par path (the helpers stay in `src/database/allocation.rs` for non-par uses).
- The `result_db = stores.null()` allocation at the top of the par dispatcher.

Verify by `grep -r 'Stitch::Concat'` — should return only the enum
definition (kept for phase 11's `par_to_vec` helper to reuse).

### 10e — Update test suite

Every existing `tests/threading.rs` test that does
`let r = parallel_for(...); for x in r { ... }` already lowers to
Stitch::Queue under 10b — no test change.

Tests that materialise (`r[5]`, `len(r)` for filter-shape, store in
field, etc.) need to either:
- Switch to the streaming shape (most cases — the test author wrote
  the materialised form because that was the only option).
- Call the future `par_to_vec` helper (phase 11) and the test stays
  in `tests/threading_chars.rs` D11 with a `par_to_vec` annotation.

Audit `tests/threading.rs` + `tests/threading_chars.rs` + `bench/11_par/`
+ every loft-script that calls `parallel_for`.

## Phase 11 (out of scope here) — `par_to_vec` helper

Re-introduces the materialised vector as an explicit opt-in:

```loft
results: vector<S> = par_to_vec(input, |x| fn(x), 4);
results.sort_by(|a,b| a.id <=> b.id);
median = results[len(results) / 2];
```

`par_to_vec` is a stdlib fn that internally:
1. Creates the result vector store.
2. Runs workers writing to per-worker output stores (phase 1).
3. Adopts each worker store via `adopt_worker_excess` (phase 2b prep).
4. Translates DbRefs via `rebase_walk_record` (phase 2a).
5. Returns the result vector.

So phase 2's rebase work *is* still necessary — just bundled inside
the explicit-opt-in helper instead of being the default path.  The
deferred work shifts to phase 11.

## Acceptance

Phase 10 is done when:

- `let r = parallel_for(input, fn, 4)` followed by single-pass
  consumption compiles + runs identically to today (Stitch::Queue
  routing under the hood).
- Materialising patterns (`r[i]`, `len(r)` for filter-shape, store
  in field, multi-pass) emit a compile error.
- `Stitch::Concat` runtime is deleted; `grep -r 'Stitch::Concat' src/`
  returns only the enum definition + reference from phase 11's
  `par_to_vec` (when phase 11 lands; until then, only the enum).
- `tests/threading.rs` + `tests/threading_chars.rs` all green
  without a `par_to_vec` (phase-10-locked test suite is
  streaming-only).
- `bench/11_par` re-run — wall-clock improvement on owned-struct
  workloads (no allocation of the result vector + no `copy_block`).
- `make ci` green at every sub-commit (10a / 10b / 10c / 10d / 10e).

## Risks

| Risk | Mitigation |
|---|---|
| Existing user code breaks at the materialising-pattern compile error | 10a's warning phase gives a deprecation window before 10c flips to error |
| Determinism / order — Queue stitch must preserve input order | Workers write into per-worker output stores in input order; main thread drains store 0 fully then 1 then 2…; same order guarantee as Concat |
| Backpressure — slow consumer + fast producers fill worker output stores | Worker output stores are bounded; if full, worker blocks until consumer drains.  Same flow control as a SPSC bounded queue.  Acceptable trade for not allocating N × struct_size |
| Some par patterns genuinely need a vector (sort, persistence, multi-pass) | Phase 11's `par_to_vec` provides the explicit opt-in.  Compile error for the implicit path nudges users to call it explicitly so the materialisation cost is visible at the call site |
| Phase 7's fused for-par must be production-ready before 10b | Phase 7 promoted to dependency of phase 10.  10b lands only after 7's Discard / Queue runtimes exist |

## Hand-off and dependencies

```
phase 1   (per-worker output stores) ──┐
phase 1.5 (rayon pool) ────────────────┤
phase 3   (Stitch enum + Discard runtime) ─┤
phase 5   (auto-light + IR walk) ──────┤
phase 7   (fused for-par + Queue runtime) ─┴──→ phase 10 (no materialisation)
                                                              │
                                                              ▼
                                                   phase 11 (par_to_vec opt-in,
                                                             reuses phase 2 rebase)
```

Phase 10 inherits the prerequisites of phases 7 + 5 + 3.  Phase 2's
remaining work (2b/2c/2d/2e wiring) folds into phase 11 — its rebase
machinery is the right tool but the wrong default.

## Cross-references

- [DESIGN.md § D1 Stitch policy](DESIGN.md#d1--stitch-policy)
- [02-stitch-not-copy.md](02-stitch-not-copy.md) — phase 2 (rebase),
  scope of remaining work shifts to phase 11.
- [03-one-native-fn.md](03-one-native-fn.md) — phase 3, Concat
  retires; Discard/Reduce/Queue stay.
- [07-fused-for-par.md](07-fused-for-par.md) — phase 7, promoted to
  the canonical par surface.
- [src/parallel.rs](../../../../../src/parallel.rs) — Stitch enum,
  StoreRebase, rebase_walk_record, adopt_worker_excess.
