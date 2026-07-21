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
//! **These cells RUN by default** (@PLN114).  They were `#[ignore]`d as too heavy;
//! measured, the whole 201-cell matrix takes ~10 seconds.  The cost was never the
//! issue — the silence was: two SIGSEGVs, a silent `+1` corruption of every narrow
//! tuple element, a 3.4x memory overhead and a cross-backend par-worker corruption
//! all sat behind that attribute until the set was run by hand.
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

// ── Phase 02 — tuple-of-T returns (U3, U7, U8) ─────────────────────────────
//
// Plan-17 phase 02 fills U3 (uniform-T tuple return), U7 (multi-T
// tuple-return arity ≥3), and U8 (T inside format / inline expr).
// The pre-flight survey predicted parser breakage on these shapes
// — turned out the (A) fix (substitute_type recurses through
// Type::Tuple; first-pass return-type prediction via
// predict_generic_return_type) closed the uniform-T cases.
//
// What still BREAKS (plan-17 phase 02 surfaced; both filed
// 2026-05-09):
// - **P237** — mixed-element-type generic tuple `(T, integer)`,
//   `(T, boolean)`, etc.  Interp SIGSEGVs / silent garbage;
//   native rustc E0308 (T not substituted in operator-method
//   calls inside the tuple constructor).  Reproducer:
//   `fn pair_with_one<T: Addable>(x: T) -> (T, integer) { (x+x, 1) }`.
// - **P238** — uniform-T tuple `(T, T)` instantiated with
//   T = text fails native rustc E0308 (`expected &str, found
//   String`).  Interp works.  Reproducer:
//   `fn pair_t<T: Equatable>(a: T) -> (T, T) { (a, a) }; pair_t("hi")`.
//
// Both bugs filed in PROBLEMS.md; reproducers in
// /tmp/p_followups/p23{7,8}_*.loft.  Cells for these shapes
// stay out of the binary until the P-issues close — the
// cross_mode harness asserts BOTH backends pass, so an
// always-failing cell would gate `--ignored` runs.

// U3 × B0 — no bound, uniform tuple-of-T return.  Verifies
// (A) fix works for the most basic case.
cross_mode!(
    u3_b0_no_bound_pair_int,
    r#"
    fn pair<T>(a: T, b: T) -> (T, T) { (a, b) }
    fn test() {
        t = pair(7, 3);
        print("{t.0}|{t.1}\n");
        assert(t.0 == 7 && t.1 == 3, "u3_b0_int");
    }
    "#
);

// U3 × B1.O — Ordered, tuple-of-T return.  Canonical
// `min_max` — the pre-flight failure that's now closed by (A).
// Verifies the fix across multiple monomorphisations
// (integer + float).
cross_mode!(
    u3_b1o_ordered_min_max,
    r#"
    fn min_max<T: Ordered>(a: T, b: T) -> (T, T) {
        if a < b { (a, b) } else { (b, a) }
    }
    fn test() {
        ti = min_max(7, 3);
        tf = min_max(2.5, 1.5);
        print("{ti.0}|{ti.1}|{tf.0}|{tf.1}\n");
        assert(ti.0 == 3 && ti.1 == 7, "u3_b1o_int");
        assert(tf.0 == 1.5 && tf.1 == 2.5, "u3_b1o_float");
    }
    "#
);

// U3 × B1.E — Equatable, uniform-T tuple-of-T return that
// uses the bound's `==` in the body.  Different from U3.B0
// (which returns args verbatim).
cross_mode!(
    u3_b1e_equatable_pair_when_eq,
    r#"
    fn pair_when<T: Equatable>(a: T, b: T, c: T) -> (T, T) {
        if a == b { (a, c) } else { (b, c) }
    }
    fn test() {
        t = pair_when(3, 3, 99);
        u = pair_when(3, 5, 99);
        print("{t.0}|{t.1}|{u.0}|{u.1}\n");
        assert(t.0 == 3 && t.1 == 99, "u3_b1e_eq_path");
        assert(u.0 == 5 && u.1 == 99, "u3_b1e_neq_path");
    }
    "#
);

