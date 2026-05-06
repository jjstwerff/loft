// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native HTTP client + WebSocket client.  HTTP uses ureq; WebSocket uses
//! plain std::net (native build) or host imports (wasm build).
//! WebSocket sessions auto-reconnect on connection failure with exponential
//! backoff capped at 10 seconds — see `ws_client::ensure_connected`.

use loft_ffi::LoftStr;

mod ws_client;

fn do_request(
    method: &str,
    url: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> (i32, String) {
    let mut req = match method {
        "GET" => ureq::get(url),
        "POST" => ureq::post(url),
        "PUT" => ureq::put(url),
        "DELETE" => ureq::delete(url),
        _ => return (0, String::new()),
    };
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let response = if let Some(b) = body {
        req.send_string(b)
    } else {
        req.call()
    };
    match response {
        Ok(resp) => {
            let status = resp.status() as i32;
            let body = resp.into_string().unwrap_or_default();
            (status, body)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            (code as i32, body)
        }
        Err(_) => (0, String::new()),
    }
}

fn parse_headers(header_text: &str) -> Vec<(&str, &str)> {
    header_text
        .split('\n')
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            line.split_once(':').map(|(k, v)| (k.trim(), v.trim()))
        })
        .collect()
}

// ── C-ABI exports ───────────────────────────────────────────────────────

/// HTTP request. Returns status code; response body available via n_http_body.
/// This function stores the body in a thread-local for the interpreter to retrieve.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_http_do(
    method_ptr: *const u8,
    method_len: usize,
    url_ptr: *const u8,
    url_len: usize,
    body_ptr: *const u8,
    body_len: usize,
    headers_ptr: *const u8,
    headers_len: usize,
) -> i32 {
    let method = unsafe { loft_ffi::text(method_ptr, method_len) };
    let url = unsafe { loft_ffi::text(url_ptr, url_len) };
    let body = unsafe { loft_ffi::text_opt(body_ptr, body_len) };
    let headers_text = unsafe { loft_ffi::text_opt(headers_ptr, headers_len) }.unwrap_or("");
    let headers = parse_headers(headers_text);
    let (status, response_body) = do_request(method, url, body, &headers);
    // Store body for n_http_body to return.
    LAST_BODY.with(|b| *b.borrow_mut() = response_body);
    status
}

/// Return the body from the last HTTP request.
#[unsafe(no_mangle)]
pub extern "C" fn n_http_body() -> LoftStr {
    LAST_BODY.with(|b| loft_ffi::ret_ref(&b.borrow()))
}

use std::cell::RefCell;

thread_local! {
    static LAST_BODY: RefCell<String> = const { RefCell::new(String::new()) };
}

// ── WebSocket client C-ABI exports ───────────────────────────────────────

/// Open (or queue for retry) a WebSocket connection.  Always returns a
/// non-negative handle unless the URL is malformed.  If the initial
/// handshake fails, the slot is created in disconnected state and the
/// next send/recv will trigger a reconnect attempt subject to backoff.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ws_connect(url_ptr: *const u8, url_len: usize) -> i32 {
    let url = unsafe { loft_ffi::text(url_ptr, url_len) };
    ws_client::connect(url)
}

/// Send a text message on a WebSocket.  Returns true on success, false if
/// the connection is not currently live (caller may retry on the next
/// poll — reconnect is automatic with backoff).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ws_client_send(
    handle: i32,
    msg_ptr: *const u8,
    msg_len: usize,
) -> bool {
    let msg = unsafe { loft_ffi::text(msg_ptr, msg_len) };
    ws_client::send(handle, msg)
}

/// Poll for the next received message.  Returns true if a message was
/// delivered (then call n_ws_client_message), false if the queue is
/// empty or the connection is currently down.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_client_recv(handle: i32) -> bool {
    ws_client::recv(handle)
}

/// Get the last message returned by `n_ws_client_recv`.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_client_message() -> LoftStr {
    LAST_WS_MSG.with(|m| {
        let new = ws_client::last_message();
        *m.borrow_mut() = new;
        loft_ffi::ret_ref(&m.borrow())
    })
}

/// Close a WebSocket session permanently (no reconnect).
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_client_close(handle: i32) {
    ws_client::close(handle);
}

/// Block the calling thread for `ms` milliseconds.  Used by tests to
/// pace WebSocket client behaviour deterministically when wall-clock
/// races would otherwise dominate (P229a — macOS scheduler is fast
/// enough that two clients complete their move sequence with no
/// observable overlap).  Negative / zero values are no-ops.
#[unsafe(no_mangle)]
pub extern "C" fn n_sleep_ms(ms: i32) {
    if ms <= 0 {
        return;
    }
    std::thread::sleep(std::time::Duration::from_millis(ms as u64));
}

thread_local! {
    static LAST_WS_MSG: RefCell<String> = const { RefCell::new(String::new()) };
}

loft_ffi::loft_register! {
    n_http_do,
    n_http_body,
    n_ws_connect,
    n_ws_client_send,
    n_ws_client_recv,
    n_ws_client_message,
    n_ws_client_close,
    n_sleep_ms,
}
