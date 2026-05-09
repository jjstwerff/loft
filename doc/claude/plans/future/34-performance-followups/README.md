<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Performance — open optimization follow-ups

The performance reference (benchmark results, root-cause
analysis vs CPython / hand-written Rust, how-the-interpreter-
executes, wasm-vs-native gap analysis) lives at
[`../../../PERFORMANCE.md`](../../../PERFORMANCE.md).

This plan tracks the **7 open optimization designs** in
PERFORMANCE.md as actionable items.  Each row points at the
PERFORMANCE.md section that holds the full design + the
ROADMAP row that schedules it.

## Status

| Item | PERFORMANCE.md section | ROADMAP row | Estimated impact | Status |
|---|---|---|---|---|
| **P1** — Superinstruction merging | [§ Design: P1](../../../PERFORMANCE.md) (line 258+) | O1 (currently noted "Opcode table full 254/256" — blocked) | Interpreter hot path; reduces dispatch overhead via combined-op opcodes | Open — blocked by opcode-table capacity (254/256 used).  Resolution: retire enough rarely-used Op codes OR widen the opcode field.  Decide before P1 itself starts. |
| **P2** — Reduce store indirection on the stack | [§ Design: P2](../../../PERFORMANCE.md) (line 408+) | (no direct row; PLANNING.md cites P2 cross-link) | Interpreter; reduces per-op pointer chasing for stack-resident values | Open — design ready, no scheduled implementation slot. |
| **P3** — Confirm integer paths carry no long sentinel | [§ Design: P3](../../../PERFORMANCE.md) (line 512+) | (no direct row) | Interpreter; verifies the Plan-01 `i32::MIN`-removal stuck and didn't get re-introduced | Open — verification + audit task; small. |
| **N1** — Direct-emit local collections in native codegen | [§ Design: N1](../../../PERFORMANCE.md) (line 549+) | **O4** "Native: direct-emit local collections" | Native; eliminates the `Stores::scratch` push for compile-time-known collection sizes | Open — design ready.  Cooperates with `lib_plans/future/03-lazy-stdlib/` (smaller scratch buffer reduces lazy-load latency) and `plans/future/21-retire-scratch/` (this is one of the consumers that would unblock retiring scratch entirely). |
| **N2** — Omit `stores` parameter from pure native functions | [§ Design: N2](../../../PERFORMANCE.md) (line 649+) | **O5** "Native: omit `stores` from pure functions" | Native; saves 8 bytes per call site for pure (non-store-touching) functions | Open — design ready. |
| **N3** — Remove long null-sentinel from generated code | [§ Design: N3](../../../PERFORMANCE.md) (line 748+) | (no direct row; tracked under O-tier in PLANNING.md) | Native; removes the dead `i64::MIN` null-check from i32-storage paths post-Plan-01 | Open — verification + cleanup; small. |
| **W1** — wasm string representation | [§ Design: W1](../../../PERFORMANCE.md) (line 811+) | (no direct row) | WASM; closes the wasm-vs-native gap on text-heavy workloads | Open — design ready, scheduled WASM-tier work. |

Other ROADMAP rows that conceptually belong here but don't
have explicit PERFORMANCE.md design content yet:

| Row | Title | Status |
|---|---|---|
| **A12** | Lazy work-variable initialization | Open — listed in PLANNING.md tier; design lives there.  Could fold into PERFORMANCE.md as a new section if scope grows. |
| **O2** | Stack raw pointer cache | Open — same pattern as A12; design in PLANNING.md. |
| **A4** | Spatial index operations | Open — same. |

These three stay as PLANNING.md-cited rows for now; if/when
they grow design content, they move into PERFORMANCE.md +
get a row in this plan.

## Why these items are here, not in PERFORMANCE.md

PERFORMANCE.md is reference documentation — it describes
how the interpreter executes today, where the wasm-vs-native
gap comes from, what each optimization would change about
the runtime.  Anyone profiling loft or proposing a new
optimization reads PERFORMANCE.md.

The open optimization items don't fit that purpose: they're
items to BUILD, not architecture to understand.  Per the
docs-vs-plans rule, they belong in `plans/future/`.  Keeping
them visible in the `plans/future/` index ensures they don't
get lost as PERFORMANCE.md grows.

The pointer-plan shape (this README references PERFORMANCE.md
sections rather than copying the design content) avoids
duplication — design details stay in one place.  When an
item ships, the work in PERFORMANCE.md gets trimmed (or
moved into the proper "how things work" section per the
closure rule) and this plan's row moves to a closure record.

## Phase ordering

Per [PERFORMANCE.md § Improvement priority order](../../../PERFORMANCE.md):

The doc itself orders the items by impact + ease-of-landing.
Suggested sequence when this plan unpauses:

1. **P3 + N3** — small verification / cleanup tasks; ship
   first to clear the deck.
2. **N2** — pure-fn `stores` omission; small native win,
   independent of other items.
3. **N1** — direct-emit local collections; cooperates with
   plans/21-retire-scratch — landing N1 narrows the scratch
   consumer set.
4. **P1** — superinstruction merging; interpreter hot path,
   biggest single interpreter win.  BLOCKED on opcode-table
   capacity decision.
5. **P2** — store indirection reduction; interpreter; smaller
   than P1 but architecturally cleaner.
6. **W1** — wasm string representation; closes the
   wasm-vs-native gap.  Scheduled when wasm becomes a
   priority workload (game-client + browser-IDE consumers).

Each item is independent — order can shift based on which
consumer (interpreter vs native vs wasm) needs the win first.

## See also

- [`../../../PERFORMANCE.md`](../../../PERFORMANCE.md) —
  full performance reference (benchmarks, root-cause
  analysis, design content for each item)
- [`../21-retire-scratch/`](../21-retire-scratch/) —
  cooperates with N1 (N1 narrows the scratch consumer set)
- [`../25-native-debug/`](../25-native-debug/) — sibling
  pointer-plan for native-codegen follow-ups
- [`../33-native-codegen-followups/`](../33-native-codegen-followups/) —
  another pointer-plan precedent (NATIVE.md companion)
- [`../../../PLANNING.md`](../../../PLANNING.md) —
  priority-ordered backlog (cites PERFORMANCE.md for the
  technical detail of each item)
- [`../../../ROADMAP.md`](../../../ROADMAP.md) — milestone
  placement
