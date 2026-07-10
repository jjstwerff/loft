#!/usr/bin/env bash
# @PLN85 generic-tuple-return leak+correctness matrix (p329/p330 class).
# Per probe: interp exit, native exit, and ASan Direct-leak-root count (ir_read
# suppressed). Expect all exit=0 and leak=0 once the fix is complete.
#   BIN=<release loft>  ABIN=<ASan loft>  ./run.sh
set -u
DIR=$(cd "$(dirname "$0")" && pwd)
BIN=${BIN:?set BIN to a release loft}
ABIN=${ABIN:?set ABIN to an ASan-instrumented loft}
# Repo root via git (a hand-counted ../ chain silently resolved to a nonexistent
# path → LSan errored → every probe reported a FALSE leak=0; see the design-protocol
# "prove the harness can fail").
ROOT=$(git -C "$DIR" rev-parse --show-toplevel)
SUPP=${SUPP:-$ROOT/.github/lsan_suppressions.txt}
[ -r "$SUPP" ] || { echo "FATAL: suppressions file not readable: $SUPP" >&2; exit 2; }
# Harness liveness proof: the intentional ir_read `Box::leak` ALWAYS fires under a
# live ASan binary WITHOUT the suppression. Reuse a real probe (avoids temp-file /
# stdlib-path pitfalls). If even this shows no LSan SUMMARY, the oracle is blind
# (wrong ABIN / detect_leaks off) and every 0 below is vacuous.
_probe0=$(ls "$DIR"/*.loft | head -1)
# The ASan binary's FIRST invocation in a fresh process occasionally emits no LSan
# report (a startup race); retry a few times before declaring the oracle blind.
raw=0
for _try in 1 2 3; do
  raw=$(ASAN_OPTIONS=detect_leaks=1 "$ABIN" --interpret "$_probe0" 2>&1 \
    | grep -c 'SUMMARY: AddressSanitizer.*leaked')
  [ "$raw" -ge 1 ] && break
done
[ "$raw" -ge 1 ] || { echo "FATAL: ASan leak oracle is blind (no LSan SUMMARY after retries) — wrong ABIN / detect_leaks off?" >&2; exit 3; }
printf 'harness liveness: oracle-live=yes (unsuppressed ir_read baseline leaks; suppressed below)\n'
printf '%-32s %6s %6s %6s\n' probe interp native leak
for f in "$DIR"/*.loft; do
  b=$(basename "$f" .loft)
  "$BIN" --interpret "$f" >/dev/null 2>&1; ie=$?
  LOFT_TIMEOUT=90 "$BIN" --native "$f" >/dev/null 2>&1; ne=$?
  lk=$(ASAN_OPTIONS=detect_leaks=1 LSAN_OPTIONS="suppressions=$SUPP" \
    "$ABIN" --interpret "$f" 2>&1 | grep -c '^Direct leak')
  printf '%-32s %6s %6s %6s\n' "$b" "$ie" "$ne" "$lk"
done
