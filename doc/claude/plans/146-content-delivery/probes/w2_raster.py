#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN146 W2 — cross-check the loft rasteriser against Pillow itself.

Generates polygons, thick lines and thin polylines, renders each one BOTH ways —
through `w2_raster.loft` and through `ImageDraw` — and diffs the alpha masks.
Every difference is the port's bug: `draw.py`'s filler *is* Pillow, so this is
the unit-level half of the corpus gate (`corpus/w2.sh` is the whole-picture half).

    ./w2_raster.py                     # the cross-check
    ./w2_raster.py --show 42           # print one case's two masks side by side

Two of the cases exist for a reason worth keeping:

  * shapes hanging off the LEFT and TOP edges, because the mirrored `ROUND_DOWN`
    is only visible on a NEGATIVE crossing — a case set that stays on the canvas
    cannot tell it from the obvious `ceil(f - 0.5)`;
  * a closed circle with its first point repeated, which is how `draw.py` hands
    one over, and a long thin quad, which is what a tapered stroke becomes.
"""
import math
import os
import random
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", "..", "..", ".."))
WORKSPACE = os.path.dirname(REPO)

W = H = 48
INK = (200, 40, 30, 255)


def cases():
    out = []
    for t in range(240):
        rng = random.Random(t * 7919 + 13)
        m = t % 3
        if m == 0:                                  # float polygons, some off-canvas
            k = rng.choice([3, 3, 4, 5, 6, 8])
            out.append(("P", [(round(rng.uniform(-4, W + 4), 4),
                               round(rng.uniform(-4, H + 4), 4)) for _ in range(k)], None))
        elif m == 1:                                # whole-pixel polygons (vertex-heavy)
            k = rng.choice([3, 4, 5, 6])
            out.append(("P", [(float(rng.randint(-2, W + 2)),
                               float(rng.randint(-2, H + 2))) for _ in range(k)], None))
        else:                                       # thick and thin segments
            out.append(("W", [(round(rng.uniform(0, W), 4), round(rng.uniform(0, H), 4)),
                              (round(rng.uniform(0, W), 4), round(rng.uniform(0, H), 4))],
                        rng.choice([1, 2, 3, 4, 6, 9, 12, 21])))
    for n, r in ((28, 18.0), (28, 4.0), (7, 20.0)):
        pts = [(24 + r * math.cos(2 * math.pi * i / n), 24 + r * math.sin(2 * math.pi * i / n))
               for i in range(n + 1)]
        out.append(("P", [(round(x, 6), round(y, 6)) for x, y in pts], None))
    out.append(("T", [(3.2, 4.7), (40.9, 9.1), (12.0, 44.0)], None))
    out.append(("T", [(0.0, 0.0), (47.0, 47.0)], None))
    for t in range(90):
        rng = random.Random(t * 31337 + 5)
        k = rng.choice([3, 4, 5])
        cx, cy = rng.uniform(-30, 12), rng.uniform(-30, 12)
        out.append(("P", [(round(cx + rng.uniform(0, 46), 4),
                           round(cy + rng.uniform(0, 46), 4)) for _ in range(k)], None))
    return out


def pillow_mask(kind, pts, w):
    im = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    if kind == "P":
        d.polygon(pts, fill=INK)
    elif kind == "T":
        d.line(pts, fill=INK)
    else:
        d.line(pts, fill=INK, width=w)
    px = im.load()
    return "".join("1" if px[x, y][3] else "0" for y in range(H) for x in range(W))


def grid(mask):
    return "\n".join(mask[y * W:(y + 1) * W].replace("0", ".").replace("1", "#") for y in range(H))


def main():
    cs = cases()
    tmp = tempfile.mkdtemp()
    path = os.path.join(tmp, "cases.txt")
    with open(path, "w") as f:
        for kind, pts, w in cs:
            f.write(f"{kind} " + " ".join(f"{x},{y}" for x, y in pts)
                    + (f" {w}" if w is not None else "") + "\n")

    loft = os.environ.get("LOFT", os.path.join(REPO, "target", "release", "loft"))
    src = os.environ.get("W2_DRAWING", os.path.join(WORKSPACE, "loft-libs-graphics", "drawing", "src"))
    r = subprocess.run([loft, os.environ.get("W2_BACKEND", "--interpret"), "--lib", src,
                        os.path.join(HERE, "w2_raster.loft")],
                       env=dict(os.environ, W2_CASES=path), capture_output=True, text=True)
    got = r.stdout.split()
    if len(got) != len(cs):
        print(f"RED  loft answered {len(got)} case(s) for {len(cs)}\n{r.stderr[-2000:]}")
        return 1

    if "--show" in sys.argv:
        i = int(sys.argv[sys.argv.index("--show") + 1])
        print(f"case {i}: {cs[i]}\n\n--- loft ---\n{grid(got[i])}\n\n--- Pillow ---\n"
              f"{grid(pillow_mask(*cs[i]))}")
        return 0

    bad = []
    for i, (kind, pts, w) in enumerate(cs):
        want = pillow_mask(kind, pts, w)
        if got[i] != want:
            bad.append((i, sum(1 for a, b in zip(got[i], want) if a != b)))
    if bad:
        print(f"RED  {len(bad)} of {len(cs)} case(s) differ from Pillow")
        for i, d in bad[:10]:
            print(f"     case {i}: {d} px — ./w2_raster.py --show {i}")
        return 1
    print(f"{len(cs)} case(s) identical to Pillow")
    return 0


if __name__ == "__main__":
    sys.exit(main())
