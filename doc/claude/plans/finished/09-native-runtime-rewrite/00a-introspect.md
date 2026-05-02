# Phase 00a — Introspection: after scaffold

**Status:** DONE (2026-05-02)

**Kind:** Review (no code; updates downstream plan files based on
what actually happened in phase 00)

**Trigger:** Phase 00 marked DONE.

**Time budget:** 1 day max.

## Why this phase exists

Phase 00 is the riskiest infrastructure step.  Hoisting every
Op-emission call site through `emit_op` while keeping byte-identical
output is genuinely hard.  Before committing to phases 01-04, we
need to know:

- How many iterations did each hoist step actually take?
- What `EmitCtx` helpers ended up necessary that weren't planned?
- Which call sites resisted the hoist and needed special treatment?
- Is the byte-identical-via-goldens approach holding up, or are we
  fighting the goldens more than the work?

Wrong answers here propagate downstream — phase 02 inherits all the
EmitCtx limitations and all the hoist patterns.

## Questions to answer

### Effort vs estimate
1. How many commits did phase 00 actually need vs the planned 7-9?
2. Which step needed the most iterations?  Why?
3. Time elapsed: budget was implicitly ~3-5 days; actual?

### Surface area surprises
4. List every helper added to `EmitCtx` during phase 00.  For each,
   one-line note: planned in advance / surfaced during step N.
5. Were any direct Op emissions in `dispatch.rs` impossible to
   route through `emit_op` and left as legacy direct calls?  List
   them and the reason.
6. Did the fn-ref dispatch arms in `emit.rs` need a special
   trait-impl shape that the plain `OpEmitter` doesn't cover?

### Golden corpus stability
7. How often did a step that was supposed to be byte-identical
   produce a diff that took non-trivial debugging to fix?
8. Did any test outside the corpus regress without the corpus
   catching it?  If yes — corpus is too narrow; flag for expansion.

### Hidden assumptions surfaced
9. Anything the plan assumed about the codebase that turned out
   different?

## Output

Update these files based on findings:

- **`02-param-adapter.md`**: if `EmitCtx::value_type(value) -> Type`
  helper turned out to require non-trivial plumbing, document
  it in step 2.1.
- **`03-parallel-emitter.md`**: if any of the planned helpers
  (`worker_fn`, `closure_shape`) need data from `EmitCtx` that
  isn't trivially accessible, update step 3.2's helper sketches.
- **`05-file.md`** through **`08-binary.md`**: if any custom
  emitter sketch references an `EmitCtx` helper that turned out
  hard, flag in the relevant phase.

If phase 00 went substantially over budget (>2× planned commits or
>2× planned days), add a top-level **risk update** to README.md and
recompute the phase budgets.

## Decision criteria

| Finding | Action |
|---|---|
| Phase 00 landed clean (≤1.3× budget, no surprise plumbing) | Continue to phase 01 as planned. |
| Phase 00 landed with 2-3 surprises that updated downstream phases | Continue to phase 01 with updated plans. |
| Phase 00 took >2× budget OR a hoist step proved structurally unsolvable | **Stop and revise.**  The simplification phases assume `emit_op` is the universal dispatch.  If that's not achievable, the whole plan needs rethink. |
| Goldens proved unmaintainable (frequent spurious diffs) | Replace golden corpus with a smaller invariant-check approach before phase 02. |

## Memory entries to save

If introspection surfaces a pattern that should persist beyond this
plan, save as a feedback or project memory.  Examples:

- "Plan 09 phase 00: hoisting `dispatch.rs` direct Op emissions
  through `emit_op` required N idiosyncratic emitters because of
  arg-shape variation.  Future codegen plans should expect similar
  surface-area divergence."
- "Plan 09 phase 00: golden corpus of M files proved sufficient /
  insufficient — adjust corpus selection rule for next plan."

## Output format

Append to this file under "Findings" — keep findings concise (1-2
sentences each, link to the actual diff if possible).

## Findings

00a fired late — after phases 00, 01, 03, 04, and 09 had all
shipped — so the introspection retroactively covers the whole
simplification cluster, not just the scaffold.

### 1. Effort vs estimate

