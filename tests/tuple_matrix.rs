// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-14 tuple-validation matrix.
//!
//! Each test exercises one cell of the (element-type × storage-
//! destination) matrix described in
//! `doc/claude/plans/14-tuple-validation/00-matrix.md`.  Every cell
//! runs under both the interpreter and `--native`; the harness in
//! `tests/common/cross_mode.rs` asserts byte-identical stdout.
//!
//! Phase 00 ships only the harness smoke test.  Phases 01–05 fill in
//! the per-cell tests.

mod common;

// ── Phase 00 — harness smoke ────────────────────────────────────────────────
//
// Verifies the cross-mode harness end-to-end against trivial input.
// A green smoke means: the loft binary builds, the temp-snippet write
// path works, both `--interpret` and `--native` modes run, and stdout
// normalisation produces matching strings.

cross_mode!(
    harness_smoke_basic,
    r#"
    print("42\n");
    assert(true, "smoke");
    "#
);

cross_mode!(
    harness_smoke_arithmetic,
    r#"
    a = 3 + 4;
    print("{a}\n");
    assert(a == 7, "smoke arithmetic");
    "#
);
