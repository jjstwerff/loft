<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN154 phase 4 — the yield, and what the corpus is made of

**Question.** 264 guards in `tests/scripts/` record a real `@falsified-at:` ref, each naming a
BUILD the guard was written to catch.  On how many of the defects this project already knows
about does the shadow speak — and, the other direction, does it speak on any build its guard
calls clean?

**Verdict — pending the full run; the sample says the answer is thin, and says why.**  The
refs are 200 distinct builds, so the full sweep is hours of machine time rather than a
session.  What the sample already establishes is the GATE direction (no false positive) and a
fact about the corpus that changes how the rest should be ordered.

---

## The two directions, and which one is the gate

* **The yield** — a report on the CONTROL is a defect this instrument would have caught.  It
  is a REPORT, not a threshold: a detector with a narrow class is not thereby a bad detector.
* **The gate** — a report on HEAD, where the guard passes, is a FALSE POSITIVE and is red.
  loft#1373 / #1377 / #1384 are excepted and carry their own reason: they are still open, so
  HEAD is a broken build for those.

## Three ways this measurement can be vacuous, and what each cost

Every one of them was hit before the numbers below were trusted.

1. **The wrong entry point.**  `falsify.sh` records it and the driver repeated it anyway: the
   corpus runner runs `main` when the file has one and every zero-parameter function
   otherwise, so a `main`-less guard run as a plain `--interpret` program executes almost
   nothing.  Measured on `a-nullable-local-…`: 19 test functions under `--tests`, exit 0
   having run none of them under a bare `--interpret`.  **16 of the first 48 rows were scored
   `silent` on a run that never happened.**  The entry point is now derived from the file.
2. **A cached binary pointed at a deleted tree.**  The sweep removes each worktree to bound
   the disk, and `shadow-control.sh` returned the worktree as the `--path`.  On a second run
   the binary is cached, the worktree is gone, and every control run exits 1 with *"cannot
   load standard library"* — which prints nothing the grep matches, so the zero reads as
   evidence.  The stdlib is now copied beside the target, and a run that cannot load is
   scored `VACUOUS`, never `silent`.
3. **No positive control.**  A sweep of a detector needs one ref it MUST catch, or a clean
   sheet is unreadable.  `64437246` — the build phase 1 was falsified against — is prepended
   to every run whatever the cut, and scores `CAUGHT`.

## What the sample found

The calibration ref scores `CAUGHT` (4 reports on the control, 0 at HEAD) and its two
siblings at the same ref score `silent` — different defects on one build, which is what a
per-guard row is for.  Over the ten highest-coverage refs the earlier pass scored **48 guards,
all silent, no false positive**; that number is not the yield, because 16 of the 48 were the
entry-point vacuity above.  What the run does establish, on every row it produced, is the GATE
direction: not one report on a build its guard calls clean.

The silence has a reason, and it is what the run is really measuring:

**The corpus's biggest ref-clusters are TYPING defects.**  The two largest — `8498fdf1` with
ten guards and `964bab93` with nine — are the nullable-model and the value-position-`match`
families, where the control build REFUSES the program and no operator ever runs.  A
memory-state shadow cannot speak about a program that does not execute, and phase 1 had
already found the same thing one guard at a time: loft#1386's control materialises an explicit
null and is fully written.

So **ordering the full run by coverage buys the least evidence**, which is the opposite of what
the driver was written to do.  The refs worth taking first are the ones whose guards actually
RUN and whose defects are memory-shaped; ordering by coverage puts the refusal families at the
front.  The driver keeps coverage order because a partial run should still be reproducible and
explainable, and the correction belongs in the ordering rather than in a filter that hides
rows.

## Running it

```bash
bash doc/claude/plans/154-stack-shadow/phase4-yield.sh <outdir> [max-refs]
```

`LOFT_HEAD_BIN` / `LOFT_HEAD_PATH` point the HEAD side at a COPY of the binary — the run takes
hours and any gate that starts meanwhile rebuilds `target/release/loft`, so without it the
first ref and the last are scored against different binaries.
