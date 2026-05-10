<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 3 — Projector view (beamer client)

## Status

Open.  Native loft renderer subscribed to the server, projected
onto the venue's main screen.  Effort: **M**.

## Goal

A loft program that:

1. Connects to the server (phase 1) as a WebSocket client.
2. Renders the **entire visible world** as a hex grid on screen,
   suitable for projection at 1080p+.
3. Auto-pans + auto-zooms the camera to follow audience activity
   (heat-field tracking).
4. Has a presenter override (hotkeys: lock camera, free-pan,
   reset zoom).

The projector view is the spectacle of the talk — what the room
collectively watches grow.  It is **passive**: it observes server
state, never sends events.

## Visual model — 3D crystal mesh

The projector renders the filled hexes as a **3D crystal mesh**
sitting on top of a flat hex-lattice background.  This is the
spectacle: the room watches crystals literally *grow upward* as
audience input spreads.

**Empty hexes are not part of the mesh.**  They render only as
the background lattice (outlined black hex tiles); no surface
geometry sits on them.  The mesh is built from filled hexes
only.

Per-filled-hex height + connectivity rules:

| Configuration | Crystal shape |
|---|---|
| Filled hex, 0 filled neighbors | **Lowest** — a single isolated crystal point |
| Filled hex, all 6 neighbors filled | **Highest** — a peak / plateau within a continuous mass |
| Filled hex, N neighbors filled (0 < N < 6) | Height monotonically rising with N |
| Two filled hexes with exactly one empty hex between them | Each crystal **extends laterally** toward the other, meeting above the empty cell to form a small **bridge** in the mesh.  The empty hex below stays black background |
| Line of filled hexes | Continuous **ridge** of crystals (since each filled hex has at least one filled neighbor along the line) |

The bridge effect is not "the empty hex has height" — it's that
each filled crystal's mesh **reaches toward** a partner filled
crystal one cell away.  When both reach toward each other, they
meet over the empty cell and form a span.  Cells with no partner
within bridge range have no lateral extension on that side.

**Reference aesthetic — ice forming on a cold window pane.**
Each crystal's silhouette is dendritic / fern-like: a central
spine with feathery secondary branches radiating outward, mostly
negative space, fine needle tips.  Bridges between adjacent
filled hexes look like two frost tendrils meeting in the middle,
not like solid arches.  Light passes through the gaps; colour
reads against the dark background; the structure feels grown,
not built.

**Crystals and bridges are not solid masses** — they're faceted
/ skeletal geometry with **visible negative space**.  The camera
can look *through* a bridge to see crystals behind it, and
through the gaps of any single crystal to see what is past it.

This means the per-crystal mesh is a sparse arrangement of
faces (spines, branches, needle tips) rather than a closed
hull.  Lighting + back-face visibility need to be tuned so the
through-look reads clearly on a projector at venue distance.

**Crystal tops are never flat plates.**  The mesh-generation
algorithm produces a top surface made of **ridge lines at even
height** (the frost spines) flanked by triangles that **slope
down into shallow crevices** (between branches).  Even a fully-
surrounded high-plateau cell shows a ridged / branched top, not
a smooth flat top.  Across many adjacent filled hexes, the ridge
lines preferentially connect into longer fern-like features,
while the crevices between them give the eye depth cues at the
appropriate scale.

### Plant vs crystal — emergent from filled-region shape

There is **one renderer**, not two.  The "plant" vs "crystal"
distinction the README mentions emerges from how the renderer
reads the shape of the filled region around each crystal:

- **Thin linear features (dotted, 1-wide, or 2-wide lines of
  filled hexes)** — the renderer detects the line direction and
  aligns each cell's ridges along it.  When the line **curves**,
  the ridges curve too: the bend is **anticipated 2 cells before
  and trailed 2 cells after** the sudden direction change, so
  the visual sweep is smooth, not stepped.  This produces the
  **plant / fern** aesthetic — flowing, organic stems that
  follow the audience's gesture.
- **Wider features (3+ hex blob, irregular cluster)** — edge
  detection cannot find a single dominant direction.  Each
  cell's ridges fall back to a **default crystal pattern**
  (radial / random with cluster-coherence bias).  This produces
  the **crystal** aesthetic — faceted territory with no
  dominant flow.

This means a single drawn line **swings between aesthetics** as
audience input continues: a thin meandering line reads as plant;
once it broadens into a blob (more taps fill in around it), the
edge detection drops out and the same area transitions to the
crystal look.  This swing is part of the spectacle — audiences
visibly change the world's character by changing how they paint.

The transition is not instantaneous: as edge detection weakens,
ridge directions blend toward the default before settling, so
the swing reads as a wave through the cluster rather than a
hard switch.

Per-cell rendering rules:

- **Empty hex (background)**: black fill, slightly lighter outline
  so the hex lattice is always visible — even on a blank world.
  No mesh geometry.
