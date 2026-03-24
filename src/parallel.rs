// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Parallel execution: dispatch a worker function over vector elements using OS threads.

#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::database::{Call, Stores};
use crate::keys::DbRef;
use crate::state::State;
use crate::vector;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

/// Shared bytecode + library for a single `parallel_for` dispatch.
type ProgramRefs = (Arc<Vec<u8>>, Arc<Vec<u8>>, Arc<Vec<crate::database::Call>>);

/// Wrapper for `*mut u8` that is `Send + Sync` for cross-thread direct writes.
/// SAFETY: callers must ensure non-overlapping writes and join all threads.
struct SendMutPtr(*mut u8);
unsafe impl Send for SendMutPtr {}
unsafe impl Sync for SendMutPtr {}

/// Immutable interpreter context shared across all worker threads.
///
/// All three fields are `Arc`-wrapped so that spawning many workers only
/// increments a reference count per thread instead of deep-copying the
/// bytecode and library on every `parallel_for` call.
pub struct WorkerProgram {
    pub bytecode: Arc<Vec<u8>>,
    pub text_code: Arc<Vec<u8>>,
    pub library: Arc<Vec<Call>>,
}

// Safety: WorkerProgram is read-only after construction; Call is fn ptr (Send).
unsafe impl Send for WorkerProgram {}
unsafe impl Sync for WorkerProgram {}

impl WorkerProgram {
    /// Clone the shared program references for a new worker `State` — O(1).
    fn clone_refs(&self) -> ProgramRefs {
        (
            Arc::clone(&self.bytecode),
            Arc::clone(&self.text_code),
            Arc::clone(&self.library),
        )
    }
}

/// Run workers in parallel, writing results directly into `out_ptr`.
/// # Panics
/// Panics if a worker thread panics.
/// Each thread writes a non-overlapping slice — no channel, no reordering.
#[allow(clippy::too_many_arguments)]
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
    let threads = n_threads.max(1).min(n_rows);
    let program = Arc::new(program);
    let mut handles = Vec::with_capacity(threads);
    let out = Arc::new(SendMutPtr(out_ptr));

    for t in 0..threads {
        let start = t * n_rows / threads;
        let end = (t + 1) * n_rows / threads;
        let worker_stores = stores.clone_for_worker();
        let prog = Arc::clone(&program);
        let input_t = *input;
        let extras = extra_args.to_vec();
        let out_t = Arc::clone(&out);
        let ret_sz = return_size as usize;

        let handle = thread::spawn(move || {
            let (bytecode, text_code, library) = prog.clone_refs();
            let mut state = State::new_worker(worker_stores, bytecode, text_code, library);
            for row_idx in start..end {
                let row_idx_i32 = i32::try_from(row_idx).expect("row index fits i32");
                let row_ref = vector::get_vector(
                    &input_t,
                    element_size,
                    row_idx_i32,
                    &state.database.allocations,
                );
                let val = state.execute_at_raw(fn_pos, &row_ref, &extras, ret_sz as u32);
                // Write return_size low bytes of val directly into the output buffer.
                unsafe {
                    let dst = out_t.0.add(row_idx * ret_sz);
                    std::ptr::copy_nonoverlapping((&raw const val).cast::<u8>(), dst, ret_sz);
                }
            }
        });
        handles.push(handle);
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }
}

/// Channel-based parallel: one u64 per row (for bool and other sub-4-byte types).
/// # Panics
/// Panics if a worker thread panics.
#[allow(clippy::too_many_arguments)]
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
            let (bytecode, text_code, library) = prog.clone_refs();
            let mut state = State::new_worker(worker_stores, bytecode, text_code, library);
            let mut batch = Vec::with_capacity(end - start);
            for row_idx in start..end {
                let row_ref = vector::get_vector(
                    &input_t,
                    element_size,
                    i32::try_from(row_idx).expect("row index fits i32"),
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

/// Parallel text returns: workers copy `Str` to owned `String` before state drops.
/// # Panics
/// Panics if a worker thread panics.
#[allow(clippy::too_many_arguments)]
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
            let (bytecode, text_code, library) = prog.clone_refs();
            let mut state = State::new_worker(worker_stores, bytecode, text_code, library);
            let mut batch = Vec::with_capacity(end - start);
            for row_idx in start..end {
                let row_ref = vector::get_vector(
                    &input_t,
                    element_size,
                    i32::try_from(row_idx).expect("row index fits i32"),
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

/// Parallel integer returns: one `i32` per row, original order.
/// # Panics
/// Panics if a worker thread panics.
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

    let threads = n_threads.max(1).min(n_rows);
    let program = Arc::new(program);
    let (tx, rx) = mpsc::channel::<Vec<(usize, i32)>>();

    // Distribute rows evenly across threads.
    let mut handles = Vec::with_capacity(threads);
    for t in 0..threads {
        let start = t * n_rows / threads;
        let end = (t + 1) * n_rows / threads;
        let worker_stores = stores.clone_for_worker();
        let prog = Arc::clone(&program);
        let tx_t = tx.clone();
        let input_t = *input;

        let handle = thread::spawn(move || {
            let (bytecode, text_code, library) = prog.clone_refs();
            let mut state = State::new_worker(worker_stores, bytecode, text_code, library);
            let mut batch = Vec::with_capacity(end - start);
            for row_idx in start..end {
                let row_idx_i32 = i32::try_from(row_idx).expect("row index fits i32");
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
    // Drop the original sender so rx finishes when all threads are done.
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
