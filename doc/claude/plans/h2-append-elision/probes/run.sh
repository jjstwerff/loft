#!/usr/bin/env bash
# H2 probe corpus — run every cell on BOTH backends against its hand-computed value.
# Usage: ./run.sh [path-to-loft]   (default: <repo>/target/release/loft)
set -uo pipefail
cd "$(dirname "$0")"
LOFT=${1:-../../../../../target/release/loft}
[ -x "$LOFT" ] || { echo "no loft binary at $LOFT" >&2; exit 2; }
declare -A EXP=(
  [p1_callres_loop]="1 2 3" [p5_twoargs]="1 2 3"   [p2_single]="1"
  [p3_straight]="1 2"       [p4_literal_tmp]="1 2 3" [p6_via_local]="1 2 3"
  [p7_append_tmp]="1 2 3"   [p8_no_tmp]="1 2 3"
)
fail=0
printf "%-16s %-9s %-8s %s\n" CELL EXPECTED BACKEND ACTUAL
for f in p*.loft; do
  c=${f%.loft}
  for b in --interpret --native; do
    got=$(LOFT_TIMEOUT=120 "$LOFT" "$b" "$f" 2>&1 | head -1 | sed 's/ *$//')
    want=${EXP[$c]:-"(none)"}
    mark=" "; [ "$got" = "$want" ] || { mark="!"; fail=$((fail+1)); }
    printf "%-16s %-9s %-8s %s%s\n" "$c" "$want" "${b#--}" "$mark" "$got"
  done
done
echo; echo "$fail cell/backend combinations differ.  Before the fix: 2 (p1 + p5, --interpret)."
