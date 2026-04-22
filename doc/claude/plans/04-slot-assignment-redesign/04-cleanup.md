<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4 — Cleanup: docs, lint, close the initiative

**Status:** open.  Blocked by Phase 3.

**Goal:** rewrite the slot-allocation documentation to describe only
V2, land the uniform-placement lint, and close the initiative by
moving it to `plans/finished/`.  This phase is the structural
guarantee that the hard constraint stays in place — without the
lint, a future contributor could reintroduce a size/shape branch
without noticing.

## Entry criteria (from Phase 3)

- `src/variables/slots.rs` is V2; V1 is gone.
- All six gates from Phase 3 passed on `main`.
- P178 / P185 regression tests are active (no `#[ignore]`).

## Steps

### 4a — Rewrite `doc/claude/SLOTS.md`

Rewrite, not edit.  The current SLOTS.md describes two zones,
sequential IR-walk placement, and an orphan placer — none of which
exist anymore.  The new version:

- § Overview: one paragraph, the uniform-placement invariant
  (size/shape-branch-free).
- § Inputs and output: the `LocalInterval` struct from SPEC.md § 1.
- § Algorithm: copy SPEC.md § 2 verbatim, plus a pointer to
  SPEC.md for the by-hand walk-throughs.
- § Runtime contract: what codegen reads (`hwm`, per-var `slot`),
  what opcodes it emits (`OpReserveFrame` once per function — or
  whatever Phase 1 picked).
- § Diagnostic tools: `LOFT_ASSIGN_LOG` (keep if still useful,
  remove if V2 has a better surface; pick one).
- § Known patterns: the same table from the old SLOTS.md, but
  entries now cite `tests/slot_v2_baseline.rs` (from Phase 0) and
  the walk-through tables from SPEC.md.

The old "zone 1 / zone 2" sections are deleted, not marked
"historical."  Git history preserves them.

### 4b — Land the uniform-placement lint

New test in `tests/doc_hygiene.rs`:

```rust
#[test]
fn slot_allocator_has_no_size_or_shape_branches() {
    // Grep src/variables/slots.rs for forbidden patterns:
    //   - `size() >`, `size() <`, `v_size >`, `v_size <`
    //   - `Value::Block(_)` / `Value::Loop(_)` as a pattern in
    //     placement code
    //   - string occurrences of `orphan` (the concept shouldn't
    //     exist anymore)
    //   - `zone1`, `zone2`, `zone_1`, `zone_2`
    // Every hit either:
    //   (a) has an `// [uniform-placement-exempt] <rationale>` line
    //       immediately above or on the same line (reviewable in
    //       code review), or
    //   (b) is a regression — fail the test.
    ...
}
```

The exemption escape hatch exists because interval-graph-colouring
steps may legitimately read `size` to compute overlap.  The rule is
that every such read is *labelled* so a reviewer can verify it's
not dispatch.

### 4c — Update PROBLEMS.md

- P178's entry: add a status block marking it "Closed as part of
  initiative 04; see `plans/finished/04-slot-assignment-redesign/`."
- P185's entry: same.
- Quick-ref table: strike both entries, mark done.
- Cross-link SLOTS.md's new § Algorithm from P178 and P185's
  "Fix path" lines so future readers land on the actual spec.

### 4d — Update CAVEATS.md if warranted

If Phase 2's divergence log recorded any behaviour changes that
are observable to users (e.g. `OpReserveFrame` emission changed,
larger frames on some patterns, slower compile time), add them to
CAVEATS.md with a minimal reproducer and the rationale.

If the Phase 2 log shows zero observable changes, CAVEATS.md is
untouched.

### 4e — Close the initiative

- Update phase-status headers: every phase file gets its
  `**Status:**` line set to `done YYYY-MM-DD` with a one-line
  summary.
- Update `doc/claude/plans/04-slot-assignment-redesign/README.md`
  status line to `done YYYY-MM-DD — all five phases landed`.
- Move `doc/claude/plans/04-slot-assignment-redesign/` →
  `doc/claude/plans/finished/04-slot-assignment-redesign/`.
- Update `doc/claude/plans/README.md`:
  - Move initiative 04 from "Current initiatives" to "Finished
    initiatives" with a one-line closure note.

### 4f — Final gate

- `cargo fmt`
- `cargo clippy -- -D warnings`
- `cargo test --release`  (suite including the new doc_hygiene lint)
- `cargo test --release --test doc_hygiene slot_allocator_has_no_size_or_shape_branches`
- `cargo test --release --test native`
- `cargo test --release --test html_wasm`

All green.

## Non-goals for Phase 4

- No algorithmic changes to V2.  Any bug found here means
  Phase 3 was premature; file it as PXXX and pause Phase 4.
- No new initiative starts.  Phase 4 closes this one and does
  nothing else.

## Ground rule — no regressions

Phase 4 is docs + a lint + a move operation.  The lint may fail
on first land if V2 contains a forbidden pattern that slipped
through Phase 3's review — in that case, fix V2 first (and
update SPEC.md and the walk-through if the fix changed
placement), then land the lint.

## Deliverables

1. `doc/claude/SLOTS.md` — rewritten for V2.
2. `tests/doc_hygiene.rs::slot_allocator_has_no_size_or_shape_branches`
   — active, green, with no exemptions or with fully-labelled ones.
3. `doc/claude/PROBLEMS.md` — P178 and P185 marked fixed,
   cross-linked to the finished initiative.
4. `doc/claude/plans/finished/04-slot-assignment-redesign/` —
   all four phase files plus the README, each with a done-status
   header.
5. `doc/claude/plans/README.md` — initiative 04 in the Finished
   table.

## Done when

- The six gates in 4f pass on `main`.
- `doc/claude/plans/04-slot-assignment-redesign/` no longer
  exists; `doc/claude/plans/finished/04-slot-assignment-redesign/`
  does.
- Running `git grep 'zone1\|zone2\|place_orphaned_vars'` in the
  repo returns matches only in `plans/finished/` and in
  `CHANGELOG.md` / git history — nothing in live code or live
  docs.
- The lint from 4b passes on every post-Phase-4 commit (enforced
  by `make ci`).

## Open questions to flag if they surface

- Does CHANGELOG.md `[Unreleased]` need a section describing the
  slot-allocator change?  The answer is usually yes for
  user-observable changes; Phase 2's divergence log is the
  evidence.  If the log says "no observable changes," one short
  `### Compiler internals` entry is enough.
- Does the PLANNING.md backlog have any items predicated on
  specific V1 behaviour (e.g. "after P178 is fixed, do X")?  If
  so, those items move forward or are closed as no-ops.
