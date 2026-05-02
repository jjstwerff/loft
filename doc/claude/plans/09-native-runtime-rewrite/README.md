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
| 00 | [Scaffold](00-scaffold.md) | P203 (structural — let-bind-on-repeat in `DefaultEmitter`) | infra | **DONE (2026-05-02)** |
| 00a | [Introspection: after scaffold](00a-introspect.md) | — | introspection | **DONE (2026-05-02)** |
| 01 | [ABI consolidation](01-abi-consolidation.md) | — (deletes `LEGACY_STORES_FNS` hardcoded list) | simplification | **DONE (2026-05-02)** |
| 02 | [Param adapter](02-param-adapter.md) | — (splits param-narrowing role of `narrow_int_cast`) | simplification | OPEN — **demoted by 00a (no longer P200 prereq)** |
| 02a | [Introspection: after param adapter](02a-introspect.md) | — | introspection | OPEN |
| 03 | [Parallel-for emitter](03-parallel-emitter.md) | — (collapsed 95-line `dispatch.rs:850-944`) — **prerequisite for P202** | simplification | **DONE (2026-05-02)** |
| 04 | [Key-keyed Op emitter](04-key-ops.md) | — (consolidates `OpGetRecord` / `OpIterate`) | simplification | **DONE (2026-05-02)** |
| 09 | [Parallel runtime consolidation](09-parallel-runtime-consolidation.md) | — (collapses 3 near-duplicate `n_parallel_for_*_native` fns into one generic core; **must land before phase 06**) | simplification | **DONE (2026-05-02)** |
| 05 | [File emitters](05-file.md) | P200 (read-side comparison emission) | bug fix | **DONE (2026-05-02)** via phase 10 step 10.3 — `IntCompareEmitter` widens both operands to i64 |
| 05a | [Introspection: after first bug fix](05a-introspect.md) | — | introspection | **DONE (2026-05-02)** via phase 10 step 10.1 |
| 06 | [Threading queue runtime fns](06-threading.md) | P202 | bug fix | **DONE (2026-05-02)** |
| 07 | [Generic text emitter](07-generics.md) | P205 | bug fix | **DONE (2026-05-02)** |
| 08 | [Binary read emitter](08-binary.md) | P200 (read side, full closure) | bug fix | **SUPERSEDED by phase 10 step 10.3** (one fix closed read + write) |
| 08a | [Retrospective](08a-introspect.md) | — | introspection | **superseded by phase 10 step 10.6** |
| 10 | [Final closure](10-final-closure.md) | **P200** + plan-09 close-out | bug fix + admin | IN PROGRESS — 5/7 steps DONE (10.1-10.4 + 10.5-scaffold); 10.5 polish + 10.6 retrospective + 10.7 directory move OPEN |

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
00 scaffold (DONE) ──→ 00a introspection (DONE)
  ├─→ 01 ABI consolidation (DONE)
  ├─→ 02 param adapter (DEMOTED — no longer P200 prereq; optional)
  ├─→ 03 parallel emitter (DONE) ───┐
  │                                 └─→ 09 parallel runtime (DONE) ──→ 06 threading (DONE — P202 closed)
  ├─→ 04 key ops (DONE)
  ├─→ 07 generics (DONE — P205 closed via emit.rs scratch routing)
  └─→ 10 final closure (IN PROGRESS — P200 closed via step 10.3; admin remaining)
        ├── absorbed phase 05 (read-side scope, was write-side)
        ├── absorbed phase 05a (fired as step 10.1)
        ├── superseded phase 08 (one fix closed read + write)
        └── absorbed phase 08a (retrospective at step 10.6)

