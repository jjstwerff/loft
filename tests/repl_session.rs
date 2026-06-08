// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 03 slice B — REPL session variable persistence.
//!
//! A variable bound in one input is visible to the next.  Each test's `assert`
//! holding (no panic) proves the prior binding was in scope with the right
//! value.  Integer and text bindings both persist.

use loft::debugger::StepMode;
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

/// `completion_model` resolves dotted-access members from the live schema: a
/// struct-typed variable exposes its fields; a value of any type exposes its
/// methods (rendered `name(`); an enum *type* exposes its variant names.  This
/// is REPL.C member completion (the `complete_word` matching is unit-tested in
/// `repl.rs`).
#[test]
fn completion_model_resolves_members_from_schema() {
    let mut s = session();
    assert!(matches!(
        s.eval("struct Point { x: integer, y: integer }"),
        Eval::Ran
    ));
    assert!(matches!(
        s.eval("enum Color { Red, Green, Blue }"),
        Eval::Ran
    ));
    assert!(matches!(s.eval("p = Point { x: 1, y: 2 }"), Eval::Ran));
    assert!(matches!(s.eval("s = \"hi\""), Eval::Ran));
    let model = s.completion_model();

    // Struct variable → its fields (membership, not equality: a struct may also
    // carry generated methods).
    let p = model.members.get("p").expect("p has members");
    assert!(
        p.contains(&"x".to_string()) && p.contains(&"y".to_string()),
        "struct fields present: {p:?}"
    );

    // A text variable → its methods, each with a trailing `(`.
    let sm = model.members.get("s").expect("s has members");
    assert!(
        sm.contains(&"starts_with(".to_string()),
        "text method `starts_with(` present: {sm:?}"
    );
    assert!(
        sm.iter().all(|m| m.ends_with('(')),
        "every text member is a callable method: {sm:?}"
    );

    // Enum *type* → its variants (sorted, bare — no trailing paren).
    assert_eq!(
        model.members.get("Color"),
        Some(&vec![
            "Blue".to_string(),
            "Green".to_string(),
            "Red".to_string()
        ]),
        "enum type → variant names: {:?}",
        model.members
    );

    // Each member list is sorted (the order `complete_word` relies on).
    for (recv, ms) in &model.members {
        assert!(
            ms.windows(2).all(|w| w[0] <= w[1]),
            "members of `{recv}` sorted: {ms:?}"
        );
    }
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

// ── @PLN15 G1: REPL :break command ───────────────────────────────────────────

/// `add_breakpoint("dbl")` (the `:break dbl` command) breaks at the named
/// function's body start; an observing call that runs it captures the frame into
/// `last_hits` with the argument value.
#[test]
fn repl_breakpoint_by_fn_name_captures_frame() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn dbl(n: integer) -> integer { n + n }"),
        Eval::Ran
    ));
    s.add_breakpoint("dbl");
    assert_eq!(s.breakpoints(), ["dbl"]);
    // an observing call that runs dbl fires the breakpoint
    assert!(matches!(s.eval("dbl(21)"), Eval::Ran));
    let hits = s.last_hits();
    assert_eq!(hits.len(), 1, "fired once: {hits:?}");
    assert_eq!(hits[0].function, "dbl");
    assert!(
        hits[0].locals.iter().any(|(n, v)| n == "n" && v == "21"),
        "n == 21: {hits:?}"
    );
    // clearing removes it; a later call captures nothing
    s.clear_breakpoints();
    assert!(s.breakpoints().is_empty());
    assert!(matches!(s.eval("dbl(99)"), Eval::Ran));
    assert!(s.last_hits().is_empty(), "no breakpoint → no hits");
}

/// A `<fn>:<line>` spec breaks at a specific line.  An unknown function is skipped,
/// not an error.  A **bare line** isn't unique in the REPL (every input restarts
/// line numbering under `<repl>`), so it resolves to nothing — function-scoped
/// specs are the only unique form.
#[test]
fn repl_breakpoint_fn_line_and_unknown() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn step(n: integer) -> integer {\n  m = n + 1;\n  m * 2\n}"),
        Eval::Ran
    ));
    s.add_breakpoint("step:2"); // the `m = n + 1` line
    assert!(matches!(s.eval("step(4)"), Eval::Ran));
    assert!(
        s.last_hits().iter().any(|h| h.function == "step"),
        "fn:line breakpoint fired: {:?}",
        s.last_hits()
    );
    // an unknown function is skipped (no panic, no hit), session stays usable
    s.clear_breakpoints();
    s.add_breakpoint("does_not_exist");
    assert!(matches!(s.eval("step(4)"), Eval::Ran));
    assert!(s.last_hits().is_empty(), "unknown fn → no hit");
    // a bare line resolves to nothing (not unique) — no hit
    s.clear_breakpoints();
    s.add_breakpoint("2");
    assert!(matches!(s.eval("step(4)"), Eval::Ran));
    assert!(s.last_hits().is_empty(), "bare line → no hit (not unique)");
}

