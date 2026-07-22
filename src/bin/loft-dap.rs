// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// loft-dap — the loft Debug Adapter (DAP over JSON-RPC / stdio).
//
// @PLN63 LSP.3 (D0–D6, doc/claude/lib_plans/63-lsp/DAP.md).  Interactive
// interpreter-mode debugging of a `.loft` program in any DAP-aware editor
// (VS Code, Neovim `nvim-dap`, JetBrains via LSP4IJ): launch, set source-line
// breakpoints, hit them, inspect locals, step, continue, evaluate, edit a value.
//
// This binary is a PURE TRANSLATOR, not a debugger.  Every capability already
// exists behind the `--rpc` debug engine's one dispatch chokepoint
// (`loft::rpc::DebugDriver::drive`), whose NDJSON wire protocol was designed as the
// DAP shape on purpose (request/response + async `stopped`/`output`/`terminated`
// events).  loft-dap holds the debuggee IN-PROCESS (no child, no port) and adds only
// the DAP-specific layer: `Content-Length` framing, the `initialize` handshake, the
// `{seq,type,command,request_seq}` envelope, and the `threads → stackTrace → scopes →
// variables` drill-down synthesized from the engine's flat frame.  No engine
// semantics live here (the protocol invariant: no adapter-only behaviour).
//
// Protocol channel discipline: stdout carries ONLY framed JSON; a debuggee fault is
// captured as a `terminated` event, and the panic message is silenced (the hook
// below) so it can never corrupt the stream.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::Path;

use loft::json::{self, Parsed};
use loft::rpc::DebugDriver;

/// The single synthetic thread loft-dap reports (multi-worker `par` → one-per-worker
/// is a follow-up).  DAP inspection is thread → frame → scope → variable.
const THREAD_ID: i64 = 1;
/// The single stack frame id (v1 surfaces the current frame only; the RPC
/// `stackTrace` returns one frame — a true multi-frame stack is an engine follow-up).
const FRAME_ID: i64 = 1000;
/// Offset for the monotonic `variablesReference` counter — every scope + expansion handle
/// is minted from it and NEVER reused (VE3), so a reference from a prior stop is never
/// mistaken for a current node: a stale handle returns an empty `variables` list, never a
/// wrong subtree.  Clear of `FRAME_ID` (a separate DAP id space) and always non-zero.
const VAR_REF_BASE: i64 = 1000;

fn main() {
    // A debuggee fault must not print to stdout (the protocol channel); silence the raw
    // panic message — the engine reports the fault as a `terminated` event instead.
    std::panic::set_hook(Box::new(|_| {}));

    let args: Vec<String> = std::env::args().collect();
    let stdlib = resolve_stdlib_dir();
    // `--lib <dir>` import paths so the debugged file can `use` libraries (the same
    // `use`-resolution `loft debug --rpc` gives); the client passes them in its adapter
    // config's args.  Absent by default (the nvim "Run current file" config passes none).
    let lib_dirs = collect_lib_dirs(&args);
    let mut driver = match DebugDriver::new(&stdlib, &lib_dirs) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("loft-dap: cannot start debug session: {e}");
            std::process::exit(1);
        }
    };

    let mut adapter = Adapter::default();
    let mut stdin = io::stdin().lock();
    let stdout = io::stdout();
    while let Some(body) = read_message(&mut stdin) {
        let Ok(msg) = json::parse(&body) else {
            continue; // a malformed frame is dropped — DAP has no error reply for it
        };
        if adapter.dispatch(&msg, &mut driver, &stdout) {
            break; // disconnect / terminate
        }
    }
}

