# Phase 02a — Introspection: after param adapter

**Status:** SUPERSEDED (2026-05-02) — phase 02 SUPERSEDED by
@PLN80 phase 05.  Plan-12 will run its own introspection cadence
under @PLN80's phase structure if needed.  Phase 05a + the
@PLAN09 retrospective (08a equivalent in phase 10 step 10.6)
absorbed the substantive lessons; 02a's specific trigger (phase
02 DONE) is now never going to fire.

**Kind:** Review (no code)

**Trigger** (no longer relevant): Phase 02 marked DONE.  This
was the first checkpoint where simplification effectiveness
could be measured.

**Time budget:** 1 day max.

## Why this phase exists

Phase 02 (param adapter) is the load-bearing simplification.  If it
landed cleanly, the per-Op-emitter pattern works and phases 03/04
should follow the same shape.  If it required heavy iteration or
the resulting code is awkward, phases 03/04 need redesign or
descoping.

This is also the last point at which we can pivot away from the
emitter pattern entirely without sunk-cost concern: phase 00 is
infrastructure that's broadly useful regardless; phase 01 is a
small consolidation; phase 02 is where the pattern gets really
tested.

## Questions to answer

### Was the adapter pattern worth it?
1. Total LOC delta: planned ~125 deleted + ~150 added = +25 net.
   Actual?
2. Subjective code-quality assessment: is the `params.rs` file
   easy to read and extend, or did it accrete its own complexity?
3. How many iterations did the byte-identical golden diff take?
   (Each iteration = an adapter's `applies()` predicate didn't
   match the original arm exactly.)

### Did the adapter ordering hold up?
4. Did the `ADAPTERS` order need changes during implementation?
   What invariants surfaced that the plan didn't anticipate?
5. Are there pairs of adapters where the order matters subtly
   enough that future contributors might break it?

### Phase 05/08 prerequisite quality
6. Does `param_adaptation_does_not_route_through_narrow_int_cast`
   actually pass?  If yes, @P200 fix is genuinely unblocked.
7. What additional `EmitCtx` helpers did phase 02 need
   (`int_width_for`, `int_signed_for`, `value_type`)?  Are they
   reusable for phase 05 or do they need refactoring?

### Compatibility with phase 03 plan
8. The parallel-emitter plan in phase 03 follows the same
   "extract helpers → wrap in OpEmitter" shape.  Does anything
   from phase 02 suggest that shape will or won't work for the
   parallel-for emission?

## Output

Update these files based on findings:

- **`03-parallel-emitter.md`**: if phase 02's adapter pattern
  ended up awkward, redesign phase 03 to use a more direct
  approach (e.g., free functions instead of trait impls).  If
  pattern works well, no change.
- **`04-key-ops.md`**: same as phase 03.
- **`05-file.md`**: confirm `int_width_for` etc. helpers from
  phase 02 are sufficient for the file emitter, or list the
  additional helpers needed.
- **`08-binary.md`**: same as phase 05.

## Decision criteria

| Finding | Action |
|---|---|
| Phase 02 landed clean (≤1.5× budget, adapters read well) | Continue with phases 03/04 as designed. |
| Phase 02 landed but `params.rs` is awkward | Redesign phases 03/04 with simpler shape; keep phase 02 as-is. |
| Phase 02 landed but adapter ordering is too fragile | Add a defensive ordering test framework before phase 03; document the ordering invariants visibly. |
| Phase 02 took >2× budget OR multiple adapters resisted byte-identical extraction | **Pivot.**  Skip phases 03/04 (the simplifications that depend on the same pattern); jump straight to bug-fix phases 05-08 using whatever phase 02 partially achieved.  Accept reduced cleanup to ship the bug fixes. |
| Phase 02 was clean but the byte-identical guard failed in production | Fix the guard before continuing; expand corpus. |

## Memory entries to save

Examples:

- "Per-Op emitter pattern: extracting N tangled `if matches!()` arms
  to a `ParamAdapter` trait worked / didn't work.  Adapter-ordering
  invariants were obvious / hidden.  Apply / avoid this pattern in
  future codegen refactors."
- "EmitCtx helper accretion: phase 02 added X helpers.  Most were
  trivial accessors; Y required non-trivial plumbing.  Future codegen
  refactors should expect ~2-3 days of plumbing per major phase."

## Findings

_(populate after phase 02 lands)_
