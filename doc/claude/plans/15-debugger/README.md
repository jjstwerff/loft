<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 15 — Debugger (interpreter-mode; host-agnostic engine, browser its natural home)

> **Identity:** `@PLN15` — [`loft-lang/plans`](https://github.com/loft-lang/plans/issues)
> issue (pending creation). Slug `debugger`.

## Status

Open — design, no implementation. This is the **purpose the REPL work serves**: the
REPL is not standalone dev-tooling, it is the *interactive surface of a breakpoint
debugger*. The plan composes three things loft already has or is building rather
than building a debugger from scratch:

- the **interpreter**, which can already step op-by-op and *suspend a frame* (the
  coroutine `yield` machinery snapshots the live stack frame);
- the **per-function interpret-over-a-compiled-baseline dispatch** (N9/C71
  shared-store dispatch + Goal-D backend parity) — the mechanism that flips one
  routine to the interpreter while the rest stays compiled;
- the **store-resident session environment** (@PLN14) — which, seeded from a paused
  frame instead of from typed bindings, *is* the breakpoint's variable view.

A breakpoint is then: *suspend the interpreter at a source line, expose the paused
frame as a REPL environment.* Everything new is the glue around that.

**Host-agnostic engine, multiple surfaces.** The debug *engine* (sub-arcs A–F) has
no UI and no browser dependency — it is a loft API: set/clear breakpoints, run,
pause, read/mutate the frame, evaluate against it, step, resume. It runs anywhere
loft runs:

- **terminal / CLI** — the **shipped REPL is the headless surface**: hit a
  breakpoint, drop into the terminal REPL at that frame. Nearly free — it is the
  REPL pointed at a paused frame instead of stdin.
- **browser** — its *natural home*: a DAP adapter + web UI (gutter breakpoints,
  variables panel, call stack, REPL console) make it rich and visual.
- **embedded** — a game's own console (the lavition "live interface" node) drives
  the same API in-process.

The browser is where it gets the best ergonomics; it is **not** where it lives. The
engine ships and is useful from a terminal before any browser UI exists.

## Goal

Set a breakpoint in a routine; loft flips that routine **and its live call-chain**
to the interpreter, pauses when the line is reached, and drops into a REPL whose
environment is the **paused frame's locals** — inspect and change live state,
evaluate against the frame, optionally break only when a condition/assertion fires,
then step or resume — driven by a **host-agnostic debug API** that a terminal, the
browser (its natural home), or an embedded console all consume, with the rest of the
program still running compiled.

## Effort + design

- **Effort:** VH — multi-phase execution-control + API + (per-surface) protocol/UI;
  spans the interpreter, the dispatch boundary, @PLN14's env, and a DAP surface.
- **Design:** — needs design (this README is the first cut); each sub-arc earns its
  own design doc as it unpauses.
- **Last touched:** 2026-06-08

## Why this isn't a bolt-on subsystem

The naive debugger is a second execution engine with its own frame model — the
brittle path. loft's is the *composition* above: the invariant is **one frame
model, one environment model, shared by the REPL session and the breakpoint
frame.** A paused frame's locals and a REPL session's bindings are the same
store-resident records (position-independent `DbRef`s); a conditional breakpoint is
just `eval` against that env. Hold that invariant and the debugger is a suspend
hook + a registry + a seed bridge; lose it and it's a parallel interpreter.

## Composition matrix — Stage A

The load-bearing claim: *a breakpoint pauses with the correct frame, and evaluating
/ mutating in the REPL-at-frame is faithful to the live program.* Probe before
building, on `--interpret`, **driving the engine API directly (no UI)**:

- **breakpoint site** — fn entry · mid-fn line · inside a loop body · inside a
  branch not taken (never fires) · last line before return.
- **frame contents** — locals of every type (scalars, text, struct, vector, enum,
  nested) visible + correctly typed in the REPL-at-frame; an unassigned-yet local
  (before its first write) reported as such, not garbage.
- **call-chain** — break N frames deep: the stack above is introspectable (each
  caller's locals + args), step-out lands in the caller's frame.
- **interpret scope** — a breakpoint reachable only through compiled callers still
  fires (the chain switched to interpret); a function with no path to any
  breakpoint stays compiled (no blanket slowdown).
- **condition / test** — a condition referencing frame locals breaks only when true;
  an assertion-style breakpoint fires on violation; a condition with a side effect
  runs once per hit, not per step.
- **mutate + resume** — change a local in the REPL-at-frame, resume, observe the
  program continue with the new value.
- **backend parity** — the *value* observed at a break equals what `--native` would
  compute for the same point (Goal-D parity is what makes interpret-on-break
  invisible).

Done when every cell is green from the **headless API** (so every surface inherits
it) and the probes graduate to a debugger regression suite.

## Sub-arcs

The host-agnostic engine is A–F; the surfaces are G.

| Item | Status |
|---|---|
| **A** — breakpoint registry + source-line → bytecode-offset map (reuse the per-op source positions the bytecode already carries) | Open |
| **B** — interpret-scope analysis: from a breakpoint set, compute the functions whose call-chain must interpret (static call-graph reachability; refine to on-demand frame-entry switching) | Open |
| **C** — suspend hook: the bytecode loop pauses at a breakpoint offset and exposes the live frame (reuse the coroutine frame-snapshot machinery) | Open |
| **D** — frame → REPL bridge: seed a `ReplSession`'s store-resident environment from the paused frame's slot table + values (**depends on @PLN14 frame-seedable env**) | Open |
| **E** — conditional / test breakpoints: an expression/assertion evaluated in the frame env decides whether to break | Open |
| **F** — stepping + resume verbs (step over / into / out, continue) driving the bytecode loop — exposed as the **host-agnostic debug API** that all surfaces call | Open |
| **G1** — terminal / CLI surface: drop into the **shipped REPL** at a paused frame (the near-free headless front-end) | Open |
| **G2** — browser surface: `loft-dap` protocol + web UI (gutter, variables, call stack, REPL console) — the *natural home* | Open |

## Phase ordering

1. **A + C together** — a registry that can pause the interpreter at a line is the
   spine; nothing is observable until execution can stop at a point.
2. **D (frame → REPL bridge)** — the payoff: a paused frame becomes an inspectable
   REPL. Gated on @PLN14's env being seedable from an arbitrary frame (the
   sharpening that plan now carries).
3. **F (debug API) + G1 (terminal surface)** — wrap A–E as the headless API and
   prove it end-to-end from the **terminal** first (the REPL is already there), so
   the engine ships useful before any browser work.
4. **B (interpret-scope)** — needed for breakpoints reached through compiled code
   and for an introspectable call stack; start with "interpret from entry point,"
   refine to call-graph-reachability so unrelated code stays compiled.
5. **E (conditional/test)** — thin once D exists: a condition is `eval` against the
   frame env.
6. **G2 (DAP + browser UI)** — the protocol adapter (`loft-dap`, scoped under
   @PLN/09-lsp LSP.3) and the web-IDE surface consume the same API as G1.

## Open design questions

1. **Interpret scope — static vs on-demand.** "Everything that calls that routine"
   needs the *dynamic* call-chain interpreted so the whole stack is introspectable.
   Static (all transitive callers) is conservative and can engulf the program;
   on-demand (compiled until a frame that can transitively reach a breakpoint is
   entered, then switch that frame + descendants) is precise but needs a "can-reach
   a breakpoint" call-graph analysis. Lean on-demand; B builds the reachability.
2. **Pause granularity.** Per-source-line (DAP-natural) vs per-bytecode-op. Line is
   the user model; map line → the offset range and pause at the first op of the
   line.
3. **Frame mutation semantics.** Editing a local at a break writes the
   store-resident env; on resume the frame must read the edited value. Does the
   seeded env *alias* the live frame's slots, or is it a copy written back on
   resume? (Aliasing is faithful but couples the REPL env to the live stack; copy
   + write-back is cleaner but must reconcile.) Same store-copy-vs-alias question
   @PLN14 Q2/Q4 raises — resolve once, shared.
4. **Compiled frames above the break.** If a caller is compiled (not yet switched),
   its frame isn't introspectable. Does step-out across a compiled frame degrade
   gracefully (show "compiled frame, no locals") or does B guarantee the whole
   reached chain is interpreted? Lean on the latter (B's job).
5. **Coroutine reuse depth.** The suspend hook should reuse the coroutine
   frame-snapshot/restore path, not a parallel one — confirm a breakpoint pause is
   expressible as the same suspend primitive (it is a yield to the debugger).

## Cross-arc dependencies

- **@PLN12** (REPL + introspection, **finished**) — the interactive surface; the
  REPL-at-frame is a `ReplSession` over the paused frame, and the terminal surface
  (G1) is that REPL directly.
- **@PLN14** (store-resident REPL session) — **load-bearing**: the breakpoint frame
  *is* the store-resident environment. @PLN14 now carries the sharpening that its
  env must be seedable from an arbitrary live frame, not only typed bindings (sub-D
  here consumes it).
- **N9/C71 per-function interpret dispatch + Goal-D parity** — the interpret-scope
  switch (B) and the invisibility of interpret-on-break (parity).
- **Coroutine suspend machinery** — the frame-snapshot primitive C reuses.
- **`loft-dap` / 09-lsp LSP.3** — the browser protocol surface (G2);
  **25-native-debug** is the `--native` GDB/LLDB complement (this plan is the
  interpreter-mode debugger); **lib_plans/07-web-ide** is the browser UI host.

## See also

- [LAVITION.md](../../LAVITION.md) — the Live-interface / State-continuity /
  Fault-containment continuity nodes this plan realizes.
- [@PLN14 store-resident REPL session](../14-store-resident-repl-session/README.md)
  + [CONVERGENCE.md](../14-store-resident-repl-session/CONVERGENCE.md) — the frame
  environment model.
- [lib_plans/future/09-lsp/](../../lib_plans/future/09-lsp/README.md) — `loft-dap`
  (LSP.3) protocol; [plans/future/25-native-debug/](../future/25-native-debug/README.md)
  — the native-mode complement.
- **Tracker:** to be filed as `@PLN15` on `loft-lang/plans`; labels `plan` ·
  `subject:loft` · `status:future`.
