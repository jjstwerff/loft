// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN12 phase 03 — interactive REPL session.
//!
//! Holds a parser with the stdlib loaded plus the accumulated session
//! statements.  Each [`ReplSession::eval`] appends the input to one shared
//! function scope and runs it, so a variable bound in one input is visible to
//! the next (`x = 1` then `x + 1` sees `x`).
//!
//! **Scope / strategy note.**  This re-evaluates the accumulated body in a
//! single shared scope each input.  As long as each binding's right-hand side
//! is deterministic and side-effect-free — any value type — that is
//! behaviourally correct: re-running yields the same values.  A statement with
//! *side effects* (I/O)
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
use std::fs::File;
use std::io::{BufRead, Write};
use std::panic::AssertUnwindSafe;
use std::path::Path;

/// Path to the auto-resume session file (`~/.loft_session`), or `None` if the
/// home directory can't be located.  Shared by the interactive driver (which
/// replays it on launch and appends new state-changing inputs to it) and the
/// `--fresh` flag in `main` (which clears it).
pub fn session_file_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".loft_session"))
}

/// Run the interactive `loft>` REPL.
///
/// Reads inputs (one statement per line, multi-line accumulated on
/// [`Eval::NeedMore`]) and writes the prompt, messages, and parse errors to
/// `chrome` (stderr in the CLI).  Evaluated **results** are printed by the
/// program itself to process stdout, so a terminal sees them and a piped caller
/// can capture them.  Returns when input reaches EOF or the user types `:quit`.
///
/// When the process's stdin is an interactive terminal, input is read through a
/// line editor (arrow-key history, in-line editing) and `input` is ignored.
/// Otherwise — a pipe, a file, a test harness, or a wasm build — input is read
/// plainly from `input`, so captured/piped output is byte-for-byte unchanged.
///
/// A runtime panic (failed `assert`, overflow) is caught — execution runs on a
/// throwaway clone of the database, so the session survives; the loop reports it
/// and continues.
///
/// # Errors
/// Returns an I/O error from loading the stdlib or writing to `chrome`.
pub fn run_repl<R: BufRead, W: Write>(
    stdlib_dir: &str,
    input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    let mut session = ReplSession::new(stdlib_dir)?;
    writeln!(chrome, "loft REPL — :help for commands, :quit to exit")?;
    // Silence the default panic handler for the duration of the loop: a runtime
    // error inside `eval` is caught below and reported cleanly, so the user
    // should not also see Rust's raw "thread panicked at …" backtrace.  Restored
    // before returning, even if the loop exits with an I/O error.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = run_loop(stdlib_dir, &mut session, input, chrome);
    std::panic::set_hook(prev_hook);
    result
}

/// Pick the input driver: the interactive line editor when stdin is a terminal,
/// the plain reader otherwise.  wasm has no line editor, so it always reads
/// plainly.
#[cfg(not(target_arch = "wasm32"))]
fn run_loop<R: BufRead, W: Write>(
    stdlib_dir: &str,
    session: &mut ReplSession,
    input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        run_interactive(stdlib_dir, session, chrome)
    } else {
        run_piped(stdlib_dir, session, input, chrome)
    }
}

#[cfg(target_arch = "wasm32")]
fn run_loop<R: BufRead, W: Write>(
    stdlib_dir: &str,
    session: &mut ReplSession,
    input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    run_piped(stdlib_dir, session, input, chrome)
}

/// The continuation-aware prompt: the primary prompt for a fresh statement, the
/// dotted prompt while a multi-line statement is still open.
fn prompt(pending: &str) -> &'static str {
    if pending.is_empty() {
        "loft> "
    } else {
        "..... > "
    }
}

/// Read input plainly from `input`, one line at a time.  Used for pipes, files,
/// test harnesses, and wasm — anywhere there is no interactive terminal — so the
/// captured output stays stable.
fn run_piped<R: BufRead, W: Write>(
    stdlib_dir: &str,
    session: &mut ReplSession,
    mut input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    let mut pending = String::new();
    let mut line = String::new();
    loop {
        write!(chrome, "{}", prompt(&pending))?;
        chrome.flush()?;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break; // EOF
        }
        // No session path: a piped/file/test run never persists or resumes.
        if process_line(
            line.trim_end(),
            session,
            &mut pending,
            stdlib_dir,
            None,
            chrome,
        )? {
            break; // :quit
        }
    }
    Ok(())
}

