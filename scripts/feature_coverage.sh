#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN92 strand 4 — source-coverage gate (the catalogue keystone).
#
# Every substantial implementation file must be attributed to a catalogue
# entry: an `@I<n>` subsystem tag (coarse — whole file) or ≥1 `@F<n>` feature
# tag (per-capability).  A file carrying NEITHER is UNCATALOGUED — its code
# escapes the feature/infra catalogue entirely, which is the drift @PLN92
# exists to stop (a real feature landing uncatalogued, like named-args once did).
#
# Baseline ratchet — same shape as scripts/lint_comments.sh, so a legacy tree
# adopts it without a big-bang cleanup:
#   T0   scripts/feature_coverage.sh --baseline  # accept today's uncataloged
#                                                 # files into the baseline
#   CI   scripts/feature_coverage.sh --check      # fail on any NEW uncataloged
#                                                 # file (not in the baseline)
#   fix  <tag the file with @I/@F> then
#        scripts/feature_coverage.sh --prune       # drop now-tagged files from
#                                                 # the baseline (ratchet down)
# The baseline is keyed by file path; its shrinking size is the cleanup
# timeline.  A NEW source file with real code MUST claim a catalogue entry to
# stay green — that is the "code gets catalogued as it is written" enforcement.
#
# Granularity: `@I` is coarse (a file-level subsystem tag covers the whole
# file); `@F` is per-capability.  This gate is therefore file-level — it proves
# every file maps to SOME catalogue entry.  It does NOT force a fresh `@F` on a
# new function inside an already-tagged subsystem file (no line-scanner can tell
# a user feature from an internal helper) — that stays a review judgement.
#
# Report modes:
#   scripts/feature_coverage.sh            # list uncataloged files
#   scripts/feature_coverage.sh -c         # counts only (the thermometer)
#   scripts/feature_coverage.sh top [N]    # biggest uncataloged files by lines
set -u
cd "$(dirname "$0")/.."

BASELINE=".feature_coverage_baseline"
MIN_LINES=25   # files below this many non-blank lines are trivial glue — dodge.

# Implementation source worth cataloguing: the language itself — Rust under
# src/, the loft stdlib (default/), and the loft INFRA tools (indexer + feature
# sync).  Consumer/demo loft programs (games, editors, viewer) are NOT the
# language and are out of scope.  Excludes per-source `.loft/` compile caches.
list_files() {
  { find src -type f -name '*.rs'
    find default -type f -name '*.loft'
    find tools/indexer tools/features -type f -name '*.loft' -not -path '*/.loft/*'
  } 2>/dev/null | sort -u
}

# A file is ATTRIBUTED if it carries any @F<n> or @I<n> tag.  Emit every
# UNATTRIBUTED file that clears the MIN_LINES triviality floor.
collect() {
  list_files | while read -r f; do
    [ -f "$f" ] || continue
    grep -qE '@[FI][0-9]' "$f" && continue            # catalogued
    nb=$(grep -cvE '^[[:space:]]*$' "$f")
    [ "$nb" -lt "$MIN_LINES" ] && continue            # trivial — dodge
    echo "$f"
  done | sort -u
}

cmd="${1:-report}"
case "$cmd" in
--baseline)
  collect > "$BASELINE"
  echo "Wrote $(wc -l < "$BASELINE") uncataloged files to $BASELINE"
  echo "(these are now grandfathered; --check reports only NEW ones)"
  ;;
--check)
  new=$(comm -23 <(collect) <(sort -u "$BASELINE" 2>/dev/null))
  if [ -n "$new" ]; then
    echo "ERROR: NEW uncataloged source file(s) — tag each with an @I subsystem"
    echo "or an @F feature (see doc/claude/plans/92-feature-catalogue/), then"
    echo "run: scripts/feature_coverage.sh --prune"
    echo "$new" | sed 's/^/  /'
    exit 1
  fi
  echo "feature coverage: no new uncataloged files (baseline $(wc -l < "$BASELINE"))."
  ;;
--prune)
  # Keep only baseline entries that are STILL uncataloged (drop tagged ones).
  comm -12 <(collect) <(sort -u "$BASELINE") > "$BASELINE.tmp"
  mv "$BASELINE.tmp" "$BASELINE"
  echo "Pruned; $(wc -l < "$BASELINE") uncataloged files remain."
  ;;
-c|--count)
  echo "$(collect | wc -l) uncataloged / $(list_files | wc -l) scanned files"
  ;;
top)
  n="${2:-20}"
  collect | while read -r f; do
    printf '%6d  %s\n' "$(grep -cvE '^[[:space:]]*$' "$f")" "$f"
  done | sort -rn | head -"$n"
  ;;
*)
  echo "Uncataloged source files ($(collect | wc -l)):"
  collect | sed 's/^/  /'
  ;;
esac
