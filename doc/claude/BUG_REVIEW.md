<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# BUG_REVIEW.md — the monthly pass that turns a month of bugs into one generalization

> **A report, never a release blocker.** Like the two documentation reviews it
> rides beside ([LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md)), this pass says
> what the month's bugs have in common and stops there. Whether a shared cause is
> worth collapsing is a judgement it deliberately does not make.

## Why a by-hand pass exists

Bugs get fixed as they arrive — that is the standing rule in
[STABILITY_ROADMAP.md](STABILITY_ROADMAP.md), and it is not what this pass is for.
Fixing a bug answers *"is this case right now?"*. It cannot answer the question that
decides whether next month is quieter:

> **Did this bug come from a place that will keep manufacturing bugs?**

That question is invisible one bug at a time. A duplicated case analysis produces
one defect per forgotten case, each looking unrelated, each fixed correctly, and the
duplicate survives every one of those fixes. Only the month's bugs *in aggregate*
show the shape — which is why this is a monthly pass and not a per-fix step.

The goal is a conversion, not a count: **one month, one class, one generalization.**
A cycle that fixes forty bugs and collapses no duplicate has not reduced next
month's forty. See [STABILITY_REDFLAGS.md § The one thesis](STABILITY_REDFLAGS.md).

## Cadence and scope

- **When:** once per monthly cycle (the `YYYY-MM` branch), before tagging — the same
  beat as the documentation review, and for the same reason: it needs a month of
  evidence to read.
- **Who:** one reviewer per pass, human or a steered agent. The watermark carries
  state, so a pass can be split or skipped without losing the thread.
- **What:** every `bug`-labelled issue in the tracker, open and closed. Closed ones
  carry most of the signal — a closed bug is a mechanism someone already diagnosed,
  which is exactly what makes title-matching work here.
- **Cost:** a quiet month is fifteen minutes. The aid does the counting; the pass is
  reading one table and making one call.

## The pass

### 0. Pre-flight (automated — run it first)

```bash
make bug-review                       # fetches from gh and reports
make bug-review ARGS="--bands 6"      # finer time slicing on a busy cycle
scripts/bug-review.py --cache i.json  # re-run offline from a saved fetch
```

Four sections come back: the population, each mechanism class's share over time,
the payoff check on keystones already landed, and enumeration exposure. None of them
is a verdict.

### 1. Pick ONE rising class

Read section 2 of the report. A class marked `RISING` is still producing bugs; a
class marked `falling` has either been fixed structurally or gone out of fashion.
**Pick one class, not three.** The output of this pass is a single conversion, and
picking three reliably produces none.

Prefer the class that is both rising and *cheap to trace* — one with three or four
bugs whose titles name the same mechanism beats one with twenty that merely share a
subsystem.

### 2. Find the duplicated case analysis behind it

The class names a symptom; this step finds the place. Group `match` blocks by the
enum they dispatch on and rank enums by how many *independent* blocks re-match their
arm set — the instrument described in
[STABILITY_REDFLAGS.md § Re-survey](STABILITY_REDFLAGS.md). What you are looking for
is one question answered in several places, or one total walk written with a
wildcard.

If the class has no duplicate behind it, say so and stop. Some months genuinely
produce unrelated one-off bugs, and recording that is a real result.

### 3. Ask whether a keystone already exists

Before designing anything, check whether the tree already has the fact and this site
simply did not adopt it. It usually does: `Value::for_each_child`,
`Type::for_each_child`, `IrNode::for_each_child`, `Stores::for_each_owned_child`,
`IntegerSpec::range_to_width`, `DbRef::NULL`, `NarrowIntKind::of`.

**Adoption beats invention.** A second keystone for a fact that already has one is
the duplicate this whole pass exists to remove.

### 4. Decide the disposition

| verdict | what it means | action |
|---|---|---|
| **Collapse** | duplicate of a fact that already has a home | fold the sites onto the keystone |
| **Make exhaustive** | a total dispatch spelled with a wildcard | delete the wildcard so a new variant breaks the build |
| **Keep, but declare** | deliberately partial and correct | add the reason, so the next reader can tell it from an accident |
| **One-off** | genuinely single-site | fix it; there is no class here |

