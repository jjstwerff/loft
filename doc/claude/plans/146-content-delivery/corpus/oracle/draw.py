#!/usr/bin/env python3
"""Sketch renderer over an annotated source (scene.draw), with a METRIC channel
and VALUE (tone) support.

Companion to ../DRAWING.md. Drawing is a perceive->mark->see-gap->adjust loop;
this tool makes the *seeing* cheap and moves metric judgments off the eye onto
exact measurement. Value support adds the late "tone" pass: a gradient sky and
filled gray masses, so dusk (dark cloud, glowing horizon, lit window) is possible.

Outputs each render (reloads when the file is saved):
  canvas.png        the drawing
  canvas_check.png  drawing + target guides
  preview.png       small image
  stats.txt         density map + element bboxes + CHECK results (metric channel)

Usage:
  python3 draw.py [scene-file]        # default: ./scene.draw next to this script
  python3 draw.py --once [scene-file] # render once and exit (agent/CI use);
                                      # exit 1 if any line is unparsed or a check fails
  SKETCH_OUT=/path python3 draw.py    # output dir (default: <tmp>/loft_sketch)

A line that matches no command is REPORTED (top of stats.txt + stderr), never
silently dropped — a typo'd mark must read as a syntax problem, not a geometry one.

Requires Pillow.

Commands (coords are fractions; origin top-left, y down; gray L in 0..1, 0=black):
  size WxH
  Background top=A bottom=B               vertical gradient sky (A at top, B at bottom)
  name <element>                          tag following marks (for measurement)
  Line (x1,y1) - (x2,y2) [w=N]
  Circle (cx,cy) r=R [n=N] [flat=F] [w=N] [<fill>]   round; <fill> => filled
  Poly (x1,y1) (x2,y2) ... [w=N] [<fill>]            stroke; <fill> => filled
  Petals (cx,cy) n=N r=R len=L w=W [bulge=B] [a0=D] <fill>   N teardrop petals arrayed
    round the centre (base at radius R, tip at R+L, widest W at bulge*L; a0 in degrees,
    0 = a petal pointing up). One filled, congruent, vertical-symmetric flower head.
  Fronds (x1,y1)-(x2,y2) n=N len=L [len2=] [w=] [w2=] [ang=] [ang2=] [mirror=1]
    [jitter=] [field=] [fray=] [bow=] [seed=] [depth=] [sub=] [stroke=r,g,b]   a LINEAR
    ARRAY of N tapered strokes rooted along the spine (veins, barbs, fur clumps, grass, a
    leg-fringe, hatching). len/ang ramp len→len2 / ang→ang2 along the run (TREND); placement
    clumps on a seeded low-frequency field + per-frond jitter, and ends fray — NON-UNIFORM by
    default (set jitter=field=fray=0 for a uniform man-made comb). mirror=1 → symmetric both
    sides. depth=2 → FRACTAL: each frond grows its own sub-array scaled by sub (real leaf
    venation; depth=1 = single level).
    <fill> = fill=L | rgb=R,G,B                      solid (gray / colour)
           | grad=R,G,B>R,G,B [dir=ax,ay,bx,by]      linear gradient (c1->c2)
           | radial=R,G,B>R,G,B [at=cx,cy,r]         radial gradient (centre->edge)
  landmark <name> = <value>
  check <prop> <op> <term> [tol T]        op: ~ < > <= >= ; arithmetic on RHS only
  # ...                                    comment / SHOULD note (ignored, searchable)
"""
import sys, time, re, os, math, tempfile
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ONCE = "--once" in sys.argv[1:]
_args = [a for a in sys.argv[1:] if a != "--once"]
SRC = _args[0] if _args else os.path.join(HERE, "scene.draw")
OUTDIR = os.environ.get("SKETCH_OUT") or os.path.join(tempfile.gettempdir(), "loft_sketch")
os.makedirs(OUTDIR, exist_ok=True)
OUT = os.path.join(OUTDIR, "canvas.png")
CHECKIMG = os.path.join(OUTDIR, "canvas_check.png")
PREVIEW = os.path.join(OUTDIR, "preview.png")
STATS = os.path.join(OUTDIR, "stats.txt")
if not os.path.exists(SRC):
    open(SRC, "a").close()

PREVIEW_W = 320
GRID_COLS, GRID_ROWS = 40, 18
RAMP = " .:-=+*#%@"

LINE = re.compile(r"Line\s*\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s*(?:@\s*([-\d.]+))?\s*-\s*"
                  r"\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s*(?:@\s*([-\d.]+))?\s*(?:w\s*=\s*(\d+))?", re.I)
SIZE = re.compile(r"size\s+(\d+)\s*x\s*(\d+)", re.I)
CIRCLE = re.compile(r"Circle\s*\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s*r=([-\d.]+)"
                    r"(?:\s+n=(\d+))?(?:\s+flat=([-\d.]+))?", re.I)
