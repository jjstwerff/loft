// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 01 — `loft introspect` CLI regression tests.
//!
//! Invokes the compiled binary via `std::process::Command` and asserts the
//! acceptance shapes from `plans/12-repl-and-introspection/01-introspection-cli.md`:
//! all-sections default, single-section selection, the default-stdlib filter
//! (the bug where the bytecode section leaked the whole stdlib), `--all-fns`,
//! and the `--fn` filter.  Assertion-based rather than byte-exact golden, so a
//! harmless codegen/format shift doesn't force a golden re-bless.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture() -> std::path::PathBuf {
    workspace_root().join("tests/data/introspect_golden.loft")
}

/// Run `loft introspect <args> <fixture>` and return (stdout, success).
fn introspect(args: &[&str]) -> (String, bool) {
    let out = Command::new(loft_bin())
        .arg("introspect")
        .args(args)
        .arg(fixture())
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

/// The text of one `=== <name> ===` section, up to the next `=== ` header.
fn section<'a>(stdout: &'a str, name: &str) -> &'a str {
    let header = format!("=== {name} ===");
    let start = stdout
        .find(&header)
        .unwrap_or_else(|| panic!("no `{header}` in:\n{stdout}"));
    let after = start + header.len();
    let rest = &stdout[after..];
    let end = rest
        .find("\n=== ")
        .map(|e| after + e)
        .unwrap_or(stdout.len());
    &stdout[after..end]
}

/// Acceptance #1 — default emits all four sections, in order, with the user fns.
#[test]
fn default_emits_all_four_sections() {
    let (out, ok) = introspect(&[]);
    assert!(ok, "introspect should exit 0; stdout:\n{out}");
    let (mut last, mut order) = (0usize, Vec::new());
    for sec in [
        "=== bytecode ===",
        "=== rust ===",
        "=== slots ===",
        "=== types ===",
    ] {
        let at = out
            .find(sec)
            .unwrap_or_else(|| panic!("missing {sec} in:\n{out}"));
        order.push(sec);
        assert!(
            at >= last,
            "sections out of order: {sec} before a prior one\n{out}"
        );
        last = at;
    }
    assert!(out.contains("n_dbl"), "user fn n_dbl missing\n{out}");
    assert!(out.contains("n_test"), "user fn n_test missing\n{out}");
}

/// Regression for the default-stdlib leak: the bytecode + slots + types
/// sections must show ONLY user functions, not the stdlib or the
/// compiler-synthesized runtime helpers (`i_parse_*`, `__iface_*`).
#[test]
fn default_filters_stdlib_and_internals() {
    let (out, ok) = introspect(&[]);
    assert!(ok);
    for name in ["bytecode", "slots", "types"] {
        let sec = section(&out, name);
        assert!(
            !sec.contains("i_parse_errors") && !sec.contains("__iface_"),
            "the `{name}` section leaked stdlib/internal fns (default filter broke):\n{sec}"
        );
        assert!(
            sec.contains("n_dbl"),
            "`{name}` section missing user fn n_dbl:\n{sec}"
        );
    }
}

/// Acceptance #2 — `--show-bytecode` emits ONLY the bytecode section.
#[test]
fn single_section_excludes_others() {
    let (out, ok) = introspect(&["--show-bytecode"]);
    assert!(ok);
    assert!(out.contains("n_dbl"), "bytecode of n_dbl missing\n{out}");
    for other in ["=== rust ===", "=== slots ===", "=== types ==="] {
        assert!(
            !out.contains(other),
            "single-section output leaked {other}\n{out}"
        );
    }
}

/// Acceptance #5 — `--all-fns` pulls the stdlib back into the bytecode section.
#[test]
fn all_fns_includes_stdlib() {
    let (out, ok) = introspect(&["--all-fns", "--show-bytecode"]);
    assert!(ok);
    assert!(
        out.contains("__iface_") || out.contains("i_parse"),
        "--all-fns should include stdlib/internal fns\n{}",
        &out[..out.len().min(2000)]
    );
}

/// Acceptance #4 — `--fn n_dbl` restricts a section to the named function.
#[test]
fn fn_filter_restricts_to_one() {
    let (out, ok) = introspect(&["--fn", "n_dbl", "--show-slots"]);
    assert!(ok);
    assert!(out.contains("n_dbl"), "filtered fn n_dbl missing\n{out}");
    assert!(
        !out.contains("n_test"),
        "--fn n_dbl should exclude n_test\n{out}"
    );
}

// ---------------------------------------------------------------------------
// @PLN103 — the `--show-ownership` overlay + `LOFT_STORES=timeline` summary.
// Assertion-based (not byte-golden): each check pins one SEMANTIC invariant per
// fact-kind, so a harmless var-number/format shift doesn't force a re-bless, but a
// regression in the ownership verdict (e.g. reverting the per-binding classification)
// turns a `Borrowed`/`Join` back into `Owned` and fails.
// ---------------------------------------------------------------------------

