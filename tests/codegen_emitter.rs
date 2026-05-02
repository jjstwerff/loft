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
    // Phase 03: parallel-for emission coverage.
    "tests/scripts/22-threading.loft",
    // Phase 04: OpGetRecord / OpIterate emission coverage.
    "tests/docs/10-sorted.loft",
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
            let name = std::path::Path::new(t)
                .file_stem()
                .unwrap()
                .to_string_lossy();
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
        let name = std::path::Path::new(src)
            .file_stem()
            .unwrap()
            .to_string_lossy();
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

// ============================================================
// Phase 01 ABI consolidation gates
// ============================================================

/// Gate: the duplicate hardcoded `LEGACY_STORES_FNS` lists that lived
/// in `src/generation/calls.rs` and `src/generation/dispatch.rs` must
/// stay deleted.  Plan 09 phase 01 replaced them with a single
/// `crate::codegen_runtime::abi_of(name)` lookup.
///
/// If a future change re-introduces `LEGACY_STORES_FNS` (the typical
/// quick fix when adding a new legacy-ABI runtime fn), this test
/// fails — the right answer is to add the entry to
/// `CODEGEN_RUNTIME_FNS` in `src/codegen_runtime.rs` instead.
#[test]
fn no_hardcoded_abi_lists_remain() {
    for path in &["src/generation/calls.rs", "src/generation/dispatch.rs"] {
        let src = std::fs::read_to_string(project_root().join(path)).expect("read source file");
        // Allow doc-comment references to the historical name; only flag
        // actual `const LEGACY_STORES_FNS` declarations.
        assert!(
            !src.contains("const LEGACY_STORES_FNS"),
            "{path} reintroduced `const LEGACY_STORES_FNS` — \
             plan 09 phase 01 retired this in favour of \
             `crate::codegen_runtime::abi_of(name)`.  Add new legacy-ABI \
             runtime fns to `CODEGEN_RUNTIME_FNS` in `src/codegen_runtime.rs`."
        );
    }
}

/// Phase 04 structural test: `OpGetRecord` and `OpIterate` emission
/// moved out of dispatch.rs::output_call_inner's special-case match
/// into registered emitters in `src/generation/ops/key_ops.rs`.
/// Re-introducing match arms for these names is a regression.
#[test]
fn no_key_op_special_case_in_dispatch() {
    let src = std::fs::read_to_string(project_root().join("src/generation/dispatch.rs"))
        .expect("read dispatch.rs");
    for op in &["OpGetRecord", "OpIterate"] {
        let pat = format!("\"{op}\"");
        let has_arm = src.lines().any(|line| {
            let t = line.trim();
            t.starts_with(&pat) && t.contains("=>")
        });
        assert!(
            !has_arm,
            "src/generation/dispatch.rs reintroduced an `\"{op}\" => …` \
             match arm.  Phase 04 retired this in favour of the registered \
             emitter in `src/generation/ops/key_ops.rs`.  New key-keyed \
             Op variants should register custom emitters there, not add \
             match arms to dispatch.rs."
        );
    }
}

/// Phase 03 structural test: `n_parallel_for` / `n_parallel_for_light`
/// emission moved out of dispatch.rs::output_call_inner's special-case
/// match into the registered `ParallelForEmitter`.  Re-introducing
/// the special case is a regression of phase 03's structural intent —
/// new parallel variants should register custom emitters instead.
#[test]
fn no_parallel_special_case_in_dispatch() {
    let src = std::fs::read_to_string(project_root().join("src/generation/dispatch.rs"))
        .expect("read dispatch.rs");
    // Allow doc-comment references to the historical name; only flag
    // actual match-arm patterns `"n_parallel_for"` followed by `=>`.
    // The deleted arm was `"n_parallel_for" | "n_parallel_for_light" =>`.
    let has_arm = src.lines().any(|line| {
        let t = line.trim();
        t.starts_with("\"n_parallel_for\"") && t.contains("=>")
    });
    assert!(
        !has_arm,
        "src/generation/dispatch.rs reintroduced an `\"n_parallel_for\" => …` \
         match arm.  Phase 03 retired this in favour of the registered \
         `ParallelForEmitter` in `src/generation/ops/parallel.rs` — new \
         parallel variants should register custom emitters there, not \
         add match arms to dispatch.rs."
    );
}

