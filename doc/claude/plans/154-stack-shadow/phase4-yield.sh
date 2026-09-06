#!/bin/bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN154 phase 4 — what the shadow FINDS, measured against the falsification corpus.
#
# Usage:  bash doc/claude/plans/154-stack-shadow/phase4-yield.sh <outdir> [max-refs]
# Writes: <outdir>/yield.tsv     ref, guard, control-reports, head-reports, verdict
#         <outdir>/summary.txt
#
# Every guard in `tests/scripts/` that records a real `@falsified-at:` ref names a BUILD the
# guard was written to catch.  Running the shadow on each of those builds answers the only
# question that matters about a detector: on how many of the defects the project already
# knows about does it speak?
#
# TWO directions, and the second is the gate:
#
#   * the YIELD — a report on the CONTROL is a defect this instrument would have caught, and
#     the number is the report;
#   * the GATE — a report on HEAD, where the guard passes, is a FALSE POSITIVE and is red.
#     (Three exceptions carry their own reason: loft#1373 / #1377 / #1384 are still OPEN, so
#     HEAD is a broken build for those and a report there is the yield, not a fault.)
#
# COST.  200 distinct refs at roughly two minutes each with a shared target directory is
# several hours, so `max-refs` takes the refs covering the MOST guards first and a partial run
# states its own sampling.  The worktrees are removed as it goes; the target directory is
# shared, so the disk holds one build at a time and cargo keeps the dependency artefacts.
set -uo pipefail
OUT="${1:?usage: phase4-yield.sh <outdir> [max-refs]}"
MAX="${2:-0}"
ROOT=$(git rev-parse --show-toplevel)
# The HEAD side of every comparison.  A COPY by default is not paranoia: this run takes hours
# and `target/release/loft` is rebuilt by any gate that happens to start meanwhile, so the
# binary the first ref was scored against would not be the one the last ref sees.
HEAD_BIN="${LOFT_HEAD_BIN:-$ROOT/target/release/loft}"
mkdir -p "$OUT"
: > "$OUT/yield.tsv"
SHADOW="${LOFT_SHADOW_CACHE:-${TMPDIR:-/tmp}/loft-shadow}"
# A copied binary needs to be told where the stdlib is; the in-tree one finds it itself.
HEAD_PATH="${LOFT_HEAD_PATH:-}"

# guard <TAB> ref, for every guard that records a real one.
list=$(grep -H -o '@falsified-at: *[0-9a-f]\{7,\}' tests/scripts/*.loft \
        | sed 's/:.*@falsified-at: */\t/' | sort -u)

# Refs by how many guards they cover, most first: a partial run then buys the most evidence.
refs=$(printf '%s\n' "$list" | cut -f2 | sort | uniq -c | sort -rn | awk '{print $2}')
[ "$MAX" -gt 0 ] && refs=$(printf '%s\n' "$refs" | head -n "$MAX")
# The CALIBRATION ref goes in whatever the cut: `64437246` is the build @PLN154 phase 1 was
# falsified against, so a run that scores it anything but CAUGHT is a broken harness rather
# than a clean corpus.  Prepended, so a partial run learns that first.
CAL="${LOFT_YIELD_CALIBRATION:-64437246}"
printf '%s\n' "$refs" | grep -qx "$CAL" || refs=$(printf '%s\n%s\n' "$CAL" "$refs")

# THE ENTRY POINT IS DERIVED, NOT PASSED — `falsify.sh`'s first lesson, and the one that
# costs a sweep its verdicts: the corpus runner runs `main` when the file has one and every
# zero-parameter function otherwise, so a `main`-less guard run as a plain program executes
# almost nothing.  Measured: `a-nullable-local-…` runs 19 test functions under `--tests` and
# exits 0 having run none of them under a bare `--interpret`.
run_guard() { # <binary> <stdlib-path-or-empty> <guard> ; echoes the report count
  local bin="$1" path="$2" g="$3" p="" mode="--interpret" args=""
  [ -n "$path" ] && p="--path $path"
  grep -q '^fn main()' "$g" || mode="--tests"
  # `// @ARGS: --lib <dir>` is how a guard says where its fixtures are.  Without it the file
  # does not COMPILE, which prints no report line and scores `silent` — the same vacuity as
  # the wrong entry point, wearing different clothes.
  args=$(sed -n 's|^// @ARGS:||p' "$g" | head -1)
  # shellcheck disable=SC2086
  local err
  # shellcheck disable=SC2086
  err=$(LOFT_TIMEOUT=45 LOFT_VERIFY_STACK=1 timeout 60 $bin $p $args $mode "$g" 2>&1 >/dev/null)
  # A run that could not even LOAD is not a silent run.  This is the failure a sweep cannot
  # see from its own output: the binary exits 1, prints nothing the grep matches, and the
  # zero reads as evidence.
  case "$err" in
    *"cannot load standard library"*) echo VACUOUS; return;;
  esac
  printf '%s\n' "$err" | grep -c '^stack verify: get' || true
}

for ref in $refs; do
  echo "== $ref" >&2
  built=$(bash "$ROOT/doc/claude/plans/154-stack-shadow/shadow-control.sh" "$ref" "$SHADOW" 2>/dev/null)
  bin=$(printf '%s\n' "$built" | sed -n 1p)
  path=$(printf '%s\n' "$built" | sed -n 2p)
  if [ ! -x "$bin" ]; then
    printf '%s\t-\t-\t-\tCONTROL-BUILD-FAILED\n' "$ref" >> "$OUT/yield.tsv"
    continue
  fi
  printf '%s\n' "$list" | awk -F'\t' -v r="$ref" '$2==r {print $1}' | while read -r g; do
    c=$(run_guard "$bin" "$path" "$g")
    h=$(run_guard "$HEAD_BIN" "$HEAD_PATH" "$g")
    if [ "$c" = VACUOUS ] || [ "$h" = VACUOUS ]; then v=VACUOUS
    elif [ "${h:-0}" -gt 0 ]; then v=FALSE-POSITIVE
    elif [ "${c:-0}" -gt 0 ]; then v=CAUGHT
    else v=silent; fi
    printf '%s\t%s\t%s\t%s\t%s\n' "$ref" "$g" "${c:-0}" "${h:-0}" "$v" >> "$OUT/yield.tsv"
  done
  # One build at a time on the disk: the tree is reproducible from the ref.
  rm -rf "${SHADOW:?}/$(git rev-parse --short "$ref")"
done

{
  echo "@PLN154 phase 4 — yield against the falsification corpus"
  echo
  awk -F'\t' '{c[$5]++} END {for (k in c) printf "  %-22s %d\n", k, c[k]}' "$OUT/yield.tsv"
  echo
  echo "refs attempted: $(printf '%s\n' "$refs" | wc -l) of $(printf '%s\n' "$list" | cut -f2 | sort -u | wc -l)"
  echo "guards scored:  $(wc -l < "$OUT/yield.tsv")"
} | tee "$OUT/summary.txt"
