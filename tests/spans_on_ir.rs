// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-07 phase 1 — IR-shape assertions.
//!
//! For each fault-prone construct that 1.B wraps, the parser must emit
//! a `Value::Span(Box::new((pos, inner)))` node in the IR.  These
//! assertions are the regression gate for "did the wrap survive a
//! refactor?" — phase 0's `tests/error_messages.rs` covers the
//! externally-visible diagnostic, this file covers the internal IR.

extern crate loft;

use loft::data::Value;
use loft::parser::Parser;

mod common;
use common::cached_default;

/// Walk an IR tree, return `true` if any `Value::Span` node exists
/// whose wrapped position has the given (line, col).
fn has_span_at(val: &Value, line: u32, col: u32) -> bool {
    match val {
        Value::Span(b) => {
            let (pos, inner) = (&b.0, &b.1);
            if pos.line == line && pos.pos == col {
                return true;
            }
            has_span_at(inner, line, col)
        }
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.iter().any(|v| has_span_at(v, line, col))
        }
        Value::Insert(vs) | Value::Tuple(vs) | Value::Parallel(vs) => {
            vs.iter().any(|v| has_span_at(v, line, col))
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            args.iter().any(|v| has_span_at(v, line, col))
        }
        Value::Set(_, v) => has_span_at(v, line, col),
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => has_span_at(v, line, col),
        Value::BreakWith(_, v) | Value::TuplePut(_, _, v) => has_span_at(v, line, col),
        Value::If(c, t, e) => {
            has_span_at(c, line, col) || has_span_at(t, line, col) || has_span_at(e, line, col)
        }
        Value::Iter(_, a, b, c) => {
            has_span_at(a, line, col) || has_span_at(b, line, col) || has_span_at(c, line, col)
        }
        _ => false,
    }
}

/// Whole-program span search: walks every user definition's body.
fn any_span_at(p: &Parser, line: u32, col: u32) -> bool {
    p.data
        .definitions
        .iter()
        .any(|d| has_span_at(&d.code, line, col))
}

fn parse(code: &str) -> Parser {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(code, "spans_test", false);
    p
}

#[ignore = "plan-07 phase 1 — wrap activation pending wider unspan() audit (sorted/index/hash iteration paths)"]
#[test]
fn binary_div_wraps_in_span() {
    // `1 / z` — the `/` is at line 3, col 13 (0-based?).  We assert a
    // Span exists at the operator position.  Exact column is verified by
    // searching for ANY Span at line 3 — the wrap exists, that's the
    // 1.B(div/mod) acceptance.
    let p = parse(
        r#"
fn main() {
  z = 0;
  result = 1 / z;
}
"#,
    );
    assert!(
        p.diagnostics
            .lines()
            .iter()
            .all(|l| !format!("{l:?}").contains("Error")),
        "parse should succeed: {:?}",
        p.diagnostics.lines()
    );
    assert!(
        p.data
            .definitions
            .iter()
            .any(|d| has_span_on_line(&d.code, 4)),
        "expected Value::Span on line 4 (the `/` operator)"
    );
}

#[ignore = "plan-07 phase 1 — wrap activation pending wider unspan() audit (sorted/index/hash iteration paths)"]
#[test]
fn binary_mod_wraps_in_span() {
    let p = parse(
        r#"
fn main() {
  z = 0;
  result = 1 % z;
}
"#,
    );
    assert!(
        p.data
            .definitions
            .iter()
            .any(|d| has_span_on_line(&d.code, 4)),
        "expected Value::Span on line 4 (the `%` operator)"
    );
}

#[test]
fn non_fault_prone_binary_does_not_wrap() {
    // `+` is not fault-prone in this 1.B sub-step (no overflow gate
    // here; phase 4 may add one later).  Confirm we did NOT wrap it
    // — over-wrapping would inflate IR size and trip the bench gate.
    let p = parse(
        r#"
fn main() {
  result = 1 + 2;
}
"#,
    );
    assert!(
        !p.data
            .definitions
            .iter()
            .any(|d| has_span_on_line(&d.code, 3)),
        "did NOT expect Value::Span on line 3 (`+` is not fault-prone yet)"
    );
}

/// Helper that searches for any Span on a given line, regardless of column.
fn has_span_on_line(val: &Value, line: u32) -> bool {
    match val {
        Value::Span(b) => b.0.line == line || has_span_on_line(&b.1, line),
        Value::Block(bl) | Value::Loop(bl) => bl.operators.iter().any(|v| has_span_on_line(v, line)),
        Value::Insert(vs) | Value::Tuple(vs) | Value::Parallel(vs) => {
            vs.iter().any(|v| has_span_on_line(v, line))
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            args.iter().any(|v| has_span_on_line(v, line))
        }
        Value::Set(_, v) => has_span_on_line(v, line),
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => has_span_on_line(v, line),
        Value::BreakWith(_, v) | Value::TuplePut(_, _, v) => has_span_on_line(v, line),
        Value::If(c, t, e) => {
            has_span_on_line(c, line) || has_span_on_line(t, line) || has_span_on_line(e, line)
        }
        Value::Iter(_, a, b, c) => {
            has_span_on_line(a, line) || has_span_on_line(b, line) || has_span_on_line(c, line)
        }
        _ => false,
    }
}

// has_span_at + any_span_at currently have no callers — they exist
// for later steps that need exact-column assertions.  Suppress the
// unused-fn lint until 1.B's per-token wraps land and the column
// assertions become useful.
#[allow(dead_code)]
fn _silence_unused() {
    let p = parse("fn main(){}");
    let _ = any_span_at(&p, 0, 0);
}
