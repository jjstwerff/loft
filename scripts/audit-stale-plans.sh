#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# audit-stale-plans.sh — the safety net under the close-on-merge automation.
# Two sweeps, because a plan drifts in two directions.
#
#   scripts/audit-stale-plans.sh [--repo <owner/repo>] [--range <git-range>]
#
# SWEEP 1 (open plans, ADVISORY).  Lists every OPEN `status:active` issue and
# flags ones that look already shipped, so nothing rots as `status:active` after
# its work landed (the drift this sweep caught by hand in the 2026-06 release:
# @PLN1/5/10/16/18/21).  Per issue #N:
#   DRIFT   — a close DIRECTIVE for @PLN<N> is already on main (should be closed;
#             run close-shipped-plans.sh, or close it directly)
#   review  — no directive found here; check by hand (the work may have shipped
#             via a loft-libs PR this repo's history can't see)
# Both are judgements a human has to make, so neither fails the run.
#
# SWEEP 2 (closed plans, FAILS).  A CLOSED plan must carry a TERMINAL status
# label (`status:finished` / `status:declined`) and no LIVE one beside it: a
# label is a query surface, so `status:next` left on a shipped plan keeps it in
# everyone's next-up queue forever.  Unlike sweep 1 this needs no judgement — the
# state and the label simply contradict each other — and the fix is one command,
# so it EXITS NON-ZERO (2) instead of printing a warning nobody reads.
#
# It is not hypothetical: @PLN48 (`status:future`) and @PLN102 (`status:next`)
# sat mislabeled for a month because close-shipped-plans.sh removed only
# `status:active` and reported `✓` anyway.  That script now strips every live
# label; this sweep is what catches the ones it cannot reach — hand-closes, and
# PRs that used `Refs` instead of `Closes`.
#
# This NEVER closes anything — it only reports.  Closing stays a reviewed action
# (close-shipped-plans.sh / the close-plans workflow).
#
# Exit: 0 clean or advisory-only · 2 at least one mislabeled closed plan.
# Needs: gh (read access to the plans repo), git.
set -euo pipefail

# Must name the same labels as close-shipped-plans.sh's LIVE_LABELS.
LIVE_LABELS=(status:active status:future status:next status:closing)

REPO="loft-lang/plans"; RANGE="main"
while [ $# -gt 0 ]; do
    case "$1" in
        --repo)  REPO="$2"; shift;;
        --range) RANGE="$2"; shift;;
        -h|--help) sed -n '2,40p' "$0"; exit 0;;
        *) echo "unknown argument: $1" >&2; exit 2;;
    esac
    shift
done

# ---------------------------------------------------------------------------
# Sweep 1 — open status:active plans whose work may already have shipped.
# ---------------------------------------------------------------------------
mapfile -t ROWS < <(gh issue list --repo "$REPO" --state open --label status:active \
    --limit 200 --json number,title -q '.[] | "\(.number)\t\(.title)"')

drift=0
if [ "${#ROWS[@]}" -eq 0 ]; then
    echo "No open status:active plans in $REPO — clean."
else
    # All commit messages on the range, once, for cheap per-issue grep.
    LOG=$(git log --format='%B' "$RANGE" 2>/dev/null || true)

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
fi

# ---------------------------------------------------------------------------
# Sweep 2 — CLOSED plans still carrying a live status label.  The hard one.
# ---------------------------------------------------------------------------
# One query for every closed plan with its labels; the live/terminal split is
# decided here rather than by four separate label-filtered queries, because
# `gh issue list --label` ANDs its labels and this question is an OR.
live_re=$(IFS='|'; echo "${LIVE_LABELS[*]}")
mapfile -t BAD < <(gh issue list --repo "$REPO" --state closed --limit 300 \
    --json number,title,labels \
    -q ".[] | select([.labels[].name] | any(test(\"^(${live_re})$\"))) |
        \"\(.number)\t\(.title)\t\([.labels[].name] | join(\",\"))\"")

echo
if [ "${#BAD[@]}" -eq 0 ]; then
    echo "No closed plan carries a live status label — clean."
else
    echo "CLOSED plans still labelled as live work in $REPO:"
    for row in "${BAD[@]}"; do
        n="${row%%$'\t'*}"; rest="${row#*$'\t'}"
        title="${rest%%$'\t'*}"; labels="${rest#*$'\t'}"
        printf '  MISLABEL #%-4s %-50s [%s]\n' "$n" "${title:0:50}" "$labels"
    done
fi

echo "---"
if [ "$drift" -gt 0 ]; then
    echo "$drift plan(s) have a close-directive already on $RANGE but are still open."
    echo "Fix: scripts/close-shipped-plans.sh --range <release-range>  (or close them directly)."
fi
if [ "${#BAD[@]}" -gt 0 ]; then
    echo "${#BAD[@]} closed plan(s) still carry a live status label."
    echo "A closed plan must be status:finished (delivered) or status:declined (de-scoped)."
    echo "Fix each: gh issue edit <n> -R $REPO --remove-label <the live one>"
    exit 2
fi
