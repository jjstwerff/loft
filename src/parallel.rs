// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Parallel execution support for the loft interpreter.
//!
//! `run_parallel_int` dispatches a compiled loft function over every element
//! of an input vector using a pool of OS threads.  Each thread gets a
//! locked (read-only) snapshot of the caller's stores and its own fresh
//! execution stack.

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

/// Run a compiled loft function over every row of `input` in parallel, returning
/// raw result bits (up to 8 bytes per row) in a `Vec<u64>`.
///
/// # Arguments
/// - `stores` — the calling state's stores; cloned read-only for workers.
/// - `program` — shared bytecode + library snapshot.
/// - `fn_pos` — bytecode offset of the worker function.
/// - `input` — `DbRef` pointing to the vector field.
/// - `element_size` — byte size of each vector element.
/// - `return_size` — byte size of the worker's return value (1, 4, or 8).
/// - `n_threads` — number of OS threads to use (clamped to row count).
///
/// # Returns
/// A `Vec<u64>` with one entry per row (in original order).  The low
/// `return_size` bytes of each entry hold the raw return bits; the high bytes
/// are zero.
///
/// # Panics
/// Panics if a worker thread panics or the internal channel send fails.
#[must_use]
#[allow(clippy::too_many_arguments)]
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
                let row_idx_i32 = i32::try_from(row_idx).expect("row index fits i32");
                let row_ref = vector::get_vector(
                    &input_t,
                    element_size,
                    row_idx_i32,
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

/// Run a compiled loft function (returning `integer`) in parallel over every
/// row of `input` and return the results ordered by row index.
///
/// # Arguments
/// - `stores` — the calling state's stores; cloned read-only for workers.
/// - `program` — shared bytecode + library snapshot.
/// - `fn_pos` — bytecode offset of the worker function.
/// - `input` — `DbRef` pointing to the vector field (same convention as `vector::length_vector`).
/// - `element_size` — byte size of each vector element (e.g. 12 for a `DbRef`).
/// - `n_threads` — number of OS threads to use (clamped to row count).
///
/// # Returns
/// A `Vec<i32>` with one entry per row, in the original row order.
///
/// # Panics
/// Panics if a worker thread panics, or if the internal channel send fails.
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
