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
}

fn parse_request(stream: &TcpStream) -> Option<(String, String, String)> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

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
    }

    let mut body = String::new();
    if content_length > 0 {
        let mut buf = vec![0u8; content_length];
        reader.read_exact(&mut buf).ok()?;
        body = String::from_utf8_lossy(&buf).to_string();
    }

    Some((method, path, body))
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
        Some((method, path, body)) => {
            LAST_METHOD.with(|m| *m.borrow_mut() = method);
            LAST_PATH.with(|p| *p.borrow_mut() = path);
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
    let headers = LAST_BODY.with(|b| b.borrow().clone()); // reuse body field for headers
    let stream = CURRENT_CONN.with(|c| c.borrow_mut().take());
    match stream {
        Some(mut s) => {
            // Read headers from the original request
            let hdrs = LAST_PATH.with(|_| {
                // Headers were parsed during tcp_accept; we need them for the upgrade.
                // For now, use a simplified approach: re-read from stored headers.
                headers.clone()
            });
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

// No loft_register_v1 needed — the interpreter finds n_* exports via dlsym.
