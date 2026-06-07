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
use crate::data::DefType;
use crate::diagnostics::{DiagEntry, Level};
use crate::introspect::{Options, Section};
use crate::parser::Parser;
use crate::state::State;
use std::io::{BufRead, Write};
use std::panic::AssertUnwindSafe;

/// Run the interactive `loft>` REPL.
///
/// Reads inputs from `input` (one statement per line, multi-line accumulated on
/// [`Eval::NeedMore`]) and writes the prompt, messages, and parse errors to
/// `chrome` (stderr in the CLI).  Evaluated **results** are printed by the
/// program itself to process stdout, so a terminal sees them and a piped caller
/// can capture them.  Returns when input reaches EOF or the user types `:quit`.
///
/// A runtime panic (failed `assert`, overflow) is caught — execution runs on a
/// throwaway clone of the database, so the session survives; the loop reports it
/// and continues.
///
/// # Errors
/// Returns an I/O error from loading the stdlib or writing to `chrome`.
pub fn run_repl<R: BufRead, W: Write>(
    stdlib_dir: &str,
    mut input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    let mut session = ReplSession::new(stdlib_dir)?;
    writeln!(chrome, "loft REPL — :help for commands, :quit to exit")?;
    let mut pending = String::new();
    let mut line = String::new();
    loop {
        write!(
            chrome,
            "{}",
            if pending.is_empty() {
                "loft> "
            } else {
                "..... > "
            }
        )?;
        chrome.flush()?;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break; // EOF
        }
        let trimmed = line.trim_end();
        // `:`-commands are only recognised at the start of a fresh statement.
        if pending.is_empty() && trimmed.starts_with(':') {
            let mut words = trimmed[1..].split_whitespace();
            let cmd = words.next().unwrap_or("");
            let filter: Vec<String> = words.map(str::to_string).collect();
            match cmd {
                "quit" | "q" => break,
                "help" | "h" => writeln!(
                    chrome,
                    "commands: :quit  :help  :reset  :fns  \
                     :bytecode [fn]  :rust [fn]  :slots [fn]"
                )?,
                "reset" => {
                    session = ReplSession::new(stdlib_dir)?;
                    writeln!(chrome, "session reset.")?;
                }
                "bytecode" => session.introspect(Section::Bytecode, filter),
                "rust" => session.introspect(Section::Rust, filter),
                "slots" => session.introspect(Section::Slots, filter),
                "fns" => session.list_fns(),
                other => writeln!(chrome, "unknown command: :{other}  (:help)")?,
            }
            continue;
        }
        pending.push_str(trimmed);
        pending.push('\n');
        let src = pending.clone();
        // Catch a runtime panic so a bad input never kills the REPL.  AssertUnwindSafe
        // is sound here: a panic corrupts only the per-eval database clone, never
        // `session`'s own state.
        match std::panic::catch_unwind(AssertUnwindSafe(|| session.eval(&src))) {
            Ok(Eval::Ran) => pending.clear(),
            Ok(Eval::NeedMore) => {} // keep accumulating; continuation prompt next
            Ok(Eval::Error(diags)) => {
                for d in diags {
                    writeln!(chrome, "{}", d.to_string_compact())?;
                }
                pending.clear();
            }
            Err(_) => {
                writeln!(
                    chrome,
                    "runtime error (session preserved; :reset to clear state)"
                )?;
                pending.clear();
            }
        }
    }
    Ok(())
}

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
    /// A binding (`x = 1`) is recorded but not executed now — an unused
    /// binding's slot is elided by the allocator, so even compiling it would
    /// panic; its value is realised when a later input observes it.  Any other
    /// input is an *observing* statement: it is wrapped as `println("{<input>}")`
    /// so a bare expression's value is shown in loft's native rendering, with a
    /// fall back to running it plain when it is a void statement (`assert`,
    /// `print`) or otherwise can't be string-interpolated.
    ///
    /// # Panics
    /// Propagates a runtime panic from the executed program (e.g. a failed
    /// `assert`, arithmetic overflow, or the interpreter's infinite-loop guard).
    /// Because execution runs on a throwaway clone of the database, the session
    /// itself survives such a panic when the caller wraps this in `catch_unwind`.
    pub fn eval(&mut self, input: &str) -> Eval {
        if Parser::statement_incomplete(input) {
            return Eval::NeedMore;
        }
        if Parser::starts_top_level_def(input) {
            // A definition (struct/enum/fn/type/…): parse it as a top-level def
            // and keep it — it persists in `data` (parse_str appends, never
            // wipes prior defs) and is callable from later inputs.  Nothing to
            // print or execute.
            let pre_defs = self.parser.data.definitions();
            let pre_diag = self.parser.diagnostics.entries().len();
            self.parser.parse_str(input, "<repl>", false);
            let produced: Vec<DiagEntry> = self.parser.diagnostics.entries()[pre_diag..].to_vec();
            if produced.iter().any(|e| e.level >= Level::Error) {
                self.parser.data.rollback_to(pre_defs);
                return Eval::Error(produced);
            }
            return Eval::Ran;
        }
        if Self::binding_name(input).is_some() {
            // Record the binding (validate only — see the doc comment).  It is
            // recompiled, in use, when a later input observes it.
            let bound = format!("{}{};\n", self.body, input);
            return match self.compile_generation(&bound, false) {
                Ok(()) => {
                    self.body = bound;
                    Eval::Ran
                }
                Err(diags) => Eval::Error(diags),
            };
        }
        // Observing: show the value.  Try the print wrapper first; if it doesn't
        // compile (void statement, or input that breaks string interpolation),
        // run the input plain so side effects still happen.
        let shown = format!("{}println(\"{{{input}}}\");\n", self.body);
        if self.compile_generation(&shown, true).is_ok() {
            return Eval::Ran;
        }
        let plain = format!("{}{};\n", self.body, input);
        match self.compile_generation(&plain, true) {
            Ok(()) => Eval::Ran,
            Err(diags) => Eval::Error(diags),
        }
    }

    /// Parse one generation fn `fn replmain_N() { <gen_body> }`; when `execute`,
    /// run it.  On a parse error rolls `data` back and returns the diagnostics.
    /// When not executing (a binding), the generation's def is rolled back too —
    /// it lives on only as source in `body`.
    fn compile_generation(&mut self, gen_body: &str, execute: bool) -> Result<(), Vec<DiagEntry>> {
        let next = self.counter + 1;
        let name = format!("replmain_{next}");
        let src = format!("fn {name}() {{\n{gen_body}}}\n");
        let pre_defs = self.parser.data.definitions();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        // Only this call's diagnostics — `Diagnostics::level` is monotonic.
        let produced: Vec<DiagEntry> = self.parser.diagnostics.entries()[pre_diag..].to_vec();
        if produced.iter().any(|e| e.level >= Level::Error) {
            // The lexer clears its diagnostics per parse_str, so this error does
            // not leak into the next input — the session stays usable after a typo.
            self.parser.data.rollback_to(pre_defs);
            return Err(produced);
        }
        self.counter = next;
        if execute {
            // scopes::check assigns slots + does lifetime analysis (the real
            // pipeline runs it between parse and byte_code; without it locals get
            // no slot and execution underflows).  A fresh State per input
            // sidesteps the @P381 CONST_STORE re-lock and isolates a runtime
            // panic to the throwaway clone.
            crate::scopes::check(&mut self.parser.data);
            let mut state = State::new(self.parser.database.clone());
            compile::byte_code(&mut state, &mut self.parser.data);
            state.execute_argv(&name, &self.parser.data, &[]);
        } else {
            self.parser.data.rollback_to(pre_defs);
        }
        Ok(())
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

    /// Compile the current session and emit one introspection `section`
    /// (bytecode / Rust / slots / types) to stdout, optionally restricted to
    /// named functions — the engine behind the REPL's `:bytecode` / `:rust` /
    /// `:slots` commands (reuses phase 01's `introspect`).
    pub fn introspect(&mut self, section: Section, fn_filter: Vec<String>) {
        crate::scopes::check(&mut self.parser.data);
        let mut state = State::new(self.parser.database.clone());
        compile::byte_code(&mut state, &mut self.parser.data);
        let end_def = self.parser.data.definitions();
        let opts = Options {
            sections: vec![section],
            fn_filter,
            ..Options::new()
        };
        let _ = crate::introspect::emit_all(&mut self.parser.data, &mut state, end_def, &opts);
    }

    /// Print the user-defined functions entered this session (name + return
    /// type), to stdout — the `:fns` command.  Synthetic generation wrappers
    /// (`repl_*` / `replmain_*`) and stdlib functions are excluded.
    pub fn list_fns(&self) {
        let data = &self.parser.data;
        for d in 0..data.definitions() {
            let def = data.def(d);
            if def.def_type != DefType::Function
                || !def.name.starts_with("n_")
                || def.name.starts_with("n_repl")
                || def.position.file != "<repl>"
            {
                continue;
            }
            let user = def.name.strip_prefix("n_").unwrap_or(&def.name);
            let ret = def.returned.show(data, &def.variables);
            println!("{user} -> {ret}");
        }
    }
}
