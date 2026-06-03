<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# `_PLAN_TEMPLATE` — the canonical loft plan skeleton

**Initial design (2026-06) — draft for review.**  Copy this directory shape when
you start a plan.  **loft and libs plans are the reference implementation**:
every other subject (moros, dryopea, the demos, lavition) builds its own plans to
*match this shape* in its own repo, so one convention spans every org repo.

A **plan** is a *proposal* (the unit of intentional change) that has grown a
directory — its **active form**.  The same item is a lightweight `PLANNING.md`
section while it's a backlog sketch; it earns an `@PLAN<NN>` number + this
directory when it activates.  Type is `bug` vs `proposal`, never "plan vs
enhancement" — see [`ISSUE_TRACKING.md` § Two kinds of item](../ISSUE_TRACKING.md).

Live exemplars to read alongside this:
- loft — `48-integer-width-discipline` (active feature) · `57-vector-store-watermark` (investigation)
- libs — `lib_plans/12-library-extraction` (active arc) · `lib_plans/08-server` (future feature)

For a probe-driven **investigation** plan (bug *classes*, cluster catalogues),
ALSO follow [`_INVESTIGATION_TEMPLATE.md`](_INVESTIGATION_TEMPLATE.md) — it layers
the clusters / probes / edge-map structure on top of this base.

---

## The split this template encodes

- **This directory = DESIGN** — the what + how + why, the phase breakdown, and
  **per-phase status**.  Versioned with the code, grepped by agents.
- **The gh Project item = STATE** — lifecycle status, milestone, board order, and
  the cross-repo dependency links.  One authority; never restated here.

So the README carries the **stable design facets** + a **link to the board** for
live state.  It deliberately does **not** restate the lifecycle Status — a second
copy is exactly the drift the board exists to kill.  Rule of thumb: facts about
*what this plan is* live in the file; facts about *where it is in its lifecycle*
live on the board.

---

## `README.md` skeleton

```markdown
# @PLAN<NN> — <one-line title>

> **Subject:** <loft | libs | moros | dryopea | bumper-plane | audience | …>
> **Type:** proposal <· investigation — if probe-driven>   ·   **Area:** <codegen | closures | store-lifetime | parser | native | wasm | stdlib | packages | …>
> **Effort:** <XS|S|M|MH|H>   ·   **Value:** <Correctness | Enabling | Polish | Quality>
> **Driven-by:** <the consumer that demanded this — the dogfood link; "—" if foundational>
> **Depends-on:** <@PLANxx / @GHxx, or "—">
> **Live status · milestone · order:** [gh Project ▸ @PLAN<NN>](<board-item-url>)  ← single source of truth for lifecycle

## Thesis

<2–4 sentences: what this plan delivers and why it matters.  If Driven-by names a
consumer, say what that consumer cannot do until this ships — that sentence is the
dogfood justification.>

## Phases

<The README is the SOURCE OF TRUTH for phase progress.  The board tracks the
plan's OVERALL status, never its individual phases — do not mirror this table to
the board.>

| Phase | Goal | Status | Outcome / ref |
|---|---|---|---|
| <NN>-<slug> | <one line> | ☐ todo · ◐ wip · ✅ shipped · ⏸ parked | <commit / test / note> |

## Design

<The actual design.  Inline for small plans; one file per phase
(`<NN>-<slug>.md`) once it outgrows the README.>

## See also

<cross-links: reference docs, sibling plans, the consumer repo>
```

---

## Directory layout

```
plans/<NN>-<slug>/        # FLAT — no future/deferred/finished subdir; state lives on the board
  README.md               # the skeleton above
  <NN>-<phase>.md         # per-phase design (optional; for larger plans)
  probes/                 # investigation plans only — see _INVESTIGATION_TEMPLATE
  experiments/            # preserved failed/partial attempts (diff + hash, not a summary)
```

> Under the new model the directory **never moves** — `git mv` between
> `future/`/`deferred/`/`finished/` is gone, so links to `plans/<NN>/` are
> permanent.  (During the lazy migration, existing plans keep their state subdir
> until they are next touched; new plans start flat.)

---

## Creating the plan — the three steps

1. **Copy the skeleton**, fill the header facets — these seed the Project item.
2. **File the gh Issue titled `@PLAN<NN> <title>`** (the `@P###` trick — the embedded
   token lets `gh search issues "@PLAN<NN>"` find it and keeps every doc reference
   pointing at `@PLAN<NN>`, unrewritten).  Add it to the Project, set its fields from
   the header, and paste the Issue URL into the README's *Live status* line.  The gh
   number `#N` is plumbing; `@PLAN<NN>` stays the reference identity.
3. From then on: **lifecycle changes happen on the board** (set Status, Milestone,
   order); **design + phase changes happen here**.  The two never duplicate.

## What goes on the board vs in this file

| On the board (state) | In this file (design) |
|---|---|
| Status · Milestone · board order | Thesis, design, rationale |
| Subject · Area · Value · Effort (the header facets, as filter fields) | **Per-phase status** (the Phases table) |
| Depends-on (linked items) · Driven-by | Probes / experiments / cross-links |

The header facets appear in *both* — but the file is where they are **authored**
(design-time, stable), and the board is where they become **queryable** (filter,
group, sort).  Only **Status / Milestone / order** are board-only, because only
they change across the plan's life.
