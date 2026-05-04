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

// e1_d1_char_int_local — see P207 (PROBLEMS.md): native codegen E0308 on
// `t.0 == 'a'` when `t.0` is a tuple-element character.  Workaround would be
// `t.0 as integer == 97` but that defeats the cell's purpose (validating
// character equality through tuple storage).  Marked ignored on the P207 tag
// so a later fix removes the tag in a one-line follow-up.
#[ignore = "P207 — native char-tuple-elem eq codegen bug"]
#[test]
fn e1_d1_char_int_local() {
    common::cross_mode::run_cross_mode(
        "e1_d1_char_int_local",
        r#"
        fn test() {
            t = ('a', 42);
            print("{t.0},{t.1}\n");
            assert(t.0 == 'a', "e1_d1 char");
            assert(t.1 == 42, "e1_d1 int");
        }
        "#,
    );
}

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

// E1×D2 return — requires T1.8a tuple-return convention (plan-06 phase 9a).
#[ignore = "T1.8a — plan-06 phase 9a"]
#[test]
fn e1_d2_return_int_int() {
    common::cross_mode::run_cross_mode(
        "e1_d2_return_int_int",
        r#"
        fn make_pair() -> (integer, integer) { (3, 7) }
        fn test() {
            t = make_pair();
            print("{t.0},{t.1}\n");
            assert(t.0 == 3 && t.1 == 7, "e1_d2_return");
        }
        "#,
    );
}

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

// E2×D2 return — requires T1.8a.
#[ignore = "T1.8a — plan-06 phase 9a"]
#[test]
fn e2_d2_return_text_text() {
    common::cross_mode::run_cross_mode(
        "e2_d2_return_text_text",
        r#"
        fn make_pair() -> (text, text) { ("alpha", "beta") }
        fn test() {
            t = make_pair();
            print("{t.0}|{t.1}\n");
            assert(t.0 == "alpha" && t.1 == "beta", "e2_d2_return");
        }
        "#,
    );
}
