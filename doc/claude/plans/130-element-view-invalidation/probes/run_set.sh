#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN130 probe runner — the secret-copy catalogue, measured.
#
# For each probe: does it pass, how many copies does the compiler ADMIT to (the
# LOFT_COPY_MANIFEST guard: emitted copies no diagnostic accounts for), and how many
# does it actually EXECUTE (LOFT_COPY_DUMP on interp, LOFT_TRACE_COPY on native — two
# flags for one fact, see the plan README).
#
# The interesting column is `uncov`: a copy the compiler emits and no diagnostic names.
#
# Two things to know before reading the numbers:
#
#   * There is an AMBIENT baseline of 1 uncovered site in every compile that loads the
#     stdlib — `n_exists`'s `__lift_1` (probe 18).  So `uncov=1` on interpret means "this
#     probe added none of its own"; `uncov=2` means it added one.
#   * A `--native` run executes BOTH generators (bytecode first, then native source), so
#     each reports its own sites and the native column shows roughly double.  That is the
#     backends disagreeing about nothing — it is one program measured twice.
#
# `copies` is an investigation aid only: it is what the run EXECUTED, and it counts the
# flat record copy, never the deep content (`copy_claims` has no hook).  The plan's guard
# is compile-time; nothing here is a gate.
#
# Usage:  ./run_set.sh [set]
#   view    (default) probes 01-09 — the index-pinned view class
#   copy    probes 10-14 — the silent-copy inventory
#   secret  probes 15-19 — secret copies found by the guard, plus their references
#   all     everything
set -uo pipefail

cd "$(dirname "$0")"
LOFT="${LOFT:-../../../../../target/release/loft}"
[ -x "$LOFT" ] || { echo "no loft binary at $LOFT (set LOFT=...)"; exit 1; }
export LOFT_TIMEOUT="${LOFT_TIMEOUT:-90}"   # probe hangs self-terminate, not the runner

case "${1:-view}" in
  view)   FILES=(0*.loft) ;;
  copy)   FILES=(1[0-4]-*.loft) ;;
  secret) FILES=(1[5-9]-*.loft) ;;
  all)    FILES=(*.loft) ;;
  *)      echo "unknown set '$1' (view|copy|secret|all)"; exit 1 ;;
esac

printf '%-34s %-22s %-22s\n' "probe" "interpret(pass/uncov/copies)" "native(pass/uncov/copies)"
for f in "${FILES[@]}"; do
  row=""
  for mode in --interpret --native; do
    out=$("$LOFT" "$mode" "$f" 2>&1)
    # A probe FAILS by assertion (the view class) or passes; both are data, not runner errors.
    case "$out" in *PASSED*) pass=pass ;; *) pass=FAIL ;; esac
    uncov=$(LOFT_COPY_MANIFEST=1 "$LOFT" "$mode" "$f" 2>&1 | grep -cE '^  (interpret|native) ')
    if [ "$mode" = "--interpret" ]; then
      copies=$(LOFT_COPY_DUMP=1 "$LOFT" "$mode" "$f" 2>&1 | grep -c '^\[copy\] record')
    else
      copies=$(LOFT_TRACE_COPY=1 "$LOFT" "$mode" "$f" 2>&1 | grep -c 'OpCopyRecord')
    fi
    row="$row$(printf '%-22s' "$pass/$uncov/$copies")"
  done
  printf '%-34s %s\n' "${f%.loft}" "$row"
done
