#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN92 strand 5 — catalogue hygiene.
#
# Two invariants over the @F / @I catalogue, checked straight from the tree:
#
#   1. No DANGLING tag — every `@F<n>` / `@I<n>` written in code or docs has a
#      catalogue entry (a matching issue in index/features.json).  Catches typos
#      and stale numbers before they mislead a reader.
#
#   2. DUAL-ANCHORED — every catalogue entry appears in BOTH implementation
#      source (>=1 code anchor: src/ default/ tools/) and documentation
#      (>=1 doc anchor: doc/, incl. its generated mirror page).  A code anchor
#      says *where it lives*; a doc anchor says *what it is* — the @P### split.
#
# `--check` fails ONLY on a dangling tag (an unambiguous bug).  Missing anchors
# are REPORTED, not fatal: some are legitimate — the unauthored @F43 stub, or a
# feature whose code sits under a coarse @I subsystem rather than its own @F tag
# (see the strand-4 scope note).  The report is the cleanup worklist.
#
#   scripts/feature_hygiene.sh            # full report
#   scripts/feature_hygiene.sh --check    # exit 1 iff a dangling tag exists
#   scripts/feature_hygiene.sh -c         # counts only
set -u
cd "$(dirname "$0")/.."

SNAP="index/features.json"
CODE_ROOTS="src default tools"
DOC_ROOTS="doc"

# Valid catalogue tokens ("F17", "I62", …) from the committed snapshot.
catalogue() {
  jq -r '.[] | (if .kind=="feature" then "F" else "I" end) + (.number|tostring)' "$SNAP" | sort -u
}

# Every @F/@I tag occurrence as "TOKEN KIND" (KIND = code | doc).
occurrences() {
  grep -rEho '@[FI][0-9]+' $CODE_ROOTS --include='*.rs' --include='*.loft' \
       --exclude-dir='.loft' 2>/dev/null | sed 's/^@//; s/$/ code/'
  grep -rEho '@[FI][0-9]+' $DOC_ROOTS --include='*.md' 2>/dev/null | sed 's/^@//; s/$/ doc/'
}

OCC="$(occurrences)"
CAT="$(catalogue)"
REF_ALL="$(printf '%s\n' "$OCC" | awk 'NF{print $1}' | sort -u)"
REF_CODE="$(printf '%s\n' "$OCC" | awk '$2=="code"{print $1}' | sort -u)"
REF_DOC="$(printf '%s\n' "$OCC" | awk '$2=="doc"{print $1}' | sort -u)"

DANGLING="$(comm -23 <(printf '%s\n' "$REF_ALL") <(printf '%s\n' "$CAT") | grep . || true)"
NO_CODE="$(comm -23 <(printf '%s\n' "$CAT") <(printf '%s\n' "$REF_CODE") | grep . || true)"
NO_DOC="$(comm -23 <(printf '%s\n' "$CAT") <(printf '%s\n' "$REF_DOC") | grep . || true)"

fmt() { [ -n "$1" ] && printf '%s\n' "$1" | sed 's/^/  @/' || echo "  (none)"; }
ndang=$([ -n "$DANGLING" ] && printf '%s\n' "$DANGLING" | grep -c . || echo 0)
nnc=$([ -n "$NO_CODE" ] && printf '%s\n' "$NO_CODE" | grep -c . || echo 0)
nnd=$([ -n "$NO_DOC" ] && printf '%s\n' "$NO_DOC" | grep -c . || echo 0)

case "${1:-report}" in
--check)
  if [ -n "$DANGLING" ]; then
    echo "ERROR: dangling @F/@I tag(s) — no catalogue entry in index/features.json:"
    fmt "$DANGLING"
    echo "Fix the typo, or mint the entry + run 'make features-fetch'."
    exit 1
  fi
  echo "feature hygiene: no dangling tags ($(printf '%s\n' "$CAT" | grep -c .) entries; $nnc without a code anchor, $nnd without a doc anchor — advisory)."
  ;;
-c|--count)
  echo "entries=$(printf '%s\n' "$CAT" | grep -c .) dangling=$ndang no-code=$nnc no-doc=$nnd"
  ;;
*)
  echo "== Catalogue hygiene (@PLN92 strand 5) =="
  echo "entries: $(printf '%s\n' "$CAT" | grep -c .)"
  echo
  echo "DANGLING tags (referenced, no catalogue entry) — FATAL under --check:"
  fmt "$DANGLING"
  echo
  echo "Entries with NO code anchor (unimplemented / under a coarse @I / stub) — advisory:"
  fmt "$NO_CODE"
  echo
  echo "Entries with NO doc anchor (should be none — the mirror is a doc anchor) — advisory:"
  fmt "$NO_DOC"
  ;;
esac
