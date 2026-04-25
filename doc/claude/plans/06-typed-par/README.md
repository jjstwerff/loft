<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 06 — Simple typed `par`: everything is a store

## Goal

Replace today's branching `par` runtime with one uniform path: every
parallel worker takes input as a Store, writes output into its own
output Store, and main-thread stitching concatenates the per-worker
output stores into a single result Store.  No special cases for
text, references, or primitives — they're all the same shape because
they all live in stores.

## Why

Today's implementation branches on type and return shape:

- **3 native functions** (`n_parallel_for_native`, `n_parallel_for_text_native`, `n_parallel_for_ref_native`) with bespoke marshalling per return kind.
- **6+ runtime variants** in `src/parallel.rs` (`run_parallel_direct` × `_raw` × `_text` × `_ref` × `_int` × `_light`).
- **3 dispatch branches** in `src/generation/dispatch.rs:755` — Text vs. Reference vs. primitive.
- **2 entry points** at the loft surface (`par(...)` vs. `par_light(...)`) — the user picks based on whether the worker allocates.
- **4 getter primitives** (`parallel_get_int` / `_long` / `_float` / `_bool`) needed because the output isn't already in a Store.

The cost: every type-shape change touches three layers (loft signature,
codegen, runtime).  Adding a new return type means a new native fn,
new dispatch arm, new getter.  Adding new optimisations (worker
slot reuse, output-store mmap, heterogeneous types) requires
rebuilding the marshalling for each branch.

The store-typed redesign collapses all of this to:

| Today | After plan-06 |
|---|---|
| 3 native fns × 4 getters | 1 native fn, 0 getters |
| 6 runtime variants | 1 runtime path (parameterised by element type) |
| 3 codegen dispatch arms | 1 |
| `par` + `par_light` user split | 1 `par`; light vs. full chosen at compile time from the worker's effect signature |
| Text via owned `String` channel | Text in the worker's output Store, same as any other field |
| Reference via `copy_block` + `copy_claims` channel | Reference in the worker's output Store; main thread merges by store-pointer rebase |

The size of the saving is real: ~1500 lines of bespoke marshalling
code retire across `src/parallel.rs` (currently 683 lines) and
`src/codegen_runtime.rs:1581-1805` (224 lines), plus the parser
auto-light heuristic at `src/parser/builtins.rs:362`.

## Architectural anchor — "everything is a store"

The whole interpreter is already store-organised: every allocation,
every variable, every parameter lives in one of `stores.0`,
`stores.1`, …, `stores.N`.  The reason `par` has 3 native functions
is that the **output of a parallel call is the only place in the
interpreter that doesn't follow this rule** — primitive results are
written through raw byte pointers, text results are owned `String`
buffers passed via channel, reference results are deep-copied via
ad-hoc `copy_block` calls.

If the worker writes its output into a pre-allocated per-worker
output Store — exactly the way every other loft fn already writes
its return value — the marshalling collapses to "stitch N per-worker
output stores into one result store" regardless of element type.

## Phases

Each phase preserves every currently-green test.  Each phase is a
single PR with its own `make ci` run.

| Phase | File | Status | Effort | Summary |
|---|---|---|---|---|
| 0 | [00-baseline-and-bench.md](00-baseline-and-bench.md) | open | XS | Pin current behaviour with characterisation tests; record perf baseline so later phases prove no regression. |
| 1 | [01-output-store.md](01-output-store.md) | open | M | Workers write to per-worker output Stores instead of `out_ptr` / channel.  Three native fns still exist; phase 1 only changes where results land. |
| 2 | [02-stitch-not-copy.md](02-stitch-not-copy.md) | open | M | Main-thread stitch via store-pointer rebase, retiring `copy_block` + `copy_claims`.  Closes P1-R3 + P1-R5 from THREADING.md. |
| 3 | [03-one-native-fn.md](03-one-native-fn.md) | open | S | Collapse `n_parallel_for_native` / `_text_native` / `_ref_native` into one polymorphic `n_parallel_native(stitch)`.  Drop the four `parallel_get_*` getters. |
| 4 | [04-typed-input-output.md](04-typed-input-output.md) | open | M | Typed surface: `parallel_for(input: vector<T>, fn: fn(T) -> U, threads: integer) -> vector<U>` — `element_size` and `return_size` retire; the type system carries them. |
| 5 | [05-auto-light.md](05-auto-light.md) | open | M | Scope-analysis pass that proves a worker writes nothing outside its own output store; codegen picks the light path automatically.  Defines the heuristic that DESIGN.md D8 references. |
| 6 | [06-cleanup-and-doc.md](06-cleanup-and-doc.md) | open | XS | Delete the now-unreachable runtime variants (~520 lines from `src/parallel.rs`, ~336 from `codegen_runtime.rs`, ~70 from `default/01_code.loft`); rewrite THREADING.md's par sections; CHANGELOG entry. |
| 7 | [07-fused-for-par.md](07-fused-for-par.md) | open | M | Fused `for x in ls par(r = foo(x), 4) { … }` construction + parser-side desugaring of the value-position `par(input, fn, threads)` call form to the same `Value::ParFor` IR node.  One primitive, one runtime path; the call form gets `vector_with_capacity` pre-alloc for free.  Replaces the earlier 3-variant idea (`par_for_each` / `par_fold` / `par_iter`).  `par_light` is removed from the user surface entirely — the auto-light heuristic from phase 5 makes it a compiler-internal decision, never exposed to users. |