- **Filled hex (crystal)**: mesh built from triangles, each
  triangle a **single solid palette colour with a hard edge** to
  its neighbours.  No gradients, no per-pixel mixing, no alpha
  blending.  The colour mix on each crystal is a mosaic:
  - **Most** triangles take the cell's **own** palette colour.
  - **Some** triangles take a **1-away neighbour's** palette
    colour.
  - **A few** triangles take a **2-away tile's** palette colour.
  The dominance ordering is self ≫ 1-away ≫ 2-away, so the
  cell's own colour reads unambiguously at a glance.  At
  projection distance the eye fuses the mosaic into a perceived
  colour-bleed across cluster boundaries — but the underlying
  geometry is always solid-colour triangles with hard edges
  between them.  Faceting + lighting carry the height shape on
  top of the mosaic.  Older cells may desaturate / darken
  depending on the generation variant.
- **Recently-changed cell**: brief highlight pulse (e.g. white
  edge flash on the crystal, decays over ~500 ms) so the audience
  sees where growth is happening even when the camera is wide.

### Growth animation

When a new hex becomes filled — whether by audience tap or by
the generation tick — the crystal **grows over ~5 seconds** to
its target height, rather than snapping.  The growth has a
**direction**: it extrudes from the cluster of *earlier-placed*
hexes toward the new one.  Visually, the audience sees the new
crystal "reaching out" from the existing structure.

This implies the renderer needs:

- A per-hex `growth_progress` value (0.0 → 1.0 over 5 seconds
  from `planted_at`).
- A per-hex `growth_origin` direction (vector from the centroid
  of nearby older filled hexes to this hex).  The growing
  crystal tilts / leans along this vector while progress < 1.0.
- Smooth interpolation of the heightfield each frame so the
  surface deforms continuously rather than per-tick stepping.

Once `growth_progress` reaches 1.0, the cell sits at its
neighbor-derived height for the remainder of its lifetime.

### Visible structure on an empty world

Even before any seeds are placed, the projector shows the hex
lattice as a flat grid of outlined black hexes — the room sees a
structured canvas, not a void.  The first tap raises the first
crystal point.

## Auto-camera

The projector pans + zooms automatically based on a recent-
activity heat field.

| Quantity | Derivation |
|---|---|
| **Heat at hex H, time T** | Sum over recent events `e` (last few seconds) of `weight(e) * decay(T - t(e))` where `weight` is 1 per seed/clear and `decay` is exponential |
| **Camera target position** | Centroid of the heat field (heat-weighted mean of hex centres) |
| **Camera zoom** | Inverse function of activity spread — concentrated activity → zoom in, dispersed → zoom out.  Bounded between configured min/max |
| **Camera motion** | Lerp toward target each frame; never snap.  Smoothing constant tuned in rehearsal |

When activity is zero (no recent events), camera **holds** rather
than panning randomly — this avoids motion sickness during quiet
moments.  CI-4 picks between hold-still and slow-orbit.

## Presenter overrides (keyboard hotkeys)

| Key | Effect |
|---|---|
| Space | Toggle camera lock — freezes auto-pan, audience visible activity continues |
| Arrow keys | Pan manually (only while locked) |
| `+` / `-` | Zoom manually (only while locked) |
| `R` | Reset to auto-camera + zoom-to-fit-all |
| `H` | Toggle headline overlay (round number, palette legend) — useful when switching rounds |
| `Esc` | Quit cleanly (sends disconnect, closes WS) |

## Sub-tasks

| # | Task | Effort |
|---|---|---|
| 3.1 | WebSocket client (subscribe-only; reuse `lib/web` ws API) | XS |
| 3.2 | 3D hex-grid renderer at projection resolution (target 1920x1080+).  Likely fork `lib/moros_editor` — already a 3D hex world | M |
| 3.3 | Camera transform — world coordinates → screen pixels with pan + zoom | S |
| 3.4 | Per-filled-hex height + lateral-reach computation: height = `f(filled_neighbor_count)`; lateral reach toward each filled hex within bridge range (gap of 1) | S |
| 3.4a | Crystal-top mesh generator: ridge lines at even height flanked by triangles sloping into shallow crevices.  Never produces flat plates.  Ridge directions on adjacent filled hexes preferentially connect so multi-hex masses read as continuous geological features | M |
| 3.4c | Edge / line detection on the filled-hex pattern: classify each cell's local neighborhood as `thin-line` (dotted / 1-wide / 2-wide line, with a detected direction + curvature) or `blob` (3+ wide / irregular).  Thin-line cells take their ridge direction from the line tangent; blob cells use the default crystal pattern.  Line curvature lookahead extends 2 cells before + 2 cells after a bend so the curve is anticipated and trailed, not stepped | M |
| 3.4d | Aesthetic-swing transition: when a cell crosses the thin-line ↔ blob boundary as audience input changes the local shape, ridge directions interpolate smoothly toward the new pattern over a short window.  Reads as a visible wave through the cluster, not a hard pop | S |
| 3.4b | Per-triangle colour assignment: each triangle gets one solid palette colour from the {self, 1-away neighbours, 2-away tiles} set with dominance self ≫ 1-away ≫ 2-away.  Hard edges between triangles, no shader-side blending.  Stable assignment (same triangle keeps its colour across frames so the mosaic reads as texture, not noise) | S |
| 3.5 | Heat-field tracker — accumulate events, decay over time | S |
| 3.6 | Camera target derivation — centroid + spread → target + zoom | S |
| 3.7 | Smooth camera motion (lerp + smoothing constant tuning) | S |
| 3.8 | Growth-animation interpolation — `growth_progress` over 5 s, `growth_origin` direction from cluster centroid, surface deforms each frame | M |
| 3.9 | Recently-changed pulse highlight | XS |
| 3.10 | Presenter hotkey handlers | XS |
| 3.11 | Headline overlay (round number, palette legend, optional player count) | S |
| 3.12 | Pre-recorded backup capture — record a 5-min run as fallback video | XS |

