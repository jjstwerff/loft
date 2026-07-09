#!/usr/bin/env bash
# @PLN85 text-tail-return-leak — boundary + oracle harness.
# Classifies each *.loft.tpl shape as clean / LEAK / UAF using the
# runtime-owner-frame oracle (isolates Class 2 from the intentional ir_read
# Box::leak baseline).  Requires an ASan-instrumented loft binary:
#   RUSTFLAGS=-Zsanitizer=address cargo +nightly build --release \
#     --target <host-triple> --bin loft
# Usage: ABIN=<asan loft> ./run_matrix.sh
set -u
ABIN=${ABIN:?set ABIN to an ASan-instrumented loft binary}
export LOFT_NO_AUTO_REBUILD=1
DIR=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
# runtime-owner leak/UAF frames, EXCLUDING the intentional ir_read Box::leak
rtf() { ASAN_OPTIONS="detect_leaks=1" "$ABIN" "$1" 2>&1 >/dev/null | c++filt \
  | grep -cE "loft::fill::append_text|loft::native::[a-z_0-9]*_dest|loft::native::struct_to_json"; }
uaf() { ASAN_OPTIONS="detect_leaks=0" "$ABIN" "$1" 2>&1 >/dev/null | grep -c "heap-use-after-free"; }
printf "%-18s %-14s %s\n" PROBE VERDICT OUTPUT
for f in "$DIR"/*.loft.tpl; do
  n=$(basename "$f" .loft.tpl)
  sed 's/%N%/5/' "$f" > "$tmp/$n.loft"
  out=$("$ABIN" "$tmp/$n.loft" 2>/dev/null | head -1)
  # UAF/LEAK take precedence — a UAF aborts before printing, so empty output is
  # EXPECTED there; VACUOUS only flags a cell that neither faulted nor produced output.
  if [ "$(uaf "$tmp/$n.loft")" -gt 0 ]; then v="USE-AFTER-FREE"
  elif [ "$(rtf "$tmp/$n.loft")" -gt 0 ]; then v="LEAK"
  elif [ -z "$out" ]; then v="VACUOUS"
  else v="clean"; fi
  printf "%-18s %-14s %s\n" "$n" "$v" "${out:-<no output>}"
done
rm -rf "$tmp"
