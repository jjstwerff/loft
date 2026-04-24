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

/// Public version of `host_call` for use from `parallel.rs`.
#[cfg(feature = "wasm")]
pub fn host_call_raw(method: &str, args: &js_sys::Array) -> wasm_bindgen::JsValue {
    host_call(method, args)
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

/// Library files embedded in the WASM build so `use <name>;` resolves in the
/// browser sandbox without a native cdylib.
const BUNDLED_LIB_FILES: &[(&str, &str)] = &[
    (
        "graphics.loft",
        include_str!("../lib/graphics/src/graphics.loft"),
    ),
    ("math.loft", include_str!("../lib/graphics/src/math.loft")),
    ("mesh.loft", include_str!("../lib/graphics/src/mesh.loft")),
    ("scene.loft", include_str!("../lib/graphics/src/scene.loft")),
    (
        "render.loft",
        include_str!("../lib/graphics/src/render.loft"),
    ),
    ("glb.loft", include_str!("../lib/graphics/src/glb.loft")),
    ("shapes.loft", include_str!("../lib/shapes/src/shapes.loft")),
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
    #[cfg(feature = "wasm")]
    console_error_panic_hook::set_once();
    // Parse the JSON input.
    let files = match parse_files_json(files_json) {
        Ok(f) => f,
        Err(e) => {
            return format!(
                "{{\"output\":\"\",\"diagnostics\":{},\"success\":false}}",
                json_str(&e)
            );
        }
    };

    // Populate VIRT_FS with default files + graphics library + user files.
    let mut all_files: Vec<(String, String)> = DEFAULT_FILES
        .iter()
        .chain(BUNDLED_LIB_FILES.iter())
        .map(|(n, c)| (n.to_string(), (*c).to_string()))
        .collect();
    for (name, content) in &files {
        all_files.push((name.clone(), content.clone()));
    }
    virt_fs_populate(&all_files);
    // Clear the output buffer.
    let _ = output_take();

    // Build and run.
    let (diag, had_error, asserts) = run_pipeline();

    // Collect results.
    let output = output_take();
    virt_fs_clear();

    // Build asserts JSON array.
    let asserts_json = if asserts.is_empty() {
        "[]".to_string()
    } else {
        let items: Vec<String> = asserts
            .iter()
            .map(|(pass, msg, file, line)| {
                format!(
                    "{{\"pass\":{pass},\"message\":{},\"file\":{},\"line\":{line}}}",
                    json_str(msg),
                    json_str(file),
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    };

    format!(
        "{{\"output\":{},\"diagnostics\":{},\"asserts\":{asserts_json},\"success\":{}}}",
        json_str(&output),
        json_str(&diag),
        !had_error,
    )
}

// ── FY.2–FY.3  Game session with frame yield ────────────────────────────────

/// Persistent game session that survives across frame yields.
/// Owns State and Data so raw pointers inside State remain valid.
struct GameSession {
    state: crate::state::State,
    // Kept alive for State's borrowed pointers; never read directly.
    #[allow(dead_code)]
    data: crate::data::Data,
}

thread_local! {
    static GAME_SESSION: RefCell<Option<GameSession>> = RefCell::new(None);
}

/// Start a game session: parse, compile, execute until the first frame yield.
/// Returns JSON `{"ok":true}` on success or `{"ok":false,"error":"..."}` on failure.
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
pub fn compile_and_start(files_json: &str) -> String {
    #[cfg(feature = "wasm")]
    console_error_panic_hook::set_once();
    // Dispose previous session.
    GAME_SESSION.with(|gs| {
        if gs.borrow().is_some() {
            #[cfg(feature = "wasm")]
            {
                let args = js_sys::Array::new();
                host_call_raw("gl_destroy_window", &args);
            }
            *gs.borrow_mut() = None;
        }
    });

    let files = match parse_files_json(files_json) {
        Ok(f) => f,
        Err(e) => return format!("{{\"ok\":false,\"error\":{}}}", json_str(&e)),
    };

    let mut all_files: Vec<(String, String)> = DEFAULT_FILES
        .iter()
        .chain(BUNDLED_LIB_FILES.iter())
        .map(|(n, c)| (n.to_string(), (*c).to_string()))
        .collect();
    for (name, content) in &files {
        all_files.push((name.clone(), content.clone()));
    }
    virt_fs_populate(&all_files);
    let _ = output_take();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use crate::compile::byte_code;
        use crate::diagnostics::Level;
        use crate::parser::Parser;
        use crate::scopes;
        use crate::state::State;

        let mut p = Parser::new();
        for (name, _) in DEFAULT_FILES {
            p.parse(name, true);
            if p.diagnostics.level() >= Level::Error {
                return Err(p.diagnostics.to_string());
            }
        }
        let lib_set: std::collections::HashSet<&str> =
            BUNDLED_LIB_FILES.iter().map(|(n, _)| *n).collect();
        let main_name = VIRT_FS.with(|fs| {
            fs.borrow()
                .keys()
                .filter(|k| !k.starts_with("default/") && !lib_set.contains(k.as_str()))
                .min()
                .cloned()
        });
        let Some(main_name) = main_name else {
            return Err("no user file found".to_string());
        };
        p.parse(&main_name, false);
        if p.diagnostics.level() >= Level::Error {
            return Err(p.diagnostics.to_string());
        }
        scopes::check(&mut p.data);
        if p.diagnostics.level() >= Level::Error {
            return Err(p.diagnostics.to_string());
        }
        let mut state = State::new(p.database);
        byte_code(&mut state, &mut p.data);
        crate::wasm_gl::register_wgl_natives(&mut state);
        state.execute_argv("main", &p.data, &[]);
        // execute_argv returns either because the program finished or because
        // frame_yield was set.  Store the session for resume_frame.
        Ok(GameSession {
            state,
            data: p.data,
        })
    }));

    virt_fs_clear();

    match result {
        Ok(Ok(session)) => {
            let yielded = session.state.database.frame_yield;
            GAME_SESSION.with(|gs| *gs.borrow_mut() = Some(session));
            if yielded {
                format!("{{\"ok\":true,\"running\":true}}")
            } else {
                let out = output_take();
                GAME_SESSION.with(|gs| *gs.borrow_mut() = None);
                format!(
                    "{{\"ok\":true,\"running\":false,\"output\":{}}}",
                    json_str(&out)
                )
            }
        }
        Ok(Err(diag)) => {
            format!("{{\"ok\":false,\"error\":{}}}", json_str(&diag))
        }
        Err(_panic) => {
            GAME_SESSION.with(|gs| *gs.borrow_mut() = None);
            format!("{{\"ok\":false,\"error\":\"internal panic\"}}")
        }
    }
}

/// Resume execution after a frame yield.  Returns JSON:
/// `{"running":true}` — yielded again, call on next requestAnimationFrame
/// `{"running":false,"output":"..."}` — program finished
/// `{"running":false,"error":"..."}` — program crashed
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
pub fn resume_frame() -> String {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        GAME_SESSION.with(|gs| {
            let mut slot = gs.borrow_mut();
            let Some(session) = slot.as_mut() else {
                return format!("{{\"running\":false}}");
            };
            let still_running = session.state.resume();
            if still_running {
                format!("{{\"running\":true}}")
            } else {
                let out = output_take();
                *slot = None;
                format!("{{\"running\":false,\"output\":{}}}", json_str(&out))
            }
        })
    }));
    match result {
        Ok(json) => json,
        Err(_panic) => {
            GAME_SESSION.with(|gs| *gs.borrow_mut() = None);
            #[cfg(feature = "wasm")]
            {
                let args = js_sys::Array::new();
                host_call_raw("gl_destroy_window", &args);
            }
            format!("{{\"running\":false,\"error\":\"internal panic\"}}")
        }
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
        while i < len && matches!(bytes[i], b' ' | b',' | b'\n' | b'\r' | b'\t') {
            i += 1;
        }
        if i >= len || bytes[i] == b']' {
            break;
        }
        if bytes[i] != b'{' {
            return Err(format!("unexpected char at {i}"));
        }
        i += 1; // consume '{'
        let name = extract_json_field(json, &mut i, "name")?;
        let content = extract_json_field(json, &mut i, "content")?;
        result.push((name, content));
        // Advance past any remaining fields to the closing '}'.
        while i < len && bytes[i] != b'}' {
            i += 1;
        }
        if i < len {
            i += 1; // consume '}'
        }
    }
    Ok(result)
}

