// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN15 debugger — vertical tracer-bullet slice.
//!
//! The smallest end-to-end proof that the interpreter can **pause at a breakpoint
//! and read the live frame**: set a breakpoint on a source line inside a function
//! body, run a call, and assert the captured frame holds the argument the caller
//! passed. Everything the debugger needs (the REPL-at-frame, stepping, conditional
//! breaks) builds on this one capability, so this is the slice that de-risks the
//! whole plan.
//!
//! The slice captures **arguments** — always live at any body point. Non-argument
//! locals need liveness-gating (only show a local once its assignment has run),
//! which is the next slice. Built test-first; each test here is the next slice's
//! spec.

use loft::compile;
use loft::debugger::StepMode;
use loft::parser::{ParseResult, Parser};
use loft::repl::{Eval, ReplSession};
use loft::state::State;

fn repl() -> Parser {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p
}

/// Define `def` then run `call` with a breakpoint on source `line`, returning the
/// captured frames.  `line` is relative to `def`'s own text (line 1 = its first
/// line), so a body line is post-prologue — where args sit in their slots.
fn run_with_breakpoint(
    p: &mut Parser,
    defs: &[&str],
    call: &str,
    bp_fn: &str,
    line: u32,
) -> Vec<loft::debugger::BreakHit> {
    for def in defs {
        match p.parse_statement(def) {
            ParseResult::Ready { .. } => {}
            other => panic!("def parse of {def:?} failed: {other:?}"),
        }
        let mut warm = State::new(p.database.clone());
        loft::scopes::check(&mut p.data); // assign slots (locals need it)
        compile::byte_code(&mut warm, &mut p.data);
    }
    let entry = match p.parse_statement(call) {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call parse of {call:?} failed: {other:?}"),
    };
    let mut state = State::new(p.database.clone());
    // Assign slots before codegen (the REPL paths do this; bare expressions get
    // away without it, but a struct-constructing call needs its work-ref slots).
    loft::scopes::check(&mut p.data);
    compile::byte_code(&mut state, &mut p.data);
    let d_nr = p.data.def_nr(&format!("n_{bp_fn}"));
    assert!(d_nr != u32::MAX, "function {bp_fn} not defined");
    assert!(
        state.set_breakpoint_fn_line(d_nr, line, &p.data),
        "no breakpoint offset for {bp_fn} line {line}; breakable = {:?}",
        state.breakable_lines()
    );
    let name = p
        .data
        .def(entry)
        .name()
        .strip_prefix("n_")
        .expect("wrapper is n_-prefixed")
        .to_string();
    state.execute_argv(&name, &p.data, &[]);
    state.debug_hits().to_vec()
}

/// A breakpoint on a body line pauses there and captures the argument the caller
/// passed — the core "pause + read the live frame".
#[test]
fn breakpoint_on_body_line_captures_argument() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn dbl(n: integer) -> integer {\n  n + n\n}"],
        "dbl(42)",
        "dbl",
        2, // the `n + n` line
    );
    assert_eq!(hits.len(), 1, "breakpoint fired once: {hits:?}");
    assert_eq!(hits[0].function, "dbl", "frame is `dbl`: {hits:?}");
    assert!(
        hits[0].locals.iter().any(|(n, v)| n == "n" && v == "42"),
        "arg n == 42 captured: {hits:?}"
    );
}

/// The breakpoint fires once per call — two calls, two captured frames, each with
/// its own argument value.
#[test]
fn breakpoint_fires_per_call() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn id(n: integer) -> integer {\n  n + 0\n}"],
        "id(7) + id(9)",
        "id",
        2,
    );
    assert_eq!(hits.len(), 2, "two calls → two hits: {hits:?}");
    let vals: Vec<&str> = hits
        .iter()
        .filter_map(|h| {
            h.locals
                .iter()
                .find(|(n, _)| n == "n")
                .map(|(_, v)| v.as_str())
        })
        .collect();
    assert!(
        vals.contains(&"7") && vals.contains(&"9"),
        "both args captured: {vals:?}"
    );
}

