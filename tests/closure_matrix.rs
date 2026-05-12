// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-15 closure-validation matrix.
//!
//! Each test exercises one cell of the (capture-shape × storage-
//! destination) matrix described in
//! `doc/claude/plans/15-closure-validation/00-matrix.md`.  Every cell
//! runs under both the interpreter and `--native`; the harness in
//! `tests/common/cross_mode.rs` (shared with `tests/tuple_matrix.rs`
//! and `tests/template_matrix.rs`) asserts byte-identical stdout.
//!
//! **Every cell is `#[ignore]` by default** — the harness shells out
//! to `loft --interpret` and `loft --native` per cell, the latter
//! invoking `rustc`.  Too heavy for the default `cargo test` path.
//! Run the matrix explicitly:
//!
//! ```bash
//! cargo test --release --test closure_matrix -- --ignored
//! ```
//!
//! or a single cell:
//!
//! ```bash
//! cargo test --release --test closure_matrix -- --ignored c0_d1_non_cap_local
//! ```
//!
//! ## The frozen matrix (phase 00, locked 2026-05-12)
//!
//! Cell legend: `PASS:test_name` / `FIX:phase` / `CLOSED:reason`.
//!
//! | | D1 — local var | D2 — direct stack | D3 — struct field | D4 — vector element |
//! |---|---|---|---|---|
//! | **C0** non-capturing | FIX:01 | FIX:01 | FIX:01 | FIX:01 |
//! | **C1** basic-type capture | FIX:02 | FIX:02 | FIX:02 | CLOSED:vec-of-capturing-closure |
//! | **C2** text capture | FIX:03 (decision: leak fix vs document) | FIX:03 | FIX:03 | CLOSED:vec-of-capturing-closure |
//! | **C3** Reference capture | FIX:04 | FIX:04 | FIX:04 | CLOSED:vec-of-capturing-closure |
//! | **C5** multi-capture | FIX:02 | FIX:02 | FIX:02 | CLOSED:vec-of-capturing-closure |
//! | **C6** nested closure | FIX:05 | FIX:05 | FIX:05 (deferred) | CLOSED:vec-of-capturing-closure |
//! | **C7** vector capture | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal |
//!
//! D5 (tuple element) is intentionally absent — it's covered by
//! `doc/claude/plans/finished/14-tuple-validation/03-closures.md`.
//!
//! Each `body` must declare `fn test() { … }` and any helper fns at
//! file scope; the harness appends `fn main() { test(); }`.  loft
//! does not allow nested fn definitions, so helpers must live
//! alongside `fn test`.

mod common;

// ── Phase 00 — harness smoke ────────────────────────────────────────────────
//
// Two smoke cells (no closure work) prove the closure_matrix.rs
// binary wires correctly to the shared cross_mode harness.  Mirrors
// the smoke pattern in tests/tuple_matrix.rs and
// tests/template_matrix.rs so a future contributor recognises the
// shape immediately.

cross_mode!(
    harness_smoke_basic,
    r#"
    fn test() {
        print("42\n");
        assert(true, "smoke");
    }
    "#
);

cross_mode!(
    harness_smoke_arithmetic,
    r#"
    fn test() {
        a = 3 + 4;
        print("{a}\n");
        assert(a == 7, "smoke arithmetic");
    }
    "#
);

// ── Phase 00 acceptance — one C0/D1 cell end-to-end ────────────────────────
//
// The phase-00 spec
// (doc/claude/plans/15-closure-validation/00-matrix.md § Acceptance)
// requires exactly one closure cell that runs green to validate the
// harness wires correctly through to closure-shaped code.  C0/D1
// (non-capturing lambda assigned to a local variable, called inline)
// is the simplest closure shape — proves the matrix wiring works
// end-to-end before phase 01 broadens to the full C0 row.
//
// The remaining C0/D2 / C0/D3 / C0/D4 cells, plus all C1+ cells,
// land in their respective phase commits.

cross_mode!(
    c0_d1_non_cap_local,
    r#"
    fn test() {
        f = fn(x: integer) -> integer { x + 1 };
        result = f(41);
        print("{result}\n");
        assert(result == 42, "c0_d1_non_cap_local");
    }
    "#
);
