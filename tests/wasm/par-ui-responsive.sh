#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# @PLN117 E1 — off-main-thread UI-responsiveness gate.  A requestAnimationFrame
# loop runs one heavy par() per frame; with the Web Worker pool each frame
# finishes faster, so the loop delivers markedly MORE frames (and a smaller
# worst-case gap) than the sequential fallback — the qualitative "UI not blocked"
# proof, made quantitative.  Asserts threaded frames >= MIN_FRAME_RATIO_X10/10 x
# sequential and a smaller max inter-frame gap, with the value stable.
#
# Calibration (env): THREADS (default 8 workers) and MIN_FRAME_RATIO_X10 (20,
# i.e. 2.0x) describe this 24-core dev box; the CI workflow lowers both for a
# 4-vCPU runner — see .github/workflows/browser-threads.yml.
#
# UI_WORK / UI_WINDOW size the SAMPLE, and they matter more than the floor: a
# ratio needs frames to be a ratio at all.  On a slow runner the dev-box values
# yielded 4 frames against 3 — noise, not a measurement — so CI asks for lighter
# per-frame work over a longer window.  They fall back to WORK / WINDOW so
# running this gate by hand still works; the separate names keep them from
# retuning par-scaling-bench, which reads WORK too.
# Exit 0 pass / 1 fail.
set -u
cd "$(dirname "$0")/../.."
. tests/wasm/gate-lib.sh
PORT="${PORT:-8764}"; ROOT=tests/wasm; REPORT="$(mktemp)"
WORK="${UI_WORK:-${WORK:-40000}}"; ELEMS="${ELEMS:-48}"; WINDOW="${UI_WINDOW:-${WINDOW:-3000}}"
THREADS="${THREADS:-8}"; MIN_FRAME_RATIO_X10="${MIN_FRAME_RATIO_X10:-20}"
CHROME="$(command -v chromium || command -v chromium-browser || command -v google-chrome || echo chromium)"
[ -f "$ROOT/pkg-mt/loft.js" ] || { echo "SKIP: no threaded bundle — run 'make wasm-mt'"; exit 0; }
python3 "$ROOT/coi-server.py" "$PORT" "$ROOT" "$REPORT" >/dev/null 2>&1 &
SRV=$!; trap 'kill $SRV 2>/dev/null; rm -f "$REPORT"' EXIT; sleep 1
run() { : > "$REPORT"; timeout $(( 40 * ${WAIT_SCALE:-1} )) "$CHROME" --headless=new --no-sandbox --disable-gpu \
  "http://127.0.0.1:$PORT/par-ui-responsive.html?threads=$1&work=$WORK&elems=$ELEMS&window=$WINDOW" >/dev/null 2>&1 & local ch=$!; await_report "$REPORT" $(( WINDOW/1000 + 6 )); stop_browser $ch; head -1 "$REPORT"; }
echo "== UI responsiveness (heavy par per rAF frame, window=${WINDOW}ms, threads=$THREADS) =="
TH="$(run "$THREADS")"; echo "  threaded:   $TH"
SQ="$(run 0)"; echo "  sequential: $SQ"
fail=0
echo "$TH" | grep -q '^E1 ' && echo "$SQ" | grep -q '^E1 ' || { echo "FAIL: a run did not report"; exit 1; }
tf=$(echo "$TH" | grep -oP 'frames=\K\d+'); sf=$(echo "$SQ" | grep -oP 'frames=\K\d+')
tg=$(echo "$TH" | grep -oP 'max_gap_ms=\K\d+'); sg=$(echo "$SQ" | grep -oP 'max_gap_ms=\K\d+')
tv=$(echo "$TH" | grep -oP 'value=\K\d+'); sv=$(echo "$SQ" | grep -oP 'value=\K\d+')
[ "$tv" = "$sv" ] || { echo "FAIL: value drift threaded=$tv sequential=$sv"; fail=1; }
[ $(( ${tf:-0} * 10 )) -ge $(( sf * MIN_FRAME_RATIO_X10 )) ] 2>/dev/null \
  || { echo "FAIL: threaded frames ($tf) < ${MIN_FRAME_RATIO_X10}/10 x sequential ($sf)"; fail=1; }
[ "${tg:-999999}" -lt "${sg:-0}" ] 2>/dev/null || echo "WARN: threaded max_gap ($tg) not < sequential ($sg)"
[ $fail -eq 0 ] && echo "PASS: threading keeps the UI responsive — ${tf} frames vs ${sf} sequential ($(( tf * 10 / (sf>0?sf:1) ))x/10), worst gap ${tg}ms vs ${sg}ms; value=$tv"
exit $fail
