<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# TRACKER_INDEX — `@P-id` / `@PLAN-id` indexer + viewer integration

**Status:** Active — opened 2026-05-13.

A small, self-rebuilding index of tracker references (P-issues
+ plan/phase IDs) across the loft repo, plus a CLI for
querying it and a viewer-side surface for browsing.  The index
becomes the canonical "where is this referenced?" answer for
both humans and Claude.

## Drivers

Three concrete problems this solves:

1. **Grep-based tag lookup is fragile.**  Today `grep -rn
   "P259" doc/` matches `P2590`, `2P259`, prose like "the
   P259 fix forward."  Adopting `@P\d+` syntax (and `@PLAN\d+`
   for plans) makes regex unambiguous: `grep -rn '@P259\b'`
   has zero false matches.

2. **Claude per-task token usage is dominated by `grep -rn`
   on docs.**  Today: every "where is this referenced?"
   question pulls dozens of files into context.  An indexed
   lookup is O(1) and pulls only the exact lines + few-line
   context.  Measurable token reduction per session.

3. **Plan-35 viewer needs a tag-aware navigation surface.**
   The /welcome landing page (phase 35-08) shows "open
   problems / recently fixed / open plans / etc." — that data
   needs a structured source.  PROBLEMS.md row-parsing is
   brittle; an index built from `@P-id` mentions is robust.

The user explicitly asked for this initiative IN ADDITION to
plan-35: "the original design ... the index that keeps up to
date with a command line tool for you to inspect it.  And
integration with the viewer."  Filed as a sibling plan rather
than a plan-35 phase because its scope is independent — the
indexer is useful even without the viewer; the viewer is
useful without the indexer.

## Architecture

```
.gitignore       /index/                       # output dir, gitignored
.git/hooks/      pre-commit → make index       # auto-refresh
scripts/         idx                           # CLI grep wrapper
tools/indexer/                                 # the scanner
  scan.sh        bash + grep + jq pipeline     # tier-1 implementation
  ARCHITECTURE.md design notes
index/           tags.json                     # {tag: [{file, line, ctx}]}
                 broken.json                   # broken @ref audit
                 stats.json                    # counts per tag prefix
```

Three CLI surfaces:

- **`make index`** — rescan the repo, write `index/*.json`.
  Idempotent, ~1 second on the loft tree.
- **`./scripts/idx <query>`** — query the JSON.  Supported
  query forms:
  - `idx tag:@P259` — exact tag match.
  - `idx prefix:@PLAN22` — all tags starting with prefix.
  - `idx file:doc/claude/PROBLEMS.md` — all tags in a file.
  - `idx broken` — list broken @ref links.
- **`make index-watch`** — optional file-watcher for live
  refresh during heavy editing sessions.  Stretch.

Output is JSON-first; humans and the viewer parse the same
data.

## Tag conventions

Two tag families, both prefixed `@`:

| Tag | Regex | Examples |
|---|---|---|
| **P-issues** | `@P\d+\b` | `@P259`, `@P262`, `@P229b` |
| **Plans + phases** | `@PLAN\d+(?:-[\w]+)*\b` | `@PLAN22`, `@PLAN35-01`, `@PLAN22-2d-iii.a` |

Adoption is INCREMENTAL.  Old bare-name forms (`P259`,
`plan-22 phase 03`) continue to work — the indexer ignores
them.  New documents use `@P259` / `@PLAN35-01`; old
documents get retroactive `@`-tagging via a one-time sed pass
when convenient.

The `@` prefix is the discriminator that makes regex trivial.
Plain words `P259` in prose stay readable; `@P259` is the
machine-grep'able form.

Tag bodies follow these rules:

- **Lowercase + alphanumeric only**: `@P259` not `@p259` not
  `@P_259`.
- **Slash-free**: phase IDs use `-` to separate
  (`@PLAN35-01`), not `/` (would conflict with file paths).
- **Sub-phases via `.`**: `@PLAN22-2d-iii.a` is allowed.
  Mirrors plan-22's `02d-iii.a` directory shape.

## Phases