/// Read input through a line editor (arrow-key history + in-line editing).  The
/// editor owns the prompt and reads the terminal directly, so the plain `input`
/// reader is not used here.  History persists to `~/.loft_history` across
/// sessions (best-effort — a failure to load or save is ignored).  Ctrl-C
/// cancels the statement in progress; Ctrl-D at an empty prompt quits.
#[cfg(not(target_arch = "wasm32"))]
fn run_interactive<W: Write>(
    stdlib_dir: &str,
    session: &mut ReplSession,
    chrome: &mut W,
) -> std::io::Result<()> {
    use rustyline::error::ReadlineError;
    let mut rl = match rustyline::DefaultEditor::new() {
        Ok(rl) => rl,
        // No usable terminal after all — fall back to the plain reader.
        Err(_) => return run_piped(stdlib_dir, session, std::io::stdin().lock(), chrome),
    };
    let history = dirs::home_dir().map(|h| h.join(".loft_history"));
    if let Some(path) = &history {
        let _ = rl.load_history(path);
    }
    // Auto-resume: replay the previous session, then append new state-changing
    // inputs to the same file.  Interactive-only — `run_piped` (pipes, files,
    // tests, wasm) never touches it, so captured output stays deterministic.
    let session_path = session_file_path();
    if let Some(path) = &session_path {
        let stats = session.resume_from(path);
        if stats.restored > 0 {
            let skipped = if stats.skipped > 0 {
                format!(" ({} skipped)", stats.skipped)
            } else {
                String::new()
            };
            writeln!(
                chrome,
                "restored {} statement(s) from last session{skipped}",
                stats.restored
            )?;
        }
        let _ = session.enable_persistence(path);
    }
    let mut pending = String::new();
    loop {
        match rl.readline(prompt(&pending)) {
            Ok(line) => {
                let _ = rl.add_history_entry(line.as_str());
                if process_line(
                    line.trim_end(),
                    session,
                    &mut pending,
                    stdlib_dir,
                    session_path.as_deref(),
                    chrome,
                )? {
                    break; // :quit
                }
            }
            // Ctrl-C drops the statement in progress and returns to a fresh prompt.
            Err(ReadlineError::Interrupted) => pending.clear(),
            // Ctrl-D (or any read error) ends the session.
            Err(ReadlineError::Eof) => break,
            Err(_) => break,
        }
    }
    if let Some(path) = &history {
        let _ = rl.save_history(path);
    }
    Ok(())
}