// U3 × B1.A — Addable, uniform-T tuple-of-T return.  The
// inline form `(a, a + b)` triggers P237 (T's operator inside
// a tuple constructor element fails to substitute) on both
// backends.  The hoisted form (sum to a local first, then
// construct) works.  Cell uses the workaround so the
// regression net catches future cases where the workaround
// itself breaks; the inline form is documented as a known-
// failure shape in PROBLEMS.md P237.
cross_mode!(
    u3_b1a_addable_pair_with_hoisted_sum,
    r#"
    fn pair_sum<T: Addable>(a: T, b: T) -> (T, T) {
        s = a + b;
        (a, s)
    }
    fn test() {
        ti = pair_sum(2, 3);
        tf = pair_sum(1.5, 2.5);
        print("{ti.0}|{ti.1}|{tf.0}|{tf.1}\n");
        assert(ti.0 == 2 && ti.1 == 5, "u3_b1a_int");
        assert(tf.0 == 1.5 && tf.1 == 4.0, "u3_b1a_float");
    }
    "#
);

// U3 destructure — `(lo, hi) = min_max(...)` parser path.
// Pre-flight predicted parser rejection; verified working
// 2026-05-09.
cross_mode!(
    u3_b1o_ordered_destructure,
    r#"
    fn min_max<T: Ordered>(a: T, b: T) -> (T, T) {
        if a < b { (a, b) } else { (b, a) }
    }
    fn test() {
        (lo, hi) = min_max(7, 3);
        print("{lo}|{hi}\n");
        assert(lo == 3 && hi == 7, "u3_destr");
    }
    "#
);

// U7 × B1.O — three-arity tuple-return.  Pre-flight predicted
// parser breakage; verified working.
cross_mode!(
    u7_b1o_ordered_triple_arity_three,
    r#"
    fn triple<T: Ordered>(a: T, b: T) -> (T, T, T) {
        if a < b { (a, b, a) } else { (b, a, b) }
    }
    fn test() {
        t = triple(7, 3);
        print("{t.0}|{t.1}|{t.2}\n");
        assert(t.0 == 3 && t.1 == 7 && t.2 == 3, "u7_b1o");
    }
    "#
);

// U7 nested — `((T, T), T)` shape.  Verifies recursive
// substitute_type traversal handles nested Tuple correctly.
cross_mode!(
    u7_b1o_ordered_nested_pair,
    r#"
    fn nested<T: Ordered>(a: T, b: T) -> ((T, T), T) {
        if a < b { ((a, b), a) } else { ((b, a), b) }
    }
    fn test() {
        t = nested(7, 3);
        print("{t.0.0}|{t.0.1}|{t.1}\n");
        assert(t.0.0 == 3 && t.0.1 == 7 && t.1 == 3, "u7_nested");
    }
    "#
);

// U8 × B1.O — inline tuple-element access in format string.
// Pre-flight predicted parser breakage on `{min_max(7,3).0}`;
// verified working 2026-05-09.
cross_mode!(
    u8_b1o_ordered_inline_format,
    r#"
    fn min_max<T: Ordered>(a: T, b: T) -> (T, T) {
        if a < b { (a, b) } else { (b, a) }
    }
    fn test() {
        print("{min_max(7, 3).0}|{min_max(7, 3).1}\n");
    }
    "#
);

// U3 × B2 — multi-bound tuple return.  Combines Addable +
// Ordered to compute (smaller-of-pair, sum).
cross_mode!(
    u3_b2_addable_ordered_pair,
    r#"
    fn small_and_sum<T: Addable + Ordered>(a: T, b: T) -> (T, T) {
        s = a + b;
        if a < b { (a, s) } else { (b, s) }
    }
    fn test() {
        t = small_and_sum(7, 3);
        u = small_and_sum(2, 5);
        print("{t.0}|{t.1}|{u.0}|{u.1}\n");
        assert(t.0 == 3 && t.1 == 10, "u3_b2_a");
        assert(u.0 == 2 && u.1 == 7, "u3_b2_b");
    }
    "#
);

// ── Phase 03 — Printable + vector-of-T (U1.B1.P, U2.B1.P,
//                  U5.B1.P, U6.B1.P) ──────────────────────────
//
// (C) closure 2026-05-04 added 6 `to_text` impls to
// `default/01_code.loft` (integer / float / single / boolean /
// character / text), making built-ins satisfy `Printable`
// automatically as the docs claim.  P208 (text concat in
// bounded body) closed 2026-05-04.  Phase 03 cells exercise
// the working surface.
//
// What still BREAKS (filed as P237 above and P239 below):
// - U3 × B1.P with method call inside tuple constructor — same
//   pattern as P237 (`(x.to_text(), "x")` triggers it because
//   to_text is a bound-supplied method).  Workaround: hoist
//   to local first.
// - U5 × * (any bound, including no-bound) — for-loop over
//   `vector<T>` in a generic context crashes both backends.
//   Filed as P239.  No clean workaround — caller must
//   specialise.  All U5 cells stay out of the binary until
//   P239 closes.
// - U6 × * with internal iteration (vector<T> input + push to
//   output) — fails via P239 (the input iteration crashes
//   before output construction matters).  U6 cells with simple
//   pass-through (no iteration) work and ship below.

