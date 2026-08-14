// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A module's file NAME is shared across the whole dependency graph (loft#912).
//!
//! Two packages may each hold `src/catalogue.loft`, but only one of them can be
//! the module called `catalogue`: the second `use catalogue;` found the name
//! already taken and imported the OTHER package's file.  The loser's own
//! functions were then simply absent, reported as `Unknown function <name>` —
//! pointing at a line inside a package the author of the other one did not
//! write, may not have read, and cannot fix.  Nothing in the output said
//! "collision", so the search went looking for a missing `pub` or a typo.
//!
//! The clash is now REPORTED by name, with both file paths in the message.  It is
//! `advice` and the resolution is unchanged, deliberately: a hard refusal breaks
//! code that builds today — `graphics` <= 0.4.2 and `mesh3d` both ship `math` /
//! `mesh` / `scene`, and this repo's own `tests/fixtures/libs/graphics` depends on
//! the registry `mesh3d` while carrying its own copies of all three.  Scoping module
//! names to their package is the fix this advice is a signpost for.
//!
//! So the `Unknown function` error still follows — the advice EXPLAINS it, it does
//! not remove it.  These tests assert exactly that much and no more, because a test
//! claiming the program now builds would be claiming the fix that has not landed.
//! They also pin the three neighbouring shapes that must stay silent, since a check
//! that fired on all of them would pass a test that only looked at the broken case.

extern crate loft;

use loft::diagnostics::Level;
use loft::parser::Parser;

