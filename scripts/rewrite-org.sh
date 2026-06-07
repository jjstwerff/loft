#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# One-time org-migration helper for moving loft into the loft-lang org.
# See doc/claude/MOVING.md for the full runbook.
#
# This is NOT a blanket s/jjstwerff/loft-lang/.  Three reasons:
#   1. the registry repo is RENAMED  (loft-registry -> registry),
#   2. some jjstwerff repos are NOT (necessarily) moving (dryopea, eagleviewer,
#      and the per-package library repos pending a canonical-naming decision),
#   3. `jjstwerff/loft-<suffix>` is a DIFFERENT repo than `jjstwerff/loft`.
# So the safe rewrites are explicit + ordered (renamed/specific first; bare
# `jjstwerff/loft` last and delimiter-guarded so a trailing `-` never matches),
# and everything that needs a human decision is REPORTED, not guessed.
#
# Idempotent.  Run from a repo root (works in loft AND in any chunk repo /
# consumer for the cross-repo sweep).
#
#   scripts/rewrite-org.sh --check   # preview; change nothing
#   scripts/rewrite-org.sh           # apply safe rewrites, then report the rest

set -euo pipefail

CHECK=0
[[ "${1-}" == "--check" ]] && CHECK=1

# Ordered sed -E substitutions.  Renamed/specific first; bare-loft last with a
# guard: `([^-A-Za-z0-9]|$)` after `loft` matches a delimiter (/ . space ) " etc.)
# or end-of-line, so `jjstwerff/loft-registry` / `-graphics` are never touched.
transform() {
  sed -E \
    -e 's#jjstwerff/loft-registry#loft-lang/registry#g' \
    -e 's#jjstwerff\.github\.io/loft#loft-lang.github.io/loft#g' \
    -e 's#jjstwerff/loft([^-A-Za-z0-9]|$)#loft-lang/loft\1#g'
}

# Tracked text files, excluding build output and the two files that
# INTENTIONALLY contain the old refs (this script + the migration doc).
mapfile -t FILES < <(
  git ls-files '*.md' '*.rs' '*.yml' '*.yaml' '*.sh' '*.toml' '*.loft' '*.json' \
    | grep -vE '^(target/|scripts/rewrite-org\.sh$|doc/claude/MOVING\.md$)'
)
[[ ${#FILES[@]} -gt 0 ]] || { echo "no tracked text files"; exit 0; }

# Refs that need a HUMAN decision: per-package library repos (canonical-naming
# decision — see MOVING.md) and consumer repos that may or may not be moving.
DECIDE='jjstwerff/(loft-(graphics|shapes|server|web|game-protocol|game-client|libs-[a-z]+)|dryopea|Dryopea|eagleviewer)'

changed=0
final=""   # accumulates the would-be-final content for accurate reporting
for f in "${FILES[@]}"; do
  rewritten="$(transform < "$f")"
  if [[ "$rewritten" != "$(cat "$f")" ]]; then
    changed=1
    [[ $CHECK == 1 ]] && echo "would change: $f"
    [[ $CHECK == 0 ]] && printf '%s\n' "$rewritten" > "$f"
  fi
  # Report against the post-rewrite content (prefix each line with the path).
  final+=$(printf '%s\n' "$rewritten" | sed "s#^#$f:#")$'\n'
done
[[ $changed == 0 ]] && echo "no safe rewrites needed (already migrated?)"

echo
echo "=== refs needing a decision (library-package + consumer repos — see MOVING.md) ==="
printf '%s' "$final" | grep -nE "$DECIDE" | sed 's/^[0-9]*://' || echo "  (none)"

echo
echo "=== any OTHER remaining jjstwerff (should be empty once decisions are applied) ==="
printf '%s' "$final" | grep "jjstwerff" | grep -vE "$DECIDE" || echo "  (clean)"