BG = re.compile(r"Background\s+top\s*=\s*([\d.]+)\s+bot(?:tom)?\s*=\s*([\d.]+)", re.I)
PETALS = re.compile(r"Petals\s*\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s+n\s*=\s*(\d+)"
                    r"\s+r\s*=\s*([-\d.]+)\s+len\s*=\s*([-\d.]+)\s+w\s*=\s*([-\d.]+)"
                    r"(?:\s+bulge\s*=\s*([-\d.]+))?(?:\s+a0\s*=\s*([-\d.]+))?", re.I)
FRONDS = re.compile(r"Fronds\s*\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s*-\s*"
                    r"\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s+n\s*=\s*(\d+)\s+len\s*=\s*([-\d.]+)", re.I)
PT = re.compile(r"\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)")
PTF = re.compile(r"\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s*(~?)\s*(?:@\s*([-\d.]+))?")
WOPT = re.compile(r"\bw\s*=\s*(\d+)", re.I)
SCOL = re.compile(r"\bstroke\s*=\s*\(?(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\)?", re.I)
FILL = re.compile(r"\bfill\s*=\s*([\d.]+)", re.I)
RGB = re.compile(r"\brgb\s*=\s*\(?\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)?", re.I)
GRAD = re.compile(r"\bgrad\s*=\s*\(?(\d+),(\d+),(\d+)\)?\s*>\s*\(?(\d+),(\d+),(\d+)\)?", re.I)
RADIAL = re.compile(r"\bradial\s*=\s*\(?(\d+),(\d+),(\d+)\)?\s*>\s*\(?(\d+),(\d+),(\d+)\)?", re.I)
DIR = re.compile(r"\bdir\s*=\s*([-\d.]+),([-\d.]+),([-\d.]+),([-\d.]+)", re.I)
ATC = re.compile(r"\bat\s*=\s*([-\d.]+),([-\d.]+),([-\d.]+)", re.I)
BGC = re.compile(r"Background\s+topc\s*=\s*\(?(\d+),(\d+),(\d+)\)?\s+botc\s*=\s*\(?(\d+),(\d+),(\d+)\)?", re.I)
LAND = re.compile(r"landmark\s+(\w+)\s*=\s*([-\d.]+)", re.I)


def gray(L):
    g = max(0, min(255, int(round(L * 255))))
    return (g, g, g)


def _paint(s):
    """A shape's fill descriptor:
       ('solid', rgb) | ('linear', c1, c2, axis|None) | ('radial', c1, c2, (cx,cy,r)|None)
       else None (stroke). Linear: c1@axis-start -> c2@axis-end (axis = fractional
       ax,ay,bx,by; None => vertical over the bbox). Radial: c1 centre -> c2 edge."""
    m = RADIAL.search(s)
    if m:
        c1 = (int(m[1]), int(m[2]), int(m[3])); c2 = (int(m[4]), int(m[5]), int(m[6]))
        a = ATC.search(s)
        return ("radial", c1, c2, (float(a[1]), float(a[2]), float(a[3])) if a else None)
    m = GRAD.search(s)
    if m:
        c1 = (int(m[1]), int(m[2]), int(m[3])); c2 = (int(m[4]), int(m[5]), int(m[6]))
        d = DIR.search(s)
        axis = (float(d[1]), float(d[2]), float(d[3]), float(d[4])) if d else None
        return ("linear", c1, c2, axis)
    m = RGB.search(s)
    if m:
        return ("solid", (int(m[1]), int(m[2]), int(m[3])))
    m = FILL.search(s)
    if m:
        return ("solid", gray(float(m[1])))
    return None


def _stroke_color(s):
    """Stroke colour from `stroke=R,G,B`, or None => the default dark ink.
       Lets a stroke carry colour (light hair strands, tinted lines) without
       becoming a fill — `rgb=` still means fill, `stroke=` means line colour."""
    m = SCOL.search(s)
    return (int(m[1]), int(m[2]), int(m[3])) if m else None


def _make_gradient(kind, c1, c2, spec, bbox, size):
    """A size=(w,h) RGB gradient image for the bbox (fractional fx0,fy0,fx1,fy1).
       Computed small (100x100) then resized — cheap and smooth."""
    fx0, fy0, fx1, fy1 = bbox
    if kind == "linear":
        ax, ay, bx, by = spec if spec else ((fx0+fx1)/2, fy0, (fx0+fx1)/2, fy1)
        dx, dy = bx-ax, by-ay
        denom = dx*dx + dy*dy or 1e-9
    else:
        cx, cy, r = spec if spec else ((fx0+fx1)/2, (fy0+fy1)/2, max(fx1-fx0, fy1-fy0)/2)
        r = r or 1e-9
    G = 100
    im = Image.new("RGB", (G, G))
    px = im.load()
    for j in range(G):
        fy = fy0 + (fy1-fy0)*(j+0.5)/G
        for i in range(G):
            fx = fx0 + (fx1-fx0)*(i+0.5)/G
            if kind == "linear":
                t = ((fx-ax)*dx + (fy-ay)*dy)/denom
            else:
                t = (((fx-cx)**2 + (fy-cy)**2) ** 0.5)/r
            t = 0.0 if t < 0 else 1.0 if t > 1 else t
            px[i, j] = (int(c1[0]+(c2[0]-c1[0])*t),
                        int(c1[1]+(c2[1]-c1[1])*t),
                        int(c1[2]+(c2[2]-c1[2])*t))
    return im.resize((max(1, size[0]), max(1, size[1])))


