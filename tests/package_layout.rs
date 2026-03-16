// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests for T2-11: external library package layout.
//! Verifies that `use mylib;` resolves `<lib-dir>/<id>/src/<id>.loft`
//! when a `loft.toml` manifest is present.

extern crate loft;

use loft::diagnostics::Level;
use loft::parser::Parser;
use loft::scopes;

/// Confirm that lib_path() locates a library stored in the packaged directory
/// layout: `tests/lib/testpkg/src/testpkg.loft` via `lib_dirs`.
#[test]
fn package_layout_use_finds_src_subdir() {
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec!["tests/lib".to_string()];
    p.parse("tests/lib/package_test_main.loft", false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no parse errors; diagnostics: {:?}",
        p.diagnostics.lines()
    );
}

/// Confirm that a version requirement in `loft.toml` that exceeds the
/// current interpreter version produces a fatal diagnostic.
#[test]
fn package_layout_version_mismatch_is_fatal() {
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec!["tests/lib".to_string()];
    // testpkg_future requires loft >= 99.0, which should always fail.
    p.parse("tests/lib/package_version_test_main.loft", false);
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected a version-mismatch error"
    );
}
