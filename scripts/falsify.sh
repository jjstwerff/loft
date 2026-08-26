#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Run a guard against the tree it was written to catch, and say which CHANNEL saw the
# difference.
#
#   scripts/falsify.sh tests/scripts/<guard>.loft <control-ref>
#
# A guard that passes on the build it was written for proves nothing, and the ways that
# happens are not exotic — four turned up in one afternoon (QUALITY.md § B6m): the wrong
# ENTRY POINT (a `main`-less guard under `--interpret` runs no assertion; a `main`-ful one
# under `--tests` runs the helpers), a success marker the error report ECHOES, a leak gate
# that is monotone so an over-free reads as an improvement, and a cell whose shape never
# reaches the code path it was written for.
#
# So this does not ask "does it pass now".  It builds `<control-ref>`, runs the guard THERE
# and HERE through the entry point the corpus runner would pick, and compares four channels
# separately — exit code, assertion failures, leaked stores, panic.  The verdict names the
# channel that moved, which is the fact a bare pass/fail hides.
#
# The control build is cached per ref under the scratch root, so a second guard against the
# same ref costs nothing.
set -uo pipefail

usage() { echo "usage: scripts/falsify.sh <guard.loft> <control-ref>" >&2; exit 2; }
[ $# -eq 2 ] || usage
GUARD="$1"; REF="$2"
[ -f "$GUARD" ] || { echo "no such guard: $GUARD" >&2; exit 2; }

ROOT=$(git rev-parse --show-toplevel)
CACHE="${LOFT_FALSIFY_CACHE:-${TMPDIR:-/tmp}/loft-falsify}"
SHA=$(git rev-parse --short "$REF") || { echo "unknown ref: $REF" >&2; exit 2; }
WT="$CACHE/$SHA"; TGT="$CACHE/$SHA-target"

# The corpus runner (`tests/wrap.rs::run_test`) runs `main` when the file HAS one and every
# zero-parameter function otherwise.  Picking the wrong one is the failure this tool exists
# to stop, so it is derived from the file rather than passed in.
if grep -qE '^[[:space:]]*fn main[[:space:]]*\(' "$GUARD"; then
  MODE_I=(--interpret); MODE_N=(--native)
else
  MODE_I=(--tests); MODE_N=(--tests --native)
fi

build() { # <dir> <target-dir> -> path to binary
  ( cd "$1" && cargo build --bin loft --target-dir "$2" >/dev/null 2>&1 ) || return 1
  echo "$2/debug/loft"
}

# Four channels, read apart.  A guard scored on one of them can be silent on the others,
# and which one moved is the thing worth printing.
signature() { # <binary> <extra-args…> ; prints "exit|asserts|leak|panic"
  local bin="$1"; shift
  local out rc
  out=$(LOFT_NATIVE_LEAK_CHECK=1 LOFT_TIMEOUT="${LOFT_FALSIFY_TIMEOUT:-180}" \
        "$bin" "$@" "$ROOT/$GUARD" 2>&1); rc=$?
  local asserts leak panic
  asserts=$(echo "$out" | grep -c "assertion failed")
  leak=$(echo "$out" | grep -oE "stores not freed at program exit: .*" | head -1 | sed 's/.*exit: //')
  panic=$(echo "$out" | grep -oE "panicked at [^:]*" | head -1)
  echo "$rc|$asserts|${leak:-none}|${panic:-none}"
}

mkdir -p "$CACHE"
if [ ! -x "$TGT/debug/loft" ]; then
  [ -d "$WT" ] || git worktree add --detach "$WT" "$SHA" >/dev/null 2>&1 || {
    echo "cannot create a worktree at $SHA" >&2; exit 1; }
  echo "building the control at $SHA (cached at $TGT) …" >&2
  build "$WT" "$TGT" >/dev/null || { echo "the control does not build" >&2; exit 1; }
fi
CONTROL="$TGT/debug/loft"
# A separate target dir on purpose: the main one may be mid-`make ci`, and cargo's build
# lock is per target dir — building into it stalls a gate that is already running.
HERE=$(build "$ROOT" "$CACHE/head-target") || { echo "this tree does not build" >&2; exit 1; }

fail=0
CHANNELS=""
printf '%-12s %-10s %-38s %s\n' backend tree "exit|asserts|leak|panic" verdict
for pair in "interpret ${MODE_I[*]}" "native ${MODE_N[*]}"; do
  name=${pair%% *}; args=${pair#* }
  # shellcheck disable=SC2086
  c=$(signature "$CONTROL" --path "$WT" $args)
  # `--path` for BOTH sides: the binary is built into its own target dir and has no
  # `default/` beside it, so without this it cannot load the stdlib and exits 1 — which
  # reads as a difference and would score every guard as falsified for the wrong reason.
  # shellcheck disable=SC2086
  h=$(signature "$HERE" --path "$ROOT" $args)
  clean_here="ok"
  [ "${h%%|*}" = "0" ] || clean_here="NOT-CLEAN"
  case "$h" in *"|none|none") ;; *) clean_here="NOT-CLEAN";; esac
  [ "$(echo "$h" | cut -d'|' -f2)" = "0" ] || clean_here="NOT-CLEAN"
  if [ "$c" = "$h" ]; then
    verdict="INERT — the control and this tree answer the same"
    fail=1
  elif [ "$clean_here" != "ok" ]; then
    verdict="THIS TREE IS NOT CLEAN"
    fail=1
  else
    verdict="falsified"
    # Name the channel that moved, so the recorded line says what was measured rather
    # than only that something was.
    for i in 1 2 3 4; do
      cf=$(echo "$c" | cut -d'|' -f$i); hf=$(echo "$h" | cut -d'|' -f$i)
      [ "$cf" = "$hf" ] && continue
      case $i in
        1) d="exit $cf -> $hf";;
        2) d="$cf assertion failures -> $hf";;
        3) d="leaked $cf -> clean";;
        4) d="panicked -> clean";;
      esac
      [ -n "$CHANNELS" ] && CHANNELS="$CHANNELS, "
      CHANNELS="$CHANNELS$name $d"
    done
  fi
  printf '%-12s %-10s %-38s %s\n' "$name" control "$c" ""
  printf '%-12s %-10s %-38s %s\n' "$name" here "$h" "$verdict"
done

echo
if [ $fail -eq 0 ]; then
  echo "Paste this into $GUARD:"
  echo "// @falsified-at: $SHA — $CHANNELS"
else
  echo "NOT falsified.  A guard that answers the same on the build it was written for is"
  echo "measuring something other than the defect — check the ENTRY POINT above first."
fi
exit $fail
