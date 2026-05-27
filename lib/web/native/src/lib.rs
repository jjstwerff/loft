// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native HTTP client + WebSocket client.  HTTP uses ureq; WebSocket uses
//! plain std::net (native build) or host imports (wasm build).
//! WebSocket sessions auto-reconnect on connection failure with exponential
//! backoff capped at 10 seconds — see `ws_client::ensure_connected`.

use loft_ffi::LoftStr;

mod ws_client;

fn do_request(
    agent: Option<&ureq::Agent>,
    method: &str,
    url: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> (i32, String, String) {
    // `ureq` only knows GET/POST/PUT/DELETE for the simple verbs we expose; an
    // unknown method is a no-op (status 0) just like a network failure.
    if !matches!(method, "GET" | "POST" | "PUT" | "DELETE") {
        return (0, String::new(), String::new());
    }
    let mut req = match agent {
        Some(a) => a.request(method, url),
        None => ureq::request(method, url),
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
            let hdrs = capture_headers(&resp);
            (status, resp.into_string().unwrap_or_default(), hdrs)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let hdrs = capture_headers(&resp); // 4xx/5xx still carry headers
            (code as i32, resp.into_string().unwrap_or_default(), hdrs)
        }
        Err(_) => (0, String::new(), String::new()),
    }
}

/// Format a response's headers as newline-joined `"name: value"` lines.  Names
/// are lowercased (HTTP header names are case-insensitive) so the loft side can
/// match with `==` and needs no `lower()`.  Repeated headers (e.g. multiple
/// `Set-Cookie`) appear as separate lines, in header order.  Must be called
/// before `into_string()`, which consumes the response.
fn capture_headers(resp: &ureq::Response) -> String {
    let mut out = String::new();
    for name in resp.headers_names() {
        let lname = name.to_ascii_lowercase();
        for val in resp.all(&name) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&lname);
            out.push_str(": ");
            out.push_str(val);
        }
    }
    out
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
) -> i64 {
    let method = unsafe { loft_ffi::text(method_ptr, method_len) };
    let url = unsafe { loft_ffi::text(url_ptr, url_len) };
    let body = unsafe { loft_ffi::text_opt(body_ptr, body_len) };
    let headers_text = unsafe { loft_ffi::text_opt(headers_ptr, headers_len) }.unwrap_or("");
    let headers = parse_headers(headers_text);
    // Take-and-reset the one-shot session selection, then clone the agent out so
    // we don't hold the registry borrow across the blocking request.  -1 = the
    // stateless path (a stray stateless call never inherits a session).
    let active = ACTIVE_AGENT.with(|c| c.replace(-1));
    let agent = if active >= 0 {
        AGENTS.with(|a| a.borrow().get(active as usize).and_then(|o| o.clone()))
    } else {
        None
    };
    let (status, response_body, response_headers) =
        do_request(agent.as_ref(), method, url, body, &headers);
    // Store body + headers for n_http_body / n_http_headers_raw to return.
    LAST_BODY.with(|b| *b.borrow_mut() = response_body);
    LAST_HEADERS.with(|h| *h.borrow_mut() = response_headers);
    i64::from(status)
}

/// Create a cookie-jar session.  `redirects` = max redirects to follow
/// (0 = don't follow, so the caller can read the `Location` header).  With the
/// `cookies` feature an `AgentBuilder` carries a default jar.  Returns the handle.
#[unsafe(no_mangle)]
pub extern "C" fn n_http_session_new(redirects: i64) -> i64 {
    let agent = ureq::AgentBuilder::new()
        .redirects(redirects.max(0) as u32)
        .build();
    AGENTS.with(|a| {
        let mut v = a.borrow_mut();
        v.push(Some(agent));
        (v.len() - 1) as i64
    })
}

/// Select the session whose agent the next `n_http_do` call routes through.
#[unsafe(no_mangle)]
pub extern "C" fn n_http_session_use(handle: i64) {
    ACTIVE_AGENT.with(|c| c.set(handle as i32));
}

/// Return the body from the last HTTP request.
#[unsafe(no_mangle)]
pub extern "C" fn n_http_body() -> LoftStr {
    LAST_BODY.with(|b| loft_ffi::ret_ref(&b.borrow()))
}

/// Return the last response's headers, newline-joined `"name: value"` (names
/// lowercased, duplicates preserved).  The loft side splits this into the
/// `HttpResponse.headers` vector.
#[unsafe(no_mangle)]
pub extern "C" fn n_http_headers_raw() -> LoftStr {
    LAST_HEADERS.with(|h| loft_ffi::ret_ref(&h.borrow()))
}

/// ASCII-lowercase a string.  Generic helper — loft has no `lower()`; lets the
/// header accessors normalise a caller-supplied name.  (Cleaner long-term home
/// is a core `text.lower()`; kept local to avoid core scope-creep.)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ascii_lower(ptr: *const u8, len: usize) -> LoftStr {
    let s = unsafe { loft_ffi::text(ptr, len) };
    loft_ffi::ret(s.to_ascii_lowercase())
}

