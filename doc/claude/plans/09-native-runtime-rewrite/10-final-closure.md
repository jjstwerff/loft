# Phase 10 — Final closure

**Status:** OPEN

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

**Action**: add a structural test that locks in the post-plan-09
native suite count.  Without this, future commits can silently
shrink the count back below today's floor.

```rust
// tests/codegen_emitter.rs
#[test]
fn native_suite_floor_holds() {
    // After plan-09 phases 06 + 07 + 10 all DONE:
    //   native_dir:           30/30
    //   native_scripts:       91/93 (or 93/93 if P204 closed)
    //   native_binary_script: PASS
    //   native_tuple_*:       PASS
    //
    // Adjust expectations as plan-11 (P204) ships.  This test's
    // job is to catch silent regression: any commit that drops
    // the count below today's floor must update the floor
    // EXPLICITLY, not silently.
    //
    // Implementation: spawn `cargo test --test native --
    // --test-threads=1`, parse the "native result: ..." lines,
    // assert each meets/exceeds the recorded floor.
    ...
}
```

**Validation**: test passes today; future commits that regress
are blocked at PR time.

### Step 10.6 — Plan-09 retrospective (08a equivalent)

**Action**: write the retrospective in
`08a-introspect.md` (the existing 08a doc; populate its
Findings section).  Cover the whole arc:

- 14 phases planned (00-09 + a-suffixes); 9 actually shipped
  (00, 00a, 01, 03, 04, 06, 07, 09, 10).  02/02a demoted, 05/05a/
  08/08a folded into 10.
- 4 P-issues addressed; 3 closed (P202, P203, P205); 1 closed
  via this phase (P200).
- Net code delta: ~XXX lines added (emitters + runtime fns +
  tests), ~YYY retired (special-case dispatch arms,
  hand-aligned tables).  Compute at retrospective time.
- 5 memory entries saved (forwarding-first, phase-doc trait
  drafts, actual-error-survey, branch-after-PR, no-EXPECT_FAIL).
- Wart-budget gates held throughout.
- Lessons for future codegen plans: consolidate tail phases
  early; bug-fix phases need actual-error survey; trait reuse
  requires per-impl uniformity.

Decision criteria for "ship plan-09":
- All plan-09 phases marked DONE in README.
- P200 + P202 + P203 + P205 all CLOSED in PROBLEMS.md.
- Native suite at recorded floor (with P204's blockers handled
  by plan-11, which is a separate gate).

**Validation**: review.  This commit closes plan-09
administratively.

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

### Step 10.3.b notes — comparison-emission code path

_(populate during step 10.3.b: which file:line emits LHS, which
emits RHS, what context flow exists between them)_

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
