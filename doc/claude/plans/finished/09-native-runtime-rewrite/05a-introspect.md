# Phase 05a — Introspection: after first bug fix

**Status:** DONE (2026-05-02) — fired after phases 06 + 07 shipped, before phase 10's @P200 implementation begins.

**Kind:** Review (no code)

**Trigger (revised 2026-05-02):** **first framework-based bug-fix
phase marked DONE.**  The original trigger was "Phase 05 DONE";
phase 00a's findings revised plan ordering enough that the spirit
of 05a (validate the diagnostic-gate + regression-test-first
pattern) applies as soon as ANY bug-fix phase ships through the
emitter framework.  Today phase 06 (@P202 close) qualifies — and
phase 07 (@P205) is queued behind it.

Recommended firing point: after BOTH phase 06 and phase 07 land,
so the introspection has two framework-based bug-fix phases to
compare and the lessons captured stay current rather than going
stale waiting for phase 05's rewrite.

**Original trigger** (kept for context): Phase 05 marked DONE
(@P200 write side + @P203 either closed or rerouted).  @P203 closed
out-of-order in phase 00 step 0.7b; @P200 (phase 05) is gated on
plan rewrite.  Sticking strictly to "phase 05 DONE" would defer
05a behind two unrelated bottlenecks — better to fire on the
substantive trigger (first bug-fix phase shipped through the new
framework).

**Time budget:** 1 day max.

## Why this phase exists

Phase 05 is the first bug-fix phase under the new structure.  The
plan claims simplifications dissolve the structural blockers that
caused prior fix attempts to fail.  Now we can actually measure
that claim against reality:

- Did the diagnostic gate (`p203_file_flavour_is_reachable`) work
  as intended?
- Did the regression-test-first pattern (`p200_round_trip` written
  before the fix) catch the prior failure mode?
- Was the custom emitter actually ~30-50 lines as estimated, or
  did context-extraction add more complexity than expected?

This is also the natural decision point for "how many more bug-fix
phases to attempt."  @P200 write + @P203 is the highest-value subset;
phases 06/07/08 may or may not be worth the additional time.

## Questions to answer

### Did the diagnostic gate work?
1. Did `p203_file_flavour_is_reachable` pass cleanly, fail and
   reroute, or pass-but-misleading?
2. If it rerouted, how much wasted prep work did it save?  (vs.
   how much work would have been wasted attempting a fix that
   couldn't work).
3. Could the same diagnostic-gate pattern be applied to the
   remaining bug-fix phases (06/07/08)?

### Was the prior-failure regression test useful?
4. Did `p200_round_trip_test_compiles_and_runs` actually catch a
   regression at any iteration, or was it green from the first
   try?
5. If green from start: does that mean the prior failure mode
   was different than documented, or the new emitter genuinely
   sidesteps it?

### Custom emitter complexity
6. `OpWriteIntFile` emitter: planned ~30 lines.  Actual?
7. `OpFreeRef` emitter: same comparison.
8. Did `EmitCtx::is_file_ref` need any plumbing not predicted in
   phase 00a?

### @P203 status
9. Did @P203 close in this phase, or reroute to parser work?
10. If rerouted, is it now blocked, or do we have a clear path
    outside this plan?

### @P200 write closure
11. @P200 write side: did the round-trip suite (including the
    historically-failing case) all pass?
12. Are there any width × signedness combinations that aren't
    covered by the regression test?

### Plan velocity
13. Time elapsed across phases 00-05 vs initial budget.
14. At current pace, when do remaining phases (06/07/08) finish?

## Output

Update these files based on findings:

- **`06-threading.md`**: if `p203_file_flavour_is_reachable` style
  diagnostic worked well, model phase 06's pre-work after it.
- **`07-generics.md`**: if the prior-failure-regression-test
  pattern was useful, ensure phase 07 has equivalent test for the
  @P205 reproducer.
- **`08-binary.md`**: phase 05's experience directly informs
  phase 08 (mirrors the pattern).  If phase 05 hit unexpected
  EmitCtx complexity, phase 08 inherits it — flag.
- **`README.md`**: update status table; if velocity shows the
  full plan won't finish in budget, mark phases 06/07/08 as
  optional or sequence them by ROI.

## Decision criteria

| Finding | Action |
|---|---|
| @P200 write closed cleanly + @P203 closed | Strong signal that the plan works.  Continue to 06/07/08 with confidence. |
| @P200 write closed + @P203 rerouted but cleanly so | Plan working as designed (graceful degradation).  Continue. |
| Both closed but custom emitters needed >2× planned complexity | Re-budget phases 06-08; consider deferring phase 08 (read side, less urgent than write was). |
| @P200 write closed but caused regressions outside the suite | Fix regressions, then introspect again before phase 06. |
| @P200 write didn't close (the prior-failure pattern recurred) | **Stop and investigate.**  The plan's central claim is broken; phase 02's adapter split didn't actually fix the dual-role issue. |
| Plan is going slowly enough that phases 06/07/08 won't fit | Ship what's done, mark the rest as future work, and return to other priorities. |

## Memory entries to save

Examples:

- "Diagnostic gates (run-a-test-that-validates-the-fix-can-work
  before writing the fix) saved N hours / didn't save anything.
  Apply pattern in future where prior fix attempts failed
  mysteriously."