/// Standard base64-encode the input bytes (helps OAuth2 / SSO header building).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_base64_encode(ptr: *const u8, len: usize) -> LoftStr {
    use base64::Engine;
    let s = unsafe { loft_ffi::text(ptr, len) };
    loft_ffi::ret(base64::engine::general_purpose::STANDARD.encode(s.as_bytes()))
}

/// Standard base64-decode; invalid input decodes to an empty string.  The
/// decoded bytes are interpreted as UTF-8 (lossily) since loft `text` is a
/// UTF-8 buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_base64_decode(ptr: *const u8, len: usize) -> LoftStr {
    use base64::Engine;
    let s = unsafe { loft_ffi::text(ptr, len) };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .unwrap_or_default();
    loft_ffi::ret(String::from_utf8_lossy(&bytes).into_owned())
}

use std::cell::{Cell, RefCell};

thread_local! {
    static LAST_BODY: RefCell<String> = const { RefCell::new(String::new()) };
    static LAST_HEADERS: RefCell<String> = const { RefCell::new(String::new()) };
    // Session handle -> Agent; index is the handle.  Agents are Arc-backed, so a
    // clone shares the cookie jar + connection pool.
    static AGENTS: RefCell<Vec<Option<ureq::Agent>>> = const { RefCell::new(Vec::new()) };
    // Agent selected for the NEXT http_do (-1 = stateless).  One-shot: http_do
    // resets it after reading.
    static ACTIVE_AGENT: Cell<i32> = const { Cell::new(-1) };
}

// ── WebSocket client C-ABI exports ───────────────────────────────────────

/// Open (or queue for retry) a WebSocket connection.  Always returns a
/// non-negative handle unless the URL is malformed.  If the initial
/// handshake fails, the slot is created in disconnected state and the
/// next send/recv will trigger a reconnect attempt subject to backoff.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ws_connect(url_ptr: *const u8, url_len: usize) -> i64 {
    let url = unsafe { loft_ffi::text(url_ptr, url_len) };
    i64::from(ws_client::connect(url))
}

/// Send a text message on a WebSocket.  Returns true on success, false if
/// the connection is not currently live (caller may retry on the next
/// poll — reconnect is automatic with backoff).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ws_client_send(
    handle: i64,
    msg_ptr: *const u8,
    msg_len: usize,
) -> bool {
    let msg = unsafe { loft_ffi::text(msg_ptr, msg_len) };
    ws_client::send(handle as i32, msg)
}

/// Send a binary message on a WebSocket.  Same byte buffer as
/// `n_ws_client_send`, but the frame goes out with opcode `0x02`
/// (binary) instead of `0x01` (text).  TTT v5 + plan-36 use this
/// for `world_snapshot` / `world_delta` blobs from the client side
/// (bulk world data; client never sends bulk in this design, but
/// the symmetry keeps the API uniform with lib/server).
///
/// # Safety
///
/// `msg_ptr` / `msg_len` must describe a valid byte slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ws_client_send_binary(
    handle: i64,
    msg_ptr: *const u8,
    msg_len: usize,
) -> bool {
    let msg = unsafe { std::slice::from_raw_parts(msg_ptr, msg_len) };
    ws_client::send_binary(handle as i32, msg)
}

/// Poll for the next received message.  Returns true if a message was
/// delivered (then call n_ws_client_message), false if the queue is
/// empty or the connection is currently down.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_client_recv(handle: i64) -> bool {
    ws_client::recv(handle as i32)
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

/// Get the opcode of the last frame surfaced by `n_ws_client_recv`.
/// 1 = text, 2 = binary, 8 = close, 9 = ping, 10 = pong.  Loft
/// programs that handle binary frames check this after a successful
/// recv to decide whether the message bytes are utf-8 text or a
/// binary blob.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_client_opcode() -> i64 {
    i64::from(ws_client::last_opcode())
}

/// Close a WebSocket session permanently (no reconnect).
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_client_close(handle: i64) {
    ws_client::close(handle as i32);
}

/// Block the calling thread for `ms` milliseconds.  Used by tests to
/// pace WebSocket client behaviour deterministically when wall-clock
/// races would otherwise dominate (P229a — macOS scheduler is fast
/// enough that two clients complete their move sequence with no
/// observable overlap).  Negative / zero values are no-ops.
#[unsafe(no_mangle)]
pub extern "C" fn n_sleep_ms(ms: i64) {
    if ms <= 0 {
        return;
    }
    std::thread::sleep(std::time::Duration::from_millis(ms as u64));
}

// ── Binary buffer construction (TTT v5 / plan-36) ──────────────────────
//
// Loft `text` is a UTF-8 byte buffer; the lexer + interpreter both
// treat the codepoint 0 (NUL) as the `character` null sentinel, so a
// `text` built via `"{c}{c}…"` interpolation silently drops zero bytes.
// Binary protocols that include zero bytes (e.g. the v5 5-byte blob
// header `[type:u8] [session:u32-LE]` for any small session id) cannot
// be assembled that way.
//
// `n_pack_*` are a thread-local builder pattern: reset, push fields by
// type, then take the buffer as a `text` whose bytes carry the binary
// payload verbatim (NUL bytes preserved).  The receiver reads bytes
// out via `n_byte_at` so multi-byte UTF-8 misinterpretation cannot
// corrupt the stream.

