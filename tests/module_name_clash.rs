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

/// loft#948 — the shadowing file declares something ELSE, so the collision BREAKS the build.
///
/// This is the case #912 was filed about and the only one a person cannot work out from the
/// output: the errors name a line inside a DEPENDENCY the consumer never edited, the missing
/// function is `pub`, the dependency is green on its own, and the cure — rename your own new
/// file — follows from nothing printed.
///
/// The advice was produced all along; `loft test` collected it and dropped it on the failure
/// path, printing only errors and warnings. So it appeared exactly when the build SURVIVED
/// and vanished when it did not.
///
/// Driven through the BINARY rather than `Parser::parse`, and that is load-bearing: the
/// shadowing file is never imported, so nothing reaches it by following `use` edges. It is
/// loaded because building the package reads every file under `src/` — which is why the
/// collision happens at all, and why this has to be tested at the `loft test` surface where
/// the output was being dropped.
#[test]
fn the_clash_is_reported_even_when_it_breaks_the_build() {
    let root = std::env::temp_dir().join(format!("loft_948_fatal_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dep = root.join("pkg_dep");
    let top = root.join("pkg_top");
    std::fs::create_dir_all(dep.join("src")).expect("mkdir dep");
    std::fs::create_dir_all(top.join("src")).expect("mkdir top");
    std::fs::create_dir_all(top.join("tests")).expect("mkdir tests");

    std::fs::write(
        dep.join("loft.toml"),
        "[package]\nname = \"pkg_dep\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_dep.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        dep.join("src/catalogue.loft"),
        "pub fn part_list() -> integer { 41 }\n",
    )
    .unwrap();
    std::fs::write(
        dep.join("src/pkg_dep.loft"),
        "use catalogue::*;\npub fn dep_answer() -> integer { part_list() + 1 }\n",
    )
    .unwrap();

    std::fs::write(
        top.join("loft.toml"),
        "[package]\nname = \"pkg_top\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_top.loft\"\n\n\
         [dependencies]\npkg_dep = { path = \"../pkg_dep\" }\n",
    )
    .unwrap();
    std::fs::write(
        top.join("src/pkg_top.loft"),
        "use pkg_dep::*;\npub fn top_answer() -> integer { dep_answer() }\n",
    )
    .unwrap();
    // Declares something else entirely and is imported by nobody, so the dependency's
    // `part_list` simply goes missing.
    std::fs::write(
        top.join("src/catalogue.loft"),
        "pub fn top_unrelated() -> text { \"x\" }\n",
    )
    .unwrap();
    std::fs::write(
        top.join("tests/answer.loft"),
        "use pkg_top::*;\nfn main() { assert(top_answer() == 42, \"answer\"); }\n",
    )
    .unwrap();

    let mut bin = std::env::current_exe().expect("test binary path");
    bin.pop();
    if bin.ends_with("deps") {
        bin.pop();
    }
    let out = std::process::Command::new(bin.join("loft"))
        .arg("test")
        .current_dir(&top)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("invoke loft test");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        all.contains("Unknown function part_list"),
        "the scaffold no longer reproduces the collision at all:\n{all}"
    );
    assert!(
        all.contains("module-name-shadowed"),
        "the collision is unreported in exactly the case that is fatal — the reader gets \
         `Unknown function` against a dependency's source and nothing naming the cause:\n{all}"
    );
    // The cure it names is `use self::` rather than a rename (loft#949).  Both packages
    // here declare a `[package] name`, which is what `self::` qualifies with, so it is
    // available — and it is the better answer: renaming churns a file and every `use` of
    // it downstream, while `self::` keeps both modules reachable and puts the name beyond
    // any consumer's reach.  A diagnostic that exists to be the signpost for an opt-in has
    // to say the opt-in's name.
    assert!(
        all.contains("catalogue.loft") && all.contains("use self::catalogue"),
        "the advice must name both files and the cure that keeps this package's answer:\n{all}"
    );
}

/// loft#948 — two files of ONE package are not this collision, so they must stay quiet.
///
/// `tests/<pkg>.loft` beside `src/<pkg>.loft` is an ordinary layout — two of this repo's own
/// fixtures use it — and the `use` binds the one the author meant. The advice is about a name
/// "shared across the whole dependency graph"; firing it where there is nothing to fix is how
/// a diagnostic teaches people to skip the ones where there is.
#[test]
fn a_same_package_basename_collision_is_not_advised() {
    let root = std::env::temp_dir().join(format!("loft_948_same_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let pkg = root.join("pkg_solo");
    std::fs::create_dir_all(pkg.join("src")).expect("mkdir src");
    std::fs::create_dir_all(pkg.join("tests")).expect("mkdir tests");
    std::fs::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"pkg_solo\"\nversion = \"0.1.0\"\nentry = \"src/pkg_solo.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        pkg.join("src/pkg_solo.loft"),
        "pub fn solo_answer() -> integer { 42 }\n",
    )
    .unwrap();
    // Named after its own package — the shape that used to draw a rename it must not draw.
    std::fs::write(
        pkg.join("tests/pkg_solo.loft"),
        "use pkg_solo;\nfn main() { assert(solo_answer() == 42, \"solo\"); }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("tests/pkg_solo.loft").to_string_lossy(), false);
    let lines = p.diagnostics.lines().to_vec();
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        !lines.iter().any(|l| l.contains("module-name-shadowed")),
        "two files of ONE package drew a cross-package collision advice:\n{lines:#?}"
    );
}

/// Build the loft#949 tree and return what `loft test` prints for a given `pkg_dep`
/// entry body.  The consumer ships its own `src/catalogue.loft` declaring `part_list`
/// with the SAME signature as the dependency's, so nothing errors — the dependency
/// simply answers with the consumer's number.
///
/// Driven through the BINARY, and that is load-bearing: nobody imports the consumer's
/// `catalogue.loft`, so no `use` edge reaches it.  It is loaded because building a
/// package reads every file under `src/` — which is what lets a consumer take a name
/// its dependency has not asked for yet.
fn use_self_tree(tag: &str, dep_entry: &str) -> String {
    let root = std::env::temp_dir().join(format!("loft_949_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dep = root.join("pkg_dep");
    let top = root.join("pkg_top");
    std::fs::create_dir_all(dep.join("src")).expect("mkdir dep");
    std::fs::create_dir_all(top.join("src")).expect("mkdir top");
    std::fs::create_dir_all(top.join("tests")).expect("mkdir tests");

    std::fs::write(
        dep.join("loft.toml"),
        "[package]\nname = \"pkg_dep\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_dep.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        dep.join("src/catalogue.loft"),
        "pub fn part_list() -> integer { 41 }\n",
    )
    .unwrap();
    std::fs::write(dep.join("src/pkg_dep.loft"), dep_entry).unwrap();

    std::fs::write(
        top.join("loft.toml"),
        "[package]\nname = \"pkg_top\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_top.loft\"\n\n\
         [dependencies]\npkg_dep = { path = \"../pkg_dep\" }\n",
    )
    .unwrap();
    std::fs::write(
        top.join("src/pkg_top.loft"),
        "use pkg_dep::*;\npub fn top_answer() -> integer { dep_answer() }\n",
    )
    .unwrap();
    // Same name, same signature — so this does not break the build, it changes the answer.
    std::fs::write(
        top.join("src/catalogue.loft"),
        "pub fn part_list() -> integer { 99 }\n",
    )
    .unwrap();
    std::fs::write(
        top.join("tests/answer.loft"),
        "use pkg_top::*;\nfn main() { println(\"answer={top_answer()}\"); }\n",
    )
    .unwrap();

    let mut bin = std::env::current_exe().expect("test binary path");
    bin.pop();
    if bin.ends_with("deps") {
        bin.pop();
    }
    let out = std::process::Command::new(bin.join("loft"))
        .arg("test")
        .current_dir(&top)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_AUTO_INSTALL", "1")
        .output()
        .expect("invoke loft test");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
    all
}

/// loft#949 — `use self::<module>` binds the package's OWN module, so a dependency
/// answers the same in every consumer.
///
/// Both arms run, and the bare-`use` arm is what makes this a proof rather than an
/// assertion: it pins that the scaffold still reproduces the wrong answer, so a green
/// `self::` arm cannot come from a tree that stopped colliding. Same files, same
/// dependency, one line different.
#[test]
fn use_self_binds_the_packages_own_module_not_a_consumers() {
    let bare = use_self_tree(
        "bare",
        "use catalogue::*;\npub fn dep_answer() -> integer { part_list() + 1 }\n",
    );
    assert!(
        bare.contains("answer=100"),
        "control: bare `use` must still bind the consumer's file — if this stopped \
         happening the scaffold no longer reproduces #949 and the other arm proves \
         nothing:\n{bare}"
    );

    let scoped = use_self_tree(
        "self",
        "use self::catalogue;\npub fn dep_answer() -> integer { part_list() + 1 }\n",
    );
    assert!(
        scoped.contains("answer=42"),
        "`use self::catalogue` must bind pkg_dep's own catalogue.loft (41 + 1), \
         whatever the consumer ships:\n{scoped}"
    );
    // No collision left to report: the module is registered under `pkg_dep::catalogue`,
    // so the consumer's `catalogue` is no longer competing for the same slot.
    assert!(
        !scoped.contains("module-name-shadowed"),
        "a scoped module must not still be reported as sharing a name:\n{scoped}"
    );
}

/// The half "prefer the local file" could not have delivered: two packages' same-named
/// modules COEXIST, each declaring its own `struct Row` and its own `make_row`, and each
/// answers its own.
///
/// A precedence rule alone cannot do this — `use_names` is a flat map and `use_add`
/// derives the source id from its size, so one `catalogue` overwrites the other however
/// the winner is chosen. `self::` registers under `<package>::<module>`, which is two
/// keys, so both stay reachable and the database type keys stay distinct too.
///
/// The values are the assertion, not the absence of an error: `dep_tag=dep` can only come
/// from pkg_dep's `Row`, `top_label=top` only from pkg_top's, and they have different
/// FIELDS — so a schema that had merged them could not produce both.
#[test]
fn two_packages_same_named_self_modules_both_stay_reachable() {
    let root = std::env::temp_dir().join(format!("loft_949_coexist_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dep = root.join("pkg_dep");
    let top = root.join("pkg_top");
    std::fs::create_dir_all(dep.join("src")).expect("mkdir dep");
    std::fs::create_dir_all(top.join("src")).expect("mkdir top");
    std::fs::create_dir_all(top.join("tests")).expect("mkdir tests");

    std::fs::write(
        dep.join("loft.toml"),
        "[package]\nname = \"pkg_dep\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_dep.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        dep.join("src/catalogue.loft"),
        "pub struct Row { n: integer, tag: text }\n\
         pub fn make_row() -> Row { Row { n: 41, tag: \"dep\" } }\n",
    )
    .unwrap();
    std::fs::write(
        dep.join("src/pkg_dep.loft"),
        "use self::catalogue;\n\
         pub fn dep_answer() -> integer { make_row().n + 1 }\n\
         pub fn dep_tag() -> text { make_row().tag }\n",
    )
    .unwrap();

    std::fs::write(
        top.join("loft.toml"),
        "[package]\nname = \"pkg_top\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_top.loft\"\n\n\
         [dependencies]\npkg_dep = { path = \"../pkg_dep\" }\n",
    )
    .unwrap();
    // Same module name, same function name, a DIFFERENT struct behind it.
    std::fs::write(
        top.join("src/catalogue.loft"),
        "pub struct Row { label: text, extra: float }\n\
         pub fn make_row() -> Row { Row { label: \"top\", extra: 2.5 } }\n",
    )
    .unwrap();
    std::fs::write(
        top.join("src/pkg_top.loft"),
        "use pkg_dep::*;\nuse self::catalogue as m;\n\
         pub fn top_answer() -> integer { dep_answer() }\n\
         pub fn top_label() -> text { m::make_row().label }\n",
    )
    .unwrap();
    std::fs::write(
        top.join("tests/answer.loft"),
        "use pkg_top::*;\n\
         fn main() { println(\"dep_answer={top_answer()} dep_tag={dep_tag()} top_label={top_label()}\"); }\n",
    )
    .unwrap();

    let mut bin = std::env::current_exe().expect("test binary path");
    bin.pop();
    if bin.ends_with("deps") {
        bin.pop();
    }
    let out = std::process::Command::new(bin.join("loft"))
        .arg("test")
        .current_dir(&top)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_AUTO_INSTALL", "1")
        .output()
        .expect("invoke loft test");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        all.contains("dep_answer=42 dep_tag=dep top_label=top"),
        "both packages' `catalogue` modules must stay reachable, each answering its \
         own:\n{all}"
    );
}

/// `use self::<m>` accepts the same import spec as `use <lib>` — the grammar is shared,
/// so the two spellings cannot drift into accepting different things.
///
/// The refused case is the point of the last row: the flat comma list is rejected for
/// `self::` too, and the message quotes the spelling the author actually wrote
/// (`use self::tools::(a, b, …)`) rather than a bare-library one they did not.
#[test]
fn use_self_takes_the_same_import_spec_as_a_library() {
    let root = std::env::temp_dir().join(format!("loft_949_spec_{}", std::process::id()));
    let pkg = root.join("pkg");
    for (tag, body, want) in [
        ("plain", "use self::tools;", "1|2"),
        ("one", "use self::tools::one;", "1|"),
        ("group", "use self::tools::(one, two);", "1|2"),
        ("rename", "use self::tools::(one as first);", "1|"),
        ("star", "use self::tools::*;", "1|2"),
    ] {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(pkg.join("src")).expect("mkdir");
        std::fs::write(
            pkg.join("loft.toml"),
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nentry = \"src/pkg.loft\"\n",
        )
        .unwrap();
        std::fs::write(
            pkg.join("src/tools.loft"),
            "pub fn one() -> integer { 1 }\npub fn two() -> integer { 2 }\n",
        )
        .unwrap();
        // `first` is bound only in the rename row; every row prints what it imported.
        let call = if tag == "rename" { "first()" } else { "one()" };
        let second = if want.ends_with('2') { "two()" } else { "\"\"" };
        std::fs::write(
            pkg.join("src/pkg.loft"),
            format!("{body}\nfn main() {{ println(\"{{{call}}}|{{{second}}}\") }}\n"),
        )
        .unwrap();

        let mut p = Parser::new();
        p.parse_dir("default", true, true).unwrap();
        p.parse(&pkg.join("src/pkg.loft").to_string_lossy(), false);
        assert!(
            p.diagnostics.level() < Level::Error,
            "`{body}` must parse:\n{}",
            p.diagnostics.lines().join("\n")
        );
    }

    // The flat comma list is refused for `self::` exactly as it is for a library.
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(pkg.join("src")).expect("mkdir");
    std::fs::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nentry = \"src/pkg.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        pkg.join("src/tools.loft"),
        "pub fn one() -> integer { 1 }\npub fn two() -> integer { 2 }\n",
    )
    .unwrap();
    std::fs::write(
        pkg.join("src/pkg.loft"),
        "use self::tools::one, two;\nfn main() { println(\"{one()}\") }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("src/pkg.loft").to_string_lossy(), false);
    let all = p.diagnostics.lines().join("\n");
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        all.contains("use self::tools::(a, b, …)"),
        "the grouping error must quote the spelling the author wrote:\n{all}"
    );
}

/// `use self::<module>` must NOT fall through to the wider `lib_path` search when the
/// module is absent.
///
/// Bare `use` searches outward by design — project `lib/`, sibling packages, the
/// registry. Letting `self::` do the same would mean a typo silently binds some other
/// package's file, which is the exact outcome the spelling exists to prevent. So an
/// absent module is an error that names the package it looked in.
#[test]
fn use_self_refuses_to_search_outside_its_own_package() {
    let root = std::env::temp_dir().join(format!("loft_949_absent_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let pkg = root.join("pkg_solo");
    std::fs::create_dir_all(pkg.join("src")).expect("mkdir src");
    std::fs::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"pkg_solo\"\nversion = \"0.1.0\"\nentry = \"src/pkg_solo.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        pkg.join("src/pkg_solo.loft"),
        "use self::nope;\npub fn solo() -> integer { 1 }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("src/pkg_solo.loft").to_string_lossy(), false);
    let all = p.diagnostics.lines().join("\n");
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        p.diagnostics.level(),
        Level::Error,
        "an absent self-module must be an error, not a silent outward search:\n{all}"
    );
    assert!(
        all.contains("pkg_solo") && all.contains("nope"),
        "the error must name the package it searched and the module it wanted:\n{all}"
    );
}

/// `self` needs a package to mean something.  In a bare script there is no
/// `[package] name` to qualify with, so the spelling is refused — and the message
/// carries the two ways forward rather than only the refusal.
#[test]
fn use_self_outside_a_package_says_what_to_do_instead() {
    let root = std::env::temp_dir().join(format!("loft_949_nopkg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir root");
    std::fs::write(
        root.join("helper.loft"),
        "pub fn helper() -> integer { 1 }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("bare.loft"),
        "use self::helper;\nfn main() { println(\"{helper()}\") }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&root.join("bare.loft").to_string_lossy(), false);
    let all = p.diagnostics.lines().join("\n");
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        p.diagnostics.level(),
        Level::Error,
        "expected an error:\n{all}"
    );
    assert!(
        all.contains("loft.toml") && all.contains("use helper;"),
        "the message must name both cures — add a manifest, or take the module by its \
         shared name:\n{all}"
    );
}

/// The other side of that refusal: where `self::` is unavailable, the clash advice must
/// not prescribe it (loft#949).
///
/// `use self::<id>` qualifies with `[package] name`, so a bare script has nothing to
/// qualify with and the spelling is refused — the test above pins that. An advice that
/// recommended it anyway would hand the reader a cure that errors, which is the failure
/// the `self::` work set out to avoid in its own ambiguity message. So the cure named
/// here is keyed to whether the file HAS a package, and a bare script hears the rename.
#[test]
fn the_clash_advice_outside_a_package_does_not_prescribe_self() {
    let root = std::env::temp_dir().join(format!("loft_949_advice_nopkg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let a = root.join("dir_a");
    let b = root.join("dir_b");
    std::fs::create_dir_all(&a).expect("mkdir a");
    std::fs::create_dir_all(&b).expect("mkdir b");
    // Neither directory carries a `loft.toml`, so neither file is in a package.
    std::fs::write(
        a.join("catalogue.loft"),
        "pub fn a_only() -> integer { 1 }\n",
    )
    .unwrap();
    std::fs::write(
        b.join("catalogue.loft"),
        "pub fn b_only() -> integer { 2 }\n",
    )
    .unwrap();
    std::fs::write(
        b.join("bare.loft"),
        "use catalogue;\nfn main() { println(\"{b_only()}\") }\n",
    )
    .unwrap();

    // Driven through the BINARY: the clash needs `catalogue` to resolve OUTWARD to
    // dir_a, which is what `--lib` sets up and what an in-process `Parser` has no
    // search path for.  Without it the `use` binds dir_b's own file, there is no
    // clash, and the test would assert nothing — which is how it first failed.
    let mut bin = std::env::current_exe().expect("test binary path");
    bin.pop();
    if bin.ends_with("deps") {
        bin.pop();
    }
    let out = std::process::Command::new(bin.join("loft"))
        .args(["--interpret", "--path", env!("CARGO_MANIFEST_DIR"), "--lib"])
        .arg(&a)
        .arg(b.join("bare.loft"))
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("invoke loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        all.contains("module 'catalogue' is declared by two files"),
        "the scaffold must still produce the clash, or this asserts nothing:\n{all}"
    );
    assert!(
        !all.contains("use self::"),
        "a bare script cannot spell `use self::` — recommending it prescribes an error:\n{all}"
    );
    assert!(
        all.contains("Rename one file"),
        "so it must hear the cure that does work for it:\n{all}"
    );
}
