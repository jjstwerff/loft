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
    let img = store.claim((r.output_buffer_size() / 8) as u32 + 1);
    let info = r.next_frame(store.buffer(img))?;
    Ok((img, info.width, info.height))
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
            if store.get_byte(img, 8 + (x + y * w) * 6, 0) > 0 {
                //print!("x");
                count += 10;
            } else if store.get_byte(img, 9 + (x + y * w) * 6, 0) > 0 {
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
