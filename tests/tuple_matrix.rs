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
//! **Every cell is `#[ignore]` by default** — the harness shells out
//! to `loft --interpret` and `loft --native` per cell, the latter
//! invoking `rustc`.  That is too heavy for the default `cargo test`
//! path.  Run the matrix explicitly:
//!
//! ```bash
//! cargo test --release --test tuple_matrix -- --ignored
//! ```
//!
//! or a single cell:
//!
//! ```bash
//! cargo test --release --test tuple_matrix -- --ignored e1_d1_int_int_local
//! ```
//!
//! Each `body` must declare `fn test() { … }` (the entry point) and
//! any helper fns at file scope; the harness appends
//! `fn main() { test(); }`.  loft does not allow nested fn
//! definitions, so helpers must live alongside `fn test`.

mod common;

// ── Phase 00 — harness smoke ────────────────────────────────────────────────

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

// ── Phase 01 — basic + text scalars across D1 (local var) and D2 (stack) ────

// E1×D1 — basic scalars in a local variable.

cross_mode!(
    e1_d1_int_int_local,
    r#"
    fn test() {
        t = (3, 7);
        print("{t.0},{t.1}\n");
        assert(t.0 == 3, "e1_d1 .0");
        assert(t.1 == 7, "e1_d1 .1");
    }
    "#
);

cross_mode!(
    e1_d1_float_bool_local,
    r#"
    fn test() {
        t = (3.5, true);
        print("{t.0},{t.1}\n");
        assert(t.0 == 3.5, "e1_d1 float");
        assert(t.1, "e1_d1 bool");
    }
    "#
);

// e1_d1_char_int_local — closed by P207 fix 2026-05-04.
// `src/generation/calls.rs::substitute_template_body` now wraps a
// Type::Character TupleGet argument with `ops::to_char(...)` (mirroring
// the existing `Value::Var` char wrap), so the OpConvIntFromCharacter
// template's `@v1 == char::from(0)` comparison gets a `char`, not the
// `i32`-typed tuple-element read.
cross_mode!(
    e1_d1_char_int_local,
    r#"
    fn test() {
        t = ('a', 42);
        print("{t.0},{t.1}\n");
        assert(t.0 == 'a', "e1_d1 char");
        assert(t.1 == 42, "e1_d1 int");
    }
    "#
);

// E1×D2 — basic scalars on the direct stack.

cross_mode!(
    e1_d2_arg_int_int,
    r#"
    fn show(p: (integer, integer)) {
        print("{p.0},{p.1}\n");
        assert(p.0 == 3, "e1_d2_arg .0");
        assert(p.1 == 7, "e1_d2_arg .1");
    }
    fn test() {
        show((3, 7));
    }
    "#
);

cross_mode!(
    e1_d2_inline_get,
    r#"
    fn test() {
        a = (3, 7).0;
        b = (3, 7).1;
        print("{a},{b}\n");
        assert(a == 3, "e1_d2 inline .0");
        assert(b == 7, "e1_d2 inline .1");
    }
    "#
);

cross_mode!(
    e1_d2_match_subj,
    r#"
    fn test() {
        result = match (3, 7) {
            (0, _) => "zero"
            (n, m) => "{n},{m}"
        };
        print("{result}\n");
        assert(result == "3,7", "e1_d2_match_subj");
    }
    "#
);

cross_mode!(
    e1_d2_if_arm,
    r#"
    fn test() {
        cond = true;
        x = if cond { (1, 2) } else { (3, 4) };
        print("{x.0},{x.1}\n");
        assert(x.0 == 1, "e1_d2_if_arm .0");
        assert(x.1 == 2, "e1_d2_if_arm .1");
    }
    "#
);

// E1×D2 return — T1.8a (tuple-return convention) lands the supporting
// codegen for tuple-of-text returns; basic int/int returns already worked
// before the fix.  Cell un-ignored once T1.8a closes.
cross_mode!(
    e1_d2_return_int_int,
    r#"
    fn make_pair() -> (integer, integer) { (3, 7) }
    fn test() {
        t = make_pair();
        print("{t.0},{t.1}\n");
        assert(t.0 == 3 && t.1 == 7, "e1_d2_return");
    }
    "#
);

