// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Minimal blocking HTTP server + WebSocket — std::net only, no external deps.
//! Polling model: loft controls the loop, native does TCP I/O.

mod websocket;

use loft_ffi::LoftStr;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

thread_local! {
    static LISTENERS: RefCell<Vec<Option<TcpListener>>> = const { RefCell::new(Vec::new()) };
    static CURRENT_CONN: RefCell<Option<TcpStream>> = const { RefCell::new(None) };
    static LAST_METHOD: RefCell<String> = const { RefCell::new(String::new()) };
    static LAST_PATH: RefCell<String> = const { RefCell::new(String::new()) };
    static LAST_BODY: RefCell<String> = const { RefCell::new(String::new()) };
    /// Raw header block from the most recent accept (line-separated
    /// `Key: Value` lines).  Stored separately from the body so the
    /// existing HTTP API keeps its `body`-only semantics while
    /// `n_ws_upgrade` can find `Sec-WebSocket-Key`.
    static LAST_HEADERS: RefCell<String> = const { RefCell::new(String::new()) };
}

fn parse_request(stream: &TcpStream) -> Option<(String, String, String, String)> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    let mut headers = String::new();
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        if line.trim().is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        headers.push_str(line.trim_end_matches(['\r', '\n']));
        headers.push('\n');
    }

    let mut body = String::new();
    if content_length > 0 {
        let mut buf = vec![0u8; content_length];
        reader.read_exact(&mut buf).ok()?;
        body = String::from_utf8_lossy(&buf).to_string();
    }

    Some((method, path, headers, body))
}

// ── C-ABI exports ───────────────────────────────────────────────────────

/// Bind a TCP listener on the given port. Returns handle (>= 0) or -1.
#[unsafe(no_mangle)]
pub extern "C" fn n_tcp_listen(port: u32) -> i32 {
    let addr = format!("0.0.0.0:{port}");
    match TcpListener::bind(&addr) {
        Ok(listener) => {
            eprintln!("loft server listening on {addr}");
            LISTENERS.with(|l| {
                let mut l = l.borrow_mut();
                let idx = l.len();
                l.push(Some(listener));
                idx as i32
            })
        }
        Err(e) => {
            eprintln!("loft_tcp_listen: cannot bind {addr}: {e}");
            -1
        }
    }
}

/// Accept the next connection and parse the HTTP request.
/// Blocks until a connection arrives. Returns true on success, false on error.
/// After success, call loft_tcp_method/path/body to read the request fields.
#[unsafe(no_mangle)]
pub extern "C" fn n_tcp_accept(handle: i32) -> bool {
    let stream = LISTENERS.with(|l| {
        let l = l.borrow();
        l.get(handle as usize)
            .and_then(|opt| opt.as_ref())
            .and_then(|listener| listener.accept().ok().map(|(s, _)| s))
    });
    let stream = match stream {
        Some(s) => s,
        None => return false,
    };
    match parse_request(&stream) {
        Some((method, path, headers, body)) => {
            LAST_METHOD.with(|m| *m.borrow_mut() = method);
            LAST_PATH.with(|p| *p.borrow_mut() = path);
            LAST_HEADERS.with(|h| *h.borrow_mut() = headers);
            LAST_BODY.with(|b| *b.borrow_mut() = body);
            CURRENT_CONN.with(|c| *c.borrow_mut() = Some(stream));
            true
        }
        None => false,
    }
}

/// Get the method of the last accepted request.
#[unsafe(no_mangle)]
pub extern "C" fn n_tcp_method() -> LoftStr {
    LAST_METHOD.with(|m| loft_ffi::ret_ref(&m.borrow()))
}

/// Get the path of the last accepted request.
#[unsafe(no_mangle)]
pub extern "C" fn n_tcp_path() -> LoftStr {
    LAST_PATH.with(|p| loft_ffi::ret_ref(&p.borrow()))
}

/// Get the body of the last accepted request.
#[unsafe(no_mangle)]
pub extern "C" fn n_tcp_body() -> LoftStr {
    LAST_BODY.with(|b| loft_ffi::ret_ref(&b.borrow()))
}

