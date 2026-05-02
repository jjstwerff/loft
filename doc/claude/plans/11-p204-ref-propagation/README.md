# Plan 11 — P204: tail-expression return discarded

**Status:** OPEN

**Closes:** **P204** (native: tail-expression `return inner_call()`
from a struct-returning function emits the inner call as a void
statement and returns the null sentinel).

**PR gate:** P204's two failing native tests (`85_yield_resume`,
`87_store_leaks`) BLOCK PR-readiness.  Per
`feedback_no_expect_fail_on_pr_bugs.md`, the bug must be actually
fixed — @EXPECT_FAIL is not an acceptable resolution.  Plan-11
must complete before any PR opens.

## Out of scope for plan-09

Plan-09 covered codegen-layer P-issues (P200, P202, P203, P205)
that ride on the per-Op emitter dispatch framework.  P204 is
parser-side scope analysis — `collect_hidden_ref_args` /
`__ref_*` placeholder propagation through `Block` arms and `Call`
resolution.  No emitter change closes it; the fix is in the
parser / IR layer.

## Diagnosis

Symptom + reproducer + diagnosis live in
[PROBLEMS.md § 204](../../PROBLEMS.md#204-tail-expression-return-of-inner-helper-call-discarded).

### Root cause

When a struct-returning function ends with `return inner_call()`,
the parser injects a hidden `__ref_*` placeholder argument that
the inner call writes its result into.  The interpreter routes the
result through the `__ref_*` mechanism so the caller's surrounding
code sees the populated struct.

Native codegen's tail-expression handling SKIPS the `__ref_*`
threading: it emits `n_inner(cell, args)` as a void statement
and then `return DbRef { store_nr: u16::MAX, rec: 0, pos: 8 }`
(the null sentinel).  The caller does
`let _src = n_wrap(...); OpCopyRecord(_src, var_q, ...)` —
`_src` is null, OpCopyRecord panics with index-out-of-bounds at
`src/database/allocation.rs:347`.

### Reproducer

`tests/scripts/repro_p204.loft` (currently `@EXPECT_FAIL` marked
— must be un-marked when plan-11 closes).

### Why this is not in plan-09

The fix touches:
- `src/parser/control.rs::collect_hidden_ref_args` (or its
  callers) — parser-side scope analysis that decides whether a
  Call's result threads through a hidden ref placeholder.
- Possibly `src/parser/expressions.rs` or `src/parser/objects.rs`
  for the tail-position detection.
- Possibly `src/data.rs` or `src/scopes.rs` for the IR
  representation of `__ref_*` propagation through Block arms.

None of these are codegen-emitter changes.  Plan-09's
infrastructure does not help close P204; a different design is
needed.

## Detailed steps with validation

### Step 11.1 — Actual-error survey

**Action**: per `feedback_actual_error_survey.md`, do the survey
BEFORE writing implementation steps.

```bash
mkdir -p /tmp/p11-survey

# Native emit for both failing tests:
cargo run --bin loft --release --quiet -- \
    --native-emit /tmp/p11-survey/85-yield-resume.rs \
    tests/scripts/85-yield-resume.loft
cargo run --bin loft --release --quiet -- \
    --native-emit /tmp/p11-survey/87-store-leaks.rs \
    tests/scripts/87-store-leaks.loft

# The minimal reproducer:
cargo run --bin loft --release --quiet -- \
    --native-emit /tmp/p11-survey/repro_p204.rs \
    tests/scripts/repro_p204.loft

# Run native tests; capture the actual panics:
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep -B5 -A30 "85_yield_resume\|87_store_leaks\|repro_p204" \
    | tee /tmp/p11-survey/runtime-errors.log
```

Document per failing site:
- Function name + file:line where the tail-expression `return inner_call()` lives
- Generated Rust shape (the void-statement + null-sentinel pattern)
- Caller's consumption pattern (which OpCopyRecord / OpGetField / etc. panics)

**Validation**: produces a list of all (function, tail-call,
caller-consumer) triples.  Confirms whether all three failing
tests share the same shape or split across multiple.

### Step 11.2 — Identify the parser-side decision point

**Action**: trace the parser path that decides whether a
tail-expression Call gets `__ref_*` threading or void-emission.

```bash
grep -rn "collect_hidden_ref_args\|__ref_\|filter_hidden\|tail.*expression\|return.*Call" \
    src/parser/ src/data.rs | head -30
grep -rn "ref_return\|hidden.*ref" src/parser/control.rs | head -20
```

Document in step 11.2's notes:
- Where `__ref_*` injection happens for non-tail-expression cases
  (the working pattern).
- Where tail-expression detection lives (or whether it's missing).
- What context flow the parser needs to know "this Call is in
  tail position of a struct-returning fn."

**Validation**: identify exactly the file:line pair where the
tail-position decision needs to be made, and whether the parser
already has the context.

### Step 11.3 — Pin the prior failure mode

**Action**: write a regression test that runs `repro_p204.loft`
under native and asserts exit 0.  Add to
`tests/codegen_emitter.rs`:

```rust
#[test]
fn p204_tail_expression_return_passes_under_native() {
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--quiet", "--",
               "tests/scripts/repro_p204.loft"])
        .status().unwrap();
    assert!(status.success(),
        "P204: tail-expression return regressed");
}
```

This commit lands FIRST (test failing) so the fix commit shows
red→green.

**Validation**: test fails today; that's the regression guard.

### Step 11.4 — Apply the fix

**Action**: based on step 11.2's findings, implement the fix.
Likely shapes (decided by survey, not pre-committed):

- **Option A**: extend `collect_hidden_ref_args` to walk through
  `Value::Block`'s tail expression when it's a Call returning
  the same struct type as the enclosing fn's return type.
- **Option B**: add an explicit "tail-position __ref_ propagation"
  pass at parse time that rewrites
  `return inner_call()` into `__ref_* = inner_call(); return __ref_*`.
- **Option C**: defer to native codegen (`src/generation/...`) —
  detect the tail-position pattern and emit
  `let _ret = n_inner(cell, args); return _ret;` instead of the
  void-statement + null-sentinel pair.

Option A is parser-side and consistent with the existing
mechanism.  Option B is a more invasive parser change.  Option C
is codegen-side and works around the parser bug rather than
fixing it.  Choice depends on which option's required context is
already available where it needs to be.

**Acceptance**:
```bash
cargo test --release --test codegen_emitter::p204_tail_expression_return_passes_under_native
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"
# Expected: 91/93 → 93/93 (after plan-09 phase 10 closes 20_binary,
# plan-11 closes 85+87)

cargo test --release --test issues 2>&1 | tail -3   # 540/540
cargo test --release --test threading 2>&1 | tail -3   # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3   # 35/35
scripts/p09_fast_gate.sh   # byte-identical or refresh w/ intentional change
```

### Step 11.5 — Remove @EXPECT_FAIL from `repro_p204.loft`

**Action**: the reproducer was @EXPECT_FAIL'd while P204 was
out-of-scope.  After the fix lands, remove the marker so the
test runs normally.

```bash
# Edit tests/scripts/repro_p204.loft — remove the @EXPECT_FAIL
# header line.  Run the native suite to confirm it passes.
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "repro_p204"
# Expected: pass (no skip, no fail).
```

**Validation**: repro_p204 in the native suite reports as a normal
pass, not a skip.

### Step 11.6 — Update PROBLEMS.md

**Action**: mark P204 CLOSED with the actual fix-path narrative
and the option chosen in step 11.4.  Reference the regression
test added.

**Validation**: review.

### Step 11.7 — Move plan-11 to finished/

**Action**: per `plans/README.md` convention, move
`plans/11-p204-ref-propagation/` to `plans/finished/` once all
phases are committed.

**Note**: this happens at PR-open time, alongside plan-09's
similar move.  Listed for plan completeness.

## Acceptance for plan-11 overall

```bash
cargo test --release --test codegen_emitter::p204_tail_expression_return_passes_under_native
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"
# Expected: 93/93 (assumes plan-09 phase 10 already closed P200)

cargo test --release --test issues 2>&1 | tail -3   # 540/540
cargo test --release --test threading 2>&1 | tail -3   # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3   # 35/35
cargo test --release --test codegen_emitter   # all green
```

PR-readiness gate after plan-11 closes: branch's native suite
parity with main (5/5 high-level + 0 sub-failures + 0
@EXPECT_FAIL'd P-issue tests).

## Estimated cost

- Step 11.1 (survey): 30 min — pure investigation.
- Step 11.2 (locate decision point): 1-2 hours — tracing
  parser-side logic.
- Step 11.3 (regression test): 5 min.
- Step 11.4 (apply fix): 2-8 hours depending on option chosen.
  Option A is smallest; option B is structural.
- Step 11.5 (un-mark): 5 min.
- Step 11.6 (PROBLEMS.md): 10 min.

Total estimate: **half-session to one full session** (3-12 hours)
depending on what step 11.2 surfaces.  The dominant uncertainty
is the parser-side architecture — if the existing infrastructure
doesn't have the tail-position context readily available, the
fix gets larger.

If the survey reveals the parser context is genuinely missing, a
sub-phase 11.4b may need to add it (estimate +1 session).

## Risks

| Risk | Mitigation |
|---|---|
| Parser fix cascades — touching `collect_hidden_ref_args` or `__ref_*` propagation breaks an unrelated test surface | Run the full suite (issues + threading + native + codegen_emitter) after every commit.  P204's reproducer in `repro_p204.loft` AND the actual failing tests (`85_yield_resume`, `87_store_leaks`) must both pass; one without the other means partial fix. |
| Step 11.2 reveals the fix needs a separate refactor first | Pause plan-11; design the prerequisite refactor as a sub-phase; resume after it lands. |
| The tail-position pattern has multiple shapes (not just `return inner()`) | Step 11.1's survey must enumerate all patterns; step 11.4's fix must cover all of them.  Add a regression test per pattern. |
| Native suite shrinks due to reachability changes | `native_suite_floor_holds` (added by plan-09 phase 10 step 10.5) catches this.  |

## Step 11.2 notes — parser-side decision point

_(populate during step 11.2 — file:line of tail-position
detection, what context is available, what context is missing)_

## Problems encountered

_(append per problem)_

## Implementation notes

_(append per non-obvious decision — including the option chosen
in step 11.4 and why)_
