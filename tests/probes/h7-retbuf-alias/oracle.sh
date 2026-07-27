#!/usr/bin/env bash
# H7 ORACLE — every cell's expected value is HAND-COMPUTED, not taken from a
# reference run, so it cannot inherit the bug it is testing.
#
#   ./oracle.sh              check the current build, both backends
#   LOFT=path ./oracle.sh    check a specific binary
#
# Exit 0 only when every cell matches on BOTH backends.  Cells marked BROKEN are
# the ones a fix must flip; cells marked OK pass today and must STAY passing —
# a fix that trades one for the other is not a fix.
set -uo pipefail
LOFT="${LOFT:-./target/release/loft}"
DIR="$(cd "$(dirname "$0")" && pwd)"

# probe                          expected            status-today
CASES="
01_loop_helper_reassign          3                   BROKEN
02_sequential_helper             3                   OK
03_loop_inline_append            3                   OK
04_loop_range_helper             3                   BROKEN
05_loop_helper_newvar            3                   OK
06_helper_mutates_param          3                   BROKEN
07_helper_fresh_copy             3                   BROKEN
08_text_elements                 3                   BROKEN
11_self_assign_noloop_twice      4                   OK
12_one_iteration                 1                   OK
13_preseeded                     5                   BROKEN
15_two_callsites                 6                   OK
16_while                         3                   BROKEN
17_struct_field                  3                   OK
30_text_accum                    abc                 OK
31_no_self_ref                   3                   OK
32_read_before_call              3                   BROKEN
"

fail=0; pass=0; regressed=0
printf "  %-32s %-8s %-8s %-8s %s\n" PROBE EXPECT INTERP NATIVE VERDICT
while read -r name expect status; do
  [ -z "${name:-}" ] && continue
  i=$(LOFT_TIMEOUT=60  "$LOFT" --interpret "$DIR/$name.loft" 2>&1 | tail -1)
  n=$(LOFT_TIMEOUT=180 "$LOFT" --native    "$DIR/$name.loft" 2>&1 | tail -1)
  if [ "$i" = "$expect" ] && [ "$n" = "$expect" ]; then
    verdict="ok"; pass=$((pass+1))
    [ "$status" = "BROKEN" ] && verdict="FIXED"
  else
    verdict="WRONG ($status)"; fail=$((fail+1))
    # An OK cell going wrong is a REGRESSION — worse than an unfixed BROKEN cell.
    [ "$status" = "OK" ] && { verdict="REGRESSION"; regressed=$((regressed+1)); }
  fi
  [ "$i" = "$n" ] || verdict="$verdict BACKENDS-DISAGREE"
  printf "  %-32s %-8s %-8s %-8s %s\n" "$name" "$expect" "$i" "$n" "$verdict"
done <<< "$CASES"

echo "  ── $pass correct, $fail wrong, $regressed regressions"
[ "$regressed" -gt 0 ] && { echo "  FAIL: a cell that passed today now fails"; exit 2; }
[ "$fail" -gt 0 ] && exit 1
echo "  PASS: every cell correct on both backends"
exit 0