/// Multiple arguments of a frame are all captured with their passed values.
#[test]
fn breakpoint_captures_multiple_arguments() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn add(a: integer, b: integer) -> integer {\n  a + b\n}"],
        "add(40, 2)",
        "add",
        2,
    );
    assert_eq!(hits.len(), 1, "one hit: {hits:?}");
    let frame = &hits[0].locals;
    assert!(
        frame.iter().any(|(n, v)| n == "a" && v == "40"),
        "a==40: {frame:?}"
    );
    assert!(
        frame.iter().any(|(n, v)| n == "b" && v == "2"),
        "b==2: {frame:?}"
    );
}

/// A struct argument is captured as its full own-format value via `show_loft` —
/// the frame view covers heap types, not just scalars.
#[test]
fn breakpoint_captures_struct_argument() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &[
            "struct Point { x: integer, y: integer }",
            "fn area(pt: Point) -> integer {\n  pt.x * pt.y\n}",
        ],
        "area(Point { x: 3, y: 4 })",
        "area",
        2,
    );
    assert_eq!(hits.len(), 1, "one hit: {hits:?}");
    let pt = hits[0]
        .locals
        .iter()
        .find(|(n, _)| n == "pt")
        .map(|(_, v)| v.as_str());
    assert_eq!(
        pt,
        Some("Point{x:3,y:4}"),
        "struct arg rendered via show_loft: {hits:?}"
    );
}

/// Liveness-gating (Q6): a non-arg local assigned *before* the breakpoint is
/// captured with its value; one assigned *at/after* the breakpoint is not yet live
/// and is excluded (rather than read as zero/garbage).
#[test]
fn breakpoint_gates_locals_by_liveness() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn calc(n: integer) -> integer {\n  a = n + 1;\n  b = a * 2;\n  b\n}"],
        "calc(10)",
        "calc",
        3, // the `b = a * 2;` line — `a` is live, `b` is not yet
    );
    assert_eq!(hits.len(), 1, "{hits:?}");
    let frame = &hits[0].locals;
    // `n` (arg) and `a` (assigned on line 2) are live.
    assert!(
        frame.iter().any(|(n, _)| n == "n"),
        "arg n visible: {frame:?}"
    );
    assert!(
        frame.iter().any(|(n, v)| n == "a" && v == "11"),
        "local a == 11 (n+1) live: {frame:?}"
    );
    // `b` is assigned on line 3 — not yet live at the line-3 breakpoint.
    assert!(
        !frame.iter().any(|(n, _)| n == "b"),
        "local b not-yet-live, excluded: {frame:?}"
    );
}

/// @PLN15 D1 — the REPL-at-frame bridge: seed a session with a captured frame and
/// evaluate an expression against its variables.  Scalar frame on a fresh session.
#[test]
fn repl_at_frame_evaluates_scalar_frame() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn calc(n: integer) -> integer {\n  a = n + 1;\n  b = a * 2;\n  b\n}"],
        "calc(10)",
        "calc",
        3,
    );
    let hit = &hits[0]; // n = 10, a = 11
    let mut s = ReplSession::new("default").expect("stdlib");
    let bound = s.seed_frame(hit);
    assert!(bound >= 2, "n and a seeded: {bound}");
    // n + a == 21 holds → the frame variables are in scope with their live values.
    assert!(
        matches!(s.eval("assert(n + a == 21, \"frame eval\")"), Eval::Ran),
        "eval over frame vars ran"
    );
}

/// D1 over a heap frame: a struct value seeds and a field expression evaluates,
/// when the session carries the program's type definitions (built from its parser).
#[test]
fn repl_at_frame_evaluates_struct_frame() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &[
            "struct Point { x: integer, y: integer }",
            "fn area(pt: Point) -> integer {\n  pt.x * pt.y\n}",
        ],
        "area(Point { x: 3, y: 4 })",
        "area",
        2,
    );
    // The session must know `Point`, so build it over the program's parser.
    let hit = hits[0].clone(); // pt = Point{x:3,y:4}
    let mut s = ReplSession::from_parser(p);
    let bound = s.seed_frame(&hit);
    assert!(bound >= 1, "pt seeded: {bound}");
    assert!(
        matches!(
            s.eval("assert(pt.x * pt.y == 12, \"struct frame\")"),
            Eval::Ran
        ),
        "eval pt.x * pt.y ran"
    );
}

