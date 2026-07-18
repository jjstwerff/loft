#!/usr/bin/env bash
# Nightly COVERAGE guard for the lib-validation matrix.
#
# registry-validation and revalidate-libs both build their matrix from the
# REGISTRY INDEX.  So a package that lives in a loft-libs-* repo but was never
# published / indexed is tested by NOTHING, and no red night ever appears — the
# lib silently drops out of CI.  That is exactly how graphics-stack packages sat
# untested until a human noticed the gap.
#
# This guard closes it: enumerate every package across every loft-lang/loft-libs-*
# repo (the ground truth), and FAIL if any is absent from the registry index (and
# therefore from the nightly matrix).  A package that is intentionally unpublished
# goes in the ignore list below so the gate stays meaningful.
#
# Usage:  scripts/lib_coverage_check.sh
# Env:    GH_TOKEN / GITHUB_TOKEN   gh auth (the default Actions token suffices —
#                                   the lib repos are public)
#         LOFT_REGISTRY_INDEX       override the index URL (default: live index)
#         LIB_COVERAGE_IGNORE       space-separated package names to exempt
#                                   (intentionally-unpublished / internal)
set -uo pipefail

INDEX_URL="${LOFT_REGISTRY_INDEX:-https://raw.githubusercontent.com/loft-lang/registry/main/index.json}"
IGNORE=" ${LIB_COVERAGE_IGNORE:-} "   # padded so word-match is exact

work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT

curl -sSfL "$INDEX_URL" -o "$work/index.json" || { echo "::error::coverage: cannot fetch registry index"; exit 1; }
jq -r '.packages | keys[]' "$work/index.json" | sort -u > "$work/indexed.txt"

repos="$(gh repo list loft-lang --limit 200 --json name --jq '.[].name' | grep '^loft-libs-' | sort)"
[ -n "$repos" ] || { echo "::error::coverage: found no loft-libs-* repos"; exit 1; }

missing=0; total=0; ignored=0
: > "$work/report.txt"
for r in $repos; do
  # every directory shipping a loft.toml is a package; its authoritative name is
  # the loft.toml's declared `name` (dir basename is only a fallback).
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    name="$(gh api "repos/loft-lang/$r/contents/$path" --jq '.content' 2>/dev/null \
            | base64 -d 2>/dev/null | grep -iE '^[[:space:]]*name[[:space:]]*=' | head -1 \
            | sed -E 's/.*=[[:space:]]*"?([A-Za-z0-9_-]+)"?.*/\1/')"
    [ -n "$name" ] || name="$(basename "$(dirname "$path")")"
    total=$((total+1))
    if [[ "$IGNORE" == *" $name "* ]]; then
      printf 'SKIP  %-14s %s/%s  — LIB_COVERAGE_IGNORE\n' "$name" "$r" "$path" >> "$work/report.txt"
      ignored=$((ignored+1))
    elif grep -qxF "$name" "$work/indexed.txt"; then
      printf 'OK    %-14s %s/%s\n' "$name" "$r" "$path" >> "$work/report.txt"
    else
      printf 'MISS  %-14s %s/%s  — not in registry index -> NOT in the nightly matrix\n' "$name" "$r" "$path" >> "$work/report.txt"
      missing=$((missing+1))
    fi
  done < <(gh api "repos/loft-lang/$r/git/trees/main?recursive=1" \
             --jq '.tree[] | select(.path|endswith("loft.toml")) | .path' 2>/dev/null)
done

sort "$work/report.txt"
covered=$((total - missing - ignored))
echo "coverage: $covered/$total repo packages in the nightly matrix ($ignored ignored, $missing UNCOVERED)."

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### Lib CI coverage — $covered/$total repo packages in the nightly matrix"
    if [ "$missing" -gt 0 ]; then
      echo "**$missing UNCOVERED** — a repo package absent from the registry index is tested by NO nightly:"
      echo '```'; grep '^MISS' "$work/report.txt"; echo '```'
    else
      echo "All repo packages are in the matrix. ✅ _($ignored intentionally ignored)_"
    fi
  } >> "$GITHUB_STEP_SUMMARY"
fi

if [ "$missing" -gt 0 ]; then
  echo "::error::lib coverage: $missing repo package(s) are absent from the registry index and thus untested by the nightly — publish/register them, or add to LIB_COVERAGE_IGNORE if retired/internal."
  grep '^MISS' "$work/report.txt" | sed 's/^/  /'
  exit 1
fi
echo "::notice::lib coverage: all $covered repo packages across $(echo "$repos" | wc -w) repos are in the nightly matrix."
