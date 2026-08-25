#!/usr/bin/env bash
# @PLN146 W6 — does a scene reach the pack without becoming a file first?
#
#   ./w6.sh              build one atlas page both ways and compare the cells
#   ./w6.sh --control    move one texel on the fileless route; the diff must see it
#
# Route A renders the scene, writes a PNG, decodes it again with `imaging` and
# packs those texels — everything a packer had to do while `drawing` could only
# hand a picture out as a file.  Route B packs the rendered canvas directly.
# Both go through `assets::page_grid`, both are WRITTEN and read back, and the
# cells are compared rect by rect, byte by byte, proxy box by proxy box.
#
# Three scenes, not one: a page holds more than one cell, and a placer that
# ignores its grid step would still agree with itself on a single tile.
set -u
cd "$(dirname "$0")"
HERE=$(pwd)
WORKSPACE=$(cd "$HERE/../../../../../.." && pwd)
DRAWING=${W6_DRAWING:-$WORKSPACE/loft-libs-graphics/drawing/src}
GRAPHICS=${W6_GRAPHICS:-$WORKSPACE/loft-libs-graphics/graphics/src}
# The pack side: a working checkout by default, so the gate follows an unpublished
# `assets` the way it follows an unpublished `graphics`.  Point it at an empty
# directory to measure the registry copy instead.
#
# ⚠ Before `assets` 0.3.0 is published this needs the placer (`texels` / `Tile` /
# `page_grid`) to be somewhere loft can see: check `tuxedo-assets-w6` out for the
# run, or point `W6_ASSETS` at a copy of its `src/`.  The registry copy is 0.2.0
# and the driver will not compile against it.
ASSETS=${W6_ASSETS:-$WORKSPACE/loft-libs-assets/assets/src}
LOFT=${LOFT:-$HERE/../../../../../target/release/loft}
LIST=${W6_LIST:-"ammo sword potion"}
BACKEND=${W6_BACKEND:---native}

control=0
[ "${1:-}" = "--control" ] && control=1

if [ ! -x "$LOFT" ]; then echo "RED  no loft binary at $LOFT — set LOFT"; exit 1; fi
if [ ! -f "$DRAWING/drawing.loft" ]; then
  echo "RED  no drawing package at $DRAWING — set W6_DRAWING"; exit 1
fi

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
mkdir -p "$out/a" "$out/b"

libargs=(--lib "$DRAWING")
[ -d "$GRAPHICS" ] && libargs+=(--lib "$GRAPHICS")
[ -d "$ASSETS" ] && libargs+=(--lib "$ASSETS")
[ "$BACKEND" = "--interpret" ] && export LOFT_NO_NATIVE_LIBS=1

W6_SCENES="$HERE/scenes" W6_OUT="$out" W6_LIST="$LIST" W6_CONTROL="$control" \
  "$LOFT" $BACKEND "${libargs[@]}" "$HERE/w6.loft" > "$out/log" 2>"$out/stderr"
status=$?
grep -v '^advice\|^note:\|^warning' "$out/log"

if [ "$status" != 0 ]; then
  echo "RED  the driver exited $status"
  tail -20 "$out/stderr"
  exit 1
fi

if grep -q '^RED' "$out/log"; then
  if [ "$control" = 1 ]; then
    echo "control fired: the two routes went red, as they must"
    exit 0
  fi
  exit 1
fi
if [ "$control" = 1 ]; then
  echo "CONTROL DID NOT FIRE — the gate is not comparing the two routes"
  exit 1
fi
echo "the fileless route builds the same atlas entry as the one through a PNG"
