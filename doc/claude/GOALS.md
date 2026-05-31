<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# GOALS.md — what "loft stable" means, made concrete

The north star for loft is **stability**.  This document splits that one word
into four goals, each with a *measurable criterion*, so "are we there yet?"
always has a yes/no — or, for the continuous goals, a healthy/stalled — answer.

## North star

**loft is stable = it is correct now AND stays correct as the world changes
underneath it** — new rustc/LLVM toolchains, new platforms, new consumer code
patterns.  The canonical failure mode this is defined against is latent
undefined behaviour: UB that passes every test on today's toolchain and
surfaces as silent corruption after a compiler bump or on a stricter platform.
"Passes today" is not "stable."

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
  to.

The two engines feed four goals: **A** (soundness) is the sanitizer engine;
**C** (capability) and **B** (release & adoption) are the dogfood engine; **D**
(parity) is verified by sanitizer-style differential testing but serves both.

---

## Goal A — Soundness (no silent corruption)

**Definition.** loft never produces wrong or corrupt results from undefined
behaviour, and stays that way across toolchain bumps.

This is a **continuous** property, not a one-time checkbox: it is satisfied by a
sanitizer engine that runs and keeps catching new UB, not by a single clean run.

**Healthy when:**
1. The interpreter execution path is **Miri-clean for hard UB** (alignment /
   OOB / use-after-free / uninitialised / leak) on a representative corpus.
2. `stack_align_guard` **zero-fires corpus-wide** (every test binary, not just
   `issues`).
3. A **Miri + guard + ASan CI gate is green on `main`**, so new UB lands red the
   day it is committed — and continues to fire on UB introduced later.

The aligned eval-stack layout (8-byte transient TOS stepping, byte-packed
locals) is a settled design decision that removes the cluster of stack-alignment
UB structurally; soundness work builds on it rather than re-litigating it.

---

## Goal B — Release & adoption

**Definition.** loft actually ships on a cadence and is usable by people who did
not write it.  A language that never reaches a stable release, or that only its
authors can run, is not stable in any sense that matters.

**Healthy when:**
1. Minor releases keep shipping, each bundling the dogfood harvest, with
   user-facing release notes (CHANGELOG.md).
2. External consumers can install and depend on loft libraries through the
   package registry without building from the monorepo.
3. There is a documented, low-friction on-ramp (install, first program, library
   use) that someone outside the project can follow.

---

## Goal C — Capability via dogfood

**Definition.** loft keeps gaining the features and ergonomics real consumers
need, and the canonical consumers keep building and running.

**Healthy when** the dogfood loop continues to drive releases — the
branch-review viewer, tracker indexer, `lib/markdown`, and the games
moros/dryopea build, run, and their lessons land as language/stdlib improvements
before each minor release.  (This goal is never "finished"; it is "healthy" or
"stalled.")

---

## Goal D — Cross-platform + cross-backend parity

**Definition.** loft behaves identically on **ubuntu / macOS-ARM / windows**
across the **interpret / native / wasm** backends — no platform- or
backend-specific divergence.  macOS-ARM is the strict-alignment platform where
unaligned reads are real faults; the three backends are three independent
implementations of the same language semantics.

**Done when:**
1. **Platform leg.** The standard 3-OS CI matrix (`ci.yml`) is green on `main`.
2. **Backend leg.** A cross-backend differential harness proves
   **interpret ≡ native ≡ wasm** on a shared corpus: the same program produces
   the same output (and the same diagnostics) on all three backends.  Parity is
   the *criterion*, not "each backend passes its own tests" — divergence between
   backends on identical input is the bug class this goal exists to catch.

---

## How the goals relate

```
            useful ──────────────────► safe
   C (dogfood capability)      A (soundness)
   B (release & adoption)      D (cross-backend parity)
        └── "is loft worth using?" ──┘   └── "is loft safe to keep using
                                              as the world changes?" ──┘
```

Dogfood makes loft *worth using*; the sanitizer makes it *safe to keep using*.
A language that is only one of those is not stable.

## See also

- [CLAUDE.md](../../CLAUDE.md) § "Development cadence — the dogfood loop" — Goal C.
- [plans/finished/53-sanitizer-ci-lever/](plans/finished/53-sanitizer-ci-lever/README.md)
  — Goal A: the sanitizer lever and the stack-alignment work.
- [plans/future/55-program-level-fuzzing/](plans/future/55-program-level-fuzzing/README.md)
  — Goals A/D: program-level fuzzing feeds both the UB sweep and the
  cross-backend differential.
- [plans/future/56-sanitizer-coverage-expansion/](plans/future/56-sanitizer-coverage-expansion/README.md)
  — Goal A continuing: remaining sanitizer-coverage items.
- [PKG_REGISTRY.md](PKG_REGISTRY.md) / [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md) — Goal B's registry on-ramp.
- [ROADMAP.md](ROADMAP.md) / [PLANNING.md](PLANNING.md) — feature backlog feeding Goal C.
