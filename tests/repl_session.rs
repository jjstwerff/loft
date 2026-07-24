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

// ── @PLN16 G1: REPL :break command ───────────────────────────────────────────

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

/// @PLN16 G1 (interactive) — with stepping on, observing a call that hits a
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
    assert!(s.debug_set("n", "99"), "write-back n = 99");
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

/// @PLN16 G1 (interactive) — the step verbs at the REPL: from `outer`'s call
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

/// @PLN16 G1 (interactive) — the REPL-at-frame: at a pause, evaluate arbitrary
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
    assert!(s.debug_set("k", "10"), "edit k");
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

/// @PLN16 D2 — a **bare heap local** (struct / vector) is read **live, in place** at the
/// pause: `show_loft` renders the actual `DbRef` from the store, with no reconstruct and
/// no fn-return deep-copy.  A bare `vector` is the case the reconstruct-eval path could
/// not return (it faulted), so this proves the live read both fixes it and is faithful.
#[test]
fn repl_interactive_eval_bare_heap_live() {
    let mut s = session();
    for d in [
        "struct Mob { hp: integer }",
        "fn run(v: vector<integer>, m: Mob) -> integer {\n  v[0] + m.hp\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("run");
    assert!(matches!(
        s.eval("run([10, 20, 30], Mob { hp: 7 })"),
        Eval::Paused
    ));
    assert_eq!(
        s.debug_eval("v").as_deref(),
        Some("[10,20,30]"),
        "bare vector — live read (the case the reconstruct path faulted on)"
    );
    assert_eq!(
        s.debug_eval("m").as_deref(),
        Some("Mob{hp:7}"),
        "bare struct — live read"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 M1a (interactive) — **whole-value heap edit**: replace a struct local with
/// a freshly-constructed one at the pause (`pt = Point{...}`), then resume and observe
/// the program use the new value.  The value is built on a throwaway State above the
/// live store's high-water and grafted into the paused stores with no `DbRef` remap.
/// `use_pt(Point{1,2})` is normally 3; editing `pt` to `{10,20}` makes it 30, so a
/// clean continue (the `assert(==30)` holds) proves the resumed program read the edit.
#[test]
fn repl_interactive_edit_whole_struct() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "fn use_pt(pt: Point) -> integer {\n  pt.x + pt.y\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("use_pt");
    assert!(matches!(
        s.eval("assert(use_pt(Point { x: 1, y: 2 }) == 30, \"edited\")"),
        Eval::Paused
    ));
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("1"), "pre-edit pt.x");
    // The whole-struct edit: build a fresh Point and point the frame local at it.
    assert!(
        s.debug_set("pt", "Point { x: 10, y: 20 }"),
        "whole-struct edit"
    );
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("10"), "edited pt.x");
    assert_eq!(s.debug_eval("pt.y").as_deref(), Some("20"), "edited pt.y");
    // Continue → the assert(==30) holds only with the edit, so a clean finish proves
    // the resumed program used the materialised value.
    assert!(
        !s.debug_continue(),
        "finishes — assert(==30) holds with the edit"
    );
    assert!(!s.is_debugging());
}