def _paint_polygon(img, pts, paint, BW, BH):
    if paint[0] == "solid":
        ImageDraw.Draw(img).polygon([(x*BW, y*BH) for x, y in pts], fill=paint[1])
        return
    _, c1, c2, spec = paint
    xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
    fx0, fy0, fx1, fy1 = min(xs), min(ys), max(xs), max(ys)
    x0p, y0p = int(fx0*BW), int(fy0*BH)
    w, h = max(1, int(fx1*BW)+1 - x0p), max(1, int(fy1*BH)+1 - y0p)
    grad = _make_gradient(paint[0], c1, c2, spec, (fx0, fy0, fx1, fy1), (w, h))
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).polygon([(p[0]*BW-x0p, p[1]*BH-y0p) for p in pts], fill=255)
    img.paste(grad, (x0p, y0p), mask)


def circle_pts(cx, cy, r, n, flat, W, H):
    ary = r * (W / H) * (1 - flat)
    return [(cx + r*math.cos(2*math.pi*i/n), cy + ary*math.sin(2*math.pi*i/n))
            for i in range(n + 1)]


def petal_polys(cx, cy, n, r, length, w, bulge, a0deg, W, H):
    """N teardrop petal outlines (fractional control points) arrayed around (cx,cy):
    each is the SAME canonical petal rotated by a0 + i*(2pi/n), built in PIXEL space
    (r/len/w scale by the single ref dim W) so petals stay congruent + round on
    non-square paper, exactly circle_pts' intent. Base at radius r, tip at r+len, widest
    |a|=w at bulge*len; a0 in degrees, 0 = a petal pointing up. Verified construction —
    see tools/petal_blueprint.py (12/12 falsification probes). Smooth each closed."""
    bw = bulge * length
    tmpl = [(0.0, 0.0), (-w, bw), (-w * 0.6, length * 0.93), (0.0, length),
            (w * 0.6, length * 0.93), (w, bw)]
    cxp, cyp, rp = cx * W, cy * H, r * W
    a0 = a0deg * math.pi / 180.0
    out = []
    for i in range(n):
        phi = a0 + i * (2 * math.pi / n)
        dx, dy = math.sin(phi), -math.cos(phi)      # outward (length) dir
        px, py = math.cos(phi), math.sin(phi)       # across dir
        poly = []
        for (a, b) in tmpl:
            ap, bp = a * W, b * W
            ox = ap * px + (rp + bp) * dx
            oy = ap * py + (rp + bp) * dy
            poly.append(((cxp + ox) / W, (cyp + oy) / H))
        out.append(poly)
    return out


def _hash01(seed, i, salt):
    """A REPRODUCIBLE pseudo-random float in [-1,1) from small non-negative ints — never
    random()/hash() (those break reproducible renders). Used for Fronds' seeded jitter."""
    x = ((seed * 73856093) ^ (i * 19349663) ^ (salt * 83492791)) & 0xFFFFFFFF
    x = (x ^ (x >> 13)) & 0xFFFFFFFF
    x = (x * 1274126177) & 0xFFFFFFFF
    return (x / 0xFFFFFFFF) * 2.0 - 1.0


def _lowfreq(seed, u):
    """A smooth LOW-FREQUENCY wave ~[-1,1] (two low harmonics, seed-derived phases).
    Smooth in u ⇒ neighbouring fronds correlate ⇒ CLUMPS + gaps, not white noise."""
    p1 = _hash01(seed, 0, 11) * math.pi
    p2 = _hash01(seed, 0, 22) * math.pi
    return 0.6 * math.sin(2 * math.pi * u + p1) + 0.4 * math.sin(4 * math.pi * u + p2)


