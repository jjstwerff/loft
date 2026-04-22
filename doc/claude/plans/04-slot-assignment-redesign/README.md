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

### Hard constraint — uniform placement

**Every local goes through the same placement path.**  The redesign
is explicitly illegal if it:

- treats **small variables** (≤ 8 B — `integer`, `boolean`,
  `character`, enum values, references-as-handles) differently from
  **large variables** (text, vectors, structs, anything via
  `Store::alloc`);
- treats **individual variables** differently from **sets of
  variables** (all locals in a Block, all locals in a Loop, all
  orphaned locals, the args + return-address prefix);
- carries any branch of the form "if this variable's size is N" or
  "if this scope contains more than one variable" or "if this
  variable's scope-shape is X."

The algorithm must accept a flat list of `(live_interval, size,
alignment)` triples and emit `slot_position` for each, with the same
code path producing the result whether the input has one variable
or five hundred, and whether each one is one byte or one kilobyte.

This constraint is deliberately stronger than "simpler."  The
recurring bug pattern (P178, P185, and the P122p/q/r series before
them) is that size-based or shape-based branches accumulate filters
every time a new aliasing case surfaces.  Making it **illegal** for
the new algorithm to branch on size or group membership is the
structural guarantee that the next aliasing case has nowhere to
hide — it either breaks the one placement path (loud failure), or
it doesn't exist.

Ergonomic exceptions the constraint does NOT rule out:
- Reading size + alignment from the input triple is fine — the
  constraint is against *branching* on them, not against *reading*
  them (an interval-graph-colouring step that looks at size to
  pick the lowest non-overlapping slot is one branch-free formula,
  not "zone 1 vs zone 2").
- The runtime still has its own notion of frame layout
  (`OpReserveFrame`, return-address offset).  The allocator speaks
  to that contract through its output, not by simulating it
  internally with a size-based split.

### Key design questions to resolve in Phase 1

- **Loop-scope lifetime.**  Loops currently need sequential
  placement because per-block `OpFreeStack` inside a loop corrupts
  the iteration.  Under the uniform-placement rule, loops and
  blocks must share the algorithm — so the runtime contract has
  to change: one function-entry `OpReserveFrame(hwm)` covers the
  whole function, and neither blocks nor loops `OpFreeStack`.  The
  cost is that block-scope slot reuse across sibling blocks has to
  come out of liveness analysis (which it already does) rather
  than out of `OpReserveFrame` / `OpFreeStack` bookkeeping.
- **Block-return aliasing.**  Today `Set(v, Block([..., result]))`
  implicitly writes `result` through `v`'s slot — treating the
  Block's locals as a set with shared placement.  Under the
  constraint, "the block writes through v's slot" has to be
  expressed in the IR (e.g. rewrite to `Block([..., Set(v,
  result)])`) rather than as a placement-time special case.
- **Arguments.**  The args + return-address prefix is currently a
  pre-reserved region that placement avoids.  Options that respect
  the constraint: (a) finalise arg positions by extending the same
  liveness graph to include args as "live from entry to last use";
  (b) keep arg placement as a runtime-layout concern owned by
  codegen, with the allocator producing slots that start at an
  offset the allocator doesn't know or care about.  Phase 1 picks
  one — no mixed approach.

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
6. **Uniform-placement audit:** grep the post-switch `slots.rs`
   (and any files it delegates to) for branches on variable size,
   scope kind (Block vs Loop vs "orphan"), or set cardinality.
   Every hit either (a) has a rationale documented in the code
   explaining why it is NOT size/shape-based placement dispatch
   (e.g. an interval-graph colouring step that reads size to
   compute overlap), or (b) is a regression against the hard
   constraint and must be removed.  This check lands as a
   `tests/doc_hygiene.rs::slot_allocator_has_no_size_or_shape_branches`
   lint in Phase 4.

## Related

- [SLOTS.md](../../SLOTS.md) — current design (will be rewritten in Phase 4).
- [PROBLEMS.md § P178](../../PROBLEMS.md) — orphan-placer argument-area overlap.
- [PROBLEMS.md § P185](../../PROBLEMS.md) — orphan-placer text-buffer overlap.
- `src/variables/slots.rs` — the subject.
- `src/variables/intervals.rs` — live-interval computation (already present, to be reused).
- `src/variables/validate.rs` — debug-only overlap validator (Phase 2's equivalence check will piggy-back on this).