/// @PLN16 M1a (interactive) — the heap-edit **matrix**: nested (inlined) struct, a
/// vector local, and a struct with a text field.  Each shape is built fresh at the
/// pause and read back via `debug_eval`; a second untouched heap local in the same
/// frame is asserted intact, proving the graft leaves the suspended frame's other
/// values alone (store-level adoption above the high-water, not a frame rewrite).
#[test]
fn repl_interactive_edit_whole_value_matrix() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "struct Line { a: Point, b: Point }",
        "struct Tagged { name: text, n: integer }",
        // Two heap locals: `lead` is edited, `keep` must stay intact across the edit.
        "fn shapes(ln: Line, keep: Point) -> integer {\n  ln.a.x + keep.x\n}",
        "fn vecfn(v: vector<integer>, keep: Point) -> integer {\n  v[0] + keep.x\n}",
        "fn textfn(t: Tagged, keep: Point) -> integer {\n  t.n + keep.x\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);

    // --- nested (inlined) struct ---
    s.add_breakpoint("shapes");
    assert!(matches!(
        s.eval("shapes(Line { a: Point { x: 1, y: 2 }, b: Point { x: 3, y: 4 } }, Point { x: 7, y: 8 })"),
        Eval::Paused
    ));
    assert!(
        s.debug_set(
            "ln",
            "Line { a: Point { x: 100, y: 2 }, b: Point { x: 3, y: 4 } }"
        ),
        "nested struct edit"
    );
    assert_eq!(
        s.debug_eval("ln.a.x").as_deref(),
        Some("100"),
        "nested field"
    );
    assert_eq!(
        s.debug_eval("keep.x").as_deref(),
        Some("7"),
        "other local intact"
    );
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- vector ---
    s.add_breakpoint("vecfn");
    assert!(matches!(
        s.eval("vecfn([10, 20, 30], Point { x: 7, y: 8 })"),
        Eval::Paused
    ));
    assert!(s.debug_set("v", "[40, 50, 60]"), "vector edit");
    assert_eq!(
        s.debug_eval("v[0]").as_deref(),
        Some("40"),
        "edited element"
    );
    assert_eq!(
        s.debug_eval("v[2]").as_deref(),
        Some("60"),
        "edited element"
    );
    assert_eq!(
        s.debug_eval("keep.x").as_deref(),
        Some("7"),
        "other local intact"
    );
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- struct with a text field ---
    s.add_breakpoint("textfn");
    assert!(matches!(
        s.eval("textfn(Tagged { name: \"a\", n: 5 }, Point { x: 7, y: 8 })"),
        Eval::Paused
    ));
    assert!(
        s.debug_set("t", "Tagged { name: \"hello\", n: 9 }"),
        "struct-with-text edit"
    );
    assert_eq!(
        s.debug_eval("t.n").as_deref(),
        Some("9"),
        "edited scalar field"
    );
    assert_eq!(
        s.debug_eval("t.name").as_deref(),
        Some("\"hello\""),
        "edited text field"
    );
    assert_eq!(
        s.debug_eval("keep.x").as_deref(),
        Some("7"),
        "other local intact"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 M1a (interactive) — a whole-value heap edit whose constructor **references
/// frame locals** (`Point{x: pt.y, y: pt.x}` swaps the live fields), and clean
/// **rejection** of a malformed / type-mismatched heap edit (no corruption; the
/// session stays paused and usable).  The frame-reference path proves `debug_eval`
/// resolves the RHS against the frame to a self-contained literal before the build.
#[test]
fn repl_interactive_edit_whole_struct_frame_ref_and_reject() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "fn use_pt(pt: Point) -> integer {\n  pt.x * 10 + pt.y\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("use_pt");
    // use_pt(Point{3,4}) = 34; swapping to Point{4,3} gives 43.
    assert!(matches!(
        s.eval("assert(use_pt(Point { x: 3, y: 4 }) == 43, \"swapped\")"),
        Eval::Paused
    ));
    // Constructor references the live frame fields (resolved by debug_eval).
    assert!(
        s.debug_set("pt", "Point { x: pt.y, y: pt.x }"),
        "frame-referencing whole-struct edit"
    );
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("4"), "swapped x");
    assert_eq!(s.debug_eval("pt.y").as_deref(), Some("3"), "swapped y");
    // An unknown reference (rejected when `debug_eval` can't resolve the RHS) and a
    // type-mismatched RHS (rejected when the typed build won't compile) are both clean
    // — no write, the value stays the swapped one, the session stays paused.  (A
    // partial constructor like `Point{x:1}` is *valid* loft — y defaults — so it is
    // not a rejection case.)
    assert!(!s.debug_set("pt", "no_such_local"), "unknown ref rejected");
    assert!(!s.debug_set("pt", "42"), "scalar-for-struct rejected");
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("4"),
        "unchanged after reject"
    );
    assert!(s.is_debugging(), "still paused after rejects");
    assert!(
        !s.debug_continue(),
        "finishes — assert(==43) holds with the swap"
    );
    assert!(!s.is_debugging());
}

/// @PLN16 M2 (interactive) — **undo/redo** mechanics on a scalar local: empty-stack
/// no-op, undo restores the pre-edit value, redo re-applies it, a stack of edits
/// unwinds/rewinds in order, and a fresh edit after an undo **forks** the timeline
/// (clears redo).  All within one suspension (undo is per-pause-point).
#[test]
fn repl_interactive_undo_redo_scalar() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn calc(n: integer) -> integer {\n  n * 10\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("calc");
    assert!(matches!(s.eval("calc(5)"), Eval::Paused));
    assert_eq!(s.debug_eval("n").as_deref(), Some("5"));
    // Nothing recorded yet.
    assert!(!s.debug_undo(), "nothing to undo initially");
    assert!(!s.debug_redo(), "nothing to redo initially");
    // One edit, undo, redo.
    assert!(s.debug_set("n", "99"));
    assert_eq!(s.debug_eval("n").as_deref(), Some("99"));
    assert!(s.debug_undo(), "undo");
    assert_eq!(s.debug_eval("n").as_deref(), Some("5"), "reverted");
    assert!(s.debug_redo(), "redo");
    assert_eq!(s.debug_eval("n").as_deref(), Some("99"), "re-applied");
    // Stack of edits unwinds and rewinds in order.
    assert!(s.debug_set("n", "7"));
    assert!(s.debug_set("n", "8"));
    assert_eq!(s.debug_eval("n").as_deref(), Some("8"));
    assert!(s.debug_undo());
    assert_eq!(s.debug_eval("n").as_deref(), Some("7"), "undo to 7");
    assert!(s.debug_undo());
    assert_eq!(s.debug_eval("n").as_deref(), Some("99"), "undo to 99");
    assert!(s.debug_redo());
    assert_eq!(s.debug_eval("n").as_deref(), Some("7"), "redo to 7");
    // A new edit after an undo forks the timeline: the redo (→ 8) is dropped.
    assert!(s.debug_set("n", "100"));
    assert!(!s.debug_redo(), "fresh edit cleared the redo stack");
    assert_eq!(s.debug_eval("n").as_deref(), Some("100"));
    assert!(!s.debug_continue());
}

