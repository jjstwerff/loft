#!/usr/bin/env bash
# Probe runner for the nested narrow-int vector width fault (loft#624 nested /
# #483 remainder / @PLAN58 cluster III).
#
# Tooling notes — every one of these is a bug I actually hit this session and
# the reason this script exists instead of an inline one-liner:
#
#   * `cmd | grep ...; rc=$?` reads GREP's status, not the program's.  A crashing
#     probe read as rc=0.  Here the program runs on its own line and $? is taken
#     immediately.
#   * `cargo build | grep error` then "Finished in 0.05s" is a CACHE hit reported
#     as a successful compile.  `--check-build` forces a real rebuild and reports
#     cargo's own exit code.
#   * A probe that prints nothing is not a pass.  A cell with no output is VACUOUS
#     and is reported as such, never as agreement.
#   * Agreement between two backends is not correctness — both can be wrong the
#     same way.  Every probe carries a HAND-COMPUTED expected value.
#
# Usage:  run.sh [--check-build] [--flag ENV=VAL] [probe.loft ...]
set -uo pipefail
LOFT_DIR=/home/jurjens/workspace/loft
LOFT=$LOFT_DIR/target/debug/loft
HERE="$(cd "$(dirname "$0")" && pwd)"

FLAG_ENV=()
PROBES=()
CHECK_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --check-build) CHECK_BUILD=1; shift;;
    --flag) FLAG_ENV+=("$2"); shift 2;;
    *) PROBES+=("$1"); shift;;
  esac
done

if [ "$CHECK_BUILD" = 1 ]; then
  # Force a real compile: a stale binary silently invalidates every cell below.
  touch "$LOFT_DIR/src/main.rs"
  ( cd "$LOFT_DIR" && cargo build -q --bin loft )
  rc=$?
  if [ $rc -ne 0 ]; then echo "BUILD FAILED (rc=$rc) — every cell below would be stale"; exit 1; fi
fi

mkdir -p "$HERE/out"   # per-cell raw output; recreated on every run
[ ${#PROBES[@]} -eq 0 ] && PROBES=("$HERE"/probes/*.loft)

pass=0; fail=0; crash=0; vac=0
printf "%-34s %-11s %-7s %s\n" "PROBE" "BACKEND" "VERDICT" "DETAIL"
printf "%-34s %-11s %-7s %s\n" "-----" "-------" "-------" "------"
for p in "${PROBES[@]}"; do
  [ -f "$p" ] || continue
  name=$(basename "$p" .loft)
  # Expected value is declared IN the probe: `//! expect: <exact stdout line>`
  want=$(grep -m1 '^//! expect:' "$p" | sed 's|^//! expect: ||')
  for be in --interpret --native; do
    out=$( cd "$LOFT_DIR" && env "${FLAG_ENV[@]}" LOFT_TIMEOUT=120 timeout 240 "$LOFT" "$be" "$p" 2>&1 )
    rc=$?                       # <- immediately after the run, no pipeline
    echo "$out" > "$HERE/out/${name}${be}.txt"
    # Strip only KNOWN log prefixes.  A blanket `grep -v '^\['` also ate every
    # answer, because a rendered vector starts with `[` — the harness-control
    # probe caught that by reporting VACUOUS instead of ok.
    got=$(echo "$out" | grep -vE '^\[(text-timeline|schema|store|par)\]' \
                      | grep -viE '^(warning|note):' | head -1)
    leak=$(echo "$out" | grep -c 'not freed')
    if [ $rc -ge 128 ] || echo "$out" | grep -q SIGSEGV; then
      printf "%-34s %-11s %-7s %s\n" "$name" "$be" "CRASH" "rc=$rc"
      crash=$((crash+1))
    elif [ -z "$got" ]; then
      printf "%-34s %-11s %-7s %s\n" "$name" "$be" "VACUOUS" "no output (rc=$rc) — cannot pass"
      vac=$((vac+1))
    elif [ -z "$want" ]; then
      printf "%-34s %-11s %-7s %s\n" "$name" "$be" "NO-EXP" "got=$got (probe declares no expect:)"
      vac=$((vac+1))
    elif [ "$got" = "$want" ]; then
      if [ "$leak" != "0" ]; then
        printf "%-34s %-11s %-7s %s\n" "$name" "$be" "LEAK" "value ok but a store leaked"
        fail=$((fail+1))
      else
        printf "%-34s %-11s %-7s %s\n" "$name" "$be" "ok" ""
        pass=$((pass+1))
      fi
    else
      printf "%-34s %-11s %-7s %s\n" "$name" "$be" "WRONG" "want=$want got=$got"
      fail=$((fail+1))
    fi
  done
done
echo
echo "pass=$pass wrong=$fail crash=$crash vacuous=$vac"
[ $((fail+crash+vac)) -eq 0 ]
