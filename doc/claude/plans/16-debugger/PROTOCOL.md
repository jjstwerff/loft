<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 16.P — Debug wire protocol (the one contract every surface speaks)

> **Identity:** a design sub-doc of `@PLN16` (debugger). Slug `debug-protocol`.
> **Status:** design — the contract for M5d (agent / scripted surface) and M5b
> (browser). Written *before* any server code so both clients are designed against a
> fixed interface. No code yet.

## Why this exists

The debug *engine* (sub-arcs A–F) is a host-agnostic Rust API on `State` /
`ReplSession`. The terminal surface (G1) drives it by hand at a `(dbg)` prompt; the
browser (M5b) and an agent (M5d) cannot — a browser needs a socket, an agent needs
structured request/response it can script. Rather than give each its own ad-hoc glue,
**both consume one wire protocol**: a small JSON request/response + event stream that
serialises the engine.

The payoff is convergence: **one protocol, many transports, many clients.** The
browser speaks it over a WebSocket; an agent (or a test / CI gate) speaks the *same*
messages over stdio or TCP; a future editor speaks a DAP translation of it. The local
server holds the live `State` and runs the debugger on the user's machine — the data
never leaves (exactly the `make view` shape: a local server, a thin remote view).

## The invariant

> **The protocol is the *complete and sole* serialisation of the debug engine: every
> `(dbg)`-prompt capability is exactly one protocol message, no capability is
> UI-only or agent-only, and the protocol layer adds *no* debug semantics — each
> request is a thin call into an existing `ReplSession` / `State` method, each event a
> thin render of engine state.**

This is the chokepoint (Goal E — one home). It makes the surfaces *interchangeable*:
anything a human can do at the prompt, an agent can script and a browser can offer,
because all three go through the one message set. The load-bearing test (probe before
building each message): *does this message correspond to an existing engine method, and
is there a `(dbg)` capability with no message (a leak) or a message with no engine
method (invented semantics)?* Either is a defect in the contract, not the code.

## The model

- **One session per connection.** A connection owns one debug session — a
  `ReplSession` holding the paused `State` (M5a's file-run engine). v1 is single
  session; multiplexing is a transport concern added later, not a protocol change.
- **Request → response, plus asynchronous events.** The client sends **requests**
  (each correlated by `id`); the server replies with one **response** per request, and
  emits **events** (`stopped`, `output`, `terminated`) with no `id` whenever execution
  state changes. This is the DAP / LSP shape, deliberately: a debugger is inherently
  asynchronous (you `continue`, then *later* a breakpoint fires).
- **Transport-agnostic.** The message *schema* is the contract; framing is the
  transport's job. **NDJSON** (one JSON object per line) over stdio / TCP — trivially
  scriptable by an agent (`printf '{…}\n' | loft debug --rpc`); **one JSON per text
  frame** over a WebSocket for the browser. Same objects either way.
- **The run's output is captured, never printed.** The debugged program's `print`
  output and tracepoint logs are streamed as `output` events — they do **not** go to
  the server's own stdout (which, for the `--rpc` transport, carries the protocol).

## The envelope

```jsonc
// request  (client → server)
{ "id": 7, "req": "eval", "expr": "pt.x * pt.y" }
// response (server → client) — echoes the id
{ "id": 7, "ok": true, "value": "12", "type": "integer" }
{ "id": 7, "ok": false, "error": "no such local" }
// event    (server → client) — no id, fire-and-forget
{ "event": "stopped", "reason": "breakpoint", "frame": { … } }
```

`id` is any client-chosen token echoed back; `req` / `event` name the message; the rest
is the payload. Unknown `req` → `{ ok:false, error:"unknown request" }` (forward
compatibility: a client may probe for features).

## Requests (client → server)

Each maps 1:1 to an engine method — the right column is the invariant made concrete.

| `req` | args | engine method | response |
|---|---|---|---|
| `launch` | `file`, `entry?` (default `main`), `args?`, `stopOnEntry?` | `load_program` + run | `{ok}`, then a `stopped` / `terminated` event |
| `setBreakpoints` | `file`, `breakpoints:[{line, condition?, log?, stop?}]` | `set_breakpoint_file_line` (+ condition = **E**; `log` + `stop:false` = **tracepoint**) | `{ok, breakpoints:[{line, verified}]}` |
| `setWatch` | `expr` | `add_watchpoint` | `{ok, id}` |
| `clearWatch` | `id?` (all if absent) | `clear_watchpoints` | `{ok}` |
| `continue` | — | `debug_step(Continue)` | `{ok}`, then `stopped` / `terminated` |
| `stepIn` · `stepOver` · `stepOut` | — | `debug_step(Into/Over/Out)` | `{ok}`, then `stopped` / `terminated` |
| `eval` | `expr` | `debug_eval` | `{ok, value, type}` |
| `setValue` | `target` (`n` / `pt.x` / `v[0]`), `value` (RHS expr) | `debug_set` | `{ok, frame}` |
| `undo` · `redo` | — | `debug_undo` / `debug_redo` | `{ok, frame}` |
| `stackTrace` | — | `break_stack` (B3) | `{ok, frames:[{function, file, line, locals}]}` |
| `disconnect` | — | drop the session | `{ok}` |

**`setBreakpoints` replaces a file's set** (DAP semantics — idempotent, no per-id
churn). The richer facets (`condition`, `log`, `stop`) are M5d-phase-1 engine work the
prompt also gains (`:break … if …`, `:trace …`); the protocol carries them from day one
so the contract is stable when they land.