## Open design questions

- **Build on `lib/moros_editor` or fresh?** — the world is 3D
  with a per-hex height field, which moros editor already
  supports (and `lib/graphics` underneath).  Strong recommend
  fork the moros renderer + replace its manual-orbit camera with
  the auto-camera here.  CI-3 confirms after a render-spike on
  representative geometry (~50 filled hexes, ~5 second growth in
  flight).
- **Crystal mesh shape — frost on cold glass** — crystals are
  not closed hulls.  Reference image: ice forming on a window
  pane — central spine + feathery branches + needle tips, mostly
  negative space.  Tops are ridge-and-crevice (never flat
  plates).  Open question: pick the procedural-frost generator
  (single-spine + perpendicular branches?  recursive dendrite?
  symmetric vs. asymmetric branching?) at CI-3 after a render-
  spike.  Keep triangle counts low enough that a busy world
  (~500 filled hexes) still hits frame budget.
- **Edge / line detection — exact classifier** — needs to
  reliably distinguish dotted-line vs 1-wide vs 2-wide line vs
  blob, return a tangent direction + curvature for each
  thin-line cell, and gracefully drop out for 3+-wide regions.
  Open questions at CI-3: what neighborhood radius does the
  classifier scan?  How is curvature computed (3-point fit?
  5-point fit?)?  How fast does the classifier converge when
  audience input changes the local shape (per frame? per server
  tick?)?
- **Aesthetic-swing window length** — when a cell crosses the
  thin-line ↔ blob boundary, the interpolation window decides
  how long the visible transition takes.  Too short = pops; too
  long = the audience can fill the area before the transition
  completes and the visual lags behind reality.  First cut:
  ~1 second, tune at CI-3.
- **Triangle-colour mix ratios** — exact share for self vs
  1-away vs 2-away (each triangle is one solid palette colour;
  this picks how many of the crystal's triangles fall into each
  bucket).  First cut: ~70% self / ~25% spread across 1-away
  neighbours / ~5% spread across 2-away tiles, biased by each
  neighbour's own colour count.  CI-3 picks the final numbers
  after seeing the mosaic at projection scale — too many
  neighbour-coloured triangles = the cell's own colour gets
  lost, too few = the crystal reads as a flat solid block
  instead of a faceted mosaic.
- **Bridge geometry — exact mesh shape** — each filled crystal
  has a base mesh whose top facets can extend laterally toward a
  filled partner one cell away.  Like the crystals themselves,
  bridges are **not solid** — they consist of struts / plates
  with gaps the camera can see through.  Two further questions:
  (a) how far does the lateral reach extend (just to the empty
  cell's centre?  past it to meet the partner?), (b) does the
  bridge top arch upward, stay level, or sag?  CI-3 picks after
  a render-spike.
- **Growth animation cost** — 5-second per-hex interpolation
  with N concurrent growths means every frame must process N
  active growths.  At ~60 FPS × N=50 = manageable; at N=500
  (very busy round) needs measurement.  Mitigation: cap
  concurrent growths at the renderer (queue overflow grows
  faster) or skip per-frame for low-progress hexes.
- **Camera framing** — fit-to-content (always show all painted
  cells) vs. follow-action (zoom in on the busiest cluster).
  Fit-to-content is "fair"; follow-action is "exciting."
  Recommend follow-action with periodic auto-zoom-out so the
  audience sees what they're missing.
- **Idle behavior** — slow drift over the painted region (camera
  treats whole world as low-heat) vs. hold still.  Test in
  rehearsal.
- **Recently-changed pulse** — every changed cell pulses, or
  only audience-driven changes (not generation-driven growth)?
  Pulsing every cell distracts from the growth aesthetic;
  pulsing only audience changes makes it clear what the audience
  is doing.  Recommend audience-only.

## Deliverable

A native loft binary (`lib_examples/audience_demo/projector.loft`
or similar) that, given a server URL on argv:

- Connects, subscribes, and renders the world full-screen.
- Auto-cameras smoothly to recent activity.
- Responds to all presenter hotkeys.
- Recovers cleanly from server disconnect (shows a "RECONNECTING"
  overlay; resumes when the server returns).

## See also

- [`README.md`](README.md) — parent plan
- [`00-audience-browser-page.md`](00-audience-browser-page.md) —
  same visual model at smaller extent
- [`01-server-state.md`](01-server-state.md) — the source of
  truth this view subscribes to
- `lib/moros_editor/` — possible basis for renderer
- `lib/graphics/examples/` — existing creative-coding examples
