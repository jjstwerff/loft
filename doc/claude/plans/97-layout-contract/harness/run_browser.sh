#!/usr/bin/env bash
# @PLN97 Step 4 — the DEFINITIVE "functions in wasm" proof: a REAL fetch() round
# trip in headless Chromium.  Serves a .store image over HTTP, loads the --html
# page (which fetch()es it) in headless chromium, and asserts the rendered DOM
# shows the loaded value.  Unlike run.sh (node + a MOCK fetch), this exercises the
# actual browser fetch() + the asyncify unwind/rewind against a live network.
#
# Needs: loft, rustup wasm32-unknown-unknown, wasm-opt, python3, chromium.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
LOFT=${LOFT:-target/release/loft}
[ -x "$LOFT" ] || cargo build --release --bin loft
CHROMIUM=${CHROMIUM:-$(command -v chromium || command -v google-chrome || command -v chromium-browser)}
[ -n "$CHROMIUM" ] || { echo "SKIP: no chromium/chrome found"; exit 0; }
H=doc/claude/plans/97-layout-contract/harness
TMP=$(mktemp -d); SERVE="$TMP/serve"; mkdir -p "$SERVE"
trap 'rm -rf "$TMP"; [ -n "${SRV:-}" ] && kill "$SRV" 2>/dev/null || true' EXIT

echo "== serve a known store image =="
HARNESS_STORE_PATH="$SERVE/world.store" "$LOFT" "$H/store_write.loft" >/dev/null

echo "== build the --html page with a RELATIVE url (fetch resolves to the served store) =="
sed 's#https://harness.local/world.store#world.store#' "$H/urlload.loft" > "$TMP/urlload_http.loft"
"$LOFT" --html "$TMP/urlload_http.loft" >/dev/null 2>&1
cp "$TMP/.loft/urlload_http.html" "$SERVE/index.html"

echo "== serve over HTTP + load in headless chromium (real fetch) =="
PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
python3 -m http.server "$PORT" --directory "$SERVE" >/dev/null 2>&1 & SRV=$!
sleep 1
timeout 60 "$CHROMIUM" --headless --no-sandbox --disable-gpu \
  --virtual-time-budget=15000 --dump-dom "http://127.0.0.1:$PORT/" 2>/dev/null > "$TMP/dom.html"

echo "== assert the RUNTIME output loaded the store (not present in page source) =="
if grep -q 'url keys=7,13,42' "$TMP/dom.html" && ! grep -q 'url keys=7,13,42' "$SERVE/index.html"; then
  echo "STEP 4 PASS — real fetch() round-trip loaded the store: $(grep -oE 'url keys=[0-9,]+' "$TMP/dom.html" | head -1)"
else
  echo "STEP 4 FAIL"; sed -n 's#.*<pre id="out">\([^<]*\).*#  <pre>=\1#p' "$TMP/dom.html" | head; exit 1
fi
