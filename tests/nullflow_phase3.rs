// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 null-flow Phase 3 — `(N-Domain)` for floats.
//!
//! Step 3.1: float / single `/` and `%` fault on a zero divisor, so they type `τ?` (like
//! integer `/`) under `LOFT_NULLFLOW`; the `divisor_provably_nonzero` elision keeps
//! `x / 2.0` non-null. Observed via the Phase-1 store warning (ON but not OFF).

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `(success, warning_count, stdout)`.
fn run(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, usize, String) {
    let script =
        std::env::temp_dir().join(format!("loft_nf3_{}_{tag}.loft", std::process::id()));
    std::fs::write(&script, body).expect("write script");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend).arg(&script).current_dir(workspace_root()).env("LOFT_TIMEOUT", "120");
    if nullflow { cmd.env("LOFT_NULLFLOW", "1"); } else { cmd.env_remove("LOFT_NULLFLOW"); }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warns = stderr.lines().filter(|l| l.starts_with("warning:")).count();
    (out.status.success(), warns, String::from_utf8_lossy(&out.stdout).into_owned())
}

/// float `/` a VARIABLE divisor — `float?` under nullflow.
const DIV_VAR: &str = "struct S { g: float }\n\
fn main() {\n  b = 0.0;\n  s = S { g: 0.0 };\n  s.g = 1.0 / b;\n  print(\"f={s.g}\\n\");\n}\n";
/// float `/` a CONSTANT divisor — non-null (provably non-zero).
const DIV_CONST: &str = "struct S { g: float }\n\
fn main() {\n  s = S { g: 0.0 };\n  s.g = 1.0 / 2.0;\n  print(\"f={s.g}\\n\");\n}\n";

#[test]
fn float_div_var_off_launders() {
    let (ok, warns, out) = run(DIV_VAR, "--interpret", false, "var_off");
    assert!(ok, "{out}");
    assert_eq!(warns, 0, "OFF: float / stays non-null");
}
#[test]
fn float_div_var_on_is_nullable_interpret() {
    let (ok, warns, out) = run(DIV_VAR, "--interpret", true, "var_on_i");
    assert!(ok, "{out}");
    assert_eq!(warns, 1, "ON: float / var is float?, so the store warns; out={out}");
    assert!(out.contains("f=null"));
}
#[test]
fn float_div_var_on_is_nullable_native() {
    let (ok, warns, out) = run(DIV_VAR, "--native", true, "var_on_n");
    assert!(ok, "{out}");
    assert_eq!(warns, 1, "ON native: float / var is float?; out={out}");
    assert!(out.contains("f=null"));
}
#[test]
fn float_div_const_stays_non_null() {
    let (ok, warns, out) = run(DIV_CONST, "--interpret", true, "const_on");
    assert!(ok, "{out}");
    assert_eq!(warns, 0, "ON: a constant non-zero divisor is provably safe → non-null");
    assert!(out.contains("f=0.5"));
}

/// `sqrt` of a negative — `float?` under nullflow (decl-gate), non-null when off.
const SQRT: &str = "struct S { g: float }\n\
fn main() {\n  x = -1.0;\n  s = S { g: 0.0 };\n  s.g = sqrt(x);\n  print(\"f={s.g}\\n\");\n}\n";

#[test]
fn sqrt_off_non_null_no_warning() {
    let (ok, warns, out) = run(SQRT, "--interpret", false, "sqrt_off");
    assert!(ok, "OFF: stdlib must load with sqrt stripped to non-null float: {out}");
    assert_eq!(warns, 0, "OFF: sqrt returns non-null float");
}
#[test]
fn sqrt_on_nullable_warns_interpret() {
    let (ok, warns, out) = run(SQRT, "--interpret", true, "sqrt_on_i");
    assert!(ok, "{out}");
    assert_eq!(warns, 1, "ON: sqrt returns float?, the store warns; out={out}");
    assert!(out.contains("f=null"));
}
#[test]
fn sqrt_on_nullable_warns_native() {
    let (ok, warns, out) = run(SQRT, "--native", true, "sqrt_on_n");
    assert!(ok, "{out}");
    assert_eq!(warns, 1, "ON native: sqrt returns float?; out={out}");
}
