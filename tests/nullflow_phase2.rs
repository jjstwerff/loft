// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 null-flow Phase 2 — `(N-Prop)`: nullability propagates through arithmetic.
//!
//! Under `LOFT_NULLFLOW`, a binary op with a nullable operand yields a nullable result
//! (`integer? + integer → integer?`, `float - float? → float?`), because the runtime already
//! carries the null sentinel / NaN through. The propagation is observed as a Phase-1 store
//! WARNING that appears ON but not OFF. C85 is the complement — two non-null operands stay
//! non-null (overflow is a produced sentinel, not a propagated input). See
//! `doc/claude/plans/102-stability-contract/float-null-domain-typing.md` § Implementation plan.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `(success, stdout, warning_count)`.  `tag` keeps the temp script unique across parallel tests.
fn run(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, String, usize) {
    let script = std::env::temp_dir().join(format!("loft_nf2_{}_{tag}.loft", std::process::id()));
    std::fs::write(&script, body).expect("write script");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_TIMEOUT", "120");
    if nullflow {
        cmd.env("LOFT_NULLFLOW", "1");
    } else {
        cmd.env_remove("LOFT_NULLFLOW");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let script_removed = std::fs::remove_file(&script).is_ok();
    let _ = script_removed;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warns = stderr.lines().filter(|l| l.starts_with("warning:")).count();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        warns,
    )
}

/// `n = 10/y` is `integer?`; `x = n + 1` must stay `integer?` (N-Prop); storing `x` into the
/// non-null field `f` is then a Phase-1 warning.  `y = 2` so the value is a real `6` (the store
/// still completes) — the point is the TYPE, observed via the warning.
const PROP_INT: &str = "struct S { f: integer }\n\
fn main() {\n  y = 2;\n  n = 10 / y;\n  x = n + 1;\n  s = S { f: 0 };\n  s.f = x;\n  print(\"f={s.f}\\n\");\n}\n";

/// `1.0 - x` with `x: float?` must stay `float?` (N-Prop on the RIGHT operand).
const PROP_FLOAT: &str = "struct S { g: float }\n\
fn main() {\n  x: float? = null;\n  m = 1.0 - x;\n  s = S { g: 0.0 };\n  s.g = m;\n  print(\"g={s.g}\\n\");\n}\n";

/// Two non-null operands — C85: the result stays non-null, so NO store warning.
const C85: &str = "struct S { f: integer }\n\
fn main() {\n  a = 3; b = 4;\n  s = S { f: 0 };\n  s.f = a * b;\n  print(\"f={s.f}\\n\");\n}\n";

#[test]
fn prop_int_off_launders_no_warning() {
    let (ok, out, warns) = run(PROP_INT, "--interpret", false, "int_off");
    assert!(ok, "OFF should compile+run: {out}");
    assert_eq!(
        warns, 0,
        "OFF: n+1 launders to non-null, so the store must NOT warn"
    );
    assert!(out.contains("f=6"));
}

#[test]
fn prop_int_on_propagates_and_warns_interpret() {
    let (ok, out, warns) = run(PROP_INT, "--interpret", true, "int_on_i");
    assert!(ok, "ON should compile+run: {out}");
    assert_eq!(
        warns, 1,
        "ON: n+1 is integer? (N-Prop), so the store must warn once"
    );
    assert!(out.contains("f=6"), "value still flows: {out}");
}

#[test]
fn prop_int_on_propagates_and_warns_native() {
    let (ok, out, warns) = run(PROP_INT, "--native", true, "int_on_n");
    assert!(ok, "ON native: {out}");
    assert_eq!(
        warns, 1,
        "ON native: the store must warn once (N-Prop): {out}"
    );
    assert!(out.contains("f=6"));
}

#[test]
fn prop_float_on_right_operand_propagates() {
    let (ok, out, warns) = run(PROP_FLOAT, "--interpret", true, "flt_on");
    assert!(ok, "ON: {out}");
    assert_eq!(
        warns, 1,
        "ON: 1.0 - float? is float? (N-Prop on the right operand)"
    );
    assert!(out.contains("g=null"), "the null flows through: {out}");
}

#[test]
fn c85_non_null_arithmetic_stays_non_null() {
    let (ok, out, warns) = run(C85, "--interpret", true, "c85");
    assert!(ok, "ON: {out}");
    assert_eq!(
        warns, 0,
        "C85: two non-null operands → non-null result, no store warning"
    );
    assert!(out.contains("f=12"));
}
