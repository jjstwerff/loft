// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-22 mutable-closures validation matrix.
//!
//! Each test exercises one cell of the (case × storage-destination)
//! matrix described in
//! `doc/claude/plans/22-mutable-closures/00-matrix.md`.  Every cell
//! runs under both the interpreter and `--native`; the harness in
//! `tests/common/cross_mode.rs` (shared with `tests/tuple_matrix.rs`,
//! `tests/template_matrix.rs`, and `tests/closure_matrix.rs`) asserts
//! byte-identical stdout.
//!
//! **Every cell is `#[ignore]` by default** — the harness shells out
//! to `loft --interpret` and `loft --native` per cell, the latter
//! invoking `rustc`.  Too heavy for the default `cargo test` path.
//! Run the matrix explicitly:
//!
//! ```bash
//! cargo test --release --test mut_closure_matrix -- --ignored
//! ```
//!
//! ## The frozen matrix (phase 00, locked 2026-05-12)
//!
//! Cell legend: `PASS:test_name` / `FIX:phase` / `REJECT:phase` /
//! `CLOSED:reason`.
//!
//! | | D1 — local var | D2 — fn parameter | D3 — struct field | D4 — return value |
//! |---|---|---|---|---|
//! | **A** read-only | PASS:00 | PASS:00 | PASS:00 | PASS:00 |
//! | **B** co-scoped (Reference) | PASS:02c | PASS:02c | PASS:02c | n/a |
//! | **B** co-scoped (scalar via cell — int/float/single/char/enum/bool/text) | PASS:02d-iii.e/iv/v/vi | PASS:02d-iii.e/iv/v/vi | PASS:02d-iii.e/iv/v/vi | n/a |
//! | **C** moved (factory) | n/a | n/a | FIX:03 | FIX:03 |
//! | **D** aliased mutating | REJECT:04 | REJECT:04 | REJECT:04 | REJECT:04 |
//! | **Mutable<T> explicit** | FIX:05 | FIX:05 | FIX:05 | FIX:05 |
//!
//! Phase 02d-iv (2026-05-12) extended scalar boxing to float /
//! single / character / plain enum + multi-scalar.
//! Phase 02d-v (2026-05-12) added boolean via OpEqInt LHS shape
//! recognition in `maybe_prepend_cell_alloc`.
//! Phase 02d-vi (2026-05-12) added text via a bypass guard in
//! `parse_assign_op` that skips the text-special branch
//! (assign_text + work-buffer machinery) when the LHS is
//! auto-deref'd boxed text.  Re-assignment (`log = "after"`)
//! and append (`log += s` lowered via compute_op_code) both
//! route through the general `OpSetText` path.
//!
//! Phase 02d-vii (2026-05-12) closes the text-return crash:
//! the work-text result buffer in text-returning fns locks
//! the closure record's store BEFORE the SetDbRef capturing
//! the boxed-text DbRef can fire (panic at
//! `src/store.rs:963`).  Surgical fix: skip text-cell boxing
//! when the parent function returns text — text mutation in
//! that shape falls back to the existing void-return write-
//! back mechanism (which works for b_d1; b_d2/b_d3 in
//! text-return remain pre-02d-vi behaviour, mutations lost
//! when the closure is invoked outside the parent's frame).
//! Deeper fix (defer the work-text lock or use a separate
//! store) is an open follow-up — see
//! `doc/claude/plans/22-mutable-closures/02d-iii-design.md`'s
//! 02d-viii row.
//!
//! Each `body` must declare `fn test() { … }` and any helper fns
//! at file scope; the harness appends `fn main() { test(); }`.

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

