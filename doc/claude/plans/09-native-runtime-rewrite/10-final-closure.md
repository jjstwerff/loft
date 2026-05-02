# Phase 10 — Final closure

**Status:** IN PROGRESS

| Step | State |
|---|---|
| 10.1 — phase 05a introspection | DONE — Findings populated 2026-05-02 |
| 10.2 — CHANGELOG entries for 06 + 07 | DONE — entries added 2026-05-02 |
| 10.3 — P200 read-side fix | DONE — `IntCompareEmitter` shipped 2026-05-02; 91/93 native |
| 10.4 — phase 08 redundancy decision | DONE — phase 08 marked SUPERSEDED 2026-05-02 |
| 10.5 — pin native baseline floor | IN PROGRESS — `native_suite_floor_holds` test added (`#[ignore]`'d, opt-in via `--ignored`); needs assertion polish before enforcing |
| 10.6 — plan-09 retrospective (08a equivalent) | OPEN — design-only at this stage; populate after plan-11 closes P204 so retrospective covers the full arc |
| 10.7 — directory move-to-finished | OPEN — happens at PR-open time, after plan-11 + plan-09 both DONE |

**Pace gate (2026-05-02):** the user noted implementation
velocity was getting ahead of the planning discipline.  Steps
10.5-10.6 are deliberately design-only for now; they implement
when the plan-11 outcome (P204 fix) lets the retrospective
cover the full arc, and when the baseline assertion can be
properly tuned against a stable post-plan-11 test count.

**Closes:** **P200** (last plan-09-scoped P-issue) + finalises
plan-09 administratively.

**Consolidates:** phases 05, 05a, 08, 08a — these phases were
sequenced before plan-09's tail collapsed into a single coherent
fix.  Phase 10 absorbs them rather than running each as separate
phase boundaries (which would add ceremony without adding
discipline now that the surface is small).

## Why a consolidated tail phase

After phases 06 + 07 closed P202 + P205, plan-09's remaining
work is:

- **One bug fix**: P200 (read-side comparison emission, per
  phase 05's rewrite).  Phase 08's "read closure" likely
  collapses into phase 05's fix or becomes redundant.
- **One introspection**: 05a's lessons-from-bug-fix-phases
  retrospective.
- **Administrative closure**: CHANGELOG entries for 06+07,
  plan-09 README final pass, 08a-style retrospective, native
  baseline floor pinning.

Splitting these across four phases (05 + 05a + 08 + 08a) means
4 separate plan files, 4 acceptance gates, 4 commit batches —
overhead that doesn't track real risk reduction.  Phase 10
collapses the tail into one plan-doc with sequential steps that
each ship a coherent commit.

## Out of scope (handled separately)

**P204** is NOT closed by phase 10 or plan-09.  P204 has its own
plan at [`plans/11-p204-ref-propagation/`](../11-p204-ref-propagation/).
P204's two failing native tests (`85_yield_resume`,
`87_store_leaks`) BLOCK PR-readiness — they will not be
@EXPECT_FAIL'd to "make the count match" (see
`feedback_no_expect_fail_on_pr_bugs.md`).  Plan-11 must complete
before the merge candidate becomes PR-eligible.

## Dependencies

- All earlier plan-09 phases complete: 00, 00a, 01, 03, 04, 06,
  07, 09 — DONE as of 2026-05-02.
- Phase 02 (param adapter) — demoted by 00a; not a blocker.
- Phase 05's rewritten plan in `05-file.md` § "Detailed steps
  with validation" — this phase 10 references those steps verbatim
  for the implementation portion (so the rewritten plan doesn't
  go to waste).
- `feedback_actual_error_survey.md` memory — phase 10 step 10.2
  uses the survey approach.

## Detailed steps with validation

### Step 10.1 — Phase 05a introspection (fire deferred review)

**Action**: phase 05a's revised trigger ("first framework-based
bug-fix phase DONE") was met when phase 06 shipped, then again
when phase 07 shipped.  Fire 05a now to capture lessons fresh.

