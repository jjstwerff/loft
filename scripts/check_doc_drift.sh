#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Drift detection for doc/claude/.  Catches the patterns that
# routinely rot in plan / reference docs:
#
#   1. Broken plan links — markdown links of the form
#      `[...](path/to/plan)` where the resolved path doesn't exist.
#      Plans move between current/future/deferred/finished and
#      links don't always follow.
#   2. Time-projection language (multi-week, 2-3 weeks, etc.).
#   3. Stale "is current" claims about retired features (text_code,
#      Type::Long, .loftc, forwarding_smoke.rs).
#
# Reports findings; does NOT fix.  Exit 0 if clean, 1 if drift
# found that's likely real (broken paths, stale claims).  Time
# projections are warnings only.
#
# Usage:
#   scripts/check_doc_drift.sh             # all checks
#   scripts/check_doc_drift.sh paths       # only path drift
#   scripts/check_doc_drift.sh time        # only time projections
#   scripts/check_doc_drift.sh stale       # only stale claims

set -u

cd "$(dirname "$0")/.."

CHECK="${1:-all}"
DRIFT=0

red()    { printf '\033[31m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }
green()  { printf '\033[32m%s\033[0m\n' "$*"; }

# ---- Check 1: broken markdown links to plans ----
check_paths() {
  echo "=== Broken plan links ==="
  local hits=0
  # Match markdown links [...](url) where url contains plans/<NN>-<slug>.
  # Resolve the url relative to the containing file.
  while IFS= read -r line; do
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    text="${rest#*:}"
    # Match markdown-link targets: ](path) — explicit ] before ( so we
    # don't capture surrounding prose.  Multiple links per line OK.
    while read -r target; do
      # Strip trailing fragment (#section) and query.
      clean="${target%%#*}"
      clean="${clean%%\?*}"
      [ -z "$clean" ] && continue
      # Skip non-plan targets in the same line.
      case "$clean" in
        *plans/*[0-9]-*) ;;
        *) continue ;;
      esac
      # Resolve relative to file's directory.
      dir=$(dirname "$file")
      candidate=$(realpath -m --relative-to=. "$dir/$clean" 2>/dev/null) || continue
      check_path="${candidate%/}"
      if [ ! -e "$check_path" ]; then
        red "  $file:$lineno → $clean (resolved: $candidate)"
        hits=$((hits + 1))
      fi
    done < <(echo "$text" \
      | grep -oE '\]\([^)]*\)' \
      | sed -E 's/^\]\(//; s/\)$//')
  done < <(grep -rn -E '\]\([^)]*(lib_plans|plans)/[^)]*[0-9]+-[a-z0-9-]+' \
              doc/claude/ CLAUDE.md --include='*.md' 2>/dev/null \
            | grep -v 'check_doc_drift.sh')
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits broken plan links"
    DRIFT=1
  fi
}

# ---- Check 2: time-projection language ----
check_time() {
  echo "=== Time projections ==="
  local hits=0
  local patterns=(
    'weeks? of focused'
    '[0-9]+-[0-9]+ weeks'
    'multi-week'
    'next [0-9]+ months'
    'expected to take'
    'Estimated cost.*hours'
    'Estimated cost.*sessions'
  )
  for pat in "${patterns[@]}"; do
    while IFS= read -r match; do
      case "$match" in
        *plans/finished/*|*CHANGELOG*|*plans/_LIFECYCLE.md*|*scripts/check_doc_drift.sh*)
          continue
          ;;
      esac
      yellow "  $match"
      hits=$((hits + 1))
    done < <(grep -rn -E "$pat" doc/claude/ CLAUDE.md --include='*.md' 2>/dev/null)
  done
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    yellow "  $hits time projections (consider effort letters XS/S/M/MH/H/VH/L)"
    # Time projections are warnings, not errors.
  fi
}

# ---- Check 3: stale claims about retired features ----
check_stale() {
  echo "=== Stale 'is current' claims about retired features ==="
  local hits=0
  # Tighter patterns: only Rust-code-block or definition-shape mentions
  # (excludes prose mentions in "removed/retired" context).
  local stale_patterns=(
    'pub.*text_code:.*Vec<u8>'
    'text_code: \*const Vec<u8>'
    'pub Long,?\s*//'
    'src/generation/ops/forwarding_smoke\.rs'
    '\.loftc.*current|byte_code_with_cache'
  )
  for pat in "${stale_patterns[@]}"; do
    while IFS= read -r match; do
      file="${match%%:*}"
      case "$file" in
        */CHANGELOG*|*/plans/finished/*|*/plans/deferred/*|*scripts/check_doc_drift.sh)
          continue
          ;;
      esac
      line_text="${match#*:}"
      line_text="${line_text#*:}"
      if echo "$line_text" | grep -qiE 'removed|retired|no longer|previous|legacy|former|was '; then
        continue
      fi
      red "  $match"
      hits=$((hits + 1))
    done < <(grep -rn -E "$pat" doc/claude/ CLAUDE.md --include='*.md' 2>/dev/null)
  done
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits potentially stale claims"
    DRIFT=1
  fi
}

case "$CHECK" in
  paths) check_paths ;;
  time)  check_time ;;
  stale) check_stale ;;
  all)
    check_paths
    echo
    check_time
    echo
    check_stale
    ;;
  *)
    echo "Usage: $0 [all|paths|time|stale]" >&2
    exit 2
    ;;
esac

if [ $DRIFT -eq 0 ]; then
  green "ALL CHECKS PASSED"
  exit 0
else
  red "DRIFT DETECTED — see report above"
  exit 1
fi
