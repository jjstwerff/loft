// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Parallel execution: dispatch a worker function over vector elements using OS threads.
//!
//! When the `threading` feature is enabled, each `run_parallel_*` function spawns OS
//! threads.  When `threading` is disabled (e.g. under WASM), the loop body runs
//! sequentially in the caller's thread — same results, no parallelism.

use crate::database::{Call, Stores, WorkerStores};
use crate::keys::DbRef;
use crate::state::State;
use crate::vector;
use std::sync::Arc;
#[cfg(feature = "threading")]
use std::thread;

/// Shared bytecode + library for a single `parallel_for` dispatch.
type ProgramRefs = (Arc<Vec<u8>>, Arc<Vec<crate::database::Call>>);

/// Wrapper for `*mut u8` that is `Send + Sync` for cross-thread direct writes.
/// SAFETY: callers must ensure non-overlapping writes and join all threads.
#[cfg(feature = "threading")]
struct SendMutPtr(*mut u8);
#[cfg(feature = "threading")]
unsafe impl Send for SendMutPtr {}
#[cfg(feature = "threading")]
unsafe impl Sync for SendMutPtr {}

/// Immutable interpreter context shared across all worker threads.
///
/// All three fields are `Arc`-wrapped so that spawning many workers only
/// increments a reference count per thread instead of deep-copying the
/// bytecode and library on every `parallel_for` call.
pub struct WorkerProgram {
    pub bytecode: Arc<Vec<u8>>,
    pub library: Arc<Vec<Call>>,
    /// Cached library index of `n_stack_trace`; `u16::MAX` = not found.
    /// Copied into each worker's `State::stack_trace_lib_nr` so that
    /// `stack_trace()` works inside parallel workers (fix #92).
    pub stack_trace_lib_nr: u16,
    /// Raw pointer to the parent's `Data` and the function-position table.
    /// `data_ptr` may be null if the caller does not have stack_trace context;
    /// when non-null, each worker's `State::data_ptr` and `fn_positions` are
    /// populated so `stack_trace()` returns named frames inside workers
    /// (fix #92 — without this the worker frame is shown as `<worker>` with
    /// no function name, file, or line).  The pointer is borrowed from a
    /// `&Data` held by the spawning frame, which outlives `thread::scope`.
    pub data_ptr: *const crate::data::Data,
    pub fn_positions: Arc<Vec<u32>>,
    /// Source-line lookup table (bytecode position → source line) shared from
    /// the parent State.  Workers populate `State::line_numbers` from this so
    /// `stack_trace()` can resolve real source lines instead of always
    /// reporting line 0 (fix #92 follow-on).
    pub line_numbers: Arc<std::collections::BTreeMap<u32, u32>>,
}

// Safety: WorkerProgram is read-only after construction; Call is fn ptr (Send).
unsafe impl Send for WorkerProgram {}
unsafe impl Sync for WorkerProgram {}

impl WorkerProgram {
    /// Clone the shared program references for a new worker `State` — O(1).
    fn clone_refs(&self) -> ProgramRefs {
        (Arc::clone(&self.bytecode), Arc::clone(&self.library))
    }

    /// Create a worker `State` from this program, with `stack_trace_lib_nr` propagated.
    fn new_state(&self, worker_stores: WorkerStores) -> State {
        let (bytecode, library) = self.clone_refs();
        let mut state = State::new_worker(worker_stores, bytecode, library);
        state.stack_trace_lib_nr = self.stack_trace_lib_nr;
        // Fix #92: propagate Data ptr + fn_positions so stack_trace() inside
        // a parallel worker can resolve the worker's d_nr → name/file/line.
        state.data_ptr = self.data_ptr;
        state.fn_positions.clone_from(&*self.fn_positions);
        state.line_numbers = (*self.line_numbers).clone();
        state
    }
}

