<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 92 — Feature & infra catalogue (`@F###`/`@I###`): gh-minted IDs, generated docs, source-coverage gate

## Status

**Open — `status:next`, wanted soon. Runs ALONGSIDE the stabilization work** — it is
tooling over docs + source, not a store/codegen change, so it is **independent of
@PLN85** and does not wait on it. No implementation yet. This README is the single
source of truth for per-strand status; `@PLN92` carries the summary + labels.

## Goal

A lightweight, **source-validated catalogue of what loft is made of**: every user
feature (`@F###`) and infrastructure subsystem (`@I###`) is a gh-minted entry with a
title and — for features — a runnable, cross-backend-tested example; per-feature user
docs are generated from those entries, and a **source-coverage gate** proves the
catalogue is complete to within a few lines, directly from loft's source.

## Effort + design

- **Effort:** MH — most of it rides existing machinery; the genuinely new piece is the coverage gate's per-region span attribution.
- **Design:** ~ (design converged over the originating discussion; per-strand detail pending).
- **Last touched:** 2026-06-30.

## Composition matrix — Stage A

N/A — this is tooling/process, not a language value/type/operation, so there is no
composition surface to matrix. The one thing needing a probe is the **coverage gate's
own correctness** (does it attribute spans and count substantive lines as intended);
that is validated by the scanner's own test cases, added with strand 4.

## Why now

Two failure modes from the originating session: a real feature (**named arguments**)
**missed during investigation** (discoverability), and **cross-doc status drift**
(@P251 — closed in one doc, open in three). **Pre-plan features are worst hit** —
nothing ever forced their cataloguing. The coverage gate fixes that at the root: bulk
untagged code can't merge, so features get catalogued *as code is written*, not
remembered later. Doc examples are already cross-backend-tested (`tests/docs/`), so
this adds **identity + discoverability + completeness over the tested examples you
already have** — not a new testing or doc-rewrite mechanism.

## The model

- **One tracker, one number sequence, two kinds.** `loft-lang/features` (a new repo,
  mirroring `loft-lang/plans` for `@PLN`) holds every entry as an issue; the issue
  number is the id; an issue **label** sets the kind, and the prefix reflects it:
  `@F<n>` = user feature, `@I<n>` = infrastructure. The number is opaque (can't go
  stale); a kind-prefix is acceptable because *kind* is stable (a flip is a rare,
  deliberate reclassification), unlike a descriptive slug.
- **Per-entry:** number + **title** (the user-value sentence for `@F`; the role for
  `@I`) + for `@F` a **runnable, cross-backend-tested example** that is simultaneously
  the spec, the "how it works" demo, and the **status source** — status is *derived*
  from the example's per-backend pass (read from the existing `tests/docs` CI run), not
  hand-maintained.
- **Dual-anchor:** bare `@F<n>` / `@I<n>` references in the docs and at the
  implementing source; the gh issue is the declaration (number + title). `idx` joins
  declaration ↔ references — exactly the `@P###` split (PROBLEMS.md row = declaration,
  in-code `@P` = reference).
- **Two-layer, kind-filtered docs:** `@F` → **generated** per-feature *user* pages
  (rides `gendoc`, from the ticket + tested example); `@I` → an *internal* architecture
  map (dev/agent docs), **never the user-facing site**; the conceptual/guide layer
  (models, `learn-loft`, comparisons) stays **curated prose** — stable, reshuffle-
  immune, and not assemblable from examples. This collapses today's per-feature
  triplication (LOFT.md prose / topic source / generated HTML) to one source.
- **Source-coverage gate (the keystone):** every substantive source region is
  attributed to an `@F` (per-capability) or `@I` (coarse, subsystem-level) entry; the
  residue — attributed to **neither** — must stay under a **ratcheting budget** (the
  "few lines that dodge"). A per-region threshold *K* forces bulk code to claim an
  entry while letting trivial glue dodge. Because infra is a **named** entry, not an
  anonymous exemption, the attribution is **auditable** — no silent dumping ground.

## Sub-arcs

