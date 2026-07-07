// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN94 Phase 5 — the CFG+dataflow ownership oracle as a STANDING check, run BESIDE the shipped
//! analysis (coexistence: it never drives codegen; SI-1 holds). This binary makes the H-tier gates
//! permanent so every CI run re-proves them:
//!
//! - **4.2 no crying wolf** — `LOFT_OWN_ORACLE=check` is 0 RED on the verified-correct corpus
//!   (`oracle_clean_on_correct_corpus`).
//! - **4.3 true positive** — the known-wrong `LOFT_NO_A1B` plan is flagged RED, and the correct
//!   default is not (`oracle_flags_the_a1b_wrong_plan`).
//! - **SI-1 observer** — the oracle does not perturb shipped output: `introspect` is byte-identical
//!   with the oracle on vs off (`oracle_is_a_pure_observer_si1`).
//! - **SI-2 backend fact-identity** — the fact reads identically under `--interpret` and `--native`
//!   (`oracle_fact_is_backend_identical_si2`, release-gated: rustc per run).
//!
//! Design + the strictness verification (why it catches the A1b class the runtime gates miss):
//! `doc/claude/plans/94-cfg-ownership-dataflow/PHASE4_DESIGN.md`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run the oracle in `mode` over `file` (with any extra env) and return its stderr lines.
fn run_oracle(file: &str, mode: &str, extra_env: &[(&str, &str)], native: bool) -> String {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(if native { "--native" } else { "--interpret" })
        .arg(root().join(file))
        .env("LOFT_NO_CACHE", "1") // force scopes::check to re-run on the user file
        .env("LOFT_OWN_ORACLE", mode);
    if native {
        cmd.env("LOFT_TIMEOUT", "120"); // rustc can hang — name the phase if it does
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("the loft binary must be runnable");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The deduplicated `RED …` findings the `check` mode reports (it re-runs per compilation unit, so
/// the same finding repeats; the SET is what matters).
fn check_reds(file: &str, extra_env: &[(&str, &str)]) -> BTreeSet<String> {
    run_oracle(file, "check", extra_env, false)
        .lines()
        .filter(|l| l.starts_with("RED "))
        .map(str::to_string)
        .collect()
}

/// The `OWN <fn> …` fact lines (mode `own`), sorted — the per-function shadow-diff summary. Used for
/// the SI-2 backend-identity comparison.
fn own_facts(file: &str, native: bool) -> Vec<String> {
    let mut v: Vec<String> = run_oracle(file, "own", &[], native)
        .lines()
        .filter(|l| l.starts_with("OWN "))
        .map(str::to_string)
        .collect();
    v.sort();
    v
}

/// The verified-correct corpus the check must stay silent on (no crying wolf). Kept to files proven
/// 0-RED (the @PLN94 probes + the collection-capture stress); `n_choose`
/// (`85-struct-copy-return-owned`, the one documented retbuf-materialisation residual) is
/// deliberately EXCLUDED until that increment lands.
const CLEAN_CORPUS: &[&str] = &[
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/00-a1b-silent-blindspot.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/01-cfg-corpus.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/02-loops-rd.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/03-ownership.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/04-precision.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/05-interproc.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/06-capture.loft",
    "tests/scripts/505-collection-capture.loft",
];

const A1B_UAF: &str = "tests/scripts/85-temp-subject-borrow-return-uaf.loft";

/// 4.2 — the check cries no wolf: 0 RED across the verified-correct corpus.
#[test]
fn oracle_clean_on_correct_corpus() {
    for &f in CLEAN_CORPUS {
        let reds = check_reds(f, &[]);
        assert!(
            reds.is_empty(),
            "the ownership oracle flagged correct code (false positive) in {f}:\n{}",
            reds.iter().cloned().collect::<Vec<_>>().join("\n")
        );
    }
}

/// 4.3 — the true-positive gate: the `LOFT_NO_A1B` wrong plan is flagged, the correct default is not.
/// This is the class the runtime gates (exit / leak / interp-vs-native) structurally MISS — see the
/// strictness table in PHASE4_DESIGN.md.
#[test]
fn oracle_flags_the_a1b_wrong_plan() {
    let wrong = check_reds(A1B_UAF, &[("LOFT_NO_A1B", "1")]);
    assert!(
        wrong
            .iter()
            .any(|r| r.contains("n_h") && r.contains("fact-disagree")),
        "the oracle did NOT flag the known-wrong LOFT_NO_A1B plan (n_h):\n{}",
        wrong.iter().cloned().collect::<Vec<_>>().join("\n")
    );
    let correct = check_reds(A1B_UAF, &[]);
    assert!(
        correct.is_empty(),
        "the oracle cried wolf on the CORRECT A1b plan:\n{}",
        correct.iter().cloned().collect::<Vec<_>>().join("\n")
    );
}

/// SI-1 — the oracle is a pure observer: `introspect` stdout (the shipped IR + bytecode) is
/// byte-identical with the oracle ON (`check`) vs OFF. Proof it only observes.
#[test]
fn oracle_is_a_pure_observer_si1() {
    let introspect = |mode: Option<&str>| {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("introspect").arg(root().join(A1B_UAF));
        if let Some(m) = mode {
            cmd.env("LOFT_OWN_ORACLE", m);
        }
        let out = cmd.output().expect("run loft introspect");
        out.stdout // the oracle writes to stderr only; stdout is the shipped artefact
    };
    assert_eq!(
        introspect(None),
        introspect(Some("check")),
        "the oracle changed shipped `introspect` output — SI-1 (observer) violated"
    );
}

/// SI-2 — backend fact-identity: the ownership fact is computed at compile time (`scopes::check`),
/// so it must read identically under `--interpret` and `--native`. Release-gated (rustc per run).
#[test]
#[ignore = "native backend needs rustc per run — the release-gate SI-2 check"]
fn oracle_fact_is_backend_identical_si2() {
    let f = "doc/claude/plans/94-cfg-ownership-dataflow/probes/03-ownership.loft";
    assert_eq!(
        own_facts(f, false),
        own_facts(f, true),
        "the ownership fact differs between --interpret and --native — SI-2 (O-NoDiverge) violated"
    );
}

/// 5.2 — the fuzzer hook: every generated `program_ownership` case (9 shapes × 2 values × 3 churn =
/// 54) runs the oracle check; the whole grammar must be 0 RED (no crying wolf at generative scale,
/// not just the fixed corpus). The generator is the @PLN85 fuzz harness's `grammar_gen.py`.
#[test]
fn oracle_clean_on_generated_fuzz_corpus() {
    let fuzz_dir = root().join("doc/claude/plans/85-store-lifetime-retirement/fuzz");
    let cells = std::env::temp_dir().join(format!("loft_oracle_fuzz_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cells);
    std::fs::create_dir_all(&cells).expect("create cell dir");
    let generated = Command::new("python3")
        .arg(fuzz_dir.join("grammar_gen.py"))
        .arg("--out")
        .arg(&cells)
        .output()
        .expect("python3 must run the grammar generator");
    assert!(
        generated.status.success(),
        "grammar_gen failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let mut offenders = Vec::new();
    let mut count = 0usize;
    for entry in std::fs::read_dir(&cells).expect("read cells") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("loft") {
            continue;
        }
        count += 1;
        // `check_reds` takes a repo-relative path; this cell is absolute, so run the binary directly.
        let out = Command::new(loft_bin())
            .arg("--interpret")
            .arg(&path)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_OWN_ORACLE", "check")
            .output()
            .expect("run loft on a fuzz cell");
        let reds: BTreeSet<String> = String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| l.starts_with("RED "))
            .map(str::to_string)
            .collect();
        if !reds.is_empty() {
            offenders.push(format!(
                "{}:\n{}",
                path.file_name().unwrap().to_string_lossy(),
                reds.iter().cloned().collect::<Vec<_>>().join("\n")
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&cells);
    assert!(count >= 54, "expected ≥54 generated cells, got {count}");
    assert!(
        offenders.is_empty(),
        "the ownership oracle cried wolf on {} generated fuzz cell(s):\n{}",
        offenders.len(),
        offenders.join("\n---\n")
    );
}