Populate `05a-introspect.md` § Findings.  Questions to answer
explicitly (from 05a's existing Questions section, refreshed
with phases 06 + 07 as the subject):

1. **Diagnostic-gate effectiveness**: did phase 06's pre-flight
   (forwarding-first recipe pre-check) and phase 07's probe (the
   skip-removal Outcome A/B test) actually catch the right
   issues before code shipped?
2. **Regression-test-first pattern**: phase 06 added 2 regression
   tests, phase 07 added 2.  Did writing them BEFORE the fix
   change implementation outcomes?
3. **Custom emitter complexity vs estimate**: phase 06 was
   estimated at "~30-50 lines" per emitter.  Actual: queue ~30,
   buf-rename ~10.  Phase 07 estimated similar; actual was a
   different shape (emit.rs patches, not custom emitter).
4. **Surprises that updated downstream phases**: phase 07's
   "two emit sites, not one" finding; phase 06's reachability
   gap.  Did phase 05's rewrite already absorb these?  If not,
   apply them now.

**Time budget**: 1 day max.  Pure doc, low risk.

**Commit**: `plan-09 phase 05a: introspection — bug-fix phase
patterns validated by 06 + 07`.

**Validation**: review the populated Findings; confirm decision
("continue" expected based on 06 + 07 going clean).

### Step 10.2 — CHANGELOG_TECHNICAL entries for 06 + 07

**Action**: `doc/claude/CHANGELOG_TECHNICAL.md` is missing
entries for plan-09 phase 06 (P202 close) and phase 07 (P205
close).  Add them under `[Unreleased]` following the existing
phase 09 + 00a entries' shape.  Each entry:

- Phase number + closure scope (which P-issue)
- Mechanism summary (3-5 bullets)
- Verification numbers (test count delta, suites green)
- Commit reference (8cf0676 + 6151231)

**Commit**: `docs(changelog): record plan-09 phases 06 + 07`.

**Validation**: review.

### Step 10.3 — Phase 05 implementation (P200 read-side fix)

**Action**: implement phase 05's rewritten plan in `05-file.md`
§ "Detailed steps with validation" steps 5.1-5.5 (renumbered
here as 10.3.a-10.3.e for clarity).

Phase 05 already documents the actual-error survey, the candidate
fix sites, and the expected acceptance criteria — phase 10 just
shepherds the implementation.

#### 10.3.a — Survey + diagnose (per `05-file.md` step 5.1)

```bash
mkdir -p /tmp/p10-survey
cargo run --bin loft --release --quiet -- \
    --native-emit /tmp/p10-survey/20-binary.rs \
    tests/scripts/20-binary.loft
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep -A20 "rustc failed for 20_binary"
```

Document per failing site (file:line, LHS shape, RHS shape).
Confirm the read-side block-tail vs i64-literal comparison
pattern holds for all sites.

#### 10.3.b — Identify the comparison-emission code path (per `05-file.md` step 5.2)

```bash
grep -rn 'OpEqInt\|OpNeInt\|op_eq_int' src/generation/ | head
grep -rn 'narrow_int_cast' src/generation/ | head
grep -n '"==" =>' src/generation/ | head
```

Find which fn emits the LHS (block-tail with `as <narrow>` cast)
and which fn emits the RHS literal with `_i64` suffix.  Document
in step 10.3.b's notes section below.

#### 10.3.c — Pin the prior failure mode (per `05-file.md` step 5.3)

Add to `tests/codegen_emitter.rs`:

```rust
#[test]
fn p200_binary_compiles_under_native() {
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--quiet", "--",
               "tests/scripts/20-binary.loft"])
        .status().unwrap();
    assert!(status.success(),
        "P200: 20-binary.loft native compile regressed");
}
```

This commit lands FIRST (with the test failing) so subsequent
commits show the fix flipping the test from red to green.

