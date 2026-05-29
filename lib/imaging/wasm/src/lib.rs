// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @P321(c) / lib_plan-29 W1 — browser-WASM bridge for `lib/imaging`'s
//! store-mutating `#native` functions (`load_png`, `save_png`).
//!
//! The `--html` codegen path routes generated bodies for `n_load_png` /
//! `n_save_png` to the `pub fn` entry points in this crate via the
//! `[wasm.bridge].routes` table in `lib/imaging/loft.toml`.  The generated
//! standalone Rust binary links this crate as
//! `--extern loft_imaging_wasm=…` and calls the bridges directly — `--html`
//! has no `State` indirection at runtime, so the `replace_native`
//! mechanism (which only works on the interpreter wasm path) doesn't apply.
//!
//! **Phase split:** Phase W1 of `lib_plans/future/29-library-wasm-bridges`
//! extracts the bridge from `src/wasm_imaging.rs` to this crate so the
//! compiler crate stops naming library symbols.  Phase W2 will extract the
//! JS half (currently in `doc/loft-gl-wasm.js`) and the test-harness half
//! (currently in `tools/wasm_repro.mjs`).

#![allow(dead_code)] // exposed for codegen-emitted call sites

use loft::database::Stores;
use loft::keys::DbRef;

/// Field offsets in the loft `Image` struct.  Match
/// `lib/imaging/native/src/lib.rs::image_fields` exactly — loft reorders
/// struct fields to place 8-byte members first, so the source order
/// `(name, width, height, data)` lays out in the store as
/// `(width@0, height@8, name@16, data@20)`.
mod image_fields {
    pub const WIDTH: u32 = 0; // integer (i64)
    pub const HEIGHT: u32 = 8; // integer (i64)
    pub const NAME: u32 = 16; // text (record ref)
    pub const DATA: u32 = 20; // vector ref (Pixel — 3 bytes each)
}

// Host imports provided by `doc/loft-gl-wasm.js` for the `--html` build.
// Only the standalone-binary path (wasm32 without the `wasm` feature)
// declares externs — non-wasm targets compile but never call the bodies.
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "loft_gl")]
unsafe extern "C" {
    /// Look up `path`'s basename in `ctrl.assets`.  Writes width/height
    /// to `w_out`/`h_out` (u32 each).  Returns `1` on hit, `0` on miss.
    safe fn imaging_query(
        path_ptr: *const u8,
        path_len: usize,
        w_out: *mut u32,
        h_out: *mut u32,
    ) -> i32;

    /// Copy the decoded RGB bytes for `path`'s basename into
    /// `dest_ptr..dest_ptr+dest_len`.  Returns `1` on success, `0` on
    /// miss / size mismatch.
    safe fn imaging_copy_rgb(
        path_ptr: *const u8,
        path_len: usize,
        dest_ptr: *mut u8,
        dest_len: usize,
    ) -> i32;

    /// Encode the RGB bytes at `data_ptr..data_ptr+data_len` (3·w·h
    /// bytes) as a PNG and trigger a browser download named by the
    /// basename of `path`.  Returns `1` on success, `0` on failure.
    safe fn imaging_save(
        path_ptr: *const u8,
        path_len: usize,
        width: u32,
        height: u32,
        data_ptr: *const u8,
        data_len: usize,
    ) -> i32;
}

/// `lib/imaging::load_png(path: text, image: Image) -> boolean`
///
/// `--html` only.  On native + wasm-bindgen targets returns `false`
/// (the cdylib / interpreter dispatch handles those flavours).
pub fn imaging_load_png(stores: &mut Stores, path: &str, image: &DbRef) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let mut width: u32 = 0;
        let mut height: u32 = 0;
        if imaging_query(path.as_ptr(), path.len(), &mut width, &mut height) == 0 {
            return false;
        }
        if width == 0 || height == 0 {
            return false;
        }
        let count = match width.checked_mul(height) {
            Some(c) => c,
            None => return false,
        };
        let pixel_bytes = (count as usize).checked_mul(3).unwrap_or(0);
        if pixel_bytes == 0 {
            return false;
        }
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let store = stores.store_mut(image);
        let vec_rec = loft::vector::alloc_vector_from_bytes(store, 3, count, &[]);
        let payload = store.buffer(vec_rec);
        if payload.len() < pixel_bytes {
            return false;
        }
        let dest_ptr = payload.as_mut_ptr();
        if imaging_copy_rgb(path.as_ptr(), path.len(), dest_ptr, pixel_bytes) == 0 {
            return false;
        }
        let store = stores.store_mut(image);
        let str_rec = store.set_str(&name);
        store.set_u32_raw(image.rec, image.pos + image_fields::NAME, str_rec);
        store.set_long(image.rec, image.pos + image_fields::WIDTH, i64::from(width));
        store.set_long(image.rec, image.pos + image_fields::HEIGHT, i64::from(height));
        store.set_u32_raw(image.rec, image.pos + image_fields::DATA, vec_rec);
        return true;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (stores, path, image);
        false
    }
}

/// `lib/imaging::save_png(image: Image, path: text) -> boolean`
///
/// `--html` only — reads width/height/data from the Image struct,
/// hands the RGB bytes to JS which encodes a PNG (`canvas.toBlob`) and
/// triggers a browser download with `path`'s basename as filename.
pub fn imaging_save_png(stores: &mut Stores, image: &DbRef, path: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let store = stores.store(image);
        let width = store.get_long(image.rec, image.pos + image_fields::WIDTH);
        let height = store.get_long(image.rec, image.pos + image_fields::HEIGHT);
        if width <= 0 || height <= 0 {
            return false;
        }
        let count = match (width as u32).checked_mul(height as u32) {
            Some(c) => c,
            None => return false,
        };
        let data_rec = store.get_u32_raw(image.rec, image.pos + image_fields::DATA);
        if data_rec == 0 {
            return false;
        }
        let store = stores.store_mut(image);
        let payload = store.buffer(data_rec);
        let needed = match (count as usize).checked_mul(3) {
            Some(n) => n,
            None => return false,
        };
        if payload.len() < needed {
            return false;
        }
        let data_ptr = payload.as_ptr();
        imaging_save(
            path.as_ptr(),
            path.len(),
            width as u32,
            height as u32,
            data_ptr,
            needed,
        ) != 0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (stores, image, path);
        false
    }
}
