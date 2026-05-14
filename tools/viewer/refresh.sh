#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# tools/viewer/refresh.sh — dump git state for loft-view to consume.
#
# Loft has no subprocess primitive yet; this script is the bridge
# between `git` and the viewer.  The viewer reads
# tools/viewer/state/*.json at request time and renders the
# branch dashboard.
#
# Re-run on demand:
#   make view-refresh           dump state without restarting server
#   make view                   dump state + start server
set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
  echo "tools/viewer/refresh.sh: needs jq (apt install jq / dnf install jq)" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STATE="$ROOT/tools/viewer/state"
mkdir -p "$STATE"

cd "$ROOT"

# 1. Branch header — branch name, HEAD sha + msg, ahead/behind vs main.
{
  branch=$(git rev-parse --abbrev-ref HEAD)
  head_sha=$(git rev-parse --short HEAD)
  head_msg=$(git log -1 --pretty=%s)
  if git rev-parse --verify main >/dev/null 2>&1; then
    # `git rev-list --left-right --count main...HEAD` outputs
    # "<commits-only-in-main>\t<commits-only-in-HEAD>", so the
    # FIRST value is "behind main" and the SECOND is "ahead of main".
    read -r behind ahead <<< "$(git rev-list --left-right --count main...HEAD | tr '\t' ' ')"
  else
    ahead=0; behind=0
  fi
  jq -n \
    --arg branch "$branch" \
    --arg head_sha "$head_sha" \
    --arg head_msg "$head_msg" \
    --argjson ahead "$ahead" \
    --argjson behind "$behind" \
    '{branch: $branch, head_sha: $head_sha, head_msg: $head_msg, ahead: $ahead, behind: $behind}'
} > "$STATE/branch.json"

# 2. Files changed vs main (name-status).  Empty array on a fresh
# repo where `main` doesn't exist yet.
#
# `git diff --name-status` emits renames as three tab-separated
# fields: `R<n>\t<old_path>\t<new_path>` (e.g. `R099` for 99%
# similarity).  Without rename-aware handling the dashboard would
# link at the OLD (pre-rename) path, which 404s after the move —
# e.g. `doc/claude/plans/06-typed-par/02-stitch-not-copy.md` shown
# instead of the actual `…/finished/06-typed-par/02-…`.  For `R*`
# / `C*` rows take `.[2]` (new path); for plain `M`/`A`/`D`/`?`
# rows take `.[1]`.
if git rev-parse --verify main >/dev/null 2>&1; then
  git diff --name-status main...HEAD | jq -Rn '
    [inputs
      | select(. != "")
      | split("\t")
      | { status: .[0],
          path: (if (.[0] | startswith("R")) or (.[0] | startswith("C"))
                 then .[2] else .[1] end) }]
  ' > "$STATE/changed.json"
else
  echo "[]" > "$STATE/changed.json"
fi

# 3. Recent commits (last 20)
git log --oneline -20 --pretty='%h%x09%s' | jq -Rn '
  [inputs | select(. != "") | split("\t") | {sha: .[0], msg: .[1]}]
' > "$STATE/commits.json"

# 4. Uncommitted (porcelain v1).  Two-char status code stripped of
# leading/trailing space; rest is the path.
git status --short | jq -Rn '
  [inputs | select(. != "") | {status: .[0:2] | gsub(" "; ""), path: .[3:]}]
' > "$STATE/uncommitted.json"

# 5. Per-file diffs vs main, capped at 100 files.  Each diff is
# saved with `/` → `__` so the filename is filesystem-safe.
DIFFS_DIR="$STATE/diffs"
rm -rf "$DIFFS_DIR" && mkdir -p "$DIFFS_DIR"
if git rev-parse --verify main >/dev/null 2>&1; then
  git diff --name-only main...HEAD | head -100 | while read -r f; do
    [ -z "$f" ] && continue
    safe="${f//\//__}"
    git diff main...HEAD -- "$f" > "$DIFFS_DIR/$safe.diff"
  done
fi

# 6. Per-commit diffs for the recent-commits list.
COMMITS_DIR="$STATE/commits"
rm -rf "$COMMITS_DIR" && mkdir -p "$COMMITS_DIR"
git log --pretty=%h -20 | while read -r sha; do
  [ -z "$sha" ] && continue
  git show "$sha" > "$COMMITS_DIR/$sha.diff"
done

echo "loft-view state refreshed: $(date)"
