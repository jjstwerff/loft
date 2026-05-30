#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# PLAN53 cluster-2 probe runner.  Classifies every NN-*.loft probe under the
# aligned eval-stack mode (LOFT_ALIGN=1 LOFT_SLOT_V2=drive) AND the production
# flag-OFF default, on the interpreter backend (the byte-packed stack that
# cluster 2 lives in; --native has no such stack).
#
# Each probe is its own subprocess under LOFT_TIMEOUT, so a runaway generator
# (the 2a HANG family) aborts cleanly with a breadcrumb instead of spinning
# the machine, and a SIGSEGV stays contained to one probe instead of taking
# the whole sweep down.  This isolation is the substrate the in-process
# `cargo test --test issues` run cannot provide.
#
# Classification:
#   PASS   probe printed "PASSED", exit 0, no assertion failure
#   FAIL   ran but value/assertion wrong (silent corruption)
#   HANG   LOFT_TIMEOUT fired (runaway / non-terminating loop)
#   CRASHn SIGSEGV/SIGBUS (signal n) — UB became a hard crash
#
# INVARIANT: every probe must PASS flag-OFF (production is clean).  A probe
# that fails flag-OFF is contaminated by an unrelated bug — fix or drop it.
# The *-ref* probes must also PASS aligned; if one fails, the cluster
# diagnosis is wrong.
#
# Usage:  ./run.sh            # aligned + flag-off table for all probes
#         ./run.sh 2a         # only probes whose name starts 2a
#         ./run.sh 2a -v      # verbose: print probe stderr on non-PASS
set -u
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../../../.." && pwd)"
BIN="$REPO_ROOT/target/release/loft"
[ -x "$BIN" ] || { echo "build first: cargo build --release --bin loft"; exit 2; }

FILTER="${1:-}"; [ "$FILTER" = "-v" ] && FILTER=""
VERBOSE=0; for a in "$@"; do [ "$a" = "-v" ] && VERBOSE=1; done

classify() { # $1=probe  $2=extra-env
  local out rc
  out=$(env $2 LOFT_TIMEOUT=6 "$BIN" --interpret "$1" 2>&1); rc=$?
  LAST_OUT="$out"
  if   [ $rc -eq 124 ] || echo "$out" | grep -qi "hard-kill\|deadline"; then echo HANG
  elif [ $rc -ge 128 ]; then echo "CRASH$((rc-128))"
  elif echo "$out" | grep -qi "assertion failed\|^error:"; then echo FAIL
  elif echo "$out" | grep -q PASSED; then echo PASS
  else echo "?$rc"; fi
}

fail=0
printf '%-42s %-9s %-9s\n' PROBE ALIGNED FLAG-OFF
for f in "$SCRIPT_DIR"/${FILTER}*.loft; do
  [ -e "$f" ] || continue
  b=$(basename "$f")
  a=$(classify "$f" "LOFT_ALIGN=1 LOFT_SLOT_V2=drive")
  o=$(classify "$f" "")
  printf '%-42s %-9s %-9s\n' "$b" "$a" "$o"
  [ "$o" != PASS ] && { echo "  !! INVARIANT BROKEN: $b fails flag-OFF"; fail=1; }
  case "$b" in *-ref*) [ "$a" != PASS ] && { echo "  !! REFERENCE BROKEN: $b fails aligned"; fail=1; };; esac
  [ $VERBOSE -eq 1 ] && [ "$a" != PASS ] && echo "$LAST_OUT" | sed 's/^/    | /' | head -6
done
exit $fail