/// Send an HTTP response on the current connection and close it.
#[unsafe(no_mangle)]
pub extern "C" fn n_tcp_respond(status: u16, body_ptr: *const u8, body_len: usize) {
    let body = unsafe { loft_ffi::text_opt(body_ptr, body_len) }.unwrap_or("");
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Length: {}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Connection: close\r\n\r\n\
         {body}",
        body.len()
    );
    CURRENT_CONN.with(|c| {
        if let Some(ref mut stream) = *c.borrow_mut() {
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    // Close the connection
    CURRENT_CONN.with(|c| *c.borrow_mut() = None);
}

/// Close a listener.
#[unsafe(no_mangle)]
pub extern "C" fn n_tcp_close(handle: i32) {
    LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        if let Some(slot) = l.get_mut(handle as usize) {
            *slot = None;
        }
    });
}

// ── WebSocket C-ABI exports (SRV.3) ─────────────────────────────────────

thread_local! {
    static WS_CONNS: RefCell<Vec<Option<TcpStream>>> = const { RefCell::new(Vec::new()) };
    static WS_LAST_MSG: RefCell<String> = const { RefCell::new(String::new()) };
    static WS_LAST_OPCODE: RefCell<u8> = const { RefCell::new(0) };
}

/// Upgrade the current HTTP connection to WebSocket. Returns handle (>= 0) or -1.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_upgrade() -> i32 {
    let hdrs = LAST_HEADERS.with(|h| h.borrow().clone());
    let stream = CURRENT_CONN.with(|c| c.borrow_mut().take());
    match stream {
        Some(mut s) => {
            if !websocket::ws_upgrade(&mut s, &hdrs) {
                return -1;
            }
            WS_CONNS.with(|conns| {
                let mut conns = conns.borrow_mut();
                let idx = conns.len();
                conns.push(Some(s));
                idx as i32
            })
        }
        None => -1,
    }
}

/// Read the next WebSocket message. Returns true on success, false on close/error.
/// After success, call loft_ws_message/loft_ws_opcode to get the data.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_recv(handle: i32) -> bool {
    WS_CONNS.with(|conns| {
        let mut conns = conns.borrow_mut();
        let stream = match conns.get_mut(handle as usize).and_then(|o| o.as_mut()) {
            Some(s) => s,
            None => return false,
        };
        match websocket::ws_read_frame(stream) {
            Some(frame) => {
                if frame.opcode == websocket::OP_CLOSE {
                    return false;
                }
                if frame.opcode == websocket::OP_PING {
                    let _ = websocket::ws_write_frame(stream, websocket::OP_PONG, &frame.payload);
                    // Recurse to get the next real message
                    return true; // signal caller to call recv again
                }
                WS_LAST_OPCODE.with(|o| *o.borrow_mut() = frame.opcode);
                WS_LAST_MSG.with(|m| {
                    *m.borrow_mut() = String::from_utf8_lossy(&frame.payload).to_string();
                });
                true
            }
            None => false,
        }
    })
}

/// Get the last received WebSocket message text.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_message() -> LoftStr {
    WS_LAST_MSG.with(|m| loft_ffi::ret_ref(&m.borrow()))
}

/// Get the last received WebSocket opcode (1=text, 2=binary, 8=close, 9=ping, 10=pong).
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_opcode() -> u8 {
    WS_LAST_OPCODE.with(|o| *o.borrow())
}

/// Send a text WebSocket message.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_send(handle: i32, msg_ptr: *const u8, msg_len: usize) -> bool {
    let msg = unsafe { std::slice::from_raw_parts(msg_ptr, msg_len) };
    WS_CONNS.with(|conns| {
        let mut conns = conns.borrow_mut();
        match conns.get_mut(handle as usize).and_then(|o| o.as_mut()) {
            Some(stream) => websocket::ws_write_frame(stream, websocket::OP_TEXT, msg),
            None => false,
        }
    })
}

/// Close a WebSocket connection.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_close(handle: i32) {
    WS_CONNS.with(|conns| {
        let mut conns = conns.borrow_mut();
        if let Some(slot) = conns.get_mut(handle as usize) {
            if let Some(stream) = slot.as_mut() {
                let _ = websocket::ws_write_frame(stream, websocket::OP_CLOSE, &[]);
            }
            *slot = None;
        }
    });
}

