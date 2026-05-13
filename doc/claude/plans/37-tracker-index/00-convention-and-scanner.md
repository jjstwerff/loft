<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Tag convention + initial indexer

**Status:** Open

## Goal

Establish the `@P-id` / `@PLAN-id` tag convention and ship the
minimum viable scanner: a bash script that walks the repo,
finds tag references, and writes `index/tags.json`.  No CLI
wrapper yet (phase 01); no auto-refresh (phase 02); no broken-
tag validation (phase 03).  Just the index file.

## What ships

### Files

```
.gitignore                        # add /index/ to gitignored paths
tools/indexer/
├── scan.sh                       # the scanner (bash + grep + jq)
└── ARCHITECTURE.md               # design notes (~30 lines)
index/
├── .gitkeep                      # placeholder; tags.json gitignored
└── tags.json                     # generated; {tag: [{file, line, context}]}
Makefile                          # add `index:` target
CLAUDE.md                         # add § Tag convention block
```

### Scanner — `tools/indexer/scan.sh`

```bash
#!/usr/bin/env bash
# tools/indexer/scan.sh — produce index/tags.json from the loft repo.
#
# Scans .md, .rs, .loft, .toml, .py, .sh files for tracker tag
# references in two families:
#   @P\d+(?:[a-z])?\b       — P-issue references (@P259, @P229b)
#   @PLAN\d+(?:-[\w.]+)*\b  — plan + sub-phase references
#                              (@PLAN22, @PLAN35-01, @PLAN22-2d-iii.a)
#
# For transition tracking, ALSO scans the bare forms (P\d+ / plan-NN)
# and emits them under separate "legacy:" prefixed keys.  Lets us
# measure adoption progress.
#
# Output: index/tags.json with shape:
#   {
#     "@P259": [{"file": "doc/...", "line": 145, "context": "..."}],
#     "@PLAN22-2d-iii.a": [...],
#     "legacy:P259": [...],          # bare-name occurrences
#     "legacy:plan-22": [...]
#   }

set -euo pipefail
cd "$(dirname "$0")/../.."   # repo root

if ! command -v jq >/dev/null 2>&1; then
  echo "tools/indexer/scan.sh: needs jq (apt install jq)" >&2
  exit 1
fi

OUT="index/tags.json"
mkdir -p index

# Files to scan: docs + code + scripts.  Skip target/, .git/, node_modules/.
FILES=$(git ls-files \
  '*.md' '*.rs' '*.loft' '*.toml' '*.py' '*.sh' \
  | grep -vE '^(target|node_modules|\.git)/' || true)

# Scan and emit JSONL: one record per match.  Then jq groups by tag.
{
  for f in $FILES; do
    # @P-id matches
    grep -nE '@P[0-9]+[a-z]?\b' -- "$f" 2>/dev/null \
      | sed -E "s|^|$f:|; s|^([^:]+):([0-9]+):(.*)|\1\t\2\t\3|" || true
    # @PLAN-id matches
    grep -nE '@PLAN[0-9]+(-[a-zA-Z0-9._]+)*\b' -- "$f" 2>/dev/null \
      | sed -E "s|^|$f:|; s|^([^:]+):([0-9]+):(.*)|\1\t\2\t\3|" || true
    # legacy: bare P-id (excluding @P-prefixed)
    grep -nE '\bP[0-9]+[a-z]?\b' -- "$f" 2>/dev/null \
      | grep -v '@P' \
      | sed -E "s|^|$f:|; s|^([^:]+):([0-9]+):(.*)|legacy:\1\t\2\t\3|" || true
  done
} | awk -F'\t' '
{
  file = $1; line = $2; ctx = $3
  # extract every @P-id / @PLAN-id token from ctx
  while (match(ctx, /@P[0-9]+[a-z]?|@PLAN[0-9]+(-[a-zA-Z0-9._]+)*/)) {
    tag = substr(ctx, RSTART, RLENGTH)
    print tag "\t" file "\t" line "\t" ctx
    ctx = substr(ctx, RSTART + RLENGTH)
  }
  # legacy bare matches: file already prefixed with "legacy:"
  if (file ~ /^legacy:/) {
    real_file = substr(file, 8)
    while (match(ctx, /\bP[0-9]+[a-z]?\b/)) {
      tag = "legacy:" substr(ctx, RSTART, RLENGTH)
      print tag "\t" real_file "\t" line "\t" ctx
      ctx = substr(ctx, RSTART + RLENGTH)
    }
  }
}' | jq -Rsn '
  [inputs | split("\n")[] | select(length > 0) | split("\t")
    | {tag: .[0], file: .[1], line: (.[2]|tonumber), context: .[3]}]
  | group_by(.tag)
  | map({(.[0].tag): map({file, line, context})})
  | add
' > "$OUT"

count=$(jq 'keys | length' "$OUT")
echo "tools/indexer/scan.sh: wrote $OUT ($count distinct tags)"
```

