// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 02 — statement-level parser entry, increment 1.
//!
//! Covers `Parser::statement_incomplete` (the REPL "read more lines?"
//! detector).  The full `parse_statement` (session re-parse + rollback +
//! `__repl_session` local persistence) lands in later increments; see
//! `plans/12-repl-and-introspection/02-statement-parser.md`.

use loft::parser::Parser;

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
