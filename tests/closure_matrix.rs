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
//! **These cells RUN by default** (@PLN114).  They were `#[ignore]`d as too heavy;
//! measured, the whole 201-cell matrix takes ~10 seconds.  The cost was never the
//! issue — the silence was: two SIGSEGVs, a silent `+1` corruption of every narrow
//! tuple element, a 3.4x memory overhead and a cross-backend par-worker corruption
//! all sat behind that attribute until the set was run by hand.
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

// ── Phase 01 — broaden C0 (non-capturing) across D2 / D3 / D4 ──────────────
//
// All four cells exercise non-capturing lambdas (no surrounding-
// scope reads).  Each destination shape stresses a different
// emission path:
//   D2/arg     — fn-ref passed by value to a function parameter
//   D2/inline  — IIFE: (fn(...) {...})(args), no intermediate var
//   D3/field   — struct field of fn-ref type, instantiated and called
//   D4/vector  — vector<fn(integer) -> integer> literal + index-call
//                (the historical P214 surface — closed 2026-05-05)

cross_mode!(
    c0_d2_non_cap_arg,
    r#"
    fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
    fn test() {
        result = apply(fn(n: integer) -> integer { n * n }, 7);
        print("{result}\n");
        assert(result == 49, "c0_d2_non_cap_arg");
    }
    "#
);

cross_mode!(
    c0_d2_non_cap_inline,
    r#"
    fn test() {
        result = (fn(x: integer) -> integer { x + 1 })(41);
        print("{result}\n");
        assert(result == 42, "c0_d2_non_cap_inline");
    }
    "#
);

cross_mode!(
    c0_d3_non_cap_field,
    r#"
    struct Holder { cb: fn(integer) -> integer }
    fn dbl(x: integer) -> integer { x + x }
    fn triple(x: integer) -> integer { x * 3 }
    fn test() {
        h1 = Holder { cb: dbl };
        h2 = Holder { cb: triple };
        a = h1.cb(10);
        b = h2.cb(10);
        print("{a},{b}\n");
        assert(a == 20, "h1.cb(10)={a}");
        assert(b == 30, "h2.cb(10)={b}");
    }
    "#
);

// @P299 — calling a CAPTURING closure stored in a struct field (the
// call-through-field dispatch) panicked on native (`invalid fn-ref`)
// because the lambda's d_nr, written as `OpSetInt4(field, pos, Int(d))`,
// was invisible to the reachability walk and the lambda was dropped from
// the fn-ref candidate set.  Closed 2026-05-21.  These cells pin the two
// shapes the int/mutation D3 cases in mut_closure_matrix don't cover:
// a struct-by-reference capture, and a text-RETURNING capture consumed
// DIRECTLY as a call argument (which also needed the `&*` text-arg
// coercion in calls.rs).
cross_mode!(
    p299_d3_capture_ref_field_call,
    r#"
    struct Point { x: integer, y: integer }
    struct Holder { cb: fn(integer) -> integer }
    fn test() {
        p = Point { x: 100, y: 0 };
        h = Holder { cb: fn(dx: integer) -> integer { p.x + dx } };
        r = h.cb(7);
        print("{r}\n");
        assert(r == 107, "h.cb(7)={r}");
    }
    "#
);

cross_mode!(
    p299_d3_capture_text_return_field_call,
    r#"
    struct G { fmt: fn(integer) -> text }
    fn test() {
        label = "z";
        g = G { fmt: fn(n: integer) -> text { "{label}: {n}" } };
        print(g.fmt(7));
        print("\n");
        assert(g.fmt(7) == "z: 7", "g.fmt(7)");
    }
    "#
);

cross_mode!(
    c0_d4_non_cap_vector,
    r#"
    fn test() {
        v: vector<fn(integer) -> integer> = [
            fn(x: integer) -> integer { x + 1 },
            fn(x: integer) -> integer { x * 2 },
        ];
        a = v[0](10);
        b = v[1](5);
        print("{a},{b}\n");
        assert(a == 11, "v[0](10)={a}");
        assert(b == 10, "v[1](5)={b}");
    }
    "#
);

