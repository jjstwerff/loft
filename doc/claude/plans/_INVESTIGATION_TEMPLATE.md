<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Investigation-plan template

Copy this file to `<NN>-<slug>/README.md` when opening a plan whose
**primary deliverable is mechanism understanding**, not implementation.

Use this template when the plan's first phases are
"catalogue + diagnose" rather than "design + build" — when you need
to characterize a failure class, family of bugs, or unfamiliar
subsystem before you can commit to a fix shape.

Use the standard [`_TEMPLATE.md`](_TEMPLATE.md) when the primary
deliverable is a feature ship.  The two shapes look different:

| | Standard plan | Investigation plan |
|---|---|---|
| Primary deliverable | Code change shipped to users | Mechanism understanding + fix-design decision |
| Reading order | Goal → Design → Sub-arcs → Implementation | Status → Probes → Cluster docs → Roadmap |
| Sub-files | DESIGN.md, IMPL.md, per-phase files | RESULTS.md, cluster-*.md, probes/ subdirectory |
| Length budget | 100-300 lines per file | 100-300 README + ~200 per cluster doc + probe headers |
| When promoted to tests | When phases ship | When mechanism is pinned and fix lands |

Investigation plans tend to have **more files** than standard plans
(probes + cluster docs + RESULTS + README).  This is justified
*proportionally to the investigation depth* — NOT a default.  A
shallow investigation belongs in `## Open work` on a reference doc;
a deep investigation earns this structure.

The canonical example is `plans/future/51-hidden-buffer-aliasing/`
(2026-05-28) — 4 clusters, 39 probes, full per-cluster mechanism
docs.  Use it as a reference for the layout below.

---

# <NN> — <Investigation title>

## Status (REQUIRED)

State of the investigation: shape catalogue progress, mechanism
verification status per cluster, what's blocked.  Lead with the
status table:

| Stage | Status |
|---|---|
| A — Probe catalogue | 🟡 in progress / ✅ complete |
| B — Mechanism investigation | 🔴 not started / 🟡 N/M clusters verified / ✅ complete |
| C — Fix design (OPTIONAL) | ⏸️ pending Stage B / N/A — mechanism uniquely determines fix |
| D — Implementation | ⏸️ pending Stage C |

**Stage C is OPTIONAL.**  When Stage B's mechanism analysis uniquely
determines the fix shape (the bug is in one specific gate; the only
question is the exact predicate change), skip C and go B → D.  Force
Stage C when the fix shape has multiple viable options worth
comparing (refcount vs targeted vs hybrid; design choices that
constrain follow-on work).  PLAN51's clusters II/III/IV/V went
B → D iteratively; only the Path C refcount alternative warranted
a Stage C deliverable, and even that was deferred pending real
need.

Then one paragraph: what triggered this investigation, what the
scope is, what's already known going in.

## Goal (REQUIRED)

One sentence.  What this plan ships when complete.  For an
investigation plan, this is typically a fix design + a phased
implementation — NOT a vague "understand X better".

## Cluster catalogue (REQUIRED — replaces Sub-arcs)

The failure modes discovered during exploration, one row per
distinct cluster.  Each cluster gets its own `cluster-<id>-<slug>.md`
investigation doc.

| ID | Cluster | Severity | Backend asymmetry | Probes | Doc |
|---|---|---|---|---|---|
| I | one-line shape | severity | both / interp / native | count | `cluster-I-<slug>.md` (link when created) |
| II | … | … | … | … | … |

## Probe suite (REQUIRED)

Each probe is a self-contained `.loft` file in `probes/` with
assertions that turn the failure into a deterministic test result.

The minimum viable index is a **flat list** keyed by probe number:
shape, cluster, status.  Use one row per probe regardless of role.

| File | Shape | Cluster | Status |
|---|---|---|---|
| `01-<slug>.loft` | <shape> | reference | passes — baseline |
| `02-<slug>.loft` | <shape> | <cluster id> | <fails with X / now PASS> |

**Optional A/B/C curation**: when the suite grows past ~15 probes,
the flat list becomes hard to scan and an explicit "primary probe per
cluster vs everything else" split helps.  PLAN51's experience (62
probes, formal A/B/C table) showed the curation overhead doesn't pay
back during implementation — investigators worked from cluster docs,
not the curation table.  Add the A/B/C split only when probe count
makes the flat list painful AND multiple investigators are reading
the suite cold.

