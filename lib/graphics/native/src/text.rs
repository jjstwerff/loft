// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Font loading and text measurement using fontdue.

use fontdue::{Font, FontSettings};
use std::cell::RefCell;

thread_local! {
    static FONTS: RefCell<Vec<Font>> = const { RefCell::new(Vec::new()) };
}

/// Load a TTF/OTF font from a byte slice. Returns a font index (>= 0) or -1 on error.
pub fn load_font_bytes(data: &[u8]) -> i32 {
    let font = match Font::from_bytes(data, FontSettings::default()) {
        Ok(f) => f,
        Err(_) => return -1,
    };
    FONTS.with(|fonts| {
        let mut fonts = fonts.borrow_mut();
        let idx = fonts.len() as i32;
        fonts.push(font);
        idx
    })
}

/// Measure the width of a string at the given size. Returns width in pixels.
pub fn measure_text(font_idx: i32, text: &str, size: f32) -> f32 {
    FONTS.with(|fonts| {
        let fonts = fonts.borrow();
        let font = match fonts.get(font_idx as usize) {
            Some(f) => f,
            None => return 0.0,
        };
        text.chars()
            .map(|c| {
                let (metrics, _) = font.rasterize(c, size);
                metrics.advance_width
            })
            .sum()
    })
}

/// Rasterize a string into an alpha bitmap. Returns (width, height, pixels).
/// Each pixel is a single u8 alpha value.
pub fn rasterize_text(font_idx: i32, text: &str, size: f32) -> (u32, u32, Vec<u8>) {
    FONTS.with(|fonts| {
        let fonts = fonts.borrow();
        let font = match fonts.get(font_idx as usize) {
            Some(f) => f,
            None => return (0, 0, Vec::new()),
        };

        // First pass: measure total width and max height
        let mut glyphs: Vec<(fontdue::Metrics, Vec<u8>)> = Vec::new();
        let mut total_width = 0u32;
        let mut max_height = 0u32;
        let mut max_y_offset = 0i32;

        for c in text.chars() {
            let (metrics, bitmap) = font.rasterize(c, size);
            total_width += metrics.advance_width as u32;
            max_height = max_height.max(metrics.height as u32);
            max_y_offset = max_y_offset.max(metrics.ymin);
            glyphs.push((metrics, bitmap));
        }

        if total_width == 0 || max_height == 0 {
            return (0, 0, Vec::new());
        }

        let line_height = (size * 1.2) as u32;
        let mut pixels = vec![0u8; (total_width * line_height) as usize];
        let mut x_cursor = 0u32;

        for (metrics, bitmap) in &glyphs {
            let gw = metrics.width as u32;
            let gh = metrics.height as u32;
            let baseline = (size * 0.8) as i32;
            let y_off = (baseline - metrics.height as i32 - metrics.ymin).max(0) as u32;

            for gy in 0..gh {
                for gx in 0..gw {
                    let dst_x = x_cursor + gx;
                    let dst_y = y_off + gy;
                    if dst_x < total_width && dst_y < line_height {
                        let src = bitmap[(gy * gw + gx) as usize];
                        let dst_idx = (dst_y * total_width + dst_x) as usize;
                        if dst_idx < pixels.len() {
                            pixels[dst_idx] = src.max(pixels[dst_idx]);
                        }
                    }
                }
            }
            x_cursor += metrics.advance_width as u32;
        }

        (total_width, line_height, pixels)
    })
}
