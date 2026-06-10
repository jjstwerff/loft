// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN16 M5d phase 2 — the **`--rpc` debug server**: an NDJSON request/response + event
//! stream over the [wire protocol](../../doc/claude/plans/16-debugger/PROTOCOL.md) that
//! drives the debug engine non-interactively.  One JSON object per line: the client
//! sends `{ "id", "req", … }` requests, the server replies `{ "id", "ok", … }` and emits
//! `{ "event", … }` events (`stopped` / `output` / `terminated`).
//!
//! Requests are parsed with loft's **inbuilt JSON parser** ([`crate::json::parse`]); a
//! value shown to the client (an `eval` result) is serialised with loft's **inbuilt JSON
//! serializer** (a struct/enum via its `.to_json()` method, scalars as raw JSON literals —
//! [`ReplSession::debug_eval_json`]).  The server is a thin driver over [`ReplSession`] —
//! every request is one engine method (the protocol invariant: no UI-only, no agent-only
//! semantics).
//!
//! The debugged program's `print` output is captured (the thread-local sink below) and
//! streamed as `output` events, so it never corrupts the protocol stream on stdout.

use crate::json::{ParseError, Parsed};
use crate::repl::ReplSession;
use std::cell::RefCell;
use std::io::{BufRead, Write};

thread_local! {
    /// While `Some`, the loft `print` builtin appends here instead of writing stdout —
    /// so the program's output can be drained into `output` events.  `None` (the default)
    /// is one branch on the print path.
    static OUTPUT_SINK: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Called by the `print` builtin (`fill.rs`): route the program's output to the capture
/// sink when one is active (returns `true`), else let the caller write stdout (`false`).
#[must_use]
pub fn print_or_capture(text: &str) -> bool {
    OUTPUT_SINK.with(|s| {
        let mut b = s.borrow_mut();
        if let Some(buf) = b.as_mut() {
            buf.push_str(text);
            true
        } else {
            false
        }
    })
}

pub(crate) fn capture_begin() {
    OUTPUT_SINK.with(|s| *s.borrow_mut() = Some(String::new()));
}
pub(crate) fn capture_end() {
    OUTPUT_SINK.with(|s| *s.borrow_mut() = None);
}
/// Take the captured program output since the last drain (empty if none / not capturing).
fn capture_take() -> String {
    OUTPUT_SINK.with(|s| {
        s.borrow_mut()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    })
}

/// Run the file-run debugger over the wire protocol: read NDJSON requests from `input`,
/// write NDJSON responses + events to `out`, until EOF or a `disconnect` request.
///
/// # Errors
/// Returns an I/O error from the input / output streams.
pub fn run_rpc<R: BufRead, W: Write>(
    stdlib_dir: &str,
    input: R,
    out: &mut W,
) -> std::io::Result<()> {
    let mut session = ReplSession::new(stdlib_dir)?;
    session.debug_stepping(true);
    // Silence the raw panic handler — a fault in the debugged program is caught and
    // reported as `terminated`, not printed.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    capture_begin();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let (messages, disconnect) = handle(&mut session, &line);
        for m in messages {
            writeln!(out, "{m}")?;
        }
        out.flush()?;
        if disconnect {
            break;
        }
    }
    capture_end();
    std::panic::set_hook(prev_hook);
    Ok(())
}

// ── request handling ─────────────────────────────────────────────────────────

