#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The release's valgrind gate, as one command (RELEASE.md § Memory safety, the checklist's
# `M-valgrind`): every script and document under memcheck, on the interpreter AND as the
# compiled native program, with one verdict.
#
#   scripts/valgrind-sweep.sh                 # everything: ~1150 files, ~15 min on 24 cores
#   scripts/valgrind-sweep.sh tests/docs      # one tree, or any list of .loft files
#   VG_JOBS=8 scripts/valgrind-sweep.sh       # fewer parallel memchecks (each takes ~200 MB)
#   VG_OUT=target/vg-probe …                  # keep the logs apart from another sweep's
#
# What counts, and why:
#   * an INVALID ACCESS of any kind (read, write, uninitialised use, bad free, a syscall
#     handed uninitialised bytes) fails the sweep — that is the class the gate exists for,
#     the one Linux's allocator hides in slack and Windows' heap checker reports as
#     STATUS_HEAP_CORRUPTION (TESTING.md § Occasional valgrind pass);
#   * a DEFINITELY LOST block fails it — memory nothing can reach any more;
#   * a "possibly lost" record does NOT.  Rust's hashbrown tables and boxed strings keep
#     interior pointers, so every process-lifetime table — the parser's `Data`, the native
#     emitter registry — reads as possibly lost at exit; measured at 179 records on a clean
#     run.  `--errors-for-leak-kinds=definite` is that decision, spelled where valgrind
#     reads it.
#   * loft's own store arena is INVISIBLE to memcheck (DEBUG.md § Debugging store-ownership
#     bugs): a leaked or over-freed STORE is a wrong answer, never a valgrind error.  That
#     half of the memory gate is `M-leaks` under `LOFT_STRICT_STORES=1`, not this one.
#
# The interpreter half runs `loft --interpret` on every file — `--tests` for tests/scripts,
# whose files have no `main` and run nothing without it (TESTING.md § The harness).  The
# native half compiles each tests/docs document with `loft --native` (unchecked, so rustc is
# not traced) and then hands the cached binary in `<dir>/.loft/cache/` to memcheck directly:
# `--trace-children` would follow rustc, and the driver's own exec of the program is what
# `--trace-children-skip` cannot single out.  Per-file logs stay in target/vg/ for reading.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
command -v valgrind >/dev/null || { echo "valgrind is not installed — the gate cannot run here"; exit 2; }
[ -x target/release/loft ] || { echo "target/release/loft is not built — cargo build --release first"; exit 2; }

OUT=${VG_OUT:-target/vg}
rm -rf "$OUT"; mkdir -p "$OUT"
JOBS=${VG_JOBS:-$(( $(nproc) * 5 / 6 ))}; [ "$JOBS" -lt 1 ] && JOBS=1
VG="valgrind --error-exitcode=77 --leak-check=full --errors-for-leak-kinds=definite"
export VG OUT

# The population: every argument (a file or a directory), or the two shipped corpora.
if [ $# -gt 0 ]; then
  for a in "$@"; do [ -d "$a" ] && find "$a" -maxdepth 1 -name '*.loft' | sort || echo "$a"; done
else
  ls tests/scripts/*.loft tests/docs/*.loft
fi > "$OUT/list.txt"

# One line per run: kind, file, exit, invalid-access count, definitely-lost bytes.
check_one() {
  kind=$1; f=$2; stem=$(basename "$f" .loft); log="$OUT/$kind-$stem.log"
  case "$kind" in
    interp) case "$f" in tests/scripts/*) mode="--interpret --tests";; *) mode="--interpret";; esac
            LOFT_TIMEOUT=300 $VG --log-file="$log" target/release/loft $mode "$f" >/dev/null 2>&1; rc=$? ;;
    native) LOFT_TIMEOUT=300 target/release/loft --native "$f" >/dev/null 2>&1 || { echo "native	$f	build-failed	-	-"; return; }
            dir=$(dirname "$f"); bin=$(ls -t "$dir"/.loft/cache/"$stem"-* 2>/dev/null | head -1)
            [ -n "$bin" ] || { echo "native	$f	no-binary	-	-"; return; }
            $VG --log-file="$log" "$bin" >/dev/null 2>&1; rc=$? ;;
  esac
  bad=$(grep -cE "^==[0-9]+== (Invalid (read|write|free)|Conditional jump|Use of uninitialised|Syscall param|Mismatched free|Jump to the invalid|Source and destination overlap)" "$log")
  lost=$(grep -oE "definitely lost: [0-9,]+ bytes" "$log" | head -1 | tr -d ', ' | grep -oE "[0-9]+" || echo 0)
  echo "$kind	$f	$rc	$bad	${lost:-0}"
}
export -f check_one

{ sed 's/^/interp /' "$OUT/list.txt"; grep '^tests/docs/' "$OUT/list.txt" | sed 's/^/native /'; } \
  | xargs -P "$JOBS" -n 2 bash -c 'check_one "$0" "$1"' > "$OUT/results.tsv"

runs=$(wc -l < "$OUT/results.tsv")
invalid=$(awk -F'\t' '$4 != "-" && $4 > 0' "$OUT/results.tsv" | wc -l)
lost=$(awk -F'\t' '$5 != "-" && $5 > 0' "$OUT/results.tsv" | wc -l)
unbuilt=$(awk -F'\t' '$3 == "build-failed" || $3 == "no-binary"' "$OUT/results.tsv" | wc -l)
timed=$(grep -lE "LOFT_TIMEOUT|timed out|watchdog" "$OUT"/*.log 2>/dev/null | wc -l)
echo "valgrind sweep: $runs runs ($(grep -c '^interp' "$OUT/results.tsv") interpreter, $(grep -c '^native' "$OUT/results.tsv") native)"
echo "  invalid accesses: $invalid file(s) · definitely lost: $lost file(s) · native not built: $unbuilt · timed out: $timed"
if [ "$invalid" -gt 0 ] || [ "$lost" -gt 0 ]; then
  echo "  RED — the offending files (kind, file, exit, invalid-access count, bytes definitely lost):"
  awk -F'\t' '($4 != "-" && $4 > 0) || ($5 != "-" && $5 > 0)' "$OUT/results.tsv" | head -40
  echo "  logs: $OUT/<kind>-<stem>.log"
  exit 1
fi
echo "  GREEN — no invalid access and nothing definitely lost, on either backend (logs in $OUT/)"
