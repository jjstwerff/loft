// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! WASM entry point and host-bridge stubs for the `wasm` Cargo feature.
//!
//! Compiled only when `--features wasm` is active.  Each host-bridge function
//! corresponds to a JS-side counterpart on `globalThis.loftHost`.
//!
//! Steps: W1.1 (this stub) → W1.2 (output capture) → W1.3–W1.8 (bridges) →
//!        W1.9 (entry point) → W1.16 (file I/O, FS-A … FS-F).
//!
//! FS-A (this file): every stub calls `globalThis.loftHost.*` via `js_sys::Reflect`
//! when compiled under `--features wasm`.  Under the default feature set the stubs
//! continue to return the same harmless defaults as before, so native tests are
//! unaffected.

// ── FS-A  js_sys call helpers (wasm only) ────────────────────────────────────

/// Return the `globalThis.loftHost` object.
#[cfg(feature = "wasm")]
fn loft_host() -> wasm_bindgen::JsValue {
    js_sys::Reflect::get(&js_sys::global(), &"loftHost".into())
        .unwrap_or(wasm_bindgen::JsValue::UNDEFINED)
}

/// Call `globalThis.loftHost[method](args…)`.  Returns `JsValue::UNDEFINED` on error.
#[cfg(feature = "wasm")]
fn host_call(method: &str, args: &js_sys::Array) -> wasm_bindgen::JsValue {
    let host = loft_host();
    let func: wasm_bindgen::JsValue =
        js_sys::Reflect::get(&host, &method.into()).unwrap_or(wasm_bindgen::JsValue::UNDEFINED);
    js_sys::Function::from(func)
        .apply(&host, args)
        .unwrap_or(wasm_bindgen::JsValue::UNDEFINED)
}

// ── W1.7 / FS-A  File I/O host bridge ────────────────────────────────────────

/// Check whether a path exists in the virtual filesystem.
pub fn host_fs_exists(path: &str) -> bool {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_exists", &args).as_bool().unwrap_or(false)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        false
    }
}

/// Read an entire text file.  Returns `None` if absent.
pub fn host_fs_read_text(path: &str) -> Option<String> {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        let v = host_call("fs_read_text", &args);
        if v.is_null() || v.is_undefined() {
            None
        } else {
            v.as_string()
        }
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        None
    }
}

/// Write `data` as text to `path`, creating or truncating.  Returns 0 on success.
pub fn host_fs_write_text(path: &str, data: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&path.into(), &data.into());
        host_call("fs_write_text", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (path, data);
        0
    }
}

/// Read raw bytes from `path`.  Returns `None` if absent.
pub fn host_fs_read_binary(path: &str) -> Option<Vec<u8>> {
    #[cfg(feature = "wasm")]
    {
        use wasm_bindgen::JsCast;
        let args = js_sys::Array::of3(&path.into(), &0.into(), &i32::MAX.into());
        let v = host_call("fs_read_binary", &args);
        if v.is_null() || v.is_undefined() {
            None
        } else if let Ok(arr) = v.dyn_into::<js_sys::Uint8Array>() {
            Some(arr.to_vec())
        } else {
            None
        }
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        None
    }
}

/// Write raw bytes to `path`, creating or truncating.  Returns 0 on success.
pub fn host_fs_write_binary(path: &str, data: &[u8]) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let arr = js_sys::Uint8Array::from(data);
        let args = js_sys::Array::of2(&path.into(), &arr.into());
        host_call("fs_write_binary", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (path, data);
        0
    }
}

/// Delete `path`.  Returns 0 on success, non-zero on error.
pub fn host_fs_delete(path: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_delete", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        1
    }
}

/// Move / rename `from` to `to`.  Returns 0 on success.
pub fn host_fs_move(from: &str, to: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&from.into(), &to.into());
        host_call("fs_move", &args).as_f64().map_or(5, |v| v as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (from, to);
        1
    }
}

/// Create a directory.  Returns 0 on success.
pub fn host_fs_mkdir(path: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_mkdir", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        1
    }
}

/// Create a directory and all parents.  Returns 0 on success.
pub fn host_fs_mkdir_all(path: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_mkdir_all", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        1
    }
}

/// Return a list of names inside `path` (directory listing).
pub fn host_fs_list_dir(path: &str) -> Vec<String> {
    #[cfg(feature = "wasm")]
    {
        use wasm_bindgen::JsCast;
        let args = js_sys::Array::of1(&path.into());
        let v = host_call("fs_list_dir", &args);
        if let Ok(arr) = v.dyn_into::<js_sys::Array>() {
            arr.iter().filter_map(|x| x.as_string()).collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        Vec::new()
    }
}

/// Return `true` if `path` is a directory.
pub fn host_fs_is_dir(path: &str) -> bool {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_is_dir", &args).as_bool().unwrap_or(false)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        false
    }
}

/// Return `true` if `path` is a regular file.
pub fn host_fs_is_file(path: &str) -> bool {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_is_file", &args).as_bool().unwrap_or(false)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        false
    }
}

