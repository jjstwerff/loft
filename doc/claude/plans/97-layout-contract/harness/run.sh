#!/usr/bin/env bash
# @PLN97 — repeatable verification that store_load_url_trusted FUNCTIONS in wasm
# (--html), with the same synchronous API + fail-closed contract as native.
#
# (A) write a known .store image; (B) build the --html wasm of a program that
# URL-loads it; (C) drive that wasm in node with a MOCKED fetch through
# AsyncifyCtrl and assert valid->loads / error->false / corrupt->false.  Also
# runs the program natively via file:// as a cross-target parity check (Step 5).
#
# Needs: loft (built if absent), rustup target wasm32-unknown-unknown, wasm-opt
# (binaryen), node.  Run from anywhere in the repo.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
LOFT=${LOFT:-target/release/loft}
[ -x "$LOFT" ] || cargo build --release --bin loft
H=doc/claude/plans/97-layout-contract/harness
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

echo "== (A) write a known store image =="
HARNESS_STORE_PATH="$TMP/world.store" "$LOFT" "$H/store_write.loft"

echo "== (B) build the --html wasm =="
# `--html` writes to <input-dir>/.loft/<name>.html (ignores the out arg), so copy
# the fixture into TMP first — keeps the repo clean and the artifact under TMP.
cp "$H/urlload.loft" "$TMP/urlload.loft"
"$LOFT" --html "$TMP/urlload.loft" >/dev/null
PAGE="$TMP/.loft/urlload.html"
grep -oE 'const wasmB64="[A-Za-z0-9+/=]+"' "$PAGE" | sed 's/const wasmB64="//;s/"$//' | base64 -d > "$TMP/urlload.wasm"
echo "   wasm $(wc -c < "$TMP/urlload.wasm") bytes"

echo "== (C) drive the wasm in node with a mocked fetch (V1b / V3) =="
node "$H/harness.js" "$TMP/urlload.wasm" "$TMP/world.store"

echo "== (D) native cross-target parity via file:// (Step 5) =="
sed "s#https://harness.local/world.store#file://$TMP/world.store#" "$H/urlload.loft" > "$TMP/urlload_native.loft"
NATIVE=$("$LOFT" "$TMP/urlload_native.loft" | tail -1)
echo "   native: $NATIVE"
[ "$NATIVE" = "url keys=7,13,42" ] && echo "   PARITY OK (wasm output == native output)" || { echo "   PARITY FAIL"; exit 1; }
echo "ALL GREEN"
