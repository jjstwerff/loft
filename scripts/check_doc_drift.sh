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

# ---- Check 4: ROADMAP plan-state cross-check ----
# Rules:
#   - Active plans (plans/<NN>, plans/future/<NN>) MUST appear on ROADMAP at
#     their canonical path.  No dual citation at the wrong bucket.
#   - Deferred plans (plans/deferred/<NN>) MUST NOT appear on ROADMAP.
#     Their home is DEFERRED.md (trigger index).
#   - Finished plans (plans/finished/<NN>) MUST NOT appear on ROADMAP as
#     action items.  Closure lives in CHANGELOG + git history.
#     Parenthetical historical mentions are tolerated.
check_roadmap() {
  echo "=== ROADMAP plan-state cross-check ==="
  local hits=0
  local roadmap=doc/claude/ROADMAP.md
  if [ ! -f "$roadmap" ]; then
    yellow "  $roadmap missing — skipping"
    return
  fi

  for tracker in plans lib_plans; do
    for bucket in '' future deferred finished; do
      bucket_dir="doc/claude/$tracker${bucket:+/$bucket}"
      [ -d "$bucket_dir" ] || continue
      for plan_dir in "$bucket_dir"/[0-9]*/; do
        [ -d "$plan_dir" ] || continue
        slug=$(basename "$plan_dir")
        if [ -z "$bucket" ]; then
          canonical="$tracker/$slug"
        else
          canonical="$tracker/$bucket/$slug"
        fi

        case "$bucket" in
          finished|deferred)
            # MUST NOT appear on ROADMAP as action item.
            # Tolerate parenthetical / "Shipped 2026-XX" historical mentions.
            offending=$(grep -nE "$tracker/$bucket/$slug" "$roadmap" \
                        | grep -vE 'Shipped [0-9]|\(plans/(finished|deferred)/|^[0-9]+:#' )
            # Also flag short-form citation in active-bucket position
            # (e.g. plans/28-const-store when plan is in deferred/).
            short=$(grep -nE "(^|[^/])$tracker/$slug([^/0-9a-z-]|$)" "$roadmap")
            if [ -n "$offending" ] || [ -n "$short" ]; then
              red "  $canonical → $bucket plan still on ROADMAP:"
              { [ -n "$offending" ] && echo "$offending"; [ -n "$short" ] && echo "$short"; } \
                | sort -u | sed 's/^/    /' | head -4
              hits=$((hits + 1))
            fi
            ;;
          *)
            # Active plan (current or future).  Must be cited at canonical path.
            if ! grep -qE "$canonical" "$roadmap"; then
              wrong=$(grep -nE "$tracker(/(future|deferred|finished))?/$slug" "$roadmap" | head -1)
              if [ -n "$wrong" ]; then
                red "  $canonical → wrong bucket on ROADMAP:"
                echo "    $wrong"
              else
                red "  $canonical → MISSING from ROADMAP"
              fi
              hits=$((hits + 1))
            else
              # Check for dual / stale citations at OTHER buckets.
              other_buckets=""
              for other_b in '' future deferred finished; do
                [ "$other_b" = "$bucket" ] && continue
                if [ -z "$other_b" ]; then
                  other_path="$tracker/$slug"
                else
                  other_path="$tracker/$other_b/$slug"
                fi
                [ "$other_path" = "$canonical" ] && continue
                # Match boundary: not followed by alphanum/dash so we don't
                # match plans/14-tuple as a substring of plans/14-tuple-foo.
                if grep -qE "$other_path([^/0-9a-zA-Z-]|/|$)" "$roadmap"; then
                  other_buckets="$other_buckets $other_path"
                fi
              done
              if [ -n "$other_buckets" ]; then
                red "  $canonical → cited at canonical AND wrong path(s):$other_buckets"
                hits=$((hits + 1))
              fi
            fi
            ;;
        esac
      done
    done
  done

  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits ROADMAP/disk drift items"
    DRIFT=1
  fi
}

