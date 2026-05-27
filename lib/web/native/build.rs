// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN12 — drift-proof native registration from the library's `#native`
//! annotations (`../src/**/*.loft`).  Bare `#native` → `n_<fn>`;
//! `#native "sym"` → override.  `include!`d by `src/lib.rs`.

fn main() {
    // Plan-25 F4: web's `n_*` impls carry `#[loft_native]`, so also emit the
    // `loft_register_bridges!` list — the interpreter dispatches web through
    // the generated marshal bridges.
    loft_ffi_build::generate_register_from_loft_with_bridges("../src");
}