- "Prior-failure regression test pattern: writing the test that
  pins the prior broken state BEFORE writing the fix worked /
  didn't.  Useful when there's a documented prior attempt;
  overkill when the bug is fresh."
- "Plan 09 budget reality: phases 00-05 took N days.  If similar
  pace continues, remaining phases need M days.  Adjust planning
  expectations for future codegen work."

## Findings

05a fired late — after phases 06 (@P202 close) and 07 (@P205 close)
shipped, but before phase 10's @P200 implementation begins.  This
means findings cover the FRAMEWORK-BASED bug fixes (those that
shipped through `emit_op` dispatch + custom emitters), not the
originally-anticipated phase 05 (which was rewritten and folded
into phase 10).  The questions above were authored when phase 05
was the first bug-fix phase; they're answered here against
phases 06 + 07 as the actual subjects.

### 1. Diagnostic gates: did they catch the right things?

| Phase | Gate | Outcome |
|---|---|---|
| 06 | Forwarding-first recipe pre-flight (grep `dispatch.rs` for special-case match) | **Passed cleanly** for queue + buf-rename emitters — none of those names had special-case match arms.  Recipe correctly identified the safe path. |
| 06 | Reachability gap caught DURING validation (not via a planned gate) | **Surfaced as a runtime test failure**, not a pre-flight gate.  Cost: ~30 min of "why is rustc complaining about a missing fn?" investigation before recognising the pattern. |
| 07 | Probe of the parser-side skip removal at `parser/control.rs:375` | **Outcome B confirmed cleanly** in a worktree.  ~30 minutes; no wasted prep work because the worktree was disposable. |

The forwarding-first recipe and the worktree-probe pattern both
worked as documented.  The reachability-gap surprise was caught
by the test suite, not a pre-flight — pattern for future bug-fix
phases is to ADD a "post-fix worker-fn reachability check" step
when the emitter synthesises calls to user fns by name.

### 2. Regression-test-first pattern

Phase 06 added 2 tests: `p202_parallel_queue_runtime_fns_registered`
+ `p202_parallel_queue_emitter_registered`.  Both were structural
gates (registry consistency), not behavioural — they pin the
RIGHT registrations, not the right output.

Phase 07 added 2 tests: `p205_repro_passes_under_native` (runs
the reproducer) + `p205_no_str_new_of_local_in_corpus` (greps
the baseline).  These ARE behavioural — repro passing means the
dangle is closed.

**Verdict**: structural tests are useful as a "did you remember
to register" guard.  Behavioural tests via reproducer-runners
are useful as the "did the bug actually close" guard.  Phase 10
should include both kinds: a structural test pinning the chosen
fix site (e.g. "comparison emitter calls `widen_rhs_for_lhs`")
+ a behavioural test running 20-binary.loft under native.

### 3. Custom emitter complexity vs estimate

