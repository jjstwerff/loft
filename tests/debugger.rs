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
fn run_with_line_breakpoint(
    p: &mut Parser,
    def: &str,
    call: &str,
    line: u32,
) -> Vec<loft::debugger::BreakHit> {
    match p.parse_statement(def) {
        ParseResult::Ready { .. } => {}
        other => panic!("def parse of {def:?} failed: {other:?}"),
    }
    let mut warm = State::new(p.database.clone());
    compile::byte_code(&mut warm, &mut p.data);

    let entry = match p.parse_statement(call) {
        ParseResult::Ready { entry_def_nr } => entry_def_nr,
        other => panic!("call parse of {call:?} failed: {other:?}"),
    };
    let mut state = State::new(p.database.clone());
    compile::byte_code(&mut state, &mut p.data);
    assert!(
        state.set_breakpoint_line(line),
        "no breakpoint offset for line {line}; breakable lines = {:?}",
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
    let hits = run_with_line_breakpoint(
        &mut p,
        "fn dbl(n: integer) -> integer {\n  n + n\n}",
        "dbl(42)",
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
    let hits = run_with_line_breakpoint(
        &mut p,
        "fn id(n: integer) -> integer {\n  n + 0\n}",
        "id(7) + id(9)",
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
    let hits = run_with_line_breakpoint(
        &mut p,
        "fn add(a: integer, b: integer) -> integer {\n  a + b\n}",
        "add(40, 2)",
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
