#!/bin/bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN154 phase 1 — run the corpus under the stack shadow.
#
# Usage:  bash doc/claude/plans/154-stack-shadow/verify-run.sh <outdir> [binary] [stdlib-path]
# Writes: <outdir>/per-script.tsv   file, verdict, reads, sites
#         <outdir>/reports.txt      every report line, prefixed by its file
#
# The gate is SILENCE: a report on a corpus program is either a find or a false positive,
# and both have to be looked at.  A script that times out or refuses to compile is recorded
# as such rather than counted clean — a run that never reached a `get_stack` proves nothing.
#
# SERIAL for the same reason the phase-0 census is: several corpus scripts create and
# delete real files, which is why `tests/wrap.rs` serialises them behind WRAP_LOCK.
S="$1"
LOFT="${2:-./target/release/loft}"
PATH_ARG=""
[ -n "${3:-}" ] && PATH_ARG="--path $3"
: > "$S/per-script.tsv"
: > "$S/reports.txt"
for f in tests/scripts/*.loft; do
  out=$(LOFT_TIMEOUT=45 LOFT_VERIFY_STACK=1 timeout 60 $LOFT $PATH_ARG --interpret "$f" 2>&1 >/dev/null)
  rc=$?
  verdict=$(printf '%s\n' "$out" | grep -m1 '^stack verify: ')
  reports=$(printf '%s\n' "$out" | grep -c '^stack verify: get')
  if [ -z "$verdict" ]; then
    printf '%s\tNO-VERDICT(rc=%s)\t0\t0\n' "$f" "$rc" >> "$S/per-script.tsv"
    continue
  fi
  case "$verdict" in
    *"no uninitialised"*) printf '%s\tclean\t0\t0\n' "$f" >> "$S/per-script.tsv" ;;
    *) n=$(printf '%s' "$verdict" | sed 's/^stack verify: \([0-9]*\) uninitialised.*/\1/')
       sites=$(printf '%s' "$verdict" | sed 's/.*, \([0-9]*\) distinct.*/\1/')
       printf '%s\tREPORTS\t%s\t%s\n' "$f" "$n" "$sites" >> "$S/per-script.tsv"
       printf '%s\n' "$out" | grep '^stack verify: get' | sed "s|^|$f\t|" >> "$S/reports.txt" ;;
  esac
done
echo DONE
