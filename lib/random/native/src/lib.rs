// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native random number generator. No loft dependency.

use rand_core::{RngCore, SeedableRng};
use rand_pcg::Pcg64;
use std::cell::RefCell;

thread_local! {
    static RNG: RefCell<Pcg64> = RefCell::new(Pcg64::from_os_rng());
}

// ── C-ABI exports ───────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn loft_rand_int(lo: i32, hi: i32) -> i32 {
    if lo == i32::MIN || hi == i32::MIN || lo > hi {
        return i32::MIN;
    }
    let range = (i64::from(hi) - i64::from(lo) + 1) as u64;
    let r = RNG.with(|rng| rng.borrow_mut().next_u64());
    lo + (r % range) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_rand_seed(seed: i64) {
    RNG.with(|rng| *rng.borrow_mut() = Pcg64::seed_from_u64(seed as u64));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_rand_indices(
    n: i32,
    out_ptr: *mut *mut i32,
    out_len: *mut usize,
) {
    let count = if n == i32::MIN || n <= 0 { 0 } else { n as usize };
    let mut indices: Vec<i32> = (0..count as i32).collect();
    let len = indices.len();
    for i in (1..len).rev() {
        let j = RNG.with(|rng| rng.borrow_mut().next_u64()) as usize % (i + 1);
        indices.swap(i, j);
    }
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

// ── Registration ────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_register_v1(
    register: unsafe extern "C" fn(*const u8, usize, *const (), *mut ()),
    ctx: *mut (),
) {
    unsafe {
        register(b"loft_rand_int".as_ptr(), 13, loft_rand_int as *const (), ctx);
        register(b"loft_rand_seed".as_ptr(), 14, loft_rand_seed as *const (), ctx);
        register(b"loft_rand_indices".as_ptr(), 17, loft_rand_indices as *const (), ctx);
        register(b"loft_free_indices".as_ptr(), 17, loft_free_indices as *const (), ctx);
    }
}
