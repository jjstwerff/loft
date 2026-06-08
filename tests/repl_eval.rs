// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 03 slice A — end-to-end REPL eval pipeline.
//!
//! Proves the existing pieces compose into a parse → compile → execute loop:
//! `Parser::parse_statement` (phase 02) feeds `compile::byte_code` + a runnable
//! wrapper that `State::execute_argv` runs, and a definition from one input is
//! callable from a later one.
//!
//! Slice A uses a fresh `State` + full compile per input — correct, not yet
//! optimized (a fresh State sidesteps the CONST_STORE re-lock from a second
//! `byte_code` on the same State, @P381).  Per-input incremental compile
//! (`byte_code_from`) + a persistent State that keeps the variable region on the
//! stack is the next slice (cross-input *variable* persistence).

use loft::compile;
use loft::parser::{ParseResult, Parser};
use loft::state::State;

/// Parse one input against the live session and, if it yielded a runnable
/// wrapper, compile the program and execute it.  Panics on a parse error so a
/// test sees the diagnostics.
fn eval(p: &mut Parser, input: &str) {
    match p.parse_statement(input) {
        ParseResult::Ready { entry_def_nr } => {
            let mut state = State::new(p.database.clone());
            compile::byte_code(&mut state, &mut p.data);
            if entry_def_nr != u32::MAX {
                // execute_argv re-adds the `n_` prefix, so pass the bare name.
                let name = p
                    .data
                    .def(entry_def_nr)
                    .name()
                    .strip_prefix("n_")
                    .expect("wrapper fn is n_-prefixed")
                    .to_string();
                state.execute_argv(&name, &p.data, &[]);
            }
        }
        other => panic!("parse of {input:?} failed: {other:?}"),
    }
}

fn repl() -> Parser {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p
}

/// A definition in one input is callable from an expression in a later input.
/// The `assert` holding (no panic / abort) proves the call actually executed
/// with the right result.
#[test]
fn expression_calls_prior_definition() {
    let mut p = repl();
    eval(&mut p, "fn dbl(x: integer) -> integer { x + x }");
    eval(&mut p, "assert(dbl(21) == 42, \"dbl works in repl\")");
}

/// A bare expression compiles + runs (result discarded) without error.
#[test]
fn bare_expression_runs() {
    let mut p = repl();
    eval(&mut p, "1 + 2");
}

/// A stdlib call works from a REPL input.
#[test]
fn stdlib_call_runs() {
    let mut p = repl();
    eval(&mut p, "assert(max(3, 7) == 7, \"max works\")");
}
