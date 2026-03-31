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

/// A7.2.2: Parser.pending_native_libs is populated when a package manifest
/// declares `native = "..."` and the resolved shared library path exists on
/// disk.
///
/// This test uses `lib_dirs` pointing to the test fixture under
/// `tests/lib/native_pkg/` which ships a mock `loft.toml` with `native`.
/// The actual shared library does not need to exist for the path-population
/// test — only for the load test below.
#[test]
fn parser_pending_native_libs_populated() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    // Point the parser at the test fixture directory that contains a native package.
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_str("use native_pkg;", "test", false);
    assert!(
        !p.pending_native_libs.is_empty(),
        "expected pending_native_libs to contain the resolved shared library path"
    );
}

// ---------------------------------------------------------------------------
// A7.2.3: extensions::load_one registers functions via loft_register_v1
// ---------------------------------------------------------------------------

/// A7.2.3: `extensions::load_one` loads the fixture cdylib and registers its
/// native functions into the State library.
///
/// Requires `tests/lib/native_pkg/native/` to contain the built fixture
/// shared library (`libloft_native_test.so` / `.dylib` / `.dll`).
/// Build with: `cargo build -p loft-native-test --release`
#[test]
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
    extensions::load_one(&mut state, lib_path);

    state.execute_argv("main", &p.data, &[]);
}
