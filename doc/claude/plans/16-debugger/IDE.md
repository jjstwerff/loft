<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 16.E — M5e: the server-backed IDE (the lavition editor)

> **Identity:** a design sub-doc of `@PLN16` (debugger). Slug `debug-ide`.
> **Status:** design — the browser surface that grows the debugger into a usable IDE.
> Builds on the shipped `--rpc` server ([PROTOCOL.md](PROTOCOL.md)) and the plan-35
> viewer shell. No browser-UI code yet; the engine side is largely already shipped.

## Why this exists — and why there is *no interpreter in the browser*

This is the **lavition editor**: the surface you author, run, debug, and *iterate* a
game from. Its defining constraint is also its defining strength —

> **The browser holds no interpreter and no language logic. The real loft engine runs
> locally; the actual game renders in a native OpenGL window. The browser is a thin view
> that edits source, shows debug state, and sends intents.**

That is not a compromise versus an "everything in WASM" playground — it is the *only*
model that can build real games:

| | server-backed IDE (this plan, M5e) | serverless WASM playground ([`07-web-ide`](../../lib_plans/future/07-web-ide/README.md)) |
|---|---|---|
| Runs the game | **native OpenGL window, full GPU, native speed** | sandboxed canvas, no native GPU, no threads-of-note |
| Filesystem | the real project on disk (assets, `lib/`, `tests/`) | IndexedDB only; no real FS |
| Test suite | runs the **whole project suite** natively | can run a single program, not a suite |
| Debugger | the @PLN16 engine, breakpoints in the *running game* | none |
| Live data | one shared store, edits hot-swap into the running game | fresh interpreter per run |
| Install | a local `loft` binary (or `make view`-style port-forward) | open a URL, zero install |

The two are **different products**. `07-web-ide` is the zero-install demo/teaching
surface; this is the working tool you make a game with. They share the WASM build and the
editor grammar, nothing else. The serverless one cannot host OpenGL, the filesystem, the
debugger, or hot-swap — so for the lavition use case the local engine is mandatory, and
once the engine is local the browser has *nothing left to interpret*. "No interpreter in
the browser" falls out of "the game runs natively," it is not a limitation we accepted.

This is [`live-prototyping`](../../GOALS.md) made literal: **a Rust main loop + loft over
one shared store, editing a single function and seeing it in the running game** — the IDE
is the seat you drive that loop from. See [LAVITION.md](../../LAVITION.md) (the engine) and
[`10-game-client`](../../lib_plans/future/10-game-client/README.md) / [`02-graphics`](../../lib_plans/future/02-graphics/README.md)
(the OpenGL runtime the game window uses).

## The invariant (extends the protocol invariant to the whole workspace)

> **Every IDE action — open, edit, save, compile, run, test, breakpoint, step, inspect,
> launch the game, hot-swap a function — is exactly one protocol message that is a thin
> call into one existing engine / `ReplSession` / workspace-host method. The browser
> renders engine state and emits intents; it never computes language results, and it
> never renders the game (the game renders itself, natively).**

This is [PROTOCOL.md](PROTOCOL.md)'s "one message ⇄ one engine method" generalised from
*debug* to *workspace*. The re-assertion sites are the message table below: each row must
name a real method, and no IDE capability may live only in the browser (a leak that drifts)
or invent semantics the engine doesn't own. The load-bearing falsification — *is there an
IDE feature that cannot be one-message-one-method?* — has exactly one honest seam: the game
window is **not** browser-rendered. We make that explicit rather than hide it: the browser
surface and the game surface are two views of one local engine, and the IDE renders the
*dev* surface (code, diagnostics, variables, console), never the game's pixels.

## Architecture — three surfaces, one local engine

