<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 120 — Debugger: make it findable, and stop it failing silently

## Status

**Open — design complete ([DESIGN.md](DESIGN.md)), one arc shipped.** The debugger
(@PLN16, @F51) works: breakpoints, live frame, expression eval, frame edits,
stepping, watchpoints, undo/redo, an NDJSON RPC surface, a DAP binary. What it
lacks is **reach** — it cannot be pointed at a multi-package program at all
(`--lib` is ignored), and agents do not know it exists — plus three failure modes
that are **silent**, which is what makes the working parts feel unreliable.

Arc **C3** (bare verb shadowed a live local) shipped 2026-07-27 with its
regression: `src/repl.rs::handle_paused` + `paused_prompt_tests`. Everything else
is open.

Every claim below was reproduced on this tree; the mechanism for each is pinned in
[DESIGN.md](DESIGN.md), not hypothesised.

## Goal

`loft debug` runs any program `loft` runs, including a multi-package one; a paused
frame shows every local in lexical scope at that line; no breakpoint is
simultaneously `verified` and permanently inert; no failure inside a session is
unnamed; and an agent who has read only `CLAUDE.md` can find the tool.

## Effort + design

- **Effort:** M (A is the only non-trivial arc; E1/E2 are XS, B/C/D/E3 are S)
- **Design:** ✓ [DESIGN.md](DESIGN.md)
- **Last touched:** 2026-07-27

## Why this is worth a plan

Three independent observations, one week — two consumers and ourselves:

- **moros** tried to debug their editor server and could not reach it at all
  (H13 + their `doc/claude/LOFT_DEBUGGER.md`): `--lib` ignored, and any call into
  native code kills the session unnamed. Their conclusion — *"the consumer-visible
  shape is 'the debugger does not work on real programs', which undersells it
  considerably"* — is arc E. They also did the bisection that turned it from a
  package problem into a **native-call-boundary** problem, with controls.
- The **zero-trust** agent found the debugger late, then wrote its own guide into
  their `doc/DEVELOPMENT.md` ("The loft debugger works — use it for pure-loft
  logic") with the edges they hit. A consumer re-deriving our documentation is the
  signal.
- Working moros H9 *in this repo*, the loft agent reached for `gdb` and hand-placed
  `println`s and never ran `loft debug` — then drove the RPC wrongly by skipping
  `launch`. The `loft-debug` skill documents that sequence **correctly**, including
  "order matters". The knowledge existed and was not reached.

Note the two failure shapes compose badly: the tool cannot be pointed at a real
program (**E**), and when it does run, its worst edges are **silent** (**B**,
**E.3**) — so a first attempt ends without a diagnosis, and the second attempt does
not happen. Discoverability (**C**) only pays off once **E** is true.

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **E1** — `--lib` is ignored, so no multi-package program can be debugged | [DESIGN.md § E.1](DESIGN.md#e1----lib-is-ignored-so-no-multi-package-program-can-be-debugged) | Open — **highest impact** |
| **E2** — the target argument does not skip flags; the message names the wrong token | [DESIGN.md § E.2](DESIGN.md#e2--the-target-argument-does-not-skip-flags) | Open — XS |
| **E3** — a native call abandons the session, unnamed (payload discarded) | [DESIGN.md § E.3](DESIGN.md#e3--a-native-call-abandons-the-session-unnamed) | Open |
| **B** — a condition that cannot be evaluated must say so, never read as `false` | [DESIGN.md § B](DESIGN.md#b--no-silent-lies-breakpoint-conditions) | Open |
| **A** — frame liveness: show lexical scope, not the bytecode reference span | [DESIGN.md § A](DESIGN.md#a--frame-liveness) | Open — the design arc |
| **C1** — `DEBUG.md` gains an `## Interactive debugging` section | [DESIGN.md § C](DESIGN.md#c--discoverability) | Open |
| **C2** — `CLAUDE.md` § Key commands gains one line | [DESIGN.md § C](DESIGN.md#c--discoverability) | Open |
| **C3** — a bare verb must not shadow a live local | [DESIGN.md § C](DESIGN.md#c--discoverability) | **Shipped** 2026-07-27 |
| **D** — `:vars` temp noise; fold the consumer's write-up back | [DESIGN.md § D](DESIGN.md#d--cleanup-and-the-consumers-write-up) | Open — blocked on A |

## Phase ordering

1. **E first**, and E1/E2 before E3. Nothing else matters if the tool cannot be
   pointed at the program: moros's reading is *"the debugger does not work on real
   programs"*, and E1 is a parameter that one sibling code path already passes.
   E2 is a one-line filter. E3 splits into "print the discarded panic payload"
   (do immediately — it is what will name the cause) and the dispatch fix after.
2. **B next.** Independent, small, and removes the worst property of the tool — a
   breakpoint that reports `verified: true` and then never fires. Do it before A,
   because a working condition is then a usable probe for A.
3. **A.** The design arc, and the root of the remaining complaints (D's `:vars`
   noise is only noise *because* the user's own variable is missing). Take A.3's
   measurement before building.
4. **D after A**, because filtering `i#index` today would remove the only signal
   about loop position while the user's `i` is invisible — the two must move
   together.
5. **C any time**, cheapest first: C1 is the biggest single gap and is one section
   in one file.

## Open design questions

1. **A — scope reconstruction source.** The IR carries block structure and
   `Variables` carries per-var scope; `capture_frame_at` currently reconstructs
   liveness from `State::vars` (a bc→var map) instead. Is the declaring block
   recoverable at a pc cheaply enough to build the frame per pause, or does it want
   a precomputed per-function scope table? [DESIGN.md § A.3](DESIGN.md#a3--where-the-scope-fact-comes-from)
   proposes the table; the measurement that settles it is named there.
2. **A — the uninitialised window.** A local in scope but not yet assigned at this
   pc (`step` at the top of the line that writes it) has a slot with undefined
   contents. Show it as `<unset>`, or omit it? The design proposes `<unset>`
   (absence is what today's model already gets wrong), but this is the one
   user-visible judgement call in A.
3. **B — report-once-and-skip, or report-and-break?** A broken condition on a hot
   line could spam. The design proposes report-once per breakpoint then break, on
   the grounds that a user who typo'd a condition wants to be stopped, not warned
   in scrollback.

## Cross-arc dependencies

- **@PLN16** (debugger) — this plan is its follow-through; PROTOCOL.md is the RPC
  contract B extends with an error shape.
- **@I91 / lib_plans/63-lsp** (LSP + DAP) — arc A changes what a frame contains, so
  the DAP surface inherits it. Out of scope here, but the DAP tests are a consumer
  of A's fix.
- **`--lean`** already gates a live/debug tier, so "debug builds keep more" has a
  precedent to follow rather than invent (A.2 rejected alternative).

## See also

- [DESIGN.md](DESIGN.md) — the full design, one section per arc.
- [`../16-debugger/PROTOCOL.md`](../16-debugger/PROTOCOL.md) — the NDJSON contract.
- [`../../DEBUG.md`](../../DEBUG.md) — the routed debugging doc; C1 lands there.
- `.claude/skills/loft-debug` § *The agent debug surface* — canonical for the RPC
  surface; C points at it rather than copying it.
- `doc/features/F51.md` — the user-facing catalogue entry (generated; edit the
  issue).
- @PLN120 — [loft-lang/plans#120](https://github.com/loft-lang/plans/issues/120).
