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
| C — Fix design | ⏸️ pending Stage B |
| D — Implementation | ⏸️ pending Stage C |

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
| I | <one-line shape> | <severity> | <both / interp / native> | <count> | [`cluster-I-<slug>.md`](cluster-I-<slug>.md) |
| II | … | … | … | … | … |

## Probe suite (REQUIRED)

Curate into three groups.  Each probe is a self-contained `.loft`
file in `probes/` with assertions that turn the failure into a
deterministic test result.

**A — Reference probes** (passes on all backends; used as baselines
to contrast against problem probes):

| File | Shape | Why kept |
|---|---|---|
| `01-<slug>.loft` | <shape> | <rationale> |

**B — Problem probes** (one per distinct failure mode; the primary
investigation targets):

| File | Shape | Cluster | Failure |
|---|---|---|---|
| `02-<slug>.loft` | <shape> | <cluster id> | <symptom> |

**C — Attic probes** (variants and confirmations that don't add
distinct insight; kept for posterity but not promoted):

| File | Why attic |
|---|---|
| `03-<slug>.loft` | Variant of probe 02 — confirms <X>; same failure mode |

**Probe naming**: `NN-<descriptive>.loft`.  Numeric ordering for
stable references; descriptive suffix.  Probes promote to
`tests/scripts/NN-<descriptive>.loft` when their cluster's fix
lands.

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

**Severity:** <user impact>
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

## Why <backend that works> escapes

<contrast: what does the working backend do differently here?>
```

Length budget per cluster doc: 100-300 lines.  Longer means
reference content is leaking in — extract to the parent
reference doc.

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

**A/B/C curation enables liberal probing**: **add probes
aggressively** — be liberal, not selective.  Missing a crucial
case (the kind that surfaces in production code, like PLAN51's
moros_map probe) is the worst failure mode; carrying a few
redundant variants costs nothing because they go to C (attic)
when curated and don't pollute the suite.

Specifically:
- If an idea for a probe occurs while reading another probe's
  output, write it.  Don't gate on "will this add insight?" —
  you can only know AFTER running it.
- Variants that confirm a known mechanism are still valuable
  evidence; they're attic'd, not deleted.
- The cost of a redundant probe is ~10 minutes of probe-writing.
  The cost of a missed shape is the next session re-discovering
  it from scratch + losing the diagnostic continuity.

Curation happens AFTER the suite stabilises (typically at the
end of Stage A), not during.  The README's A/B/C table is the
explicit curation artifact.

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
