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
        // @P339: include kerning pairs (e.g. A–V, T–o) so the measured width
        // matches the rasterized layout — otherwise `measure("AV")` ==
        // `measure("A")+measure("V")` exactly and kerned text overflows /
        // mis-centres.  Must mirror the same kern walk in `rasterize_text`.
        let mut width = 0.0f32;
        let mut prev: Option<char> = None;
        for c in text.chars() {
            if let Some(p) = prev {
                width += font.horizontal_kern(p, c, size).unwrap_or(0.0);
            }
            let (metrics, _) = font.rasterize(c, size);
            width += metrics.advance_width;
            prev = Some(c);
        }
        width
    })
}

/// Return the font's ASCENT in pixels at the given size — the distance from
/// the baseline up to the top of the glyphs, taken from fontdue's real
/// horizontal line metrics (`font.horizontal_line_metrics`).  @P340: callers
/// align mixed-size text on a common baseline by offsetting their draw-y by
/// the ascent (the top-anchored `draw_text` puts the baseline at
/// `y + ascent`).  Falls back to the legacy `size * 0.8` approximation when a
/// font exposes no horizontal metrics (rare; bitmap-only fonts).
pub fn font_ascent(font_idx: i32, size: f32) -> f32 {
    FONTS.with(|fonts| {
        let fonts = fonts.borrow();
        let Some(font) = fonts.get(font_idx as usize) else {
            return 0.0;
        };
        font.horizontal_line_metrics(size)
            .map_or(size * 0.8, |lm| lm.ascent)
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

        // First pass: measure total width and max height.  @P339: each glyph
        // carries the kern adjustment (in px) to apply BEFORE it, vs the
        // previous char — mirrors the kern walk in `measure_text`.  A float
        // pen accumulates advances + kerns; the bitmap width is its ceiling.
        let mut glyphs: Vec<(fontdue::Metrics, Vec<u8>, f32)> = Vec::new();
        let mut pen = 0.0f32;
        let mut max_height = 0u32;
        let mut max_y_offset = 0i32;
        let mut prev: Option<char> = None;

        for c in text.chars() {
            let kern = prev
                .and_then(|p| font.horizontal_kern(p, c, size))
                .unwrap_or(0.0);
            pen += kern;
            let (metrics, bitmap) = font.rasterize(c, size);
            pen += metrics.advance_width;
            max_height = max_height.max(metrics.height as u32);
            max_y_offset = max_y_offset.max(metrics.ymin);
            glyphs.push((metrics, bitmap, kern));
            prev = Some(c);
        }

        let total_width = pen.ceil().max(0.0) as u32;
        if total_width == 0 || max_height == 0 {
            return (0, 0, Vec::new());
        }

        let line_height = (size * 1.2) as u32;
        let mut pixels = vec![0u8; (total_width * line_height) as usize];
        let mut x_pen = 0.0f32;

        for (metrics, bitmap, kern) in &glyphs {
            x_pen += *kern;
            let x_cursor = x_pen.max(0.0) as u32;
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
            x_pen += metrics.advance_width;
        }

        (total_width, line_height, pixels)
    })
}
