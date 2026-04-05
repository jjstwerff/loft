// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! WebSocket frame parser and writer (RFC 6455).
//! No external dependencies — pure std.

use std::io::{Read, Write};
use std::net::TcpStream;

/// WebSocket opcodes.
pub const OP_TEXT: u8 = 0x01;
pub const OP_BINARY: u8 = 0x02;
pub const OP_CLOSE: u8 = 0x08;
pub const OP_PING: u8 = 0x09;
pub const OP_PONG: u8 = 0x0A;

/// A decoded WebSocket frame.
pub struct WsFrame {
    pub opcode: u8,
    pub payload: Vec<u8>,
}

/// Compute the SHA-1 hash (for the WebSocket accept key).
/// Minimal implementation — only used for the 20-byte handshake hash.
fn sha1(data: &[u8]) -> [u8; 20] {
    // SHA-1 constants
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..80 {
            w[i] = (w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(w[i]);
            e = d; d = c; c = b.rotate_left(30); b = a; a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut result = [0u8; 20];
    result[0..4].copy_from_slice(&h0.to_be_bytes());
    result[4..8].copy_from_slice(&h1.to_be_bytes());
    result[8..12].copy_from_slice(&h2.to_be_bytes());
    result[12..16].copy_from_slice(&h3.to_be_bytes());
    result[16..20].copy_from_slice(&h4.to_be_bytes());
    result
}

/// Base64 encode bytes.
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 { result.push(CHARS[((n >> 6) & 63) as usize] as char); } else { result.push('='); }
        if chunk.len() > 2 { result.push(CHARS[(n & 63) as usize] as char); } else { result.push('='); }
    }
    result
}

/// Compute the WebSocket accept key from the client's Sec-WebSocket-Key.
pub fn ws_accept_key(client_key: &str) -> String {
    let mut input = client_key.trim().to_string();
    input.push_str("258EAFA5-E914-47DA-95CA-5AB5DC11D68B");
    let hash = sha1(input.as_bytes());
    base64_encode(&hash)
}

/// Perform the WebSocket upgrade handshake on an already-accepted TCP stream.
/// Returns true if the upgrade succeeded.
pub fn ws_upgrade(stream: &mut TcpStream, headers: &str) -> bool {
    // Find Sec-WebSocket-Key in headers
    let key = headers.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        if k.trim().eq_ignore_ascii_case("sec-websocket-key") {
            Some(v.trim().to_string())
        } else {
            None
        }
    });
    let key = match key {
        Some(k) => k,
        None => return false,
    };
    let accept = ws_accept_key(&key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).is_ok()
}

/// Read one WebSocket frame from the stream.
pub fn ws_read_frame(stream: &mut TcpStream) -> Option<WsFrame> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).ok()?;
    let opcode = header[0] & 0x0F;
    let masked = (header[1] & 0x80) != 0;
    let mut payload_len = (header[1] & 0x7F) as u64;

    if payload_len == 126 {
        let mut buf = [0u8; 2];
        stream.read_exact(&mut buf).ok()?;
        payload_len = u16::from_be_bytes(buf) as u64;
    } else if payload_len == 127 {
        let mut buf = [0u8; 8];
        stream.read_exact(&mut buf).ok()?;
        payload_len = u64::from_be_bytes(buf);
    }

    let mask = if masked {
        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).ok()?;
        Some(buf)
    } else {
        None
    };

    let mut payload = vec![0u8; payload_len as usize];
    stream.read_exact(&mut payload).ok()?;

    if let Some(mask) = mask {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i % 4];
        }
    }

    Some(WsFrame { opcode, payload })
}

/// Write a WebSocket frame to the stream (server → client, unmasked).
pub fn ws_write_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> bool {
    let mut frame = Vec::new();
    frame.push(0x80 | opcode); // FIN + opcode

    let len = payload.len();
    if len < 126 {
        frame.push(len as u8);
    } else if len < 65536 {
        frame.push(126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    frame.extend_from_slice(payload);

    stream.write_all(&frame).is_ok()
}
