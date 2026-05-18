<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01 — CLI query wrapper

**Status:** **Shipped 2026-05-13**.  All 7 query forms
verified end-to-end against the live `index/tags.json`.

## What actually shipped

`scripts/idx` (~150 lines) provides:

| Form | Behaviour | Verified |
|---|---|---|
| `idx help` (or no arg) | Print usage block | ✓ |
| `idx tag:@P259` | JSON array of refs to a single tag | ✓ |
| `idx prefix:@PLAN22` | Object: every tag with the prefix → refs | ✓ |
| `idx file:<path>` | Array of `{tag, refs}` for tags in that file (sorted) | ✓ |
| `idx all` | Array of `{tag, count}` sorted by count desc | ✓ |
| `idx broken` | Array of broken @-refs (populated by phase 03) | ✓ |
| `idx <unknown>` | Friendly error + exit 2 | ✓ |

### Context-extraction flags (added 2026-05-13)

When a tag's single-line `context` isn't enough to understand
what's around the reference, `tag:` queries accept excerpt
flags that read the file and add an `excerpt` field per ref:

| Flag | Behaviour |
|---|---|
| `--before N` | Include N lines BEFORE the tag's line (useful for in-code tags where setup context above the comment matters) |
| `--after N` | Include N lines AFTER the tag's line (alias: `--lines N` for back-compat) |
| `--para N` | Include lines until N consecutive empty lines AFTER the tag (overrides `--after`).  Combine with `--before` for full paragraph context. |
| `--max-bytes B` | Cap excerpt at B bytes (default 4096).  If the tag's line ALONE exceeds B (e.g., PROBLEMS.md's 4 KB-per-row table format), excerpt is truncated to B with `...[truncated]` suffix — never expanded. |

Example:

```bash
$ ./scripts/idx tag:legacy:P259 --before 1 --para 1 --max-bytes 600 \
    | jq '[.[] | select(.file == "src/parser/vectors.rs")] | .[0]'
{
  "file": "src/parser/vectors.rs",
  "line": 800,
  "context": "                    // P259: when the captured variable is a heap-owned cell",
  "excerpt": "                    ));\n                    // P259: when the captured variable is a heap-owned cell\n                    // (Reference(__cell_*, _)), the closure record now holds\n                    // ...[truncated]"
}
```

CLAUDE.md updated: § Tracker tags recommends
`./scripts/idx ...` over `jq`/`grep`.

## Goal

Ship `scripts/idx` — a tiny bash wrapper around `index/tags.json`
that gives Claude (and humans) a stable query interface.
Eliminates the per-query `jq` boilerplate and provides the
canonical CLI entry point that CLAUDE.md will recommend.

## What ships

### `scripts/idx`

```bash
#!/usr/bin/env bash
# scripts/idx — query the tracker-tag index.
# Reads index/tags.json (rebuild with `make index`).
#
# Forms:
#   idx tag:@P259                 — exact tag match
#   idx tag:legacy:P259           — bare-name fallback
#   idx prefix:@PLAN22            — all tags starting with prefix
#   idx file:doc/.../PROBLEMS.md  — all tags referenced in a file
#   idx all                       — all tag names + counts
#   idx broken                    — broken @-references (phase 03)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IDX="$ROOT/index/tags.json"
[ -f "$IDX" ] || { echo "$IDX missing — run: make index" >&2; exit 1; }

case "${1:-}" in
  tag:*)    jq --arg t "${1#tag:}" '.[$t] // []' "$IDX" ;;
  prefix:*) jq --arg p "${1#prefix:}" '
              [ to_entries[]
                | select(.key | startswith($p))
                | { (.key): .value } ] | add // {}' "$IDX" ;;
  file:*)   jq --arg f "${1#file:}" '
              [ to_entries[]
                | { tag: .key,
                    refs: [.value[] | select(.file == $f)] }
                | select(.refs | length > 0) ]' "$IDX" ;;
  all)      jq '[ to_entries[] | { tag: .key, count: (.value | length) } ]
                | sort_by(-.count)' "$IDX" ;;
  broken)   jq '.broken // []' "$IDX" ;;  # phase 03 populates this
  ""|-h|--help|help) sed -n '2,15p' "$0" ;;
  *)        echo "unknown query: $1 (try: idx help)" >&2; exit 2 ;;
esac
```

~30 lines including comments.  All output is JSON; pipe to
`jq -r` for plain text if needed.

### Usage examples

```bash
# Where is P259 referenced today (bare-name form)?
$ ./scripts/idx tag:legacy:P259 | jq '.[:3]'

# Every PLAN22 reference + sub-phase?
$ ./scripts/idx prefix:@PLAN22

# Tags in a single file?
$ ./scripts/idx file:doc/claude/PROBLEMS.md | jq '.[].tag' | head

# Most-referenced tags overall?
$ ./scripts/idx all | jq '.[:10]'
```

## Critical files

| Path | Action |
|---|---|
| `scripts/idx` | NEW (~30 lines) |
| `CLAUDE.md` | UPDATE § Tracker tags — replace the `jq` examples with `./scripts/idx ...` |

## Acceptance

- `./scripts/idx tag:legacy:P259` returns the same JSON array
  as `jq '.["legacy:P259"]' index/tags.json`.
- `./scripts/idx prefix:@PLAN22` returns an object with all
  PLAN22-* tags as keys.
- `./scripts/idx file:doc/claude/PROBLEMS.md` returns an
  array of `{tag, refs}` objects.
- `./scripts/idx all` returns tags sorted by reference count
  (descending).
- `./scripts/idx help` prints the usage block.
- Unknown query forms exit 2 with a friendly message.

## Risks

| Risk | Mitigation |
|---|---|
| `jq` filter syntax errors only surface at query time | Smoke test in `tests/index_hygiene.rs` (phase 03) runs each query form once on the real index |
| Quoting hazards in path arguments with spaces | Document: paths must not contain spaces (loft repo convention; no path with a space exists today) |

## Cross-references

- [Phase 00 — convention + scanner](00-convention-and-scanner.md) — produces the `tags.json` this phase queries
- [Phase 05 — Claude integration](05-claude-integration.md) — bumps `./scripts/idx` to the canonical reference-lookup in CLAUDE.md