/// @PLN15 E — conditional / test breakpoint: a condition evaluated against each
/// captured frame selects which hits to keep ("break when `n > 1`").
#[test]
fn conditional_breakpoint_filters_by_frame_condition() {
    let mut p = repl();
    let hits = run_with_breakpoint(
        &mut p,
        &["fn step(n: integer) -> integer {\n  n * 10\n}"],
        "step(1) + step(2) + step(3)",
        "step",
        2,
    );
    assert_eq!(hits.len(), 3, "fires per call: {hits:?}");
    let mut s = ReplSession::from_parser(p);
    assert_eq!(
        hits.iter().filter(|h| s.frame_holds(h, "n > 1")).count(),
        2,
        "n > 1 holds for n = 2 and 3"
    );
    assert_eq!(
        hits.iter().filter(|h| s.frame_holds(h, "n > 100")).count(),
        0,
        "n > 100 holds for none"
    );
    assert_eq!(
        hits.iter().filter(|h| s.frame_holds(h, "n >= 1")).count(),
        3,
        "n >= 1 holds for all"
    );
}

/// @PLN15 F (the hard one) — change a value in the REPL at a breakpoint, then
/// resume and continue with the changed value.  `calc(5)` is normally 50; we edit
/// `n` to 99 at the breakpoint so it returns 990, and the program's assert
/// (`calc(5) == 990`) then passes — proving the edit was picked up on resume.
#[test]
fn step_picks_up_repl_edited_value() {
    let mut p = repl();
    match p.parse_statement("fn calc(n: integer) -> integer {\n  n * 10\n}") {
        ParseResult::Ready { .. } => {}
        other => panic!("def failed: {other:?}"),
    }
    let mut warm = State::new(p.database.clone());
    loft::scopes::check(&mut p.data);
    compile::byte_code(&mut warm, &mut p.data);
    // The program asserts the EDITED result (990); false without the edit (50).
    let entry = match p.parse_statement("assert(calc(5) == 990, \"edited n to 99\")") {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call failed: {other:?}"),
    };
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data);
    compile::byte_code(&mut state, &mut p.data);
    let calc = p.data.def_nr("n_calc");
    state.enable_stepping();
    assert!(
        state.set_breakpoint_fn_line(calc, 2, &p.data),
        "breakpoint set"
    );
    let name = p
        .data
        .def(entry)
        .name()
        .strip_prefix("n_")
        .unwrap()
        .to_string();

    // Run → suspends at calc's body (n == 5, not yet multiplied).
    state.execute_argv(&name, &p.data, &[]);
    assert!(state.is_paused(), "suspended at breakpoint");
    let hit = state.paused_frame().expect("frame").clone();
    assert!(
        hit.locals.iter().any(|(n, v)| n == "n" && v == "5"),
        "n == 5 at pause: {hit:?}"
    );

    // The user edits the value in a REPL seeded from the paused frame.
    let mut s = ReplSession::new("default").expect("stdlib");
    s.seed_frame(&hit);
    assert!(matches!(s.eval("n = 99"), Eval::Ran), "REPL edit n = 99");
    let edited: i64 = s.value_of("n").expect("value").parse().expect("int");
    assert_eq!(edited, 99);

    // Write the edited value back into the live frame, then resume.
    assert!(
        state.set_frame_value("n", edited, &p.data),
        "write-back n = 99"
    );
    state.resume();

    // calc(5) returned 99 * 10 == 990, so the program's assert held → no error.
    assert!(
        state.database.runtime_error.is_none(),
        "edit picked up on resume (assert calc(5)==990 passed); err = {:?}",
        state.database.runtime_error
    );
}

