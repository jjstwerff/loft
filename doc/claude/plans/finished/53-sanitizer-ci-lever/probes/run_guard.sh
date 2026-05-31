#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# PLAN53 cluster-2 GUARD-detector runner — for sub-clusters whose bug produces
# CORRECT output but a misaligned ACCESS (e.g. 2j: the par-worker entry base is
# self-consistent at 4, so results are right, but every access is 4-off an
# 8-boundary).  Such bugs are invisible to run.sh (functional PASS/FAIL); the
# detector is the `stack_align_guard` binary, which asserts at the access site.
#
# This is the homegrown analogue of the Miri lane the plan installs: a probe
# "passes" iff it produces ZERO guard diagnostics under LOFT_ALIGN.
#
# For each probe it reports two columns:
#   GUARD    — guard binary under LOFT_ALIGN: SILENT (good) | FIRES (bug present)
#   NORMAL   — normal binary: aligned/flagoff functional result (must be PASS/PASS;
#              2j-class bugs are NOT functional, so this stays green throughout)
#
# INVARIANT: every probe PASS/PASS under NORMAL; every `*-ref*` SILENT under GUARD.
# A reproducer is FIRES under GUARD until its fix lands, then flips to SILENT.
#
# Usage:  ./run_guard.sh 2j         # guard table for the 2j probes
#         ./run_guard.sh 2j -v      # verbose: print the guard panic line
set -u
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../../../.." && pwd)"
BIN="$REPO_ROOT/target/release/loft"
GBIN="$REPO_ROOT/target/release/loft-guard"

FILTER="${1:-}"; [ "$FILTER" = "-v" ] && FILTER=""
VERBOSE=0; for a in "$@"; do [ "$a" = "-v" ] && VERBOSE=1; done

# Build a SEPARATE guard binary so run.sh's normal binary isn't clobbered.
echo "building guard binary (target/release/loft --features stack_align_guard → loft-guard)..."
( cd "$REPO_ROOT" && cargo build --release --bin loft --features stack_align_guard >/dev/null 2>&1 ) || {
  echo "guard build failed"; exit 2; }
cp "$BIN" "$GBIN"
# Restore the plain binary so the two lanes don't fight over target/release/loft.
( cd "$REPO_ROOT" && cargo build --release --bin loft >/dev/null 2>&1 ) || { echo "plain build failed"; exit 2; }
[ -x "$BIN" ] && [ -x "$GBIN" ] || { echo "missing binaries"; exit 2; }

guard_run() { # $1 probe → SILENT | FIRES
  local out
  out=$(LOFT_ALIGN=1 LOFT_SLOT_V2=drive LOFT_TIMEOUT=6 "$GBIN" --interpret "$1" 2>&1)
  LAST_GUARD="$out"
  if echo "$out" | grep -qiE "unaligned stack access|alignment broken|panicked at .*mod\.rs:1[0-9]{3}"; then
    echo FIRES
  elif echo "$out" | grep -q PASSED; then echo SILENT
  else echo "FIRES"; fi   # any other panic counts as not-clean
}
norm_run() { # $1 probe $2 env → PASS|FAIL|HANG|CRASH
  local out rc
  out=$(env $2 LOFT_TIMEOUT=6 "$BIN" --interpret "$1" 2>&1); rc=$?
  if   [ $rc -eq 124 ]; then echo HANG
  elif [ $rc -ge 128 ]; then echo "CRASH$((rc-128))"
  elif echo "$out" | grep -qi "assertion failed\|^error:"; then echo FAIL
  elif echo "$out" | grep -q PASSED; then echo PASS
  else echo "?$rc"; fi
}

fail=0
printf '%-34s %-8s %-9s %-9s\n' PROBE GUARD NORM-align NORM-off
for f in "$SCRIPT_DIR"/${FILTER}*.loft; do
  [ -e "$f" ] || continue
  b=$(basename "$f")
  g=$(guard_run "$f")
  na=$(norm_run "$f" "LOFT_ALIGN=1 LOFT_SLOT_V2=drive")
  no=$(norm_run "$f" "")
  printf '%-34s %-8s %-9s %-9s\n' "$b" "$g" "$na" "$no"
  [ "$no" != PASS ] && { echo "  !! INVARIANT: $b not functional flag-OFF"; fail=1; }
  [ "$na" != PASS ] && { echo "  !! INVARIANT: $b not functional aligned (2j is not a functional bug)"; fail=1; }
  case "$b" in *-ref*) [ "$g" != SILENT ] && { echo "  !! REFERENCE fires guard: $b"; fail=1; };; esac
  [ $VERBOSE -eq 1 ] && [ "$g" = FIRES ] && echo "$LAST_GUARD" | grep -iE "panicked|unaligned|broken" | head -2 | sed 's/^/    | /'
done
exit $fail
