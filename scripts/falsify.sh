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
# and HERE through the entry point the corpus runner would pick, and compares six channels
# separately — exit code, assertion failures, leaked stores, panic, stack-store free refusals
# (`BUG (#306)`), and the guard's own `@EXPECT_ERROR` declarations.  The verdict names the
# channel that moved, which is the fact a bare pass/fail hides.
#
# A guard need only move ONE channel on ONE backend.  A backend-divergence guard cannot move
# both by construction, so an inert side is reported as expected rather than counted against
# it; what the gate is for is a guard that moves nothing anywhere (loft#1224).
#
# The control build is cached per ref under the scratch root, so a second guard against the
# same ref costs nothing.
#
# ⚠ ONE CHANNEL IS BLIND, and it is blind for the corpus's usual guard shape.  The leak column
# is read off the run's stderr ("stores not freed at program exit"), which only a `main`-ful
# run under `--interpret` prints: `--tests` does not leak-check at all (the corpus leak gate
# lives in `tests/wrap.rs`, which this does not run).  So a `main`-less guard — the standard
# form — reports `leak none` on BOTH trees whatever it leaks, and a guard written to catch a
# LEAK is therefore recorded INERT, i.e. mislabelled a lock.  Measured 2026-08-27 on
# `a-nullable-return-joins-its-branch-arms.loft`, whose leaking cell `make ci` failed on while
# this reported `0|0|none|none|0` for both trees (QUALITY.md B6p).  Until `--tests` grows a leak
# check, score a leak guard by giving it a `main` and running it under `--interpret`.
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
  # loft#1224 — an ANNOTATION-SCORED guard is run as a plain program whatever its entry point,
  # because that is the only mode whose OUTPUT carries the thing being compared.
  #
  # `--tests` does ENFORCE the annotation — a declared error that never occurs is `FAIL
  # (expected parse error but file parsed cleanly)`, exit 1, measured on all four fn-name ×
  # has-main shapes.  What it does not do is PRINT the diagnostic: a passing file reports `ok`
  # and exits 0, and the message text appears zero times.  The channel below matches on that
  # text, so scored under `--tests` a refusal guard reads identical on both trees — `0/1`, exit
  # 0 — and every such run was INERT.  The claim is checkable there; it is not COMPARABLE there.
  if grep -qE '@EXPECT_ERROR|@EXPECT_FAIL' "$1"; then
    MODE_I=(--interpret); MODE_N=(--native)
  elif grep -qE '^[[:space:]]*fn main[[:space:]]*\(' "$1"; then
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

# Six channels, read apart.  A guard scored on one of them can be silent on the others,
# and which one moved is the thing worth printing.
#
# The REFUSAL channel is here because `tests/wrap.rs` Part A2 already fails a corpus file on
# it and this script did not read it — so a guard for an ownership defect that moves only
# that channel scored INERT while `make ci` would have failed on the control.  Two
# consecutive rule-led walks (@FR-L-Null, @FR-O-Proxy) had to measure it by hand.  A
# `BUG (#306)` line means a whole-store free aimed at the eval-stack store that only the
# allocator's guard stopped, and the guard keeps the store alive — which is exactly why
# values, exit code and the leak report can all stay put while it fires.
#
# The EXPECT channel is the same lesson one guard-kind over: an annotation-scored file's
# channel is the diagnostic it declared, and reading only the five above scored it INERT
# whatever it did (loft#1224).

# How many of a guard's own `@EXPECT_ERROR` / `@EXPECT_FAIL` declarations the run produced,
# as "matched/declared" — or "-" when the file declares none.
#
# This is the channel an annotation-scored guard actually moves, and without it such a guard is
# unscoreable here: its PASSING answer IS a diagnostic, so the exit code is 1 on both trees and
# every other channel reads identical, while the thing that changed — whether the declared
# message was produced at all — goes unread.  The bulk path skips these files for exactly that
# reason; the single-guard path ran them anyway and called the result INERT (loft#1224).
expect_channel() { # <guard-path> <output> -> "matched/declared" | "-"
  local file="$1" out="$2" want matched=0 declared=0
  while IFS= read -r want; do
    [ -n "$want" ] || continue
    declared=$((declared + 1))
    case "$out" in *"$want"*) matched=$((matched + 1)) ;; esac
  done < <(sed -n 's/.*@EXPECT_\(ERROR\|FAIL\)://p' "$file" \
           | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')
  if [ "$declared" -eq 0 ]; then echo "-"; else echo "$matched/$declared"; fi
}

