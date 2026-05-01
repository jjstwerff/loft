# Plan 10 — Scope-exit emission rewrite

## Goal

Make resource cleanup (specifically OpFreeRef at scope exit)
independent of the parser's dep-tracking system being correct.
Today, OpFreeRef is emitted only where dep-tracking attaches a
`__ref_*` placeholder; gaps in dep-tracking → silent resource leaks
or wrong-cleanup-timing.  This plan replaces the precise-but-fragile
dep-tracking emission with a mechanical scope-walk and a runtime
no-op fast-path, then adds Drop-based safety-net for file
resources.

## Why

P203 (file handle not flushed on `{...}` block exit) is the visible
symptom of an upstream problem: the dep-tracking analysis that
should attach `__ref_*` to file-ref-returning calls misses some
cases.  Plan 09's per-Op emitter pattern can fix the *shape* of
OpFreeRef when it fires, but cannot fix "OpFreeRef doesn't fire."

P204 is a related dep-tracking gap (tail-expression return discarded
because `__ref_*` propagation through `Block` arms in `Call`
resolution misses).  Naive fixes have repeatedly regressed other
tests because the dep-tracking system's invariants are subtle.

The plan's central insight: **don't fix the dep-tracking system —
remove its responsibility for cleanup-emission.**  A mechanical
scope-walk is simpler, more reliable, and has no subtle invariants.
Dep-tracking can still exist for non-cleanup purposes (aliasing
analysis, closure capture, parallel isolation) but stops being the
single point of failure for resource cleanup.

## Approach

Surprise (from phase 00 survey of the actual code): **two of the
three pieces are already in place.**

1. **OpFreeRef runtime is already safe** —
   `src/codegen_runtime.rs:98-122` already early-returns on
   already-freed slots (line 100) and out-of-bounds store_nr
   (line 103).  Phase 01 collapses to verification + tests.
2. **Drop on file handles is already in place** —
   `src/database/mod.rs:162` declares
   `pub files: Vec<Option<std::fs::File>>`; setting a slot to
   `None` drops the previous `Some(File)`, which flushes + closes
   the OS handle.  Phase 03 collapses to verification + tests.
3. **The actual fix is a small edit in `src/scopes.rs:1053`** —
   the gate that decides whether to emit OpFreeRef per local.
   Today: `(dep.is_empty() || is_work_ref) && !in_ret && !skip_free`.
   After phase 02: `!in_ret && !skip_free`.  The dep gate was an
   optimisation that masked emission gaps for cases the dep
   tracker mishandled (P203 file ref, work-refs).

The runtime safety + Drop semantics from pieces 1 and 2 are what
*makes* the gate loosening in piece 3 safe.  Together: cleanup
correctness no longer depends on the dep tracker being perfect.

## Cadence

| Commit shape | Description |
|---|---|
| Survey | Phase 00 only.  Verify the three preliminary findings; trace why OpFreeRef doesn't fire for P203's file ref (which gate condition fails). |
| Runtime safety tests | Phase 01.  Pin the existing OpFreeRef safety with regression tests; document the contract in LIFETIME.md.  No code change. |
| Gate loosening | Phase 02.  3-line edit in `src/scopes.rs:1053` that drops the `dep.is_empty()` gate.  Closes P203.  Plus suppression-list updates for any test that regresses. |
| Drop semantics tests | Phase 03.  Pin the existing `Vec<Option<File>>` Drop with regression tests; document the contract.  Optional (only if defence-in-depth wanted). |
| Introspection | 02a (after the load-bearing change), 03a (retrospective). |

## Status

| # | Phase | Closes | Kind | Status |
|---|-------|--------|------|--------|
| 00 | [Survey](00-survey.md) | — | infrastructure | OPEN |
| 01 | [Verify runtime safety + add tests](01-runtime-noop.md) | — | verification | OPEN |
| 02 | [Loosen the scope-exit emission gate](02-scope-walk.md) | P203 + cleanup-side of P204 | bug fix | OPEN |
| 02a | [Introspection: after gate loosening](02a-introspect.md) | — | introspection | OPEN |
| 03 | [Verify Drop semantics + add tests](03-drop-safety-net.md) | — (defence in depth) | verification | OPEN |
| 03a | [Retrospective](03a-retrospective.md) | — | introspection | OPEN |

Status legend: OPEN → IN PROGRESS → DONE.

## Relationship to plan 09

Plan 09 (per-Op emitter rewrite) and this plan (scope-exit emission)
are **complementary**:

- Plan 09 fixes the *shape* of generated cleanup code (file-flavour
  emitter knows to flush + close).
- Plan 10 fixes *whether* cleanup gets generated at all
  (scope-walk guarantees emission).

If plan 10 lands before plan 09's phase 05, plan 09's step 5.1b
diagnostic gate becomes redundant — the scope-walk already
guarantees OpFreeRef fires.  Plan 09's phase 05 simplifies to just
the file-flavour emitter (no rerouting branch needed).

If plan 09's phase 05 lands first and step 5.1b reroutes P203 to
parser work, this plan picks up that work.

Either order works.  Recommended: **plan 10 first** if file-related
bugs are the priority; **plan 09 first** if codegen simplification
is the priority.

## What stays out of scope

- **P204 (tail-expression return discarded)**: shares the dep-
  tracking root cause but the symptom is in `Call` resolution, not
  scope exit.  Plan 10's scope-walk doesn't help.  Track P204 as a
  sibling issue (likely plan 11).
- **Dep-tracking elimination wholesale**: only the
  cleanup-emission half is addressed.  Other consumers (aliasing,
  closure capture, parallel isolation) keep dep-tracking unchanged.

## Acceptance gate (every commit)

```bash
cargo build --release --tests
cargo test --release --test issues 2>&1 | tail -3        # 540/540
cargo test --release --test threading 2>&1 | tail -3     # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3  # 35/35
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"                              # ≥ baseline
```

## Risks

| Risk | Mitigation |
|---|---|
| Scope-walk emits OpFreeRef where today's dep-tracking deliberately suppresses it (e.g., for owned-by-callee refs) | Phase 00 survey catalogues every "deliberate suppression" case; phase 02 preserves them via an explicit allow-list rather than relying on emission omission. |
| Runtime no-op fast-path masks bugs that today manifest as visible double-frees | Add a debug-build assertion that distinguishes "already freed" from "not a ref" — only the latter is silently absorbed. |
| Drop safety net changes timing of file flush in user-visible ways | Drop fires at slot-reclaim, which is later than scope exit.  The scope-walk in phase 02 ensures user-visible timing is correct; phase 03 only catches missed cases.  Document the timing contract clearly. |
| Test relies on OpFreeRef NOT firing for some specific var | Survey in phase 00 + diagnostic in phase 02 should catch this; if a test fails, investigate before pushing through. |

## Related

- [P203](../../PROBLEMS.md) — primary closure target.
- [P204](../../PROBLEMS.md) — sibling dep-tracking issue; out of scope for this plan.
- [Plan 09](../09-native-runtime-rewrite/README.md) — per-Op emitter rewrite; complementary.
- [LIFETIME.md](../../LIFETIME.md) — dep tracking and scope-based freeing design.
