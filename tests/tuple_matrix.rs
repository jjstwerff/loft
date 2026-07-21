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
//! **These cells RUN by default** (@PLN114).  They were `#[ignore]`d as too heavy;
//! measured, the whole 201-cell matrix takes ~10 seconds.  The cost was never the
//! issue — the silence was: two SIGSEGVs, a silent `+1` corruption of every narrow
//! tuple element, a 3.4x memory overhead and a cross-backend par-worker corruption
//! all sat behind that attribute until the set was run by hand.
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

cross_mode!(
    e3_d2_nested_return,
    r#"
    fn make_nested() -> ((integer, integer), integer) { ((10, 20), 30) }
    fn test() {
        // Shallow destructure: outer tuple split into the inner tuple
        // `t` plus the scalar `c`.  Deep `((a, b), c) = …` rejects at
        // the parser ("Tuple destructuring requires plain variable
        // names") — that's a separate destructure-pattern parser
        // limitation, not the return convention.  The shallow form
        // exercises the same return path (nested tuple value crosses
        // the function boundary; the synthetic `__tuple<…>` rewrite
        // from Plan-14 phase 07 carries the lifetime correctly).
        (t, c) = make_nested();
        print("{t.0},{t.1},{c}\n");
        assert(t.0 == 10 && t.1 == 20 && c == 30, "e3_d2_nested_return");
    }
    "#
);

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

// ── Phase 04 — element type 5 (struct references) ────────────────────────────
//
// Tuples whose elements are `Type::Reference` (struct DbRefs).  The
// **decision** for T1.8c is **MOVE semantics**: the source tuple's
// scope-exit emits no per-element `OpFreeRef` (`scopes.rs:1000-1009`
// `continue` stub), and each destructured destination variable is
// typed as `Type::Reference` and gets normal scope-exit cleanup —
// exactly what the runtime already does today.  See plan-14 phase 04
// (`04-references.md`) for the full rationale and the rejected
// "copy + null" alternative.
//
// Pre-flight passes on both backends for single-iteration shapes
// (swap, arg, return, mixed Ref+int, mixed Ref+text); a separate
// loop-iteration aliasing bug (filed P250) is parked behind a
// `_loop_*` follow-up and not gated here — these cells lock the
// move-semantics shape that works today as a regression guard.

cross_mode!(
    e5_d1_struct_ref_local,
    r#"
    struct P { v: integer }
    fn test() {
        a = P { v: 1 };
        b = P { v: 2 };
        // store-only: tuple of two struct refs, read both fields back.
        t = (a, b);
        print("{t.0.v},{t.1.v}\n");
        assert(t.0.v == 1 && t.1.v == 2, "e5_d1_struct_ref_local");
    }
    "#
);

cross_mode!(
    e5_d1_struct_ref_swap,
    r#"
    struct Point { x: integer, y: integer }
    fn two_points(a: Point, b: Point) -> (Point, Point) { (b, a) }
    fn test() {
        p1 = Point { x: 1, y: 2 };
        p2 = Point { x: 3, y: 4 };
        (q1, q2) = two_points(p1, p2);
        print("{q1.x},{q2.x}\n");
        assert(q1.x == 3 && q2.x == 1, "e5_d1_struct_ref_swap");
    }
    "#
);

cross_mode!(
    e5_d2_struct_ref_arg,
    r#"
    struct P { v: integer }
    fn show(t: (P, P)) {
        print("{t.0.v},{t.1.v}\n");
        assert(t.0.v == 10 && t.1.v == 20, "e5_d2_struct_ref_arg");
    }
    fn test() {
        a = P { v: 10 };
        b = P { v: 20 };
        show((a, b));
    }
    "#
);

cross_mode!(
    e5_d2_struct_ref_return,
    r#"
    struct P { v: integer }
    fn make_pair(a: P, b: P) -> (P, P) { (b, a) }
    fn test() {
        pa = P { v: 10 };
        pb = P { v: 20 };
        (q1, q2) = make_pair(pa, pb);
        print("{q1.v},{q2.v}\n");
        assert(q1.v == 20 && q2.v == 10, "e5_d2_struct_ref_return");
    }
    "#
);

cross_mode!(
    e5_d1_ref_int_local,
    r#"
    struct P { v: integer }
    fn test() {
        a = P { v: 7 };
        // mixed Reference + integer slot
        t = (a, 42);
        print("{t.0.v},{t.1}\n");
        assert(t.0.v == 7 && t.1 == 42, "e5_d1_ref_int_local");
    }
    "#
);