// ── Plan-06 phase 1 step 4.5 — shared run_parallel_* template ─────────────────
//
// Five of the six run_parallel_* variants share a uniform shape after the
// channel→thread::scope conversion in steps 2–4:
//
//   1. Spawn N worker threads via `thread::scope`.
//   2. Each worker gets a fresh `clone_for_worker()` snapshot, an Arc-bumped
//      reference to the WorkerProgram, and a row range `start..end`.
//   3. Each worker computes some R from its row range and returns it.
//   4. The main thread joins all workers, collecting `Vec<R>` in worker-id order.
//
// `parallel_workers` captures that scaffolding; the per-variant `run_parallel_*`
// fns become thin wrappers that pass a closure describing the per-worker work.
// `run_parallel_light` is **not** on this template — it uses
// `clone_for_light_worker(pool_slice)` with a mutable borrow of pre-allocated
// pool stores.  Phase 5's `Arc<Store>` rewrite makes every path light by
// default; at that point the template absorbs run_parallel_light too.

/// Common scope-spawn-collect scaffolding for the five non-light
/// `run_parallel_*` variants.
///
/// The closure `f` receives `(start, end, worker_stores, &Arc<WorkerProgram>)`
/// and must produce a `Send` value `R`.  Each worker's R is collected in
/// worker-id order and returned as `Vec<R>`.
///
/// Implementation notes:
/// - `f` is captured by reference inside the spawn closures.  The spawn
///   closures are `move`, so they capture `&f` by `Copy`; rust enforces
///   `f: Sync` so concurrent calls are safe.
/// - Each worker gets its own `WorkerStores` via `clone_for_worker`; the
///   closure may call `add_output_slot` / `take_slot` as needed and
///   include them in its `R` payload.
/// - `n_threads` is clamped to `min(n_threads, n_rows)`; for `n_rows == 0`
///   the caller is expected to short-circuit before calling.
#[cfg(feature = "threading")]
fn parallel_workers<R, F>(
    stores: &Stores,
    program: WorkerProgram,
    n_threads: usize,
    n_rows: usize,
    f: F,
) -> Vec<R>
where
    R: Send,
    F: Fn(usize, usize, WorkerStores, &Arc<WorkerProgram>) -> R + Sync,
{
    let threads = n_threads.max(1).min(n_rows.max(1));
    let program = Arc::new(program);
    thread::scope(|s| {
        let f_ref = &f;
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let start = t * n_rows / threads;
                let end = (t + 1) * n_rows / threads;
                let worker_stores = stores.clone_for_worker();
                let prog = Arc::clone(&program);
                s.spawn(move || f_ref(start, end, worker_stores, &prog))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker thread panicked"))
            .collect()
    })
}

/// Sequential equivalent of `parallel_workers` for the `not(threading)` path.
/// Runs the closure once with `start = 0`, `end = n_rows`, and a single
/// `clone_for_worker()` snapshot.  Returned `Vec<R>` always has length 1.
#[cfg(not(feature = "threading"))]
fn parallel_workers<R, F>(
    stores: &Stores,
    program: WorkerProgram,
    _n_threads: usize,
    n_rows: usize,
    f: F,
) -> Vec<R>
where
    F: FnOnce(usize, usize, WorkerStores, &Arc<WorkerProgram>) -> R,
{
    let worker_stores = stores.clone_for_worker();
    let prog = Arc::new(program);
    vec![f(0, n_rows, worker_stores, &prog)]
}

