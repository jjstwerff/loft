<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Fuzz-proof gate — prove the store-lifetime class closed *by construction*

> **Part of [@PLN85](README.md)** (store-lifetime retirement). **Status:** SLOT OPEN —
> design, not built. **This is wide-release gate 1** (the floor that does not betray you,
> [GOALS.md § The deeper aim](../../GOALS.md); the bar lives in
> [STABILITY_ROADMAP.md § the wide-release bar](../../STABILITY_ROADMAP.md)). Written as a
> `design-protocol` hypothesis: the fuzz-proof is the **falsification instrument** for the
> claim "the store-lifetime class is closed," designed to BREAK the invariant, not confirm it.

## Why a separate slot when @PLN85 already closed each cluster

The investigation reached outcome (b): the store-lifetime bugs are *independent mechanisms*,
each fixed at its own chokepoint with a per-cluster regression guard (clusters II / III / V /
C / 462 — see [README.md](README.md)). Those guards prove the **known shapes stay fixed**.
They do **not** prove an **unknown composition** can't violate the invariant. "No new report
this week" is anecdotal silence, not proof — and at one dogfooding agent the class kept
spawning bugs precisely because each new *composition* found a hole the last fix didn't cover.

This slot closes that gap: turn "every known dangerous shape is fixed + guarded" into "the
**class** is closed by construction," proven by a standing generative instrument run at scale.
That is the difference between *quiet* and *sealed* — and only *sealed* clears the gate.

## The one invariant being proven

(From [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md); the `deps` ownership chokepoint.)

> At every program point each heap store has **exactly one owner**; all mutation flows through
> that owner; a non-owning alias is **read-only** and **never outlives** its owner.

The whole class is this one invariant violated four ways — the instrument must catch all four:

| Violation | Observable signal the oracle checks |
|---|---|
| **leak** — ownership dropped, no free | store count grows across runs (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`) |
| **double-free** — ownership duplicated | native sanitizer hit / store-table corruption |
| **use-after-free** — alias outlives owner | sanitizer hit / OOB store index (the `65535` family) |
| **silent corruption** — two owners mutate one store | cross-backend **value + length divergence** (the #437 NRVO shape) |

Silent corruption is the dangerous one: no crash, no leak — only a wrong value. So a leak-and-
crash oracle is **insufficient**; the cross-backend value diff is the load-bearing check.

## The instrument (what to build)

A generative harness that emits **random valid loft programs** over the ownership-composition
space, runs each through every oracle, and turns any finding into a minimized regression.

- **Generator grammar — seed from the existing corpus.** The `probes/` directory already
  encodes the known dangerous shapes (matrix A–F, borrowed-view, adopt-free, 462, coalesce);
  the generator's grammar must *reach at least those*, then mutate/compose beyond them.
  Composition axes (the bug-bearing ones from the clusters):
  `delivery {return | bind | arg-pass | field-store}` ×
  `source {local | param | borrowed-view (v[i] / match-arm / if-arm) | nested}` ×
  `value {dense vector | nullable | struct | enum | hash}` ×
  `churn {none | reuse-slot | par}` × `backend {interpret | native+ASan}`.
- **Oracle (all four, every program):** (a) cross-backend **value + length** equality, interp
  as the reference; (b) **zero leak** on both backends; (c) **zero sanitizer finding** on the
  ASan/UBSan native build; (d) **clean process exit** (the teardown-crash trap — "PASSED prints"
  is not enough, check the exit code).
- **Minimize + graduate.** Each counterexample shrinks to a `tests/scripts/85-*.loft` regression
  (the same per-cluster guard mechanism, now fed by the fuzzer instead of by hand).
- **Don't reinvent the harness — focus the existing instrument plans.** @PLN53 (program-level
  fuzzing) and @PLN54 (sanitizer coverage expansion) are the standing instruments; the store-level
  fuzz harness (store.rs LLRB / coalesce / claims) is the layer below. This slot is the
  **@PLN85-specific focusing** of those onto the ownership invariant + the cross-backend oracle —
  it consumes them, it does not duplicate them.

## Build-order dependency — BLOCKED BY @PLN25 (the load-bearing constraint)

The generator's value grammar and the oracle's ownership model **both depend on the settled
value/null model**: what a value *is* (dense vs nullable) and how it copies vs borrows is exactly
what @PLN25 decides, and ownership flows through the `deps` facts that model defines. Fuzzing a
moving value model proves nothing. This is why earlier @PLN85 attempts flailed — there wasn't
enough of @PLN25 settled to know what to build.

Consequence: **@PLN25 leads.** The vectors-half is settled (dense default, merged), so the
fuzz-proof can **start now on the vectors-settled subset** and **expand as scalars land**
(scalars in flight — see [@PLN25 RESUME.md](../25-nullable-sequences/RESUME.md)). Do not gate the
whole instrument on @PLN25 being 100% done; gate each composition axis on its value-model piece
being settled.

## Done criteria — what "gate met" means

1. The harness runs as a standing job (CI or scheduled) with **zero findings across all four
   oracles, both backends**, over a meaningful budget (N programs / M cpu-hours — set the number
   when the generator exists; record it, no silent cap).
2. **Coverage is non-vacuous:** every historical cluster shape (II / III / V / C / 462) is
   provably within the generator's reachable space, so zero-findings means "covers the known
   class," not "the grammar is too narrow to express the bug."
3. Then the class is **closed by construction** — the wide-release gate-1 definition of
   *stabilized*. Until (1)+(2) hold, the memory model is *quiet*, not *sealed*.

## Status + next action

- **Status:** SLOT OPEN (design). Nothing built yet.
- **Next:** (a) derive the generator grammar from `probes/` + the cluster invariants; (b) stand
  up the cross-backend + leak + sanitizer oracle; (c) wire onto @PLN53/@PLN54; (d) start on the
  vectors-settled subset, minimize the first counterexample to a `85-*.loft` regression.
- **Method gate:** every M+ step runs the `design-protocol` skill; this doc IS the hypothesis.

## See also

- [README.md](README.md) — the @PLN85 clusters this generalizes.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the `deps` invariant the oracle checks.
- [STABILITY_ROADMAP.md § the wide-release bar](../../STABILITY_ROADMAP.md) — gate 1, and the
  @PLN25 dependency.
- @PLN53 (program-level fuzzing) · @PLN54 (sanitizer coverage expansion) — the instruments this consumes.
- [@PLN25 RESUME.md](../25-nullable-sequences/RESUME.md) — the value model this is blocked by.
