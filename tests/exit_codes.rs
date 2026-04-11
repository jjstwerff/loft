// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Binary-level exit-code tests for L7.
//!
//! These tests invoke the compiled `loft` binary via `std::process::Command` so
//! they can verify the OS exit code — something the library-level test harness
//! cannot do.  The binary must be rebuilt (`cargo test` does this automatically
//! for integration tests).

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A program with no diagnostics must run and exit 0.
/// 46-caveats.loft is a clean caveat regression suite that should print "caveats: all ok".
#[test]
fn warning_only_program_exits_zero() {
    let script = workspace_root().join("tests/scripts/46-caveats.loft");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "expected exit 0 for warnings-only program, got {:?}; stdout={stdout:?}; stderr={stderr:?}",
        out.status.code()
    );
    assert!(
        stdout.contains("caveats: all ok"),
        "expected 'caveats: all ok' in output; got {stdout:?}"
    );
}

/// A program with a genuine parse error must exit non-zero.
#[test]
fn parse_error_exits_nonzero() {
    // Write a minimal syntax-error script to a temp file.
    let dir = std::env::temp_dir();
    let path = dir.join("loft_l7_test_parse_error.loft");
    std::fs::write(&path, "fn main() { x = 1\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    assert!(
        !out.status.success(),
        "expected non-zero exit for parse-error program, got exit 0"
    );
}

// ── P131: Loft CLI forwards script-level arguments (FIXED) ─────────────────
//
// `src/main.rs` now treats every token after the script path — including
// `--*` ones — as a script argument that is appended to `user_args` and
// forwarded to the script's `arguments()`. An explicit `--` separator is
// also accepted and skipped. The script must run cleanly when invoked
// with extra script-level arguments.
#[test]
fn p131_cli_forwards_script_dashdash_arg() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p131_args_test.loft");
    std::fs::write(&path, "fn main() { println(\"ran\"); }\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .arg("--mode")
        .arg("glb")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "expected exit 0 with --mode forwarded; stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("ran"),
        "expected script body to run; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// Explicit `--` separator must also be accepted (and consumed) before
/// script arguments.
#[test]
fn p131_cli_explicit_dashdash_separator() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p131_sep_test.loft");
    std::fs::write(&path, "fn main() { println(\"ran\"); }\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .arg("--")
        .arg("--mode")
        .arg("glb")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    assert!(
        out.status.success(),
        "expected exit 0 with `--` separator; stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// P131: `arguments()` must return only the script-level arguments,
/// not the loft binary name or loft CLI flags like `--interpret`.
#[test]
fn p131_arguments_returns_only_script_args() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p131_arguments_content.loft");
    // Print each argument on its own line so we can inspect them.
    std::fs::write(&path, "fn main() { for a in arguments() { println(a) } }\n")
        .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .arg("--mode")
        .arg("glb")
        .arg("extra")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "expected exit 0; stderr={stderr:?}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["--mode", "glb", "extra"],
        "arguments() should return only script-level args, not loft flags; got: {lines:?}"
    );
}

// ── W1.1: --html produces a self-contained HTML file ──────────────────────

/// W1.1: `--html` must produce a valid HTML file with embedded WASM.
/// Requires the `wasm32-unknown-unknown` rustup target — skipped in CI
/// environments where the target is not installed.
#[test]
fn w1_1_html_export_produces_file() {
    let dir = std::env::temp_dir();
    let src = dir.join("loft_w1_1_test.loft");
    let out = dir.join("loft_w1_1_test.html");
    std::fs::write(&src, "fn main() { println(\"html-ok\"); }\n").unwrap();
    let result = Command::new(loft_bin())
        .arg("--html")
        .arg(&out)
        .arg(&src)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&src);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    if stderr.contains("wasm32-unknown-unknown") && stderr.contains("not be installed") {
        eprintln!("SKIP: wasm32-unknown-unknown target not installed");
        return;
    }
    assert!(
        result.status.success(),
        "expected --html to succeed; stdout={stdout:?}; stderr={stderr:?}"
    );
    let html = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    assert!(
        html.contains("<!DOCTYPE html>"),
        "HTML should start with doctype"
    );
    assert!(
        html.contains("loft_start"),
        "HTML should reference loft_start entry point"
    );
    assert!(
        html.contains("buildLoftImports"),
        "HTML should contain the GL bridge"
    );
    // WASM binary is embedded as base64 — file should be substantial
    assert!(
        html.len() > 5000,
        "HTML too small ({} bytes) — WASM likely missing",
        html.len()
    );
}