def fronds(x1, y1, x2, y2, n, length, length2, w, w2, ang, ang2,
           mirror, jitter, field, fray, seed, bow, W, H, depth=1, sub=0.32):
    """A LINEAR ARRAY of N tapered strokes rooted along the spine (x1,y1)-(x2,y2): each
    frond is the template placed at u_i along the spine and tilted forward off the normal.
    Placement clumps on a seeded low-frequency field (uniform u=(i+0.5)/n when field=0),
    len/w/ang follow a TREND ramp ± seeded jitter, ends FRAY, and mirror pairs share the
    SAME sample reflected across the spine (equal across the axis). Built in PIXEL space
    so it's correct on non-square paper. Returns (centerline_pts_frac, widths, side, i) per
    frond. FRACTAL: depth>1 re-applies Fronds to EACH frond's centerline as a spine (scaled
    by `sub`) — self-similar venation (midrib→primaries→secondaries→…); depth=1 is byte-
    identical to single-level. Verified construction — see tools/fronds_blueprint.py (10/10)."""
    P1 = (x1 * W, y1 * H)
    P2 = (x2 * W, y2 * H)
    dx, dy = P2[0] - P1[0], P2[1] - P1[1]
    Ln = math.hypot(dx, dy) or 1e-9
    d = (dx / Ln, dy / Ln)
    nrm = (-d[1], d[0])
    base = []
    for i in range(n):
        ub = (i + 0.5) / n
        u = min(1.0, max(0.0, ub + field * _lowfreq(seed, ub) * (0.5 / n)))
        t = u
        Ltr = length + (length2 - length) * t
        dte = min(u, 1 - u)
        frayf = 1 - fray * max(0.0, 1 - dte / 0.25)
        Lpx = Ltr * frayf * (1 + jitter * 0.4 * _hash01(seed, i, 1)) * W
        an = (ang + (ang2 - ang) * t) + jitter * 15.0 * _hash01(seed, i, 2)
        phi = math.radians(an)
        wi = (w + (w2 - w) * t) * (1 + jitter * 0.3 * _hash01(seed, i, 3))
        Rx, Ry = P1[0] + u * dx, P1[1] + u * dy
        for side in ([1, -1] if mirror else [1]):
            nx, ny = nrm[0] * side, nrm[1] * side
            fx = nx * math.cos(phi) + d[0] * math.sin(phi)   # tilt the normal forward (→P2)
            fy = ny * math.cos(phi) + d[1] * math.sin(phi)
            Tx, Ty = Rx + Lpx * fx, Ry + Lpx * fy
            pts = [(Rx / W, Ry / H), (Tx / W, Ty / H)]
            wid = [wi, max(0.5, wi * 0.15)]                  # taper root→tip
            if bow != 0.0:                                   # curved frond (flow follows form)
                px, py = -fy, fx                             # perp to frond dir
                Mx = (Rx + Tx) / 2 + bow * Lpx * px
                My = (Ry + Ty) / 2 + bow * Lpx * py
                pts = [(Rx / W, Ry / H), (Mx / W, My / H), (Tx / W, Ty / H)]
                wid = [wi, (wi + wid[1]) / 2, wid[1]]
            base.append((pts, wid, side, i))
    if depth <= 1:
        return base
    # FRACTAL recursion: each frond's centerline (root→tip) is the spine of a smaller array.
    out = list(base)
    cn = max(2, round(n * 0.55))
    for k, (pts, wid, _s, _i) in enumerate(base):
        (rx, ry), (tx, ty) = pts[0], pts[-1]
        out += fronds(rx, ry, tx, ty, cn, length * sub, length * sub * 0.5,
                      w * 0.5, max(0.6, w2 * 0.5), ang, ang2, 1, jitter, field, fray,
                      seed * 31 + k + 1, bow, W, H, depth - 1, sub)
    return out


def _smooth_pts(pts, flags, closed, samples=10, vals=None):
    """Expand control points to a dense polyline. A point flagged smooth (~) curves
    (Catmull-Rom tangent = half the neighbour chord); a corner uses the segment chord
    => straight. So a segment between two corners is exactly straight, and any segment
    touching a smooth point curves — mixed linear/smooth in one outline. No-op (returns
    pts) if nothing is smooth, so plain Poly is unchanged.

    With `vals` (a per-control-point scalar list, e.g. per-point widths) it returns
    (pts, vals) and linearly interpolates the scalars across the same expansion."""
    n = len(pts)
    if n < 3 or not any(flags):
        return (pts, vals) if vals is not None else pts

    def P(i):
        return pts[i % n] if closed else pts[max(0, min(n-1, i))]

    def half(i):
        a, b = P(i-1), P(i+1)
        return ((b[0]-a[0]) * 0.5, (b[1]-a[1]) * 0.5)

    out = [pts[0]]
    for i in range(n if closed else n-1):
        a, b = pts[i % n], pts[(i+1) % n]
        chord = (b[0]-a[0], b[1]-a[1])
        ta = half(i % n) if flags[i % n] else chord
        tb = half((i+1) % n) if flags[(i+1) % n] else chord
        for s in range(1, samples+1):
            t = s/samples; t2 = t*t; t3 = t2*t
            h00, h10, h01, h11 = 2*t3-3*t2+1, t3-2*t2+t, -2*t3+3*t2, t3-t2
            out.append((h00*a[0]+h10*ta[0]+h01*b[0]+h11*tb[0],
                        h00*a[1]+h10*ta[1]+h01*b[1]+h11*tb[1]))
    if vals is not None:
        vout = [vals[0]]
        for i in range(n if closed else n-1):
            va, vb = vals[i % n], vals[(i+1) % n]
            for s in range(1, samples+1):
                vout.append(va + (vb-va) * (s/samples))
        return out, vout
    return out