// ── Phase 00 — Case A baseline (read-only captures across all 4 destinations) ─
//
// Case A covers closures whose body does NOT mutate any captured
// binding — today's value-snapshot semantics, no analysis change
// expected from later phases.  These cells serve as the regression
// net: every plan-22 phase that lands a classifier or lowering MUST
// keep all 4 Case A cells green.  A failure here means the phase
// over-classified a read-only body as mutating.
//
// Coverage is broader than plan-15's c0_d* cells (which only
// validated non-CAPTURING closures) — these cells specifically
// CAPTURE a binding and READ it without writing.  That's the
// boundary plan-22's classifier must distinguish from Case B.

cross_mode!(
    a_d1_read_only_local,
    r#"
    fn test() {
        n = 5;
        f = fn(x: integer) -> integer { x + n };
        result = f(10);
        print("{result}\n");
        assert(result == 15, "a_d1 result={result}");
    }
    "#
);

cross_mode!(
    a_d2_read_only_arg,
    r#"
    fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
    fn test() {
        n = 5;
        result = apply(fn(x: integer) -> integer { x + n }, 10);
        print("{result}\n");
        assert(result == 15, "a_d2 result={result}");
    }
    "#
);

cross_mode!(
    a_d3_read_only_field,
    r#"
    struct Box { cb: fn(integer) -> integer }
    fn test() {
        n = 5;
        b = Box { cb: fn(x: integer) -> integer { x + n } };
        result = b.cb(10);
        print("{result}\n");
        assert(result == 15, "a_d3 result={result}");
    }
    "#
);

cross_mode!(
    a_d4_read_only_return,
    r#"
    fn make_reader(label: text) -> fn(integer) -> text {
        fn(n: integer) -> text { "{label}:{n}" }
    }
    fn test() {
        f = make_reader("tag");
        result = f(42);
        print("{result}\n");
        assert(result == "tag:42", "a_d4 result='{result}'");
    }
    "#
);

// ── Phase 02c — Case B (Reference capture, mutating, co-scoped) ─────────────
//
// The big ergonomic win.  When a closure body mutates a captured
// struct (Reference type), the mutation propagates to the outer
// scope through the closure record's auto-Reference attribute
// (12-byte DbRef share, OpSetDbRef + OpGetDbRef instead of inline
// bytes + OpCopyRecord + OpGetField).
//
// Sub-phases shipped 2026-05-12:
//   - 02a — pass-1 mutation detection foundation
//   - 02b — auto-Reference storage encoding
//   - 02c — wire mutation flags into synthesize_closure_record
//
// Each cell exercises a different destination shape:
//   - b_d1 — closure stored in a local variable, called inline
//   - b_d2 — closure passed as fn parameter, called by the callee
//   - b_d3 — closure stored in a struct field, called via field
//
// All three should observe the outer struct's mutation after the
// closure runs.  Pre-02c these cells would silently fail (closure
// mutates its private copy; outer unchanged).

cross_mode!(
    b_d1_ref_capture_local_mutates,
    r#"
    struct State { x: integer }
    fn test() {
        s = State { x: 0 };
        f = fn() { s.x = 7; };
        f();
        print("after f(): {s.x}\n");
        assert(s.x == 7, "b_d1 expected 7, got {s.x}");
    }
    "#
);

cross_mode!(
    b_d2_ref_capture_arg_mutates,
    r#"
    struct State { x: integer }
    fn invoke(f: fn()) { f(); }
    fn test() {
        s = State { x: 0 };
        invoke(fn() { s.x = 11; });
        print("after invoke: {s.x}\n");
        assert(s.x == 11, "b_d2 expected 11, got {s.x}");
    }
    "#
);

// P258 closed 2026-05-12 — native codegen for the auto-Reference
// closure-record attribute now uses `db.dbref()` (12-byte
// Parts::DbRef) matching interp's typedef.rs branch.
cross_mode!(
    b_d3_ref_capture_field_mutates,
    r#"
    struct State { x: integer }
    struct Loop { cb: fn() }
    fn test() {
        s = State { x: 0 };
        loop = Loop { cb: fn() { s.x = 13; } };
        loop.cb();
        print("after loop.cb: {s.x}\n");
        assert(s.x == 13, "b_d3 expected 13, got {s.x}");
    }
    "#
);

