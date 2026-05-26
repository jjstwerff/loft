// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN12 — drift-proof native registration from the library's `#native`
//! annotations (`../src/**/*.loft`).  Bare `#native` → `n_<fn>`;
//! `#native "sym"` → override.  `include!`d by `src/lib.rs`.

fn main() {
    loft_ffi_build::generate_register_from_loft("../src");
}