def _ribbon(d, pts, widths, color, BW, BH, S):
    """A stroke as a filled ribbon, so the pen width can vary per point (taper).
       Offsets each point along the local normal by half its width on both sides
       and fills the band — a hair strand thick at the root, thin at the tip."""
    P = [(x*BW, y*BH) for x, y in pts]
    if len(P) < 2:
        return
    hw = [max(0.5, w*S/2.0) for w in widths]
    n = len(P)
    left, right = [], []
    for i in range(n):
        if i == 0:
            tx, ty = P[1][0]-P[0][0], P[1][1]-P[0][1]
        elif i == n-1:
            tx, ty = P[i][0]-P[i-1][0], P[i][1]-P[i-1][1]
        else:
            tx, ty = P[i+1][0]-P[i-1][0], P[i+1][1]-P[i-1][1]
        L = math.hypot(tx, ty) or 1.0
        nx, ny = -ty/L, tx/L
        left.append((P[i][0]+nx*hw[i], P[i][1]+ny*hw[i]))
        right.append((P[i][0]-nx*hw[i], P[i][1]-ny*hw[i]))
    d.polygon(left + right[::-1], fill=color)


def parse(text):
    W = H = 800
    ops = []          # ordered draw program
    elems = {}        # name -> [minx, miny, maxx, maxy]
    landmarks = {}
    checks = []
    unparsed = []     # (line-number, text) for every line no command accepted
    bg_transparent = False
    cur = [None]

    def acc(x, y):
        if cur[0] is None:
            return
        b = elems.get(cur[0])
        if b is None:
            elems[cur[0]] = [x, y, x, y]
        else:
            b[0] = min(b[0], x); b[1] = min(b[1], y)
            b[2] = max(b[2], x); b[3] = max(b[3], y)

    def accpts(pts):
        for x, y in pts:
            acc(x, y)

    for lineno, raw in enumerate(text.splitlines(), 1):
        s = raw.strip()
        if not s or s.startswith("#"):
            continue
        low = s.lower()
        if low.startswith("name "):
            cur[0] = s.split(None, 1)[1].strip(); continue
        if low.startswith("landmark"):
            m = LAND.match(s)
            if m:
                landmarks[m[1]] = float(m[2])
            else:
                unparsed.append((lineno, s))
            continue
        if low.startswith("check"):
            checks.append(s[5:].strip()); continue
        if low.startswith("background"):
            if "transparent" in low or "none" in low:
                bg_transparent = True; continue
            mc = BGC.search(s)
            if mc:
                ops.append(("grad", (int(mc[1]), int(mc[2]), int(mc[3])),
                            (int(mc[4]), int(mc[5]), int(mc[6]))))
            else:
                m = BG.search(s)
                if m:
                    ops.append(("grad", gray(float(m[1])), gray(float(m[2]))))
                else:
                    unparsed.append((lineno, s))
            continue
        m = SIZE.fullmatch(s)
        if m:
            W, H = int(m[1]), int(m[2]); continue
        m = CIRCLE.search(s)
        if m:
            cx, cy, r = float(m[1]), float(m[2]), float(m[3])
            n = int(m[4]) if m[4] else 28
            flat = float(m[5]) if m[5] else 0.0
            pts = circle_pts(cx, cy, r, n, flat, W, H)
            accpts(pts)
            paint = _paint(s)
            if paint is not None:
                ops.append(("fill", pts, paint))
            else:
                wm = WOPT.search(s)
                ops.append(("stroke", pts, int(wm[1]) if wm else 3, _stroke_color(s)))
            continue
        if low.startswith("petals"):
            m = PETALS.search(s)
            if m:
                cx, cy, n = float(m[1]), float(m[2]), int(m[3])
                r, length, pw = float(m[4]), float(m[5]), float(m[6])
                bulge = float(m[7]) if m[7] else 0.5
                a0 = float(m[8]) if m[8] else 0.0
                paint = _paint(s) or ("solid", (38, 32, 36))
                for poly in petal_polys(cx, cy, n, r, length, pw, bulge, a0, W, H):
                    sm = _smooth_pts(poly, [True] * len(poly), closed=True)
                    accpts(sm)
                    ops.append(("fill", sm, paint))
            continue
        if low.startswith("fronds"):
            m = FRONDS.search(s)
            if m:
                def optf(key, dflt):
                    mm = re.search(r"\b" + key + r"\s*=\s*([-\d.]+)", s, re.I)
                    return float(mm[1]) if mm else dflt
                x1, y1, x2, y2 = float(m[1]), float(m[2]), float(m[3]), float(m[4])
                n, length = int(m[5]), float(m[6])
                length2 = optf("len2", length)
                w = optf("w", 3.0); w2 = optf("w2", w)
                ang = optf("ang", 30.0); ang2 = optf("ang2", ang)
                mirror = int(optf("mirror", 0))
                # DEFAULT non-uniform + construction-hiding; zero them for a man-made comb
                jitter, field, fray = optf("jitter", 0.5), optf("field", 0.6), optf("fray", 0.4)
                bow, seed = optf("bow", 0.0), int(optf("seed", 1))
                depth, sub = int(optf("depth", 1)), optf("sub", 0.32)   # FRACTAL venation
                col = _stroke_color(s) or (38, 32, 36)
                for (pts, wid, _side, _i) in fronds(x1, y1, x2, y2, n, length, length2,
                        w, w2, ang, ang2, mirror, jitter, field, fray, seed, bow, W, H, depth, sub):
                    if len(pts) > 2:
                        cl, ww = _smooth_pts(pts, [False, True, False], closed=False, vals=wid)
                    else:
                        cl, ww = pts, wid
                    accpts(cl)
                    ops.append(("stroke", cl, ww, col))
            continue
        if low.startswith("poly"):
            raw = PTF.findall(s)
            pts = [(float(a), float(b)) for a, b, _, _ in raw]
            flags = [t == "~" for _, _, t, _ in raw]
            wraw = [w for _, _, _, w in raw]
            paint = _paint(s)
            if len(pts) < (3 if paint is not None else 2):
                unparsed.append((lineno, s)); continue
            if paint is not None:
                pts = _smooth_pts(pts, flags, closed=True)
                accpts(pts)
                ops.append(("fill", pts, paint))
            else:
                wm = WOPT.search(s)
                base = int(wm[1]) if wm else 3
                if any(wraw):
                    widths = [float(w) if w else base for w in wraw]
                    pts, widths = _smooth_pts(pts, flags, closed=False, vals=widths)
                    accpts(pts)
                    ops.append(("stroke", pts, widths, _stroke_color(s)))
                else:
                    pts = _smooth_pts(pts, flags, closed=False)
                    accpts(pts)
                    ops.append(("stroke", pts, base, _stroke_color(s)))
            continue
        m = LINE.search(s)
        if m:
            base = int(m[7]) if m[7] else 3
            p = [(float(m[1]), float(m[2])), (float(m[4]), float(m[5]))]
            accpts(p)
            if m[3] or m[6]:
                w1 = float(m[3]) if m[3] else base
                w2 = float(m[6]) if m[6] else base
                ops.append(("stroke", p, [w1, w2], _stroke_color(s)))
            else:
                ops.append(("stroke", p, base, _stroke_color(s)))
            continue
        unparsed.append((lineno, s))
    return W, H, ops, elems, landmarks, checks, bg_transparent, unparsed


