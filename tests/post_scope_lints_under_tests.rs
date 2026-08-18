// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#985 — the post-scope-check lints run on the TEST path, not only on the program path.
//!
//! Five lints share one precondition: they read the ownership verdicts and the materialised
//! copies that exist only after `scopes::check`. That put them in one block on `main.rs`'s
//! program path — and `loft test` / `--tests`, which is the path a LIBRARY's CI takes, ran
//! none of them. A library could ship a `#superseded` steer pointing at nothing (a hard
//! ERROR anywhere else) and writes that land in a copy, with a completely green suite.
//!
//! That is the hole the lint was written for: @PLN107's motivating case is a published
//! `graphics` canvas that shipped every drawing primitive as a no-op through the
//! copy-mutate shape, and its CI is `LOFT_DENY_WARNINGS=1 loft --interpret --tests tests`.
//!
//! What made it hard to notice is that the split is INSIDE the diagnostic set:
//! `warning[never-read]` fires under `--tests` and always did, so "tests are quiet" was
//! never the rule.
//!
//! The two questions the fix had to answer, both pinned below:
//!
//!   - **where** — once per loaded FILE, after `scopes::check`, and BEFORE the diagnostics
//!     are collected for the reader. The scope check used to run after test discovery,
//!     which is why nothing it produced could ever be seen.
//!   - **how often** — once, not once per test. Every test compiles its own bytecode from
//!     one `Data`, so a per-test call would report each finding N times
//!     ([`one_finding_is_reported_once_across_many_tests`]).

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run `source` under `--tests`; return `(stdout+stderr, exit code)`.
fn run_tests(tag: &str, source: &str, env: &[(&str, &str)]) -> (String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("loft_985_{tag}_{}.loft", std::process::id()));
    std::fs::write(&path, source).expect("write probe");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg("--tests")
        .arg(&path)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code(),
    )
}

/// A `#superseded` steer whose successor does not resolve. This is an **error** on the
/// program path — the whole point of @PLN102 arc C is that a signpost can never point at
/// nothing — and a test run reported `1 passed` while the advice told the reader to use a
/// name that does not exist.
const DANGLING_STEER: &str = "\
fn scaled(v: integer, by: integer) -> integer { v * by }
#superseded \"no_such_thing\"
fn doubled(v: integer) -> integer { scaled(v, 2) }
fn test_it() { assert(doubled(21) == 42, \"still answers\"); }
";

/// The motivating shape: a whole-value bind COPIES (C86), so the write lands in the copy
/// and the caller's canvas never changes. The test asserting that it did NOT change passes
/// either way — which is exactly why the lint has to be the thing that speaks.
const LOST_WRITE: &str = "\
struct Canvas { data: vector<integer> }
fn paint(c: Canvas, at: integer, colour: integer) {
    d = c.data;
    d[at] = colour;
}
fn test_paint() {
    c = Canvas { data: [0, 0, 0] };
    paint(c, 1, 9);
    assert(c.data[1] == 0, \"the canvas never changed\");
}
";

#[test]
fn a_dangling_superseded_steer_fails_a_test_run() {
    let (out, code) = run_tests("steer", DANGLING_STEER, &[]);
    assert!(
        out.contains("superseded-unknown-successor"),
        "the hard error must reach the test path too — a library whose CI is a test run \
         could otherwise publish a steer that points at nothing (loft#985)\n{out}"
    );
    assert_ne!(
        code,
        Some(0),
        "and it must FAIL the run, exactly as it fails a program compile\n{out}"
    );
}

#[test]
fn a_lost_write_is_reported_under_tests() {
    let (out, _) = run_tests("lostwrite", LOST_WRITE, &[]);
    assert!(
        out.contains("lost-write"),
        "the copy-mutate write is lost and the passing assertion cannot say so — the lint \
         has to (loft#985)\n{out}"
    );
}

/// The half that matters for a library: its CI is `LOFT_DENY_WARNINGS=1 … --tests`, so the
/// warning must not merely print — it must fail the file.
#[test]
fn deny_warnings_fails_the_file_on_a_lost_write() {
    let (out, _) = run_tests("deny", LOST_WRITE, &[("LOFT_DENY_WARNINGS", "1")]);
    assert!(
        out.contains("deny-warnings") && out.contains("FAIL"),
        "a library's CI must go red on it, not just print it (loft#985)\n{out}"
    );
}

/// Once per loaded file, not once per test. Each test compiles its own bytecode from the
/// same `Data`, so calling the lints per test would say the same thing three times.
#[test]
fn one_finding_is_reported_once_across_many_tests() {
    let source = "\
struct Canvas { data: vector<integer> }
fn paint(c: Canvas, at: integer, colour: integer) {
    d = c.data;
    d[at] = colour;
}
fn test_a() { c = Canvas { data: [0] }; paint(c, 0, 1); assert(c.data[0] == 0, \"a\"); }
fn test_b() { c = Canvas { data: [0] }; paint(c, 0, 2); assert(c.data[0] == 0, \"b\"); }
fn test_c() { c = Canvas { data: [0] }; paint(c, 0, 3); assert(c.data[0] == 0, \"c\"); }
";
    let (out, _) = run_tests("once", source, &[]);
    let hits = out.matches("lost-write").count();
    assert_eq!(
        hits, 1,
        "three tests, one finding, one report — a per-test call would say it three times \
         (loft#985)\n{out}"
    );
    assert!(
        out.contains("3 passed"),
        "and all three tests must still run\n{out}"
    );
}

/// The control for the split that made this hard to see: a parse-time diagnostic already
/// reached the test path, so a blanket "tests are quiet" reading was never right, and the
/// fix must not have changed it.
#[test]
fn a_parse_time_warning_still_reaches_the_test_path() {
    let source = "\
fn test_unused() {
    unread = 5;
    assert(1 == 1, \"fine\");
}
";
    let (out, _) = run_tests("neverread", source, &[]);
    assert!(
        out.contains("never-read"),
        "the parse-time lint reached --tests before this change and must still\n{out}"
    );
}
