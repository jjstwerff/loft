// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 03 — interactive REPL session (integer scope, slice B start).
//!
//! Holds a parser with the stdlib loaded plus the accumulated session
//! statements.  Each [`ReplSession::eval`] appends the input to one shared
//! function scope and runs it, so a variable bound in one input is visible to
//! the next (`x = 1` then `x + 1` sees `x`).
//!
//! **Scope / strategy note.**  This re-evaluates the accumulated body in a
//! single shared scope each input.  For pure integer computation — the case
//! this slice targets — that is behaviourally correct: re-running deterministic
//! arithmetic yields the same values.  A statement with *side effects* (I/O)
//! would re-run on each later input; eliminating that is the incremental,
//! stack-resident model (compile only the new statement, keep the variable
//! region on the stack across `reset_for_repl`) planned in
//! `plans/12-repl-and-introspection/03-state-reset-and-append.md`.  That
//! refinement sits behind this same `ReplSession` API.

use crate::compile;
use crate::diagnostics::{DiagEntry, Level};
use crate::parser::Parser;
use crate::state::State;

/// Outcome of evaluating one REPL input.
#[derive(Debug)]
pub enum Eval {
    /// The statement parsed and ran; the session advanced.
    Ran,
    /// Input ends mid-construct (open bracket, unterminated string, trailing
    /// operator).  The caller should read another line and re-call with the
    /// concatenated input.  The session is unchanged.
    NeedMore,
    /// Parse error; the session is left exactly as it was before the call.
    Error(Vec<DiagEntry>),
}

/// A live REPL session: stdlib + the statements entered so far.
pub struct ReplSession {
    parser: Parser,
    /// Accumulated statements, each terminated with `;`, replayed in one shared
    /// function scope so earlier bindings stay visible.
    body: String,
    /// Monotonic counter naming each generation's synthetic entry fn.
    counter: u32,
}

impl ReplSession {
    /// Start a session with the standard library loaded from `stdlib_dir`
    /// (e.g. `"default"`, or an absolute path to a release bundle's `default/`).
    ///
    /// # Errors
    /// Returns the I/O error if the stdlib directory cannot be read.
    pub fn new(stdlib_dir: &str) -> std::io::Result<Self> {
        let mut parser = Parser::new();
        parser.parse_dir(stdlib_dir, true, false)?;
        Ok(Self {
            parser,
            body: String::new(),
            counter: 0,
        })
    }

    /// Evaluate one input line/statement against the session.
    ///
    /// Returns [`Eval::NeedMore`] for incomplete input, [`Eval::Error`] (session
    /// unchanged) on a parse error, or [`Eval::Ran`] when the statement parsed
    /// and executed.
    ///
    /// # Panics
    /// Propagates a runtime panic from the executed program (e.g. a failed
    /// `assert`, arithmetic overflow, or the interpreter's infinite-loop guard).
    pub fn eval(&mut self, input: &str) -> Eval {
        if Parser::statement_incomplete(input) {
            return Eval::NeedMore;
        }
        // Build this generation: all bindings so far + this input, in one shared
        // fn scope so earlier bindings are visible.  A fresh generation name
        // avoids redefining the previous entry fn.
        let is_binding = Self::binding_name(input).is_some();
        let gen_body = format!("{}{};\n", self.body, input);
        let next = self.counter + 1;
        let name = format!("replmain_{next}");
        let src = format!("fn {name}() {{\n{gen_body}}}\n");

        let pre_defs = self.parser.data.definitions();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        // Only this call's diagnostics — `Diagnostics::level` is monotonic.
        let produced: Vec<DiagEntry> = self.parser.diagnostics.entries()[pre_diag..].to_vec();
        if produced.iter().any(|e| e.level >= Level::Error) {
            // Roll `data` back to the pre-call state.  The lexer clears its
            // diagnostics per parse_str, so this error does not leak into the
            // next input — the session stays usable after a typo.
            self.parser.data.rollback_to(pre_defs);
            return Eval::Error(produced);
        }
        self.counter = next;

        if is_binding {
            // A binding defines a variable; its value is realised when a later
            // input *observes* it.  Discard this generation's throwaway fn: an
            // unused binding's slot is elided by the allocator, so even
            // *compiling* it (byte_code walks every def) would panic.  The
            // binding lives on as source in `body` and is recompiled — now in
            // use — when a later input observes it; re-running deterministic
            // integer arithmetic yields the same value.
            self.parser.data.rollback_to(pre_defs);
            self.body = gen_body;
        } else {
            // An observing statement (expression / call): every prior binding is
            // now used, so the allocator keeps their slots.  Compile + run.  A
            // fresh State per input sidesteps the @P381 CONST_STORE re-lock from
            // a second byte_code on the same State (correctness over speed here).
            // A non-binding defines no variable, so it is not persisted.
            // scopes::check assigns slots + does lifetime analysis — the real
            // pipeline runs it between parse and byte_code; without it locals
            // get no slot (slot == u16::MAX) and execution underflows.
            crate::scopes::check(&mut self.parser.data);
            let mut state = State::new(self.parser.database.clone());
            compile::byte_code(&mut state, &mut self.parser.data);
            state.execute_argv(&name, &self.parser.data, &[]);
        }
        Eval::Ran
    }

    /// If `input` is a simple binding `<name> = <expr>` (not `==`/`+=`/…),
    /// return the bound name.  The caller echoes it as a trailing read so the
    /// variable keeps a stack slot in this and every later generation (an
    /// unused binding would otherwise have its slot elided).
    fn binding_name(input: &str) -> Option<String> {
        let t = input.trim_start();
        let name: String = t
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let first = name.chars().next()?;
        if !(first.is_ascii_alphabetic() || first == '_') {
            return None;
        }
        let rest = t[name.len()..].trim_start();
        let mut cs = rest.chars();
        // A single `=` (assignment), not `==` (comparison).
        if cs.next() == Some('=') && cs.next() != Some('=') {
            Some(name)
        } else {
            None
        }
    }
}