// Two-call cell — proves the mutation is genuinely persistent
// across closure invocations (not a one-shot side effect of the
// first call's snapshot).
cross_mode!(
    b_d1_ref_capture_repeated_calls,
    r#"
    struct Counter { n: integer }
    fn test() {
        c = Counter { n: 0 };
        bump = fn() { c.n = c.n + 1; };
        bump();
        bump();
        bump();
        print("after 3x bump: {c.n}\n");
        assert(c.n == 3, "expected 3, got {c.n}");
    }
    "#
);

// ── Phase 02d-iii — Case B (scalar capture, mutating, co-scoped) ────────────
//
// Plan-22 phase 02d-iii.e shipped 2026-05-12 — boxes scalar
// captures (Integer / Float / Single / Character / Enum) via a
// hidden `__cell_<T>` record at the outer-binding site.  The
// closure record holds an auto-Reference DbRef to the same cell
// (12-byte share-by-DbRef), so closure mutations propagate back
// to the outer scope through shared DbRef.  See
// `doc/claude/plans/22-mutable-closures/02d-iii-design.md` for
// the per-sub-step breakdown (a-e).
//
// Text is intentionally EXCLUDED from boxing today (text
// mutation `log += s` flows through the existing void-return
// write-back mechanism in `parse_call_ref`).  Text-cell boxing
// is planned for 02d-iv.

cross_mode!(
    b_d1_int_capture_local_mutates,
    r#"
    fn test() {
        n = 0;
        f = fn() { n = n + 1; };
        f();
        f();
        print("after 2x f(): {n}\n");
        assert(n == 2, "b_d1 expected 2, got {n}");
    }
    "#
);

cross_mode!(
    b_d2_int_capture_arg_mutates,
    r#"
    fn invoke(f: fn()) { f(); }
    fn test() {
        n = 0;
        invoke(fn() { n = n + 5; });
        print("after invoke: {n}\n");
        assert(n == 5, "b_d2 expected 5, got {n}");
    }
    "#
);

cross_mode!(
    b_d3_int_capture_field_mutates,
    r#"
    struct Loop { cb: fn() }
    fn test() {
        n = 0;
        loop = Loop { cb: fn() { n = n + 7; } };
        loop.cb();
        print("after loop.cb: {n}\n");
        assert(n == 7, "b_d3 expected 7, got {n}");
    }
    "#
);

cross_mode!(
    b_d1_int_capture_repeated_calls,
    r#"
    fn test() {
        count = 0;
        bump = fn() { count = count + 1; };
        bump();
        bump();
        bump();
        print("after 3x bump: {count}\n");
        assert(count == 3, "expected 3, got {count}");
    }
    "#
);

// ── Phase 02d-iv — Case B (additional primitive types) ──────────────────────
//
// 02d-iv extends scalar-capture boxing to the remaining
// supported primitives: float, character, single, plain enum,
// plus the multi-scalar shape (two distinct scalars boxed
// simultaneously into separate `__cell_<T>` records).  Boolean
// is intentionally excluded (02d-iii.b's auto-deref wraps
// boolean in `OpEqInt(...)` for which there's no Set
// counterpart in `call_to_set_op`).  Text re-assignment is
// likewise not yet supported (text mutation `log += s` flows
// through the existing void-return write-back mechanism, not
// through cells).

cross_mode!(
    b_d1_float_capture_local_mutates,
    r#"
    fn test() {
        x = 0.0;
        bump = fn() { x = x + 1.5; };
        bump();
        bump();
        print("float: {x}\n");
        assert(x == 3.0, "b_d1_float expected 3.0, got {x}");
    }
    "#
);

cross_mode!(
    b_d1_character_capture_local_mutates,
    r#"
    fn test() {
        c = 'a';
        bump = fn() { c = 'z'; };
        bump();
        print("char: {c}\n");
        assert(c == 'z', "b_d1_character expected z, got {c}");
    }
    "#
);

