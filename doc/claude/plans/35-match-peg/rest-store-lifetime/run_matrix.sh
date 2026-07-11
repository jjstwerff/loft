#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Run every ..rest store-lifetime probe on BOTH backends and print the ground-truth
# matrix: value-correct? leak? — vs the hypothesised @LEAK in each probe header.
# Leak ground truth = the interpreter store-leak check (LOFT_STORES=warn "not freed")
# and native LOFT_NATIVE_LEAK_CHECK.
set -u
cd "$(dirname "$0")"
ROOT=$(cd ../../../../.. && pwd)
L="$ROOT/target/debug/loft"
PROBES=probes
printf "%-32s | %-4s %-4s | %-4s %-4s | hypo | verdict\n" "probe" "iV" "iL" "nV" "nL" ""
printf -- "---------------------------------|-----------|-----------|------|--------\n"
mism=0
for f in "$PROBES"/*.loft; do
  name=$(basename "$f" .loft)
  hypo=$(grep -oE '@LEAK (ok|leak)' "$f" | awk '{print $2}')
  # interpret: value (PASS printed?) + leak (no "not freed"?)
  iout=$(LOFT_STORES=warn "$L" --interpret --timeout 30 "$f" 2>&1)
  iV=$([ "$(echo "$iout" | grep -c '^PASS$')" -gt 0 ] && echo ok || echo BAD)
  iL=$([ "$(echo "$iout" | grep -ci 'not freed')" -gt 0 ] && echo leak || echo ok)
  # native: value + leak
  nout=$(LOFT_NATIVE_LEAK_CHECK=1 LOFT_TIMEOUT=120 timeout 140 "$L" --native "$f" 2>&1)
  if echo "$nout" | grep -qiE 'error\[|compilation failed'; then nV=ERR; else
    nV=$([ "$(echo "$nout" | grep -c '^PASS$')" -gt 0 ] && echo ok || echo BAD); fi
  nL=$([ "$(echo "$nout" | grep -ci 'not freed\|leak')" -gt 0 ] && echo leak || echo ok)
  # verdict: does actual interp leak match the hypothesis? and is the value correct?
  verdict=OK
  [ "$iL" != "$hypo" ] && verdict="LEAK≠hypo" && mism=$((mism+1))
  [ "$iV" != "ok" ] && verdict="VALUE-BAD" && mism=$((mism+1))
  [ "$nV" = "BAD" ] && verdict="NAT-VALUE-BAD" && mism=$((mism+1))
  printf "%-32s | %-4s %-4s | %-4s %-4s | %-4s | %s\n" "$name" "$iV" "$iL" "$nV" "$nL" "$hypo" "$verdict"
done
printf -- "---------------------------------|-----------|-----------|------|--------\n"
echo "mismatches (actual≠hypothesis, or wrong value): $mism"
echo "iV/nV = interpret/native VALUE (ok=PASS printed, BAD=wrong/no PASS, ERR=native compile fail)"
echo "iL/nL = interpret/native LEAK (leak=store not freed at exit)"
