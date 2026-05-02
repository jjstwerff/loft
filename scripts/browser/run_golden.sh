#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Plan-06 phase 8 — golden-image renderer + comparator for headless WebGL.
#
# Renders an HTML page in headless Chrome, captures a PNG via Chrome's
# --screenshot flag, and (if a golden PNG exists) reports the per-pixel
# diff.  Used for browser-side WebGL regression tests.
#
# Usage:
#   run_golden.sh capture <name>       — render <name>.html, write
#                                         /tmp/loft_golden_<name>.png
#   run_golden.sh check <name> <ref>   — render + compare against <ref>
#                                         (PNG path).  Exit 0 if pixel-exact,
#                                         1 if diff (and write
#                                         /tmp/loft_golden_<name>_diff.png).
#   run_golden.sh update <name> <ref>  — render + overwrite <ref> with the
#                                         fresh image (after manual review).
#
# Requires: google-chrome, python3, python3-PIL (Pillow) for diff.
#
# Tolerance: pixel-exact today; SwiftShader is deterministic across runs
# on the same Chrome version.  Add per-pixel epsilon if cross-version
# stability becomes a concern (currently unnecessary).

set -euo pipefail

PORT="${PORT:-8765}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CHROME=""
for c in google-chrome google-chrome-stable chromium chromium-browser; do
    if command -v "$c" >/dev/null 2>&1; then CHROME="$c"; break; fi
done
[[ -z "$CHROME" ]] && { echo "FAIL no chrome binary"; exit 1; }

mode="${1:-}"
name="${2:-}"
ref="${3:-}"

case "$mode" in
    capture|check|update) ;;
    *) echo "usage: $0 {capture|check|update} <name> [<ref.png>]"; exit 2 ;;
esac
[[ -z "$name" ]] && { echo "usage: $0 $mode <name> [<ref.png>]"; exit 2; }

# Start COOP/COEP server (also fine for non-SAB pages — overhead is
# negligible).
python3 "$SCRIPT_DIR/coop_server.py" "$PORT" "$SCRIPT_DIR" >/dev/null 2>&1 &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT
sleep 1

OUT="/tmp/loft_golden_${name}.png"
rm -f "$OUT"

# Headless render + screenshot.  --window-size locks the canvas viewport;
# --hide-scrollbars keeps the page chrome out of the screenshot.
"$CHROME" --headless --no-sandbox \
    --hide-scrollbars \
    --window-size=256,256 \
    --screenshot="$OUT" \
    --virtual-time-budget=2000 \
    "http://127.0.0.1:$PORT/${name}.html" >/dev/null 2>&1

# Headless Chrome's --screenshot path can race with the comparator
# below: chrome exits as soon as the in-memory bitmap is encoded, but
# the OS may still be flushing the PNG bytes to disk.  Validate the
# file is a complete, openable PNG before proceeding.
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
    if [[ -f "$OUT" ]] && python3 -c "
from PIL import Image
import sys
try:
    img = Image.open('$OUT')
    img.load()
except Exception:
    sys.exit(1)
" 2>/dev/null; then
        break
    fi
    sleep 0.2
done
if [[ ! -f "$OUT" ]]; then
    echo "FAIL chrome did not produce screenshot at $OUT"
    exit 1
fi

case "$mode" in
    capture)
        echo "OK captured: $OUT  ($(stat -c%s "$OUT") bytes)"
        ;;
    check)
        [[ -z "$ref" ]] && { echo "usage: $0 check <name> <ref.png>"; exit 2; }
        [[ ! -f "$ref" ]] && { echo "FAIL reference $ref not found"; exit 1; }
        python3 - "$OUT" "$ref" "/tmp/loft_golden_${name}_diff.png" <<'PYEOF'
import sys
from PIL import Image, ImageChops
got, ref, diff_path = sys.argv[1:4]
a = Image.open(got).convert('RGB')
b = Image.open(ref).convert('RGB')
if a.size != b.size:
    print(f"FAIL size mismatch: got {a.size}, ref {b.size}")
    sys.exit(1)
diff = ImageChops.difference(a, b)
bbox = diff.getbbox()
if bbox is None:
    print("OK pixel-exact")
    sys.exit(0)
# Count differing pixels (any channel non-zero).
diff_pixels = sum(1 for px in diff.getdata() if px != (0, 0, 0))
diff.save(diff_path)
print(f"FAIL {diff_pixels} differing pixels; bbox={bbox}; diff written to {diff_path}")
sys.exit(1)
PYEOF
        ;;
    update)
        [[ -z "$ref" ]] && { echo "usage: $0 update <name> <ref.png>"; exit 2; }
        cp -f "$OUT" "$ref"
        echo "OK updated $ref from $OUT"
        ;;
esac
