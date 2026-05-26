// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN12 — drift-proof native registration.  `loft-ffi-build` scans the
//! library's loft source (`../src/**/*.loft`) for `#native` bindings and
//! generates the `loft_register! { … }` list — the SAME co-located
//! annotations the compiler binds against, so the register list can't drift.
//! Bare `#native` → `n_<fn>`; `#native "sym"` → the override.  `include!`d
//! by `src/lib.rs`.

fn main() {
    loft_ffi_build::generate_register_from_loft("../src");
}
