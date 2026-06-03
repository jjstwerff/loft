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
  keep the legacy `@P###` token in the title so OLD doc references still grep
  (`gh search issues "@P396"`).  The canonical way docs *reference* an issue is the
  indexed `@GH###` token (below), not the bare gh number.
- **Reference token: `@GH###`** (the indexed tracker) — docs reference a gh issue
  as `@GH<number>` (e.g. `@GH247`), NOT bare `#247` / `loft#247`.  The `@`-prefix
  makes it an INDEXED token exactly like `@P###` / `@PLAN###`: `scan.loft` finds
  every reference + backlink **fully offline**, and `@GH247` maps to a
  deterministic URL (`github.com/<repo>/issues/247`) with no `gh` call.  Token
  families: `@P###` = legacy/closed (PROBLEMS.md archive), `@PLAN###` = plans,
  `@GH###` = live issues.  Optional validation (does it exist / is it closed) is a
  bolt-on `make index-gh` (`gh issue list --json number,state`), not a
  prerequisite.  Cross-repo: bare `@GH###` = this repo; a qualified spelling for
  other repos (`@GH:<repo>:<n>`) is TBD and the less-common case.
- **Labels** — meanings live in [`.github/LABELS.md`](../../.github/LABELS.md)
  (the glossary so other agents don't dig into the loft source to learn what
  `area:codegen` covers) + each label's GitHub `description` (the inline gloss).
  Beyond GitHub's defaults `bug`/`enhancement`/…:
  - severity: `sev:high` / `sev:medium` / `sev:low`
  - **workaround** (created): `wa:clean` / `wa:partial` / `wa:none` — see
    [§ Workarounds](#workarounds--the-agents-can-you-keep-moving-signal)
  - area: `area:codegen` / `area:closures` / `area:store-lifetime` /
    `area:parser` / `area:native` / `area:wasm` / `area:stdlib` /
    `area:packages` / …
  - cross-cutting: `both-backends`, `needs-design`
- **Cross-repo** — a bug in repo A that blocks repo B → an Issue in A, referenced
  from a `blocked-by`-labelled tracking Issue in B (`jjstwerff/loft#247`).  The
  dogfood loop (moros / dryopea drive loft) lives on these links.
- **Roadmap** — a `gh` Project board across the orgs for "which release bundles
  which consumer-driven work"; ROADMAP.md can't span orgs.

## Workarounds — the agent's "can you keep moving?" signal

A bug's workaround is the **primary thing the loft agent communicates to others**
(moros / dryopea / library authors / users): *can you route around this, or are
you blocked?*  Every bug carries both halves:

- a **`### Workaround` section** in the body — what to do today, OR what you tried
  that did NOT work;
- a **`wa:*` label** (the queryable metadata):
  - `wa:clean` — a simple idiomatic alternative, **verified working**;
  - `wa:partial` — a workaround exists but is awkward / loses the intended
    behaviour, **verified working**;
  - `wa:none` — nothing works; this **blocks** whoever hits it.

**THE RULE: a workaround is VERIFIED or it is not claimed.**  Run it on current
HEAD, both backends, and put the command + result in the section.  An **unverified
or WRONG workaround is worse than `wa:none`** — a downstream developer who follows
a confident-but-broken workaround loses time *and* stops trusting the signal,
which is the signal's entire value.  Same rigor as the probe-first fix rule: a
claim is a thing to *verify*, not assert.  (Pilots: #246 `wa:none` — the rebind
was tested, yields `[]`; #247 `wa:partial` — the non-capturing alternative was run
on both backends, printed `10`/`7`.)

**Triage:** `gh issue list --label "wa:none"` is the blocked-set — often more
urgent than raw `sev:`, because severity is "how bad when hit" and `wa:` is "can
you avoid being hit."

## Resolving an issue (the close half)

Filing is half the loop; closing is the other half.

- **Reference the issue in the fixing commit** — `Fixes #NNN` / `Closes #NNN` in
  the commit (or PR body); GitHub auto-closes it when that lands on the default
  branch.  On a working branch with no PR, close manually after pushing
  (`gh issue close NNN --comment "fixed in <hash>"`).
- **A fix needs a regression** — link the `tests/scripts/NNN` / `tests/*.rs` that
  locks it in.  A closed issue with no regression is a re-opening waiting to
  happen.
- **Re-verify the workaround on close** if the issue had one — a fix can make a
  `wa:partial`/`wa:none` moot; keep the closed record accurate.
- **Don't file a bug you fix in the same change** — the fix + its test ARE the
  record (CLAUDE.md § Bug-filing policy).

## Features & enhancements

Bugs are the focus above.  FEATURE requests use the `feature_request` template +
the `enhancement` label, and connect to the roadmap: a planned feature lives in
ROADMAP.md / PLANNING.md (or a `plans/` slot if multi-phase); a `gh` Project board
tracks "which release bundles which consumer-driven work" across the org repos.
The Issue is the lightweight capture; the plan/roadmap is the design + sequencing.

> **Transition note.** "PROBLEMS.md" / "P-issue row" references elsewhere in the
> docs (plans/README, DEVELOPMENT.md, …) are repointed to GitHub Issues as they're
> touched.  PROBLEMS.md is frozen to OPEN bugs — it's the closed/historical archive.

## Migration plan

| Step | Action | Status |
|---|---|---|
| 1 | **Pilot** — file @P396/@P397 as Issues (#247/#246), drop from PROBLEMS.md | ✅ done |
| 2 | Create the `sev:*` / `area:*` / `wa:*` / cross-cutting labels | ✅ done (17: sev:/area:/wa:/regression/flaky/blocked-by/hit-by:) |
| 2.5 | **`@GH###` indexed tracker** — add `@GH` as a recognized prefix in `scan.loft` (reference-finding + backlinks + deterministic issue URL, all offline); broaden `idx broken` to not false-flag migrated `@P###`; optional `make index-gh` validation bolt-on. | ☐ **REMAINING** (the one code task; until then `idx broken` may flag the 7 migrated `@P###`) |
| 3 | Migrate the OPEN PROBLEMS.md rows → Issues | ✅ done — @P391→#248, @P389→#249, @P384→#250, @P351→#251, @P340→#252 (+ pilots #246/#247) |
| 4 | Freeze PROBLEMS.md — closed/historical record; FIXED rows + `###` design entries stay | ✅ done — freeze header + `@P→#` map; 0 open rows left |
| 5 | Flip the meta-doc filing rule "→ PROBLEMS.md" → "→ GitHub Issue" | ✅ done — CLAUDE.md (bug-filing + doc index + reading-by-goal), `_INVESTIGATION_TEMPLATE § Closing`, `plans/README § workflows` |
| 6 | Retire / repoint USER_FACING.md (Issues are user-facing; the mirror is redundant) | ☐ remaining |
| 7 | Apply the template + labels in dryopea / lavition / `loft-libs-*` as each needs bug-filing | ☐ ongoing |

## Agent note

`gh issue list/view/create/search` is the bug-layer interface; `idx` + files keep
serving plans/investigations.  Repros stay in-repo (commits, `tests/scripts/`,
`probes/`), so fixing a bug still has its context local — the Issue is the
tracker, the repo holds the artifacts.  The grep-ability lost on open bugs is
small; the grep-ability kept on everything that matters for *fixing* is total.