#### 10.3.d — Apply the fix (per `05-file.md` step 5.4)

Three candidate fix sites identified in `05-file.md`:

- **Option A**: drop block-tail narrow when consumer is `==`
  against fitting constant
- **Option B**: widen the constant at comparison-emission time
  to match LHS narrow type
- **Option C**: cast both sides to common width

Choose based on step 10.3.b's findings (which option's required
context is already available at the relevant emit site).
Document the choice.

**Acceptance**:
```bash
cargo test --release --test codegen_emitter::p200_binary_compiles_under_native
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"
# Expected: 90/93 → 91/93 (P200 sub-failure closed)
cargo test --release --test issues 2>&1 | tail -3   # 540/540
cargo test --release --test threading 2>&1 | tail -3   # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3   # 35/35
scripts/p09_fast_gate.sh   # byte-identical or refresh w/ intentional change
```

#### 10.3.e — PROBLEMS.md + plan README update

Mark P200 CLOSED in PROBLEMS.md with the actual fix-path narrative.
Update plan-09 README phase 05 status to DONE.

### Step 10.4 — Phase 08 redundancy decision

**Action**: re-read phase 08's plan in `08-binary.md` AFTER
step 10.3 lands.  Three outcomes:

- **(a) Phase 08 is redundant** — phase 05's fix closes both
  read AND write side.  Mark phase 08 DONE-AS-REDUNDANT in the
  README; close 08-binary.md with a reference to step 10.3's
  commit.
- **(b) Phase 08 covers a narrow case 10.3 didn't reach** —
  re-scope phase 08 in step 10.4's notes; ship as a separate
  follow-on.
- **(c) Phase 08 needs major rework** — same actual-error-survey
  pattern as phase 05's rewrite.  May add another iteration to
  phase 10.

**Validation**: review.  Document the choice + rationale.

### Step 10.5 — Pin native baseline floor

**Status (2026-05-02):** test scaffold added in commit `d8a3c73`
but kept `#[ignore]`-flagged + assertion logic still needs
correction.  Currently fails when run with `--ignored` because
the floor-detection looks for "native result: 30 passed,"
that doesn't appear (only `native_scripts` emits a `native result`
summary; `native_dir` doesn't).  Step stays IN PROGRESS until
the assertions match how each test reports its outcome.

**Goal**: add a structural test that locks in the post-plan-09
native suite count.  Without this, future commits can silently
shrink the count back below today's floor.

**Assertion design (post-correction)**:

The native suite has 5 high-level tests; only `native_scripts`
prints a "native result: N passed, …" summary line.  The
others (`native_dir`, `native_binary_script`, `native_tuple_*`)
report through the standard `test <name> ... ok|FAILED` line.
The assertion logic must therefore split:

```rust
// Each high-level test name + ok status:
for name in [
    "native_dir",
    "native_binary_script",
    "native_tuple_return_script",
    "native_tuple_script",
] {
    assert!(combined.contains(&format!("test {name} ... ok")),
            "{name} regressed");
}

// native_scripts uses the "native result: N passed, ..." summary:
let script_floor_ok = combined.contains("native result: 91 passed,")
    || combined.contains("native result: 92 passed,")
    || combined.contains("native result: 93 passed,");
assert!(script_floor_ok,
        "native_scripts floor regressed (expected ≥ 91/93)");
```

**Floor today (after step 10.3)**:
- `native_dir`: 30/30 → test reports `... ok`
- `native_scripts`: 91/93 (2 P204 sub-failures remaining)
- `native_binary_script`: ok (was FAILED pre-step-10.3)
- `native_tuple_return_script`: ok
- `native_tuple_script`: ok

**Floor after plan-11 closes P204**: 93/93 for `native_scripts`.
Update the assertion accordingly when plan-11 lands.

**Why `#[ignore]`**: the test spawns the full native suite
(~30s).  Too slow for default `cargo test` runs; opt-in via
`cargo test --test codegen_emitter native_suite_floor_holds -- --ignored`.
Run it manually before commit; CI runs it with `--ignored` if
configured.

