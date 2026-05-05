// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! WebSocket client (RFC 6455).  Native build: std::net + manual framing.
//! Browser/wasm build: imports the same operations from the host JS bridge
//! (see tests/wasm/host.mjs / doc/loft-rt.js `loftHost.ws_*`).
//!
//! This is the symmetric counterpart of `lib/server/native/src/websocket.rs`:
//! the server reads masked frames and writes unmasked frames; the client
//! reads unmasked frames and writes masked frames.  Handshake initiation
//! lives here; handshake response lives there.

#[cfg(not(target_arch = "wasm32"))]
mod native_impl {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    /// One WebSocket "session".  Owns the (possibly absent) live TCP
    /// stream, the URL needed to reconnect, the last reconnect attempt
    /// time, and a queue of pending received messages.  The session
    /// stays alive across reconnect attempts; only `close()` destroys it.
    struct Conn {
        url: String,
        stream: Option<TcpStream>,
        inbox: VecDeque<String>,
        last_attempt: Instant,
        backoff_ms: u64,
    }

    /// Backoff schedule (milliseconds).  After each failure the index
    /// advances; on success it resets to 0.  Max throttle ~10s, so we
    /// never spam the server harder than once every 10 seconds in the
    /// worst case.
    const BACKOFF_MS: &[u64] = &[100, 250, 500, 1_000, 2_500, 5_000, 10_000];

    thread_local! {
        static CONNS: RefCell<Vec<Option<Conn>>> = const { RefCell::new(Vec::new()) };
        static LAST_MSG: RefCell<String> = const { RefCell::new(String::new()) };
    }

    // ── SHA-1, base64, mask key (small + dependency-free) ──────────────

