// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Drift-proof native registration.  `loft-ffi-build` scans the library's loft
//! source (`../src/**/*.loft`) for bare `#native` annotations and generates the
//! register list — the SAME co-located annotations the compiler binds against,
//! so the register list cannot drift.  Bare `#native` → `n_<fn>`.  `include!`d
//! by `src/lib.rs`.
//!
//! Plan-74: the `n_*` impl carries `#[loft_native]`, so this emits the
//! `loft_register_bridges!` list too — the interpreter dispatches through the
//! generated uniform marshal bridge, not the deleted legacy arm-set.

fn main() {
    loft_ffi_build::generate_register_from_loft_with_bridges("../src");
}