// E1n — `integer not null` element (T1.7).

cross_mode!(
    e1n_d1_local,
    r#"
    fn test() {
        t: (integer not null, integer) = (5, 9);
        print("{t.0},{t.1}\n");
        assert(t.0 == 5, "e1n_d1 .0");
        assert(t.1 == 9, "e1n_d1 .1");
    }
    "#
);

cross_mode!(
    e1n_d2_arg,
    r#"
    fn show(p: (integer not null, integer)) {
        print("{p.0},{p.1}\n");
        assert(p.0 == 5 && p.1 == 9, "e1n_d2_arg");
    }
    fn test() {
        p: (integer not null, integer) = (5, 9);
        show(p);
    }
    "#
);

// E2 — text element (T1.8b lifetime is the active risk).

cross_mode!(
    e2_d1_text_text_local,
    r#"
    fn test() {
        t = ("alpha", "beta");
        print("{t.0}|{t.1}\n");
        assert(t.0 == "alpha", "e2_d1 .0");
        assert(t.1 == "beta", "e2_d1 .1");
    }
    "#
);

cross_mode!(
    e2_d1_text_int_local,
    r#"
    fn test() {
        t = ("answer", 42);
        print("{t.0}|{t.1}\n");
        assert(t.0 == "answer", "e2_d1 mixed .0");
        assert(t.1 == 42, "e2_d1 mixed .1");
    }
    "#
);

cross_mode!(
    e2_d2_arg_text_text,
    r#"
    fn show(p: (text, text)) {
        print("{p.0}|{p.1}\n");
        assert(p.0 == "alpha" && p.1 == "beta", "e2_d2_arg");
    }
    fn test() {
        show(("alpha", "beta"));
    }
    "#
);

cross_mode!(
    e2_d2_inline_text,
    r#"
    fn test() {
        a = ("alpha", "beta").0;
        b = ("alpha", "beta").1;
        print("{a}|{b}\n");
        assert(a == "alpha", "e2_d2_inline .0");
        assert(b == "beta", "e2_d2_inline .1");
    }
    "#
);

// E2×D2 return — closed by T1.8a fix (tuple-of-text return codegen).
cross_mode!(
    e2_d2_return_text_text,
    r#"
    fn make_pair() -> (text, text) { ("alpha", "beta") }
    fn test() {
        t = make_pair();
        print("{t.0}|{t.1}\n");
        assert(t.0 == "alpha" && t.1 == "beta", "e2_d2_return");
    }
    "#
);

// ── Phase 02 — nested tuples (E3 × D1, D2) ──────────────────────────────────
//
// Closes the matrix cells for tuples-containing-tuples: `((A, B), C)`,
// `(A, (B, C))`, `((A, B), (C, D))`, plus mixed text + element-of-element
// assignment.  P212's panic at codegen.rs:1527 (closed 2026-05-04 by the
// recursive `emit_tuple_put_ops` helper) is the fix that makes these run;
// the cells below are the matrix-wiring half of phase 02.

cross_mode!(
    e3_d1_nested_local,
    r#"
    fn test() {
        t = ((1, 2), 3);
        print("{t.0.0},{t.0.1},{t.1}\n");
        assert(t.0.0 == 1, "e3_d1 t.0.0");
        assert(t.0.1 == 2, "e3_d1 t.0.1");
        assert(t.1 == 3,   "e3_d1 t.1");
    }
    "#
);

cross_mode!(
    e3_d1_nested_deep,
    r#"
    fn test() {
        t = ((1, 2), (3, 4));
        print("{t.0.0},{t.0.1},{t.1.0},{t.1.1}\n");
        assert(t.0.0 == 1 && t.0.1 == 2 && t.1.0 == 3 && t.1.1 == 4, "e3_d1_deep");
    }
    "#
);

// e3_d1_text_inside — closed by P247 fix 2026-05-11 (nested-tuple
// text read in format strings emits `.clone()` + `&*({block})` wrap).
cross_mode!(
    e3_d1_text_inside,
    r#"
    fn test() {
        t = ((1, "a"), (2, "b"));
        print("{t.0.0}|{t.0.1}|{t.1.0}|{t.1.1}\n");
        assert(t.0.0 == 1 && t.0.1 == "a" && t.1.0 == 2 && t.1.1 == "b",
               "e3_d1_text_inside");
    }
    "#
);

