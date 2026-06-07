// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 03 slice B — REPL session integer-variable persistence.
//!
//! A variable bound in one input is visible to the next.  Each test's `assert`
//! holding (no panic) proves the prior binding was in scope with the right
//! value.  Integer scope only, per the slice's start.

use loft::repl::{Eval, ReplSession};

fn session() -> ReplSession {
    ReplSession::new("default").expect("load stdlib")
}

#[test]
fn integer_variable_persists_across_inputs() {
    let mut s = session();
    assert!(matches!(s.eval("x = 1"), Eval::Ran));
    assert!(matches!(
        s.eval("assert(x + 1 == 2, \"x persists\")"),
        Eval::Ran
    ));
}

#[test]
fn variable_depends_on_earlier_variable() {
    let mut s = session();
    assert!(matches!(s.eval("a = 10"), Eval::Ran));
    assert!(matches!(s.eval("b = a * 2"), Eval::Ran));
    assert!(matches!(s.eval("assert(b == 20, \"b == a*2\")"), Eval::Ran));
}

#[test]
fn rebinding_updates_value() {
    let mut s = session();
    assert!(matches!(s.eval("n = 5"), Eval::Ran));
    assert!(matches!(s.eval("n = n + 100"), Eval::Ran));
    assert!(matches!(
        s.eval("assert(n == 105, \"n updated\")"),
        Eval::Ran
    ));
}

#[test]
fn incomplete_input_asks_for_more() {
    let mut s = session();
    assert!(matches!(s.eval("y = (1 +"), Eval::NeedMore));
}

/// A parse error is reported (and `data` is rolled back).
#[test]
fn parse_error_is_reported() {
    let mut s = session();
    assert!(matches!(s.eval("z = 3"), Eval::Ran));
    assert!(matches!(s.eval("z = 1 2 3"), Eval::Error(_)));
}

/// Recovery after a parse error: a clean input still works.  The lexer's
/// diagnostics are cleared per `parse_str`, so a prior error no longer
/// re-`fill`s into the parser and poisons the next parse.
#[test]
fn parse_error_leaves_session_usable() {
    let mut s = session();
    assert!(matches!(s.eval("z = 3"), Eval::Ran));
    assert!(matches!(s.eval("z = 1 2 3"), Eval::Error(_)));
    assert!(matches!(
        s.eval("assert(z == 3, \"z survived error\")"),
        Eval::Ran
    ));
}