/// Handle one request line; returns the NDJSON messages to emit (response first, then
/// any events) and whether the client asked to disconnect.
pub(crate) fn handle(session: &mut ReplSession, line: &str) -> (Vec<String>, bool) {
    let parsed = match crate::json::parse(line) {
        Ok(p) => p,
        Err(e) => return (vec![resp_err(0, &parse_err(&e))], false),
    };
    let id = num(&parsed, "id").unwrap_or(0);
    let req = text(&parsed, "req").unwrap_or("");
    let mut out = Vec::new();
    match req {
        "launch" => match session.load_program(text(&parsed, "file").unwrap_or("")) {
            Ok(Ok(())) => out.push(resp_ok(id, "")),
            Ok(Err(diags)) => out.push(resp_err(id, &diags_msg(&diags))),
            Err(e) => out.push(resp_err(id, &format!("cannot read file: {e}"))),
        },
        "replEval" => {
            // @PLN16 M5e — the REPL panel's context-aware eval.  A top-level expression
            // prints its value to the capture sink, so drain anything pending first, then
            // drain again after to capture *this* eval's output (the value / any prints),
            // keeping REPL output distinct from a program `run`'s streamed output.
            let _ = capture_take();
            let outcome = session.repl_eval(text(&parsed, "input").unwrap_or(""), true);
            let printed = capture_take();
            let mut extra = format!("\"context\":\"{}\"", outcome.context);
            if outcome.more {
                extra.push_str(",\"more\":true");
            }
            if let Some(v) = &outcome.value {
                let _ = std::fmt::Write::write_fmt(&mut extra, format_args!(",\"value\":{v}"));
            }
            if !printed.is_empty() {
                let _ = std::fmt::Write::write_fmt(
                    &mut extra,
                    format_args!(",\"output\":{}", esc(&printed)),
                );
            }
            out.push(resp_ok(id, &extra));
            if !outcome.diagnostics.is_empty() {
                out.push(diagnostics_event("<repl>", &outcome.diagnostics));
            }
        }
        "compile" => {
            // @PLN16 M5e slice 2 — the compiler-console feed: check the file (no run, no
            // load) and emit its diagnostics (errors + warnings) as a structured event.
            let file = text(&parsed, "file").unwrap_or("");
            match session.compile(file) {
                Ok(diags) => {
                    out.push(resp_ok(id, ""));
                    out.push(diagnostics_event(file, &diags));
                }
                Err(e) => out.push(resp_err(id, &format!("cannot read file: {e}"))),
            }
        }
        "setBreakpoints" => {
            set_breakpoints(session, &parsed);
            out.push(resp_ok(id, ""));
        }
        "setWatch" => {
            let ok = session.add_watchpoint(text(&parsed, "expr").unwrap_or(""));
            out.push(if ok {
                resp_ok(id, "")
            } else {
                resp_err(id, "not a watchable scalar region")
            });
        }
        "clearWatch" => {
            session.clear_watchpoints();
            out.push(resp_ok(id, ""));
        }
        "run" => {
            out.push(resp_ok(id, ""));
            let entry = text(&parsed, "entry").unwrap_or("main");
            let halted = session.eval_observe(&format!("{entry}()"));
            report(session, "breakpoint", halted, &mut out);
        }
        "continue" => {
            out.push(resp_ok(id, ""));
            let halted = session.debug_continue();
            report(session, "breakpoint", halted, &mut out);
        }
        "stepIn" | "stepOver" | "stepOut" => {
            out.push(resp_ok(id, ""));
            let mode = match req {
                "stepIn" => crate::debugger::StepMode::Into,
                "stepOut" => crate::debugger::StepMode::Out,
                _ => crate::debugger::StepMode::Over,
            };
            let halted = session.debug_step(mode);
            report(session, "step", halted, &mut out);
        }
        "eval" => {
            let v = session.debug_eval_json(text(&parsed, "expr").unwrap_or(""));
            out.push(resp_ok(
                id,
                &format!("\"value\":{}", v.as_deref().unwrap_or("null")),
            ));
        }
        "setValue" => {
            let ok = session.debug_set(
                text(&parsed, "target").unwrap_or(""),
                text(&parsed, "value").unwrap_or(""),
            );
            out.push(if ok {
                resp_ok(id, &frame_field(session))
            } else {
                resp_err(id, "edit rejected")
            });
        }
        "undo" => out.push(step_resp(id, session.debug_undo(), session)),
        "redo" => out.push(step_resp(id, session.debug_redo(), session)),
        "stackTrace" => out.push(resp_ok(id, &frame_field(session))),
        "disconnect" => {
            out.push(resp_ok(id, ""));
            return (out, true);
        }
        other => out.push(resp_err(id, &format!("unknown request: {other}"))),
    }
    (out, false)
}

