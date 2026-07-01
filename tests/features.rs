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
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("loft")))
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    files
}

#[test]
fn features_examples_interpret() {
    let files = feature_examples();
    assert!(
        !files.is_empty(),
        "no tests/docs/features/*.loft found — run `make features-gen`"
    );
    let mut failures = Vec::new();
    for f in &files {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args(["--interpret", &f.to_string_lossy()])
            .env("LOFT_TIMEOUT", "60")
            .output()
            .expect("spawn loft");
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
    assert!(
        failures.is_empty(),
        "{} feature example(s) failed on --interpret:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}
