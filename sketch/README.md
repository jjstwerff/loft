# sketch — an iterative drawing tool

A companion to [`../DRAWING.md`](../DRAWING.md) (the design rationale). Drawing here
is the craft kind — a perceive → mark → see-gap → adjust loop — not generative image
synthesis. The tool's job is to make the *seeing* cheap and to move metric judgments
(position / size / composition) off the eye onto exact measurement.

## Requires

Python 3 + [Pillow](https://pillow.readthedocs.io) (`pip install pillow`).

## Run

```bash
python3 draw.py                 # renders ./scene.draw, watches it, re-renders on save
python3 draw.py path/to.draw    # a different scene
SKETCH_OUT=/dir python3 draw.py # choose the output dir (default <tmp>/loft_sketch)
```

Editing the scene file *is* undo (reload on save). Rendering is **supersampled**
(drawn at 3× and downscaled) so edges are anti-aliased, not hard polygon facets.

## Outputs (in `$SKETCH_OUT`, never the repo)

| file | what | when to read it |
|---|---|---|
| `stats.txt` | density map + element bboxes + `CHECK` results + a **composition/notan report** | most iterations — text, near-free |
| `canvas.png` | the drawing | the gestalt look |
| `canvas_check.png` | drawing + target guides (green landmark, gray bboxes, red = failed check) | to *see* a gap |
| `preview.png` | small image | a cheaper image look |

The **composition report** (in `stats.txt`) gives the global, gestalt facts no
per-element check can: L/R and top/bottom weight balance, the center-of-mass of the
darks vs the thirds grid, the value spread (notan health), and the largest flat
region (dead space / dominant field). *Caveat:* it measures **dark** mass — in a
light-on-dark scene the true focal point is the bright region, which it won't track.

## The scene source

Plain text: marks + inline `# SHOULD:` intent comments + `check` predicates. The
renderer ignores `#` lines, so they stay searchable (`grep -n SUN scene.draw`).

```
size WxH
Background top=A bottom=B               grayscale sky gradient (A top, B bottom; L in 0..1)
Background topc=R,G,B botc=R,G,B         colour sky gradient
name <element>                          tag following marks (so they can be measured)
Line (x1,y1) - (x2,y2) [w=N]
Circle (cx,cy) r=R [n=N] [flat=F] [w=N] [fill=L | rgb=R,G,B]   round (aspect-corrected)
Poly (x1,y1) (x2,y2) ... [w=N] [fill=L | rgb=R,G,B]            stroke, or filled
landmark <name> = <value>
check <prop> <op> <term> [tol T]        op: ~ < > <= == ; arithmetic on the RHS only
                                        prop: <element>.{left,right,top,bottom,cx,cy,w,h}
# ...                                    comment / SHOULD note
```

- Coords are **fractions** of the paper (0..1); origin top-left, **y grows down**.
- Fills: `fill=L` is grayscale (L 0..1, 0 = black); `rgb=R,G,B` is colour (0..255). A
  shape with neither is *stroked*; with one, it's *filled*.
- **Each measurable thing should get its own `name`** — tagging a sub-part (a chimney,
  a light-spill) under a parent bloats the parent's bbox and breaks its checks.

The `check` predicates are the **metric channel**: intent encoded as coordinates,
measured exactly and reported PASS/FAIL with deltas — so the eye never estimates "is
the sun high enough." See [`scene.draw`](scene.draw) for a worked example and
`example.png` for its render.