/// Run workers, writing results directly into `out_ptr`.
/// # Panics
/// Panics if a worker thread panics.
/// Each thread writes a non-overlapping slice — no channel, no reordering.
#[allow(clippy::too_many_arguments)]
// Threading path moves `program` into `Arc::new(program)`; non-threading path
// only borrows it, hence the feature-gated allow.  The raw `out_ptr` is written
// to from inside `unsafe { }` blocks in both paths; making the public function
// `unsafe` would cascade across every `par(...)` call site and the QUALITY 6a
// native-codegen path, so the allow narrows to the non-threading build where
// clippy can trace the bare deref inline.  `dead_code` for the same reason as
// `needless_pass_by_value`: only the threading callers live in main.rs's
// binary crate view; under `--no-default-features` those callers are cfg'd out.
#[cfg_attr(
    not(feature = "threading"),
    allow(
        clippy::needless_pass_by_value,
        clippy::not_unsafe_ptr_arg_deref,
        dead_code
    )
)]
pub fn run_parallel_direct(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    return_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    out_ptr: *mut u8,
    n_rows: usize,
) {
    if n_rows == 0 {
        return;
    }
    // WASM threading dispatches to JS host bridge (Worker Threads).
    #[cfg(all(feature = "threading", feature = "wasm"))]
    {
        let _ = (stores, program, extra_args);
        // Call globalThis.loftHost.parallel_run(fn_pos, input_store, input_rec,
        //   input_pos, element_size, return_size, n_threads, out_store, out_rec, out_pos, n_rows)
        let args = js_sys::Array::new();
        args.push(&wasm_bindgen::JsValue::from(fn_pos));
        args.push(&wasm_bindgen::JsValue::from(input.store_nr));
        args.push(&wasm_bindgen::JsValue::from(input.rec));
        args.push(&wasm_bindgen::JsValue::from(input.pos));
        args.push(&wasm_bindgen::JsValue::from(element_size));
        args.push(&wasm_bindgen::JsValue::from(return_size));
        args.push(&wasm_bindgen::JsValue::from(n_threads as u32));
        args.push(&wasm_bindgen::JsValue::from(n_rows as u32));
        crate::wasm::host_call_raw("parallel_run", &args);
    }
    // Native OS threads — uses the parallel_workers template.  Each
    // worker allocates its own output slot inside the closure; the
    // returned (Stores, slot_nr, n_bytes, start) tuple lets the
    // parent copy back into out_ptr at the right offset.  The
    // cross-thread `SendMutPtr(out_ptr)` raw-pointer hack is gone.
    #[cfg(all(feature = "threading", not(feature = "wasm")))]
    {
        let ret_sz = return_size as usize;
        let input_t = *input;
        let extras = extra_args.to_vec();
        let results = parallel_workers(
            stores,
            program,
            n_threads,
            n_rows,
            |start, end, mut ws, prog| {
                let row_count = end - start;
                let bytes_needed = row_count * ret_sz;
                let slot_words = (bytes_needed.div_ceil(8)).max(1) as u32;
                let slot = ws.add_output_slot(slot_words);
                let mut state = prog.new_state(ws);
                // SAFETY: slot's buffer was just allocated for this worker;
                // we write exactly bytes_needed bytes contiguously.
                let slot_ptr = state.database.allocations[slot.store_nr as usize].base_ptr();
                for (local_idx, row_idx) in (start..end).enumerate() {
                    let row_ref = vector::get_vector(
                        &input_t,
                        element_size,
                        row_idx as i64,
                        &state.database.allocations,
                    );
                    let val = state.execute_at_raw(fn_pos, &row_ref, &extras, ret_sz as u32);
                    unsafe {
                        let dst = slot_ptr.add(local_idx * ret_sz);
                        std::ptr::copy_nonoverlapping(
                            (&raw const val).cast::<u8>(),
                            dst,
                            ret_sz,
                        );
                    }
                }
                (state.database, slot.store_nr, bytes_needed, start)
            },
        );
        for (worker_db, slot_nr, n_bytes, start) in results {
            // SAFETY: per-worker disjoint ranges; n_bytes ≤ slot capacity.
            unsafe {
                let src = worker_db.allocations[slot_nr as usize].base_ptr();
                let dst = out_ptr.add(start * ret_sz);
                std::ptr::copy_nonoverlapping(src, dst, n_bytes);
            }
        }
    }
    #[cfg(not(feature = "threading"))]
    {
        let _ = n_threads;
        let mut state = program.new_state(stores.clone_for_worker());
        let ret_sz = return_size as usize;
        for row_idx in 0..n_rows {
            let row_idx_i32 = row_idx as i64;
            let row_ref = vector::get_vector(
                input,
                element_size,
                row_idx_i32,
                &state.database.allocations,
            );
            let val = state.execute_at_raw(fn_pos, &row_ref, extra_args, return_size);
            unsafe {
                let dst = out_ptr.add(row_idx * ret_sz);
                std::ptr::copy_nonoverlapping((&raw const val).cast::<u8>(), dst, ret_sz);
            }
        }
    }
}

