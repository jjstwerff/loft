<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 120 — Debugger: make it findable, and stop it failing silently

## Status

**Active — every arc shipped except A (2026-07-27).** Reach (**E**), the silent
failures (**B**, **E3**), discoverability (**C**) and half of the cleanup (**D1**)
are done and gated; **A** — the frame's liveness model — is the one arc left. Its
design is now **complete and measured** ([DESIGN.md § A](DESIGN.md#a--frame-liveness)):
three facts, one query, five consumer sites, a hand-computed expectation table, and
its gate. Ready to build; no open question left in A.

What that changed, concretely: a multi-package program can be debugged
(`--lib` reaches the interactive path), a call into native code no longer kills
the session, a failure inside a session names its cause, a breakpoint condition
can no longer be both `verified` and permanently inert, `DEBUG.md` and `CLAUDE.md`
finally mention the tool, and typing `c` at the prompt reads your local instead of
resuming the program.

**Still true, and the reason A matters:** a paused frame shows only variables whose
bytecode references bracket the stopped instruction, so a local read later on the
same line — or one whose last read has passed — is missing even though it is in
scope. On the probe, breaking on `total = total + step` shows **nothing about the
loop at all** — not `i`, not `step`, not even the `i#index` temp.

**And one thing that is no longer true.** Designing A turned up a fact that
reshapes it: `i` and `step` **share a stack slot** (the allocator is scope-blind by
design), so this plan's original target — show the locals in lexical scope, with
their values — is both unachievable (`i`'s value is gone at line 5) and unsafe (it
would print `i`'s bytes under `step`'s name at line 4). A's invariant is restated to
carry an explicit third state per local; see [DESIGN.md § A.1](DESIGN.md#a1--the-fact-that-changes-the-arc-two-locals-share-one-slot-verified).

Every claim below was reproduced on this tree; the mechanism for each is pinned in
[DESIGN.md](DESIGN.md), not hypothesised. Where designing A contradicted an earlier
claim of this plan's, the correction is recorded next to it rather than quietly
edited away — three of them, all in § A.

## Goal

`loft debug` runs any program `loft` runs, including a multi-package one; a paused
frame shows every local in lexical scope at that line, each with its own value or an
explicit reason it has none — never with another local's bytes; no breakpoint is
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
| **E1** — `--lib` ignored, so no multi-package program could be debugged | [DESIGN.md § E.1](DESIGN.md#e1----lib-ignored--shipped-2026-07-27) | **Shipped** 2026-07-27 |
| **E2** — the target argument did not skip flags | [DESIGN.md § E.2](DESIGN.md#e2--the-target-argument-does-not-skip-flags--shipped-2026-07-27) | **Shipped** 2026-07-27 |
| **E3** — a native call abandoned the session, unnamed | [DESIGN.md § E.3](DESIGN.md#e3--a-native-call-abandoned-the-session-unnamed--shipped-2026-07-27) | **Shipped** 2026-07-27 |
| **B** — a condition that cannot be evaluated must say so, never read as `false` | [DESIGN.md § B](DESIGN.md#b--no-silent-lies-breakpoint-conditions--shipped-2026-07-27) | **Shipped** 2026-07-27 |
| **A** — frame liveness: show lexical scope, and say why a local has no value | [DESIGN.md § A](DESIGN.md#a--frame-liveness) | **Open — the one arc left.** Design complete 2026-07-27 |
| **C1** — `DEBUG.md` gains an `## Interactive debugging` section | [DESIGN.md § C](DESIGN.md#c--discoverability) | **Shipped** 2026-07-27 |
| **C2** — `CLAUDE.md` § Key commands gains one line | [DESIGN.md § C](DESIGN.md#c--discoverability) | **Shipped** 2026-07-27 |
| **C3** — a bare verb must not shadow a live local | [DESIGN.md § C](DESIGN.md#c--discoverability) | **Shipped** 2026-07-27 |
| **D1** — `:vars` temp noise | [DESIGN.md § D](DESIGN.md#d--cleanup-and-the-consumers-write-up) | **Partly shipped** — `__`-temps filtered + `:vars all`; `i#index` still shown, to be filtered with A |
| **D2/D3** — fold the consumers' write-ups back | [DESIGN.md § D](DESIGN.md#d--cleanup-and-the-consumers-write-up) | Open |

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
   noise is only noise *because* the user's own variable is missing). Its six build
   steps are ordered in [DESIGN.md § A.8](DESIGN.md#a8--steps): the two recording
   steps land **inert** (nothing reads them), so the frame does not change until the
   query is wired.
4. **D after A**, because A is what makes the frame worth filtering — not, as
   originally stated, because `i#index` is a signal worth keeping (§ A.0 measures
   that it is not shown at either loop line).
5. **C any time**, cheapest first: C1 is the biggest single gap and is one section
   in one file.

## Open design questions

**All closed.** Kept with their answers, because two of them were answered
*against* the design that posed them.

1. **A — scope reconstruction source.** *Answered:* a per-definition table of
   `(pc_start, pc_end, scope)` spans recorded by codegen's block walk. The reason
   the earlier answer gave for preferring it (a pause-time IR walk needs a
   scope-parent relation that does not exist) turned out to be unnecessary as well:
   codegen emits a child block *inside* its parent's span, so **pc-range containment
   IS the nesting relation** — there is no scope tree to build.
   [DESIGN.md § A.3](DESIGN.md#a3--the-three-facts-and-where-each-comes-from).
2. **A — the uninitialised window.** *Answered: `<unset>`, and it is no longer a
   judgement call — it is a safety requirement.* `reserve_frame` does not zero
   locals, so an unwritten `text`/`vector` slot holds a garbage pointer; and because
   locals share slots, an unwritten slot often holds **another live local's value**,
   so reading it is a wrong answer rather than a blank one. A second marker joins it:
   `<reused by step>` for a local that is in scope but whose slot has been taken over.
3. **B — report-once-and-skip, or report-and-break?** *Shipped as proposed:*
   report once per breakpoint, then break.

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