// ── Phase 02 — basic-type captures (C1 single, C5 multi-basic) ─────────────
//
// C1: single integer capture (`n = 5; |x| { x + n }`).
// C5: multi-basic capture (n + b + factor — int + bool + int).  Text
//     and Reference captures stay out of phase 02; they land in
//     phase 03 (text — active LIFETIME.md leak risk) and phase 04
//     (Reference) respectively.
//
// Every capturing-closure shape exercises the closure-record
// allocation path (the 16-byte fn-ref slot + the closure DbRef).
// P213 closed the struct-field capture surface 2026-05-04 with
// `Parts::ChildRec` layout-widening; phase 02 pins the regression
// guard for that arc plus the simpler D1/D2 shapes.
//
// D4 (vector element) stays CLOSED for capturing closures per
// the loft-write skill restriction — separate cells in the
// CLOSED-cell sweep at the end of the binary.

cross_mode!(
    c1_d1_int_capture_local,
    r#"
    fn test() {
        n = 5;
        f = fn(x: integer) -> integer { x + n };
        result = f(10);
        print("{result}\n");
        assert(result == 15, "c1_d1_int_capture_local");
    }
    "#
);

cross_mode!(
    c1_d2_int_capture_arg,
    r#"
    fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
    fn test() {
        n = 5;
        result = apply(fn(x: integer) -> integer { x + n }, 10);
        print("{result}\n");
        assert(result == 15, "c1_d2_int_capture_arg");
    }
    "#
);

cross_mode!(
    c1_d3_int_capture_field,
    r#"
    struct Box { cb: fn(integer) -> integer }
    fn test() {
        n = 5;
        b = Box { cb: fn(x: integer) -> integer { x + n } };
        result = b.cb(10);
        print("{result}\n");
        assert(result == 15, "c1_d3_int_capture_field");
    }
    "#
);

// #313 — the definition-ORDER axis the original D3 cells missed: the
// invoking function is defined (and second-pass parsed) BEFORE the
// function that constructs the struct with the capturing closure.
// Pre-fix, `assigned_lambda_d_nr` re-accumulated in body-parse order
// after the inter-pass `data.reset()`, so the earlier-parsed reader
// baked in the legacy 4B field shape (closure half = null sentinel)
// → interp SIGSEGV / native OOB u16::MAX.  Covers both the
// local-copy (`c = k.cb; c(p)`) and direct (`k.cb(p)`) call shapes.
cross_mode!(
    c1_d3_int_capture_field_cross_fn_invoker_first,
    r#"
    struct Counter { n: integer not null }
    struct K { cb: fn(text) }
    fn fire(k: K, p: text) { c = k.cb; c(p); }
    fn fire_direct(k: K, p: text) { k.cb(p); }
    fn test() {
        w = Counter { n: 0 };
        k = K { cb: fn(p: text) { w.n = w.n + 1; print("got {p} n={w.n}\n"); } };
        fire(k, "a");
        fire_direct(k, "b");
        assert(w.n == 2, "after two fires w.n={w.n}");
    }
    "#
);

// #313 sibling symptom — a TEXT capture on the same invoker-first
// shape failed differently pre-fix (allocation.rs OOB panic while
// claiming the closure record into the host's store).
cross_mode!(
    c2_d3_text_capture_field_cross_fn_invoker_first,
    r#"
    struct K { cb: fn() }
    fn fire(k: K) { c = k.cb; c(); }
    fn test() {
        msg = "hello";
        k = K { cb: fn() { print("said {msg}\n"); } };
        fire(k);
    }
    "#
);

// #323 — factory-returned closure with a Reference capture, under
// allocation pressure.  The capture's store must survive the factory's
// return (owned by the closure record's cascade, not the dead frame);
// pre-fix both backends corrupted whichever spam object reused the slot
// (interp only looked sound without pressure).
cross_mode!(
    c3_d4_reference_capture_factory_reuse_pressure,
    r#"
    struct Counter { n: integer not null }
    fn make() -> fn() -> integer {
        w = Counter { n: 7 };
        fn() -> integer { w.n = w.n + 1; w.n }
    }
    fn test() {
        f = make();
        spam1 = Counter { n: 31337 };
        spam2 = Counter { n: 41414 };
        a = f();
        b = f();
        print("{a},{b},{spam1.n},{spam2.n}\n");
        assert(a == 8 && b == 9, "factory capture: {a},{b}");
        assert(spam1.n == 31337 && spam2.n == 41414, "spam corrupted: {spam1.n},{spam2.n}");
    }
    "#
);

