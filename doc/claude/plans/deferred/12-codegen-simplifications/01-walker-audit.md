# Phase 01 — Walker audit (`pre_eval.rs`)

**Status:** DONE (2026-05-02)

**Closes:** latent Span-miss bugs in
`src/generation/pre_eval.rs::patch_hoisted_returns` and its
helpers.  Plan-11's @P204 fix surfaced the pattern; this phase
generalises it.

**Tier:** 1 (correctness + cheap)

**Estimated cost:** 30-60 minutes.

## The bug class

Walker patterns in `pre_eval.rs` match against
`Value::Line(_)`, `Value::Set(...)`, `Value::Return(...)`, etc.
directly via `matches!(op, ...)` or `match &operators[i] { ... }`.

Operators in the IR are commonly wrapped in `Value::Span(box (pos,
inner))` — the parser's position-tagging wrapper added at
`src/data.rs:456`.  When a walker matches against the raw `op`
without unspanning, it sees `Value::Span(_)` and bails on the
fall-through `_ =>` arm even though the underlying value matches.

This is **"code that compiled but never executed"** — the walker
infrastructure is correct, but its enabling condition is never
reached because the upstream wrap is invisible.

Plan-11 closed @P204 by fixing one such walker:
`detect_ref_tail_capture`.  Three lines changed; bug closed.

## Sites to audit

Surveyed during @PLAN12 design (2026-05-02) in
`src/generation/pre_eval.rs::patch_hoisted_returns`:

| Line | Walker site | Matches against | Risk severity |
|---|---|---|---|
| 100-107 | `position` for `Return(Var(__ret_*))` | `Value::Return(_)` raw | Skip-collapse — benign |
| 126-132 | `position` for `Set(target, Call/Text)` | `Value::Set(_, _)` raw | Skip-collapse — benign |
| 135-140 | `any` via `value_mentions_var` | recursion | **HIGH — could miscompile** |
| 153-155 | `any` for `Return(Null)` | `Value::Return(_)` raw | Skip Pass 2 — benign |
| 164-167 | `position` for `Return(Null)` | `Value::Return(_)` raw | Skip collapse — benign |
| 170-172 | `rposition` for non-Line non-free | `Value::Line(_)` + `is_free_op` | Wrong target — could miscompile |
| 189-210 | `value_mentions_var` recursion | full match | **HIGH — could miscompile** |

The HIGH-severity sites: `value_mentions_var` returns false on
`Value::Span(_) => _ => false`, missing variable references
inside Span-wrapped operators.  This propagates to
`target_used_between` (line 135-140) which decides whether the
hoisted-return collapse is safe.  An incorrect false → "safe to
collapse" decision → collapse runs → misalignment between Set's
side effects and Return's expected semantics.

The lower-severity sites bail-out skip optimisation but don't
miscompile (they conservatively keep the original IR).

## Detailed steps

### Step 1.1 — Inspect each walker site for actual Span-wrap exposure

Run the survey:
```bash
grep -n "matches!(op\|matches!(.*Value::\|match.*\&operators\[" src/generation/pre_eval.rs
```

For each site, note:
- What it's matching for
- What happens if it bails on Span (skip optimisation vs. wrong decision)
- Whether the trigger conditions actually exercise Span-wrapped IR (run a small repro per site if uncertain)

### Step 1.2 — Patch `value_mentions_var` to handle Span

Add explicit Span-handling at the top of `value_mentions_var`:

```rust
fn value_mentions_var(op: &Value, var_nr: u16) -> bool {
    match op {
        Value::Span(b) => Self::value_mentions_var(&b.1, var_nr),
        Value::Var(v) => *v == var_nr,
        // ... rest unchanged ...
    }
}
```

This is the highest-priority fix per the severity table.

### Step 1.3 — Patch each walker `matches!` / `match` site to unspan

Apply the @PLAN11 pattern to each remaining site:

```rust
// Before:
.position(|op| matches!(op, Value::Set(v, val) if ...))

// After:
.position(|op| matches!(op.unspan(), Value::Set(v, val) if ...))
```

For each of the 6 walker sites, add `.unspan()` before the
match.  All sites are in `pre_eval.rs:96-180`.

### Step 1.4 — Add structural test guarding the unspan

Add to `tests/codegen_emitter.rs`:

```rust
/// Plan-12 phase 01: structural test pinning the unspan-handling
/// pattern in pre_eval.rs walker sites.  Without unspan, walkers
/// silently bail on `Value::Span(_)` wrappers — the bug class
/// that caused P204.  Forbid the pattern from regressing.
#[test]
fn pre_eval_walkers_unspan() {
    let src = std::fs::read_to_string(project_root().join("src/generation/pre_eval.rs"))
        .expect("read pre_eval.rs");
    // Every walker site that matches against Value variants must
    // unspan first.  Detect by counting "matches!(op" / "match &operators[i]"
    // patterns and asserting each is paired with `.unspan()`.
    //
    // This is a heuristic gate — it doesn't catch every form, but
    // it prevents the obvious regressions that would re-open
    // P204-style bugs.
    let walker_lines: Vec<_> = src
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("matches!(op") || l.contains("match operators["))
        .collect();
    for (n, l) in walker_lines {
        assert!(
            l.contains(".unspan()") || l.contains("unspan_mut"),
            "pre_eval.rs:{} — walker site doesn't unspan: `{}`.  \
             Plan-12 phase 01 forbids this pattern; add `.unspan()` \
             before the match (see plan-11 P204 fix for the rationale).",
            n + 1,
            l.trim()
        );
    }
}
```

