<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 04 — Plan-35 viewer integration

**Status:** Open (depends on plan-35 phase 03 + 08)

## Goal

Wire the loft-view binary to read `index/tags.json` and surface
tag references as cross-doc navigation.  The `/welcome`
landing page (plan-35 phase 08) consumes the index buckets
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
| `/welcome` | Landing page (plan-35 phase 08) — uses index data for the bucketed status view |

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
  that mentions P259.
- Editing a doc + reloading the page → if the index hasn't
  refreshed, the "stale index" footer note appears.
- Missing `index/tags.json` → "no index — run `make index`"
  banner; viewer doesn't crash.

## Risks

| Risk | Mitigation |
|---|---|
| Loft's JSON parser ergonomics for `vector<struct>` aren't yet smooth | If parsing into typed structs is painful, parse into a flatter representation (parallel arrays); or fall back to scanning the file with hand-written parsing.  File a P-issue against the JSON ecosystem if this surfaces. |
| Index reads on every request hurt latency | Cache parsed index in memory; invalidate on file mtime change.  Use loft closures (just shipped via plan-22) for the cache. |
| Tag references in source files (e.g., `// P259` in a Rust comment) point at a doc but aren't the doc's responsibility | Per-doc sidebar shows refs grouped by where they live (docs vs code) so the user can filter mentally |

## Dependencies

This phase depends on:

- **Plan-35 phase 03** — markdown rendering (the per-doc page
  needs HTML-rendered docs to attach a sidebar to).
- **Plan-35 phase 08** — `/welcome` landing route + bucketed
  layout.

If those aren't shipped yet, phase 04 stays open.  Phases
00-03 of plan-37 ship independently and don't block on the
viewer.

## Cross-references

- [Phase 00 — scanner](00-convention-and-scanner.md) — produces the data
- [Plan-35 phase 03](../35-branch-review-viewer/03-markdown-minimal.md)
- [Plan-35 phase 08 — newcomer landing (stretch)](../35-branch-review-viewer/README.md#stretches-post-v1-listed-for-traceability)
