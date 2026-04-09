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