| Phase | Planned commits | Actual | Spread |
|---|---|---|---|
| 00 (scaffold) | 7-9 | 6 (steps 0.2-0.9) + 4 follow-ons (smoke, eval gates, ci-fix tail) = 10 total | 2026-05-01 → 05-02 |
| 01 (ABI) | not specified | 4 | 2026-05-02 |
| 03 (parallel-for emitter) | not specified | 2 (emitter + EmitCtx polish) | 2026-05-02 |
| 04 (key-keyed) | not specified | 2 (impl + status) | 2026-05-02 |
| 09 (parallel runtime) | 5-6 | 1 + 1 doc + 2 ci | 2026-05-02 |

Phase 00's body landed in 6 commits — under the 7-9 budget.  The
+4 follow-ons (smoke test, evaluation gates, CI cleanup, fmt
followup) are infrastructure that the original plan didn't budget
for but turned out essential.  **Conclusion**: budget should
include explicit "post-step infrastructure" line items for future
plans of similar shape.

### 2. EmitCtx surface area

| Helper | Plan or surfaced | Notes |
|---|---|---|
| `w: &mut dyn Write` | planned | step 0.2 |
| `def_fn: &Definition` | planned | step 0.2 |
| `output: &mut Output<'b>` | planned | step 0.2 |
| `emit(v)` | **surfaced phase 03** | every emitter call site spelled out `ctx.output.output_code_inner(&mut *ctx.w, v)` — too verbose; helper added in `da46db1` |
| `emit_i32_slot(v)` | **surfaced phase 03** | same friction for i32-typed slots |

Helpers planned for later phases (`int_width_for`, `int_signed_for`,
generic-binding accessors) have NOT been added — phase 05's plan
calls for them but phase 05 hasn't started, and phase 05's scope is
itself misaligned (see § 5 below).

### 3. Direct Op emissions in dispatch.rs

| | Count | Δ |
|---|---|---|
| Pre-plan | 26 | — |
| After phase 03 (n_parallel_for) | 25 | −1 |
| After phase 04 (OpGetRecord, OpIterate) | 24 | −2 |
| Today | **24** | budget 26 (shrink-only) |

The wart-budget gate is doing its job: 2 arms retired without
regression, and `dispatch_op_arm_budget_not_exceeded` blocks
silent re-additions.

### 4. Fn-ref dispatch (step 0.7)

Did need a special trait shape — well, sort of.  The dispatch
itself uses the plain `OpEmitter`, but step 0.7 introduced a
codegen-only IR variant `Value::RawExpr` to thread synthesised
arg expressions through the walker.  That's the largest surface-
area surprise of phase 00.  **Mitigated** by
`no_unsanctioned_codegen_value_variants` gate (sanctioned list
`["RawExpr"]`); future codegen synthesis must use string-aware
companion entry points instead.

### 5. Golden corpus stability

Byte-identical contract held across all 6 hoist commits + every
subsequent simplification phase.  Zero unexplained drift.  Two
genuine surprises:

- **Forwarding-first recipe trap** (phase 00 runtime smoke test):
  initial 16-Op forwarding list included Ops in dispatch.rs's
  special-case match.  The byte-identical gate flagged the
  break immediately.  Resolution: pre-flight `grep -n '"OpYourOp" =>' src/generation/dispatch.rs`
  before adding to forwarding list.  **The recipe shipped to all
  bug-fix phase docs — phase 03/04 used it and avoided the same
  trap.**
- **Phase 09 trait-shape mismatch**: the phase-doc sketched
  `ParShape` with one associated type (`WorkerOut`) and uniform
  `Vec<WorkerOut>` worker output.  Implementation revealed text
  uses per-worker store slots and ref returns full Stores clones;
  trait grew a second associated type `Self::Batches` plus
  `store_results` method.  **Lesson**: phase-doc trait sketches
  are draft contracts, not specifications — bug-fix phases should
  expect to extend the trait shape on first contact.

Tests outside corpus that regressed without corpus catching:
**none observed.**

### 6. Hidden assumptions surfaced

