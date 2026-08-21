#!/usr/bin/env python3
"""@PLN146 W0 — what grammar does the corpus actually use?

Arc W's loft port owes exactly this surface and no more.  Run over `scenes/`
and print, per command, how many scenes use it and which option keys appear.
"""
import collections, glob, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
CMDS = ("size", "Background", "name", "Line", "Circle", "Poly", "Petals",
        "Fronds", "landmark", "check")
OPT = re.compile(r"(?<![\w.])([A-Za-z]\w*)\s*=")
FILLS = ("fill", "rgb", "grad", "radial")

cmd_scenes = collections.defaultdict(set)
cmd_lines = collections.Counter()
cmd_opts = collections.defaultdict(collections.Counter)
extras = collections.Counter()
smooth = set()
per_point_w = set()

for path in sorted(glob.glob(os.path.join(HERE, "scenes", "*.draw"))):
    scene = os.path.basename(path)[:-5]
    for raw in open(path, encoding="utf-8"):
        s = raw.split("#", 1)[0].strip()
        if not s:
            continue
        head = s.split()[0]
        if head not in CMDS:
            extras[head] += 1
            continue
        cmd_scenes[head].add(scene)
        cmd_lines[head] += 1
        for key in OPT.findall(s):
            cmd_opts[head][key] += 1
        if head in ("Poly", "Line"):
            if "~" in s:
                smooth.add(scene)
            if re.search(r"\)\s*@", s):
                per_point_w.add(scene)
        if head == "Background" and "transparent" in s:
            cmd_opts[head]["transparent"] += 1

total = len(glob.glob(os.path.join(HERE, "scenes", "*.draw")))
print(f"corpus: {total} scene(s)\n")
print(f"{'command':<12}{'scenes':>7}{'lines':>7}  options used")
for c in CMDS:
    if c not in cmd_scenes:
        print(f"{c:<12}{0:>7}{0:>7}  — UNUSED by the corpus")
        continue
    opts = ", ".join(f"{k}({n})" for k, n in sorted(cmd_opts[c].items()))
    print(f"{c:<12}{len(cmd_scenes[c]):>7}{cmd_lines[c]:>7}  {opts or '—'}")
print()
print(f"smooth points  `~`   : {len(smooth)} scene(s)")
print(f"per-point width `@N` : {len(per_point_w)} scene(s)")
if extras:
    print("\nlines matching no known command (these would be UNPARSED):")
    for k, n in extras.most_common():
        print(f"  {k}  x{n}")
