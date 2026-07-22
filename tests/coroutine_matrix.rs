// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN16 coroutine-validation matrix.
//!
//! Each test exercises one cell of the (yielded-value-type × drive-
//! context) matrix described in
//! `doc/claude/plans/16-coroutine-validation/00-matrix.md`.  Every cell
//! runs under both the interpreter and `--native`; the harness in
//! `tests/common/cross_mode.rs` (shared with `tuple_matrix`,
//! `closure_matrix`, `template_matrix`) asserts byte-identical stdout.
//!
//! **These cells RUN by default** (@PLN114).  They were `#[ignore]`d as too heavy;
//! measured, the whole 201-cell matrix takes ~10 seconds.  The cost was never the
//! issue — the silence was: two SIGSEGVs, a silent `+1` corruption of every narrow
//! tuple element, a 3.4x memory overhead and a cross-backend par-worker corruption
//! all sat behind that attribute until the set was run by hand.
//!
//! ```bash
//! cargo test --release --test coroutine_matrix -- --ignored
//! ```
//!
//! or a single cell:
//!
//! ```bash
//! cargo test --release --test coroutine_matrix -- --ignored y1_x1_int_for_loop
//! ```
//!
//! ## The frozen matrix (phase 00, locked 2026-05-23)
//!
//! Cell legend: `PASS:test_name` / `FIX:phase` / `CLOSED:reason`.
//!
//! | | X1 — for-loop | X2 — manual `next()` | X3 — higher-order | X4 — comprehension | X5 — yield from |
//! |---|---|---|---|---|---|
//! | **Y1** scalar    | FIX:01 | FIX:01 | FIX:01 | FIX:01 | CLOSED:CO1.4-deferred |
//! | **Y2** text      | FIX:02 | FIX:02 | FIX:02 | FIX:02 | CLOSED:CO1.4-deferred |
//! | **Y3** Reference | FIX:03 | FIX:03 | FIX:03 | FIX:03 | CLOSED:CO1.4-deferred |
//! | **Y4** tuple     | FIX:04 | FIX:04 (gated on T1.8a) | FIX:04 | FIX:04 | CLOSED:CO1.4-deferred |
//! | **Y5** closure   | FIX:05 (depends on @PLAN15) | FIX:05 | FIX:05 | FIX:05 | CLOSED:CO1.4-deferred |
//! | **Y6** vector    | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:CO1.4-deferred |
//! | **Y7** float     | PASS:y7_x1 | PASS:y7_x2 | ok¹ | ok¹ | CLOSED:CO1.4-deferred |
//! | **Y8** single    | PASS:y8_x1 | PASS:y8_x2 | ok¹ | ok¹ | CLOSED:CO1.4-deferred |
//! | **Y9** enum      | PASS:y9_x1 | PASS:y9_x2 | ok¹ | ok¹ | CLOSED:CO1.4-deferred |
//!
//! ¹ float/single/enum × X3 (higher-order) and X4 (comprehension) verified on
//!   both backends via the full type×context grid (2026-06-18, #401); no
//!   dedicated `cross_mode` cell yet.
//!
//! Each `body` must declare `fn test() { … }` (the entry point) and
//! any helper fns at file scope; the harness appends
//! `fn main() { test(); }`.  loft does not allow nested fn
//! definitions, so helpers must live alongside `fn test`.

mod common;

// ── Phase 00 — harness smoke ────────────────────────────────────────────────
//
// One smoke cell that exercises the cross-mode harness with the
// simplest yielded-scalar / for-loop combination — the cheapest path
// through the coroutine state machine.  If this cell ever breaks,
// every other coroutine cell below should be treated as
// "infrastructure regression, not coroutine regression" until the
// smoke is green again.

cross_mode!(
    y1_x1_int_for_loop_smoke,
    r#"
    fn count_up() -> iterator<integer> {
        i = 0;
        while i < 3 {
            yield i;
            i = i + 1;
        }
    }
    fn test() {
        sum = 0;
        for v in count_up() {
            sum = sum + v;
        }
        print("{sum}\n");
        assert(sum == 3, "y1_x1 for-loop sum");
    }
    "#
);

// ── Phase 01 — Y1 (basic scalars) × X1–X4 ──────────────────────────────────

