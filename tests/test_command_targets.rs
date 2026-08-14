// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `loft test` never reports a green for a file it did not run (loft#916).
//!
//! Everything after the first target used to be dropped in silence:
//! `loft test good.loft alsogood.loft` ran the first, printed `ok … 1 file`, and
//! exited 0 — even though `alsogood.loft` contains a failing test.  The file count
//! was the only place it showed, and that reads as correct unless you already knew
//! how many you asked for.  Naming two files is the natural move when a change
//! touches two suites and the whole run is slow, which is exactly when nobody
//! re-reads the count; it cost a sabotage sweep whose second half never executed and
//! was reported green.
//!
//! One target per run, and a second one is now an ERROR rather than a drop.  The
//! rows below pin that, and pin that the spellings which DO work still do — a check
//! that refused an argument after any flag would break `--lib <dir>`, whose value is
//! also a bare token.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A package with one passing and one failing test file.
fn fixture(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("loft_916_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tests")).expect("mkdir tests");
    std::fs::write(
        root.join("loft.toml"),
        "[package]\nname = \"t916\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    std::fs::write(
        root.join("tests/good.loft"),
        "fn test_one() { assert(1 + 2 == 3, \"add\"); }\n",
    )
    .expect("good");
    std::fs::write(
        root.join("tests/alsogood.loft"),
        "fn test_two() { assert(2 + 2 == 5, \"THIS TEST FAILS ON PURPOSE\"); }\n",
    )
    .expect("alsogood");
    root
}

fn run(root: &PathBuf, args: &[&str]) -> (i32, String) {
    let out = Command::new(loft_bin())
        .current_dir(root)
        .args(args)
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft test");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// The defect: a second file must never be dropped into a green.  Asserted on the
/// EXIT CODE as well as the text, because the exit code is what a CI job reads and
/// it was the half that made this dangerous rather than merely confusing.
#[test]
fn a_second_target_is_refused_not_dropped() {
    let root = fixture("two");
    let (code, out) = run(&root, &["test", "good.loft", "alsogood.loft"]);
    assert_ne!(code, 0, "naming two files must not exit 0:\n{out}");
    assert!(
        out.contains("one target per run"),
        "the refusal must say what is wrong:\n{out}"
    );
    assert!(
        out.contains("good.loft") && out.contains("alsogood.loft"),
        "both targets must be named, so it is obvious which was dropped:\n{out}"
    );
    assert!(
        !out.contains("test result: ok."),
        "no green may be printed for a run that did not happen:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The order-swapped form from the report: it is the SECOND POSITION that was
/// dropped, not a particular file, so the refusal must not depend on which file
/// happens to fail.
#[test]
fn the_refusal_does_not_depend_on_which_file_fails() {
    let root = fixture("swapped");
    let (code, out) = run(&root, &["test", "alsogood.loft", "good.loft"]);
    assert_ne!(code, 0, "the swapped order must be refused too:\n{out}");
    assert!(
        out.contains("one target per run"),
        "the swapped order must be refused for the same reason:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The control that makes the refusal meaningful: with NO target both files run, and
/// the failing one is reported.  Without this row, a `loft test` that refused
/// everything would pass the tests above.
#[test]
fn no_target_still_runs_every_file() {
    let root = fixture("all");
    let (code, out) = run(&root, &["test"]);
    assert_ne!(code, 0, "the failing file must fail the run:\n{out}");
    assert!(
        out.contains("2 files"),
        "both files must run when none is named:\n{out}"
    );
    assert!(
        out.contains("THIS TEST FAILS ON PURPOSE"),
        "the failing assertion must be reported:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// One target still works, in each spelling — a bare file, a file with a `::`
/// selector, and a file followed by a FLAG.  The last is the row that matters for
/// the shape of the check: a flag's value is a bare token too, so a rule written as
/// "no bare token anywhere after the target" would break `--lib <dir>`.
#[test]
fn one_target_still_works_in_every_spelling() {
    let root = fixture("single");

    let (code, out) = run(&root, &["test", "good.loft"]);
    assert_eq!(code, 0, "a single passing file must pass:\n{out}");
    assert!(out.contains("1 file"), "and report one file:\n{out}");

    let (code, out) = run(&root, &["test", "alsogood.loft"]);
    assert_ne!(code, 0, "a single failing file must fail:\n{out}");

    let (code, out) = run(&root, &["test", "good.loft::test_one"]);
    assert_eq!(code, 0, "a `::selector` must still resolve:\n{out}");

    let (code, out) = run(&root, &["test", "good.loft", "--no-warnings"]);
    assert_eq!(code, 0, "a trailing flag is not a second target:\n{out}");

    let _ = std::fs::remove_dir_all(&root);
}