```
  ┌─────────── Browser (thin IDE UI) ───────────┐        the dev surface
  │  file tree · editor · gutter breakpoints     │        (HTML/JS over the
  │  toolbar [Run ▶ Test Suite Debug Game]        │        plan-35 viewer shell)
  │  ┌── Compiler console ──┐ ┌── Program console ┐│
  │  │ diagnostics          │ │ stdout / traces   ││
  │  └──────────────────────┘ └───────────────────┘│
  │  Debug: variables · watch · call stack         │
  └───────────────▲────────────────────────────────┘
                  │  one JSON message per WebSocket frame  (localhost / SSH-forward)
                  ▼
  ┌─────────── Local server: `loft debug --serve <port>` ──────────────┐
  │  a thin protocol driver over the shipped engine (src/rpc.rs shape)  │
  │  ReplSession ──► parser/diagnostics ─ compile ─ run ─ test ─ debug  │
  │  one shared Store ◄──────────────── hot-swap a function (N9/C71)    │
  └───────────────▲──────────────────────────────────┬─────────────────┘
                  │ same process / shared store        │ native window
                  ▼                                    ▼
       the project on disk                  ┌─── OpenGL game window ───┐
       (src/ lib/ tests/ assets/)           │  the actual game, native │
                                            │  GPU, native frame loop   │
                                            └───────────────────────────┘
```

The data never leaves the machine — same shape as `make view` over an SSH port-forward.
The game window is a separate native surface the engine owns; the browser can *control* it
(run / pause / step / reload) but does not display it.

## The protocol extension — the workspace layer