    fn sha1(data: &[u8]) -> [u8; 20] {
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
                w[i] = u32::from_be_bytes([
                    chunk[i * 4],
                    chunk[i * 4 + 1],
                    chunk[i * 4 + 2],
                    chunk[i * 4 + 3],
                ]);
            }
            for i in 16..80 {
                w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
            }
            let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
            for i in 0..80 {
                let (f, k) = match i {
                    0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                    20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                    40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                    _ => (b ^ c ^ d, 0xCA62C1D6u32),
                };
                let temp = a
                    .rotate_left(5)
                    .wrapping_add(f)
                    .wrapping_add(e)
                    .wrapping_add(k)
                    .wrapping_add(w[i]);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = temp;
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

    fn base64(data: &[u8]) -> String {
        const CHARS: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(CHARS[((n >> 18) & 63) as usize] as char);
            out.push(CHARS[((n >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                out.push(CHARS[((n >> 6) & 63) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(CHARS[(n & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    fn pseudo_random_bytes(n: usize) -> Vec<u8> {
        // Deterministic-shape but entropy-mixed PRNG; sufficient for the
        // 16-byte handshake nonce and the 4-byte frame mask.  Not crypto.
        use std::time::{SystemTime, UNIX_EPOCH};
        let mut seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xDEADBEEF);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            out.push((seed >> 33) as u8);
        }
        out
    }

    // ── URL parser: ws://host:port/path  (no TLS, no auth) ─────────────

    fn parse_ws_url(url: &str) -> Option<(String, u16, String)> {
        let rest = url.strip_prefix("ws://")?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()?),
            None => (authority.to_string(), 80u16),
        };
        Some((host, port, path.to_string()))
    }

    // ── Handshake (client side) ────────────────────────────────────────

    fn do_handshake(stream: &mut TcpStream, host: &str, port: u16, path: &str) -> bool {
        let nonce = base64(&pseudo_random_bytes(16));
        let req = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host}:{port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {nonce}\r\n\
             Sec-WebSocket-Version: 13\r\n\
             \r\n"
        );
        if stream.write_all(req.as_bytes()).is_err() {
            return false;
        }
        // Read until we see CRLF CRLF.
        let mut reader = BufReader::new(stream.try_clone().ok().unwrap_or_else(|| stream.try_clone().unwrap()));
        let mut status_ok = false;
        let mut accept_seen: Option<String> = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line.is_empty() {
                return false;
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if trimmed.starts_with("HTTP/1.1 101") || trimmed.starts_with("HTTP/1.0 101") {
                status_ok = true;
            }
            if let Some((k, v)) = trimmed.split_once(':') {
                if k.trim().eq_ignore_ascii_case("sec-websocket-accept") {
                    accept_seen = Some(v.trim().to_string());
                }
            }
        }
        if !status_ok {
            return false;
        }
        // Verify accept = base64(sha1(nonce ++ GUID))
        let mut input = nonce.clone();
        input.push_str("258EAFA5-E914-47DA-95CA-5AB5DC11D68B");
        let expected = base64(&sha1(input.as_bytes()));
        match accept_seen {
            Some(seen) => seen == expected,
            None => false,
        }
    }

    // ── Frame I/O (client perspective: read unmasked, write masked) ────

    pub const OP_TEXT: u8 = 0x01;
    pub const OP_CLOSE: u8 = 0x08;
    pub const OP_PING: u8 = 0x09;
    pub const OP_PONG: u8 = 0x0A;

    fn write_masked_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> bool {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode); // FIN + opcode
        let len = payload.len();
        let len_byte_base = 0x80; // mask bit always set on client→server
        if len < 126 {
            frame.push(len_byte_base | (len as u8));
        } else if len < 65536 {
            frame.push(len_byte_base | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(len_byte_base | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mask = pseudo_random_bytes(4);
        frame.extend_from_slice(&mask);
        for (i, b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i % 4]);
        }
        stream.write_all(&frame).is_ok()
    }

    fn read_unmasked_frame(stream: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
        let mut header = [0u8; 2];
        stream.read_exact(&mut header).ok()?;
        let opcode = header[0] & 0x0F;
        let masked = (header[1] & 0x80) != 0;
        let mut len = (header[1] & 0x7F) as u64;
        if len == 126 {
            let mut buf = [0u8; 2];
            stream.read_exact(&mut buf).ok()?;
            len = u16::from_be_bytes(buf) as u64;
        } else if len == 127 {
            let mut buf = [0u8; 8];
            stream.read_exact(&mut buf).ok()?;
            len = u64::from_be_bytes(buf);
        }
        let mask = if masked {
            let mut buf = [0u8; 4];
            stream.read_exact(&mut buf).ok()?;
            Some(buf)
        } else {
            None
        };
        let mut payload = vec![0u8; len as usize];
        stream.read_exact(&mut payload).ok()?;
        if let Some(m) = mask {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= m[i % 4];
            }
        }
        Some((opcode, payload))
    }

    // ── C-ABI surface (called from n_ws_* in lib.rs) ───────────────────

    /// Compute the throttle delay for a given backoff step.
    fn backoff_delay(step: u64) -> Duration {
        let idx = (step as usize).min(BACKOFF_MS.len() - 1);
        Duration::from_millis(BACKOFF_MS[idx])
    }

    /// Attempt to (re)open the TCP connection + WebSocket handshake.
    /// Returns Ok(stream) on success, Err(()) on any failure.  The caller
    /// is responsible for backoff bookkeeping.
    fn try_open(url: &str) -> Result<TcpStream, ()> {
        let (host, port, path) = parse_ws_url(url).ok_or(())?;
        let addr = format!("{host}:{port}");
        // Try every resolved socket address (IPv4 + IPv6) in turn.  On
        // a typical Linux box `localhost` resolves to `::1` first, but
        // many loft servers listen on `0.0.0.0` (IPv4 only) — picking
        // just the first address would refuse a perfectly reachable
        // connection.  Walk the iterator until one succeeds or all
        // fail.
        let addrs: Vec<_> = addr.to_socket_addrs().map_err(|_| ())?.collect();
        if addrs.is_empty() {
            return Err(());
        }
        let mut last_err = None;
        for sa in &addrs {
            match TcpStream::connect_timeout(sa, Duration::from_secs(2)) {
                Ok(s) => {
                    let mut stream = s;
                    if do_handshake(&mut stream, &host, port, &path) {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
                        return Ok(stream);
                    }
                    last_err = Some("handshake");
                }
                Err(_) => {
                    last_err = Some("connect");
                }
            }
        }
        let _ = last_err;
        Err(())
    }

    use std::net::ToSocketAddrs;

    /// Drop the current stream (if any) and step backoff forward.
    fn mark_disconnected(conn: &mut Conn) {
        conn.stream = None;
        conn.last_attempt = Instant::now();
        conn.backoff_ms = (conn.backoff_ms + 1).min(BACKOFF_MS.len() as u64 - 1);
    }

    /// If disconnected and the backoff window has elapsed, try to
    /// reconnect.  Returns true iff the connection is currently live
    /// (either was already, or just succeeded).
    fn ensure_connected(conn: &mut Conn) -> bool {
        if conn.stream.is_some() {
            return true;
        }
        let elapsed = conn.last_attempt.elapsed();
        if elapsed < backoff_delay(conn.backoff_ms) {
            return false;
        }
        match try_open(&conn.url) {
            Ok(s) => {
                conn.stream = Some(s);
                conn.backoff_ms = 0;
                conn.last_attempt = Instant::now();
                true
            }
            Err(_) => {
                conn.last_attempt = Instant::now();
                conn.backoff_ms = (conn.backoff_ms + 1).min(BACKOFF_MS.len() as u64 - 1);
                false
            }
        }
    }

    pub fn connect(url: &str) -> i32 {
        // Validate the URL up front; malformed URLs return -1 so the
        // caller can distinguish "bad input" from "server not yet up".
        if parse_ws_url(url).is_none() {
            return -1;
        }
        // First attempt is best-effort; if it fails the slot is created
        // anyway so subsequent send/recv can drive the reconnect loop.
        let stream = try_open(url).ok();
        let conn = Conn {
            url: url.to_string(),
            stream,
            inbox: VecDeque::new(),
            // If first attempt failed, schedule the next try after the
            // first backoff step; if it succeeded, last_attempt sits in
            // the past and backoff stays at 0.
            last_attempt: Instant::now(),
            backoff_ms: 0,
        };
        CONNS.with(|c| {
            let mut c = c.borrow_mut();
            let id = c.len();
            c.push(Some(conn));
            id as i32
        })
    }

    pub fn send(handle: i32, msg: &str) -> bool {
        CONNS.with(|c| {
            let mut c = c.borrow_mut();
            let conn = match c.get_mut(handle as usize).and_then(|s| s.as_mut()) {
                Some(c) => c,
                None => return false,
            };
            if !ensure_connected(conn) {
                return false;
            }
            let stream = conn.stream.as_mut().expect("stream present after ensure_connected");
            let ok = write_masked_frame(stream, OP_TEXT, msg.as_bytes());
            if !ok {
                mark_disconnected(conn);
            }
            ok
        })
    }

    pub fn recv(handle: i32) -> bool {
        CONNS.with(|c| {
            let mut c = c.borrow_mut();
            let conn = match c.get_mut(handle as usize).and_then(|s| s.as_mut()) {
                Some(c) => c,
                None => return false,
            };
            // Drain anything already buffered first.
            if let Some(msg) = conn.inbox.pop_front() {
                LAST_MSG.with(|m| *m.borrow_mut() = msg);
                return true;
            }
            if !ensure_connected(conn) {
                return false;
            }
            let stream = conn.stream.as_mut().expect("stream present after ensure_connected");
            // Try one short read; on WouldBlock / TimedOut we just return
            // false so the loft program can spin.
            match read_unmasked_frame(stream) {
                Some((op, payload)) if op == OP_TEXT => {
                    let s = String::from_utf8_lossy(&payload).into_owned();
                    LAST_MSG.with(|m| *m.borrow_mut() = s);
                    true
                }
                Some((op, payload)) if op == OP_PING => {
                    // Reply pong, then return false so the caller polls again.
                    let _ = write_masked_frame(stream, OP_PONG, &payload);
                    false
                }
                Some((op, _)) if op == OP_CLOSE => {
                    mark_disconnected(conn);
                    false
                }
                Some(_) => false,
                None => {
                    // Read returned nothing (timeout or peer dropped).
                    // If the timeout elapsed cleanly the stream is still
                    // valid; otherwise treat as disconnect.  We detect
                    // the difference by checking whether take_error() or
                    // peer_addr() now fails — but for simplicity we
                    // assume short timeouts are fine and only mark as
                    // disconnected on later send/recv that actually
                    // errors.  Return false (no message yet).
                    false
                }
            }
        })
    }

    pub fn last_message() -> String {
        LAST_MSG.with(|m| m.borrow().clone())
    }

    pub fn close(handle: i32) {
        CONNS.with(|c| {
            let mut c = c.borrow_mut();
            if let Some(slot) = c.get_mut(handle as usize) {
                if let Some(conn) = slot.as_mut() {
                    if let Some(stream) = conn.stream.as_mut() {
                        let _ = write_masked_frame(stream, OP_CLOSE, &[]);
                    }
                }
                *slot = None;
            }
        });
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use std::cell::RefCell;

    thread_local! {
        static LAST_MSG: RefCell<String> = const { RefCell::new(String::new()) };
    }

    // ── Imports from the JS host (loftHost.ws_*) ───────────────────────

    unsafe extern "C" {
        fn host_ws_connect(url_ptr: *const u8, url_len: usize) -> i32;
        fn host_ws_send(handle: i32, msg_ptr: *const u8, msg_len: usize) -> i32;
        // Returns 1 if a message is available (and writes its length into
        // out_len); 0 if the queue is empty; -1 if the connection is closed.
        fn host_ws_recv(handle: i32, out_buf_ptr: *mut u8, out_buf_cap: usize) -> i32;
        fn host_ws_close(handle: i32);
    }

    pub fn connect(url: &str) -> i32 {
        unsafe { host_ws_connect(url.as_ptr(), url.len()) }
    }

    pub fn send(handle: i32, msg: &str) -> bool {
        unsafe { host_ws_send(handle, msg.as_ptr(), msg.len()) == 1 }
    }

    pub fn recv(handle: i32) -> bool {
        let mut buf = vec![0u8; 65536];
        let n = unsafe { host_ws_recv(handle, buf.as_mut_ptr(), buf.len()) };
        if n <= 0 {
            return false;
        }
        buf.truncate(n as usize);
        let s = String::from_utf8_lossy(&buf).into_owned();
        LAST_MSG.with(|m| *m.borrow_mut() = s);
        true
    }

    pub fn last_message() -> String {
        LAST_MSG.with(|m| m.borrow().clone())
    }

    pub fn close(handle: i32) {
        unsafe { host_ws_close(handle) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native_impl::{close, connect, last_message, recv, send};

#[cfg(target_arch = "wasm32")]
pub use wasm_impl::{close, connect, last_message, recv, send};
