<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN144 — Renderer doctrine, inherited from crawler's RENDER.md

`crawler/RENDER.md` (422 lines) designs a showcase-grade 2D GPU renderer for loft in more
depth than this plan's arc A did, and **crawler is no longer building it**: its scope note of
2026-07-22 says the 2D view is being replaced by first-person 3D (their plan #11 retires
`view.loft` in P9), leaving the `graphics` substrate flow-backs as what still earns its keep.

So this is not a competing design to reconcile — it is **orphaned doctrine to inherit**, and
this plan is the right home for it. What follows is what arc A adopts, what it declines, and
the one place the two models genuinely differ.

## Adopted

- **"Accumulate, merge adjacent, flush — NEVER reorder."** 2D correctness is painter's
  algebra: overlapping translucent draws must composite in call order, so the batcher merges
  *consecutive* state-compatible calls and breaks on a state change. It does not sort.
  Arc A derived the same rule from the walk-behind-a-fence case; RENDER.md states it as the
  batcher's founding contract, and got there first.
- **Premultiplied-alpha atlas**, composited `(ONE, ONE_MINUS_SRC_ALPHA)`, with **1 px padding
  per entry** (no linear-filter bleed) and **mipmaps when minified**. A5 had the
  premultiplication; the padding and the mipmaps are theirs, and both are the difference
  between a rotated sprite compositing correctly and nearly correctly.
- **The atlas builds itself, at load time, with no programmer direction.** Skyline/shelf
  packing into ~2048² pages at `load_image` time — the natural off-frame point, since packing
  on first *draw* hitches the hot frame. Heuristics replace direction: an oversized image
  bypasses to its own texture, a full page opens the next, dynamic entries (glyphs) get LRU
  eviction. This supersedes F1's earlier "the packer decides A3's batch count", which made a
  human responsible for something a heuristic does better.
- **Per-instance 2×3 affine** rather than pos/rot/scale — their 2.5D spec delta. A skew
  shadow is then a sheared dark re-emission of the same atlas entry, for free.
- **Frame stats that name the REASON for each batch break**, plus atlas-page occupancy. The
  price of "behind the curtain" is that degradation is invisible; this is what keeps the
  no-burden promise honest under failure rather than only on the happy path. It is also a
  sharper spec than this plan's stats-overlay tool idea, and replaces it.
- **The honest cost, stated by them:** y-sorted emission interleaves materials, so a 2.5D
  scene takes more batch breaks than a layer-grouped 2D one. The shared atlas and ubershader
  mitigate; the frame stats measure it. Arc A's A3 cliff is this, named earlier.

## Declined for now

SDF shape fast paths, arbitrary paths with tessellation caching, gradients and the post-fx
chain. All of it is good and none of it is needed to make a game authorable — it is arc G's
territory, and pulling it in now would trade the plan's shippability for its ambition.

## Where the two models differ — and the seam that resolves it

RENDER.md's user-facing model is a **NanoVG-class immediate canvas**, and it explicitly
declines a retained scene graph, citing Clutter/Cogl as "the overcorrection — a retained GPU
scene graph as the USER-facing model; the simple case got harder and it never displaced
Cairo". Its stated divergence from GTK4 is *no node TREE*, because "a rotating-camera
roguelike redraws the full viewport every frame, so flat instance buffers win and per-node
tree-building overhead buys nothing".

Two things dissolve most of that tension:

1. **Arc A is not a node tree either.** A1's design call is a **flat `vector<Node>` with
   integer parent indices** — chosen to avoid an ownership cycle, and it happens to be the
   flat instance buffer RENDER.md argues for. The per-node tree-building overhead they
   reject is not something this design pays.
2. **RENDER.md already contains the seam.** Its §5 — *static content = the same verbs,
   recorded*: `record … → draw_batch(batch, mvp)` — is a retained display list arrived at
   from the other side. `stage` is a **producer of those recorded batches**, not a rival
   renderer: one batcher, one atlas, one premultiplied path, two authoring models available.

What genuinely remains is a question of **who owns the scene between frames**, and the answer
is decided by the consumer, not by the library. A roguelike that redraws everything every frame
wants the canvas. A GameMaker-shaped game with thousands of instances and a moving camera
wants the retained form, because A8's camera is then one uniform instead of an O(N) rewrite.
Both sit on the same batcher, which is the only part that must not be duplicated.
