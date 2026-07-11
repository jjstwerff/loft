// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 null-flow Phase 4 — `(N-Cast)`: a cast `as τ` is an assertion; the text parse folds in.
//!
//! Under `LOFT_NULLFLOW`, a BARE `text as τ` is a compile error (a parse can't be proven),
//! directing to the checked `as τ?` (value or null) or the assert-or-default `as τ ?? d`.
//! OFF keeps the DN3 auto-`τ?` parse. See float-null-domain-typing.md § Implementation plan.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf { std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")) }
fn workspace_root() -> std::path::PathBuf { std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

fn run(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, String, String) {
    let script = std::env::temp_dir().join(format!("loft_nf4_{}_{tag}.loft", std::process::id()));
    std::fs::write(&script, body).expect("write script");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend).arg(&script).current_dir(workspace_root()).env("LOFT_TIMEOUT", "120");
    if nullflow { cmd.env("LOFT_NULLFLOW", "1"); } else { cmd.env_remove("LOFT_NULLFLOW"); }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script);
    (out.status.success(),
     String::from_utf8_lossy(&out.stdout).into_owned(),
     String::from_utf8_lossy(&out.stderr).into_owned())
}

const BARE: &str = "fn main() { s = \"3.14\"; x = s as float; print(\"r={x}\\n\"); }\n";
const COAL: &str = "fn main() { s = \"3.14\"; x = s as float ?? 0.0; print(\"r={x}\\n\"); }\n";
const CHECKED: &str = "fn main() { s = \"abc\"; x = s as float?; print(\"r={x ?? -1.0}\\n\"); }\n";

#[test]
fn bare_text_cast_off_is_auto_nullable() {
    let (ok, out, _e) = run(BARE, "--interpret", false, "bare_off");
    assert!(ok, "OFF: bare `text as float` stays the DN3 auto-nullable parse: {out}");
    assert!(out.contains("r=3.14"));
}
#[test]
fn bare_text_cast_on_is_a_compile_error() {
    let (ok, _o, err) = run(BARE, "--interpret", true, "bare_on");
    assert!(!ok, "ON: a bare `text as float` must be a compile error");
    assert!(err.contains("may fail") && err.contains("float?"), "stderr: {err}");
}
#[test]
fn text_cast_coalesce_on_ok_interpret() {
    let (ok, out, err) = run(COAL, "--interpret", true, "coal_i");
    assert!(ok, "ON: `as float ?? d` is the assert-or-default form; stderr: {err}");
    assert!(out.contains("r=3.14"), "{out}");
}
#[test]
fn text_cast_coalesce_on_ok_native() {
    let (ok, out, err) = run(COAL, "--native", true, "coal_n");
    assert!(ok, "ON native: {err}");
    assert!(out.contains("r=3.14"), "{out}");
}
#[test]
fn text_cast_checked_on_yields_null_on_bad_parse() {
    let (ok, out, err) = run(CHECKED, "--interpret", true, "chk_i");
    assert!(ok, "ON: `as float?` is the checked cast; stderr: {err}");
    assert!(out.contains("r=-1"), "bad parse → null → ?? -1.0; out={out}");
}