/// @PLN15 G1 (interactive) — with stepping on, observing a call that hits a
/// breakpoint **suspends** into the paused sub-mode; edit a value in the frame,
/// continue, and the edit is picked up — the full pause → edit → resume cycle
/// driven through one held session.  `calc(5)` is normally 50; editing `n` to 99
/// makes it 990 on resume, which the program's assert then confirms.
#[test]
fn repl_interactive_break_edit_continue() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn calc(n: integer) -> integer {\n  n * 10\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("calc");
    // The assert is true only with the edit (990), false without it (50) — so a
    // clean run proves the edited value was used.
    assert!(matches!(
        s.eval("assert(calc(5) == 990, \"edited\")"),
        Eval::Paused
    ));
    assert!(s.is_debugging(), "suspended inside calc");
    let f = s.paused_frame().expect("frame");
    assert_eq!(f.function, "calc");
    assert!(
        f.locals.iter().any(|(n, v)| n == "n" && v == "5"),
        "n == 5 at the pause (pre-multiply): {f:?}"
    );
    // Edit `n` in the live frame; the frame view refreshes to the new value.
    assert!(s.debug_set("n", 99), "write-back n = 99");
    assert!(
        s.paused_frame()
            .unwrap()
            .locals
            .iter()
            .any(|(n, v)| n == "n" && v == "99"),
        "frame refreshed to n == 99: {:?}",
        s.paused_frame()
    );
    // Continue → resumes with the edit; the run finishes and the sub-mode is left.
    assert!(!s.debug_continue(), "run finished (no further pause)");
    assert!(!s.is_debugging(), "back to normal mode");
    // Breakpoints persist across runs, so calc(2) would suspend again; clear them
    // and confirm the session evaluates normally after a debug run.
    s.clear_breakpoints();
    assert!(
        matches!(s.eval("calc(2)"), Eval::Ran),
        "session still works"
    );
}

/// @PLN15 G1 (interactive) — the step verbs at the REPL: from `outer`'s call
/// line, step **into** `inner`, then step **out** back to `outer`, then continue
/// to completion — all through the held session.
#[test]
fn repl_interactive_step_into_and_out() {
    let mut s = session();
    for d in [
        "fn inner(x: integer) -> integer {\n  x + 1\n}",
        "fn outer(n: integer) -> integer {\n  a = inner(n);\n  a + 100\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("outer:2"); // the `a = inner(n)` line
    assert!(matches!(s.eval("outer(5)"), Eval::Paused));
    assert_eq!(s.paused_frame().unwrap().function, "outer");
    assert!(s.debug_step(StepMode::Into), "into inner");
    let f = s.paused_frame().unwrap();
    assert_eq!(f.function, "inner", "stepped into inner: {f:?}");
    assert!(
        f.locals.iter().any(|(n, v)| n == "x" && v == "5"),
        "inner's x == 5: {f:?}"
    );
    assert!(s.debug_step(StepMode::Out), "out back to outer");
    assert_eq!(s.paused_frame().unwrap().function, "outer");
    assert!(!s.debug_continue(), "finishes");
    assert!(!s.is_debugging());
}

/// @PLN15 G1 (interactive) — the REPL-at-frame: at a pause, evaluate arbitrary
/// expressions against the live frame (not just read a value back).  Covers a
/// heap (struct) argument and an integer arg, reads compound expressions, and
/// confirms a live edit is reflected in a subsequent frame eval.
#[test]
fn repl_interactive_eval_at_frame() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "fn area(pt: Point, k: integer) -> integer {\n  pt.x * pt.y * k\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("area");
    assert!(matches!(
        s.eval("area(Point { x: 3, y: 4 }, 2)"),
        Eval::Paused
    ));
    // Read expressions against the frame's live variables.
    assert_eq!(s.debug_eval("k").as_deref(), Some("2"), "bare arg");
    assert_eq!(
        s.debug_eval("pt.x * pt.y").as_deref(),
        Some("12"),
        "struct field arithmetic"
    );
    assert_eq!(s.debug_eval("pt.x + k").as_deref(), Some("5"), "mixed args");
    // A live integer edit is reflected in a later frame eval.
    assert!(s.debug_set("k", 10), "edit k");
    assert_eq!(
        s.debug_eval("pt.x * pt.y * k").as_deref(),
        Some("120"),
        "eval reflects the edit"
    );
    // A nonsense expression yields None, not a panic; the session stays paused.
    assert_eq!(s.debug_eval("no_such_var + 1"), None);
    assert!(s.is_debugging(), "still paused after a failed eval");
    assert!(!s.debug_continue());
}

/// Without stepping enabled, breakpoints stay in **record-and-continue** mode: an
/// observing run completes (`Eval::Ran`, not `Paused`) and the hits land in
/// `last_hits` — the programmatic mode the conditional-breakpoint sweep relies on.
#[test]
fn repl_breakpoints_record_when_not_stepping() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn calc(n: integer) -> integer {\n  n * 10\n}"),
        Eval::Ran
    ));
    s.add_breakpoint("calc"); // stepping NOT enabled
    assert!(matches!(s.eval("calc(5)"), Eval::Ran), "run completes");
    assert!(!s.is_debugging(), "never suspends without stepping");
    assert!(
        s.last_hits().iter().any(|h| h.function == "calc"),
        "hit recorded: {:?}",
        s.last_hits()
    );
}
