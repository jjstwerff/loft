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
#
# Opt-out: lines containing the literal `<!--noindex-->`
# marker are skipped entirely.  Use in design docs that need
# to MENTION fake @P-id / @PLAN-id examples without indexing
# them as real references.

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
  2>/dev/null \
  | grep -v '<!--noindex-->' \
  > "$RAW_TMP" || true

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

# ── Broken-tag validation (phase 03) ────────────────────────────
# Cross-reference every @P<N> against PROBLEMS.md row IDs;
# every @PLAN<N> against the four plan directory roots.
# Sub-phase IDs (@PLAN35-04) validate only the parent plan
# exists for v1; per-phase-file existence is a future step.

# 1. Sets of valid IDs.
VALID_PIDS=$(grep -oE '^\| [0-9]+ \|' doc/claude/PROBLEMS.md \
  | grep -oE '[0-9]+' | sort -u)
VALID_PLANS=$(find doc/claude/plans \
  -maxdepth 2 -mindepth 1 -type d \
  -regex 'doc/claude/plans/\(finished/\|future/\|deferred/\)?[0-9]+-.*' \
  2>/dev/null \
  | grep -oE '/[0-9]+-' | grep -oE '[0-9]+' | sort -u)

# 2. Sets of referenced IDs.  Strip optional trailing letter
# from @P-id (e.g., @P229b → 229).
REF_PIDS=$(jq -r '
  keys[]
  | select(startswith("@P") and (startswith("@PLAN") | not))
  | sub("^@P"; "")
  | sub("[a-z]$"; "")
' "$OUT" | sort -u)
REF_PLANS=$(jq -r '
  keys[]
  | select(startswith("@PLAN"))
  | capture("^@PLAN(?<n>[0-9]+)").n
' "$OUT" | sort -u)

# 3. Diff: referenced minus valid → broken.  Awk-based set
# difference avoids the bash-3 / process-substitution gap.
BROKEN_PIDS=$(awk 'NR==FNR{valid[$1]=1; next} !($1 in valid)' \
  <(echo "$VALID_PIDS") <(echo "$REF_PIDS") | grep -v '^$' || true)
BROKEN_PLANS=$(awk 'NR==FNR{valid[$1]=1; next} !($1 in valid)' \
  <(echo "$VALID_PLANS") <(echo "$REF_PLANS") | grep -v '^$' || true)

# 4. Build {tag, refs:[file:line, ...]} entries for each broken ID.
BROKEN_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP"' EXIT
for n in $BROKEN_PIDS; do
  jq -r --arg n "$n" '
    to_entries[]
    | select(.key | test("^@P" + $n + "[a-z]?$"))
    | .key as $tag
    | .value[] | "\($tag)\t\(.file):\(.line)"
  ' "$OUT" >> "$BROKEN_TMP"
done
for n in $BROKEN_PLANS; do
  jq -r --arg n "$n" '
    to_entries[]
    | select(.key | test("^@PLAN" + $n + "(-.*)?$"))
    | .key as $tag
    | .value[] | "\($tag)\t\(.file):\(.line)"
  ' "$OUT" >> "$BROKEN_TMP"
done

# 5. Group + merge into the existing tags.json under "broken" key.
if [ -s "$BROKEN_TMP" ]; then
  BROKEN_JSON=$(awk -F'\t' '
    { tag=$1; ref=$2; refs[tag] = refs[tag] (refs[tag] ? "," : "") "\"" ref "\"" }
    END {
      printf "["
      first=1
      for (t in refs) {
        if (!first) printf ","
        printf "{\"tag\":\"%s\",\"refs\":[%s]}", t, refs[t]
        first=0
      }
      printf "]"
    }
  ' "$BROKEN_TMP")
else
  BROKEN_JSON='[]'
fi

# 6. Merge into output.
jq --argjson broken "$BROKEN_JSON" '. + {broken: $broken}' "$OUT" > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"

# ── Markdown-link extraction (phase 09 — backlinks) ─────────────
# For each .md file, grep every `[text](path.md)` or
# `[text](path.md#anchor)` link, resolve the target against
# the source file's directory, and emit `(target, source_file,
# line, anchor, context)` tuples.  Joined into the `links`
# bucket of tags.json (target → list of inbound refs).
#
# Path resolution rules:
#   - http(s):// and mailto: schemes skipped (external).
#   - `./` prefix stripped.
#   - `../` segments walk up the source's directory.
#   - Bare anchors (`#section`) skipped (intra-file).

LINKS_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$LINKS_TMP"' EXIT

# Step 1: collect all `<src>:<line>:[text](target)` matches across .md files.
# Reuse $FILES_TMP filtered to .md — avoids a second `git ls-files`
# call which is expensive on SSHFS / network mounts.
grep -E '\.md$' "$FILES_TMP" \
  | xargs -r grep -nHE '\][(]([^)#[:space:]]+)\.md(#[^)]*)?[)]' 2>/dev/null \
  | grep -v '<!--noindex-->' \
  > "$LINKS_TMP" || true

# Step 2: extract + resolve.  Awk walks each line, finds every
# link occurrence, computes the target path relative to repo root.
LINKS_TSV=$(awk -F: '
function resolve(base, target,    parts, n, out, i, segs, m) {
    # Strip leading ./ repeatedly.
    while (substr(target, 1, 2) == "./") {
        target = substr(target, 3);
    }
    # Apply ../ segments — walk up the base.
    while (substr(target, 1, 3) == "../") {
        # Drop the last segment of base.
        m = match(base, /\/[^\/]*$/);
        if (m == 0) {
            base = "";
        } else {
            base = substr(base, 1, m - 1);
        }
        target = substr(target, 4);
    }
    if (base == "") return target;
    return base "/" target;
}
function dirof(p,    m) {
    m = match(p, /\/[^\/]*$/);
    if (m == 0) return "";
    return substr(p, 1, m - 1);
}
{
    file = $1
    line = $2
    content = $3
    for (i = 4; i <= NF; i++) content = content ":" $i
    base = dirof(file)
    s = content
    # Repeatedly find `](X.md...)` patterns.  We avoid a full
    # `[text](url)` regex because awk lacks lookahead.
    while (match(s, /\][(]([^)]+\.md)(#[^)]*)?[)]/)) {
        # Capture RSTART/RLENGTH BEFORE calling resolve() — its
        # internal match() calls would otherwise clobber them,
        # so the substr advance after this loop would walk to
        # the wrong position (causing duplicate emits).
        rs = RSTART
        rl = RLENGTH
        token = substr(s, rs, rl)
        # Strip leading `](` and trailing `)`.
        inner = substr(token, 3, length(token) - 3)
        # Split on `#` for optional anchor.
        hash = index(inner, "#")
        anchor = ""
        target_raw = inner
        if (hash > 0) {
            target_raw = substr(inner, 1, hash - 1)
            anchor = substr(inner, hash + 1)
        }
        # Skip schemes
        if (target_raw ~ /^(https?|mailto):/) {
            s = substr(s, rs + rl)
            continue
        }
        if (substr(target_raw, 1, 1) == "/") {
            # Already repo-rooted — strip leading slash.
            resolved = substr(target_raw, 2)
        } else {
            resolved = resolve(base, target_raw)
        }
        print resolved "\t" file "\t" line "\t" anchor "\t" content
        s = substr(s, rs + rl)
    }
}
' "$LINKS_TMP")

# Step 3: assemble the `links` bucket.  jq groups by target.
# We write the bucket to a temp file rather than --argjson —
# the loft tree has ~3000 links and the JSON exceeds ARG_MAX
# (~128 KB on Linux).  --slurpfile reads via fd, no argv limit.
LINKS_BUCKET_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$LINKS_TMP" "$LINKS_BUCKET_TMP"' EXIT
if [ -n "$LINKS_TSV" ]; then
    printf "%s" "$LINKS_TSV" | jq -Rn '
        [ inputs
          | split("\n")[]
          | select(length > 0)
          | split("\t")
          | { target: .[0],
              file:   .[1],
              line:   (.[2] | tonumber),
              anchor: .[3],
              context: .[4] }
        ]
        | group_by(.target)
        | map({ (.[0].target): (
                  map({file, line, anchor, context})
                  | sort_by(.file, .line)
                  | unique
                ) })
        | add // {}
    ' > "$LINKS_BUCKET_TMP"
else
    echo '{}' > "$LINKS_BUCKET_TMP"
fi

jq --slurpfile links "$LINKS_BUCKET_TMP" '. + {links: $links[0]}' "$OUT" > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"

# ── Broken-link detection (phase 09) ────────────────────────────
# Walk every key in `.links`; if the resolved target file does
# not exist, record it under `broken_links: [{target, refs: [...]}]`.
# Mirrors the shape of `broken` (broken @-tag refs).
BROKEN_LINKS_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$LINKS_TMP" "$LINKS_BUCKET_TMP" "$BROKEN_LINKS_TMP"' EXIT
jq -r '.links | keys[]' "$OUT" | while read -r target; do
  if [ ! -f "$target" ]; then
    echo "$target"
  fi
done > "$BROKEN_LINKS_TMP" || true

if [ -s "$BROKEN_LINKS_TMP" ]; then
  BROKEN_LINKS_JSON=$(jq -Rs --slurpfile idx "$OUT" '
    split("\n")
    | map(select(length > 0))
    | map({
        target: .,
        refs: ($idx[0].links[.] // [] | map("\(.file):\(.line)"))
      })
  ' "$BROKEN_LINKS_TMP")
else
  BROKEN_LINKS_JSON='[]'
fi

# Merge via stdin (argv could overflow on large outputs).
BROKEN_LINKS_BUCKET_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$LINKS_TMP" "$LINKS_BUCKET_TMP" "$BROKEN_LINKS_TMP" "$BROKEN_LINKS_BUCKET_TMP"' EXIT
printf '%s' "$BROKEN_LINKS_JSON" > "$BROKEN_LINKS_BUCKET_TMP"
jq --slurpfile bl "$BROKEN_LINKS_BUCKET_TMP" '. + {broken_links: $bl[0]}' "$OUT" > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"

count=$(jq 'keys | length' "$OUT")
new_count=$(jq '[keys[] | select(startswith("legacy:") | not) | select(. != "broken" and . != "links" and . != "broken_links")] | length' "$OUT")
legacy_count=$(jq '[keys[] | select(startswith("legacy:"))] | length' "$OUT")
total_refs=$(jq '[to_entries[] | select(.key != "broken" and .key != "links" and .key != "broken_links") | .value | length] | add // 0' "$OUT")
broken_count=$(jq '.broken | length' "$OUT")
links_count=$(jq '.links | length' "$OUT")
links_total=$(jq '[.links | to_entries[] | .value | length] | add // 0' "$OUT")
broken_links_count=$(jq '.broken_links | length' "$OUT")

echo "tools/indexer/scan.sh: wrote $OUT"
echo "  $count distinct tags ($new_count new-form, $legacy_count legacy-form)"
echo "  $total_refs total references"
echo "  $links_count link targets ($links_total inbound links)"
if [ "$broken_count" -gt 0 ]; then
  echo "  $broken_count broken @-references — run: ./scripts/idx broken"
fi
if [ "$broken_links_count" -gt 0 ]; then
  echo "  $broken_links_count broken markdown links — run: ./scripts/idx broken-links"
fi