def props(b):
    return dict(left=b[0], top=b[1], right=b[2], bottom=b[3],
                w=b[2]-b[0], h=b[3]-b[1], cx=(b[0]+b[2])/2, cy=(b[1]+b[3])/2)


def value_of(token, elems, landmarks):
    try:
        return float(token), None
    except ValueError:
        pass
    if "." in token:
        e, p = token.split(".", 1)
        if e not in elems:
            return None, f"{e}?"
        d = props(elems[e])
        if p not in d:
            return None, f"{e}.{p}?"
        return d[p], None
    if token in landmarks:
        return landmarks[token], None
    return None, f"{token}?"


def term_of(expr, elems, landmarks):
    m = re.match(r"^(\S+)\s*([+-])\s*([\d.]+)$", expr.strip())
    if m:
        base, err = value_of(m[1], elems, landmarks)
        if base is None:
            return None, err
        return base + (float(m[3]) if m[2] == "+" else -float(m[3])), None
    return value_of(expr.strip(), elems, landmarks)


def eval_check(c, elems, landmarks):
    tol = 0.02
    mt = re.search(r"\btol\s+([\d.]+)", c)
    if mt:
        tol = float(mt[1]); c = c[:mt.start()].strip()
    m = re.match(r"^(\S+)\s*(~|<=|>=|<|>|==)\s*(.+)$", c)
    if not m:
        return {"text": f"check: {c}  (unparsed)", "ok": False, "target": None, "prop": None}
    lhs, le = value_of(m[1], elems, landmarks)
    rhs, re_ = term_of(m[3], elems, landmarks)
    if lhs is None or rhs is None:
        return {"text": f"check: {c}  ERR {le or re_}", "ok": False, "target": None, "prop": None}
    op = m[2]; delta = lhs - rhs
    ok = {"~": abs(delta) <= tol, "<": lhs < rhs, ">": lhs > rhs,
          "<=": lhs <= rhs, ">=": lhs >= rhs, "==": abs(delta) < 1e-9}[op]
    txt = (f"check: {m[1]} {op} {m[3].strip()}  ->  {lhs:.3f} vs {rhs:.3f}  "
           f"d={delta:+.3f}  {'PASS' if ok else 'FAIL'}")
    return {"text": txt, "ok": ok, "target": rhs, "prop": m[1]}


