// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Browser-WASM bridge for the `web` library's WebSocket client (@PLN84 ZT-C).
//!
//! `loft --html` routes each `#native "n_<x>"` whose symbol is in the
//! `[wasm.bridge].routes` table (`loft.toml`) to the matching `pub fn` here;
//! the generated standalone Rust binary links this crate as `--extern
//! web_wasm=…` and calls the bridge directly (no cdylib `dlopen`, no `State`
//! indirection at runtime).
//!
//! Why ROUTED, not a bare host import (the design's open R3, now resolved):
//! the WS client needs `n_ws_client_message() -> text` and `n_pack_take() ->
//! text`.  The compiler's bare host-import path (`#[link(wasm_import_module =
//! "loft_gl")] safe fn n_ws_client_message() -> i32`) cannot return a loft
//! `text` — it declares the import returning `i32` then tries to read `.ptr`/
//! `.len` off that `i32` (a generated-code compile error, observed on this
//! exact lib).  Routing the natives through these `pub fn … -> String`
//! bridges sidesteps that entirely: the bridge owns wasm memory, so it copies
//! frame bytes in via the ptr/len ABI and returns an owned `String` the
//! routed-call path marshals correctly (the crypto `text -> text` shape).
//!
//! The low-level WebSocket I/O is HOST IMPORTS under module `loft_web`,
//! declared here and provided by `wasm/host.js` (a `Map<handle, {socket,
//! inbound[], current}>`).  Bytes cross via the ptr/len ABI over the wasm
//! linear memory; `host.js` always re-fetches `getMem().buffer` (it can
//! grow/detach).  The asyncify frame-yield (`ws_yield`) is the dedicated
//! suspend import (design option 2): `loft --html` adds `loft_web.ws_yield` to
//! the `wasm-opt --asyncify --pass-arg`, so a synchronous loft poll loop can
//! return to the JS event loop between iterations and let `WebSocket.onmessage`
//! deliver frames.  Native `web::yield_frame()` is a no-op.
//!
//! `native == wasm`: the latched-message + opcode contract, the text/binary
//! frame split, and the pack/byte_at byte semantics all match
//! `native/src/lib.rs` + `native/src/ws_client.rs` exactly.

#![allow(dead_code)] // exposed for codegen-emitted call sites

use loft::database::Stores;
use std::cell::RefCell;

// ── Host imports (module `loft_web`, provided by wasm/host.js) ──────────────
//
// Poll → len → copy → opcode, so opcode + arbitrary length both cross cleanly
// (a single-call fixed-scratch shape cannot carry the opcode and caps size).
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "loft_web")]
unsafe extern "C" {
    /// Open a WebSocket; returns a non-negative handle or -1 on a malformed URL.
    fn ws_connect(url_ptr: *const u8, url_len: usize) -> i32;
    /// Send a TEXT frame (opcode 1).  1 = sent / 0 = not open.
    fn ws_send(h: i32, ptr: *const u8, len: usize) -> i32;
    /// Send a BINARY frame (opcode 2).  1 = sent / 0 = not open.
    fn ws_send_binary(h: i32, ptr: *const u8, len: usize) -> i32;
    /// Dequeue the next inbound frame as the latched "current"; 1 = a frame
    /// is now current, 0 = nothing / connection down.
    fn ws_poll(h: i32) -> i32;
    /// Byte length of the latched current frame.
    fn ws_msg_len(h: i32) -> i32;
    /// Copy the latched current frame bytes into wasm memory at `ptr`.
    fn ws_msg_copy(h: i32, ptr: *mut u8);
    /// Opcode of the latched current frame: 1 = text, 2 = binary.
    fn ws_opcode(h: i32) -> i32;
    /// Close the socket.
    fn ws_close(h: i32);
    /// Asyncify suspend point: return to the JS event loop for one frame.
    /// Instrumented by `wasm-opt --asyncify --pass-arg=…,loft_web.ws_yield`.
    fn ws_yield();
}