// ── Multi-client server primitives (TIC_TAC_TOE v2 ground layer) ─────────
//
// The legacy flow is `n_tcp_accept` (blocking) → `n_ws_upgrade` (consumes
// CURRENT_CONN) → one client at a time.  The multi-client flow below
// combines accept + parse + upgrade into a single non-blocking call so
// the loft program can hold many concurrent WebSocket clients and poll
// each without head-of-line blocking on any one of them.
//
//   loft loop:
//     loop {
//         id = n_ws_accept_nonblocking(listener);  // -1 if no pending
//         if id >= 0 { register new client }
//         for each active id: n_ws_recv (returns false fast on no data)
//         small sleep to avoid CPU spin
//     }
//
// Per-client streams are set non-blocking with a short read timeout
// (20 ms) on accept so n_ws_recv polls cleanly.

/// Try to accept a pending connection on a non-blocking listener.  If
/// one is pending, parse the HTTP request, perform the WebSocket
/// upgrade, register the stream as a client, and return its id (>= 0).
/// If no connection is pending, returns -1.  Returns -2 on a listener
/// or upgrade error so loft can distinguish "not yet" from "broken".
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_accept_nonblocking(listener_handle: i32) -> i32 {
    // Snapshot the listener and ensure non-blocking, then try accept.
    let stream_opt = LISTENERS.with(|l| {
        let l = l.borrow();
        let listener = l
            .get(listener_handle as usize)
            .and_then(|opt| opt.as_ref())?;
        let _ = listener.set_nonblocking(true);
        match listener.accept() {
            Ok((s, _)) => Some(Ok(s)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
            Err(_) => Some(Err(())),
        }
    });
    let mut stream = match stream_opt {
        None => return -1,
        Some(Ok(s)) => s,
        Some(Err(())) => return -2,
    };
    // The accepted stream inherits non-blocking state on some platforms;
    // force blocking for the HTTP read (small, finite), then switch to
    // a short read timeout for the post-upgrade WS read polling.
    let _ = stream.set_nonblocking(false);
    let (headers_opt, _path_opt) = match parse_request(&stream) {
        Some((_method, path, headers, _body)) => (Some(headers), Some(path)),
        None => return -2,
    };
    let headers = headers_opt.unwrap_or_default();
    if !websocket::ws_upgrade(&mut stream, &headers) {
        return -2;
    }
    // Switch to short-timeout reads so n_ws_recv polls without blocking.
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(20)));
    WS_CONNS.with(|conns| {
        let mut conns = conns.borrow_mut();
        // Reuse a freed slot if any (id stability across reconnects is
        // not required at this layer; ids are reused after close).
        for (i, slot) in conns.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(stream);
                return i as i32;
            }
        }
        let idx = conns.len();
        conns.push(Some(stream));
        idx as i32
    })
}

/// Total length of the WS_CONNS table (active + closed slots).  Loft
/// programs iterate `0..n_ws_clients_len()` and skip slots where
/// `n_ws_client_active` returns false.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_clients_len() -> i32 {
    WS_CONNS.with(|conns| conns.borrow().len() as i32)
}

/// True iff the WS_CONNS slot at `id` is currently occupied (a live
/// client connection).  Slots become inactive after `n_ws_close` or
/// when a peer disconnects.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_client_active(id: i32) -> bool {
    WS_CONNS.with(|conns| {
        conns
            .borrow()
            .get(id as usize)
            .map(|o| o.is_some())
            .unwrap_or(false)
    })
}

loft_ffi::loft_register! {
    n_tcp_listen,
    n_tcp_accept,
    n_tcp_method,
    n_tcp_path,
    n_tcp_body,
    n_tcp_respond,
    n_tcp_close,
    n_ws_upgrade,
    n_ws_recv,
    n_ws_message,
    n_ws_send,
    n_ws_close,
    n_ws_accept_nonblocking,
    n_ws_clients_len,
    n_ws_client_active,
}
