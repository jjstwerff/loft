// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 DN1 read-consumption regression (both backends).
//!
//! Exercises the `text?` READ-consumption paths surfaced by the web `try_recv`
//! consumer sweep — method call, single char-index, open-range slice, and format
//! interpolation on a nullable text after a null-check. Under `LOFT_PLN25_DN1=1`
//! the value is `Optional(Text)` and each read must peel to its base (the parser
//! index/format peels + the native `&str` borrow peel). The output must match the
//! gate-OFF run of the same script byte-for-byte, on BOTH the interpreter and the
//! `--native` backend.
//!
//! The gate-OFF path is covered by the normal script sweep (the file self-asserts
//! `total == 246`); this binary drives the gate-ON path a subprocess at a time so
//! the `LOFT_PLN25_DN1` `OnceLock` starts fresh for each run.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run_dn1(backend: &str) -> String {
    let script = workspace_root().join("tests/scripts/25-nullable-read-consumption.loft");
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_PLN25_DN1", "1")
        // rustc can hang on the native path; bound it (0 = off is the default).
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "DN1 {backend} run failed (exit {:?}); stdout={stdout:?}; stderr={stderr:?}",
        out.status.code()
    );
    stdout
}

/// The expected output — identical across gate-OFF/DN1 × interpret/native.
const EXPECTED: &str = "frame MAP:hello\nframe MAP:hello\nframe MAP:hello\nnull fmt: null\ntotal=246\n";

#[test]
fn dn1_text_read_consumption_interpret() {
    let stdout = run_dn1("--interpret");
    assert_eq!(
        stdout, EXPECTED,
        "DN1 interpret output must match the gate-OFF byte-identical baseline"
    );
}

#[test]
fn dn1_text_read_consumption_native() {
    let stdout = run_dn1("--native");
    assert_eq!(
        stdout, EXPECTED,
        "DN1 native output must match the gate-OFF byte-identical baseline"
    );
}
