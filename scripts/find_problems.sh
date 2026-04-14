#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# One-pass-find-all-problems workflow (see doc/claude/TESTING.md).
#
# Default mode: runs `cargo test --release --no-fail-fast` in the
# background, tees the raw log to /tmp/loft_test.log (or $1), and
# lets you get on with other work.  The summary writes to
# /tmp/loft_problems.txt (or $2) when the run finishes.  Avoids the
# fix-one-rerun-see-next loop that pays the compile + test-startup
# cost on every iteration.
#
# Peek mode (no compile): `./scripts/find_problems.sh --peek` inspects
# the in-flight log (/tmp/loft_test.log) and prints any failures
# discovered so far.  Shows last script run before a SIGSEGV so
# wrap-suite crashes point at the specific .loft file that blew up.
#
# Usage:
#   ./scripts/find_problems.sh                         # run+wait (foreground)
#   ./scripts/find_problems.sh --bg                    # run in background
#   ./scripts/find_problems.sh /tmp/log /tmp/problems  # custom paths
#   ./scripts/find_problems.sh --peek                  # in-flight peek
#   ./scripts/find_problems.sh --wait                  # wait for a --bg run
#
# Reach for this any time a refactor is expected to surface multiple
# failures (e.g. after renaming a widely-used API, touching parser
# code paths, or replacing a native's signature).  For focused work
# on ONE test family, prefer a prefix filter instead:
#   cargo test --release --test issues q3_to_json
#
# Rule: never run `cargo test --release` (the full suite) in the
# foreground.  Always go through `--bg` so the blocking run does
# not occupy the terminal for 60-90 s.  `cargo clippy` and single-
# file tests stay foreground.
set -euo pipefail

LOG_DEFAULT=/tmp/loft_test.log
OUT_DEFAULT=/tmp/loft_problems.txt
PID_FILE=/tmp/loft_test.pid

# Extract a compact failure summary from the raw log.
# $1: log path, $2: output path
summarise() {
  local log="$1" out="$2"
  {
    echo "=== Test binaries that reported FAILED ==="
    grep -a -E "^test .* FAILED$" "$log" || echo "(none)"
    echo
    echo "=== Test stdout blocks for FAILED tests ==="
    grep -a -B1 -A10 "^---- " "$log" || echo "(none)"
    echo
    echo "=== SIGSEGV / signal crashes (with last context) ==="
    # For each SIGSEGV line, include the last 15 lines of context
    # before it — typically captures the last `run "tests/scripts/..."`
    # line so crashes point at a specific .loft file.
    if grep -aq "signal:" "$log"; then
      awk '
        /signal:/ {
          for (i = NR - 15; i < NR; i++) if (i > 0 && buf[i]) print buf[i]
          print "    *** " $0
          print "    ---"
        }
        { buf[NR] = $0 }
      ' "$log"
    else
      echo "(none)"
    fi
    echo
    echo "=== cargo-level target failures (compile or link) ==="
    grep -a -B1 -A3 "error: test failed\|error: .* target\(s\) failed" "$log" || echo "(none)"
    echo
    echo "=== panic! / thread panics (inline) ==="
    grep -a -B1 -A3 "thread .* panicked at" "$log" | head -80 || echo "(none)"
    # If a wrap-suite test SIGSEGV'd, cargo captured its stdout
    # into the void — re-run wrap with --nocapture to recover
    # the last `run "tests/scripts/..."` print before the crash.
    if grep -aq "wrap.* signal:" "$log" || grep -aq "test failed.*--test wrap" "$log"; then
      echo
      echo "=== wrap-suite SIGSEGV rerun with --nocapture ==="
      echo "(to recover the crashing script name)"
      cargo test --release --test wrap loft_suite -- --nocapture --test-threads=1 2>&1 \
        | grep -E '^(run |thread |test |error:|Caused|  process|Warning: [0-9]+ stores)' \
        | tail -50 || echo "(rerun failed)"
    fi
  } > "$out"
}

# `--peek`: look at the in-flight log without starting a run.
if [[ "${1:-}" == "--peek" ]]; then
  LOG="${2:-$LOG_DEFAULT}"
  if [[ ! -f "$LOG" ]]; then
    echo "no log at $LOG yet — run without --peek to start a fresh pass" >&2
    exit 1
  fi
  running="no"
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    running="yes (pid $(cat "$PID_FILE"))"
  fi
  echo "=== in-flight peek (log: $LOG, $(wc -l < "$LOG") lines, running=$running) ==="
  failures=$(grep -a -E "^test .* FAILED$" "$LOG" || true)
  segfaults=$(grep -a "signal:" "$LOG" || true)
  if [[ -z "$failures" && -z "$segfaults" ]]; then
    echo "no failures yet"
    echo "current tail:"
    tail -5 "$LOG"
    exit 0
  fi
  if [[ -n "$failures" ]]; then
    echo "$failures"
    echo
    grep -a -B1 -A10 "^---- " "$LOG" || true
  fi
  if [[ -n "$segfaults" ]]; then
    echo
    echo "SIGSEGV detected — last context before crash:"
    awk '
      /signal:/ {
        for (i = NR - 15; i < NR; i++) if (i > 0 && buf[i]) print buf[i]
        print "    *** " $0
      }
      { buf[NR] = $0 }
    ' "$LOG"
  fi
  exit 0
fi

# `--wait`: wait for a background run to finish, then summarise.
if [[ "${1:-}" == "--wait" ]]; then
  LOG="${2:-$LOG_DEFAULT}"
  OUT="${3:-$OUT_DEFAULT}"
  if [[ ! -f "$PID_FILE" ]]; then
    echo "no background run found (expected $PID_FILE)" >&2
    exit 1
  fi
  pid=$(cat "$PID_FILE")
  echo "waiting for cargo test pid $pid..."
  while kill -0 "$pid" 2>/dev/null; do sleep 2; done
  rm -f "$PID_FILE"
  summarise "$LOG" "$OUT"
  echo "wrote problems summary to $OUT"
  wc -l "$OUT"
  exit 0
fi

# `--bg`: start the run in the background and return immediately.
if [[ "${1:-}" == "--bg" ]]; then
  LOG="${2:-$LOG_DEFAULT}"
  OUT="${3:-$OUT_DEFAULT}"
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    echo "a background run is already in flight (pid $(cat "$PID_FILE"))" >&2
    echo "use --peek to inspect or --wait to block until it finishes" >&2
    exit 1
  fi
  # Tee via a subshell so the script returns after backgrounding.
  (cargo test --release --no-fail-fast > "$LOG" 2>&1
   summarise "$LOG" "$OUT") &
  echo "$!" > "$PID_FILE"
  echo "background run started (pid $!), log: $LOG, summary on finish: $OUT"
  echo "use --peek to inspect in flight, --wait to block until done"
  exit 0
fi

# Default: foreground run — stream output AND write summary.
LOG="${1:-$LOG_DEFAULT}"
OUT="${2:-$OUT_DEFAULT}"
cargo test --release --no-fail-fast 2>&1 | tee "$LOG"
summarise "$LOG" "$OUT"
echo
echo "wrote problems summary to $OUT"
wc -l "$OUT"
