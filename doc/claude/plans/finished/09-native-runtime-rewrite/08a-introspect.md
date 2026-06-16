# Phase 08a — Introspection: retrospective

**Status:** SUPERSEDED (2026-05-02) — folded into phase 10
step 10.6.  The original 08a was sequenced as a closer for
phases 06/07/08; @PLAN09's tail consolidation absorbed it
into phase 10's final retrospective step (which fires after
@PLAN11 closes @P204 so the retrospective covers the full arc
including the sibling-plan handover).

**Kind:** Retrospective (no code; produces lasting memory entries
and informs future plans)

**Original trigger** (no longer relevant): Final landed phase
among 06/07/08.  May fire after fewer phases if 05a decided to
stop early.

**Active trigger**: phase 10 step 10.6 — fires after @PLAN11
closes @P204.  See `10-final-closure.md` § Step 10.6 for the
populate-at-retrospective outline.

**Time budget:** 1 day max.

## Why this phase exists

Plan 09 was a substantial bet on the per-Op emitter abstraction
plus the diagnostic-gate-driven bug-fix pattern.  The retrospective
captures what worked and what didn't, in durable form, so the next
codegen plan benefits.

This is the only phase whose primary output is **memory entries**
rather than code or doc updates.

## Questions to answer

### The bet
1. Did the per-Op emitter pattern actually pay off?  Compare:
   - LOC growth: net delta across all phases
   - Bug-fix velocity: phases 05/06/07/08 vs. estimates
   - Mental model: is the codegen easier to reason about now?
2. Were the simplification phases (01-04) prerequisites for the
   bug fixes (05-08), as claimed?  Or could the bug fixes have
   landed without them?

### The diagnostic gates
3. For each bug-fix phase, did the gate fire (catch a wrong
   assumption) or pass clean (assumption was right)?
4. How many gates rerouted a fix vs. how many proceeded as
   planned?

### The introspection phases
5. Did 00a / 02a / 05a actually change downstream plans, or were
   they decorative?
6. Did any introspection cause a stop/pivot decision, or did all
   phases continue as planned?

### Surface area
7. Total `EmitCtx` helpers added: list them.  Which were trivial
   accessors vs. which required plumbing?
8. Is `src/generation/ops/` a clean extension surface for future
   custom emitters, or accreted-complex?

### What didn't get done
9. Which phases didn't land?  Why?
10. Of the four P-issues (@P200, @P202, @P203, @P205), how many
    closed?  How many rerouted to other plans?

### Lessons for next plan
11. What pattern worked that we'd repeat?
12. What didn't work that we'd avoid?
13. Which estimates were most wrong (high vs low)?

## Output

### Memory entries (primary output)

Write at least 3-5 durable memory entries.  Examples:

- **Per-Op emitter pattern (project memory)**: "Plan 09 used a
  per-Op `OpEmitter` trait + registry on top of `#rust` template
  substitution.  Worked / didn't.  Use / avoid when next codegen
  refactor is needed."

- **Diagnostic gates (feedback memory)**: "Plan 09's gate pattern
  (run a test that confirms the fix's prerequisite holds before
  writing the fix) caught N false-starts.  Apply when a prior fix
  attempt failed mysteriously, the codebase is partially
  understood, or the fix touches shared mutable state."

- **Byte-identical baselines (feedback memory)**: "Plan 09's
  golden-corpus diff testing on every refactor commit caught X
  silent regressions.  Worth the ~50KB committed bytes for codegen
  work; overkill for application-level refactors."

- **Introspection cadence (feedback memory)**: "Plan 09 inserted
  introspection phases after high-risk steps.  Caused N
  pivots/stops.  Apply to plans with >5 phases or where each
  phase is non-trivial."

- **EmitCtx accretion (project memory)**: "Plan 09 ended with N
  helpers on `EmitCtx`.  Future codegen extensions should expect
  plumbing complexity proportional to the IR-context depth they
  need."

### Updates to other plans / docs

- **`PROBLEMS.md`**: confirm closure status for each P-issue;
  add fix-path back-references.
- **`NATIVE.md`**: document the emitter dispatch pattern as a
  permanent feature; reference where custom emitters live.
- **`CHANGELOG_TECHNICAL.md`**: summary entry for plan 09.

### If plan ended early

If phases 06/07/08 didn't all land, document which P-issues remain
open and the fix-path each would take.  Open follow-up tracking
entries (PROBLEMS.md or a new plan) so the work isn't lost.

## Decision criteria

This phase doesn't decide anything for plan 09 itself — that's
done.  It decides:

