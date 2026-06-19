// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Test fixture for native extension loading and marshalling.
//!
//! Tests all common patterns: scalar, vector, struct ref, const borrowing.
//! Every function validates its inputs and returns a checkable result.
//!
//! Plan-74: each `n_ext_*` impl carries `#[loft_native]`, so the interpreter
//! dispatches it through the generated uniform marshal bridge
//! (`<fn>__loft_bridge`), not the deleted legacy `dispatch_call` arms.  The
//! loft decls bind the `#native "loft_ext_*"` symbols, so the registration and
//! bridge tables key the bridges under those `loft_ext_*` names (pointing at the
//! `n_ext_*__loft_bridge` siblings).  Vector args arrive as a `LoftRef` to the
//! vector data record, read here via `LoftStore::vector_len`/`vector_data_ptr`.

#![allow(clippy::missing_safety_doc)]

use loft_ffi::{LoftRef, LoftStore};
use loft_ffi_macros::loft_native;

/// Borrow a vector argument's elements as a `&[T]` for the call's duration.
/// Empty when the ref is null or the vector is empty.
#[inline]
unsafe fn vec_slice<'a, T>(store: &LoftStore, vec: &LoftRef) -> &'a [T] {
    if vec.rec == 0 {
        return &[];
    }
    let count = unsafe { store.vector_len(vec) } as usize;
    if count == 0 {
        return &[];
    }
    let ptr = unsafe { store.vector_data_ptr(vec) }.cast::<T>();
    unsafe { std::slice::from_raw_parts(ptr, count) }
}

// ── Pattern 1: Scalar → Scalar (baseline) ────────────────────────────

#[loft_native]
#[unsafe(no_mangle)]
pub extern "C" fn n_ext_add_one(x: i64) -> i64 {
    x + 1
}

/// dlsym bait — returns a different value so tests detect wrong dispatch
/// (A7.2.4 registry-vs-dlsym priority).  No bridge: reached only via the raw
/// `loft_register_v1` / dlsym path under test.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_add_one(x: i64) -> i64 {
    x + 1000
}

/// Unregistered — for the guard test (A7.2.5): exported as a raw C-ABI symbol
/// but never registered, so a `#native` decl naming it must panic at wire time.
#[unsafe(no_mangle)]
pub extern "C" fn loft_ext_unregistered(x: i64) -> i64 {
    x + 9999
}

// ── Pattern 2: Vector<integer> as input ──────────────────────────────
// Post-2c: loft `integer` is i64 at rest, so the elements are i64.

/// Sum all elements of a vector<integer>.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_vec_sum(store: LoftStore, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    if data.is_empty() {
        return -1;
    }
    eprintln!("[ext_vec_sum] count={} first={} last={}", data.len(), data[0], data[data.len() - 1]);
    data.iter().sum::<i64>()
}

// ── Pattern 3: Vector<single> as input (single-precision) ────────────

/// Sum of vector<single> elements.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_vec_sum_f32(store: LoftStore, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<f32>(&store, &vec) };
    if data.is_empty() {
        return -1;
    }
    eprintln!("[ext_vec_sum_f32] count={} first={} last={}", data.len(), data[0], data[data.len() - 1]);
    data.iter().sum::<f32>() as i64
}

// ── Pattern 4: Scalars before vector ─────────────────────────────────

/// offset + sum(data).
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_offset_sum(store: LoftStore, offset: i64, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    if data.is_empty() {
        return offset;
    }
    eprintln!("[ext_offset_sum] offset={} count={}", offset, data.len());
    offset + data.iter().sum::<i64>()
}

// ── Pattern 5: Vector between scalars ────────────────────────────────

/// a + sum(data) + b.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_sandwich_sum(store: LoftStore, a: i64, vec: LoftRef, b: i64) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    if data.is_empty() {
        return a + b;
    }
    eprintln!("[ext_sandwich_sum] a={} count={} b={}", a, data.len(), b);
    a + data.iter().sum::<i64>() + b
}

// ── Pattern 6: Vector from struct field ──────────────────────────────

/// Read a vector<integer> that comes from a struct field access (e.g. canvas.data).
/// This tests the indirect reference path where the DbRef on the stack
/// points to a struct field containing the vector record number.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_struct_vec_len(store: LoftStore, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    eprintln!("[ext_struct_vec_len] count={}", data.len());
    data.len() as i64
}

// ── Pattern 7: Vector inside loop with if ────────────────────────────

/// Same as vec_sum but with logging to detect store issues in loops.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ext_loop_vec_sum(store: LoftStore, vec: LoftRef) -> i64 {
    let data = unsafe { vec_slice::<i64>(&store, &vec) };
    if data.is_empty() {
        eprintln!("[ext_loop_vec_sum] EMPTY");
        return -1;
    }
    let sum: i64 = data.iter().sum();
    eprintln!("[ext_loop_vec_sum] count={} sum={}", data.len(), sum);
    sum
}

// ── Registration ─────────────────────────────────────────────────────
// The loft decls bind `#native "loft_ext_*"`, so register the raw `n_ext_*`
// symbols under those names (`loft_register_v1`, for the dlsym-priority and
// guard tests) AND register the generated bridges under the same names
// (`loft_register_bridges_v1`) — the interpreter prefers the bridge.

loft_ffi::loft_register! {
    loft_ext_add_one => n_ext_add_one,
    loft_ext_vec_sum => n_ext_vec_sum,
    loft_ext_vec_sum_f32 => n_ext_vec_sum_f32,
    loft_ext_offset_sum => n_ext_offset_sum,
    loft_ext_sandwich_sum => n_ext_sandwich_sum,
    loft_ext_struct_vec_len => n_ext_struct_vec_len,
    loft_ext_loop_vec_sum => n_ext_loop_vec_sum,
}

loft_ffi::loft_register_bridges! {
    "loft_ext_add_one" => n_ext_add_one__loft_bridge,
    "loft_ext_vec_sum" => n_ext_vec_sum__loft_bridge,
    "loft_ext_vec_sum_f32" => n_ext_vec_sum_f32__loft_bridge,
    "loft_ext_offset_sum" => n_ext_offset_sum__loft_bridge,
    "loft_ext_sandwich_sum" => n_ext_sandwich_sum__loft_bridge,
    "loft_ext_struct_vec_len" => n_ext_struct_vec_len__loft_bridge,
    "loft_ext_loop_vec_sum" => n_ext_loop_vec_sum__loft_bridge,
}
