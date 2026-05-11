// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-07 phase 4 — typed runtime error binary tests.
//!
//! End-to-end check that the loft `panic("msg")` and failed
//! `assert(test, "msg")` builtins surface as a `RuntimeError`-rendered
//! pretty error (rustc-style `error:` header, `--> file:line:col`,
//! source line, caret) and exit non-zero — replacing the legacy Rust
//! panic that ate the call stack and printed an obscure `panicked at`
//! frame.
//!
//! Phase 4 step 4.1 + 4.2 + 4.11 + 4.13 are the foundation; later
//! steps (4.3-4.10, 4.12, 4.14, 4.15) add more kinds + backtrace
//! capture + the renderer's frame list.  Each new kind lands here as
//! a `kind_<name>` test.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run a loft snippet under `--interpret` and return (stdout, stderr,
/// exit-status-code).  Status is `Some(c)` when the process exited
/// normally; on signal, returns `None`.
fn run_loft_snippet(name: &str, source: &str) -> (String, String, Option<i32>) {
    let script_path = std::env::temp_dir().join(format!("loft_{name}.loft"));
    std::fs::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Phase 4 step 4.11 — `panic("msg")` builtin produces a typed runtime
/// error rendered through the phase-2 pretty renderer.  The rendered
/// stderr includes the kind label (`panic:`), the user message, the
/// source location, and a caret line.
#[test]
fn kind_user_panic_prints_pretty_error() {
    let source = "\
fn main() {
  panic(\"boom!\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_user_panic", source);
    assert_eq!(
        code,
        Some(1),
        "panic builtin should exit 1; stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stderr.contains("error:") && stderr.contains("panic: boom!"),
        "stderr missing 'error:' header or message; got: {stderr:?}"
    );
    assert!(
        stderr.contains("--> ") && stderr.contains(":2:"),
        "stderr missing source location pointing at line 2 (the panic call); got: {stderr:?}"
    );
    // Caret line ends the rendering — single `^` after the source line.
    assert!(
        stderr.contains("^"),
        "stderr missing caret marker; got: {stderr:?}"
    );
    // Ensure the LEGACY Rust panic message is GONE — the conversion's
    // whole point.  A `panicked at` line in stderr means the typed
    // error was bypassed.
    assert!(
        !stderr.contains("panicked at"),
        "stderr still contains a Rust panic — typed RuntimeError conversion bypassed; got: {stderr:?}"
    );
}

/// Phase 4 step 4.13 — failed `assert(test, "msg")` produces an
/// `AssertionFailed` typed error rendered through the same renderer.
/// Successful prints from earlier in the program still reach stdout
/// (the assert halts execution AT the failing call, not earlier).
#[test]
fn kind_assertion_failed_prints_pretty_error_after_partial_stdout() {
    let source = "\
fn main() {
  print(\"before\\n\");
  assert(2 + 2 == 5, \"math is broken\");
  print(\"after\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_assert", source);
    assert_eq!(code, Some(1), "failed assert should exit 1");
    assert!(
        stdout.contains("before"),
        "pre-assert stdout should still flush; got: {stdout:?}"
    );
    assert!(
        !stdout.contains("after"),
        "post-assert stdout should NOT print — execution must halt; got: {stdout:?}"
    );
    assert!(
        stderr.contains("error:") && stderr.contains("assertion failed: math is broken"),
        "stderr missing assertion-failed entry; got: {stderr:?}"
    );
    assert!(
        stderr.contains("--> ") && stderr.contains(":3:"),
        "stderr missing source location at line 3; got: {stderr:?}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "stderr still contains a Rust panic; got: {stderr:?}"
    );
}

/// Phase 4 step 4.3 — integer `/` by zero produces a `DivideByZero`
/// runtime error pointing at the `/` operator's source position
/// (Plan-07 phase 1's Span wrap on binary `/`).
#[test]
fn kind_divide_by_zero_int_prints_pretty_error() {
    let source = "\
fn main() {
  a = 10;
  b = 0;
  c = a / b;
  print(\"{c}\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_div0_int", source);
    assert_eq!(code, Some(1), "divide by zero should exit 1");
    assert!(
        !stdout.contains("c="),
        "post-fault stdout should NOT print; got: {stdout:?}"
    );
    assert!(
        stderr.contains("error:") && stderr.contains("divide by zero"),
        "stderr missing divide-by-zero entry; got: {stderr:?}"
    );
    // Caret should point at the `/` operator's column (Plan-07 phase 1
    // wraps binary `/` in `Value::Span` at the operator token).
    assert!(
        stderr.contains("--> ") && stderr.contains(":4:"),
        "stderr missing source location at line 4 (the `/` line); got: {stderr:?}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "stderr still contains a Rust panic — typed RuntimeError conversion bypassed; got: {stderr:?}"
    );
}

/// Phase 4 step 4.4 — integer `%` by zero produces the same
/// `DivideByZero` kind (mod-by-zero shares the kind because the
/// underlying op is the same indeterminate-result fault).
#[test]
fn kind_mod_by_zero_int_prints_pretty_error() {
    let source = "\
fn main() {
  c = 7 % 0;
  print(\"{c}\\n\");
}
";
    let (_stdout, stderr, code) = run_loft_snippet("rt_mod0_int", source);
    assert_eq!(code, Some(1), "mod by zero should exit 1");
    assert!(
        stderr.contains("error:") && stderr.contains("divide by zero"),
        "stderr missing divide-by-zero entry for `%`; got: {stderr:?}"
    );
}

/// The nullable `??`-rescued shape STILL produces the sentinel +
/// fallback; conversion to typed errors only changes the
/// non-nullable variant per the C54.G-hybrid design.  Regression
/// guard so a future "raise on every div" change doesn't break
/// `?? 0` programs.
#[test]
fn divide_by_zero_with_nullable_rescue_does_not_raise() {
    let source = "\
fn main() {
  a = 10;
  b = 0;
  c = a / b ?? 42;
  print(\"{c}\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_div0_nullable", source);
    assert_eq!(code, Some(0), "?? rescue should exit 0; stderr={stderr:?}");
    assert!(
        stdout.contains("42"),
        "expected `?? 42` to surface; got: {stdout:?}"
    );
    assert!(
        !stderr.contains("divide by zero"),
        "?? rescue path must NOT raise typed error; got: {stderr:?}"
    );
}

/// A program without a panic / failed assert exits 0 and writes
/// nothing to stderr from the runtime-error path — guards against the
/// renderer firing on an empty `runtime_error` slot.
#[test]
fn no_runtime_error_exits_clean() {
    let source = "\
fn main() {
  print(\"hi\\n\");
  assert(true, \"trivially true\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_none", source);
    assert_eq!(code, Some(0), "clean run should exit 0; stderr={stderr:?}");
    assert!(
        stdout.contains("hi"),
        "expected stdout 'hi'; got: {stdout:?}"
    );
    // Stderr might carry warnings/leak reports; only assert the
    // runtime-error renderer didn't fire.
    assert!(
        !stderr.contains("error:") || !stderr.contains("--> "),
        "renderer fired without a typed error; got: {stderr:?}"
    );
}
