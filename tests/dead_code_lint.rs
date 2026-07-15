// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN107 dead-code lint — the ORACLE (regression net for `use_analysis::warn_dead_stores`).
//!
//! Locks the diagnostics emitted for the plan's shape corpus
//! (`doc/claude/plans/107-dead-code-lint/spec.loft`) on **both** backends. The lint is
//! **default ON** (S5): it warns when a non-escaping local OWNS a copy that is mutated via an
//! `OpSet*` but never read (the copy-mutate footgun) — `LOFT_NO_DEAD_STORES` opts out.
//!
//! Two disjoint diagnostics share the substring "is never read", so they are counted apart:
//!   - `test_used` (`unused_variables`, shipped): "Variable X is never read" — three locals in
//!     the corpus (`a`, `total`, `x`). Unaffected by this lint.
//!   - this lint: "'d' is mutated but its value is never read" — exactly one (the W-copy `d`),
//!     an OWNED copy. Aliases (`&`, reference-struct fields, element views) must stay SILENT
//!     (the S4a false-positive fix), verified against real corpus scripts below.
//!
//! Why the binary (not the in-process harness): these are end-to-end compile diagnostics on
//! stderr — same approach as `tests/runtime_warnings.rs`. `LOFT_NO_CACHE` because the program
//! cache skips the re-parse (hence the diagnostics) on a warm run.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn spec_path() -> PathBuf {
    workspace("doc/claude/plans/107-dead-code-lint/spec.loft")
}

const DEAD_STORE_MSG: &str = "is mutated but its value is never read";

/// The three locals `test_used` (unused_variables) flags in the corpus — disjoint from this
/// lint (a var mutated via `OpSet` was read as the write base at parse, so `uses>0`).
const EXPECT_NEVER_READ: [&str; 3] = [
    "Variable a is never read", // W-scalar    — a = 3; a += 1  (self-read doesn't rescue)
    "Variable total is never read", // W-accumulator — total += i, unread after the loop
    "Variable x is never read", // N-effectful — binding unread; the RHS effect would stay
];

/// Run `file` on `backend` (`--interpret` / `--native`) with the given extra env, returning
/// `(stdout, stderr, exit_code)`. `LOFT_NO_CACHE` forces a cold compile so the parse-time
/// diagnostics actually fire; `LOFT_TIMEOUT` bounds the native rustc step.
fn run_env(backend: &str, file: &PathBuf, env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(file)
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Dead-store warnings emitted (this lint).
fn dead_stores(diag: &str) -> usize {
    diag.matches(DEAD_STORE_MSG).count()
}

/// `test_used` never-read warnings — the dead-store message also contains "is never read", so
/// subtract those to isolate `unused_variables`.
fn never_reads(diag: &str) -> usize {
    diag.matches("is never read").count() - dead_stores(diag)
}

// ── Default ON: the corpus warns exactly on the W-copy dead store ────────────────────────────

fn assert_default(backend: &str) {
    let (stdout, diag, code) = run_env(backend, &spec_path(), &[]);

    // Compiles + runs clean (native: the real codegen guard).
    assert_eq!(
        code,
        Some(0),
        "[{backend}] corpus did not exit 0\n{stdout}\n---\n{diag}"
    );
    assert!(
        stdout.contains("N-effect (binding x unread"),
        "[{backend}] corpus did not run to completion\n{stdout}"
    );

    // Exactly one dead store — the W-copy `d` (an owned vector copy). More = a false positive
    // returned; zero = the lint regressed / got silently disabled.
    let ds = dead_stores(&diag);
    assert_eq!(
        ds, 1,
        "[{backend}] want exactly 1 dead-store warning (W-copy d), got {ds}\n{diag}"
    );
    assert!(
        diag.contains(&format!("'d' {DEAD_STORE_MSG}")),
        "[{backend}] the dead store must be reported on `d`\n{diag}"
    );

    // `test_used` untouched — its three never-read warnings still fire, and `d` gets ONLY the
    // dead-store message (no double warning).
    for w in EXPECT_NEVER_READ {
        assert!(
            diag.contains(w),
            "[{backend}] never-read {w:?} must still fire\n{diag}"
        );
    }
    assert_eq!(
        never_reads(&diag),
        3,
        "[{backend}] never-read set drifted\n{diag}"
    );
    assert!(
        !diag.contains("Variable d is never read"),
        "[{backend}] `d` must get ONLY the dead-store warning\n{diag}"
    );
}

#[test]
fn default_on_warns_w_copy_interpret() {
    assert_default("--interpret");
}

#[test]
fn default_on_warns_w_copy_native() {
    assert_default("--native");
}

// ── Opt-out: LOFT_NO_DEAD_STORES silences this lint (but not test_used) ──────────────────────

fn assert_opt_out(backend: &str) {
    let (_out, diag, code) = run_env(backend, &spec_path(), &[("LOFT_NO_DEAD_STORES", "1")]);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] opt-out run did not exit 0\n{diag}"
    );
    assert_eq!(
        dead_stores(&diag),
        0,
        "[{backend}] LOFT_NO_DEAD_STORES must silence the lint\n{diag}"
    );
    // `test_used` is a different lint — the opt-out must not touch it.
    assert_eq!(
        never_reads(&diag),
        3,
        "[{backend}] opt-out must not affect unused_variables\n{diag}"
    );
}

