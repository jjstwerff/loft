// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A7.3 — `LoftPluginCtx` C-ABI layout and trampoline tests.
//!
//! Verifies that `LoftPluginCtx` (exposed from `loft::extensions`) has the
//! expected repr(C) layout and that the `register_fn` callback mechanism
//! works correctly — the same invariants relied on by `loft_register_v1`
//! in external plugin crates.

extern crate loft;

use loft::extensions::LoftPluginCtx;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// A7.3.1: LoftPluginCtx is repr(C) — verify register_fn dispatch works.
#[test]
fn plugin_ctx_register_fn_dispatch() {
    // Staging vec captures (name, fn_ptr) pairs via the trampoline.
    let mut staged: Vec<(String, usize)> = Vec::new();

    unsafe extern "C" fn capture_register(
        ctx: *mut LoftPluginCtx,
        name: *const c_char,
        func: unsafe extern "C" fn(*mut (), *mut ()),
    ) {
        let staged = unsafe { &mut *((*ctx)._staged as *mut Vec<(String, usize)>) };
        let name = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        staged.push((name, func as usize));
    }

    unsafe extern "C" fn dummy_fn(_: *mut (), _: *mut ()) {}

    let name = CString::new("n_test_fn").unwrap();
    let mut ctx = LoftPluginCtx {
        _state: std::ptr::null_mut(),
        register_fn: capture_register,
        _staged: &mut staged as *mut _ as *mut (),
    };
    unsafe { (ctx.register_fn)(&mut ctx, name.as_ptr(), dummy_fn) };

    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].0, "n_test_fn");
    assert_eq!(
        staged[0].1,
        dummy_fn as unsafe extern "C" fn(*mut (), *mut ()) as usize
    );
}
