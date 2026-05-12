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
