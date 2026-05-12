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
