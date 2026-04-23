<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 2 — Build V2 in parallel, verify with invariants

**Status:** ready.  Blocked by Phase 1 (complete).

**Goal:** implement the V2 allocator as specified in SPEC.md, living
alongside V1.  Codegen still uses V1.  On every `cargo test` run,
V2 also runs and its output is checked against the **invariants**
I1–I6 from SPEC.md § 5a.  No user-visible behaviour changes.

## Why parallel (not replace-in-place)

Replacing V1 in a single commit has two problems:
- Any V2 bug surfaces as a test-suite blast, with no way to
  isolate the failing placement.
- The invariant-extension of `validate_slots` (step 2c) needs to
  be exercised on real code paths before it becomes the release
  gate — running it under V1 first catches any bug in the check
  itself.

Parallel lets V2 land incrementally: scaffolding, then the algorithm,
then the invariant harness, then green-the-suite — with V1 still
carrying prod traffic throughout.

## Inputs (from Phase 1)

- [`SPEC.md`](SPEC.md) — the 9-step algorithm, `SlotKind` axis, the
  correctness gate (invariants I1–I6), and the walk-through tables.
- [`walkthroughs.md`](walkthroughs.md) — three end-to-end traces and
  the all-fixture structural rationale.
- [`SPEC_GAPS.md`](SPEC_GAPS.md) — resolved open questions.

## Implementation surface

Three files are touched:

| File | Role |
|---|---|
| `src/variables/slots_v2.rs` (new) | V2 algorithm, implementing SPEC § 2 |
| `src/variables/validate.rs` (edit) | Extended `validate_slots` — I1 → I1+I2+I3+I4+I5+I6 |
| `tests/slot_v2_baseline.rs` (edit) | Replace `.slots(…)` layout locks with `.invariants_pass()` |

`slots_v2.rs` MUST NOT import from `src/variables/slots.rs` (V1).
Any shared helpers (interval construction, kind classification) go
into `src/variables/intervals.rs` or a new
`src/variables/placement_common.rs`.  Copy-paste between V1 and V2
is acceptable only if the identical V1 copy gets the `// Phase-3
delete` marker at the same commit.

## Steps

### 2a — Extend `validate_slots` with I2–I6

Before writing V2, land the invariant checker.  V1 must also pass
all six invariants on the current test corpus — this is the proof
that I2–I6 are stated correctly and that the diagnostics are
actionable.

- Add I2 (argument-isolation), I3 (frame-bounded), I4 (every-var-placed),
  I5 (kind-consistency-on-shared-slots), I6 (loop-iteration-safety)
  to `find_conflict` / `validate_slots`.  Each invariant emits a
  distinct diagnostic prefix (`[I2]`, `[I3]`, …) so failures are
  self-identifying.
- Extend the existing `mod tests` in `src/variables/slots.rs`
  (where seven I1 unit tests already live at line 582+) with one
  hand-crafted bad `Function` per new invariant (I2–I6),
  asserting `find_conflict` — or the new per-invariant check
  helper — reports the expected violation.  Integration-level
  tests (`tests/`) cannot access `find_conflict` because
  `debug_assertions` is off for the loft package under
  `cargo test`; unit tests are the correct home.
- Run full suite.  V1 must still be green — any invariant failure
  on V1 output is a real bug in V1 and gets filed as its own
  issue before Phase 2 proceeds.  (Expectation: V1 passes all six;
  the P185 fixture stays `#[ignore]`-d because its I1 violation
  was the reason it was ignored in the first place.)

### 2b — Scaffolding

- Add `src/variables/slots_v2.rs` with the module shell and a
  panic-stub entry point.
- Wire it into `src/variables/mod.rs` but do not call it from any
  production path.
- Land a `LOFT_SLOT_V2=validate` env-var handler in `Function` that,
  when set, calls `assign_slots_v2` after V1 runs and invokes the
  extended `validate_slots` on the V2 output.  When unset, V2 is
  inert — V1 drives codegen unchanged.
- Green `cargo test --release`.  This commit proves the plumbing
  doesn't break V1.

### 2c — Implementation

- Translate SPEC § 2's 9 numbered steps into Rust, staying close to
  the step-by-step structure so a reviewer can read the code with
  SPEC.md open alongside.  The SPEC's implementation sketch
  (§ "Implementation sketch") is the starting point.
- Populate `per_block_var_size` as a post-pass summary (SPEC § 3.1)
  so bytecode codegen's existing `block.var_size` reads keep working.
- Include the three end-to-end traces from `walkthroughs.md` §§ 1–3
  as unit tests in `slots_v2.rs`.  Each builds a `Function` with
  the documented intervals and asserts the output matches the
  walk-through's computed slots.
- Run `LOFT_SLOT_V2=validate cargo test --release`.  V2 must pass
  all six invariants on every compiled function — that is the gate.
  Slot numbers will differ from V1; that is expected and not a
  failure.

### 2d — Transition the fixture catalogue

The 24 `.slots(…)`-locked fixtures in `tests/slot_v2_baseline.rs`
were harvested from V1.  They document V1's behaviour; under V2
the slot numbers shift (but the invariants hold).  Replace the
locks with invariant assertions:

