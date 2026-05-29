// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @P321c Phase 2 — wasm-side bridge for `lib/imaging`'s store-mutating
//! `#native` functions (`load_png`, `save_png`).
//!
//! `--html` codegen routes generated bodies for `n_load_png` / `n_save_png`
//! to the `pub fn` entry points here (via the `WASM_BRIDGE_FNS` table at
//! `src/generation/mod.rs`).  The bodies call `host_call_raw(...)` to
//! reach JS.
//!
//! Unlike `src/wasm_gl.rs`'s `wgl_*` fns (which are registered via
//! `state.replace_native` for the INTERPRETER wasm path), these are
//! plain `pub fn`s called directly from the generated standalone-binary
//! Rust — `--html` has no `State` indirection at runtime, so the
//! `replace_native` mechanism doesn't apply.
//!
//! Phase 2 (this file): JS bridge wired up but JS body is a stub that
//! returns 0.  Bridges return `false`, leaving loft semantics as
//! "load_png failed → png() returns null" — same as a missing file on
//! native.
//!
//! Phase 3 (planned): JS stub becomes the real decode (createImageBitmap +
//! Canvas + getImageData, via asset preload).

#![allow(dead_code)] // exposed for codegen-emitted call sites

use crate::database::Stores;
use crate::keys::DbRef;

/// `lib/imaging::load_png(path: text, image: Image) -> boolean`
///
/// Reads a pre-loaded PNG from the JS-side asset table (`ctrl.assets[name]`)
/// and writes width/height/name + an allocated pixel vector into the
/// caller-provided `Image` struct.  Returns `true` on success.
///
/// Field offsets in `Image` (per `lib/imaging/native/src/lib.rs:23-28`):
/// - `width: integer` at offset 0 (8 bytes i64)
/// - `height: integer` at offset 8 (8 bytes i64)
/// - `name: text` at offset 16 (4 bytes text-record handle)
/// - `data: vector<Pixel>` at offset 20 (4 bytes vector handle)
///
/// Phase 2: JS bridge stub returns 0 — this fn returns `false`.
pub fn imaging_load_png(stores: &mut Stores, path: &str, image: &DbRef) -> bool {
    #[cfg(feature = "wasm")]
    {
        let _ = stores;
        let args = js_sys::Array::new();
        args.push(&path.into());
        args.push(&i64::from(image.store_nr).into());
        args.push(&i64::from(image.rec).into());
        args.push(&i64::from(image.pos).into());
        let result = crate::wasm::host_call_raw("imaging_load_png", &args);
        result.as_f64().unwrap_or(0.0) != 0.0
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (stores, path, image);
        false
    }
}

/// `lib/imaging::save_png(image: Image, path: text) -> boolean`
///
/// Reads width/height/data from the `Image` struct and packages the pixel
/// bytes for JS-side encoding (Canvas `toBlob` → download).  Phase 2:
/// stub returns `false`.
pub fn imaging_save_png(stores: &mut Stores, image: &DbRef, path: &str) -> bool {
    #[cfg(feature = "wasm")]
    {
        let _ = stores;
        let args = js_sys::Array::new();
        args.push(&i64::from(image.store_nr).into());
        args.push(&i64::from(image.rec).into());
        args.push(&i64::from(image.pos).into());
        args.push(&path.into());
        let result = crate::wasm::host_call_raw("imaging_save_png", &args);
        result.as_f64().unwrap_or(0.0) != 0.0
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (stores, image, path);
        false
    }
}