**Validation**: test passes today (after assertion correction)
and on every subsequent commit that maintains or beats the
floor; fails immediately when a commit silently regresses.

**Out of scope for this step**: making the test run as part of
default `cargo test`.  Even if expensive, the gate catches
regressions when explicitly run; defaulting it on adds 30s to
every commit's gate which crosses into "test budget" territory
that's a separate decision.

### Step 10.6 — Plan-09 retrospective (08a equivalent)

**Status (2026-05-02):** OPEN — design only.  Populate after
plan-11 closes P204 so the retrospective covers the full
narrative (plan-09 was always sequenced with P204 deferred to
a sibling plan; the retrospective should reflect what actually
happened including how P204 routing decisions played out).

**Action**: populate `08a-introspect.md` Findings section, plus
add a closure summary at the top of plan-09 README.

**Outline (to populate at retrospective time)**:

#### Phase counts vs original plan
- Originally listed phases: 14 (00 / 00a / 01 / 02 / 02a / 03 /
  04 / 05 / 05a / 06 / 07 / 08 / 08a / 09).
- Phases actually shipped (with their commit hashes):
  - 00 (scaffold) — 8f88639 → beb162f range
  - 00a (introspection) — 3febb29
  - 01 (ABI consolidation) — 3dff7c5 → 2005f6e range
  - 03 (parallel-for emitter) — 44693cc + da46db1
  - 04 (key-keyed ops) — 19a7a86
  - 09 (parallel runtime consolidation) — 22070e2
  - 06 (P202 close) — 8cf0676
  - 07 (P205 close) — 6151231
  - 10 (final closure incl P200 close) — d8a3c73 + tail
- Phases demoted/superseded:
  - 02 (param adapter) — DEMOTED by 00a; never implemented
  - 02a — superseded (02 demoted)
  - 05 — folded into phase 10 step 10.3 (read-side scope)
  - 05a — fired late as phase 10 step 10.1
  - 08 — superseded by phase 10 step 10.3 (one fix closed both
    read + write)
  - 08a — folded into this retrospective

Net: 9 phases shipped, 5 phases consolidated/superseded.  Plan
discipline held — every consolidation was documented as it
happened, not retroactively rationalised.

#### P-issue closures
- P203 — closed by phase 00 step 0.7b (template let-bind-on-repeat)
- P202 — closed by phase 06 (`n_parallel_queue*` runtime + emitter)
- P205 — closed by phase 07 (scratch routing for bounded-generic
  text returns)
- P200 — closed by phase 10 step 10.3 (`IntCompareEmitter` widens
  comparison operands to i64)
- P204 — out of plan-09 scope; closed separately via plan-11

#### Code delta
Compute at retrospective time:
- Lines added (emitters + runtime fns + tests + docs)
- Lines retired (special-case dispatch arms, hand-aligned
  tables, write-side scaffolding made redundant)
- Net delta + commentary on whether the simplification cluster
  paid off

Suggested computation:
```bash
git diff --stat $(git merge-base HEAD origin/main) HEAD -- src/ tests/
```

#### Memory entries saved during plan-09
- `feedback_forwarding_first_recipe.md` — pre-flight pattern.
- `feedback_phase_doc_trait_drafts.md` — trait sketches are
  drafts, not specs.
- `feedback_actual_error_survey.md` — survey before
  implementing.
- `feedback_branch_after_pr_only.md` — branch hygiene rule.
- `feedback_no_expect_fail_on_pr_bugs.md` — no @EXPECT_FAIL on
  real bugs.
- `feedback_zero_regression_tolerance.md` — zero tolerance for
  regressions even at multi-year cost.
- `feedback_ci_gate.md` (existing) — extended in 00a's findings
  with per-commit guidance for hot-path edits.

Six new entries + one extended.  Patterns transferable to
future codegen / parser plans.