| # | Phase | Effort | What ships | Status |
|---|---|---|---|---|
| 0 | [Tag convention + initial indexer](00-convention-and-scanner.md) | XS | `tools/indexer/scan.sh` + `make index` target + CLAUDE.md docs of the tag convention.  No retroactive tagging yet — indexer scans both old (`P259`) and new (`@P259`) forms with separate prefixes for transition tracking. | Open |
| 1 | [CLI query wrapper](01-cli-query.md) | XS | `scripts/idx` bash wrapper around `index/tags.json`.  Supports `tag:` / `prefix:` / `file:` / `all` / `broken` / `help`.  CLAUDE.md updated to recommend it as the canonical reference-lookup. | **Shipped 2026-05-13** |
| 2 | [Auto-refresh on commit](02-auto-refresh.md) | XS | `tools/indexer/install-hook.sh` writes a marker-bracketed snippet to `.git/hooks/pre-commit`; idempotent across re-runs.  Hook re-runs the scanner when an indexed file is staged.  `make index-install-hook` invokes it.  DEBUG.md gains § Tracker-tag indexer with install + usage docs. | **Shipped 2026-05-13** |
| 3 | [Broken-tag validator](03-broken-validator.md) | S | Indexer detects `@P-id` references that don't resolve (e.g., `@P9999`) AND `@PLAN-id` references whose plan dir doesn't exist.  CI gate via `tests/index_hygiene.rs`. | Open |
| 4 | [Plan-35 viewer integration](04-viewer-integration.md) | S | Plan-35 viewer reads `index/tags.json` and surfaces tag references.  Each plan README + PROBLEMS row links to all its references.  /welcome landing's "where could I help" tags pulled from this data. | Open (depends on plan-35 phase 08) |
| 5 | [Claude integration](05-claude-integration.md) | XS | Update CLAUDE.md "## Key commands" with `./scripts/idx <query>` as the canonical reference-lookup.  Add a § Tag convention section.  Optional MCP wrapper for token-efficient queries. | Open |
| 6 | [Retroactive tagging + closeout](06-closeout.md) | S | One-shot sed pass: convert `P\d+` → `@P\d+` in PROBLEMS.md / plan READMEs / commit-message conventions.  CHANGELOG entry.  Move plan to finished/. | Open |

Total estimated effort: **~1 week** of focused work.  Phases
00 + 01 are the minimum viable indexer (~1 day); the rest
compound the value.

## Acceptance — full plan

- `@P\d+` and `@PLAN\d+(?:-[\w]+)*` are the canonical forms;
  ROADMAP / DEVELOPMENT / CLAUDE.md document the convention.
- `make index` rebuilds `index/tags.json` in ≤ 2 seconds on
  the loft tree (~1100 .md/.rs/.loft files).
- `./scripts/idx tag:@P259` returns JSON with all references
  to P259 + 2-line context per match.
- Pre-commit hook keeps `index/` fresh on every commit.
- `tests/index_hygiene.rs` catches broken `@P-id` /
  `@PLAN-id` references at CI time.
- Plan-35 viewer `/welcome` landing pulls "open problems /
  recently fixed / active plans / future plans" from the
  index instead of grep'ing files at request time.
- CLAUDE.md instructs Claude to use `./scripts/idx` for
  reference lookups (measurable token reduction per session).
- All 6 phases close → plan moves to `plans/finished/37-…`.

## Why this is a separate plan from plan-35

Plan-35 ships a viewer.  Plan-37 ships an indexer.  They
INTERSECT at one phase (35-08 newcomer landing pulls
data from the indexer; 37-04 wires the viewer to read it),
but the indexer is useful **without** the viewer (Claude
queries it directly via `./scripts/idx`) and the viewer is
useful **without** the indexer (the existing tree + file
rendering works regardless).

Splitting also keeps each plan ≤7 phases — plan-35 was
already at 7 with two stretches (08 + 09 + 10) waiting.
Adding the indexer to plan-35 would have pushed it to ~12
phases, beyond the "max 3 active plans" cap's spirit.

## Cross-references

- [`plans/35-branch-review-viewer/README.md`](../35-branch-review-viewer/README.md)
  — the viewer this plan integrates with.
- [`plans/35-branch-review-viewer/README.md § Stretches`](../35-branch-review-viewer/README.md#stretches-post-v1-listed-for-traceability)
  — phase 35-08 newcomer landing depends on this plan's
  data shape.
- [`PROBLEMS.md`](../../PROBLEMS.md) — primary source for
  P-id references; the indexer parses its row format.
- [`ROADMAP.md`](../../ROADMAP.md) — gets a row in
  § Near-term focus once a phase ships.
