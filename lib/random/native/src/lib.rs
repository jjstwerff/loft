// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native random number generator using loft-ffi for store allocation.
//! Export names match `#native` symbols — no registration function needed.

#![allow(clippy::missing_safety_doc)]

use loft_ffi::{LoftRef, LoftStore};
use rand_core::{RngCore, SeedableRng};
use rand_pcg::Pcg64;
use std::cell::RefCell;

thread_local! {
    static RNG: RefCell<Pcg64> = RefCell::new(Pcg64::from_os_rng());
}

#[unsafe(no_mangle)]
pub extern "C" fn n_rand(lo: i32, hi: i32) -> i32 {
    if lo == i32::MIN || hi == i32::MIN || lo > hi {
        return i32::MIN;
    }
    let range = (i64::from(hi) - i64::from(lo) + 1) as u64;
    let r = RNG.with(|rng| rng.borrow_mut().next_u64());
    lo + (r % range) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn n_rand_seed(seed: i64) {
    RNG.with(|rng| *rng.borrow_mut() = Pcg64::seed_from_u64(seed as u64));
}

/// Returns a vector of `n` integers `[0, 1, ..., n-1]` in random order.
/// Allocates the vector directly in the loft store via `LoftStore` callbacks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_rand_indices(mut store: LoftStore, n: i32) -> LoftRef {
    let count = if n == i32::MIN || n <= 0 {
        0usize
    } else {
        n as usize
    };
    // Build shuffled indices via Fisher-Yates.
    let mut indices: Vec<i32> = (0..count as i32).collect();
    for i in (1..indices.len()).rev() {
        let j = RNG.with(|rng| rng.borrow_mut().next_u64()) as usize % (i + 1);
        indices.swap(i, j);
    }
    // Allocate vector in store and push elements.
    let mut vec = unsafe { store.alloc_vector(4, count as u32) };
    for &val in &indices {
        unsafe { store.vector_push_int(&mut vec, val) };
    }
    vec
}

loft_ffi::loft_register! {
    n_rand,
    n_rand_seed,
    n_rand_indices,
}