# ---- Check 5: closed/deferred plan references from normal docs ----
# Closure rule: when a plan ships, reference content moves OUT to
# doc/claude/<NAME>.md.  Other docs link to the reference home, NOT
# the closed plan.  Same for deferred plans (their content is
# either reference-extracted or part of design-content kept in the
# plan README; non-plan docs shouldn't deep-link into deferred plan
# files for ongoing reference).
#
# Allowed: links FROM other plan READMEs (cross-arc references) and
# FROM the closed/deferred plan's siblings.  Flagged: links from
# normal reference docs (doc/claude/<NAME>.md, CLAUDE.md, lib/<name>/*.md).
check_refs() {
  echo "=== finished/deferred plan refs from normal docs ==="
  local hits=0
  # Find every markdown link target containing plans/finished/ or plans/deferred/.
  # Tolerance pattern (closure narrative + status annotations + design-pointer phrases).
  local tol_pat='closed by|closure record|shipped (via|by)|historical|retrospective|moved to .*finished|moved to .*deferred|\(closed [0-9]|\(active\)|\(shipped|\(deferred|preserved at|originally documented|design lives|design content|Phase A \+ D|fixture catalogue|spec captured in|catalogue from|deferred follow-ups|trigger conditions'
  while IFS= read -r line; do
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    text="${rest#*:}"
    # Skip:
    # - Links FROM plan READMEs themselves (cross-arc references are fine).
    # - Links FROM presentations/ (not real documentation; archaeology).
    case "$file" in
      */plans/*|*/lib_plans/*|*/presentations/*) continue ;;
    esac
    # Build context window: 3 lines back + current + 1 forward.  Markdown
    # paragraphs wrap heavily so a closure-narrative phrase can sit several
    # lines above the link.
    context=$(awk -v n="$lineno" 'NR>=n-3 && NR<=n+1' "$file" 2>/dev/null)
    while read -r target; do
      clean="${target%%#*}"
      clean="${clean%%\?*}"
      [ -z "$clean" ] && continue
      case "$clean" in
        *plans/finished/*|*plans/deferred/*) ;;
        *) continue ;;
      esac
      # Tolerate explicit closure-narrative + status-annotation patterns.
      if echo "$context" | grep -qiE "$tol_pat"; then
        continue
      fi
      # Deep-link to a phase file (not just the README) is always drift —
      # reference content should have moved out per the closure rule.
      case "$clean" in
        *plans/finished/*/*.md|*plans/deferred/*/*.md)
          case "$clean" in
            */README.md|*/README.md\#*) continue ;;
          esac
          red "  $file:$lineno → $clean (deep-link to phase file)"
          hits=$((hits + 1))
          continue
          ;;
      esac
      red "  $file:$lineno → $clean"
      hits=$((hits + 1))
    done < <(echo "$text" \
      | grep -oE '\]\([^)]*\)' \
      | sed -E 's/^\]\(//; s/\)$//')
  done < <(grep -rn -E '\]\([^)]*plans/(finished|deferred)/' \
              doc/claude/ CLAUDE.md lib/ --include='*.md' 2>/dev/null \
            | grep -v 'check_doc_drift.sh')

  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits suspect refs (normal doc → finished/deferred plan)"
    red "  Expected: link to the reference home (doc/claude/<NAME>.md) or to the canonical lib doc."
    red "  Tolerated: explicit closure-narrative ('closed by', 'shipped via', 'historical', etc.)"
    DRIFT=1
  fi
}

case "$CHECK" in
  paths)   check_paths ;;
  time)    check_time ;;
  stale)   check_stale ;;
  roadmap) check_roadmap ;;
  refs)    check_refs ;;
  all)
    check_paths
    echo
    check_time
    echo
    check_stale
    echo
    check_roadmap
    echo
    check_refs
    ;;
  *)
    echo "Usage: $0 [all|paths|time|stale|roadmap|refs]" >&2
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