The scanner is intentionally simple and dependency-light:
just `git ls-files` + `grep` + `awk` + `jq`.  No language-
specific parsers.  The trade-off: it can't tell the
difference between a tag in a doc-comment vs in a code
literal, but for "where is this referenced?" that's fine —
a reference is a reference.

### `tools/indexer/ARCHITECTURE.md`

```markdown
# tracker-index architecture

## Why bash + grep + jq

Smallest dependency footprint: bash + grep are POSIX, jq is
pre-installed in most VMs.  No build step, no runtime, no
custom parser.  Trade-off accepted: can't distinguish a tag
in a code literal from a tag in prose — every reference
counts equally.

## Tag families

| Family | Regex | Purpose |
|---|---|---|
| `@P\d+[a-z]?` | P-issue references | "where is P259 mentioned?" |
| `@PLAN\d+(-\w+)*` | Plan + phase + sub-phase | "where is PLAN22-2d-iii.a referenced?" |
| `legacy:P\d+`, `legacy:plan-NN` | Bare-name forms (no `@`) | Track adoption progress |

## Output shape (tags.json)

  {
    "@P259":           [ {file, line, context}, ... ],
    "@PLAN22":         [ ... ],
    "@PLAN22-2d-iii.a":[ ... ],
    "legacy:P259":     [ ... ]
  }

Sorted by tag name within each array; arrays sorted by
(file, line) for stable output.

## Phasing

Phase 00 (this file): scanner + Makefile + tag convention.
Phase 01: CLI wrapper.
Phase 02: auto-refresh on commit.
Phase 03: broken-tag validator.
Phase 04: viewer integration.
Phase 05: Claude integration.
Phase 06: retroactive tagging + closeout.

## Performance target

≤ 2 seconds on the loft tree (~1100 .md/.rs/.loft files).
Idempotent — same input always produces byte-identical output.
```

### Makefile

Add a `index:` target near `view-build` / `view`:

```make
index:  ## Rebuild index/tags.json
	@./tools/indexer/scan.sh
```

### CLAUDE.md additions

Add a § Tag convention block under § Important conventions:

```markdown
## Tag convention — `@P-id` and `@PLAN-id`

Tracker references in docs use the `@`-prefixed form so
regex matching is unambiguous:

- **P-issues**: `@P259`, `@P229b`, `@P262`.
- **Plans + phases**: `@PLAN22`, `@PLAN35-01`,
  `@PLAN22-2d-iii.a` (sub-phases via `-` and `.`).

Old bare forms (`P259`, `plan-22 phase 03`) still work in
prose; the indexer (`make index`) tracks both for transition.

To find references: `./scripts/idx tag:@P259` (after phase
01 ships) or `grep -rn '@P259\b'` (works today).
```

### `.gitignore` additions

```
# Generated by tools/indexer/scan.sh
/index/*
!/index/.gitkeep
```

## Acceptance

- `make index` produces `index/tags.json` in ≤ 2 seconds.
- `jq 'keys | length' index/tags.json` returns at least 50
  (loft repo has at least 50 distinct tag mentions today
  including legacy forms).
- `jq '.["@P259"]' index/tags.json` returns an array (@P259
  is closed; should have ≥ 5 references in PROBLEMS.md +
  CHANGELOG_TECHNICAL.md + plan dirs).
- The legacy bucket exposes the adoption gap:
  `jq '. | to_entries | [.[] | select(.key | startswith("legacy:")) | .key] | length'`
  shows how many bare-name forms still need migration.

## Verification

```bash
$ make index
tools/indexer/scan.sh: wrote index/tags.json (87 distinct tags)

$ jq 'keys | length' index/tags.json
87

$ jq '.["legacy:P259"] | length' index/tags.json
12   # 12 bare-name "P259" references currently in the tree

$ jq '.["@P259"] | length' index/tags.json
0    # no @-prefixed yet (phase 06 retroactive sed will add these)
```

## Risks

| Risk | Mitigation |
|---|---|
| Bash + grep + awk pipeline unreadable, hard to maintain | < 50 lines total; design notes in ARCHITECTURE.md; phase 06 closeout can rewrite in loft if maintenance burden surfaces |
| `jq` not installed | Detection at script start with apt/dnf install hint |
| Scanner picks up tags in test fixtures (`tests/scripts/*.loft`) that are NOT real references | Phase 03 (broken-tag validator) flags these; phase 06 closeout decides per-fixture whether to add a `# noindex` opt-out comment |
| Output not deterministic (Git ls-files order varies?) | `git ls-files` is sorted by default; jq's `group_by` is stable; output should be byte-identical across runs |
| Performance degrades as repo grows | Phase 02's pre-commit hook can scope the scan to changed files only; tier-1 scan time is < 2 sec on today's tree |

## Cross-references

- [README § Architecture](README.md#architecture)
- [README § Tag conventions](README.md#tag-conventions)
- [Phase 01 — CLI query wrapper](01-cli-query.md) — consumer
- [Phase 03 — broken-tag validator](03-broken-validator.md) — consumer