cross_mode!(
    c5_d1_multi_capture_local,
    r#"
    fn test() {
        n = 5;
        flag = true;
        factor = 3;
        f = fn(x: integer) -> integer {
            if flag { (x + n) * factor } else { x }
        };
        a = f(10);
        b = f(2);
        print("{a},{b}\n");
        assert(a == 45, "c5_d1 a={a}");
        assert(b == 21, "c5_d1 b={b}");
    }
    "#
);

cross_mode!(
    c5_d2_multi_capture_arg,
    r#"
    fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
    fn test() {
        base = 100;
        factor = 3;
        result = apply(fn(n: integer) -> integer { base + n * factor }, 7);
        print("{result}\n");
        assert(result == 121, "c5_d2 result={result}");
    }
    "#
);

cross_mode!(
    c5_d3_multi_capture_field,
    r#"
    struct Acc { add: fn(integer) -> integer }
    fn test() {
        base = 100;
        factor = 3;
        a = Acc { add: fn(n: integer) -> integer { base + n * factor } };
        result = a.add(7);
        print("{result}\n");
        assert(result == 121, "c5_d3 result={result}");
    }
    "#
);

// ── Phase 03 — text captures (C2) — the active LIFETIME risk ──────────────
//
// P227 closed text-returning fn-ref calls 2026-05-05 (interp + native)
// — three coordinated changes (parser work-buffer count, RefVar(Text)
// hidden attribute, native dispatch wrapper).  Phase 03 pins the
// regression guard for that arc across D1/D2/D3.
//
// LIFETIME risk: LIFETIME.md flags `Type::Function` as "NOT YET
// HANDLED" — the closure DbRef at offset+4 of the 16-byte fn-ref
// slot is "never explicitly freed."  For text captures this matters
// because the captured text lives in the closure record's heap
// allocation.  If the closure record never gets freed, the captured
// text never gets freed either → store leak.
//
// Phase 03 decision (2026-05-12 — recorded after running the
// cells under `tests/leak.rs::p15_phase03_closure_text_capture_*_no_leak`):
// **the leak does NOT manifest** in any of the C2 destinations
// pinned here.  D1 (local var) frees the closure record at
// stack-frame exit via the standard local-cleanup path; D3
// (struct field) frees it via P213's `Parts::ChildRec` cascade
// when the host struct goes out of scope.  Both confirmed clean
// over a 100-iteration tight loop with `state.check_store_leaks()`.
//
// The LIFETIME.md "NOT YET HANDLED" annotation overstates the
// gap for these shapes.  Phase 06 should update LIFETIME.md to
// reflect the actual freed-at-scope-exit behaviour.  No P-issue
// filed — the gap is documentation drift, not a runtime bug.

cross_mode!(
    c2_d1_text_capture_local,
    r#"
    fn test() {
        label = "tag";
        f: fn(integer) -> text = fn(n: integer) -> text { "{label}: {n}" };
        result = f(42);
        print("{result}\n");
        assert(result == "tag: 42", "c2_d1 result='{result}'");
    }
    "#
);

cross_mode!(
    c2_d2_text_capture_arg,
    r#"
    fn apply(f: fn(integer) -> text, x: integer) -> text { f(x) }
    fn test() {
        label = "tag";
        result = apply(fn(n: integer) -> text { "{label}: {n}" }, 42);
        print("{result}\n");
        assert(result == "tag: 42", "c2_d2 result='{result}'");
    }
    "#
);

cross_mode!(
    c2_d3_text_capture_field,
    r#"
    struct G { fmt: fn(integer) -> text }
    fn test() {
        label = "z";
        g = G { fmt: fn(n: integer) -> text { "{label}: {n}" } };
        result = g.fmt(7);
        print("{result}\n");
        assert(result == "z: 7", "c2_d3 result='{result}'");
    }
    "#
);

