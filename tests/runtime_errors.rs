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

/// @P356 — run a snippet under `--interpret` with `LOFT_DEV_SOFT_HALT=1`,
/// the opt-in fail-fast mode that surfaces RECOVERABLE faults (OOB /
/// negative index) loudly (stderr `soft-halt:` + non-zero exit) for
/// debugging.  Verifies the structured-fault rendering still works for the
/// index kinds, which now log-and-continue by default.
fn run_loft_snippet_soft_halt(name: &str, source: &str) -> (String, String, Option<i32>) {
    let script_path = std::env::temp_dir().join(format!("loft_{name}.loft"));
    std::fs::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .env("LOFT_DEV_SOFT_HALT", "1")
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

/// C80 / E-Uncomp — integer `/` by zero is a recoverable calculation fault:
/// null sentinel + continue (exit 0), not a halt.  (Was: a `DivideByZero`
/// dev-halt pretty error — reversed by C80.)
#[test]
fn kind_divide_by_zero_int_returns_null_and_continues() {
    let source = "\
fn main() {
  a = 10;
  b = 0;
  c = a / b;
  print(\"{c}\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_div0_int", source);
    // C80 / E-Uncomp: integer `/` by zero is a RECOVERABLE calculation fault — it
    // yields the null sentinel and CONTINUES (exit 0), like OOB below.  (Formerly
    // a dev-halt pretty error; reversed by C80.)
    assert_eq!(
        code,
        Some(0),
        "div-by-zero is null-and-continue (exit 0); stderr={stderr:?}"
    );
    assert!(
        stdout.contains("null"),
        "post-fault stdout should print the null result; got: {stdout:?}"
    );
    assert!(
        !stderr.contains("error:"),
        "div-by-zero must NOT raise a typed error; got: {stderr:?}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "stderr still contains a Rust panic — bypassed the null-and-continue path; got: {stderr:?}"
    );
}

/// C80 / E-Uncomp — integer `%` by zero is the same recoverable fault as `/`:
/// null sentinel + continue (exit 0), not a halt.
#[test]
fn kind_mod_by_zero_int_returns_null_and_continues() {
    let source = "\
fn main() {
  z = 0;
  c = 7 % z;
  print(\"{c}\\n\");
}
";
    let (stdout, stderr, code) = run_loft_snippet("rt_mod0_int", source);
    assert_eq!(
        code,
        Some(0),
        "mod-by-zero is null-and-continue (exit 0); stderr={stderr:?}"
    );
    assert!(
        stdout.contains("null"),
        "mod-by-zero should yield null and continue; got: {stdout:?}"
    );
    assert!(
        !stderr.contains("error:"),
        "mod-by-zero must NOT raise a typed error; got: {stderr:?}"
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

/// @P356 — vector positive-index OOB is a RECOVERABLE fault: by default
/// `v[5]` on a length-3 vector returns `null` and execution CONTINUES
/// (exit 0).  Runtime aborts for reversible faults belong only in opt-in
/// debugging; the value is the type's null sentinel and the compile-time
/// warning already nudged toward `v[i] ?? <fallback>`.  Both backends
/// behave identically.
#[test]
fn kind_index_out_of_bounds_vector_returns_null_and_continues() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5];
  print(\"x={x}\\n\");
}
";
    let (stdout, _stderr, code) = run_loft_snippet("rt_oob_vec", source);
    assert_eq!(code, Some(0), "recoverable OOB should exit 0");
    assert!(
        stdout.contains("x=null"),
        "OOB index should yield null and continue; got: {stdout:?}"
    );
}

/// @P356 — `LOFT_DEV_SOFT_HALT` fail-fast still surfaces the structured OOB
/// fault (idx / len rendered inline) for debugging, so the diagnostic path
/// is preserved even though the default is now log-and-continue.
#[test]
fn kind_index_out_of_bounds_vector_soft_halt_surfaces() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5];
  print(\"x={x}\\n\");
}
";
    let (_stdout, stderr, code) = run_loft_snippet_soft_halt("rt_oob_vec_sh", source);
    assert_eq!(code, Some(1), "soft-halt OOB should exit non-zero");
    assert!(
        stderr.contains("index 5 out of bounds for length 3"),
        "soft-halt stderr missing structured OOB message; got: {stderr:?}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "stderr still contains a Rust panic; got: {stderr:?}"
    );
}

