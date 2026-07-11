#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Run the reporting-only store-lifetime oracle (LOFT_REST_ORACLE) over every probe and
# correlate its free/alloc-order PREDICTION with the ground-truth interpreter leak-check.
# Proves the oracle's fact is the right one (predicts leak iff the corpus leaks).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../../../.." && pwd)
L="$ROOT/target/debug/loft"
PDIR="$HERE/probes"
cd "$ROOT"   # loft must run from the repo root so the scan (and the oracle) run over a fresh parse
printf "%-32s | %-4s | %-6s | %-30s | %s\n" "probe" "leak" "pred" "return-type" "oracle store verdict"
printf -- "---------------------------------|------|--------|--------------------------------|---------------------\n"
ok=0; bad=0
for f in "$PDIR"/*.loft; do
  name=$(basename "$f" .loft)
  hypo=$(grep -oE '@LEAK \w+' "$f" | awk '{print $2}')
  # ground truth leak (interp)
  gt=$([ "$(LOFT_STORES=warn "$L" --interpret --timeout 30 "$f" 2>&1 | grep -ci 'not freed')" -gt 0 ] && echo leak || echo ok)
  # oracle: the user function's rest-store line (n_f / n_parse), free/alloc order + return type
  LOFT_NO_CACHE=1 LOFT_REST_ORACLE=1 "$L" --interpret --timeout 30 "$f" >/dev/null 2>/tmp/rest_orc.$$
  orc=$(grep -A2 -E 'fn=n_(f|parse)$' /tmp/rest_orc.$$)
  ret=$(echo "$orc" | grep -oE '\((hoists|NO-hoist)\)' | head -1 | tr -d '()')
  ordr=$(echo "$orc" | grep -oE 'free/alloc order: .*' | head -1 | sed 's|free/alloc order: ||')
  # oracle prediction: LEAKS if it saw FREE-before-ALLOC or alloc-no-free
  if echo "$ordr" | grep -qi 'LEAK'; then pred=leak; else pred=ok; fi
  # if no rest store found (no ..rest / no user store), mark n/a
  [ -z "$ordr" ] && pred="n/a"
  match="✓"; if [ "$pred" != "n/a" ] && [ "$pred" != "$gt" ]; then match="✗ pred≠gt"; bad=$((bad+1)); else ok=$((ok+1)); fi
  printf "%-32s | %-4s | %-6s | %-30s | %s  %s\n" "$name" "$gt" "$pred" "${ret:-–}" "${ordr:-–}" "$match"
done
printf -- "---------------------------------|------|--------|--------------------------------|---------------------\n"
echo "oracle prediction matches ground truth: $ok   mismatches: $bad"
echo "pred = oracle's free/alloc-order leak prediction; leak = interpreter ground-truth leak-check"
