# sketch — an iterative drawing tool

A small experiment companion to [`../DRAWING.md`](../DRAWING.md) (the design
rationale). Drawing here is the craft kind — a perceive → mark → see-gap → adjust
loop — not generative image synthesis. This tool's job is to make the *seeing*
cheap and to move metric judgments (position/size) off the eye onto exact
measurement.

## Requires

Python 3 + [Pillow](https://pillow.readthedocs.io) (`pip install pillow`).

## Run

```bash
python3 draw.py                 # renders ./scene.draw, watches it, re-renders on save
python3 draw.py path/to.draw    # a different scene
SKETCH_OUT=/some/dir python3 draw.py   # choose the output dir
```

Outputs go to `$SKETCH_OUT` (default `<tmp>/loft_sketch/`) — **never into the
repo**:

| file | what | when to read it |
|---|---|---|
| `stats.txt` | density map + element bboxes + `CHECK` results | most iterations — it's text, near-free |
| `canvas_check.png` | drawing + target guides (green landmark, gray bboxes, red = a failed check) | to *see* a gap |
| `canvas.png` | the clean drawing | final gestalt look |
| `preview.png` | small image | cheaper image look |

## The scene source

A scene is a plain text file: strokes, inline `# SHOULD:` intent comments, a
`# RULES` header, and `check` predicates. The renderer ignores `#` lines, so they
stay searchable (`grep -n SUN scene.draw`). Editing the file *is* undo.

```
size WxH
name <element>                            tag following strokes (so they can be measured)
Line (x1,y1) - (x2,y2) [w=N]
Circle (cx,cy) r=R [n=N] [flat=F] [w=N]   round by default (aspect-corrected)
Poly (x1,y1) (x2,y2) ... [w=N]
landmark <name> = <value>                 a reference position
check <prop> <op> <term> [tol T]          op: ~ < > <= == ; prop: <element>.{left,right,top,bottom,cx,cy,w,h}
# ...                                      comment / SHOULD note
```

The `check` predicates are the **metric channel**: intent encoded as coordinates,
measured exactly and reported in `stats.txt` as PASS/FAIL with deltas — so the
eye never has to estimate "is the sun high enough." See
[`scene.draw`](scene.draw) for a worked example, and `example.png` for its render.

Coords are fractions of the paper; origin top-left, y grows down.
