#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Plan-06 phase 8 browser-capability smoke test.
#
# Verifies the headless-Chrome stack supports the four capabilities loft
# needs for browser-side parallel par():
#   - SharedArrayBuffer
#   - crossOriginIsolated (COOP/COEP via HTTP server)
#   - WebGL (via SwiftShader software rasterizer — no GPU needed)
#   - Console capture (via --enable-logging=stderr)
#
# Exit 0 = all four pass.  Exit 1 = any fail.
# Output: one line "OK <CAPS-json>" or "FAIL <reason>".

set -euo pipefail

PORT="${PORT:-8765}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Pick a chrome binary.
CHROME=""
for c in google-chrome google-chrome-stable chromium chromium-browser; do
    if command -v "$c" >/dev/null 2>&1; then
        CHROME="$c"
        break
    fi
done
if [[ -z "$CHROME" ]]; then
    echo "FAIL no chrome/chromium binary in PATH"
    exit 1
fi

# Start the COOP/COEP server in the background.
python3 "$SCRIPT_DIR/coop_server.py" "$PORT" "$SCRIPT_DIR" >/dev/null 2>&1 &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT
sleep 1

# Drive the page via headless Chrome; capture stderr for console.* output.
STDERR_LOG="$(mktemp)"
trap 'rm -f "$STDERR_LOG"; kill "$SERVER_PID" 2>/dev/null || true' EXIT
DOM="$("$CHROME" --headless --no-sandbox \
    --enable-features=SharedArrayBuffer \
    --enable-logging=stderr --log-level=0 \
    --dump-dom \
    "http://127.0.0.1:$PORT/headless_caps.html" 2>"$STDERR_LOG")"

# Parse: CAPS-json from <title>, console-probe count from stderr.
CAPS="$(printf '%s' "$DOM" | grep -oE 'CAPS:\{[^}]*\}' | head -1 | sed 's/^CAPS://')"
if [[ -z "$CAPS" ]]; then
    echo "FAIL no CAPS title found"
    head -10 "$STDERR_LOG" >&2
    exit 1
fi

PROBE_COUNT="$(grep -c 'CONSOLE_PROBE' "$STDERR_LOG" || true)"
if [[ "$PROBE_COUNT" -lt 3 ]]; then
    echo "FAIL console capture: expected 3 CONSOLE_PROBE lines in stderr, got $PROBE_COUNT"
    exit 1
fi

# Sanity-parse the JSON via python (avoid jq dep).
python3 -c "
import json, sys
caps = json.loads('''$CAPS''')
fails = []
if not caps.get('sab'): fails.append('sab=false')
if not caps.get('coi'): fails.append('coi=false')
if caps.get('gl') == 'none': fails.append('gl=none')
if fails:
    print('FAIL', ','.join(fails), caps)
    sys.exit(1)
print('OK', json.dumps(caps))
"
