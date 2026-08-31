// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @I81 / @PLN92 strand 3 — the interpret half of the "example-must-run" guard.
//!
//! Every `tests/docs/features/*.loft` is a `## Example` extracted from a
//! `loft-lang/features` issue by `tools/features/gen.loft` (only complete-program
//! examples land here — library / syntax fragments are mirrored but not tested).
//! This runs each on the interpreter and asserts a clean exit; if an authored
//! example stops running, CI goes red.  The native half is
//! `tests/native.rs::native_features`; the no-drift half is `make features-check`.

use std::path::PathBuf;
use std::process::Command;

/// Collect the generated feature examples (sorted for a stable failure order).
fn feature_examples() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir("tests/docs/features") {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    files
}

// @speed 3.5
#[test]
fn features_examples_interpret() {
    let files = feature_examples();
    assert!(
        !files.is_empty(),
        "no tests/docs/features/*.loft found — run `make features-gen`"
    );
    let mut failures = Vec::new();
    // loft#1238 — time every example, and report the slowest few WITH the failure.
    //
    // This test hard-kills an example at `LOFT_TIMEOUT`, and when it fired the report named
    // the example and the phase and nothing else — so a run that took 71s could not be told
    // apart from one where a single example stalled while the rest were instant. Both readings
    // were live, and choosing between them needed a reproduction nobody had: the example that
    // tripped it takes 0.1s on its own, and twelve concurrent copies finish in 0.16s.
    //
    // The timing is collected unconditionally and printed only ON FAILURE, so a green run stays
    // silent. It is not a threshold and it does not gate: it turns the next occurrence into
    // evidence about WHICH of the two shapes this is, which is what the issue is missing.
    let mut timings: Vec<(std::time::Duration, PathBuf)> = Vec::new();
    for f in &files {
        let started = std::time::Instant::now();
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args(["--interpret", &f.to_string_lossy()])
            .env("LOFT_TIMEOUT", "60")
            .output()
            .expect("spawn loft");
        timings.push((started.elapsed(), f.clone()));
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() || combined.contains("panicked") {
            let tail: Vec<&str> = combined.lines().rev().take(6).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            failures.push(format!("{}:\n  {}", f.display(), tail.join("\n  ")));
        }
    }
    let slowest = if failures.is_empty() {
        String::new()
    } else {
        timings.sort_by_key(|(d, _)| std::cmp::Reverse(*d));
        let total: f64 = timings.iter().map(|(d, _)| d.as_secs_f64()).sum();
        let rows: Vec<String> = timings
            .iter()
            .take(5)
            .map(|(d, p)| {
                format!(
                    "  {:>7.2}s  {}",
                    d.as_secs_f64(),
                    p.file_name().unwrap_or(p.as_os_str()).to_string_lossy()
                )
            })
            .collect();
        format!(
            "\n\n{} examples took {total:.1}s in total; the slowest were:\n{}\n\
             (one example far above the rest is a stall in THAT example; every example slow \
             is the box being saturated — loft#1238)",
            timings.len(),
            rows.join("\n")
        )
    };
    assert!(
        failures.is_empty(),
        "{} feature example(s) failed on --interpret:\n{}{slowest}",
        failures.len(),
        failures.join("\n---\n")
    );
}
