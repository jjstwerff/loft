// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 arc D / D2b — opt-in stdlib startup cache wiring.
//!
//! On a **warm** run (the `LOFT_STDLIB_CACHE` env var set and a valid bundle on
//! disk), [`warm_load_stdlib`] mmaps the precompiled stdlib bundle — the native
//! [`crate::data::Data`] **and** its database type schema — into the parser,
//! skipping the parse of `default/` entirely (~12× faster, see § arc D probe).
//! On a **cold** run, [`save_stdlib_cache`] writes that bundle after the parse.
//!
//! The cache file is keyed by [`crate::cache::stdlib_cache_key`] (stdlib
//! content, loft version, build id, target, feature set), so any stdlib edit
//! or toolchain bump lands at a fresh path and the stale bundle is simply
//! never read.  Default behaviour (env var unset) is unchanged: both functions
//! are no-ops, so normal runs always parse.

use crate::parser::Parser;

/// The cache-bundle path when the cache is enabled and the stdlib is readable;
/// `None` to disable (env var unset/empty, or `default/` unreadable).
#[cfg(feature = "mmap")]
fn cache_target(default_dir: &str) -> Option<std::path::PathBuf> {
    if std::env::var_os("LOFT_STDLIB_CACHE").is_none_or(|v| v.is_empty()) {
        return None;
    }
    let srcs = crate::cache::collect_stdlib_sources(default_dir);
    if srcs.is_empty() {
        return None;
    }
    Some(crate::cache::stdlib_cache_path(
        &crate::cache::stdlib_cache_key(&srcs),
    ))
}

/// Warm path: if the cache is enabled and a valid bundle exists, load the stdlib
/// `Data` + type schema into `p` and return `true` (the caller then skips
/// parsing `default/`).  Returns `false` to fall back to a cold parse.
#[cfg(feature = "mmap")]
#[must_use]
pub fn warm_load_stdlib(p: &mut Parser, default_dir: &str) -> bool {
    let Some(path) = cache_target(default_dir) else {
        return false;
    };
    match crate::ir_read::open_bundle_into(&path.to_string_lossy(), &mut p.database) {
        Ok(data) => {
            p.data = data;
            true
        }
        Err(_) => false, // cache miss (different key / absent) → cold parse
    }
}

/// Non-`mmap` builds (e.g. the wasm config) have no file-backed store, so the
/// cache is always a no-op and the caller always parses.
#[cfg(not(feature = "mmap"))]
#[must_use]
pub fn warm_load_stdlib(_p: &mut Parser, _default_dir: &str) -> bool {
    false
}

/// Cold path: after `default/` has been parsed into `p`, write the stdlib bundle
/// so the next run can warm-load it.  No-op when the cache is disabled.
#[cfg(feature = "mmap")]
pub fn save_stdlib_cache(p: &Parser, default_dir: &str) {
    let Some(path) = cache_target(default_dir) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = crate::ir_store::save_bundle(&p.data, &p.database.types, &path.to_string_lossy());
}

/// Non-`mmap` builds: no bundle to write.
#[cfg(not(feature = "mmap"))]
pub fn save_stdlib_cache(_p: &Parser, _default_dir: &str) {}
