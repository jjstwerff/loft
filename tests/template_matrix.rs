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

// ── Phase 01 — basic body + T-return baseline ──────────────────────────────
//
// Plan-17 phase 01 fills U1 × {B0, B1.O, B1.E, B1.A} and U2 ×
// {B1.E, B1.A} cells (B1.A baselines exist via U1.B4 smoke and
// U2.B0 PASS-pre; this phase adds explicit per-bound coverage).
// U1.B1.P + U2.B1.P (Printable) move to phase 03 because the
// stdlib's `to_text` story needed a separate decision (see plan-17
// README — closed (C) added 6 to_text impls 2026-05-04).

// U1 × B0 — no bound, T not used in body.  Verifies opaque-T
// pass-through compiles and monomorphises across multiple
// concrete arg types (integer, text, boolean) in the same
// program.  Emits a "Parameter x is never read" warning under
// both backends — expected; the harness compares stdout, not
// stderr.
cross_mode!(
    u1_b0_no_bound_unused_t,
    r#"
    fn count<T>(_x: T) -> integer { 1 }
    fn test() {
        a = count(7);
        b = count("hi");
        c = count(true);
        print("{a}|{b}|{c}\n");
        assert(a == 1 && b == 1 && c == 1, "u1_b0_unused");
    }
    "#
);

// U1 × B1.O — Ordered body op.  Returns integer (-1/0/1) so the
// cell exercises ONLY the body operation (`a > b` / `a < b`
// comparisons via the bound), separate from U2's T-return path.
cross_mode!(
    u1_b1o_ordered_compare,
    r#"
    fn cmp<T: Ordered>(a: T, b: T) -> integer {
        if a > b { 1 } else if a < b { -1 } else { 0 }
    }
    fn test() {
        x = cmp(3, 7);
        y = cmp(7, 3);
        z = cmp(5, 5);
        print("{x}|{y}|{z}\n");
        assert(x == -1 && y == 1 && z == 0, "u1_b1o_int");
    }
    "#
);

// U1 × B1.E — Equatable body op.  Returns boolean so the cell
// exercises only the equality check via the bound; verifies the
// bounded `==` resolves to the concrete type's OpEq across both
// integer and text monomorphisations.
cross_mode!(
    u1_b1e_equatable_check,
    r#"
    fn same<T: Equatable>(a: T, b: T) -> boolean { a == b }
    fn test() {
        ai = same(3, 3);
        bi = same(3, 5);
        at = same("hi", "hi");
        bt = same("hi", "bye");
        print("{ai}|{bi}|{at}|{bt}\n");
        assert(ai == true && bi == false, "u1_b1e_int");
        assert(at == true && bt == false, "u1_b1e_text");
    }
    "#
);

// U1 × B1.A — Addable body op (multi-step).  Distinct from the
// smoke's `dbl` shape: chains `+` across three args.  Verifies
// the bound-supplied `+` resolves correctly under the multi-
// monomorphisation case (integer + float).  Note: stdlib
// `Addable` declares `+` only — `-` (binary subtraction) requires
// a concrete type or a hypothetical Subtractable bound, NOT
// covered by Addable.
cross_mode!(
    u1_b1a_addable_sum_three,
    r#"
    fn sum3<T: Addable>(a: T, b: T, c: T) -> T { a + b + c }
    fn test() {
        i = sum3(1, 2, 3);
        f = sum3(1.5, 2.5, 4.0);
        print("{i}|{f}\n");
        assert(i == 6, "u1_b1a_int");
        assert(f == 8.0, "u1_b1a_float");
    }
    "#
);

// U2 × B1.E — Equatable T-return.  Picks one of the args based
// on equality; the function returns T (not boolean) so the
// generic-return-type machinery is exercised in addition to the
// bound-supplied operator.
cross_mode!(
    u2_b1e_equatable_pick,
    r#"
    fn pick<T: Equatable>(a: T, b: T, c: T) -> T {
        if a == b { a } else { c }
    }
    fn test() {
        i = pick(3, 3, 99);
        j = pick(3, 5, 99);
        s = pick("yes", "yes", "NO");
        t = pick("yes", "no", "NO");
        print("{i}|{j}|{s}|{t}\n");
        assert(i == 3 && j == 99, "u2_b1e_int");
        assert(s == "yes" && t == "NO", "u2_b1e_text");
    }
    "#
);

// U2 × B1.A — Addable T-return (different shape from smoke
// dbl).  Returns the smaller of (a+b) vs c for an Addable +
// Ordered bound; structured to exercise both T-return and
// bound-supplied operators in one cell.
cross_mode!(
    u2_b1ao_addable_ordered_min_sum,
    r#"
    fn min_sum<T: Addable + Ordered>(a: T, b: T, c: T) -> T {
        s = a + b;
        if s < c { s } else { c }
    }
    fn test() {
        i = min_sum(2, 3, 10);
        j = min_sum(7, 8, 4);
        print("{i}|{j}\n");
        assert(i == 5, "u2_b1ao_sum_smaller");
        assert(j == 4, "u2_b1ao_c_smaller");
    }
    "#
);
