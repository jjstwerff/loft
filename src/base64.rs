// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! Built-in base64 encoder kept in the compiler crate for one
//! compiler-internal use: `main.rs::wasm_b64` base64-encodes the
//! embedded WASM bytes in `--html` export.  The crypto LIBRARY's
//! base64 lives in `lib/crypto/native/src/base64.rs` (its own copy
//! of this file — pure-Rust, no external deps, kept in sync until
//! the compiler stops needing base64 internally).  Decode + URL-safe
//! encode are unused inside the compiler but stay here so the file
//! mirrors the lib copy.

#![allow(dead_code)]

const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[must_use]
pub fn encode(data: &[u8]) -> String {
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[must_use]
pub fn encode_url(data: &[u8]) -> String {
    encode(data)
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_string()
}

#[must_use]
pub fn decode(input: &str) -> Vec<u8> {
    fn val(c: u8) -> u8 {
        match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => 0,
        }
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| *b != b'=' && *b != b'\n')
        .collect();
    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let n = u32::from(val(chunk[0])) << 18
            | u32::from(val(chunk[1])) << 12
            | if chunk.len() > 2 {
                u32::from(val(chunk[2])) << 6
            } else {
                0
            }
            | if chunk.len() > 3 {
                u32::from(val(chunk[3]))
            } else {
                0
            };
        result.push((n >> 16) as u8);
        if chunk.len() > 2 {
            result.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            result.push(n as u8);
        }
    }
    result
}
