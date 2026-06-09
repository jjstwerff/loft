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
use crate::data::{DefType, Type};
#[cfg(not(target_arch = "wasm32"))]
use crate::database::Parts;
use crate::diagnostics::{DiagEntry, Level};
use crate::introspect::{Options, Section};
use crate::parser::Parser;
use crate::state::State;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
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
    // @PLN15 G1 — the REPL is the interactive debugger surface: a breakpoint hit
    // suspends into the paused sub-mode (inspect / edit / step), rather than the
    // record-and-continue mode programmatic callers use.
    session.debug_stepping(true);
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

/// The continuation-aware prompt: the `(dbg)` prompt while suspended at a
/// breakpoint (@PLN15 G1), the dotted prompt while a multi-line statement is still
/// open, the primary prompt otherwise.
fn prompt(pending: &str, debugging: bool) -> &'static str {
    if !pending.is_empty() {
        "..... > "
    } else if debugging {
        "(dbg) "
    } else {
        "loft> "
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
        write!(chrome, "{}", prompt(&pending, session.is_debugging()))?;
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

/// True when `s` is a plain identifier (`[A-Za-z_][A-Za-z0-9_]*`).  Keeps
/// synthetic definition names (`main_vector<…>`, `__tuple<…>`) out of the
/// completion list.
#[cfg(not(target_arch = "wasm32"))]
fn is_plain_ident(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// True when `method` is a member a user can call as `recv.method(...)`: a plain
/// identifier that is neither a compiler-internal (`__…`) nor an operator
/// overload — operator methods register as `Op<Name>` (`Op` + an uppercase
/// letter) and are invoked through the operator, not by name.
#[cfg(not(target_arch = "wasm32"))]
fn is_callable_method(method: &str) -> bool {
    is_plain_ident(method)
        && !method.starts_with("__")
        && !(method.len() > 2
            && method.starts_with("Op")
            && method.as_bytes()[2].is_ascii_uppercase())
}

/// The methods defined on the type named `type_name`, each rendered with a
/// trailing `(` so completion shows it is callable (and the cursor lands inside
/// the call).  Methods register under the length-prefixed name
/// `t_<len><type_name>_<method>` — the same `Type::name` the receiver resolves
/// to — so one `strip_prefix` recovers exactly this type's methods, for structs,
/// text, vectors, and every base type alike.
#[cfg(not(target_arch = "wasm32"))]
fn methods_for_type(data: &crate::data::Data, type_name: &str) -> Vec<String> {
    let prefix = format!("t_{}{type_name}_", type_name.len());
    let mut out = Vec::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type == DefType::Function
            && let Some(method) = def.name.strip_prefix(&prefix)
            && is_callable_method(method)
        {
            out.push(format!("{method}("));
        }
    }
    out
}

/// The `:`-commands Tab completion offers when the line starts with `:`.
#[cfg(not(target_arch = "wasm32"))]
const REPL_COMMANDS: [&str; 10] = [
    "quit", "help", "reset", "fns", "vars", "type", "break", "bytecode", "rust", "slots",
];

/// The completion model the live completer matches against, rebuilt by the
/// interactive loop after each input.  `names` are the bare identifiers
/// (globals + session variables); `members` maps a dotted receiver — a
/// struct-typed variable to its field names, or an enum *type* name to its
/// variant names — to the candidates valid after `receiver.`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub struct CompletionModel {
    /// Bare-identifier candidates: globals (user + stdlib fns, type names) and
    /// the variables bound this session.  Sorted + deduped.
    pub names: Vec<String>,
    /// Dotted-access candidates keyed by receiver token: a struct variable → its
    /// field names; an enum type name → its variant names.  Each list sorted.
    pub members: HashMap<String, Vec<String>>,
}

/// The trailing identifier of `s` (`"1 + p"` → `"p"`), or `None` when `s` does
/// not end in an identifier character (`"arr[0]"` → `None`).  Reads the receiver
/// token immediately before a `.`.
#[cfg(not(target_arch = "wasm32"))]
fn trailing_ident(s: &str) -> Option<&str> {
    let start = s
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(s.len(), |(i, _)| i);
    let id = &s[start..];
    (!id.is_empty()).then_some(id)
}