1. **Phase 05's WRITE-side scope was wrong.**  Phase 05's
   diagnosis described `f += val` template hard-coding `i32` as
   the P200 bug.  The actual failing sites in 20-binary today are
   READ-side comparison emission (E0308 between narrowed `as u8/u16/u32`
   block-tail and `_i64` literal RHS).  Discovery happened during
   phase 05 step 5.0's stub registration when probing `--native-emit`
   output for the test.

   **Lesson**: bug-fix phases need an explicit "Actual error
   survey" pre-step before writing implementation steps — the plan
   was authored based on the symptom description, not against
   today's `--native-emit` output.  Phase 05 needs scope rewrite.

2. **Phase 02 is no longer a P200 prerequisite.**  Phase 02 splits
   `narrow_int_cast`'s param-narrowing role (role #2).  Phase 05's
   actual bug is the block-tail role (role #1).  Phase 02 doesn't
   dissolve the P200 blocker anymore; it's a pure simplification
   without bug-fix dependency.  **Decision**: demote phase 02 to
   optional, defer until phase 05 lands and we know whether the
   block-tail fix needs a phase 02b.

3. **Pre-existing CI breakage accreted across simplification phases.**
   Phase 01 step 1.7 introduced a hand-aligned `RuntimeFn` table
   that `cargo fmt --check` rejects.  Phases 00/01/03/04 added
   clippy-pedantic violations (`map().unwrap_or()`, `borrow as raw
   pointer`, lifetime elision, `let _stub = …` patterns, unused
   `Write` import).  Total: 1 fmt + 5 clippy errors — undetected
   for ~5 phases.  Closed in commit `f4d288a` + `97d17cc` (today).

   **Lesson**: feedback memory `feedback_ci_gate.md` says "run
   fmt + clippy -D warnings + no-default-features check before
   push."  Should extend to "before each commit" for phases that
   touch hot paths — drift accumulates faster than push cadence.
   Add to fast gate? (rejected — fast gate is for byte-identical;
   `make ci` is the heavyweight check).

### 7. What worked beyond expectation

- **Wart-budget gates** (`dispatch_op_arm_budget_not_exceeded`,
  `no_unsanctioned_codegen_value_variants`).  Both fired as
  intended; no false positives; structural test additions in
  phase 03/04 followed naturally.  Pattern transferable to other
  plans.

- **Forwarding-first recipe**.  Phase 00 surfaced it as an
  emergency response to the dispatch-match conflict; phase 03/04
  used it as a planned pre-flight.  Documented in phase 00 doc.
  Pattern transferable.

- **Byte-identical baseline + fast gate**.  4-second turnaround
  meant byte-identical violations were caught during the work
  step, not in CI.  Per-phase progress markers (legacy fn count,
  dispatch arm count, custom emitter count) gave clear signal of
  plan-progress without hand-counting.

### Decision

Per the decision criteria table:

> "Phase 00 landed with 2-3 surprises that updated downstream
> phases" → **Continue with updated plans.**

Three surprises that surface here update downstream phases:
1. `Value::RawExpr` wart-budget gate is in place; no further
   action needed but bug-fix phases must avoid extending it.
2. Phase 02's prerequisite role is demoted; documented in phase
   02's status block update (this commit).
3. Phase 05's scope misalignment is documented in phase 05's
   status block (commit `a2218be`); plan rewrite required before
   implementation.

**Continue to phase 06** (P202 close — ready, doc updated for
ParShape pattern in commit `a2218be`).  Phase 07 (P205, Outcome B
confirmed) can run in parallel.  Phase 05 needs rewrite before
attempt.

### Memory entries to save

- **`feedback_forwarding_first_recipe.md`** — the pre-flight
  pattern that caught two distinct trap classes.  Durable beyond
  plan-09.
- **`feedback_ci_gate_per_commit.md`** — extend the existing CI
  gate memory to explicitly call out per-commit checking for
  hot-path edits, since this branch accreted 1 fmt + 5 clippy
  errors over 5 phases.
- **`project_plan_09_simplification_cluster.md`** — record what
  the simplification cluster delivered: 5 phases, 1 P-issue
  closed (P203), wart-budget gates, byte-identical contract held.
  Surfaces decisions for bug-fix phases (phase 02 demoted, phase
  05 needs rewrite, phase 06 ready).
