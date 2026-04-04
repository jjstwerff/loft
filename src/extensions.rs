// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native extension loader.
//!
//! Package native crates export pure C-ABI functions that the interpreter
//! loads via dlopen.  The cdylib never touches `Stores` — only primitives
//! cross the boundary.  See `EXTERNAL_LIBS.md` for the full design.

use std::sync::OnceLock;

// ── C-ABI function pointer types for imaging cdylib ─────────────────────

/// `loft_decode_png(path, len, &w, &h, &pixels, &pixels_len) -> bool`
pub type DecodePngFn =
    unsafe extern "C" fn(*const u8, usize, *mut u32, *mut u32, *mut *mut u8, *mut usize) -> bool;

/// `loft_free_pixels(ptr, len)`
pub type FreePixelsFn = unsafe extern "C" fn(*mut u8, usize);

static DECODE_PNG: OnceLock<DecodePngFn> = OnceLock::new();
static FREE_PIXELS: OnceLock<FreePixelsFn> = OnceLock::new();

/// Get the loaded PNG decode function, or `None` if the imaging cdylib
/// hasn't been loaded.
pub fn get_decode_png() -> Option<DecodePngFn> {
    DECODE_PNG.get().copied()
}

/// Get the pixel buffer free function.
pub fn get_free_pixels() -> Option<FreePixelsFn> {
    FREE_PIXELS.get().copied()
}

// ── C-ABI function pointer types for random cdylib ──────────────────────

pub type RandIntFn = extern "C" fn(i32, i32) -> i32;
pub type RandSeedFn = extern "C" fn(i64);
pub type RandIndicesFn = unsafe extern "C" fn(i32, *mut *mut i32, *mut usize);
pub type FreeIndicesFn = unsafe extern "C" fn(*mut i32, usize);

static RAND_INT: OnceLock<RandIntFn> = OnceLock::new();
static RAND_SEED: OnceLock<RandSeedFn> = OnceLock::new();
static RAND_INDICES: OnceLock<RandIndicesFn> = OnceLock::new();
static FREE_INDICES: OnceLock<FreeIndicesFn> = OnceLock::new();

pub fn get_rand_int() -> Option<RandIntFn> {
    RAND_INT.get().copied()
}
pub fn get_rand_seed() -> Option<RandSeedFn> {
    RAND_SEED.get().copied()
}
pub fn get_rand_indices() -> Option<RandIndicesFn> {
    RAND_INDICES.get().copied()
}
pub fn get_free_indices() -> Option<FreeIndicesFn> {
    FREE_INDICES.get().copied()
}

// ── Library loading ─────────────────────────────────────────────────────

/// Load all pending native extension libraries.
/// Resolves C-ABI symbols from each cdylib and stores function pointers.
#[cfg(feature = "native-extensions")]
pub fn load_all(_state: &mut crate::state::State, paths: Vec<String>) {
    for path in paths {
        load_one(&path);
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn load_all(_state: &mut crate::state::State, _paths: Vec<String>) {}

#[cfg(feature = "native-extensions")]
fn load_one(path: &str) {
    use libloading::Library;
    use std::collections::HashSet;
    use std::sync::Mutex;

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

    // Resolve known symbols.  Each package registers its symbols here.
    // This is explicit — no generic registration loop needed.
    if let Ok(f) = unsafe { lib.get::<DecodePngFn>(b"loft_decode_png\0") } {
        let _ = DECODE_PNG.set(*f);
    }
    if let Ok(f) = unsafe { lib.get::<FreePixelsFn>(b"loft_free_pixels\0") } {
        let _ = FREE_PIXELS.set(*f);
    }
    // Random symbols
    if let Ok(f) = unsafe { lib.get::<RandIntFn>(b"loft_rand_int\0") } {
        let _ = RAND_INT.set(*f);
    }
    if let Ok(f) = unsafe { lib.get::<RandSeedFn>(b"loft_rand_seed\0") } {
        let _ = RAND_SEED.set(*f);
    }
    if let Ok(f) = unsafe { lib.get::<RandIndicesFn>(b"loft_rand_indices\0") } {
        let _ = RAND_INDICES.set(*f);
    }
    if let Ok(f) = unsafe { lib.get::<FreeIndicesFn>(b"loft_free_indices\0") } {
        let _ = FREE_INDICES.set(*f);
    }

    loaded.insert(canonical);
    std::mem::forget(lib);
}

// ── Auto-build ──────────────────────────────────────────────────────────

/// Auto-build a package's native crate if the shared library is missing.
/// Returns the path to the built library, or `None` if no native source exists.
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
