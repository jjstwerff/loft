<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# tracker-index architecture

Phase 00 of [`plans/37-tracker-index/`](../../doc/claude/plans/37-tracker-index/README.md).

## Why bash + grep + jq

Smallest possible dependency footprint: bash + grep are POSIX,
jq is pre-installed in most VMs.  No build step, no runtime,
no custom parser.  Trade-off accepted: can't distinguish a
tag in a code literal from a tag in prose — every reference
counts equally.

## Tag families

| Family | Regex | Purpose |
|---|---|---|
| `@P\d+[a-z]?` | P-issue references | "where is P259 mentioned?" |
| `@PLAN\d+(-[\w.]+)*` | Plan + phase + sub-phase | "where is PLAN22-2d-iii.a referenced?" |
| `legacy:P\d+`, `legacy:plan-NN` | Bare-name (no `@`) forms | Track adoption progress |

The `legacy:` prefix lets us measure the gap between
"adopted-the-convention" references and "still need
migrating" references.

## Output shape — `index/tags.json`

```json
{
  "@P259":            [ {file, line, context}, ... ],
  "@PLAN22":          [ ... ],
  "@PLAN22-2d-iii.a": [ ... ],
  "legacy:P259":      [ ... ],
  "legacy:plan-22":   [ ... ]
}
```

Within each array, entries are `(file, line)` sorted and
deduplicated.  `tags.json` is byte-identical across runs on
the same source tree.

## Phasing

| Phase | What ships |
|---|---|
| 00 (this file) | Scanner + Makefile + tag convention in CLAUDE.md |
| 01 | `scripts/idx` CLI query wrapper |
| 02 | git pre-commit hook auto-refresh |
| 03 | Broken-tag validator + CI hygiene test |
| 04 | Plan-35 viewer integration |
| 05 | Claude integration (CLAUDE.md instructions, optional MCP) |
| 06 | Retroactive tagging sweep + closeout |

## Performance target

≤ 2 seconds on the loft tree (~1100 .md/.rs/.loft files).
Idempotent — same input always produces byte-identical
output.
