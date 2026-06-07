// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 02 — statement-level parser entry.
//!
//! Increment 1: `Parser::statement_incomplete` (the REPL "read more lines?"
//! detector).  Increment 2: `parse_statement` for top-level definitions —
//! incremental append against the live session + transactional rollback.
//! The `__repl_session` local-persistence path (bare expressions) lands in a
//! later increment; see `plans/12-repl-and-introspection/02-statement-parser.md`.

use loft::parser::{ParseResult, Parser};

/// A parser with the stdlib loaded — the REPL's starting state.
fn session() -> Parser {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p
}

#[test]
fn complete_statements_are_complete() {
    for s in [
        "x = 1",
        "dbl(21)",
        "fn dbl(x: integer) -> integer { x + x }",
        "struct Pair { a: integer, b: integer }",
        "\"a string\"",
        "x.field", // ends on an identifier, not a trailing `.`
        "[1, 2, 3]",
        "if a > b { 1 } else { 2 }",
    ] {
        assert!(
            !Parser::statement_incomplete(s),
            "expected COMPLETE, got incomplete: {s:?}"
        );
    }
}

#[test]
fn open_constructs_need_more() {
    for s in [
        "fn dbl(x: integer) -> integer {", // open brace
        "dbl(",                            // open paren
        "[1, 2,",                          // open bracket
        "struct Pair {",                   // open struct body
    ] {
        assert!(
            Parser::statement_incomplete(s),
            "expected NeedMore (open bracket): {s:?}"
        );
    }
}

#[test]
fn trailing_operator_needs_more() {
    for s in ["x = 1 +", "a *", "x.", "1 <", "[1,"] {
        assert!(
            Parser::statement_incomplete(s),
            "expected NeedMore (trailing operator): {s:?}"
        );
    }
}

#[test]
fn unterminated_string_needs_more() {
    assert!(Parser::statement_incomplete("x = \"unterminated"));
    // An escaped quote inside the string does NOT close it.
    assert!(Parser::statement_incomplete("\"a\\\"b"));
    // A balanced string is complete.
    assert!(!Parser::statement_incomplete("\"a\\\"b\""));
}

#[test]
fn line_comment_does_not_confuse_brackets() {
    // The `}` is inside a comment, so the brace is still open.
    assert!(Parser::statement_incomplete("fn f() { // }"));
    // A complete line with a trailing comment is complete.
    assert!(!Parser::statement_incomplete("x = 1 // a note"));
}

// ── parse_statement (increment 2): top-level definitions ──────────────────

#[test]
fn top_level_struct_registers() {
    let mut p = session();
    let r = p.parse_statement("struct ReplPair { a: integer, b: integer }");
    assert!(matches!(r, ParseResult::Ready { .. }), "got {r:?}");
    assert!(
        p.data.def_nr("ReplPair") != u32::MAX,
        "ReplPair not registered in data"
    );
}

#[test]
fn top_level_fn_registers() {
    let mut p = session();
    let r = p.parse_statement("fn dbl(x: integer) -> integer { x + x }");
    assert!(matches!(r, ParseResult::Ready { .. }), "got {r:?}");
    assert!(p.data.def_nr("n_dbl") != u32::MAX, "n_dbl not registered");
}

#[test]
fn incomplete_def_needs_more() {
    let mut p = session();
    assert!(matches!(
        p.parse_statement("fn dbl(x: integer) -> integer {"),
        ParseResult::NeedMore
    ));
}

/// Acceptance #3 — a parse error leaves `data` byte-identical (def count
/// unchanged) to the pre-call state.
#[test]
fn parse_error_rolls_back() {
    let mut p = session();
    let pre = p.data.definitions();
    let r = p.parse_statement("struct Bad { ??? }");
    assert!(
        matches!(r, ParseResult::Error(_)),
        "expected Error, got {r:?}"
    );
    assert_eq!(
        p.data.definitions(),
        pre,
        "rollback failed: definition count changed"
    );
}

/// A later statement sees a definition from an earlier one (the live-session
/// property the REPL needs).
#[test]
fn later_statement_sees_earlier_def() {
    let mut p = session();
    assert!(matches!(
        p.parse_statement("struct ReplPoint { x: integer }"),
        ParseResult::Ready { .. }
    ));
    let r = p.parse_statement("fn px(p: ReplPoint) -> integer { p.x }");
    assert!(
        matches!(r, ParseResult::Ready { .. }),
        "cross-statement reference failed: {r:?}"
    );
    assert!(p.data.def_nr("n_px") != u32::MAX);
}
