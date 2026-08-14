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
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Parse the embedded stdlib + `program_src` into an interpreter session ready to
/// debug (compiled, not yet run).  Returns `false` if either does not parse
/// clean.  Replaces any previous session.
#[must_use]
pub fn start(program_src: &str) -> bool {
    let mut p = Box::new(crate::parser::Parser::new());
    for (name, content) in crate::stdlib_sources::STDLIB_SOURCES {
        if !p.parse_source(content, name, true) {
            return false;
        }
    }
    p.parse_source(program_src, "program.loft", false);
    if p.diagnostics.level() >= crate::diagnostics::Level::Error {
        return false;
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
        });
    });
    true
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
        "bp" if !arg.is_empty() => {
            let ok = sess.state.set_breakpoint_fn_start(arg, data).is_some();
            vec![format!("D:{} bp {arg}", if ok { "ok" } else { "err" })]
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
                let locals = hit
                    .locals
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
    let name = format!("__eval_{}", sess.counter + 1);
    let src = format!("fn {name}({sig}) -> {ret_ty} {{\n  ({expr})\n}}\n");
    let pre_diag = sess.parser.diagnostics.entries().len();
    sess.parser.parse_str(&src, "<debug>", false);
    let failed = sess.parser.diagnostics.entries()[pre_diag..]
        .iter()
        .any(|e| e.level >= crate::diagnostics::Level::Error);
    if failed {
        return None;
    }
    sess.counter += 1;
    crate::scopes::check(&mut sess.parser.data);
    let d = sess.parser.data.def_nr(&format!("n_{name}"));
    if d == u32::MAX {
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
}
