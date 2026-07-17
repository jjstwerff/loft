// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN110 3a — the TEXT strict-index units lint ORACLE (regression net for the
//! DEFAULT-ON `text_index_units_lint_enabled` warning in `fields.rs::parse_text_index`).
//!
//! After the @PLN110 flip `len(text)` is a CHARACTER count while `text[i]` is byte-indexed, so
//! `for i in 0..len(s) { s[i] }` walks char-count byte positions and misreads multi-byte text.
//! Unlike the @PLN102 vector lint (opt-in, mismatched-collection), this one is DEFAULT ON and
//! fires on the SAME text bounded and indexed — for text the units are always mismatched.
//!
//! The corpus labels each function FIRE (must warn by default) or OK (silent). This locks:
//! (1) default ON = exactly the one FIRE site; (2) opt-out `LOFT_NO_STRICT_INDEX_TEXT` = zero;
//! (3) both backends agree (it is a parse-time lint, so `--native` must match `--interpret`).
//!
//! Binary-invoked like `tests/strict_index_lint.rs`; `LOFT_NO_CACHE` so a warm program cache does
//! not skip the re-parse (and thus the diagnostic).

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("doc/claude/plans/110-len-size-semantics/strict-index-text-corpus.loft")
}

/// Message-only substring — avoids the `strict-index` token that also appears on the corpus
/// filename `-->` source-location line (which would double-count every warning).
const LINT_MSG: &str = "is byte-indexed";

fn run(backend: &str, opt_out: bool) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(corpus())
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1");
    if opt_out {
        cmd.env("LOFT_NO_STRICT_INDEX_TEXT", "1");
    } else {
        cmd.env_remove("LOFT_NO_STRICT_INDEX_TEXT");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn lint_count(diag: &str) -> usize {
    diag.matches(LINT_MSG).count()
}

// ── Default ON: exactly the one FIRE site; every OK case silent ──────────────────────────────

fn assert_default_on(backend: &str) {
    let (stdout, diag, code) = run(backend, false);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] corpus did not exit 0\n{stdout}\n---\n{diag}"
    );
    // Exactly one warning — the FIRE case. More = a false positive on an OK case (char iteration,
    // size-bounded byte walk, or a vector index); fewer = the lint regressed.
    let n = lint_count(&diag);
    assert_eq!(
        n, 1,
        "[{backend}] want exactly 1 text strict-index warning (the len(text)-bounded byte index), \
         got {n}\n{diag}"
    );
    assert!(
        diag.contains("strict-index-text-corpus.loft:14:"),
        "[{backend}] the FIRE at line 14 (for i in 0..len(s) {{ s[i] }}) must warn\n{diag}"
    );
    assert!(
        diag.contains("index `i`"),
        "[{backend}] the offending index `i` must be named\n{diag}"
    );
}

#[test]
fn default_on_interpret() {
    assert_default_on("--interpret");
}

#[test]
fn default_on_native() {
    assert_default_on("--native");
}

// ── Opt-out: zero warnings, corpus still runs clean ──────────────────────────────────────────

fn assert_opt_out(backend: &str) {
    let (stdout, diag, code) = run(backend, true);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] corpus did not exit 0 under opt-out\n{stdout}\n---\n{diag}"
    );
    assert_eq!(
        lint_count(&diag),
        0,
        "[{backend}] LOFT_NO_STRICT_INDEX_TEXT must silence the lint\n{diag}"
    );
}

#[test]
fn opt_out_interpret() {
    assert_opt_out("--interpret");
}

#[test]
fn opt_out_native() {
    assert_opt_out("--native");
}
