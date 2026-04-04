// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! PNG decoder for the imaging package.
//! No loft dependency — only depends on the `png` crate.
//! Exports a C-ABI function that decodes a PNG file into raw RGB bytes.

use png::Decoder;
use std::fs::File;
use std::io::BufReader;

/// Decode a PNG file at `path` into raw RGB pixel bytes.
/// Returns `(width, height, pixels)` or `None` on failure.
pub fn decode_png(path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let file = File::open(path).ok()?;
    let decoder = Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let buf_size = reader.output_buffer_size();
    let mut pixels = vec![0u8; buf_size];
    let info = reader.next_frame(&mut pixels).ok()?;
    pixels.truncate(info.buffer_size());
    Some((info.width, info.height, pixels))
}

/// C-ABI entry point for the interpreter's dlopen path.
/// Decodes a PNG and returns the pixel data via out-pointers.
/// The caller must free the returned pixel buffer with `loft_free_pixels`.
///
/// # Safety
/// All pointers must be valid and non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_decode_png(
    path_ptr: *const u8,
    path_len: usize,
    out_width: *mut u32,
    out_height: *mut u32,
    out_pixels: *mut *mut u8,
    out_pixels_len: *mut usize,
) -> bool {
    let path = match std::str::from_utf8(unsafe { std::slice::from_raw_parts(path_ptr, path_len) })
    {
        Ok(s) => s,
        Err(_) => return false,
    };
    match decode_png(path) {
        Some((w, h, mut pixels)) => {
            unsafe {
                *out_width = w;
                *out_height = h;
                *out_pixels_len = pixels.len();
                *out_pixels = pixels.as_mut_ptr();
            }
            std::mem::forget(pixels); // caller owns the buffer
            true
        }
        None => false,
    }
}

/// Free a pixel buffer allocated by `loft_decode_png`.
///
/// # Safety
/// `ptr` and `len` must match a previous `loft_decode_png` call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_free_pixels(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, len, len) });
    }
}
