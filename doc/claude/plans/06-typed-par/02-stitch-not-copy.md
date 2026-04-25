<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 2 — Main-thread stitch via store rebase

**Status: open**

## Goal

Replace `copy_block` + `copy_claims` deep-copy collection with a
**store-rebase pass**: workers' output stores remain allocated
after join; the main thread builds a single result vector by
translating cross-store DbRef pointers through a rebase map.

This retires:
- The per-result `copy_block` call inside the parent collection loop
  (today's `src/parallel.rs::run_parallel_ref`).
- The `copy_claims` call that recreates worker-side claim
  bookkeeping in the parent (P1-R3 in THREADING.md).
- The implicit "worker stores must be empty after collection"
  invariant (P1-R5 — no Rust-type-level proof of non-aliasing).

## Why a rebase, not a copy

`copy_block` walks every byte of every worker result and writes it
into the parent store.  For a struct result of size 256 B with
100 K elements, that's 25.6 MB of memcpy per parallel call.  At
4 threads writing in parallel, the wall-clock cost matters.

A rebase is structurally different.  Instead of moving bytes, it
**re-indexes the DbRefs in the result vector** so they continue
pointing at valid memory after the worker's output store gets
adopted by the parent's store table.

```
Before stitch:                After stitch:
─ Parent stores ────         ─ Parent stores ────
  store 0 (input)               store 0 (input)
  store 1 (result vec)          store 1 (result vec)
                                store 2 (worker A's output, adopted)
─ Worker A stores ──            store 3 (worker B's output, adopted)
  store 0 (output)
                              ─ Result vector layout ─
─ Worker B stores ──            elem 0 → DbRef(2, rec, pos)
  store 0 (output)              elem 1 → DbRef(2, rec, pos)
                                elem 2 → DbRef(3, rec, pos)
                                elem 3 → DbRef(3, rec, pos)
```

The result vector's elements are DbRefs into the now-parent-owned
worker output stores.  No memcpy.  Memory cost is the same as
today (the bytes still live somewhere); wall-clock saves the copy
pass.

## What "rebase" actually means

Each worker output store, while owned by the worker, has a
worker-local `store_nr` (e.g. `0` in the worker's `Stores`).  When
the main thread adopts it, it gets a parent-side `store_nr` (e.g.
`5`).  Any DbRef inside that store referring to itself or to a
sibling worker store needs translation.

```rust
// src/parallel.rs (new)
struct StoreRebase {
    /// Maps (worker_id, worker_local_store_nr) → parent_store_nr.
    map: HashMap<(u32, u32), u32>,
}

impl StoreRebase {
    fn translate(&self, worker_id: u32, db_ref: DbRef) -> DbRef {
        let parent_store = self.map[&(worker_id, db_ref.store_nr)];
        DbRef { store_nr: parent_store, rec: db_ref.rec, pos: db_ref.pos }
    }
}
```

Build the rebase map at adoption time (one entry per worker output
store).  Walk the per-worker output stores once, translating any
`DbRef` field through the map.  For 4 workers each with a 100 K
result vector, that's 400 K DbRef translations — cheap (8-byte
field reads, hash lookup, 8-byte write).

**Cost comparison:**
| Path | Wall-clock for 100 K struct(256 B) results |
|---|---|
| Today's `copy_block` | 25.6 MB memcpy (~10 ms on typical SSD-bound CI) |
| Phase 2 rebase | 400 K × 50 ns = 20 ms (CPU-bound but fully cached) |

The rebase is **not faster** in pure cycles for primitive results.
The win is on large structs (32 B → 1 KB results) where the rebase
stays at 50 ns/element while `copy_block` scales linearly with
struct size.  Crossover is around 32-byte results.

For struct workloads larger than 32 B the rebase wins; for tiny
results (8-byte primitives), phase 2 has no measurable win — but
it pays off in **lifetime simplification** (P1-R3, P1-R5 closed).

## How to handle multi-store worker outputs

A worker that allocates nested vectors or sub-structs creates DbRefs
across multiple worker-internal stores.  Example: worker returns
`vector<Point>` where `Point` is `{ name: text, coords: vector<float> }`;
the worker's output store contains `Point` records, each pointing
at a separate sub-store for `coords` and another for the text.

The rebase map handles this naturally — it's keyed by
`(worker_id, worker_local_store_nr)`, so all of the worker's
internal stores get adopted and translated together.  The walk
recurses through DbRef fields; any field pointing at a store_nr in
the rebase map gets translated, others stay (those are pointers to
parent-shared / input stores, which the worker only read).

**Cycle handling.**  Worker results may contain DbRef cycles
(`a.next = b; b.next = a`).  The rebase walk uses a `visited`
HashSet keyed by `(store_nr, rec, pos)` to break the cycle.

## Per-commit landing plan

### 2a — `StoreRebase` infrastructure

- Add `StoreRebase` struct + `translate` impl.
- Add `Stores::adopt_worker_output(worker_output_store) -> u32`
  that takes the worker's output store, gives it a parent-side
  `store_nr`, returns the new store_nr.
- Unit tests in `tests/parallel_rebase.rs`: identity rebase
  (no cross-store refs), single-cross rebase, multi-store rebase,
  cyclic-ref rebase.

No runtime change yet; phase 1's `copy_block` collection still runs.

### 2b — switch reference path to rebase

- `src/parallel.rs::run_parallel_ref` replaces `copy_block` +
  `copy_claims` with `adopt_worker_output` + rebase walk.
- The result vector's elements become DbRefs into the adopted
  worker stores instead of fresh DbRefs in the parent's result
  store.
- Bench-2 (struct results) measured: expected ~30 % faster on
  256-byte structs; ±0 % on 8-byte structs.

### 2c — switch text path to rebase

- Text in worker stores already uses the `Str` type which
  references a per-store string area.  Rebase needs to translate
  the `Str` pointer's owning store too.
- `src/parallel.rs::run_parallel_text` adopts worker stores and
  the rebase walk translates `Str.store_nr` fields.
- Bench-3 (text results) measured: expected slight improvement
  from removing the per-string copy.

### 2d — switch primitive path

- For primitive results (i32, i64, f32, f64, bool, byte): no DbRefs
  inside the worker output store, so the rebase walk is a no-op.
- The simpler change: skip `copy_block` and use the worker output
  stores' contents directly as the result vector backing.
- Bench-1 (primitive results) measured: expected ±5 % (the
  store-adoption is cheap; the savings from skipping `copy_block`
  for primitives is small because primitives are byte-copied
  anyway).

### 2e — retire `copy_claims` infrastructure

After 2b–2d land, `copy_claims` has no callers in the par path.
Delete the helper from `src/database/structures.rs` (it remains in
the codebase for non-par uses, if any).  Verify by grep.  Update
THREADING.md's P1-R3 entry to "closed in plan-06 phase 2e".

## Correctness — no double-free

Today's `copy_block` followed by worker store deallocation gives
clean ownership: bytes are in the parent; worker stores are dead.

Phase 2 changes the model: worker stores are **adopted** by the
parent.  They become regular parent stores, freed via the parent's
existing store-deallocation when the result vector goes out of
scope.

**Risk:** if any worker output store is freed twice (once by the
worker's `WorkerOutputStore::drop`, once by the parent's adoption
+ later free), the runtime double-frees and corrupts.

**Mitigation:** `WorkerOutputStore::drop` checks an `adopted: bool`
flag set by `adopt_worker_output`.  If adopted, drop is a no-op
(the parent owns the store now).  If not adopted (worker panicked
mid-flight, or adoption failed), drop frees normally.

## Cross-cutting interactions

| DESIGN.md item | Phase 2 contribution |
|---|---|
| D2 worker-store relationship | Closes P1-R3 (no claims HashSet) and P1-R5 (Rust ownership tracks store adoption — no need for the `locked` boolean's debug-only check) |
| D5 empty / degenerate inputs | Empty input → zero worker stores → empty rebase map → empty result vector (rebase is a no-op) |
| D6 WASM single-threaded | Sequential fallback already writes directly into the result store; no rebase needed |

## Test fixtures

All phase-0 characterisation tests must pass byte-for-byte.

New fixtures:

| Fixture | Asserts |
|---|---|
| `tests/issues.rs::par_phase2_rebase_correctness` | A worker returning a struct with a sub-vector field; rebase preserves the cross-store DbRef |
| `tests/issues.rs::par_phase2_cycle_safe` | Worker returns a struct with a self-cycle; rebase handles via `visited` HashSet |
| `tests/issues.rs::par_phase2_no_double_free` | Worker panics; the `WorkerOutputStore` drops normally; the parent never adopted; no leak, no double-free |
| `tests/leak.rs::par_phase2_leak_check` | Run all phase-0 fixtures under `LOFT_STORES=warn`; count parent-store allocations.  Compare with phase 1's count.  Adopted-stores count appears in the parent now (expected; the bytes have to live somewhere) but no extra orphan allocations |

## Acceptance criteria

- Phase-0 characterisation suite passes byte-for-byte.
- Bench-2 (structs) shows ≥ 20 % wall-clock improvement on 256-B
  structs at 4 threads vs. phase 1.  No regression on 8-B structs.
- Bench-3 (text) within ±5 % of phase 1.
- Bench-1 (primitives) within ±5 % of phase 1.
- `LOFT_STORES=warn` reports zero leaked stores across the full
  suite, including the panic fixture.
- `make ci` green at every sub-commit (2a / 2b / 2c / 2d / 2e).

## Risks

| Risk | Mitigation |
|---|---|
| Cross-store DbRef rebase walks miss a field type | Phase 0's struct/sub-struct/text fixtures cover the common cases; add a generic walk based on `Type::field_offsets` so any future struct shape is covered automatically |
| Cycle detection's `visited` HashSet is slow for very-deep nested results | Cycle is rare in worker results (most workers return acyclic data); accept the cost.  If a real workload regresses, switch to a per-store visited bit (1 bit/record) |
| Adopted stores don't fit in the parent's store table | The parent's store table is dynamically grown; phase 2 doesn't change that.  100-thread workloads adopt 100 stores — well within today's limits |
| Existing leak tests fail because adopted stores show up as parent allocations | Update the leak test's accounting to subtract the count of intentionally-adopted stores; this is a test-harness accounting fix, not a runtime change |

## Out of scope

- Stitch-policy enum (phase 3).
- Native-fn collapse (phase 3).
- Typed surface (phase 4).
- Auto-light analyser (phase 5).

## Hand-off to phase 3

After phase 2 lands, the runtime has:
- workers writing to per-worker output stores (phase 1),
- main thread adopting those stores + rebasing DbRefs (phase 2).

Three native fns still exist with bespoke entry points.  Phase 3
collapses them into one polymorphic dispatcher parameterised by
the `Stitch` policy enum (DESIGN.md D1).
