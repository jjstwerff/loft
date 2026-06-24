// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN89 — the differential oracle (formal/operational.md deviations D-op-1/2).
//!
//! The interpreter (`src/state/`) and the native generator (`src/generation/`)
//! are two separate implementations of the same language, kept in agreement only
//! by tests. A program the interpreter runs fine but `--native` miscompiles (or
//! leaks, or halts differently) ships until some test happens to exercise it —
//! a coverage lottery (#433 was the canonical case; this session's overflow-log
//! attempt was another: it broke `--native` with E0499 while the interpreter was
//! perfectly happy).
//!
//! This oracle turns that class into a CAUGHT failure: every program in
//! `tests/oracle/` is run on BOTH backends and their observable outcome must
//! AGREE — normalised stdout (value / null), exit code (halt), and leak-freedom.
//! The operational.md rules guide what the corpus should cover; the corpus
//! GROWS over time, and every fixed divergence graduates a guard program here.
//!
//! The corpus sweep is `#[ignore]` by default: each program shells out to the
//! `loft` binary twice and `--native` invokes `rustc` per program, too heavy for
//! the default `cargo test` path. Run it explicitly:
//!
//! ```bash
//! cargo test --release --test differential_oracle -- --ignored
//! ```
//!
//! The `positive_control_*` test (NOT ignored) proves the comparison can
//! actually fail — a green sweep is only evidence once the detector is known to
//! fire (engineering-rigor: a silent sentinel needs a positive control).

use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Captured outcome of one backend run.
struct ModeRun {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
}

/// Run one corpus program under `mode_flag` (`--interpret` / `--native`).
/// `LOFT_TIMEOUT` bounds the native `rustc` compile so a runaway can't hang the
/// suite (the native path can otherwise block indefinitely).
fn run_mode(mode_flag: &str, path: &Path, env: &[(&str, &str)]) -> ModeRun {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(mode_flag)
        .arg(path)
        .current_dir(workspace_root())
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn loft in {mode_flag} mode: {e}"));
    ModeRun {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit_code: out.status.code(),
    }
}

/// Normalise stdout for cross-backend comparison: CRLF→LF, trailing
/// whitespace per line, and a collapsed trailing-blank-line tail. (Mirrors
/// `tests/common/cross_mode.rs::normalise_stdout` — the native binary may emit
/// CRLF on Windows.)
fn normalise_stdout(s: &str) -> String {
    let lf = s.replace("\r\n", "\n");
    let mut lines: Vec<&str> = lf.lines().map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Both backends report a store leak identically — a `… stores not freed at
/// program exit …` line on stderr. The interpreter prints it unconditionally;
/// the native binary only when `LOFT_NATIVE_LEAK_CHECK` is set (which the sweep
/// sets for the native run).
fn leaked(run: &ModeRun) -> bool {
    run.stderr.contains("stores not freed")
}

/// The oracle's invariant, as a pure function so it can be unit-tested: the
/// interpreter and native runs must AGREE on stdout (value / null) and exit
/// code (halt), and NEITHER may leak. Returns the list of divergences for one
/// program — empty means the two backends agree.
fn divergences(interp: &ModeRun, native: &ModeRun) -> Vec<String> {
    let mut d = Vec::new();
    let i = normalise_stdout(&interp.stdout);
    let n = normalise_stdout(&native.stdout);
    if i != n {
        d.push(format!(
            "stdout differs:\n    interp = {i:?}\n    native = {n:?}"
        ));
    }
    if interp.exit_code != native.exit_code {
        d.push(format!(
            "exit code differs: interp={:?} native={:?}",
            interp.exit_code, native.exit_code
        ));
    }
    if leaked(interp) {
        d.push(format!(
            "interpreter leaked a store:\n    {}",
            interp.stderr.trim()
        ));
    }
    if leaked(native) {
        d.push(format!(
            "native leaked a store:\n    {}",
            native.stderr.trim()
        ));
    }
    d
}

/// The corpus: every `.loft` under `tests/oracle/`, in alphabetical order.
fn corpus() -> Vec<PathBuf> {
    let dir = workspace_root().join("tests/oracle");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "loft"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "oracle corpus is empty: {}",
        dir.display()
    );
    files
}

/// Run the whole corpus on both backends; a divergence on ANY program is a
/// caught cross-backend bug. Heavy (rustc per program) — `#[ignore]` by default.
#[test]
#[ignore = "differential oracle — run with --test differential_oracle -- --ignored"]
fn oracle_corpus_agrees_across_backends() {
    let mut report = Vec::new();
    for path in corpus() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let interp = run_mode("--interpret", &path, &[]);
        let native = run_mode("--native", &path, &[("LOFT_NATIVE_LEAK_CHECK", "1")]);
        let d = divergences(&interp, &native);
        if !d.is_empty() {
            report.push(format!("✗ {name}\n  {}", d.join("\n  ")));
        }
    }
    assert!(
        report.is_empty(),
        "differential oracle caught {} cross-backend divergence(s):\n\n{}",
        report.len(),
        report.join("\n\n"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(stdout: &str, stderr: &str, code: Option<i32>) -> ModeRun {
        ModeRun {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code: code,
        }
    }

    /// Positive control — the detector must FIRE on each divergence kind, and
    /// must NOT fire when the two runs agree. Without this, a green corpus sweep
    /// proves nothing (the detector could be silently dead).
    #[test]
    fn positive_control_divergences_are_detected() {
        let base = run("x=1\n", "", Some(0));

        // agreement → no divergence (no false positive)
        assert!(
            divergences(&base, &run("x=1\n", "", Some(0))).is_empty(),
            "identical runs must agree"
        );

        // stdout divergence (the silent-corruption class)
        assert!(
            !divergences(&base, &run("x=2\n", "", Some(0))).is_empty(),
            "a stdout difference must be caught"
        );

        // exit-code divergence (the halt class — #433: interp ran, native E0308)
        assert!(
            !divergences(&base, &run("x=1\n", "", Some(1))).is_empty(),
            "an exit-code difference must be caught"
        );

        // a native store leak (the @PLN85 ownership class)
        assert!(
            !divergences(
                &base,
                &run(
                    "x=1\n",
                    "Warning: 1 stores not freed at program exit",
                    Some(0)
                )
            )
            .is_empty(),
            "a native store leak must be caught"
        );

        // an interpreter leak too
        assert!(
            !divergences(&run("x=1\n", "Warning: 1 stores not freed", Some(0)), &base).is_empty(),
            "an interpreter store leak must be caught"
        );
    }

    /// stdout normalisation must not mask a real value difference, but must
    /// absorb trailing-newline / CRLF noise (else every program "diverges").
    #[test]
    fn normalisation_absorbs_noise_not_signal() {
        assert_eq!(
            normalise_stdout("a\nb\n"),
            normalise_stdout("a\r\nb\r\n\n\n")
        );
        assert_ne!(normalise_stdout("a=1\n"), normalise_stdout("a=2\n"));
    }
}