#### Wart-budget gate behaviour
- `dispatch_op_arm_budget_not_exceeded`: 26 → 24 (phases 03 + 04
  retired 2 arms).  No regressions across the plan.
- `no_unsanctioned_codegen_value_variants`: held; sanctioned
  list `["RawExpr"]` unchanged.
- `parallel_runtime_consolidated`: phase 09 added; held through
  phase 06 + 10.
- `p202_*` + `p205_*` + `p200_*`: bug-specific structural gates
  added per phase; all held.

#### Lessons for future plans
- **Consolidate tail phases early.**  Phases 05/05a/08/08a
  were planned as four separate phases; consolidating into
  phase 10 saved ceremony without losing discipline.  Apply
  to future plans where the tail surface is small.
- **Bug-fix phases need actual-error survey.**  Phase 05's
  original plan targeted a write-side fix; survey revealed
  read-side comparison emission.  Codified in
  `feedback_actual_error_survey.md`.
- **Trait reuse requires per-impl uniformity.**  Phase 09's
  `ParShape` worked because for-par variants share their
  skeleton.  Phase 06's queue variants didn't share enough;
  flat fns won.  Codified in
  `feedback_phase_doc_trait_drafts.md`.
- **Forwarding-first recipe catches dispatch-path traps.**
  Phase 00's runtime smoke test caught it; phases 03/04 used
  it as planned pre-flight.  Codified in
  `feedback_forwarding_first_recipe.md`.
- **Pace gates are real.**  When implementation gets ahead of
  planning discipline, the user's rule is "update plans for
  now only."  Don't accelerate past clarity.

#### PR-readiness gate
- Plan-09 phases all DONE in README ✓
- Plan-09's 4 P-issues all CLOSED in PROBLEMS.md ✓
- Native suite at recorded floor ✓ (after step 10.5 lands)
- Plan-11 closes P204 (separate gate; PR opens after both)

**Validation**: review.  This commit closes plan-09
administratively but does NOT open the PR — that happens after
plan-11 closes P204 + PR-readiness criteria all met (per the
zero-regression rule).

### Step 10.7 — Plan-09 directory move-to-finished

**Action**: per the convention in `doc/claude/plans/README.md`
("When an initiative is fully closed (all phases committed, no
open follow-ups), move its entire subdirectory into `finished/`"),
move `plans/09-native-runtime-rewrite/` to `plans/finished/`.

This step happens AFTER plan-11 closes P204 and the PR opens.
NOT during phase 10 itself — phase 10 only marks plan-09's
phases DONE; the directory move is the "ship it" gesture.

**Note**: this step doesn't run in phase 10 — it runs as part
of the PR commit sequence.  Listed here for plan completeness.

## Acceptance for phase 10 overall

```bash
# Each step's commit lands clean per its own gates.

# After step 10.3 (P200 fix):
cargo test --release --test codegen_emitter::p200_binary_compiles_under_native
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"  # 91/93 (P200 closed; P204 still open)

# After step 10.5 (baseline pin):
cargo test --release --test codegen_emitter::native_suite_floor_holds

# After step 10.6 (retrospective):
# All plan-09 phases marked DONE.
# All plan-09 P-issues marked CLOSED in PROBLEMS.md.

# PR-readiness gate (separate from phase 10):
# Plan-11 closes P204 → native suite 93/93 → main parity → PR opens.
```

## Gate updates per step

| Step | Gate update |
|---|---|
| 10.1 | 05a Findings populated; status DONE.  No code. |
| 10.2 | CHANGELOG entries for 06 + 07 added.  No code. |
| 10.3 | P200 regression test added; native floor moves to 91/93 (or higher if 08 absorbed).  Possible baseline refresh for 20-binary corpus entry. |
| 10.4 | 08-binary.md status decided.  May add a Notes section. |
| 10.5 | New `native_suite_floor_holds` structural test in `tests/codegen_emitter.rs`. |
| 10.6 | 08a Findings populated; plan-09 README phase status final pass. |
| 10.7 | Plan-09 directory moves at PR-open time, not during phase 10. |