| Item | Status | Notes |
|---|---|---|
| **1 — Identity** | Open | Stand up `loft-lang/features`; `@F###`/`@I###` = issue number, kind from label; ticket shape (number + title + `@F` example + status label). = the `@PLN` bootstrap. |
| **2 — Scanner / index** | Open | Extend `scan.loft` for the `@F`/`@I` prefixes (same byte-scan as `@P`); resolve titles; emit `idx features` / `idx infra` (number → title → doc + code sites). **Cheap discoverability win.** |
| **3 — Doc generation (two-layer, kind-filtered)** | Open | `@F` → generated user pages via `gendoc`; `@I` → internal architecture map; concept/guide layer stays curated. Collapses the triplication. |
| **4 — Source-coverage gate (keystone)** | Open | Per-region span attribution to `@F`/`@I`; per-region threshold *K* + ratcheting residue budget (reuse `.lint_comments_baseline`); CI gate. The only genuinely new mechanism. |
| **5 — Hygiene** | Open | Every `@F`/`@I` declared once + dual-anchored (≥1 doc + ≥1 code); optionally fold in the cross-doc status-drift check (the @P251 class). |

## Phase ordering

1 (identity) → 2 (scanner + `idx features` — cheapest, immediate discoverability) →
4 (coverage gate, seeded at today's count and ratcheted down — this is what *forces*
tagging, so it lands early to drive adoption) → 3 (doc generation, rides `gendoc`).
Hygiene (5) folds into 2 and 4. Adoption is a **ratchet, not a big-bang backfill**:
new bulk code must claim an entry from day one; the existing backlog is chipped down.

## Rides existing machinery — why MH, not VH

- `scan.loft` (@PLN42, loft-native) already byte-scans `@P`/`@PLAN` with context and reads PROBLEMS.md for valid ids — the `@F`/`@I` prefix is the same path.
- `idx` already resolves gh-backed tags (`idx gh:N`, `@GH` refs).
- `gendoc` + `src/documentation.rs` already generate the user docs (HTML + print/typst) from sources.
- `tests/docs/*.loft` already run doc examples cross-backend — the derived status source.
- `.lint_comments_baseline` + the "comment quality vs baseline" CI gate is the exact **scan-source → baseline → ratchet** pattern the coverage gate reuses.

The new work is concentrated in strand 4: function-span bracketing + the
attributed/unattributed tally in `scan.loft`.

## Open design questions

1. **Tracker shape** — confirm one `loft-lang/features` repo with a kind-label driving the `@F`/`@I` prefix (vs two repos). One repo keeps a single clean number sequence.
2. **Attribution granularity + "meaningful line"** — per-function unit (point-tags don't span); exclude blanks/comments/`use`/derives/braces. Definition is fuzzy (as in any coverage tool) but tractable.
3. **Budget shape** — per-region threshold *K* (forces bulk, lets trivia dodge); also a global cap?
4. **`@I` discipline** — even as a named citizen, keep it from bloating: `@I` is **coarse** (one per subsystem), entries reviewed, possibly a cap on `@I` growth. This is what keeps the gate honest.
5. **Title home** — gh issue title as canonical (no second copy) vs a cached `FEATURES.md`; is the generated `index/features.json` the registry-of-record?
6. **`@F` page richness** — terse generated reference (title + example) vs an optional short notes field; keep tickets lightweight, push richness to the curated guide layer.
7. **Status-drift check** — fold the @P251-class cross-doc status check into strand 5, or keep separate? The indexer already reads PROBLEMS.md.
8. **Rust-now / loft-later** — the comment-tag + region-detection model must survive the Rust→loft source transition (@PLN91); it is language-agnostic by design.

## Cross-arc dependencies

- **@PLN42** (tracker index) — this *extends* `scan.loft` / `idx`; the catalogue is its next output. Active.
- **@PLN91** (self-hosting) — the attribution model must survive Rust→loft source; `@F`/`@I` coverage eventually runs over the loft source. Independent in timing (this ships first).
- **`.lint_comments_baseline` / "comment quality vs baseline"** — the ratchet mechanism strand 4 reuses (not a plan; a precedent).

## See also

- [plans/42-tracker-index/](../42-tracker-index/) · `scripts/idx` · `tools/indexer/src/scan.loft` — the loft-native indexer this extends.
- `src/gendoc.rs` · `src/documentation.rs` · `tests/docs/*.loft` — the doc-generation pipeline + the cross-backend example tests (the status source).
- [PROBLEMS.md](../../PROBLEMS.md) / `@P###` and `loft-lang/plans` / `@PLN###` — the gh-numbered-identity precedents this mirrors as a third namespace.
- [DOC.md](../../DOC.md) / [DOC_QUALITY.md](../../DOC_QUALITY.md) — doc architecture; the two-layer model.
- [LOFT.md](../../LOFT.md) — the prose reference whose per-feature sections strand 3 replaces with generated pages (its conceptual layer stays).
- `@PLN92` — the tracker issue this plan realizes.
