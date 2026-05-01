# Phase 05a — Introspection: after first bug fix

**Status:** OPEN

**Kind:** Review (no code)

**Trigger:** Phase 05 marked DONE (P200 write side + P203 either
closed or rerouted).

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
phases to attempt."  P200 write + P203 is the highest-value subset;
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

### P203 status
9. Did P203 close in this phase, or reroute to parser work?
10. If rerouted, is it now blocked, or do we have a clear path
    outside this plan?

### P200 write closure
11. P200 write side: did the round-trip suite (including the
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
  P205 reproducer.
- **`08-binary.md`**: phase 05's experience directly informs
  phase 08 (mirrors the pattern).  If phase 05 hit unexpected
  EmitCtx complexity, phase 08 inherits it — flag.
- **`README.md`**: update status table; if velocity shows the
  full plan won't finish in budget, mark phases 06/07/08 as
  optional or sequence them by ROI.

## Decision criteria

| Finding | Action |
|---|---|
| P200 write closed cleanly + P203 closed | Strong signal that the plan works.  Continue to 06/07/08 with confidence. |
| P200 write closed + P203 rerouted but cleanly so | Plan working as designed (graceful degradation).  Continue. |
| Both closed but custom emitters needed >2× planned complexity | Re-budget phases 06-08; consider deferring phase 08 (read side, less urgent than write was). |
| P200 write closed but caused regressions outside the suite | Fix regressions, then introspect again before phase 06. |
| P200 write didn't close (the prior-failure pattern recurred) | **Stop and investigate.**  The plan's central claim is broken; phase 02's adapter split didn't actually fix the dual-role issue. |
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

_(populate after phase 05 lands)_
