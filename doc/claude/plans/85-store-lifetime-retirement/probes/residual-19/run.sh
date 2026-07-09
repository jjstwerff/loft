#!/usr/bin/env bash
# @PLN85 residual-19 leak-oracle runner. Counts append_text leak frames
# (runtime-owner, excl ir_read) per probe. Set ABIN to an ASan-instrumented loft.
#   ABIN=target/aarch64-apple-darwin/release/loft ./run.sh
set -u
DIR=$(cd "$(dirname "$0")" && pwd)
ABIN=${ABIN:?set ABIN to an ASan loft binary}
lc() { ASAN_OPTIONS=detect_leaks=1 "$ABIN" "$1" 2>&1 | c++filt \
  | grep -E '#[0-9]+ .* in ' | grep 'loft::fill::append_text' | grep -v ir_read | wc -l | tr -d ' '; }
for f in "$DIR"/*.loft; do
  printf '%-40s leak=%s\n' "$(basename "$f" .loft)" "$(lc "$f")"
done
