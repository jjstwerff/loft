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

// ─── whole-program cache (arc E) ─────────────────────────────────────────────
//
// The stdlib cache above caches `default/` only; the whole-program cache caches
// the ENTIRE post-parse program (stdlib + the script's lazily-loaded libs + the
// user file) keyed on the script path, validated by a drift manifest of every
// parsed source's content hash.  On a repeated run of an unchanged program it
// skips ALL parsing.  The caller (`main.rs`) gates these on the cache env var;
// they assume the gate has already passed.

#[cfg(feature = "mmap")]
fn hex32(key: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in key {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// True iff `manifest` exists and every listed source's current content hash
/// still matches (no input drifted since the bundle was written).
#[cfg(feature = "mmap")]
fn manifest_matches(manifest: &std::path::Path) -> bool {
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return false;
    };
    let mut lines = text.lines();
    // @PLAN54 G2/M6 — the first line pins THIS build's signature.  A binary
    // upgrade (new store layout / codegen / version / features) changes it, so a
    // stale bundle is a clean cache miss (reparse), never a wrong-format load
    // (`Store::is_store_file`'s fixed magic can't catch a layout change).
    match lines.next().and_then(|l| l.strip_prefix("sig ")) {
        Some(sig) if sig == crate::cache::build_signature() => {}
        _ => return false,
    }
    let mut any = false;
    for line in lines {
        let Some((hexhash, path)) = line.split_once(' ') else {
            return false;
        };
        match crate::cache::file_hash(path) {
            Some(cur) if hex32(&cur) == hexhash => any = true,
            _ => return false,
        }
    }
    any // an empty source list is not a valid hit
}

/// Whole-program warm load: if a valid bundle exists for `script_abspath` and
/// every source it was built from is unchanged, load the entire `Data` + type
/// schema into `p` and return `true` (the caller then skips **all** parsing).
#[cfg(feature = "mmap")]
#[must_use]
pub fn warm_load_program(
    p: &mut Parser,
    script_abspath: &str,
    store_out: &mut Option<(crate::database::Stores, crate::keys::DbRef)>,
) -> bool {
    let (bundle, manifest) = crate::cache::program_cache_paths(script_abspath);
    if !manifest_matches(&manifest) {
        return false;
    }
    let bundle_s = bundle.to_string_lossy();
    // @PLAN54 G2/M6 — with LOFT_CODEGEN_STORE, do a *skeleton* load: mmap the
    // bundle, reconstruct only the def table (bodies stay in the store), and
    // hand the store back so codegen reads bodies straight from it — skipping
    // read_data's body rebuild.  Otherwise full read_data (the M2/M5 default).
    if std::env::var_os("LOFT_CODEGEN_STORE").is_some() {
        match crate::ir_read::open_program_store(&bundle_s) {
            Ok((stores, root, data, schema)) => {
                p.database.install_schema(schema);
                p.data = data;
                *store_out = Some((stores, root));
                true
            }
            Err(_) => false,
        }
    } else {
        match crate::ir_read::open_bundle_into(&bundle_s, &mut p.database) {
            Ok(data) => {
                p.data = data;
                true
            }
            Err(_) => false,
        }
    }
}

/// Cold path: after the full parse, write the whole-program bundle + its drift
/// manifest for `script_abspath` (every deduped parsed source + its content
/// hash).  The bundle is published first, then the manifest atomically — so a
/// manifest is only ever present alongside a complete bundle.
#[cfg(feature = "mmap")]
pub fn save_program(p: &Parser, script_abspath: &str) {
    use std::fmt::Write as _;
    let (bundle, manifest) = crate::cache::program_cache_paths(script_abspath);

    let mut paths: Vec<&String> = p.parsed_sources.iter().collect();
    paths.sort_unstable();
    paths.dedup();
    let mut lines = String::new();
    // @PLAN54 G2/M6 — pin the build signature first so a binary upgrade
    // invalidates this bundle (see `manifest_matches`).
    let _ = writeln!(lines, "sig {}", crate::cache::build_signature());
    for path in &paths {
        let Some(h) = crate::cache::file_hash(path) else {
            return; // an unreadable source → don't cache (would never validate)
        };
        let _ = writeln!(lines, "{} {path}", hex32(&h));
    }
    if paths.is_empty() {
        return; // no sources → an unvalidatable manifest; skip caching
    }

    if let Some(parent) = bundle.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if crate::ir_store::save_bundle(&p.data, &p.database.types, &bundle.to_string_lossy()).is_err()
    {
        return;
    }
    // Manifest last + atomically — a stale/partial manifest would just be a miss.
    let tmp = manifest.with_extension("manifest.tmp");
    if std::fs::write(&tmp, lines.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &manifest);
    }
    // @PLAN54 G2 / track 1 — with the cache default-on, bound the directory size
    // by evicting the oldest bundles after each cold save.
    crate::cache::prune_program_cache();
}

/// Non-`mmap` builds: the whole-program cache is unavailable.
#[cfg(not(feature = "mmap"))]
#[must_use]
pub fn warm_load_program(
    _p: &mut Parser,
    _script_abspath: &str,
    _store_out: &mut Option<(crate::database::Stores, crate::keys::DbRef)>,
) -> bool {
    false
}
#[cfg(not(feature = "mmap"))]
pub fn save_program(_p: &Parser, _script_abspath: &str) {}
