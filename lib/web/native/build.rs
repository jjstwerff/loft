// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN12 phase 2 Option A (2026-05-24) — delegates to the shared
//! `loft-ffi-build` crate so the TOML scanner + register-invocation
//! emitter live in exactly one place across all libraries.
//!
//! Generated file: `$OUT_DIR/loft_register_gen.rs` — `include!`d at
//! module scope by `src/lib.rs`.  Adding a new symbol is a single
//! row in `../loft.toml::[native.functions]`.

fn main() {
    loft_ffi_build::generate_register_invocation("../loft.toml");
}
