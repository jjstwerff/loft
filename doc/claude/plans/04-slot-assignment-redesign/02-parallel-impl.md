<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 2 — Build V2 in parallel, verify on every test

**Status:** open.  Blocked by Phase 1.

**Goal:** implement the V2 allocator as specified in SPEC.md, living
alongside V1.  Codegen still uses V1.  On every `cargo test` run,
V2 also runs and its output is checked against the correctness gates
from SPEC.md § 5.  No user-visible behaviour changes.

## Why parallel (not replace-in-place)

Replacing V1 in a single commit has two problems:
- Any V2 bug surfaces as a test-suite blast, with no way to
  compare the failing placement against V1's output.
- The "hard constraint" audit in Phase 4 needs to compare V1 vs
  V2 on the same IR, which only works while both exist.

Parallel also means V2 lands incrementally: passing 100 tests, then
500, then the whole suite, with V1 still carrying prod traffic.

## Inputs (from Phase 1)

- `SPEC.md` — the algorithm, the three design decisions, the
  correctness gates, the walk-through tables.

## Implementation surface

A new module: `src/variables/slots_v2.rs`.

Exports: `fn assign_slots_v2(f: &mut Function, data: &Data) -> Result
<V2Report, V2Error>`.  Same signature shape as V1's `assign_slots`;
returns a struct rather than mutating in place so the equivalence
check in Phase 2b can compare without a copy.

The module MUST NOT import from `src/variables/slots.rs`.  Any
shared helpers (interval construction, alignment lookup) get
extracted into `src/variables/intervals.rs` (already present) or
a new `src/variables/placement_common.rs` with explicitly named,
reviewable exports.  Copy-paste between slots.rs and slots_v2.rs
is acceptable only if the identical code is deleted when V1 is
removed in Phase 3 — add a `// Phase-3 delete` marker at the site.

## Steps

### 2a — Scaffolding

- Add `src/variables/slots_v2.rs` with the module shell and a
  panic-stub entry point.
- Wire it into `src/variables/mod.rs` but do not call it.
- Land a no-op `LOFT_SLOT_V2=compare` env-var handler in `Function`
  that records "V2 ran but produced no output" to prove plumbing
  is live.
- Green `cargo test --release`.  This commit proves the scaffolding
  doesn't break V1.

### 2b — Implementation

- Translate SPEC.md § 2's numbered algorithm into Rust, staying as
  close to the step-by-step structure as possible so a reviewer
  can read the code with SPEC.md open alongside.
- Produce `V2Report { assignments: Vec<(VarNr, u16)>, hwm: u16,
  diagnostics: Vec<String> }`.
- Include the SPEC § 4 walk-through tables as unit tests in
  `slots_v2.rs` — each fixture becomes a `#[test]` that builds a
  `Function` with the documented intervals and asserts the output
  matches the walk-through row exactly.

### 2c — Equivalence harness

- Extend `src/variables/validate.rs` (debug-only) with
  `validate_v2_report(f, report)` that checks every gate from
  SPEC.md § 5:
  - overlap-free,
  - `report.hwm ≤ f.var_size` (V1's hwm),
  - every var in the function has exactly one assignment in the
    report (no orphans, no duplicates).
- Plumb it behind `LOFT_SLOT_V2=validate`: when set, every
  `assign_slots` call in the test harness also runs
  `assign_slots_v2` and invokes `validate_v2_report`.  Failures
  panic with the function name, the V1 assignment, the V2 report,
  and the first diverging or violating interval.
- Land a CI-friendly mode: `LOFT_SLOT_V2=validate cargo test
  --release` runs the whole suite with the equivalence check on.
  This mode is opt-in, not default (V2 might still be incomplete;
  we don't want CI red on a known gap).

### 2d — Iterate to green

- Run `LOFT_SLOT_V2=validate cargo test --release` and fix V2 bugs
  until the whole suite passes with the equivalence check on.
- For each bug: if V2 diverges from V1 on a fixture that locked
  observed placement, decide (per SPEC.md § 4) whether to update
  V2 or rewrite the fixture.  Log every decision in a Phase 2
  logbook in `02b-phase2-divergences.md`.
- Per [plans/README.md](../README.md)'s ground rule: V2 goes green
  before any V1 code is deleted.  A partial V2 is fine for
  commits, as long as `cargo test --release` without `LOFT_SLOT_V2`
  stays green (V1 is the source of truth during Phase 2).

## Non-goals for Phase 2

- No codegen changes.  V1's output drives every `OpReserveFrame`,
  `OpFreeStack`, and `stack_pos` read.
- No V1 deletions, even of dead helpers.  Phase 3 handles deletes.
- No SLOTS.md rewrite (Phase 4's job).

## Ground rule — no regressions

`cargo test --release` without `LOFT_SLOT_V2` must stay green at
every commit in this phase.  The phase lands incrementally:
scaffolding first (trivially green), implementation second, harness
third, debug-iteration fourth.  No commit is allowed to require
`LOFT_SLOT_V2` to stay green.

Per the `plans/README.md` ground rule, a step that surfaces a
scope surprise (e.g. SPEC.md missed an IR shape, or a V1 quirk
turns out to be load-bearing) pauses Phase 2 and updates SPEC.md
before the next commit.

## Deliverables

1. `src/variables/slots_v2.rs` — V2 implementation, entry point
   and walk-through tests.
2. `src/variables/validate.rs` — `validate_v2_report` addition.
3. `Function::assign_slots_v2_compare` (or similar) — the plumbing
   that lets `LOFT_SLOT_V2=validate` cross-check V1 and V2.
4. `02b-phase2-divergences.md` — log of every V2-vs-V1 divergence
   found during iteration and how each was resolved.
5. `LOFT_SLOT_V2=validate cargo test --release` — all passes.

## Done when

- `LOFT_SLOT_V2=validate cargo test --release` passes the full
  suite (zero panics, zero overlap violations, zero hwm regressions).
- Every walk-through from SPEC.md § 4 has a corresponding unit
  test in `slots_v2.rs` that runs green in `cargo test --release`
  (not behind the env var).
- `02b-phase2-divergences.md` has an entry for every fixture where
  V2 chose a different placement than V1, with a written rationale
  and a status of "accept" or "tighten spec."

## Open questions to flag if they surface

- Does `LOFT_SLOT_V2=validate` add enough test-time overhead to be
  annoying?  If so, Phase 2 adds a `LOFT_SLOT_V2=compare_once` mode
  that runs the check once per function per session.
- Does V2 need access to bytecode-level information (e.g. exact
  opcode boundaries) that `Function` doesn't carry today?  If yes,
  Phase 2 adds the plumbing; if no, the `LocalInterval` struct from
  SPEC.md § 1 is self-contained and Phase 2 stays within the
  `src/variables/` boundary.