#[test]
fn opt_out_silences_interpret() {
    assert_opt_out("--interpret");
}

#[test]
fn opt_out_silences_native() {
    assert_opt_out("--native");
}

// ── S4a regression: ALIASES stay silent (the write propagates → not a dead store) ────────────

/// Real corpus scripts whose mutated-but-unread local is a BORROW (a `&`-reference, a
/// reference-struct field alias, or a vector-element view). The write reaches the borrowed
/// source, so the lint must NOT fire — this locks the S4a ownership fix against regressions.
#[test]
fn s4a_aliases_stay_silent() {
    let aliases = [
        "tests/scripts/178-ref-alias-copy.loft", // snap = s (s: &Foo)
        "tests/scripts/434-pln87-scalar-reference.loft", // & scalar reference
        "tests/scripts/25-index-elision-borrower.loft", // e = v[i] element view
        "tests/scripts/85-borrow-elision-element-borrower.loft",
    ];
    for rel in aliases {
        let (_o, diag, _c) = run_env("--interpret", &workspace(rel), &[]);
        assert_eq!(
            dead_stores(&diag),
            0,
            "alias script {rel} must emit NO dead-store warning (the write propagates)\n{diag}"
        );
    }
}

// ── S1: the read / write-target classifier (observable via LOFT_DUMP_READS) ──────────────────

/// Parse the `LOFT_DUMP_READS` dump into `(fn, var) -> (reads, write_targets)`.
fn classifier_dump() -> HashMap<(String, String), (u32, u32)> {
    let (_o, stderr, _c) = run_env("--interpret", &spec_path(), &[("LOFT_DUMP_READS", "1")]);
    let mut m = HashMap::new();
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("dead-store-dbg: ") else {
            continue;
        };
        let (mut f, mut v, mut r, mut w) = (None, None, None, None);
        for tok in rest.split_whitespace() {
            if let Some(x) = tok.strip_prefix("fn=") {
                f = Some(x.to_string());
            } else if let Some(x) = tok.strip_prefix("var=") {
                v = Some(x.to_string());
            } else if let Some(x) = tok.strip_prefix("reads=") {
                r = x.parse().ok();
            } else if let Some(x) = tok.strip_prefix("write_targets=") {
                w = x.parse().ok();
            }
        }
        if let (Some(f), Some(v), Some(r), Some(w)) = (f, v, r, w) {
            m.insert((f, v), (r, w));
        }
    }
    m
}

#[test]
fn s1_classifier_isolates_w_copy_dead_store() {
    let m = classifier_dump();
    let cell = |f: &str, v: &str| -> (u32, u32) {
        *m.get(&(f.to_string(), v.to_string())).unwrap_or_else(|| {
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort();
            panic!("no classifier dump for {f}/{v}; keys={keys:?}")
        })
    };

    // THE signal — W-copy `d` is mutated but never read (the copy-fill `d = b.data` OpAppendVector
    // must NOT count as a read).
    assert_eq!(
        cell("n_w_copy", "d"),
        (0, 1),
        "W-copy d must be reads=0 write_targets=1"
    );

    // Non-signals — an N-row either keeps a real read or has no write-target.
    assert!(cell("n_n_read", "d").0 >= 1, "N-read d must have reads>=1");
    assert!(
        cell("n_n_fresh_used", "e").0 >= 1,
        "N-fresh e must have reads>=1"
    );
    assert_eq!(
        cell("n_n_copy_read", "d").1,
        0,
        "N-copy-read d must have write_targets=0"
    );
    // `test_used`-owned scalars present no S2 signal → no double warning.
    assert_eq!(
        cell("n_w_scalar", "a"),
        (1, 0),
        "W-scalar a: self-read only, no OpSet write-target"
    );
    assert_eq!(
        cell("n_n_effectful", "x"),
        (0, 0),
        "N-effectful x: no write-target"
    );

    // S3 escape/branch/loop guards — each mutated copy is also genuinely READ (reads>=1).
    assert!(
        cell("n_n_escape_call", "d").0 >= 1,
        "N-escape-call d: passed to a call"
    );
    assert!(
        cell("n_n_cond_read", "d").0 >= 1,
        "N-cond-read d: read on a branch"
    );
    assert!(
        cell("n_n_loop_read", "buf").0 >= 1,
        "N-loop-read buf: cross-iteration read"
    );

    // Construction guard — `z = Box{…}` fills READ z (reads>0), so despite write_targets>0 the
    // reads==0 signal is absent (no false positive on construction).
    let (zr, zw) = cell("n_n_construct_unread", "z");
    assert!(
        zr >= 1 && zw >= 1,
        "construct-unread z: reads>=1 (fills) + wt>=1: got ({zr},{zw})"
    );
}