**Probe naming**: `NN-<descriptive>.loft`.  Numeric ordering for
stable references; descriptive suffix.  Probes promote to
`tests/scripts/NN-<descriptive>.loft` when their cluster's fix
lands.

**Probe promotion gate**: a probe is graduation-ready only when it
passes ALL of:

1. **Assertions pass** (`probe NN PASSED` prints).
2. **Clean process exit** — no SIGSEGV / panic at teardown.  Run the
   probe and check the exit code; "PASSED prints" is not enough
   (PLAN51's probe 08 printed PASSED then SIGSEGV'd at process exit,
   corrupting the loft_suite run during graduation).
3. **No leak warning** — run with `LOFT_STORES=warn` (or rely on the
   loft_suite leak gate); zero `stores not freed` lines.
4. **Bounded runtime** — completes in seconds, not minutes.  An
   infinite loop in a graduated test hangs the suite (PLAN51's
   probe 22 hangs forever; graduation would have wedged CI).

A probe that passes assertions but fails any other gate stays in
`probes/` with a status note; substitute a representative variant
when graduating.

## Reference ↔ problem pairings (REQUIRED if probes ≥ 5)

Each problem probe paired with its closest passing reference.
Diffing the pair is the diagnostic shortcut for understanding
each failure.

| Problem | Reference | What the diff reveals |
|---|---|---|
| 02 | 01 | <one-sentence summary of the divergence> |

This pairing structure makes Stage B's mechanism work concrete:
"trace probe X vs probe Y under `<env vars>` and identify the
divergent opcode emission".

## Tool gaps (OPTIONAL but recommended)

Tools added or verified during this plan's investigation work.

| Tool | Status | Used for |
|---|---|---|
| `LOFT_LOG=<key>` | New / Already-existed / Verified-suitable | <what it pinned> |

Tools added during a plan are part of its output, not separate
work.  Closing the plan should leave the tools in tree.

## Status & next-session roadmap (REQUIRED)

For each cluster, the action needed to advance and its effort
estimate:

| Cluster | Mechanism status | Action needed | Effort |
|---|---|---|---|
| I | ✅ Fully understood | — | — |
| II | 🟢 Hypothesis verified by side evidence | Read <file:line>; pin opcode | <XS-L> |
| III | 🤔 Hypothesized | Trace probe X with `<env var>` | <XS-L> |

Then prose: aggregate effort estimate (Phase B-finish + Phase C
+ Phase D), recommended sequence, quickest-user-visible-win
callout.

## See also (REQUIRED)

- Reference doc(s) the investigation touches.
- Other plans on related subsystems.
- Source files central to the investigation.

---

# Per-cluster investigation document template

Each `cluster-<id>-<slug>.md` follows this shape:

```markdown
# Cluster <ID> — <name>

**Severity (split by failure mode):**
- **Corruption / panic / hang:** <user impact, or NONE if leak-only>
- **Leak:** <store count per iter / NONE if no leak>

Track these separately.  "FIXED" must be qualified by which failure
mode is closed (PLAN51's Cluster III was marked FIXED on corruption
closure while leaks persisted under Cluster II — that conflation
caused two false-fix moments in the next session).

**Affected probes:** <list>
**Backend asymmetry:** <which backends fail>

## Mechanism (verified / hypothesized)

<concrete description; trace evidence cited as /tmp/<file>>

## Reference probe — XX (<name>, <status>)

<loft code>

**Lowered IR / behavior**: <key observation>

## Problem probe — YY (<name>, <status>)

<loft code>

**Lowered IR / behavior**: <key observation>

## The divergence

<one-paragraph explanation of what makes Y fail and X pass>

## What we know vs. don't

| | Status |
|---|---|
| <claim> | ✅ Verified via <evidence> / 🟢 Strong hypothesis / 🤔 Plausible / ❌ Unknown |

## Investigation tasks

1. <concrete action — read file:line, run trace, dump artifact>
2. …

## Fix surface

Options ranked by effort and scope.  Each option lists what it
fixes and what it doesn't.

## Fix iterations (REQUIRED when fix attempt count > 1)

A short journal of fix attempts that landed-then-retracted, or
landed-but-needed-follow-up.  Each entry: what was tried, what
assumption it corrected, why it was insufficient.  Two paragraphs
per attempt max.

The commit messages capture each landed change in isolation, but
the *sequence* (and why attempt N didn't suffice) is the load-
bearing context for future investigators who hit a similar shape.
PLAN51's Cluster II took 3 attempts (sentinel-only → free+sentinel
→ narrowed `is_hidden_buf_arg`); without this journal, the third
attempt's predicate would look arbitrary.

Drop this section if the fix landed on the first attempt.

## Why <backend that works> escapes

<contrast: what does the working backend do differently here?>
```

