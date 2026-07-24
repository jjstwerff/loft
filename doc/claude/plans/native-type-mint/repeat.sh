#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Repeat-run harness for the native duplicate-type-mint fault.
#
# The fault is PER-PROCESS RANDOM: the same binary on the same file emits a
# correct program in some runs and a broken one in others.  A single run is
# therefore worthless as evidence — it reports whichever variant it landed on.
# Every reading below is N runs, and the verdict is the COUNT.
#
# Three ways a single run can lie, all handled here:
#   * a lucky variant           -> N runs, report ok/bad as a ratio
#   * a stale compiled binary   -> the per-directory `.loft` cache is removed
#                                  before EVERY run, so each one recompiles
#   * a run that produced nothing (compile failure, timeout) -> counted as
#     VACUOUS, never as a pass
#
# `zz_control` must ALWAYS report bad: it proves the harness can see a failure.
#
# Usage:  repeat.sh [N] [loft-binary]
set -uo pipefail
N="${1:-20}"
LOFT="${2:-/home/jurjens/workspace/loft/target/debug/loft}"
HERE="$(cd "$(dirname "$0")" && pwd)"

run_probe() {           # $1 = probe file, $2 = expected stdout line
    local probe="$1" want="$2" ok=0 bad=0 vac=0
    local dir; dir="$(dirname "$probe")"
    for _ in $(seq 1 "$N"); do
        rm -rf "$dir/.loft"
        local out; out=$(LOFT_TIMEOUT=180 timeout 300 "$LOFT" --native "$probe" 2>&1)
        local got; got=$(echo "$out" | grep -m1 -F "$want")
        if [ -z "$(echo "$out" | tr -d '[:space:]')" ]; then
            vac=$((vac+1))
        elif [ -n "$got" ]; then
            ok=$((ok+1))
        else
            bad=$((bad+1))
        fi
    done
    printf "%-26s ok=%-4s bad=%-4s vacuous=%-4s" "$(basename "$probe" .loft)" "$ok" "$bad" "$vac"
    if [ "$(basename "$probe" .loft)" = "zz_control" ]; then
        [ "$ok" = "0" ] && echo "  (control: correctly never ok)" || echo "  ** CONTROL PASSED — HARNESS IS BLIND **"
    else
        [ "$bad" = "0" ] && [ "$vac" = "0" ] && echo "  GREEN" || echo "  <-- FAULT"
    fi
}

echo "N=$N runs per probe, binary: $LOFT"
printf "%-26s %s\n" "PROBE" "VERDICT"
printf "%-26s %s\n" "-----" "-------"
for p in "$HERE"/probes/*.loft; do
    [ -f "$p" ] || continue
    want=$(grep -m1 '^//! expect:' "$p" | sed 's|^//! expect: ||')
    run_probe "$p" "$want"
done
