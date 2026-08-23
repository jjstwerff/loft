#!/usr/bin/env bash
# @PLN146 W2 — is the loft renderer's picture the ORACLE's picture?
#
#   ./w2.sh              render the W2 subset in loft and diff it against golden/
#   ./w2.sh --control    inject a one-pixel error first, and require the diff to see it
#
# The subset is COMPUTED from the scenes, not listed: a scene qualifies when it
# uses only what W2 owes (`size`, `Background`, `name`, `Line`, `Circle`, `Poly`
# with solid fills).  So a scene that gains a `Fronds` line leaves the gate by
# itself, and one that loses its last gradient joins it — neither needs an edit
# here, and the gate cannot quietly stop covering something.
#
# `golden/` is the oracle's own output, gated by `w0.sh`; the comparison is on
# DECODED PIXELS rather than file bytes, because two PNG encoders agreeing byte
# for byte is a different (and much weaker) claim than two renderers agreeing.
set -u
cd "$(dirname "$0")"
HERE=$(pwd)
WORKSPACE=$(cd "$HERE/../../../../../.." && pwd)
DRAWING=${W2_DRAWING:-$WORKSPACE/loft-libs-graphics/drawing/src}
LOFT=${LOFT:-$HERE/../../../../../target/release/loft}
BACKEND=${W2_BACKEND:---native}

control=0
[ "${1:-}" = "--control" ] && control=1

if [ ! -f "$DRAWING/drawing.loft" ]; then
  echo "RED  no drawing package at $DRAWING — set W2_DRAWING to its src/ directory"
  exit 1
fi
if [ ! -x "$LOFT" ]; then
  echo "RED  no loft binary at $LOFT — set LOFT"
  exit 1
fi

# The subset, measured off the scenes.
list=""
skipped=""
for scene in scenes/*.draw; do
  name=$(basename "$scene" .draw)
  if grep -qiE '^[[:space:]]*(Fronds|Petals)|grad=|radial=|fill=' "$scene"; then
    skipped="$skipped $name"
  else
    list="$list $name"
  fi
done
list=${list# }

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

src=$DRAWING
if [ "$control" = 1 ]; then
  # A control that cannot be confused with a real regression: shift ONE pixel
  # column of every filled span.  If the diff below still says green, it is not
  # reading the pictures.
  src=$out/control-src
  cp -r "$DRAWING" "$src"
  sed -i 's|round_down(pg_xx\[pg_p\]? as float), ink);|round_down(pg_xx[pg_p]? as float) - 1, ink);|' \
    "$src/raster.loft"
  echo "control: every filled span is one pixel short"
fi

W2_SCENES="$HERE/scenes" W2_OUT="$out" W2_LIST="$list" \
  "$LOFT" $BACKEND --lib "$src" "$HERE/w2.loft" 2>"$out/stderr" | tee "$out/render.log"
if grep -q "^RED" "$out/render.log"; then
  echo "RED  the parser did not accept every line of the subset"
  exit 1
fi

python3 - "$out" "$HERE/golden" "$list" "$skipped" "$control" <<'PY'
import sys, os
from PIL import Image

out, golden, listed, skipped, control = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5] == "1"
names = listed.split()
bad = 0
worst = None
for n in names:
    got, want = os.path.join(out, n + ".png"), os.path.join(golden, n + ".png")
    if not os.path.exists(got):
        print(f"RED  {n} — the renderer wrote no PNG")
        bad += 1
        continue
    a, b = Image.open(got), Image.open(want)
    if a.size != b.size:
        print(f"RED  {n} — {a.size} against the oracle's {b.size}")
        bad += 1
        continue
    ap, bp = a.convert("RGBA").tobytes(), b.convert("RGBA").tobytes()
    if ap != bp:
        d = sum(1 for i in range(0, len(ap), 4) if ap[i:i + 4] != bp[i:i + 4])
        print(f"RED  {n} — {d} of {len(ap) // 4} pixels differ from the oracle")
        bad += 1
        if worst is None or d > worst[1]:
            worst = (n, d)
if skipped.split():
    print(f"outside the W2 grammar, not rendered: {' '.join(sorted(skipped.split()))}")
if control:
    if bad:
        print(f"control fired: {bad} of {len(names)} scene(s) went red, as they must")
        sys.exit(0)
    print("CONTROL DID NOT FIRE — the diff is not reading the pictures")
    sys.exit(1)
if bad:
    print(f"{bad} of {len(names)} scene(s) differ from the oracle")
    sys.exit(1)
print(f"{len(names)} scene(s) pixel-identical to the oracle")
PY
