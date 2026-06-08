// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 03 slice B — REPL session variable persistence.
//!
//! A variable bound in one input is visible to the next.  Each test's `assert`
//! holding (no panic) proves the prior binding was in scope with the right
//! value.  Integer and text bindings both persist.

use loft::repl::{Eval, ReplSession};
use std::path::PathBuf;

fn session() -> ReplSession {
    ReplSession::new("default").expect("load stdlib")
}

/// A unique temp path for a session file, keyed by the test's tag + this
/// process's id so parallel test binaries can't collide.
fn tmp_session(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("loft_test_{tag}_{}.session", std::process::id()))
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

/// Text bindings persist across inputs, the same as integers — the on-ramp
/// case a beginner hits first (`name = "Alice"`).  Confirms persistence is not
/// integer-only.
#[test]
fn text_variable_persists_across_inputs() {
    let mut s = session();
    assert!(matches!(s.eval("name = \"Alice\""), Eval::Ran));
    assert!(matches!(
        s.eval("assert(name == \"Alice\", \"name persists\")"),
        Eval::Ran
    ));
}

// ── REPL.S: auto-resume via text-replay ──────────────────────────────────────

/// A session's state-changing inputs (a binding + a def) are persisted, and a
/// fresh session resumes them: the variable and the function are both usable
/// again, with no entries skipped.
#[test]
fn session_persists_and_resumes() {
    let path = tmp_session("resume");
    let _ = std::fs::remove_file(&path);
    {
        let mut a = session();
        a.enable_persistence(&path).expect("enable persistence");
        assert!(matches!(a.eval("x = 41"), Eval::Ran));
        assert!(matches!(
            a.eval("fn dbl(n: integer) -> integer { n + n }"),
            Eval::Ran
        ));
    } // `a` dropped — its session file stays on disk
    let mut b = session();
    let stats = b.resume_from(&path);
    assert_eq!(stats.restored, 2, "binding + def restored: {stats:?}");
    assert_eq!(stats.skipped, 0, "nothing skipped: {stats:?}");
    assert!(matches!(
        b.eval("assert(x == 41, \"x restored\")"),
        Eval::Ran
    ));
    assert!(matches!(
        b.eval("assert(dbl(x) == 82, \"dbl restored\")"),
        Eval::Ran
    ));
    let _ = std::fs::remove_file(&path);
}

/// A stale/corrupt entry between two good ones is skipped, not fatal: the two
/// good bindings still restore and the session stays usable.  This is the
/// fault-tolerance guarantee — a poison line never bricks resume.
#[test]
fn resume_skips_poison_entry() {
    let path = tmp_session("poison");
    // good binding, garbage (a parse error, same shape as `z = 1 2 3`), good
    // binding — NUL-separated exactly as the REPL writes them.
    std::fs::write(&path, "a = 1\0c = 1 2 3\0b = 2\0").expect("write session");
    let mut s = session();
    let stats = s.resume_from(&path);
    assert_eq!(stats.restored, 2, "two good entries restored: {stats:?}");
    assert_eq!(stats.skipped, 1, "one poison entry skipped: {stats:?}");
    assert!(matches!(
        s.eval("assert(a + b == 3, \"a,b survived poison\")"),
        Eval::Ran
    ));
    let _ = std::fs::remove_file(&path);
}

// ── REPL.C: Tab-completion candidates ────────────────────────────────────────

/// `completion_names` covers the user's session (bound vars, defined fns) plus
/// stdlib globals, and excludes the synthetic generation wrappers and internal
/// `<…>`-shaped names.  The list is sorted.
#[test]
fn completion_names_cover_session_and_stdlib() {
    let mut s = session();
    assert!(matches!(s.eval("price = 10"), Eval::Ran));
    assert!(matches!(
        s.eval("fn tripled(n: integer) -> integer { n * 3 }"),
        Eval::Ran
    ));
    let names = s.completion_names();
    assert!(names.iter().any(|n| n == "price"), "bound var: {names:?}");
    assert!(names.iter().any(|n| n == "tripled"), "user fn: {names:?}");
    assert!(
        names.iter().any(|n| n == "println"),
        "stdlib global: {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with("replmain") || n.contains('<')),
        "no synthetic/internal names leak in: {names:?}"
    );
    assert!(names.windows(2).all(|w| w[0] <= w[1]), "sorted: {names:?}");
}

/// Only state-changing inputs are persisted: an observing statement (a bare
/// expression) is evaluated for its echo but writes nothing to the session
/// file, so resume never re-runs it.
#[test]
fn observe_not_persisted() {
    let path = tmp_session("observe");
    let _ = std::fs::remove_file(&path);
    let mut s = session();
    s.enable_persistence(&path).expect("enable persistence");
    assert!(matches!(s.eval("k = 9"), Eval::Ran)); // binding → persisted
    assert!(matches!(s.eval("k + 1"), Eval::Ran)); // observing → NOT persisted
    drop(s);
    let contents = std::fs::read_to_string(&path).expect("session file exists");
    assert_eq!(
        contents, "k = 9\0",
        "only the binding persisted, not the observe: {contents:?}"
    );
    let _ = std::fs::remove_file(&path);
}
