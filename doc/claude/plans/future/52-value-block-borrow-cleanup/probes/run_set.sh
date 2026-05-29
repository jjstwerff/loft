#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# PLAN52 probe-set runner.  Runs a curated subset of probes under both
# --interpret and --native, summarising PASS/FAIL/CRASH/HANG.  Use as
# fix-attempt validation gate — much faster than the full 60-probe sweep.
#
# Usage:
#   ./run_set.sh A        # run set A (cluster I core)
#   ./run_set.sh A -v     # verbose: include probe output on failure
#   ./run_set.sh all      # run every set including Z (known-broken)
#
# Sets — see ../README.md § "Curated probe sets" for the mapping rationale.
#
# Each probe runs with LOFT_TIMEOUT=10 (PLAN49) + LOFT_TIMEOUT_CLEAN_EXIT
# so a parser hang or runaway aborts cleanly with a localised breadcrumb
# instead of needing `pkill -9` from a human.
#
# Status codes per probe:
#   PASS       — probe printed "PASSED" and exited 0
#   FAIL       — probe ran, assertion or value check failed
#   CRASH      — SIGSEGV / SIGBUS detected in output
#   HANG       — LOFT_TIMEOUT fired
#   PARSE-ERR  — parser refused the program
#   COMPILE-ERR— native compile failed with rustc error
#
# Exit code: 0 if every probe PASS on both backends; 1 otherwise.
#
# Portability: macOS's default /bin/bash is 3.2 with no `declare -A`,
# so probes-per-set are looked up via `case` instead of an assoc array.

set -u
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../../../.." && pwd)"
LOFT_BIN="$REPO_ROOT/target/release/loft"

# Probe-set definitions (kept in sync with README's "Curated probe sets"
# table).  Add new sets here AND in usage() + set_name().
probes_in_set() {
  case "$1" in
    A) echo "02 13 14 15 16 17" ;;
    B) echo "26 29 31 32 33 38 39 44 81" ;;
    C) echo "46 49 53" ;;
    D) echo "09 19 30 56 78" ;;
    E) echo "21 22 23 36 40 41 50" ;;
    F) echo "45 65 67 86" ;;
    G) echo "47 48 82" ;;
    H) echo "01 03 07 08 10 18 35 42 43 55 60" ;;
    I) echo "24 71" ;;
    J) echo "91 92 93 94 95 96 97" ;;
    Z) echo "51 52 80 85" ;;
    *) return 1 ;;
  esac
}

set_name() {
  case "$1" in
    A) echo "cluster-I-core" ;;
    B) echo "cluster-I-garbage-consumer" ;;
    C) echo "cluster-I-crash" ;;
    D) echo "cluster-III-format" ;;
    E) echo "cluster-IV-heap-type" ;;
    F) echo "cluster-VI-closure" ;;
    G) echo "cluster-VII-chained-call" ;;
    H) echo "baselines-regression-guard" ;;
    I) echo "real-library" ;;
    J) echo "cluster-IV-Vec-nested-field-push" ;;
    Z) echo "known-parser-bugs-skip" ;;
    *) echo "?" ;;
  esac
}

usage() {
  echo "Usage: $0 <SET> [-v]"
  echo
  echo "Available sets:"
  for s in A B C D E F G H I J Z; do
    probes=$(probes_in_set "$s")
    count=$(echo "$probes" | wc -w | tr -d ' ')
    name=$(set_name "$s")
    printf "  %s  (%s)  %d probes\n" "$s" "$name" "$count"
  done
  echo "  all  — run every set in order A..J (skip Z)"
  echo
  echo "Options:"
  echo "  -v   Show probe output on FAIL/CRASH (default: just status code)"
  exit 1
}

[[ $# -ge 1 ]] || usage
SET_LETTER="$1"
VERBOSE=0
[[ "${2:-}" == "-v" ]] && VERBOSE=1

if [[ ! -x "$LOFT_BIN" ]]; then
  echo "loft binary not found at $LOFT_BIN — run 'cargo build --release --bin loft' first" >&2
  exit 2
fi

# Try to use the same SDKROOT that the rest of the toolchain expects.
if command -v xcrun >/dev/null 2>&1; then
  export SDKROOT="$(xcrun --show-sdk-path 2>/dev/null || true)"
fi
export LOFT_TIMEOUT="${LOFT_TIMEOUT:-10}"
export LOFT_TIMEOUT_CLEAN_EXIT=1

# Classify a probe-run's combined stdout+stderr + exit code.
classify() {
  local out="$1"
  local rc="$2"
  if echo "$out" | grep -q "PASSED"; then
    echo PASS
  elif echo "$out" | grep -qE "SIGBUS|SIGSEGV|loft crash"; then
    echo CRASH
  elif echo "$out" | grep -q "\[timeout\] hard-kill"; then
    echo HANG
  elif echo "$out" | grep -qE "error\[E[0-9]+\]|native compilation failed"; then
    echo COMPILE-ERR
  elif echo "$out" | grep -qE "^error: (Expect token|Unknown|Field access|cannot be captured|Variable .* cannot|Struct .* has a field)"; then
    echo PARSE-ERR
  elif echo "$out" | grep -q "assertion failed"; then
    echo FAIL
  elif [[ $rc -ne 0 ]]; then
    echo FAIL
  else
    echo PASS
  fi
}

run_one() {
  local probe_num="$1"
  local backend="$2"
  local probe_file
  probe_file=$(ls "$SCRIPT_DIR"/"$probe_num"-*.loft 2>/dev/null | head -1)
  if [[ -z "$probe_file" ]]; then
    echo "MISSING"
    return
  fi
  local out rc
  out=$("$LOFT_BIN" "--$backend" "$probe_file" 2>&1)
  rc=$?
  local status
  status=$(classify "$out" "$rc")
  echo "$status"
  if [[ $VERBOSE -eq 1 && "$status" != "PASS" ]]; then
    echo "$out" | sed 's/^/    | /' >&2
  fi
}

run_set() {
  local letter="$1"
  local probes
  probes=$(probes_in_set "$letter")
  if [[ -z "$probes" ]]; then
    echo "Unknown set: $letter" >&2
    return 1
  fi
  local name
  name=$(set_name "$letter")
  echo "=== Set $letter ($name) ==="
  local fail_count=0
  printf "%-6s  %-12s  %-12s\n" "probe" "interpret" "native"
  for p in $probes; do
    local i_status n_status
    i_status=$(run_one "$p" "interpret")
    n_status=$(run_one "$p" "native")
    printf "%-6s  %-12s  %-12s\n" "$p" "$i_status" "$n_status"
    if [[ "$i_status" != "PASS" || "$n_status" != "PASS" ]]; then
      fail_count=$((fail_count + 1))
    fi
  done
  echo
  if [[ $fail_count -eq 0 ]]; then
    echo "Set $letter: all probes PASS on both backends."
  else
    echo "Set $letter: $fail_count probe(s) not PASS — see status column."
  fi
  return $fail_count
}

if [[ "$SET_LETTER" == "all" ]]; then
  total_fail=0
  for s in A B C D E F G H I J; do
    run_set "$s"
    total_fail=$((total_fail + $?))
    echo
  done
  echo "TOTAL: $total_fail non-PASS probes across sets A..J"
  [[ $total_fail -eq 0 ]] && exit 0 || exit 1
fi

if ! probes_in_set "$SET_LETTER" >/dev/null; then
  echo "Unknown set: $SET_LETTER" >&2
  echo
  usage
fi

run_set "$SET_LETTER"
exit $?
