// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native PNG decoder using loft-ffi for direct store access.

#![allow(clippy::missing_safety_doc)]

use loft_ffi::{LoftRef, LoftStore};
use png::Decoder;
use std::fs::File;
use std::io::BufReader;

/// Field offsets for the Image struct in the loft store.
mod image_fields {
    pub const NAME: u16 = 0;  // text (record ref)
    pub const WIDTH: u16 = 4; // integer
    pub const HEIGHT: u16 = 8; // integer
    pub const DATA: u16 = 12; // vector ref (Pixel elements, 3 bytes each)
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
        // Write Image struct fields.
        store.set_text(image.rec, image.pos, image_fields::NAME, name);
        store.set_int(image.rec, image.pos, image_fields::WIDTH, w as i32);
        store.set_int(image.rec, image.pos, image_fields::HEIGHT, h as i32);
        // Create pixel vector and bulk-copy RGB data (3 bytes per Pixel).
        let vec = store.alloc_vector_from_bytes(3, pixels.len() as u32 / 3, pixels.as_ptr(), pixels.len());
        store.set_int(image.rec, image.pos, image_fields::DATA, vec.rec as i32);
    }
    true
}