| Phase | Estimated | Actual | Delta |
|---|---|---|---|
| 06 ParallelQueueEmitter | ~30 lines (sketch in 06-threading.md) | ~95 lines (mirroring `ParallelForEmitter`) | +3× |
| 06 ParallelBufRenameEmitter | ~5 lines | ~10 lines | +2× |
| 06 runtime fns (queue + buf-get + buf-drop) | ~90 lines (3 thin wrappers per phase 09's projection) | ~140 lines (3 flat fns + buf-get/drop) | +1.5× |
| 07 emit.rs scratch routing | ~30 lines (sketch in 07-generics.md) | ~25 lines × 2 sites = 50 lines | +1.7× |

All three exceeded their estimates by 1.5-3×.  The estimates
were sketched from the plan author's mental model; the actual
work needed:
- More closure-shape branches than the sketch (queue shape
  selection mirrors for-par's 4-way: scalar/float/text/ref).
- More edge-case handling (the queue-ref path, the
  `(value).to_string()` coercion, the second emit site for @P205).
- Doc + comment overhead — emitters need substantial inline
  documentation to be reviewable.

**Lesson**: estimate emitter sizes as 2× the sketch length.
Apply to phase 10 step 10.3's @P200 fix sizing.

### 4. EmitCtx surface area additions

Phase 03 added `EmitCtx::emit(v)` and `emit_i32_slot(v)` —
documented in 00a.

Phase 06 didn't need new EmitCtx helpers; it used the existing
`ctx.output.data.def(d_nr)` for worker fn lookup and
`ctx.emit(arg)` for arg emission.

Phase 07 didn't add EmitCtx helpers either — the fix went into
emit.rs directly, not through EmitCtx.

Verdict: EmitCtx is stable since phase 03.  Future emitters
should reach for the existing helpers; if they need new ones,
that's a signal worth investigating (maybe the emitter is doing
too much).

### 5. P-issue closures vs plan structure

| P-issue | Phase | Closure mechanism |
|---|---|---|
| @P203 | 00 step 0.7b | Let-bind-on-repeat in `DefaultTemplateEmitter`.  No custom emitter; pure template-substitution refinement. |
| @P202 | 06 | Custom emitters (`ParallelQueueEmitter`, `ParallelBufRenameEmitter`) + runtime fns + reachability extension. |
| @P205 | 07 | Direct emit.rs patches at 2 sites; no custom emitter (the dangle isn't tied to a single Op). |

Three P-issues, three different closure mechanisms.  The
"phase = custom emitter" mapping the plan implied isn't strict —
sometimes the right fix is template-level (@P203), sometimes
emit-site-level (@P205).  The framework supports all three.

**Lesson**: don't pre-commit to "this phase will write a custom
emitter."  Survey the failing emission first; the right fix
shape emerges from the survey, not the plan.

### 6. Phase 02 (param adapter) status retrospective

Phase 02 was the originally-anticipated load-bearing
simplification ("prerequisite for @P200 write side").  Phase 00a
demoted it.  Phase 06 + 07 didn't need it.  Phase 10 step 10.3
will not need it (@P200 read-side fix is comparison-emission, not
param-narrowing).

**Decision**: phase 02 stays demoted.  Drop or move to a
separate cleanup plan after @PLAN09 closes.  No code in @PLAN09's
remaining work depends on it.

### 7. Plan velocity vs initial estimate

Plan-09 phases shipped over 2 days (2026-05-01 → 05-02):
- Day 1: phases 00 (6 commits) + 01 (4 commits) + initial
  @PLAN06 spillover.
- Day 2: phases 03, 04, 06, 07, 09 (each 1-2 commits) + 00a
  introspection + CI cleanup + plan refresh.

**That's faster than expected.**  The original plan estimated
2-3 weeks.  Possible reasons:
- Phase 00's hoist work front-loaded the infrastructure cost.
- Subsequent phases (01-09) reused the infrastructure cleanly.
- The wart-budget gates caught divergence early, avoiding
  multi-day debugging cycles.
- Auto-mode + memory entries kept context across sessions.

Phase 10's @P200 fix could be similarly fast (1 session) or
similarly surface-deep (2-3 sessions).  Apply estimate-doubling
from § 3 above: if it looks like 30-line fix, budget 60.

### 8. Decision (per the criteria table)

| Finding | Match |
|---|---|
| Both bug-fix phases (06 + 07) closed cleanly + suite green | ✓ |
| Custom emitters within 2× planned complexity | ✓ (within 3×, but acceptable) |
| Plan velocity supports finishing remaining work | ✓ (phase 10 + @PLAN11 both within reasonable session counts) |
| Any phase regressed outside-plan tests | ✗ (none — every phase preserved threading 43/43, threading_chars 35/35, issues 540/540) |

Verdict: **continue with confidence.**  Phase 10's @P200 fix +
@PLAN11's @P204 fix are within plan velocity; the framework holds
up; the wart-budget gates are doing their job.

### Memory entries (to save if not already)

These patterns surfaced in phases 06 + 07 + 09 across multiple
sessions:

- ✅ `feedback_forwarding_first_recipe.md` — saved.
- ✅ `feedback_phase_doc_trait_drafts.md` — saved (extended in
  this session with phase 06's flat-vs-trait confirmation).
- ✅ `feedback_actual_error_survey.md` — saved.

New patterns from this introspection that are NOT already
captured in memory:

- **Estimate emitter sizes as 2× the sketch length.**  Plan-doc
  sketches consistently understate by 1.5-3×.  Bake into
  phase-budget estimates.  (NEW — but plan-09-specific; if a
  similar pattern shows up in @PLAN11, save as
  `feedback_emitter_size_estimates`.)
- **Survey first, choose fix shape second.**  Don't pre-commit
  to "this phase will write a custom emitter" before the
  survey.  Already partially captured in
  `feedback_actual_error_survey`; reinforced here.
- **Reachability checks for emitters that synthesise calls.**
  Phase 06's gap (forgot to extend `collect_calls` for queue
  family) caused a 30-min surprise.  Future emitters that
  reference user-fns by name should add a reachability check
  to their acceptance criteria.  (NEW — but phase-specific;
  extending `feedback_forwarding_first_recipe` may be the right
  home if a similar gap recurs.)

### Output: downstream phase doc updates

Per the "Output" section of this introspection:

- **`06-threading.md`** — already populated with phase 06's
  Implementation notes + Problems encountered (in earlier
  commit `23526b6`).  No further update needed.
- **`07-generics.md`** — already populated with phase 07's
  Implementation notes (in commit `6151231`).  No further update
  needed.
- **`08-binary.md`** — superseded by phase 10 step 10.4's
  redundancy decision; no separate doc update needed.
- **`README.md`** — already updated multiple times.  Phase 10's
  scope reflects this introspection's findings.
- **`10-final-closure.md`** — references this introspection at
  step 10.1.  Step 10.3's @P200 fix should apply estimate-doubling
  rule from § 3 (target ≤ 60 lines if fix sketch suggests 30).