/// Process one input line against the session: dispatch a `:`-command, or feed
/// the line into the accumulating statement and evaluate it when complete.
/// Returns `Ok(true)` when the user asked to quit (`:quit`).  Shared by both
/// input drivers so interactive and piped sessions behave identically.
fn process_line<W: Write>(
    trimmed: &str,
    session: &mut ReplSession,
    pending: &mut String,
    stdlib_dir: &str,
    session_path: Option<&Path>,
    chrome: &mut W,
) -> std::io::Result<bool> {
    // `:`-commands are only recognised at the start of a fresh statement.
    if pending.is_empty() && trimmed.starts_with(':') {
        let mut words = trimmed[1..].split_whitespace();
        let cmd = words.next().unwrap_or("");
        let filter: Vec<String> = words.map(str::to_string).collect();
        match cmd {
            "quit" | "q" => return Ok(true),
            "help" | "h" => writeln!(
                chrome,
                "commands: :quit  :help  :reset  :fns  :type <expr>  \
                 :bytecode [fn]  :rust [fn]  :slots [fn]"
            )?,
            "reset" => {
                *session = ReplSession::new(stdlib_dir)?;
                // Clearing state clears the persisted session too, so the next
                // launch starts clean; keep persisting to the now-empty file.
                if let Some(path) = session_path {
                    ReplSession::clear_session(path);
                    let _ = session.enable_persistence(path);
                }
                writeln!(chrome, "session reset.")?;
            }
            "bytecode" => session.introspect(Section::Bytecode, filter),
            "rust" => session.introspect(Section::Rust, filter),
            "slots" => session.introspect(Section::Slots, filter),
            "fns" => session.list_fns(),
            "type" => {
                let expr = filter.join(" ");
                match session.infer_type(&expr) {
                    Some(t) => println!("{t}"),
                    None => writeln!(chrome, "could not infer the type of `{expr}`")?,
                }
            }
            other => writeln!(chrome, "unknown command: :{other}  (:help)")?,
        }
        return Ok(false);
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
    Ok(false)
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

/// How a [`ReplSession::resume_from`] replay went.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResumeStats {
    /// Saved entries that replayed cleanly.
    pub restored: usize,
    /// Entries skipped because they no longer parse/run — a stale or corrupt
    /// line never bricks the resume.
    pub skipped: usize,
}

/// A live REPL session: stdlib + the statements entered so far.
pub struct ReplSession {
    parser: Parser,
    /// Accumulated statements, each terminated with `;`, replayed in one shared
    /// function scope so earlier bindings stay visible.
    body: String,
    /// Monotonic counter naming each generation's synthetic entry fn.
    counter: u32,
    /// Append handle to the session file when persistence is on (the
    /// interactive driver enables it; piped/test sessions leave it `None`, so
    /// they neither read nor write any session file).
    record: Option<File>,
    /// True only while [`resume_from`](Self::resume_from) is feeding saved
    /// inputs back in — suppresses re-recording them to the file.
    replaying: bool,
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
            record: None,
            replaying: false,
        })
    }

    /// Turn on session persistence: every later state-changing input appends to
    /// `path` (created if absent).  The interactive driver calls this after
    /// [`resume_from`](Self::resume_from); piped/test sessions never do, so they
    /// stay file-free and their output stays deterministic.
    ///
    /// # Errors
    /// Returns the I/O error if `path` cannot be opened for appending.
    pub fn enable_persistence(&mut self, path: &Path) -> std::io::Result<()> {
        self.record = Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?,
        );
        Ok(())
    }

    /// Append one state-changing input to the session file, when persistence is
    /// on and we are not mid-resume.  Entries are NUL-separated: NUL never
    /// occurs in loft source, so a multi-line statement survives verbatim and
    /// splits back out unambiguously.  Best-effort — a write failure is dropped
    /// rather than allowed to break the live session.
    fn record_input(&mut self, input: &str) {
        if self.replaying {
            return;
        }
        if let Some(f) = self.record.as_mut() {
            let _ = f.write_all(input.as_bytes());
            let _ = f.write_all(b"\0");
            let _ = f.flush();
        }
    }

    /// Replay a saved session from `path` to rebuild state, before persistence
    /// is enabled.  Each NUL-separated entry is fed back through
    /// [`eval`](Self::eval) with recording suppressed; an entry that no longer
    /// parses (or panics) is skipped, never aborting the resume.  A missing file
    /// is an empty session.  Returns how many entries were restored vs skipped.
    pub fn resume_from(&mut self, path: &Path) -> ResumeStats {
        let mut stats = ResumeStats::default();
        let Ok(bytes) = std::fs::read(path) else {
            return stats; // no prior session
        };
        let text = String::from_utf8_lossy(&bytes);
        self.replaying = true;
        for entry in text.split('\0') {
            if entry.trim().is_empty() {
                continue;
            }
            // Same panic isolation as the live loop: a poison entry must not
            // abort the resume.  Resume replays only defs/bindings (neither
            // executes), so a panic here would be a parser fault, not runtime.
            match std::panic::catch_unwind(AssertUnwindSafe(|| self.eval(entry))) {
                Ok(Eval::Ran) => stats.restored += 1,
                _ => stats.skipped += 1, // Error / NeedMore / panic
            }
        }
        self.replaying = false;
        stats
    }

    /// Discard the saved session at `path` (the `:reset` command and the
    /// `--fresh` flag) so the next launch starts clean.  Best-effort.
    pub fn clear_session(path: &Path) {
        let _ = std::fs::remove_file(path);
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
            self.record_input(input); // a def changes session state — persist it
            return Eval::Ran;
        }
        if Self::binding_name(input).is_some() {
            // Record the binding (validate only — see the doc comment).  It is
            // recompiled, in use, when a later input observes it.
            let bound = format!("{}{};\n", self.body, input);
            return match self.compile_generation(&bound, false) {
                Ok(()) => {
                    self.body = bound;
                    self.record_input(input); // a binding changes state — persist it
                    Eval::Ran
                }
                Err(diags) => Eval::Error(diags),
            };
        }
        // Observing: show the value.  Bind it to a temp first, then print the
        // temp — `__replval = <input>; println("{__replval}")`.  Binding first
        // (rather than interpolating `<input>` into the format string directly)
        // means a text result echoes too (no nested-quote breakage), and works
        // uniformly for scalars, text, and structs.  If the input is a void
        // statement (`assert`, `print`) the temp binds to void and this fails to
        // compile — fall back to running the input plain so its side effects
        // still happen.
        let shown = format!(
            "{}__replval = {input};\nprintln(\"{{__replval}}\");\n",
            self.body
        );
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
            // A failed `assert`, `panic(…)`, or a fault-site opcode is captured
            // here (execution stops cleanly, no Rust panic) rather than thrown —
            // surface it as an error instead of silently swallowing it.  Roll the
            // throwaway gen back so the failed line leaves no def behind.
            if let Some(err) = state.database.runtime_error.take() {
                self.parser.data.rollback_to(pre_defs);
                return Err(vec![err.to_diag_entry()]);
            }
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

    /// Infer the static type of `expr` against the current session, without
    /// running it — the `:type` command.  Returns the rendered type, or `None`
    /// if it doesn't type-check.  Compile-time only: it binds `expr` to a temp
    /// in a throwaway generation, reads the temp's inferred type from the
    /// function's variable table, and rolls the probe back — no execution.
    pub fn infer_type(&mut self, expr: &str) -> Option<String> {
        let name = format!("replmain_{}", self.counter + 1);
        let probe = format!("{}__t = {expr};\n", self.body);
        let src = format!("fn {name}() {{\n{probe}}}\n");
        let pre_defs = self.parser.data.definitions();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        let result = if failed {
            None
        } else {
            let d = self.parser.data.def_nr(&format!("n_{name}"));
            if d == u32::MAX {
                None
            } else {
                let def = self.parser.data.def(d);
                let vars = &def.variables;
                let mut found = None;
                for i in 0..vars.count() {
                    if vars.name(i) == "__t" {
                        found = Some(vars.tp(i).show(&self.parser.data, vars));
                        break;
                    }
                }
                found
            }
        };
        self.parser.data.rollback_to(pre_defs); // discard the probe def
        result
    }
}
