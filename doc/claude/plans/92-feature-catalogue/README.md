<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 92 — Feature & infra catalogue (`@F###`/`@I###`): gh-minted IDs, generated docs, source-coverage gate

## Status

**Open — `status:next`, active. Runs ALONGSIDE the stabilization work** — tooling over
docs + source, independent of @PLN85. **Progress:** strand 2 (`scan.loft` `@F`/`@I`
indexing) shipped + verified; Pass 1 done — **80 issues minted** (`@F1`–`@F56`,
`@I57`–`@I80`) in `loft-lang/features`; **Pass 2 done** — bare `@F`/`@I` source tags at
the 79/80 implementing sites (build green, comment-only); **Pass 3 authoring done** —
**79/80 issues fully authored** (`@F43` random deferred as a library), every `@F`
`## Example` run byte-identical on `--interpret` + `--native`. **ROI gate cleared** (see
[Success criterion](#success-criterion--gated-on-doc-improvement)): authoring tested
examples caught real reference errors, now fixed. **Design settled:** the *issue is the
canonical, self-contained doc; everything else derives one-way* (see
[The model](#the-model)). **Strand 3 (issue→project sync) SHIPPED** —
`tools/features/gen.loft` (the @I81 generator) reads the committed `index/features.json`
snapshot and renders the 81 issues into a committed mirror (`doc/features/`) plus 45
runnable examples (`tests/docs/features/`), with both CI guards green: no-drift
(`make features-check`) + example-must-run on both backends (`tests/features.rs`,
`tests/native.rs::native_features`). **Strand 4 (source-coverage gate) SHIPPED** —
`scripts/feature_coverage.sh` ratchets a file-level attribution baseline
(`.feature_coverage_baseline`): a new implementation file with no `@I`/`@F` tag fails
`--check` runs in the red-but-non-blocking **Feature catalogue** CI job (same posture as
Doc hygiene — not a required check). The 90-file debt was driven to **0 uncataloged / 131
files**: every implementation file now maps to a catalogue entry. **Strand 5 (hygiene)
SHIPPED** — `scripts/feature_hygiene.sh` (same job) checks dual-anchoring + dangling tags:
**0 dangling, catalogue fully dual-anchored** bar the `@F43` stub. **Remaining:** only the
strand-3 follow-on (rendering LOFT.md's per-feature sections + HTML from the issues). This
README is the single source of truth for per-strand status.

## Goal

A lightweight, **source-validated catalogue of what loft is made of**: every user
feature (`@F###`) and infrastructure subsystem (`@I###`) is a gh-minted entry with a
title and — for features — a runnable, cross-backend-tested example; per-feature user
docs are generated from those entries, and a **source-coverage gate** proves the
catalogue is complete to within a few lines, directly from loft's source.

## Success criterion — gated on doc improvement

**The only justification is significantly better language documentation.** All the
machinery (`@F`/`@I` tags, `idx`, coverage gate, sync automation) is *scaffolding* — it
*enforces* completeness, no-drift, tested examples, and findability, but it does **not
write a good doc**. **The value is the authoring** — a consistent, complete, tested
`## What it is` / `## How it aids you` / `## Example` per feature; the scaffolding only
keeps authored docs good. Machinery without authoring = a beautifully-tracked catalogue
of thin docs, and **not worth the effort**.

**Go/no-go (cheap, before more machinery):** author 2–3 features fully (e.g. `@F17`
named args, `@F29` match, `@F22` closures) as self-contained what/how/tested-example
issues and compare side-by-side with today's LOFT.md sections. **Materially better** →
build the minimal scaffolding to scale it. **Not** → stop. An afternoon of writing, not
a build. This is the falsification test for the whole plan.

**Result — GATE CLEARED (2026-07-01).** Pass 3 authored 79/80 issues; the proof
([PASS3-authoring-proof.md](PASS3-authoring-proof.md)) found the lift *significantly
better for the terse half, modest for the already-detailed half*. The decisive evidence
came from the authoring itself: because **every `## Example` is run**, authoring caught
reference errors a fragment never would — `sizeof(integer)` documented as `4` (it is
**8**) and an `n: integer = null` idiom the language **rejects** (the form is
`null as integer`). Both fixed. That is the improvement made concrete — the format
enforces *correctness*, not just consistency.

## Effort + design

- **Effort:** MH — most of it rides existing machinery; the genuinely new piece is the coverage gate's per-region span attribution.
- **Design:** ✓ (doc-source architecture settled: self-contained zero-deferral issues, one-way derive, sync automation with drift + example CI guards).
- **Last touched:** 2026-07-01.

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
- **The issue is the canonical, self-contained documentation — ZERO deferral.** Each
  entry is a complete standalone doc that **never points at the project for any of its
  content**: the **title** (value sentence) plus, in the body, **`## What it is`**
  (precise prose, not keywords), **`## How it aids you`** (concrete), and **`## Example`**
  (inline, runnable, cross-backend). `@I` entries carry a full role description + a
  source-region *locator* (a pointer, not deferred content). Opening the issue tells you
  the whole feature without leaving it. This is the locked issue-body template.
- **Everything else derives one-way, by automation.** A generator pulls the issues and
  writes a **committed in-project mirror** — per-feature docs that agents `grep` and
  `scan.loft` indexes, and each `## Example` **extracted into `tests/docs/@F<n>.loft`**
  so CI runs it cross-backend. LOFT.md's per-feature sections + the user-facing HTML are
  **rendered from the issues** too; only LOFT.md's conceptual/guide layer stays
  hand-written. The issue never references the mirror — the mirror is its shadow. Two
  **anti-drift guards (Goal E):** (1) a CI check regenerates the mirror and **fails on
  any diff** — the copy can't silently lag; (2) the extracted example is a real test —
  **every example must run or CI is red**. So "fully filled" is *machine-verified*, not
  trusted. This is the per-feature triplication (LOFT.md / topic source / HTML) collapsed
  to one source: the issue.
- **Status is derived** — read from the extracted example's per-backend `tests/docs`
  pass, never hand-maintained. `@F` → user docs; `@I` → an internal architecture map,
  **never the user-facing site**.
- **Dual-anchor (the *locator* layer):** bare `@F<n>` / `@I<n>` in the docs +
  implementing source; `idx` joins them — the `@P###` split. This says *where the feature
  lives*; the issue says *what it is*. Two separate layers, both from the one number.
  `idx tag:@F<n> --code` / `--doc` / `--mirror` splits a tag's references into
  implementation, documentation, or just the canonical mirror page
  `doc/features/<slug>.md`.
- **Source-coverage gate (the keystone):** every substantive source region is
  attributed to an `@F` (per-capability) or `@I` (coarse, subsystem-level) entry; the
  residue — attributed to **neither** — must stay under a **ratcheting budget** (the
  "few lines that dodge"). A per-region threshold *K* forces bulk code to claim an
  entry while letting trivial glue dodge. Because infra is a **named** entry, not an
  anonymous exemption, the attribution is **auditable** — no silent dumping ground.

## Sub-arcs

| Item | Status | Notes |
|---|---|---|
| **1 — Identity** | **Done** | `loft-lang/features` stood up; **80 issues minted** (`@F1`–`@F56`, `@I57`–`@I80`); `kind:feature`/`kind:infra` labels. |
| **2 — Scanner / index** | **Shipped** | `scan.loft` indexes `@F`/`@I` (same byte-scan as `@P`, no trailing-letter rule); `idx tag:`/`prefix:` work; verified on match + reject cases. |
| **3 — Authoring + issue→project sync (the value)** | **Shipped (core); LOFT.md/HTML render deferred** | ✅ **Authored** 80/81 self-contained issues (`@F43` deferred as a library); ROI gate cleared (caught + fixed real doc drift). ✅ **Sync automation** — `tools/features/gen.loft` (@I81) reads the committed `index/features.json` snapshot and regenerates the mirror `doc/features/` (agents `grep`/`idx` + a generated TOC) + 45 runnable examples `tests/docs/features/`; fragments (library/syntax examples, no `fn main`) + the unauthored stub are mirrored/skipped, not run-tested. ✅ **Two CI guards green** — no-drift (`make features-check`, wired into the index-hygiene job) + example-must-run on both backends (`tests/features.rs` interpret, `tests/native.rs::native_features`). ⬜ Follow-on: render LOFT.md per-feature sections + user-facing HTML from the issues. |
| **4 — Source-coverage gate (keystone)** | **Shipped (file-level; 0 debt)** | `scripts/feature_coverage.sh` — a file is attributed if it carries any `@I` (coarse subsystem) or `@F` (per-capability) tag; an untagged impl file over the `MIN_LINES` floor is uncataloged. Ratchets `.feature_coverage_baseline` (mirrors `lint_comments.sh`: `--baseline`/`--check`/`--prune`); `--check` fails on any NEW uncataloged file, run in a dedicated red-but-non-blocking **Feature coverage** CI job (not a required check — same posture as Doc hygiene). Baseline driven **90 → 0 / 131**: every impl file maps to a catalogue entry. *Scope note:* file-level (matching coarse `@I`) — it doesn't force a fresh `@F` on a new function inside an already-tagged subsystem; that stays review judgement. |
| **5 — Hygiene** | **Shipped** | `scripts/feature_hygiene.sh` — every `@F`/`@I` dual-anchored (≥1 code + ≥1 doc site) + no dangling tag (a ref to a number with no catalogue entry). Runs in the **Feature catalogue** CI job (red-but-non-blocking) beside coverage. Current: **0 dangling, 0 missing-doc, 1 missing-code** (the `@F43` stub) — the catalogue is fully dual-anchored. The cross-doc status-drift check (@P251 class) is moot here — the mirror is generated, so it can't drift from the issue. |

## Phase ordering

1 (identity) → 2 (scanner + `idx features` — cheapest, immediate discoverability) →
4 (coverage gate, seeded at today's count and ratcheted down — this is what *forces*
tagging, so it lands early to drive adoption) → 3 (doc generation, rides `gendoc`).
Hygiene (5) folds into 2 and 4. Adoption is a **ratchet, not a big-bang backfill**:
new bulk code must claim an entry from day one; the existing backlog is chipped down.

## Backfill checklist — adopting the catalogue over the existing tree

The ratchet (above) stops *new* untagged bulk; this is how the *existing* backlog is
tagged — **three passes, each completed before the next starts**, so the catalogue is
reviewed as a whole before any tag is written, and source tags land before doc tags.

**Pass 1 — Map source → propose → create issues. ✅ DONE** (mapping reviewed; 80 issues minted).
- [ ] Walk the source roots (`src/parser/`, `src/state/`, `src/fill.rs`, `src/generation/`, `src/database/`, `default/*.loft`, …), grouping regions into candidates: user-facing capability → `@F`; internal subsystem → `@I` (coarse — one per subsystem).
- [ ] For each candidate, draft a value/role **title** and the **source region** it covers.
- [ ] Emit the proposed mapping as a **review artifact** (region → proposed `@F`/`@I` + title) — written down, nothing created or tagged yet.
- [ ] Review for **coverage + granularity** (every substantial region claimed; `@I` stays coarse), then **create the approved entries** as `loft-lang/features` issues — this mints the `@F###`/`@I###` numbers.

**Pass 2 — Details pass: write the tags in the SOURCE. ✅ DONE** *(bare `@F`/`@I` tags at the 79/80 implementing sites; build green, comment-only; **unrelated to the authoring/ROI gate** — coverage + discoverability groundwork)*
- [x] Write the bare `// @F<n>` / `// @I<n>` at each implementing site (per-function for `@F`, subsystem-level for `@I`).
- [x] `make index` + `idx features` / `idx infra` to confirm each tag links to its code site.
- [x] Seed the **coverage gate** (strand 4) baseline at the now-current unattributed count; ratchet down from there. *(seeded: 90 uncataloged files in `.feature_coverage_baseline`.)*

**Pass 3 — Authoring pass: write the self-contained feature doc IN THE ISSUE. ✅ AUTHORING DONE** *(the substantive work — careful prose, not harvest; **this is where the docs improve, and where the ROI gate applies**; 79/80 authored, `@F` examples cross-backend-verified — automation + anchors below still open)*
- [x] For each `@F`, author the standalone body — `## What it is` (precise), `## How it aids you` (concrete), `## Example` (inline, runnable) — **zero deferral to the project**. For each `@I`, a full role description + source-region locator. *(79/80; `@F43` random deferred as a library.)*
- [x] Run the strand-3 automation: render the in-project mirror + extract `## Example` → `tests/docs`; the two CI guards (regen-diff, example-runs) must be green. *(shipped: `tools/features/gen.loft` @I81; guards `make features-check` + `tests/features.rs` + `native_features`.)*
- [x] Place the `<!-- @F<n> -->` doc anchor where the rendered per-feature section lands. *(the generator emits `<!-- @F<n> -->` + a `# @F<n>` header into each `doc/features/<slug>.md`.)*
- [x] Run the **hygiene checks** (strand 5): every entry declared once + dual-anchored (≥1 doc + ≥1 code). *(shipped: `scripts/feature_hygiene.sh`; 0 dangling, fully dual-anchored bar the `@F43` stub.)*

**Why this order:** map-before-tag reviews the catalogue as a whole (right granularity,
no premature numbers); source-before-docs lets the coverage gate — which measures
*source* — go green first, so the doc anchors then link into a catalogue that already
matches the code.

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
5. **~~Title/home~~ — RESOLVED:** the **issue** is the canonical, self-contained home (title + `## What` / `## How` / `## Example`, zero deferral); the in-project mirror + `index/features.json` are generated, drift-guarded shadows.
6. **~~`@F` page richness~~ — RESOLVED:** the issue is a *full* self-contained doc (not terse) — the rendered page is its shadow. One source, richness in the issue.
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
