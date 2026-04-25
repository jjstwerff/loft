<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 1 — Workers write to per-worker output Stores

**Status: open**

## Goal

Replace the three different "where does worker output go?" paths
(`out_ptr` raw bytes, `Vec<String>` channel, `Vec<DbRef>` channel)
with one uniform path: every worker writes its output into a
pre-allocated per-worker output Store, exactly the way every loft
fn already writes its return value.

Phase 1 is **transitional**: the three native fns
(`n_parallel_for_native` / `_text_native` / `_ref_native`) still
exist, dispatch is unchanged, the user-visible surface is unchanged.
What changes is **where the worker writes the result**.  Stitching
into a single result vector still goes through the existing copy
logic — phase 2 retires that.

## Why transitional

Trying to unify everything (output store + stitch + native dispatch)
in one phase would mean ~1500 lines of churn in one PR.  Phase 1
isolates the cheapest change (the worker side) so we can validate
that "everything writes to a store" works before touching the
collection side.  Phase 0's bench harness gates the perf regression.

## What changes per return-type path

| Today's path | Worker writes to | After phase 1 worker writes to |
|---|---|---|
| Direct (primitive) | `out_ptr` raw byte slice owned by main thread | Per-worker output Store; main thread copies from store into result vector |
| Text | `Vec<String>` sent via mpsc channel | Per-worker output Store with text fields; main thread reads strings from store |
| Reference (struct) | `Vec<DbRef>` sent via mpsc channel + `copy_block` + `copy_claims` | Per-worker output Store containing the worker's struct results; main thread `copy_block`s from worker store into result store (one less indirection — no channel) |

The three paths still **dispatch to three different native fns**
because the result-vector type differs (vector<i32> vs. vector<text>
vs. vector<Struct>).  Phase 3 collapses the dispatch.  Phase 1 only
makes the workers' write target uniform.

## Changes per cross-cutting concern

| Concern (from DESIGN.md) | Phase 1 contribution |
|---|---|
| D1 Stitch policy | Not introduced yet; phase 1 keeps today's `Concat` behaviour implicitly |
| D2 Worker / parent relationship | Each worker's output store is owned exclusively by the worker thread.  Parent stores still locked-cloned (today's mechanism) until phase 2 lifts the relationship. |
| D5 Empty / degenerate inputs | New code paths must handle empty input (allocate 0-element output store, skip workers) and single-worker (allocate one output store, no parallelism) |
| D6 WASM single-threaded | The output-store allocation happens in the calling thread; workers run sequentially.  No queue, no stitch. |

## Per-commit landing plan

### 1a — primitive output stores

Touch points:
- `src/parallel.rs::run_parallel_direct` (currently writes via `out_ptr`).
- `src/codegen_runtime.rs:1581-1700` (`n_parallel_for_native` and the four `parallel_get_*` getters).
- `src/database/allocation.rs:449` (`clone_for_worker`).

Mechanic:
1. Before spawning workers, allocate `threads` output stores via
   `Stores::alloc_worker_output(elem_type, slots_per_worker)`.
   Slot count = `(input_count + threads - 1) / threads`.
2. Each worker receives ownership of its output store.
3. Worker's loop body changes from `out_ptr.add(t * slots).write(r)` to
   `worker_output_store.set_long(slot_idx, r)` (or `.set_i32_raw`,
   etc., based on element type).
4. After join, main-thread loop walks the per-worker output stores
   in order, copying their values into the result vector via
   existing `vector_add`-style ops.

Post-commit: `make ci` green; phase 0 characterisation suite passes;
bench-1 (1 M `i64`, 4 threads) within ±5 % of phase 0 baseline (the
expectation is **slightly slower** than today's raw `out_ptr` write
because we add one indirection — the slowdown is acceptable as the
cost of giving up the unsafe pointer; phase 2 + 3 will reclaim the
cost via a faster stitch).

### 1b — text output stores

Touch points:
- `src/parallel.rs::run_parallel_text` (currently uses
  `mpsc::channel<Vec<String>>`).
- `src/codegen_runtime.rs::n_parallel_for_text_native`.

Mechanic:
1. Per-worker output store allocated as a `vector<text>` store.
2. Worker writes its `String` result via `set_str(slot_idx, &result)`
   — same path text always uses inside loft.
3. After join, main-thread copies the text-pointer entries from each
   worker store into the result vector.  No channel.

Specific issue resolved: today's `Vec<String>` channel allocates a
`String` per result + an `mpsc` slot + a final main-thread copy.
Phase 1b reduces this to one store write per result + main-thread
copy; the channel is gone.

