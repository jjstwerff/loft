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
const EXPECTED: &str =
    "frame MAP:hello\nframe MAP:hello\nframe MAP:hello\nnull fmt: null\ntotal=246\n";

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

/// Run a self-asserting script under `LOFT_PLN25_DN1=1` and require a clean exit
/// (exit 0 = every internal `assert` passed). Used for the scripts whose value is
/// their own asserts rather than a stdout signature.
fn dn1_script_exits_zero(backend: &str, rel_path: &str) {
    let script = workspace_root().join(rel_path);
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_PLN25_DN1", "1")
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "DN1 {backend} {rel_path} failed (exit {:?}); stdout={stdout:?}; stderr={stderr:?}",
        out.status.code()
    );
}

/// @PLN25 scalar-nullable-vector-element slice: `vector<τ?>` for a scalar τ must
/// store/read/iterate the typed-sentinel null under DN1, on both backends. These
/// scripts self-assert; the flagship + three-state-boolean were previously blocked
/// on this slice and now go green.
const DN1_SVEC_SCRIPTS: &[&str] = &[
    "tests/scripts/25-nullable-vector-scalar-element.loft",
    "tests/scripts/25-nullable-sequences.loft",
    "tests/scripts/292-pln17-three-state-boolean.loft",
    // @PLN25 `character?` native null-sentinel wrap (Call / Var / Block / TupleGet).
    "tests/scripts/25-nullable-character.loft",
];

#[test]
fn dn1_scalar_vector_element_interpret() {
    for s in DN1_SVEC_SCRIPTS {
        dn1_script_exits_zero("--interpret", s);
    }
}

#[test]
fn dn1_scalar_vector_element_native() {
    for s in DN1_SVEC_SCRIPTS {
        dn1_script_exits_zero("--native", s);
    }
}
