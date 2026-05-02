# Phase 09 — Parallel runtime consolidation

**Status:** DONE (2026-05-02)

**Kind:** Simplification (no P-issue closes; **must land before
phase 06** to prevent queue variants multiplying the duplication)

**Depends on:** Phase 03 (parallel-for emitter family — supplies
the call-site dispatch that consumes whatever runtime shape this
phase settles on).

## What's tangled today

`src/codegen_runtime.rs` has three near-duplicate parallel-for
runtime fns:

| Line | Fn | Role |
|---|---|---|
| 1645 | `n_parallel_for_native` | Scalar return (i64) |
| 1727 | `n_parallel_for_text_native` | Text return (String → store via `set_str`) |
| 1827 | `n_parallel_for_ref_native` | Heap ref return (struct/vector — copy_record) |

Each is ~80-100 lines.  They share:

- Allocation of result store via `alloc_par_result(stores, n, return_sz)`
- Worker dispatch via `crate::parallel::parallel_workers(...)`
- Per-thread `Stores` clone via `clone_for_worker`
- Result merge via `crate::parallel::merge_batches`
- Finalisation via `finalize_par_result`

What differs:
- Closure return type (`i64` / `String` / `DbRef`)
- Result-storage idiom (`set_long` / `set_str` / `copy_record`)
- Return-size dispatch (8 / 1 / i32 for scalar; fixed for text; struct-size for ref)

Phase 06 (P202) adds queue variants: `n_parallel_queue_native`,
`n_parallel_queue_text_native`, `n_parallel_queue_ref_native`.
Without this phase, that's **six** near-duplicate runtime fns
totalling ~500 lines.

## Why the emitter pattern enables this

Phase 03's `ParallelForEmitter` (and phase 06's queue siblings)
dispatch to whatever runtime fn name we settle on.  If we
parameterise the runtime side over its shape-specific behaviour,
the emitters keep calling `pick_helper(ret)` to choose the right
fn — they don't care whether the chosen fn is a thin wrapper
around a generic core or a hand-written body.

This phase consolidates the runtime fns; the emitters from phase 03
become the bridge.

## Detailed steps with validation

### Step 9.1 — Capture baseline parallel runtime fn corpus

**Action**: snapshot current parallel-for runtime fn bodies for
diff comparison after consolidation:

```bash
mkdir -p /tmp/p09-step09-baseline
for fn in n_parallel_for_native n_parallel_for_text_native n_parallel_for_ref_native; do
    awk "/pub fn $fn/,/^}$/" src/codegen_runtime.rs > /tmp/p09-step09-baseline/$fn.rs
    wc -l /tmp/p09-step09-baseline/$fn.rs
done
```

**Validation**: each file ~80-100 lines.  Total ~250 lines pre-
consolidation.

### Step 9.2 — Capture behavioural baseline

**Action**: full parallel-suite test pass with timing:
```bash
cargo test --release --test wrap par 2>&1 | tee /tmp/p09-step09-baseline/par-test.log
cargo test --release --test threading 2>&1 | tee /tmp/p09-step09-baseline/threading-test.log
cargo test --release --test threading_chars 2>&1 | tee /tmp/p09-step09-baseline/threading_chars-test.log
```

Baseline counts (e.g., 43/43 threading, 35/35 threading_chars)
become the regression bound.

**Validation**: capture passes.  These numbers don't decrease across
this phase.

### Step 9.3 — Extract shape trait

**Action**: in `src/codegen_runtime.rs`, define a `ParShape` trait
that captures the per-shape variations:

```rust
trait ParShape {
    type WorkerOut: Send;
    /// Allocate result storage for `n` results; return (db, vec_rec, header_rec, return_sz).
    fn alloc(stores: &mut Stores, n: usize) -> (DbRef, u32, u32, u32);
    /// Store one result at offset `fld` in `vec_rec`.
    fn store(stores: &mut Stores, db: &DbRef, vec_rec: u32, fld: u32, val: Self::WorkerOut);
}

struct ScalarShape { return_size: i32 }
impl ParShape for ScalarShape {
    type WorkerOut = i64;
    fn alloc(...) -> ... { /* return_size.clamp(1,8) → return_sz */ }
    fn store(stores, db, vec_rec, fld, val) {
        match self.return_size { 8 => set_long, 1 => set_byte, _ => set_i32_raw }
    }
}

struct TextShape;
impl ParShape for TextShape {
    type WorkerOut = String;
    fn alloc(...) -> ... { /* fixed return_sz for text */ }
    fn store(...) { /* set_str */ }
}

struct RefShape { struct_size: i32, known_type: i32 }
impl ParShape for RefShape {
    type WorkerOut = DbRef;
    fn alloc(...) -> ... { /* uses self.struct_size */ }
    fn store(...) { /* copy_record */ }
}
```