## Commit shape

5-7 commits across the steps.  Each step ships independently:

1. Step 10.1 — 05a introspection (1 commit).
2. Step 10.2 — CHANGELOG entries (1 commit).
3. Step 10.3 — P200 fix (3-4 commits: regression test, fix, narrative, baseline refresh).
4. Step 10.4 — phase 08 decision (1 commit, doc only).
5. Step 10.5 — baseline pin (1 commit).
6. Step 10.6 — retrospective + plan-09 closure (1 commit).

## Problems encountered

_(append per problem)_

### Step 10.3 outcome (2026-05-02) — IntCompareEmitter

The actual-error survey confirmed all 5 failing sites are E0308
between block-tail-narrowed LHS and `_i64` literal RHS.

**Code path identified**:
- LHS narrow: `src/generation/emit.rs:900` (block-tail) calls
  `narrow_int_cast(&bl.result)` and emits `(value) as u8`.
- RHS literal: `src/generation/emit.rs:53` emits `Value::Int(v)`
  as `{v}_i64` (via the default Int emission, no
  `i32_literal_context` flag set at comparison sites).
- Comparison: `default/01_code.loft:192` template `@v1 == @v2`
  stitches them — the template has no LHS/RHS type info.

**Fix chosen**: option C from phase 05's plan (cast both sides
to common width).  Implementation: new
`src/generation/ops/int_compare.rs::IntCompareEmitter` that
emits `((lhs) as i64) <op> ((rhs) as i64)`.  `as i64` is widening
for u8/u16/i8/i16 and a no-op for i64.

**Why option C over A/B**:
- Option A (drop block-tail narrow when consumer is `==`): would
  need context flow ("what consumes this block") — not readily
  available at the block-tail emit site.  Would also break
  cases where the consumer DOES need the narrow type (e.g.
  function returns whose signature is narrow).
- Option B (narrow RHS to match LHS): would require detecting
  LHS narrow type at RHS emit time — same context-flow issue.
  Also lossy for non-fitting RHS values (silent truncation).
- Option C (widen both): simplest detection (none needed —
  always wrap), preserves narrow LHS via `(... as u8) as i64`
  (the narrow cast still applies, then widens to i64), and the
  comparison's logical semantics is integer equality so widening
  preserves correctness.

**Registered for**: `OpEqInt`, `OpNeInt`, `OpLtInt`, `OpLeInt`.
`OpGtInt` / `OpGeInt` don't exist — the parser desugars `>` /
`>=` into `<` / `<=` with swapped operands.

**Verified**:
- `tests/scripts/20-binary.loft` compiles + runs under native
  (was: rustc E0308).
- `native_binary_script` test flips from FAILED → ok.
- `native_scripts` count: 90/93 → 91/93 (all 5 P200 sub-failures
  retired).
- threading 43/43, threading_chars 35/35, issues 540/540 unchanged.
- Two regression tests in `tests/codegen_emitter.rs`:
  `p200_binary_compiles_under_native` (behavioural) +
  `p200_int_compare_emitter_registered` (structural).

**Net delta**: 65 lines added (`int_compare.rs` + 2 tests + docs);
0 lines retired.  Below the doubled estimate (60 → 65) — within
ballpark of the 05a-doubling rule.

## Implementation notes

_(append per non-obvious decision)_

### Why "phase 10" not "phase 11"?

Plan-09's phases are numbered within plan-09's own sequence
(00-09).  Phase 10 is the next sequence number.  `plan-11`
(at `plans/11-p204-ref-propagation/`) is a SEPARATE PLAN
covering P204; the "11" is its plan-level identifier in
`plans/`, not a phase within plan-09.

Plan-09 phase 10 + plan-11 are independent: plan-09 closes
its own P-issues; plan-11 closes P204.  Both must complete
before PR.