/// Build a two-package tree under a unique temp root:
///
/// ```text
/// pkg_dep/src/catalogue.loft   pub fn part_list()
/// pkg_dep/src/pkg_dep.loft     use catalogue;
/// pkg_top/src/<extra>          (the row under test)
/// pkg_top/src/pkg_top.loft     <top_body>
/// ```
///
/// `pkg_top` depends on `pkg_dep` by path.  Returns the parser's diagnostics
/// after parsing `pkg_top`'s entry file.
fn parse_two_packages(tag: &str, extra: &[(&str, &str)], top_body: &str) -> (Level, Vec<String>) {
    let root = std::env::temp_dir().join(format!("loft_912_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dep = root.join("pkg_dep");
    let top = root.join("pkg_top");
    std::fs::create_dir_all(dep.join("src")).expect("mkdir dep");
    std::fs::create_dir_all(top.join("src")).expect("mkdir top");

    std::fs::write(
        dep.join("loft.toml"),
        "[package]\nname = \"pkg_dep\"\nversion = \"0.1.0\"\nentry = \"src/pkg_dep.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        dep.join("src/catalogue.loft"),
        "pub fn part_list() -> integer { 7 }\n",
    )
    .unwrap();
    std::fs::write(
        dep.join("src/pkg_dep.loft"),
        "use catalogue;\npub fn dep_entry() -> integer { part_list() }\n",
    )
    .unwrap();

    std::fs::write(
        top.join("loft.toml"),
        "[package]\nname = \"pkg_top\"\nversion = \"0.1.0\"\nentry = \"src/pkg_top.loft\"\n\n\
         [dependencies]\npkg_dep = { path = \"../pkg_dep\" }\n",
    )
    .unwrap();
    for (name, body) in extra {
        std::fs::write(top.join("src").join(name), body).unwrap();
    }
    std::fs::write(top.join("src/pkg_top.loft"), top_body).unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&top.join("src/pkg_top.loft").to_string_lossy(), false);
    let out = (p.diagnostics.level(), p.diagnostics.lines().to_vec());
    let _ = std::fs::remove_dir_all(&root);
    out
}

/// The reported shape: the consumer grows a `src/catalogue.loft` of its own and
/// `use`s it, while its dependency already has one.  The message must name the
/// clash and BOTH files — the whole cost of this bug was that it named neither.
#[test]
fn clashing_module_basename_is_reported_naming_both_files() {
    let (level, diag) = parse_two_packages(
        "clash",
        &[("catalogue.loft", "pub fn top_only() -> integer { 3 }\n")],
        "use pkg_dep;\nuse catalogue;\n\
         pub fn top_entry() -> integer { pkg_dep::part_list() }\n\
         pub fn top_two() -> integer { top_only() }\n",
    );
    let all = diag.join("\n");
    assert!(
        all.contains("module 'catalogue' is declared by two files"),
        "expected the clash to be named; got:\n{all}"
    );
    assert!(
        all.contains("pkg_top") && all.contains("pkg_dep"),
        "both files must be named, so the reader does not have to find the second one:\n{all}"
    );
    // The clash must be reported as ADVICE.  Escalating it would break packages that
    // build today, which is the whole reason the resolution is left alone for now.
    assert!(
        diag.iter()
            .any(|d| d.starts_with("Advice[module-name-shadowed]")),
        "the clash must be advice, not a gating tier:\n{all}"
    );
    // The symptom still follows — the advice explains it rather than removing it.
    // Asserting its absence here would claim the scoping fix that has not landed.
    assert!(
        all.contains("Unknown function"),
        "the underlying mis-resolution is unchanged; if this stopped happening the \
         advice has become redundant and should be revisited:\n{all}"
    );
    let _ = level;
}

/// The same clash reached from the OTHER direction — the consumer's module is
/// loaded first, so the DEPENDENCY's `use catalogue;` is the one that finds the
/// name taken.  That is the direction the issue was filed from, where the error
/// landed on a line inside the dependency.  Both orders must be caught, or the
/// bug merely moves when a `use` is reordered.
#[test]
fn clash_is_caught_when_the_dependency_loses() {
    let (level, diag) = parse_two_packages(
        "clash_rev",
        &[("catalogue.loft", "pub fn top_only() -> integer { 3 }\n")],
        "use catalogue;\nuse pkg_dep;\n\
         pub fn top_entry() -> integer { pkg_dep::part_list() }\n\
         pub fn top_two() -> integer { top_only() }\n",
    );
    let all = diag.join("\n");
    assert!(
        all.contains("module 'catalogue' is declared by two files"),
        "the reversed load order must be caught too; got:\n{all}"
    );
    assert!(
        diag.iter()
            .any(|d| d.starts_with("Advice[module-name-shadowed]")),
        "the clash is advice in both directions:\n{all}"
    );
    let _ = level;
}

/// Control — a DIFFERENT basename is the documented fix, and it must still work.
/// This is the row that says the guard did not simply refuse intra-package
/// modules whose dependency happens to have modules of its own.
#[test]
fn a_distinct_basename_still_loads() {
    let (level, diag) = parse_two_packages(
        "renamed",
        &[("choices.loft", "pub fn top_only() -> integer { 3 }\n")],
        "use pkg_dep;\nuse choices;\n\
         pub fn top_entry() -> integer { pkg_dep::part_list() }\n\
         pub fn top_two() -> integer { top_only() }\n",
    );
    assert!(
        !diag.join("\n").contains("declared by two files"),
        "a distinct name is not a clash:\n{}",
        diag.join("\n")
    );
    assert!(
        level < Level::Error,
        "renaming the module must leave a clean parse:\n{}",
        diag.join("\n")
    );
}

/// Control — ONE module `use`d from two files of the same package resolves to the
/// same file both times.  Refusing this would break every multi-file package, and
/// it is the shape a name-only check would get wrong.
#[test]
fn one_module_used_from_two_files_of_one_package_is_not_a_clash() {
    let (level, diag) = parse_two_packages(
        "twice",
        &[
            ("choices.loft", "pub fn top_only() -> integer { 3 }\n"),
            (
                "extra.loft",
                "use choices;\npub fn extra_fn() -> integer { top_only() + 1 }\n",
            ),
        ],
        "use choices;\nuse extra;\nuse pkg_dep;\n\
         pub fn top_entry() -> integer { pkg_dep::part_list() }\n\
         pub fn top_two() -> integer { top_only() + extra_fn() }\n",
    );
    assert!(
        !diag.join("\n").contains("declared by two files"),
        "the same file reached twice is one module, not two:\n{}",
        diag.join("\n")
    );
    assert!(
        level < Level::Error,
        "a multi-file package must still parse cleanly:\n{}",
        diag.join("\n")
    );
}

/// Control — a package file whose name matches a DECLARED DEPENDENCY is already
/// governed by the dep-shadowing guard in `lib_path`: the dependency wins, on
/// purpose.  The clash check must defer to that rather than report a second,
/// contradictory verdict on the same name.
#[test]
fn a_file_named_like_a_declared_dependency_is_not_a_clash() {
    let (level, diag) = parse_two_packages(
        "shadow",
        &[("pkg_dep.loft", "pub fn shadow_fn() -> integer { 99 }\n")],
        "use pkg_dep;\npub fn top_entry() -> integer { pkg_dep::part_list() }\n",
    );
    assert!(
        !diag.join("\n").contains("declared by two files"),
        "a declared dependency name is resolved by the shadow guard, not refused:\n{}",
        diag.join("\n")
    );
    assert!(
        level < Level::Error,
        "the shadow guard's own resolution must stay clean:\n{}",
        diag.join("\n")
    );
}
