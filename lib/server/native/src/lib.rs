// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Minimal blocking HTTP server — std::net only, no external deps.
//! Polling model: loft controls the loop, native does TCP I/O.

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
pub extern "C" fn loft_tcp_listen(port: u32) -> i32 {
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
pub extern "C" fn loft_tcp_accept(handle: i32) -> bool {
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
pub extern "C" fn loft_tcp_method(out: *mut *const u8, out_len: *mut usize) {
    LAST_METHOD.with(|m| {
        let m = m.borrow();
        unsafe {
            *out = m.as_ptr();
            *out_len = m.len();
        }
    });
}

/// Get the path of the last accepted request.
#[unsafe(no_mangle)]
pub extern "C" fn loft_tcp_path(out: *mut *const u8, out_len: *mut usize) {
    LAST_PATH.with(|p| {
        let p = p.borrow();
        unsafe {
            *out = p.as_ptr();
            *out_len = p.len();
        }
    });
}

/// Get the body of the last accepted request.
#[unsafe(no_mangle)]
pub extern "C" fn loft_tcp_body(out: *mut *const u8, out_len: *mut usize) {
    LAST_BODY.with(|b| {
        let b = b.borrow();
        unsafe {
            *out = b.as_ptr();
            *out_len = b.len();
        }
    });
}

/// Send an HTTP response on the current connection and close it.
#[unsafe(no_mangle)]
pub extern "C" fn loft_tcp_respond(status: u16, body_ptr: *const u8, body_len: usize) {
    let body = if body_ptr.is_null() || body_len == 0 {
        ""
    } else {
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(body_ptr, body_len)) }
    };
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
pub extern "C" fn loft_tcp_close(handle: i32) {
    LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        if let Some(slot) = l.get_mut(handle as usize) {
            *slot = None;
        }
    });
}

// ── Registration ────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_register_v1(
    register: unsafe extern "C" fn(*const u8, usize, *const (), *mut ()),
    ctx: *mut (),
) {
    macro_rules! reg {
        ($name:expr, $fn:ident) => {
            register($name.as_ptr(), $name.len(), $fn as *const (), ctx)
        };
    }
    unsafe {
        reg!(b"loft_tcp_listen", loft_tcp_listen);
        reg!(b"loft_tcp_accept", loft_tcp_accept);
        reg!(b"loft_tcp_method", loft_tcp_method);
        reg!(b"loft_tcp_path", loft_tcp_path);
        reg!(b"loft_tcp_body", loft_tcp_body);
        reg!(b"loft_tcp_respond", loft_tcp_respond);
        reg!(b"loft_tcp_close", loft_tcp_close);
    }
}
