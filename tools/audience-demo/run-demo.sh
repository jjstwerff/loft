#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN6 — one-shot launcher for the audience generative-art demo.
#
# Makes the demo "just work on any wifi": it INSPECTS THE CURRENT LAN ADDRESS,
# regenerates the scan-to-join QR for that address, starts the single-port
# server, and opens the OpenGL projector pointed at the same address.  Nothing
# is hard-coded to one network — re-run it after switching wifi and the QR +
# projector pick up the new IP automatically.
#
#   tools/audience-demo/run-demo.sh                 # detect IP, QR, server + projector
#   tools/audience-demo/run-demo.sh --no-projector  # server + QR only (headless / projector elsewhere)
#   tools/audience-demo/run-demo.sh --no-server      # projector only (server runs on another machine)
#   tools/audience-demo/run-demo.sh --ip 192.168.1.7 # force the LAN IP (skip detection)
#   tools/audience-demo/run-demo.sh --fullscreen     # projector fullscreen (honours LOFT_GL_MONITOR)
#   tools/audience-demo/run-demo.sh --port 9000 --qr /tmp/join.png
#
# The phone page auto-derives its WebSocket from location.host, so phones on the
# same wifi just open  http://<lan-ip>:<port>/  (or scan the QR).
set -euo pipefail

# ── Repo root + loft binary ──────────────────────────────────────────────────
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOFT="$ROOT/target/release/loft"

# ── Defaults / flags ─────────────────────────────────────────────────────────
PORT="${PORT:-18083}"
QR_OUT="${LOFT_PROJ_QR:-/tmp/audience-join-qr.png}"
IP_OVERRIDE=""
RUN_SERVER=1
RUN_PROJECTOR=1
FULLSCREEN=""

usage() { sed -n '3,28p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --no-projector) RUN_PROJECTOR=0 ;;
    --no-server)    RUN_SERVER=0 ;;
    --fullscreen)   FULLSCREEN=1 ;;
    --ip)           IP_OVERRIDE="${2:?--ip needs an address}"; shift ;;
    --port)         PORT="${2:?--port needs a number}"; shift ;;
    --qr)           QR_OUT="${2:?--qr needs a path}"; shift ;;
    -h|--help)      usage 0 ;;
    *) echo "unknown option: $1" >&2; usage 1 ;;
  esac
  shift
done

# ── Detect the current LAN IPv4 ──────────────────────────────────────────────
# Preference order: (1) a wireless interface (what phones share), (2) the source
# address of the default route (the active outbound interface), (3) the first
# private RFC-1918 address on a real (non-virtual) interface.
detect_lan_ip() {
  local addr iface d
  for d in /sys/class/net/*/wireless; do
    [ -e "$d" ] || continue
    iface="$(basename "$(dirname "$d")")"
    addr="$(ip -4 -o addr show "$iface" 2>/dev/null | grep -oP 'inet \K[0-9.]+' | head -1)"
    [ -n "$addr" ] && { printf '%s\n' "$addr"; return 0; }
  done
  addr="$(ip route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[0-9.]+' | head -1)"
  [ -n "$addr" ] && { printf '%s\n' "$addr"; return 0; }
  ip -4 -o addr show 2>/dev/null | grep -vE '\bdocker|\bbr-|\bveth|\blo\b' \
    | grep -oP 'inet \K[0-9.]+' \
    | grep -E '^(192\.168|10\.|172\.(1[6-9]|2[0-9]|3[01]))\.' | head -1
}

IP="${IP_OVERRIDE:-$(detect_lan_ip || true)}"
if [ -z "$IP" ]; then
  echo "ERROR: could not detect a LAN IPv4 address. Pass one with --ip <addr>." >&2
  exit 1
fi
URL_HTTP="http://$IP:$PORT/"
URL_WS="ws://$IP:$PORT/ws"

# ── Regenerate the scan-to-join QR for the current address ───────────────────
make_qr() {
  local url="$1" out="$2"
  if python3 -c 'import qrcode' >/dev/null 2>&1; then
    python3 - "$url" "$out" <<'PY'
import sys, qrcode
url, out = sys.argv[1], sys.argv[2]
qr = qrcode.QRCode(border=2, box_size=12)
qr.add_data(url); qr.make(fit=True)
qr.make_image(fill_color="black", back_color="white").save(out)
PY
    return 0
  fi
  if command -v qrencode >/dev/null 2>&1; then
    qrencode -s 12 -m 2 -o "$out" "$url"; return 0
  fi
  # Online fallback (needs internet, not just the LAN).
  local enc
  enc="$(python3 -c 'import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))' "$url" 2>/dev/null || echo "$url")"
  curl -fsS -m 8 "https://api.qrserver.com/v1/create-qr-code/?size=600x600&qzone=2&data=$enc" -o "$out"
}

QR_OK=0
if make_qr "$URL_HTTP" "$QR_OUT" 2>/dev/null && [ -s "$QR_OUT" ]; then
  QR_OK=1
else
  echo "WARNING: could not generate a QR (install the python 'qrcode' module or 'qrencode')." >&2
  echo "         The short URL below still works — type it into the phone browser." >&2
fi

# ── Build the loft binary if absent ──────────────────────────────────────────
if [ ! -x "$LOFT" ]; then
  echo "loft binary not found — building (cargo build --release)…"
  ( cd "$ROOT" && cargo build --release )
fi

# ── Start the single-port server (unless told not to / already up) ───────────
STARTED_SERVER=0
SERVER_PID=""
cleanup() { [ "$STARTED_SERVER" = 1 ] && [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

server_listening() { ss -ltn 2>/dev/null | grep -q ":$PORT "; }

if [ "$RUN_SERVER" = 1 ]; then
  if server_listening; then
    echo "server already listening on :$PORT — reusing it."
  else
    echo "starting server on :$PORT …"
    ( cd "$ROOT" && exec "$LOFT" --interpret --no-warnings --lib lib/ \
        tools/audience-demo/single_port_server.loft ) &
    SERVER_PID=$!
    STARTED_SERVER=1
    for _ in $(seq 1 50); do server_listening && break; sleep 0.2; done
    server_listening || { echo "ERROR: server did not come up on :$PORT" >&2; exit 1; }
  fi
fi

# ── Banner ───────────────────────────────────────────────────────────────────
cat <<BANNER

  ┌────────────────────────────────────────────────┐
  │  AUDIENCE DEMO — phones on the SAME WIFI open:   │
  │                                                  │
  │      $URL_HTTP
  │                                                  │
  $( [ "$QR_OK" = 1 ] && printf '│  scan-to-join QR: %-30s│\n' "$QR_OUT" )
  └────────────────────────────────────────────────┘
  (If a phone can't connect, allow inbound TCP $PORT in the firewall.)

BANNER

# ── Projector (foreground; owns the GL window) ───────────────────────────────
if [ "$RUN_PROJECTOR" = 1 ]; then
  echo "opening projector (native; first run compiles, ~1 min)…"
  export LOFT_PROJ_SERVER="$URL_WS"
  [ "$QR_OK" = 1 ] && export LOFT_PROJ_QR="$QR_OUT"
  [ -n "$FULLSCREEN" ] && export LOFT_PROJ_FULLSCREEN=1
  ( cd "$ROOT" && exec "$LOFT" --no-warnings --lib lib/ tools/audience-demo/projector.loft )
elif [ "$STARTED_SERVER" = 1 ]; then
  echo "server running — press Ctrl-C to stop."
  wait "$SERVER_PID"
fi
