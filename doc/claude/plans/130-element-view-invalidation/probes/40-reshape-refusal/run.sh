#!/bin/bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# @PLN130 F9 probe 40 — compile every cell on BOTH backends and print its value line or the
# refusal.  `LOFT_BIN=/usr/local/bin/loft ./run.sh` runs it against the installed binary, which
# is the before/after oracle (it has no refusal, so every cell there prints a value).
set -u
cd "$(dirname "$0")" || exit 1
# Repo root is six directories up from doc/claude/plans/<plan>/probes/40-reshape-refusal.
L=${LOFT_BIN:-$(cd ../../../../../.. && pwd)/target/release/loft}
[ -x "$L" ] || { echo "no loft binary at $L — build it, or set LOFT_BIN"; exit 1; }
for f in S1 S2 S3 S4 S5 S6 S7 S8 S10 S11 S12 S13 \
         X1 X2 X3 X4 X5 X6 X7 X8 X9 X13 X14 X15 X16; do
  for be in --interpret --native; do
    out=$(LOFT_TIMEOUT=60 LOFT_NO_CACHE=1 LOFT_STRICT_STORES=1 LOFT_ERRORS=compact \
          "$L" $be "$f.loft" 2>&1)
    val=$(printf '%s\n' "$out" | grep -E "^$f " | tr '\n' '|')
    # First clause only — the remedy sentence is the same for every cell.  Cut on the em dash
    # rather than a byte count, which would split it and print a replacement character.
    err=$(printf '%s\n' "$out" | grep -iE '^Error' | head -1 | sed 's/ —.*//')
    printf '%-4s %-11s %-26s %s\n' "$f" "$be" "${val:-<refused>}" "$err"
  done
done