/// @P356 — vector negative index out of range after Python-style addressing
/// is RECOVERABLE: `v[-1]` (one before the end) still works; `v[-N]` with
/// `N > len` returns `null` and continues (exit 0).  Soft-halt surfaces the
/// `NegativeIndex` fault.
#[test]
fn kind_negative_index_vector_returns_null_and_continues() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  print(\"v[-1]={v[-1]}\\n\");
  x = v[-10];
  print(\"x={x}\\n\");
}
";
    let (stdout, _stderr, code) = run_loft_snippet("rt_neg_idx_vec", source);
    assert_eq!(code, Some(0), "recoverable negative index should exit 0");
    assert!(
        stdout.contains("v[-1]=30"),
        "negative-index Python-style addressing should still work; got: {stdout:?}"
    );
    assert!(
        stdout.contains("x=null"),
        "out-of-range negative index should yield null and continue; got: {stdout:?}"
    );

    let (_so, stderr, sh) = run_loft_snippet_soft_halt("rt_neg_idx_vec_sh", source);
    assert_eq!(sh, Some(1), "soft-halt negative index should exit non-zero");
    assert!(
        stderr.contains("negative index -10"),
        "soft-halt stderr missing structured NegativeIndex message; got: {stderr:?}"
    );
}

/// @P356 — vector-of-struct-ref OOB goes through `OpVectorRef` (separate
/// dispatch from primitive `OpGetVector`); regression guard that the
/// struct-ref path is ALSO recoverable + surfaced under soft-halt.
#[test]
fn kind_index_out_of_bounds_vector_of_struct_soft_halt_surfaces() {
    let source = "\
struct P { v: integer }
fn main() {
  v = [P{v:1}, P{v:2}, P{v:3}];
  x = v[5];
}
";
    let (_stdout, _stderr, code) = run_loft_snippet("rt_oob_struct_vec", source);
    assert_eq!(code, Some(0), "recoverable struct-ref OOB should exit 0");

    let (_so, stderr, sh) = run_loft_snippet_soft_halt("rt_oob_struct_vec_sh", source);
    assert_eq!(sh, Some(1), "soft-halt struct-ref OOB should exit non-zero");
    assert!(
        stderr.contains("index 5 out of bounds for length 3"),
        "soft-halt stderr missing struct-ref OOB; got: {stderr:?}"
    );
}

