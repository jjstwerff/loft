// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! WASM entry point and host-bridge stubs for the `wasm` Cargo feature.
//!
//! Compiled only when `--features wasm` is active.  Each host-bridge function
//! corresponds to a JS-side counterpart on `globalThis.loftHost`.
//!
//! Steps: W1.1 (this stub) → W1.2 (output capture) → W1.3–W1.8 (bridges) → W1.9 (entry point).

// ── W1.7  File I/O host bridge stubs ─────────────────────────────────────────

/// Check whether a path exists in the virtual filesystem.
pub fn host_fs_exists(_path: &str) -> bool {
    // TODO W1.9: call extern "C" { fn fs_exists(ptr, len) -> bool; }
    false
}

/// Read an entire text file.  Returns empty string if absent.
pub fn host_fs_read_text(_path: &str) -> String {
    // TODO W1.9
    String::new()
}

/// Write `data` as text to `path`, creating or truncating.
pub fn host_fs_write_text(_path: &str, _data: &str) {}

/// Read raw bytes from `path`.  Returns empty Vec if absent.
pub fn host_fs_read_binary(_path: &str) -> Vec<u8> {
    // TODO W1.9
    Vec::new()
}

/// Write raw bytes to `path`, creating or truncating.
pub fn host_fs_write_binary(_path: &str, _data: &[u8]) {}

/// Delete `path`.  Returns true on success.
pub fn host_fs_delete(_path: &str) -> bool {
    false
}

/// Move / rename `from` to `to`.  Returns true on success.
pub fn host_fs_move(_from: &str, _to: &str) -> bool {
    false
}

/// Create a directory.  Returns true on success.
pub fn host_fs_mkdir(_path: &str) -> bool {
    false
}

/// Create a directory and all parents.  Returns true on success.
pub fn host_fs_mkdir_all(_path: &str) -> bool {
    false
}

/// Return a list of names inside `path` (directory listing).
pub fn host_fs_list_dir(_path: &str) -> Vec<String> {
    Vec::new()
}

/// Return `true` if `path` is a directory.
pub fn host_fs_is_dir(_path: &str) -> bool {
    false
}

/// Return `true` if `path` is a regular file.
pub fn host_fs_is_file(_path: &str) -> bool {
    false
}

/// Return the byte size of `path`, or -1 if absent.
pub fn host_fs_file_size(_path: &str) -> i64 {
    -1
}

// ── W1.6  Time and environment host bridges ──────────────────────────────────

/// Return the current time as milliseconds since the Unix epoch.
pub fn host_time_now() -> i64 {
    // TODO W1.9: call extern "C" { fn time_now() -> i64; }
    0
}

/// Return microseconds elapsed since some fixed start point (monotonic).
pub fn host_time_ticks() -> i64 {
    // TODO W1.9: call extern "C" { fn time_ticks() -> i64; }
    0
}

/// Return the value of environment variable `name`, or empty string if absent.
pub fn host_env_variable(_name: &str) -> String {
    // TODO W1.9: call extern "C" { fn env_variable(ptr, len) -> ... }
    String::new()
}

/// Return the command-line arguments (always empty under WASM).
pub fn host_arguments() -> Vec<String> {
    // TODO W1.9
    Vec::new()
}

/// Return the current working directory (empty under WASM).
pub fn host_fs_cwd() -> String {
    // TODO W1.9: call extern "C" { fn fs_cwd() -> ... }
    String::new()
}

/// Return the user home directory (empty under WASM).
pub fn host_fs_user_dir() -> String {
    // TODO W1.9: call extern "C" { fn fs_user_dir() -> ... }
    String::new()
}

/// Return the program executable directory (empty under WASM).
pub fn host_fs_program_dir() -> String {
    // TODO W1.9: call extern "C" { fn fs_program_dir() -> ... }
    String::new()
}

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
