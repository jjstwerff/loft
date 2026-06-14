#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# close-shipped-plans.sh — close loft-lang/plans issues that a shipped change
# resolves.  GitHub's `Fixes #N` auto-close is SAME-REPO ONLY; plans live in a
# separate repo (loft-lang/plans), so a loft / loft-libs PR can never auto-close
# them.  This is the explicit cross-repo closer: it scans text for plan-close
# DIRECTIVES, then sets each referenced plan issue to `status:finished` + closes
# it.  It is idempotent (already-closed issues are skipped) and safe to re-run.
#
#   scripts/close-shipped-plans.sh [--range <git-range>] [--body-file <file>]
#                                  [--repo <owner/repo>] [--dry-run] [--yes]
#
#   --range <a>..<b>   scan commit messages in this git range (the release flow:
#                      the commits that just reached main).  Default: from the
#                      most recent tag merged into HEAD to HEAD.
#   --body-file <f>    scan this text file instead (a PR body — used by the
#                      close-plans GitHub Action).  Wins over --range.
#   --repo             plans repo (default: loft-lang/plans).
#   --dry-run          print what would close; change nothing.
#   --yes              skip the confirm prompt.
#
# A close DIRECTIVE is an explicit verb + plan ref (NOT a bare `@PLN22` mention,
# which appears in many non-closing commits):
#     Closes @PLN22            Fixes loft-lang/plans#22
#     Resolves @PLN5           closed loft-lang/plans#10
# (verbs: close/closes/closed/fix/fixes/fixed/resolve/resolves/resolved.)
#
# Needs: gh (authenticated, write access to the plans repo), git.
set -euo pipefail

REPO="loft-lang/plans"; RANGE=""; BODY_FILE=""; DRY=0; YES=0
while [ $# -gt 0 ]; do
    case "$1" in
        --range)     RANGE="$2"; shift;;
        --body-file) BODY_FILE="$2"; shift;;
        --repo)      REPO="$2"; shift;;
        --dry-run)   DRY=1;;
        --yes)       YES=1;;
        -h|--help)   sed -n '2,40p' "$0"; exit 0;;
        *) echo "unknown argument: $1" >&2; exit 2;;
    esac
    shift
done

# 1. Gather the text to scan.
if [ -n "$BODY_FILE" ]; then
    [ -f "$BODY_FILE" ] || { echo "no such body file: $BODY_FILE" >&2; exit 2; }
    TEXT=$(cat "$BODY_FILE")
    SRC="body file $BODY_FILE"
else
    if [ -z "$RANGE" ]; then
        last_tag=$(git tag --merged HEAD --sort=-creatordate 2>/dev/null | head -1 || true)
        RANGE="${last_tag:+$last_tag..}HEAD"
    fi
    git rev-parse "${RANGE%%..*}" >/dev/null 2>&1 || { echo "bad git range: $RANGE" >&2; exit 2; }
    TEXT=$(git log --format='%B' "$RANGE")
    SRC="git range $RANGE"
fi

# 2. Extract plan numbers from close directives only.  The verb and the ref may
#    sit on the same line (commit/PR bodies wrap the directive onto one line).
NUMS=$(printf '%s\n' "$TEXT" \
    | grep -ioE '(close[sd]?|fix(e[sd]?)?|resolve[sd]?)[[:space:]]+(loft-lang/plans#|@PLN)[0-9]+' \
    | grep -oE '[0-9]+$' \
    | sort -un || true)

if [ -z "$NUMS" ]; then
    echo "no plan-close directives found in $SRC."
    echo "  (use e.g. 'Closes @PLN22' / 'Fixes loft-lang/plans#22' in the PR body.)"
    exit 0
fi

echo "Plan-close directives in $SRC → $REPO issues: $(echo "$NUMS" | tr '\n' ' ')"
[ "$DRY" = 1 ] && echo "(dry-run — no changes)"
if [ "$DRY" != 1 ] && [ "$YES" != 1 ]; then
    printf "Set status:finished + close these plan issues? [y/N] "
    read -r ans; case "$ans" in y|Y) ;; *) echo "aborted."; exit 0;; esac
fi

# 3. Close each (idempotent).
for n in $NUMS; do
    state=$(gh issue view "$n" --repo "$REPO" --json state -q .state 2>/dev/null || echo MISSING)
    case "$state" in
        MISSING) echo "  #$n — not found in $REPO, skipping" ;;
        CLOSED)  echo "  #$n — already closed, skipping" ;;
        OPEN)
            if [ "$DRY" = 1 ]; then
                echo "  #$n — would set status:finished + close"
            else
                gh issue edit "$n" --repo "$REPO" \
                    --remove-label status:active --add-label status:finished >/dev/null 2>&1 || true
                gh issue close "$n" --repo "$REPO" \
                    --comment "Shipped to main — closed by close-shipped-plans (from $SRC). status:finished." >/dev/null
                echo "  #$n — status:finished + closed ✓"
            fi
            ;;
        *) echo "  #$n — unexpected state '$state', skipping" ;;
    esac
done