**Keep-but-declare is a real outcome, not a dodge.** A walker that answers `false`
for every shape it does not care about is correct. What is wrong is only that it is
spelled identically to one that forgot — so the fix is a sentence, not an arm.

### 5. Run the payoff check on last cycle's conversion

Section 3 of the report answers it: did the class that got a keystone last cycle
actually get quieter? Record the verdict in the watermark table either way.

A `NO EFFECT` is the most valuable line the report can print — it means the fact
that was landed was not the one manufacturing the bugs, and the premise deserves
re-opening rather than another site being folded onto it.

The check **abstains** when a class had almost no bugs before its keystone landed.
That is not a gap; a class with nothing to fall from cannot demonstrate a fall, and
printing a verdict there would send the next cycle to re-open a premise that was
never tested.

### 6. Record and route

Add a row to the watermark table below. Land XS conversions on the spot. Route M+
ones to [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md) at their priority.

**Do not file the remaining bugs in the class.** Per the roadmap's standing rule the
deliverable is the collapsed structure, and the cases that matter most here are the
ones nobody has hit yet — those have no ticket to file.

## What the report's numbers mean (and three traps built into them)

These are lessons from building the aid; each one produced a wrong answer first.

- **Bucket by issue NUMBER, not close date.** The tracker is young and a release
  close-out lands hundreds of old issues at once. Measured on this tracker, 282 of
  513 closes fell in a single month, which makes every calendar window read as
  "everything is recent".
- **Measure a trend against the PEAK, not the first band.** A class that did not
  exist early, rose, and has since fallen reads as `RISING` when compared to zero —
  and points the cycle at work already done.
- **Exposure is omission rate × usage, not omission rate.** In the walker scan
  `ParFor` is omitted from 87 % of partial walkers and `Tuple` from 72 %, yet `Tuple`
  carried 22 bugs and `ParFor` almost none. The tail is not safe, it is unexercised.
  So section 4 is read as a *forecast*: an often-omitted variant that a consumer is
  about to start using is next month's class.

## Watermark table

One row per cycle. `Class named` is what the pass picked; `Payoff` is filled in by
the NEXT cycle's step 5, which is what keeps the claim honest.

| Cycle | Bands reviewed | Class named | Disposition | Payoff (filled next cycle) |
|---|---|---|---|---|
| `2026-08` | #246–#1029 (334 bugs) | tuple / generic / null → one root: the type-variable fact | Collapse + Make exhaustive ([Cluster F](STABILITY_REDFLAGS.md)) | — |

Retrospective entries, measured when the protocol was written rather than by a pass:

| Cycle | Class | Keystone landed | Payoff |
|---|---|---|---|
| `2026-06` | narrow-int / width | `IntegerSpec::range_to_width` | **9.6 % → 2.0 %** — the one measured payoff so far |
| `2026-07` | keyed collections | `Stores::for_each_owned_child` | cannot judge — the class had no bugs before it landed |

## What this is NOT

- **Not bug triage.** It never decides whether a bug is worth fixing, or fixes one.
- **Not a release gate.** It cannot block a tag. Nothing in it is required to be
  green, because none of it is pass/fail.
- **Not an issue-filing pass.** The opposite: it exists so that a class is collapsed
  instead of enumerated as tickets.
- **Not a substitute for matrix-first.** It says *where* to look. The boundary of any
  defect it points at is still established by probes (CLAUDE.md § Debugging policy),
  and the axes a matrix holds FIXED still have to be counted.

## See also

- [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) — the red-flag map this pass feeds,
  and the worked example of a class traced to its duplicate.
- [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md) — where M+ conversions are ordered,
  and the *fix, don't file* standing rule.
- [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) — the sibling monthly pass; same
  cadence, same report-never-gate status.
- [RELEASE.md § Monthly reviews](RELEASE.md) — where this sits in the cycle.
