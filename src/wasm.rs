// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! WASM entry point and host-bridge stubs for the `wasm` Cargo feature.
//!
//! Compiled only when `--features wasm` is active.  Each host-bridge function
//! corresponds to a JS-side counterpart on `globalThis.loftHost`.
//!
//! Steps: W1.1 (this stub) → W1.2 (output capture) → W1.3–W1.8 (bridges) → W1.9 (entry point).

// ── W1.5  Random host bridge ─────────────────────────────────────────────────

/// Return a random integer in `[lo, hi]` inclusive.  Called when `wasm` is
/// enabled and `random` is not — the host provides the RNG.
pub fn host_random_int(lo: i32, hi: i32) -> i32 {
    // TODO W1.9: call extern "C" { fn random_int(lo: i32, hi: i32) -> i32; }
    // Placeholder: return lo so the interpreter does not panic.
    lo.max(hi)
}

/// Reseed the host-side RNG.  Called when `wasm` is enabled and `random` is not.
pub fn host_random_seed(_seed: i64) {
    // TODO W1.9: call extern "C" { fn random_seed(seed: i64); }
}

// ── W1.4  Logger host bridge ─────────────────────────────────────────────────

/// Write a log line to the host console.  Under WASM the real filesystem is
/// unavailable; this stub forwards the message to `globalThis.loftHost.log_write`
/// (wired up in W1.9) or does nothing when the host bridge is not yet set up.
pub fn host_log_write(_line: &str) {
    // TODO W1.9: call extern "C" { fn host_log_write(ptr: *const u8, len: usize); }
}

// ── W1.2  Output capture ─────────────────────────────────────────────────────

use std::cell::RefCell;

thread_local! {
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Append `text` to the per-thread output buffer.
pub fn output_push(text: &str) {
    OUTPUT.with(|buf| buf.borrow_mut().push_str(text));
}

/// Drain and return the accumulated output since the last call.
pub fn output_take() -> String {
    OUTPUT.with(|buf| std::mem::take(&mut *buf.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_capture() {
        output_push("hello ");
        output_push("world");
        assert_eq!(output_take(), "hello world");
        assert_eq!(output_take(), ""); // cleared after take
    }
}