cross_mode!(
    e5_d1_ref_text_local,
    r#"
    struct P { v: integer }
    fn test() {
        a = P { v: 9 };
        // mixed Reference + owned text slot
        u = (a, "tag");
        print("{u.0.v},{u.1}\n");
        assert(u.0.v == 9 && u.1 == "tag", "e5_d1_ref_text_local");
    }
    "#
);
// P250 closed (2026-05-11): the loop-iteration destructure shape
// works on both backends.  Root cause was that the destructured
// LHS Reference vars (`q1`, `q2`) were emitted as independent
// owners, each with its own `OpFreeRef` at scope exit.  Both refs
// share the synthetic-tuple's `store_nr` (they're projections via
// `OpGetField`), so the first scope-exit free reclaimed the entire
// tuple store — leaving the outer-scope `tmp` variable holding a
// stale DbRef whose store_nr got recycled by the next iteration's
// `pa` allocation.  The second iteration's `tmp = make_pair(...)`
// reassignment then ran `OpFreeRef(tmp)` on that stale DbRef,
// silently destroying the freshly-allocated `pa`.  Fix: in
// `parser/expressions.rs`'s synthetic-struct destructure path,
// mark each Reference-typed LHS as `vars.depend(v_nr, tmp)` so
// scope analysis treats them as borrows and skips the per-LHS
// free.  `tmp`'s `OpFreeRef` alone reclaims the storage at the
// right time.
cross_mode!(
    e5_d1_struct_ref_loop,
    r#"
    struct P { v: integer }
    fn make_pair(a: P, b: P) -> (P, P) { (a, b) }
    fn test() {
        last_q1 = -1;
        last_q2 = -1;
        for i in 0..3 {
            pa = P { v: i };
            pb = P { v: i + 100 };
            (q1, q2) = make_pair(pa, pb);
            last_q1 = q1.v;
            last_q2 = q2.v;
        }
        assert(last_q1 == 2 && last_q2 == 102, "e5_d1_struct_ref_loop");
    }
    "#
);

// ── Phase 05 — destination 3 (tuples in struct fields) ───────────────────────
//
// **Decision: LIFT** (already shipped — Plan-06 phase 4d removed
// the `Type::Tuple` rejection in `parse_field`; tuple fields now
// lay out their elements inline using the synthetic `__tuple<…>`
// struct's positions).  Pre-flight pass on E1, E1n, E2, E3, E5
// shapes (read, whole-tuple update, single-element update, nested,
// (text, text), (Reference, Reference)) all green on both backends.
//
// E4 (closure-typed tuple element STORED in a struct field, then
// CALLED via `s.field.0(args)`) hits a separate fn-ref-via-
// struct-field-via-tuple-element dispatch bug — interp panics with
// `index out of bounds` in `database/allocation.rs:162`, native
// rejects with rustc E0308/E0605 from a tuple-vs-i64 cast.  Filed
// P251.  E4_d3 cell here is STORE-ONLY (mirrors the e4_d1_closure_local
// pattern) and locks the read side; the call shape lands as a
// follow-up cell when P251 closes.

cross_mode!(
    e1_d3_field_int_int,
    r#"
    struct S { t: (integer, integer) }
    fn test() {
        s = S { t: (3, 7) };
        print("{s.t.0},{s.t.1}\n");
        assert(s.t.0 == 3 && s.t.1 == 7, "e1_d3_field_int_int");
    }
    "#
);

cross_mode!(
    e1_d3_field_update,
    r#"
    struct S { t: (integer, integer) }
    fn test() {
        s = S { t: (3, 7) };
        // whole-tuple write to the field
        s.t = (10, 20);
        print("{s.t.0},{s.t.1}\n");
        assert(s.t.0 == 10 && s.t.1 == 20, "field whole-tuple update");
        // single-element write through the field
        s.t.0 = 99;
        print("{s.t.0},{s.t.1}\n");
        assert(s.t.0 == 99 && s.t.1 == 20, "field element update");
    }
    "#
);

cross_mode!(
    e1n_d3_field_intnotnull_bool,
    r#"
    struct S { t: (integer not null, boolean) }
    fn test() {
        s = S { t: (42, true) };
        print("{s.t.0},{s.t.1}\n");
        assert(s.t.0 == 42 && s.t.1, "e1n_d3_field_intnotnull_bool");
    }
    "#
);

