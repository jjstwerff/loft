<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Quality — open active sprints + designs

The quality reference (open programmer-biting issues, design
content for each active item, compiler-blocker analysis,
enhancement tiers, recommended landing order) lives at
[`../../../QUALITY.md`](../../../QUALITY.md).

This plan tracks the **open active items** in QUALITY.md as
actionable rows.  Each item points at the QUALITY.md section
that holds the full design.

## Status

The items group into three clusters: JSON, native runtime,
compiler bugs.

### JSON cluster — `JsonValue` enum + serialiser ecosystem

| Item | QUALITY.md section | Status |
|---|---|---|
| **P54** — `JsonValue` enum (active sprint) | [§ Active sprint — P54](../../../QUALITY.md) (line 58+) | Active sprint.  Multi-step transition from text-based JSON to a first-class `JsonValue` enum.  Steps remaining tracked in the section's checklist.  ROADMAP row P54 cites PROBLEMS.md #54 + QUALITY.md. |
| **Q1** — JSON parse-error diagnostics | [§ Active design — Q1](../../../QUALITY.md) (line 412+) | Open — design ready.  Improves the diagnostic shape for malformed-JSON parse errors. |
| **Q2** — free-form object iteration + kind peek | [§ Active design — Q2](../../../QUALITY.md) (line 596+) | Open — design ready.  API for iterating untyped JSON objects + peeking at value kinds. |
| **Q3** — `to_json` serialiser + struct serialisation | [§ Active design — Q3](../../../QUALITY.md) (line 763+) | Open — design ready.  Complement to `Type.parse()` deserialiser; symmetric serialisation API. |
| **Q4** — `JsonValue` construction in loft code | [§ Active design — Q4](../../../QUALITY.md) (line 914+) | Open — design ready.  Builder API for constructing `JsonValue` trees in loft (without going through serialise → text → parse). |
| **P54-U** — unified JSON parser | [§ Active design — P54-U](../../../QUALITY.md) (line 1097+) | Open — design ready.  Phase 3 (mentioned in RELEASE.md) deletes ~540 lines of legacy `src/database/structures.rs::parsing` scanner once a walker-native `Diagnostic` shape replaces the `"line N:M path:X"` error-path format. |

### Native runtime cluster

| Item | QUALITY.md section | Status |
|---|---|---|
| **Dep-inference** — for native fn returns (zero-leak unblock) | [§ Active design — Dep-inference](../../../QUALITY.md) (line 1498+) | Open — design ready.  Closes the closure / native-fn dep-tracking gap that currently leaks DbRefs in some text-returning native paths.  Part of the broader scratch-retirement direction (`../21-retire-scratch/`). |

### Compiler-blocker cluster

| Item | QUALITY.md section | Status |
|---|---|---|
| **B2-B7** — struct-enum bugs gating P54 | [§ Compiler blockers — struct-enum bugs](../../../QUALITY.md) (line 1655+) | Open — diagnostic recipes per bug.  Some may have closed in the plan-17 / plan-19 sweeps; needs audit (un-skip the relevant test reproducers and re-run to find which still bite).  Each remaining bug is a P-issue close. |

### Open programmer-biting issues + Enhancement tiers

QUALITY.md also maintains:
- **Open programmer-biting issues** ([§ Open programmer-biting issues](../../../QUALITY.md), line 22) — running list updated as new issues surface; bugs follow the bug exception (PROBLEMS.md + test + commit, no plan needed) but tier-list-shaped designs may grow into new plans here.
- **Enhancement tiers** ([§ Enhancement tiers](../../../QUALITY.md), line 2163) — quality investments ranked by leverage; tier names may flow into new plans as items mature.
- **Recommended landing order** ([§ Recommended landing order](../../../QUALITY.md), line 2432) — the doc's own suggested sequence across all open items.

These three sections stay as reference content in QUALITY.md;
this plan does not duplicate them.

## Why these items are here, not in QUALITY.md

QUALITY.md is reference documentation — it describes open
issues + their design content, the compiler-blocker analysis,
and the enhancement-tier ranking.  Anyone planning the next
investment-of-time reads QUALITY.md.

The active sprint + active designs don't fit pure-reference
status: they're items to BUILD.  Per the docs-vs-plans rule,
they belong in `plans/future/`.  Keeping them visible in the
`plans/future/` index ensures they don't get lost as
QUALITY.md grows.

The pointer-plan shape (this README references QUALITY.md
sections rather than copying their content) avoids
duplication — design details stay in one place.  When an
item ships, the work in QUALITY.md gets trimmed (P54-style:
the section gets a "LANDED via …" header, then eventually
removed) and this plan's row moves to a closure record.

The closed C54 (integer→i64) section in QUALITY.md is the
canonical pattern: kept as reference under
`## ~~C54 — integer i64~~ — LANDED 2026-04-21` until eventually
removed once readers stop needing the cross-reference.

## Phase ordering

Per [QUALITY.md § Recommended landing order](../../../QUALITY.md):

QUALITY.md itself maintains the canonical sequence; this plan
defers to it.  Summary at writing time:

1. **B2-B7 audit** — fast pass to identify which compiler
   blockers actually still bite.  May close several with
   no implementation work (just confirm closed by plan-17
   / plan-19 sweeps).
2. **P54 active sprint** — JSON `JsonValue` enum continues.
   Q1-Q4 active designs land alongside as the surface
   matures.
3. **P54-U unified parser** — phase 3 deletes legacy
   scanner once walker covers all paths (already covers
   success path zero-fallback).
4. **Dep-inference** — native fn return leak fix; cooperates
   with `../21-retire-scratch/`.
5. **Enhancement tiers** — driven opportunistically as
   priorities allow.

## See also

- [`../../../QUALITY.md`](../../../QUALITY.md) — full
  quality reference (open issues, active designs,
  compiler-blocker recipes, enhancement tiers, recommended
  landing order)
- [`../../../PROBLEMS.md`](../../../PROBLEMS.md) — bug
  tracker (P54 + B2-B7 cite specific P-issue rows)
- [`../21-retire-scratch/`](../21-retire-scratch/) — sibling
  plan; Dep-inference closes one of the consumers blocking
  scratch retirement
- [`../34-performance-followups/`](../34-performance-followups/) —
  another pointer-plan precedent (PERFORMANCE.md companion)
- [`../33-native-codegen-followups/`](../33-native-codegen-followups/) —
  another pointer-plan (NATIVE.md companion)
- [`../../lib_plans/future/11-packages/`](../../../lib_plans/future/11-packages/) —
  another pointer-plan (PACKAGES.md companion)
