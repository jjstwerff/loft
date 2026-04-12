// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests for T1-2: wildcard and selective imports.
//! Verifies that `use mylib::*` and `use mylib::name` bring library names
//! into scope without a qualifier.

extern crate loft;

use loft::diagnostics::Level;
use loft::parser::Parser;
use loft::platform::sep_str;
use loft::scopes;

/// `use importlib::*` makes all names (add, mul, Point) directly accessible.
#[test]
fn wildcard_import_makes_names_accessible() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}wildcard_import_main.loft"), false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}

/// `use importlib::add` makes only `add` directly accessible; mul and Point are not imported.
#[test]
fn selective_import_makes_named_item_accessible() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}selective_import_main.loft"), false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}

/// `use importlib::nope` where `nope` does not exist in importlib produces an error.
#[test]
fn selective_import_of_unknown_name_is_error() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}bad_import_main.loft"), false);
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected an error for importing nonexistent name 'nope'"
    );
}

/// C53: match arms accept bare and qualified library enum variants.
#[test]
fn match_accepts_library_enum_variants() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}match_lib_enum_main.loft"), false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}
