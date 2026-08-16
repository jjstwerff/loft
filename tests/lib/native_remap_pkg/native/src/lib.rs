// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#907 fixture: a native crate that implements its `#native` symbols under
//! other Rust names, and exports a decoy under each `#native` name.
//!
//! The registration list is HAND-WRITTEN on purpose.  `loft-ffi-build`'s
//! generator derives `"S" => S__loft_bridge` from the `#native` annotations and
//! so cannot express a remap at all — which is why the shape only ever appears
//! in hand-maintained lists, and why this fixture cannot use the generator that
//! `native_scalar_pkg` (the clean-binding sibling) does.
//!
//! Each decoy is a plain `extern "C"` with NO `#[loft_native]`, so it registers
//! no bridge: a backend that resolves through the bridge table cannot reach it,
//! and a backend that binds the `#native` string literally reaches nothing else.
//! That is what makes the return value name the resolution path.

#![allow(clippy::missing_safety_doc)]

use loft_ffi::{LoftRef, LoftStore};
use loft_ffi_macros::loft_native;

/// The real implementation of loft's `remap_answer`.
#[loft_native]
#[no_mangle]
pub extern "C" fn n_remap_answer() -> i64 {
    7
}

/// Decoy under the `#native` name.  Same ABI as the real one, so calling it is
/// well-defined and the wrong answer is stable rather than luck.
#[no_mangle]
pub extern "C" fn loft_remap_answer() -> i64 {
    -1000
}

/// The real implementation of loft's `remap_sum`: a heap argument arrives as a
/// `(LoftStore, LoftRef)` pair, the same marshalling every store-aware native
/// gets from both backends.
#[loft_native]
#[no_mangle]
pub unsafe extern "C" fn n_remap_sum(store: LoftStore, data: LoftRef) -> i64 {
    if data.rec == 0 {
        return 0;
    }
    let count = unsafe { store.vector_len(&data) } as usize;
    if count == 0 {
        return 0;
    }
    let ptr = unsafe { store.vector_data_ptr(&data) }.cast::<i64>();
    unsafe { std::slice::from_raw_parts(ptr, count) }.iter().sum()
}

/// Decoy under the `#native` name, in the older raw `(ptr, count)` shape — the
/// real signature mismatch loft#907 was reported on.  It never dereferences
/// `_ptr`: a mis-resolved call arrives with the arguments shifted by a register,
/// so the pointer would be whatever landed there.  Answering a constant keeps
/// the fixture a test rather than a segfault.
#[no_mangle]
pub extern "C" fn loft_remap_sum(_ptr: *const i64, _count: u32) -> i64 {
    -2000
}

loft_ffi::loft_register! {
    loft_remap_answer => n_remap_answer,
    loft_remap_sum => n_remap_sum,
}

loft_ffi::loft_register_bridges! {
    "loft_remap_answer" => n_remap_answer__loft_bridge,
    "loft_remap_sum" => n_remap_sum__loft_bridge,
}
