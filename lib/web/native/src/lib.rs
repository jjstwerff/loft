// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native HTTP client. No loft dependency — uses ureq only.

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

/// Parse newline-separated "Key: Value" header text into pairs.
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

/// HTTP request with optional body and headers.
/// Headers are newline-separated "Key: Value" text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_http_request(
    method_ptr: *const u8,
    method_len: usize,
    url_ptr: *const u8,
    url_len: usize,
    body_ptr: *const u8,
    body_len: usize,
    headers_ptr: *const u8,
    headers_len: usize,
    out_status: *mut i32,
    out_body: *mut *mut u8,
    out_body_len: *mut usize,
) {
    let method = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(method_ptr, method_len))
    };
    let url =
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(url_ptr, url_len)) };
    let body = if body_ptr.is_null() || body_len == 0 {
        None
    } else {
        Some(unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(body_ptr, body_len))
        })
    };
    let headers_text = if headers_ptr.is_null() || headers_len == 0 {
        ""
    } else {
        unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(headers_ptr, headers_len))
        }
    };
    let headers = parse_headers(headers_text);
    let (status, response_body) = do_request(method, url, body, &headers);
    unsafe {
        *out_status = status;
        let mut bytes = response_body.into_bytes();
        *out_body_len = bytes.len();
        *out_body = bytes.as_mut_ptr();
        std::mem::forget(bytes);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_free_string(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, len, len) });
    }
}

// ── Registration ────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_register_v1(
    register: unsafe extern "C" fn(*const u8, usize, *const (), *mut ()),
    ctx: *mut (),
) {
    unsafe {
        register(
            b"loft_http_request".as_ptr(),
            17,
            loft_http_request as *const (),
            ctx,
        );
        register(
            b"loft_free_string".as_ptr(),
            16,
            loft_free_string as *const (),
            ctx,
        );
    }
}
