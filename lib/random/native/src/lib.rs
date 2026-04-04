// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Random number generator for the random package.
//! No loft dependency — only depends on rand_core and rand_pcg.

use rand_core::{RngCore, SeedableRng};
use rand_pcg::Pcg64;
use std::cell::RefCell;

thread_local! {
    static RNG: RefCell<Pcg64> = RefCell::new(Pcg64::from_os_rng());
}

/// Return a random integer in [lo, hi] (inclusive).
/// Returns i32::MIN (null sentinel) if lo > hi or either is null.
pub fn rand_int(lo: i32, hi: i32) -> i32 {
    if lo == i32::MIN || hi == i32::MIN || lo > hi {
        return i32::MIN;
    }
    let range = (i64::from(hi) - i64::from(lo) + 1) as u64;
    let r = RNG.with(|rng| rng.borrow_mut().next_u64());
    lo + (r % range) as i32
}

/// Seed the thread-local RNG for reproducible sequences.
pub fn rand_seed(seed: i64) {
    RNG.with(|rng| *rng.borrow_mut() = Pcg64::seed_from_u64(seed as u64));
}

/// Fisher-Yates shuffle of a mutable i32 slice.
pub fn shuffle_ints(v: &mut [i32]) {
    let n = v.len();
    for i in (1..n).rev() {
        let j = RNG.with(|rng| rng.borrow_mut().next_u64()) as usize % (i + 1);
        v.swap(i, j);
    }
}

// ── C-ABI exports for interpreter dlopen ────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn loft_rand_int(lo: i32, hi: i32) -> i32 {
    rand_int(lo, hi)
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_rand_seed(seed: i64) {
    rand_seed(seed);
}

/// Shuffle indices [0..n) and return them via out-pointer.
/// Caller must free with `loft_free_indices`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_rand_indices(
    n: i32,
    out_ptr: *mut *mut i32,
    out_len: *mut usize,
) {
    let count = if n == i32::MIN || n <= 0 { 0 } else { n as usize };
    let mut indices: Vec<i32> = (0..count as i32).collect();
    shuffle_ints(&mut indices);
    unsafe {
        *out_len = indices.len();
        *out_ptr = indices.as_mut_ptr();
    }
    std::mem::forget(indices);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_free_indices(ptr: *mut i32, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, len, len) });
    }
}