## Ground rules

Inherits the global plans rule from
[doc/claude/plans/README.md](../README.md):

> A plan's job is to split work into manageable chunks that each land
> cleanly without introducing new problems.  Every phase must
> preserve every currently-green test, every currently-correct
> user-facing behaviour, and either ship a new invariant or be a
> no-op refactor — never a degrade-now-fix-later bargain.

Specific to this plan:

1. **No perf regression past phase 0's baseline.**  The
   characterisation benchmark in `tests/bench/par_baseline.rs`
   (created in phase 0) must run within ±5 % at every phase boundary.
   The store-stitching path *should* match or beat the byte-pointer
   path; if it doesn't, the phase pauses for investigation.
2. **WASM-single-threaded path must keep working.**  WASM has no
   threads under default features.  Workers run sequentially in a
   for-loop; the store-typed model must not assume real parallelism.
3. **Reference aliasing rules tighten, not loosen.**  Today a worker
   can hold a `DbRef` into a parent-side store (locked + debug-checked).
   Phase 2's stitch path must keep that same enforcement; if anything,
   make it compile-time.
4. **`par_light` users see no behaviour change.**  Phase 5 makes the
   compiler auto-select the light path; an explicit `par_light(...)`
   call continues to work as a no-op alias for one release, then
   gets a deprecation warning, then is removed in 1.0.0.
5. **Each phase has at least one new test.**  Phase 0's baseline
   tests cover the *current* behaviour; phases 1–6 each add an
   invariant test for the new shape.

## Risks (all addressable, none plan-blocking)

| Risk | Mitigation |
|---|---|
| Output Store allocation per call dominates a tight loop | Pool output stores keyed by `(element_type, max_elements)`; reuse across calls.  Measured in phase 0; deferred to phase 1 if real. |
| Reference results across worker stores need pointer rebasing | Phase 2 introduces a `StoreRebase` map (`worker_store_id → result_store_offset`) at stitch time.  Same idea as `copy_block` but at store granularity, not record. |
| WASM async/yield interaction | Phase 1 defers WASM until the for-loop fallback is verified.  Future W1.14 (Web Worker pool) is unaffected — same store-typed shape, different scheduler. |
| Type-checker cannot prove "worker only writes its output store" | Phase 5's auto-light heuristic falls back to the full path when proof fails.  Conservative; never produces unsafe results. |

## Out of scope

These are not addressed by plan-06 even though they're tempting:

- **Heterogeneous worker results** (each worker returns a different
  type).  Today `par` workers all return the same type; that stays.
  Heterogeneous results are a different feature, not a simplification.
- **Cross-worker reference graphs** (worker A's result references
  worker B's output store).  Workers stay independent; results are
  flat.
- **Worker-pool reuse for non-`par` constructs** (e.g. `parallel { }`
  blocks).  Plan-06 covers `par(...)` only; A15 structured concurrency
  inherits the simplifications by virtue of routing through the
  same runtime, but its own surface stays.

## Cross-references

- [DESIGN.md](DESIGN.md) — cross-cutting decisions referenced from
  every phase: Stitch policy enum (D1), worker / parent store
  relationship (D2), fn return-type accessor (D3), failure model
  (D4), degenerate-input handling (D5), WASM fallback (D6),
  `Value::ParFor` IR shape (D7), auto-light heuristic (D8),
  source-span propagation (D9), call-site migration (D10).
- [THREADING.md](../../THREADING.md) — current par design, especially
  §§ "Data flow" and "Isolation guarantees".
- [ROADMAP.md § 1.1+ A14 / A15 / W1.14](../../ROADMAP.md#11-backlog) —
  related parallel work; this plan is independent but compatible.
- [src/parallel.rs](../../../../src/parallel.rs) — current 683-line
  runtime; ~520 lines retired in phase 6 (~800 lines net across
  plan-06 phases).
- [src/codegen_runtime.rs:1581-1805](../../../../src/codegen_runtime.rs) —
  current 3-fn native dispatch; collapses in phase 3.
- [doc/claude/plans/README.md](../README.md) — global plan ground rules.
