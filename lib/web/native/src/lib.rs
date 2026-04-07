// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native HTTP client. No loft dependency — uses ureq + loft-ffi only.

use loft_ffi::LoftStr;

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

loft_ffi::loft_register! {
    n_http_do,
    n_http_body,
}