// e3_d1_elem_elem_assign — closed by P248 fix 2026-05-11
// (nested-LHS extractor + nested-tuple TuplePut codegen).
cross_mode!(
    e3_d1_elem_elem_assign,
    r#"
    fn test() {
        t = ((1, 2), (3, 4));
        t.0.1 = 99;
        t.1.0 = 77;
        print("{t.0.0},{t.0.1},{t.1.0},{t.1.1}\n");
        assert(t.0.0 == 1 && t.0.1 == 99 && t.1.0 == 77 && t.1.1 == 4,
               "e3_d1_elem_elem_assign");
    }
    "#
);

cross_mode!(
    e3_d2_nested_arg,
    r#"
    fn show(p: ((integer, integer), integer)) {
        print("{p.0.0},{p.0.1},{p.1}\n");
        assert(p.0.0 == 1 && p.0.1 == 2 && p.1 == 3, "e3_d2_nested_arg");
    }
    fn test() {
        show(((1, 2), 3));
    }
    "#
);

// e3_d2_nested_return DEFERRED — T1.8a (nested-tuple return convention)
// not yet implemented.  The phase 02 design doc lists this as the one
// cell that lands together with T1.8a; the cross_mode! macro doesn't
// take per-test attributes today, so re-add this cell when T1.8a
// closure makes it pass.

// ── Phase 03 — closure-element tuples (E4 × D1, D2) ─────────────────────────
//
// Tuples whose elements are closures (Type::Function).  Storing works,
// but CALLING through the tuple (`t.0(10)`) crashes — the CallRef IR
// today only addresses bare-name fn-ref vars, not TupleGet sources.
// Filed P249 (2026-05-11).  Workaround: hoist the closure into a Var
// first (`f = t.0; f(10)`).  Cells stay in the matrix as live
// regression guards.

cross_mode!(
    e4_d1_closure_local,
    r#"
    fn test() {
        add5 = fn(x: integer) -> integer { x + 5 };
        t = (add5, 99);
        // store-only: read tuple's non-closure half + verify
        // closure-half still occupies the slot (no crash on print
        // of t.1 next to t.0).
        print("{t.1}\n");
        assert(t.1 == 99, "e4_d1_closure_local stored");
    }
    "#
);

cross_mode!(
    e4_d1_closure_call,
    r#"
    fn test() {
        add5 = fn(x: integer) -> integer { x + 5 };
        t = (add5, 99);
        result = t.0(10);
        print("{result},{t.1}\n");
        assert(result == 15 && t.1 == 99, "e4_d1_closure_call");
    }
    "#
);

cross_mode!(
    e4_d1_closure_swap,
    r#"
    fn test() {
        a = fn(x: integer) -> integer { x + 1 };
        b = fn(x: integer) -> integer { x * 2 };
        t = (a, b);
        r0 = t.0(10);
        r1 = t.1(10);
        print("{r0},{r1}\n");
        assert(r0 == 11 && r1 == 20, "e4_d1_closure_swap");
    }
    "#
);

cross_mode!(
    e4_d1_capture_survives,
    r#"
    fn test() {
        captured = 42;
        read_captured = fn() -> integer { captured };
        t = (read_captured, "tag");
        print("{t.0()}|{t.1}\n");
        assert(t.0() == 42 && t.1 == "tag", "e4_d1_capture_survives");
    }
    "#
);

cross_mode!(
    e4_d2_closure_arg,
    r#"
    fn invoke(p: (fn(integer) -> integer, text)) -> integer {
        print("{p.1}\n");
        p.0(7)
    }
    fn test() {
        sq = fn(n: integer) -> integer { n * n };
        result = invoke((sq, "sq-tag"));
        print("{result}\n");
        assert(result == 49, "e4_d2_closure_arg");
    }
    "#
);
// e4_d2_closure_return DEFERRED — same T1.8a return-convention block
// as e3_d2_nested_return.  Lands when T1.8a does.
