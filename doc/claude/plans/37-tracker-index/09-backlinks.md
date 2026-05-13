<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 09 — Backlinks: "who links to me"

**Status:** Shipped 2026-05-14 — detection + CLI surface land
on the current branch.  Viewer "Referenced by" sidebar
(@PLAN35 phase 04 follow-up) is the only deferred sub-task;
data + queries are ready for it to consume.

## Goal

Extend the index to answer **"who links to me?"** for any
file or tag.  Two flavors, both heavily used in plan dirs:

1. **Tag backlinks** — `@P259` or `@PLAN22` mentions across
   the tree.  Already partially solved by phases 00-03;
   phase 09 surfaces them as first-class queries on the
   referenced ENTITY (not just on the tag string).
2. **File backlinks** — `[text](path/to/doc.md)` markdown <!--noindex-->
   links.  Indexed per-target so any plan README can ask
   "what other docs cite me?"

The user's framing (2026-05-13): "add to the eventual index
a who links to me (both regular tags and file tags mostly
in use for plans)".  Plans cross-reference each other
densely (@PLAN22 → @PLAN15 → @PLAN09 → ...); knowing the
inbound edges is essential for refactor + closeout work
without grep'ing 250 files.

## What ships

### Scanner extension — markdown link extraction

The bash scanner (and later the loft scanner) gets a third
extractor pass alongside `@P-id` + `@PLAN-id`:

- Match: `\[([^\]]*)\]\(([^)]+\.md)(#[^)]*)?\)`
- Capture: link text, target path, optional anchor.
- Resolve target path against the current file's directory:
  - Drop `./` prefix.
  - Apply `..` segments.
  - Normalise to a path relative to repo root.
- Skip schemes (`http://`, `https://`, `mailto:`).

### Index output — new `links` bucket

`index/tags.json` gains a top-level key:

```json
{
  "@P259":            [ {file, line, context}, ... ],
  "@PLAN22":          [ ... ],
  "broken":           [ ... ],
  "links": {
    "doc/claude/plans/finished/22-mutable-closures/README.md": [
      {"file": "doc/claude/CHANGELOG_TECHNICAL.md", "line": 23,
       "anchor": null, "context": "..."},
      {"file": "doc/claude/plans/finished/35-branch-review-viewer/README.md",
       "line": 99, "anchor": "drivers", "context": "..."}
    ],
    "doc/claude/PROBLEMS.md": [ ... ]
  }
}
```

Keys are absolute-from-repo-root paths.  Each entry lists
the `(file, line)` of every link pointing at that target,
plus the anchor fragment if present.

For consistency, files referenced via tags ALSO get an
entry (the file containing the tag's row in PROBLEMS.md, or
the plan README for `@PLAN22`).  This unifies the answer
to "who cites this entity?" across both link styles.

### CLI form — `idx incoming:<path>`

```bash
$ ./scripts/idx incoming:doc/claude/plans/finished/22-mutable-closures/README.md
[
  {"file": "doc/claude/CHANGELOG_TECHNICAL.md", "line": 23,
   "anchor": null,
   "context": "  - 22-mutable-closures (2026-05-13) — closures novices expect"},
  {"file": "doc/claude/plans/finished/35-branch-review-viewer/README.md", "line": 99,
   "anchor": "drivers",
   "context": "...uses [plan-22 closures](../finished/22-mutable-closures/README.md#drivers)..."},
  ...
]
```

Plus a tag form for symmetry:

```bash
$ ./scripts/idx incoming:@PLAN22
# Same as `idx tag:@PLAN22` but via the unified backlink lens.
```

The query path normalises trailing `/` and missing `.md`
extensions for ergonomics:

- `idx incoming:plans/finished/22-mutable-closures/`
  resolves to the README.
- `idx incoming:PROBLEMS.md` resolves the partial name to
  the unique full path if unambiguous.

### Viewer integration (@PLAN35 phase 04 + 08)

The viewer's per-file pages gain a "Referenced by" sidebar:

```
─── doc/claude/plans/finished/22-mutable-closures/README.md
                                            ┌─────────────────
                                            │ Referenced by (8)
                                            │ ▸ CHANGELOG_TECHNICAL.md:23
                                            │ ▸ plan-35 README §drivers
                                            │ ▸ plan-37 README §intro
                                            │ ▸ ... (5 more)
                                            └─────────────────
```

Same data source — `index/tags.json`'s `links` bucket.
Viewer phase 04 already mentions this; phase 09 here makes
the underlying data exist.

### Plan-doc maintenance use case

When a plan moves from `future/` to `plans/N-…/` to
`finished/`, every link pointing at the old path becomes
broken.  Today this is caught (sometimes) by manual diff
review; phase 09 surfaces it explicitly:

```bash
$ ./scripts/idx incoming:doc/claude/plans/future/22-mutable-closures/
# (after the move to finished/, this returns leftover refs
# at any callers that didn't update their paths)
```

The broken-link audit (phase 03) can EXTEND to flag
incoming links to non-existent paths — same machinery,
applied to the `links` bucket.

