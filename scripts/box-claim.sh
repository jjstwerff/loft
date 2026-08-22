#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Run a long command while CLAIMING this checkout, so a sibling checkout's
# `make ci` can see it.
#
#   scripts/box-claim.sh cargo nextest run --profile ci
#
# WHY THIS EXISTS.  `make ci`'s `ci-guard` refuses a second gate in the same tree and
# WARNS about one in a sibling tree, both keyed on a live pid in `.ci-running`.  But only
# `make ci` ever wrote that file — a bare `cargo nextest run` claimed nothing and was
# invisible to the check.
#
# ⚠ A marker that covers one of the two ways to load the box is WORSE than no marker: the
# check reads as "clear" when it means "did not look".  Measured 2026-08-21 — two agents
# collided twice in one afternoon on a 24-thread box (load 66, then 42), and on the second
# collision one of the two runs was an ad-hoc `cargo nextest` loop that no claim covered.
# It then survived the kill of its own wrapper and kept relaunching, invisible throughout.
#
# The trap releases the claim on ANY exit — success, failure, or signal — because a lock
# that outlives its holder fails runs that should pass, gets deleted by hand, and is then
# trusted by nobody.  `ci-guard` also liveness-checks the pid, so the two defences agree.
set -uo pipefail
cd "$(dirname "$0")/.."

if [ $# -eq 0 ]; then
  echo "usage: scripts/box-claim.sh <command...>" >&2
  exit 2
fi

if [ -f .ci-running ] && kill -0 "$(cat .ci-running 2>/dev/null)" 2>/dev/null; then
  echo "box-claim: REFUSED — this tree is already claimed (pid $(cat .ci-running))." >&2
  exit 1
fi

for d in ../*/; do
  [ "$(cd "$d" 2>/dev/null && pwd -P)" = "$(pwd -P)" ] && continue
  [ -f "$d/.ci-running" ] || continue
  kill -0 "$(cat "$d/.ci-running" 2>/dev/null)" 2>/dev/null || continue
  echo "box-claim: WARNING — $d is also claimed (pid $(cat "$d/.ci-running"))."
  echo "  Sharing $(nproc) threads; a TIMING measurement here will be worthless."
done

echo $$ > .ci-running
trap 'rm -f .ci-running' EXIT INT TERM HUP
"$@"
