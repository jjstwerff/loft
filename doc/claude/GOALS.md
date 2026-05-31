<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# GOALS.md — what "loft stable" means, made concrete

The north star for loft is **stability**.  This document splits that one word
into four goals.  Each goal carries a **Check** — a command to run or a fact to
observe — so progress is something you *evaluate*, not something you assert.  The
Check is timeless; its result is not recorded here (run it to find out where you
stand today).

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
behaviour, and stays that way across toolchain bumps.  This is a **continuous**
property: it is satisfied by a sanitizer engine that runs and keeps catching new
UB, not by a single clean run.

**Check.**
- `ci.yml`'s per-PR `guard` and `asan` jobs are green on `main` (and on the PR
  under review).
- `cargo test --features stack_align_guard` zero-fires across the whole corpus
  (every test binary, not just `issues`).
- The nightly `miri.yml` Miri gate is green.

Met-and-healthy when all three hold *and keep holding* as new code lands.

---

## Goal B — Release & adoption

**Definition.** loft actually ships on a cadence and is usable by people who did
not write it.  A language that never reaches a stable release, or that only its
authors can run, is not stable in any sense that matters.

**Check.**
- A release tag exists within the project's release cadence (`git tag` →
  latest), each carrying a CHANGELOG.md entry.
- `loft install <name>` resolves and fetches a published library end-to-end
  against the registry.
- A clean-machine on-ramp — install → first program → `use` a library —
  completes from the docs alone.
- *Adoption proxy* (no clean counter exists): at least one library has been
  published by someone outside the project.

---

## Goal C — Capability via dogfood

**Definition.** loft keeps gaining the features and ergonomics real consumers
need, and the canonical consumers keep building and running.

**Check** — the consumer build matrix; each row is a command, score it `N/total`:
- branch-review viewer builds and runs on HEAD;
- tracker indexer (`make index`) succeeds;
- `lib/markdown` suite passes;
- the games moros / dryopea build and run against current loft;
- the last release's CHANGELOG carries a consumer-driven harvest section.

Never "finished"; it reads as a fraction, and a falling fraction is the alarm.

---

## Goal D — Cross-platform + cross-backend parity

**Definition.** loft behaves identically on **ubuntu / macOS-ARM / windows**
across the **interpret / native / wasm** backends — no platform- or
backend-specific divergence.  macOS-ARM is the strict-alignment platform where
unaligned reads are real faults; the three backends are three independent
implementations of the same language semantics.

**Check.**
- The 3-OS CI matrix (`ci.yml`) is green on `main`.
- A differential run executes one shared corpus on interpret / native / wasm and
  asserts **identical output and diagnostics** — zero divergences.  Per-backend
  green is *not* the criterion; agreement between backends on identical input is.

---

## The two floors — why dogfood is paused, and when it resumes

The dogfood loop is the agenda-setter (CLAUDE.md § "Development cadence"), but it
is currently **paused by deliberate decision** — because the loop did its job and
hit two walls:

- it **kept surfacing instability** → the **soundness floor** (Goal A);
- it **fought the lib/package structure** → the **structure floor** (Goal B's
  packaging half).

Building a game on either un-cleared floor is building on sand.  So Goal C (and
the game work specifically) is gated on A and B — *by choice, not neglect*.

The danger of a deliberate pause is that open-ended floors make it permanent by
inertia: soundness can always absorb one more sanitizer leg, packaging one more
polish.  So each floor has an **explicit resume-bar tied to what a game actually
needs** — not "all of A" or "all of B":

- **Soundness floor — cleared when:** the sanitizer gate is green on `main`
  **and** the curated Miri/ASan set covers the surfaces the games exercise (eval
  stack, store claim/copy/resize, vectors, fn-refs, text).  *Not* "every Goal-A
  coverage leg shipped."
- **Structure floor — cleared when:** the libraries a game depends on (graphics,
  game_client / game_protocol, server) are extracted, installable, and
  version-stable through the registry.  *Not* "the whole package toolchain
  polished."

When both bars read true, the pause ends and the dogfood loop goes back to
setting the agenda.  Until then, A and B are the work *because* C and D asked for
them.

---

## How the goals relate

```
            useful ──────────────────► safe
   C (dogfood capability)      A (soundness)  ┐
   B (release & adoption)      D (parity)     ├── the two floors C/D
        │                                     ┘   are gated on
        └── paused until the soundness floor (A) and
            structure floor (B) clear, then resumes as agenda-setter
```

Dogfood makes loft *worth using*; the sanitizer makes it *safe to keep using*.
A language that is only one of those is not stable — and right now the dogfood
loop is deliberately waiting on the two floors it asked the sanitizer/packaging
work to build.

## See also

- [CLAUDE.md](../../CLAUDE.md) § "Development cadence — the dogfood loop" — Goal C.
- [plans/finished/53-sanitizer-ci-lever/](plans/finished/53-sanitizer-ci-lever/README.md)
  — Goal A: the sanitizer lever and the stack-alignment work.
- [plans/future/55-program-level-fuzzing/](plans/future/55-program-level-fuzzing/README.md)
  — Goals A/D: program-level fuzzing feeds both the UB sweep and the
  cross-backend differential.
- [plans/future/56-sanitizer-coverage-expansion/](plans/future/56-sanitizer-coverage-expansion/README.md)
  — Goal A continuing: remaining sanitizer-coverage items.
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction/README.md) — Goal B's structure floor: the package-ecosystem extraction.
- [PKG_REGISTRY.md](PKG_REGISTRY.md) / [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md) — Goal B's registry on-ramp.
- [ROADMAP.md](ROADMAP.md) / [PLANNING.md](PLANNING.md) — feature backlog feeding Goal C.