**Validation**:
```bash
cargo build --release  # compiles; no callers yet
```

### Step 9.4 — Implement generic core

**Action**: `n_parallel_for_native_core<S: ParShape, F>(...)` —
the consolidated body.  Takes the shape, the worker closure, and
the standard parallel-for arguments.  Calls the existing
`run_native_workers_*` helper appropriate for the shape (or a
generic helper that takes the worker closure plus the shape's
`WorkerOut` type).

```rust
fn n_parallel_for_native_core<S: ParShape, F>(
    stores: &mut Stores,
    input: DbRef,
    elem_size: i32,
    threads: i32,
    shape: S,
    worker: F,
) -> DbRef
where
    F: Fn(&UnsafeCell<Stores>, DbRef) -> S::WorkerOut + Send + Sync,
{
    let n = vector::length_vector(&input, &stores.allocations) as usize;
    let (result_db, vec_rec, header_rec, return_sz) = S::alloc(stores, n);
    let results = run_workers(stores, &input, elem_size, threads, n, &worker);
    let mut fld = 8u32;
    for val in results {
        S::store(stores, &result_db, vec_rec, fld, val);
        fld += return_sz;
    }
    finalize_par_result(stores, result_db, n, vec_rec, header_rec)
}
```

`run_workers` may need its own genericisation since today
`run_native_workers_primitive` is i64-typed.  Generic over
`WorkerOut: Send` is straightforward.

**Validation**: compiles in isolation.

### Step 9.5 — Re-implement the three public fns as thin wrappers

**Action**: each existing public fn becomes ~3 lines that call the
generic core with its shape:

```rust
pub fn n_parallel_for_native(
    stores: &mut Stores,
    input: DbRef,
    elem_size: i32,
    return_size: i32,
    threads: i32,
    worker: impl Fn(&UnsafeCell<Stores>, DbRef) -> i64 + Send + Sync,
) -> DbRef {
    n_parallel_for_native_core(stores, input, elem_size, threads,
        ScalarShape { return_size }, worker)
}

pub fn n_parallel_for_text_native(...) -> DbRef {
    n_parallel_for_native_core(stores, input, elem_size, threads,
        TextShape, worker)
}

pub fn n_parallel_for_ref_native(...) -> DbRef {
    n_parallel_for_native_core(stores, input, elem_size, threads,
        RefShape { struct_size, known_type }, worker)
}
```

**Validation**:
```bash
# Behavioural: parallel-suite still passes
cargo test --release --test wrap par
cargo test --release --test threading 2>&1 | tail -3       # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3 # 35/35
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"

# Performance: no regression on parallel benches
cargo bench --bench parallel 2>&1 | tee /tmp/p09-step09-bench.log
diff /tmp/p09-step09-baseline/par-test.log <(cargo test --release --test wrap par 2>&1)
```

### Step 9.6 — LOC + structural test

**Action**: confirm consolidation:

```rust
// tests/codegen_emitter.rs
#[test]
fn parallel_runtime_consolidated() {
    let src = std::fs::read_to_string("src/codegen_runtime.rs").unwrap();
    // Each public fn body should be ≤ 15 lines (thin wrapper).
    for fn_name in ["n_parallel_for_native", "n_parallel_for_text_native",
                    "n_parallel_for_ref_native"] {
        let pattern = format!("pub fn {fn_name}");
        let start = src.find(&pattern).expect(fn_name);
        let body_start = src[start..].find('{').unwrap() + start;
        let body_end = find_matching_brace(&src, body_start);
        let body_lines = src[body_start..body_end].lines().count();
        assert!(body_lines <= 15,
            "{fn_name} body is {body_lines} lines — consolidation incomplete");
    }
}
```

**Validation**: test passes.

### Step 9.7 — Update phase 06 plan

**Action**: edit `06-threading.md` to reference the consolidated
core.  Phase 06's queue-variant runtime fns will be 3 thin wrappers
on top of `n_parallel_queue_native_core<S: ParShape, F>(...)` —
the shape trait carries over.

**Validation**: review.

## Acceptance for phase 09 overall

```bash
cargo test --release --test codegen_emitter::parallel_runtime_consolidated
cargo test --release --test wrap par
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"

# Net delta: ~250 lines retired across the three fns; ~120 lines
# added for the shape trait + generic core.  Net ~130 lines saved.
# Phase 06 will add 3 wrappers (~45 lines) instead of 3 full fns
# (~250 lines).  Cumulative saving: ~330 lines.
```