Length budget per cluster doc: **100-300 lines**.  Longer means
historical "failed approach" sections are accumulating — push
closed approaches into a single "Historical: previous attempts"
appendix at the bottom, summarising each in 2-4 lines.  Reference
content that drifts in should extract to the parent reference doc.
PLAN51's cluster-II doc hit 381 lines before pruning; the
appendix pattern would have held it under budget.

---

## Authoring notes (delete from your plan README)

**Length budget**: 100-300 lines for the README + 100-300 per
cluster doc + probe-file headers (typically 20-50 lines each).
A complete investigation plan is 50-80 files total in a deep
case; 10-20 in a shallow case.

**Probe-first discipline**: write probes BEFORE source reading.
The probe suite is the executable spec for what "understood" means.
A mechanism hypothesis is verified when a probe-pair diff confirms
it.  Source reading without a probe to ground it tends to
recursively explore code paths without convergence.

**Liberal probing**: **add probes aggressively** — be liberal, not
selective.  Missing a crucial case (the kind that surfaces in
production code, like PLAN51's moros_map probe) is the worst failure
mode; carrying a few redundant variants costs nothing because they
sit in `probes/` documenting the explored space.

Specifically:
- If an idea for a probe occurs while reading another probe's
  output, write it.  Don't gate on "will this add insight?" —
  you can only know AFTER running it.
- Variants that confirm a known mechanism are still valuable
  evidence; they don't need to be promoted to `tests/scripts/`
  but they belong in the probe directory.
- The cost of a redundant probe is ~10 minutes of probe-writing.
  The cost of a missed shape is the next session re-discovering
  it from scratch + losing the diagnostic continuity.

The flat probe-suite table covers most cases.  A/B/C curation
becomes worthwhile only when the suite exceeds ~15 probes AND
multiple investigators read it cold.

**Real-library extraction**: include at least one probe extracted
from production code in your subsystem (the canonical loft case:
`gridmesh`, `moros_map`, `audience_crystal`).  Pure synthetic
probes can miss real-world shapes — PLAN51's probe 39 caught a
leak class that no synthetic probe surfaced.

**Verified-vs-hypothesized accountability**: every mechanism
statement in a cluster doc is either VERIFIED (with cited
evidence — trace file path, code line read, etc.) or
HYPOTHESIZED (marked with 🤔).  No middle ground.  When asked
"do we know what's going on?", the table answers honestly.

**Tools-as-needed**: don't write a debugging framework upfront.
Check the existing toolchain (CLAUDE.md inventory) for what's
already available.  Add the ONE tool you need DURING the
investigation, when its absence is blocking.  Other "nice-to-have"
tools should stay as ad-hoc eprintln patches reverted before
commit.

**When NOT to use this template**: if the failure class is
narrow enough that you can pin the mechanism in one source-
read session, just file a P-issue + fix.  Investigation plans
are for failure CLASSES with multiple sub-mechanisms.  PLAN51
qualified (5 clusters, 39 probes, mixed mechanisms); a single
crash bug doesn't.

**On closing an investigation plan**: see [`_LIFECYCLE.md`](_LIFECYCLE.md).
Probe migration to `tests/scripts/` happens as each cluster's
fix lands, NOT all at once.  The plan stays open during
phased implementation; closes when the last cluster's regression
is committed and the cluster doc moves to the relevant reference
doc (e.g. NATIVE.md `## Open work` for native-specific issues).