P204 — out of plan-09 scope; full plan at plans/11-p204-ref-propagation/
```

Phase 09 was sequenced **between 03 and 06** because phase 06
would otherwise multiply the duplication that phase 09 retires.
That sequencing held; phase 06 shipped with its runtime fns flat
(per phase 06's "Implementation notes — trait reuse vs flat")
and gained from phase 09's `ParShape` only indirectly — the
closure-shape selection logic is shared via `closure_shape` /
`queue_helper_name` in `parallel.rs`.

Phase 07 (P205) used a **diagnostic probe** that revealed
Outcome B — the `text_return` parser-side promotion is
structurally incomplete for bounded-generic specialisations.
The fix landed as direct emit.rs scratch routing at two sites
(Value::Return wrap_text + block-tail wrap_result), not as an
Op-level custom emitter.  See `07-generics.md` § Implementation
notes for the rationale.

Phase 02's prereq role to phase 05 was demoted by **phase 00a**:
phase 05's actual P200 bug was the block-tail role of
`narrow_int_cast`, not its param-narrowing role.  The fix
shipped via phase 10 step 10.3's `IntCompareEmitter` —
comparison-emission, not param adaptation.  Phase 02 retains
independent simplification value but is no longer on the
critical path; status remains DEMOTED.

Phase 05's plan was rewritten on 2026-05-02 to target read-side
comparison emission (per phase 00a's actual-error survey).
Phase 10's step 10.3 implements that revised plan and closes
P200; the original write-side scope (separate `OpWriteIntFile`
emitter) was not needed — the `IntCompareEmitter` at the
comparison level closed both read AND write sites in one fix.

## Native suite progression

Reference for "are we beating main yet?" (main is at 5/5).

| Milestone | native_dir | native_scripts | High-level | Gap to main |
|---|---|---|---|---|
| Pre-plan-09 | 29/30 | 87/93 | 2/5 | -3 |
| After phase 06 (P202) — 2026-05-02 | **30/30** | 89/93 | **3/5** | -2 |
| After phase 07 (P205) — 2026-05-02 | 30/30 | **90/93** | 3/5 | -2 |
| After phase 10 step 10.3 (P200) — 2026-05-02 | 30/30 | **91/93** | **4/5** | -1 |
| After P204 fixed via plan-11 — projected | 30/30 | 93/93 | **5/5** | 0 (PR-ready) |

Each row pins an acceptance floor.  The active branch must beat
the highest unmet floor before moving on; future commits that
silently shrink any milestone count are regressions.

## Why each P-issue becomes believable

| P-issue | Prior blocker | What dissolves it | Phase |
|---|---|---|---|
| P200 | `narrow_int_cast` dual role; fix collided with itself | Revised by 00a: the bug was block-tail comparison-emission, not param narrowing.  Phase 10 step 10.3 added `IntCompareEmitter` that widens both operands of `OpEqInt`/`OpNeInt`/`OpLtInt`/`OpLeInt` via `(operand as i64)`.  All 5 P200 sub-failures retired in one fix; phase 02 (param-narrowing split) confirmed not needed.  Phase 08 also superseded (no separate write-side fix needed) | 10 step 10.3 **CLOSED 2026-05-02** |
| P202 | Adding queue fns duplicates 95-line parallel-for case | Phase 03 gave queue fns a slot in the emitter family; phase 09 collapsed for-par runtime fns; phase 06 added 3 flat queue runtime fns (~90 LOC vs the originally-projected ~120 LOC trait + wrappers — see phase 06 § Implementation notes for the trait-reuse-vs-flat decision) | 03 → 09 → 06 **CLOSED 2026-05-02** |
| P203 | Template double-substitution: `OpConvIntFromEnum` substitutes `@v1` twice, so `delete(path) == FileResult.Ok` calls `delete()` twice (first deletes file, second returns NotFound) | Phase 00 step 0.7b adds let-bind-on-repeat to `DefaultTemplateEmitter` — auto-detects repeated placeholders and binds once.  Closes the bug class for all 5 affected templates simultaneously | 00 (step 0.7b) **CLOSED** |
| P205 | Direct skip-removal might cascade; template lacks type-binding info; sibling Ops may share the dangle | Phase 07 ran the probe (2026-05-02) → Outcome B confirmed.  Implementation revealed the dangle is at TWO emit.rs sites (Value::Return wrap + block-tail wrap_result), not a single Op.  Fix: detect "function returns Type::Text but has no `Type::RefVar(Type::Text(_))` attribute" and route the value through `stores.scratch` so the backing String lives as long as `stores` | 07 **CLOSED 2026-05-02** |

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

## P204 — explicitly out of scope (must close via plan-11 before PR)

P204 (tail-expression return discarded) is a parser/scope-analysis
bug in `collect_hidden_ref_args`, not a codegen-template bug.  No
emitter change closes it.

**Sibling plan** at
[plans/11-p204-ref-propagation/README.md](../11-p204-ref-propagation/README.md)
— must complete before PR-open.  P204's two failing native tests
(`85_yield_resume`, `87_store_leaks`) block PR-readiness against
`main`'s 5/5 native pass; per `feedback_no_expect_fail_on_pr_bugs.md`,
**@EXPECT_FAIL is not an acceptable resolution** — the bug must
be actually fixed.

PR-readiness gate is therefore:
1. Plan-09 phase 10 closes P200 → branch reaches 91/93 native.
2. Plan-11 closes P204 → branch reaches 93/93 native.
3. Native parity with `main` (5/5 high-level + 0 sub-failures) →
   PR opens.

## Acceptance gate (every commit)

```bash
cargo build --release --tests
cargo test --release --test issues 2>&1 | tail -3        # 540/540
cargo test --release --test threading 2>&1 | tail -3     # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3  # 35/35
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"                              # ≥ baseline
```

Bug-fix phases (06, 07, 10 step 10.3) additionally run the
regression tests introduced alongside the fix, which pin the
prior failure mode.  Any regression aborts the commit.

Per `feedback_zero_regression_tolerance.md`: regressions are
never accepted, regardless of how long the proper fix takes.
The fast gate, baseline corpus, structural emitter tests, and
this acceptance gate together enforce that.

## Fast gate (every step)

`scripts/p09_fast_gate.sh` (~4 seconds) is the routine step-by-step
check between commits — it emits the doc-test corpus, diffs against
`/tmp/p09-baseline/`, runs the P-issue reproducers, and prints
phase-01 progress markers (legacy fn count, dispatch arm count,
custom emitter count).

**Convention**: every phase doc lists a `Gate updates per step`
table noting:
- Which steps refresh the baseline (`--capture`).
- Which steps add new structural assertions to
  `tests/codegen_emitter.rs`.
- Which steps shift the gate's progress markers (legacy fn count,
  dispatch arm count, etc.).

The gate's progress output stays accurate as work lands because
each phase explicitly captures its expected impact.  Phase 01's
table is the reference shape.

## Risks

| Risk | Mitigation |
|---|---|
| Hoisting all call sites through `emit_op` perturbs an emission shape that worked accidentally | Phase 00 default emitter is byte-identical to today's substitution.  Any divergence is a phase-00 bug; the registry stays empty until full suite green. |
| A simplification phase regresses a non-P-issue test path | Acceptance gate runs the full test suite per commit; regressions abort. |
| A bug-fix phase falls into the same trap as the prior attempt | Pre-work step explicitly identifies the prior failure mode and writes a regression test that catches it.  The fix doesn't ship until the regression test passes alongside the suite. |
| Phase 07 (P205) probe shows the skip-removal alone works → emitter unnecessary | That's a positive outcome.  Phase 07 ships the 1-line parser fix and skips the emitter work. |
| Codegen-only `Value` variants accrete (one was added in step 0.7 for fn-ref dispatch) | Wart-budget gate `tests/codegen_emitter.rs::no_unsanctioned_codegen_value_variants` — sanctioned list is `["RawExpr"]`; new entries fail.  Future codegen synthesis must use string-aware companion entry points instead of new `Value` variants. |
| `dispatch.rs::output_call_inner` accumulates more direct-emission match arms (parallel dispatch system) | Wart-budget gate `tests/codegen_emitter.rs::dispatch_op_arm_budget_not_exceeded` — current budget 26; shrinks as phases register custom emitters; raising the budget requires NATIVE.md justification. |
| First real custom emitter (phase 05+) hits unforeseen lifetime / helper-access issues | Recommendation in phase 00 scaffold doc: write a smoke-test custom emitter (~30 min) before phase 05 to surface gotchas early. |

## Related

- [P199](../../PROBLEMS.md#199) — UnsafeCell ABI swap; runtime-fn
  pattern that emitters can target.
- [P200](../../PROBLEMS.md), [P202](../../PROBLEMS.md),
  [P203](../../PROBLEMS.md), [P205](../../PROBLEMS.md) — closed by
  phases 05-08 after simplification prerequisites land.
- [P204](../../PROBLEMS.md) — parser-side; out of scope here.
