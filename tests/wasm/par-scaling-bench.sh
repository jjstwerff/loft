#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# @PLN117 E2 — par() scaling gate.  A CPU-heavy par() must speed up with the
# wasm Web Worker pool size: par-time (parse overhead subtracted) decreases
# monotonically 1→2→4 workers and reaches >=2.5x at 4 workers, every run agrees
# on the value, and distinct-worker count tracks the pool size.  Prints the full
# curve (incl. 8 workers + the sequential fallback) for the record.
# Exit 0 pass / 1 fail.
set -u
cd "$(dirname "$0")/../.."
PORT="${PORT:-8768}"; ROOT=tests/wasm; REPORT="$(mktemp)"
WORK="${WORK:-200000}"; ELEMS="${ELEMS:-48}"
CHROME="$(command -v chromium || command -v chromium-browser || command -v google-chrome || echo chromium)"
[ -f "$ROOT/pkg-mt/loft.js" ] || { echo "SKIP: no threaded bundle — run 'make wasm-mt'"; exit 0; }
python3 "$ROOT/coi-server.py" "$PORT" "$ROOT" "$REPORT" >/dev/null 2>&1 &
SRV=$!; trap 'kill $SRV 2>/dev/null; rm -f "$REPORT"' EXIT; sleep 1
runP() { : > "$REPORT"; timeout 70 "$CHROME" --headless=new --no-sandbox --disable-gpu \
  "http://127.0.0.1:$PORT/par-scaling-bench.html?threads=$1&work=$WORK&elems=$ELEMS&trials=2" >/dev/null 2>&1 & local ch=$!; sleep "${2:-15}"; kill $ch 2>/dev/null; head -1 "$REPORT"; }
declare -A PAR DW VAL
echo "== par() scaling (work=$WORK elems=$ELEMS) =="
for p in 1 2 4 8 0; do
  r="$(runP $p)"; echo "  $r"
  PAR[$p]="$(echo "$r" | grep -oP 'par_ms=\K\d+')"
  DW[$p]="$(echo "$r" | grep -oP 'dw=\K\d+')"
  VAL[$p]="$(echo "$r" | grep -oP 'value=\K\d+')"
  echo "$r" | grep -q 'success=true' || { echo "FAIL: P=$p unsuccessful"; exit 1; }
done
fail=0
# value stable across all pool sizes
for p in 2 4 8 0; do [ "${VAL[$p]}" = "${VAL[1]}" ] || { echo "FAIL: value drift P=$p (${VAL[$p]} vs ${VAL[1]})"; fail=1; }; done
# distinct workers tracks pool size (1,2,4,8)
for p in 1 2 4 8; do [ "${DW[$p]:-0}" -ge "$p" ] 2>/dev/null || { echo "FAIL: P=$p ran on ${DW[$p]:-?} workers"; fail=1; }; done
# monotonic 1>2>4 and >=2.5x at 4 workers
[ "${PAR[2]}" -lt "${PAR[1]}" ] 2>/dev/null || { echo "FAIL: no speedup 1->2 (${PAR[1]}->${PAR[2]})"; fail=1; }
[ "${PAR[4]}" -lt "${PAR[2]}" ] 2>/dev/null || { echo "FAIL: no speedup 2->4 (${PAR[2]}->${PAR[4]})"; fail=1; }
s4=$(( PAR[1] * 100 / PAR[4] ))
[ "$s4" -ge 250 ] || { echo "FAIL: 4-worker speedup ${s4}% < 250%"; fail=1; }
s8=$(( PAR[1] * 100 / PAR[8] ))
if [ $fail -eq 0 ]; then
  echo "PASS: par() scales — 1w=${PAR[1]}ms 2w=${PAR[2]}ms 4w=${PAR[4]}ms(${s4}%) 8w=${PAR[8]}ms(${s8}%); value=${VAL[1]} stable; fallback(P0)=${PAR[0]}ms~=1w"
fi
exit $fail
