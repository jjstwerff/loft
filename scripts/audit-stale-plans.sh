#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# audit-stale-plans.sh — the safety net under the close-on-merge automation.
# Lists every OPEN `status:active` plan issue and flags ones that look already
# shipped, so nothing rots as `status:active` after its work landed (the drift
# this exact sweep caught by hand in the 2026-06 release: @PLN1/5/10/16/18/21).
#
#   scripts/audit-stale-plans.sh [--repo <owner/repo>] [--range <git-range>]
#
# For each status:active issue #N it reports a verdict:
#   DRIFT   — a close DIRECTIVE for @PLN<N> is already on main (should be closed;
#             run close-shipped-plans.sh, or close it directly)
#   review  — no directive found here; check by hand (the work may have shipped
#             via a loft-libs PR this repo's history can't see)
#
# This NEVER closes anything — it only reports.  Closing stays a reviewed action
# (close-shipped-plans.sh / the close-plans workflow).
# Needs: gh (read access to the plans repo), git.
set -euo pipefail

REPO="loft-lang/plans"; RANGE="main"
while [ $# -gt 0 ]; do
    case "$1" in
        --repo)  REPO="$2"; shift;;
        --range) RANGE="$2"; shift;;
        -h|--help) sed -n '2,25p' "$0"; exit 0;;
        *) echo "unknown argument: $1" >&2; exit 2;;
    esac
    shift
done

mapfile -t ROWS < <(gh issue list --repo "$REPO" --state open --label status:active \
    --limit 200 --json number,title -q '.[] | "\(.number)\t\(.title)"')

if [ "${#ROWS[@]}" -eq 0 ]; then
    echo "No open status:active plans in $REPO — clean."
    exit 0
fi

# All commit messages on the range, once, for cheap per-issue grep.
LOG=$(git log --format='%B' "$RANGE" 2>/dev/null || true)

drift=0
echo "Open status:active plans in $REPO ($RANGE history for drift check):"
for row in "${ROWS[@]}"; do
    n="${row%%$'\t'*}"; title="${row#*$'\t'}"
    if printf '%s\n' "$LOG" \
        | grep -iqE "(close[sd]?|fix(e[sd]?)?|resolve[sd]?)[[:space:]]+(loft-lang/plans#|@PLN)$n\b"; then
        printf '  DRIFT   #%-4s %s\n' "$n" "${title:0:60}"
        drift=$((drift+1))
    else
        printf '  review  #%-4s %s\n' "$n" "${title:0:60}"
    fi
done

echo "---"
if [ "$drift" -gt 0 ]; then
    echo "$drift plan(s) have a close-directive already on $RANGE but are still open."
    echo "Fix: scripts/close-shipped-plans.sh --range <release-range>  (or close them directly)."
fi
