// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native extension for the imaging package. No loft dependency.

use png::Decoder;
use std::fs::File;
use std::io::BufReader;

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

// ── C-ABI exports ───────────────────────────────────────────────────────

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
            std::mem::forget(pixels);
            true
        }
        None => false,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_free_pixels(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, len, len) });
    }
}

// ── Registration ────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_register_v1(
    register: unsafe extern "C" fn(*const u8, usize, *const (), *mut ()),
    ctx: *mut (),
) {
    unsafe {
        register(b"loft_decode_png".as_ptr(), 15, loft_decode_png as *const (), ctx);
        register(b"loft_free_pixels".as_ptr(), 16, loft_free_pixels as *const (), ctx);
    }
}