```rust
// Before:
#[test]
fn parent_refs_plus_child_loop_index() {
    code!("…").slots("…V1's layout…");
}

// After:
#[test]
fn parent_refs_plus_child_loop_index() {
    code!("…").invariants_pass();
}
```

`.invariants_pass()` is a new helper on `Test` that:
- Runs the test body through compile.
- Asserts runtime correctness (the existing `.result(…)` path).
- Asserts `validate_slots` panicked-nothing (i.e., I1–I6 all hold).

The per-fixture doc-comment rationale survives unchanged — it now
describes *which invariant* the fixture exercises (column "Invariant(s)"
from `walkthroughs.md` § 4).

`p185_late_local_after_inner_loop` gets un-ignored in the same
commit: V2 passes it by construction, where V1 failed I1.

### 2e — Iterate to green

- Run `LOFT_SLOT_V2=validate cargo test --release` and fix any V2
  bugs until the whole suite passes.
- For each invariant failure: the panic from `validate_slots`
  names the invariant and the offending pair.  Fix V2 (the spec
  is the target; V1 is not).
- Record every notable decision in a Phase 2 logbook
  `02b-phase2-findings.md`: invariant failures that surfaced real
  V2 bugs, any compute_intervals discrepancies, any codegen
  surprises from the `per_block_var_size` compatibility surface.

### 2f — Optimality report

Once all invariants pass, run the corpus with V2 driving (an
internal `LOFT_SLOT_V2=drive` switch) and record O1 (`v2.hwm` vs
`v1.hwm` per function).  Aggregate the numbers into
`02c-optimality-report.md`.  A V2 regression over V1 on any function
is investigated; the finding either (a) identifies a V1 optimisation
worth incorporating into V2, (b) identifies a compute_intervals bug,
or (c) is an acceptable tradeoff with rationale.

The optimality report gates Phase 3 only in aggregate: if V2's
total corpus `hwm` regresses by more than 5 %, Phase 2 does one
more iteration.  Individual regressions are acceptable if documented.

## Non-goals for Phase 2

- **No codegen changes.**  V1's output drives every
  `OpReserveFrame`, `OpFreeStack`, and `stack_pos` read in the
  bytecode interpreter and in native codegen.  The function-entry
  `OpReserveFrame(hwm)` optimisation (SPEC § 3.1 follow-up) is
  Phase 3 or later.
- **No V1 deletions.**  Phase 3 handles deletes.  Phase 2 keeps
  `src/variables/slots.rs` intact so the equivalence harness has
  two allocators to compare.
- **No SLOTS.md rewrite.**  Phase 4 does the documentation sweep.

## Ground rule — no regressions

`cargo test --release` without `LOFT_SLOT_V2` must stay green at
every commit in this phase.  Phase 2 lands in the order above
(2a → 2f); no commit is allowed to require `LOFT_SLOT_V2` to stay
green.

Per [plans/README.md](../../README.md)'s ground rule, a step that
surfaces a scope surprise (SPEC missed an IR shape, a compute_intervals
quirk, a validate_slots false positive) pauses Phase 2 and
updates SPEC.md / walkthroughs.md / SPEC_GAPS.md before the next
commit.

## Deliverables

1. `src/variables/validate.rs` — extended with I2–I6 and per-invariant
   diagnostics.
2. `src/variables/slots_v2.rs` — V2 implementation plus unit tests
   for the three end-to-end walk-throughs.
3. `src/variables/slots.rs::tests` — new unit tests, one per
   new invariant (I2–I6), asserting each check reports the
   expected violation.
4. `tests/slot_v2_baseline.rs` — `.slots(…)` replaced by
   `.invariants_pass()`; `p185_late_local_after_inner_loop`
   un-ignored.
5. `Test::invariants_pass()` helper in `tests/testing.rs`.
6. `02b-phase2-findings.md` — logbook of bugs found + decisions.
7. `02c-optimality-report.md` — per-function and aggregate `hwm`
   comparison.
8. `LOFT_SLOT_V2=validate cargo test --release` — green across
   the entire suite.

## Done when

- `LOFT_SLOT_V2=validate cargo test --release` passes the full
  suite with zero invariant violations.
- `cargo test --release` (without the env var) stays green at
  every commit in the phase.
- Every fixture in `tests/slot_v2_baseline.rs` uses
  `.invariants_pass()`; no `.slots(…)` lock survives.
- `p185_late_local_after_inner_loop` is un-ignored and passes.
- `02c-optimality-report.md` shows no corpus-aggregate `hwm`
  regression > 5 %.

## Open questions to flag if they surface

- Does `LOFT_SLOT_V2=validate` add enough test-time overhead to be
  annoying?  If so, add a `LOFT_SLOT_V2=compare_once` mode that
  runs the check once per function per session.
- Does V2 need access to bytecode-level information (exact opcode
  boundaries) that `Function` doesn't carry today?  If yes, Phase 2
  adds the plumbing; if no, `LocalInterval` (SPEC § 1) is
  self-contained and Phase 2 stays within the `src/variables/`
  boundary.
- **Property-test extension.**  A `proptest` / QuickCheck generator
  that emits well-typed loft programs and asserts I1–I6 on every
  compiled output is a stretch goal for Phase 2 — it catches
  cases the fixture catalogue doesn't cover.  Defer to Phase 3 if
  Phase 2 runs long; the fixtures + existing test suite already
  give high coverage.
