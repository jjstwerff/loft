#!/usr/bin/env bash
# H2 blocker — the FIRST-ASSIGNMENT join-binding axis (the `j*.loft` cells).
#
# `run.sh` varies the call site, `run-callee.sh` varies the callee's return.  Neither
# separates the axis that actually decides this defect: whether the binding is a FIRST
# assignment or a REASSIGNMENT.  The interpreter emits the @PLN85 runtime adopt-vs-copy
# guard (`OpBindOrCopy`) only on the reassign path; a first Set decides adopt-vs-copy
# STATICALLY from `returns_borrowed_view()`.  For a callee whose return is a runtime
# `??` join (`t[i] ?? m_none()`) that static verdict is right on one arm and wrong on
# the other, so the fresh arm's store is orphaned.  A loop body re-declares its local
# every iteration, which is why the leak scales with the trip count.
#
# Every cell is a first Set (a fresh local, never reassigned) of an owned ref from a
# call.  The axis is what the callee hands back and which arm runs.
#
# Usage: ./run-join.sh [path-to-loft]   (default: <repo>/target/release/loft)
set -uo pipefail
cd "$(dirname "$0")"
LOFT=${1:-../../../../../target/release/loft}
[ -x "$LOFT" ] || { echo "no loft binary at $LOFT" >&2; exit 2; }

# Hand-computed per cell — printed here so a wrong fixture cannot masquerade as a pass.
want_val() {
  case "$1" in
    j1_join_fresh)      echo "-1 -1 -1";;   # every call takes the fallback arm
    j2_join_borrow)     echo "1 2 3";;      # every call borrows t's element
    j3_join_mixed)      echo "1 -1 3";;     # i=1 forced out of range
    j4_always_fresh)    echo "0 1 2";;      # callee allocates, no join at all
    j5_always_borrow)   echo "1 2 3";;      # both arms borrow t
    j6_base_not_var)    echo "-1 -1 -1";;   # base reaches the callee as s.items, not a Var
    j7_enum_join)       echo "-1 -1 -1";;   # struct-enum binding, fallback arm
    j8_reassign_join)   echo "-1 -1 -1";;   # control: reassignment, already guarded
    *)                  echo "??";;
  esac
}

fail=0
printf "%-20s %-9s %-7s %-7s %-5s %s\n" CELL BACKEND VALUE LENGTH LEAK VERDICT
for f in j*.loft; do
  c=${f%.loft}
  for b in --interpret --native; do
    # The native leak report is gated on LOFT_NATIVE_LEAK_CHECK, NOT on LOFT_STORES —
    # setting only the latter makes the native leak column silently vacuous.
    out=$(LOFT_STORES=warn LOFT_NATIVE_LEAK_CHECK=1 LOFT_TIMEOUT=120 "$LOFT" "$b" "$f" 2>/tmp/h2_join_err)
    val=$(printf '%s' "$out" | sed -n '1p' | sed 's/ *$//')
    len=$(printf '%s' "$out" | sed -n '2p' | sed 's/ *$//')
    leak=$(grep -c "not freed" /tmp/h2_join_err)
    v_ok=ok; l_ok=ok; k_ok=ok
    [ "$val" = "$(want_val "$c")" ] || v_ok=BAD
    [ "$len" = "len=3" ]            || l_ok=BAD
    [ "$leak" = "0" ]               || k_ok=LEAK
    verdict=PASS
    if [ "$v_ok$l_ok$k_ok" != "okokok" ]; then
      verdict="FAIL(val='$val' $(grep -o 'not freed.*' /tmp/h2_join_err | head -1))"
      fail=$((fail+1))
    fi
    printf "%-20s %-7s %-7s %-7s %-5s %s\n" "$c" "${b#--}" "$v_ok" "$l_ok" "$k_ok" "$verdict"
  done
done
rm -f /tmp/h2_join_err
echo
echo "$fail cell/backend combinations differ from the hand-computed expectation."
cat <<'EOF'
Baseline on a CLEAN tree (2026-07-21) — 5 failing combinations, every one a LEAK with
CORRECT values, which is why a value-only harness reads this defect as green:
  j1_join_fresh    interpret  M×3  the target: 3 iterations, 3 orphaned fallback stores
  j3_join_mixed    interpret  M×1  only the iteration taking the fresh arm leaks
  j6_base_not_var  BOTH       M×3  no local names the base, so neither backend can
                                   witness it — the guard's blind spot, not the bug
  j8_reassign_join native     M×1  native drops the store displaced by the first
                                   reassignment; the interpreter frees it

j7 (struct-enum) is GREEN on both backends: an enum return takes the plain-adopt path,
so the scope-exit free already reclaims it.  Written before measuring, this file
predicted j6 interpret-only, j7 leaking, and j8 clean — all three wrong.  Neither
backend is the clean one; they have DIFFERENT holes.

A run reporting 0 has either fixed the defect or gone blind.  The instrument's own
falsification check: j1 must FAIL on a clean tree.  j2/j5 are the other half of the
guard — a fix that frees unconditionally turns them into a use-after-free of `t`,
which shows up as a wrong VALUE, not as a leak.  j8's interpret half is the
reassignment control: it is green because that path already emits the guard, so if it
ever breaks, the fix landed on the wrong path.
EOF
