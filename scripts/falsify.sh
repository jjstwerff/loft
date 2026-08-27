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

usage() {
  echo "usage: scripts/falsify.sh <guard.loft> <control-ref>" >&2
  echo "       scripts/falsify.sh --bulk <listfile>   # <guard>TAB<control-ref> per line" >&2
  exit 2
}
BULK=""
if [ "${1:-}" = "--bulk" ]; then
  [ $# -eq 2 ] || usage
  BULK="$2"; [ -f "$BULK" ] || { echo "no such list: $BULK" >&2; exit 2; }
else
  [ $# -eq 2 ] || usage
  GUARD="$1"; REF="$2"
  [ -f "$GUARD" ] || { echo "no such guard: $GUARD" >&2; exit 2; }
fi

ROOT=$(git rev-parse --show-toplevel)
CACHE="${LOFT_FALSIFY_CACHE:-${TMPDIR:-/tmp}/loft-falsify}"
if [ -z "$BULK" ]; then
  SHA=$(git rev-parse --short "$REF") || { echo "unknown ref: $REF" >&2; exit 2; }
  WT="$CACHE/$SHA"; TGT="$CACHE/$SHA-target"
fi

# ⚠ TEMPORARY, and it should come out: every `--path` below carries a TRAILING SLASH because
# `run_tests` builds the stdlib directory as `default_dir.to_string() + "default"` — a join
# with no separator, so `--path /tree` looks for `/treedefault` and says "cannot load default
# library".  That exit 1 reads as a difference and scored every `main`-less guard as falsified
# by the TREE rather than by the invocation; the first sweep over the corpus lost a quarter of
# its verdicts to it.
#
# This is a caller compensating for a contract defect, which is the kind of thing that outlives
# the memory of why it is here.  It is loft#1112 — delete the slashes when that lands.
#
# The corpus runner (`tests/wrap.rs::run_test`) runs `main` when the file HAS one and every
# zero-parameter function otherwise.  Picking the wrong one is the failure this tool exists
# to stop, so it is derived from the file rather than passed in.
entry_modes() { # <guard> ; sets MODE_I / MODE_N
  if grep -qE '^[[:space:]]*fn main[[:space:]]*\(' "$1"; then
    MODE_I=(--interpret); MODE_N=(--native)
  else
    MODE_I=(--tests); MODE_N=(--tests --native)
  fi
}
[ -n "$BULK" ] || entry_modes "$GUARD"

build() { # <dir> <target-dir> -> path to binary
  ( cd "$1" && cargo build --bin loft --target-dir "$2" >/dev/null 2>&1 ) || return 1
  echo "$2/debug/loft"
}

# Four channels, read apart.  A guard scored on one of them can be silent on the others,
# and which one moved is the thing worth printing.
signature() { # <binary> <guard-path> <extra-args…> ; prints "exit|asserts|leak|panic"
  local bin="$1" file="$2"; shift 2
  local out rc
  # `timeout` as well as `LOFT_TIMEOUT`, and the outer one is not redundant: an OLD control
  # running a NEW guard can hang somewhere loft's own watchdog does not reach, and a bulk
  # sweep then stops silently on one file.  Measured — a control run sat for ten minutes
  # against a 180 s `LOFT_TIMEOUT`.  A run the outer bound kills scores `exit 124`, which is
  # a difference like any other and says plainly which side could not finish.
  local lim="${LOFT_FALSIFY_TIMEOUT:-180}"
  out=$(timeout -k 5 "$((lim + 20))" env LOFT_NATIVE_LEAK_CHECK=1 LOFT_TIMEOUT="$lim" \
        "$bin" "$@" "$file" 2>&1); rc=$?
  local asserts leak panic
  asserts=$(echo "$out" | grep -c "assertion failed")
  leak=$(echo "$out" | grep -oE "stores not freed at program exit: .*" | head -1 | sed 's/.*exit: //')
  panic=$(echo "$out" | grep -oE "panicked at [^:]*" | head -1)
  echo "$rc|$asserts|${leak:-none}|${panic:-none}"
}

mkdir -p "$CACHE"

# ── bulk ─────────────────────────────────────────────────────────────────────────────────
# Retrofitting the corpus: one control build per REF rather than per guard, into a SHARED
# target dir so the dependency crates are compiled once (measured 61 s cold, 8.7 s warm).
# Interpret only — the native run costs a rustc invocation per file and the question here is
# "did this guard ever fail", which one backend answers.
if [ -n "$BULK" ]; then
  HERE=$(build "$ROOT" "$CACHE/head-target") || { echo "this tree does not build" >&2; exit 1; }
  SHARED="$CACHE/shared-target"
  # Read the ref list on FD 3, not stdin.  `git worktree add` and `cargo build` both read
  # stdin, and inside a `… | while read` loop they swallow the rest of the list — the first
  # sweep stopped silently after 51 of 186 refs, in order, with an exit status of 0.
  while read -r ref <&3; do
    [ -n "$ref" ] || continue
    wt="$CACHE/wt-$ref"
    if [ ! -d "$wt" ]; then
      git worktree add --detach "$wt" "$ref" >/dev/null 2>&1 </dev/null || {
        awk -F'\t' -v r="$ref" '$2==r {printf "%s\t%s\tno-worktree\t\n", $1, r}' "$BULK"; continue; }
    fi
    if ! ( cd "$wt" && cargo build --bin loft --target-dir "$SHARED" >/dev/null 2>&1 </dev/null ); then
      awk -F'\t' -v r="$ref" '$2==r {printf "%s\t%s\tno-build\t\n", $1, r}' "$BULK"
      git worktree remove --force "$wt" >/dev/null 2>&1
      continue
    fi
    while read -r g <&4; do
      # An annotation-scored file is not run for a verdict at all: the harness reads
      # `@EXPECT_ERROR` / `@EXPECT_FAIL` and a REFUSAL is its passing answer, so comparing
      # exit codes across two trees says nothing about whether the file ever caught anything.
      if grep -qE '@EXPECT_ERROR|@EXPECT_FAIL' "$ROOT/$g"; then
        printf '%s\t%s\tannotation-scored\t\n' "$g" "$ref"; continue
      fi
      entry_modes "$ROOT/$g"
      c=$(signature "$SHARED/debug/loft" "$ROOT/$g" --path "$wt/" "${MODE_I[@]}")
      h=$(signature "$HERE" "$ROOT/$g" --path "$ROOT/" "${MODE_I[@]}")
      if [ "$h" != "0|0|none|none" ]; then
        printf '%s\t%s\there-not-clean\t%s\n' "$g" "$ref" "$h"
      elif [ "$c" = "$h" ]; then
        printf '%s\t%s\tINERT\t%s\n' "$g" "$ref" "$c"
      else
        ch=""
        for i in 1 2 3 4; do
          cf=$(echo "$c" | cut -d'|' -f$i); hf=$(echo "$h" | cut -d'|' -f$i)
          [ "$cf" = "$hf" ] && continue
          case $i in
            1) d="exit $cf -> $hf";; 2) d="$cf assertion failures -> $hf";;
            3) d="leaked $cf -> clean";; 4) d="panicked -> clean";;
          esac
          [ -n "$ch" ] && ch="$ch, "; ch="$ch$d"
        done
        printf '%s\t%s\tfalsified\t%s\n' "$g" "$ref" "$ch"
      fi
    done 4< <(awk -F'\t' -v r="$ref" '$2==r {print $1}' "$BULK")
    git worktree remove --force "$wt" >/dev/null 2>&1
  done 3< <(cut -f2 "$BULK" | sort -u)
  exit 0
fi
# ─────────────────────────────────────────────────────────────────────────────────────────

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
  c=$(signature "$CONTROL" "$ROOT/$GUARD" --path "$WT/" $args)
  # `--path` for BOTH sides: the binary is built into its own target dir and has no
  # `default/` beside it, so without this it cannot load the stdlib and exits 1 — which
  # reads as a difference and would score every guard as falsified for the wrong reason.
  # shellcheck disable=SC2086
  h=$(signature "$HERE" "$ROOT/$GUARD" --path "$ROOT/" $args)
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
