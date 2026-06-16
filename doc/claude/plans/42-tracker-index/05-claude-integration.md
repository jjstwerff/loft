<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 05 — Claude integration

**Status:** Shipped 2026-05-13 (MCP wrapper deferred — optional)

## What shipped

- CLAUDE.md § Key commands lists `./scripts/idx tag:@P259`
  with a brief usage hint (the `--before`/`--after`/`--para`
  flags are mentioned for context lookups).
- CLAUDE.md `make view` row mentions the new `/tag/<bare>`
  route from @PLN42 phase 04a so the browser-side equivalent
  is discoverable.
- The full canonical-lookup paragraph (§ Tracker tags →
  "Looking up tracker references") was already in place from
  phase 01; phase 05 did not re-touch it.
- A persistent memory entry (`feedback_use_idx_not_grep.md`)
  was saved to the user's auto-memory so future Claude
  sessions default to `./scripts/idx` for tracker lookups
  without needing the rule re-explained per session.

## What's deferred

- **MCP wrapper** — listed as optional in the original plan;
  not built.  The `./scripts/idx` CLI surface is enough for
  the token-reduction goal; an MCP wrapper would tighten it
  further but adds tool-config burden.  Promote when token
  budget pressure surfaces a concrete need.

## Original plan

## Goal (original)

Make `./scripts/idx` Claude's canonical reference-lookup
mechanism, displacing per-task `grep -rn` patterns.  The win
is measurable per-session token reduction: instead of
pulling 5-10 files into context to find P-id mentions, query
the index once and get a structured answer.

## What ships

### CLAUDE.md updates

The § Tracker tags block (added in phase 00) gets a Claude-
specific subsection:

```markdown
### Looking up tracker references — use the indexer

Default workflow for "where is X referenced?":

```bash
./scripts/idx tag:@P259               # exact tag
./scripts/idx prefix:@PLAN22          # all PLAN22-* refs
./scripts/idx file:doc/.../FOO.md     # tags in a single file
./scripts/idx all | jq '.[:10]'       # top 10 by count
```

Prefer this over `grep -rn '@P259' ...` — it's faster,
returns structured JSON Claude can iterate, and avoids
pulling unnecessary file content into context.  Run `make
index` first if `index/tags.json` is missing or stale (the
pre-commit hook keeps it fresh on most workflows).

For bare-name (legacy) lookups during the transition:
`./scripts/idx tag:legacy:P259`.
```

### Optional: MCP server

If token efficiency matters enough, ship a tiny MCP wrapper
(`tools/indexer/mcp.py` or similar) that exposes:

- `idx_tag(tag: str) -> list[ref]`
- `idx_prefix(prefix: str) -> list[tag]`
- `idx_file(file: str) -> list[ref]`

Claude calls those tools instead of shelling out to `idx`.
Same data, slightly tighter context budget per query.

The MCP wrapper is **optional** — phase 05 ships even if it
doesn't.  The CLI is the foundation; MCP is a polish.

### Memory hint for Claude

A short note in this file (and CLAUDE.md) that Claude's
auto-memory should record: "When the loft project asks
'where is X referenced', use `./scripts/idx tag:@X` or
`./scripts/idx tag:legacy:X` rather than `grep -rn`."

The auto-memory system isn't deterministic; the hint just
nudges the right pattern.

## Acceptance

- CLAUDE.md § Tracker tags has a "Looking up tracker
  references" subsection naming `./scripts/idx` as the
  default.
- A few sample Claude-driven lookups in the next session
  show fewer files pulled into context for the same
  question.  Measurable in token usage but not tested
  programmatically.
- If MCP wrapper ships, Claude uses it without prompt
  engineering (the tool is in the tool-list).

## Risks

| Risk | Mitigation |
|---|---|
| Claude continues to `grep` out of habit | Repeat the canonical-lookup hint in three places (CLAUDE.md § Key commands, § Tracker tags, this phase doc) so the pattern catches via repeated exposure |
| MCP wrapper adds tool-config burden | Optional — skip it if the CLI is enough |
| The indexer becomes Claude's only lookup, then a stale `tags.json` makes it lie | Phase 02's pre-commit hook + phase 03's broken-tag CI gate catch most stale-index cases; the viewer (phase 04) shows a "stale index" banner |

## Cross-references

- [Phase 01 — CLI query wrapper](01-cli-query.md) — the lookup tool this phase recommends
- [CLAUDE.md § Tracker tags](../../../../CLAUDE.md) — destination for the lookup-pattern subsection
