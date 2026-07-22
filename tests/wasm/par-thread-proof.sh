#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN117 E3 — headless in-browser gate: prove a loft `par` dispatches across
# real Web Worker threads, and that skipping the pool falls back to sequential
# without crashing (arc D).  Requires: a threaded bundle (make wasm-mt),
# python3, and a headless-capable chromium/chrome.  Exits 0 on pass, 1 on fail.
#
#   tests/wasm/par-thread-proof.sh
set -u
cd "$(dirname "$0")/../.."
PORT="${PORT:-8788}"
ROOT=tests/wasm
REPORT="$(mktemp)"
CHROME="$(command -v chromium || command -v chromium-browser || command -v google-chrome || echo chromium)"
if [ ! -f "$ROOT/pkg-mt/loft.js" ]; then
  echo "SKIP: no threaded bundle — run 'make wasm-mt' first"; exit 0
fi
python3 "$ROOT/coi-server.py" "$PORT" "$ROOT" "$REPORT" >/dev/null 2>&1 &
SRV=$!
trap 'kill $SRV 2>/dev/null; rm -f "$REPORT"' EXIT
sleep 1
run() {  # $1 = query, $2 = seconds to wait
  : > "$REPORT"
  timeout 45 "$CHROME" --headless=new --no-sandbox --disable-gpu \
    "http://127.0.0.1:$PORT/par-thread-proof.html$1" >/dev/null 2>&1 &
  local ch=$!; sleep "$2"; kill $ch 2>/dev/null
  head -1 "$REPORT"
}
echo "== parallel (initThreadPool 4) =="
P="$(run '?threads=4' 18)"; echo "  $P"
echo "== sequential fallback (no initThreadPool) =="
S="$(run '?threads=0' 16)"; echo "  $S"

fail=0
dw="$(echo "$P" | grep -oP 'distinct_workers=\K\d+')"
echo "$P" | grep -q 'success=true' || { echo "FAIL: parallel run not successful"; fail=1; }
[ "${dw:-0}" -ge 2 ] 2>/dev/null || { echo "FAIL: expected distinct_workers>=2, got '${dw:-none}'"; fail=1; }
echo "$S" | grep -q 'success=true' || { echo "FAIL: sequential fallback crashed"; fail=1; }
echo "$S" | grep -q 'distinct_workers=1' || echo "WARN: fallback distinct_workers != 1 ($S)"
# value gate: both runs must agree on par_sum
pp="$(echo "$P" | grep -oP 'par_sum=\K\d+')"; ss="$(echo "$S" | grep -oP 'par_sum=\K\d+')"
[ -n "$pp" ] && [ "$pp" = "$ss" ] || { echo "FAIL: par_sum mismatch parallel=$pp sequential=$ss"; fail=1; }
if [ $fail -eq 0 ]; then echo "PASS: par dispatched across $dw workers; fallback correct; par_sum=$pp"; fi
exit $fail
