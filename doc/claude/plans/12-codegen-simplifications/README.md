# Plan 12 — Codegen simplifications (post-09 follow-ups)

**Status:** OPEN

**Origin:** Surfaced during plan-09's simplification audit
(2026-05-02) after plan-09 + plan-11 closed all 5 in-scope
P-issues.  This plan captures the residual simplifications that
were noted as worth doing but out-of-scope for plan-09 itself.

**Scope:** Three tiers of simplification, sized for incremental
landing.  Tier 1 is correctness (latent Span-miss walker bugs);
Tiers 2-3 are structural cleanup that reduces the maintenance
surface for future codegen work.

## Why now

Plan-09 + plan-11 delivered:
- 5 P-issues closed (P200, P202, P203, P204, P205)
- `OpEmitter` registry framework + 5 production custom emitters
- `ParShape` runtime consolidation
- 7 memory entries codifying patterns

But the `dispatch.rs` special-case match still has 24 arms; the
`#rust"..."` template path coexists with the registry; and the
walker audit during plan-11 surfaced ≥5 sites with the same
Span-miss pattern that caused P204.  This plan addresses those.

## Tiered structure

| Tier | Cost | Value | Phases |
|---|---|---|---|
| 1 — Correctness + cleanup | ~45 min total | High (latent bugs + dead-weight removal) | 01, 02 |
| 2 — Structural cleanup | ~1-2 sessions | High (reduces dispatch.rs special cases; clarifies narrow_int_cast) | 03, 04, 05 |
| 3 — Deep refactor | 2-3 weeks | High (unifies template + emitter paths) | deferred to plan-13 |

## Phases

| # | Phase | Tier | Value | Status |
|---|-------|------|-------|--------|
| 01 | [Walker audit (`pre_eval.rs`)](01-walker-audit.md) | 1 | latent Span-miss bugs | DONE (2026-05-02) |
| 02 | [Retire `forwarding_smoke.rs`](02-forwarding-smoke-retire.md) | 1 | dead-weight in registry | DONE (2026-05-02) |
| 03 | [Migrate format/append dispatch arms](03-dispatch-format-append.md) | 2 | 12 dispatch.rs arms → custom emitters | OPEN |
| 04 | [Migrate free/record dispatch arms](04-dispatch-free-record.md) | 2 | 10 dispatch.rs arms → custom emitters | OPEN |
| 05 | [Split `narrow_int_cast` dual role](05-narrow-int-cast-split.md) | 2 | param vs block-tail narrowing | OPEN |
| 06 | [`#rust"..."` template migration plan stub](06-rust-template-migration-stub.md) | 3 | relocated to `deferred/13-rust-template-migration/` (2026-05-02) | RELOCATED |

## What stays (out of scope)

- **The `OpEmitter` framework itself.**  Phase 00 of plan-09 is
  load-bearing; not touched.
- **Bug-fix work.**  All in-scope P-issues are closed.
- **`Value::RawExpr`.**  Sanctioned per phase 00's wart-budget;
  retiring it requires a separate fn-ref dispatch refactor.
- **`#rust"..."` annotations in `default/*.loft`.**  Phase 06's
  stub points at the future plan-13 that handles them.

## Sequencing

Tier 1 (phases 01-02) should land first — they're cheap and
low-risk.  Tier 2 phases (03-05) are independent of each other
and can land in any order or in parallel.  Tier 3 (phase 06)
is a stub pointing at plan-13; do not implement under plan-12.

## Dependency on plan-09 / plan-11

Plan-09 and plan-11 must be merged before plan-12 starts.  Plan-12
operates on the post-merge codebase; doing it pre-merge would
conflict-merge against the active branch.

Per `feedback_branch_after_pr_only.md`: plan-12 opens its own
branch from `main` AFTER plan-09's PR merges.  No exception.

## PR strategy

Plan-12 is small enough to ship as ONE PR (Tier 1 + Tier 2
combined ~3-4 hours of focused work).  Or split:

- **Plan-12a**: Tier 1 (correctness + dead-weight removal).
  Small, fast, low-risk PR.
- **Plan-12b**: Tier 2 (dispatch arm migration + narrow_int_cast).
  Medium PR; structural cleanup.

Decision deferred to plan-12 implementation start.

## Acceptance gate (every phase commit)

```bash
cargo build --release --tests
cargo test --release --test issues 2>&1 | tail -3        # 540/540
cargo test --release --test threading 2>&1 | tail -3     # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3  # 35/35
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"  # 95/95 (plan-09 + plan-11 floor)
cargo test --release --test codegen_emitter 2>&1 | tail -3  # 17 passed
cargo fmt --all -- --check
cargo clippy --tests --release -- -D warnings
cargo check --no-default-features
scripts/p09_fast_gate.sh
```

Per `feedback_zero_regression_tolerance.md`: any regression
aborts the commit.  No exceptions.

## Risks

| Risk | Mitigation |
|---|---|
| Walker audit reveals more bugs than the 5 surfaced | Audit is iterative; document each find, fix in-place, re-run gate.  No predetermined scope ceiling. |
| Dispatch arm migration breaks emission for one Op | Each arm migrates independently; per-arm regression test in `tests/codegen_emitter.rs` catches it. |
| `narrow_int_cast` split surfaces a third role | Stop and document; phase 05 may need sub-phasing. |
| Plan-12 ships in parallel with plan-13 (template migration) and they conflict | Plan-13 doesn't open until plan-12 merges; sequence enforced via plans/README convention. |

## Memory entries (saved during plan-09; relevant here)

- `feedback_forwarding_first_recipe.md` — pre-flight pattern
  for new emitters.
- `feedback_actual_error_survey.md` — survey before
  implementing.
- `feedback_zero_regression_tolerance.md` — no shortcuts.
- `feedback_no_expect_fail_on_pr_bugs.md` — no @EXPECT_FAIL on
  real bugs.

These all apply to plan-12.

## See also

- `plans/finished/09-native-runtime-rewrite/README.md` — parent plan
  whose audit surfaced these simplifications.
- `plans/finished/11-p204-ref-propagation/README.md` — surfaced the
  Span-miss walker pattern that phase 01 of plan-12
  generalises.
