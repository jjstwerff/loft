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

# The LIVE status labels a plan can be sitting on when it ships.  A closed plan
# must carry a TERMINAL one (`status:finished` / `status:declined`) and no live
# one beside it, because a label is a query surface: `status:next` left on a
# closed plan keeps it in everyone's next-up queue forever.
#
# Removing only `status:active` is what left @PLN48 (`future`) and @PLN102
# (`next`) mislabeled — and the run reported `✓` both times, which is why it went
# unnoticed until a by-hand audit found them.  `scripts/audit-stale-plans.sh` now
# FAILS on the class, so this list and that check must name the same labels.
LIVE_LABELS="status:active status:future status:next status:closing"

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
# The live labels currently on issue $1, one per line (empty when none).
stale_labels() {
    local have
    have=$(gh issue view "$1" --repo "$REPO" --json labels -q '.labels[].name' 2>/dev/null || true)
    local l
    for l in $LIVE_LABELS; do
        # `if`, not `&&`: under `set -e` an AND-OR list that fails as the last
        # command of a loop body takes the shell down with it, and "this plan has
        # no stale label" is the COMMON case.
        if printf '%s\n' "$have" | grep -qx "$l"; then
            printf '%s\n' "$l"
        fi
    done
    return 0
}

for n in $NUMS; do
    state=$(gh issue view "$n" --repo "$REPO" --json state -q .state 2>/dev/null || echo MISSING)
    case "$state" in
        MISSING) echo "  #$n — not found in $REPO, skipping" ;;
        CLOSED)
            # Idempotent re-run, or a hand-close. Either way the LABELS may still
            # be wrong, and saying so is the difference between this drifting
            # silently and someone fixing it.
            stale=$(stale_labels "$n" | tr '\n' ' ')
            if [ -n "${stale// /}" ]; then
                echo "  #$n — already closed, but still labelled: ${stale% } (run scripts/audit-stale-plans.sh)"
            else
                echo "  #$n — already closed, skipping"
            fi
            ;;
        OPEN)
            if [ "$DRY" = 1 ]; then
                stale=$(stale_labels "$n" | tr '\n' ' ')
                echo "  #$n — would set status:finished + close${stale:+, dropping ${stale% }}"
            else
                # EVERY live label goes, not just `status:active`. Only the ones
                # actually present are named: `--remove-label` on an absent label
                # is an error, and swallowing that error is how a failed label
                # edit used to print `✓`.
                rm_args=()
                while IFS= read -r l; do
                    [ -n "$l" ] && rm_args+=(--remove-label "$l")
                done < <(stale_labels "$n")
                if gh issue edit "$n" --repo "$REPO" \
                       ${rm_args[@]+"${rm_args[@]}"} --add-label status:finished >/dev/null; then
                    label_note=""
                else
                    label_note=" (LABEL UPDATE FAILED — fix by hand)"
                fi
                gh issue close "$n" --repo "$REPO" \
                    --comment "Shipped to main — closed by close-shipped-plans (from $SRC). status:finished." >/dev/null
                echo "  #$n — status:finished + closed ✓$label_note"
            fi
            ;;
        *) echo "  #$n — unexpected state '$state', skipping" ;;
    esac
done