Bench-3 (100 K text results) expected: ±5 % of phase 0 baseline,
likely slightly faster from removing the channel.

### 1c — reference output stores

Touch points:
- `src/parallel.rs::run_parallel_ref` (currently uses
  `mpsc::channel<DbRef>` + main-thread `copy_block` + `copy_claims`).
- `src/codegen_runtime.rs::n_parallel_for_ref_native`.

Mechanic:
1. Per-worker output store allocated as a `vector<Reference<T>>`
   store.
2. Worker constructs its struct result in its own store, then
   `vector_add(worker_output, struct_dbref)` — same path any loft
   fn returning a struct uses.
3. After join, main-thread `copy_block`s each worker's vector
   contents into the result vector.  Channel removed.

The `copy_claims` machinery still runs in phase 1c — phase 2 retires
it via the rebase pass.

Bench-2 (100 K struct results) expected: similar to phase 0
baseline; the channel removal saves a small amount, the extra
intermediate store costs a similar amount.

## Loft-side prerequisites

One new accessor in `Stores`:

```rust
// src/database/allocation.rs
impl Stores {
    /// Allocate a per-worker output store sized for `slot_count`
    /// elements of `elem_type`.  The worker takes ownership; the
    /// store is freed when the worker join completes.
    pub fn alloc_worker_output(
        &mut self,
        elem_type: u16,
        slot_count: u32,
    ) -> WorkerOutputStore { /* ... */ }
}
```

`WorkerOutputStore` is a thin wrapper around `Store` with `Drop`
that releases the store back to the parent when the worker finishes.
Same lifetime contract as today's clone but unidirectional (worker
writes → parent reads, never the reverse).

## Test fixtures

All existing phase-0 characterisation tests **must keep passing
byte-for-byte**.  Phase 1 changes the runtime path but not the
output.

New fixtures:

| Fixture | Asserts |
|---|---|
| `tests/issues.rs::par_phase1_output_store_lifetime` | `WorkerOutputStore` allocated and released in matched pairs; no leak under `LOFT_STORES=warn` |
| `tests/issues.rs::par_phase1_text_no_channel` | `mpsc` channel allocation is zero (verify by patching `tests/parallel_intrumentation.rs` to count channel allocs; the count drops to zero after 1b) |
| `tests/issues.rs::par_phase1_empty_input_no_worker_alloc` | `len(input) == 0` returns immediately; no `WorkerOutputStore` allocated |
| `tests/issues.rs::par_phase1_panic_propagation` | Worker panics on element 5; parent receives panic; no orphan worker stores remain |

## Acceptance criteria

- All phase-0 characterisation tests pass byte-for-byte.
- New fixtures pass on Linux / macOS / Windows.
- Bench-1 (primitive) within ±5 % of phase 0 baseline.
- Bench-2 (struct) within ±5 % of phase 0 baseline.
- Bench-3 (text) within ±5 % of phase 0 baseline (small improvement
  expected from channel removal but not required).
- `make ci` green at every sub-commit (1a / 1b / 1c).
- `LOFT_STORES=warn` reports zero leaked stores across the full
  suite.

## Risks

| Risk | Mitigation |
|---|---|
| Per-worker output store allocation dominates bench-1's tight loop | Pool output stores by `(elem_type, slot_count)` keyed cache; reuse across calls.  Evaluate at end of 1a; defer if unneeded. |
| Three native fns now allocate output stores three different ways | Phase 1 keeps the dispatch; phase 3 collapses it.  Triple paths is acceptable as transitional state. |
| Struct results in 1c need worker-store-to-result-store copy | `copy_block` already handles this; phase 2 retires `copy_block` in favour of the rebase pass. |
| WASM target's sequential fallback also needs the output-store allocation | Allocate output store in the calling thread, write directly into the result vector (no separate worker store).  Identical-output sequential path. |

## Out of scope

- Any change to the dispatch (3 native fns stay).
- Any change to the user surface.
- The `claims` HashSet overhead from D2 — retired in phase 2.
- Stitch policy enum — phase 3.
- Auto-light heuristic — phase 5.

## Hand-off to phase 2

After phase 1 lands, every worker writes to an output store but
the main thread still uses today's `copy_block` + `copy_claims`
collection.  Phase 2 introduces the store-rebase pass that retires
those, removing P1-R3 (`claims` HashSet overhead) and P1-R5 (no
Rust-level proof of non-aliasing).
