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
//! | **B** co-scoped (Reference) | FIX:02 | FIX:02 | FIX:02 | n/a |
//! | **B** co-scoped (scalar via cell) | FIX:02 | FIX:02 | FIX:02 | n/a |
//! | **C** moved (factory) | n/a | n/a | FIX:03 | FIX:03 |
//! | **D** aliased mutating | REJECT:04 | REJECT:04 | REJECT:04 | REJECT:04 |
//! | **Mutable<T> explicit** | FIX:05 | FIX:05 | FIX:05 | FIX:05 |
//!
//! Phase 00 ships smokes + Case A baseline cells only.  B/C/D/M
//! cells land in their respective phase commits.
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
