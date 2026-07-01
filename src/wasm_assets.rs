// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F54 — Browser / WASM target (--html / --native-wasm)

//! @lib_plan-29 W1d — generic browser-WASM asset table accessor.
//!
//! Holds ONLY library-agnostic helpers that any wasm-asset consumer
//! might need (today: `asset_exists`, called by
//! `database::io::get_file` to report PNG assets as `TextFile` so
//! `file().png()` reaches the imaging bridge instead of short-
//! circuiting to `null` on the browser's absent filesystem).
//!
//! Library-specific bridges (imaging's `load_png` / `save_png`) live
//! in their package's own `wasm/` crate (e.g.
//! `lib/imaging/wasm/src/lib.rs`); the compiler reaches them via the
//! `[wasm.bridge]` section in each `loft.toml`, not via this module.
//!
//! On non-wasm targets every fn here is a no-op stub (callers compile
//! but never reach the wasm path).

#![allow(dead_code)] // exposed for cross-target compilation

// Host imports provided by `doc/loft-gl-wasm.js` for the `--html` build.
// Only the standalone-binary path (wasm32 without the `wasm` feature)
// declares externs — wasm-bindgen and native paths return `false` from
// the public helper below.
#[cfg(all(target_arch = "wasm32", not(feature = "wasm")))]
#[link(wasm_import_module = "loft_gl")]
unsafe extern "C" {
    /// `1` iff `ctrl.assets` has an entry under `path`'s basename.
    safe fn host_asset_exists(path_ptr: *const u8, path_len: usize) -> i32;
}

/// Returns `true` when the browser-side asset table has an entry
/// keyed by `path`'s basename.  Library-agnostic: every wasm-asset
/// consumer (today imaging, tomorrow audio / fonts / etc.) shares
/// the same `ctrl.assets[basename]` lookup.
pub fn asset_exists(path: &str) -> bool {
    #[cfg(all(target_arch = "wasm32", not(feature = "wasm")))]
    {
        host_asset_exists(path.as_ptr(), path.len()) != 0
    }
    #[cfg(not(all(target_arch = "wasm32", not(feature = "wasm"))))]
    {
        let _ = path;
        false
    }
}
