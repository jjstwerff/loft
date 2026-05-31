<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# GOALS.md — what "loft stable" means, made concrete

The north star for loft is **stability**.  That single word has been covering
two independent things, which made progress feel fuzzy.  This document splits it
into four goals, each with a *measurable done-criterion* and an honest current
status, so "are we there yet?" always has a yes/no answer.

> Last reconciled: 2026-05-31.  Update the **Status** lines as goals advance;
> the **Definition** / **Done when** lines change only by deliberate decision.

## North star

**loft is stable = it is correct now AND stays correct as the world changes
underneath it** — new rustc/LLVM toolchains, new platforms, new consumer code
patterns.  The @P383 incident (UB latent for many releases, masked on rustc
1.95, surfaced as silent macOS corruption on 1.96/LLVM 21) is the failure mode
this is defined against: "passes today" is not "stable."

## The two engines (both required; neither finds the other's bugs)

- **Dogfood — "is loft useful?"**  Build real consumers (branch-review viewer,
  tracker indexer, `lib/markdown`, the games moros/dryopea), harvest the
  language lessons, fix the language, ship the harvest.  Finds missing features,
  ergonomic pain, feature-interaction bugs, design inconsistencies — the bugs
  users hit *today*.  (See CLAUDE.md § "Development cadence".)
- **Sanitizer — "is loft safe?"**  Mechanically detect UB (alignment, UAF,
  uninitialised reads, aliasing, leaks) with Miri + the homegrown
  `stack_align_guard` + fuzzing, independent of whether output is currently
  correct.  Finds the *correct-today-corrupt-tomorrow* bugs dogfooding is blind
  to.  (See [plans/future/53-sanitizer-ci-lever](plans/future/53-sanitizer-ci-lever/README.md).)

Goals **C** and **D** are the dogfood engine; goals **A** and **B** are the
sanitizer engine.

---

## Goal A — Soundness (no silent corruption)

**Definition.** loft never produces wrong or corrupt results from undefined
behaviour, and stays that way across toolchain bumps.

**Done when:**
1. The interpreter execution path is **Miri-clean for hard UB** (alignment /
   OOB / use-after-free / uninitialised / leak) on a representative corpus.
2. `stack_align_guard` **zero-fires corpus-wide** (every test binary, not just
   `issues`).
3. A **Miri + guard CI gate is green on `main`**, so new UB lands red the day it
   is committed.

**Status (2026-05-31): partial but real.**  Clusters 3 (store-aliasing), 4
(uninit fn-ref padding), 5 (`free_text` leak) are LANDED *production* fixes;
`p213` is Miri-clean (hard UB) under V2.  But: the guard is only proven on
`issues` (685/0, ubuntu); the Miri gate (`.github/workflows/miri.yml`) is not on
`main`; the corpus-wide Miri/guard sweep is not done.

---

## Goal B — Resolve the stack layout (V1 vs V2), once

**Definition.** Decide and validate the eval-stack layout instead of leaving it
permanently half-flagged.  The aligned stepping + V2 slot allocator removes the
cluster-2 alignment UB structurally; the question was whether to make it the
default or stay on the byte-packed unaligned V1 stack.

**Done when** EITHER:
- **(B1)** V2 is the production default with the full validation in
  [plans/future/53-sanitizer-ci-lever/TESTING.md](plans/future/53-sanitizer-ci-lever/TESTING.md)
  green — full suite under V2 on all 3 OS, differential V1≡V2, guard zero-fires,
  Miri clean, perf within threshold; OR
- **(B2)** an explicit, recorded decision to stay on V1 with the sanitizer gate
  (Goal A) as the safety net, and the V1 alignment UB accepted as
  detected-but-tolerated on the supported platforms.

**Status (2026-05-31): B1 LANDED.**  V2 is the production default (merged in
#235); the V1 allocator + the `LOFT_ALIGN`/`LOFT_SLOT_V2` flag plumbing have been
removed (`plan53-v1-cleanup`), so there is no longer a layout to choose between —
the half-flagged state is gone.  Evidence: all 11 original full-suite failures
closed; full suite green under V2 on ubuntu + macOS-ARM (the `#235` standard CI
is green on all 3 OS); both Miri lanes clean; `stack_align_guard` zero-fires
corpus-wide; V1-vs-V2 perf within ±1% (recursive fib 0.996, measured before V1
removal).  Residual: the standalone V2 3-OS validation matrix
(`v2-validation.yml`) is now redundant with `ci.yml` and can be retired.

---

## Goal C — Cross-platform + cross-backend parity

**Definition.** loft behaves identically on **ubuntu / macOS-ARM / windows**
across the **interpret / native / wasm** backends — no platform- or
backend-specific UB or divergence.  macOS is ARM64 (strict alignment) — the
platform where unaligned reads are real faults and where @P383 hit.

**Done when:**
1. The standard 3-OS CI matrix (`ci.yml`) is green on the working branch and on
   `main`.
2. When/if V2 ships (Goal B1), the V2 3-OS matrix
   (`.github/workflows/v2-validation.yml`) is green too.

**Status (2026-05-31): unverified.**  The current branch has never passed the
standard 3-OS CI (it was even red locally on a one-line fmt diff, since fixed);
the V2 matrix is red on all three OSes (Goal B).

---

## Goal D — Capability via dogfood (the engine that works)

**Definition.** loft keeps gaining the features and ergonomics real consumers
need, and the canonical consumers keep building and running.

**Done when:** the dogfood loop continues to drive releases — the branch-review
viewer, tracker indexer, `lib/markdown`, and the games moros/dryopea build, run,
and their lessons land as language/stdlib improvements before each minor
release.  (This goal is never "finished"; it is "healthy" or "stalled.")

**Status: healthy / ongoing.**  This is the established model (CHANGELOG.md
shows the cadence since 0.8.3); it is what makes loft worth keeping safe.

---

## Priority & sequencing

1. **Ship Goal A's wins now — don't let Goal B hold them hostage.**  Clusters
   3/4/5 + the `execute_log` debug-path fix are sound, ungated, shippable
   production improvements.  Land them on `main` and put the Miri gate live
   there.  This advances A independently of the much larger B arc.
2. **Treat Goal B as its own multi-step arc**, gated on TESTING.md, driven by
   the V2 full-suite validation surfacing and closing failures (2f, native
   tuple, par-fn-ref, …) one sub-cluster per fix — the same loop that closed
   2a–2j / 3 / 4 / 5.
3. **Keep Goal D the agenda-setter.**  The sanitizer engine (A/B) exists to keep
   the dogfood engine (C/D) safe; it should not crowd it out.

## How the goals relate

```
            useful ──────────────► safe
   D (dogfood capability)   A (soundness)
   C (cross-platform)       B (stack-layout decision)
        └── "is loft worth using?" ──┘   └── "is loft safe to keep using
                                              as the world changes?" ──┘
```

Dogfood makes loft *worth using*; the sanitizer makes it *safe to keep using*.
A language that is only one of those is not stable.

## See also

- [CLAUDE.md](../../CLAUDE.md) § "Development cadence — the dogfood loop" — Goal D.
- [plans/future/53-sanitizer-ci-lever/](plans/future/53-sanitizer-ci-lever/README.md)
  — Goals A/B: the sanitizer lever, cluster fixes, the V2 alignment work.
- [plans/future/53-sanitizer-ci-lever/TESTING.md](plans/future/53-sanitizer-ci-lever/TESTING.md)
  — Goal B's exit criteria (what must pass to flip V2 to default).
- [ROADMAP.md](ROADMAP.md) / [PLANNING.md](PLANNING.md) — feature backlog feeding Goal D.