// Y1×X1 — for-loop driving an integer generator that uses `for v in [...]`
// internally.  Covers the P219 path (for-yield inside a generator) end-to-end.
cross_mode!(
    y1_x1_int_for_yield_in_generator,
    r#"
    fn nums() -> iterator<integer> {
        for n in [10, 20, 30] {
            yield n;
        }
    }
    fn test() {
        sum = 0;
        for v in nums() {
            sum = sum + v;
        }
        print("{sum}\n");
        assert(sum == 60, "y1_x1 for-yield sum");
    }
    "#
);

// Y1×X1 — generator whose body calls a HELPER that yields.  Stackful
// semantics: the suspended frame must capture the helper's local state.
cross_mode!(
    y1_x1_int_generator_with_helper_yield,
    r#"
    fn inner() -> iterator<integer> {
        yield 10;
        yield 20;
    }
    fn outer() -> iterator<integer> {
        yield 1;
        for n in inner() {
            yield n;
        }
        yield 2;
    }
    fn test() {
        sum = 0;
        for v in outer() {
            sum = sum + v;
        }
        print("{sum}\n");
        assert(sum == 33, "y1_x1 helper-yield sum");
    }
    "#
);

// Y1×X2 — manual `next()` calls.  Each next() returns one value; after
// all yields plus one EXTRA call (to advance past the last yield),
// exhausted() returns true.  Matches the contract used in
// `tests/scripts/51-coroutines.loft::one_val + 2× next`: a 3-yield
// generator needs 4 next() calls before exhausted() flips.
cross_mode!(
    y1_x2_int_manual_next,
    r#"
    fn count() -> iterator<integer> {
        yield 10;
        yield 20;
        yield 30;
    }
    fn test() {
        it = count();
        a = next(it);
        b = next(it);
        c = next(it);
        next(it);
        done = exhausted(it);
        print("{a},{b},{c},{done}\n");
        assert(a == 10 && b == 20 && c == 30, "y1_x2 values");
        assert(done, "y1_x2 exhausted after extra next");
    }
    "#
);

// Y1×X3 — higher-order: pump generator output through a closure.  The
// idiomatic equivalent of `map(gen, fn)` is a for-loop that calls the
// closure on each yielded value and accumulates the result.
//
// **@P324 FIXED 2026-05-23** — the CO1.9/S28 over-broad guard was
// demoted to a debug-only warning so the for-loop+accumulate idiom no
// longer panics on interp.  Native was always fine.
cross_mode!(
    y1_x3_int_closure_map,
    r#"
    fn count() -> iterator<integer> {
        yield 1;
        yield 2;
        yield 3;
    }
    fn test() {
        f = fn(x: integer) -> integer { x * 10 };
        out: vector<integer> = [];
        for v in count() {
            out += [f(v)];
        }
        print("{out[0]},{out[1]},{out[2]}\n");
        assert(out[0] == 10 && out[1] == 20 && out[2] == 30, "y1_x3 closure mapped");
    }
    "#
);

// Y1×X3 — reduce shape: closure accumulates iterator output.
cross_mode!(
    y1_x3_int_closure_reduce,
    r#"
    fn count() -> iterator<integer> {
        yield 1;
        yield 2;
        yield 3;
        yield 4;
    }
    fn test() {
        add = fn(a: integer, b: integer) -> integer { a + b };
        acc = 0;
        for v in count() {
            acc = add(acc, v);
        }
        print("{acc}\n");
        assert(acc == 10, "y1_x3 reduce sum");
    }
    "#
);

// ── Phase 03 — Y3 (References) × X1–X4 ─────────────────────────────────────
//
// Y3 yields DbRef-bearing struct records.  The driving lifetime risk:
// each yielded struct lives in a STORE that the generator allocated; the
// consumer reads its fields between yields.  S28 guard interplay
// (@P324) is the active hazard.

