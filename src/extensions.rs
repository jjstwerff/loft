// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A7.2 — Native extension loader.
//!
//! Loads platform shared libraries that expose a `loft_register_v1` C-ABI
//! entry point and registers their native functions with the interpreter via
//! `State::static_fn()`.
//!
//! The feature is gated behind the `native-extensions` Cargo feature flag to
//! keep the default build free of the `libloading` dependency.

use crate::state::State;

/// Load all pending native extension libraries into `state`.
///
/// Called from `main.rs` between `byte_code()` and `state.execute_argv()`.
/// `paths` comes from `Parser::pending_native_libs`.
#[cfg(feature = "native-extensions")]
pub fn load_all(state: &mut State, paths: Vec<String>) {
    for path in paths {
        load_one(state, &path);
    }
}

/// No-op when `native-extensions` feature is disabled.
#[cfg(not(feature = "native-extensions"))]
pub fn load_all(_state: &mut State, _paths: Vec<String>) {}

/// Load a single native extension shared library at `path` and register all
/// functions it exposes via its `loft_register_v1` entry point.
///
/// # Panics
/// Panics if the library cannot be opened or does not export `loft_register_v1`.
#[cfg(feature = "native-extensions")]
pub fn load_one(state: &mut State, path: &str) {
    use crate::database::Stores;
    use crate::keys::DbRef;
    use crate::state::Call;
    use libloading::{Library, Symbol};
    use std::ffi::CStr;
    use std::os::raw::c_char;

    #[repr(C)]
    struct LoftPluginCtx {
        _state: *mut (),
        register_fn: unsafe extern "C" fn(
            ctx: *mut LoftPluginCtx,
            name: *const c_char,
            func: unsafe extern "C" fn(*mut (), *mut ()),
        ),
        _staged: *mut (),
    }

    unsafe extern "C" fn trampoline_register(
        ctx: *mut LoftPluginCtx,
        name: *const c_char,
        func: unsafe extern "C" fn(*mut (), *mut ()),
    ) {
        let staged = unsafe { &mut *((*ctx)._staged as *mut Vec<(String, Call)>) };
        let name = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        let call: Call = unsafe { std::mem::transmute(func) };
        staged.push((name, call));
    }

    let lib = unsafe { Library::new(path) }
        .unwrap_or_else(|e| panic!("loft: failed to open native extension '{path}': {e}"));

    let mut staged: Vec<(String, Call)> = Vec::new();
    let mut ctx = LoftPluginCtx {
        _state: std::ptr::null_mut(),
        register_fn: trampoline_register,
        _staged: &mut staged as *mut _ as *mut (),
    };

    {
        let register: Symbol<unsafe extern "C" fn(*mut LoftPluginCtx)> =
            unsafe { lib.get(b"loft_register_v1\0") }.unwrap_or_else(|e| {
                panic!("loft: native extension '{path}' does not export loft_register_v1: {e}")
            });
        unsafe { register(&mut ctx) };
    }

    for (name, call) in staged {
        state.static_fn(&name, call);
    }

    // Keep the library alive for the interpreter's lifetime.
    // Leak it intentionally — the process exits before cleanup matters.
    std::mem::forget(lib);
}

/// No-op stub when `native-extensions` feature is disabled.
#[cfg(not(feature = "native-extensions"))]
#[allow(dead_code)]
pub fn load_one(_state: &mut State, _path: &str) {}

/// Resolve the platform-correct shared-library filename from a stem.
///
/// `stem` is the value from `loft.toml [library] native = "..."`.
///
/// | OS      | Result            |
/// |---------|-------------------|
/// | Linux   | `lib<stem>.so`    |
/// | macOS   | `lib<stem>.dylib` |
/// | Windows | `<stem>.dll`      |
#[must_use]
pub fn platform_lib_name(stem: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(windows) {
        format!("{stem}.dll")
    } else {
        format!("lib{stem}.so")
    }
}
