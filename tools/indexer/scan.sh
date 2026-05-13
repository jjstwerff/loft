#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# tools/indexer/scan.sh — produce index/tags.json from the loft repo.
# Phase 00 of plan-37 (tracker-index).
#
# Scans .md, .rs, .loft, .toml, .py, .sh files for tracker tag
# references in two families:
#   @P\d+(?:[a-z])?\b       — P-issue refs (@P259, @P229b)
#   @PLAN\d+(?:-[\w.]+)*\b  — plan + phase + sub-phase refs
#
# For transition tracking, ALSO emits the bare forms (P\d+ /
# plan-NN) under "legacy:" prefixed keys.
#
# Output: index/tags.json — see ARCHITECTURE.md for shape.
# Performance target: ≤ 2 seconds on the loft tree.

set -euo pipefail
cd "$(dirname "$0")/../.."   # repo root

if ! command -v jq >/dev/null 2>&1; then
  echo "tools/indexer/scan.sh: needs jq (apt install jq / dnf install jq)" >&2
  exit 1
fi

OUT="index/tags.json"
mkdir -p index

# Files to scan: docs + code + scripts.  git ls-files honours
# .gitignore and gives sorted output for determinism.
FILES_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP"' EXIT
git ls-files \
  '*.md' '*.rs' '*.loft' '*.toml' '*.py' '*.sh' \
  | grep -vE '^(target|node_modules)/' > "$FILES_TMP" || true

if [ ! -s "$FILES_TMP" ]; then
  echo "tools/indexer/scan.sh: no files to scan" >&2
  echo '{}' > "$OUT"
  exit 0
fi

# Single grep -H over all files — much faster than per-file shell loops.
# Combined regex with three alternation groups; awk picks them apart.
RAW_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP"' EXIT

xargs -a "$FILES_TMP" grep -nHE \
  '@P[0-9]+[a-z]?\b|@PLAN[0-9]+(-[a-zA-Z0-9._]+)*\b|\bP[0-9]+[a-z]?\b|\bplan-[0-9]+\b' \
  > "$RAW_TMP" 2>/dev/null || true

# Each line of $RAW_TMP is `<file>:<lineno>:<content>`.
# Awk extracts every tag occurrence and emits TSV: tag\tfile\tline\tcontext
awk -F: '
{
  # Reconstruct file (everything before the first :), line (next), content (rest).
  file = $1
  line = $2
  # content is the rest of the line — re-join from $3 onward
  content = $3
  for (i = 4; i <= NF; i++) content = content ":" $i

  # Find every @P-id token
  s = content
  while (match(s, /@P[0-9]+[a-z]?/)) {
    tag = substr(s, RSTART, RLENGTH)
    print tag "\t" file "\t" line "\t" content
    s = substr(s, RSTART + RLENGTH)
  }
  # Find every @PLAN-id token
  s = content
  while (match(s, /@PLAN[0-9]+(-[a-zA-Z0-9._]+)*/)) {
    tag = substr(s, RSTART, RLENGTH)
    print tag "\t" file "\t" line "\t" content
    s = substr(s, RSTART + RLENGTH)
  }
  # Find legacy bare P-id (NOT preceded by @)
  s = content
  while (match(s, /(^|[^@a-zA-Z0-9])P[0-9]+[a-z]?/)) {
    tok = substr(s, RSTART, RLENGTH)
    # strip optional leading non-alnum
    sub(/^[^@a-zA-Z0-9]/, "", tok)
    print "legacy:" tok "\t" file "\t" line "\t" content
    s = substr(s, RSTART + RLENGTH)
  }
  # Find legacy bare plan-NN
  s = content
  while (match(s, /\bplan-[0-9]+/)) {
    tok = substr(s, RSTART, RLENGTH)
    print "legacy:" tok "\t" file "\t" line "\t" content
    s = substr(s, RSTART + RLENGTH)
  }
}
' "$RAW_TMP" \
  | jq -Rsn '
      [ inputs
        | split("\n")[]
        | select(length > 0)
        | split("\t")
        | { tag: .[0], file: .[1], line: (.[2] | tonumber), context: .[3] }
      ]
      | group_by(.tag)
      | map({ (.[0].tag): (
                map({file, line, context})
                | sort_by(.file, .line)
                | unique
              ) })
      | add // {}
    ' > "$OUT"

count=$(jq 'keys | length' "$OUT")
new_count=$(jq '[keys[] | select(startswith("legacy:") | not)] | length' "$OUT")
legacy_count=$(jq '[keys[] | select(startswith("legacy:"))] | length' "$OUT")
total_refs=$(jq '[.[] | length] | add // 0' "$OUT")

echo "tools/indexer/scan.sh: wrote $OUT"
echo "  $count distinct tags ($new_count new-form, $legacy_count legacy-form)"
echo "  $total_refs total references"
