// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Minimal blocking HTTP server + WebSocket — std::net only, no external deps.
//! Polling model: loft controls the loop, native does TCP I/O.

mod websocket;

use loft_ffi::LoftStr;
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
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

fn parse_request(stream: &mut TcpStream) -> Option<(String, String, String, String)> {
    // Read the header block BYTE-BY-BYTE.  We deliberately do NOT use
    // BufReader here: a custom client may send WebSocket frames
    // immediately after its `Upgrade: websocket` request without
    // waiting for the server's `101 Switching Protocols` response.
    // A BufReader's internal buffer would slurp those leading WS
    // frame bytes along with the header bytes; on drop they vanish,
    // so the first ws_recv after the upgrade misses the client's
    // first frame.  Reading byte-by-byte until "\r\n\r\n" leaves the
    // post-header bytes in the kernel buffer for ws_read_frame.
    // Mirrors the client-side fix in lib/web/native/src/ws_client.rs.
    let mut window: [u8; 4] = [0, 0, 0, 0];
    let mut header_text = String::new();
    loop {
        let mut byte = [0u8; 1];
        match stream.read(&mut byte) {
            Ok(0) => return None, // EOF before headers complete
            Ok(_) => {}
            Err(_) => return None,
        }
        header_text.push(byte[0] as char);
        window[0] = window[1];
        window[1] = window[2];
        window[2] = window[3];
        window[3] = byte[0];
        if window == *b"\r\n\r\n" {
            break;
        }
        // Cap to avoid slurping a hostile peer's giant header block forever.
        if header_text.len() > 16 * 1024 {
            return None;
        }
    }

    let mut lines = header_text.split("\r\n");
    let request_line = lines.next()?;
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    let mut headers = String::new();
    let mut content_length: usize = 0;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
        headers.push_str(line);
        headers.push('\n');
    }

    let mut body = String::new();
    if content_length > 0 {
        let mut buf = vec![0u8; content_length];
        stream.read_exact(&mut buf).ok()?;
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
    let mut stream = match stream {
        Some(s) => s,
        None => return false,
    };
    match parse_request(&mut stream) {
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
///
/// # Safety
///
/// `body_ptr` / `body_len` must describe a valid byte slice or be `(NULL, 0)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_tcp_respond(status: u16, body_ptr: *const u8, body_len: usize) {
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
///
/// # Safety
///
/// `msg_ptr` / `msg_len` must describe a valid byte slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ws_send(handle: i32, msg_ptr: *const u8, msg_len: usize) -> bool {
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
// The clean event-pump entry point (`n_ws_next_event`) is below.  Loft
// programs use it via `Server::run(on_connect, on_message)` and never
// see the slot table directly.  The split entry points
// (`n_ws_accept_nonblocking`, `n_ws_clients_len`,
// `n_ws_client_active`) are kept as a private fallback path.
//
// Per-client streams are set non-blocking with a short read timeout
// (20 ms) on accept so polling stays cheap.

/// Three-way result of accepting a pending connection on a
/// non-blocking listener.  Both NoneYet and Error look identical
/// to the event pump (nothing to deliver this poll), but the
/// legacy `n_ws_accept_nonblocking` entry point keeps its -1 / -2
/// distinction by reading this directly.
enum AcceptOutcome {
    Pending(i32),
    NoneYet,
    Error,
}

fn try_accept_inner(listener_handle: i32) -> AcceptOutcome {
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
        None => return AcceptOutcome::NoneYet,
        Some(Ok(s)) => s,
        Some(Err(())) => return AcceptOutcome::Error,
    };
    // The accepted stream inherits non-blocking state on some platforms;
    // force blocking for the HTTP read (small, finite), then switch to
    // a short read timeout for the post-upgrade WS read polling.
    let _ = stream.set_nonblocking(false);
    let (headers_opt, _path_opt) = match parse_request(&mut stream) {
        Some((_method, path, headers, _body)) => (Some(headers), Some(path)),
        None => return AcceptOutcome::Error,
    };
    let headers = headers_opt.unwrap_or_default();
    if !websocket::ws_upgrade(&mut stream, &headers) {
        return AcceptOutcome::Error;
    }
    // Switch to short-timeout reads so polling stays non-blocking.
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(20)));
    let id = WS_CONNS.with(|conns| {
        let mut conns = conns.borrow_mut();
        // Reuse a freed slot if any (id stability across reconnects
        // is not required at this layer; ids are reused after close).
        for (i, slot) in conns.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(stream);
                return i as i32;
            }
        }
        let idx = conns.len();
        conns.push(Some(stream));
        idx as i32
    });
    AcceptOutcome::Pending(id)
}

