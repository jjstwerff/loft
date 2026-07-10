#!/usr/bin/env bash
# @PLN85 — the leakers the enforcing nightly leak gate uncovered across the BROADER
# corpus (tests/scripts + libraries), beyond the issues suite that was driven to 0.
# Runs the per-file leak scan over the tracked list; each remaining leaker is a probe
# to drive to 0 (same discipline as the tuple / forward-ref classes).
#   ABIN=<asan loft>  ./run.sh
set -u
DIR=$(cd "$(dirname "$0")" && pwd)
ROOT=$(git -C "$DIR" rev-parse --show-toplevel)
export LSAN_OPTIONS="suppressions=$ROOT/.github/lsan_suppressions.txt"
cd "$ROOT"
ABIN=${ABIN:?set ABIN to an ASan loft} bash scripts/asan_leak_scan.sh $(sed 's/#.*//' "$DIR/leakers.txt")