// Non-wasm fallbacks so the crate compiles for `cargo check` on the host (the
// bridge is only ever *linked* for wasm32 by --html; these are never called).
#[cfg(not(target_arch = "wasm32"))]
mod host_stub {
    pub unsafe fn ws_connect(_: *const u8, _: usize) -> i32 {
        -1
    }
    pub unsafe fn ws_send(_: i32, _: *const u8, _: usize) -> i32 {
        0
    }
    pub unsafe fn ws_send_binary(_: i32, _: *const u8, _: usize) -> i32 {
        0
    }
    pub unsafe fn ws_poll(_: i32) -> i32 {
        0
    }
    pub unsafe fn ws_msg_len(_: i32) -> i32 {
        0
    }
    pub unsafe fn ws_msg_copy(_: i32, _: *mut u8) {}
    pub unsafe fn ws_opcode(_: i32) -> i32 {
        1
    }
    pub unsafe fn ws_close(_: i32) {}
    pub unsafe fn ws_yield() {}
}
#[cfg(not(target_arch = "wasm32"))]
use host_stub::*;

// ── Latched single-slot register (matches the native contract) ──────────────
// `recv` latches the current frame's bytes + opcode; `message()` / `opcode()`
// read them.  Not per-connection — the "recv then immediately read
// message/opcode before any other recv" contract holds across a yield because
// the yield happens at the loop top, after the read (plans/84 R5).
thread_local! {
    static LAST_MSG: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static LAST_OP: RefCell<i32> = const { RefCell::new(0) };
    static PACK_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

// ── WebSocket lifecycle ─────────────────────────────────────────────────────

/// `n_ws_connect(url) -> integer` — open a session; -1 on malformed URL.
pub fn ws_client_connect(_stores: &mut Stores, url: &str) -> i32 {
    unsafe { ws_connect(url.as_ptr(), url.len()) }
}

/// `n_ws_client_send(handle, msg) -> boolean` — send a TEXT frame.
pub fn ws_client_send(_stores: &mut Stores, handle: i32, msg: &str) -> bool {
    unsafe { ws_send(handle, msg.as_ptr(), msg.len()) == 1 }
}

/// `n_ws_client_send_binary(handle, msg) -> boolean` — send a BINARY frame
/// (opcode 2).  loft `text` is a byte buffer; the raw bytes ship verbatim, so
/// embedded zero bytes survive (C3).
pub fn ws_client_send_binary(_stores: &mut Stores, handle: i32, msg: &str) -> bool {
    unsafe { ws_send_binary(handle, msg.as_ptr(), msg.len()) == 1 }
}

/// `n_ws_client_recv(handle) -> boolean` — poll one frame; on success latch its
/// bytes + opcode for `message()` / `opcode()`.  Mirrors the native poll:
/// `true` ⇒ a TEXT/BINARY frame landed; `false` ⇒ nothing yet / down.
pub fn ws_client_recv(_stores: &mut Stores, handle: i32) -> bool {
    unsafe {
        if ws_poll(handle) != 1 {
            return false;
        }
        let n = ws_msg_len(handle);
        let mut v = vec![0u8; n.max(0) as usize];
        if !v.is_empty() {
            ws_msg_copy(handle, v.as_mut_ptr());
        }
        let op = ws_opcode(handle);
        LAST_MSG.with(|m| *m.borrow_mut() = v);
        LAST_OP.with(|o| *o.borrow_mut() = op);
    }
    true
}

/// `n_ws_client_message() -> text` — the latched current frame's bytes as a
/// `text`.  loft `text` is a byte buffer; `from_utf8_unchecked` preserves
/// arbitrary bytes (including NULs), matching the native binary path.
pub fn ws_client_message(_stores: &mut Stores) -> String {
    LAST_MSG.with(|m| unsafe { String::from_utf8_unchecked(m.borrow().clone()) })
}

/// `n_ws_client_opcode() -> integer` — opcode of the latched frame (1 text /
/// 2 binary).  The C3 fix: the native stub hardcoded 1, mangling binary frames.
pub fn ws_client_opcode(_stores: &mut Stores) -> i32 {
    LAST_OP.with(|o| *o.borrow())
}

/// `n_ws_client_close(handle)` — close the session.
pub fn ws_client_close(_stores: &mut Stores, handle: i32) {
    unsafe { ws_close(handle) }
}

/// `n_yield_frame()` — under `--html` this lowers (through the asyncify pass)
/// to the `loft_web.ws_yield` suspend import: it returns to the JS event loop
/// for one frame so `WebSocket.onmessage` can deliver, then resumes.  Native
/// `web::yield_frame()` is a no-op.
pub fn yield_frame(_stores: &mut Stores) {
    unsafe { ws_yield() }
}

/// `n_sleep_ms(ms)` — no-op on wasm (the host event loop is the pacing
/// mechanism; there is no thread to sleep).  A program that paces with
/// `sleep_ms` on native should `yield_frame` on the browser; this keeps the
/// shared source compiling unchanged.
pub fn sleep_ms(_stores: &mut Stores, _ms: i32) {}

// ── Pure-compute: pack builder + byte_at (no host import) ───────────────────
// Byte-identical semantics with native/src/lib.rs — these touch no host state.

/// `n_pack_reset()` — empty the per-thread pack buffer.
pub fn pack_reset(_stores: &mut Stores) {
    PACK_BUF.with(|b| b.borrow_mut().clear());
}

/// `n_pack_u8(b)` — append one byte (masked to 8 bits).
pub fn pack_u8(_stores: &mut Stores, b: i32) {
    PACK_BUF.with(|buf| buf.borrow_mut().push((b & 0xff) as u8));
}

/// `n_pack_u16_le(v)` — append a 2-byte little-endian value.
pub fn pack_u16_le(_stores: &mut Stores, v: i32) {
    PACK_BUF.with(|buf| buf.borrow_mut().extend_from_slice(&((v & 0xffff) as u16).to_le_bytes()));
}

/// `n_pack_u32_le(v)` — append a 4-byte little-endian value.  loft `integer`
/// is i32 → bit-cast to u32 to preserve the LE pattern across the sign edge.
pub fn pack_u32_le(_stores: &mut Stores, v: i32) {
    PACK_BUF.with(|buf| buf.borrow_mut().extend_from_slice(&(v as u32).to_le_bytes()));
}

/// `n_pack_take() -> text` — take the buffer as a `text` whose bytes are
/// exactly the buffer bytes (NULs preserved); resets the buffer.
pub fn pack_take(_stores: &mut Stores) -> String {
    PACK_BUF.with(|buf| {
        let v = std::mem::take(&mut *buf.borrow_mut());
        unsafe { String::from_utf8_unchecked(v) }
    })
}

/// `n_byte_at(idx, t) -> integer` — byte at `idx` of `t` (regardless of UTF-8
/// interpretation); -1 if out of range.  Used to decode binary frames.
pub fn byte_at(_stores: &mut Stores, idx: i32, t: &str) -> i32 {
    let bytes = t.as_bytes();
    if idx < 0 || (idx as usize) >= bytes.len() {
        return -1;
    }
    i32::from(bytes[idx as usize])
}

// ── WsGroup: client-side stub (R2) ──────────────────────────────────────────
// A browser client rarely multiplexes (the native sync-agent is the
// multiplexer).  Stub the group API: clear/add are no-ops; poll reports
// "nothing ready" so a group loop drains cleanly without trapping.  Revisit if
// a real browser-multiplexer use case appears.
pub fn ws_group_clear(_stores: &mut Stores) {}
pub fn ws_group_add(_stores: &mut Stores, _handle: i32) {}
pub fn ws_group_poll(_stores: &mut Stores) -> i32 {
    -1
}