/// Try to accept a pending connection on a non-blocking listener.  If
/// one is pending, parse the HTTP request, perform the WebSocket
/// upgrade, register the stream as a client, and return its id (>= 0).
/// If no connection is pending, returns -1.  Returns -2 on a listener
/// or upgrade error so loft can distinguish "not yet" from "broken".
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_accept_nonblocking(listener_handle: i32) -> i32 {
    match try_accept_inner(listener_handle) {
        AcceptOutcome::Pending(id) => id,
        AcceptOutcome::NoneYet => -1,
        AcceptOutcome::Error => -2,
    }
}

/// Used by the event pump: collapses NoneYet and Error into None
/// because both mean "nothing to deliver this poll".  Errors during
/// the listener phase are not surfaced to the loft program — they
/// would just be a noisy distraction during the normal idle path.
fn try_accept_one(listener_handle: i32) -> Option<i32> {
    match try_accept_inner(listener_handle) {
        AcceptOutcome::Pending(id) => Some(id),
        AcceptOutcome::NoneYet | AcceptOutcome::Error => None,
    }
}

/// Total length of the WS_CONNS table (active + closed slots).
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_clients_len() -> i32 {
    WS_CONNS.with(|conns| conns.borrow().len() as i32)
}

/// True iff the WS_CONNS slot at `id` is currently occupied.
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

// ── Event pump primitives (clean loft surface) ──────────────────────────
//
// The event pump is the single supported path for multi-client
// servers in loft.  Loft programs call `Server::run(on_connect,
// on_message)`, which internally drains events via
// `n_ws_next_event` until it returns false, then sleeps briefly
// and tries again.
//
// At-most-one-event-per-call keeps event order roughly real-time
// (the loft side cannot fall behind by more than one event).
//
// The Disconnected kind (2) is surfaced so the loft drain loop can
// keep advancing, but the loft `run()` body discards it without
// calling any application callback.  This was the user's explicit
// directive: the loft side does not know about disconnects.

thread_local! {
    static WS_EVENT_KIND:      Cell<i32>       = const { Cell::new(-1) };
    static WS_EVENT_CLIENT_ID: Cell<i32>       = const { Cell::new(-1) };
    static WS_EVENT_PAYLOAD:   RefCell<String> = const { RefCell::new(String::new()) };
}

enum PollOutcome {
    NoData,
    Frame(String),
    Disconnected,
}

fn poll_one_client(id: i32) -> PollOutcome {
    WS_CONNS.with(|conns| {
        let mut conns = conns.borrow_mut();
        let Some(stream) = conns.get_mut(id as usize).and_then(|o| o.as_mut()) else {
            return PollOutcome::NoData;
        };
        // PING is handled inline (write PONG, then keep probing).
        // Anything else returns immediately: NoData on timeout,
        // Frame on a real text frame, Disconnected on close / EOF.
        loop {
            match websocket::ws_read_frame_detailed(stream) {
                websocket::ReadOutcome::NoData => return PollOutcome::NoData,
                websocket::ReadOutcome::Closed => return PollOutcome::Disconnected,
                websocket::ReadOutcome::Frame(frame) => {
                    if frame.opcode == websocket::OP_CLOSE {
                        return PollOutcome::Disconnected;
                    }
                    if frame.opcode == websocket::OP_PING {
                        let _ = websocket::ws_write_frame(
                            stream,
                            websocket::OP_PONG,
                            &frame.payload,
                        );
                        continue;
                    }
                    let payload = String::from_utf8_lossy(&frame.payload).to_string();
                    return PollOutcome::Frame(payload);
                }
            }
        }
    })
}

/// Drain at most one event from the listener+clients on this server.
/// Returns true if an event was found (call n_ws_event_kind /
/// n_ws_event_client_id / n_ws_event_payload to read it).  Returns
/// false when nothing is pending.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_next_event(listener_handle: i32) -> bool {
    if let Some(cid) = try_accept_one(listener_handle) {
        WS_EVENT_KIND.with(|k| k.set(0));
        WS_EVENT_CLIENT_ID.with(|c| c.set(cid));
        WS_EVENT_PAYLOAD.with(|p| p.borrow_mut().clear());
        return true;
    }
    let len = WS_CONNS.with(|c| c.borrow().len()) as i32;
    for i in 0..len {
        let active = WS_CONNS.with(|c| {
            c.borrow().get(i as usize).is_some_and(|o| o.is_some())
        });
        if !active {
            continue;
        }
        match poll_one_client(i) {
            PollOutcome::NoData => continue,
            PollOutcome::Frame(s) => {
                WS_EVENT_KIND.with(|k| k.set(1));
                WS_EVENT_CLIENT_ID.with(|c| c.set(i));
                WS_EVENT_PAYLOAD.with(|p| *p.borrow_mut() = s);
                return true;
            }
            PollOutcome::Disconnected => {
                WS_CONNS.with(|c| {
                    if let Some(slot) = c.borrow_mut().get_mut(i as usize) {
                        *slot = None;
                    }
                });
                WS_EVENT_KIND.with(|k| k.set(2));
                WS_EVENT_CLIENT_ID.with(|c| c.set(i));
                WS_EVENT_PAYLOAD.with(|p| p.borrow_mut().clear());
                return true;
            }
        }
    }
    false
}