## Gate updates per step

This phase is library-level (runtime fn bodies), not codegen.
Emission stays byte-identical throughout — the public fn names +
ABI tags don't change.  Gate's `legacy_count` drops from 2 → 0
when steps 9.5 + (deferred) `n_parallel_for_*` migration ship,
which finally unblocks phase 01 step 1.7 cleanup.

| Step | Gate update |
|---|---|
| 9.1-9.2 | Capture parallel-runtime baselines + behavioural log. |
| 9.3-9.5 | Refactor inside `codegen_runtime.rs`; emission unchanged. |
| 9.5+1.6 (parallel migration) | Flip `n_parallel_for_native` / `n_parallel_for_ref_native` from `LegacyStores` → `Cell`.  `legacy_count` reaches 0; gate prints "phase 01 step 1.7 cleanup is now unblocked". |
| 9.6 | New `parallel_runtime_consolidated` structural test. |

## Commit shape

5-6 commits across the steps; ships as one PR.

## Problems encountered

_(append per problem — likely: `run_native_workers_*` may not
genericise cleanly over `WorkerOut` if its closure-arg shape is
shape-specific.  If so, keep two worker dispatchers (primitive +
heap) but share the alloc / store / finalize core.)_

### Worker-dispatcher genericisation rejected (2026-05-02)

The plan-doc sketch genericised `run_workers` over a single
`WorkerOut: Send` type with a uniform `Vec<WorkerOut>` return.
That doesn't fit text or ref:

- **Text** uses per-worker output store slots (`add_output_slot`,
  `set_str` interning) and the parent reads back via `get_str`.
  The dispatcher returns `Vec<String>` but the merge logic on the
  worker side is store-aware.
- **Ref** returns `Vec<(Vec<(usize, DbRef)>, Stores)>` — each batch
  carries the worker's full `Stores` clone so the parent can
  `copy_from_worker[_unowned]` across the worker/parent store
  boundary.

A single `Vec<WorkerOut>` would have erased the cross-store deep-
copy capability ref needs.

**Resolution**: the trait grew a second associated type
`Self::Batches` plus a `store_results` method — each shape carries
its own batch type and merge implementation.  The shared core
only owns the alloc → run_workers → store_results → finalise
sequencing.  The three existing `run_native_workers_*` helpers
stay as free fns, called from the trait's `run_workers`
implementations.

## Implementation notes

_(append per non-obvious decision)_

### Trait shape vs the plan-doc sketch (2026-05-02)

The plan-doc sketched `ParShape` with one associated type
(`WorkerOut`) plus `alloc` and `store` methods.  Implementation
revealed the worker phase varies more than the merge phase, so the
trait grew:

- `Self::Batches` associated type — captures the shape-specific
  worker output (scalar: `Vec<i64>`; text: `Vec<String>`; ref:
  `Vec<(Vec<(usize, DbRef)>, Stores)>`).
- `run_workers(...)` static method — wraps the existing
  `run_native_workers_*` free fns.  Static (not `&self`) because
  the worker phase doesn't need shape state — the closure's
  `WorkerOut` type already drives dispatcher selection via the
  trait impl.
- `store_results(&self, ...)` method — `&self` carries shape
  state (`return_size` / `struct_size` / `known_type`) into the
  per-shape merge logic.  Replaces the plan's `alloc` + `store`
  pair; alloc reduces to a single `return_sz()` query (`u32`)
  consumed by `alloc_par_result` in the shared core.

### Net delta vs the plan-doc sketch (2026-05-02)

Pre-consolidation (per `wc -l /tmp/p09-step09-baseline/*.rs`):
36 + 24 + 39 = 99 body lines across the three public fns (worker
dispatchers excluded — they're shared infrastructure).

Post-consolidation (`pub fn` start to closing brace):
20 + 13 + 24 = 57 body lines across the three thin wrappers, plus
20 lines of `n_parallel_for_native_core` and ~150 lines of trait
+ 3 impls (`ScalarShape`, `TextShape`, `RefShape`).

Net: ~210 lines emitted to replace ~99 lines.  Worse on raw line
count, but:
- Phase 06 will add 3 queue-variant public fns at ~3 lines each
  (~10 line increase) instead of 3 ~80-line fns (~250 line
  increase).  Cumulative saving over the queue-extension path:
  ~240 lines.
- The shape-specific logic is now one place to read, and the
  invariant "every par variant goes alloc → run_workers →
  store_results → finalise" is enforced by `n_parallel_for_native_core`'s
  signature.

The structural test `parallel_runtime_consolidated` pins this:
each public fn body ≤ 15 lines and must call
`n_parallel_for_native_core`.