cross_mode!(
    b_d1_single_capture_local_mutates,
    r#"
    fn test() {
        x = 0.0f;
        bump = fn() { x = x + 0.5f; };
        bump();
        bump();
        bump();
        print("single: {x}\n");
        assert(x == 1.5f, "b_d1_single expected 1.5, got {x}");
    }
    "#
);

cross_mode!(
    b_d1_enum_capture_local_mutates,
    r#"
    enum Mode { On, Off }
    fn test() {
        m = Mode.Off;
        bump = fn() { m = Mode.On; };
        bump();
        print("enum: {m}\n");
        assert(m == Mode.On, "b_d1_enum expected On");
    }
    "#
);

cross_mode!(
    b_d1_multi_scalar_capture,
    r#"
    fn test() {
        n = 0;
        x = 0.0;
        bump = fn() {
            n = n + 1;
            x = x + 0.25;
        };
        bump();
        bump();
        bump();
        bump();
        print("n={n} x={x}\n");
        assert(n == 4, "expected n=4, got {n}");
        assert(x == 1.0, "expected x=1.0, got {x}");
    }
    "#
);

// b_d2 variants — closure passed as fn arg (cell DbRef shared
// via auto-Reference closure-record attribute).
cross_mode!(
    b_d2_float_capture_arg_mutates,
    r#"
    fn invoke(f: fn()) { f(); }
    fn test() {
        x = 0.0;
        invoke(fn() { x = x + 2.5; });
        print("float: {x}\n");
        assert(x == 2.5, "b_d2_float expected 2.5, got {x}");
    }
    "#
);

cross_mode!(
    b_d2_character_capture_arg_mutates,
    r#"
    fn invoke(f: fn()) { f(); }
    fn test() {
        c = 'a';
        invoke(fn() { c = 'q'; });
        print("char: {c}\n");
        assert(c == 'q', "b_d2_character expected q, got {c}");
    }
    "#
);

// b_d3 variants — closure stored in a struct field.
cross_mode!(
    b_d3_float_capture_field_mutates,
    r#"
    struct Loop { cb: fn() }
    fn test() {
        x = 0.0;
        loop = Loop { cb: fn() { x = x + 4.0; } };
        loop.cb();
        print("float: {x}\n");
        assert(x == 4.0, "b_d3_float expected 4.0, got {x}");
    }
    "#
);

cross_mode!(
    b_d3_enum_capture_field_mutates,
    r#"
    enum State { A, B }
    struct Loop { cb: fn() }
    fn test() {
        s = State.A;
        loop = Loop { cb: fn() { s = State.B; } };
        loop.cb();
        print("enum: {s}\n");
        assert(s == State.B, "b_d3_enum expected B");
    }
    "#
);

// ── Phase 02d-v — boolean cells ──────────────────────────────────────────────
//
// 02d-v adds the `OpEqInt(OpGetByte, ...)` shape recognition
// to `maybe_prepend_cell_alloc` so the alloc preamble fires
// for boolean too.  Boolean reads are wrapped by 02d-iii.b's
// auto-deref as `Call(OpEqInt, [Call(OpGetByte, [Var, 0, 0]),
// Int(1)])` — a nested shape distinct from the direct
// `Call(OpGet<T>, [Var, 0])` used for other primitives.
// Boolean writes route through towards_set's existing boolean
// branch which produces the correct OpSetByte IR with the
// bool→byte conversion (collections.rs:493).

cross_mode!(
    b_d1_boolean_capture_local_mutates,
    r#"
    fn test() {
        b = false;
        f = fn() { b = true; };
        f();
        print("bool: {b}\n");
        assert(b == true, "b_d1_boolean expected true");
    }
    "#
);

cross_mode!(
    b_d2_boolean_capture_arg_mutates,
    r#"
    fn invoke(f: fn()) { f(); }
    fn test() {
        b = false;
        invoke(fn() { b = true; });
        print("bool: {b}\n");
        assert(b == true, "b_d2_boolean expected true");
    }
    "#
);

