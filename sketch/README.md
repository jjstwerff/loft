# sketch — an iterative drawing tool

A companion to [`../doc/claude/DRAWING.md`](../doc/claude/DRAWING.md) (the design rationale). Drawing here
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
Line (x1,y1)[@N] - (x2,y2)[@N] [w=N] [stroke=R,G,B]
Circle (cx,cy) r=R [n=N] [flat=F] [w=N] [<fill> | stroke=R,G,B] round (aspect-corrected)
Poly (x1,y1)[~][@N] (x2,y2)[~][@N] ... [w=N] [<fill> | stroke=R,G,B] stroke, or filled
  <fill> = fill=L | rgb=R,G,B                                solid (gray / colour)
         | grad=R,G,B>R,G,B [dir=ax,ay,bx,by]                linear gradient (c1->c2)
         | radial=R,G,B>R,G,B [at=cx,cy,r]                   radial gradient (centre->edge)
landmark <name> = <value>
check <prop> <op> <term> [tol T]        op: ~ < > <= == ; arithmetic on the RHS only
                                        prop: <element>.{left,right,top,bottom,cx,cy,w,h}
# ...                                    comment / SHOULD note
```

- Coords are **fractions** of the paper (0..1); origin top-left, **y grows down**.
- Fills: `fill=L` is grayscale (L 0..1, 0 = black); `rgb=R,G,B` is colour (0..255). A
  shape with no fill is *stroked*; with one, it's *filled*.
- **Gradient fills (soft modelling).** `grad=c1>c2` is a linear gradient (c1 at the
  axis start → c2 at the end; default axis is vertical over the shape's bbox, or set
  it with `dir=ax,ay,bx,by` in fractions). `radial=c1>c2` runs centre→edge (centre =
  the shape's centroid, radius = half its extent, or set `at=cx,cy,r`). Both are
  computed small, resized, and supersampled, so the transition is smooth — use them
  for *form*: a rounded face (radial, lit centre → shadowed edge), a soft cloud
  belly, a glow, a lit-to-shadow plane. This is what turns a flat fill into modelling.
- **Smooth points (`~`).** A `~` after a `Poly` point makes it a smooth curve
  (Catmull-Rom); no `~` = a corner. Mix them on one outline — smooth the organic edges
  (a jaw, a hill, a cloud), corner the structural (a roof ridge). Faceted polygons
  become hand-drawn curves without subdividing by hand.
- **Coloured strokes (`stroke=R,G,B`).** A stroke (a shape with no `<fill>`) is dark
  ink by default; `stroke=` tints it. This is the *texture* channel — light + shadow
  strokes fanning along a growth direction make hair / beard / fur / grass; curve them
  with `~` so they read grown, not combed.
- **Per-point width (`@N`).** An `@N` after a point sets the pen width *at that point*
  (strokes only); width tapers linearly between points, drawn as a filled ribbon. A
  strand thick at the root (`@6`) thinning to a tip (`@0.5`) reads as a real hair — a
  uniform line reads as wire. Combine with `~` for a curved, tapering strand. `w=N` is
  the default for points without `@`.
- **Each measurable thing should get its own `name`** — tagging a sub-part (a chimney,
  a light-spill) under a parent bloats the parent's bbox and breaks its checks.

The `check` predicates are the **metric channel**: intent encoded as coordinates,
measured exactly and reported PASS/FAIL with deltas — so the eye never estimates "is
the sun high enough." See [`scene.draw`](scene.draw) for a worked example and
`example.png` for its render.