/// Read the kind of the last event surfaced by n_ws_next_event.
/// 0 = Connected, 1 = Message, 2 = Disconnected.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_event_kind() -> i32 {
    WS_EVENT_KIND.with(|k| k.get())
}

/// Read the client id of the last event surfaced by n_ws_next_event.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_event_client_id() -> i32 {
    WS_EVENT_CLIENT_ID.with(|c| c.get())
}

/// Read the payload of the last event surfaced by n_ws_next_event.
/// Empty string for Connected and Disconnected events.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_event_payload() -> LoftStr {
    WS_EVENT_PAYLOAD.with(|p| loft_ffi::ret_ref(&p.borrow()))
}

/// Sleep for `ms` milliseconds.  The loft `run()` loop calls this
/// when a drain pass produced zero events to avoid CPU-spinning in
/// the no-clients-yet phase.  Doing the sleep in Rust keeps the
/// loft side oblivious to timing primitives — there is no general
/// `sleep` in the loft stdlib today.
#[unsafe(no_mangle)]
pub extern "C" fn n_ws_idle_sleep_ms(ms: i32) {
    if ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
    }
}

/// Send a text frame to every active WebSocket client.  Returns the
/// number of successful sends.  No iteration in loft.  The handle
/// argument is accepted for API symmetry with send_to / disconnect
/// but is currently ignored — WS_CONNS is a single thread-local
/// table shared across all servers in this thread.
///
/// # Safety
///
/// `msg_ptr` / `msg_len` must describe a valid byte slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ws_broadcast(
    _handle: i32,
    msg_ptr: *const u8,
    msg_len: usize,
) -> i32 {
    let msg = unsafe { std::slice::from_raw_parts(msg_ptr, msg_len) };
    WS_CONNS.with(|conns| {
        let mut conns = conns.borrow_mut();
        let mut count: i32 = 0;
        for slot in conns.iter_mut() {
            if let Some(stream) = slot.as_mut()
                && websocket::ws_write_frame(stream, websocket::OP_TEXT, msg)
            {
                count += 1;
            }
        }
        count
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
    n_ws_next_event,
    n_ws_event_kind,
    n_ws_event_client_id,
    n_ws_event_payload,
    n_ws_broadcast,
    n_ws_idle_sleep_ms,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    /// P221: when a client sends WS frame bytes immediately after the
    /// upgrade request without waiting for `101 Switching Protocols`,
    /// `parse_request` must NOT swallow them.  The bytes after the
    /// `\r\n\r\n` header terminator must remain in the kernel buffer
    /// for the next reader (e.g. `ws_read_frame`).  The original
    /// `BufReader::new(stream)` implementation absorbed those bytes
    /// into its internal buffer and lost them when the BufReader
    /// dropped at end of `parse_request`.
    #[test]
    fn p221_parse_request_leaves_post_header_bytes_in_kernel_buffer() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();

        let trailing: &[u8] = b"WS-FRAME-BYTES-XYZ";
        let trailing_owned = trailing.to_vec();
        let client = thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
            // Aggressive client: write the upgrade request AND the
            // first WS frame back-to-back, before reading the 101.
            let req = b"GET /ws HTTP/1.1\r\n\
                Host: 127.0.0.1\r\n\
                Upgrade: websocket\r\n\
                Connection: Upgrade\r\n\
                Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                Sec-WebSocket-Version: 13\r\n\
                \r\n";
            s.write_all(req).expect("write headers");
            s.write_all(&trailing_owned).expect("write trailing");
            // Hand the stream back via a small read so the test can
            // close once the server has consumed the trailing bytes.
            let mut sink = [0u8; 8];
            let _ = s.read(&mut sink);
        });

        let (mut server_stream, _peer) = listener.accept().expect("accept");
        let parsed = parse_request(&mut server_stream).expect("parse");
        assert_eq!(parsed.0, "GET");
        assert_eq!(parsed.1, "/ws");

        // The trailing bytes the client appended after the header
        // terminator must still be readable from the kernel buffer.
        server_stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut buf = vec![0u8; trailing.len()];
        server_stream.read_exact(&mut buf).expect("read trailing");
        assert_eq!(&buf[..], trailing, "post-header bytes were swallowed");

        drop(server_stream);
        let _ = client.join();
    }
}