// U2 × B1.P — Printable bound, T return that's text via
// to_text().  Verifies the (C) closure's built-in
// satisfaction across integer / float / boolean / text
// monomorphisations + the P208 fix.
cross_mode!(
    u2_b1p_printable_show,
    r#"
    fn show<T: Printable>(x: T) -> text { x.to_text() }
    fn test() {
        i = show(42);
        f = show(3.14);
        b = show(true);
        s = show("hi");
        print("{i}|{f}|{b}|{s}\n");
        assert(i == "42", "u2_b1p_int");
        assert(b == "true", "u2_b1p_bool");
        assert(s == "hi", "u2_b1p_text");
    }
    "#
);

// U1 × B1.P — Printable body op + text concatenation.
// Pre-flight (B-native) failure path; P208 closed 2026-05-04.
// Verifies the fix across multiple monomorphisations.
cross_mode!(
    u1_b1p_printable_concat,
    r#"
    fn shout<T: Printable>(x: T) -> text { x.to_text() + "!" }
    fn test() {
        i = shout(42);
        f = shout(3.14);
        s = shout("hi");
        print("{i}|{f}|{s}\n");
        assert(i == "42!", "u1_b1p_int");
        assert(s == "hi!", "u1_b1p_text");
    }
    "#
);

// U6 simplest — pass-through `vector<T>` return, no iteration
// inside the generic body.  Works because there's no for-loop
// to trip P239; the vector value passes through verbatim.
cross_mode!(
    u6_b0_no_bound_passthrough,
    r#"
    fn passthru<T>(v: vector<T>) -> vector<T> { v }
    fn test() {
        nums: vector<integer> = [1, 2, 3];
        out = passthru(nums);
        print("{out.len()}|{out[0]}|{out[1]}|{out[2]}\n");
        assert(out.len() == 3, "u6_b0_len");
        assert(out[0] == 1 && out[1] == 2 && out[2] == 3, "u6_b0_vals");
    }
    "#
);

// ── Phase 04 — user-defined interface (B3) ─────────────────────
//
// User-defined interfaces use the `interface` keyword.  Built-in
// types satisfy them structurally if the methods exist; user
// structs can implement them by defining matching free fns.
// Phase 04 verifies that <T: UserIface> bounded fns dispatch
// correctly across multiple impls.
//
// What still BREAKS (filed earlier; phase-04-relevant):
// - Multi-bound + 2+ bound-operator locals + tuple return → P240
//   (cross-mode divergence).  All cells here either avoid the
//   2+-locals shape OR use single-bound only.

// U2 × B3 — user-defined Shape interface, T return type is
// concrete (float).  Two impls (Circle, Square); verifies
// both monomorphisations produce the right area().
cross_mode!(
    u2_b3_user_iface_shape_area,
    r#"
    interface Shape { fn area(self: Self) -> float }
    struct Circle { radius: float }
    fn area(self: Circle) -> float { self.radius * self.radius * 3.14 }
    struct Square { side: float }
    fn area(self: Square) -> float { self.side * self.side }
    fn show_area<T: Shape>(s: T) -> float { s.area() }
    fn test() {
        c = Circle { radius: 2.0 };
        sq = Square { side: 3.0 };
        print("{show_area(c)}|{show_area(sq)}\n");
        assert(show_area(c) == 12.56, "u2_b3_circle");
        assert(show_area(sq) == 9.0, "u2_b3_square");
    }
    "#
);

// U1 × B3 — body op using user-iface method + text concat.
// Verifies single bound + multi-impl + body-op composition.
cross_mode!(
    u1_b3_user_iface_showable_concat,
    r#"
    interface Showable { fn label(self: Self) -> text }
    struct Item { id: integer }
    fn label(self: Item) -> text { "item-{self.id}" }
    struct Note { msg: text }
    fn label(self: Note) -> text { "note:{self.msg}" }
    fn shout<T: Showable>(s: T) -> text { s.label() + "!" }
    fn test() {
        i = Item { id: 7 };
        n = Note { msg: "hi" };
        print("{shout(i)}|{shout(n)}\n");
        assert(shout(i) == "item-7!", "u1_b3_item");
        assert(shout(n) == "note:hi!", "u1_b3_note");
    }
    "#
);