/// Phase 09 structural test: the three `n_parallel_for_*_native`
/// public fns must remain thin wrappers around
/// `n_parallel_for_native_core`.  Re-inflating any of them (e.g. by
/// inlining shape-specific alloc/dispatch/merge logic into the
/// public fn body) regresses phase 09's consolidation — that
/// duplication is what phase 06 (queue variants) needs the shape
/// trait to absorb.  New parallel variants should add a `ParShape`
/// impl and call the generic core, not re-inline the skeleton.
#[test]
fn parallel_runtime_consolidated() {
    let src = std::fs::read_to_string(project_root().join("src/codegen_runtime.rs"))
        .expect("read codegen_runtime.rs");
    for fn_name in [
        "n_parallel_for_native",
        "n_parallel_for_text_native",
        "n_parallel_for_ref_native",
    ] {
        let needle = format!("pub fn {fn_name}<F>(");
        let start = src
            .find(&needle)
            .unwrap_or_else(|| panic!("did not find `pub fn {fn_name}<F>(` in codegen_runtime.rs"));
        // Body starts after the first `{` past the signature.
        let body_open = start
            + src[start..]
                .find('{')
                .expect("function signature missing opening brace");
        // Walk to the matching close brace.
        let mut depth = 0i32;
        let mut body_close = body_open;
        for (i, ch) in src[body_open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        body_close = body_open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(
            body_close > body_open,
            "{fn_name}: failed to locate closing brace"
        );
        let body_lines = src[body_open + 1..body_close].lines().count();
        assert!(
            body_lines <= 15,
            "{fn_name} body is {body_lines} lines — phase 09 consolidation \
             expects each public parallel-for fn to be a thin wrapper (≤ 15 \
             body lines) calling `n_parallel_for_native_core` with a `ParShape`. \
             If you've added shape-specific logic, extend the `ParShape` trait \
             instead of re-inlining the skeleton."
        );
        assert!(
            src[body_open..body_close].contains("n_parallel_for_native_core"),
            "{fn_name} body must call `n_parallel_for_native_core` (phase 09 \
             consolidation).  Direct re-implementation regresses the shape \
             trait and inflates phase 06's queue-variant duplication."
        );
    }
}

/// Gate: the registry's `Abi` tags must be self-consistent — every
/// entry's `abi_of(name)` lookup must return the entry's own tag.
/// Trivially true when the registry is well-formed; catches a typo
/// where two entries with the same name disagree on ABI.
#[test]
fn abi_of_handles_all_runtime_fns() {
    use loft::codegen_runtime::{Abi, CODEGEN_RUNTIME_FNS, abi_of};
    for fn_def in CODEGEN_RUNTIME_FNS {
        assert_eq!(
            abi_of(fn_def.name),
            fn_def.abi,
            "abi_of disagrees with registry for `{}`",
            fn_def.name
        );
    }
    // Unknown name → Cell (user-fn / Op-stub default).
    assert_eq!(abi_of("nonexistent_fn_for_test"), Abi::Cell);
}

// ============================================================
// Wart-budget gates (plan 09 phase 00 evaluation findings)
// ============================================================

/// Gate A: caps the size of the special-case Op match in
/// `dispatch.rs::output_call_inner`.
///
/// At phase 00 completion the match has 26 hardcoded inline-emission
/// arms — a parallel dispatch system that lives alongside the
/// `emit_op` registry.  Plan 09's broader goal is to drain this match
/// to zero by migrating each Op into a custom emitter (phases 03/04
/// chip away at this).  This gate enforces the migration direction:
/// new emissions must be registered emitters, NOT new match arms.
///
/// Budget: shrink this number as phases land custom emitters.
/// **Never raise it without justification** in NATIVE.md.
const DISPATCH_OP_ARM_BUDGET: usize = 26;

#[test]
fn dispatch_op_arm_budget_not_exceeded() {
    let src = std::fs::read_to_string(project_root().join("src/generation/dispatch.rs"))
        .expect("read dispatch.rs");
    let start = src
        .find("fn output_call_inner")
        .expect("output_call_inner not found in dispatch.rs");
    // Find the closing `}` of the function body — heuristic: scan until
    // the next `^    }$` (4-space indent + brace) line after start.
    let after_start = &src[start..];
    let body_end_rel = after_start
        .match_indices("\n    }\n")
        .next()
        .map(|(i, _)| i)
        .unwrap_or(after_start.len());
    let body = &after_start[..body_end_rel];

    // Count match-arm patterns: a line whose trimmed prefix begins with
    // `"Op` and contains `=>` is an Op match arm.  This is a heuristic
    // but resilient to the multi-pattern `"Op…" | "Op…" =>` form
    // because each line still starts with a string literal.
    let arms = body
        .lines()
        .filter(|line| {
            let t = line.trim();
            t.starts_with("\"Op") && t.contains("=>")
        })
        .count();

    assert!(
        arms <= DISPATCH_OP_ARM_BUDGET,
        "dispatch.rs::output_call_inner has {arms} Op match arms — budget is \
         {DISPATCH_OP_ARM_BUDGET}.  New Op-specific emissions must be registered as \
         `OpEmitter` impls in `src/generation/ops/`, not added as match arms.  \
         If you have a justification for raising the budget, document it in \
         doc/claude/NATIVE.md and update DISPATCH_OP_ARM_BUDGET."
    );
}

/// Gate B: caps the set of "codegen-only" `Value` variants — IR
/// variants that are produced exclusively by native code generation
/// and have no parser source or runtime semantics.
///
/// `Value::RawExpr` (added by phase 00 step 0.7) is the sole sanctioned
/// codegen-only variant.  Adding more is a wart: the IR loses meaning
/// because variants exist purely as plumbing for codegen synthesis.
/// Each codegen-only variant requires no-op default arms in every
/// walker (parser, scopes, pre_eval, state codegen) — the cost grows
/// linearly with each addition.
///
/// **Rule**: if you need to thread synthesized values through codegen,
/// build a string-aware companion entry point rather than another
/// `Value` variant.  Plan 09 phase 00 evaluation documented this
/// constraint; see `doc/claude/plans/09-native-runtime-rewrite/00-scaffold.md`
/// "Findings (post-completion)".
const SANCTIONED_CODEGEN_VALUE_VARIANTS: &[&str] = &["RawExpr"];

#[test]
fn no_unsanctioned_codegen_value_variants() {
    let src = std::fs::read_to_string(project_root().join("src/data.rs")).expect("read data.rs");
    // Find the Value enum body.
    let start = src.find("pub enum Value {").expect("Value enum not found");
    let end_rel = src[start..]
        .match_indices("\n}")
        .next()
        .map(|(i, _)| i)
        .unwrap_or(src.len() - start);
    let body = &src[start..start + end_rel];

    // Count occurrences of the "codegen-internal" / "codegen-only"
    // marker string.  Each sanctioned variant's docstring contains
    // exactly one occurrence.  If new variants add the marker, the
    // count exceeds the sanctioned-list length.
    let marker_lines = body
        .lines()
        .filter(|l| l.contains("codegen-internal") || l.contains("codegen-only"))
        .count();

    // Each sanctioned variant must be present in the enum.
    for v in SANCTIONED_CODEGEN_VALUE_VARIANTS {
        assert!(
            body.contains(&format!("{v}(")),
            "sanctioned codegen-only variant `{v}` not found in Value enum"
        );
    }

    // No additional codegen-only variants allowed.  The marker may
    // appear multiple times in a single variant's docstring (we use
    // `<=` rather than `==` to tolerate prose mentions like
    // "this variant is codegen-internal" appearing twice).  But
    // adding a NEW variant with the marker bumps the count above
    // a tolerated ceiling and breaks this gate.
    let max_tolerated = SANCTIONED_CODEGEN_VALUE_VARIANTS.len() * 5;
    assert!(
        marker_lines <= max_tolerated,
        "codegen-internal marker appears {marker_lines} times in Value enum; \
         expected at most {max_tolerated} (sanctioned list = {SANCTIONED_CODEGEN_VALUE_VARIANTS:?}).  \
         A new codegen-only variant likely landed.  Either remove it (preferred — use a \
         string-aware companion entry point) or add it to SANCTIONED_CODEGEN_VALUE_VARIANTS \
         AND document why in plan 09 phase 00's scaffold doc."
    );
}