## Critical files

| Path | Action |
|---|---|
| `tools/indexer/scan.sh` | EXTEND: third extractor pass for markdown links; output the `links` bucket |
| `scripts/idx` | ADD `incoming:<path>` form (works on both file paths and `@`-tags) |
| `tools/indexer/scan.loft` (@PLAN37 phase 07) | EXTEND when ported: same extractor + bucket |
| `tools/viewer/src/main.loft` (@PLAN35) | CONSUMER — phase 04 of @PLAN35 reads this bucket for the per-file sidebar |
| `tests/index_hygiene.rs` | EXTEND: validate that broken file links (target doesn't exist) fail CI |

## Acceptance — shipped state

- `./scripts/idx incoming:doc/claude/PROBLEMS.md` returns
  **41** files that cite PROBLEMS.md ✓
- `./scripts/idx incoming:doc/claude/plans/finished/22-mutable-closures/README.md`
  returns **20** docs citing @PLAN22 ✓
- `./scripts/idx incoming:doc/claude/plans/finished/22-mutable-closures/`
  resolves trailing `/` to README.md (same 20 results) ✓
- `./scripts/idx incoming:PROBLEMS.md` (basename only):
  returns `{ambiguous: [...]}` listing the 4 candidate
  paths ending in `/PROBLEMS.md` — caller picks the
  intended one ✓
- `./scripts/idx broken-links` returns broken markdown
  links (61 surfaced today on the loft tree — most are
  off-by-one `..` counts in `doc/claude/plans/<dir>/README.md`
  citing top-level docs as `../X.md` instead of `../../X.md`)
- Path resolution handles `..`, `./`, anchors, repo-root
  `/...` paths, and trailing slashes ✓
- Performance: scanner runs in **1.5 sec** on the 953-file
  loft tree ✓ (target was < 2 sec; new link extraction
  added ~0.3 sec)
- Viewer (@PLAN35 phase 04 follow-up): "Referenced by"
  sidebar consumes `.links` bucket — **deferred**, not
  blocking phase 09 close.

## Follow-ups filed

- **Broken-link cleanup** — 61 markdown links across the
  doc tree resolve to non-existent targets.  No CI gate
  added to `tests/index_hygiene.rs` yet (would lock in the
  cleanup as a release-blocker prematurely; per the user's
  framing, ship detection first, gate after the backlog
  is cleared).  Categories:
  - ~48 in `doc/claude/plans/<dir>/README.md` citing
    top-level reference docs (DESIGN.md, PROBLEMS.md, …)
    with `../X.md` instead of `../../X.md`.
  - 3 @PLAN22 references at the old `plans/22-` path
    (move to `finished/` happened during the plan close).
  - 3 @PLAN35 references at the old `plans/35-` path
    (same closeout drift).
  - 3 lib_plan typos (`doc/claude/lib_plans/plans/...`).
  - 5 stale `.claude/skills/` references.
  - 5 missing-doc citations (`DX.md`, `LSP.md`,
    `WEB_SERVER_LIB.md`, `FOO.md`, `WASM.md`).
  - Run `./scripts/idx broken-links | jq '.[] | .target'`
    for the live list.
- **Viewer "Referenced by" sidebar** — wire `tools/viewer/src/main.loft`'s
  per-file route to read `.links[<path>]` and render a
  sidebar.  Data is ready; UI work is @PLAN35 phase 04
  scope.

## Risks

| Risk | Mitigation |
|---|---|
| Markdown link regex over-matches code-fenced examples | The `<!--noindex-->` marker (phase 03) applies; phase 09 also skips lines starting with whitespace + ` ``` ` (fenced code block boundaries) |
| Path-resolution edge cases (Windows backslashes, UNC paths) | Document: forward-slash paths only.  Reject backslashed paths during validation. |
| Backlinks bucket bloats `tags.json` | Single-pass index; per-target entry is small (~50 bytes / link).  Loft tree has ~3000 links → ~150 KB added.  Negligible relative to total file size. |
| Anchor fragments not validated against destination | Phase 09 stores anchors but doesn't verify them.  A future phase can validate anchors against the destination file's heading slugs. |
| Refactor-driven mass updates churn the bucket on every commit | The `links` bucket is regenerated from scratch on each `make index`; auto-refresh hook (phase 02) keeps it fresh.  No incremental-update complexity. |

## Cross-references

- [Phase 00 — scanner](00-convention-and-scanner.md) — extended here with markdown link extraction
- [Phase 01 — CLI](01-cli-query.md) — extended here with `incoming:` form
- [Phase 03 — broken-tag validator](03-broken-validator.md) — extended here to also flag broken file links
- [Phase 04 — viewer integration](04-viewer-integration.md) — first major consumer of the `links` bucket
- [Plan-35 phase 03](../finished/35-branch-review-viewer/03-markdown-minimal.md) — markdown rendering uses the same link-resolution logic (link rewrite to `/file/<resolved>`)
