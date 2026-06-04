#!/usr/bin/env python3
"""Sketch renderer over an annotated source file, with a METRIC channel.

Companion to ../DRAWING.md (the design rationale). Drawing is a perceive->mark->
see-gap->adjust loop; this tool's job is to make the *seeing* cheap and to move
metric judgments (position/size) off the eye onto exact measurement.

It watches a scene source and re-renders on save. On every render it writes:
  canvas.png        clean drawing            (gestalt look)
  canvas_check.png  drawing + target guides  (gap made visible)
  preview.png       small image
  stats.txt         density map + element bboxes + CHECK results (the metric channel)

Usage:
  python3 draw.py [scene-file]        # default: ./scene.draw next to this script
  SKETCH_OUT=/path python3 draw.py    # output dir (default: <tmp>/loft_sketch)

Requires Pillow (`pip install pillow`).

Source commands (coords are fractions of paper; origin top-left, y down):
  size WxH
  name <element>                              tag following strokes (for measurement)
  Line (x1,y1) - (x2,y2) [w=N]
  Circle (cx,cy) r=R [n=N] [flat=F] [w=N]     round by default (aspect-corrected)
  Poly (x1,y1) (x2,y2) ... [w=N]
  landmark <name> = <value>                   a reference y (or any scalar)
  check <prop> <op> <term> [tol T]            op: ~ < > <= >= ; term: num|prop|land [+/- num]
                                              prop: <element>.{left,right,top,bottom,cx,cy,w,h}
  # ...                                       comment / SHOULD note (ignored, searchable)
"""
import sys, time, re, os, math, tempfile
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "scene.draw")
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

LINE = re.compile(r"Line\s*\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s*-\s*"
                  r"\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s*(?:w\s*=\s*(\d+))?", re.I)
SIZE = re.compile(r"size\s+(\d+)\s*x\s*(\d+)", re.I)
CIRCLE = re.compile(r"Circle\s*\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)\s*r=([-\d.]+)"
                    r"(?:\s+n=(\d+))?(?:\s+flat=([-\d.]+))?(?:\s+w=(\d+))?", re.I)
PT = re.compile(r"\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)")
WOPT = re.compile(r"w\s*=\s*(\d+)", re.I)
LAND = re.compile(r"landmark\s+(\w+)\s*=\s*([-\d.]+)", re.I)


def parse(text):
    W = H = 800
    segs = []
    elems = {}        # name -> [minx, miny, maxx, maxy]
    landmarks = {}
    checks = []
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

    def addseg(x1, y1, x2, y2, w):
        segs.append((x1, y1, x2, y2, w)); acc(x1, y1); acc(x2, y2)

    for raw in text.splitlines():
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
            continue
        if low.startswith("check"):
            checks.append(s[5:].strip()); continue
        m = SIZE.fullmatch(s)
        if m:
            W, H = int(m[1]), int(m[2]); continue
        m = CIRCLE.search(s)
        if m:
            cx, cy, r = float(m[1]), float(m[2]), float(m[3])
            n = int(m[4]) if m[4] else 28
            flat = float(m[5]) if m[5] else 0.0
            w = int(m[6]) if m[6] else 3
            ary = r * (W / H) * (1 - flat)
            pts = [(cx + r*math.cos(2*math.pi*i/n), cy + ary*math.sin(2*math.pi*i/n))
                   for i in range(n + 1)]
            for p, q in zip(pts, pts[1:]):
                addseg(p[0], p[1], q[0], q[1], w)
            continue
        if low.startswith("poly"):
            pts = [(float(a), float(b)) for a, b in PT.findall(s)]
            wm = WOPT.search(s.split(")")[-1])
            w = int(wm[1]) if wm else 3
            for p, q in zip(pts, pts[1:]):
                addseg(p[0], p[1], q[0], q[1], w)
            continue
        m = LINE.search(s)
        if m:
            w = int(m[5]) if m[5] else 3
            addseg(float(m[1]), float(m[2]), float(m[3]), float(m[4]), w)
    return W, H, segs, elems, landmarks, checks


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
    """Return dict: text, ok, target (for overlay), prop (lhs token)."""
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


def write_stats(img, W, H, nseg, elems, results):
    gray = img.convert("L")
    gw, gh = gray.size
    cw, ch = gw / GRID_COLS, gh / GRID_ROWS
    lines = [f"segments: {nseg}", f"paper: {W}x{H}", ""]
    lines.append("density (darker = more ink):")
    for ry in range(GRID_ROWS):
        row = []
        for rx in range(GRID_COLS):
            box = (int(rx*cw), int(ry*ch),
                   max(int((rx+1)*cw), int(rx*cw)+1), max(int((ry+1)*ch), int(ry*ch)+1))
            dark = 255 - gray.crop(box).getextrema()[0]
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
    with open(STATS, "w") as f:
        f.write("\n".join(lines) + "\n")


def render():
    try:
        text = open(SRC).read()
    except FileNotFoundError:
        text = ""
    W, H, segs, elems, landmarks, checks = parse(text)
    img = Image.new("RGB", (W, H), "white")
    d = ImageDraw.Draw(img)
    for x1, y1, x2, y2, w in segs:
        d.line([(x1*W, y1*H), (x2*W, y2*H)], fill="black", width=w)
    img.save(OUT)
    ph = max(1, round(PREVIEW_W * H / W))
    img.resize((PREVIEW_W, ph)).save(PREVIEW)

    results = [eval_check(c, elems, landmarks) for c in checks]

    # overlay: targets/landmarks/bboxes drawn so the gap is VISIBLE
    over = img.copy()
    od = ImageDraw.Draw(over)
    GRAY, GREEN, RED = (170, 170, 170), (40, 150, 40), (220, 40, 40)
    for b in elems.values():
        od.rectangle([int(b[0]*W), int(b[1]*H), int(b[2]*W), int(b[3]*H)], outline=GRAY)
    for v in landmarks.values():
        od.line([(0, int(v*H)), (W, int(v*H))], fill=GREEN, width=1)
    for r in results:
        if not r["ok"] and r["target"] is not None and r["prop"] and \
           any(r["prop"].endswith(s) for s in (".cy", ".top", ".bottom")):
            y = int(r["target"] * H)
            od.line([(0, y), (W, y)], fill=RED, width=2)
    over.save(CHECKIMG)

    write_stats(img, W, H, len(segs), elems, results)


def main():
    print(f"sketch: source={SRC}  out={OUTDIR}", file=sys.stderr)
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
