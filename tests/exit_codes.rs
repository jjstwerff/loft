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

/// A program whose only diagnostic is a compile-time warning must still run
/// and exit 0.  46-caveats.loft triggers a C14 zero-padding-on-text warning
/// (`{t:05}`) but is otherwise valid and should print "caveats: all ok".
#[test]
#[ignore = "L7: binary exits 1 on any diagnostic including warnings — not fixed yet"]
fn warning_only_program_exits_zero() {
    let script = workspace_root().join("tests/scripts/46-caveats.loft");
    let out = Command::new(loft_bin())
        .arg(&script)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "expected exit 0 for warnings-only program, got {:?}; stdout={stdout:?}",
        out.status.code()
    );
    assert!(
        stdout.contains("caveats: all ok"),
        "expected 'caveats: all ok' in output; got {stdout:?}"
    );
}

/// A program with a genuine parse error must exit non-zero.
#[test]
#[ignore = "L7: exit-code tests use CARGO_BIN_EXE_loft"]
fn parse_error_exits_nonzero() {
    // Write a minimal syntax-error script to a temp file.
    let dir = std::env::temp_dir();
    let path = dir.join("loft_l7_test_parse_error.loft");
    std::fs::write(&path, "fn main() { x = 1\n").expect("write temp file");
    let out = Command::new(loft_bin())
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
