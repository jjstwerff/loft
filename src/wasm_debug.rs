// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN98 P3.4 — the browser debug CLIENT.

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
            let v = sess
                .state
                .paused_frame()
                .and_then(|f| {
                    f.locals
                        .iter()
                        .find(|(n, _)| n == arg)
                        .map(|(_, v)| v.clone())
                })
                .unwrap_or_else(|| "<unknown>".to_string());
            vec![format!("D:eval {arg}={v}")]
        }
        _ => vec![format!("D:err unknown command {cmd:?}")],
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
            let frame = sess.state.database.host_input_native();
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
            // eval the live frame local.
            assert_eq!(apply(sess, "eval n"), vec!["D:eval n=40"]);
            // resume -> runs to completion.
            assert_eq!(apply(sess, "resume"), vec!["D:terminated"]);
        });
    }
}
