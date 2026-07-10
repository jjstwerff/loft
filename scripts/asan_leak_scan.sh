#!/usr/bin/env bash
# @PLN85 — PER-FILE interpreter leak scan (specific logging the aggregate in-process
# runners can't give).  `loft_suite`/`library_suite` run the whole `.loft` corpus in
# ONE process, so LSan reports at process exit and cannot name the leaking file.  This
# runs the ASan `loft` binary over each file SEPARATELY under detect_leaks=1, so every
# leak is attributed to its file AND its demangled owner frame — emitted as a GitHub
# `::error file=…::` annotation (clickable in the run / PR) plus a plain summary line.
#
# Exits non-zero if ANY file leaks.  ir_read + the macOS dyld-TLS frame are suppressed
# via .github/lsan_suppressions.txt (pass it in LSAN_OPTIONS).
#
#   ABIN=<asan loft>  [LSAN_OPTIONS=suppressions=…]  asan_leak_scan.sh <file.loft>...
set -u
ABIN=${ABIN:?set ABIN to an ASan-instrumented loft binary}
[ -x "$ABIN" ] || { echo "::error::ASan loft binary not executable: '$ABIN'"; exit 2; }

demangle() { if command -v rustfilt >/dev/null; then rustfilt; \
  elif command -v c++filt >/dev/null; then c++filt; else cat; fi; }

fail=0
scanned=0
leakers=0
for f in "$@"; do
  [ -f "$f" ] || continue
  scanned=$((scanned + 1))
  out=$(ASAN_OPTIONS="detect_leaks=1:${ASAN_OPTIONS:-}" "$ABIN" --interpret "$f" 2>&1 || true)
  roots=$(printf '%s\n' "$out" | grep -c '^Direct leak' || true)
  if [ "$roots" -gt 0 ]; then
    fail=1
    leakers=$((leakers + 1))
    owner=$(printf '%s\n' "$out" | demangle \
      | awk '/^Direct leak/{p=1} p; /^$/{p=0}' \
      | sed -E 's/^[[:space:]]*#[0-9]+ 0x[0-9a-f]+ in //; s/\+0x.*$//' \
      | grep -E 'loft::' | grep -vE 'ir_read|loft::main|4loft4main|__rust' \
      | head -1)
    echo "::error file=${f}::interpreter leak — ${roots} root(s); owner: ${owner:-<unknown>}"
    printf 'LEAK  %-52s roots=%s  owner=%s\n' "$f" "$roots" "${owner:-?}"
  fi
done
echo "=== leak scan: ${leakers} leaking file(s) of ${scanned} scanned ==="
exit $fail
