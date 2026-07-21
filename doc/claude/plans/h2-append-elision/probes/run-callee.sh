#!/usr/bin/env bash
# H2 callee-return axis — the r*.loft cells.
#
# The p*.loft corpus varies the CALL-SITE shape with the callee held fixed (every cell
# calls the same struct-literal `mk`).  That is the wrong axis for validating a fix whose
# correctness depends on WHAT STORE the callee hands back: the value-before-slot patch
# went green on all 8 p-cells while leaking `M×18` in a shape none of them expressed.
# These cells vary the callee and hold the call site fixed.
#
# Unlike run.sh (value only) every cell asserts THREE properties, because the failure
# modes are different and each hides the others: a wrong VALUE (H2's corruption), a wrong
# LENGTH (a delivery that doubles or drops elements reads as leak-free), and a LEAK (the
# regression that blocked the fix).  Both backends — H2 is interpreter-only, so a
# single-backend run reports green on the broken one.
#
# Usage: ./run-callee.sh [path-to-loft]   (default: <repo>/target/release/loft)
set -uo pipefail
cd "$(dirname "$0")"
LOFT=${1:-../../../../../target/release/loft}
[ -x "$LOFT" ] || { echo "no loft binary at $LOFT" >&2; exit 2; }

WANT_LEN="len=3"
# Expectation is `1 2 3` for every cell BY CONSTRUCTION (each appends the three seeded
# values in order) — held constant so any divergence is the axis talking, never the
# fixture.  The one exception is spelled out rather than left implicit: r10 forces the
# out-of-range arm, so every element is legitimately the fallback record.
want_val() { case "$1" in r10_orelse_fresh_loop) echo "-1 -1 -1";; *) echo "1 2 3";; esac; }

fail=0
printf "%-20s %-9s %-7s %-7s %-5s %s\n" CELL BACKEND VALUE LENGTH LEAK VERDICT
for f in r*.loft; do
  c=${f%.loft}
  for b in --interpret --native; do
    out=$(LOFT_STORES=warn LOFT_TIMEOUT=120 "$LOFT" "$b" "$f" 2>/tmp/h2_cell_err)
    val=$(printf '%s' "$out" | sed -n '1p' | sed 's/ *$//')
    len=$(printf '%s' "$out" | sed -n '2p' | sed 's/ *$//')
    leak=$(grep -c "not freed" /tmp/h2_cell_err)
    v_ok=ok; l_ok=ok; k_ok=ok
    [ "$val" = "$(want_val "$c")" ] || v_ok=BAD
    [ "$len" = "$WANT_LEN" ] || l_ok=BAD
    [ "$leak" = "0" ]        || k_ok=LEAK
    verdict=PASS
    if [ "$v_ok$l_ok$k_ok" != "okokok" ]; then verdict="FAIL($val)"; fail=$((fail+1)); fi
    printf "%-20s %-7s %-7s %-7s %-5s %s\n" "$c" "${b#--}" "$v_ok" "$l_ok" "$k_ok" "$verdict"
  done
done
rm -f /tmp/h2_cell_err
echo
echo "$fail cell/backend combinations differ from the hand-computed expectation."
cat <<'EOF'
Baseline on a CLEAN tree (no fix applied), 2026-07-21 — 3 failing combinations:
  r1_retbuf_loop      interpret  VALUE `1 null null`  — H2 itself (same shape as p1)
  r5_branch_loop      interpret  LEAK  2 stores       — INDEPENDENT of H2: needs no
  r6_branch_straight  interpret  LEAK  2 stores         loop, and native is clean

WITH doc/claude/plans/h2-append-elision/value-before-slot.patch — also 3, different set:
  r1 FIXED (the point of the patch); r5/r6 unchanged (the patch neither causes nor
  fixes them); and NEW:
  r10_orelse_fresh_loop interpret LEAK `M×3` — the patch's blocker, reproduced here in
    3s.  It is the same defect the full suite reported as `M×18` in
    tests/use_analysis.rs::join_own_fixes_elem_accumulate_both_backends (224s to find).
    The hoisted temp is reassigned per iteration; when the callee takes its FRESH arm
    (`t[i] ?? m_none()` out of range) the displaced store is never freed.  A fix for the
    hoist is only done when this cell is green AND r1 stays green.

A run that reports 0 has either fixed all of these or lost the ability to see them —
revert the patch and confirm r1 fails again before believing a green sweep.
EOF
