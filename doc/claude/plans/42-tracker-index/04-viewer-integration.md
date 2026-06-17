<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 04 — Plan-35 viewer integration

**Status:**
- **04a (`/tag/<tag>` route + missing-index banner)** — Shipped 2026-05-13
- **04b — per-doc sidebar** — Shipped 2026-05-14 (the
  "Referenced by" + "Tags on this page" sections appear at
  the bottom of every `/file/<path>` page; both render to
  empty when the index is missing or the file has no
  associated entries, so the page degrades gracefully).
- **04b — welcome landing** — Shipped 2026-05-14.  `/welcome`
  route in `tools/viewer/src/main.loft` consumes seven new
  `index/tags.json` buckets — `problems_open`,
  `problems_recent` (closed in last 30 days),
  `plans_active`, `plans_recent` (finished in last 60 days),
  `plans_future`, `plans_deferred`, `lib_plans_future`.
  Layout is a two-column grid (active + recently finished
  plans on top, future + deferred below) with full-width
  problem sections beneath.  Empty buckets suppress
  entirely; long lists collapse via `<details>`.  The page
  is independent of @PLAN35's never-shipped phase 08
  stretches — `tools/indexer/scan.sh` produces the buckets
  directly via bash + `awk` + `jq`, bypassing the loft-side
  curation-engine design 35-08 had envisioned.  Linked from
  the dashboard's quick-nav (`W` tile, replaced the `·
  Stdlib` slot — Stdlib remains accessible via `/tree/default`).

## What 04a shipped

A self-contained slice that the viewer could host without
waiting on @PLAN35:

- `GET /tag/<bare_name>` — reads `index/tags.json` via
  `json_parse`, surfaces BOTH the canonical (`@P259`) and
  legacy (`legacy:P259`) buckets together so the page is the
  full reference list during the migration.
- "No index" banner when `index/tags.json` is absent (instructs
  the user to run `make index`); viewer doesn't crash.
- "No references found" message when the tag has zero matches
  in either bucket.
- Landing page (`/`) gains an example-tag section pointing at
  `/tag/P259`, `/tag/P262`, `/tag/PLAN35`, `/tag/PLAN37`.

URL convention is `/tag/<bare>` (`/tag/P259`, `/tag/PLAN35-01`).
The page renders both `@P259` and `legacy:P259` together since
the user's mental model is "show me everything that mentions
@P259," not "discriminate between the two indexer keys."

**Pre-existing bug filed during 04a work**: @P264 — `json_parse`
mangles non-ASCII strings (3-byte `→` becomes 6-byte `âââ` due
to byte-by-byte codepoint widening in the JString decoder).
Affects the rendered context strings on tag pages.  Reproducer
saved to `/tmp/p_followups/p264_json_utf8.sh`.  Workaround for
the viewer is none — the corruption happens upstream of any
text the viewer can intercept.  Filed per the bug-filing policy
in CLAUDE.md.

## What 04b is still waiting on

## Goal (04b — original full plan)

Wire the loft-view binary to read `index/tags.json` and surface
tag references as cross-doc navigation.  The `/welcome`
landing page (@PLAN35 phase 08) consumes the index buckets
directly; per-doc pages get a "referenced from" sidebar.

## What ships

### Index reader in the viewer

`tools/viewer/src/index.loft` (new module — phase 04 may
trigger the move to multi-file viewer source if `main.loft`
is too dense by then):

```loft
struct TagRef {
    file: text,
    line: integer,
    context: text
}

pub fn read_index() -> File {
    file("index/tags.json")
}

pub fn refs_for_tag(tag: text) -> vector<TagRef> { ... }
pub fn open_problems() -> vector<text> { ... }      // P-ids open in PROBLEMS.md
pub fn closed_recently() -> vector<text> { ... }    // P-ids closed in last 30 days
pub fn active_plans() -> vector<text> { ... }       // dirs in plans/[0-9]*/
pub fn finished_recently() -> vector<text> { ... }  // last 60 days mtime
```

The reader uses loft's standard JSON parser.  If
`index/tags.json` is missing, render a "no index — run `make
index`" banner instead of crashing.

### New routes

| Route | Purpose |
|---|---|
| `/tag/<tag>` | All references to a tag, with file:line + 2-line context per match |
| `/welcome` | Landing page (@PLAN35 phase 08) — uses index data for the bucketed status view |

### Per-doc "referenced from" sidebar

When viewing `doc/claude/PROBLEMS.md` at `/file/...`, the
sidebar shows: "this file is referenced by N other files via
its @P-id rows."  Click → `/tag/@P259` → list of references.

When viewing a plan README, same: "this plan is referenced by
M other docs via @PLAN-id."

For source files (`.rs`, `.loft`), shows: "this file mentions
N tracker tags" → list of tags + their target docs.

### Stale-index detection

If `index/tags.json` mtime is older than the viewer process
start time AND any indexed file mtime is newer than
`tags.json`, render an unobtrusive "index is stale — run
`make index`" footer note.  Doesn't crash; doesn't auto-
rebuild (the viewer has no subprocess primitive).

## Acceptance

- Browse to `/welcome` → bucketed status (active plans,
  recently shipped, open problems, recently closed) all
  populate from `index/tags.json` data.
- Browse to `/file/doc/claude/PROBLEMS.md` → sidebar shows
  the tag references for each P-id row.
- Click any `@P259` link → `/tag/@P259` lists every file:line
  that mentions @P259.
- Editing a doc + reloading the page → if the index hasn't
  refreshed, the "stale index" footer note appears.
- Missing `index/tags.json` → "no index — run `make index`"
  banner; viewer doesn't crash.

## Risks

| Risk | Mitigation |
|---|---|
| Loft's JSON parser ergonomics for `vector<struct>` aren't yet smooth | If parsing into typed structs is painful, parse into a flatter representation (parallel arrays); or fall back to scanning the file with hand-written parsing.  File a P-issue against the JSON ecosystem if this surfaces. |
| Index reads on every request hurt latency | Cache parsed index in memory; invalidate on file mtime change.  Use loft closures (just shipped via @PLAN22) for the cache. |
| Tag references in source files (e.g., `// P259` in a Rust comment) point at a doc but aren't the doc's responsibility | Per-doc sidebar shows refs grouped by where they live (docs vs code) so the user can filter mentally |

## Dependencies

This phase depends on:

- **Plan-35 phase 03** — markdown rendering (the per-doc page
  needs HTML-rendered docs to attach a sidebar to).
- **Plan-35 phase 08** — `/welcome` landing route + bucketed
  layout.

If those aren't shipped yet, phase 04 stays open.  Phases
00-03 of @PLN42 ship independently and don't block on the
viewer.

## Cross-references

- [Phase 00 — scanner](00-convention-and-scanner.md) — produces the data
- [Plan-35 phase 03](../finished/35-branch-review-viewer/03-markdown-minimal.md)
- [Plan-35 phase 08 — newcomer landing (stretch)](../finished/35-branch-review-viewer/README.md#stretches-post-v1-listed-for-traceability)