// Y3×X1 — for-loop over `iterator<CY3Pt>`.  Each yield emits a fresh
// struct record; the consumer reads `pt.x + pt.y` between yields.
//
// **@P326 FIXED 2026-05-23** — native state machine now has a DbRef
// channel: `LoftCoroutine::next_dbref` + `coroutine_next_dbref` runtime
// helper + size=12 dispatch arm + struct-construction work-var
// pre-declaration (`__ref_*` joins `__vdb_*` / `__work_*`).
cross_mode!(
    y3_x1_ref_for_loop,
    r#"
    struct CY3Pt { x: integer not null, y: integer not null }
    fn points() -> iterator<CY3Pt> {
        yield CY3Pt { x: 1, y: 2 };
        yield CY3Pt { x: 3, y: 4 };
        yield CY3Pt { x: 5, y: 6 };
    }
    fn test() {
        sum = 0;
        for pt in points() {
            sum = sum + pt.x + pt.y;
        }
        print("{sum}\n");
        assert(sum == 21, "y3_x1 ref-yield field-sum");
    }
    "#
);

// ── Phase 04 — Y4 (tuples) × X1–X4 ─────────────────────────────────────────
//
// Tuple yields stress the state machine's value-marshalling: each yielded
// tuple is multi-component and must be reconstructed on the consumer side.

// Y4×X1 — for-loop over `iterator<(integer, integer)>`.
//
// **@P327 FIXED 2026-05-23 (both backends).**  Interp side: for-loop
// uses `OpCoroutineExhausted` (via `iter_for` tuple-coroutine branch).
// Native side: unified `next_into(stores, &mut [i64])` channel from
// plan-16 phase 01 — `value_size`'s high byte distinguishes tuple
// yields from text (both 16 bytes); the consumer allocates a stack
// `[i64; N]` buffer and rebuilds the tuple from buffer slots.
cross_mode!(
    y4_x1_tuple_for_loop,
    r#"
    fn pairs() -> iterator<(integer, integer)> {
        yield (1, 10);
        yield (2, 20);
        yield (3, 30);
    }
    fn test() {
        sum = 0;
        for p in pairs() {
            sum = sum + p.0 + p.1;
        }
        print("{sum}\n");
        assert(sum == 66, "y4_x1 tuple for-loop sum");
    }
    "#
);

// Y4×X2 — manual next() on a tuple-yielding generator.
cross_mode!(
    y4_x2_tuple_manual_next,
    r#"
    fn pairs() -> iterator<(integer, integer)> {
        yield (10, 20);
        yield (30, 40);
    }
    fn test() {
        it = pairs();
        a = next(it);
        b = next(it);
        print("{a.0},{a.1},{b.0},{b.1}\n");
        assert(a.0 == 10 && a.1 == 20, "y4_x2 first");
        assert(b.0 == 30 && b.1 == 40, "y4_x2 second");
    }
    "#
);

// Y3×X2 — manual next() on a struct-yielding generator.  @P326 FIXED.
cross_mode!(
    y3_x2_ref_manual_next,
    r#"
    struct CY3Pt { x: integer not null, y: integer not null }
    fn points() -> iterator<CY3Pt> {
        yield CY3Pt { x: 10, y: 20 };
        yield CY3Pt { x: 30, y: 40 };
    }
    fn test() {
        it = points();
        a = next(it);
        b = next(it);
        print("{a.x},{a.y},{b.x},{b.y}\n");
        assert(a.x == 10 && a.y == 20, "y3_x2 first");
        assert(b.x == 30 && b.y == 40, "y3_x2 second");
    }
    "#
);

// ── Phase 02 — Y2 (text) × X1–X4 ───────────────────────────────────────────
//
// Active risk per @PLAN16 README: yielded text's backing String lives on
// the suspended frame, not on the consumer's stack.  Each cell here pins
// the lifetime contract on one drive context.

// Y2×X1 — for-loop over a text generator.  The simplest text-yield shape:
// each `yield "literal"` produces a static-text element; the for-loop
// consumes them in order.  Closed @P211 pinned this in `tests/issues.rs`;
// this cell is the matrix-shaped regression carrier.
cross_mode!(
    y2_x1_text_for_loop,
    r#"
    fn names() -> iterator<text> {
        yield "alice";
        yield "bob";
        yield "carol";
    }
    fn test() {
        joined = "";
        for s in names() {
            joined = joined + s + ",";
        }
        print("{joined}\n");
        assert(joined == "alice,bob,carol,", "y2_x1 text concat");
    }
    "#
);

