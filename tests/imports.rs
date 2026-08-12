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

// ── loft#788: a bare name two packages both declare ───────────────────────────

/// Parse one `tests/lib` main file with the fixture libraries available.
fn parse_lib_main(file: &str) -> Parser {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}{file}"), false);
    scopes::check(&mut p.data);
    p
}

fn errors_of(p: &Parser) -> String {
    p.diagnostics.lines().join("\n")
}

/// loft#788 — a bare `Chunk` / `shared()` / `SHARED_C` that TWO packages declare
/// must be refused, naming both packages.
///
/// The pair of orders is the test, not either file alone: the defect was that
/// the same source line compiled to a different program depending on which
/// `use` came first, so a fix that refuses only one order has not fixed it. Both
/// files below are identical apart from the two swapped `use` lines.
#[test]
fn a_bare_name_two_packages_declare_is_refused_in_either_order() {
    for file in ["dupname_ab_main.loft", "dupname_ba_main.loft"] {
        let p = parse_lib_main(file);
        let msgs = errors_of(&p);
        assert!(
            p.diagnostics.level() >= Level::Error,
            "{file}: a bare ambiguous name must not silently pick a winner: {msgs}"
        );
        assert!(
            msgs.contains("`Chunk` is declared by more than one package"),
            "{file}: the message must name what is ambiguous: {msgs}"
        );
        // Both packages named, so the reader can pick — a message naming only
        // the winner would describe the bug rather than the choice.
        assert!(
            msgs.contains("dupname_a::Chunk") && msgs.contains("dupname_b::Chunk"),
            "{file}: both spellings must be offered: {msgs}"
        );
    }
}

/// loft#788 — a function and a constant collide the same way, and worse: both
/// import orders COMPILE and run, answering differently.
///
/// The struct case at least errors on a field; these two were silent.
#[test]
fn an_ambiguous_call_and_constant_are_refused_too() {
    let p = parse_lib_main("dupname_ab_main.loft");
    let msgs = errors_of(&p);
    assert!(
        msgs.contains("`shared` is declared by more than one package"),
        "a bare CALL is ambiguous the same way: {msgs}"
    );
    assert!(
        msgs.contains("`SHARED_C` is declared by more than one package"),
        "so is a bare constant: {msgs}"
    );
    // The storage spelling of a function is `n_shared`; a message telling
    // someone to write `dupname_a::n_shared` names something they cannot type.
    assert!(
        !msgs.contains("::n_"),
        "the mangled name must not reach the message: {msgs}"
    );
}

/// loft#788 control — QUALIFIED says which, so it must keep compiling.
///
/// Without this the fix would "pass" by refusing the collision everywhere,
/// including the shape #305 built to make work.
#[test]
fn a_qualified_name_is_never_ambiguous() {
    let p = parse_lib_main("dupname_qualified_main.loft");
    assert!(
        p.diagnostics.level() < Level::Error,
        "qualified names say which package they mean: {}",
        errors_of(&p)
    );
}

/// loft#788 control — two packages may share a name a program never writes
/// bare, and that program compiles today.
///
/// This is why the refusal is at the USE and not at the `use`: reporting when
/// the import is applied would break a working program for a collision it never
/// has (COMPATIBILITY.md — no functioning program breaks).
#[test]
fn an_unused_collision_still_compiles() {
    let p = parse_lib_main("dupname_unused_main.loft");
    assert!(
        p.diagnostics.level() < Level::Error,
        "an unused collision is not a question: {}",
        errors_of(&p)
    );
}

// ── loft#850: a method and a free function of one name, across packages ───────

/// loft#850 — a bare call reaches the function that accepts the receiver it was
/// GIVEN, whichever package declared it.
///
/// Three packages here each declare a `struct Thing`, and a method is filed under
/// the mangled key `t_5Thing_go`, which spells the type's NAME and nothing about
/// its package — so all three competed for one key and the first import won it.
/// The runtime half (which package's body actually ran) is pinned across all
/// three backends by `tests/scripts/850-cross-package-method-name-collision.loft`
/// and its swapped-order twin; this asserts the compile-time half, that nothing
/// is refused.
#[test]
fn a_method_and_a_free_function_of_one_name_resolve_by_receiver() {
    for file in ["dupmethod_ab_main.loft", "dupmethod_ba_main.loft"] {
        let p = parse_lib_main(file);
        assert!(
            p.diagnostics.level() < Level::Error,
            "{file}: the receiver says which `go` is meant: {}",
            errors_of(&p)
        );
    }
}

