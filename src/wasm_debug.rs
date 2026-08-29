// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I78 — Live-reload dispatch: the browser debug CLIENT (@PLN98 P3.4), the
// live/debug tier's browser-side sibling to live_dispatch.rs.

//! The interactive debug client a browser (`--html --debug`) build runs: the
//! program executes INTERPRETED over a parked [`State`], breakpoints PAUSE it
//! cooperatively (P3.2 — `execute_argv` / `debug_step` return to the JS event
//! loop, never block), and `D!:` control frames arrive over the `host_input`
//! channel (relayed from the server the client holds a WebSocket to), with `D:`
//! replies going back over the host output.  It reuses the `State`'s own debug
//! methods directly — NOT the native `debug_cmd_dispatch` / TCP machinery — so
//! the whole path is wasm-safe (no threads, no sockets, no process spawn).
//!
//! Split for testability: [`apply`] is a pure `(session, frame) -> replies`
//! step (unit-tested natively); [`pump`] wires it to the wasm host I/O.

use std::cell::RefCell;

use crate::debugger::StepMode;
use crate::state::State;

/// The parked interpreter + its owning parser, plus the run's lifecycle flags.
struct Session {
    /// Owns the `Data` the `State` executes against (its address must stay put;
    /// boxed, held here for the session's lifetime).
    parser: Box<crate::parser::Parser>,
    state: Box<State>,
    /// `execute_argv` has been entered (later resumes use `debug_step`).
    started: bool,
    /// `main` ran to completion — further `resume` is a no-op.
    done: bool,
    /// Names each synthetic eval fn (`__eval_<n>`) uniquely across evals.
    counter: u32,
    /// The first definition index belonging to the PROGRAM rather than the stdlib.
    /// The stdlib is parsed first, so everything from here on is the reader's own code —
    /// which is what `fns` has to list and what the panel offers to call.
    user_from: u32,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Parse the embedded stdlib + `program_src` into an interpreter session ready to
/// debug (compiled, not yet run).  Returns `false` if either does not parse
/// clean.  Replaces any previous session.
///
/// The program goes through [`parse_str`](crate::parser::Parser::parse_str) — the entry
/// the REPL uses — and that choice is load-bearing, not incidental.  Every `eval` compiles
/// a synthetic function through the same call, and `parse_str` begins with
/// `Data::reset`, which resolves names under `STD_SOURCE`.  A program parsed through
/// `parse_source` instead registers its definitions under its OWN source, where that
/// resolution cannot see them: `eval len("abc")` answered 3 while `eval fib(10)` answered
/// `<unavailable>` — the stdlib reachable, the program's own functions not, which is the
/// opposite of what a REPL over a paused program is for.  Parsing both in one source is
/// what makes the definitions in scope.
#[must_use]
pub fn start(program_src: &str) -> bool {
    start_reporting(program_src).is_ok()
}

/// [`start`], with the diagnostics on failure instead of a bare `false`.
///
/// A page that offers a REPL over the code in front of the reader has to say WHY the code
/// did not compile — "it didn't work" is not something a reader can act on. The bool form
/// stays for the callers that only branch on it.
///
/// # Errors
/// The parse diagnostics, rendered, when the stdlib or the program does not compile.
pub fn start_reporting(program_src: &str) -> Result<(), String> {
    let mut p = Box::new(crate::parser::Parser::new());
    for (name, content) in crate::stdlib_sources::STDLIB_SOURCES {
        if !p.parse_source(content, name, true) {
            return Err(format!(
                "the bundled standard library ({name}) did not compile"
            ));
        }
    }
    let user_from = p.data.definitions();
    p.parse_str(program_src, "program.loft", false);
    if p.diagnostics.level() >= crate::diagnostics::Level::Error {
        return Err(p.diagnostics.to_string());
    }
    crate::scopes::check(&mut p.data);
    let mut state = Box::new(State::new(p.database.clone()));
    crate::compile::byte_code(&mut state, &mut p.data);
    // Stepping mode so a registered breakpoint SUSPENDS (returns) instead of
    // record-and-continue — the cooperative pause.
    state.enable_stepping();
    SESSION.with(|s| {
        *s.borrow_mut() = Some(Session {
            parser: p,
            state,
            started: false,
            done: false,
            counter: 0,
            user_from,
        });
    });
    Ok(())
}

/// Apply one `D!:`-stripped control command to `sess`, returning the `D:` replies
/// (0+).  The command grammar mirrors the TCP debug channel's:
/// - `bp <fn>` — break at the entry of `<fn>` (`D:ok`/`D:err`).
/// - `run` / `resume` — start (or continue) the run to the next breakpoint or
///   completion; `D:hit <fn> <locals>` when it pauses, `D:terminated` when done.
/// - `step` — resume one source line.
/// - `eval <name>` — read frame local `<name>` at the pause (`D:eval <name>=<v>`).
fn apply(sess: &mut Session, cmd: &str) -> Vec<String> {
    let (verb, arg) = cmd
        .split_once(' ')
        .map_or((cmd, ""), |(a, b)| (a, b.trim()));
    // Disjoint borrows: `state` and `parser` are separate fields.
    let data = &sess.parser.data;
    match verb {
        // `bp <fn>` names a function; `bp <line>` names a line of the program, which is
        // what a gutter click in a browser panel has to send — it knows where the reader
        // clicked and not which function that is inside.  A loft function name cannot
        // start with a digit, so the two forms cannot be confused.
        "bp" if !arg.is_empty() => {
            let ok = match arg.parse::<u32>() {
                Ok(line) => sess
                    .state
                    .set_breakpoint_file_line("program.loft", line, data)
                    .is_some(),
                Err(_) => sess.state.set_breakpoint_fn_start(arg, data).is_some(),
            };
            vec![format!("D:{} bp {arg}", if ok { "ok" } else { "err" })]
        }
        // The program's own callable functions, so a REPL panel can say what there is to
        // call instead of leaving the reader to scroll up and guess.  Drawn from the
        // definitions the PROGRAM added (everything after the stdlib), rendered through
        // `api_surface::signature_of` — the same spelling `loft api` and the LSP hover use.
        "fns" => {
            let mut out = Vec::new();
            for d in sess.user_from..data.definitions() {
                if let Some(("fn", name)) = crate::api_surface::classify(data, d)
                    && !name.starts_with("__")
                    && name != "main"
                {
                    let sig = crate::api_surface::signature_of(data, d, "fn");
                    out.push(format!("{name}{sig}"));
                }
            }
            vec![format!("D:fns {}", out.join("|"))]
        }
        "run" | "resume" | "step" => {
            if sess.done {
                return vec!["D:terminated".to_string()];
            }
            let mode = if verb == "step" {
                StepMode::Into
            } else {
                StepMode::Continue
            };
            if sess.started {
                let paused = sess.state.debug_step(mode, data);
                if !paused {
                    sess.done = true;
                }
            } else {
                sess.started = true;
                // The entry run: a registered breakpoint suspends (stepping mode).
                sess.state.execute_argv("main", data, &[]);
                if !sess.state.is_paused() {
                    sess.done = true;
                }
            }
            if let Some(hit) = sess.state.paused_frame() {
                // `user_locals` and not `locals`: the frame also holds the compiler's own
                // scratch (`__work_N` format buffers, the `#`-infixed loop machinery), and
                // a reader looking at a panel of their own variables should not have to
                // know which of them they wrote.  Display-only — `eval <name>` still
                // resolves a temp for anyone who wants one.
                let locals = hit
                    .user_locals()
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join("|");
                vec![format!("D:hit {} {locals}", hit.function)]
            } else {
                vec!["D:terminated".to_string()]
            }
        }
        "eval" if !arg.is_empty() => {
            vec![format!("D:eval {arg}={}", eval_expr(sess, arg))]
        }
        _ => vec![format!("D:err unknown command {cmd:?}")],
    }
}

/// @PLN98 P3.4 — full-expression eval over the paused frame.  Binds every
/// referenced live local as a typed arg of a synthetic fn and evaluates it via
/// [`State::eval_frame_reenter`](crate::state::State::eval_frame_reenter), reading
/// each local where it lives.  Handles arbitrary expressions (`2 + 2`, `n + 2`,
/// `h["a"].v`, `len(v)`, `s.field`).  A bare heap ident is read live in place; an
/// expression that can't be bound/compiled (a `text` local, a parse error) yields
/// `<unavailable>` rather than a wrong value.
///
/// The reach is measured, not assumed: a SCALAR result and a STRUCT result evaluate, a
/// `text` or `vector` result does not (loft#1187).  `<unavailable>` is the safe answer
/// there rather than a missing feature — every route that returns a text through this path
/// corrupts the store, because the `Type::Text` arm of `eval_frame_reenter` is only sound
/// for a call-returned-owned buffer.  The native debugger has the full surface; it reaches
/// it through the REPL's reconstruct path instead.
fn eval_expr(sess: &mut Session, expr: &str) -> String {
    let expr = expr.trim();
    if is_bare_ident(expr) {
        if let Some(v) = sess.state.eval_frame_heap(expr, false, &sess.parser.data) {
            return v;
        }
        if let Some(v) = sess.state.paused_frame().and_then(|f| {
            f.locals
                .iter()
                .find(|(n, _)| n == expr)
                .map(|(_, v)| v.clone())
        }) {
            return v;
        }
    }
    eval_via_reenter(sess, expr).unwrap_or_else(|| "<unavailable>".to_string())
}

fn is_bare_ident(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The identifiers appearing in `expr` (over-approximate — string contents are
/// included, but only actual frame-local names are ever bound, so it's harmless).
fn idents(expr: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut std::collections::HashSet<String>| {
        if cur
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            out.insert(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    for c in expr.chars() {
        if c.is_alphanumeric() || c == '_' {
            cur.push(c);
        } else if !cur.is_empty() {
            flush(&mut cur, &mut out);
        }
    }
    if !cur.is_empty() {
        flush(&mut cur, &mut out);
    }
    out
}

/// Bind the referenced live locals as args, infer the result type, and evaluate
/// `expr` over the paused frame.  A heap result can't ride the frame base, so it's
/// serialised in-fn with `.to_json()` (as [`State::eval_frame_reenter`] documents).
fn eval_via_reenter(sess: &mut Session, expr: &str) -> Option<String> {
    let refs = idents(expr);
    let mut binds: Vec<(String, String)> = Vec::new();
    {
        let frame = sess.state.paused_frame()?;
        for (name, _) in &frame.locals {
            if refs.contains(name)
                && let Some(ty) = sess.state.frame_local_arg_type(name, &sess.parser.data)
            {
                binds.push((name.clone(), ty));
            }
        }
    }
    let arg_names: Vec<String> = binds.iter().map(|(n, _)| n.clone()).collect();
    let sig = binds
        .iter()
        .map(|(n, t)| format!("{n}: {t}"))
        .collect::<Vec<_>>()
        .join(", ");
    let ret = infer_ret(sess, &sig, expr)?;
    if is_scalar_type(&ret) {
        build_run(sess, &sig, expr, &ret, &arg_names)
    } else {
        build_run(
            sess,
            &sig,
            &format!("({expr}).to_json()"),
            "text",
            &arg_names,
        )
    }
}

/// Compile `fn _(sig) {{ __t = (expr); }}` and read `__t`'s (base) type.  Rolled
/// back — a throwaway probe.  `None` if it doesn't type-check.
fn infer_ret(sess: &mut Session, sig: &str, expr: &str) -> Option<String> {
    let name = format!("__evalinfer_{}", sess.counter + 1);
    let src = format!("fn {name}({sig}) {{\n  __t = ({expr});\n}}\n");
    let pre_defs = sess.parser.data.definitions();
    let pre_diag = sess.parser.diagnostics.entries().len();
    sess.parser.parse_str(&src, "<debug>", false);
    let failed = sess.parser.diagnostics.entries()[pre_diag..]
        .iter()
        .any(|e| e.level >= crate::diagnostics::Level::Error);
    let result = if failed {
        None
    } else {
        let d = sess.parser.data.def_nr(&format!("n_{name}"));
        (d != u32::MAX)
            .then(|| {
                let def = sess.parser.data.def(d);
                let vars = &def.variables;
                (0..vars.count())
                    .find(|&i| vars.name(i) == "__t")
                    .map(|i| vars.tp(i).show(&sess.parser.data, vars))
            })
            .flatten()
    };
    sess.parser.data.rollback_to(pre_defs);
    result.map(|s| base_type(&s).to_string())
}

/// Compile `fn _(sig) -> ret_ty {{ (expr) }}` and evaluate it over the paused frame
/// (the args are `arg_names`, the live frame locals).  The def is kept — its
/// bytecode is appended into the paused State by `eval_frame_reenter`.
fn build_run(
    sess: &mut Session,
    sig: &str,
    expr: &str,
    ret_ty: &str,
    arg_names: &[String],
) -> Option<String> {
    // The counter advances whether or not this compiles, and a wreck is rolled back, so
    // one expression that does not evaluate cannot reach the next one.  Both halves are
    // needed and for the same reason: the name was reused after a failure AND the failed
    // parse's definition was left in `data`, so the following eval collided with it and
    // every eval from then on answered `<unavailable>` — a session a reader ends with
    // their first typo.
    sess.counter += 1;
    let name = format!("__eval_{}", sess.counter);
    let src = format!("fn {name}({sig}) -> {ret_ty} {{\n  ({expr})\n}}\n");
    let pre_defs = sess.parser.data.definitions();
    let pre_diag = sess.parser.diagnostics.entries().len();
    sess.parser.parse_str(&src, "<debug>", false);
    let failed = sess.parser.diagnostics.entries()[pre_diag..]
        .iter()
        .any(|e| e.level >= crate::diagnostics::Level::Error);
    if failed {
        sess.parser.data.rollback_to(pre_defs);
        return None;
    }
    crate::scopes::check(&mut sess.parser.data);
    let d = sess.parser.data.def_nr(&format!("n_{name}"));
    if d == u32::MAX {
        sess.parser.data.rollback_to(pre_defs);
        return None;
    }
    let ret_type = sess.parser.data.def(d).returned.clone();
    sess.state
        .eval_frame_reenter(&mut sess.parser.data, d, arg_names, &ret_type, false)
}

fn base_type(show: &str) -> &str {
    let base = show.split('[').next().unwrap_or(show);
    base.strip_prefix("ref(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(base)
}

fn is_scalar_type(t: &str) -> bool {
    matches!(
        t,
        "integer" | "float" | "single" | "boolean" | "character" | "byte"
    ) || t.starts_with("integer(")
}

/// Apply one debug command to the live session and return its `D:` replies.
///
/// The DIRECT entry, for a caller that already holds the session in its own process — the
/// doc site's panel, which calls straight into the wasm module and has no relay to speak
/// `D!:` frames through, and the native tests, which drive the same grammar [`pump`] does.
/// `pump` is the same step wired to the host I/O channel instead.
///
/// `D:err no session` when [`start`] has not run, so a caller that lost its session gets an
/// answer rather than silence.
#[must_use]
pub fn command(cmd: &str) -> Vec<String> {
    SESSION.with(|s| {
        let mut g = s.borrow_mut();
        match g.as_mut() {
            Some(sess) => apply(sess, cmd),
            None => vec!["D:err no session".to_string()],
        }
    })
}

// ── @PLN149 step 8 — the entries a doc page calls ───────────────────────────
//
// `crate::wasm::compile_and_run` runs a program and hands back its output; these two let a
// PAGE drive the same program — set a breakpoint, pause, read the frame, evaluate an
// expression against it, resume.  Until now this module was reachable only from a
// `--html --debug` build with the source BAKED IN and a WebSocket relay feeding it `D!:`
// frames.  A doc page has neither: it holds the source the reader is editing and calls
// straight in.  The command grammar is the same one [`pump`] applies, so the page and the
// relay drive one implementation rather than two that agree until they do not.

/// Start a debug session over `source`, replacing any previous one.
///
/// Returns JSON `{"ok":true}`, or `{"ok":false,"error":"…"}` carrying the diagnostics — a
/// page that offers to run the code in front of the reader has to say why it will not.
///
/// A bare script (top-level statements, no `fn main`) is desugared exactly as
/// `compile_and_run` desugars it, so the two entries accept the same inputs.
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
#[must_use]
pub fn debug_start(source: &str) -> String {
    let desugared = crate::script::script_desugar(source);
    let src = desugared.as_deref().unwrap_or(source);
    // The capture buffer is per-thread and shared with `compile_and_run`; a stale tail from
    // an earlier run would otherwise surface as output of this session's first `run`.
    let _ = take_output();
    let value = match start_reporting(src) {
        Ok(()) => json_object(&[("ok", crate::json::Parsed::Bool(true))]),
        Err(e) => json_object(&[
            ("ok", crate::json::Parsed::Bool(false)),
            ("error", crate::json::Parsed::Str(e)),
        ]),
    };
    crate::json::to_json_string(&value)
}

/// Apply one debug command to the live session.
///
/// Returns JSON `{"replies":[…],"output":"…"}` — the `D:` replies the command produced and
/// whatever the program printed while it ran.  The two travel together because a `run` or a
/// `resume` produces both, and a page fetching them separately could paint a pause before
/// the output that led to it.
///
/// The grammar is [`command`]'s: `bp <fn>` / `bp <line>`, `run`, `resume`, `step`,
/// `eval <expr>`, `fns`.
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
#[must_use]
pub fn debug_command(cmd: &str) -> String {
    use crate::json::Parsed;
    let replies = command(cmd);
    let value = json_object(&[
        (
            "replies",
            Parsed::Array(replies.into_iter().map(Parsed::Str).collect()),
        ),
        ("output", Parsed::Str(take_output())),
    ]);
    crate::json::to_json_string(&value)
}

/// A JSON object from `(key, value)` pairs, rendered through the one serialiser this repo
/// has (`json::to_json_string`, RFC 8259) rather than a fourth hand-rolled escaper.
fn json_object(fields: &[(&str, crate::json::Parsed)]) -> crate::json::Parsed {
    crate::json::Parsed::Object(
        fields
            .iter()
            .map(|(k, v)| ((*k).to_string(), 0, v.clone()))
            .collect(),
    )
}

/// Whatever the program printed since the last call.
///
/// Only a browser build captures print — `crate::wasm::output_push` is where the print op
/// routes under the `wasm` feature.  Natively the output goes to stdout as it always does,
/// so this is empty and the JSON field is present-but-empty rather than absent: one shape
/// for the page to read whichever build served it.
fn take_output() -> String {
    #[cfg(feature = "wasm")]
    {
        crate::wasm::output_take()
    }
    #[cfg(not(feature = "wasm"))]
    {
        String::new()
    }
}

/// The pump the JS driver calls (per animation frame): drain every pending
/// `host_input` frame, apply the `D!:`-tagged debug ones to the session (routing
/// non-`D!:` messages back as program input is the browser driver's job), and
/// emit each `D:` reply over the host output for the server relay.  No-op when no
/// session is active.
pub fn pump() {
    SESSION.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(sess) = guard.as_mut() else {
            return;
        };
        loop {
            // 0 = take what is queued and return; this pump must never block the
            // page's one thread, which is what would deliver the next frame.
            let frame = sess.state.database.host_input_native(0);
            if frame.is_empty() {
                break;
            }
            let Some(cmd) = frame.trim().strip_prefix("D!:") else {
                continue; // program input — not ours (the JS side re-queues it)
            };
            for reply in apply(sess, cmd.trim()) {
                crate::live_dispatch::wasm_host_log(&format!("{reply}\n"));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{SESSION, apply, start};

    // Drive the full debug cycle over the session with the SAME command grammar
    // the browser relay feeds: bp -> run (pause) -> eval a live local -> resume
    // (terminate). Uses `apply` directly (host I/O is the wasm-only wiring).
    #[test]
    fn debug_session_bp_run_eval_resume_cycle() {
        assert!(start(
            "fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}\n\
             fn main() { compute(40); }\n"
        ));
        SESSION.with(|s| {
            let mut g = s.borrow_mut();
            let sess = g.as_mut().expect("session");
            assert_eq!(apply(sess, "bp compute"), vec!["D:ok bp compute"]);
            // run -> pauses at compute's entry; the arg n=40 is live.
            let hit = apply(sess, "run");
            assert!(
                hit[0].starts_with("D:hit compute") && hit[0].contains("n=40"),
                "paused in compute with n=40: {hit:?}"
            );
            // eval the live frame local + a full EXPRESSION over it (the P3.4
            // full-eval: binds `n` as an arg of a synthetic fn, reenters).
            assert_eq!(apply(sess, "eval n"), vec!["D:eval n=40"]);
            assert_eq!(apply(sess, "eval n + 2"), vec!["D:eval n + 2=42"]);
            assert_eq!(apply(sess, "eval n * n"), vec!["D:eval n * n=1600"]);
            assert_eq!(apply(sess, "eval 2 + 3"), vec!["D:eval 2 + 3=5"]);
            // resume -> runs to completion.
            assert_eq!(apply(sess, "resume"), vec!["D:terminated"]);
        });
    }

    // The program's OWN functions are callable from `eval`, which is the whole point of a
    // REPL over a paused program: a reader types `fib(10)`, not `n + 2`.  They were not.
    // `eval` compiles its synthetic fn through `parse_str`, which resolves under
    // `STD_SOURCE`, while the program had been registered under its own source — so a
    // stdlib call worked and a call to the program answered `<unavailable>`.
    //
    // Both sides are asserted together on purpose: a guard on the program call alone would
    // also pass on a build where nothing resolves at all.
    #[test]
    fn eval_calls_the_programs_own_functions_and_the_stdlib() {
        assert!(start(
            "fn fib(n: integer) -> integer { if n < 2 { return n; } fib(n-1) + fib(n-2) }\n\
             fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}\n\
             fn main() { compute(40); }\n"
        ));
        SESSION.with(|s| {
            let mut g = s.borrow_mut();
            let sess = g.as_mut().expect("session");
            assert_eq!(apply(sess, "bp compute"), vec!["D:ok bp compute"]);
            assert!(apply(sess, "run")[0].starts_with("D:hit compute"));
            // A user fn on a literal.  fib(10) is 55, computed by hand.
            assert_eq!(apply(sess, "eval fib(10)"), vec!["D:eval fib(10)=55"]);
            // A user fn on the paused frame's own live local: compute's `n` is 40.
            assert_eq!(
                apply(sess, "eval fib(n - 30)"),
                vec!["D:eval fib(n - 30)=55"]
            );
            // The stdlib side of the split, which never broke and must not start.
            assert_eq!(
                apply(sess, "eval len(\"abc\")"),
                vec!["D:eval len(\"abc\")=3"]
            );
        });
    }

    // One expression that does not evaluate must not end the session.  It did: a failed
    // `build_run` left its half-parsed definition in `data` AND did not advance the
    // counter, so the next eval built the same name over the wreck and every eval after
    // the first failure answered `<unavailable>` — including ones that had just worked.
    //
    // A text-valued expression is the failing input here because that is the shape a
    // reader hits first (loft#1187 tracks making it evaluate); any non-evaluating
    // expression exercises the same path.
    #[test]
    fn a_failed_eval_does_not_end_the_session() {
        assert!(start(
            "fn fib(n: integer) -> integer { if n < 2 { return n; } fib(n-1) + fib(n-2) }\n\
             fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}\n\
             fn main() { compute(40); }\n"
        ));
        let s = SESSION.with(|s| {
            let mut g = s.borrow_mut();
            let sess = g.as_mut().expect("session");
            let _ = apply(sess, "bp compute");
            let _ = apply(sess, "run");
            // Works before.
            let before = apply(sess, "eval fib(10)");
            // Does not evaluate — and must not be fatal.
            let bad = apply(sess, "eval \"a\" + \"b\"");
            // Still works after, which is the whole assertion.
            let after = apply(sess, "eval fib(10)");
            let other = apply(sess, "eval n + 2");
            (before, bad, after, other)
        });
        assert_eq!(s.0, vec!["D:eval fib(10)=55"]);
        assert_eq!(s.1, vec!["D:eval \"a\" + \"b\"=<unavailable>"]);
        assert_eq!(
            s.2,
            vec!["D:eval fib(10)=55"],
            "a failed eval poisoned the session"
        );
        assert_eq!(s.3, vec!["D:eval n + 2=42"]);
    }
}
