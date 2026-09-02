#!/usr/bin/env bash
# Scores the DIAGNOSTIC channel by SEVERITY and CODE.  A coarse "any diagnostic
# fired" test scores a dead-assignment as a pass and cannot tell a hard refusal
# from a warning, which hid two findings on the first run.
#
# The cell must also RUN: a binary that never executed is silent too, so
# SILENT-because-nothing-happened is checked apart from SILENT-because-clean.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(git -C "$HERE" rev-parse --show-toplevel)"
L="${LOFT_BIN:-$ROOT/target/release/loft}"
[ -x "$L" ] || { echo "no loft binary at $L (set LOFT_BIN, or cargo build --release --bin loft)"; exit 2; }
cd "$HERE"
fail=0
printf '%-22s %-8s %-9s %-4s %s\n' CELL WANT GOT RAN "FIRST DIAGNOSTIC"
for f in *.loft; do
  want=$(head -1 "$f" | sed 's|// want: ||')
  err=$(LOFT_TIMEOUT=30 timeout 60 "$L" --path "$ROOT/" --interpret "$f" 2>/dev/null >/dev/null 2>&1; \
        LOFT_TIMEOUT=30 timeout 60 "$L" --path "$ROOT/" --interpret "$f" 2>&1 >/dev/null)
  out=$(LOFT_TIMEOUT=30 timeout 60 "$L" --path "$ROOT/" --interpret "$f" 2>/dev/null)
  line=$(printf '%s\n' "$err" | grep -m1 -E '^(warning|error|advice)')
  case "$line" in
    error*)   got=ERROR ;;
    warning*) got=WARN ;;
    advice*)  got=ADVICE ;;
    *)        got=SILENT ;;
  esac
  # a cell that produced no stdout AND no diagnostic never ran — vacuous, not clean
  if [ -n "$out" ]; then ran=yes; else ran=NO; fi
  if [ "$got" = "SILENT" ] && [ "$ran" = "NO" ]; then st="VACU"; fail=1
  elif [ "$want" = "$got" ]; then st="ok  "
  else st="FAIL"; fail=1; fi
  printf '%s %-22s %-8s %-9s %-4s %s\n' "$st" "${f%.loft}" "$want" "$got" "$ran" "$(printf '%s' "$line" | cut -c1-64)"
done
exit $fail
