// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Shared helpers for integration test binaries.
//!
//! Each item is `#[allow(dead_code)]` because this module is pulled
//! into multiple test binaries via `mod common;`, and not every
//! binary uses every helper.  Without the allow, binaries that don't
//! consume a given helper produce a warning that turns into a CI
//! failure under `-D warnings`.

#[allow(dead_code)]
pub mod cross_mode;

/// A server-test port, offset by `LOFT_TEST_PORT_OFFSET` (default 0).  The engine-host /
/// wasm-relay tests bind FIXED ports; two suites run at once — e.g. two agents in sibling
/// checkouts (`loft` and `loft2`) — collide on them and flake.  `find_problems.sh` exports a
/// distinct offset per checkout so their port ranges never overlap.  A plain `cargo test` (no
/// offset) keeps the base ports.
#[allow(dead_code)]
pub fn test_port(base: u16) -> u16 {
    let offset = std::env::var("LOFT_TEST_PORT_OFFSET")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    base.saturating_add(offset)
}

use loft::data::Data;
use loft::database::Stores;
use loft::parser::Parser;
use std::path::PathBuf;
use std::sync::OnceLock;

/// On Windows MSVC, the build-script output dirs holding native import libraries
/// (e.g. `windows.0.48.5.lib` from `windows-sys`) must be passed to a hand-driven
/// `rustc` as `-L` paths — cargo adds them via `cargo:rustc-link-search` but a
/// test that links a cdylib by hand does not, so the link fails
/// `LNK1181: cannot open input file …`.  Mirrors `native_lib::native_lib_search_dirs`
/// and the `--native` test runner.  Empty (a no-op) off Windows.
///
/// `rlib` is `target/<profile>/libloft.rlib` or `target/<profile>/deps/libloft-*.rlib`.
#[allow(dead_code)]
#[cfg(not(windows))]
pub fn native_lib_search_dirs(_rlib: &std::path::Path) -> Vec<PathBuf> {
    Vec::new()
}

#[allow(dead_code)]
#[cfg(windows)]
pub fn native_lib_search_dirs(rlib: &std::path::Path) -> Vec<PathBuf> {
    // Walk up to the profile dir (release/ or debug/), then scan `build/<crate>-<hash>/`.
    let Some(profile_dir) = rlib.parent().and_then(|p| {
        if p.file_name().is_some_and(|n| n == "deps") {
            p.parent()
        } else {
            Some(p)
        }
    }) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(profile_dir.join("build")) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let build_entry = entry.path();
        // `out/` and its immediate subdirs (libs generated into OUT_DIR).
        let out = build_entry.join("out");
        if out.is_dir() {
            dirs.push(out.clone());
            if let Ok(subs) = std::fs::read_dir(&out) {
                dirs.extend(
                    subs.filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_dir()),
                );
            }
        }
        // `cargo:rustc-link-search` directives cached in `build/<crate>-<hash>/output`
        // (e.g. `windows_x86_64_msvc` ships its `.lib` inside the registry package).
        if let Ok(content) = std::fs::read_to_string(build_entry.join("output")) {
            for line in content.lines() {
                if let Some(p) = line
                    .strip_prefix("cargo:rustc-link-search=native=")
                    .or_else(|| line.strip_prefix("cargo:rustc-link-search="))
                {
                    let p = PathBuf::from(p);
                    if p.is_dir() && !dirs.contains(&p) {
                        dirs.push(p);
                    }
                }
            }
        }
    }
    dirs
}

#[allow(dead_code)]
static DEFAULT_PARSED: OnceLock<(Data, Stores)> = OnceLock::new();

/// Parse the default library once per test binary and cache the result.
/// Each test clones the schema cheaply instead of re-parsing three files.
#[allow(dead_code)]
pub fn cached_default() -> (Data, Stores) {
    let (data, db) = DEFAULT_PARSED.get_or_init(|| {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        (p.data, p.database)
    });
    (data.clone(), db.clone())
}
