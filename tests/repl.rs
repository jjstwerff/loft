// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 04 — interactive REPL shell, end-to-end.
//!
//! Drives the `loft` binary with piped stdin and asserts on stdout (where
//! evaluated results print; the prompt + errors go to stderr, discarded here).

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `loft <args>` feeding `input` on stdin, return captured stdout.
fn repl(args: &[&str], input: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn loft");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Acceptance #1 — `loft repl` evaluates input and prints results; a binding
/// persists so a later read shows it.
#[test]
fn basic_session_prints_result() {
    let out = repl(&["repl"], "x = 40 + 2\nx\n:quit\n");
    assert!(out.contains("42"), "expected 42 in stdout, got {out:?}");
}

/// A bare `loft` (no args) drops into the REPL.
#[test]
fn bare_loft_starts_repl() {
    let out = repl(&[], "1 + 2\n:quit\n");
    assert!(out.contains('3'), "expected 3 in stdout, got {out:?}");
}

/// Acceptance #2 — multi-line input (a function spanning lines) works, and the
/// function is callable afterward.
#[test]
fn multi_line_fn_then_call() {
    let out = repl(
        &["repl"],
        "fn dbl(n: integer) -> integer {\nn + n\n}\ndbl(21)\n:quit\n",
    );
    assert!(out.contains("42"), "expected 42 in stdout, got {out:?}");
}

/// Acceptance #3 — a parse error doesn't crash; the session keeps working.
#[test]
fn recovers_from_parse_error() {
    let out = repl(&["repl"], "x = 1 2 3\nx = 7\nx\n:quit\n");
    assert!(
        out.contains('7'),
        "session should continue after error: {out:?}"
    );
}

/// Struct results print in loft's native rendering.
#[test]
fn struct_result_echo() {
    let out = repl(
        &["repl"],
        "struct P { a: integer, b: integer }\nP { a: 1, b: 2 }\n:quit\n",
    );
    assert!(
        out.contains("a:1") && out.contains("b:2"),
        "struct echo: {out:?}"
    );
}

/// `:quit` exits cleanly even with no input after it.
#[test]
fn quit_exits() {
    let out = repl(&["repl"], ":quit\n");
    assert!(
        out.is_empty() || !out.contains("error"),
        "clean quit: {out:?}"
    );
}
