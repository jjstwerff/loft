<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan lifecycle checklist — closing or deferring

Use this when a plan exits the active state.  Two outcomes share
most of the procedure:

- **Closing** (move to `finished/`): all phases shipped.
- **Deferring** (move to `deferred/`): some or no phases shipped;
  remaining work has a **concrete trigger**.

If remaining work has no concrete trigger, the design moves to
[`../DESIGN_DECISIONS.md`](../DESIGN_DECISIONS.md), not `deferred/`.
"Will get to it later" is not a trigger; "when 3+ template-path
bugs accumulate" is.

## Pick the outcome

| Situation | Outcome |
|---|---|
| All phases shipped | **Close** — `finished/` |
| Some phases shipped, others paused with concrete trigger | **Partial defer** — `deferred/`, Status table grows SHIPPED / DEFERRED rows.  Canonical: plan-28, plan-12. |
| No phases shipped, all paused with concrete trigger | **Full defer** — `deferred/` |
| Some phases shipped, others abandoned without trigger | Apply this checklist for the shipped portion as if closing it; remaining design moves to `DESIGN_DECISIONS.md` |
| Waiting on a date or "appetite" with no concrete signal | Stay in `future/` (date-bound) or move design to `DESIGN_DECISIONS.md` (appetite-bound).  Don't defer. |

## Steps 1-3 — Per shipped phase, extract reference content

These steps run for **every shipped phase**, regardless of close
vs defer.  Skip for phases being deferred (they keep their design
content in the plan README).

### Step 1 — Tag each section

Read the plan README + each phase file.  Tag every section as one
of:

- **REFERENCE** ("how things work today after this ships") → moves out
- **CLOSURE RECORD** ("what was done, when, what was retracted, why") → stays
- **HISTORICAL ARCHAEOLOGY** ("what was tried that didn't ship") → stays as historical record (don't move into reference; reference is for current truth)

### Step 2 — Pick the reference home

Two shapes:

- **CREATE-AND-MOVE** — no comprehensive reference doc exists.
  Create `doc/claude/<NAME>.md` (or extend `lib/<name>/<doc>.md`
  for library-scoped reference) with the extracted content.
  Canonical example: `31-html-export → HTML_EXPORT.md`.
- **TRIM-ONLY** — reference doc already covers shipped state.
  Trim plan README + cross-link to the existing home.
  Canonical example: `04-slot-assignment-redesign → SLOTS.md`.

### Step 3 — Trim plan README to closure-record shape

Target ~50-150 lines.  Lead with:

> **Status — DONE YYYY-MM-DD** (or **SHIPPED** for partial defers)
> Reference for the post-plan content moved to `<path>`.  This file
> is a closure record only.

Drop pre-shipping content (Goal-as-motivation, Why-this-is-an-
initiative, Ground rules, Verification gates, Risks).  Keep
Status block, per-phase outcome table (compressed), the technical
insight or P-issue table that's the headline value, See also
cross-links.

For partial defers, the plan README also keeps **full design
content for deferred phases** alongside the closure-record for
shipped ones — see plan-28's Phase B + Phase C sections as the
canonical shape.

## Steps 4-6 — Common to close + defer

### Step 4 — Reclassify ROADMAP rows

For each ROADMAP row touched by the plan:

- **Shipped parts** — remove from ROADMAP (closure lives in
  CHANGELOG + git history per the maintenance rule).
- **Deferred parts that are roadmap-tracked** — leave on ROADMAP;
  update plan path to `plans/deferred/<NN>-<slug>/`.
- **Deferred parts that are trigger-only** — remove from ROADMAP.
  The DEFERRED.md row is enough.

For the "Plans index by category" subsection at the bottom of
ROADMAP.md, move/remove rows accordingly.

### Step 5 — Move the directory + update tracker tables

```bash
# Closing:
git mv doc/claude/plans/<NN>-<slug>           doc/claude/plans/finished/<NN>-<slug>
# OR  doc/claude/plans/future/<NN>-<slug>    → finished/

# Deferring:
git mv doc/claude/plans/<NN>-<slug>           doc/claude/plans/deferred/<NN>-<slug>
# OR  doc/claude/plans/future/<NN>-<slug>    → deferred/
```

Update `plans/README.md` (or `lib_plans/README.md`) tracker
tables:
- Remove from "Current" / "Future" table.
- Closing: add to "Finished" table (one line: closure date + ref home).
- Deferring: add to "Deferred" table + add a row to
  [`DEFERRED.md`](DEFERRED.md) with the trigger.

### Step 6 — Grep + rewrite incoming links (THE most-skipped step)

```bash
grep -rn "plans/<NN>-<slug>\|plans/future/<NN>-<slug>\|plans/finished/<NN>-<slug>\|plans/deferred/<NN>-<slug>" \
  CLAUDE.md doc/claude/ --include="*.md"
```

Rewrite each match:
- **Design / how-it-works links** → reference home (the
  `doc/claude/<NAME>.md` location).
- **Closure-record links** → keep pointing at the closed/deferred
  plan.
- **Path-only updates** (the link should still point at the plan
  but the path changed) → fix the relative path.

Verify `scripts/check_doc_drift.sh` reports no broken plan paths
afterwards.

## Lifecycle after defer

The plan stays in `deferred/` while ANY remaining phase has a
concrete trigger.

- **Trigger fires** → move plan back to `plans/future/` (or directly
  into a current arc); remove DEFERRED.md row; add ROADMAP rows;
  rewrite incoming links per Step 6.
- **Remaining phases ship** → re-apply this checklist, this time
  closing.
- **Remaining phases reclassified as won't-do** → design moves to
  `DESIGN_DECISIONS.md`; remove plan directory + DEFERRED.md row.

## Pitfalls

1. **"Will get to it later" is not a trigger.**  If the trigger is
   "when contributor appetite arrives" without a concrete signal
   (a bug pattern, user-request shape, count threshold), the work
   belongs in DESIGN_DECISIONS.md.
2. **Half-done phases corrupt the Status table.**  Either close
   the phase properly (extract its reference content) or roll it
   back (drop it from "shipped" status).  Don't leave "Phase 3 is
   sort of done."
3. **Step 4 (ROADMAP reclassification) is the most-forgotten
   step.**  Shipped parts staying on ROADMAP is the most common
   drift; an audit then thinks they're still open.
4. **Step 6 (link rewrite) is the second-most-forgotten.**  Run
   `scripts/check_doc_drift.sh` after every close/defer to catch
   broken paths.
5. **Stale "removed in plan-X" notes.**  When you retire a feature
   (Type::Long, text_code, .loftc, forwarding_smoke.rs), update
   the docs that mentioned it as current.  Drift script flags
   common shapes.

## Cross-links

- [`README.md § Three workflows`](README.md#three-workflows-for-todo-items--pick-the-lightest-that-fits) — when to use a plan vs the lighter `## Open work` flow.
- [`README.md § Light flow lifecycle`](README.md#light-flow-lifecycle---open-work-in-reference-docs) — close + defer for the light flow (no directory move; just edit a row).
- [`_TEMPLATE.md`](_TEMPLATE.md) — for new plans.
- [`DEFERRED.md`](DEFERRED.md) — trigger index this checklist feeds.
- `scripts/check_doc_drift.sh` — drift detection for path / shipped-claim staleness.
