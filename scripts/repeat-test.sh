#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Repeat one test N times under load and count the outcomes.
#
# For a fault that fires on SOME runs, a single green run carries almost no information
# and "I ran it again and it passed" carries none at all.  What decides is a count, and
# it needs three things a plain loop does not give you:
#
#   * N >= 12.  A fault that fires 1 run in 5 shows up as 12/12 green about 7% of the
#     time; at N=3 that rises to 51%, which is a coin toss reported as a fix.
#   * VACUOUS runs counted separately.  Several loft suites `return` early with
#     "skipping: …" when a prerequisite is missing, so they PASS while testing nothing.
#     Folding those into the pass column is how a harness certifies a fix it never
#     exercised.
#   * A must-fail CONTROL.  If the same harness cannot make the known-bad build fail,
#     it is not reproducing the condition, and its green run says nothing about the fix.
#     `--control <binary>` runs a second binary through the identical loop for exactly
#     this; without it the report says so rather than claiming a result.
#
# Load matters because timing faults hide on an idle machine: `--load N` saturates N
# cores for the duration, which is what a parallel suite does to a test that waits on a
# freshly spawned process.
#
# Usage:
#   scripts/repeat-test.sh --bin target/release/deps/engine_host_connector-<hash> \
#       --filter keyframes_survive_total_datagram_loss --runs 12 --load 8 \
#       [--control <other-binary>]
set -uo pipefail

BIN=""; CONTROL=""; FILTER=""; RUNS=12; LOAD=0
die() { echo "repeat-test: $*" >&2; exit 2; }

while [ $# -gt 0 ]; do
  case "$1" in
    --bin)     BIN="${2:-}";     shift 2 ;;
    --control) CONTROL="${2:-}"; shift 2 ;;
    --filter)  FILTER="${2:-}";  shift 2 ;;
    --runs)    RUNS="${2:-}";    shift 2 ;;
    --load)    LOAD="${2:-}";    shift 2 ;;
    -h|--help) sed -n '5,28p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done
[ -n "$BIN" ] || die "--bin is required"
[ -x "$BIN" ] || die "$BIN is not executable"

load_pids=()
start_load() {
  [ "$LOAD" -gt 0 ] || return 0
  for _ in $(seq 1 "$LOAD"); do
    ( while :; do :; done ) & load_pids+=($!)
  done
}
stop_load() { [ ${#load_pids[@]} -eq 0 ] || kill "${load_pids[@]}" 2>/dev/null; load_pids=(); }
trap stop_load EXIT INT TERM

# Run `$1` (a test binary) `$RUNS` times; echo "pass fail vacuous".
run_series() {
  local bin="$1" pass=0 fail=0 vac=0 out
  local n="${RUNS}"
  for i in $(seq 1 "$n"); do
    out=$("$bin" "$FILTER" --test-threads=1 --nocapture 2>&1)
    if echo "$out" | grep -q "skipping:"; then
      vac=$((vac + 1)); printf 'V'
    elif echo "$out" | grep -qE "^test result: ok\."; then
      # "ok" with 0 tests run is also vacuous — a filter that matches nothing.
      if echo "$out" | grep -qE "^test result: ok\. 0 passed"; then
        vac=$((vac + 1)); printf 'V'
      else
        pass=$((pass + 1)); printf '.'
      fi
    else
      fail=$((fail + 1)); printf 'F'
      [ -n "${VERBOSE:-}" ] && echo && echo "$out" | tail -20
    fi
  done
  echo
  echo "$pass $fail $vac"
}

echo "== $RUNS runs, filter='$FILTER', load=$LOAD cores =="
start_load
cp=""; cf=""; cv=""
if [ -n "$CONTROL" ]; then
  [ -x "$CONTROL" ] || die "$CONTROL is not executable"
  # INTERLEAVED, not one series then the other.  Ambient load varies over minutes, so
  # back-to-back series compare two different conditions and the difference between them
  # is unreadable: a first attempt scored subject 2 failures and control 0 purely because
  # the heavy phase of the background suite had passed by the control's turn.
  echo "   (interleaved subject/control, so both see the same load)"
  p=0; f=0; v=0; cp=0; cf=0; cv=0
  for i in $(seq 1 "$RUNS"); do
    read -r a b c <<<"$(RUNS=1 run_series "$BIN" | tail -1)"
    p=$((p+a)); f=$((f+b)); v=$((v+c))
    read -r a b c <<<"$(RUNS=1 run_series "$CONTROL" | tail -1)"
    cp=$((cp+a)); cf=$((cf+b)); cv=$((cv+c))
  done
  echo "  subject:  $p pass, $f fail, $v VACUOUS"
  echo "  control:  $cp pass, $cf fail, $cv VACUOUS"
else
  read -r p f v <<<"$(run_series "$BIN" | tail -1)"
  echo "  subject:  $p pass, $f fail, $v VACUOUS"
fi
stop_load

echo
if [ "$v" -gt 0 ]; then
  echo "VERDICT: $v of $RUNS subject runs tested NOTHING — fix that before reading the rest."
  exit 2
fi
if [ -n "$CONTROL" ] && [ "${cf:-0}" -eq 0 ]; then
  echo "VERDICT: INCONCLUSIVE — the control never failed, so this loop does not reproduce"
  echo "         the condition.  The subject's $p/$RUNS says nothing about the fix."
  exit 2
fi
if [ "$f" -eq 0 ]; then
  if [ -n "$CONTROL" ]; then
    echo "VERDICT: subject $p/$RUNS clean while the control failed $cf/$RUNS — the fix holds"
    echo "         under a loop that demonstrably provokes the fault."
  else
    echo "VERDICT: subject $p/$RUNS clean.  NO CONTROL was run, so this cannot distinguish"
    echo "         a fix from a loop that never provokes the fault."
  fi
  exit 0
fi
echo "VERDICT: still failing — $f of $RUNS."
exit 1
