// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN12 — drift-proof native registration.  `loft-ffi-build` scans
//! this crate's own `src/**/*.rs` for `extern "C" fn n_*` definitions
//! and generates the `loft_register! { … }` list, so defining a
//! function IS registering it — no hand-maintained symbol list, no
//! `[native.functions]` manifest table to fall out of sync.  The
//! generated file is `include!`d at module scope by `src/lib.rs`.

fn main() {
    loft_ffi_build::generate_register_from_sources("src");
}
