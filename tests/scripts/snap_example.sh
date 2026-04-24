#!/bin/bash
# Generic GL snapshot helper for any `lib/graphics/examples/*.loft` file.
#
# Launches the loft example under Xvfb, waits for its window to appear,
# lets it render a few frames, then captures a screenshot via xdotool +
# ImageMagick `import`.  Post-processes with `convert -separate -swap 0,2
# -combine` to undo the Xvfb/Mesa-swrast R↔B channel swap (see P133).
#
# Usage:
#   snap_example.sh <loft_file> <output_png> [wait_seconds] [window_name_regex] [key_script] [env]
#
# `key_script` is an optional `;`-separated sequence of `KEY@MS` steps
# sent via xdotool before the final screenshot.  MS is the number of
# milliseconds to sleep *after* pressing KEY (so the game loop can react).
# KEY uses xdotool names: `space`, `Return`, `p`, `Left`, `Right`, `F1`.
#
# `env` is an optional `;`-separated list of `KEY=VAL` pairs exported
# before launching loft.  Used by the PBR snapshot entries to set
# `LOFT_FAKE_TICKS_US` / `LOFT_FAKE_NOW_MS` so animation-driven scenes
# capture identically on every run.  Opt-in per entry — time-sensitive
# examples like Brick Buster that pre-date the env vars keep rendering
# with a real clock.
#
# Example:
#   snap_example.sh brick-buster.loft /tmp/play.png 0.5 "rick Buster" "space@500"
#     → opens the game, waits 0.5s, sends SPACE, waits 500ms, captures.
#
# Examples:
#   snap_example.sh lib/graphics/examples/25-brick-buster.loft /tmp/brk.png
#   snap_example.sh lib/graphics/examples/10-2d-canvas.loft /tmp/canvas.png 2 "canvas"
#
# Run under Xvfb: `xvfb-run -a -s "-screen 0 800x600x24" snap_example.sh …`
#
# Exit codes:
#   0 — screenshot captured successfully
#   1 — window never appeared, capture failed, or loft exited
#        before a screenshot could be taken
#   2 — bad arguments
set -e

if [ "$#" -lt 2 ]; then
  echo "usage: $0 <loft_file> <output_png> [wait_seconds] [window_name_regex]" >&2
  exit 2
fi

LOFT_FILE="$1"
OUTPUT="$2"
WAIT_SECONDS="${3:-1}"
WINDOW_NAME="${4:-.}"
KEY_SCRIPT="${5:-}"
ENV_VARS="${6:-}"

# TSV empty-column placeholder.  `IFS=$'\t'` collapses adjacent tabs so
# middle columns use `-` to mean "no value"; normalise here so both
# callers (test_gl_snapshots.sh and direct invocations) agree.
[ "$KEY_SCRIPT" = "-" ] && KEY_SCRIPT=""
[ "$ENV_VARS" = "-" ] && ENV_VARS=""

if [ ! -f "$LOFT_FILE" ]; then
  echo "FAIL: loft file not found: $LOFT_FILE" >&2
  exit 2
fi

cd "$(dirname "$0")/../.."

LOG="/tmp/loft_snap_$$.log"

# Export any `KEY=VAL;...` pairs from the TSV entry's env column.  Used
# by PBR snapshot entries to set LOFT_FAKE_TICKS_US / LOFT_FAKE_NOW_MS
# so animation-driven scenes capture identically on every run.  Scoped
# to this process and any children (i.e. the loft binary below).
if [ -n "$ENV_VARS" ]; then
  IFS=';' read -ra _EVS <<<"$ENV_VARS"
  for pair in "${_EVS[@]}"; do
    [ -z "$pair" ] && continue
    case "$pair" in
      *=*) export "$pair" ;;
      *)
        echo "FAIL: bad env entry '$pair' (use KEY=VAL)" >&2
        exit 2 ;;
    esac
  done
fi

# Launch the example.  Redirect both streams so the test log captures any
# loft-side warnings (e.g. font not loaded) that should be surfaced on
# snapshot failure.
./target/release/loft --interpret \
  --path "$(pwd)/" --lib "$(pwd)/lib/" \
  "$LOFT_FILE" >"$LOG" 2>&1 &
LOFT_PID=$!

# Poll for the window to appear, up to 5 seconds.
WIN_ID=""
for _ in 1 2 3 4 5 6 7 8 9 10; do
  sleep 0.5
  WIN_ID=$(xdotool search --name "$WINDOW_NAME" 2>/dev/null | tail -1)
  [ -n "$WIN_ID" ] && break
done

# Let the example render a few frames before capturing.
sleep "$WAIT_SECONDS"

# Play scripted key input — each step is "KEY@MS".  winit under Xvfb
# delivers key events to whichever window has X keyboard focus, so we
# focus the loft window once up-front (no WM is running, so
# `windowactivate` would fail — `windowfocus` is enough and does not
# require a window manager).
if [ -n "$KEY_SCRIPT" ]; then
  xdotool windowfocus "$WIN_ID" 2>/dev/null || true
  IFS=';' read -ra _STEPS <<<"$KEY_SCRIPT"
  for step in "${_STEPS[@]}"; do
    key="${step%@*}"
    ms="${step##*@}"
    if [ "$key" = "$step" ] || [ -z "$ms" ]; then
      echo "FAIL: bad key-script step '$step' (use KEY@MS)" >&2
      kill "$LOFT_PID" 2>/dev/null || true
      wait "$LOFT_PID" 2>/dev/null || true
      rm -f "$LOG"
      exit 2
    fi
    # `xdotool key` sends a full press+release pair.  Under Xvfb the
    # press is held long enough (~20-30ms, spanning 1-2 frames at 60fps)
    # for edge-detected handlers in the target program to latch the
    # transition.  Longer holds are achieved by scheduling successive
    # steps rather than splitting keydown/keyup.
    xdotool key "$key" 2>/dev/null || true
    # Convert ms → seconds with awk so fractional sleeps work everywhere.
    sleep "$(awk "BEGIN {printf \"%.3f\", $ms / 1000.0}")"
  done
fi

if [ -z "$WIN_ID" ]; then
  echo "FAIL: no loft window matched regex '$WINDOW_NAME'" >&2
  cat "$LOG" >&2
  kill "$LOFT_PID" 2>/dev/null || true
  wait "$LOFT_PID" 2>/dev/null || true
  rm -f "$LOG"
  exit 1
fi

import -window "$WIN_ID" "$OUTPUT" 2>/tmp/loft_import_$$.log
IMPORT_RC=$?

kill "$LOFT_PID" 2>/dev/null || true
wait "$LOFT_PID" 2>/dev/null || true

if [ "$IMPORT_RC" -ne 0 ]; then
  echo "FAIL: import failed" >&2
  cat "/tmp/loft_import_$$.log" >&2
  rm -f "$LOG" "/tmp/loft_import_$$.log"
  exit 1
fi

# P133: swap R↔B post-capture (Xvfb/Mesa-swrast framebuffer artifact).
convert "$OUTPUT" -separate -swap 0,2 -combine "$OUTPUT" \
  2>>"/tmp/loft_import_$$.log" || {
    echo "FAIL: channel-swap post-process failed" >&2
    cat "/tmp/loft_import_$$.log" >&2
    rm -f "$LOG" "/tmp/loft_import_$$.log"
    exit 1
  }

rm -f "$LOG" "/tmp/loft_import_$$.log"
exit 0