/// Channel-based parallel: one u64 per row (for bool and other sub-4-byte types).
/// # Panics
/// Panics if a worker thread panics.
#[allow(clippy::too_many_arguments)]
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
#[cfg_attr(
    not(feature = "threading"),
    allow(clippy::needless_pass_by_value, dead_code)
)]
#[must_use]
pub fn run_parallel_raw(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    return_size: u32,
    n_threads: usize,
    extra_args: &[u64],
) -> Vec<u64> {
    let n_rows = vector::length_vector(input, &stores.allocations) as usize;
    if n_rows == 0 {
        return Vec::new();
    }
    let input_t = *input;
    let extras = extra_args.to_vec();
    let batches = parallel_workers(stores, program, n_threads, n_rows, |start, end, ws, prog| {
        let mut state = prog.new_state(ws);
        let mut batch = Vec::with_capacity(end - start);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            batch.push(state.execute_at_raw(fn_pos, &row_ref, &extras, return_size));
        }
        (start, batch)
    });
    let mut results = vec![0u64; n_rows];
    for (start, batch) in batches {
        for (offset, val) in batch.into_iter().enumerate() {
            results[start + offset] = val;
        }
    }
    results
}

/// Parallel text returns: workers copy `Str` to owned `String` before state drops.
/// # Panics
/// Panics if a worker thread panics.
#[allow(clippy::too_many_arguments)]
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
#[cfg_attr(
    not(feature = "threading"),
    allow(clippy::needless_pass_by_value, dead_code)
)]
#[must_use]
pub fn run_parallel_text(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    n_rows: usize,
    n_hidden_text: usize,
) -> Vec<String> {
    if n_rows == 0 {
        return Vec::new();
    }
    let input_t = *input;
    let extras = extra_args.to_vec();
    let batches = parallel_workers(stores, program, n_threads, n_rows, |start, end, ws, prog| {
        let mut state = prog.new_state(ws);
        let mut batch: Vec<String> = Vec::with_capacity(end - start);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            batch.push(state.execute_at_text(fn_pos, &row_ref, &extras, n_hidden_text));
        }
        (start, batch)
    });
    let mut results: Vec<String> = (0..n_rows).map(|_| String::new()).collect();
    for (start, batch) in batches {
        for (offset, val) in batch.into_iter().enumerate() {
            results[start + offset] = val;
        }
    }
    results
}

/// Parallel struct-reference returns: workers send back `(index, DbRef)` batches
/// together with their `Stores` so the main thread can deep-copy struct data.
/// # Panics
/// Panics if a worker thread panics.
#[allow(clippy::too_many_arguments)]
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
#[cfg_attr(
    not(feature = "threading"),
    allow(clippy::needless_pass_by_value, dead_code)
)]
#[must_use]
pub fn run_parallel_ref(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    n_rows: usize,
) -> Vec<(Vec<(usize, DbRef)>, crate::database::Stores)> {
    if n_rows == 0 {
        return Vec::new();
    }
    let input_t = *input;
    let extras = extra_args.to_vec();
    parallel_workers(stores, program, n_threads, n_rows, |start, end, ws, prog| {
        let mut state = prog.new_state(ws);
        let mut batch = Vec::with_capacity(end - start);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            batch.push((row_idx, state.execute_at_ref(fn_pos, &row_ref, &extras)));
        }
        (batch, state.database)
    })
}

