<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Issue tracking — investigations in files, open bugs in GitHub Issues

**Status: DRAFT (2026-06).**  Pilot done (#246/#247); the rest of the migration is
gated on approval.  Rationale + evaluation: this is the multi-project answer —
loft / dryopea / lavition / `loft-lang/*` all have bug-filing needs, and discrete
bugs are a commodity that GitHub Issues serves better than N per-repo markdown
files, while **investigations** (which work, and which GitHub can't hold) stay in
files.

## The split — what lives where

| Kind | Home | Why |
|---|---|---|
| **Investigations** (multi-phase, probe-driven bug *classes*) | files: `plans/*/` + `probes/` + cluster catalogue | working artifacts GitHub can't hold (probe corpora, cluster docs); agent-grepped; versioned with the code |
| **Open bugs** (discrete: repro / severity / workaround / fix-path) | **GitHub Issues**, per repo | commodity records; cross-repo refs; triage / search / milestones; external discoverability (Goal B); one uniform tool across all repos |
| **Closed bugs** (the FIXED record) | `PROBLEMS.md` (frozen archive) + git history + the regression test | the fix + its test ARE the record; keep the history greppable |
| **Design entries** (the big `###` P-sections, e.g. @P213) | reference docs (`doc/claude/*.md`) | these are design docs, not bug rows — they don't belong as one-line Issues |
| **Benign tradeoffs / open work** (not defects) | `QUALITY.md § Open work` | a known tradeoff with a fix mapped, not a bug |

**Investigations PRODUCE Issues.**  The closure rule
([`plans/_INVESTIGATION_TEMPLATE.md § Closing`](plans/_INVESTIGATION_TEMPLATE.md#closing-an-investigation-plan-required))
now files **GitHub Issues** (not PROBLEMS.md rows) for the still-open findings +
sibling bugs.  The file-based deep-dive emits commodity bug records into the
commodity tool; the active-phase no-file rule is unchanged.

## Convention (apply in EVERY repo — this is what makes multi-project pay off)

Without one shared convention, N repos' Issues drift exactly like N PROBLEMS.md
files would.  The win is the *uniform* convention, not GitHub itself.

- **Templates** — copy `.github/ISSUE_TEMPLATE/{bug_report,feature_request}.yml`
  into each repo (dryopea, lavition, `loft-libs-*`).
- **Title** — `[<area>] <one-line>`.  For any issue migrated from PROBLEMS.md,
  keep the legacy `@P###` token in the title so the ~100 existing doc references
  still resolve (`grep @P396` / `gh search issues "@P396"` finds it).  New issues:
  no `@P` token — the gh number is the ref.
- **Labels to create** (beyond GitHub's defaults `bug`/`enhancement`/…):
  - severity: `sev:high` / `sev:medium` / `sev:low`
  - area: `area:codegen` / `area:closures` / `area:store-lifetime` /
    `area:parser` / `area:native` / `area:wasm` / `area:stdlib` /
    `area:packages` / …
  - cross-cutting: `both-backends`, `needs-design`
- **Cross-repo** — a bug in repo A that blocks repo B → an Issue in A, referenced
  from a `blocked-by`-labelled tracking Issue in B (`jjstwerff/loft#247`).  The
  dogfood loop (moros / dryopea drive loft) lives on these links.
- **Roadmap** — a `gh` Project board across the orgs for "which release bundles
  which consumer-driven work"; ROADMAP.md can't span orgs.

## Migration plan

| Step | Action | Status |
|---|---|---|
| 1 | **Pilot** — file @P396/@P397 as Issues (#247/#246), drop from PROBLEMS.md | ✅ done |
| 2 | Create the `sev:*` / `area:*` labels (loft repo) | ☐ on approval |
| 3 | Migrate the remaining ~8 OPEN PROBLEMS.md rows → Issues (keep `@P` in title; link repro / probe / test) | ☐ |
| 4 | Freeze PROBLEMS.md — header it "closed/historical record"; FIXED rows stay; the `###` design entries graduate to docs or stay as reference | ☐ |
| 5 | Flip the meta-doc rule: `_INVESTIGATION_TEMPLATE § Closing` + `plans/README § Edge-probe`/`§ Sibling bugs` — "file → PROBLEMS.md" → "file → GitHub Issue" | ☐ |
| 6 | Retire / repoint USER_FACING.md (Issues are user-facing; the mirror is redundant) | ☐ |
| 7 | Apply the template + labels in dryopea / lavition / `loft-libs-*` as each needs bug-filing | ☐ ongoing |

## Agent note

`gh issue list/view/create/search` is the bug-layer interface; `idx` + files keep
serving plans/investigations.  Repros stay in-repo (commits, `tests/scripts/`,
`probes/`), so fixing a bug still has its context local — the Issue is the
tracker, the repo holds the artifacts.  The grep-ability lost on open bugs is
small; the grep-ability kept on everything that matters for *fixing* is total.