cross_mode!(
    b_d3_boolean_capture_field_mutates,
    r#"
    struct Loop { cb: fn() }
    fn test() {
        b = false;
        loop = Loop { cb: fn() { b = true; } };
        loop.cb();
        print("bool: {b}\n");
        assert(b == true, "b_d3_boolean expected true");
    }
    "#
);

cross_mode!(
    b_d1_boolean_capture_repeated_calls,
    r#"
    fn test() {
        flag = false;
        toggle = fn() { flag = !flag; };
        toggle();
        toggle();
        toggle();
        print("flag: {flag}\n");
        assert(flag == true, "expected true after 3x toggle, got {flag}");
    }
    "#
);

// ── Phase 02d-vi — text cells ────────────────────────────────────────────────
//
// 02d-vi adds text to the cell-based propagation path.  The
// bypass guard in `parse_assign_op` skips the text-special
// branch when the LHS is auto-deref'd boxed text, letting the
// general `towards_set` + `call_to_set_op` (OpGetText →
// OpSetText) + `maybe_prepend_cell_alloc` pipeline handle
// both re-assignment (`log = "after"`) and append
// (`log += s` lowered to `log = log + s` via compute_op_code).

cross_mode!(
    b_d1_text_capture_local_mutates,
    r#"
    fn test() {
        log = "before";
        f = fn() { log = "after"; };
        f();
        print("text: {log}\n");
        assert(log == "after", "b_d1_text expected after, got {log}");
    }
    "#
);

cross_mode!(
    b_d1_text_capture_local_appends,
    r#"
    fn test() {
        acc = "";
        f = fn(s: text) { acc += s; };
        f("a");
        f("bb");
        f("ccc");
        print("text: {acc}\n");
        assert(acc == "abbccc", "b_d1_text_append expected abbccc, got {acc}");
    }
    "#
);

cross_mode!(
    b_d2_text_capture_arg_mutates,
    r#"
    fn invoke(f: fn()) { f(); }
    fn test() {
        log = "before";
        invoke(fn() { log = "after"; });
        print("text: {log}\n");
        assert(log == "after", "b_d2_text expected after, got {log}");
    }
    "#
);

cross_mode!(
    b_d3_text_capture_field_mutates,
    r#"
    struct Loop { cb: fn() }
    fn test() {
        log = "before";
        loop = Loop { cb: fn() { log = "after"; } };
        loop.cb();
        print("text: {log}\n");
        assert(log == "after", "b_d3_text expected after, got {log}");
    }
    "#
);

// Phase 02d-vii — text APPEND in a text-returning fn.  This
// shape used to crash with "Write to locked store" before the
// 02d-vii guard (the work-text result buffer locks the
// closure record's store before SetDbRef can capture the
// boxed-text DbRef).  The guard now skips text-cell boxing in
// text-returning fns; text append still propagates via the
// existing void-return write-back mechanism (the
// `OpAppendText`-on-stack-text path that pre-dates 02d-vi).
//
// LIMITATION: text RE-ASSIGN (`acc = "after"`) in a closure
// inside a text-returning fn does NOT propagate today — the
// pre-02d-vi write-back only handles append, not reassign.
// b_d1_text_capture_local_mutates (void return + reassign)
// still passes via the cell mechanism.  Re-assign in
// text-returning fns is an open follow-up tied to the same
// "deeper fix" listed for 02d-viii.
cross_mode!(
    b_d1_text_append_in_text_return_fn,
    r#"
    fn build() -> text {
        acc = "";
        f = fn(s: text) { acc += s; };
        f("a");
        f("bb");
        acc
    }
    fn test() {
        r = build();
        print("{r}\n");
        assert(r == "abb", "b_d1_text_append_in_text_return expected abb, got {r}");
    }
    "#
);
