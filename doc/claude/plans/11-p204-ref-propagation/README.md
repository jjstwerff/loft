# Plan 11 — P204: tail-expression return discarded

**Status:** stub (design pending)

**Closes:** **P204** (native: tail-expression `return inner_call()`
from a struct-returning function emits the inner call as a void
statement and returns the null sentinel).

**Out of scope for plan-09.**  Plan-09 covers per-Op codegen
emitter dispatch + four narrow-scope P-issues (P200, P202, P203,
P205); P204 is parser-side scope analysis in
`collect_hidden_ref_args` / `__ref_*` placeholder propagation,
NOT codegen emission.  No emitter change closes it.

## Why this stub exists now (2026-05-02)

Plan-09's bug-fix phases (06 done, 07 + 05 pending) close 4 of
the 6 native sub-failures on `roadmap-lsp-eclipse`.  After
plan-09 completes, the remaining 2 sub-failures
(`85_yield_resume`, `87_store_leaks`) are P204's manifestations
and block PR-readiness against `main` (which has 5/5 native
pass).

Two paths once plan-09 is otherwise PR-ready:

1. **Open plan-11 properly** — design the `__ref_*` propagation
   fix; ship before PR.  Cleanest result.
2. **Add @EXPECT_FAIL on the 2 tests** — bookmark P204 in the
   skip list, defer the fix.  Faster, but leaves the bug live.

The choice happens at PR time; this stub exists to make the
P204 reference visible from `doc/claude/plans/` rather than
hiding inside PROBLEMS.md.

## P204 reference

Symptom + reproducer + diagnosis live in
[PROBLEMS.md § 204](../../PROBLEMS.md#204-tail-expression-return-of-inner-helper-call-discarded).
Reproducer: `tests/scripts/repro_p204.loft` (currently
`@EXPECT_FAIL`).

## Design notes (skeleton — fill when plan opens)

- Root cause: native codegen's `Call` value emission for
  tail-expressions doesn't see the surrounding `__ref_*`
  placeholder that the parser injects for struct-returning fns.
- The interpreter routes the result through the `__ref_*` mechanism;
  native skips that path and emits the call as a void statement.
- Likely fix sites: parser-side scope analysis in
  `collect_hidden_ref_args` and/or codegen-side handling of
  `Block` arms whose tail is a `Call` returning a heap-allocated
  type.
- Plan-09 phase 00a's introspection captured the lesson: bug-fix
  phases need an "actual error survey" pre-step.  Apply that
  here when the plan opens.

## When this opens

Trigger: plan-09 is otherwise complete (phases 05, 07, 08 DONE)
and `roadmap-lsp-eclipse` test count equals or beats `main`
EXCEPT for P204's 2 sub-failures.  At that point the user
chooses path (1) or (2) above.
