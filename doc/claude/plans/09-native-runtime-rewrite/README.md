# Plan 09 — Per-Op emitter dispatch

## Goal

Add a per-Op codegen-time emitter dispatch on top of today's
`#rust"…@v0…"` template substitution, then use the cleaner
structure to close P-issues whose direct fixes have previously
failed.

The `#rust` annotations stay where they work.  Custom emitters are
written only for Ops where the template lacks the context needed
to emit correct code.

## Why simplification comes first

The four open P-issues in native codegen (P200, P202, P203, P205)
have either resisted fix attempts or sat unfixed because of
structural blockers — not because the bugs themselves are deep.
Specifically:

- **P200** was attempted and reverted: the fix collided with
  `src/generation/emit.rs::narrow_int_cast`'s **dual role** as
  both block-tail-expression coercion AND parameter narrowing.
  Fixing one role broke the other.  No surface-level edit in the
  current code can avoid this — the call sites genuinely share the
  cast-decision code.
- **P203** is a template double-substitution bug — `default/01_code.loft:705`'s
  `OpConvIntFromEnum` template substitutes `@v1` twice, so a
  side-effecting comparison like `delete(path) == FileResult.Ok`
  evaluates `delete()` twice (first call deletes the file, second
  returns NotFound).  Five templates have this hazard.  Plan 09's
  **DefaultTemplateEmitter** in phase 00 step 0.7b auto-detects
  repeated placeholders and emits a `let` binding once — closes
  P203 + the bug class structurally.  (Direct template let-bind
  is also viable as an interim fix; see PROBLEMS.md.)
- **P205**'s direct fix (remove a parser-side skip) might cascade
  into other tests; the emission code can't recover because it
  lacks the type-binding info needed to emit owned-`String` vs
  borrowed-`Str` correctly.
- **P202** is just missing runtime fns, but adding them today
  duplicates the 95-line parallel-for special case in
  `dispatch.rs:837-930`.

**The simplifications dissolve these structural blockers.**  Phase
02 (param adapter) splits `narrow_int_cast`'s dual role.  Phase 00
(scaffold) adds the `EmitCtx` where helpers like `is_file_ref` live.
Phase 03 (parallel-emitter family) makes adding queue fns ~15 lines
each instead of ~95.  After the simplifications, the previously-
failed direct fixes become tractable.

## Approach

Two artefacts:

1. **`OpEmitter` trait + registry in `src/generation/ops/`** — one
   trait impl per Op that needs custom emission.  Every code path
   that emits an Op routes through `emit_op(ctx, name, args)` —
   single dispatch point.

2. **Default emitter** — when no custom emitter is registered, the
   dispatch falls through to today's `#rust` template substitution.
   No behaviour change for Ops without overrides.

That's it.  No per-test registry, no migration tracking.  An Op is
"fixed" when its custom emitter is registered and the relevant
test passes.  The existing test suite is the authority.

## Cadence

Four kinds of phase:

- **Dispatch hoist** (phase 00 only): hoists every Op-emission call
  site through `emit_op(ctx, name, args)`.  Default emitter is
  byte-identical to today's substitution → no behaviour change.

- **Simplification** (phases 01-04, ~50-150 lines net delete):
  replaces a scattered special-case in `calls.rs` / `dispatch.rs` /
  `codegen_runtime.rs` with a per-Op or per-parameter-type emitter.
  Dissolves the structural blockers behind the bug-fix phases.

- **Bug fix** (phases 05-08, ~30-80 lines): adds one custom
  `OpEmitter` impl on top of the simplified structure, registers
  it, closes a specific P-issue.  Each bug-fix phase has a
  **diagnostic gate** (pre-work that documents the prior failure
  mode and shows how the new approach avoids it) and a
  **prior-failure regression test** that lands BEFORE the fix.

- **Introspection** (phases 00a, 02a, 05a, 08a, no code): time-boxed
  reviews after each high-risk milestone.  Update downstream plan
  files based on what actually happened.  May trigger continue /
  pivot / stop decisions.  Output includes durable memory entries.