/// The DAP translation state: the outgoing `seq` counter, the launched program, and the
/// per-stop bookkeeping the drill-down reads from (Decision 3 — synthesize the DAP tree
/// from the engine's flat frame).
#[derive(Default)]
struct Adapter {
    /// Monotonic outgoing `seq` (responses + spontaneous events share it) — DAP requires
    /// every message to carry one.  Distinct from a request's `seq`, which is echoed as
    /// `request_seq` and forwarded to the engine as the RPC `id`.
    seq: i64,
    /// The launched program's path (from `launch`), used as the `stackTrace` source.
    program: String,
    /// `stopOnEntry` from `launch`; consumed at `configurationDone`.
    stop_on_entry: bool,
    /// Relabel the next `stopped` as `reason:"entry"` (set when a stopOnEntry run installs
    /// the entry breakpoint; the engine reports it as `breakpoint`).
    pending_entry: bool,
    /// Whether a run is currently suspended at a stop (drives whether the drill-down and a
    /// `variables` read return content vs empty).
    paused: bool,
    /// The current stop's locals `(name, rendered value)` — the `variables` panel content.
    locals: Vec<(String, String)>,
    /// The `variablesReference` handle for the current `Locals` scope; stale after a resume.
    locals_ref: i64,
    /// The monotonic source of every `variablesReference` (scopes + expansion handles) —
    /// only grows, so a handle is never reused across stops (VE3).
    next_ref: i64,
    /// VE — expansion handles: a `variablesReference` → the cached JSON node it expands.
    /// One `eval` per top-level struct/vector local caches its whole tree here, so drilling
    /// deeper navigates in memory (no re-eval).  Cleared on every resume + stop.
    var_values: HashMap<i64, Parsed>,
    /// The current frame's function name (the `stackTrace` frame label).
    frame_func: String,
    /// The source line the current stop is parked on (the editor's current-line marker).
    frame_line: Option<i64>,
}

