// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-17 bounded-generic / interface validation matrix.
//!
//! Each test exercises one cell of the (T-parameter usage × bound
//! shape) matrix described in
//! `doc/claude/plans/17-template-validation/00-matrix.md`.  Every
//! cell runs under both the interpreter and `--native`; the harness
//! in `tests/common/cross_mode.rs` asserts byte-identical stdout.
//!
//! **Every cell is `#[ignore]` by default** (the `cross_mode!`
//! macro applies the attribute uniformly across all matrix
//! binaries).  The harness shells out to `loft --interpret` and
//! `loft --native` per cell; the latter invokes `rustc`.  That is
//! too heavy for the default `cargo test` path.  Run the matrix
//! explicitly:
//!
//! ```bash
//! cargo test --release --test template_matrix -- --ignored
//! ```
//!
//! or a single cell:
//!
//! ```bash
//! cargo test --release --test template_matrix -- --ignored u1_b4_addable_dbl_int
//! ```
//!
//! Cell names follow the plan-17 convention `u<U>_b<B>[<sub>]_<sub>`
//! to avoid collision with plan-14 (`e<E>_d<D>`), plan-15
//! (`c<C>_d<D>`), and plan-16 (`y<Y>_x<X>`).
//!
//! Each `body` must declare `fn test() { … }` (the entry point) and
//! any helper fns at file scope; the harness appends
//! `fn main() { test(); }`.  loft does not allow nested fn
//! definitions, so helpers (including `fn dbl<T: Addable>(...)`)
//! must live alongside `fn test`.

mod common;

// ── Phase 00 — harness smoke ────────────────────────────────────────────────
//
// A single known-passing cell that proves the binary compiles, the
// `cross_mode!` macro is wired correctly, and the cross-mode harness
// can drive a bounded-generic shape under both backends.  The smoke
// uses the canonical `dbl<T: Addable>(x: T) -> T { x + x }` shape
// from the Phase 00 matrix (B4 × U1 = PASS-pre).

cross_mode!(
    harness_smoke_template,
    r#"
    fn dbl<T: Addable>(x: T) -> T { x + x }
    fn test() {
        a = dbl(7);
        print("{a}\n");
        assert(a == 14, "smoke_addable_dbl_int");
    }
    "#
);

// ── PASS-pre cells from the matrix ──────────────────────────────────────────
//
// These cells passed during the Phase 00 pre-flight survey
// (2026-05-04).  The tests get written here so the matrix stays
// uniform AND the regression net catches later breakage; no
// production code change was needed for any of them.

// U1 × B4 (op-sugar Addable), float specialisation — same
// monomorphisation path as the int smoke above.
cross_mode!(
    u1_b4_addable_dbl_float,
    r#"
    fn dbl<T: Addable>(x: T) -> T { x + x }
    fn test() {
        b = dbl(3.5);
        print("{b}\n");
        assert(b == 7.0, "u1_b4_float");
    }
    "#
);

// U2 × B0 (no bound, T return) — baseline opaque-T pass-through.
cross_mode!(
    u2_b0_no_bound_identity_int,
    r#"
    fn id<T>(x: T) -> T { x }
    fn test() {
        a = id(42);
        print("{a}\n");
        assert(a == 42, "u2_b0_int");
    }
    "#
);

// U2 × B1.O (Ordered, T return) — bounded identity + comparison.
cross_mode!(
    u2_b1o_ordered_max_int,
    r#"
    fn max<T: Ordered>(a: T, b: T) -> T { if a > b { a } else { b } }
    fn test() {
        m = max(3, 7);
        print("{m}\n");
        assert(m == 7, "u2_b1o_int");
    }
    "#
);

// U2 × B2 (multi-bound, T return) — multi-bound monomorphisation.
cross_mode!(
    u2_b2_multibound_eq_or_gt_int,
    r#"
    fn cmp_eq<T: Ordered + Equatable>(a: T, b: T) -> integer {
        if a == b { 0 } else if a > b { 1 } else { -1 }
    }
    fn test() {
        x = cmp_eq(3, 7);
        y = cmp_eq(7, 7);
        z = cmp_eq(9, 7);
        print("{x}|{y}|{z}\n");
        assert(x == -1, "u2_b2_lt");
        assert(y == 0, "u2_b2_eq");
        assert(z == 1, "u2_b2_gt");
    }
    "#
);
