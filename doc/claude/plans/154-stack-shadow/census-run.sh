#!/bin/bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN154 phase 0 — run the corpus under the stack-write census.
#
# Usage:  bash doc/claude/plans/154-stack-shadow/census-run.sh <outdir>
# Writes: <outdir>/per-script.tsv   file, ops, put_stack bytes, other bytes
#         <outdir>/per-op.tsv       file, opcode, unattributed bytes, ops
#
# SERIAL on purpose: several corpus scripts create and delete real files, which is why
# `tests/wrap.rs` serialises them behind WRAP_LOCK.  The op budget is what makes a full
# sweep tractable — without it the handful of scripts that run tens of millions of ops
# eat the wall clock and then report nothing, because the timeout kills them before exit.
S="$1"
: > "$S/per-script.tsv"
: > "$S/per-op.tsv"
for f in tests/scripts/*.loft; do
  out=$(LOFT_TIMEOUT=25 LOFT_STACK_CENSUS=1 LOFT_STACK_CENSUS_MAX_OPS=200000 timeout 20 ./target/release/loft --interpret "$f" 2>&1 >/dev/null)
  line=$(printf '%s\n' "$out" | grep -m1 '^stack census: ')
  if [ -z "$line" ]; then
    printf '%s\t0\t0\t0\n' "$f" >> "$S/per-script.tsv"
    continue
  fi
  ops=$(printf '%s' "$line" | sed 's/^stack census: \([0-9]*\) ops.*/\1/')
  put=$(printf '%s\n' "$out" | sed -n 's/^  via put_stack : *\([0-9]*\).*/\1/p')
  oth=$(printf '%s\n' "$out" | sed -n 's/^  other routes  : *\([0-9]*\).*/\1/p')
  printf '%s\t%s\t%s\t%s\n' "$f" "$ops" "${put:-0}" "${oth:-0}" >> "$S/per-script.tsv"
  printf '%s\n' "$out" | sed -n 's/^    op *\([0-9]*\)  \([^ ]*\) *\([0-9]*\) bytes  over *\([0-9]*\) ops/\2\t\3\t\4/p' \
    | while IFS=$'\t' read -r name bytes nops; do
        printf '%s\t%s\t%s\t%s\n' "$f" "$name" "$bytes" "$nops" >> "$S/per-op.tsv"
      done
done
echo DONE