// Y2×X1 — text yielded by a `while` loop with a captured parameter.
// Format-string interpolation reads the parameter on each yield.  Closed
// @P218 pinned the work-buffer scoping; this cell is the matrix carrier.
cross_mode!(
    y2_x1_text_format_with_param,
    r#"
    fn make_labels(label: text, n: integer) -> iterator<text> {
        i = 0;
        while i < n {
            yield "{label}={i}";
            i = i + 1;
        }
    }
    fn test() {
        total = 0;
        for s in make_labels("x", 3) {
            total = total + s.len();
        }
        print("{total}\n");
        assert(total == "x=0".len() + "x=1".len() + "x=2".len(), "y2_x1 format-with-param len");
    }
    "#
);

// Y2×X2 — manual `next()` on a text generator.
cross_mode!(
    y2_x2_text_manual_next,
    r#"
    fn names() -> iterator<text> {
        yield "alice";
        yield "bob";
    }
    fn test() {
        it = names();
        a = next(it);
        b = next(it);
        next(it);
        done = exhausted(it);
        print("{a},{b},{done}\n");
        assert(a == "alice", "y2_x2 first");
        assert(b == "bob", "y2_x2 second");
        assert(done, "y2_x2 exhausted");
    }
    "#
);

// Y2×X3 — text yielded into a closure-driven map.  Builds a single text
// accumulator (no store mutation between yields once the closure is
// applied — the `+ s` concat returns a new String each iteration).
// NOTE: this cell may trip @P324 too if the text concat backing store
// is mutated between yields — file the divergence if it surfaces.
cross_mode!(
    y2_x3_text_closure_chain,
    r#"
    fn names() -> iterator<text> {
        yield "a";
        yield "b";
        yield "c";
    }
    fn test() {
        wrap = fn(s: text) -> text { "[" + s + "]" };
        acc = "";
        for s in names() {
            acc = acc + wrap(s);
        }
        print("{acc}\n");
        assert(acc == "[a][b][c]", "y2_x3 closure-chain");
    }
    "#
);

// Y2×X4 — text comprehension `[for s in gen() { transform(s) }]`.
// **@P325 FIXED 2026-05-23** (same shape as Y1×X4 above).
cross_mode!(
    y2_x4_text_comprehension,
    r#"
    fn names() -> iterator<text> {
        yield "alice";
        yield "bob";
    }
    fn test() {
        out = [for s in names() { "<" + s + ">" }];
        print("{out[0]},{out[1]}\n");
        assert(out[0] == "<alice>" && out[1] == "<bob>", "y2_x4 text comprehension");
    }
    "#
);

// Y1×X4 — comprehension `[for v in gen() { expr }]` over a generator.
// Builds a vector from the iterator's elements via loft's comprehension
// syntax (LOFT.md § Comprehensions).
//
// **@P325 FIXED 2026-05-23** — comprehensions over coroutines now emit
// an `OpCoroutineExhausted(__gen_N)` break check (mirror of the @P327
// fix for the for-loop driver).  Without this, the loop ran forever
// appending until the store overflowed its 2 GiB word limit
// (interp panic via `store.rs:643`; native = same path).
cross_mode!(
    y1_x4_int_comprehension,
    r#"
    fn count() -> iterator<integer> {
        yield 1;
        yield 2;
        yield 3;
    }
    fn test() {
        out = [for v in count() { v * 100 }];
        print("{out[0]},{out[1]},{out[2]}\n");
        assert(out[0] == 100 && out[1] == 200 && out[2] == 300, "y1_x4 comprehension");
    }
    "#
);

// ── Phase 05 — Y5 (closures) × X1–X2 ───────────────────────────────────────
//
// @P328 — closure-yielding generators: native via the unified `next_into`
// channel (tag 2 = `(u32, DbRef)` rebuild); interp via the fn-ref-yield
// rewrite at parse time (bare `Value::Int(d_nr)` → `Value::FnRef(d, MAX, _)`
// + null-DbRef sentinel emit when `clos_var == u16::MAX`).

