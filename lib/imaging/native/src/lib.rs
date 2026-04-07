// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native PNG decoder using loft-ffi for direct store access.

#![allow(clippy::missing_safety_doc)]

use loft_ffi::{LoftRef, LoftStore};
use png::Decoder;
use std::fs::File;
use std::io::{BufReader, BufWriter};

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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_save_png(
    store: LoftStore,
    image: LoftRef,
    path_ptr: *const u8,
    path_len: usize,
) -> bool {
    let path = unsafe { loft_ffi::text(path_ptr, path_len) };
    let w = unsafe { store.get_int(image.rec, image.pos, image_fields::WIDTH) } as u32;
    let h = unsafe { store.get_int(image.rec, image.pos, image_fields::HEIGHT) } as u32;
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

loft_ffi::loft_register! {
    n_load_png,
    n_save_png,
}
