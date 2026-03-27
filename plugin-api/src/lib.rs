// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! # loft-plugin-api
//!
//! Stable C-ABI interface for writing native extension plugins for the
//! [loft](https://github.com/jjstwerff/loft) scripting language interpreter.
//!
//! ## Usage
//!
//! A plugin crate depends on this crate and the `loft` rlib (for `Stores`
//! and `DbRef`), exposes a `#[no_mangle] pub extern "C" fn loft_register_v1`
//! entry point, and is compiled as a `cdylib`.
//!
//! ```rust,ignore
//! use loft::database::Stores;
//! use loft::keys::DbRef;
//! use loft_plugin_api::LoftPluginCtx;
//!
//! fn n_my_fn(stores: &mut Stores, stack: &mut DbRef) {
//!     let x = *stores.get::<i32>(stack);
//!     stores.put(stack, x + 1);
//! }
//!
//! #[no_mangle]
//! pub extern "C" fn loft_register_v1(ctx: *mut LoftPluginCtx) {
//!     unsafe {
//!         let r = (*ctx).register_fn;
//!         r(ctx, b"n_my_fn\0".as_ptr() as _, n_my_fn as _);
//!     }
//! }
//! ```
//!
//! ## ABI Stability
//!
//! `LoftPluginCtx` is `repr(C)` and append-only.  New fields may be added at
//! the end in minor versions.  Plugins compiled against an older minor version
//! remain binary-compatible.
//!
//! The exported symbol name encodes the major version:
//! - `loft_register_v1` — for interpreter 1.x
//! - `loft_register_v2` — for interpreter 2.x (if ABI breaks)

use std::ffi::c_char;

/// C-ABI context passed to `loft_register_v1` in a native extension.
///
/// Plugins receive a `*mut LoftPluginCtx` and call `register_fn` once for
/// each native function they expose.  Fields beginning with `_` are internal
/// to the interpreter and must not be read or written by plugins.
#[repr(C)]
pub struct LoftPluginCtx {
    /// Reserved for future interpreter state pointer.  Must be null.
    /// Plugins must not dereference it.
    pub _state: *mut (),
    /// Call this once per native function to register it.
    ///
    /// `name` is a null-terminated C string following the loft naming
    /// convention (`n_<fn>` for globals, `t_<N><Type>_<method>` for methods).
    /// `func` is cast from `fn(&mut Stores, &mut DbRef)` at the plugin side.
    pub register_fn: unsafe extern "C" fn(
        ctx: *mut LoftPluginCtx,
        name: *const c_char,
        func: unsafe extern "C" fn(*mut (), *mut ()),
    ),
    /// Internal staging pointer used by the interpreter trampoline.
    /// Must not be accessed by plugins.
    pub _staged: *mut (),
}

/// # Safety
///
/// `LoftPluginCtx` contains raw pointers.  The interpreter constructs one on
/// the stack before calling `loft_register_v1` and all pointers are valid for
/// the duration of that call only.
unsafe impl Send for LoftPluginCtx {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn register_fn_dispatches_correctly() {
        let mut captured: Vec<(String, usize)> = Vec::new();

        unsafe extern "C" fn capture(
            ctx: *mut LoftPluginCtx,
            name: *const c_char,
            func: unsafe extern "C" fn(*mut (), *mut ()),
        ) {
            let v = unsafe { &mut *((*ctx)._staged as *mut Vec<(String, usize)>) };
            let n = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
            v.push((n, func as usize));
        }

        unsafe extern "C" fn dummy(_: *mut (), _: *mut ()) {}

        let name = CString::new("n_example").unwrap();
        let mut ctx = LoftPluginCtx {
            _state: std::ptr::null_mut(),
            register_fn: capture,
            _staged: &mut captured as *mut _ as *mut (),
        };
        unsafe { (ctx.register_fn)(&mut ctx, name.as_ptr(), dummy) };

        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, "n_example");
        assert_eq!(captured[0].1, dummy as usize);
    }
}
