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

// ── W1.9  Virtual filesystem (VIRT_FS) ───────────────────────────────────────

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Per-thread virtual filesystem used by `compile_and_run()`.
    /// Maps filename → content.  Populated before parsing; cleared after execution.
    static VIRT_FS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Populate the virtual filesystem with the given `(name, content)` pairs.
pub fn virt_fs_populate(files: &[(String, String)]) {
    VIRT_FS.with(|fs| {
        let mut map = fs.borrow_mut();
        for (name, content) in files {
            map.insert(name.clone(), content.clone());
        }
    });
}

/// Return the content of `name` from the virtual filesystem, or `None` if absent.
pub fn virt_fs_get(name: &str) -> Option<String> {
    VIRT_FS.with(|fs| fs.borrow().get(name).cloned())
}

/// Clear all entries from the virtual filesystem.
pub fn virt_fs_clear() {
    VIRT_FS.with(|fs| fs.borrow_mut().clear());
}

// ── W1.9  compile_and_run() — WASM/native entry point ────────────────────────

/// Embedded default standard library files (compiled into the WASM binary).
const DEFAULT_FILES: &[(&str, &str)] = &[
    (
        "default/01_code.loft",
        include_str!("../default/01_code.loft"),
    ),
    (
        "default/02_images.loft",
        include_str!("../default/02_images.loft"),
    ),
    (
        "default/03_text.loft",
        include_str!("../default/03_text.loft"),
    ),
    (
        "default/04_stacktrace.loft",
        include_str!("../default/04_stacktrace.loft"),
    ),
    (
        "default/05_coroutine.loft",
        include_str!("../default/05_coroutine.loft"),
    ),
];

/// Run a loft program supplied as a JSON array of `{name, content}` file objects.
///
/// Returns a JSON string: `{"output": "...", "diagnostics": [...], "success": true|false}`.
///
/// The default standard library files are embedded in the binary; user files
/// are taken from `files_json`.  Any `use <id>;` statement is resolved against
/// files whose name matches `<id>.loft` in the supplied file list.
///
/// # Errors
/// Returns a JSON error object if `files_json` cannot be parsed.
///
/// When compiled with `--features wasm` and exported via `wasm-bindgen`, this
/// function is callable from JavaScript as:
/// ```js
/// const result = JSON.parse(loft.compile_and_run(JSON.stringify([
///   {name: 'main.loft', content: 'fn main() { println("hi") }'}
/// ])));
/// ```
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
pub fn compile_and_run(files_json: &str) -> String {
    // Parse the JSON input.
    let files = match parse_files_json(files_json) {
        Ok(f) => f,
        Err(e) => {
            return format!(
                "{{\"output\":\"\",\"diagnostics\":[{{\"level\":\"error\",\"message\":{:?}}}],\"success\":false}}",
                e
            );
        }
    };

    // Populate VIRT_FS with default files + user files.
    let mut all_files: Vec<(String, String)> = DEFAULT_FILES
        .iter()
        .map(|(n, c)| (n.to_string(), (*c).to_string()))
        .collect();
    for (name, content) in &files {
        all_files.push((name.clone(), content.clone()));
    }
    virt_fs_populate(&all_files);
    // Clear the output buffer.
    let _ = output_take();

    // Build and run.
    let success = run_pipeline();

    // Collect results.
    let output = output_take();
    virt_fs_clear();

    if success {
        format!(
            "{{\"output\":{},\"diagnostics\":[],\"success\":true}}",
            json_str(&output)
        )
    } else {
        format!(
            "{{\"output\":{},\"diagnostics\":[{{\"level\":\"error\",\"message\":\"compile or runtime error\"}}],\"success\":false}}",
            json_str(&output)
        )
    }
}

/// Parse `[{name: string, content: string}]` JSON into a Vec of pairs.
fn parse_files_json(json: &str) -> Result<Vec<(String, String)>, String> {
    let json = json.trim();
    if !json.starts_with('[') {
        return Err("expected JSON array".to_string());
    }
    // Minimal hand-rolled parser sufficient for well-formed wasm-bridge input.
    // Avoids pulling in a full JSON library.
    let mut result = Vec::new();
    let mut i = 1usize; // skip '['
    let bytes = json.as_bytes();
    let len = bytes.len();
    while i < len {
        // Skip whitespace and commas.
        while i < len && (bytes[i] == b' ' || bytes[i] == b',' || bytes[i] == b'\n') {
            i += 1;
        }
        if i >= len || bytes[i] == b']' {
            break;
        }
        if bytes[i] != b'{' {
            return Err(format!("unexpected char at {i}"));
        }
        let name = extract_json_field(json, &mut i, "name")?;
        let content = extract_json_field(json, &mut i, "content")?;
        result.push((name, content));
        // Skip closing '}'.
        while i < len && bytes[i] != b'}' {
            i += 1;
        }
        i += 1; // consume '}'
    }
    Ok(result)
}

/// Extract a `"key": "value"` pair from a JSON object string starting at `*pos`.
fn extract_json_field(json: &str, pos: &mut usize, key: &str) -> Result<String, String> {
    let key_pat = format!("\"{}\"", key);
    if let Some(k) = json[*pos..].find(&key_pat) {
        let after_key = *pos + k + key_pat.len();
        if let Some(colon) = json[after_key..].find(':') {
            let after_colon = after_key + colon + 1;
            return extract_json_string(json, after_colon);
        }
    }
    Err(format!("field '{key}' not found"))
}

/// Extract a JSON string value starting near `start`, returning the unescaped content.
fn extract_json_string(json: &str, start: usize) -> Result<String, String> {
    let s = json[start..].trim_start();
    let offset = start + (json[start..].len() - s.len());
    if !s.starts_with('"') {
        return Err("expected string".to_string());
    }
    let mut out = String::new();
    let mut chars = s[1..].chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Ok(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some(c) => out.push(c),
                None => return Err("unterminated escape".to_string()),
            },
            c => out.push(c),
        }
    }
    let _ = offset; // suppress unused warning
    Err("unterminated string".to_string())
}

/// Minimally escape a string for JSON output.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Execute the full pipeline using files in VIRT_FS.
/// Returns `true` on success.
fn run_pipeline() -> bool {
    use crate::compile::byte_code;
    use crate::parser::Parser;
    use crate::scopes;
    use crate::state::State;

    let mut p = Parser::new();
    // Parse default standard library (embedded in VIRT_FS under "default/").
    for (name, _) in DEFAULT_FILES {
        p.parse(name, true);
        if !p.diagnostics.is_empty() {
            return false;
        }
    }
    // Find the user's main file (first file not in default/).
    let main_name = VIRT_FS.with(|fs| {
        fs.borrow()
            .keys()
            .filter(|k| !k.starts_with("default/"))
            .min()
            .cloned()
    });
    let Some(main_name) = main_name else {
        return false;
    };
    if !p.parse(&main_name, false) {
        return false;
    }
    scopes::check(&mut p.data);
    if !p.diagnostics.is_empty() {
        return false;
    }
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute_argv("main", &p.data, &[]);
    true
}

// ── W1.2  Output capture ─────────────────────────────────────────────────────

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