thread_local! {
    static LAST_WS_MSG: RefCell<String> = const { RefCell::new(String::new()) };
    static PACK_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static LAST_PACKED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Empty the per-thread pack buffer.  Call before each blob.
#[unsafe(no_mangle)]
pub extern "C" fn n_pack_reset() {
    PACK_BUF.with(|b| b.borrow_mut().clear());
}

/// Append a single byte to the pack buffer.  `b` is masked to 8 bits.
#[unsafe(no_mangle)]
pub extern "C" fn n_pack_u8(b: i64) {
    PACK_BUF.with(|buf| buf.borrow_mut().push((b & 0xff) as u8));
}

/// Append a 2-byte little-endian unsigned value to the pack buffer.
#[unsafe(no_mangle)]
pub extern "C" fn n_pack_u16_le(v: i64) {
    PACK_BUF.with(|buf| {
        let mut buf = buf.borrow_mut();
        let v = (v & 0xffff) as u16;
        buf.extend_from_slice(&v.to_le_bytes());
    });
}

/// Append a 4-byte little-endian unsigned value to the pack buffer.
/// Loft `integer` is 64-bit — take the low 32 bits (cast to u32) to
/// preserve the LE byte pattern across the sign boundary.
#[unsafe(no_mangle)]
pub extern "C" fn n_pack_u32_le(v: i64) {
    PACK_BUF.with(|buf| {
        let mut buf = buf.borrow_mut();
        let v = v as u32;
        buf.extend_from_slice(&v.to_le_bytes());
    });
}

/// Take the contents of the pack buffer and surface them as a `text`
/// whose UTF-8 bytes are exactly the buffer bytes.  Resets the buffer.
/// The resulting `text` lives in `LAST_PACKED` until the next
/// `n_pack_take` call — copy / send before reusing.
#[unsafe(no_mangle)]
pub extern "C" fn n_pack_take() -> LoftStr {
    PACK_BUF.with(|buf| {
        let v = std::mem::take(&mut *buf.borrow_mut());
        LAST_PACKED.with(|p| {
            *p.borrow_mut() = unsafe { String::from_utf8_unchecked(v) };
            loft_ffi::ret_ref(&p.borrow())
        })
    })
}

/// Read the byte at `idx` of the text payload.  Out-of-range returns
/// -1.  Negative `idx` is also out-of-range.  Used to decode binary
/// frames received via `try_recv` / `pump`.
///
/// Argument order is `(idx, text_ptr, text_len)` so the auto-marshal
/// recognises the `(I64, Text) -> I64` signature in
/// `src/extensions.rs::auto_marshal_dispatcher`.  The natural
/// `(text, idx)` order would have demanded a `(Text, I64) -> I64`
/// branch we'd otherwise need to add.
///
/// # Safety
///
/// `text_ptr` / `text_len` must describe a valid byte slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_byte_at(idx: i64, text_ptr: *const u8, text_len: usize) -> i64 {
    if idx < 0 || (idx as usize) >= text_len {
        return -1;
    }
    unsafe { i64::from(*text_ptr.add(idx as usize)) }
}

// ── WsGroup: multiplexed client-side receiver ───────────────────────────
//
// A thread-local list of WsHandler ids.  `poll` scans them round-robin
// and returns the first one with a message ready.  One timeout across
// all handles instead of one per handle — the key win for multi-client
// drain performance.

thread_local! {
    static WS_GROUP: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) };
    static WS_GROUP_OFFSET: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Clear the group handle list.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_group_clear() {
    WS_GROUP.with(|g| g.borrow_mut().clear());
    WS_GROUP_OFFSET.with(|o| o.set(0));
}

/// Add a handle to the group.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_group_add(handle: i64) {
    WS_GROUP.with(|g| g.borrow_mut().push(handle as i32));
}

/// Poll the group round-robin.  Returns the handle that has a message
/// (read it with n_ws_client_message), or -1 if none are ready.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_group_poll() -> i64 {
    let (handles, offset) = WS_GROUP.with(|g| {
        let g = g.borrow();
        (g.clone(), WS_GROUP_OFFSET.with(|o| o.get()))
    });
    let result = ws_client::poll_group(&handles, offset);
    if result >= 0 {
        // Advance offset past the handle that fired so the next poll
        // starts from the one after it — round-robin fairness.
        if let Some(pos) = handles.iter().position(|&h| h == result) {
            WS_GROUP_OFFSET.with(|o| o.set((pos + 1) % handles.len()));
        }
    }
    i64::from(result)
}

// @PLAN12 phase 2 final step (2026-05-24): the `loft_ffi::loft_register!`
// invocation is generated by `build.rs` from
// `lib/web/loft.toml::[native.functions]`, so the symbol list lives in
// exactly one place.  Adding a new web symbol is now a single edit to
// `loft.toml` (plus the `pub unsafe extern "C" fn` body above).
include!(concat!(env!("OUT_DIR"), "/loft_register_gen.rs"));
