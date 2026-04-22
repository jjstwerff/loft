<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 1 — Design the uniform-placement algorithm

**Status:** open.  Blocked by Phase 0.

**Goal:** produce a complete, executable-on-paper specification of
the V2 slot allocator.  Resolve the three open design questions
from [README.md](README.md) (loop-scope lifetime, block-return
aliasing, argument region).  Walk the spec through every Phase 0
fixture by hand and record the resulting placement.  No code changes.

## Inputs (from Phase 0)

- `tests/slot_v2_baseline.rs` — the fixture catalogue.
- SLOTS.md § "Scope shapes and orphan-placement" — the exhaustive
  list of IR shapes that V1 handles with shape-based dispatch.
- The size-or-shape-branch catalogue (Phase 0 step 5) — every
  branch V2 has to subsume.

## The spec to produce

A single document in this directory,
`doc/claude/plans/04-slot-assignment-redesign/SPEC.md`, organised
as follows.

### 1. Inputs and output of the allocator

Exactly what the allocator takes and returns.  The input is a flat
list of `LocalInterval { var_nr, live: Range<Pc>, size: u16, align:
u16, requires_ref_slot: bool }` triples, one per local and per work-
ref.  Arguments either join the list as "live from entry to last
arg-use" (option A) or are pre-placed and excluded (option B).
Phase 1 picks one.

The output is a `Vec<SlotAssignment { var_nr, slot: u16 }>` and a
single `hwm: u16`.  Nothing else — no zone split, no "this variable
is in the orphan set," no "this block reserves N bytes."

### 2. The algorithm itself

Written as a numbered list of unconditional steps, no `match` on
size or scope kind.  A starting sketch:

1. Sort `LocalInterval`s by live-range start (stable on ties).
2. For each interval in order:
   a. Compute the set of currently-live intervals that would
      overlap this one.
   b. Find the lowest slot offset `s` such that the range
      `[s, s + size)` is not occupied by any live-overlapping
      interval's `[slot, slot + size)`, and `s` respects `align`.
   c. Emit `SlotAssignment { var_nr, slot: s }`.
   d. Update `hwm = max(hwm, s + size)`.

The interval-graph-colouring step (2b) may read `size` and `align`
from the input; that is not a size-based branch — it is an
unconditional formula that happens to take size as a parameter.

### 3. The three design questions, resolved

For each of the three:
- **Loop-scope lifetime.**  Pick: (a) V2 produces slots that
  imply function-entry `OpReserveFrame(hwm)` + no `OpFreeStack`
  per block / per loop (simpler runtime contract, potentially
  larger frames on block-heavy functions), or (b) V2 still
  respects per-block `OpReserveFrame` / `OpFreeStack` but without
  branching in the allocator (would require the IR to carry the
  block-reserve markers so placement can read them without
  inspecting scope kind).
  Phase 1 picks one and shows, via the Phase 0 fixtures, that
  every existing block-scope reuse still works and no regression
  surfaces.
- **Block-return aliasing.**  Pick: (a) rewrite the IR in
  `scan_set` / `inline_struct_return` so `Set(v, Block([..., r]))`
  becomes `Block([..., Set(v, r)])` before allocation — the
  allocator never sees the aliasing; or (b) extend the input
  tuple to include an optional `aliases_with: Option<VarNr>` and
  treat aliased pairs as a single interval spanning both lifetimes.
  Phase 1 picks one and lists the fixtures that now pass without
  a "block-return" special case.
- **Argument region.**  Pick option A or B from the README and
  show, on the three argument-heavy fixtures from Phase 0 (P178's
  `&Struct` + non-trivial body, P143's multi-arg ref-returning
  call, `with_lots_of_args`), that placement comes out identical
  to V1.

### 4. Walk-through tables

One table per Phase 0 fixture, column headings:
`interval | live range | size | candidate slot | chosen slot | reason`.

Showing the by-hand trace for every fixture makes it obvious when
the spec's algorithm diverges from V1's observed placement.  At
that point we either:
- accept the divergence (V2 produces a valid-but-different layout —
  record that the Phase 0 fixture needs to be rewritten as an
  invariant check rather than a placement check), or
- tighten the spec to match V1 (only if V1's placement was
  load-bearing — e.g. a downstream native-codegen assumption).

### 5. Correctness gates for Phase 2

What V2 must satisfy beyond "compiles":
- For every input, the output is overlap-free (same invariant
  `validate_slots` checks today).
- `hwm` ≤ V1's `hwm` on every Phase 0 fixture (regression guard
  against the uniform algorithm accidentally using more stack).
  If a fixture shows V2 > V1, the spec needs review before
  Phase 2 starts.
- Behavioural equivalence: every existing `cargo test --release`
  test passes under V2.  This is the real correctness gate —
  slot-position equivalence is not (and under the uniform
  constraint, can't be).

### 6. What V2 still delegates to codegen / runtime

- `OpReserveFrame` / `OpFreeStack` emission (codegen reads `hwm`).
- Return-address offset.
- Native-codegen lifetime management in `src/generation/` (reads
  `stack_pos` as opaque input, no allocator logic).

## Non-goals for Phase 1

- No code is written.
- No fixture files are rewritten (Phase 0's output stands).
- No decision about `OpReserveFrame` opcode changes (if the loop-
  scope resolution demands a new runtime contract, Phase 1
  specifies what the new contract looks like but does not implement
  it — Phase 2 / 3 does).

## Ground rule — no regressions

Phase 1 is a paper exercise; there are no regressions to gate on
here.  But every fixture in Phase 0 must appear in § 4 with a
concrete slot assignment, and any "divergence from V1" note must
include the fixture ID and a written rationale.

## Deliverables

1. `SPEC.md` with sections 1–6 above.
2. An updated README.md "Design direction" section pointing at
   SPEC.md and summarising the resolved design decisions (one
   sentence per question).
3. A one-page walk-through showing the algorithm applied to the
   P178 and P185 fixtures end-to-end, included verbatim in SPEC.md.

## Done when

- Every Phase 0 fixture has a table row in SPEC.md § 4.
- The three open design questions have been resolved by picking
  one of the listed options with written rationale.
- The algorithm spec in § 2 is ≤ 15 numbered steps, none of them
  branching on variable size, scope kind, or set cardinality.
- Reviewers (the user, or a second reading session) can trace
  the P185 fixture through the spec and independently derive the
  slot positions shown.

## Open questions to flag if they surface

- Does the interval-graph colouring step need to know about
  `Store::alloc`-backed variables (text, vector) vs inline
  variables?  The uniform constraint says no — both are just
  `(live, size, align)` triples — but if V1's behaviour
  distinguishes them somehow, that has to come out in the spec.
- Does `requires_ref_slot` warrant a separate dimension in the
  colouring (so ref-holding slots can't be reused for non-ref
  locals even if sizes match)?  Current V1 logic around
  `is_work_ref` suggests yes; Phase 1 writes it down either way.