// ── Phase 04 — Reference captures (C3) across D1 / D2 / D3 ─────────────────
//
// C3 captures a struct (DbRef-allocated) into a closure body that
// reads field(s) from it.  The closure record holds the captured
// DbRef; the captured struct must outlive the closure record.
//
// Risk per the plan: DbRef-in-closure-record dep tracking; the
// "move-vs-copy semantics gap analogous to plan-14 T1.8c" (where
// a Reference variable assigned into a tuple's slot was
// inadvertently freed by the source's scope-exit cleanup).  If
// that gap exists for closures, the captured Point would be
// freed on the outer scope's exit while the closure still holds
// its DbRef — read-after-free.
//
// Phase 04 finding (2026-05-12): no read-after-free observed.
// The dep mechanism in `vectors.rs:666-669` (Type::Function
// carries closure-record dep `[w]`) plus `Parts::ChildRec`
// cascade for D3 ensures the captured Point's store record
// stays live until the closure record is freed.
// `tests/leak.rs::p15_phase04_closure_ref_capture_*_no_leak`
// adds 100-iteration tight-loop guards for D1 + D3.

cross_mode!(
    c3_d1_ref_capture_local,
    r#"
    struct Point { x: integer, y: integer }
    fn test() {
        p = Point { x: 10, y: 20 };
        f = fn(dx: integer) -> integer { p.x + dx };
        result = f(5);
        print("{result}\n");
        assert(result == 15, "c3_d1 result={result}");
    }
    "#
);

cross_mode!(
    c3_d2_ref_capture_arg,
    r#"
    struct Point { x: integer, y: integer }
    fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
    fn test() {
        p = Point { x: 50, y: 0 };
        result = apply(fn(dx: integer) -> integer { p.x + dx }, 5);
        print("{result}\n");
        assert(result == 55, "c3_d2 result={result}");
    }
    "#
);

cross_mode!(
    c3_d3_ref_capture_field,
    r#"
    struct Point { x: integer, y: integer }
    struct Holder { cb: fn(integer) -> integer }
    fn test() {
        p = Point { x: 100, y: 0 };
        h = Holder { cb: fn(dx: integer) -> integer { p.x + dx } };
        result = h.cb(7);
        print("{result}\n");
        assert(result == 107, "c3_d3 result={result}");
    }
    "#
);

// ── Phase 05 — nested closures (C6) across D1 / D2 / D3 ────────────────────
//
// C6 captures a fn-ref local into the closure body of another
// lambda.  P215 closed the nested-closure name resolution surface
// 2026-05-05 (the inner lambda's d_nr + closure DbRef are stored
// in the outer closure record's slot).
//
// Constraint: the INNER lambda must itself be non-capturing.
// Capturing-source-into-closure remains deferred (would need
// `synthesize_closure_record` to register the 8B split layout
// when the source itself captures).  This matches the matrix's
// C6 row (which uses non-capturing inner lambdas).
//
// The matrix flagged D3 as "deferred — depends on C3 dep
// propagation."  Phase 04 confirmed C3 dep propagation works,
// so D3 is included here rather than deferred to a future phase.
// Matrix doc updated to reflect the un-defer.

cross_mode!(
    c6_d1_nested_closure_local,
    r#"
    fn test() {
        inner = fn(x: integer) -> integer { x + 5 };
        outer = fn(y: integer) -> integer { inner(y) + 1 };
        result = outer(10);
        print("{result}\n");
        assert(result == 16, "c6_d1 result={result}");
    }
    "#
);

cross_mode!(
    c6_d2_nested_closure_arg,
    r#"
    fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
    fn test() {
        inner = fn(x: integer) -> integer { x + 5 };
        result = apply(fn(y: integer) -> integer { inner(y) + 1 }, 10);
        print("{result}\n");
        assert(result == 16, "c6_d2 result={result}");
    }
    "#
);

cross_mode!(
    c6_d3_nested_closure_field,
    r#"
    struct Holder { cb: fn(integer) -> integer }
    fn test() {
        inner = fn(x: integer) -> integer { x + 5 };
        h = Holder { cb: fn(y: integer) -> integer { inner(y) + 1 } };
        result = h.cb(10);
        print("{result}\n");
        assert(result == 16, "c6_d3 result={result}");
    }
    "#
);