/// @PLN16 M2 (interactive) — an undo's revert reaches **resume**: edit `n` to 99,
/// undo it, continue; the `assert(== 50)` holds only if the resumed call used the
/// reverted `n == 5` (a failed undo would leave 99 → 990 → the assert would panic).
#[test]
fn repl_interactive_undo_resumes_with_reverted_value() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn calc(n: integer) -> integer {\n  n * 10\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("calc");
    assert!(matches!(
        s.eval("assert(calc(5) == 50, \"undone resumes to original\")"),
        Eval::Paused
    ));
    assert!(s.debug_set("n", "99"));
    assert!(s.debug_undo(), "undo the edit");
    assert!(
        !s.debug_continue(),
        "finishes — assert(==50) holds with the revert"
    );
    assert!(!s.is_debugging());
}

/// @PLN16 M2 (interactive) — undo/redo across the **heap edit kinds**: a scalar
/// struct field (`pt.x`), a scalar vector element (`v[i]`), a struct field on a
/// text-bearing struct (the text must stay intact), and a **whole-value** replacement
/// (`pt = Point{…}`).  Each reverts to its pre-edit value and re-applies — proving the
/// journaled `Modify` (and, for whole-value, the frame-slot `DbRef` swap) round-trips.
#[test]
fn repl_interactive_undo_redo_heap_kinds() {
    let mut s = session();
    for d in [
        "struct Point { x: integer, y: integer }",
        "struct Tagged { name: text, n: integer }",
        "fn field_fn(pt: Point) -> integer {\n  pt.x\n}",
        "fn vec_fn(v: vector<integer>) -> integer {\n  v[0]\n}",
        "fn text_fn(t: Tagged) -> integer {\n  t.n\n}",
        "fn whole_fn(pt: Point) -> integer {\n  pt.x + pt.y\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);

    // --- scalar struct field ---
    s.add_breakpoint("field_fn");
    assert!(matches!(
        s.eval("field_fn(Point { x: 1, y: 2 })"),
        Eval::Paused
    ));
    assert!(s.debug_set("pt.x", "9"));
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("9"));
    assert!(s.debug_undo());
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("1"), "field reverted");
    assert!(s.debug_redo());
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("9"),
        "field re-applied"
    );
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- scalar vector element ---
    s.add_breakpoint("vec_fn");
    assert!(matches!(s.eval("vec_fn([10, 20, 30])"), Eval::Paused));
    assert!(s.debug_set("v[0]", "99"));
    assert_eq!(s.debug_eval("v[0]").as_deref(), Some("99"));
    assert!(s.debug_undo());
    assert_eq!(
        s.debug_eval("v[0]").as_deref(),
        Some("10"),
        "element reverted"
    );
    assert!(s.debug_redo());
    assert_eq!(
        s.debug_eval("v[0]").as_deref(),
        Some("99"),
        "element re-applied"
    );
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- scalar field on a text-bearing struct: text must survive undo/redo ---
    s.add_breakpoint("text_fn");
    assert!(matches!(
        s.eval("text_fn(Tagged { name: \"a\", n: 5 })"),
        Eval::Paused
    ));
    assert!(s.debug_set("t.n", "9"));
    assert!(s.debug_undo());
    assert_eq!(s.debug_eval("t.n").as_deref(), Some("5"), "n reverted");
    assert_eq!(
        s.debug_eval("t.name").as_deref(),
        Some("\"a\""),
        "text intact"
    );
    assert!(s.debug_redo());
    assert_eq!(s.debug_eval("t.n").as_deref(), Some("9"), "n re-applied");
    assert!(!s.debug_continue());
    s.clear_breakpoints();

    // --- whole-value replacement ---
    s.add_breakpoint("whole_fn");
    assert!(matches!(
        s.eval("whole_fn(Point { x: 1, y: 2 })"),
        Eval::Paused
    ));
    assert!(s.debug_set("pt", "Point { x: 10, y: 20 }"));
    assert_eq!(s.debug_eval("pt.x").as_deref(), Some("10"));
    assert!(s.debug_undo());
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("1"),
        "whole-value reverted"
    );
    assert_eq!(
        s.debug_eval("pt.y").as_deref(),
        Some("2"),
        "whole-value reverted"
    );
    assert!(s.debug_redo());
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("10"),
        "whole-value re-applied"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 M2 (interactive) — undo/redo of a **text** edit (the trickiest case: the
/// 16-byte `Str` slot is restored by a raw-byte `Modify`, so undo must repoint it at
/// the original argument text and redo back at the debugger-owned buffer, with no
/// double-free).  Edit a text argument, undo to the original, redo to the edit.
#[test]
fn repl_interactive_undo_redo_text() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn greet(msg: text) -> integer {\n  msg.len()\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("greet");
    assert!(matches!(s.eval("greet(\"hi\")"), Eval::Paused));
    assert_eq!(s.debug_eval("msg").as_deref(), Some("\"hi\""));
    assert!(s.debug_set("msg", "\"hello\""), "text edit");
    assert_eq!(s.debug_eval("msg").as_deref(), Some("\"hello\""));
    assert!(s.debug_undo(), "undo text edit");
    assert_eq!(
        s.debug_eval("msg").as_deref(),
        Some("\"hi\""),
        "text reverted"
    );
    assert!(s.debug_redo(), "redo text edit");
    assert_eq!(
        s.debug_eval("msg").as_deref(),
        Some("\"hello\""),
        "text re-applied"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 G1 (interactive) — edit-and-continue across **scalar types**, not just
/// integers: a live `integer` / `float` / `boolean` local is each edited at the
/// pause (one by literal, one by an expression evaluated against the frame), the
/// reads reflect the edits, and a **text** argument is editable too (its `Str` slot
/// repoints at a debugger-owned buffer), with `debug_eval` confirming the new value.
#[test]
fn repl_interactive_edit_scalar_types() {
    let mut s = session();
    assert!(matches!(
        s.eval(
            "fn mix(n: integer, f: float, b: boolean, msg: text) -> float {\n  \
             if b { n * f } else { 0.0 }\n}"
        ),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("mix");
    assert!(matches!(s.eval("mix(2, 1.5, true, \"hi\")"), Eval::Paused));
    // integer (literal), float (literal), boolean (expression `!b`).
    assert!(s.debug_set("n", "10"), "int edit");
    assert!(s.debug_set("f", "2.0"), "float edit");
    assert!(s.debug_set("b", "!b"), "bool expr edit (true → false)");
    assert_eq!(s.debug_eval("n").as_deref(), Some("10"));
    assert_eq!(s.debug_eval("f").as_deref(), Some("2.0"));
    assert_eq!(s.debug_eval("b").as_deref(), Some("false"));
    // A type-mismatched edit is rejected (float literal into an integer local).
    assert!(!s.debug_set("n", "3.5"), "type-mismatched edit rejected");
    assert_eq!(s.debug_eval("n").as_deref(), Some("10"), "n unchanged");
    // A text argument is editable via a literal: its `Str` slot repoints at a
    // stable buffer, and the edit is visible both in the frame view and via eval.
    assert!(s.debug_set("msg", "\"bye\""), "text arg edit");
    assert_eq!(
        s.debug_eval("msg").as_deref(),
        Some("\"bye\""),
        "msg reflects edit"
    );
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

/// @P293 — value-snapshotting a **text** binding must not crash.  Re-binding a text
/// *variable* (`y = t`) or a *concat* (`z = t + "!"`) once double-freed the buffer
/// the synthetic capture fn returned (the entry-fn text return borrowed a local
/// `String` freed on teardown).  The capture now routes text through a store-resident
/// single-element vector, so the value is snapshotted correctly and re-binding works.
#[test]
fn text_binding_capture_does_not_crash() {
    let mut s = session();
    assert!(matches!(s.eval("t = \"hi\""), Eval::Ran));
    // bare variable read
    assert!(matches!(s.eval("y = t"), Eval::Ran));
    assert!(matches!(
        s.eval("assert(y == \"hi\", \"y == t\")"),
        Eval::Ran
    ));
    // concat (a borrowed work-text — the second crash shape)
    assert!(matches!(s.eval("z = t + \"!\""), Eval::Ran));
    assert!(matches!(
        s.eval("assert(z == \"hi!\", \"z == t + !\")"),
        Eval::Ran
    ));
    // chained rebind keeps the value
    assert!(matches!(s.eval("w = z"), Eval::Ran));
    assert!(matches!(
        s.eval("assert(w == \"hi!\", \"w == z\")"),
        Eval::Ran
    ));
}

/// @P293 — the value-snapshot API and the debugger's eval-at-frame both render a
/// text value (a variable, a concat, a borrowed `text[self]`) without crashing.
#[test]
fn value_of_renders_text_expressions() {
    let mut s = session();
    assert!(matches!(s.eval("msg = \"hi\""), Eval::Ran));
    assert_eq!(s.value_of("msg").as_deref(), Some("\"hi\""));
    assert_eq!(s.value_of("msg + \"!\"").as_deref(), Some("\"hi!\""));
    assert_eq!(s.value_of("msg.to_uppercase()").as_deref(), Some("\"HI\""));
    // eval-at-frame on a text argument (the @PLN16 debugger surface)
    assert!(matches!(
        s.eval("fn g(m: text) -> integer {\n  if m == \"x\" { 1 } else { 0 }\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("g");
    assert!(matches!(s.eval("g(\"hi\")"), Eval::Paused));
    assert_eq!(s.debug_eval("m").as_deref(), Some("\"hi\""));
    assert_eq!(s.debug_eval("m + \"!\"").as_deref(), Some("\"hi!\""));
    assert!(!s.debug_continue());
}

/// @PLN16.J — a **struct field** edited at the `(dbg)` prompt (`pt.x = 5`) routes
/// through `debug_set` to the in-place field write, and the refreshed frame reflects
/// it.  `debug_set` evaluates the RHS against the frame first, so an expression RHS
/// (`pt.x = pt.y + 1`) works too.
#[test]
fn repl_interactive_edit_struct_field() {
    let mut s = session();
    assert!(matches!(
        s.eval("struct Point { x: integer, y: integer }"),
        Eval::Ran
    ));
    assert!(matches!(
        s.eval("fn area(pt: Point) -> integer {\n  m = pt.x;\n  pt.x * pt.y\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("area");
    assert!(matches!(s.eval("area(Point { x: 3, y: 4 })"), Eval::Paused));

    assert!(s.debug_set("pt.x", "5"), "literal field edit");
    assert_eq!(
        s.debug_eval("pt.x").as_deref(),
        Some("5"),
        "field reflects edit"
    );
    assert!(s.debug_set("pt.y", "pt.x + 1"), "expression field edit");
    assert_eq!(s.debug_eval("pt.y").as_deref(), Some("6"), "y = x + 1");
    // a bad field is rejected, frame intact
    assert!(!s.debug_set("pt.z", "9"), "unknown field rejected");
    assert!(!s.debug_continue());
}

/// @PLN16.J (M1b) — a **nested** struct path edited at the `(dbg)` prompt
/// (`o.inner.a = 9`) routes through `debug_set` → `set_frame_path` and the refreshed
/// frame reflects it.
#[test]
fn repl_interactive_edit_nested_field() {
    let mut s = session();
    assert!(matches!(
        s.eval("struct Inner { a: integer, b: integer }"),
        Eval::Ran
    ));
    assert!(matches!(
        s.eval("struct Outer { n: integer, inner: Inner }"),
        Eval::Ran
    ));
    assert!(matches!(
        s.eval("fn f(o: Outer) -> integer {\n  m = o.n;\n  o.inner.a\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("f");
    assert!(matches!(
        s.eval("f(Outer { n: 1, inner: Inner { a: 5, b: 6 } })"),
        Eval::Paused
    ));
    assert!(s.debug_set("o.inner.a", "9"), "nested path edit");
    assert_eq!(
        s.debug_eval("o.inner.a").as_deref(),
        Some("9"),
        "nested field reflects edit"
    );
    assert_eq!(
        s.debug_eval("o.inner.b").as_deref(),
        Some("6"),
        "sibling untouched"
    );
    assert!(!s.debug_continue());
}

/// @PLN16 M5a (file-run debugger) — load a real program under its own filename, break
/// at `file:line`, run `main()`, and pause in the right function.  Proves the file:line
/// breakpoint resolves to the **user file's** function (not a stdlib function that
/// happens to share the line number), reads the live local, and resumes cleanly.
#[test]
fn repl_file_debugger_breaks_at_file_line() {
    let mut s = session();
    // Line 2 (`n * 2`) is inside `helper`; line 5 calls it from `main`.
    let prog = "fn helper(n: integer) -> integer {\n  \
                n * 2\n}\nfn main() {\n  \
                a = helper(21);\n  \
                assert(a == 42, \"ok\")\n}\n";
    assert!(
        s.load_program_str(prog, "prog.loft").is_ok(),
        "program loads"
    );
    assert!(s.defines_function("main"), "main is defined");
    assert!(
        s.breakable_lines_in_file("prog.loft").contains(&2),
        "line 2 is breakable: {:?}",
        s.breakable_lines_in_file("prog.loft")
    );
    s.debug_stepping(true);
    s.add_file_breakpoint("prog.loft", 2);
    // Running main() pauses inside helper at line 2 — scoped to prog.loft, so stdlib's
    // own line 2 is never a breakpoint.
    assert!(
        matches!(s.eval("main()"), Eval::Paused),
        "paused at prog.loft:2"
    );
    let f = s.paused_frame().expect("frame");
    assert_eq!(
        f.function, "helper",
        "paused in the user's helper, not stdlib"
    );
    assert_eq!(s.debug_eval("n").as_deref(), Some("21"), "helper's live n");
    // Continue with the original value → assert(42 == 42) holds → clean finish.
    assert!(!s.debug_continue(), "run finishes cleanly");
    assert!(!s.is_debugging());
}

/// @PLN16 M5a — a `file:line` with no breakable code resolves to nothing (the file-run
/// debugger reports it + hints), and a line in a different file (the stdlib) is never
/// matched.
#[test]
fn repl_file_debugger_unbreakable_line() {
    let mut s = session();
    let prog = "fn main() {\n  a = 1;\n  print(\"{a}\")\n}\n";
    assert!(s.load_program_str(prog, "prog.loft").is_ok());
    // Line 4 is the closing `}` — no emitted code → not breakable.
    assert!(
        !s.breakable_lines_in_file("prog.loft").contains(&4),
        "line 4 (the `}}`) is not breakable"
    );
    s.debug_stepping(true);
    s.add_file_breakpoint("prog.loft", 4);
    // The breakpoint silently doesn't resolve, so the run finishes without pausing.
    assert!(
        matches!(s.eval("main()"), Eval::Ran),
        "no pause on an unbreakable line"
    );
    assert!(!s.is_debugging());
}

/// @PLN16 M5a — the full `loft debug <file>:<line>` flow through `run_file_debug`:
/// write a real `.loft` file, break at a line, auto-run `main()` to it, then drive the
/// interactive `(dbg)` prompt from piped input (read the frame, continue, quit).  The
/// pause announcement + frame go to the output stream; the run then resumes to the end.
#[test]
fn repl_file_debugger_end_to_end() {
    let path = tmp_session("filedebug");
    let prog = "fn helper(n: integer) -> integer {\n  \
                n * 2\n}\nfn main() {\n  \
                a = helper(21);\n  \
                assert(a == 42, \"ok\")\n}\n";
    std::fs::write(&path, prog).expect("write temp program");
    let file = path.to_str().unwrap();
    // Piped session: read `n` at the frame, continue to the end, quit.
    let input = std::io::Cursor::new(b"n\n:continue\n:quit\n".to_vec());
    let mut out: Vec<u8> = Vec::new();
    loft::repl::run_file_debug("default", file, 2, input, &mut out).expect("debug run");
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("break at"),
        "announces the breakpoint: {text}"
    );
    assert!(
        text.contains("paused in helper") && text.contains("n = 21"),
        "pauses in helper with n = 21: {text}"
    );
    assert!(text.contains("resumed"), "continues to the end: {text}");
    let _ = std::fs::remove_file(&path);
}

/// @PLN16 M3 (watchpoints) — set a watch on a struct field, resume, and the run pauses
/// the moment a later write changes it (reporting old → new).  Proves the per-op poll
/// in the resume loop catches an in-place heap mutation and stops one op past the write.
#[test]
fn repl_watchpoint_fires_on_field_change() {
    let mut s = session();
    for d in [
        "struct Counter { n: integer }",
        "fn bump(c: Counter) -> integer {\n  c.n = c.n + 1;\n  c.n = c.n + 10;\n  c.n\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("bump"); // first body line, before `c.n = c.n + 1`
    assert!(matches!(s.eval("bump(Counter { n: 5 })"), Eval::Paused));
    assert_eq!(s.debug_eval("c.n").as_deref(), Some("5"), "c.n starts at 5");
    assert!(s.add_watchpoint("c.n"), "watch the field");
    assert_eq!(s.watchpoints(), vec!["c.n".to_string()], "watch registered");
    // Continue → the run pauses when `c.n = c.n + 1` changes it to 6.
    assert!(s.debug_continue(), "watch fired → paused again");
    let hit = s.take_watch_hit().expect("first watch hit");
    assert_eq!(
        (hit.label.as_str(), hit.old.as_str(), hit.new.as_str()),
        ("c.n", "5", "6"),
        "reports the change"
    );
    assert!(s.take_watch_hit().is_none(), "hit is taken once");
    // Continue → `c.n = c.n + 10` fires it again (6 → 16).
    assert!(s.debug_continue(), "second change fires the watch");
    let hit2 = s.take_watch_hit().expect("second watch hit");
    assert_eq!((hit2.old.as_str(), hit2.new.as_str()), ("6", "16"));
    // Continue → no more writes; the run finishes.
    assert!(!s.debug_continue(), "run finishes");
    assert!(!s.is_debugging());
}

/// @PLN16 M3 — a watch on a **vector element** fires when that element is written, and
/// an unwatchable expression (a bare local) is rejected.
#[test]
fn repl_watchpoint_vector_element_and_reject() {
    let mut s = session();
    assert!(matches!(
        s.eval("fn edit(v: vector<integer>) -> integer {\n  v[0] = 99;\n  v[0]\n}"),
        Eval::Ran
    ));
    s.debug_stepping(true);
    s.add_breakpoint("edit");
    assert!(matches!(s.eval("edit([10, 20, 30])"), Eval::Paused));
    // A bare local is a stack slot — not a watchable region.
    assert!(!s.add_watchpoint("v"), "bare local rejected");
    assert!(s.add_watchpoint("v[0]"), "element watchable");
    assert!(s.debug_continue(), "v[0] = 99 fires the watch");
    let hit = s.take_watch_hit().expect("watch hit");
    assert_eq!(
        (hit.label.as_str(), hit.old.as_str(), hit.new.as_str()),
        ("v[0]", "10", "99")
    );
    assert!(!s.debug_continue());
}

/// @PLN16 rich-bp — a **conditional breakpoint** (`:break f if n == 3`) breaks only on
/// the call where the condition holds, silently auto-resuming past the others — the
/// "fails on one call in N" case.
#[test]
fn repl_conditional_breakpoint_breaks_only_when_true() {
    let mut s = session();
    for d in [
        "fn f(n: integer) -> integer {\n  n + 100\n}",
        "fn run() -> integer {\n  f(1);\n  f(2);\n  f(3);\n  f(4)\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_breakpoint("f if n == 3");
    // f(1)/f(2) auto-resume (condition false); the run suspends at f(3).
    assert!(
        matches!(s.eval("run()"), Eval::Paused),
        "paused only at f(3)"
    );
    assert_eq!(
        s.debug_eval("n").as_deref(),
        Some("3"),
        "broke at the matching call"
    );
    // f(4)'s condition is false → no further break → the run finishes.
    assert!(!s.debug_continue(), "no further break");
    assert!(!s.is_debugging());
}

/// @PLN16 rich-bp — a **tracepoint** (`:trace f n`) logs the expression on each hit and
/// **continues** (never pauses); the emitted lines are collected for the driver.
#[test]
fn repl_tracepoint_logs_and_continues() {
    let mut s = session();
    for d in [
        "fn f(n: integer) -> integer {\n  n + 100\n}",
        "fn run() -> integer {\n  f(1);\n  f(2)\n}",
    ] {
        assert!(matches!(s.eval(d), Eval::Ran), "def {d:?}");
    }
    s.debug_stepping(true);
    s.add_tracepoint("f n");
    // The tracepoint logs n at each f call and the run continues to the end.
    assert!(
        matches!(s.eval("run()"), Eval::Ran),
        "a tracepoint never pauses"
    );
    assert!(!s.is_debugging());
    assert_eq!(
        s.take_trace_output(),
        vec!["n = 1".to_string(), "n = 2".to_string()],
        "logged both calls"
    );
}

/// STATUS: OPEN — these two are `#[ignore]`d reproductions, not passing guards.
///
/// @P293's remedy (wrap the value in a one-element vector so it is COPIED into
/// store-resident memory) does NOT extend one level up to a vector: capturing
/// `[(v)]` as `vector<vector<τ>>` moves the panic from `keys.rs` to
/// `database/types.rs:71` — `Double structure type src0::main_vector<vector<integer>>`.
/// That ties #618's main signature to the "possibly related second signature" in the
/// report (`v = [9000000000, 0]` faulting at the same `types.rs` site): they are ONE
/// blocker.  Each REPL capture re-parses a synthetic program into the same `Data`, so
/// it re-registers the `main_vector<…>` wrapper struct; the collision-qualified name
/// (`src0::…`) collides too, because both defs live in the same source.  A real fix has
/// to make those synthetic defs unique per capture generation (or dedup them), which is
/// the fn-return / borrowed-heap-value substrate the report names — not a call-site wrap.
///
/// #618 regression: `ReplSession::value_of` panicked for any **vector** binding.
///
/// `capture_typed` runs `fn replmain_N() -> vector<τ> { … v }`, i.e. it returns a
/// bare heap value that borrows a body local.  The fn-return deep-copy then
/// targets a store the capture generation never allocated, so the destination
/// `DbRef` is the null sentinel and `append_vector` indexes `allocations[65535]`
/// — `index out of bounds: the len is 2 but the index is 65535` at `keys.rs`.
/// A struct binding was unaffected; a vector faulted on a single bind.
///
/// Same family as @P293 (which hit this for `text`), and the same remedy: wrap
/// the value in a one-element vector so it is COPIED into store-resident memory,
/// then strip the outer brackets.  The in-tree caller (`frame_value_of`) guards
/// with `catch_unwind`, but an embedder calling the public `value_of` directly
/// has no such guard and the process aborts — so this must not merely degrade to
/// `None`, it must return the value.
#[test]
fn value_of_renders_vector_bindings_618() {
    let mut s = session();
    assert!(matches!(s.eval("v = [1, 2, 3]"), Eval::Ran));
    assert_eq!(s.value_of("v").as_deref(), Some("[1,2,3]"));

    // Re-binding was in the reported boundary table too.
    assert!(matches!(s.eval("v = [4, 5]"), Eval::Ran));
    assert_eq!(s.value_of("v").as_deref(), Some("[4,5]"));

    // Two separate vector bindings — reading the second used to panic as well.
    let mut t = session();
    assert!(matches!(t.eval("a = [1, 2]"), Eval::Ran));
    assert!(matches!(t.eval("b = [3, 4]"), Eval::Ran));
    assert_eq!(t.value_of("b").as_deref(), Some("[3,4]"));
    assert_eq!(t.value_of("a").as_deref(), Some("[1,2]"));
}

/// #618 — the same capture path for a `vector<text>` and for an element type
/// wider than 32 bits.  The report's "possibly related second signature" was a
/// `v = [9000000000, 0]` binding panicking at `database/types.rs` instead of
/// `keys.rs`; both are the fn-return copy, so guard the wide element here too.
#[test]
fn value_of_renders_text_and_wide_vector_bindings_618() {
    let mut s = session();
    assert!(matches!(s.eval("vt = [\"a\", \"b\"]"), Eval::Ran));
    assert_eq!(s.value_of("vt").as_deref(), Some("[\"a\",\"b\"]"));

    let mut w = session();
    assert!(matches!(w.eval("vw = [9000000000, 0]"), Eval::Ran));
    assert_eq!(w.value_of("vw").as_deref(), Some("[9000000000,0]"));
}

/// #618 — a struct binding was already correct; keep it in the guard so the
/// vector wrap can't be "fixed" by regressing the path that worked.
#[test]
fn value_of_struct_binding_still_renders_618() {
    let mut s = session();
    assert!(matches!(s.eval("struct P { x: integer }"), Eval::Ran));
    assert!(matches!(s.eval("p = P { x: 1 }"), Eval::Ran));
    assert_eq!(s.value_of("p").as_deref(), Some("P{x:1}"));
// ── FU.3: one typo must produce one message ─────────────────────────────────

/// A failed input reported diagnostics about the *replayed session body* as if
/// the user had just typed them: `prnt("hi")` after `x = 5` answered with the
/// real error **and** `Variable x is never read`, which is false — `x` was read,
/// by the very input that failed to compile.  Every prelude line clamps onto the
/// user's line 1, so N earlier bindings produced N false warnings all pointing
/// at the same wrong column.
#[test]
fn a_failed_input_does_not_warn_about_the_replayed_session() {
    let mut s = session();
    assert!(matches!(s.eval("x = 5"), Eval::Ran));
    assert!(matches!(s.eval("y = 6"), Eval::Ran));
    assert!(matches!(s.eval("z = 7"), Eval::Ran));

    let Eval::Error(diags) = s.eval("prnt(\"hi\")") else {
        panic!("a typo'd function name must be an error");
    };

    // The real error survives — dropping it would leave the REPL rejecting the
    // input while saying nothing.
    assert!(
        diags
            .iter()
            .any(|d| d.level >= loft::diagnostics::Level::Error),
        "the error itself must still reach the user; got {diags:?}"
    );
    // ...and it is the ONLY thing reported.
    let noise: Vec<String> = diags
        .iter()
        .filter(|d| d.level < loft::diagnostics::Level::Error)
        .map(loft::diagnostics::DiagEntry::to_string_compact)
        .collect();
    assert!(
        noise.is_empty(),
        "a typo must not warn about earlier, correct bindings; got {noise:?}"
    );

    // The session is unharmed: x really was readable all along.
    assert!(
        matches!(
            s.eval("assert(x + y + z == 18, \"bindings survived\")"),
            Eval::Ran
        ),
        "the replayed bindings must still be live after the failed input"
    );}