def _largest_flat_rect(flat):
    """Largest all-1 rectangle in a binary grid (1 = flat/uniform cell).
    Returns (area_cells, r0, c0, r1, c1)."""
    rows = len(flat)
    cols = len(flat[0]) if rows else 0
    height = [0] * cols
    best = (0, 0, 0, 0, 0)
    for r in range(rows):
        for c in range(cols):
            height[c] = height[c] + 1 if flat[r][c] else 0
        stack = []  # (start_col, height)
        for c in range(cols + 1):
            cur = height[c] if c < cols else 0
            start = c
            while stack and stack[-1][1] > cur:
                sc, sh = stack.pop()
                area = sh * (c - sc)
                if area > best[0]:
                    best = (area, r - sh + 1, sc, r, c - 1)
                start = sc
            stack.append((start, cur))
    return best


def composition_lines(img):
    """Global composition / notan report (visual weight = darkness)."""
    g = img.convert("L")
    gw, gh = g.size
    SW, SH = 100, 70
    px = g.resize((SW, SH)).load()
    total = cx = cy = 0.0
    left = right = top = bottom = 0.0
    dk = md = lt = 0
    for yy in range(SH):
        for xx in range(SW):
            v = px[xx, yy]
            d = (255 - v) / 255.0
            total += d
            cx += d * (xx + 0.5) / SW
            cy += d * (yy + 0.5) / SH
            if xx < SW / 2:
                left += d
            else:
                right += d
            if yy < SH / 2:
                top += d
            else:
                bottom += d
            if v < 85:
                dk += 1
            elif v < 170:
                md += 1
            else:
                lt += 1
    out = ["", "COMPOSITION (visual weight = darkness):"]
    if left + right > 0:
        lp = 100 * left / (left + right)
        v = "balanced" if 42 <= lp <= 58 else ("LEFT-heavy" if lp > 58 else "RIGHT-heavy")
        out.append(f"  L/R weight: {lp:.0f}/{100-lp:.0f}  ({v})")
    if top + bottom > 0:
        tp = 100 * top / (top + bottom)
        v = "balanced" if 42 <= tp <= 58 else ("TOP-heavy" if tp > 58 else "BOTTOM-heavy")
        out.append(f"  T/B weight: {tp:.0f}/{100-tp:.0f}  ({v})")
    if total > 0:
        mx, my = cx / total, cy / total
        thirds = [(1/3, 1/3), (2/3, 1/3), (1/3, 2/3), (2/3, 2/3)]
        nd, tx, ty = min(((((mx-a)**2+(my-b)**2)**0.5), a, b) for a, b in thirds)
        cd = ((mx-0.5)**2 + (my-0.5)**2) ** 0.5
        if cd < 0.07:
            fv = "DEAD-CENTRE (static)"
        elif nd < 0.10:
            fv = f"near thirds ({tx:.2f},{ty:.2f}) (dynamic)"
        else:
            fv = "off-grid"
        out.append(f"  weight centre: ({mx:.2f},{my:.2f})  {fv}")
    td = dk + md + lt
    out.append(f"  value spread: dark {100*dk/td:.0f}% / mid {100*md/td:.0f}% / light {100*lt/td:.0f}%")
    if 100*md/td > 55:
        nv = "muddy (mid-dominated)"
    elif 100*dk/td > 55:
        nv = "dark-dominant"
    elif 100*lt/td > 55:
        nv = "light-dominant"
    else:
        nv = "clear spread"
    out.append(f"  notan: {nv}")
    GC, GR = 24, 16
    cw, ch = gw / GC, gh / GR
    flat = []
    for r in range(GR):
        row = []
        for c in range(GC):
            box = (int(c*cw), int(r*ch),
                   max(int((c+1)*cw), int(c*cw)+1), max(int((r+1)*ch), int(r*ch)+1))
            mn, mxv = g.crop(box).getextrema()
            row.append(1 if (mxv - mn) < 28 else 0)
        flat.append(row)
    area, r0, c0, r1, c1 = _largest_flat_rect(flat)
    if area > 0:
        fx0, fy0, fx1, fy1 = c0/GC, r0/GR, (c1+1)/GC, (r1+1)/GR
        reg = g.crop((int(fx0*gw), int(fy0*gh), int(fx1*gw), int(fy1*gh)))
        mv = reg.resize((1, 1)).getpixel((0, 0))
        kind = "dark mass" if mv < 90 else ("light/empty field" if mv > 165 else "mid field")
        out.append(f"  largest flat region: x[{fx0:.2f}..{fx1:.2f}] y[{fy0:.2f}..{fy1:.2f}]  "
                   f"{100*area/(GC*GR):.0f}% of frame, {kind} (v={mv})")
    return out


