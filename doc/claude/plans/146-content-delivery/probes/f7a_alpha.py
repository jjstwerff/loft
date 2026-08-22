#!/usr/bin/env python3
"""@PLN146 F7a — what shape should alpha-derivation produce?

F7 derives a collision proxy from a sprite's own alpha.  `shapes` ships `Rect`
and `Circle`, so before F7 depends on it, measure what each candidate proxy
costs over the real corpus:

  proxy ⊇ opaque                                   containment — every candidate
  overshoot = (proxy_area - opaque_area) / opaque_area   what it costs

Run from the plan dir:

  python3 probes/f7a_alpha.py            the summary table F7a.md quotes
  python3 probes/f7a_alpha.py --per      per-sprite rows as well
"""
import collections
import glob
import math
import os
import statistics
import sys

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
GOLD = os.path.join(HERE, "..", "corpus", "golden")
ALPHA_ON = 8          # a texel counts as opaque above this alpha
BANDS = (1, 2, 4, 8, 16, 32)


def opaque_points(path):
    """Every opaque texel of a sprite, or `None` for a golden with no alpha."""
    im = Image.open(path)
    if im.mode != "RGBA":
        return None, 0
    a = im.split()[3].load()
    w, h = im.size
    return [(x, y) for y in range(h) for x in range(w) if a[x, y] > ALPHA_ON], w * h


def hull(pts):
    """Monotone-chain convex hull, CCW.  The shape `shapes` has no kind for."""
    pts = sorted(set(pts))
    if len(pts) < 3:
        return pts

    def half(seq):
        out = []
        for p in seq:
            while len(out) >= 2:
                (ax, ay), (bx, by) = out[-2], out[-1]
                if (bx - ax) * (p[1] - ay) - (by - ay) * (p[0] - ax) > 0:
                    break
                out.pop()
            out.append(p)
        return out

    return half(pts)[:-1] + half(reversed(pts))[:-1]


def poly_area(poly):
    s = 0.0
    for i, (x, y) in enumerate(poly):
        x2, y2 = poly[(i + 1) % len(poly)]
        s += x * y2 - x2 * y
    return abs(s) / 2.0


def row_boxes(pts, k):
    """A k-box proxy: split the mask into k horizontal bands, take each band's
    tight box.  One pass over the alpha, and every box is a `shapes` Rect."""
    ys = [y for _, y in pts]
    lo, hi = min(ys), max(ys)
    span = max(1, hi - lo + 1)
    bands = collections.defaultdict(list)
    for x, y in pts:
        bands[min(k - 1, (y - lo) * k // span)].append((x, y))
    boxes = []
    for b in bands.values():
        xs = [p[0] for p in b]
        bys = [p[1] for p in b]
        boxes.append((min(xs), min(bys), max(xs) - min(xs) + 1, max(bys) - min(bys) + 1))
    return boxes


def col_boxes(pts, k):
    """The same decomposition along the other axis."""
    return [(y, x, h, w) for x, y, w, h in row_boxes([(y, x) for x, y in pts], k)]


def components(pts):
    """4-connected components — the decomposition that looks obvious and is not."""
    live = set(pts)
    seen = set()
    out = []
    for p in pts:
        if p in seen:
            continue
        stack = [p]
        seen.add(p)
        comp = []
        while stack:
            x, y = stack.pop()
            comp.append((x, y))
            for q in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                if q in live and q not in seen:
                    seen.add(q)
                    stack.append(q)
        out.append(comp)
    return out


def box_area(boxes):
    return sum(w * h for _, _, w, h in boxes)


def bbox(pts):
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    return min(xs), min(ys), max(xs) - min(xs) + 1, max(ys) - min(ys) + 1


def measure(pts):
    """Every candidate proxy's overshoot for one sprite."""
    area = float(len(pts))
    _, _, bw, bh = bbox(pts)
    cx = sum(p[0] for p in pts) / area
    cy = sum(p[1] for p in pts) / area
    r = max(math.hypot(x - cx, y - cy) for x, y in pts) + 0.5
    out = {
        "aabb": bw * bh / area - 1,
        "circle": math.pi * r * r / area - 1,
        "hull": poly_area(hull(pts)) / area - 1,
        "components": sum(box_area([bbox(c)]) for c in components(pts)) / area - 1,
    }
    for k in BANDS:
        out[f"rows{k}"] = box_area(row_boxes(pts, k)) / area - 1
        out[f"best{k}"] = min(out[f"rows{k}"], box_area(col_boxes(pts, k)) / area - 1)
    return out


def main():
    per = "--per" in sys.argv[1:]
    names, rows = [], []
    for path in sorted(glob.glob(os.path.join(GOLD, "*.png"))):
        pts, _ = opaque_points(path)
        if not pts:
            continue                      # a golden with no alpha channel
        names.append(os.path.basename(path)[:-4])
        rows.append(measure(pts))

    if per:
        cols = ["aabb", "circle", "hull", "rows8", "rows16", "best16"]
        print(f"{'sprite':<20}" + "".join(f"{c:>10}" for c in cols))
        for name, r in zip(names, rows):
            print(f"{name:<20}" + "".join(f"{r[c]:>+10.1%}" for c in cols))
        print()

    def line(label, key):
        v = [r[key] for r in rows]
        worst = max(v)
        print(f"{label:<22}{statistics.mean(v):>+9.1%}{statistics.median(v):>+9.1%}"
              f"{worst:>+9.1%}  {names[v.index(worst)]:<14}"
              f"{sum(1 for x in v if x <= 1.0):>4}/{len(v)}")

    print(f"{'proxy':<22}{'mean':>9}{'median':>9}{'worst':>9}  {'worst sprite':<14}"
          f"{'<=+100%':>9}")
    line("one Rect (AABB)", "aabb")
    line("one Circle", "circle")
    line("convex hull", "hull")
    line("per-component boxes", "components")
    for k in BANDS:
        line(f"{k} Rect bands (rows)", f"rows{k}")
    line("16 bands, better axis", "best16")
    picks = sum(1 for r in rows if r["best16"] < r["rows16"])
    print(f"\n{picks} of {len(rows)} sprites are tighter banded by column than by row.")


if __name__ == "__main__":
    sys.exit(main())