/// Parallel integer returns: one `i32` per row, original order.
/// # Panics
/// Panics if a worker thread panics.
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
#[cfg_attr(
    not(feature = "threading"),
    allow(clippy::needless_pass_by_value, dead_code)
)]
#[must_use]
pub fn run_parallel_int(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    n_threads: usize,
) -> Vec<i64> {
    let n_rows = vector::length_vector(input, &stores.allocations) as usize;
    if n_rows == 0 {
        return Vec::new();
    }
    let input_t = *input;
    let batches = parallel_workers(stores, program, n_threads, n_rows, |start, end, ws, prog| {
        let mut state = prog.new_state(ws);
        let mut batch = Vec::with_capacity(end - start);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            batch.push(state.execute_at(fn_pos, &row_ref));
        }
        (start, batch)
    });
    let mut results = vec![i64::MIN; n_rows];
    for (start, batch) in batches {
        for (offset, val) in batch.into_iter().enumerate() {
            results[start + offset] = val;
        }
    }
    results
}

// ── A14.4 — run_parallel_light ───────────────────────────────────────────────

/// Lightweight parallel dispatch — borrows main stores read-only instead
/// of deep-copying them.  Structurally identical to `run_parallel_direct` but
/// uses `clone_for_light_worker` with a pre-allocated `WorkerPool`.
///
/// # Panics
/// Panics if any row index exceeds `i32::MAX`.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code, unused_variables)]
// A14.6 will wire this into the parser
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
#[cfg_attr(
    not(feature = "threading"),
    allow(clippy::needless_pass_by_value, clippy::not_unsafe_ptr_arg_deref,)
)]
pub fn run_parallel_light(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    return_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    out_ptr: *mut u8,
    n_rows: usize,
    pool: &mut WorkerPool,
) {
    if n_rows == 0 {
        return;
    }
    #[cfg(feature = "threading")]
    {
        let threads = n_threads.max(1).min(n_rows);
        let program = Arc::new(program);
        let out = Arc::new(SendMutPtr(out_ptr));

        thread::scope(|s| {
            for t in 0..threads {
                let start = t * n_rows / threads;
                let end = (t + 1) * n_rows / threads;
                // borrow main stores + get pool slice for this worker.
                let worker_stores = unsafe { stores.clone_for_light_worker(pool.slice_mut(t)) };
                let prog = Arc::clone(&program);
                let input_t = *input;
                let extras = extra_args.to_vec();
                let out_t = Arc::clone(&out);
                let ret_sz = return_size as usize;

                s.spawn(move || {
                    let mut state = prog.new_state(worker_stores);
                    for row_idx in start..end {
                        let row_idx_i32 = row_idx as i64;
                        let row_ref = vector::get_vector(
                            &input_t,
                            element_size,
                            row_idx_i32,
                            &state.database.allocations,
                        );
                        let val = state.execute_at_raw(fn_pos, &row_ref, &extras, ret_sz as u32);
                        unsafe {
                            let dst = out_t.0.add(row_idx * ret_sz);
                            std::ptr::copy_nonoverlapping(
                                (&raw const val).cast::<u8>(),
                                dst,
                                ret_sz,
                            );
                        }
                    }
                });
            }
        });
    }
    #[cfg(not(feature = "threading"))]
    {
        let worker_stores = unsafe { stores.clone_for_light_worker(pool.slice_mut(0)) };
        let mut state = program.new_state(worker_stores);
        for row_idx in 0..n_rows {
            let row_idx_i32 = row_idx as i64;
            let row_ref = vector::get_vector(
                input,
                element_size,
                row_idx_i32,
                &state.database.allocations,
            );
            let val = state.execute_at_raw(fn_pos, &row_ref, extra_args, return_size);
            unsafe {
                let dst = out_ptr.add(row_idx * return_size as usize);
                std::ptr::copy_nonoverlapping(
                    (&raw const val).cast::<u8>(),
                    dst,
                    return_size as usize,
                );
            }
        }
    }
}