The debug messages ([PROTOCOL.md § Requests/Events](PROTOCOL.md#requests-client--server))
are unchanged and carry the whole debugger. M5e adds a **workspace** layer on the same
envelope, same invariant — each row a thin call into an existing or one-line-new method.

### Requests (client → server)

| `req` | args | method | response |
|---|---|---|---|
| `listFiles` | `path?` | viewer's tree walk (`/tree`) | `{ok, entries:[{name, dir}]}` |
| `readFile` | `path` | host read (viewer's `/raw`) | `{ok, content}` |
| `writeFile` | `path`, `content` | host write **(sandboxed to the workspace root)** | `{ok}` |
| `compile` | `file` | `ReplSession::load_program` (parse + scope-check, no run) | `{ok}`, then a `diagnostics` event |
| `run` | `file`, `entry?` | `load_program` + run (== `launch`) | `{ok}`, then `output` / `stopped` / `terminated` |
| `runTests` | `file?` | `test_runner::run_tests` on a file (or `tests/`) | `{ok}`, then `testResult*` + `testSummary` |
| `runSuite` | `dir?` (default project) | `test_runner::run_tests` over the project | `{ok}`, then `testResult*` + `testSummary` |
| `launchGame` | `file`, `entry?` | run with the OpenGL window host (the `make game` path) | `{ok}`, then `gameState` |
| `reload` | `function?` | hot-swap one fn into the running game (per-fn interpret, N9/C71) | `{ok}`, `gameState` |
| `stop` | — | terminate the running program / game | `{ok}`, `terminated` |

`launch` / `setBreakpoints` / `setWatch` / `continue` / `step*` / `eval` / `setValue` /
`undo` / `redo` / `stackTrace` / `disconnect` are the **existing** debug set — they apply
to the same `file`, so debugging *is* "run with breakpoints," including breakpoints in the
running game's frame logic.

### Events (server → client)

| `event` | payload | emitted when |
|---|---|---|
| `diagnostics` | `file`, `items:[{line, col, level, message}]` | a `compile` / `run` produces parser/scope diagnostics — **the compiler console** |
| `output` | `category` (`stdout`/`trace`), `text` | the program / game prints, or a tracepoint fires — **the program console** (exists) |
| `testResult` | `name`, `status` (`pass`/`fail`), `message?`, `file`, `line?` | one test finishes (streamed, so the panel fills live) |
| `testSummary` | `passed`, `failed`, `durationMs` | a `runTests` / `runSuite` completes |
| `gameState` | `phase` (`running`/`paused`/`stopped`), `fps?` | the game window changes state |
| `stopped` · `terminated` | (see PROTOCOL.md) | a breakpoint fires / the run ends |

Two consoles fall straight out of two event streams: **compiler** = `diagnostics`,
**program** = `output`. No new rendering logic — the browser appends each event to its pane.

The one genuinely new *capability* is `writeFile` (the editor's save) — everything else is
a serialisation of a method that already exists. `writeFile` is the security seam, handled
under § Safety.

## The browser shell (reuse the plan-35 viewer)

The viewer (`tools/viewer/src/main.loft`: `server::listen` + routes `/`, `/tree`, `/file`,
`/raw`, `/static`) already serves a file tree and code-with-line-numbers. M5e adds, on the
same shell:

```
┌── tree ──┬──────────── editor (CodeMirror 6) ───────────┬─ Debug ─┐
│ src/     │  1  fn update(e: Entity) {                    │ vars    │
│  main    │ ●2    e.x = e.x + e.vx        ← gutter break  │  e.x 12 │
│  enemy   │  3    if e.x > 800 { … }                      │ watch   │
│ lib/     │  4  }                                          │ stack   │
│ tests/   │                                                │ update  │
├──────────┴───── toolbar: [Run ▶][Test][Suite][Debug][Game ▶][Reload]
│ ┌─ Compiler ──────────────┐ ┌─ Program ──────────────────┐ ┌─ Tests ─┐
│ │ enemy.loft:3:7 warning…  │ │ frame 0 spawned 4 enemies  │ │ ✓ 41    │
│ └──────────────────────────┘ └────────────────────────────┘ │ ✗ 2 →   │
```

Panels: **file tree** (`/tree`), **editor** (the `/file` pane made editable), **gutter
breakpoints** (click → `setBreakpoints`), **toolbar** (each button → one request),
**dual console** (compiler `diagnostics` + program `output`), **debug panel**
(variables / watch / call stack from `stopped` events — zero extra round-trips, the frame
ships whole), **tests panel** (`testResult` stream, click a failure → jump to the line, or
drop into the debugger on it).

The editor starts minimal (a line-numbered editable pane) and graduates to CodeMirror 6
with the loft grammar — the editor polish, hover, and squiggle work is already specced in
[`14-viewer-lsp-bridge/05-browser-editor`](../../lib_plans/future/14-viewer-lsp-bridge/05-browser-editor.md);
M5e consumes it rather than re-specifying it.

## Build slices (incremental — each independently usable)

Each slice: one transport/UI step, the protocol rows it lights up, the engine method
behind them (almost always already shipped), and a test in the `tests/rpc.rs` shape
(drive the messages over a pipe / WS, assert the JSON) plus a headless-chromium smoke
(the dev box ships chromium + both wasm targets).

1. **Foundation — `--serve` + shell + Run + program console — LANDED (2026-06-10).**
   `loft debug <file> --serve [--port <n>]` (`src/serve.rs`): an HTTP + WebSocket server on
   `127.0.0.1`. `GET /` returns a minimal shell (the file's source in a read-only pane, a
   **Run** button, a **Program** console); the shell opens a WebSocket and drives the *same*
   `handle()` driver as `--rpc` — one JSON request per text frame, `run` → `output` events
   stream into the console, then `terminated`. The transport is the only new code: a small
   inline SHA-1 + WS framing (test-vector verified, no new crate); the engine, protocol, and
   message set are unchanged. Program `print` rides the shared capture sink (`output`
   events), never the socket. Tests: `tests/serve.rs` drives the full path over a real
   socket (HTTP shell, WS handshake, launch → run → output → terminated) + unit tests for
   SHA-1 / the RFC 6455 accept key / frame round-trip. *Smallest visible IDE; proves
   transport + one console.* (Slice 1 shows the source inline rather than via a `/file`
   endpoint — the editor/`readFile` path is slice 3.) The live game loop (slice 6) is why
   this is a WebSocket, not request/response — it needs bidirectional server-push.
2. **Compiler console — `compile` + `diagnostics`.** A `compile` message returns
   diagnostics; the Compiler console lists them; inline squiggles + gutter markers on the
   editor (the 14-bridge R2 drawing). *The dual console is complete.* Engine:
   `load_program` already produces diagnostics — expose them as a structured event.
3. **Editor — editable pane + `writeFile` + save/reload.** Make the code pane editable;
   `writeFile` saves to disk (sandboxed); a re-`run`/`compile` picks it up. *You can now
   edit and re-run without leaving the browser.* The one new write capability + its sandbox.
4. **Debugger UI — gutter breakpoints + step controls + panels.** Click the gutter →
   `setBreakpoints`; **Debug** launches with breakpoints; step buttons → `stepIn/Over/Out`;
   the variables / watch / call-stack panels render the `stopped` frame. *The debugger is
   in the browser.* Engine: entirely shipped (A–F + M1–M3 + M5a) — this slice is pure UI
   over the existing protocol.
5. **Test + suite runners — `runTests` / `runSuite` + results panel.** Run a file's tests
   or the whole project suite; `testResult` streams fill the panel live; click a failure →
   jump to its line, or set a breakpoint and **Debug** it. *Run/Test/Suite buttons all
   work.* Engine: `test_runner::run_tests` exists (`loft --tests` / `loft test`) — wrap it
   to emit per-test events.
6. **The game loop (lavition) — `launchGame` + `reload` (hot-swap) + debug the running
   game.** **Game ▶** launches the program in a **native OpenGL window** (the `make game`
   host); editing a function + **Reload** hot-swaps just that function into the running
   game over the shared store (per-fn interpret on the compiled baseline, N9/C71); a
   breakpoint in game logic pauses the game and drops the same variables panel in the
   browser. *The full live-prototyping loop: edit → see it in the running game → breakpoint
   it.* Engine: the graphics runtime ([`02-graphics`](../../lib_plans/future/02-graphics/README.md)) +
   shared-store per-fn dispatch (N9/C71) — this slice is where the IDE *becomes the engine
   editor*.

Slices 1–5 make a usable IDE for any loft program (CLI tools, servers, libraries); slice 6
is the lavition payoff. They land in order; 1 is the prerequisite for all.

## Dogfood driver — the `story` / crawler roguelike

The first real consumer is the **`story` roguelike** (a sibling loft project, the "crawler"
— a pure-loft Angband-like: hex sim, procedural generation, monster AI, combat, loot, XP),
agent-built, shipping native via `make play` (OpenGL) and WebGL via `make game` →
`story.html`, at a shippable vertical slice. It is the right driver for three reasons:

- **Its bug class *is* the debugger's reason to exist.** The crawler's dominant pain is
  **silent wrong-results in store/heap-lifetime code** — struct-vector index + `??`, struct
  copies dropping a `text` field, cross-module `&Struct` mutation lost, struct-field vector
  growth desyncing (its `LOFT_ISSUES.md` C1–C5, C18; the family of loft #248). For that
  class, `print` is *worse than useless*: one entry is a literal **Heisenbug** (adding a
  `println` flips the result), and the project's own `CLAUDE.md` warns its agent "don't
  trust print-debugging." The debugger's answer is exactly what print can't do —
  **non-intrusive observation** (the D0 variables panel reads the live frame and injects no
  op), **watchpoints** (pause at the desyncing write — tailor-made for the C18 parallel-array
  desync), and **conditional breakpoints** (catch "the 3rd call onward" / "1-in-N element").
- **It is the slice-6 game, already.** Native OpenGL + WebGL, with a deferred v1.1 UI backlog
  (the Character Page, inventory screen, quick-slot bar) authored to live in `view.loft` /
  `story.loft` so *both* renderers get it — precisely the edit-a-function-see-it-in-the-
  running-game loop slices 3 + 6 deliver. Its deferred **live ticking spawner** is C18
  (struct-field vector growth) — watchpoint-shaped work waiting for this tool.
- **It is also the debugger's honest stress test.** The debugger **shares the store engine**
  with the very bugs the crawler hunts, so for the store-lifetime family it is an
  *observation* aid, not immunity: the bare-heap-`eval` fault behind [README § **D2**](README.md)
  is itself a member of that family, and the crawler's data is *all* `vector<struct>` —
  exactly where it bites. That is why D2 (live-frame eval) is elevated, not polish: trustworthy
  heap eval is load-bearing for this consumer. The variables panel (live-frame render) is the
  robust surface today; bare-heap `eval` is the fragile one.

`LOFT_ISSUES.md`'s C1–C20 doubles as a **ready-made validation matrix** — *"can the debugger
observe each better than `print` could?"* Running it both proves the tool and surfaces the
next substrate gap (the bet: it re-confirms D2 as the priority). The immediate dogfood move,
before any browser code: the next time the agent hits a silent wrong-result in the crawler's
v1.1 work, drive `--rpc` with a **watchpoint** instead of adding a `println`. *(The debugger
is interpreter-mode, so it covers the crawler's interpreter-bug class but not its
codegen-divergence (C9/C13), toolchain (C15), or GL (C16/C17) classes — those need the
native / graphics tooling.)*

## Reuse + one-home map

| Piece | Reuses | New here |
|---|---|---|
| Transport | the `--rpc` `handle()` driver (`src/rpc.rs`); loft's socket stack (`server` lib / multiplayer) | WS framing of the same messages |
| Shell | plan-35 viewer (`/tree`, `/file`, `/static`) | toolbar, panels, gutter |
| Editor polish | [`14-bridge/05`](../../lib_plans/future/14-viewer-lsp-bridge/05-browser-editor.md) (squiggles, hover, CodeMirror) | wiring to `diagnostics` events |
| Debug | the entire @PLN16 A–F engine + protocol | UI only |
| Compile / test | `ReplSession::load_program`, `test_runner::run_tests` | structured `diagnostics` / `testResult` events |
| Game + hot-swap | [`02-graphics`](../../lib_plans/future/02-graphics/README.md) OpenGL host, N9/C71 per-fn dispatch, the one shared store | `launchGame` / `reload` messages |
| Language intelligence (go-to-def, completion) | [`09-lsp`](../../lib_plans/future/09-lsp/README.md) `loft-lsp` | later; not v1 |

## Safety

- **`writeFile` is sandboxed to the workspace root** — the server canonicalises the path
  and refuses any write that escapes the root (no `..`, no absolute paths outside). This is
  the one new write surface; everything else is read or run.
- **Bounded runs + panic isolation** (PROTOCOL.md § Safety) — a `run` / `runSuite` /
  `launchGame` honours `--max-steps`; a fault is caught and reported as `terminated`, never
  killing the server.
- **Local by default.** The server binds `localhost`; remote use is an explicit SSH
  port-forward (the `make view` pattern), so the data and the game stay on the machine.
- **The game window is native, not piped to the browser** — no frame-streaming surface to
  secure, no GPU in the browser; the IDE only sends control intents (`launchGame` / `stop`
  / `reload`) and reads `gameState`.

## Testing strategy

- **Protocol-level** (the spine): extend the `tests/rpc.rs` shape — drive each new message
  over a pipe / in-memory WS and assert the JSON: `compile → diagnostics`,
  `writeFile → readFile` roundtrip, `runTests → testResult* + testSummary`.
- **Browser smoke**: a headless-chromium script connects to a `--serve` instance, clicks
  Run, asserts the Program console fills — the `check_html_bundle.mjs` real-render shape.
- **Game loop**: the existing GL test harness (`make test-gl-headless` / `-smoke` /
  `-golden`) gates `launchGame` + `reload`; a hot-swap test asserts a function edit changes
  the next rendered frame.

## What this is NOT (boundaries)

- **Not [`07-web-ide`](../../lib_plans/future/07-web-ide/README.md)** — that is the
  serverless WASM playground (a different product; see the table up top). M5e never runs
  the interpreter in the browser.
- **Not a scene / asset editor yet** — visual scene editing is
  [`13-scriptable-scenes`](../../lib_plans/future/13-scriptable-scenes/README.md); M5e edits
  *code* and drives the game, it does not yet place entities visually.
- **Not language intelligence** — completion, go-to-def, refactor are
  [`09-lsp`](../../lib_plans/future/09-lsp/README.md); M5e ships diagnostics (free from the
  compiler) and leaves the rest to the LSP surface.

## Cross-references

- [PROTOCOL.md](PROTOCOL.md) — the wire contract M5e extends; the debug message set is
  unchanged, the workspace layer above is additive.
- [README.md](README.md) — the @PLN16 sub-arc table; M5e is the browser surface, M5d the
  agent surface, both over the one protocol.
- [LAVITION.md](../../LAVITION.md) / [GOALS.md](../../GOALS.md) — the engine and the
  live-prototyping purpose this IDE is the seat for.
- [`02-graphics`](../../lib_plans/future/02-graphics/README.md),
  [`10-game-client`](../../lib_plans/future/10-game-client/README.md) — the native OpenGL
  game runtime the **Game ▶** surface launches.
- [`14-viewer-lsp-bridge`](../../lib_plans/future/14-viewer-lsp-bridge/README.md) — the
  viewer-shell + local-sidecar pattern and the browser-editor polish M5e consumes.
