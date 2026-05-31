// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native PNG decoder using loft-ffi for direct store access.

#![allow(clippy::missing_safety_doc)]

use loft_ffi::{LoftRef, LoftStore};
use loft_ffi_macros::loft_native;
use png::Decoder;
use std::fs::File;
use std::io::{BufReader, BufWriter};

/// Field offsets for the Image struct in the loft store.
// loft reorders struct fields to place 8-byte members first for alignment, so
// the source order (name, width, height, data) lays out in the store as:
//   width  @ 0  (integer → i64, 8 bytes)
//   height @ 8  (integer → i64, 8 bytes)
//   name   @ 16 (text    → u32 record ref, 4 bytes)
//   data   @ 20 (vector  → u32 record ref, 4 bytes)
// Verified against the interpreter/native read offsets (`OpGetInt(img,0)`,
// `OpGetInt(img,8)`, `OpGetText(img,16)`, `OpGetField(img,20)`).  @P321c.
mod image_fields {
    pub const WIDTH: u16 = 0; // integer (i64)
    pub const HEIGHT: u16 = 8; // integer (i64)
    pub const NAME: u16 = 16; // text (record ref)
    pub const DATA: u16 = 20; // vector ref (Pixel elements, 3 bytes each)
}

fn decode_png(path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let file = File::open(path).ok()?;
    let decoder = Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let buf_size = reader.output_buffer_size();
    let mut pixels = vec![0u8; buf_size];
    let info = reader.next_frame(&mut pixels).ok()?;
    pixels.truncate(info.buffer_size());
    Some((info.width, info.height, pixels))
}

/// Decode a PNG file and write the result directly into an Image struct.
/// The Image fields (name, width, height, data) are written via LoftStore.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_load_png(
    mut store: LoftStore,
    path_ptr: *const u8,
    path_len: usize,
    image: LoftRef,
) -> bool {
    let path = unsafe { loft_ffi::text(path_ptr, path_len) };
    let (w, h, pixels) = match decode_png(path) {
        Some(data) => data,
        None => return false,
    };
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    unsafe {
        // Write Image struct fields.  width/height are plain `integer` →
        // 8-byte i64 (set_long); name/data are 4-byte record refs (set_int).
        store.set_text(image.rec, image.pos, image_fields::NAME, name);
        store.set_long(image.rec, image.pos, image_fields::WIDTH, i64::from(w));
        store.set_long(image.rec, image.pos, image_fields::HEIGHT, i64::from(h));
        // Create pixel vector and bulk-copy RGB data (3 bytes per Pixel).
        let vec = store.alloc_vector_from_bytes(3, pixels.len() as u32 / 3, pixels.as_ptr(), pixels.len());
        store.set_int(image.rec, image.pos, image_fields::DATA, vec.rec as i32);
    }
    true
}

fn encode_png(path: &str, width: u32, height: u32, rgb_data: &[u8]) -> bool {
    let file = match File::create(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = match encoder.write_header() {
        Ok(w) => w,
        Err(_) => return false,
    };
    writer.write_image_data(rgb_data).is_ok()
}

/// Encode an Image struct as a PNG file.
/// Reads width, height, and pixel data (3 bytes per Pixel: r, g, b) from the store.
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_save_png(
    store: LoftStore,
    image: LoftRef,
    path_ptr: *const u8,
    path_len: usize,
) -> bool {
    let path = unsafe { loft_ffi::text(path_ptr, path_len) };
    // width/height are plain `integer` → 8-byte i64 (get_long); data is a
    // 4-byte record ref (get_int).
    let w = unsafe { store.get_long(image.rec, image.pos, image_fields::WIDTH) } as u32;
    let h = unsafe { store.get_long(image.rec, image.pos, image_fields::HEIGHT) } as u32;
    let data_rec = unsafe { store.get_int(image.rec, image.pos, image_fields::DATA) } as u32;
    if w == 0 || h == 0 || data_rec == 0 {
        return false;
    }
    let data_ref = LoftRef { store_nr: image.store_nr, rec: data_rec, pos: 0 };
    let count = unsafe { store.vector_len(&data_ref) };
    let expected = w * h;
    if count < expected {
        return false;
    }
    // Each Pixel is 3 bytes (r, g, b) stored contiguously in the vector.
    let ptr = unsafe { store.vector_data_ptr(&data_ref) };
    let rgb_data = unsafe { std::slice::from_raw_parts(ptr, (expected * 3) as usize) };
    encode_png(path, w, h, rgb_data)
}

// @PLAN12 phase 2 — the `loft_ffi::loft_register! { … }` symbol list is
// generated from `../loft.toml::[native.functions]` by `build.rs` (via
// `loft-ffi-build`) and `include!`d here, so the cdylib's exported symbol
// set is never hand-maintained.
include!(concat!(env!("OUT_DIR"), "/loft_register_gen.rs"));