// ── A14.2 — WorkerPool ──────────────────────────────────────────────────────

use crate::store::Store;

/// Pre-allocated pool of stores for `par_light` workers.
/// Worker `i` owns the exclusive slice `[i*spw .. (i+1)*spw]`.
pub struct WorkerPool {
    stores: Vec<Store>,
    stores_per_worker: usize,
}

impl WorkerPool {
    /// Create a pool with `n_workers × stores_per_worker` stores, each with
    /// `store_capacity` words of initial capacity.
    // Only called from the threading-enabled path; under `--no-default-features`
    // the binary's view of this module has no callers.  See `run_parallel_direct`.
    #[cfg_attr(not(feature = "threading"), allow(dead_code))]
    #[must_use]
    pub fn new(n_workers: usize, stores_per_worker: usize, store_capacity: u32) -> Self {
        let total = n_workers * stores_per_worker;
        let stores = (0..total).map(|_| Store::new(store_capacity)).collect();
        WorkerPool {
            stores,
            stores_per_worker,
        }
    }

    /// Return the exclusive mutable slice for worker `idx`.
    #[must_use]
    pub fn slice_mut(&mut self, worker_idx: usize) -> &mut [Store] {
        let spw = self.stores_per_worker;
        &mut self.stores[worker_idx * spw..(worker_idx + 1) * spw]
    }
}

/// Run N independent arms concurrently, each at a given bytecode position.
#[allow(dead_code)]
/// Each arm runs as a void function (no args, no return value).
/// Uses the same store-isolation model as `par()`: each arm gets a read-only snapshot.
/// # Panics
/// Panics if any arm thread panics.
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
// (dead_code already allowed above — only needless_pass_by_value is feature-gated.)
#[cfg_attr(not(feature = "threading"), allow(clippy::needless_pass_by_value))]
pub fn run_parallel_block(stores: &Stores, program: WorkerProgram, arm_positions: &[u32]) {
    if arm_positions.is_empty() {
        return;
    }
    #[cfg(all(feature = "threading", not(feature = "wasm")))]
    {
        let program = Arc::new(program);
        thread::scope(|s| {
            for &pos in arm_positions {
                let worker_stores = stores.clone_for_worker();
                let prog = Arc::clone(&program);
                s.spawn(move || {
                    let mut state = prog.new_state(worker_stores);
                    state.execute_at_void(pos);
                });
            }
        });
    }
    #[cfg(not(feature = "threading"))]
    {
        // Sequential fallback (WASM or threading disabled).
        for &pos in arm_positions {
            let mut state = program.new_state(stores.clone_for_worker());
            state.execute_at_void(pos);
        }
    }
}

#[cfg(test)]
mod pool_tests {
    use super::*;

    #[test]
    fn worker_slices_are_disjoint() {
        let mut pool = WorkerPool::new(4, 3, 16);
        // Each worker's slice has 3 stores.
        let ptrs: Vec<*const Store> = (0..4).map(|i| pool.slice_mut(i).as_ptr()).collect();
        // All slice start pointers must be distinct.
        for i in 0..4 {
            for j in (i + 1)..4 {
                assert_ne!(ptrs[i], ptrs[j], "worker {i} and {j} share a slice");
            }
        }
    }

    #[test]
    fn pool_stores_can_claim_after_init() {
        let mut pool = WorkerPool::new(2, 2, 16);
        for s in pool.slice_mut(0) {
            s.init();
            s.free = false;
            let rec = s.claim(4);
            assert!(rec > 0, "claim on pool store must succeed");
        }
    }
}
