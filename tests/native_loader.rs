// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A7.2 — `cdylib` native extension loader tests.
//!
//! Tests the manifest `native` field, `pending_native_libs` propagation on
//! Parser, and the `extensions::load_all()` dispatch path.

extern crate loft;

use loft::manifest::{Manifest, read_manifest};
use loft::parser::Parser;

mod common;
use common::cached_default;

// ---------------------------------------------------------------------------
// A7.2.1: manifest `native` field is parsed and accessible
// ---------------------------------------------------------------------------

/// A7.2.1: `read_manifest` returns the `native` field from `[library]`.
#[test]
fn manifest_parses_native_field() {
    use std::io::Write;
    let dir = std::env::temp_dir();
    let path = dir.join(format!("loft_a72_test_{}.toml", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(b"[package]\nloft = \">=0.8\"\n\n[library]\nnative = \"loft_myext\"\n")
        .unwrap();
    let m: Manifest = read_manifest(path.to_str().unwrap()).unwrap();
    assert_eq!(m.native.as_deref(), Some("loft_myext"));
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// A7.2.2: Parser accumulates pending_native_libs when a manifest has `native`
// ---------------------------------------------------------------------------

/// A7.2.2: Parser resolves the native library path when a package manifest
/// declares `native = "..."`.  The path is only added to `pending_native_libs`
/// when the pre-built `.so` exists or `auto_build_native` succeeds.
/// The test fixture has no buildable native crate, so the list stays empty —
/// but parsing must still succeed without errors.
#[test]
fn parser_native_pkg_parses_without_error() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_str("use native_pkg;", "test", false);
    // No parse errors — the #native stub is registered even without the .so.
    let has_errors = p.diagnostics.lines().iter().any(|l| l.starts_with("Error"));
    assert!(
        !has_errors,
        "unexpected errors: {:?}",
        p.diagnostics.lines()
    );
}

// ---------------------------------------------------------------------------
// A7.2.3: extensions::load_one registers functions via loft_register_v1
// ---------------------------------------------------------------------------

/// A7.2.3: `extensions::load_one` loads a cdylib and registers its functions.
///
/// Requires the fixture shared library to be pre-built.
/// Build with: `cargo build -p loft-native-test --release`
#[test]
#[ignore = "A7.2: fixture cdylib not built — run: cargo build -p loft-native-test --release"]
fn load_one_registers_native_functions() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let native_decl = r#"
pub fn ext_add_one(x: integer) -> integer not null;
#native "n_ext_add_one"
"#;
    let source = r#"
fn main() {
    println("{ext_add_one(41)}")
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    assert!(
        p.diagnostics.is_empty(),
        "diagnostics: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    // Load the fixture cdylib; it registers "n_ext_add_one".
    let lib_path = if cfg!(target_os = "macos") {
        "tests/lib/native_pkg/native/libloft_native_test.dylib"
    } else if cfg!(windows) {
        "tests/lib/native_pkg/native/loft_native_test.dll"
    } else {
        "tests/lib/native_pkg/native/libloft_native_test.so"
    };
    extensions::load_all(&mut state, vec![lib_path.to_string()]);

    state.execute_argv("main", &p.data, &[]);
}
