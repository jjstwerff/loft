<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan template

Copy this file to `<NN>-<slug>/README.md` when opening a new
plan.  Numbering is the next free integer in `plans/future/`
or `lib_plans/future/` (independent counters).

The sections below are the canonical shape.  Sections marked
**(REQUIRED)** must be present in every plan; sections marked
**(OPTIONAL)** depend on plan shape.

---

# <NN> — <Plan title>

## Status (REQUIRED)

One paragraph.  State of the world today + what this plan
will change.  Examples:

- "Open — design ready, no implementation yet.  Routes here
  per the docs-vs-plans rule."
- "Mixed: JSON shipped (`Type.parse`); HTTP planned for 1.1+.
  Three-file split: README (overview), JSON.md (shipped
  reference), HTTP_CLIENT.md (planned design)."
- "Pointer-plan: full design lives at
  [`../../../FOO.md`](../../../FOO.md); this plan tracks
  open work as actionable rows."
- "Pure future plan; pre-flight 50% bug yield expected."

## Goal (REQUIRED)

One sentence.  What this plan ships when complete.  Avoid
strategy / advertising language; that lives in ROADMAP.md
or RELEASE.md.

## Effort + design (OPTIONAL — recommended)

Echo the ROADMAP `E` and `Design` columns so plan readers
don't bounce out to ROADMAP for sizing.

- **Effort:** XS / S / M / MH / H / VH
- **Design:** ✓ (detailed) / ~ (partial) / — (needs design)
- **Last touched:** YYYY-MM-DD (auto via `git log -1`)

## Sub-arcs (REQUIRED if multi-phase; OPTIONAL otherwise)

Status table.  One row per arc / phase / sub-feature.  Each
row links back to the design source if reference content
lives elsewhere (pointer-plan shape) or to the phase file
if it lives here.

| Item | Source | Status |
|---|---|---|
| **A** — short title | (link) | Open / In-flight / Shipped |
| **B** — short title | (link) | Open / Blocked on X |

## Phase ordering (OPTIONAL — for multi-arc plans)

Suggested sequence when this plan unpauses.  Numbered list,
each item one or two sentences.  Note dependencies between
arcs.

## Open design questions (OPTIONAL)

Questions that need answers before implementation starts.
Numbered list.  Each question's resolution becomes a
decision recorded in `DESIGN_DECISIONS.md` or absorbed
into the design content here.

## Cross-arc dependencies (OPTIONAL)

When this plan depends on or cooperates with sibling plans,
list them here as a bullet list with one-line rationale.
Helps the dependency graph stay visible.

## See also (REQUIRED)

- Reference doc(s) that this plan implements / extends.
- Sibling plans that cooperate or that block this one.
- ROADMAP.md row(s) that schedule this plan's items.

---

## Authoring notes (delete from your plan README)

**Length budget:** 100-300 lines.  Plans longer than 300
lines tend to be reference content that should move to
`doc/claude/<NAME>.md` or be split.

**File names within the plan dir:** `README.md` is required.
Sub-files for distinct concerns: `IMPL.md`, `DISCUSSION.md`,
`PROTOCOL.md`, etc.  Multi-file plans should match the
EVENT_LOOP / OPENGL / WEB_SERVICES precedent: each sub-file
has a single concern.

**On opening a new plan:**
1. Copy this template to `plans/future/<NN>-<slug>/README.md`
   (or `lib_plans/future/<NN>-<slug>/`).
2. Fill in Status + Goal first.
3. Add ROADMAP row(s) citing this plan + tag with value
   category (S / R / G / F / U / C / Q / N).
4. Add lib_plans/README.md or plans/README.md table row.
5. **Do NOT add a CLAUDE.md doc index entry by default.**
   Plans are discoverable via plans/README.md (which IS in
   CLAUDE.md doc index).  Add a CLAUDE.md entry ONLY if the
   plan introduces a NEW top-level reference concept that
   would otherwise have no doc-root home — vanishingly rare
   for plans (most plan content lives at a reference doc
   that already has its own CLAUDE.md entry).

**On closing a plan:** see [`_CLOSURE_CHECKLIST.md`](_CLOSURE_CHECKLIST.md).