/// Run `introspect --show-ownership <flags> tests/data/ownership_corpus.loft`.
fn ownership(flags: &[&str]) -> String {
    let corpus = workspace_root().join("tests/data/ownership_corpus.loft");
    let out = Command::new(loft_bin())
        .arg("introspect")
        .arg("--show-ownership")
        .args(flags)
        .arg(&corpus)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    assert!(
        out.status.success(),
        "introspect --show-ownership should exit 0; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The overlay renders every invisible-fact-kind correctly on the committed IR.
#[test]
fn show_ownership_renders_each_fact_kind() {
    let out = ownership(&[]);
    // The verdict is backend-shared (P2.0) — the overlay says so.
    assert!(
        out.contains("backend-shared"),
        "missing the backend-shared note:\n{out}"
    );
    // K1 — a borrowed field projection is a genuine alias of its source param.
    assert!(
        out.contains("_mv_items_1") && out.contains("Borrowed(base=e)"),
        "K1: borrowed field projection not flagged as Borrowed(base=e):\n{out}"
    );
    // K2 — a whole-value bind COPIES (Owned); a projection read is a VIEW (Borrowed).
    let k2 = section_fn(&out, "n_k2_copy_vs_view");
    assert!(
        k2.contains("\n  1         b                      Owned"),
        "K2: `b = src` should be an Owned copy:\n{k2}"
    );
    assert!(
        k2.contains("first") && k2.contains("Borrowed(base="),
        "K2: `b[0]` should be a Borrowed view:\n{k2}"
    );
    // K4 — the empty `[]` arm is a REAL owned vector (the #562 fix), not a bare null;
    // and the return delivers via the return buffer.
    let k4 = section_fn(&out, "n_k4_emptyarm");
    assert!(
        k4.contains("_empty_arm_1") && k4.contains("Owned (backing="),
        "K4: empty-arm should own a fresh store:\n{k4}"
    );
    assert!(
        k4.contains("delivery:") && k4.contains("materialised"),
        "K4: return delivery should be materialised:\n{k4}"
    );
    // K1 runtime JOIN — owned on one path, a borrowed view on the other; scalars elided.
    let kj = section_fn(&out, "n_k1_owned_or_borrow");
    assert!(
        kj.contains("Join(base=pool)"),
        "K1-join: `r`/return should be Join(base=pool):\n{kj}"
    );
    assert!(
        kj.contains("(scalar)"),
        "scalars should render `— (scalar)`:\n{kj}"
    );
}

/// The overlay is deterministic (two runs identical) — safe to diff/golden.
#[test]
fn show_ownership_is_deterministic() {
    assert_eq!(ownership(&[]), ownership(&[]));
}

/// `LOFT_STORES=timeline` distinguishes a large WORKING SET from a leak — the
/// disambiguation `=warn` cannot make. A clean program: peak > 0, `NO leak`.
#[test]
fn timeline_summary_reports_working_set_no_leak() {
    let corpus = workspace_root().join("tests/data/ownership_corpus.loft");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&corpus)
        .env("LOFT_STORES", "timeline")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("[timeline] SUMMARY:") && err.contains("(working set)"),
        "missing timeline SUMMARY:\n{err}"
    );
    assert!(
        err.contains("NO leak"),
        "a clean program should report NO leak:\n{err}"
    );
    // Stable ids: at least one `alloc #<nr>.<seq>` with a dotted seq.
    assert!(
        err.contains("[timeline] alloc #") && err.contains('.'),
        "missing stable per-store ids:\n{err}"
    );
}

/// The text of one function's `--show-ownership` block, up to the next `fn ` header.
fn section_fn<'a>(stdout: &'a str, fn_name: &str) -> &'a str {
    let header = format!("fn {fn_name} ");
    let start = stdout
        .find(&header)
        .unwrap_or_else(|| panic!("no `{header}` in:\n{stdout}"));
    let rest = &stdout[start + header.len()..];
    let end = rest
        .find("\nfn ")
        .map_or(stdout.len(), |e| start + header.len() + e);
    &stdout[start..end]
}

/// Regression gate for BOTH captured-group UAF fixes (`plans/captured-group-elem-uaf.md`):
/// the 35m materialisation fix (vector-match text arm byte-copies into an owned buffer)
/// and the 35c fix (`collect_return_sources` skips trailing frees, so a returned enum
/// record is freed with `OpFreeRefIfDistinct`, not a plain `OpFreeRef`). Neither
/// `--show-ownership` UAF overlay may fire on the fixture (`bad`, `good`, `parse`); if
/// either fix regresses, its overlay fires and this test catches it. Each overlay's own
/// firing is proved parser-free in `use_analysis::{uaf_overlay_tests, return_source_tests}`,
/// so this gate does not need to reproduce the bugs.
#[test]
fn ownership_overlay_silent_after_captured_group_fix() {
    let out = Command::new(loft_bin())
        .arg("introspect")
        .arg("--show-ownership")
        .arg(workspace_root().join("tests/data/uaf_overlay.loft"))
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "introspect failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let count = stdout.matches("⚠ UAF").count();
    assert_eq!(
        count, 0,
        "materialisation fix regressed — free-before-dependent-read overlay fired:\n{stdout}"
    );
}

/// CLI error — a missing input file exits non-zero.
#[test]
fn missing_file_errors() {
    let out = Command::new(loft_bin())
        .arg("introspect")
        .arg("/no/such/introspect_input.loft")
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft");
    assert!(!out.status.success(), "missing file should exit non-zero");
}
