<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Deferred Library Plans — Trigger Index

Index of every `lib_plans/deferred/*` plan with the **concrete signal**
that should re-activate it.  Distinct from:

- `lib_plans/future/*` — paused with intent to finish; tracked on
  [`../ROADMAP.md`](../ROADMAP.md), not here.
- [`../plans/DEFERRED.md`](../plans/DEFERRED.md) — core-language /
  compiler / runtime parked work (separate tracker).
- [`../USER_FACING.md`](../USER_FACING.md) — user-visible subset.

**Convention.**  Every row carries a `Trigger` value.  When the
signal arrives, the plan moves from `lib_plans/deferred/` to
`lib_plans/future/` (or directly into a current plan), the ROADMAP
gains a row citing it, and the row leaves this file.

Full per-plan trigger detail lives in each plan's README; this
file is the one-line index.

---

## Deferred library plans

| Plan | One-line summary | Trigger |
|---|---|---|
| _(empty — `lib_plans/deferred/` has no plans currently)_ |  |  |

---

## How rows leave this file

- **Trigger fires + work starts** — plan moves from `deferred/` to
  `future/` (or directly into a current plan); ROADMAP gains a row;
  the entry here is removed.
- **Reclassified as won't-do** — entry moves to
  `DESIGN_DECISIONS.md` with rationale.
- **Plan ships** — closure via `_CLOSURE_CHECKLIST.md`; this file
  isn't involved.

---

## Cross-references

- [`README.md`](README.md) — library plans index (current + future
  + deferred + finished tables).
- [`../plans/DEFERRED.md`](../plans/DEFERRED.md) — core-language /
  compiler / runtime deferred-trigger index.
- [`../ROADMAP.md`](../ROADMAP.md) — work-list view.  Future
  library plans live there; deferred plans do not (they only
  return to ROADMAP when their trigger fires).
- [`../USER_FACING.md`](../USER_FACING.md) — user-visible deferred
  items (subset of this list, filtered).
- [`../PROBLEMS.md`](../PROBLEMS.md) — cross-cutting P-issue
  tracker.
