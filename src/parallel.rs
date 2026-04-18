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
use std::sync::mpsc;
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
    // Native OS threads via thread::scope.
    #[cfg(all(feature = "threading", not(feature = "wasm")))]
    {
        let threads = n_threads.max(1).min(n_rows);
        let program = Arc::new(program);
        let out = Arc::new(SendMutPtr(out_ptr));

        thread::scope(|s| {
            for t in 0..threads {
                let start = t * n_rows / threads;
                let end = (t + 1) * n_rows / threads;
                let worker_stores = stores.clone_for_worker();
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
    #[cfg(feature = "threading")]
    {
        let threads = n_threads.max(1).min(n_rows);
        let program = Arc::new(program);
        let (tx, rx) = mpsc::channel::<Vec<(usize, u64)>>();
        let mut handles = Vec::with_capacity(threads);
        for t in 0..threads {
            let start = t * n_rows / threads;
            let end = (t + 1) * n_rows / threads;
            let worker_stores = stores.clone_for_worker();
            let prog = Arc::clone(&program);
            let tx_t = tx.clone();
            let input_t = *input;
            let extras = extra_args.to_vec();
            let handle = thread::spawn(move || {
                let mut state = prog.new_state(worker_stores);
                let mut batch = Vec::with_capacity(end - start);
                for row_idx in start..end {
                    let row_ref = vector::get_vector(
                        &input_t,
                        element_size,
                        row_idx as i64,
                        &state.database.allocations,
                    );
                    let val = state.execute_at_raw(fn_pos, &row_ref, &extras, return_size);
                    batch.push((row_idx, val));
                }
                tx_t.send(batch).expect("channel send failed");
            });
            handles.push(handle);
        }
        drop(tx);
        let mut results = vec![0u64; n_rows];
        for batch in rx {
            for (idx, val) in batch {
                results[idx] = val;
            }
        }
        for h in handles {
            h.join().expect("worker thread panicked");
        }
        results
    }
    #[cfg(not(feature = "threading"))]
    {
        let _ = n_threads;
        let mut state = program.new_state(stores.clone_for_worker());
        let mut results = vec![0u64; n_rows];
        for (row_idx, result) in results.iter_mut().enumerate() {
            let row_ref = vector::get_vector(
                input,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            *result = state.execute_at_raw(fn_pos, &row_ref, extra_args, return_size);
        }
        results
    }
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
    #[cfg(feature = "threading")]
    {
        let threads = n_threads.max(1).min(n_rows);
        let program = Arc::new(program);
        let (tx, rx) = mpsc::channel::<Vec<(usize, String)>>();
        let mut handles = Vec::with_capacity(threads);
        for t in 0..threads {
            let start = t * n_rows / threads;
            let end = (t + 1) * n_rows / threads;
            let worker_stores = stores.clone_for_worker();
            let prog = Arc::clone(&program);
            let tx_t = tx.clone();
            let input_t = *input;
            let extras = extra_args.to_vec();
            let handle = thread::spawn(move || {
                let mut state = prog.new_state(worker_stores);
                let mut batch = Vec::with_capacity(end - start);
                for row_idx in start..end {
                    let row_ref = vector::get_vector(
                        &input_t,
                        element_size,
                        row_idx as i64,
                        &state.database.allocations,
                    );
                    let s = state.execute_at_text(fn_pos, &row_ref, &extras, n_hidden_text);
                    batch.push((row_idx, s));
                }
                tx_t.send(batch).expect("channel send failed");
            });
            handles.push(handle);
        }
        drop(tx);
        let mut results: Vec<String> = (0..n_rows).map(|_| String::new()).collect();
        for batch in rx {
            for (idx, val) in batch {
                results[idx] = val;
            }
        }
        for h in handles {
            h.join().expect("worker thread panicked");
        }
        results
    }
    #[cfg(not(feature = "threading"))]
    {
        let _ = n_threads;
        let mut state = program.new_state(stores.clone_for_worker());
        let mut results: Vec<String> = (0..n_rows).map(|_| String::new()).collect();
        for (row_idx, result) in results.iter_mut().enumerate() {
            let row_ref = vector::get_vector(
                input,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            *result = state.execute_at_text(fn_pos, &row_ref, extra_args, n_hidden_text);
        }
        results
    }
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
    #[cfg(feature = "threading")]
    {
        let threads = n_threads.max(1).min(n_rows);
        let program = Arc::new(program);
        let (tx, rx) = mpsc::channel::<(Vec<(usize, DbRef)>, crate::database::Stores)>();
        let mut handles = Vec::with_capacity(threads);
        for t in 0..threads {
            let start = t * n_rows / threads;
            let end = (t + 1) * n_rows / threads;
            let worker_stores = stores.clone_for_worker();
            let prog = Arc::clone(&program);
            let tx_t = tx.clone();
            let input_t = *input;
            let extras = extra_args.to_vec();
            let handle = thread::spawn(move || {
                let mut state = prog.new_state(worker_stores);
                let mut batch = Vec::with_capacity(end - start);
                for row_idx in start..end {
                    let row_ref = vector::get_vector(
                        &input_t,
                        element_size,
                        row_idx as i64,
                        &state.database.allocations,
                    );
                    let val = state.execute_at_ref(fn_pos, &row_ref, &extras);
                    batch.push((row_idx, val));
                }
                tx_t.send((batch, state.database))
                    .expect("channel send failed");
            });
            handles.push(handle);
        }
        drop(tx);
        let mut results = Vec::with_capacity(threads);
        for batch in rx {
            results.push(batch);
        }
        for h in handles {
            h.join().expect("worker thread panicked");
        }
        results
    }
    #[cfg(not(feature = "threading"))]
    {
        let _ = n_threads;
        let mut state = program.new_state(stores.clone_for_worker());
        let mut batch = Vec::with_capacity(n_rows);
        for row_idx in 0..n_rows {
            let row_ref = vector::get_vector(
                input,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            let val = state.execute_at_ref(fn_pos, &row_ref, extra_args);
            batch.push((row_idx, val));
        }
        vec![(batch, state.database)]
    }
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
) -> Vec<i32> {
    let n_rows = vector::length_vector(input, &stores.allocations) as usize;
    if n_rows == 0 {
        return Vec::new();
    }
    #[cfg(feature = "threading")]
    {
        let threads = n_threads.max(1).min(n_rows);
        let program = Arc::new(program);
        let (tx, rx) = mpsc::channel::<Vec<(usize, i32)>>();

        let mut handles = Vec::with_capacity(threads);
        for t in 0..threads {
            let start = t * n_rows / threads;
            let end = (t + 1) * n_rows / threads;
            let worker_stores = stores.clone_for_worker();
            let prog = Arc::clone(&program);
            let tx_t = tx.clone();
            let input_t = *input;

            let handle = thread::spawn(move || {
                let mut state = prog.new_state(worker_stores);
                let mut batch = Vec::with_capacity(end - start);
                for row_idx in start..end {
                    let row_idx_i32 = row_idx as i64;
                    let row_ref = vector::get_vector(
                        &input_t,
                        element_size,
                        row_idx_i32,
                        &state.database.allocations,
                    );
                    let val = state.execute_at(fn_pos, &row_ref);
                    batch.push((row_idx, val));
                }
                tx_t.send(batch).expect("channel send failed");
            });
            handles.push(handle);
        }
        drop(tx);
        let mut results = vec![i32::MIN; n_rows];
        for batch in rx {
            for (idx, val) in batch {
                results[idx] = val;
            }
        }
        for h in handles {
            h.join().expect("worker thread panicked");
        }
        results
    }
    #[cfg(not(feature = "threading"))]
    {
        let _ = n_threads;
        let mut state = program.new_state(stores.clone_for_worker());
        let mut results = vec![i32::MIN; n_rows];
        for (row_idx, result) in results.iter_mut().enumerate() {
            let row_idx_i32 = row_idx as i64;
            let row_ref = vector::get_vector(
                input,
                element_size,
                row_idx_i32,
                &state.database.allocations,
            );
            *result = state.execute_at(fn_pos, &row_ref);
        }
        results
    }
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
