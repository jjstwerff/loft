#!/usr/bin/env bash
# @PLN108 S0 — the copy-cost win baseline.  NOT a CI gate (timing is machine-dependent);
# run manually to record/compare.  Sweeps the UNRELATED live heap and the thread count for a
# fixed par workload whose workers read none of that heap.  A rising par_ms = the per-worker
# byte-copy of the parent stores.  Re-run with LOFT_PAR_SHARE=1 (S7) — the growth must vanish.
#
#   ./run.sh              # interpreter (default)
#   ./run.sh --native     # native backend (also affected; slower to compile)
#   LOFT_PAR_SHARE=1 ./run.sh   # S7 comparison (once the flag exists)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../../../.." && pwd)"
LOFT="${LOFT_BIN:-$ROOT/target/release/loft}"
PROBE="$HERE/par_copy_probe.loft"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
BACKEND="--interpret"
[[ "${1:-}" == "--native" ]] && BACKEND="--native"

echo "loft=$LOFT  backend=$BACKEND  LOFT_PAR_SHARE=${LOFT_PAR_SHARE:-<unset>}"
run() { LOFT_TIMEOUT=200 "$LOFT" $BACKEND "$1" 2>&1 | grep -E '^heap_mb=' | tail -1; }

echo "== heap sweep (threads=8, elems=64) — the worker reads NONE of this heap =="
for mb in 0 15 30 61 122; do
  nodes=$(( mb * 1048576 / 32 ))   # 32 B/node
  printf "  %4s MB : " "$mb"; PROBE_NODES=$nodes run "$PROBE"
done

echo "== thread sweep (heap=61 MB, elems=64) — more threads should NOT be slower =="
for t in 1 4 8 16; do
  # native rejects a variable thread count → bake the literal into a temp probe per value
  sed "s/par(b = spin(a), 8)/par(b = spin(a), $t)/" "$PROBE" > "$TMP/p$t.loft"
  printf "  %2s thr : " "$t"; PROBE_NODES=2000000 run "$TMP/p$t.loft"
done
echo "(par_ms rising with heap OR with threads = par is INVERTED by the per-worker copy — the @PLN108 target)"