### Step 1.5 — Run full regression sweep

```bash
cargo test --release --test issues 2>&1 | tail -3        # 540/540
cargo test --release --test threading 2>&1 | tail -3     # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3  # 35/35
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"  # 95/95
cargo test --release --test codegen_emitter 2>&1 | tail -3
scripts/p09_fast_gate.sh
```

Per `feedback_zero_regression_tolerance.md`: any regression
aborts.

### Step 1.6 — Document findings

Append to this doc's § Findings section:
- Which sites were demonstrably broken (had a reproducer)
- Which sites were latent (no reproducer surfaces it today, but
  the fix is still applied as insurance)
- Whether `value_mentions_var`'s Span-bail caused any test
  failure that the audit fixed — if yes, add a behavioural
  regression test alongside the structural one in step 1.4.

### Step 1.7 — Audit OTHER walker patterns in src/generation/

The audit so far covered `pre_eval.rs`.  Other files may have
the same pattern.  Survey:

```bash
grep -rn "matches!(.*Value::\|match.*\&operators\[\|match.*&v\[" src/generation/ | grep -v "test"
```

For each non-`pre_eval.rs` walker site found, repeat the
inspection + patching + structural-test pattern.  Document hits
in this doc's § Audit findings.

## Acceptance

```bash
cargo test --release --test codegen_emitter::pre_eval_walkers_unspan
# + the full regression sweep from step 1.5
```

All green.  Walker pattern is now Span-aware throughout.

## Findings

`pre_eval.rs::patch_hoisted_returns` (the 6 walker sites surveyed in
the table at the top of this doc) and `value_mentions_var` were
**all latent** — no in-tree reproducer surfaces an actual miscompile
today, but the byte-identical baseline test confirms the unspan
adds emit identically when the input IR happens not to be Span-
wrapped, and the unspan kicks in when it is.

The HIGH-severity site in the original triage table —
`value_mentions_var` — was patched defensively as the plan
prescribed; no test failure was uncovered.  Pattern is now
@P204-style insurance, not bug fix.

Over-eager fixes (reverted before commit): patching `needs_pre_eval`,
`create_stack_var`, the deeper `collect_pre_evals_inner` arg-handling
sites, and the `body_is_only_create_stacks` filter all caused the
byte-identical baseline to diverge — the existing emission was
correct because Span wrappers don't reach those leaf sites in
practice.  Lesson: only patch sites the plan explicitly identifies;
each unspan addition is a behaviour change that must be validated
against the byte-identical baseline.

## Audit findings (other files in src/generation/)

The step 1.7 sweep found three more sites worth fixing.  Each
passed the byte-identical baseline test after patching, so they're
applied as insurance against future regressions:

- **`emit.rs:738`** — `Set(__ret_N, _)` walker (`has_ret_temp`
  detection).  Unspans the operator before matching `Value::Set`.
- **`emit.rs:772, 848, 885`** — `Return(_)` walkers in the
  block-emission tail-handling path.  Unspan before matching.
- **`coroutine.rs::detect_yield_from`** — destructures the
  `yield from` desugar pattern.  Five `let Value::* = &lp.operators[i]`
  / `&bl.operators[i]` sites, each now `lp.operators[i].unspan()`.
- **`coroutine.rs::contains_yield`** — recursive walker; Span variant
  added at the top of the match like `value_mentions_var`.
- **`coroutine.rs::collect_segments`** — `inner_op` derivation peels
  Return/Drop wrappers; now also unspans `op` first.

Sites that look similar but were intentionally NOT patched (passed
audit, baseline confirmed unaffected): `dispatch.rs` `to` checks,
`calls.rs:98+121+129+139+274+287` (the same arg-handling sites that
caused emission divergence in `pre_eval.rs`'s parallel structure),
`emit.rs:157+259+573+574+598+799` (codegen-internal IR, not parser
output), `mod.rs:1653+1659` (filter / bool flag, no behaviour
impact), `text.rs:17` (Line filter, no behaviour impact under
current paths).

**Total walker sites audited:** 7 (`pre_eval.rs::patch_hoisted_returns`
+ `value_mentions_var`).  **Total walker sites patched:** 7 in
`pre_eval.rs` + 4 in `emit.rs` + 5 in `coroutine.rs` = **16 sites
across 3 files**.

## Memory candidate

If the audit reveals 3+ confirmed bugs across 2+ files, save:

```
feedback_walker_unspan_pattern.md — When implementing a walker
that matches against `Value::*` variants, always call
`.unspan()` before matching.  Skipping unspan is "code that
compiled but never executed" — the walker is unreachable for
Span-wrapped IR (which the parser commonly produces).  Plan-11's
P204 + plan-12's audit are the case studies.
```

Otherwise @PLAN12 phase 01's findings + the structural test are
sufficient documentation.
