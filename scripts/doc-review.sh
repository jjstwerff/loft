#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Monthly library-documentation review worklist (@PLN141).  A REPORT, never a
# gate — like `make speed`.  It feeds the by-hand protocol in
# doc/claude/LIBRARY_DOC_REVIEW.md: the automated `check_doc_drift.sh examples`
# gate catches example tags that DANGLE or DUPLICATE, but it cannot see a doc
# that still resolves yet no longer describes what the code does (staleness),
# nor an example that is valid but no longer the clearest one.  Those need a
# human; this script hands that human a bounded, high-signal worklist so the
# monthly pass reads the functions most likely to have drifted, not all ~350.
#
# Usage:
#   scripts/doc-review.sh                       # coverage + inventory, default+lib
#   scripts/doc-review.sh --since <ref>         # + public API changed since <ref>
#   scripts/doc-review.sh --since <ref> lib/git # limit to one tree
#
# <ref> is normally last month's watermark commit (see the protocol's watermark
# table).  Exit is always 0 — this reports, it does not block.

set -u
cd "$(dirname "$0")/.."

SINCE=""
TREES=()
while [ $# -gt 0 ]; do
  case "$1" in
    --since) SINCE="${2:-}"; shift 2 ;;
    --since=*) SINCE="${1#--since=}"; shift ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) TREES+=("$1"); shift ;;
  esac
done
[ ${#TREES[@]} -eq 0 ] && TREES=(default lib)

echo "== Pre-flight: worked-example gate (must be green before the manual pass) =="
if scripts/check_doc_drift.sh examples >/tmp/doc_review_gate.$$ 2>&1; then
  tail -1 /tmp/doc_review_gate.$$
else
  echo "  GATE RED — resolve dangling/duplicate citations first:"
  sed 's/^/    /' /tmp/doc_review_gate.$$
fi
rm -f /tmp/doc_review_gate.$$

for tree in "${TREES[@]}"; do
  [ -e "$tree" ] || continue
  echo
  echo "== $tree =="
  # Coverage — a health signal, NOT a target: most functions are self-evident
  # from their signature and correctly carry no example (the opt-in ratchet).
  total=$(grep -rh "pub fn " "$tree" --include='*.loft' 2>/dev/null | wc -l | tr -d ' ')
  cited=$(grep -rho "// Example: @[A-Z][A-Z][A-Z]-[0-9][0-9][0-9]" "$tree" --include='*.loft' 2>/dev/null | wc -l | tr -d ' ')
  echo "  public fns: ${total:-0}   worked-example citations: ${cited:-0}"

  # Inventory — every citation, so the reviewer can spot-check that each still
  # points at the clearest demonstration (step 4 of the protocol).
  inv=$(grep -rn "// Example: @[A-Z][A-Z][A-Z]-[0-9][0-9][0-9]" "$tree" --include='*.loft' 2>/dev/null)
  if [ -n "$inv" ]; then
    echo "  -- example citations (open each cited test; is it still the clearest use?) --"
    echo "$inv" | sed 's/^/    /'
  fi

  # Staleness worklist — a changed public signature is the #1 source of a stale
  # doc.  Re-read the `///` above each of these against the current body.
  if [ -n "$SINCE" ]; then
    changed=$(git diff --unified=0 "$SINCE"..HEAD -- "$tree" 2>/dev/null \
              | grep -E '^\+[^+].*pub fn ' | sed -E 's/^\+[[:space:]]*/    /')
    echo "  -- public API changed since $SINCE (re-read the doc + example of each) --"
    if [ -n "$changed" ]; then
      echo "$changed"
    else
      echo "    (none — no public signature changed in this tree)"
    fi
  fi
done

echo
echo "Next: work the protocol — doc/claude/LIBRARY_DOC_REVIEW.md — then bump the watermark."