cross_mode!(
    e2_d3_field_text_text,
    r#"
    struct S { t: (text, text) }
    fn test() {
        s = S { t: ("hello", "world") };
        print("{s.t.0},{s.t.1}\n");
        assert(s.t.0 == "hello" && s.t.1 == "world", "field read");
        // single-element text update
        s.t.0 = "X";
        print("{s.t.0},{s.t.1}\n");
        assert(s.t.0 == "X" && s.t.1 == "world", "field text element update");
    }
    "#
);

cross_mode!(
    e3_d3_field_nested,
    r#"
    struct S { t: ((integer, integer), boolean) }
    fn test() {
        s = S { t: ((1, 2), true) };
        print("{s.t.0.0},{s.t.0.1},{s.t.1}\n");
        assert(s.t.0.0 == 1 && s.t.0.1 == 2 && s.t.1, "e3_d3_field_nested");
    }
    "#
);

// P251 closed (2026-05-11): the codegen for storing a tuple
// containing a fn-ref element INTO a struct field now projects
// `.0` from the runtime `(u32, DbRef)` fn-ref tuple before the
// `OpSetInt4` cast.  Fix lands in
// `src/parser/mod.rs::emit_set_one_element` `Type::Function`
// arm — when the value is `Value::Var(v)` with `v` typed
// `Type::Function` (and not in the closure-vars table), wrap
// in `Value::FnRefDnr(v)` so native emit produces
// `(var_v.0 as i64)` instead of `(var_v) as i32`.  The direct
// fn-ref-struct-field-write path (`emit_fn_ref_field_write`)
// already did this; extending the tuple-element-of-struct-field
// path closed the remaining gap.  Both store-only and call-
// through-field shapes (`s.t.0(arg)` AND `f = s.t.0; f(arg)`)
// work on both backends.
cross_mode!(
    e4_d3_field_closure_local,
    r#"
    struct S { t: (fn(integer) -> integer, integer) }
    fn test() {
        add5 = fn(x: integer) -> integer { x + 5 };
        s = S { t: (add5, 99) };
        // Store-only check (the original P251 surface symptom):
        print("{s.t.1}\n");
        assert(s.t.1 == 99, "e4_d3_field_closure_local store");
        // Call-through-field check (compounding shape from P251):
        r1 = s.t.0(10);
        assert(r1 == 15, "e4_d3_field_closure_local call");
    }
    "#
);

cross_mode!(
    e5_d3_field_struct_ref,
    r#"
    struct P { v: integer }
    struct Pair { t: (P, P) }
    fn test() {
        a = P { v: 10 };
        b = P { v: 20 };
        pair = Pair { t: (a, b) };
        print("{pair.t.0.v},{pair.t.1.v}\n");
        assert(pair.t.0.v == 10 && pair.t.1.v == 20, "e5_d3_field_struct_ref");
    }
    "#
);

// ── #395 — generic function instantiated over a TUPLE element type ───────────
//
// A generic template defers the tuple-return rewrite ("T resolves later");
// `try_generic_instantiation` must re-apply it so the monomorph returns
// `Reference(__tuple)`, not a bare `Tuple`.  Before the fix: interp read the
// return DbRef inline as garbage (`34359738371 2`), and native either failed
// to compile (E0308) or emitted an invalid identifier `t_..__tuple<int,int>_..`.
// The `assert`s make this a real correctness guard, not just interp==native.

cross_mode!(
    p395_generic_over_tuple_pair,
    r#"
    fn first<T>(v: vector<T>) -> T { v[0] }
    fn test() {
        v: vector<(integer, integer)> = [(3, 4), (5, 6)];
        t = first(v);
        print("{t.0} {t.1}\n");
        assert(t.0 == 3 && t.1 == 4, "p395 generic over (integer, integer)");
    }
    "#
);

cross_mode!(
    p395_generic_over_tuple_triple,
    r#"
    fn first<T>(v: vector<T>) -> T { v[0] }
    fn test() {
        v: vector<(integer, integer, integer)> = [(3, 4, 5)];
        t = first(v);
        print("{t.0} {t.1} {t.2}\n");
        assert(t.0 == 3 && t.1 == 4 && t.2 == 5, "p395 generic over 3-tuple");
    }
    "#
);

cross_mode!(
    p395_generic_over_tuple_text_mixed,
    r#"
    fn first<T>(v: vector<T>) -> T { v[0] }
    fn test() {
        v: vector<(text, integer)> = [("ab", 7)];
        t = first(v);
        print("{t.0} {t.1}\n");
        assert(t.0 == "ab" && t.1 == 7, "p395 generic over (text, integer)");
    }
    "#
);
