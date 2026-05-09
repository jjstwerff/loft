<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 04 — Slot assignment redesign (CLOSED)

**Reference for the SHIPPED slot assignment lives at
[`doc/claude/SLOTS.md`](../../../SLOTS.md).**  That doc covers
the V1 production allocator, the V2 shadow validator, frame
layout, both zones, codegen invariants I1-I7, plan-04's Phase 0
fixture catalogue, and diagnostic tools.  Anyone reading or
modifying slot assignment reads SLOTS.md.

This file is a closure record only.  The 15 sub-files in this
directory (`SPEC.md`, `walkthroughs.md`, `SPEC_GAPS.md`, the
phase plan files, the V2 design analyses) are **historical
implementation record for the retracted V2 design** — preserved
as archaeology, not as current reference.

## Status — 2026-04-23: closed

All phases landed.  Plan moved to `plans/finished/`.

**Outcome — three successive refits, no V1 retirement.**  The
original goal (replace V1 with V2 single-pass algorithm) was
**retracted** after the V2-drive attempts failed on variables
declared at outer scope but first-Set in inner scope.  V1
remains the production allocator.  V2
(`src/variables/slots_v2.rs`) stays as a shadow validator
invokable via `LOFT_SLOT_V2=validate`.

What did land:

| Phase | Commit | Outcome |
|---|---|---|
| **A** | `9f759ee` | Invariant **I7 — scope-frame consistency** in `validate.rs`.  Converts `Incorrect var X[slot] versus TOS` runtime panics into compile-time `[I7]` diagnostics. |
| **B.1** | `9f759ee` | Positional init primitives `OpInitRefSentinel(pos)` + `OpInitCreateStack(pos, dep_pos)` added alongside existing `OpInitText(pos)` / `OpInitRef(pos)` (from 2h.1). |
| **B.2** | `bea156a…5e35948` | Rewired every codegen call site + parser-emitted compound op via `generate_call` interception; deleted `OpText` (−1 opcode).  Three remaining compound ops (`OpConvRefFromNull`, `OpNullRefSentinel`, `OpCreateStack`) became dead runtime code but kept dictionary entries. |
| **B.3** | atomic bundle `06a8d14` + follow-up v2 `f47cc93` | Single function-entry `OpReserveFrame(frame_hwm)` (replaces per-block reserves); slot-move deletion. |
| **B.4** | (within atomic bundle) | Final cleanup pass. |

Companion plan-05 (`finished/05-orphan-placer-elimination/`)
deleted `place_orphaned_vars` — together with plan-04's I7
invariant, structurally prevents the slot-above-TOS bug class.

## Why the V2 retraction

Two V2-drive attempts failed on the same shape:

1. **codegen-is-allocator** (rejected) — codegen would compute
   slot positions inline.  Failed: codegen sees IR in walk
   order, but slot placement needs full liveness info first
   (chicken/egg).
2. **V2-drive switchover** (rejected) — replace V1 entirely
   with V2.  Failed: variables declared at outer scope but
   first-Set in inner scope (cross-scope deferred initialisation
   pattern) didn't fit V2's "first-def == first-set" assumption.
   The attempt produced overlap on patterns like
   `x: integer = 0; if c { x = 5 } else { x = 10 }` where
   the outer-scope `x: integer = 0` is first-Set in the
   if-branches.

Result: V1 ships; V2 stays alive as a shadow validator
(invariant-checker against the same input).  Future revisits
to V2-drive would need a different design that handles
cross-scope deferred init.

## What's preserved as historical record

The 15 sub-files in this directory document the V2 design
journey:

- `SPEC.md` — full V2 specification (the algorithm V2 would
  use if it drove placement).
- `walkthroughs.md` — per-fixture walkthroughs of how V2
  would place each pattern.
- `SPEC_GAPS.md` — critique log of V2 spec ambiguities found
  during Phase 1.
- `00-characterize.md`, `00a-audit.md`, `01-design.md`,
  `02-parallel-impl.md`, `02c-optimality-report.md`,
  `02h-codegen-refactor.md`, `03-switch.md`, `04-cleanup.md`
  — phase-by-phase plan documents.
- `b3-function-entry-reserve.md`, `b3-par-inline.md`,
  `b3-par-type-lie.md` — Phase B.3 design analyses.

Anyone investigating slot-assignment internals or considering
a future V2-drive attempt reads these for the design
archaeology.  For everyday work on slot assignment, read
`SLOTS.md` instead.

## See also

- [`doc/claude/SLOTS.md`](../../../SLOTS.md) — shipped V1
  allocator + V2 shadow validator reference (where you should
  be reading if you want to know how slot assignment works
  today)
- [`finished/05-orphan-placer-elimination/`](../05-orphan-placer-elimination/) — companion plan that deleted `place_orphaned_vars`
- CHANGELOG_TECHNICAL.md — opcode-table changes (`OpText`
  deletion, `OpReserveFrame` consolidation)
