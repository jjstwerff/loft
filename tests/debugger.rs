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
use loft::parser::{ParseResult, Parser};
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