## Order of work

Simplifications **must** land before bug fixes.  Phase numbering
reflects this order — 00 first, then 01-04 (simplification), then
05-08 (bug fixes).  Introspection phases (00a, 02a, 05a, 08a) fire
between work phases at decision points.

## Status

| # | Phase | Closes | Kind | Status |
|---|-------|--------|------|--------|
| 00 | [Scaffold](00-scaffold.md) | P203 (structural — let-bind-on-repeat in `DefaultTemplateEmitter`) | infra | OPEN |
| 00a | [Introspection: after scaffold](00a-introspect.md) | — | introspection | OPEN |
| 01 | [ABI consolidation](01-abi-consolidation.md) | — (deletes `LEGACY_STORES_FNS` hardcoded list) | simplification | OPEN |
| 02 | [Param adapter](02-param-adapter.md) | — (splits dual-role `narrow_int_cast`) — **prerequisite for P200** | simplification | OPEN |
| 02a | [Introspection: after param adapter](02a-introspect.md) | — | introspection | OPEN |
| 03 | [Parallel-for emitter](03-parallel-emitter.md) | — (collapses 95-line `dispatch.rs:837-930`) — **prerequisite for P202** | simplification | OPEN |
| 04 | [Key-keyed Op emitter](04-key-ops.md) | — (consolidates `OpGetRecord` / `OpIterate`) | simplification | OPEN |
| 09 | [Parallel runtime consolidation](09-parallel-runtime-consolidation.md) | — (collapses 3 near-duplicate `n_parallel_for_*_native` fns into one generic core; **must land before phase 06**) | simplification | OPEN |
| 05 | [File emitters](05-file.md) | P200 (write side) | bug fix | OPEN |
| 05a | [Introspection: after first bug fix](05a-introspect.md) | — | introspection | OPEN |
| 06 | [Threading queue runtime fns](06-threading.md) | P202 | bug fix | OPEN |
| 07 | [Generic text emitter](07-generics.md) | P205 | bug fix | OPEN |
| 08 | [Binary read emitter](08-binary.md) | P200 (read side, full closure) | bug fix | OPEN |
| 08a | [Retrospective](08a-introspect.md) | — | introspection | OPEN |

Status legend: OPEN → IN PROGRESS → DONE.

### Introspection cadence

Each introspection phase is time-boxed to 1 day max and produces:
- updates to the remaining phase files based on what actually
  happened (so phases adapt rather than committing blindly to
  initial designs);
- continue / pivot / stop decisions when findings warrant;
- durable memory entries when patterns surface that should outlive
  this plan.

The four insertion points are deliberately at the **highest-risk
boundaries**: after the scaffold (phase 00 is the riskiest infra
step), after the load-bearing simplification (phase 02 tests the
emitter pattern), after the first bug fix (phase 05 validates the
diagnostic-gate concept), and at the end (retrospective).  Earlier
introspection means earlier escape hatches.

## Dependency chain — what unblocks what

```
00 scaffold
  ├─→ 01 ABI consolidation (independent simplification)
  ├─→ 02 param adapter ──────────→ 05 file (P200 write + P203)
  │                              └─→ 08 binary (P200 read)
  ├─→ 03 parallel emitter ───┐
  │                          └─→ 09 parallel runtime ──→ 06 threading (P202)
  ├─→ 04 key ops (independent)
  └─→ 07 generics (P205) — needs scaffold + 02 only if probe cascades
```

Phase 09 is sequenced **between 03 and 06** because phase 06 will
otherwise multiply the duplication that phase 09 retires.  Phases
04 and 05 can run in parallel with 09 since they don't touch the
parallel runtime fns.

Phase 07 (P205) has a **diagnostic probe** that may short-circuit
to a 1-line parser fix without needing the emitter — see phase 07
for the branching.

## Why each P-issue becomes believable

