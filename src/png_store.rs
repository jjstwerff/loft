// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Opening png images inside a store
#![allow(clippy::cast_possible_truncation)]
use crate::store::Store;
use png::Decoder;
use std::fs::File;
use std::io::BufReader;

pub fn read(file_path: &str, store: &mut Store) -> std::io::Result<(u32, u32, u32)> {
    let decoder = Decoder::new(BufReader::new(File::open(file_path)?));
    let mut reader = decoder.read_info()?;
    let img = store.claim((reader.output_buffer_size() / 8) as u32 + 1);
    let info = reader.next_frame(store.buffer(img))?;
    Ok((img, info.width, info.height))
}

#[test]
fn show_png() {
    let mut store = Store::new(12 + 256 * 256 * 3 / 8);
    let (img, _h, w) = read("tests/example/map.png", &mut store).unwrap();
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