def write_stats(img, W, H, nops, elems, results, unparsed):
    g = img.convert("L")
    gw, gh = g.size
    cw, ch = gw / GRID_COLS, gh / GRID_ROWS
    lines = []
    if unparsed:
        lines.append(f"UNPARSED {len(unparsed)} line(s) — NOT drawn (fix the syntax "
                     "before judging geometry):")
        for ln, txt in unparsed:
            lines.append(f"  line {ln}: {txt}")
        lines.append("")
    lines += [f"ops: {nops}", f"paper: {W}x{H}", ""]
    lines.append("density (darker = more ink):")
    for ry in range(GRID_ROWS):
        row = []
        for rx in range(GRID_COLS):
            box = (int(rx*cw), int(ry*ch),
                   max(int((rx+1)*cw), int(rx*cw)+1), max(int((ry+1)*ch), int(ry*ch)+1))
            dark = 255 - g.crop(box).getextrema()[0]
            row.append(RAMP[min(len(RAMP)-1, dark*len(RAMP)//256)])
        lines.append("".join(row))
    lines.append("")
    lines.append("elements (bbox frac):")
    for name, b in elems.items():
        lines.append(f"  {name:8} x[{b[0]:.2f}..{b[2]:.2f}] y[{b[1]:.2f}..{b[3]:.2f}]")
    lines.append("")
    npass = sum(1 for r in results if r["ok"])
    lines.append(f"CHECKS  {npass}/{len(results)} pass:")
    for r in results:
        lines.append("  " + r["text"])
    lines += composition_lines(img)
    with open(STATS, "w") as f:
        f.write("\n".join(lines) + "\n")


def render():
    """Render SRC to the output files. Returns True when every line parsed and
    every check passes — the --once exit status."""
    try:
        text = open(SRC).read()
    except FileNotFoundError:
        text = ""
    W, H, ops, elems, landmarks, checks, bg_transparent, unparsed = parse(text)
    for ln, txt in unparsed:
        print(f"UNPARSED line {ln}: {txt}", file=sys.stderr)
    S = 3  # supersample, then downscale => anti-aliased (no hard faceted edges)
    BW, BH = W * S, H * S
    # `Background transparent` renders on a transparent base, so the LANCZOS
    # downscale anti-aliases edges (and any light detail, e.g. eye-glints) against
    # alpha instead of white — a re-compositable sprite, no colour-key hack.
    if bg_transparent:
        img = Image.new("RGBA", (BW, BH), (0, 0, 0, 0))
    else:
        img = Image.new("RGB", (BW, BH), "white")
    d = ImageDraw.Draw(img)
    for op in ops:
        if op[0] == "grad":
            _, top, bot = op
            for yy in range(BH):
                t = yy / max(1, BH - 1)
                c = tuple(int(top[i] + (bot[i] - top[i]) * t) for i in range(3))
                d.line([(0, yy), (BW, yy)], fill=c)
        elif op[0] == "fill":
            _, pts, paint = op
            _paint_polygon(img, pts, paint, BW, BH)
        elif op[0] == "stroke":
            _, pts, w, col = op
            color = col if col else (38, 32, 36)
            if isinstance(w, list):
                _ribbon(d, pts, w, color, BW, BH, S)
            else:
                for p, q in zip(pts, pts[1:]):
                    d.line([(p[0]*BW, p[1]*BH), (q[0]*BW, q[1]*BH)], fill=color, width=max(1, w*S))
    img = img.resize((W, H), Image.LANCZOS)
    img.save(OUT)
    # Flatten-on-white copy for preview / check-overlay / stats — reads correctly
    # even when OUT is a transparent RGBA sprite.
    flat = img if img.mode == "RGB" else Image.alpha_composite(
        Image.new("RGBA", img.size, (255, 255, 255, 255)), img).convert("RGB")
    ph = max(1, round(PREVIEW_W * H / W))
    flat.resize((PREVIEW_W, ph)).save(PREVIEW)

    results = [eval_check(c, elems, landmarks) for c in checks]

    over = flat.copy()
    od = ImageDraw.Draw(over)
    GRAY, GREEN, RED = (170, 170, 170), (40, 150, 40), (220, 40, 40)
    for b in elems.values():
        od.rectangle([int(b[0]*W), int(b[1]*H), int(b[2]*W), int(b[3]*H)], outline=GRAY)
    for v in landmarks.values():
        od.line([(0, int(v*H)), (W, int(v*H))], fill=GREEN, width=1)
    for r in results:
        if not r["ok"] and r["target"] is not None and r["prop"] and \
           any(r["prop"].endswith(s) for s in (".cy", ".top", ".bottom")):
            yy = int(r["target"] * H)
            od.line([(0, yy), (W, yy)], fill=RED, width=2)
    over.save(CHECKIMG)

    write_stats(flat, W, H, len(ops), elems, results, unparsed)
    return not unparsed and all(r["ok"] for r in results)


def main():
    print(f"sketch: source={SRC}  out={OUTDIR}", file=sys.stderr)
    if ONCE:
        sys.exit(0 if render() else 1)
    last = None
    render()
    while True:
        try:
            m = os.path.getmtime(SRC)
        except OSError:
            m = None
        if m != last:
            last = m
            render()
        time.sleep(0.1)


if __name__ == "__main__":
    main()
