// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN107 dead-code lint — **S0 ORACLE** (the regression net every later step checks).
//!
//! Locks the baseline set of dead-code diagnostics emitted for the plan's shape corpus
//! (`doc/claude/plans/107-dead-code-lint/spec.loft`) on **both** backends, so each
//! subsequent step (S1 read-count → S2 warning → …) has something to move against and the
//! W-copy gap is a single, visible flip.
//!
//! Baseline TODAY: loft's shipped `unused_variables` (`Function::test_used`,
//! `src/variables/mod.rs:1666`) flags exactly three locals — `a` (W-scalar), `total`
//! (W-accumulator), `x` (N-effectful binding). The motivating **W-copy** dead store
//! (`d = b.data; d[0] = 9`, `d` never read) is **SILENT** — its only "use" is a
//! write-target base, which bumps `uses`, so `test_used` thinks `d` is used. Closing that
//! is S2; when it lands, update `EXPECT_NEVER_READ` / the W-copy assertion here (the count
//! becomes 4). See `doc/claude/plans/107-dead-code-lint/README.md`.
//!
//! Why the binary (not the in-process harness): these are end-to-end compile diagnostics on
//! stderr — same approach as `tests/runtime_warnings.rs`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn spec_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("doc/claude/plans/107-dead-code-lint/spec.loft")
}

/// Compile+run the corpus on `backend` (`--interpret` / `--native`) and return
/// `(stdout, stderr, exit_code)`. Diagnostics land on stderr; stdout + a zero exit prove the
/// corpus ran to completion (so the native test genuinely guards codegen, not just the
/// pre-codegen parse warnings). `LOFT_TIMEOUT` bounds the native rustc step.
fn run(backend: &str) -> (String, String, Option<i32>) {
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(spec_path())
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// The user locals `test_used` flags as never-read TODAY. S2 will additionally flag the
/// W-copy `d` store — a DIFFERENT message ("is mutated but its value is never read"),
/// asserted absent below until then.
const EXPECT_NEVER_READ: [&str; 3] = [
    "Variable a is never read", // W-scalar    — a = 3; a += 1  (self-read doesn't rescue)
    "Variable total is never read", // W-accumulator — total += i, unread after the loop
    "Variable x is never read", // N-effectful — binding unread; the RHS effect would stay
];

fn assert_baseline(backend: &str) {
    let (stdout, diag, code) = run(backend);

    // The corpus must COMPILE + RUN clean on this backend (not just parse). For native this
    // is the real "codegen green" guard — the warnings alone are emitted pre-codegen.
    assert_eq!(
        code,
        Some(0),
        "[{backend}] corpus did not exit 0\n--- stdout ---\n{stdout}\n--- stderr ---\n{diag}"
    );
    assert!(
        stdout.contains("N-effect (binding x unread"),
        "[{backend}] corpus did not run to completion (last line missing)\n--- stdout ---\n{stdout}"
    );

    // Present today (proves stderr capture works — a broken harness fails here).
    for w in EXPECT_NEVER_READ {
        assert!(
            diag.contains(w),
            "[{backend}] expected never-read warning {w:?}\n--- stderr ---\n{diag}"
        );
    }

    // Exactly three never-read warnings — locks the set against drift in EITHER direction.
    // S2 (W-copy) does NOT add a "never read" line (it uses the dead-store message), so this
    // count stays 3 through S1/S2; a new never-read warning anywhere is a real regression.
    let n = diag.matches("is never read").count();
    assert_eq!(
        n, 3,
        "[{backend}] never-read warning set drifted (got {n}, want 3)\n--- stderr ---\n{diag}"
    );

    // THE GAP — the W-copy dead store is SILENT today. This is the single assertion S2 flips
    // (from `!contains` to `contains`, and `d` gains a warning).
    assert!(
        !diag.contains("is mutated but its value is never read"),
        "[{backend}] W-copy dead-store warning appeared before S2 landed\n--- stderr ---\n{diag}"
    );

    // No whole-var overwrite-before-read in the corpus → track_write stays quiet. Guards that
    // the existing `unused_assignments` lint doesn't start mis-firing on these shapes.
    assert!(
        !diag.contains("Dead assignment"),
        "[{backend}] unexpected dead-assignment warning\n--- stderr ---\n{diag}"
    );
}

#[test]
fn s0_baseline_interpret() {
    assert_baseline("--interpret");
}

/// The never-read / dead-store lints run in `scopes::check`, BEFORE backend selection, so
/// native MUST emit the identical set. This also proves the corpus compiles on native —
/// S1's "codegen byte-identical" guarantee depends on native staying green.
#[test]
fn s0_baseline_native() {
    assert_baseline("--native");
}

// ── S1: the read / write-target classifier (observable; no warning yet) ─────────────────
//
// `LOFT_DUMP_READS` prints `dead-store-dbg: fn=F var=V uses=U reads=R write_targets=W` per
// user local. S1 adds the classifier that separates a value-observing READ from an `OpSet*`
// WRITE-TARGET base — decoupled from `uses` (codegen) — so the W-copy dead store becomes
// visible as `reads=0, write_targets>0`. This locks those numbers so S2 (which turns the
// signal into a warning) has a regression net for the classifier itself.

/// Parse the `LOFT_DUMP_READS` dump into `(fn, var) -> (reads, write_targets)`.
fn classifier_dump() -> HashMap<(String, String), (u32, u32)> {
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(spec_path())
        .env("LOFT_DUMP_READS", "1")
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft binary");
    let stderr = String::from_utf8_lossy(&out.stderr);
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
    // user fns are stored as `n_<name>`.
    let cell = |f: &str, v: &str| -> (u32, u32) {
        *m.get(&(f.to_string(), v.to_string())).unwrap_or_else(|| {
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort();
            panic!("no classifier dump for {f}/{v}; keys={keys:?}")
        })
    };

    // THE signal — W-copy `d` is mutated (`d[0]=9`) but its value is never read. The copy-fill
    // `d = b.data` (an OpAppendVector) must NOT count as a read here (reads stays 0).
    assert_eq!(
        cell("n_w_copy", "d"),
        (0, 1),
        "W-copy d must be reads=0 write_targets=1 (the S2 dead-store signal)"
    );

    // Non-signals — every N-row either keeps a real read or has no write-target.
    assert!(
        cell("n_n_read", "d").0 >= 1,
        "N-read d must have reads>=1 (d[0] is read back)"
    );
    assert!(
        cell("n_n_fresh_used", "e").0 >= 1,
        "N-fresh e must have reads>=1 (passed to a call)"
    );
    assert_eq!(
        cell("n_n_copy_read", "d").1,
        0,
        "N-copy-read d must have write_targets=0 (no OpSet)"
    );

    // `test_used`-owned scalars must NOT present the S2 signal → no double warning at S2.
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
}