// ── Phase 05 — B4 nested + B5 two-T (mostly blocked) ───────────
//
// B5 (two-T parameters) is a confirmed PARSER FEATURE GAP per
// the plan-17 README — `<A: B1, B: B2>`, `<A, B>`, and
// `where`-clauses all parse-error at the comma in pre-flight
// (2026-05-04).  Re-verified 2026-05-09: still parse-errors.
// All B5 cells are CLOSED:feature-gap; no cells in the binary.
//
// B4 (op-sugar) was largely covered by `harness_smoke_template`
// + `u1_b4_addable_dbl_float`.  Composition into more complex
// shapes mostly hits other P-issues:
// - B4 inside tuple element (`(x, x + x)`) — P237.
// - B4 inside vector push — P241.
// - B4 result interpolated in format-string `{x + x}` — P242.
//
// One workaround cell ships below: explicit `to_text()` to
// route around P242 for op-sugar in inline format.

// U8 × B4 (workaround form) — op-sugar result hoisted to local
// then to_text()'d for format-string interpolation.  Direct
// inline `println("val={x + x}")` triggers P242; this cell
// pins the workaround so the regression net catches future
// breakage of THAT path.
cross_mode!(
    u8_b4_addable_to_text_format,
    r#"
    fn show_dbl<T: Addable + Printable>(x: T) {
        s = x + x;
        label = s.to_text();
        println("dbl={label}");
    }
    fn test() {
        show_dbl(7);
        show_dbl(3.5);
    }
    "#
);

// U8 × B1.P — P242 closure 2026-05-09: format-string
// interpolation of `x: T` where T is the current generic
// type variable AND the bound supplies `to_text` (Printable)
// now routes through the bound's `t_<len><tv>_to_text` stub
// automatically.  Reproducer from P242 was
// `fn show<T: Printable>(x: T) { println("val={x}") }`;
// this cell pins it.
cross_mode!(
    u8_b1p_printable_inline_t,
    r#"
    fn show<T: Printable>(x: T) {
        println("val={x}");
    }
    fn test() {
        show(7);
        show(3.14);
        show("hi");
    }
    "#
);

// Plan-17 U2 × Bx — uniform `(T, T)` tuple return for T = text.
// Closes P238: native codegen emitted `t_4text_pair_t(cell,
// "hi".to_string())` (call-site wrapped `&str` literal in
// `.to_string()` because the assignment target's
// `tuple_text_to_string` flag leaked into argument processing) and
// the fn body emitted `return (var_a, var_a)` without
// `.to_string()` wrap (the parser's tuple-of-text →
// synthetic-struct rewrite never fires for generic
// monomorphisations because at parse time the tuple element type
// was a generic struct, not Text).  Both bugs fixed at the
// codegen layer in `src/generation/dispatch.rs::output_call`
// (clear `tuple_text_to_string` for arg processing) and
// `src/generation/emit.rs` Value::Return arm (set the flag for
// `Value::Tuple` returned from a tuple-of-text-typed fn).
cross_mode!(
    u2_b2_equatable_uniform_tuple_t,
    r#"
    fn pair_t<T: Equatable>(a: T) -> (T, T) {
        (a, a)
    }
    fn test() {
        t = pair_t(42);
        s = pair_t("hi");
        println("{t.0}|{t.1}|{s.0}|{s.1}");
    }
    "#
);

// Plan-17 U3 × B1.A — bound-supplied operator INSIDE a tuple
// constructor element.  Closes P237: `Value::Tuple` was missing
// from `substitute_type_in_value`'s recursion in
// src/parser/mod.rs, so calls to bound-supplied operator stubs
// (`t_1T_OpAdd(x, x)` etc.) sitting inside tuple-constructor
// elements stayed pointing at the GENERIC stub instead of being
// re-resolved to the concrete type's method during
// monomorphisation.  Native rejected with rustc E0308 (`expected
// DbRef, found i64`); interp returned silent garbage / SIGSEGV.
// Workaround was to hoist the operator into a typed local first
// (covered by `u3_b1a_addable_pair_with_hoisted_sum` already in
// the matrix).  Fix: added Value::Tuple, TuplePut, BreakWith,
// Yield, CallRef arms to substitute_type_in_value.
cross_mode!(
    u3_b1a_addable_inline_pair_with_sum,
    r#"
    fn pair_with_one<T: Addable>(x: T) -> (T, integer) {
        (x + x, 1)
    }
    fn test() {
        t = pair_with_one(5);
        u = pair_with_one(2.5);
        println("{t.0}|{t.1}|{u.0}|{u.1}");
    }
    "#
);