/// Return the byte size of `path`, or -1 if absent.
pub fn host_fs_file_size(path: &str) -> i64 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_file_size", &args)
            .as_f64()
            .map_or(-1, |v| v as i64)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        -1
    }
}

/// Seek the JS-side binary cursor for `path` to `pos`.
pub fn host_fs_seek(path: &str, pos: i64) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&path.into(), &(pos as f64).into());
        host_call("fs_seek", &args);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (path, pos);
    }
}

/// Read `n` bytes from the JS-side cursor position for `path`.  Advances the cursor.
pub fn host_fs_read_bytes(path: &str, n: usize) -> Option<Vec<u8>> {
    #[cfg(feature = "wasm")]
    {
        use wasm_bindgen::JsCast;
        let args = js_sys::Array::of2(&path.into(), &(n as f64).into());
        let v = host_call("fs_read_bytes", &args);
        if v.is_null() || v.is_undefined() {
            None
        } else if let Ok(arr) = v.dyn_into::<js_sys::Uint8Array>() {
            Some(arr.to_vec())
        } else {
            None
        }
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (path, n);
        None
    }
}

/// Write `bytes` at the JS-side cursor position for `path`.  Advances the cursor.
pub fn host_fs_write_bytes(path: &str, bytes: &[u8]) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let arr = js_sys::Uint8Array::from(bytes);
        let args = js_sys::Array::of2(&path.into(), &arr.into());
        host_call("fs_write_bytes", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (path, bytes);
        0
    }
}

/// Return the current JS-side cursor position for `path`.
pub fn host_fs_get_cursor(path: &str) -> i64 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_get_cursor", &args)
            .as_f64()
            .map_or(0, |v| v as i64)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        0
    }
}

// ── W1.6  Time and environment host bridges ──────────────────────────────────

/// Return the current time as milliseconds since the Unix epoch.
pub fn host_time_now() -> i64 {
    #[cfg(feature = "wasm")]
    {
        host_call("time_now", &js_sys::Array::new())
            .as_f64()
            .map_or(0, |v| v as i64)
    }
    #[cfg(not(feature = "wasm"))]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Return the current time as milliseconds since the Unix epoch (monotonic approximation).
pub fn host_time_ticks() -> i64 {
    #[cfg(feature = "wasm")]
    {
        host_call("time_ticks", &js_sys::Array::new())
            .as_f64()
            .map_or(0, |v| v as i64)
    }
    #[cfg(not(feature = "wasm"))]
    host_time_now()
}

/// Return the value of environment variable `name`, or empty string if absent.
pub fn host_env_variable(name: &str) -> String {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&name.into());
        host_call("env_variable", &args)
            .as_string()
            .unwrap_or_default()
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = name;
        String::new()
    }
}

/// Return the command-line arguments (always empty under WASM).
pub fn host_arguments() -> Vec<String> {
    #[cfg(feature = "wasm")]
    {
        use wasm_bindgen::JsCast;
        let v = host_call("arguments", &js_sys::Array::new());
        if let Ok(arr) = v.dyn_into::<js_sys::Array>() {
            arr.iter().filter_map(|x| x.as_string()).collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    }
    #[cfg(not(feature = "wasm"))]
    Vec::new()
}

/// Return the current working directory.
pub fn host_fs_cwd() -> String {
    #[cfg(feature = "wasm")]
    {
        host_call("fs_cwd", &js_sys::Array::new())
            .as_string()
            .unwrap_or_default()
    }
    #[cfg(not(feature = "wasm"))]
    String::new()
}

/// Return the user home directory.
pub fn host_fs_user_dir() -> String {
    #[cfg(feature = "wasm")]
    {
        host_call("fs_user_dir", &js_sys::Array::new())
            .as_string()
            .unwrap_or_default()
    }
    #[cfg(not(feature = "wasm"))]
    String::new()
}

/// Return the program executable directory.
pub fn host_fs_program_dir() -> String {
    #[cfg(feature = "wasm")]
    {
        host_call("fs_program_dir", &js_sys::Array::new())
            .as_string()
            .unwrap_or_default()
    }
    #[cfg(not(feature = "wasm"))]
    String::new()
}

// ── W1.5  Random host bridge ─────────────────────────────────────────────────

/// Return a random integer in `[lo, hi]` inclusive.
pub fn host_random_int(lo: i32, hi: i32) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&lo.into(), &hi.into());
        host_call("random_int", &args)
            .as_f64()
            .map_or(lo, |v| v as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        lo.max(hi)
    }
}

/// Reseed the host-side RNG.
pub fn host_random_seed(seed: i64) {
    #[cfg(feature = "wasm")]
    {
        let hi = ((seed >> 32) as i32).into();
        let lo = (seed as i32).into();
        let args = js_sys::Array::of2(&hi, &lo);
        host_call("random_seed", &args);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = seed;
    }
}

// ── W1.4  Logger host bridge ─────────────────────────────────────────────────

/// Write a log line to the host console.
pub fn host_log_write(line: &str) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&line.into());
        host_call("log_write", &args);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = line;
    }
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
