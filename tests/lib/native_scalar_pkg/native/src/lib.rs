// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Minimal native crate for the `native_scalar_pkg` fixture (@PLN26).
//!
//! Exports one scalar C-ABI symbol, `n_native_answer`, bound to the loft
//! `native_answer()` wrapper.  A no-arg integer return marshals as a plain
//! `extern "C" fn() -> i32`, so this crate needs no loft-ffi — it exists only
//! to give the C-ABI exec path (and the Windows import-library link + DLL
//! staging, @PLN26 phase 4) a real, cheap-to-build native_crate package to link.
#[no_mangle]
pub extern "C" fn n_native_answer() -> i32 {
    42
}
