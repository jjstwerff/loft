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
. tests/wasm/gate-lib.sh
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
run() {  # $1 = query, $2 = seconds to wait for the page's line
  : > "$REPORT"
  timeout $(( 45 * ${WAIT_SCALE:-1} )) "$CHROME" --headless=new --no-sandbox --disable-gpu \
    "http://127.0.0.1:$PORT/par-thread-proof.html$1" >/dev/null 2>&1 &
  local ch=$!; await_report "$REPORT" "$2"; stop_browser $ch
  head -1 "$REPORT"
}
# The FIRST page of the FIRST gate in a job pays a one-time cost the rest never
# see: chromium's cold profile plus compiling the threaded wasm bundle.  Measured
# on a 4-vCPU CI runner, four WARM runs of the sibling gate cost 11s total while
# this cold one blew a 54s budget and reported nothing — read as "the page
# produced nothing", i.e. a red gate for a working build.  Padding every budget
# would hide a page that really hangs, so pay the cost ONCE here instead, in a
# discarded run with a cap of its own.  The elapsed time is printed rather than
# swallowed: a warm-up that takes minutes is itself a finding.
echo "== warm-up (cold chromium + wasm compile; result discarded) =="
_w0=$SECONDS
run '?threads=4' 90 >/dev/null
echo "  warm-up took $(( SECONDS - _w0 ))s"

echo "== parallel (initThreadPool 4) =="
P="$(run '?threads=4' 18)"; echo "  $P"
echo "== sequential fallback (no initThreadPool) =="
S="$(run '?threads=0' 16)"; echo "  $S"

echo "== nested par (a par worker that itself runs a par) =="
N="$(run '?threads=4&nested=1' 16)"; echo "  $N"

fail=0
dw="$(echo "$P" | grep -oP 'distinct_workers=\K\d+')"
echo "$P" | grep -q 'success=true' || { echo "FAIL: parallel run not successful"; fail=1; }
[ "${dw:-0}" -ge 2 ] 2>/dev/null || { echo "FAIL: expected distinct_workers>=2, got '${dw:-none}'"; fail=1; }
echo "$S" | grep -q 'success=true' || { echo "FAIL: sequential fallback crashed"; fail=1; }
echo "$S" | grep -q 'distinct_workers=1' || echo "WARN: fallback distinct_workers != 1 ($S)"
# value gate: both runs must agree on par_sum
pp="$(echo "$P" | grep -oP 'par_sum=\K\d+')"; ss="$(echo "$S" | grep -oP 'par_sum=\K\d+')"
[ -n "$pp" ] && [ "$pp" = "$ss" ] || { echo "FAIL: par_sum mismatch parallel=$pp sequential=$ss"; fail=1; }
# Nested par: a hand-computed value, so agreeing backends cannot hide a wrong one.
echo "$N" | grep -q 'success=true' || { echo "FAIL: nested par run not successful"; fail=1; }
echo "$N" | grep -q 'par_sum=712' || { echo "FAIL: nested par expected par_sum=712, got '$N'"; fail=1; }
if [ $fail -eq 0 ]; then echo "PASS: par dispatched across $dw workers; fallback correct; nested par correct; par_sum=$pp"; fi
exit $fail
