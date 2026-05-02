# Phase 06 — Threading queue runtime fns

**Status:** DONE (2026-05-02)

**Closes:** **P202** (native missing `n_parallel_queue` family).

**Depends on:**
- Phase 00 (scaffold) — `emit_op` dispatch
- Phase 03 (parallel-emitter family) — without it, the queue
  emitters each duplicate the 95-line `dispatch.rs:837-930`
  special case
- **Phase 09 (parallel runtime consolidation)** — the
  `ParShape` trait + `n_parallel_for_native_core` collapse the 3
  near-duplicate `n_parallel_for_*_native` fns into one generic
  core; phase 06 replicates that pattern for the queue family
  (see step 6.4 below).

## Diagnosis

P202 is the simplest of the open issues — no architectural blocker,
just missing code.  The `n_parallel_for_native` and
`n_parallel_for_ref_native` fns exist in `src/codegen_runtime.rs`;
their queue-shaped siblings (`n_parallel_queue`,
`n_parallel_dequeue`, `n_parallel_finish` — exact names per the
interpreter's parallel queue API) don't.  Native call sites
referencing them fail at link/compile time.

The work is mechanical translation from interpreter → native, plus
emitter registration in phase 03's `parallel.rs` framework.

## Prior attempts

None.

## Why this works now

- Interpreter implementation is the reference; semantics are
  established.
- Phase 03 has factored the parallel emitter so each new helper
  registers as ~15 lines, not 95.
- Phase 01 has consolidated the cell-ABI tag, so the new fns slot
  in with `Abi::Cell`.
- **Phase 09** has factored the parallel runtime fns over a
  `ParShape` trait (`WorkerOut` + `Batches` associated types,
  `return_sz` / `run_workers` / `store_results` methods).  The
  three `n_parallel_for_*_native` public fns are now ~3-line
  wrappers around `n_parallel_for_native_core<S: ParShape, F>(...)`.
  Phase 06's queue variants follow the same pattern: a
  `ParQueueShape` trait (or a generalised `ParShape` if the trait
  shapes coincide cleanly), a `n_parallel_queue_native_core<S, F>`
  generic body, and 3 thin public-fn wrappers per closure shape
  (scalar / text / ref).  Net add: ~3 wrappers × 3 lines + 1
  generic core, instead of 3 full ~80-line bodies.

## Detailed steps with validation

> **Pre-flight (per [forwarding-first recipe](00-scaffold.md#verifying-a-new-op-emitter-the-forwarding-first-recipe))**:
> the `n_parallel_queue*` runtime fns are new — no existing
> dispatch.rs special case to conflict with.  Register forwarding
> emitters first as the runtime smoke test, then replace with real
> bodies.

### Step 6.1 — Identify the queue API surface

**Action**: locate the interpreter's parallel-queue opcodes.
Check:
- `src/fill.rs` — opcodes named `OpParallelQueue*` or similar
- `default/01_code.loft` — the loft-side `for ... par(...)`
  desugar; lists the helper fns it calls
- `THREADING.md` — design doc

Document in "Diagnosis findings":
- Each fn's name, argument types, return type
- Each fn's role in the queue-flow protocol
  (queue → dequeue → finish)
- Whether the fn is `cell` or `legacy_stores` ABI in the
  interpreter context (translated to cell for native)

**Validation**: write down the API.  Review against THREADING.md.

### Step 6.2 — Confirm worker-thread isolation rules

**Action**: study `n_parallel_for_native`'s implementation in
`src/codegen_runtime.rs` for the existing isolation pattern.
Specifically:
- Which `Stores` does each worker see?
- Where does the `UnsafeCell` get cast back to `&mut Stores`?
- What synchronisation guards shared writes?

Document the contract in "Diagnosis findings".  Getting this wrong
silently corrupts data — there's no compiler check.

**Validation**: review against THREADING.md.  Confirm match with
existing `n_parallel_for_native`.

### Step 6.3 — Write the failing regression test

**Action**: create a `for ... par(...)` workload that uses the
queue API and currently fails (compile or runtime) under native:

```loft
// tests/scripts/p202_queue.loft
fn worker(item: integer) -> integer {
    item * 2
}

fn main() {
    let inputs = [1, 2, 3, 4, 5, 6, 7, 8]
    let outputs = vector<integer>()
    for x in inputs par(threads = 4, queue = true) {
        outputs.append(worker(x))
    }
    // outputs may be in any order; sum is deterministic.
    let total: integer = 0
    for v in outputs { total += v }
    assert(total == 72, "queue parallel sum")
}
```

```bash
# Confirm it fails today under native:
cargo run --bin loft --release -- tests/scripts/p202_queue.loft
# Expected: nonzero exit (compile error or runtime crash).
echo "Exit: $?"
```

Add to `tests/codegen_emitter.rs`:
```rust
#[test]
fn p202_queue_parallel_runs_under_native() {
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--",
               "tests/scripts/p202_queue.loft"])
        .status().unwrap();
    assert!(status.success(), "P202: queue-parallel still failing");
}
```

**Validation**: this test currently fails — that's the regression
guard before the fix ships.

### Step 6.4 — Add `n_parallel_queue*` runtime fns (ParShape-based)

**Action**: in `src/codegen_runtime.rs`, add the queue fn family
following phase 09's consolidation pattern.

1. **Decide trait reuse vs new trait**.  Compare the queue API's
   shape (alloc / worker / merge / finalise) to phase 09's
   `ParShape`.  Two outcomes:
   - **Reuse `ParShape` directly** — if the queue family's per-
     shape variation lines up with the existing trait's
     `WorkerOut` / `Batches` / `return_sz` / `run_workers` /
     `store_results` decomposition.  Add a queue-specific
     `run_workers` impl per shape (scalar / text / ref) that wraps
     the queue worker dispatcher.  This is the cleanest outcome.
   - **Introduce `ParQueueShape`** — if the queue protocol exposes
     additional shape state (e.g. queue capacity, finish-callback)
     that doesn't fit `ParShape`.  Mirror phase 09's design:
     `ParQueueShape` with associated types + method set; three
     impls (`QueueScalarShape`, `QueueTextShape`, `QueueRefShape`).
   Document the choice in "Implementation notes."

2. **Generic core**.  Add
   `fn n_parallel_queue_native_core<S, F>(cell, ..., shape: &S, worker: F) -> DbRef`
   following the alloc → run_workers → store_results → finalise
   skeleton of `n_parallel_for_native_core`.

3. **Three thin public fns**: `n_parallel_queue_native`,
   `n_parallel_queue_text_native`, `n_parallel_queue_ref_native`
   each ~3 lines calling the generic core with the appropriate
   shape impl.

4. Register all three in `CODEGEN_RUNTIME_FNS` with `Abi::Cell`
   tags (phase 01 ABI).

**Validation**:
```bash
cargo build --release
# The new fns compile; no callers yet.
```

**Acceptance criterion**: each new public fn body is ≤ 15 lines and
must call the queue generic core.  Extend the
`parallel_runtime_consolidated` structural test in
`tests/codegen_emitter.rs` to cover the new wrappers (or add a
sibling `parallel_queue_runtime_consolidated` test that asserts
the same shape).  This pins the consolidation invariant — without
it, queue variants can drift back to inlined bodies.

### Step 6.5 — Register the emitter(s) in `parallel.rs`

**Action**: in `src/generation/ops/parallel.rs` (created in
phase 03), add `ParallelQueueEmitter` etc. that reuses the existing
`pick_helper` / `closure_shape` / `emit_extra_bindings` /
`emit_closure` helpers.  Register for the `n_parallel_queue` and
`n_parallel_finish` Op names.

**Validation**:
```bash
# Regression test now passes:
cargo test --release --test codegen_emitter::p202_queue_parallel_runs_under_native

# Existing parallel-for tests still work:
cargo test --release --test wrap par
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test issues 2>&1 | tail -3
```

### Step 6.6 — Per-shape regression tests

**Action**: extend `p202_queue.loft` (or add siblings) to exercise
each closure shape: scalar return, float return, text return,
ref return.

```bash
# tests/scripts/p202_queue_text.loft  — text-returning worker
# tests/scripts/p202_queue_ref.loft   — ref-returning worker
# tests/scripts/p202_queue_float.loft — float-returning worker
```

Each shape exercises a different branch of the parallel-emitter
family.  All must pass under native.

**Validation**:
```bash
for shape in scalar text ref float; do
    cargo run --bin loft --release -- tests/scripts/p202_queue_${shape}.loft
    test $? -eq 0 || echo "FAIL at shape $shape"
done
```

### Step 6.7 — Update PROBLEMS.md

**Action**: mark P202 CLOSED with "fix path: phase 06 of plan 09".
List the regression tests added.

**Validation**: review.

## Acceptance for phase 06 overall

```bash
cargo test --release --test codegen_emitter::p202_queue_parallel_runs_under_native
cargo test --release --test wrap par
cargo test --release --test wrap threading
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

## Gate updates per step

| Step | Gate update |
|---|---|
| 6.4 | New `n_parallel_queue*` runtime fns added to `CODEGEN_RUNTIME_FNS`.  Gate's runtime fn count grows; ABI tags should be `Cell` (not `LegacyStores`). |
| 6.5 | `ParallelQueueEmitter` family registered.  `custom_count` increments by ~3-4 (one per queue variant). |
| 6.6 | New regression tests for queue / per-shape behaviour. |

## Commit shape

5-6 commits across the steps; ships as one PR.

## Diagnosis findings

_(populate during pre-work; document the queue API and the
isolation contract)_

## Problems encountered

_(append per problem — store isolation across threads, lifetime of
`UnsafeCell<Stores>` shared across worker threads — see THREADING.md
for the existing model)_

## Implementation notes

_(append per non-obvious decision)_