| P-issue | Prior blocker | What dissolves it | Phase |
|---|---|---|---|
| P200 | `narrow_int_cast` dual role; fix collided with itself | Phase 02 splits the cast into per-type adapters AND extracts shared `narrow_for_int` helper that retires the dual-role; bug-fix emitters bypass it | 02 → 05 + 08 |
| P202 | Adding queue fns duplicates 95-line parallel-for case | Phase 03 gives queue fns a 15-line slot in the emitter family; phase 09 collapses runtime fns so queue variants are 3-line wrappers | 03 → 09 → 06 |
| P203 | Template double-substitution: `OpConvIntFromEnum` substitutes `@v1` twice, so `delete(path) == FileResult.Ok` calls `delete()` twice (first deletes file, second returns NotFound) | Phase 00 step 0.7b adds let-bind-on-repeat to `DefaultTemplateEmitter` — auto-detects repeated placeholders and binds once.  Closes the bug class for all 5 affected templates simultaneously | 00 (step 0.7b) |
| P205 | Direct skip-removal might cascade; template lacks type-binding info; sibling Ops may share the dangle | Phase 07 surveys corpus for all dangling-shape sites, runs probe, then either 1-line fix or per-Op emitter | 07 |

Each bug-fix phase includes:

- **Diagnosis** — root cause beyond the symptom.
- **Prior attempts** — what was tried and why it failed.
- **Why this works now** — how the simplification dissolves the
  prior blocker.
- **Pre-work (gates implementation)** — concrete diagnostic steps
  that must complete before the fix is attempted.

## What stays as-is

- **`#rust"…@v0…"` annotations in `default/*.loft`.**  Source of
  truth for any Op that doesn't have a custom emitter.
- **The interpreter codegen path** (`src/state/codegen.rs`).
  Untouched.
- **P199.A's `pub mod rt` and `CODEGEN_RUNTIME_FNS`.**  Existing
  runtime fns stay; phase 01 generalises their ABI tag.

## What changes

- **All Op-emission call sites route through `emit_op`** —
  `output_call_template`, `output_call_user_fn`, `dispatch.rs`
  direct emissions, `emit.rs` fn-ref dispatch.
- **Custom emitters live in `src/generation/ops/<op>.rs`** — one
  file per Op with override logic.

## P204 — explicitly out of scope

P204 (tail-expression return discarded) is a parser/scope-analysis
bug in `collect_hidden_ref_args`, not a codegen-template bug.  No
emitter change closes it.  Track P204 separately — likely a
sibling plan focused on the `__ref_*` propagation path through
`Block` arms and `Call` resolution.

## Acceptance gate (every commit)

```bash
cargo build --release --tests
cargo test --release --test issues 2>&1 | tail -3        # 540/540
cargo test --release --test threading 2>&1 | tail -3     # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3  # 35/35
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"                              # ≥ baseline
```

Bug-fix phases (05-08) additionally run the regression test
introduced by their pre-work step, which pins the prior failure
mode.  Any regression aborts the commit.

## Risks

| Risk | Mitigation |
|---|---|
| Hoisting all call sites through `emit_op` perturbs an emission shape that worked accidentally | Phase 00 default emitter is byte-identical to today's substitution.  Any divergence is a phase-00 bug; the registry stays empty until full suite green. |
| A simplification phase regresses a non-P-issue test path | Acceptance gate runs the full test suite per commit; regressions abort. |
| A bug-fix phase falls into the same trap as the prior attempt | Pre-work step explicitly identifies the prior failure mode and writes a regression test that catches it.  The fix doesn't ship until the regression test passes alongside the suite. |
| Phase 07 (P205) probe shows the skip-removal alone works → emitter unnecessary | That's a positive outcome.  Phase 07 ships the 1-line parser fix and skips the emitter work. |

## Related

- [P199](../../PROBLEMS.md#199) — UnsafeCell ABI swap; runtime-fn
  pattern that emitters can target.
- [P200](../../PROBLEMS.md), [P202](../../PROBLEMS.md),
  [P203](../../PROBLEMS.md), [P205](../../PROBLEMS.md) — closed by
  phases 05-08 after simplification prerequisites land.
- [P204](../../PROBLEMS.md) — parser-side; out of scope here.