signature() { # <binary> <guard-path> <extra-args…> ; "exit|asserts|leak|panic|refusals|expect"
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
  local asserts leak panic refusals
  asserts=$(echo "$out" | grep -c "assertion failed")
  leak=$(echo "$out" | grep -oE "stores not freed at program exit: .*" | head -1 | sed 's/.*exit: //')
  panic=$(echo "$out" | grep -oE "panicked at [^:]*" | head -1)
  refusals=$(echo "$out" | grep -c "BUG (#306)")
  echo "$rc|$asserts|${leak:-none}|${panic:-none}|$refusals|$(expect_channel "$file" "$out")"
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
      if [ "$h" != "0|0|none|none|0" ]; then
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

# loft#1224 — the verdict is an OR across backends, not an AND, and it names the inert side.
#
# A guard for a BACKEND DIVERGENCE can only move one channel by construction: if both backends
# moved it would not be a divergence.  Scoring `fail=1` on any inert backend therefore reported
# NOT FALSIFIED for every such guard — measured on the native-only loft#1217 and loft#1222,
# where native went 1 -> 0 and interpret was correctly identical on both trees.  What the gate
# is for is a guard that moves NOTHING, so that is what it now reports; a backend that stays put
# while its sibling moves is named rather than counted as a failure.
falsified_any=0
notclean=0
INERT_SIDES=""
CHANNELS=""
printf '%-12s %-10s %-46s %s\n' backend tree "exit|asserts|leak|panic|refusals|expect" verdict
for pair in "interpret ${MODE_I[*]}" "native ${MODE_N[*]}"; do
  name=${pair%% *}; args=${pair#* }
  # shellcheck disable=SC2086
  c=$(signature "$CONTROL" "$ROOT/$GUARD" --path "$WT/" $args)
  # `--path` for BOTH sides: the binary is built into its own target dir and has no
  # `default/` beside it, so without this it cannot load the stdlib and exits 1 — which
  # reads as a difference and would score every guard as falsified for the wrong reason.
  # shellcheck disable=SC2086
  h=$(signature "$HERE" "$ROOT/$GUARD" --path "$ROOT/" $args)
  h_expect=$(echo "$h" | cut -d'|' -f6)
  # loft#1224 — "clean" means the guard PASSES, and for an annotation-scored file passing is a
  # refusal: it exits 1 and prints the message it declared.  Judging it by exit code alone
  # reported THIS TREE IS NOT CLEAN for a guard that was working exactly as written.  So a file
  # that declares expectations is clean when it produced all of them, and every other file is
  # clean when it exits 0 with nothing leaked, asserted, panicked or refused.
  clean_here="ok"
  if [ "$h_expect" = "-" ]; then
    [ "${h%%|*}" = "0" ] || clean_here="NOT-CLEAN"
    case "$h" in *"|none|none|0|-") ;; *) clean_here="NOT-CLEAN";; esac
    [ "$(echo "$h" | cut -d'|' -f2)" = "0" ] || clean_here="NOT-CLEAN"
  else
    [ "${h_expect%/*}" = "${h_expect#*/}" ] || clean_here="NOT-CLEAN"
  fi
  if [ "$c" = "$h" ]; then
    verdict="INERT — the control and this tree answer the same"
    [ -n "$INERT_SIDES" ] && INERT_SIDES="$INERT_SIDES, "
    INERT_SIDES="$INERT_SIDES$name"
  elif [ "$clean_here" != "ok" ]; then
    verdict="THIS TREE IS NOT CLEAN"
    notclean=1
  else
    verdict="falsified"
    falsified_any=1
    # Name the channel that moved, so the recorded line says what was measured rather
    # than only that something was.
    for i in 1 2 3 4 5 6; do
      cf=$(echo "$c" | cut -d'|' -f$i); hf=$(echo "$h" | cut -d'|' -f$i)
      [ "$cf" = "$hf" ] && continue
      case $i in
        1) d="exit $cf -> $hf";;
        2) d="$cf assertion failures -> $hf";;
        3) d="leaked $cf -> clean";;
        4) d="panicked -> clean";;
        5) d="$cf stack-store free refusal(s) (BUG #306) -> $hf";;
        6) d="expectations matched $cf -> $hf";;
      esac
      [ -n "$CHANNELS" ] && CHANNELS="$CHANNELS, "
      CHANNELS="$CHANNELS$name $d"
    done
  fi
  printf '%-12s %-10s %-46s %s\n' "$name" control "$c" ""
  printf '%-12s %-10s %-46s %s\n' "$name" here "$h" "$verdict"
done

echo
if [ $notclean -eq 1 ]; then
  echo "NOT falsified.  This tree does not pass the guard, so nothing here says whether the"
  echo "guard can CATCH anything — fix the tree first, then re-run."
  exit 1
elif [ $falsified_any -eq 1 ]; then
  # An inert backend beside a moved one is expected for a backend-divergence guard, so say so
  # in the recorded line rather than withholding the verdict (loft#1224).
  [ -n "$INERT_SIDES" ] && CHANNELS="$CHANNELS; $INERT_SIDES INERT (expected for a
  backend-divergence guard — only one side can move)"
  echo "Paste this into $GUARD:"
  echo "// @falsified-at: $SHA — $CHANNELS"
  exit 0
else
  echo "NOT falsified.  A guard that answers the same on the build it was written for is"
  echo "measuring something other than the defect — check the ENTRY POINT above first."
  exit 1
fi
