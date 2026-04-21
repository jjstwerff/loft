// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Test fixture for native extension loading and marshalling.
//!
//! Tests all common patterns: scalar, vector, struct ref, const borrowing.
//! Every function validates its inputs and returns a checkable result.

// ── Pattern 1: Scalar → Scalar (baseline) ────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn n_ext_add_one(x: i32) -> i32 {
    x + 1
}

/// dlsym bait — returns different value so tests detect wrong dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_add_one(x: i32) -> i32 {
    x + 1000
}

/// Unregistered — for guard test (A7.2.5).
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_unregistered(x: i32) -> i32 {
    x + 9999
}

// ── Pattern 2: Vector<integer> as input ──────────────────────────────
// Post-2c: loft `integer` is i64 at rest.  The raw pointer is i64 and
// the macro emits `*const i64`.

/// Sum all elements of a vector<integer>. C-ABI version (raw pointer).
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_vec_sum(data_ptr: *const i64, data_count: u32) -> i32 {
    if data_ptr.is_null() || data_count == 0 {
        return -1;
    }
    let data = unsafe { std::slice::from_raw_parts(data_ptr, data_count as usize) };
    eprintln!("[ext_vec_sum] count={} first={} last={}", data.len(), data[0], data[data.len()-1]);
    data.iter().sum::<i64>() as i32
}

/// Interpreter wrapper — extracts vector from LoftStore + LoftRef.
loft_ffi::vec_wrapper!(n_ext_vec_sum, loft_ext_vec_sum(data: vec<i64>) -> i32);

// ── Pattern 3: Vector<f32> as input (single-precision) ───────────────

/// Sum of vector<single> elements. C-ABI version.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_vec_sum_f32(data_ptr: *const f32, data_count: u32) -> i32 {
    if data_ptr.is_null() || data_count == 0 {
        return -1;
    }
    let data = unsafe { std::slice::from_raw_parts(data_ptr, data_count as usize) };
    eprintln!("[ext_vec_sum_f32] count={} first={} last={}", data.len(), data[0], data[data.len()-1]);
    data.iter().sum::<f32>() as i32
}

loft_ffi::vec_wrapper!(n_ext_vec_sum_f32, loft_ext_vec_sum_f32(data: vec<f32>) -> i32);

// ── Pattern 4: Scalars before vector ─────────────────────────────────

/// offset + sum(data). C-ABI version.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_offset_sum(offset: i32, data_ptr: *const i64, data_count: u32) -> i32 {
    if data_ptr.is_null() || data_count == 0 {
        return offset;
    }
    let data = unsafe { std::slice::from_raw_parts(data_ptr, data_count as usize) };
    eprintln!("[ext_offset_sum] offset={} count={}", offset, data.len());
    offset + data.iter().sum::<i64>() as i32
}

loft_ffi::vec_wrapper!(n_ext_offset_sum, loft_ext_offset_sum(offset: i32, data: vec<i64>) -> i32);

// ── Pattern 5: Vector between scalars ────────────────────────────────

/// a + sum(data) + b. C-ABI version.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_sandwich_sum(a: i32, data_ptr: *const i64, data_count: u32, b: i32) -> i32 {
    if data_ptr.is_null() || data_count == 0 {
        return a + b;
    }
    let data = unsafe { std::slice::from_raw_parts(data_ptr, data_count as usize) };
    eprintln!("[ext_sandwich_sum] a={} count={} b={}", a, data.len(), b);
    a + data.iter().sum::<i64>() as i32 + b
}

loft_ffi::vec_wrapper!(n_ext_sandwich_sum, loft_ext_sandwich_sum(a: i32, data: vec<i64>, b: i32) -> i32);

// ── Pattern 6: Vector from struct field ──────────────────────────────

/// Read a vector<integer> that comes from a struct field access (e.g. canvas.data).
/// This tests the indirect reference path where the DbRef on the stack
/// points to a struct field containing the vector record number.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_struct_vec_len(data_ptr: *const i64, data_count: u32) -> i32 {
    eprintln!("[ext_struct_vec_len] count={}", data_count);
    let _ = data_ptr;
    data_count as i32
}

loft_ffi::vec_wrapper!(n_ext_struct_vec_len, loft_ext_struct_vec_len(data: vec<i64>) -> i32);

// ── Pattern 7: Vector inside loop with if ────────────────────────────

/// Same as vec_sum but with logging to detect store issues in loops.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_loop_vec_sum(data_ptr: *const i64, data_count: u32) -> i32 {
    if data_ptr.is_null() || data_count == 0 {
        eprintln!("[ext_loop_vec_sum] EMPTY: ptr={:?} count={}", data_ptr, data_count);
        return -1;
    }
    let data = unsafe { std::slice::from_raw_parts(data_ptr, data_count as usize) };
    let sum: i64 = data.iter().sum();
    eprintln!("[ext_loop_vec_sum] count={} sum={}", data.len(), sum);
    sum as i32
}

loft_ffi::vec_wrapper!(n_ext_loop_vec_sum, loft_ext_loop_vec_sum(data: vec<i64>) -> i32);

// ── Registration ─────────────────────────────────────────────────────

loft_ffi::loft_register! {
    loft_ext_add_one => n_ext_add_one,
    loft_ext_vec_sum => n_ext_vec_sum,
    loft_ext_vec_sum_f32 => n_ext_vec_sum_f32,
    loft_ext_offset_sum => n_ext_offset_sum,
    loft_ext_sandwich_sum => n_ext_sandwich_sum,
    loft_ext_struct_vec_len => n_ext_struct_vec_len,
    loft_ext_loop_vec_sum => n_ext_loop_vec_sum,
}
