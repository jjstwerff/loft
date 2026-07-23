# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN117 — shared helper for the headless browser-threading gates.  Sourced by
# them, never executed on its own.
#
# `await_report FILE CAP` returns as soon as the page has posted its line, or
# after CAP seconds (multiplied by $WAIT_SCALE).  Each harness page posts
# exactly ONE line and coi-server.py writes it in a single shot, so waiting for
# the first line reads what `head -1` would read anyway — only without a fixed
# sleep.  That matters at both ends: on this box the measured work takes well
# under a second against 15-30s of padding, and on a 4-vCPU CI runner a pad
# tuned to a 24-core box is the difference between a verdict and an empty report
# read as "the page produced nothing".  WAIT_SCALE stretches the cap for slow
# hardware; the cap itself still bounds a page that hangs.
#
# `stop_browser PID` ends a page and lets the machine settle before the next
# cell.  The kill is asynchronous — chrome's renderer processes outlive the
# parent by a moment — so a measurement started immediately after one ends
# partly measures the previous teardown, which showed up as a 4-worker cell
# slower than the 2-worker one.  The old fixed sleeps hid this because the pad
# WAS the settle.  $SETTLE seconds, 1 by default.
await_report() {
  local file="$1" cap=$(( $2 * ${WAIT_SCALE:-1} )) waited=0
  while [ "$waited" -lt $(( cap * 10 )) ]; do
    # A posted line is complete when it lands: the server writes `line + "\n"`
    # in one `write` under a `with open(...)`, so a non-empty file is a whole
    # line.  The short settle keeps that true if the kernel splits the write.
    if [ -s "$file" ]; then sleep 0.2; return 0; fi
    sleep 0.1
    waited=$(( waited + 1 ))
  done
  return 1
}

stop_browser() {
  kill "$1" 2>/dev/null
  wait "$1" 2>/dev/null
  sleep "${SETTLE:-1}"
}
