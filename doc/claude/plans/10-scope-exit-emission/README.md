# Plan 10 — Scope-exit gate simplification (deferred)

> **Status: DEFERRED.**  Plan 10 was originally framed as a P203
> fix — that framing was wrong (P203 turned out to be a template
> double-substitution bug in `default/01_code.loft:705`, tracked
> separately in PROBLEMS.md).  The underlying simplification —
> pulling the multi-condition cleanup gate at `src/scopes.rs:1053`
> apart from the dep-tracking system — still has merit, but is no
> longer urgent.  Pick this up when another bug surfaces in the
> gate's territory, when dep-tracking needs maintenance, or when
> a contributor wants a focused codegen-clarity project.

## Goal

Simplify the cleanup-emission half of the dep-tracking system in
`src/scopes.rs`.  Today, OpFreeRef emission at scope exit is gated
by:

```rust
let emit = (dep.is_empty() || is_work_ref) && !in_ret && !function.is_skip_free(v);
```

Three positive conditions, all needed.  The `dep.is_empty() || is_work_ref`
half is an optimisation that masks emission gaps when the dep
tracker mishandles a var.  Removing it relies on the runtime side
already being safe to call OpFreeRef on already-freed slots —
which it is (`src/codegen_runtime.rs:100-104`) — and lets the
cleanup correctness no longer depend on dep-tracking precision.

The gate becomes:

```rust
let emit = !in_ret && !function.is_skip_free(v);
```

Two conditions, both about the var's role (return vs explicitly
suppressed), nothing about the dep-tracker's analysis.

## Why this isn't urgent

Three reasons P-issue urgency drained out of this plan:

1. **P203 is solved by a different fix** (template double-sub at
   `default/01_code.loft:705`).  The original framing — that
   loosening the gate would close P203 — was refuted by the
   strace-driven phase 00 diagnostic.
2. **The runtime already handles the harder case.**  OpFreeRef
   early-returns on `store_nr == u16::MAX`.  `Vec<Option<File>>`
   already provides Drop on slot replacement.  The infrastructure
   the simplification would build on is already in place — there's
   no precondition work blocking later effort.
3. **The current gate works.**  No known bug today is caused by
   the gate's complexity.  Simplification is purely cognitive —
   making the cleanup path easier to reason about for future
   contributors.

## What stays in scope

- **Characterising the gate** (phase 00) — written for whoever
  picks this up later, so they can avoid re-doing the survey work.
- **Loosening the gate** (phase 01) — the actual simplification.
  ~3-line edit + suppression-list audit + structural test.

## What's out of scope

- **Bug fixes** — not driven by any open P-issue.  If a P-issue
  surfaces in this territory, the plan can pick it up; until then,
  the work is purely structural.
- **Replacing dep-tracking wholesale** — only the cleanup-emission
  half is addressed.  Aliasing analysis, closure capture, parallel
  isolation keep dep-tracking unchanged.
- **Drop-based safety net** for file handles — already in place
  via `Vec<Option<File>>`; nothing to do.
- **Runtime no-op fast-path** for OpFreeRef — already in place
  (lines 100, 103); nothing to do.

## Status

| # | Phase | Kind | Status |
|---|-------|------|--------|
| 00 | [Characterise the gate](00-characterize.md) | survey | OPEN — written but no work scheduled |
| 01 | [Simplify the gate](01-simplify-gate.md) | simplification | OPEN — deferred |

Status legend: OPEN → IN PROGRESS → DONE.

## Triggers to revisit

Pick up this plan when any of these happens:

- A new bug surfaces that's gated by `(dep.is_empty() ||
  is_work_ref)` — the simplification would solve a class of
  symptoms at once.
- The dep-tracking system needs maintenance for unrelated reasons
  (e.g., new aliasing analysis requirement) — bundle this
  simplification with that work.
- A contributor wants a small, well-defined codegen-clarity
  project that ships net structural improvement.

Until one of these fires, the plan sits.

## What plan 10 already accomplished (phase 00 history)

The first phase 00 attempt (2026-05-02) was scoped as a P203
diagnostic gate.  It ran the strace + instrumentation diagnostics
that identified P203's actual root cause (template double-
substitution in `default/01_code.loft`).  That discovery is
recorded in `PROBLEMS.md` as P203's fix path.  The diagnostic work
remains in [00-characterize.md](00-characterize.md) under
"Historical context — P203 diagnostic" because the trace data is
useful evidence for whoever picks up the simplification later
(it confirms the runtime safety + Drop infrastructure is solid).

## Acceptance gate (every commit, when work resumes)

```bash
cargo build --release --tests
cargo test --release --test issues 2>&1 | tail -3        # 540/540
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"                              # ≥ baseline
```

## Risks

| Risk | Mitigation |
|---|---|
| Suppression list grows large under the loosened gate | Phase 00 catalogues current suppression cases; if the prediction is high, revisit before phase 01. |
| Loosening the gate exposes a real bug today's gate masks | Acceptance gate runs full suite; regressions abort.  Treat any regression as a real bug to file separately. |
| Plan stays deferred indefinitely | Acceptable — there's no user-visible value at risk.  The triggers above describe when the trade flips. |

## Related

- [P203](../../PROBLEMS.md) — closed by template fix in
  `default/01_code.loft`, NOT by this plan.
- [Plan 09](../09-native-runtime-rewrite/README.md) — per-Op
  emitter rewrite; complementary structural work.
- [LIFETIME.md](../../LIFETIME.md) — dep tracking and scope-based
  freeing design.