| Finding | Action for future |
|---|---|
| Pattern broadly successful | Apply per-Op emitters in future codegen work; mark plan 09 as a reference template. |
| Pattern partially successful | Save lessons to memory; next plan can opt in selectively. |
| Pattern unsuccessful (most P-issues didn't close, complexity grew) | Save the lessons clearly; future codegen plans should pick a different abstraction. |

## Findings

Populated 2026-05-02 after @PLAN11 closed @P204 — the last
sibling-plan dependency for full PR-readiness.

### 1. The bet — per-Op emitter pattern

**Did it pay off?** Yes, decisively.

| Metric | Outcome |
|---|---|
| P-issues closed | 4/4 in @PLAN09 scope (@P200, @P202, @P203, @P205) + 1 in sibling @PLAN11 (@P204) |
| LOC growth | ~210 lines for `ParShape` + 3 impls; ~140 lines for queue runtime + emitters; ~50 lines for @P205 emit-site fix; ~60 lines for `IntCompareEmitter`; 3 lines for @P204.  Net additions ~600 lines code; ~95 lines retired (parallel-for runtime consolidation). |
| Bug-fix velocity | All 5 P-issues closed within 2 days of focused work (single-session-per-phase pace).  Faster than the 2-3 week original estimate. |
| Mental model | Codegen now has a single dispatch entry (`emit_op`) with custom emitters in `src/generation/ops/`.  New Op contributors can add an emitter file without touching `dispatch.rs`'s special-case match.  Wart-budget gate prevents accretion. |

**Were simplifications (01-04) actual prerequisites for the bug
fixes?** Mixed.

- Phase 01 (ABI consolidation): YES — every subsequent phase
  used `Abi::Cell`.  Without this, the queue + buf-rename
  emitters in phase 06 would have needed `Abi::LegacyStores`
  handling.
- Phase 02 (param adapter): NO — demoted by 00a.  @P200's actual
  fix didn't need the param-narrowing split.  Phase 02 is
  preserved in @PLN80 phase 05 as a pure simplification.
- Phase 03 (parallel-for emitter): YES — phase 06 mirrored the
  `ParallelForEmitter` shape for queue variants.  Without
  phase 03, phase 06 would have duplicated 95 lines of dispatch
  special-case per queue Op.
- Phase 04 (key-keyed Op emitter): partially — the emitter
  pattern was useful, but key_ops doesn't directly enable a
  bug-fix phase.  Standalone simplification value.
- Phase 09 (parallel runtime consolidation): partially — phase
  06's queue runtime didn't reuse `ParShape` (chose flat;
  documented in `feedback_phase_doc_trait_drafts.md`).  Phase
  09's emitter-side `closure_shape` / `helper_name` factoring
  WAS reused in phase 06 indirectly.

### 2. Diagnostic gates

For each bug-fix phase, did the gate fire?

| Phase | Diagnostic gate | Outcome |
|---|---|---|
| 06 (@P202) | Forwarding-first recipe pre-flight | Cleanly identified safe path (no special-case match conflicts).  Fired as designed. |
| 06 (@P202) | Reachability check (`collect_calls`) | NOT a planned gate — surfaced as a runtime test failure during validation.  ~30 min wasted before recognising the pattern. |
| 07 (@P205) | Skip-removal probe (Outcome A vs B) | Cleanly confirmed Outcome B; ~30 min in disposable worktree.  Fired as designed. |
| 10.3 (@P200) | Actual-error survey (per `feedback_actual_error_survey.md`) | Found read-side, not write-side.  Fired as designed.  Phase 05 plan rewrite saved hours of wrong implementation. |
| 11 (@P204) | Actual-error survey | Confirmed all 3 failing tests share the same shape.  Fired as designed. |

**Total**: 4/5 planned gates fired cleanly.  1 gap (the
reachability gap in phase 06) caught by tests, not by a
pre-flight gate.  Pattern for future: when an emitter
synthesises calls to user fns BY NAME, add a reachability
check to its acceptance criteria.

### 3. Introspection phases

| Phase | Fired? | Effect |
|---|---|---|
| 00a | YES (after phase 09 + 04 + 06 + 07 + 09 shipped) | Demoted phase 02; revised phase 05 trigger; surfaced 3 lessons → 3 memory entries |
| 02a | NO | Phase 02 superseded; trigger never fired |
| 05a | YES (folded into phase 10 step 10.1, fired post-06 + post-07) | Validated framework; estimate-doubling rule surfaced |
| 08a | THIS DOCUMENT | Final retrospective |

**Took 2/4 introspection phases.**  The other 2 (02a, 08a-as-
originally-scheduled) were superseded by scope changes.  Pattern:
introspection phases need their triggers tied to OUTCOMES not
phase numbers — "fire after first framework bug fix" worked
better than "fire after phase 05 DONE."

### 4. Surface area

`EmitCtx` final state:

| Helper | Plan or surfaced |
|---|---|
| `w: &mut dyn Write` | Planned (phase 00 step 0.2) |
| `def_fn: &Definition` | Planned (phase 00 step 0.2) |
| `output: &mut Output<'b>` | Planned (phase 00 step 0.2) |
| `emit(v)` | Surfaced phase 03 polish |
| `emit_i32_slot(v)` | Surfaced phase 03 polish |

5 helpers total; 3 planned + 2 surfaced.  Plan-09's surface area
ended LESS bloated than feared.  No `int_width_for` /
`int_signed_for` were needed (those were phase 05's original
plan; the actual @P200 fix didn't need them).

`src/generation/ops/` final state:

- `mod.rs` — registry + `EmitCtx` + dispatch
- `default.rs` — fall-through emitter
- `forwarding_smoke.rs` — runtime smoke test (slated for
  retirement in @PLN80 phase 02)
- `parallel.rs` — for-par + queue + buf-rename emitters
- `key_ops.rs` — OpGetRecord + OpIterate
- `int_compare.rs` — OpEqInt / OpNeInt / OpLtInt / OpLeInt

6 files; 5 production emitter modules + 1 dispatch infrastructure
file.  Clean extension surface.

### 5. What didn't get done

- Phase 02 (param adapter) — DEMOTED then SUPERSEDED → @PLN80
  phase 05.  Preserved as a follow-up.
- Phase 02a — superseded with phase 02.
- Phase 05 (file emitter, original write-side scope) — folded
  into phase 10 step 10.3 (read-side fix).  Original write-side
  emitter never built; turned out unnecessary.
- Phase 08 (binary read emitter) — superseded by phase 10 step
  10.3 (one fix closed both read + write).
- Phase 08a — folded into this retrospective.

5 phases consolidated/superseded.  Net 9 phases shipped (00,
00a, 01, 03, 04, 06, 07, 09, 10) of the original 14.

### 6. Lessons for next codegen plan

**What worked, repeat**:

1. **Wart-budget gates** — `dispatch_op_arm_budget_not_exceeded`
   + `no_unsanctioned_codegen_value_variants` held throughout.
   Pattern: add a structural test that pins the current state
   (count, allowed list); shrink-only.
2. **Forwarding-first recipe** — pre-flight pattern caught
   dispatch-path traps in phase 00 + 03 + 04.  Codified in
   `feedback_forwarding_first_recipe.md`.
3. **Actual-error survey** — `feedback_actual_error_survey.md`.
   Phase 05's rewrite + phase 11's same-day close are the
   case studies.  ANY bug-fix plan should run the survey
   BEFORE writing implementation steps.
4. **Byte-identical baseline** — `scripts/p09_fast_gate.sh`
   ran every step in <5s; caught silent emission divergence
   immediately.
5. **Memory entries** — 7 saved during @PLAN09 + @PLAN11.
   Codify patterns once; reference from future plans.
6. **Tail-phase consolidation** — phase 10 absorbed 4 planned
   phases (05/05a/08/08a).  When a tail surface is small,
   consolidating saves ceremony without losing discipline.

**What didn't work, avoid**:

1. **Plan-doc trait sketches as specifications** — phase 09's
   `ParShape` sketch was too narrow; phase 06's queue trait
   was wrong-shape entirely (chose flat).  Codified in
   `feedback_phase_doc_trait_drafts.md`.
2. **Estimating emitter sizes by sketch length** — actuals
   ran 1.5-3× sketch.  Codify estimate-doubling for next
   codegen plan.
3. **"Code that compiled but never executed"** — `detect_ref_tail_capture`
   shipped to fix 87-store-leaks but never fired in production
   (Span-miss).  Plan-12 phase 01's walker audit catches the
   class.

### 7. Decision per the criteria table

| Finding | Match |
|---|---|
| Pattern broadly successful | ✓ — apply per-Op emitters in future codegen work; @PLAN09 is a reference template |

Plan-09 is a **reference template** for future codegen
refactors.  The combination of:
- `OpEmitter` trait + registry
- Wart-budget gates
- Byte-identical baselines + fast gate
- Forwarding-first recipe
- Actual-error survey
- Tail-phase consolidation
- Memory-entry codification

...delivered 5 P-issues closed across 2 days with zero
regressions.  Future codegen plans should opt into this
template.

## Memory entries written

This retrospective references 7 memory entries saved during
@PLAN09 + @PLAN11:

1. `feedback_forwarding_first_recipe.md` — pre-flight pattern.
2. `feedback_phase_doc_trait_drafts.md` — sketches not specs.
3. `feedback_actual_error_survey.md` — survey before
   implementing.
4. `feedback_branch_after_pr_only.md` — branch hygiene.
5. `feedback_no_expect_fail_on_pr_bugs.md` — no @EXPECT_FAIL on
   real bugs.
6. `feedback_zero_regression_tolerance.md` — zero tolerance.
7. (extended) `feedback_ci_gate.md` — per-commit CI guidance.

No new memory entries written by this retrospective itself —
the patterns were captured as they emerged.  This retrospective
references them; doesn't duplicate them.