/// `setBreakpoints { file, breakpoints: [{ line, condition?, log?, stop? }] }` — replace
/// the session's breakpoints with the request's (v1: replaces all; single-file debug).
fn set_breakpoints(session: &mut ReplSession, parsed: &Parsed) {
    session.clear_breakpoints();
    let file = text(parsed, "file").unwrap_or("");
    let Some(Parsed::Array(bps)) = field(parsed, "breakpoints") else {
        return;
    };
    for bp in bps {
        let Some(line) = num(bp, "line") else {
            continue;
        };
        let condition = text(bp, "condition").map(str::to_string);
        let stop = bp_bool(bp, "stop").unwrap_or(true);
        let actions: Vec<String> = match field(bp, "log") {
            Some(Parsed::Array(a)) => a.iter().filter_map(as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        session.add_file_breakpoint_rich(file, line as u32, condition, actions, stop);
    }
}

/// After a run/continue/step, emit the program output + tracepoints + the stop verdict.
fn report(session: &mut ReplSession, step_reason: &str, paused: bool, out: &mut Vec<String>) {
    let captured = capture_take();
    for chunk in captured.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        if !line.is_empty() {
            out.push(event(
                "output",
                &format!("\"category\":\"stdout\",\"text\":{}", esc(line)),
            ));
        }
    }
    for t in session.take_trace_output() {
        out.push(event(
            "output",
            &format!("\"category\":\"trace\",\"text\":{}", esc(&t)),
        ));
    }
    if paused {
        let watch = session.take_watch_hit();
        let reason = if watch.is_some() {
            "watch"
        } else {
            step_reason
        };
        let mut body = format!("\"reason\":\"{reason}\",{}", frame_field(session));
        if let Some(h) = watch {
            use std::fmt::Write as _;
            let _ = write!(
                body,
                ",\"watch\":{{\"label\":{},\"old\":{},\"new\":{}}}",
                esc(&h.label),
                esc(&h.old),
                esc(&h.new)
            );
        }
        out.push(event("stopped", &body));
    } else {
        out.push(event("terminated", ""));
    }
}

/// A `frame` field for the current pause (`"frame": { function, locals: [...] }`), or
/// `"frame":null` when not paused.  Locals carry their own-format rendering as `value`.
fn frame_field(session: &ReplSession) -> String {
    match session.paused_frame() {
        Some(f) => {
            let locals: Vec<String> = f
                .locals
                .iter()
                .map(|(n, v)| format!("{{\"name\":{},\"value\":{}}}", esc(n), esc(v)))
                .collect();
            format!(
                "\"frame\":{{\"function\":{},\"locals\":[{}]}}",
                esc(&f.function),
                locals.join(",")
            )
        }
        None => "\"frame\":null".to_string(),
    }
}

/// An `ok` response carrying the refreshed frame (for `undo`/`redo`).
fn step_resp(id: i64, ok: bool, session: &ReplSession) -> String {
    if ok {
        resp_ok(id, &frame_field(session))
    } else {
        resp_err(id, "nothing to do")
    }
}

// ── tiny JSON helpers ────────────────────────────────────────────────────────

/// A field of a JSON object, or `None` if `p` isn't an object / lacks the key.
fn field<'a>(p: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match p {
        Parsed::Object(fields) => fields.iter().find(|(k, _, _)| k == key).map(|(_, _, v)| v),
        _ => None,
    }
}
fn as_str(p: &Parsed) -> Option<&str> {
    match p {
        Parsed::Str(s) => Some(s),
        _ => None,
    }
}
fn text<'a>(p: &'a Parsed, key: &str) -> Option<&'a str> {
    field(p, key).and_then(as_str)
}
#[allow(clippy::cast_possible_truncation)]
fn num(p: &Parsed, key: &str) -> Option<i64> {
    match field(p, key) {
        Some(Parsed::Number(n)) => Some(*n as i64),
        _ => None,
    }
}
fn bp_bool(p: &Parsed, key: &str) -> Option<bool> {
    match field(p, key) {
        Some(Parsed::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// A JSON string literal (quoted + escaped) — the one home for protocol string escaping.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn resp_ok(id: i64, extra: &str) -> String {
    if extra.is_empty() {
        format!("{{\"id\":{id},\"ok\":true}}")
    } else {
        format!("{{\"id\":{id},\"ok\":true,{extra}}}")
    }
}
fn resp_err(id: i64, msg: &str) -> String {
    format!("{{\"id\":{id},\"ok\":false,\"error\":{}}}", esc(msg))
}
fn event(name: &str, body: &str) -> String {
    if body.is_empty() {
        format!("{{\"event\":\"{name}\"}}")
    } else {
        format!("{{\"event\":\"{name}\",{body}}}")
    }
}

/// @PLN16 M5e slice 2 — the compiler console's feed: a `diagnostics` event listing the
/// file's diagnostics (errors + warnings), each structured (line / col / level / message)
/// so the editor can mark them and the console can list them.  Emitted even when the list
/// is empty, so a re-compile clears stale markers.
fn diagnostics_event(file: &str, diags: &[crate::diagnostics::DiagEntry]) -> String {
    let items = diags
        .iter()
        .map(|d| {
            format!(
                "{{\"line\":{},\"col\":{},\"level\":{},\"message\":{}}}",
                d.line,
                d.col,
                esc(level_str(d.level)),
                esc(&d.message)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    event(
        "diagnostics",
        &format!("\"file\":{},\"items\":[{items}]", esc(file)),
    )
}

/// The protocol spelling of a diagnostic [`Level`](crate::diagnostics::Level).
fn level_str(level: crate::diagnostics::Level) -> &'static str {
    use crate::diagnostics::Level;
    match level {
        Level::Debug => "debug",
        Level::Warning => "warning",
        Level::Error => "error",
        Level::Fatal => "fatal",
    }
}

fn diags_msg(diags: &[crate::diagnostics::DiagEntry]) -> String {
    diags
        .iter()
        .map(crate::diagnostics::DiagEntry::to_string_compact)
        .collect::<Vec<_>>()
        .join("; ")
}
fn parse_err(e: &ParseError) -> String {
    format!("bad json: {e:?}")
}
