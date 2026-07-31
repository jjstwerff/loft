#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# loft#709 — headless gate for a builtin the browser target cannot serve.
#
# A canvas cannot do everything a desktop window can, so some calls have no
# browser handler.  That is fine.  What is not fine is learning it at BUILD
# time: whether a call can be served is a fact about this run on this target,
# not about whether the program is well-formed, and a build refusal forces one
# source to fork into two entry points differing only in which calls they may
# NAME — which destroys the property that makes two renderers worth having
# (each is the other's control).
#
# Three claims, each measured rather than assumed:
#   1. `loft --html` on a program that names an unserviceable builtin WRITES a
#      page (it used to exit 1 and write nothing).
#   2. It says which calls those are — the diagnosis was the good half.
#   3. The page instantiates in a real browser and the call returns its declared
#      zero, so the program's own `if !ok` branch is what runs.  This is the
#      claim that matters: a page that builds but dies at instantiate with a
#      LinkError naming an import index (loft#668) would pass 1 and 2.
#
# Self-contained: the probe declares its own `#native` import, so the gate needs
# no registry package and cannot drift with one.
#
# Requires: a built loft, python3, a headless-capable chromium.
# Exits 0 on pass, 1 on fail.
#
#   tests/wasm/html-unavailable-builtin.sh
set -u
cd "$(dirname "$0")/../.."
LOFT=target/release/loft
PORT="${PORT:-8797}"
WORK="$(mktemp -d)"
CHROME="$(command -v chromium || command -v chromium-browser || command -v google-chrome || echo chromium)"
trap 'rm -rf "$WORK"' EXIT
if [ ! -x "$LOFT" ]; then echo "SKIP: no $LOFT — run 'cargo build --release' first"; exit 0; fi
if ! command -v "$CHROME" >/dev/null 2>&1; then echo "SKIP: no headless chromium"; exit 0; fi
if ! command -v python3 >/dev/null 2>&1; then echo "SKIP: no python3"; exit 0; fi

cat > "$WORK/probe.loft" <<'LOFT'
// A builtin with no browser handler.  The signature already carries the answer:
// it returns whether it worked, and every correct caller checks that anyway —
// a screenshot can fail for a dozen reasons besides the target.
pub fn gl_absent_probe(width: integer) -> boolean;
#native "loft_gl_absent_probe"

fn main() {
  if gl_absent_probe(7) { println("probe: served"); }
  else { println("probe: unavailable"); }
}
LOFT

echo "== building the page =="
BUILD="$(LOFT_TIMEOUT=$(( 600 * ${WAIT_SCALE:-1} )) "$LOFT" --html "$WORK/probe.loft" 2>&1)"
PAGE="$WORK/.loft/probe.html"

fail=0
# 1. a page exists at all
[ -s "$PAGE" ] || { echo "FAIL: no page written — the build refused"; echo "$BUILD" | tail -20; exit 1; }
# 2. and it said which call the host cannot serve
echo "$BUILD" | grep -q "loft_gl_absent_probe" \
  || { echo "FAIL: the build did not name the unserviceable call"; fail=1; }

echo "== running the page =="
python3 -m http.server "$PORT" --directory "$WORK/.loft" >/dev/null 2>&1 &
SRV=$!
trap 'kill $SRV 2>/dev/null; rm -rf "$WORK"' EXIT
sleep 1
DOM="$(timeout $(( 120 * ${WAIT_SCALE:-1} )) "$CHROME" --headless --disable-gpu --no-sandbox \
        --virtual-time-budget=$(( 10000 * ${WAIT_SCALE:-1} )) \
        --dump-dom "http://127.0.0.1:$PORT/probe.html" 2>/dev/null)"

# 3. the page came up AND the call answered.  Matching the program's own output
# (not merely "no LinkError") is what makes this non-vacuous: a page that failed
# to instantiate prints nothing at all, so an empty DOM cannot pass.
case "$DOM" in
  *"probe: unavailable"*) ;;
  *"probe: served"*) echo "FAIL: the browser claims to serve a call it has no handler for"; fail=1 ;;
  *) echo "FAIL: the page produced no output — it did not instantiate"; fail=1 ;;
esac

[ $fail -eq 0 ] && echo "PASS: an unserviceable builtin builds, runs, and answers false in the browser"
exit $fail
