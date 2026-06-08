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

// ── phase 05: introspection commands ─────────────────────────────────────────

const DEF: &str = "fn dbl(n: integer) -> integer { n + n }\n";

#[test]
fn bytecode_command() {
    let out = repl(&["repl"], &format!("{DEF}:bytecode dbl\n:quit\n"));
    assert!(
        out.contains("n_dbl") && out.contains("=== bytecode ==="),
        "bytecode: {out:?}"
    );
}

#[test]
fn rust_command() {
    let out = repl(&["repl"], &format!("{DEF}:rust dbl\n:quit\n"));
    assert!(out.contains("fn n_dbl"), "rust: {out:?}");
}

#[test]
fn slots_command() {
    let out = repl(&["repl"], &format!("{DEF}:slots dbl\n:quit\n"));
    assert!(
        out.contains("n_dbl") && out.contains("arg"),
        "slots: {out:?}"
    );
}

#[test]
fn fns_command_lists_user_fns() {
    let out = repl(
        &["repl"],
        &format!("{DEF}fn inc(x: integer) -> integer {{ x + 1 }}\n:fns\n:quit\n"),
    );
    assert!(out.contains("dbl -> integer"), "fns: {out:?}");
    assert!(out.contains("inc -> integer"), "fns: {out:?}");
}

/// An unknown `:command` doesn't crash the session.
#[test]
fn unknown_command_is_safe() {
    let out = repl(&["repl"], ":nonsense\n1 + 1\n:quit\n");
    assert!(
        out.contains('2'),
        "session continues after unknown cmd: {out:?}"
    );
}

// ── result echo / :type / runtime-error reporting ────────────────────────────

/// Run `loft <args>` feeding `input`, return (stdout, stderr).
fn repl_full(args: &[&str], input: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn loft");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A bare string-literal expression echoes its value (the bind-then-print path,
/// which earlier failed on the nested quotes).
#[test]
fn text_literal_echoes() {
    let out = repl(&["repl"], "\"hi there\"\n:quit\n");
    assert!(out.contains("hi there"), "text echo: {out:?}");
}

/// `:type <expr>` reports the inferred type without running the expression.
#[test]
fn type_command_infers() {
    let out = repl(&["repl"], "x = 5\n:type x + 1\n:type \"hi\"\n:quit\n");
    assert!(out.contains("integer"), "type of int expr: {out:?}");
    assert!(out.contains("text"), "type of text expr: {out:?}");
}

/// A failed `assert` is reported (not silently swallowed) and the session keeps
/// going — and the report is clean (no raw Rust panic backtrace).
#[test]
fn runtime_error_reported_and_recovers() {
    let (out, err) = repl_full(&["repl"], "assert(false, \"boom\")\n7\n:quit\n");
    assert!(err.contains("assertion failed"), "error reported: {err:?}");
    assert!(
        !err.contains("panicked at"),
        "no raw panic backtrace: {err:?}"
    );
    assert!(
        out.contains('7'),
        "session continues after error: out={out:?}"
    );
}

// ── :vars — value-bearing variable listing (REPL.T) ──────────────────────────

/// `:vars` prints each bound variable with its current value, in loft's native
/// rendering (text without quotes), to stdout.
#[test]
fn vars_command_shows_values() {
    let out = repl(&["repl"], "x = 5\nname = \"Alice\"\n:vars\n:quit\n");
    assert!(out.contains("x = 5"), "int var value: {out:?}");
    assert!(out.contains("name = Alice"), "text var value: {out:?}");
}

/// `:vars` reflects the *latest* value after a rebind, not the original.
#[test]
fn vars_command_reflects_latest_value() {
    let out = repl(&["repl"], "n = 5\nn = n + 100\n:vars\n:quit\n");
    assert!(out.contains("n = 105"), "rebound value: {out:?}");
}

/// `:vars` with nothing bound reports that (to stderr), and the session
/// continues.
#[test]
fn vars_command_empty_message() {
    let (_out, err) = repl_full(&["repl"], ":vars\n1 + 1\n:quit\n");
    assert!(err.contains("no variables"), "empty-vars message: {err:?}");
}

// ── session semantics: side-effecting bindings (REPL.X) ──────────────────────

/// A binding is recorded, not executed; the accumulated body re-runs on each
/// *observing* statement, so a side effect in a binding's RHS repeats once per
/// later observation.  Here `a = noisy()` prints "ran" each time `a` is
/// observed — two observations → "ran" twice.  This pins the known REPL.X
/// limitation (plan-12 § Deferred follow-ups); when stack-resident execution
/// lands, the side effect runs once and this expected count drops to 1.
#[test]
fn side_effecting_binding_reruns_per_observation() {
    let out = repl(
        &["repl"],
        "fn noisy() -> integer {\n  println(\"ran\");\n  42\n}\na = noisy()\na + 0\na + 0\n:quit\n",
    );
    let runs = out.matches("ran").count();
    assert_eq!(
        runs, 2,
        "side effect re-runs once per observation under REPL.X; got {runs} in {out:?}"
    );
}
