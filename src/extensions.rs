// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native extension loader.
//!
//! Package native crates (cdylib) export `loft_register_v1`, a C-ABI function
//! that registers all native symbols with the interpreter via a callback.
//! Only C primitives cross the boundary — no Rust types are shared.
//!
//! See `EXTERNAL_LIBS.md` for the full design.

use std::collections::HashMap;
use std::sync::Mutex;

/// Wrapper for `*const ()` that is Send — function pointers from cdylibs are
/// valid for the process lifetime (the Library handle is leaked).
#[derive(Clone, Copy)]
struct FnPtr(*const ());
unsafe impl Send for FnPtr {}

/// Global registry of native function pointers loaded from cdylibs.
static NATIVE_REGISTRY: Mutex<Option<HashMap<String, FnPtr>>> = Mutex::new(None);

/// Look up a native function by symbol name and cast to the expected type.
/// Returns `None` if the symbol hasn't been loaded.
///
/// # Safety
/// The caller must ensure `T` matches the actual C-ABI signature of the symbol.
pub unsafe fn get_native_fn<T: Copy>(name: &str) -> Option<T> {
    let guard = NATIVE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let registry = guard.as_ref()?;
    let fp = registry.get(name)?;
    Some(unsafe { std::ptr::read(std::ptr::from_ref(&fp.0).cast::<T>()) })
}

/// The C-ABI registration callback type.
type RegisterFn =
    unsafe extern "C" fn(unsafe extern "C" fn(*const u8, usize, *const (), *mut ()), *mut ());

/// The registration callback: called once per symbol by the cdylib.
unsafe extern "C" fn collect(
    name_ptr: *const u8,
    name_len: usize,
    fn_ptr: *const (),
    ctx: *mut (),
) {
    let collected = unsafe { &mut *ctx.cast::<Vec<(String, *const ())>>() };
    let name = std::str::from_utf8(unsafe { std::slice::from_raw_parts(name_ptr, name_len) })
        .unwrap_or("<invalid>");
    collected.push((name.to_string(), fn_ptr));
}

/// Load all pending native extension libraries.
#[cfg(feature = "native-extensions")]
pub fn load_all(_state: &mut crate::state::State, paths: Vec<String>) {
    for path in paths {
        load_one(&path);
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn load_all(_state: &mut crate::state::State, _paths: Vec<String>) {}

/// Load a single native extension shared library.
///
/// The library must export `loft_register_v1`:
/// ```c
/// void loft_register_v1(
///     void (*register)(const char* name, size_t name_len, void* fn_ptr, void* ctx),
///     void* ctx
/// );
/// ```
#[cfg(feature = "native-extensions")]
fn load_one(path: &str) {
    use libloading::Library;
    use std::collections::HashSet;

    static LOAD_LOCK: Mutex<Option<HashSet<String>>> = Mutex::new(None);

    let canonical = std::fs::canonicalize(path)
        .unwrap_or_else(|_| std::path::PathBuf::from(path))
        .to_string_lossy()
        .into_owned();

    let mut guard = LOAD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let loaded = guard.get_or_insert_with(HashSet::new);
    if loaded.contains(&canonical) {
        return;
    }

    let lib = match unsafe { Library::new(path) } {
        Ok(l) => l,
        Err(e) => {
            eprintln!("loft: cannot load native extension '{path}': {e}");
            return;
        }
    };

    let register_sym = match unsafe { lib.get::<RegisterFn>(b"loft_register_v1\0") } {
        Ok(f) => *f,
        Err(e) => {
            eprintln!("loft: '{path}' does not export loft_register_v1: {e}");
            return;
        }
    };

    let mut collected: Vec<(String, *const ())> = Vec::new();
    unsafe {
        register_sym(collect, std::ptr::addr_of_mut!(collected).cast::<()>());
    }

    // Store in the global registry.
    let mut reg_guard = NATIVE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let registry = reg_guard.get_or_insert_with(HashMap::new);
    for (name, ptr) in collected {
        registry.insert(name, FnPtr(ptr));
    }

    loaded.insert(canonical);
    std::mem::forget(lib);
}

// ── Auto-build ──────────────────────────────────────────────────────────

/// Auto-build a package's native crate if the shared library is missing.
#[must_use]
pub fn auto_build_native(pkg_dir: &str, stem: &str) -> Option<String> {
    let cargo_toml = format!("{pkg_dir}/native/Cargo.toml");
    if !std::path::Path::new(&cargo_toml).exists() {
        return None;
    }
    let lib_name = platform_lib_name(stem);
    let release_path = format!("{pkg_dir}/native/target/release/{lib_name}");
    if std::path::Path::new(&release_path).exists() {
        return Some(release_path);
    }
    let debug_path = format!("{pkg_dir}/native/target/debug/{lib_name}");
    if std::path::Path::new(&debug_path).exists() {
        return Some(debug_path);
    }
    let built_path = release_path;
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "--manifest-path", &cargo_toml])
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    match status {
        Ok(s) if s.success() && std::path::Path::new(&built_path).exists() => Some(built_path),
        _ => None,
    }
}

/// Resolve the platform-correct shared-library filename from a stem.
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
