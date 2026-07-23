#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# @PLN117 arc C — memory-model gate.  Allocation-heavy `par` (struct+text AND
# vector return shapes) under shared linear memory must agree with the native
# reference on EVERY rep, both parallel and sequential, with proven real
# concurrency.  A shared-memory race / lifetime / adoption fault fails it.
# Exit 0 pass / 1 fail.
set -u
cd "$(dirname "$0")/../.."
. tests/wasm/gate-lib.sh
PORT="${PORT:-8785}"; ROOT=tests/wasm; REPORT="$(mktemp)"
CHROME="$(command -v chromium || command -v chromium-browser || command -v google-chrome || echo chromium)"
[ -f "$ROOT/pkg-mt/loft.js" ] || { echo "SKIP: no threaded bundle — run 'make wasm-mt'"; exit 0; }
python3 "$ROOT/coi-server.py" "$PORT" "$ROOT" "$REPORT" >/dev/null 2>&1 &
SRV=$!; trap 'kill $SRV 2>/dev/null; rm -f "$REPORT"' EXIT; sleep 1
run() { : > "$REPORT"; timeout $(( 70 * ${WAIT_SCALE:-1} )) "$CHROME" --headless=new --no-sandbox --disable-gpu \
  "http://127.0.0.1:$PORT/par-memory-proof.html$1" >/dev/null 2>&1 & local ch=$!; await_report "$REPORT" "$2"; stop_browser $ch; head -1 "$REPORT"; }
fail=0
# case → native reference value (native --interpret == --native)
check() {  # $1=label $2=query $3=wait $4=expected-ref $5=need-concurrency(1/0)
  local r; r="$(run "$2" "$3")"; echo "  [$1] $r"
  echo "$r" | grep -q "success=true"      || { echo "  FAIL($1): unsuccessful/crash"; fail=1; }
  echo "$r" | grep -q "distinct=$4 "      || { echo "  FAIL($1): values other than native ref $4"; fail=1; }
  if [ "$5" = 1 ]; then local mw; mw="$(echo "$r" | grep -oP 'min_workers=\K\d+')"
    [ "${mw:-0}" -ge 2 ] 2>/dev/null      || { echo "  FAIL($1): not concurrent (min_workers=${mw:-?})"; fail=1; }; fi
}
echo "== struct+text return =="
check "struct/par" '?case=struct&threads=4&reps=12' 34 '5559680/3328' 1
check "struct/seq" '?case=struct&threads=0&reps=3'  18 '5559680/3328' 0
echo "== vector return =="
check "vector/par" '?case=vector&threads=4&reps=12' 30 '577100/1600' 1
check "vector/seq" '?case=vector&threads=0&reps=3'  16 '577100/1600' 0
[ $fail -eq 0 ] && echo "PASS: shared-memory par == native ref for struct+text AND vector returns, parallel (concurrent) + sequential"
exit $fail
