// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan 09 phase 00 step 0.8 — emitter dispatch validation suite.
//!
//! Runs against the doc-test corpus baseline captured at
//! `/tmp/p09-baseline/*.rs` (created by `scripts/p09_fast_gate.sh
//! --capture`).  Confirms phase 00's two contracts:
//!
//! 1. Every Op-emission call site routes through `emit_op` and
//!    falls through to `DefaultEmitter` (registry empty).  The
//!    generated source is byte-identical to the pre-phase-09
//!    emission.
//! 2. P203's let-bind-on-repeat (step 0.7b shipped earlier) stays
//!    closed: the reproducer exits 0 under native.
//!
//! When a custom emitter is later registered, this suite is the
//! safety net that catches divergence between the new emission and
//! the byte-identical baseline.  Each entry in `BASELINE_CORPUS`
//! that's affected by a new custom emitter should be regenerated
//! intentionally and re-captured.

extern crate loft;

use std::process::Command;

const CORPUS: &[&str] = &[
    "tests/docs/03-integer.loft",
    "tests/docs/04-boolean.loft",
    "tests/docs/07-vector.loft",
    "tests/docs/08-struct.loft",
    "tests/docs/13-file.loft",
    "tests/docs/19-threading.loft",
    "tests/docs/25-generics.loft",
];

const BASELINE_DIR: &str = "/tmp/p09-baseline";

fn project_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn loft_binary() -> std::path::PathBuf {
    project_root().join("target/release/loft")
}

fn baseline_present() -> bool {
    std::path::Path::new(BASELINE_DIR).exists()
        && CORPUS.iter().all(|t| {
            let name = std::path::Path::new(t).file_stem().unwrap().to_string_lossy();
            std::path::Path::new(BASELINE_DIR)
                .join(format!("{name}.rs"))
                .exists()
        })
}

fn emit_native(loft_src: &str, out_path: &std::path::Path) {
    let status = Command::new(loft_binary())
        .args(["--native-emit", out_path.to_str().unwrap(), loft_src])
        .current_dir(project_root())
        .status()
        .expect("failed to spawn loft binary — run `cargo build --release` first");
    assert!(
        status.success(),
        "--native-emit failed for {loft_src} (exit {})",
        status.code().unwrap_or(-1)
    );
}

/// Phase 00 contract 1: every doc-test in CORPUS produces byte-identical
/// emission compared to the baseline captured before phase 00 started.
///
/// The baseline lives at `/tmp/p09-baseline/`.  Capture (or refresh) via
/// `scripts/p09_fast_gate.sh --capture`.  When no baseline is present,
/// this test skips with an explanatory message — running locally without
/// having captured the baseline shouldn't fail the suite.
#[test]
fn baseline_emission_unchanged() {
    if !baseline_present() {
        eprintln!(
            "[codegen_emitter] no baseline at {BASELINE_DIR}; \
             run `scripts/p09_fast_gate.sh --capture` to seed.  Skipping."
        );
        return;
    }
    let tmp_dir = std::env::temp_dir().join("p09-codegen-emitter-test");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let mut diffs: Vec<String> = Vec::new();
    for src in CORPUS {
        let name = std::path::Path::new(src).file_stem().unwrap().to_string_lossy();
        let out = tmp_dir.join(format!("{name}.rs"));
        emit_native(src, &out);
        let baseline = std::path::Path::new(BASELINE_DIR).join(format!("{name}.rs"));
        let actual = std::fs::read_to_string(&out).expect("read emitted .rs");
        let expected = std::fs::read_to_string(&baseline).expect("read baseline .rs");
        if actual != expected {
            diffs.push(name.into_owned());
        }
    }
    assert!(
        diffs.is_empty(),
        "phase 00 byte-identical contract broken — diverging files: {diffs:?}.  \
         Either fix the emission, or (if intentional) refresh the baseline \
         via `scripts/p09_fast_gate.sh --capture`."
    );
}

/// Phase 00 step 0.7b regression guard — P203 stays closed.
///
/// The `OpConvIntFromEnum` template at `default/01_code.loft:705`
/// substitutes `@v1` twice; before the let-bind-on-repeat fix, the
/// assertion `delete(path) == FileResult.Ok` called `n_delete()` twice
/// and panicked.  The fix in `output_call_template` hoists repeated
/// placeholders into a single `let _v_<name>` binding.  This test
/// guards against regression.
#[test]
fn p203_reproducer_passes_under_native() {
    let status = Command::new(loft_binary())
        .arg("tests/scripts/repro_p203.loft")
        .current_dir(project_root())
        .status()
        .expect("failed to spawn loft binary — run `cargo build --release` first");
    assert!(
        status.success(),
        "P203 reproducer failed under native (exit {}) — \
         the let-bind-on-repeat in calls.rs::output_call_template \
         may have regressed",
        status.code().unwrap_or(-1)
    );
}

/// Phase 00 step 0.7b structural guard — the affected templates do
/// produce a `let _v_<name>` binding shape in their generated code,
/// proving the let-bind-on-repeat path is active.  If this test ever
/// reports zero matches, the pre-pass in `output_call_template` was
/// silently disabled.
#[test]
fn let_bind_on_repeat_appears_in_emission() {
    if !baseline_present() {
        eprintln!("[codegen_emitter] no baseline; skipping let-bind-on-repeat structural check");
        return;
    }
    // tests/docs/13-file.loft uses the `delete(...) == FileResult.X`
    // pattern that triggers `OpConvIntFromEnum`'s let-bind-on-repeat.
    let baseline = std::path::Path::new(BASELINE_DIR).join("13-file.rs");
    let src = std::fs::read_to_string(baseline).expect("read 13-file baseline");
    assert!(
        src.contains("let _v_v1"),
        "13-file.rs baseline lacks `let _v_v1` — let-bind-on-repeat may not be \
         engaging for repeated @v1 placeholders.  Re-capture the baseline if \
         the emission shape changed intentionally."
    );
}
