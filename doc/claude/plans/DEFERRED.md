<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Deferred Plans — Trigger Index

Index of every `plans/deferred/*` plan with the **concrete signal**
that should re-activate it.  Distinct from:

- `plans/future/*` — paused with intent to finish; tracked on
  [`../ROADMAP.md`](../ROADMAP.md), not here.
- `DESIGN_DECISIONS.md` — closed-by-decision (won't do, no trigger).
- `PROBLEMS.md` — open bugs with reproducers.

**Convention.**  Every row carries a `Trigger` value.  When the
signal arrives, the plan moves from `plans/deferred/` to
`plans/future/` (or directly into a current plan), the ROADMAP
gains a row citing it, and the row leaves this file.

Full per-plan trigger detail lives in each plan's README; this
file is the one-line index.

---

## Deferred plans

| Plan | One-line summary | Trigger |
|---|---|---|
| [`deferred/10-scope-exit-emission/`](deferred/10-scope-exit-emission/) | Drop the `(dep.is_empty() ‖ is_work_ref)` gate on scope-exit emission; align with the lift-frame rule. | A new bug surfaces that's gated by the current condition, OR the lift-frame work uncovers a clean unifying invariant. |
| [`deferred/12-codegen-simplifications/`](deferred/12-codegen-simplifications/) | Tier-2 codegen simplifications.  Tier 1 already shipped; Tier 2 is @PLAN13's preamble. | Same as @PLAN13 — 3+ template-path bugs, major codegen evolution forcing ≥50 Op-annotation touches, or contributor appetite for a large structural refactor. |
| [`deferred/13-rust-template-migration/`](deferred/13-rust-template-migration/) | Move per-Op codegen from string templates to typed Rust emitter functions. | Multiple template-path bugs accumulate (3+ P-issues over a few months tracing back to template-substitution edge cases), OR a major codegen evolution makes the per-emitter form pay back its migration cost. |
| [`deferred/28-const-store/`](deferred/28-const-store/) | Phase B (mmap cache loading) + Phase C (WASM pre-compiled stdlib). | Phase B: Phase C lands a large embedded stdlib cache (mmap pays off only above some size threshold).  Phase C: contributor appetite for H-effort `Data` serialization across 130+ public members + recursive enums, OR demonstrated need for sub-100ms WASM cold-start past what `include_bytes!` + parse achieves.  Both phases roadmap-tracked (CS.B / CS.C1-C3). |

---

## How rows leave this file

A row leaves DEFERRED.md when its trigger fires:

- **Trigger fires + work starts** — plan moves from `deferred/` to
  `future/` (or directly into a current plan); ROADMAP gains a
  row; the entry here is removed.
- **Reclassified as won't-do** — entry moves to
  `DESIGN_DECISIONS.md` with rationale.
- **Plan ships** — closure via the standard route
  (`_LIFECYCLE.md`); this file isn't involved.

---

## Cross-references

- [`README.md`](README.md) — plans index (current + future +
  deferred + finished tables).
- [`../ROADMAP.md`](../ROADMAP.md) — work-list view.  Future plans
  live there; deferred plans do not appear (they only return to
  ROADMAP when their trigger fires).
- [`../DESIGN_DECISIONS.md`](../DESIGN_DECISIONS.md) — closed-by-
  decision register (won't do).
- [`../PROBLEMS.md`](../PROBLEMS.md) — open bugs with reproducers.
- [`../USER_FACING.md`](../USER_FACING.md) — user-visible deferred
  items (subset of this list, filtered).