## Events (server → client)

| `event` | payload | emitted when |
|---|---|---|
| `stopped` | `reason` (`breakpoint`/`step`/`watch`/`entry`), `frame:{function,file,line,locals:[{name,value,type}]}`, `watch?:{label,old,new}` | execution pauses |
| `output` | `category` (`stdout`/`trace`), `text` | the program prints, or a tracepoint fires |
| `terminated` | `exitReason?` | the run finishes (no further pause) |

**`stopped` carries the whole frame summary** (function + line + locals as
own-format–rendered `value`s, the same the `(dbg)` prompt shows) so a client renders a
variables panel with **zero extra round-trips** — the agent reads one event, the
browser paints one frame. Drill-down into a big value is a follow-up `eval`. A
**tracepoint** never emits `stopped`: it emits `output{category:"trace"}` and the run
continues — that is the agent's structured trace (and the browser's logpoint).

## Transports

- **`loft debug <file> --rpc`** — NDJSON over stdin/stdout. The agent / CI surface: one
  process, scriptable, no network. (Protocol on stdout; program output rides `output`
  events, so the two never mix.)
- **`loft debug <file> --serve <port>`** — a local WebSocket the browser connects to
  (data stays local, like `make view` over an SSH port-forward). The loft HTTP `server`
  lib is request/response today; the live channel reuses loft's existing real-time
  socket stack (the multiplayer servers) or a thin server-side WebSocket.
- **DAP (future).** A `loft-dap` adapter translates this protocol to the Debug Adapter
  Protocol so VS Code / any DAP editor can attach. Not v1 — but the request/response +
  event shape is chosen to make that a *translation*, never a redesign.

## Safety

- **Bounded runs.** `launch` / `continue` / `step*` honour a `--max-steps` budget; a
  run that exceeds it emits `terminated{exitReason:"step budget"}` rather than hanging a
  non-interactive client.
- **Panic isolation.** A runtime fault in the debugged program is caught (the existing
  `catch_unwind` around the paused run) and reported as `terminated{exitReason:"…"}` —
  it abandons the session, never the server.
- **No partial state on error.** A failed request (`ok:false`) leaves the session
  exactly as it was — the same guarantee the prompt gives.

## Reuse + one-home

A server is a **thin protocol driver** over the shipped engine, replacing the terminal
`run_loop`: parse a request → call the named `ReplSession` / `State` method → serialise
the result / the next event. **No new engine, no new debug semantics** — the same
breakpoints (A), conditions (E), stepping (F), frame read/eval (D0/D1), edits (M1/M1a),
undo (M2), watches (M3), and file-run (M5a) the prompt uses. If a message ever needs
logic that isn't already an engine method, that logic belongs in the engine (shared by
all surfaces), not in the server.

## Phasing

1. **Rich breakpoints (engine).** `condition` (E in-loop) + `log`/`stop` (tracepoint),
   wired into the registry (A) + suspend hook (C), and surfaced at the `(dbg)` prompt.
   The protocol already names them, so this fills in capability behind a fixed contract.
2. **The `--rpc` server.** The NDJSON stdio driver over the engine — the smallest
   end-to-end slice, and the one that makes the **agent** surface real. Validate the
   whole message set here (it needs no browser).
3. **The `--serve` WebSocket + browser client.** The same driver over a WebSocket; the
   browser shell reuses the viewer (tree + code-with-line-numbers) and adds the gutter,
   variables / watch panels, and REPL console — incremental → IDE.

Step 2 is the contract's proving ground: if the agent can drive a full debug session
over `--rpc`, the browser is "the same messages with a UI."

## See also

- [README.md](README.md) — the @PLN16 sub-arc table; **M5d** (agent / scripted surface)
  is the consumer this protocol serves, **M5b** is the browser.
- [REPL.md](../../REPL.md) — the `(dbg)` prompt whose capabilities this protocol mirrors
  message-for-message.
- [lib_plans/future/14-viewer-lsp-bridge/](../../lib_plans/future/14-viewer-lsp-bridge/README.md)
  — the "viewer + local sidecar" pattern this reuses; the browser client is the same
  shape with the debugger as the sidecar.
- [lib_plans/future/09-lsp/](../../lib_plans/future/09-lsp/README.md) — `loft-dap` (the
  future DAP translation) is scoped alongside the loft-lsp server there.