/// Extract a `"key": "value"` pair from a JSON object string starting at `*pos`.
/// Advances `*pos` to just past the closing `"` of the value.
fn extract_json_field(json: &str, pos: &mut usize, key: &str) -> Result<String, String> {
    let key_pat = format!("\"{}\"", key);
    if let Some(k) = json[*pos..].find(&key_pat) {
        let after_key = *pos + k + key_pat.len();
        if let Some(colon) = json[after_key..].find(':') {
            let after_colon = after_key + colon + 1;
            let (value, end) = extract_json_string(json, after_colon)?;
            *pos = end;
            return Ok(value);
        }
    }
    Err(format!("field '{key}' not found"))
}

/// Extract a JSON string value starting near `start`.
/// Returns `(unescaped_content, byte_position_after_closing_quote)`.
fn extract_json_string(json: &str, start: usize) -> Result<(String, usize), String> {
    let slice = &json[start..];
    let trimmed = slice.trim_start();
    let offset = start + (slice.len() - trimmed.len()); // absolute position of opening '"'
    if !trimmed.starts_with('"') {
        return Err("expected string".to_string());
    }
    let inner = &trimmed[1..]; // skip opening '"'
    let mut out = String::new();
    let mut chars = inner.char_indices();
    while let Some((byte_off, c)) = chars.next() {
        match c {
            '"' => {
                // byte_off is relative to inner; +1 for opening '"', +1 past closing '"'
                return Ok((out, offset + 1 + byte_off + 1));
            }
            '\\' => match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, 'r')) => out.push('\r'),
                Some((_, '"')) => out.push('"'),
                Some((_, '\\')) => out.push('\\'),
                Some((_, '/')) => out.push('/'),
                Some((_, c)) => out.push(c),
                None => return Err("unterminated escape".to_string()),
            },
            c => out.push(c),
        }
    }
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
/// Returns `(diagnostic_string, had_error)`.  Warnings produce a non-empty
/// diagnostic string but `had_error = false`; errors set `had_error = true`.
/// Assert result: (passed, message, file, line).
type AssertResult = (bool, String, String, u32);

