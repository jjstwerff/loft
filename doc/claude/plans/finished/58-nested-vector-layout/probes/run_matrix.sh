#!/usr/bin/env bash
# @PLAN58 probe-matrix runner.  Runs every probe across both backends ±--vec4.
# Usage: ./run_matrix.sh [release|debug]   (default: release)
# Classifies each cell: PASS / SIGSEGV / TIMEOUT / FAIL(rc).
set -u
cd "$(dirname "$0")"
PROFILE="${1:-release}"
BIN="../../../../../target/${PROFILE}/loft"
[ -x "$BIN" ] || { echo "missing $BIN — run: cargo build --${PROFILE}"; exit 2; }

classify() { # $1=rc  $2=output
  case "$1" in
    0)   echo "$2" | grep -q PASSED && echo "PASS" || echo "OK?(rc0,noPASS)" ;;
    124|143) echo "TIMEOUT" ;;
    139) echo "SIGSEGV" ;;
    *)   if echo "$2" | grep -q "error:"; then echo "FAIL-COMPILE"; else echo "FAIL($1)"; fi ;;
  esac
}

run1() { # $1=probe $2="extra flags" $3=timeout
  local out rc
  out=$(timeout "$3" "$BIN" $2 "$1" 2>&1); rc=$?
  classify "$rc" "$out"
}

printf "%-32s | %-12s | %-12s | %-12s | %-12s\n" "probe" "interp" "interp+vec4" "native" "native+vec4"
printf -- "---------------------------------+--------------+--------------+--------------+-------------\n"
for p in $(ls -1 *.loft | sort); do
  i0=$(run1 "$p" "--interpret" 20)
  i1=$(run1 "$p" "--interpret --vec4" 20)
  n0=$(run1 "$p" "--native" 90)
  n1=$(run1 "$p" "--native --vec4" 90)
  printf "%-32s | %-12s | %-12s | %-12s | %-12s\n" "${p%.loft}" "$i0" "$i1" "$n0" "$n1"
done
