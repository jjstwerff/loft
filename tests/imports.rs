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

/// P173: two files that `use` each other (cyclic intra-package import)
/// must resolve both sides' public types so every cross-file reference
/// links to the real definition.  Before the P173 fix this failed with
/// "Undefined type TypeA" / "Undefined type TypeB" because `use X;` queued
/// an import that was applied before X's definitions were registered.
///
/// The fix: `parse_file` runs `actual_types_deferred` with a buffer that
/// collects unresolved stubs; after the full recursion (and a round of
/// `import_all_overwrite`), `resolve_deferred_unknowns` patches the stubs
/// to their real definitions via `rewrite_unknown_refs`.
#[test]
fn p173_intra_cycle_resolves_cross_file_types() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}p173_cycle_main.loft"), false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected cyclic `use` to resolve; got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN22 Phase 3 — `use … as …` aliasing.  Parses a main file that uses all
/// three alias forms against `enumlib` (library alias `use enumlib as el`, type
/// alias `use enumlib::Status as St`, function alias `use enumlib::make as mk`)
/// and asserts every alias binds (an unbound alias surfaces as "Unknown
/// function" / "Undefined type").
#[test]
fn pln22_phase3_use_as_aliasing() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}p3_alias_main.loft"), false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN22 Phase 4 — grouped selective import `use lib::(a as x, b);`.  Parses a
/// main file that imports two names from `enumlib` in one parenthesised group,
/// with per-name aliases, and asserts both bind.
#[test]
fn pln22_phase4_grouped_import() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}p4_group_main.loft"), false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN22 Phase 4 — the flat comma list `use lib::a, b` is dropped; multiple
/// names must be parenthesised.  Parsing the flat form must produce an error.
#[test]
fn pln22_phase4_flat_list_rejected() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}p4_flat_rejected.loft"), false);
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected the flat `use lib::a, b` list to be rejected"
    );
}

/// @PLN102 C97 — a library may define a name that also exists in the stdlib (`clamp`);
/// it is module-scoped (reached as `c97_shadowlib::clamp`) and does NOT trigger the C95
/// "Cannot redefine" error, while the bare name stays the stdlib's.  This is the fix that
/// lets the stdlib grow without breaking a shipped library (the shapes/time break).
#[test]
fn pln102_c97_library_may_define_a_stdlib_name() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}c97_shadow_main.loft"), false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "a library defining the stdlib name `clamp` must be module-scoped, not a C95 redefinition; got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN13 C101 — `std`/`core` are reserved package names (a library may not claim a
/// language-namespace name), and `std::name` is the stdlib's qualified form — the escape
/// hatch that still reaches a stdlib symbol shadowed by a user def or a `use lib::*`.
#[test]
fn pln13_c101_reserved_names_and_std_qualifier() {
    // The canonical reserved list refuses the namespace names, admits ordinary ones.
    assert!(loft::libscan::is_reserved_package_name("std"));
    assert!(loft::libscan::is_reserved_package_name("core"));
    assert!(!loft::libscan::is_reserved_package_name("regex"));
    assert!(!loft::libscan::is_reserved_package_name("stdlib"));

    // `std::name` resolves to the stdlib prelude: a program qualifying a stdlib call
    // parses clean (the escape hatch freezes with the contract).
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse_str(
        "fn main() { x = std::max(3, 9); print(\"{x}\"); }",
        "c101_std.loft",
        false,
    );
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "std::max must resolve to the stdlib; got: {:?}",
        p.diagnostics.lines()
    );
}
