// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests for T2-11: external library package layout.
//! Verifies that `use mylib;` resolves `<lib-dir>/<id>/src/<id>.loft`
//! when a `loft.toml` manifest is present.

extern crate loft;

use loft::diagnostics::Level;
use loft::parser::Parser;
use loft::platform::sep_str;
use loft::scopes;

/// Confirm that lib_path() locates a library stored in the packaged directory
/// layout: `tests/lib/testpkg/src/testpkg.loft` via `lib_dirs`.
#[test]
fn package_layout_use_finds_src_subdir() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}package_test_main.loft"), false);
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
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    // testpkg_future requires loft >= 99.0, which should always fail.
    p.parse(
        &format!("tests{s}lib{s}package_version_test_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected a version-mismatch error"
    );
}

/// P129: native_packages must not contain duplicate crate entries.
/// A package with `[native] crate` parsed through lib_path_manifest should
/// not produce a second entry if register_native_manifest already added it.
#[test]
fn p129_no_duplicate_native_packages() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    // Parse a file that uses the native_crate_pkg package.
    p.parse(
        &format!("tests{s}lib{s}native_crate_import_main.loft"),
        false,
    );
    scopes::check(&mut p.data);
    // Count occurrences of the crate name — must be exactly 1.
    let count = p
        .data
        .native_packages
        .iter()
        .filter(|(c, _)| c == "loft-native-crate-test")
        .count();
    assert!(
        count <= 1,
        "P129: native_packages has {count} entries for loft-native-crate-test, expected at most 1"
    );
}

/// Regression: struct field types in use-loaded packages must resolve correctly.
/// Multiple structs + #native declarations + functions with return null.
#[test]
fn struct_fields_resolve_in_use_loaded_package() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}struct_order_main.loft"), false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Struct field types should resolve in use-loaded packages; errors: {:?}",
        p.diagnostics.lines()
    );
}
