#!/usr/bin/env bash
# @PLN85 text-tail-return-leak — boundary + oracle harness (v2: VALUE + LEAK + UAF).
#
# Attempt 1 (see text-tail-return-leak.md) regressed a test on VALUE (an empty
# return), which a leak-only oracle green-lights — so this harness asserts THREE
# things per probe:
#   VALUE : N=1 output == committed .golden   (catches an over-aggressive free that
#           empties/wrongs a TRANSFER shape — the attempt-1 failure mode)
#   LEAK  : N=105 has zero runtime-owner leak frames (append_text/*_dest/struct_to_json,
#           excluding the intentional ir_read Box::leak baseline)
#   UAF   : N=1 reports no heap-use-after-free
# A fix is correct only when EVERY probe reads VALUE=ok AND clean.
#
# Requires an ASan-instrumented loft binary; a non-ASan binary for the VALUE check
# is optional (defaults to ABIN, whose UAF cell will read VACUOUS — expected).
#   RUSTFLAGS=-Zsanitizer=address cargo +nightly build --release \
#     --target <host-triple> --bin loft
# Usage: ABIN=<asan loft> [VBIN=<stable loft>] ./run_matrix.sh
set -u
ABIN=${ABIN:?set ABIN to an ASan-instrumented loft binary}
VBIN=${VBIN:-$ABIN}                       # value check prefers a NON-ASan binary
export LOFT_NO_AUTO_REBUILD=1
DIR=$(cd "$(dirname "$0")" && pwd); tmp=$(mktemp -d)
rtf() { ASAN_OPTIONS="detect_leaks=1" "$ABIN" "$1" 2>&1 >/dev/null | c++filt \
  | grep -cE "loft::fill::append_text|loft::native::[a-z_0-9]*_dest|loft::native::struct_to_json"; }
uaf() { ASAN_OPTIONS="detect_leaks=0" "$ABIN" "$1" 2>&1 >/dev/null | grep -c "heap-use-after-free"; }
printf "%-16s %-8s %-14s %s\n" PROBE VALUE MEMORY GOLDEN
for f in "$DIR"/*.loft.tpl; do
  n=$(basename "$f" .loft.tpl); g=$(cat "$DIR/$n.golden" 2>/dev/null)
  sed 's/%N%/1/'   "$f" > "$tmp/$n.1.loft"
  sed 's/%N%/105/' "$f" > "$tmp/$n.105.loft"
  out=$("$VBIN" "$tmp/$n.1.loft" 2>/dev/null)
  val=$([ "$out" = "$g" ] && echo ok || echo WRONG)
  if   [ "$(uaf "$tmp/$n.1.loft")"   -gt 0 ]; then mem="USE-AFTER-FREE"
  elif [ "$(rtf "$tmp/$n.105.loft")" -gt 0 ]; then mem="LEAK"
  else mem="clean"; fi
  printf "%-16s %-8s %-14s %s\n" "$n" "$val" "$mem" "$g"
done
rm -rf "$tmp"