/// Pure completion logic, shared by the live completer and its unit tests.
/// Given the `model` and the cursor at byte `pos` in `line`, returns the offset
/// where the replaced word starts and the matching candidates.  Three contexts,
/// in priority order:
///
/// - a leading `:word` → [`REPL_COMMANDS`];
/// - `receiver.partial` (the cursor sits past a `.`) → **only** that receiver's
///   `members`, never the global list — an unresolved receiver (`foo.`,
///   `arr[0].`) yields nothing rather than leaking globals after the dot;
/// - otherwise a bare identifier prefix → `model.names`.  An empty *bare* prefix
///   yields nothing (a stray Tab never dumps the whole list), but an empty
///   prefix right after `receiver.` lists all of that receiver's members.
#[cfg(not(target_arch = "wasm32"))]
fn complete_word(model: &CompletionModel, line: &str, pos: usize) -> (usize, Vec<String>) {
    let head = &line[..pos];
    // `:command` — only while the cursor is still inside the leading `:word`.
    if let Some(after) = head.strip_prefix(':')
        && !after.contains(char::is_whitespace)
    {
        let out = REPL_COMMANDS
            .iter()
            .filter(|c| c.starts_with(after))
            .map(|c| (*c).to_string())
            .collect();
        return (1, out);
    }
    // Identifier under the cursor: walk back over identifier characters.
    let start = head
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(pos, |(i, _)| i);
    let prefix = &head[start..];
    // Dotted member access: once the cursor is past a `.`, only this receiver's
    // members are valid candidates — never the global identifier list.
    if let Some(stem) = head[..start].strip_suffix('.') {
        let out = trailing_ident(stem)
            .and_then(|recv| model.members.get(recv))
            .map(|ms| {
                ms.iter()
                    .filter(|m| m.starts_with(prefix))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        return (start, out);
    }
    if prefix.is_empty() {
        return (start, Vec::new());
    }
    let out = model
        .names
        .iter()
        .filter(|n| n.starts_with(prefix))
        .cloned()
        .collect();
    (start, out)
}

/// rustyline glue: completes loft identifiers and `:`-commands from the live
/// session.  `names` is refreshed by the interactive loop after each input.
/// Hinting, highlighting, and validation use rustyline's defaults.
#[cfg(not(target_arch = "wasm32"))]
struct ReplHelper {
    model: CompletionModel,
}

#[cfg(not(target_arch = "wasm32"))]
impl rustyline::completion::Completer for ReplHelper {
    type Candidate = String;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        Ok(complete_word(&self.model, line, pos))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl rustyline::hint::Hinter for ReplHelper {
    type Hint = String;
}
#[cfg(not(target_arch = "wasm32"))]
impl rustyline::highlight::Highlighter for ReplHelper {}
#[cfg(not(target_arch = "wasm32"))]
impl rustyline::validate::Validator for ReplHelper {}
#[cfg(not(target_arch = "wasm32"))]
impl rustyline::Helper for ReplHelper {}

/// Read input through a line editor (arrow-key history, in-line editing, and Tab
/// completion of session identifiers + `:`-commands).  The editor owns the
/// prompt and reads the terminal directly, so the plain `input` reader is not
/// used here.  History persists to `~/.loft_history` across sessions
/// (best-effort — a failure to load or save is ignored).  Ctrl-C cancels the
/// statement in progress; Ctrl-D at an empty prompt quits.
#[cfg(not(target_arch = "wasm32"))]
fn run_interactive<W: Write>(
    stdlib_dir: &str,
    session: &mut ReplSession,
    chrome: &mut W,
) -> std::io::Result<()> {
    use rustyline::error::ReadlineError;
    // `List` shows all candidates on an ambiguous Tab rather than cycling.
    let config = rustyline::Config::builder()
        .completion_type(rustyline::CompletionType::List)
        .build();
    let mut rl: rustyline::Editor<ReplHelper, rustyline::history::DefaultHistory> =
        match rustyline::Editor::with_config(config) {
            Ok(rl) => rl,
            // No usable terminal after all — fall back to the plain reader.
            Err(_) => return run_piped(stdlib_dir, session, std::io::stdin().lock(), chrome),
        };
    rl.set_helper(Some(ReplHelper {
        model: CompletionModel::default(),
    }));
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
    // Seed Tab completion with whatever resume restored, then keep it current.
    if let Some(h) = rl.helper_mut() {
        h.model = session.completion_model();
    }
    let mut pending = String::new();
    loop {
        match rl.readline(prompt(&pending, session.is_debugging())) {
            Ok(line) => {
                let _ = rl.add_history_entry(line.as_str());
                let quit = process_line(
                    line.trim_end(),
                    session,
                    &mut pending,
                    stdlib_dir,
                    session_path.as_deref(),
                    chrome,
                )?;
                // A new def or binding changes the candidate set — refresh it.
                if let Some(h) = rl.helper_mut() {
                    h.model = session.completion_model();
                }
                if quit {
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
    // @PLN15 G1 — while suspended at a breakpoint, inputs drive the paused
    // sub-mode (step verbs / value edits / frame eval), not a fresh evaluation.
    // A debug op runs user code (a resumed program, a frame expression); catch a
    // runtime panic so it abandons the debug session rather than killing the REPL,
    // mirroring the eval path below.
    if pending.is_empty() && session.is_debugging() {
        let outcome =
            std::panic::catch_unwind(AssertUnwindSafe(|| handle_paused(trimmed, session, chrome)));
        let Ok(res) = outcome else {
            session.abort_debug();
            writeln!(
                chrome,
                "runtime error in the paused run — debug session abandoned \
                 (session preserved)"
            )?;
            return Ok(false);
        };
        return res;
    }
    // `:`-commands are only recognised at the start of a fresh statement.
    if pending.is_empty() && trimmed.starts_with(':') {
        let mut words = trimmed[1..].split_whitespace();
        let cmd = words.next().unwrap_or("");
        let filter: Vec<String> = words.map(str::to_string).collect();
        match cmd {
            "quit" | "q" => return Ok(true),
            "help" | "h" => writeln!(
                chrome,
                "commands: :quit  :help  :reset  :fns  :vars  :type <expr>  \
                 :break <fn>|<fn>:<line>  :bytecode [fn]  :rust [fn]  :slots [fn]"
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
            "vars" => match session.show_vars() {
                Ok(true) => {}
                Ok(false) => writeln!(chrome, "no variables bound yet")?,
                Err(diags) => {
                    for d in diags {
                        writeln!(chrome, "{}", d.to_string_compact())?;
                    }
                }
            },
            "type" => {
                let expr = filter.join(" ");
                match session.infer_type(&expr) {
                    Some(t) => println!("{t}"),
                    None => writeln!(chrome, "could not infer the type of `{expr}`")?,
                }
            }
            "break" => match filter.first().map(String::as_str) {
                None if session.breakpoints().is_empty() => {
                    writeln!(
                        chrome,
                        "no breakpoints (`:break <fn>` / `<fn>:<line>` to add)"
                    )?;
                }
                None => writeln!(chrome, "breakpoints: {}", session.breakpoints().join(", "))?,
                Some("clear") => {
                    session.clear_breakpoints();
                    writeln!(chrome, "breakpoints cleared")?;
                }
                Some(_) => {
                    let spec = filter.join(" ");
                    if spec.parse::<u32>().is_ok() {
                        // A bare line isn't unique (same line number in every
                        // function; the REPL has no real file).  Steer to the
                        // function-scoped forms.
                        writeln!(
                            chrome,
                            "a bare line isn't unique — use `:break <fn>` or `<fn>:<line>`"
                        )?;
                    } else {
                        session.add_breakpoint(&spec);
                        writeln!(chrome, "breakpoint set: {spec}")?;
                    }
                }
            },
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
        Ok(Eval::Paused) => {
            // The run hit a breakpoint and suspended — show the frame and enter
            // the paused sub-mode; the next inputs are routed to `handle_paused`.
            pending.clear();
            print_pause(session, chrome)?;
        }
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

/// @PLN15 G1 — handle one input while **suspended** at a breakpoint.  Step verbs
/// resume execution (`:step`/`:s` into, `:next`/`:n` over, `:finish`/`:o` out,
/// `:continue`/`:c` to the next breakpoint or the end); `name = <int>` edits the
/// live frame; any other expression is **evaluated against the frame** (`n * 2`,
/// `pt.x`); `:vars` re-shows the frame; `:quit`/`:q` leaves the REPL.  Returns
/// `Ok(true)` only to quit.  Verbs work with or without the leading colon, so a
/// paused user can type `step` or `:step`.
fn handle_paused<W: Write>(
    trimmed: &str,
    session: &mut ReplSession,
    chrome: &mut W,
) -> std::io::Result<bool> {
    use crate::debugger::StepMode;
    let t = trimmed.trim();
    match t.strip_prefix(':').unwrap_or(t) {
        "quit" | "q" => return Ok(true),
        "step" | "s" => step_and_report(session, StepMode::Into, chrome)?,
        "next" | "n" => step_and_report(session, StepMode::Over, chrome)?,
        "finish" | "o" => step_and_report(session, StepMode::Out, chrome)?,
        "continue" | "c" => step_and_report(session, StepMode::Continue, chrome)?,
        "vars" => print_pause(session, chrome)?,
        "help" | "h" => writeln!(
            chrome,
            "paused: :step(:s) into  :next(:n) over  :finish(:o) out  :continue(:c)  \
             :vars  |  `name = <expr>` edits a scalar local  |  any expression is \
             evaluated at the frame  |  :quit"
        )?,
        _ => match parse_assign(t) {
            // `name = <expr>` writes the live frame (picked up on resume); the RHS
            // is evaluated against the frame, so an expression works too.
            Some((name, rhs)) if session.debug_set(name, rhs) => {
                print_pause(session, chrome)?;
            }
            Some((name, _)) => writeln!(
                chrome,
                "couldn't set `{name}` — unknown local, or a non-scalar / \
                 type-mismatched value (text + struct edits aren't supported yet)"
            )?,
            // Anything else is an expression read against the frame's live values;
            // the value prints to stdout like a normal REPL result.
            None => match session.debug_eval(t) {
                Some(v) => println!("{v}"),
                None => writeln!(
                    chrome,
                    "couldn't evaluate `{t}` at the frame \
                     (:step/:next/:finish/:continue, `name = <expr>` to edit, :help)"
                )?,
            },
        },
    }
    Ok(false)
}

/// Print the current paused frame (function + its in-scope variables), or nothing
/// when no longer paused.
fn print_pause<W: Write>(session: &ReplSession, chrome: &mut W) -> std::io::Result<()> {
    if let Some(f) = session.paused_frame() {
        let vars: Vec<String> = f.locals.iter().map(|(n, v)| format!("{n} = {v}")).collect();
        writeln!(chrome, "⏸ paused in {} | {}", f.function, vars.join(", "))?;
    }
    Ok(())
}

/// Resume per `mode` and report: the new frame if it paused again, else that the
/// run finished and the sub-mode is left.
fn step_and_report<W: Write>(
    session: &mut ReplSession,
    mode: crate::debugger::StepMode,
    chrome: &mut W,
) -> std::io::Result<()> {
    if session.debug_step(mode) {
        print_pause(session, chrome)?;
    } else {
        writeln!(chrome, "▶ resumed — run finished")?;
    }
    Ok(())
}

/// Parse a `name = <expr>` edit typed at the paused prompt into `(name, rhs)`: a
/// single plain identifier on the left, `=`, a non-empty right side.  `None` for a
/// non-assignment — a read expression, a comparison (`n == 5`, whose `rhs` would
/// start with `=`), or a compound assign (`n += 1`, whose `lhs` isn't a bare
/// identifier) — so those route to frame evaluation instead.  The RHS is left
/// unparsed: [`ReplSession::debug_set`] evaluates it against the frame.
fn parse_assign(s: &str) -> Option<(&str, &str)> {
    let (lhs, rhs) = s.split_once('=')?;
    let name = lhs.trim();
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || name.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return None;
    }
    let rhs = rhs.trim();
    // A leading `=` means the operator was `==` (a comparison), not an assignment.
    if rhs.is_empty() || rhs.starts_with('=') {
        return None;
    }
    Some((name, rhs))
}

/// Reduce `Type::show`'s debug form to the loft-source type name: drop the
/// `[...]` dep-tracking list, then unwrap a `ref(...)` reference wrapper
/// (`vector<integer>["__vdb_1"]` → `vector<integer>`; `ref(P)` → `P`).  Shared by
/// the value-snapshot capture (its cap-fn return type + `show_loft` schema
/// lookup) and Tab completion's struct-field resolution.
fn base_type_name(show: &str) -> &str {
    let base = show.split('[').next().unwrap_or(show);
    base.strip_prefix("ref(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(base)
}

/// Render an `f64` as a loft `float` literal — always with a decimal point so a
/// whole number like `3` re-parses as `float`, not `integer`.  The f64-typed
/// adapter over the shared round-trip rule
/// ([`crate::state::loft_float_literal`]); the frame renderer uses the same rule.
fn float_literal(v: f64) -> String {
    crate::state::loft_float_literal(&v.to_string())
}

/// Render `raw` as a quoted, escaped loft `text` literal — the form the parser
/// re-reads.  Used by the REPL.X value-snapshot to store a captured text binding
/// as `name = "…"`.  Delegates to the shared
/// [`state::loft_text_literal`](crate::state::loft_text_literal) so the snapshot
/// and the breakpoint frame renderer escape identically.
fn escape_loft_text(raw: &str) -> String {
    crate::state::loft_text_literal(raw)
}

/// Render the return value (read off `state`'s stack top) of type `ret_ty` as an
/// own-format loft literal — the per-type half of [`ReplSession::capture_binding`].
/// `name` is the type's loft-source name (for the `show_loft` / enum schema
/// lookup).  Covers **every** value type:
///
/// - **inline** values read at their width and rendered directly: `integer`
///   (64-bit), `single`, `float`, `boolean`, `character`, `text`, and a simple
///   enum (an inline 1-based discriminant byte → `Enum.Variant`);
/// - **`DbRef`-backed heap** values (struct, vector, struct-enum variant): the
///   return is a 12-byte `DbRef` on the stack top → [`show_loft`] renders the
///   own-format literal.
///
/// `None` only on an unresolved type name (a fallback to source, never reached in
/// practice for a value the session just produced).
fn render_capture(state: &mut State, ret_ty: &Type, name: &str) -> Option<String> {
    match ret_ty {
        Type::Integer(_) => Some(state.get_stack::<i64>().to_string()),
        Type::Float => Some(float_literal(*state.get_stack::<f64>())),
        Type::Single => Some(format!("{}f", *state.get_stack::<f32>())),
        Type::Boolean => {
            let v = *state.get_stack::<u8>() != 0;
            Some(if v { "true" } else { "false" }.to_string())
        }
        Type::Character => char::from_u32(*state.get_stack::<u32>()).map(|c| format!("'{c}'")),
        Type::Text(_) => Some(escape_loft_text(
            state.get_stack::<crate::keys::Str>().str(),
        )),
        // Heap value backed by a `DbRef`: struct, vector, struct-enum variant.
        // The return is a 12-byte `DbRef` on the stack top → `show_loft` renders
        // its own-format literal (`P{a:7,b:9}`, `[10,20,30]`, …).  `name` is the
        // loft-source type name; its schema `tp` comes from `Stores::name`.
        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => {
            let tp = state.database.name(name);
            if tp == u16::MAX {
                return None;
            }
            let db = *state.get_stack::<crate::keys::DbRef>();
            let mut out = String::new();
            state.database.show_loft(&mut out, &db, tp);
            Some(out)
        }
        // Simple enum: an inline 1-based discriminant byte → `Enum.Variant`.
        Type::Enum(_, false, _) => {
            let tp = state.database.name(name);
            if tp == u16::MAX {
                return None;
            }
            let disc = *state.get_stack::<u8>();
            if disc == 0 {
                Some("null".to_string())
            } else {
                Some(format!("{name}.{}", state.database.enum_val(tp, disc)))
            }
        }
        _ => None,
    }
}

/// Outcome of trying to value-snapshot a binding's RHS (REPL.X capture).
enum Capture {
    /// The value was captured — store `name = <this literal>`.
    Done(String),
    /// Not capturable (un-inferable type, cap-fn parse failure, unresolved type
    /// name) and the RHS did **not** fault — fall back to storing the RHS as
    /// source.
    Skip,
    /// The RHS **faulted** while being run to snapshot it — surface the error at
    /// the binding and store nothing, so the effect ran once and the session is
    /// not poisoned by a re-running source binding.
    Failed(Vec<DiagEntry>),
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
    /// @PLN15 G1 — the run hit a breakpoint and **suspended** (interactive
    /// stepping is on).  The session now holds the live frame: inspect it with
    /// [`ReplSession::paused_frame`], edit a value with
    /// [`ReplSession::debug_set`], and resume with [`ReplSession::debug_step`] /
    /// [`ReplSession::debug_continue`].  The observing statement finishes (and
    /// prints its value) when the run is resumed to completion.
    Paused,
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
    /// @PLN15 G1 — breakpoint specs (`:break` command), **function-scoped**:
    /// `"foo"` (body start) or `"foo:3"` (line 3 of `foo`).  Re-applied to the
    /// fresh `State` of every observing run.
    breakpoints: Vec<String>,
    /// Frames captured at breakpoints during the most recent observing run
    /// (record-and-continue mode — when `stepping` is off).
    last_hits: Vec<crate::debugger::BreakHit>,
    /// @PLN15 G1 — **interactive stepping**: when on, an observing run that
    /// reaches a breakpoint *suspends* into the paused sub-mode (held in `paused`)
    /// instead of recording all hits and continuing.  The interactive driver turns
    /// it on; programmatic/piped callers that want the full hit list leave it off.
    stepping: bool,
    /// @PLN15 G1 — a run suspended at a breakpoint, held across REPL inputs so the
    /// user can inspect the frame, edit a value, and step.  `None` unless paused.
    /// Boxed because `State` is large and the paused case is rare.
    paused: Option<Box<State>>,
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
            breakpoints: Vec::new(),
            last_hits: Vec::new(),
            stepping: false,
            paused: None,
        })
    }

    /// Build a session over an existing `parser` already loaded with a program's
    /// definitions — used by the @PLN15 debugger to evaluate at a paused frame with
    /// the program's types + functions in scope.  The accumulated body starts
    /// empty; persistence is off.
    #[must_use]
    pub fn from_parser(parser: Parser) -> Self {
        Self {
            parser,
            body: String::new(),
            counter: 0,
            record: None,
            replaying: false,
            breakpoints: Vec::new(),
            last_hits: Vec::new(),
            stepping: false,
            paused: None,
        }
    }

    /// Seed this session with a paused frame's variables (a @PLN15
    /// [`BreakHit`](crate::debugger::BreakHit)): each captured
    /// `(name, own-format literal)` becomes a binding `name = <literal>`, so
    /// expressions evaluated afterwards run against the frame's live values — the
    /// **REPL-at-frame** bridge.  It reuses the literal each value already renders
    /// to, so it needs no store-resident environment (exact value-for-value seeding
    /// with frame mutation is the @PLN14 upgrade).  Returns the number of variables
    /// bound; a value whose type isn't in scope (seeding a struct on a stdlib-only
    /// session) is skipped, not fatal.
    pub fn seed_frame(&mut self, hit: &crate::debugger::BreakHit) -> usize {
        let mut bound = 0;
        for (name, literal) in &hit.locals {
            if matches!(self.eval(&format!("{name} = {literal}")), Eval::Ran) {
                bound += 1;
            }
        }
        bound
    }

    /// Evaluate a boolean `condition` against a captured frame — the @PLN15 E
    /// **conditional / test breakpoint** primitive.  Seeds the frame (D1) then
    /// `assert`s the condition: returns `true` iff it holds, so a caller keeps only
    /// the hits where it does ("break when `i > 3`") or where an invariant is
    /// violated ("break when `!(balance >= 0)`").  The session is mutated
    /// (re-seeded each call — a rebind, safe for one breakpoint's same-shaped
    /// frames), so build it with [`from_parser`](Self::from_parser) so heap values'
    /// types are in scope.  This filters *recorded* hits post-run; skipping the
    /// capture in-loop (so a non-matching breakpoint never even pauses) would need
    /// the condition pre-compiled against the frame — a later refinement.
    pub fn frame_holds(&mut self, hit: &crate::debugger::BreakHit, condition: &str) -> bool {
        self.seed_frame(hit);
        matches!(
            self.eval(&format!("assert({condition}, \"cond\")")),
            Eval::Ran
        )
    }

    /// The current value of `expr` in this session, rendered as an own-format loft
    /// literal (`"99"`, `"Point{x:3,y:4}"`) — or `None` if it doesn't evaluate.
    /// Reuses the value-snapshot capture, so it covers every type the REPL renders.
    /// The @PLN15 debugger uses it to read a value the user edited at a breakpoint
    /// (`n = 99`) before writing it back into the live frame.
    pub fn value_of(&mut self, expr: &str) -> Option<String> {
        match self.capture_binding(expr) {
            Capture::Done(lit) => Some(lit),
            Capture::Skip | Capture::Failed(_) => None,
        }
    }

    /// Add a breakpoint (the `:break` command).  **Function-scoped** forms only —
    /// `foo` (the body start of function `foo`) or `foo:3` (line 3 of `foo`) —
    /// because that is the only form unique in the REPL (every input parses under
    /// the synthetic file `"<repl>"` with line numbers restarting at 1, so a bare
    /// line is not unique; `file:line` is for a file-run debugger).  Re-applied to
    /// the fresh `State` of every later observing run.
    pub fn add_breakpoint(&mut self, spec: &str) {
        let spec = spec.trim().to_string();
        if !spec.is_empty() && !self.breakpoints.contains(&spec) {
            self.breakpoints.push(spec);
        }
    }

    /// The breakpoint specs set this session.
    #[must_use]
    pub fn breakpoints(&self) -> &[String] {
        &self.breakpoints
    }

    /// Remove all breakpoints.
    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    /// Frames captured at breakpoints during the most recent observing run.
    #[must_use]
    pub fn last_hits(&self) -> &[crate::debugger::BreakHit] {
        &self.last_hits
    }

    /// @PLN15 G1 — turn **interactive stepping** on or off.  When on, an observing
    /// run that reaches a breakpoint *suspends* into the paused sub-mode (rather
    /// than recording every hit and continuing): inspect with
    /// [`paused_frame`](Self::paused_frame), edit with [`debug_set`](Self::debug_set),
    /// resume with [`debug_step`](Self::debug_step) / [`debug_continue`](Self::debug_continue).
    /// The interactive REPL driver enables it; programmatic callers that want the
    /// full hit list (e.g. a conditional-breakpoint sweep via
    /// [`frame_holds`](Self::frame_holds)) leave it off.
    pub fn debug_stepping(&mut self, on: bool) {
        self.stepping = on;
    }

    /// Whether a run is currently **suspended** at a breakpoint (the paused
    /// sub-mode is active).
    #[must_use]
    pub fn is_debugging(&self) -> bool {
        self.paused.is_some()
    }

    /// The frame at the current suspension, or `None` if not paused.
    #[must_use]
    pub fn paused_frame(&self) -> Option<&crate::debugger::BreakHit> {
        self.paused.as_deref().and_then(State::paused_frame)
    }

    /// Edit scalar local `name` in the **live** paused frame to the value of `rhs`
    /// (the user types `n = 99`, `f = 2.0`, `b = !b` at the paused prompt), then
    /// refresh the frame view.  `rhs` is **evaluated against the frame** first (so
    /// it may be any expression — `n + 1`, `!b` — not just a literal), then written
    /// type-directed by the local's declared type.  Returns `false` when not paused,
    /// `rhs` doesn't evaluate, `name` isn't a local, the value's type doesn't match
    /// the local, or the local is text / heap (those need the store-resident
    /// write-back, not yet built).  The edit is picked up when the run resumes — the
    /// @PLN15 F edit-and-continue, driven from the REPL.
    pub fn debug_set(&mut self, name: &str, rhs: &str) -> bool {
        let Some(lit) = self.debug_eval(rhs) else {
            return false;
        };
        let Some(state) = self.paused.as_deref_mut() else {
            return false;
        };
        let ok = state.set_frame_literal(name, &lit, &self.parser.data);
        if ok {
            state.refresh_paused_frame(&self.parser.data);
        }
        ok
    }

    /// Resume the suspended run, stopping per `mode` (the step verbs —
    /// [`StepMode`](crate::debugger::StepMode)).  Returns `true` if it paused again
    /// (the new frame is in [`paused_frame`](Self::paused_frame)), `false` if the
    /// run finished — in which case the paused sub-mode is left
    /// ([`is_debugging`](Self::is_debugging) becomes `false`).
    pub fn debug_step(&mut self, mode: crate::debugger::StepMode) -> bool {
        let Some(state) = self.paused.as_deref_mut() else {
            return false;
        };
        let still = state.debug_step(mode, &self.parser.data);
        if !still {
            self.paused = None;
        }
        still
    }

    /// Continue to the next breakpoint or the end of the run (the `:continue`
    /// verb) — [`debug_step`](Self::debug_step) with
    /// [`StepMode::Continue`](crate::debugger::StepMode::Continue).
    pub fn debug_continue(&mut self) -> bool {
        self.debug_step(crate::debugger::StepMode::Continue)
    }

    /// Evaluate `expr` against the **current paused frame** and render its value as
    /// an own-format loft literal (`"15"`, `"Point{x:3,y:4}"`), or `None` if it
    /// doesn't evaluate / there is no pause.  This is the REPL-at-frame: typing an
    /// expression at the `(dbg)` prompt reads the frame's live variables (`n * 2`,
    /// `pt.x * pt.y`).  It reuses the D1 bridge — the frame's captured variables
    /// render to literals, so binding them as the evaluation body and running
    /// [`value_of`](Self::value_of) covers **every** type the frame holds.
    ///
    /// Read-only: it runs on a throwaway `State` (the held paused state is
    /// untouched), and any value the user has edited with
    /// [`debug_set`](Self::debug_set) is already reflected in the frame's literals.
    /// The session body is swapped to the frame bindings only for this call and
    /// always restored — even if the evaluation unwinds.
    pub fn debug_eval(&mut self, expr: &str) -> Option<String> {
        let prefix = {
            let frame = self.paused.as_deref()?.paused_frame()?;
            let mut p = String::new();
            for (name, lit) in &frame.locals {
                p.push_str(name);
                p.push_str(" = ");
                p.push_str(lit);
                p.push_str(";\n");
            }
            p
        };
        let saved = std::mem::replace(&mut self.body, prefix);
        let result =
            std::panic::catch_unwind(AssertUnwindSafe(|| self.value_of(expr))).unwrap_or(None);
        self.body = saved; // restore the real session body, panic or not
        result
    }

    /// Abandon a paused debug session, dropping the held state — used to recover the
    /// REPL after a debug operation panics.  The breakpoints stay set.
    pub fn abort_debug(&mut self) {
        self.paused = None;
    }

    /// Resolve + set the session's breakpoint specs on a freshly-compiled `state`
    /// (called by `compile_generation` before an observing run).  Specs are
    /// **function-scoped** — `<fn>:<line>` or `<fn>` (body start) — because that is
    /// the only form unique in the REPL: every input parses under the synthetic
    /// file `"<repl>"` with line numbers restarting at 1, so a bare or file:line
    /// number is not unique here (file:line is for a file-run debugger).  An
    /// unresolvable spec (unknown fn / no such line) is skipped this run, not error.
    fn apply_breakpoints(&self, state: &mut State) {
        for spec in &self.breakpoints {
            if let Some((name, line)) = spec.split_once(':') {
                if let Ok(l) = line.trim().parse::<u32>() {
                    let d = self.parser.data.def_nr(&format!("n_{}", name.trim()));
                    state.set_breakpoint_fn_line(d, l, &self.parser.data);
                }
            } else {
                state.set_breakpoint_fn_start(spec, &self.parser.data);
            }
        }
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
        if let Some(var) = Self::binding_name(input) {
            // REPL.X value-snapshot — run the RHS once, capture the value, and
            // store the binding as a literal (`name = 42`) so re-running `body`
            // on every later observe does NOT repeat a side effect.
            let rhs = input.split_once('=').map_or("", |(_, r)| r.trim());
            if !rhs.is_empty() {
                match self.capture_binding(rhs) {
                    Capture::Done(lit) => {
                        let snap = format!("{var} = {lit}");
                        let bound = format!("{}{snap};\n", self.body);
                        if self.compile_generation(&bound, false, false).is_ok() {
                            self.body = bound;
                            self.record_input(&snap); // persist the snapshot, not the RHS
                            return Eval::Ran;
                        }
                        // (rare) the rendered literal didn't recompile — fall through.
                    }
                    // The RHS faulted while we snapshotted it: surface the error at
                    // the binding and store NOTHING — the effect ran once and no
                    // re-running source binding can poison later observes.
                    Capture::Failed(diags) => return Eval::Error(diags),
                    Capture::Skip => {} // not capturable — fall back to source
                }
            }
            // Fall back: record the binding as source (validate only).  It is
            // recompiled, in use, when a later input observes it.
            let bound = format!("{}{};\n", self.body, input);
            return match self.compile_generation(&bound, false, false) {
                Ok(()) => {
                    self.body = bound;
                    self.record_input(input);
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
        if self.compile_generation(&shown, true, true).is_ok() {
            return if self.is_debugging() {
                Eval::Paused
            } else {
                Eval::Ran
            };
        }
        let plain = format!("{}{};\n", self.body, input);
        match self.compile_generation(&plain, true, true) {
            Ok(()) if self.is_debugging() => Eval::Paused,
            Ok(()) => Eval::Ran,
            Err(diags) => Eval::Error(diags),
        }
    }

    /// Parse one generation fn `fn replmain_N() { <gen_body> }`; when `execute`,
    /// run it.  On a parse error rolls `data` back and returns the diagnostics.
    /// When not executing (a binding), the generation's def is rolled back too —
    /// it lives on only as source in `body`.
    fn compile_generation(
        &mut self,
        gen_body: &str,
        execute: bool,
        debug: bool,
    ) -> Result<(), Vec<DiagEntry>> {
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
            // @PLN15 G1 — apply the session's breakpoints to this run.  In stepping
            // mode a hit *suspends* execution; otherwise it records-and-continues.
            // Only on a real observing run (`debug`), not the value-render re-runs
            // (`:vars`, snapshot validation).
            if debug && !self.breakpoints.is_empty() {
                self.apply_breakpoints(&mut state);
                if self.stepping {
                    state.enable_stepping();
                }
            }
            state.execute_argv(&name, &self.parser.data, &[]);
            if debug && state.is_paused() {
                // Suspended at a breakpoint (interactive stepping): hold the live
                // state so the caller can inspect / edit / step it.  The gen def
                // stays in `data` — its bytecode is what the held state runs — and
                // the observing wrapper's `println` fires when the run is resumed
                // to completion.  Return early; the run is not yet finished.
                self.paused = Some(Box::new(state));
                return Ok(());
            }
            if debug {
                self.last_hits = state.debug_hits().to_vec();
                for hit in &self.last_hits {
                    let vars: Vec<String> = hit
                        .locals
                        .iter()
                        .map(|(n, v)| format!("{n} = {v}"))
                        .collect();
                    println!("⏸ break in {} | {}", hit.function, vars.join(", "));
                }
            }
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

    /// REPL.X value-snapshot — run a binding's RHS **once**, capture its value,
    /// and render it as an own-format loft literal so the binding can be stored
    /// as `name = <literal>` (side-effect-free on every later re-run of `body`).
    /// Returns a [`Capture`]: `Done(literal)` to store the snapshot, `Failed` if
    /// the RHS faulted (surface the error, store nothing), or `Skip` if it is not
    /// capturable (fall back to storing the RHS as source).
    ///
    /// Mechanism: build `fn replmain_N() -> <T> { <body> <rhs> }` so the RHS is
    /// the trailing return expression, run it on a throwaway `State`, and read
    /// the value off the **stack top** — where `execute_at` reads its own return,
    /// so no new execution entry point.  Dispatch on the value's type:
    ///
    /// Dispatch is on the value's exact [`Type`] (in [`render_capture`]) and
    /// covers **every** type: inline scalars/text/simple-enum rendered directly,
    /// and `DbRef`-backed heap values (struct, vector, struct-enum) rendered by
    /// `show_loft` on the returned `DbRef`.  A binding whose RHS isn't a simple
    /// `<name> = <expr>`, or whose value's type name doesn't resolve, falls back
    /// to storing the RHS as source (re-run).
    fn capture_binding(&mut self, rhs: &str) -> Capture {
        // `Type::show` is a debug form: it appends a dep-tracking list
        // (`vector<integer>["__vdb_1"]`) and wraps a struct as `ref(P)`.  The
        // cap-fn return type and the `show_loft` schema lookup both need the
        // loft-SOURCE name — so reduce to the base type name.
        let Some(ty_show) = self.infer_type(rhs) else {
            return Capture::Skip;
        };
        let ty = base_type_name(&ty_show).to_string();
        let next = self.counter + 1;
        let name = format!("replmain_{next}");
        let src = format!("fn {name}() -> {ty} {{\n{}{rhs}\n}}\n", self.body);
        let pre_defs = self.parser.data.definitions();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        if failed {
            self.parser.data.rollback_to(pre_defs);
            return Capture::Skip;
        }
        self.counter = next;
        // The value's exact type drives the stack read (the inferred type
        // *string* above is only for the fn signature + the schema lookup).
        let cap_d = self.parser.data.def_nr(&format!("n_{name}"));
        let ret_ty = self.parser.data.def(cap_d).returned.clone();
        crate::scopes::check(&mut self.parser.data);
        let mut state = State::new(self.parser.database.clone());
        compile::byte_code(&mut state, &mut self.parser.data);
        state.execute_argv(&name, &self.parser.data, &[]);
        // The RHS just ran (its side effect happened once).  A fault here is a
        // real binding error — surface it, don't fall back to source (which would
        // re-run the fault on every later observe and poison the session).
        if let Some(err) = state.database.runtime_error.take() {
            self.parser.data.rollback_to(pre_defs);
            return Capture::Failed(vec![err.to_diag_entry()]);
        }
        let lit = render_capture(&mut state, &ret_ty, &ty);
        self.parser.data.rollback_to(pre_defs); // discard the throwaway cap gen
        lit.map_or(Capture::Skip, Capture::Done)
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

    /// The identifiers Tab completion offers (the `:`-commands are added by the
    /// completer itself): every global function (user + stdlib, operators
    /// excluded), every struct/enum/base-type name, and the variables bound this
    /// session.  Synthetic and operator definitions are filtered out by
    /// [`is_plain_ident`].  Sorted + deduped; recomputed by the interactive loop
    /// after each input.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn completion_names(&self) -> Vec<String> {
        let data = &self.parser.data;
        let mut names: Vec<String> = Vec::new();
        for d in 0..data.definitions() {
            let def = data.def(d);
            match def.def_type {
                DefType::Function => {
                    // Operators and methods (`t_…`) aren't called by bare name.
                    if def.is_operator() {
                        continue;
                    }
                    if let Some(n) = def.name.strip_prefix("n_")
                        && !n.starts_with("repl")
                        && is_plain_ident(n)
                    {
                        names.push(n.to_string());
                    }
                }
                DefType::Struct | DefType::Enum | DefType::Type if is_plain_ident(&def.name) => {
                    names.push(def.name.clone());
                }
                _ => {}
            }
        }
        names.extend(self.bound_var_names());
        names.sort();
        names.dedup();
        names
    }

    /// The full completion model for the current session: the bare-identifier
    /// [`completion_names`](Self::completion_names) plus dotted-access `members`.
    /// A receiver's members are what may follow `receiver.`:
    ///
    /// - a **variable** → its type's methods (each rendered `method(` so the
    ///   completion shows it is callable and leaves the cursor inside the call),
    ///   plus, if the type is a struct, its field names (bare — a field names a
    ///   value, not a call);
    /// - an **enum type** name → its variant names (`Color.Red`, bare).
    ///
    /// Rebuilt by the interactive loop after each input.  `&mut` because
    /// resolving each variable's type runs a throwaway, rolled-back
    /// type-inference probe (no execution).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn completion_model(&mut self) -> CompletionModel {
        let names = self.completion_names();
        // Resolve variable types first (a `&mut` probe) before borrowing the
        // schema immutably below.
        let var_types = self.infer_var_types();
        let mut members: HashMap<String, Vec<String>> = HashMap::new();
        let data = &self.parser.data;
        let db = &self.parser.database;
        // Enum *type* names → their variant names (for `Color.Red` qualified
        // access).  Struct *type* names get no members — a struct is built with
        // `Type{…}`, not dotted — so only enums are added here.
        for d in 0..data.definitions() {
            let def = data.def(d);
            if def.def_type == DefType::Enum && is_plain_ident(&def.name) {
                let tp = db.name(&def.name);
                if tp != u16::MAX
                    && let Parts::Enum(variants) = &db.types[tp as usize].parts
                {
                    let mut vs: Vec<String> = variants.iter().map(|(_, n)| n.clone()).collect();
                    vs.sort();
                    members.insert(def.name.clone(), vs);
                }
            }
        }
        // *Variables* → their type's methods (`method(`) plus, for a struct, its
        // fields (bare).  The type name comes from `Type::name` — the SAME name
        // methods register under (`t_<len><name>_<method>`), so the lookup
        // matches for structs, text, vectors, and every base type uniformly.
        for (var, ty) in &var_types {
            let tname = ty.name(data);
            let mut ms: Vec<String> = methods_for_type(data, &tname);
            let tp = db.name(&tname);
            if tp != u16::MAX
                && let Parts::Struct(fields) = &db.types[tp as usize].parts
            {
                ms.extend(fields.iter().map(|f| f.name.clone()));
            }
            if !ms.is_empty() {
                ms.sort();
                ms.dedup();
                members.insert(var.clone(), ms);
            }
        }
        CompletionModel { names, members }
    }

    /// Infer the static type of every bound variable in a *single* probe: build
    /// `fn replmain_N() { <body> __cmpl0 = v0; … }`, read each temp's inferred
    /// type from the function's variable table, and roll the probe back — one
    /// parse for all variables.  Returns `var → Type` (the type indices it holds
    /// point at definitions that predate, and so survive, the rollback); a
    /// variable whose type can't be read is omitted.  Compile-time only —
    /// nothing executes, so a side-effecting binding does not run here.
    #[cfg(not(target_arch = "wasm32"))]
    fn infer_var_types(&mut self) -> HashMap<String, Type> {
        use std::fmt::Write;
        let vars = self.bound_var_names();
        let mut out = HashMap::new();
        if vars.is_empty() {
            return out;
        }
        let name = format!("replmain_{}", self.counter + 1);
        let mut probe = self.body.clone();
        for (i, v) in vars.iter().enumerate() {
            let _ = writeln!(probe, "__cmpl{i} = {v};");
        }
        let src = format!("fn {name}() {{\n{probe}}}\n");
        let pre_defs = self.parser.data.definitions();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        if !failed {
            let d = self.parser.data.def_nr(&format!("n_{name}"));
            if d != u32::MAX {
                let def = self.parser.data.def(d);
                let v = &def.variables;
                for i in 0..v.count() {
                    if let Some(idx) = v
                        .name(i)
                        .strip_prefix("__cmpl")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(src_var) = vars.get(idx)
                    {
                        out.insert(src_var.clone(), v.tp(i).clone());
                    }
                }
            }
        }
        self.parser.data.rollback_to(pre_defs);
        out
    }

    /// The variables bound this session, each once in first-seen order (a rebind
    /// keeps the original position).  Derived from the binding lines accumulated
    /// in `body`.  Feeds both `:vars` and Tab completion.
    fn bound_var_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for line in self.body.lines() {
            if let Some(name) = Self::binding_name(line)
                && !names.contains(&name)
            {
                names.push(name);
            }
        }
        names
    }

    /// Print each session variable and its current value (`name = value`, in
    /// loft's native rendering) to stdout — the `:vars` command.  Returns
    /// `Ok(false)` when nothing is bound (the caller reports it) or `Ok(true)`
    /// once values are printed.  Realising the values re-runs the accumulated
    /// body, so a side effect in a binding's RHS repeats here too (REPL.X).
    ///
    /// # Errors
    /// Returns the diagnostics if realising a value raises a parse or runtime
    /// error.
    pub fn show_vars(&mut self) -> Result<bool, Vec<DiagEntry>> {
        use std::fmt::Write;
        let names = self.bound_var_names();
        if names.is_empty() {
            return Ok(false);
        }
        // Append one `println("name = {name}")` per variable after the body, so
        // a single run renders every current value through the same path a bare
        // expression uses.
        let mut gen_src = self.body.clone();
        for n in &names {
            let _ = writeln!(gen_src, "println(\"{n} = {{{n}}}\");");
        }
        self.compile_generation(&gen_src, true, false)?;
        Ok(true)
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod completion_tests {
    use super::{CompletionModel, complete_word};
    use std::collections::HashMap;

    /// A model mirroring a session with globals `Point dbl print println x`; a
    /// struct variable `p` with a method `scale` and fields `{x, y}`; a text
    /// variable `s` with methods `{length, starts_with}`; and an enum type
    /// `Color{Red, Green, Blue}`.  `names` is pre-sorted, as `completion_names`
    /// returns it; each `members` list is sorted with methods rendered `name(`,
    /// as `completion_model` builds them.
    fn model() -> CompletionModel {
        let names = ["Point", "dbl", "print", "println", "x"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let members = HashMap::from([
            (
                "p".to_string(),
                vec!["scale(".to_string(), "x".to_string(), "y".to_string()],
            ),
            (
                "s".to_string(),
                vec!["length(".to_string(), "starts_with(".to_string()],
            ),
            (
                "Color".to_string(),
                vec!["Blue".to_string(), "Green".to_string(), "Red".to_string()],
            ),
        ]);
        CompletionModel { names, members }
    }

    #[test]
    fn completes_identifier_prefix() {
        let (start, out) = complete_word(&model(), "pr", 2);
        assert_eq!(start, 0);
        assert_eq!(out, vec!["print".to_string(), "println".to_string()]);
    }

    /// Only the word under the cursor is replaced, not the whole line.
    #[test]
    fn completes_word_mid_line() {
        let line = "1 + db";
        let (start, out) = complete_word(&model(), line, line.len());
        assert_eq!(start, 4); // "db" begins at byte 4
        assert_eq!(out, vec!["dbl".to_string()]);
    }

    #[test]
    fn completes_colon_command() {
        let (start, out) = complete_word(&model(), ":by", 3);
        assert_eq!(start, 1); // replace after the ':'
        assert_eq!(out, vec!["bytecode".to_string()]);
    }

    /// A `:command` with an argument falls through to identifier completion of
    /// the argument (e.g. `:rust db` completes a fn name).
    #[test]
    fn colon_command_arg_completes_identifier() {
        let line = ":rust db";
        let (start, out) = complete_word(&model(), line, line.len());
        assert_eq!(start, 6);
        assert_eq!(out, vec!["dbl".to_string()]);
    }

    /// A stray Tab with no prefix offers nothing (rather than every name).
    #[test]
    fn empty_prefix_yields_nothing() {
        let (_start, out) = complete_word(&model(), "1 + ", 4);
        assert!(out.is_empty());
    }

    // ── REPL.C member completion ─────────────────────────────────────────────

    /// `p.` with an empty prefix lists all of the struct variable's members —
    /// its methods (rendered `name(`) and its fields, together and sorted.
    #[test]
    fn dot_lists_all_struct_members() {
        let (start, out) = complete_word(&model(), "p.", 2);
        assert_eq!(start, 2); // replace after the dot
        assert_eq!(
            out,
            vec!["scale(".to_string(), "x".to_string(), "y".to_string()]
        );
    }

    /// A field prefix after the dot filters to the matching field.
    #[test]
    fn dot_filters_struct_fields_by_prefix() {
        let (start, out) = complete_word(&model(), "p.x", 3);
        assert_eq!(start, 2);
        assert_eq!(out, vec!["x".to_string()]);
    }

    /// A method completes with its trailing `(` so the user sees it is callable.
    #[test]
    fn dot_completes_method_with_paren() {
        let (start, out) = complete_word(&model(), "s.st", 4);
        assert_eq!(start, 2);
        assert_eq!(out, vec!["starts_with(".to_string()]);
    }

    /// A non-struct receiver (here a `text` variable) still completes its
    /// methods after the dot — `.` is never a dead end for a typed value.
    #[test]
    fn dot_lists_methods_for_non_struct() {
        let (start, out) = complete_word(&model(), "s.", 2);
        assert_eq!(start, 2);
        assert_eq!(out, vec!["length(".to_string(), "starts_with(".to_string()]);
    }

    /// An enum *type* name completes its variants after the dot.
    #[test]
    fn dot_completes_enum_variants() {
        let (start, out) = complete_word(&model(), "Color.G", 7);
        assert_eq!(start, 6);
        assert_eq!(out, vec!["Green".to_string()]);
    }

    /// Member completion works mid-line, replacing only the partial field.
    #[test]
    fn dot_completes_mid_line() {
        let line = "1 + p.x";
        let (start, out) = complete_word(&model(), line, line.len());
        assert_eq!(start, 6); // the `x` after `p.`
        assert_eq!(out, vec!["x".to_string()]);
    }

    /// A non-matching field prefix yields nothing — not a fall-through to globals.
    #[test]
    fn dot_unknown_field_yields_nothing() {
        let (_start, out) = complete_word(&model(), "p.zzz", 5);
        assert!(out.is_empty());
    }

    /// An unknown / non-struct receiver offers nothing, and never leaks the
    /// global list after a dot (`x` is a global but must not appear here).
    #[test]
    fn dot_unknown_receiver_yields_nothing() {
        let (_start, out) = complete_word(&model(), "foo.x", 5);
        assert!(out.is_empty(), "globals must not leak after a dot: {out:?}");
    }

    /// A non-identifier receiver (index / call result) needs live inference — the
    /// documented residual — so it yields nothing rather than guessing.
    #[test]
    fn dot_non_identifier_receiver_yields_nothing() {
        let (_start, out) = complete_word(&model(), "arr[0].x", 8);
        assert!(out.is_empty());
    }
}