/// @P356 — text positive-index OOB is RECOVERABLE: `s[100]` on a length-5
/// string returns the null char and continues (exit 0); soft-halt surfaces
/// the structured `IndexOutOfBounds`.
#[test]
fn kind_index_out_of_bounds_text_returns_null_and_continues() {
    let source = "\
fn main() {
  s = \"hello\";
  c = s[100];
  print(\"after={s}\\n\");
}
";
    let (stdout, _stderr, code) = run_loft_snippet("rt_oob_text", source);
    assert_eq!(code, Some(0), "recoverable text OOB should exit 0");
    assert!(
        stdout.contains("after=hello"),
        "text OOB should continue past the fault; got: {stdout:?}"
    );

    let (_so, stderr, sh) = run_loft_snippet_soft_halt("rt_oob_text_sh", source);
    assert_eq!(sh, Some(1), "soft-halt text OOB should exit non-zero");
    assert!(
        stderr.contains("index 100 out of bounds for length 5"),
        "soft-halt stderr missing text-OOB; got: {stderr:?}"
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

// ── loft#1053 — a fault inside a `par` worker stops the WHOLE program ─────────────────
//
// `assert` and `panic` are not exceptions; they are the program's own request to stop, and
// the decided semantics is that the stop is total — the whole program, not one arm of it.
// A worker raises against its own `Stores` CLONE, which is dropped at join, so every halt
// raised inside a worker used to die with it: the arms printed nothing and the program
// exited 0, having computed a result from workers that had all failed their assertions
// (`par_fold` cheerfully returned 190).
//
// One case per par FAMILY, because the fix was first written for the block form alone and
// the other three stayed silent — the families do not share a spawn path, and a guard on
// one proves nothing about the others.

/// The shape each case shares: a worker that always fails, and a program that would print
/// `survived` if the halt were swallowed.
fn assert_par_family_halts(name: &str, source: &str, expected: &str) {
    let (stdout, stderr, code) = run_loft_snippet(name, source);
    assert_eq!(
        code,
        Some(1),
        "{name}: a failing worker must stop the whole program\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(expected),
        "{name}: expected {expected:?} in stderr, got: {stderr}"
    );
    assert!(
        !stdout.contains("survived"),
        "{name}: the program continued past the failing par construct\nstdout: {stdout}"
    );
}

#[test]
fn i1053_parallel_block_arm_assert_halts_the_program() {
    assert_par_family_halts(
        "i1053_block",
        "fn w(n: integer) -> integer { assert(false, \"BLOCK ARM\"); n }\n\
         fn main() { o = 1; parallel { w(o); w(o); } println(\"survived\"); }\n",
        "assertion failed: BLOCK ARM",
    );
}

#[test]
fn i1053_par_queue_worker_assert_halts_the_program() {
    assert_par_family_halts(
        "i1053_queue",
        "fn w(n: integer) -> integer { assert(false, \"QUEUE WORKER\"); n * 2 }\n\
         fn main() { v: vector<integer> = []; for i in 0..6 { v += [i]; } t = 0;\n\
         \x20 for a in v par(b = w(a), 2) { t += b; } println(\"survived t={t}\"); }\n",
        "assertion failed: QUEUE WORKER",
    );
}

#[test]
fn i1053_par_discard_worker_assert_halts_the_program() {
    assert_par_family_halts(
        "i1053_discard",
        "fn w(n: integer) -> integer { assert(false, \"DISCARD WORKER\"); n }\n\
         fn main() { v: vector<integer> = []; for i in 0..6 { v += [i]; }\n\
         \x20 for a in v par(b = w(a), 2) { } println(\"survived\"); }\n",
        "assertion failed: DISCARD WORKER",
    );
}

#[test]
fn i1053_par_fold_worker_assert_halts_the_program() {
    assert_par_family_halts(
        "i1053_fold",
        "fn add(a: integer, b: integer) -> integer { assert(false, \"FOLD WORKER\"); a + b }\n\
         fn main() { v: vector<integer> = []; for i in 0..20 { v += [i]; }\n\
         \x20 println(\"survived {par_fold(v, 0, add, 2)}\"); }\n",
        "assertion failed: FOLD WORKER",
    );
}

/// `panic` too, not just `assert` — they share the halt path, and a fix that covered only
/// the assert would leave the louder of the two silent.
#[test]
fn i1053_par_worker_panic_halts_the_program() {
    assert_par_family_halts(
        "i1053_panic",
        "fn w(n: integer) -> integer { panic(\"QUEUE PANIC\"); n }\n\
         fn main() { v: vector<integer> = []; for i in 0..6 { v += [i]; } t = 0;\n\
         \x20 for a in v par(b = w(a), 2) { t += b; } println(\"survived\"); }\n",
        "panic: QUEUE PANIC",
    );
}

/// The other half: a clean par program must be untouched.  A halt-on-worker-fault change
/// that also stopped healthy runs would pass every test above.
#[test]
fn i1053_clean_par_programs_are_unaffected() {
    let (stdout, stderr, code) = run_loft_snippet(
        "i1053_clean",
        "fn w(n: integer) -> integer { n * 2 }\n\
         fn add(a: integer, b: integer) -> integer { a + b }\n\
         fn main() { v: vector<integer> = []; for i in 0..10 { v += [i]; } t = 0;\n\
         \x20 for a in v par(b = w(a), 2) { t += b; }\n\
         \x20 println(\"t={t} s={par_fold(v, 0, add, 2)}\"); }\n",
    );
    assert_eq!(code, Some(0), "a clean par program must exit 0: {stderr}");
    assert!(
        stdout.contains("t=90") && stdout.contains("s=45"),
        "clean par results must be unchanged, got: {stdout}"
    );
}