/// loft#850 — when the call genuinely cannot resolve, the hint names the
/// receiver in a form the reader can type, and does not blame the stdlib for a
/// package's choice.
///
/// The bare receiver name is the one spelling that identifies nothing in this
/// situation, since several packages declare it; and "stdlib declared `go` as a
/// method" pointed at a file that never mentions `go`. Both halves are asserted
/// because fixing either alone still leaves the reader without a next step.
#[test]
fn an_unresolvable_call_names_the_package_that_declares_the_method() {
    let p = parse_lib_main("dupmethod_hint_main.loft");
    let msgs = errors_of(&p);
    assert!(
        msgs.contains("dupmethod_b::Thing"),
        "the hint must name the receiver's package: {msgs}"
    );
    assert!(
        !msgs.contains("stdlib declared"),
        "the stdlib declared nothing here: {msgs}"
    );
}

/// @PLN102 C98 / loft#852 — a library's public function must not claim the
/// CONSUMER's variable namespace.
///
/// `use lib;` beside a local named after one of the library's `pub fn`s has to
/// compile, or every short verb a library exports (`turn`, `step`, `run`,
/// `wait`, `next`, `open`, `send`) becomes a word no consumer of that library
/// may use as a local — a break that arrives on someone else's release, that
/// nothing announces, and that no consumer can prepare for.
///
/// Values and functions live in separate namespaces, so all three facts hold at
/// once and all three are asserted: the local keeps its own value, an
/// unqualified call reaches the library function from a scope with no such
/// local, and a call reaches it from the scope that has one. Asserting only
/// "it compiles" would pass a fix that bound the name to the wrong thing.
///
/// Run on BOTH backends through the binary rather than parsed in-process: the
/// resolution is the parser's, but the values are what a consumer sees, and
/// only running proves the call did not silently answer the local.
#[test]
fn pln102_c98_a_local_may_shadow_a_library_function() {
    let s = sep_str();
    let main = format!("tests{s}lib{s}issue852_main.loft");
    let libs = format!("tests{s}lib");
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg(backend)
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("turn=42 call=200 other=300"),
            "{backend}: a local named after a library's `pub fn` must bind (42), \
             and the function stay callable from both a scope with the local (200) \
             and one without (300); got stdout {stdout:?} stderr {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// loft#853 — a library's free function must win its own QUALIFIED call, even when the
/// stdlib declares that name as a METHOD on the first argument's type.
///
/// loft#850 taught the method lookup to re-ask in the receiver type's OWN source when
/// the caller's scope answered with another package's method. But every type has an own
/// source, and for a builtin it is the stdlib — so searching it unconditionally let a
/// stdlib method on `text` outrank a library's free function of the same name.
/// `regex::split(pattern, input)` resolved to `split(self: text, separator: character)`
/// and the published `regex` package stopped compiling, which the freeze forbids.
///
/// The control line matters as much as the subject: a fix that reached the free function
/// by losing the stdlib's own text surface would satisfy the first assertion and break
/// every program in the language. Run through the binary so the resolution is checked by
/// its RESULT, not by the absence of a diagnostic — a wrong-but-compiling resolution is
/// exactly what loft#850 was about.
#[test]
fn issue853_a_library_free_fn_outranks_a_stdlib_method_of_the_same_name() {
    let s = sep_str();
    let main = format!("tests{s}lib{s}issue853_main.loft");
    let libs = format!("tests{s}lib");
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg(backend)
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("qualified=5 stdlib_free=3 stdlib_method=true"),
            "{backend}: `lib::split(text, text)` must reach the LIBRARY's free fn (5) while \
             the stdlib's own free fn (3) and text methods (true) still resolve; \
             got stdout {stdout:?} stderr {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