/// Run `defs` + `call` in **stepping mode** with a breakpoint on `bp_fn` line
/// `line`; return the `State` suspended there, for the test to drive `debug_step`.
fn run_to_pause(p: &mut Parser, defs: &[&str], call: &str, bp_fn: &str, line: u32) -> State {
    for def in defs {
        match p.parse_statement(def) {
            ParseResult::Ready { .. } => {}
            other => panic!("def parse of {def:?} failed: {other:?}"),
        }
        let mut warm = State::new(p.database.clone());
        loft::scopes::check(&mut p.data);
        compile::byte_code(&mut warm, &mut p.data);
    }
    let entry = match p.parse_statement(call) {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call parse of {call:?} failed: {other:?}"),
    };
    let mut state = State::new(p.database.clone());
    loft::scopes::check(&mut p.data);
    compile::byte_code(&mut state, &mut p.data);
    let d_nr = p.data.def_nr(&format!("n_{bp_fn}"));
    state.enable_stepping();
    assert!(
        state.set_breakpoint_fn_line(d_nr, line, &p.data),
        "breakpoint on {bp_fn}:{line}"
    );
    let name = p
        .data
        .def(entry)
        .name()
        .strip_prefix("n_")
        .unwrap()
        .to_string();
    state.execute_argv(&name, &p.data, &[]);
    assert!(state.is_paused(), "suspended at {bp_fn}:{line}");
    state
}

/// `outer` calls `inner`; line 2 of `outer` is the call, line 3 reads the result.
const NESTED: &[&str] = &[
    "fn inner(x: integer) -> integer {\n  x + 1\n}",
    "fn outer(n: integer) -> integer {\n  a = inner(n);\n  a + 100\n}",
];

/// Step *into*: from `outer`'s `inner(n)` line, descend into `inner`.
#[test]
fn step_into_descends_into_callee() {
    let mut p = repl();
    let mut state = run_to_pause(&mut p, NESTED, "outer(5)", "outer", 2);
    assert_eq!(state.paused_frame().unwrap().function, "outer");
    assert!(state.debug_step(StepMode::Into, &p.data), "still running");
    let f = state.paused_frame().unwrap();
    assert_eq!(f.function, "inner", "stepped into inner: {f:?}");
    assert!(
        f.locals.iter().any(|(n, v)| n == "x" && v == "5"),
        "inner's x == 5: {f:?}"
    );
}

/// Step *over*: run `inner(n)` to completion without pausing in it, then stop at
/// the next line in `outer` — where the result `a` is now live.
#[test]
fn step_over_runs_call_and_stays_in_frame() {
    let mut p = repl();
    let mut state = run_to_pause(&mut p, NESTED, "outer(5)", "outer", 2);
    assert!(state.debug_step(StepMode::Over, &p.data), "still running");
    let f = state.paused_frame().unwrap();
    assert_eq!(
        f.function, "outer",
        "stayed in outer (did not descend): {f:?}"
    );
    assert!(
        f.locals.iter().any(|(n, v)| n == "a" && v == "6"),
        "a == inner(5) == 6: {f:?}"
    );
}

/// Step *out*: from inside `inner`, run to its return and pause back in `outer`.
#[test]
fn step_out_returns_to_caller() {
    let mut p = repl();
    let mut state = run_to_pause(&mut p, NESTED, "outer(5)", "inner", 2);
    assert_eq!(state.paused_frame().unwrap().function, "inner");
    assert!(state.debug_step(StepMode::Out, &p.data), "still running");
    assert_eq!(
        state.paused_frame().unwrap().function,
        "outer",
        "stepped out to outer"
    );
}

/// Negative control: debugging on but no breakpoint registered → zero hits, and
/// the program still runs (the `assert` inside proves execution completed).
#[test]
fn debug_on_without_breakpoint_yields_no_hits() {
    let mut p = repl();
    match p.parse_statement("assert(max(3, 7) == 7, \"runs\")") {
        ParseResult::Ready { entry_def_nr } => {
            let mut state = State::new(p.database.clone());
            compile::byte_code(&mut state, &mut p.data);
            state.enable_debug(); // debugging on, but no breakpoint set
            let name = p
                .data
                .def(entry_def_nr)
                .name()
                .strip_prefix("n_")
                .unwrap()
                .to_string();
            state.execute_argv(&name, &p.data, &[]);
            assert!(state.debug_hits().is_empty(), "no breakpoint → no hits");
        }
        other => panic!("parse failed: {other:?}"),
    }
}
