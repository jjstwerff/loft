// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Opening png images inside a store
#![allow(clippy::cast_possible_truncation)]
use crate::store::Store;
use png::Decoder;
#[cfg(not(feature = "wasm"))]
use std::fs::File;
#[cfg(not(feature = "wasm"))]
use std::io::BufReader;

/// Decode a PNG from any `Read` source into the store.
/// Returns `(img_record, width, height)`.
fn decode_into_store<R: std::io::Read>(
    reader: R,
    store: &mut Store,
) -> std::io::Result<(u32, u32, u32)> {
    let decoder = Decoder::new(reader);
    let mut r = decoder.read_info()?;
    let buf_size = r.output_buffer_size();
    // Allocate with 8-byte vector header: [next:4][length:4][pixel data...]
    let img = store.claim((buf_size / 8) as u32 + 2);
    let pixel_count = buf_size / 3; // 3 bytes per Pixel (r, g, b as u8)
    #[allow(clippy::cast_possible_wrap)]
    store.set_int(img, 4, pixel_count as i32);
    // Decode PNG directly into offset 8 (after the vector header).
    let buf = store.buffer(img);
    let header_bytes = 8;
    if buf.len() > header_bytes {
        let info = r.next_frame(&mut buf[header_bytes..])?;
        Ok((img, info.width, info.height))
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "PNG buffer too small",
        ))
    }
}

/// Read a PNG from the filesystem (native path).
#[cfg(not(feature = "wasm"))]
pub fn read(file_path: &str, store: &mut Store) -> std::io::Result<(u32, u32, u32)> {
    decode_into_store(BufReader::new(File::open(file_path)?), store)
}

/// Read a PNG via the WASM host bridge (wasm path).
#[cfg(feature = "wasm")]
pub fn read(file_path: &str, store: &mut Store) -> std::io::Result<(u32, u32, u32)> {
    let bytes = crate::wasm::host_fs_read_binary(file_path).unwrap_or_default();
    decode_into_store(std::io::Cursor::new(bytes), store)
}

#[test]
fn show_png() {
    let mut store = Store::new(12 + 256 * 256 * 3 / 8);
    let (img, w, _h) = read("tests/example/map.png", &mut store).unwrap();
    let mut count = 0;
    for y in 0..128 {
        if y % 2 == 0 {
            continue;
        }
        for x in 0..128 {
            // Vector header is 8 bytes = 16 store units; each pixel is 3 bytes = 6 units.
            if store.get_byte(img, 16 + (x + y * w) * 6, 0) > 0 {
                //print!("x");
                count += 10;
            } else if store.get_byte(img, 17 + (x + y * w) * 6, 0) > 0 {
                //print!("b");
                count += 11;
            } else {
                //print!(".");
                count += 1;
            }
        }
        //println!();
    }
    assert_eq!(34972, count);
}
