<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 04 — Slot assignment redesign

**Status:** open — Phase 0 ready to start.

**Goal:** replace the current two-zone slot allocator plus orphan-
placement post-pass with a single algorithm that assigns every local
variable a slot in one deterministic pass.  The current design has
produced a recurring class of safety bugs (P178, P185, and likely
more) that each required a targeted patch on top of the existing
heuristics.  The redesign removes the heuristics rather than patching
them one at a time.

## Context

`src/variables/slots.rs::assign_slots` is the single authority for
variable slot positions.  Its current shape (as documented in
[SLOTS.md](../../SLOTS.md)):

- **Two zones.**  Small (≤ 8 B) variables go to zone 1 with greedy
  interval colouring and `OpReserveFrame`.  Large (> 8 B) variables
  and every loop-scope variable go to zone 2 with sequential IR-walk
  placement.  The split matches a real runtime distinction (loops
  can't reuse zone 1 across iterations) but the two halves have
  different placement algorithms that must agree on invariants like
  "no slot is shared with a still-live variable."
- **Special cases in the IR walk.**  Block-return, Insert preamble,
  Loop-scope, orphaned variables — each is handled by a separate
  branch in `process_scope` / `place_large_and_recurse`.
- **Orphan placer.**  `place_orphaned_vars` runs after the main walk
  for variables whose scope is not a Block/Loop node.  Started slot
  search from `0` before P178 → overlapped arguments; patched with
  `local_start` parameter.  Started without considering iterator-
  temporary liveness before P185 → overlapped a live text buffer;
  no patch yet.

The cumulative result is ~800 lines of slot-placement code, multiple
passes over the IR, and a documented pattern of "add a tactical
filter to `place_orphaned_vars` whenever a new aliasing bug is
reported."  P185's root cause is the third orphan-placement
regression in this class; the pattern makes another one likely.

## Why a redesign and not another patch

- **Classified safety bugs:** heap corruption / use-after-free, not
  just wrong values.  Any single-instance fix leaves the neighbouring
  configurations vulnerable until someone stumbles on them — exactly
  how P185 was found (generator script crashes mid-write).
- **Heuristic cost:** each patch adds a guard without retiring the
  code path that needs the guard.  The two-zone design, sequential
  IR-walk, and separate orphan placer will still coexist after any
  P185-specific fix, so the next aliasing edge case still has room
  to hide.
- **Test cost:** every new special case needs its own fixture.  The
  `p178_*`, `p185_*`, and SLOTS.md pattern-tests table would grow
  indefinitely under the patch-by-patch approach.

## Design direction (not locked)

Single-pass placement driven by liveness intervals:

1. Compute live intervals for every local (already done — see
   `src/variables/intervals.rs`).
2. Sort intervals by start, assign each the lowest slot not occupied
   by a live-overlapping interval of incompatible size.
3. Emit `OpReserveFrame(hwm)` once at function entry; loops inherit
   the function-level frame.
4. Codegen reads slots as today.

Key design questions to resolve in Phase 1:
- Can we drop the small/large zone split entirely?  (Loops currently
  need zone-2 sequential placement because `OpFreeStack` inside a
  loop doesn't work.  If function-entry `OpReserveFrame` replaces
  per-block reservation, this restriction goes away.)
- What replaces the "Block-return shares its parent's slot" special
  case?  Liveness says the parent local and the block-result local
  have overlapping lifetimes, so they can't share — unless the
  IR-walk preserves the semantic that the block writes through the
  parent's slot.  Either the IR is rewritten to remove the implicit
  sharing, or the slot allocator keeps a notion of "aliased pair."
- Argument slots: today args occupy `stack_pos == u16::MAX` until
  codegen fills them in.  Either finalise arg slots during
  `assign_slots`, or pre-reserve the arg region and have the allocator
  start placement above it.

## Ground rule — no regressions

Per [`plans/README.md`](../README.md): every phase must preserve the
full test suite green.  No "rewrite everything then fix the fallout"
— each phase lands a single narrow change with a regression guard.

## Phases

| # | Phase | File | Status | Blocks |
|---|---|---|---|---|
| 0 | **Characterize** — lock current behaviour with tests (P178, P185, every SLOTS.md pattern as an explicit assertion), audit every `place_orphaned_vars` call site, produce a fixture catalogue. | [00-characterize.md](00-characterize.md) | open | 1 |
| 1 | **Design** — specify the single-pass algorithm; walk it through every fixture from Phase 0 and show by-hand what placement it produces.  Resolve open design questions above.  No code changes. | (not written) | blocked by 0 | 2 |
| 2 | **Parallel implementation** — build the new allocator behind a `LOFT_SLOT_V2` debug env var.  On every test run, compute both old and new placements and assert they match.  No codegen uses V2 yet. | (not written) | blocked by 1 | 3 |
| 3 | **Switch** — flip codegen to V2.  Remove V1.  Delete `place_orphaned_vars` and the zone split. | (not written) | blocked by 2 | 4 |
| 4 | **Cleanup** — update SLOTS.md to describe only V2, remove obsolete patterns from the test table, retire `LOFT_ASSIGN_LOG` if V2 has better diagnostics. | (not written) | blocked by 3 | — |

Phase 0 is the one that unlocks the rest — without an exhaustive
fixture catalogue there's no way to show Phase 2's equivalence
assertion is meaningful.

## Non-goals

- **Changing the runtime frame layout.**  The interpreter's stack
  representation (`Store::stack`, `OpReserveFrame`, `OpFreeStack`) is
  not in scope.  V2 must produce slot positions that slot into the
  existing runtime without opcode changes.
- **Native-codegen-specific slot logic.**  `src/generation/` consumes
  `stack_pos` as an opaque input; the redesign targets the
  interpreter path, and the native path inherits whatever V2
  produces.
- **Performance targets.**  Not a performance-driven rewrite.  Any
  throughput change (up or down) is acceptable if correctness and
  simplicity improve; the plan tracks wall-clock on `cargo test
  --release` only as a guardrail, not a metric.

## Success criteria

1. P178's and P185's regression tests un-`#[ignore]`'d and passing
   without per-case patches in the allocator.
2. `place_orphaned_vars` removed from the tree.
3. `src/variables/slots.rs` is under 800 lines (currently ~1676)
   *or* the new file has one exported entry point and a single
   inner helper (no "zone N" branching).
4. Every fixture from Phase 0 produces identical slot assignments
   under V2 (locked by Phase 2's equivalence assertion before the
   switch).
5. `tests/issues.rs` slot-related `#[ignore]` count stays at zero
   after Phase 3.

## Related

- [SLOTS.md](../../SLOTS.md) — current design (will be rewritten in Phase 4).
- [PROBLEMS.md § P178](../../PROBLEMS.md) — orphan-placer argument-area overlap.
- [PROBLEMS.md § P185](../../PROBLEMS.md) — orphan-placer text-buffer overlap.
- `src/variables/slots.rs` — the subject.
- `src/variables/intervals.rs` — live-interval computation (already present, to be reused).
- `src/variables/validate.rs` — debug-only overlap validator (Phase 2's equivalence check will piggy-back on this).