fn run_pipeline() -> (String, bool, Vec<AssertResult>) {
    use crate::compile::byte_code;
    use crate::diagnostics::Level;
    use crate::parser::Parser;
    use crate::scopes;
    use crate::state::State;

    let mut p = Parser::new();
    for (name, _) in DEFAULT_FILES {
        p.parse(name, true);
        let lvl = p.diagnostics.level();
        if lvl == Level::Error || lvl == Level::Fatal {
            return (p.diagnostics.to_string(), true, Vec::new());
        }
    }
    let lib_names: std::collections::HashSet<&str> =
        BUNDLED_LIB_FILES.iter().map(|(n, _)| *n).collect();
    let main_name = VIRT_FS.with(|fs| {
        fs.borrow()
            .keys()
            .filter(|k| !k.starts_with("default/") && !lib_names.contains(k.as_str()))
            .min()
            .cloned()
    });
    let Some(main_name) = main_name else {
        return ("no user file found".to_string(), true, Vec::new());
    };
    p.parse(&main_name, false);
    let lvl = p.diagnostics.level();
    if lvl == Level::Error || lvl == Level::Fatal {
        return (p.diagnostics.to_string(), true, Vec::new());
    }
    scopes::check(&mut p.data);
    let lvl = p.diagnostics.level();
    if lvl == Level::Error || lvl == Level::Fatal {
        return (p.diagnostics.to_string(), true, Vec::new());
    }
    let diag = p.diagnostics.to_string();
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    // GL6.1–GL6.3: register WebGL bridge functions for graphics library.
    crate::wasm_gl::register_wgl_natives(&mut state);
    // Enable assert reporting for the playground.
    state.database.report_asserts = true;
    state.execute_argv("main", &p.data, &[]);
    let asserts = std::mem::take(&mut state.database.assert_results);
    let had_fatal = state.database.had_fatal;
    (diag, had_fatal, asserts)
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

// ── W1.18-2  Worker entry point for WASM threading ──────────────────────────

/// Entry point called by each Worker Thread.  The JS worker loop calls
/// this with the function index and element range.  The worker reads from the
/// shared WASM memory (Store heap) and writes results directly back.
///
/// This is a no-op stub until the wasm-threads feature build is available.
/// The actual implementation needs access to the shared State, which requires
/// the wasm-threads + atomics build flags.
#[cfg(feature = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn worker_entry(_fn_index: u32, _start: u32, _end: u32) {
    // TODO(W1.18-2): implement when wasm-threads feature build is available.
    // The worker needs to:
    // 1. Access the shared Store heap via WASM linear memory
    // 2. Create a lightweight State for bytecode execution
    // 3. Loop from start..end, executing fn_index for each element
    // 4. Write results directly to the shared output buffer
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