impl Adapter {
    /// Dispatch one DAP request; returns `true` when the session should end (disconnect).
    fn dispatch(&mut self, msg: &Parsed, driver: &mut DebugDriver, out: &io::Stdout) -> bool {
        let command = field_str(msg, "command").unwrap_or_default();
        let request_seq = field_i64(msg, "seq").unwrap_or(0);
        let args = field(msg, "arguments");

        match command.as_str() {
            // ── D1 handshake ──────────────────────────────────────────────────────
            "initialize" => {
                // STRICT order: capabilities response FIRST, then the `initialized`
                // event — the client waits for `initialized` before `setBreakpoints`,
                // and reversing them hangs the session.
                self.respond(out, request_seq, &command, true, Some(capabilities()), None);
                self.event(out, "initialized", obj(vec![]));
            }

            // ── D2 launch (load only; the run is deferred to configurationDone) ────
            "launch" => {
                let program = args
                    .and_then(|a| field_str(a, "program"))
                    .unwrap_or_default();
                self.stop_on_entry = args
                    .and_then(|a| field_bool(a, "stopOnEntry"))
                    .unwrap_or(false);
                self.program = program.clone();
                let rpc = json::to_json_string(&obj(vec![
                    ("id", Parsed::Int(request_seq)),
                    ("req", Parsed::Str("launch".into())),
                    ("file", Parsed::Str(program)),
                ]));
                let (msgs, _) = driver.drive(&rpc);
                match rpc_ok(&msgs, request_seq) {
                    Ok(_) => self.respond(out, request_seq, &command, true, None, None),
                    Err(e) => self.respond(out, request_seq, &command, false, None, Some(&e)),
                }
            }

            // ── D3 breakpoints ────────────────────────────────────────────────────
            "setBreakpoints" => {
                let path = args
                    .and_then(|a| field(a, "source"))
                    .and_then(|s| field_str(s, "path"))
                    .unwrap_or_default();
                let rpc_bps: Vec<Parsed> = args
                    .and_then(|a| field(a, "breakpoints"))
                    .and_then(as_array)
                    .map(|arr| {
                        arr.iter()
                            .map(|b| {
                                let mut e =
                                    vec![("line", Parsed::Int(field_i64(b, "line").unwrap_or(0)))];
                                // Conditions pass straight through — the engine's
                                // resolve loop evaluates them (proven by tests/rpc.rs).
                                if let Some(c) = field_str(b, "condition") {
                                    e.push(("condition", Parsed::Str(c)));
                                }
                                obj(e)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let rpc = json::to_json_string(&obj(vec![
                    ("id", Parsed::Int(request_seq)),
                    ("req", Parsed::Str("setBreakpoints".into())),
                    ("file", Parsed::Str(path)),
                    ("breakpoints", Parsed::Array(rpc_bps)),
                ]));
                let (msgs, _) = driver.drive(&rpc);
                // Map the engine's `[{line, verified}]` to DAP `Breakpoint[]`; an
                // unbreakable line comes back `verified:false` so a dead breakpoint
                // is reported now, not by a stop that never comes.
                let verified: Vec<Parsed> = rpc_ok(&msgs, request_seq)
                    .ok()
                    .and_then(|r| field(&r, "breakpoints").and_then(as_array).cloned())
                    .unwrap_or_default()
                    .iter()
                    .map(|b| {
                        obj(vec![
                            (
                                "verified",
                                Parsed::Bool(field_bool(b, "verified").unwrap_or(false)),
                            ),
                            ("line", Parsed::Int(field_i64(b, "line").unwrap_or(0))),
                        ])
                    })
                    .collect();
                let body = obj(vec![("breakpoints", Parsed::Array(verified))]);
                self.respond(out, request_seq, &command, true, Some(body), None);
            }

            // ── D2 configurationDone — the deferred launch actually runs here ──────
            "configurationDone" => {
                self.respond(out, request_seq, &command, true, None, None);
                if self.stop_on_entry {
                    // stopOnEntry → pause at main's first statement via a function
                    // breakpoint, then relabel that first stop `reason:"entry"`.
                    driver.set_function_breakpoint("main");
                    self.pending_entry = true;
                }
                self.mark_resume();
                let (msgs, _) = driver.drive(r#"{"id":0,"req":"run","entry":"main"}"#);
                self.emit_events(out, &msgs);
            }

            // ── D4 inspection drill-down (adapter-local synthesis) ────────────────
            "threads" => {
                let body = obj(vec![(
                    "threads",
                    Parsed::Array(vec![obj(vec![
                        ("id", Parsed::Int(THREAD_ID)),
                        ("name", Parsed::Str("main".into())),
                    ])]),
                )]);
                self.respond(out, request_seq, &command, true, Some(body), None);
            }
            "stackTrace" => {
                let frames = if self.paused {
                    vec![obj(vec![
                        ("id", Parsed::Int(FRAME_ID)),
                        ("name", Parsed::Str(self.frame_func.clone())),
                        ("line", Parsed::Int(self.frame_line.unwrap_or(1))),
                        ("column", Parsed::Int(1)),
                        (
                            "source",
                            obj(vec![
                                ("path", Parsed::Str(self.program.clone())),
                                ("name", Parsed::Str(basename(&self.program))),
                            ]),
                        ),
                    ])]
                } else {
                    vec![]
                };
                let total = i64::try_from(frames.len()).unwrap_or(0);
                let body = obj(vec![
                    ("stackFrames", Parsed::Array(frames)),
                    ("totalFrames", Parsed::Int(total)),
                ]);
                self.respond(out, request_seq, &command, true, Some(body), None);
            }
            "scopes" => {
                let body = obj(vec![(
                    "scopes",
                    Parsed::Array(vec![obj(vec![
                        ("name", Parsed::Str("Locals".into())),
                        ("variablesReference", Parsed::Int(self.locals_ref)),
                        ("expensive", Parsed::Bool(false)),
                    ])]),
                )]);
                self.respond(out, request_seq, &command, true, Some(body), None);
            }
            "variables" => {
                let want = args
                    .and_then(|a| field_i64(a, "variablesReference"))
                    .unwrap_or(0);
                let vars = self.build_variables(driver, want);
                let body = obj(vec![("variables", Parsed::Array(vars))]);
                self.respond(out, request_seq, &command, true, Some(body), None);
            }

            // ── D5 stepping + continue ────────────────────────────────────────────
            "continue" => {
                let body = obj(vec![("allThreadsContinued", Parsed::Bool(true))]);
                self.respond(out, request_seq, &command, true, Some(body), None);
                self.mark_resume();
                let (msgs, _) = driver.drive(r#"{"id":0,"req":"continue"}"#);
                self.emit_events(out, &msgs);
            }
            "next" | "stepIn" | "stepOut" => {
                self.respond(out, request_seq, &command, true, None, None);
                self.mark_resume();
                let verb = match command.as_str() {
                    "next" => "stepOver",
                    "stepIn" => "stepIn",
                    _ => "stepOut",
                };
                let (msgs, _) = driver.drive(&format!("{{\"id\":0,\"req\":\"{verb}\"}}"));
                self.emit_events(out, &msgs);
            }

            // ── D6 evaluate + setVariable ─────────────────────────────────────────
            "evaluate" => {
                let expr = args
                    .and_then(|a| field_str(a, "expression"))
                    .unwrap_or_default();
                let rpc = json::to_json_string(&obj(vec![
                    ("id", Parsed::Int(request_seq)),
                    ("req", Parsed::Str("eval".into())),
                    ("expr", Parsed::Str(expr)),
                ]));
                let (msgs, _) = driver.drive(&rpc);
                match rpc_ok(&msgs, request_seq) {
                    Ok(resp) => {
                        let val = field(&resp, "value").cloned().unwrap_or(Parsed::Null);
                        let body = obj(vec![
                            ("result", Parsed::Str(render_value(&val))),
                            ("variablesReference", Parsed::Int(0)),
                        ]);
                        self.respond(out, request_seq, &command, true, Some(body), None);
                    }
                    Err(e) => self.respond(out, request_seq, &command, false, None, Some(&e)),
                }
            }
            "setVariable" | "setExpression" => {
                // setVariable carries `name`; setExpression carries `expression`.
                let target = args
                    .and_then(|a| field_str(a, "name").or_else(|| field_str(a, "expression")))
                    .unwrap_or_default();
                let value = args.and_then(|a| field_str(a, "value")).unwrap_or_default();
                let rpc = json::to_json_string(&obj(vec![
                    ("id", Parsed::Int(request_seq)),
                    ("req", Parsed::Str("setValue".into())),
                    ("target", Parsed::Str(target.clone())),
                    ("value", Parsed::Str(value.clone())),
                ]));
                let (msgs, _) = driver.drive(&rpc);
                match rpc_ok(&msgs, request_seq) {
                    Ok(resp) => {
                        // The engine returns the refreshed frame; update the cache so the
                        // next `variables` read reflects the edit, and echo the new value.
                        if let Some(frame) = field(&resp, "frame") {
                            self.locals = read_locals(frame);
                        }
                        let shown = self
                            .locals
                            .iter()
                            .find(|(n, _)| *n == target)
                            .map_or(value, |(_, v)| v.clone());
                        let body = obj(vec![
                            ("value", Parsed::Str(shown)),
                            ("variablesReference", Parsed::Int(0)),
                        ]);
                        self.respond(out, request_seq, &command, true, Some(body), None);
                    }
                    Err(e) => self.respond(out, request_seq, &command, false, None, Some(&e)),
                }
            }

            // ── boundaries — an honest capability bit or a clean error, never a wrong
            //    picture (§ Refusals) ───────────────────────────────────────────────
            "pause" => {
                // v1 has no async interrupt; `supportsTerminateRequest` is advertised
                // instead — a stop request maps to terminate/disconnect.
                self.respond(
                    out,
                    request_seq,
                    &command,
                    false,
                    None,
                    Some("pause is not supported (no async interrupt); use terminate"),
                );
            }
            "disconnect" | "terminate" => {
                let _ = driver.drive(&format!("{{\"id\":{request_seq},\"req\":\"disconnect\"}}"));
                self.respond(out, request_seq, &command, true, None, None);
                return true;
            }
            other => {
                self.respond(
                    out,
                    request_seq,
                    other,
                    false,
                    None,
                    Some(&format!("unsupported request: {other}")),
                );
            }
        }
        false
    }

    /// A resume (`run`/`continue`/`step`) invalidates the current stop: the paused state
    /// clears, so a `variables` read against a now-stale reference returns empty until the
    /// next stop mints a fresh one.
    fn mark_resume(&mut self) {
        self.paused = false;
        self.locals.clear();
        self.var_values.clear();
    }

    /// Translate the RPC messages produced by a resume into DAP events, ignoring the RPC
    /// response line (the DAP response was already sent for the triggering request).
    fn emit_events(&mut self, out: &io::Stdout, msgs: &[String]) {
        for line in msgs {
            let Ok(ev) = json::parse(line) else { continue };
            let Some(kind) = field_str(&ev, "event") else {
                continue; // a response line — already answered
            };
            match kind.as_str() {
                "output" => {
                    let cat = field_str(&ev, "category").unwrap_or_else(|| "stdout".into());
                    let text = field_str(&ev, "text").unwrap_or_default();
                    // The engine strips the trailing newline per line; DAP `output` is a
                    // stream, so restore it to keep console lines separate.  A tracepoint
                    // log is diagnostic, not program stdout → the `console` category.
                    let dap_cat = if cat == "trace" { "console" } else { "stdout" };
                    self.event(
                        out,
                        "output",
                        obj(vec![
                            ("category", Parsed::Str(dap_cat.into())),
                            ("output", Parsed::Str(format!("{text}\n"))),
                        ]),
                    );
                }
                "stopped" => {
                    self.on_stop(&ev);
                    let reason = if self.pending_entry {
                        self.pending_entry = false;
                        "entry".to_string()
                    } else {
                        // Map the engine reasons to DAP: `breakpoint`/`step` are shared
                        // spellings; a watch hit is DAP's `data breakpoint`.
                        match field_str(&ev, "reason").as_deref() {
                            Some("watch") => "data breakpoint".to_string(),
                            Some(r) => r.to_string(),
                            None => "breakpoint".to_string(),
                        }
                    };
                    self.event(
                        out,
                        "stopped",
                        obj(vec![
                            ("reason", Parsed::Str(reason)),
                            ("threadId", Parsed::Int(THREAD_ID)),
                            ("allThreadsStopped", Parsed::Bool(true)),
                        ]),
                    );
                }
                "terminated" => {
                    self.paused = false;
                    self.locals.clear();
                    // A synthesized `exited` (the debuggee process model) precedes
                    // `terminated` (the debug session ended).
                    self.event(out, "exited", obj(vec![("exitCode", Parsed::Int(0))]));
                    self.event(out, "terminated", obj(vec![]));
                }
                _ => {} // diagnostics / test events are not part of the debug flow
            }
        }
    }

    /// Cache the frame carried by a `stopped` event and mint this stop's Locals handle.
    fn on_stop(&mut self, ev: &Parsed) {
        self.var_values.clear(); // a fresh expansion tree per stop
        self.locals_ref = self.mint();
        self.paused = true;
        let frame = field(ev, "frame");
        self.frame_func = frame
            .and_then(|f| field_str(f, "function"))
            .unwrap_or_default();
        self.frame_line = frame.and_then(|f| field_i64(f, "line"));
        self.locals = frame.map(read_locals).unwrap_or_default();
    }

    // ── VE — structured variable expansion (DAP_ADVANCED.md § VE) ─────────────────────
    /// Mint the next `variablesReference` from the monotonic counter (never reused → VE3).
    fn mint(&mut self) -> i64 {
        self.next_ref += 1;
        VAR_REF_BASE + self.next_ref
    }

    /// Register a JSON node under a fresh handle so the client can expand it.
    fn mint_value(&mut self, value: Parsed) -> i64 {
        let handle = self.mint();
        self.var_values.insert(handle, value);
        handle
    }

    /// Build the `variables` list for a reference: the Locals scope (top-level frame locals,
    /// each given an expansion handle when it evaluates to a non-empty struct/vector), a
    /// registered expansion handle (the cached JSON node's immediate children), or empty for
    /// a stale / unknown reference or when not paused.
    fn build_variables(&mut self, driver: &mut DebugDriver, want: i64) -> Vec<Parsed> {
        if !self.paused {
            return vec![];
        }
        if want == self.locals_ref {
            // VE0 — top level: keep the flat-frame value; add a handle only when the local
            // evaluates to an expandable value.  One `eval` per local (a pure read).
            let locals = self.locals.clone();
            locals
                .iter()
                .map(|(name, value)| {
                    let handle = match self.eval_json(driver, name) {
                        Some(v) if expandable(&v) => self.mint_value(v),
                        _ => 0,
                    };
                    var_entry(name, value, handle)
                })
                .collect()
        } else if let Some(node) = self.var_values.get(&want).cloned() {
            // VE1/VE2 — one level of the cached JSON tree; an object/array child gets its
            // own handle (drilling deeper navigates the cached tree, no re-eval).
            self.expand_node(&node)
        } else {
            vec![] // VE3 — a stale or unknown handle
        }
    }

    /// The immediate children of a cached JSON node as DAP variables (object fields / array
    /// elements); each child that is itself object/array gets a fresh expansion handle.
    fn expand_node(&mut self, node: &Parsed) -> Vec<Parsed> {
        match node {
            Parsed::Object(entries) => entries
                .iter()
                .map(|(k, _, v)| var_entry(k, &render_value(v), self.child_handle(v)))
                .collect(),
            Parsed::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, v)| var_entry(&format!("[{i}]"), &render_value(v), self.child_handle(v)))
                .collect(),
            _ => vec![], // a scalar node has no children (never registered as expandable)
        }
    }

    /// An expansion handle for a child value when it is itself expandable, else `0` (leaf).
    fn child_handle(&mut self, v: &Parsed) -> i64 {
        if expandable(v) {
            self.mint_value(v.clone())
        } else {
            0
        }
    }

    /// Evaluate `expr` in the paused frame and return its value as JSON (`None` when it
    /// evaluates to null or the eval fails) — the RPC `eval` verb, a pure read.
    fn eval_json(&mut self, driver: &mut DebugDriver, expr: &str) -> Option<Parsed> {
        let rpc = json::to_json_string(&obj(vec![
            ("id", Parsed::Int(0)),
            ("req", Parsed::Str("eval".into())),
            ("expr", Parsed::Str(expr.to_string())),
        ]));
        let (msgs, _) = driver.drive(&rpc);
        let resp = rpc_ok(&msgs, 0).ok()?;
        match field(&resp, "value") {
            Some(Parsed::Null) | None => None,
            Some(v) => Some(v.clone()),
        }
    }

    fn next_seq(&mut self) -> i64 {
        self.seq += 1;
        self.seq
    }

    /// Send a DAP response for `request_seq`'s `command`.
    fn respond(
        &mut self,
        out: &io::Stdout,
        request_seq: i64,
        command: &str,
        success: bool,
        body: Option<Parsed>,
        message: Option<&str>,
    ) {
        let seq = self.next_seq();
        let mut entries = vec![
            ("seq", Parsed::Int(seq)),
            ("type", Parsed::Str("response".into())),
            ("request_seq", Parsed::Int(request_seq)),
            ("success", Parsed::Bool(success)),
            ("command", Parsed::Str(command.to_string())),
        ];
        if let Some(m) = message {
            entries.push(("message", Parsed::Str(m.to_string())));
        }
        if let Some(b) = body {
            entries.push(("body", b));
        }
        send(out, &obj(entries));
    }

    /// Send a spontaneous DAP event.
    fn event(&mut self, out: &io::Stdout, name: &str, body: Parsed) {
        let seq = self.next_seq();
        send(
            out,
            &obj(vec![
                ("seq", Parsed::Int(seq)),
                ("type", Parsed::Str("event".into())),
                ("event", Parsed::Str(name.to_string())),
                ("body", body),
            ]),
        );
    }
}

/// The v1 capability set (DAP.md § Envelope mechanics).  `supportsStepBack` is
/// deliberately absent (reverse execution is a follow-up), and hit-conditional
/// breakpoints are NOT advertised — the engine has no hit-count, so advertising it would
/// let a client set a condition the adapter silently ignores (a wrong picture).
fn capabilities() -> Parsed {
    obj(vec![
        ("supportsConfigurationDoneRequest", Parsed::Bool(true)),
        ("supportsConditionalBreakpoints", Parsed::Bool(true)),
        ("supportsEvaluateForHovers", Parsed::Bool(true)),
        ("supportsTerminateRequest", Parsed::Bool(true)),
        ("supportsSetVariable", Parsed::Bool(true)),
    ])
}

/// The RPC response line for `id`, or its error message.  `Ok` holds the parsed response
/// object (from which the caller reads `breakpoints` / `value` / `frame`).
fn rpc_ok(msgs: &[String], id: i64) -> Result<Parsed, String> {
    for line in msgs {
        let Ok(p) = json::parse(line) else { continue };
        if field_i64(&p, "id") == Some(id) {
            return if field_bool(&p, "ok") == Some(true) {
                Ok(p)
            } else {
                Err(field_str(&p, "error").unwrap_or_else(|| "request failed".into()))
            };
        }
    }
    Err("no response from the debug engine".into())
}

/// Read a frame's `locals` array (`[{name, value}]`) into `(name, value)` pairs.
fn read_locals(frame: &Parsed) -> Vec<(String, String)> {
    field(frame, "locals")
        .and_then(as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|l| {
                    Some((
                        field_str(l, "name")?,
                        field_str(l, "value").unwrap_or_default(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Render an RPC `eval` value (raw JSON) as a DAP display string: a text value shows
/// unquoted; anything else (number / array / struct) shows as its JSON text.
fn render_value(v: &Parsed) -> String {
    match v {
        Parsed::Str(s) => s.clone(),
        other => json::to_json_string(other),
    }
}

/// Whether a JSON value has children to drill into — a non-empty object or array.  A node
/// is a leaf (DAP `variablesReference: 0`) iff this is false (VE's leaf invariant).
fn expandable(v: &Parsed) -> bool {
    match v {
        Parsed::Object(o) => !o.is_empty(),
        Parsed::Array(a) => !a.is_empty(),
        _ => false,
    }
}

/// A DAP `Variable`: `name`, display `value`, empty `type` (the flat frame carries none),
/// and an expansion `variablesReference` (0 = leaf).
fn var_entry(name: &str, value: &str, var_ref: i64) -> Parsed {
    obj(vec![
        ("name", Parsed::Str(name.to_string())),
        ("value", Parsed::Str(value.to_string())),
        ("type", Parsed::Str(String::new())),
        ("variablesReference", Parsed::Int(var_ref)),
    ])
}

// ── json helpers over loft::json::Parsed (mirrors loft-lsp) ──────────────────────────
fn field<'a>(v: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match v {
        Parsed::Object(e) => e.iter().find(|(k, _, _)| k == key).map(|(_, _, val)| val),
        _ => None,
    }
}
fn field_str(v: &Parsed, key: &str) -> Option<String> {
    match field(v, key) {
        Some(Parsed::Str(s)) => Some(s.clone()),
        _ => None,
    }
}
fn field_i64(v: &Parsed, key: &str) -> Option<i64> {
    field(v, key).and_then(Parsed::as_i64)
}
fn field_bool(v: &Parsed, key: &str) -> Option<bool> {
    match field(v, key) {
        Some(Parsed::Bool(b)) => Some(*b),
        _ => None,
    }
}
fn as_array(v: &Parsed) -> Option<&Vec<Parsed>> {
    match v {
        Parsed::Array(a) => Some(a),
        _ => None,
    }
}

/// Build a JSON object from `(key, value)` pairs (the byte-offset slot is 0 — used only by
/// the schema walker's diagnostics, irrelevant for serialization).
fn obj(entries: Vec<(&str, Parsed)>) -> Parsed {
    Parsed::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), 0, v))
            .collect(),
    )
}

/// The file name of a path (the `stackTrace` source label).
fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

// ── framing ──────────────────────────────────────────────────────────────────────────
// One `Content-Length: N\r\n <headers> \r\n<N bytes>` message — the DAP wire format
// (identical to LSP framing).  Duplicated from loft-lsp because it is binary-local (not
// part of the loft rlib); both are ~20 lines over loft's own json mod.
fn read_message(stdin: &mut impl BufRead) -> Option<String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if stdin.read_line(&mut line).ok()? == 0 {
            return None; // EOF
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break; // blank line ends the header block
        }
        if let Some(v) = header.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
    }
    let mut buf = vec![0u8; content_length?];
    stdin.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn send(stdout: &io::Stdout, msg: &Parsed) {
    let body = json::to_json_string(msg);
    let mut out = stdout.lock();
    // Content-Length is the BYTE length of the UTF-8 body.
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

/// Resolve the stdlib `default/` directory relative to the binary — dev tree, installed
/// prefix, or release layout (mirrors loft-lsp).
fn resolve_stdlib_dir() -> String {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let candidates = [
        exe_dir.join("../../default"), // dev: target/{release,debug}/loft-dap
        exe_dir.join("../share/loft/default"), // installed: <prefix>/bin -> <prefix>/share/loft
        exe_dir.join("../default"),    // default beside the binary dir
    ];
    for c in candidates {
        if c.is_dir() {
            return c.to_string_lossy().into_owned();
        }
    }
    "default".to_string()
}

/// The `--lib <dir>` import paths from the adapter's argv (de-duplicated), so the debugged
/// file can `use` libraries — the same collection `loft debug --rpc` does.
fn collect_lib_dirs(args: &[String]) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--lib" {
            let raw = &args[i + 1];
            let abs = std::fs::canonicalize(raw)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| raw.clone());
            if !dirs.contains(&abs) {
                dirs.push(abs);
            }
            i += 1;
        }
        i += 1;
    }
    dirs
}
