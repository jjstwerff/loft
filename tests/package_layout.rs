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

/// @PLN102 arc B — a package whose `loft.toml` carries a MALFORMED version
/// constraint (`^0.9`, an unsupported operator) is rejected LOUDLY at load,
/// not silently accepted.  Before arc B, `check_version` degraded any non-`>=`
/// form to `0.0.0` and always passed; the caret would have loaded fine.
#[test]
fn arc_b_malformed_constraint_is_fatal() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_badconstraint_test_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected a fatal for the malformed version constraint '^0.9'"
    );
    assert!(
        p.diagnostics
            .lines()
            .iter()
            .any(|l| l.contains("invalid loft version requirement")),
        "Expected the malformed-constraint diagnostic, got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN102 arc B — a package with an UPPER bound the running interpreter does
/// not meet (`<=0.1` against calendar-versioned loft) is rejected.  This is the
/// core regression: before arc B the upper bound was silently ignored and the
/// package loaded, the category-S silent failure the plan removes.
#[test]
fn arc_b_unsatisfiable_upper_bound_is_fatal() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_upperbound_test_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected a fatal for the unsatisfiable upper bound '<=0.1'; \
         before arc B this loaded silently"
    );
    assert!(
        p.diagnostics
            .lines()
            .iter()
            .any(|l| l.contains("requires loft")),
        "Expected the version-requirement diagnostic, got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN102 arc B-semantic — a package that requires a compatibility `contract`
/// newer than this loft provides (`CONTRACT_VERSION` is 0 pre-1.0) is a hard,
/// loud reject: loft is too old for the library's epoch.
#[test]
fn arc_b_contract_too_new_is_fatal() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_contract_future_test_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected a fatal for a contract requirement newer than this loft"
    );
    assert!(
        p.diagnostics
            .lines()
            .iter()
            .any(|l| l.contains("requires loft contract")),
        "Expected the contract-too-old diagnostic, got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN102 arc B-semantic — a package declaring the CURRENT contract epoch loads
/// clean.  Guards against the gate becoming a blanket reject (a vacuous
/// too-new test would pass even if every contract were rejected).
#[test]
fn arc_b_contract_current_loads_clean() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_contract_ok_test_main.loft"),
        false,
    );
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "A package at the current contract epoch should load; diagnostics: {:?}",
        p.diagnostics.lines()
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

/// Dep-shadowing regression: a package file named EXACTLY like a declared
/// dependency must not shadow that dependency in `use` resolution.
///
/// The package root sits in `lib_dirs` (that is what makes intra-package
/// `use otherfile;` work), so before the guard in `Parser::lib_path`,
/// `use shadowlib;` inside `consumer/shadowlib.loft` resolved to the file
/// itself: the real library never loaded, `shadowlib::Probe` came back
/// "Undefined type", and every qualified reference in the package errored.
/// This is how `tools/audience-demo/server.loft` (a package file named
/// `server.loft` next to a `server` dependency) silently broke.
///
/// Runs the real binary because the shadowing needs main.rs's package
/// detection (the walk-up that pushes the package root onto `lib_dirs`) —
/// an in-process `Parser` with hand-set `lib_dirs` cannot reproduce it.
#[test]
fn declared_dep_beats_same_named_package_file() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new(root.join("target/release/loft"))
        .args(["--interpret", "--no-warnings"])
        .arg(root.join("tests/fixtures/dep_shadow/consumer/shadowlib.loft"))
        .current_dir(&root)
        .output()
        .expect("run the dep_shadow consumer fixture");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("shadow-ok 42"),
        "dep-shadowing guard regressed: `use shadowlib;` did not resolve to \
         the declared dependency.  stdout={stdout:?} stderr={stderr:?}"
    );
}

/// #337 — PACKAGES.md resolution step 2: `use a;` resolves through the
/// consuming package's `[dependencies] a = { path = "…" }` even when the
/// dep is NOT a sibling package (previously only lib/, lib_dirs, and
/// sibling layouts worked; the compile-time resolver never consulted
/// path entries).
#[test]
fn i337_manifest_path_dep_resolves_non_sibling() {
    let tmp = std::env::temp_dir().join("loft_i337_pathdep");
    let _ = std::fs::remove_dir_all(&tmp);
    let dep_root = tmp.join("elsewhere").join("nested").join("a");
    std::fs::create_dir_all(dep_root.join("src")).unwrap();
    std::fs::write(
        dep_root.join("loft.toml"),
        "[package]\nname = \"a\"\nversion = \"0.0.1\"\n[library]\nentry = \"src/a.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        dep_root.join("src").join("a.loft"),
        "pub fn a_hello() -> text { \"hello from a\" }\n",
    )
    .unwrap();
    let b_root = tmp.join("b");
    std::fs::create_dir_all(b_root.join("src")).unwrap();
    std::fs::write(
        b_root.join("loft.toml"),
        "[package]\nname = \"b\"\nversion = \"0.0.1\"\n[library]\nentry = \"src/b.loft\"\n\
         [dependencies]\na = { path = \"../elsewhere/nested/a\" }\n",
    )
    .unwrap();
    std::fs::write(
        b_root.join("src").join("b.loft"),
        "use a;\n\nfn main() {\n  log_info(\"{a_hello()}\");\n}\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&b_root.join("src").join("b.loft").to_string_lossy(), false);
    scopes::check(&mut p.data);
    assert!(
        p.diagnostics.level() < Level::Error,
        "path dep should resolve; diagnostics: {:?}",
        p.diagnostics.lines()
    );
}