cross_mode!(
    y5_x1_closure_for_loop_noncapturing,
    r#"
    fn fns() -> iterator<fn(integer) -> integer> {
        yield fn(x: integer) -> integer { x * 10 };
        yield fn(x: integer) -> integer { x + 100 };
    }
    fn test() {
        total = 0;
        for f in fns() {
            total = total + f(7);
        }
        print("{total}\n");
        assert(total == 177, "y5_x1 closure for-loop (non-cap)");
    }
    "#
);

cross_mode!(
    y5_x2_closure_manual_next_capturing,
    r#"
    fn fns(base: integer) -> iterator<fn(integer) -> integer> {
        yield fn(x: integer) -> integer { x + base };
    }
    fn test() {
        it = fns(100);
        f = next(it);
        result = f(5);
        print("{result}\n");
        assert(result == 105, "y5_x2 closure manual next (capturing)");
    }
    "#
);

// ── #401 — Y7/Y8/Y9 (float / single / enum) × X1 ──────────────────────────
//
// These element types were absent from the frozen matrix above.  Their null
// sentinel (NaN for float/single, the enum sentinel) differs from the i64
// transport's exhaustion sentinel, so the value-sentinel termination path
// (`convert(value, Boolean)` → `OpNot`) never fired: the interp codegen of the
// doomed check spun forever (12+ min hang), and native mis-typed the i64
// channel as f64 (E0308).  Fixed by routing EVERY coroutine loop through the
// state-based `OpCoroutineExhausted`, plus tagged native value channels
// (3 = f64::from_bits, 4 = f32::from_bits, 5 = enum-as-uN) with matching
// producer `to_bits` writes.  See gh #401.

cross_mode!(
    y7_x1_float_for_loop,
    r#"
    fn vals() -> iterator<float> { yield 1.5; yield 2.5; yield 3.0; }
    fn test() {
        sum = 0.0;
        for v in vals() { sum = sum + v; }
        print("{sum}\n");
        assert(sum == 7.0, "y7_x1 float for-loop sum");
    }
    "#
);

cross_mode!(
    y8_x1_single_for_loop,
    r#"
    fn vals() -> iterator<single> { yield 1.5 as single; yield 2.5 as single; }
    fn test() {
        sum = 0.0 as single;
        for v in vals() { sum = sum + v; }
        print("{sum}\n");
        assert(sum == (4.0 as single), "y8_x1 single for-loop sum");
    }
    "#
);

cross_mode!(
    y9_x1_enum_for_loop,
    r#"
    enum Color { Red, Green, Blue }
    fn vals() -> iterator<Color> { yield Color.Red; yield Color.Blue; }
    fn test() {
        count = 0;
        last = Color.Red;
        for v in vals() { count = count + 1; last = v; }
        print("{count}\n");
        assert(count == 2, "y9_x1 enum for-loop count");
        assert(last == Color.Blue, "y9_x1 enum for-loop last value");
    }
    "#
);

// X2 — manual `next()`.  #401 follow-up: the manual-next codegen path
// (control.rs) duplicated the channel decision and dropped float/single/enum,
// so `let a: f64 = next(g)` was native E0308.  Now both paths share
// `coroutine_layout::channel_tag`.

cross_mode!(
    y7_x2_float_manual_next,
    r#"
    fn vals() -> iterator<float> { yield 1.5; yield 2.5; }
    fn test() {
        g = vals();
        a = next(g);
        b = next(g);
        print("{a} {b}\n");
        assert(a == 1.5 && b == 2.5, "y7_x2 float manual next");
    }
    "#
);

cross_mode!(
    y8_x2_single_manual_next,
    r#"
    fn vals() -> iterator<single> { yield 1.5 as single; yield 2.5 as single; }
    fn test() {
        g = vals();
        a = next(g);
        b = next(g);
        print("{a} {b}\n");
        assert(a == (1.5 as single) && b == (2.5 as single), "y8_x2 single manual next");
    }
    "#
);

cross_mode!(
    y9_x2_enum_manual_next,
    r#"
    enum Color { Red, Green, Blue }
    fn vals() -> iterator<Color> { yield Color.Red; yield Color.Blue; }
    fn test() {
        g = vals();
        a = next(g);
        b = next(g);
        print("{a == Color.Red} {b == Color.Blue}\n");
        assert(a == Color.Red && b == Color.Blue, "y9_x2 enum manual next");
    }
    "#
);
