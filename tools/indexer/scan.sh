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

# Force byte-oriented locale so BSD awk (macOS default) doesn't try
# multibyte conversion on UTF-8 input.  doc/claude/PROBLEMS.md and
# friends use em-dashes (`—`), curly quotes, and other multibyte
# codepoints inside body text; without LC_ALL=C, `tolower()` /
# `gsub()` raise `towc: multibyte conversion failure` and exit
# non-zero, breaking `make index` under `set -euo pipefail`.  Byte-
# oriented operation is fine: every awk pattern here is ASCII (table
# pipes, P-issue prefixes, dates), and `length` / `substr` / `index`
# operate on bytes uniformly across gawk and BSD awk so output stays
# stable across platforms.
export LC_ALL=C

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

# `xargs -a FILE` is GNU-only — BSD xargs (macOS default) reads
# from stdin only, so use shell redirection for cross-platform
# compatibility.
xargs grep -nHE \
  '@P[0-9]+[a-z]?\b|@PLAN[0-9]+(-[a-zA-Z0-9._]+)*\b|\bP[0-9]+[a-z]?\b|\bplan-[0-9]+\b' \
  < "$FILES_TMP" \
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
  # Find legacy bare plan-NN.  Note: original used \\b which gawk
  # silently never matched (POSIX awk lacks \\b), so for a long
  # stretch the index had zero legacy:plan- buckets.  Discovered
  # 2026-05-15 by the loft scanner port.  Use the same leading-
  # boundary form the bare P-id pass uses above, then strip the
  # captured leading char with sub.
  s = content
  while (match(s, /(^|[^a-zA-Z0-9_])plan-[0-9]+/)) {
    tok = substr(s, RSTART, RLENGTH)
    sub(/^[^a-zA-Z0-9_]/, "", tok)
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
# Use shell globs rather than `find -regex` — the GNU-style
# alternation `\(finished/\|future/\|deferred/\)?` doesn't parse
# under BSD find (macOS default), and `find` failing under
# `set -euo pipefail` exits the script silently.
VALID_PLANS=$(
  {
    ls -d doc/claude/plans/[0-9]*-*/ 2>/dev/null
    ls -d doc/claude/plans/finished/[0-9]*-*/ 2>/dev/null
    ls -d doc/claude/plans/future/[0-9]*-*/ 2>/dev/null
    ls -d doc/claude/plans/deferred/[0-9]*-*/ 2>/dev/null
  } | grep -oE '/[0-9]+-' | grep -oE '[0-9]+' | sort -u
)

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
# Pass BROKEN_JSON via stdin/file rather than `--argjson` argv —
# Windows MSYS bash has a much smaller argv limit than Linux/macOS,
# so any non-trivial broken set raises `jq: Argument list too long`.
# Same shape as the BROKEN_LINKS merge below (`--slurpfile`).
BROKEN_BUCKET_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$BROKEN_BUCKET_TMP"' EXIT
printf '%s' "$BROKEN_JSON" > "$BROKEN_BUCKET_TMP"
jq --slurpfile broken "$BROKEN_BUCKET_TMP" '. + {broken: $broken[0]}' "$OUT" > "$OUT.tmp"
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
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$BROKEN_BUCKET_TMP" "$LINKS_TMP"' EXIT

# Step 1: collect all `<src>:<line>:[text](target)` matches across .md files.
# Reuse $FILES_TMP filtered to .md — avoids a second `git ls-files`
# call which is expensive on SSHFS / network mounts.
#
# Per-file awk processing tracks fenced code-block state so example
# links inside ```...``` blocks (illustrating syntax, not real refs)
# don't pollute the links bucket.  The cost over a single grep
# pipe is ~100 ms on the loft tree; well under the 0.5 sec budget.
grep -E '\.md$' "$FILES_TMP" | while read -r f; do
  awk -v file="$f" '
    BEGIN { in_fence = 0; fence_marker = "" }
    {
      stripped = $0
      sub(/^[[:space:]]+/, "", stripped)
      # Track fence state.
      if (in_fence == 0) {
        if (substr(stripped, 1, 3) == "```") {
          in_fence = 1; fence_marker = "```"; next
        }
        if (substr(stripped, 1, 3) == "~~~") {
          in_fence = 1; fence_marker = "~~~"; next
        }
      } else {
        if (substr(stripped, 1, 3) == fence_marker) {
          in_fence = 0; fence_marker = ""
        }
        next
      }
      # Skip noindex lines.
      if (index($0, "<!--noindex-->") > 0) next
      # Emit only lines containing a `](X.md...)` pattern.
      if (match($0, /\][(][^)#[:space:]]+\.md(#[^)]*)?[)]/)) {
        printf "%s:%d:%s\n", file, NR, $0
      }
    }
  ' "$f"
done > "$LINKS_TMP" || true

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
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$BROKEN_BUCKET_TMP" "$LINKS_TMP" "$LINKS_BUCKET_TMP"' EXIT
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
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$BROKEN_BUCKET_TMP" "$LINKS_TMP" "$LINKS_BUCKET_TMP" "$BROKEN_LINKS_TMP"' EXIT
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
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$BROKEN_BUCKET_TMP" "$LINKS_TMP" "$LINKS_BUCKET_TMP" "$BROKEN_LINKS_TMP" "$BROKEN_LINKS_BUCKET_TMP"' EXIT
printf '%s' "$BROKEN_LINKS_JSON" > "$BROKEN_LINKS_BUCKET_TMP"
jq --slurpfile bl "$BROKEN_LINKS_BUCKET_TMP" '. + {broken_links: $bl[0]}' "$OUT" > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"

# ── Open P-issues bucket (viewer dashboard surface) ─────────────
# Pull every PROBLEMS.md row whose severity column contains the
# word "open" or marks the issue `(partial)` — same regex as
# `make problems`.  Each entry: {tag, line, severity, summary}
# where summary is the first sentence of the body (post-2026-05-14
# the body starts with `**@P<n>** — `, so we strip that prefix
# and take up to the first period).  Renders on the viewer's
# landing page between Recent activity and All changes.
PROBS_FILE="doc/claude/PROBLEMS.md"
PROBLEMS_OPEN_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$BROKEN_BUCKET_TMP" "$LINKS_TMP" "$LINKS_BUCKET_TMP" "$BROKEN_LINKS_TMP" "$BROKEN_LINKS_BUCKET_TMP" "$PROBLEMS_OPEN_TMP"' EXIT
if [ -f "$PROBS_FILE" ]; then
  awk -F'|' '
    /^\| [0-9]+ \|/ {
      n = $2; gsub(/ /, "", n);
      body = $3;
      sev = $4; gsub(/^ +| +$/, "", sev);
      fix = $5; gsub(/^ +| +$/, "", fix);
      if (tolower(sev) !~ /(^| |[(])open( |,|[)]|$)|[(]partial/) next;
      # Strip the leading `**@P<n>** — ` tag-prefix added by the
      # 2026-05-14 self-tagging migration.  Then trim and take
      # up to the first period.
      sub("^ *\\*\\*@P" n "\\*\\* — ", "", body);
      sub(/^ +/, "", body);
      # Cut at the first ". " (period + space) — a bare "."
      # would split file extensions (`tests/multiplayer_v2.rs`)
      # mid-token and produce useless summaries.
      pos = index(body, ". ");
      if (pos > 0) body = substr(body, 1, pos);
      if (length(body) > 280) body = substr(body, 1, 277) "...";
      if (length(fix) > 280)  fix  = substr(fix,  1, 277) "...";
      # Escape for JSON.
      gsub(/\\/, "\\\\", body); gsub(/"/, "\\\"", body);
      gsub(/\\/, "\\\\", sev);  gsub(/"/, "\\\"", sev);
      gsub(/\\/, "\\\\", fix);  gsub(/"/, "\\\"", fix);
      printf "{\"tag\":\"@P%s\",\"line\":%d,\"severity\":\"%s\",\"summary\":\"%s\",\"fix\":\"%s\"}\n", n, NR, sev, body, fix;
    }
  ' "$PROBS_FILE" \
    | jq -s '.' > "$PROBLEMS_OPEN_TMP"
else
  echo '[]' > "$PROBLEMS_OPEN_TMP"
fi
jq --slurpfile po "$PROBLEMS_OPEN_TMP" '. + {problems_open: $po[0]}' "$OUT" > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"

# ── Recently-closed P-issues bucket (last 30 days) ───────────────
# Same row format, inverse severity filter ("closed" without
# "(open" / "(partial").  Each entry: {tag, line, summary, closed}
# where `closed` is parsed from the body's `Closed (YYYY-MM-DD)`
# marker; rows without a parseable date are skipped (the curation
# is "what shipped recently," not "every closed bug ever").  The
# 30-day window is computed in awk via `mktime` against today.
PROBLEMS_RECENT_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$BROKEN_BUCKET_TMP" "$LINKS_TMP" "$LINKS_BUCKET_TMP" "$BROKEN_LINKS_TMP" "$BROKEN_LINKS_BUCKET_TMP" "$PROBLEMS_OPEN_TMP" "$PROBLEMS_RECENT_TMP"' EXIT
if [ -f "$PROBS_FILE" ]; then
  # Compute the 30-day-ago cutoff in shell rather than awk — gawk's
  # `mktime` isn't available in BSD awk (macOS default), so the prior
  # version exited non-zero under `set -euo pipefail` and broke
  # `make index` in CI on macOS.  YYYY-MM-DD strings sort
  # lexicographically, so the awk side reduces to a plain `<` compare.
  # Try GNU `date -d` first, then BSD `date -v`; both produce the
  # same `YYYY-MM-DD` shape so downstream is unchanged.
  CUTOFF_DATE=$(date -u -d '30 days ago' +%Y-%m-%d 2>/dev/null \
                || date -u -v-30d +%Y-%m-%d 2>/dev/null \
                || echo "0000-00-00")
  awk -F'|' -v cutoff="$CUTOFF_DATE" '
    /^\| [0-9]+ \|/ {
      n = $2; gsub(/ /, "", n);
      body = $3;
      sev = $4; gsub(/^ +| +$/, "", sev);
      # Closed rows: severity contains "closed" but NOT "(open" or "(partial".
      if (tolower(sev) !~ /[(]closed[)]|closed$/) next;
      if (tolower(sev) ~ /[(]open|[(]partial/) next;
      # Find the most recent Closed (YYYY-MM-DD) in the body.
      tmp = body; close_date = "";
      while (match(tmp, /Closed \(([0-9]{4})-([0-9]{2})-([0-9]{2})\)/)) {
        d = substr(tmp, RSTART, RLENGTH);
        # Capture groups via a second match on the matched substring.
        if (match(d, /[0-9]{4}-[0-9]{2}-[0-9]{2}/)) {
          close_date = substr(d, RSTART, RLENGTH);
        }
        tmp = substr(tmp, RSTART + RLENGTH);
      }
      if (close_date == "") next;
      # YYYY-MM-DD sorts lexicographically.  Skip rows older than cutoff.
      if (close_date < cutoff) next;
      sub("^ *\\*\\*@P" n "\\*\\* — ", "", body);
      sub(/^ +/, "", body);
      pos = index(body, ". ");
      if (pos > 0) body = substr(body, 1, pos);
      if (length(body) > 280) body = substr(body, 1, 277) "...";
      gsub(/\\/, "\\\\", body); gsub(/"/, "\\\"", body);
      printf "{\"tag\":\"@P%s\",\"line\":%d,\"closed\":\"%s\",\"summary\":\"%s\"}\n", n, NR, close_date, body;
    }
  ' "$PROBS_FILE" \
    | jq -s '. | sort_by(.closed) | reverse' > "$PROBLEMS_RECENT_TMP"
else
  echo '[]' > "$PROBLEMS_RECENT_TMP"
fi
jq --slurpfile pr "$PROBLEMS_RECENT_TMP" '. + {problems_recent: $pr[0]}' "$OUT" > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"

# ── Plan buckets (active / future / deferred / recent-finished) ──
# Walk plan directories and pick the first `# ` heading from each
# README.md as the plan's display title.  README-less directories
# are skipped (defensive — every plan has a README in practice).
# `recent_finished` filters by directory mtime within last 60 days.
PLANS_ACTIVE_TMP=$(mktemp)
PLANS_FUTURE_TMP=$(mktemp)
PLANS_DEFERRED_TMP=$(mktemp)
PLANS_RECENT_TMP=$(mktemp)
LIB_PLANS_FUTURE_TMP=$(mktemp)
trap 'rm -f "$FILES_TMP" "$RAW_TMP" "$BROKEN_TMP" "$LINKS_TMP" "$LINKS_BUCKET_TMP" "$BROKEN_LINKS_TMP" "$BROKEN_LINKS_BUCKET_TMP" "$PROBLEMS_OPEN_TMP" "$PROBLEMS_RECENT_TMP" "$PLANS_ACTIVE_TMP" "$PLANS_FUTURE_TMP" "$PLANS_DEFERRED_TMP" "$PLANS_RECENT_TMP" "$LIB_PLANS_FUTURE_TMP"' EXIT

# Helper — walk a directory of plan/[NN-slug] subdirs, emit JSON
# entries `{slug, path, title}` for each that has a README.md.
# Title comes from the first `# ` heading; tag-bracket prefix
# (`@PLAN37 — `) is preserved if present, since it's part of the
# canonical plan name.
emit_plans() {
  local plan_root="$1"  # e.g. doc/claude/plans
  local sub="$2"        # "" for active (top-level), "future" / "deferred" / "finished"
  local mtime_window="$3"  # seconds, 0 to skip filter
  local search_dir
  if [ -z "$sub" ]; then
    search_dir="$plan_root"
  else
    search_dir="$plan_root/$sub"
  fi
  [ -d "$search_dir" ] || return 0
  local now
  now=$(date -u +%s)
  for d in "$search_dir"/[0-9]*-*/; do
    [ -d "$d" ] || continue
    local readme="$d/README.md"
    [ -f "$readme" ] || continue
    if [ "$mtime_window" -gt 0 ]; then
      local m
      # `stat -c %Y` is GNU syntax; macOS BSD stat uses `-f %m`.
      # Try both so the mtime check works on either platform; fall
      # through to `0` to make the rest of the script tolerant.
      m=$(stat -c %Y "$d" 2>/dev/null || stat -f %m "$d" 2>/dev/null || echo 0)
      [ $((now - m)) -le "$mtime_window" ] || continue
    fi
    local slug
    slug=$(basename "$d")
    local rel
    rel=${d#./}
    rel=${rel%/}
    local title
    title=$(grep -m1 '^# ' "$readme" | sed 's/^# //; s/[[:space:]]*$//')
    [ -n "$title" ] || title="$slug"
    title=$(printf '%s' "$title" | sed 's/\\/\\\\/g; s/"/\\"/g')
    rel=$(printf '%s' "$rel" | sed 's/\\/\\\\/g; s/"/\\"/g')
    slug=$(printf '%s' "$slug" | sed 's/\\/\\\\/g; s/"/\\"/g')
    printf '{"slug":"%s","path":"%s","title":"%s"}\n' "$slug" "$rel" "$title"
  done
}

emit_plans "doc/claude/plans" "" 0 \
  | jq -s '. | sort_by(.slug)' > "$PLANS_ACTIVE_TMP"
emit_plans "doc/claude/plans" "future" 0 \
  | jq -s '. | sort_by(.slug)' > "$PLANS_FUTURE_TMP"
emit_plans "doc/claude/plans" "deferred" 0 \
  | jq -s '. | sort_by(.slug)' > "$PLANS_DEFERRED_TMP"
emit_plans "doc/claude/plans" "finished" $((60 * 24 * 60 * 60)) \
  | jq -s '. | sort_by(.slug)' > "$PLANS_RECENT_TMP"
emit_plans "doc/claude/lib_plans" "future" 0 \
  | jq -s '. | sort_by(.slug)' > "$LIB_PLANS_FUTURE_TMP"

jq --slurpfile pa "$PLANS_ACTIVE_TMP" \
   --slurpfile pf "$PLANS_FUTURE_TMP" \
   --slurpfile pd "$PLANS_DEFERRED_TMP" \
   --slurpfile pr "$PLANS_RECENT_TMP" \
   --slurpfile lf "$LIB_PLANS_FUTURE_TMP" \
   '. + {plans_active: $pa[0], plans_future: $pf[0], plans_deferred: $pd[0], plans_recent: $pr[0], lib_plans_future: $lf[0]}' \
   "$OUT" > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"

META_KEYS='. != "broken" and . != "links" and . != "broken_links" and . != "problems_open" and . != "problems_recent" and . != "plans_active" and . != "plans_future" and . != "plans_deferred" and . != "plans_recent" and . != "lib_plans_future"'
count=$(jq 'keys | length' "$OUT")
new_count=$(jq "[keys[] | select(startswith(\"legacy:\") | not) | select($META_KEYS)] | length" "$OUT")
legacy_count=$(jq '[keys[] | select(startswith("legacy:"))] | length' "$OUT")
total_refs=$(jq "[to_entries[] | select(.key as \$k | $META_KEYS) | .value | length] | add // 0" "$OUT")
broken_count=$(jq '.broken | length' "$OUT")
links_count=$(jq '.links | length' "$OUT")
links_total=$(jq '[.links | to_entries[] | .value | length] | add // 0' "$OUT")
broken_links_count=$(jq '.broken_links | length' "$OUT")
problems_open_count=$(jq '.problems_open | length' "$OUT")
problems_recent_count=$(jq '.problems_recent | length' "$OUT")
plans_active_count=$(jq '.plans_active | length' "$OUT")
plans_future_count=$(jq '.plans_future | length' "$OUT")
plans_deferred_count=$(jq '.plans_deferred | length' "$OUT")
plans_recent_count=$(jq '.plans_recent | length' "$OUT")

echo "tools/indexer/scan.sh: wrote $OUT"
echo "  $count distinct tags ($new_count new-form, $legacy_count legacy-form)"
echo "  $total_refs total references"
echo "  $links_count link targets ($links_total inbound links)"
echo "  $problems_open_count open P-issues, $problems_recent_count closed in last 30 days"
echo "  plans: $plans_active_count active, $plans_future_count future, $plans_deferred_count deferred, $plans_recent_count finished in last 60 days"
if [ "$broken_count" -gt 0 ]; then
  echo "  $broken_count broken @-references — run: ./scripts/idx broken"
fi
if [ "$broken_links_count" -gt 0 ]; then
  echo "  $broken_links_count broken markdown links — run: ./scripts/idx broken-links"
fi
