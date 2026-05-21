<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 3 — Projector view (beamer client)

## Status

**Partial — flat-2D MVP with FPS-style auto-camera shipped**
(2026-05-20).  The native loft renderer
[`tools/audience-demo/projector.loft`](../../../../tools/audience-demo/projector.loft)
opens a 1280×800 OpenGL window, subscribes to the multi-client server,
and renders the hex world as a flat-2D pointy-top grid with the
empty-cell backdrop visible for spatial reference.

**Removal fade-out shipped** (2026-05-20): the projector now matches the
browser client's fade.  The cell shader was unified on **per-vertex
RGBA** (stride-10 `gl_upload_vertices` layout — `pos.xyz` + unused normal
pad + `rgba` at `location 2`) with `GL_BLEND` enabled, so any vertex can
carry alpha.  Opaque cells bake `a = 1.0` (no visual change); an erased
cell's ground footprint is re-emitted each frame at `a` ramping 1 → 0
over `FADE_FRAMES` (~600 ms, matching the browser's `FADE_MS`), keyed in
a `World.fades` hash so a re-paint cancels an in-flight fade.  No library
change — the RGBA layout was already supported.  Also unlocks the
"old paints recede into history" alpha effect noted under § Growth.

**Per the 2026-05-21 design clarification:** the OpenGL client is BOTH
the projector (3D crystals only, auto-camera, no ground grid) AND
the desktop client (3D crystals + 2D ground layer, user mouse paints
into the ground layer).  The flat-2D rendering that already ships
stays — it becomes the desktop client's GROUND LAYER, and the 3D
crystal mesh layers on top.

**Mode is a runtime state, not a CLI flag.**  Two modes:

| Mode | Trigger | Camera | Ground layer | Mouse paint |
|---|---|---|---|---|
| **Player-controlled** | Any mouse / scroll / keyboard input — also the initial state at startup | Free (mouse-drag rotate, scroll zoom, WASD pan) | Visible | Yes |
| **Demo (= projector)** | No input for `CAMERA_IDLE_TIMEOUT_S` (initial guess: 60 s) | Auto-tracks latest edit (existing single-edit camera or the future heat-field overview) | Fades out over `CAMERA_GROUND_FADE_S` (initial: 3 s) | No |

Demo mode reverts to player-controlled mode as soon as the user
touches mouse or keyboard.  The ground layer fades back in over the
same `CAMERA_GROUND_FADE_S`.  This is the natural projector mode
for the talk — the presenter sets it up, leaves it alone, and after
a minute it becomes the pure spectacle.

Auto-camera shipped (matches the Auto-camera section below, just on
2D first):
- **FPS pose**: tilt = pitch, rotation = yaw, position = ground
  coordinate.  Looking down (tilt=0) is the at-rest extremity; tilting
  forward (tilt>0) compresses screen-Y so distant ground reads closer
  to centre.
- **Look-at view shift**: `view = pos + tilt * (end - pos)` — at full
  tilt the goal sits at screen centre, even before the camera has
  translated.
- **Edit sequence**: new paint → tilt kick + rotation to face it →
  ease-in over 0.2 s → orient settles within ~6° → 0.1 s pause →
  translate → rotation re-tracks if a follow-up paint arrives mid-trip.
- **Catchup**: msg_id 6 snapshot-request handshake the projector
  fires on connect + as a watchdog if the world stays empty.

Matrix order is FPS-correct: `T × S_screen_y × R × S_uniform` (uniform
scale → rotate → screen-Y squash → translate), so pitch foreshortening
rides on the screen Y axis regardless of yaw.

Remaining for full phase 3: 3D crystal mesh (see § Visual model below
— frost ridges, bridges between cluster cells, hard-edge palette
triangles per the design).  The auto-camera layer is ready to drive a
3D mesh layer the moment the geometry generator lands.

Effort remaining: **M** (the mesh generation is the bulk; the
camera + WS wiring is done).

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

- **Empty hex (background)**: NOT rendered.  The projector is the
  pure spectacle — only filled-hex crystals appear; empty regions
  read as the dark background of the venue, not as an outlined
  lattice.  The base hex grid is **not visible on the projector**
  (the desktop client renders it, but the projector does not).
  This keeps the auditorium's gaze on the crystal mesh.
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

- A per-cell `growth_progress` value derived from the cell's
  `c_age` (0.0 at age 0; 1.0 once age ≥ ~5 seconds × tick rate,
  e.g. 50 ticks at 10 Hz).  Server ships `c_age` as 2 bytes; no
  separate `planted_at` field.
- A per-cell `growth_origin` direction, **computed client-side**
  by comparing this cell's age to its 6 neighbours' ages.
  Older-neighbour cluster on one side ⇒ direction extrudes from
  there toward this cell.  Recomputed each frame for cells whose
  growth_progress < 1.0; not needed once growth completes.
- Smooth interpolation of the surface each frame so the mesh
  deforms continuously rather than per-tick stepping.

Once `growth_progress` reaches 1.0, the cell sits at its
server-supplied height (`c_height` from the chunk payload) for
the remainder of its lifetime.

### Distance fade + adaptive draw distance

Crystals beyond a configurable distance from the camera centre
**fade out** (alpha → 0 over a fade band) rather than being
abruptly clipped at the view edge.  Anchors audience attention
on the focused cluster and lightens render cost on far-away
crystals.

| Constant | Default |
|---|---|
| `CAMERA_FADE_START_HEXES` | 100 (fade begins at this many hex-distances from the camera centre) |
| `CAMERA_FADE_BAND_HEXES` | (CI-3 picks; first cut ~20 hex distances of falloff) |
| `CAMERA_FADE_MIN_HEXES` | 30 (lower bound the auto-tuner is allowed to shrink the fade-start to under load) |
| `CAMERA_FADE_MAX_HEXES` | 250 (upper bound the auto-tuner can grow to when frames are cheap) |

Each client (desktop + projector — the phone is 2D and skips
this entirely) **measures its own frame time** and adapts
`CAMERA_FADE_START_HEXES` between the configured min and max:

- Frame time consistently above the budget (e.g. >18 ms at
  60 FPS target) → shrink the draw distance by a small step
  each second to reduce render cost.
- Frame time comfortably under budget (e.g. <12 ms) → grow the
  draw distance by a small step each second until either the
  max is reached or frame time approaches the budget.

The auto-tuner is intentionally slow (per-second adjustments,
small steps) so the audience does not see crystals popping in
and out as the distance changes.  All four constants live as
renderer-side values tunable at the prototype + rehearsal
stages without touching server code.

Rendering note: the fade applies to the crystal mesh only — the
projector still does not render the base hex lattice at all, so
the fade has nothing to mute on empty cells.

### Chunked mesh lifecycle — bounded memory + render cost

**Mesh lifetime tracks 32×32 chunks, not individual cells.**  This
matches the server-side `lib/world` chunk layout (the same one the
phone client + audience server already use) and bounds the projector's
mesh count to "chunks within the camera's draw distance" rather than
"every cell painted in the lifetime of the demo".

Per chunk in the projector:

| State | Meaning | Renders? |
|---|---|---|
| **Unloaded** | Chunk's cells exist in the server world map but no GPU meshes built | No |
| **Loading** | Chunk just entered fade range; per-cell meshes being built (one per frame to spread cost) | Partial — only the cells already built |
| **Loaded** | All cells' meshes built and uploaded | Yes (with distance fade alpha applied) |
| **Fading** | Camera moving away; alpha decaying toward 0 over the fade band | Yes (decaying alpha) |
| **Discarded** | Alpha reached 0; GPU meshes freed | No |

Per frame, the projector computes each chunk's distance from the
camera centre and transitions states accordingly:

- Distance < `CAMERA_FADE_START_HEXES`: chunk loads (if not already)
  and renders at full alpha.
- `CAMERA_FADE_START_HEXES` < distance < `CAMERA_FADE_START_HEXES +
  CAMERA_FADE_BAND_HEXES`: chunk renders with `alpha = 1 - (d - start)
  / band`.  No new cell mesh built for this chunk in fading state —
  only existing meshes fade out.
- Distance > `CAMERA_FADE_START_HEXES + CAMERA_FADE_BAND_HEXES`:
  chunk discarded.  GPU buffers freed.

When a new audience paint lands in a chunk that's currently in
`Fading` or `Discarded` state, the chunk stays in that state — the
new paint waits until the camera approaches that chunk again before
its mesh is built and shown.  This is intentional: the user explicitly
wants the chunk system to be used as a **fade-out + don't-paint-next
mechanism** so the projector never grows unbounded mesh storage and
never tries to render activity in chunks the audience can't see.

The natural consequence: chunks beyond fade distance accumulate
server-side state but contribute zero GPU cost.  Memory + render time
stay bounded by `(chunks_in_view) × (32 × 32)` cells, regardless of
total demo runtime.

### Visible structure on an empty world

A blank world reads as a dark stage — no hex lattice on the
projector.  The first audience tap raises the first crystal,
which is the first thing the room sees on the screen.  This
deliberately contrasts with the desktop client (which always
shows the base lattice) and the phone client (which IS the flat
lattice).  The projector is reserved for crystals.

## Auto-camera

**Shipped (2026-05-20):** single-edit "look-at" camera with FPS-style pose.
On each new paint event the camera anticipates with a tilt kick, rotates
to face the edit, pauses 0.1 s, then translates with the goal at screen
centre via a look-at view shift.  See `tools/audience-demo/projector.loft`
for the implementation.  Tuning knobs live in one block at the top of the
camera section (CAMERA_LERP, CAMERA_LOOKAT_DIST, CAMERA_PAUSE_FRAMES, …).

**Future — multi-edit move-up + zoom-out phase**
*(design captured; not yet implemented.  Pickup signal: recent
activity spans more than one region at the same time and the
audience is actively painting in multiple far-apart spots.)*

### Why the current camera is insufficient

The shipped single-edit camera (look-at + tilt + pause + translate)
takes the simple path: target = latest paint, view shifts to it,
camera lands there.  When activity stays in one region, this reads
as "the camera follows the action".  When activity is dispersed
(two audience members painting in opposite corners), the camera
ping-pongs between them — each new paint pulls it away from the
previous one, and the OVERVIEW of the world the audience has built
gets lost on every move.

Live observation 2026-05-20: with edits spread across `(-9, 45)` →
`(-32, 7)` → `(-22, -2)` → `(-4, -53)` in a few seconds, the
camera chased each one, hiding the surrounding structure between
moves.  Audience can no longer "read" the world as a whole.

### Heat field — recent activity centroid + spread

Replace the single-target model with a heat field of recent events.
Each `4:` paint / `4:c=0` erase the projector receives gets recorded
in a fixed-size ring of `(x, y, tick)` tuples (capped, e.g., last 32
events).  Per frame, compute:

| Quantity | Derivation |
|---|---|
| **Per-event weight `w(e)`** | `exp(-(now - e.tick) / TAU_FRAMES)` — exponential decay, half-life ≈ 4 s |
| **Effective count** | `Σ w(e)` — fractional "how many events are still relevant"; collapses to 0 when nothing recent |
| **Centroid (cx, cy)** | `(Σ w·x / Σ w,  Σ w·y / Σ w)` — heat-weighted mean of recent positions |
| **Spread radius `r`** | `√(Σ w·((x−cx)² + (y−cy)²) / Σ w)` — heat-weighted RMS distance from centroid |
| **Latest edit (lx, ly, lt)** | The most recent ring entry by tick — fed into the existing single-edit anticipation gestures |

When `r` is small (all activity in one region), the heat field
behaves like the single-edit model — centroid ≈ latest paint,
single-edit camera takes over.  When `r` grows large, the camera
zooms out + moves up so the whole heat field is visible at once.

### Two operating modes, with a smooth transition

| Mode | Trigger | Camera behaviour |
|---|---|---|
| **Single-edit** | `r < SPREAD_NARROW` (e.g., 6 hexes) | Current shipped behaviour: target = latest paint, look-at shift, full tilt anticipation.  Zoom = 1.0. |
| **Overview** | `r > SPREAD_WIDE` (e.g., 14 hexes) | Camera target = centroid.  Zoom OUT so the heat field's bounding circle fits comfortably in view.  Tilt drops toward 0 (top-down looking down on the whole region).  Move-UP feeling comes from the zoom-out combined with tilt → 0 — visually equivalent to the camera lifting to a higher altitude. |
| **Blended** | `SPREAD_NARROW ≤ r ≤ SPREAD_WIDE` | Lerp every parameter (target between latest-edit and centroid, zoom between 1 and overview zoom, tilt between anticipation and 0) by the normalised `r` position in the band.  Continuous transition; no mode-flip jitter. |

The "move up" of the user spec maps to the zoom-out + tilt-toward-0
combination — there's no real Z axis in the flat-2D projector, but
visually "smaller hexes spread further apart with top-down view"
reads exactly as "the camera rose to a higher altitude".  When the
3D crystal mesh ships, this can grow into a real Z translation; the
heat-field math stays the same.

### Tuning constants (initial guesses, rehearse to validate)

```
HEAT_BUFFER_SIZE  = 32      // ring buffer capacity (≈ 3 s of typical paint cadence)
TAU_FRAMES        = 240     // ~4 s half-life at 60 fps
SPREAD_NARROW     = 6.0     // hex distance below which single-edit mode is fully active
SPREAD_WIDE       = 14.0    // hex distance above which overview mode is fully active
OVERVIEW_FIT_PAD  = 1.5     // zoom-out factor; bounding circle * this = screen radius
OVERVIEW_ZOOM_MIN = 0.20    // never zoom out below this (preserves readability of hex tiles)
```

### Caps

- Zoom is hard-capped at `1.0` — never zooms IN above normal.  User
  spec: "the camera should not zoom in on distant edits, that
  hampers the immersion and the overview of the current structure".
- Zoom is hard-capped at `OVERVIEW_ZOOM_MIN` — too far out and hex
  tiles become unreadable; the world becomes abstract dots.
- "Move up" is purely visual (zoom + tilt drop).  Phase-3 3D crystal
  mesh might later introduce a real Z translation as an additional
  channel of the same blend; not in scope here.

### Implementation gates

@P287 closed (2026-05-20) the struct-field slice-assignment crash that
previously blocked the ring buffer's trim-oldest pattern.  With that
fix shipped, the ring can be a fixed-size `vector<PaintEvent>` with
the parser-side materialisation handling the trim — no special
in-place mutation needed.

When activity is zero (no recent events with non-negligible weight),
camera **holds** rather than panning randomly — this avoids motion
sickness during quiet moments.  CI-4 picks between hold-still and
slow-orbit.

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

- ~~**Build on `lib/moros_editor` or fresh?**~~ — RESOLVED
  2026-05-10: **fork the lib/moros_editor renderer**.  Moros
  editor already has a 3D hex-world renderer + camera +
  lighting on top of `lib/graphics`.  This demo forks the
  renderer module, swaps the manual-orbit camera for an
  auto-camera (projector) or mouse-drag + WASD camera (desktop
  client), and drops the wall / item / spawn-point gear that
  this demo doesn't need.  Saves a lot of plumbing and reuses
  tested code.  CI-3 confirms with a render-spike on
  representative geometry (~50 filled hexes, ~5 second growth in
  flight) that the fork is the right shape before the projector
  + desktop renderer code paths diverge.
- ~~**Crystal mesh shape — frost on cold glass**~~ — RESOLVED
  2026-05-10: **recursive dendrite generator**.  Each crystal
  is a branching tree — central spine extrudes upward,
  secondary branches fork at angles, tertiary needles fork off
  those.  Depth budget (suggested 3 levels) caps triangle count.
  Symmetric vs. asymmetric branching is a remaining tuning
  knob: pick at CI-3 after a render-spike on representative
  geometry.  Keep triangle counts low enough that a busy world
  (~500 filled hexes) still hits frame budget; reduce depth
  level dynamically if the budget is exceeded.
- **Edge / line detection — exact classifier** — needs to
  reliably distinguish dotted-line vs 1-wide vs 2-wide line vs
  blob, return a tangent direction + curvature for each
  thin-line cell, and gracefully drop out for 3+-wide regions.
  - **Neighborhood radius**: RESOLVED 2026-05-10 at **radius 2
    (5×5 hex window, ~19 hex lookups per cell)** as the
    starting default.  Enough to catch 1-wide and 2-wide lines
    and the 2-cell curve lookahead.  Re-validate once the
    first prototype is running; if ambiguity is visible, widen
    to radius 3 or pick the adaptive path.
  - Remaining at CI-3: how curvature is computed (3-point fit?
    5-point fit?), how fast the classifier converges when
    audience input changes the local shape (per frame? per
    server tick?).
- ~~**Aesthetic-swing window length**~~ — RESOLVED 2026-05-10
  at **~2 seconds**.  Cinematic; the swing reads as a longer-
  form motion through the cluster, matching the ~5 s growth
  animation tempo.  Trade-off accepted: audience filling an
  area faster than 2 s will see the visual lag behind reality
  for the duration of the swing.  Re-tune at CI-3 if rehearsal
  shows the lag feels disconnected.
- ~~**Triangle-colour mix ratios**~~ — RESOLVED 2026-05-10 at
  **70% self / 25% 1-away neighbours / 5% 2-away tiles**, biased
  by each neighbour's own colour count.  Self colour clearly
  dominant; 1-away visible enough to bleed at cluster
  boundaries; 2-away just a hint.  Re-validate at CI-3 against
  projection scale; tighter (more self) if the mosaic feels
  muddy, looser (more neighbour) if crystals read as flat solid
  blocks.
- ~~**Bridge geometry — exact mesh shape**~~ — RESOLVED
  2026-05-10: each filled crystal extends a lateral lobe to the
  **centre of the empty cell**, where the two lobes meet at the
  **same height** as the source crystals (level top, no arch,
  no sag).  Like the crystals themselves, bridges are not solid
  — they consist of struts / plates with gaps the camera can
  see through.  Simple, clean, sluggish-friendly aesthetic;
  easy to compute.
- ~~**Growth animation cost**~~ — RESOLVED 2026-05-10: **no
  preemptive cap**.  Sluggish-by-design tempo (~5 s per growth,
  ~30 s per decay window, 10 Hz tick rate) means sustained
  100+ concurrent animations is unlikely.  Measure actual cost
  during the first prototype and rehearsal; if a frame budget
  breach shows up, decide on the right cap (hard cap, LOD
  simplification, or per-frame skipping) then.  Don't optimise
  ahead of the measurement.
- ~~**Camera framing**~~ — RESOLVED 2026-05-10:
  **follow-action with periodic auto-zoom-out**.  Camera
  centres on the heat field of recent activity; zoom inversely
  follows the spread of activity (concentrated → zoom in,
  dispersed → zoom out).  Every ~30 s (sluggish-by-design
  cadence), camera auto-pulls back to a fit-all view for ~5 s
  so the audience sees what they're missing, then resumes
  follow-action.  Presenter hotkey override always available
  to lock or steer manually.
- ~~**Idle behavior**~~ — RESOLVED 2026-05-10: **slow drift
  over the painted region**.  When activity stops, camera
  treats the whole painted area as low-heat and pans slowly
  across it.  Keeps the spectacle alive during quiet stretches;
  matches the sluggish-by-design tempo.  Drift speed is a
  renderer-side constant tunable at first prototype; presenter
  override always available to lock the camera if a quiet
  moment wants to read as deliberate stillness.
- ~~**Recently-changed pulse**~~ — RESOLVED 2026-05-10:
  **audience changes only**.  Pulses fire on placement / erase
  events that came from audience taps and swipes; decay-removals
  do **not** pulse.  Makes audience-driven activity visually
  distinct from the autonomous decay rule and prevents the
  screen from constantly twinkling as old cells die.

## Performance — crystal mesh rebuild cost (stress test 2026-05-21)

The projector rebuilds the crystal mesh + VBO via `build_crystal_vbo`
(→ `audience_crystal::crystal_segments_aged`).  **It does NOT rebuild
every frame** — PF1 caching is already in place: `main`'s render loop
rebuilds the crystal + ground VBOs only when `world.version` changes
(`projector.loft` ~787, the `last_built_version` gate), and `version`
bumps only inside `apply_frame` on a paint / erase / repaint.  Because
the message drain runs before the rebuild check, multiple paints landing
in one frame coalesce into a single rebuild.  So **steady-state render is
just the draw calls + the camera matrix**; the timings below are the cost
**per paint event** (the rebuild that PF2 makes cheaper), not a per-frame
cost.  [`tools/audience-demo/crystal_stress.loft`](../../../../tools/audience-demo/crystal_stress.loft)
ramps a filled-hex cluster 1 → 100 across four fill PATTERNS (block /
line / sparse / ring) and times that rebuild path on `--native`.  Run:

```bash
cargo run --release --bin loft -- --native --lib lib tools/audience-demo/crystal_stress.loft
# leak check:  LOFT_NATIVE_LEAK_CHECK=1 ... (validated clean over the full ramp)
```

### Original algorithm (furthest-on-axis mains) — fork explosion

The first measurement (mains reaching to the furthest filled cell on
each axis, `find_furthest_on_axis` + `OVER_REACH`) showed a catastrophic
shape-dependent blow-up because `n_forks = ray_len / spacing` grows with
main *length*, and spread clusters produce very long mains:

| hexes | block | line | sparse | ring |
|---|---|---|---|---|
| 50 | 13.8 ms (1364) | 4.1 ms (592) | 19.9 ms (1876) | **85 ms** (4433) |
| 80 | 23.8 ms (2062) | 8.0 ms (938) | **51 ms** (3312) | **318 ms** (10130) |
| 100 | **38 ms** (2830) | 11.5 ms (1170) | **68 ms** (4132) | budget blown |

ring/80 = 10130 segments (vs block/80 = 2062); at ring ≥ 90 the segment
vectors thrashed the allocator hard enough to OOM a sustained run.

### Revised algorithm (3-hex mains, old→new) — fork explosion gone

The algorithm was changed (2026-05-21, `crystal_segments_aged`): mains
are now capped at `MAX_MAIN_HEXES` steps and a cell draws one short main
back to the **nearest older filled cell on each of its six axes**
(crystals grow old→new; cells separated by a gap > 1 form independent
crystals that merge when a later paint narrows the gap).  Capping main
length **bounds the fork count by construction**, so the shape-dependent
blow-up is gone — every pattern now scales the same bounded way:

| hexes | block | line | sparse | ring |
|---|---|---|---|---|
| 50 | 11.1 ms (1119) | 4.3 ms (471) | 10.6 ms (1113) | 4.7 ms (522) |
| 80 | 24.3 ms (1875) | 9.5 ms (741) | 21.6 ms (1761) | **9.9 ms** (810) |
| 100 | 35.6 ms (2379) | 14.0 ms (921) | 30.5 ms (2190) | **14.2 ms** (990) |

**ring/80: 318 ms → 9.9 ms (32×), 10130 → 810 segments.**  Segment count
is now ~linear in cell count for every shape, and the OOM is gone.

**What's left:** the per-build time is still **O(N²)** (`us/hex²` flat at
~3000–4500 for the dense cases) — the cost is now the per-cell work
inside `crystal_segments_aged`, each O(N): the nearest-older-on-axis scan
(`for cs_j in 0..cs_i`), `nbr_colors_at` (×2 / cell), and `cell_h` →
`snap_nbr_count` (a full neighbour scan, called per endpoint).  block/100
is still ~36 ms — a large dense crystal exceeds the 60 fps budget per
*rebuild*.

**Fixes (priority order — the algorithm change reshaped these):**

- **PF1 — cache the crystal mesh + VBO; rebuild only on snapshot
  change.  ✅ ALREADY IMPLEMENTED** (verified 2026-05-21).  `main`'s
  render loop bakes the crystal + ground VBOs only when `world.version`
  changes (the `last_built_version` gate, `projector.loft` ~787);
  `version` bumps only in `apply_frame` on paint/erase/repaint, and
  multiple paints in one frame coalesce to one rebuild (the message
  drain precedes the gate).  Steady-state render is one `gl_draw` per
  VBO.  So mesh-gen already runs at audience-tap rate, not 60 Hz — the
  timings above are the per-paint rebuild cost, which is what PF2
  addresses.  (The only genuinely per-frame VBO work is the fade layer,
  which MUST rebuild each frame because its vertex alpha animates.)
- **PF2 — spatial index** (`hash<cell_key → index>` built once per
  rebuild) shared by the nearest-older-on-axis lookup, `nbr_colors_at`
  and `snap_nbr_count`/`cell_h`, turning the remaining O(N²) → O(N).
  ✅ **DONE** (commit `8219a416`) — the per-axis lookup is O(1) against a
  coord→index hash.
- **PF3 — ~~bound per-main fork count~~ DONE by the algorithm change**:
  `MAX_MAIN_HEXES` caps main length, so forks are bounded structurally.
- **PF4 — ~~preallocate the `CrystalMesh` parallel vectors~~ DONE at the
  language level by P5 + P6** (2026-05-21).  The "crash by scale" OOM was
  the allocator, not the mesh code: **P5** (amortised ×2 vector growth in
  `vector_append`) stops the per-`+= [x]` realloc churn, and **P6** (lazy
  free-block coalescing in `Store::claim`) lets repeated rebuilds reuse
  freed space instead of growing.  Validated 2026-05-21: a block-500 mesh
  (12 738 segments) that previously fragmented to ~250 MB / 101 815 free
  blocks per build now uses **~1.5–2.8 MB / 8–9 free blocks**, and 300
  consecutive rebuilds hold a **flat ~1.6–4.4 MB** steady state (was 7.6 GB
  → OOM at block-800).  No CrystalMesh code change was needed.

### Further memory reduction (post-P5/P6) — opportunities

P5/P6 removed the catastrophic fragmentation; a block-500 mesh is now
~2 MB.  To push it lower (useful for huge crystals / many cached
meshes), in priority order:

- **M1 — narrow the `CrystalMesh` element types (≈60 %, low effort, zero
  downside).**  Per segment today: `kinds`/`colors`/`cell_ix` are
  `integer` (i64, 8 B each) and the six coordinate arrays
  `x0s…z1s` are `float` (f64, 48 B) → **72 B/seg**.  loft stores narrow
  vector elements at their true width (measured: `vector<u8>` 5.5× /
  `vector<u16>` 2.4× smaller than `vector<integer>`; f32 halves f64).
  - coords `float`→`single` (f64→f32): the dominant 48 B → 24 B.  **The
    GPU VBO is consumed as `*const f32`** (`loft_gl_upload_mesh`) and
    `build_crystal_vbo` already casts every vertex `as single` — so this
    halves the coord memory AND removes the f64→f32 conversion at
    VBO-build time, with no precision change vs what is rendered.
  - `kinds`,`colors` `integer`→`u8` (small enum / palette 0–9).
  - `cell_ix` `integer`→`u16` (or `i32` if cells can exceed 65 535).
  - Net **72 → 28 B/seg ≈ 60 %** → block-500 ~2 MB → **~0.8 MB**.  Effort
    S: change the field types in `crystal.loft`, add `as single`/`as u8`/
    `as u16` at the `+= [x]` sites in `crystal_segments_aged`, read the
    now-`single` fields directly in `build_crystal_vbo`.
- **M2 — reclaim the amortised-growth slack (≈30 % more; needs a small
  language follow-up).**  P5's ×2 growth leaves builds at ~57–73 %
  utilisation.  The deep-copy path already shrinks-to-fit (length-based
  copy), so the slack only lives in the freshly-built arrays — build them
  exactly-sized via a **`reserve(v, n)`** builtin (the deferred P5
  follow-up wrapping the existing `OpPreAllocVector`) or a
  **`shrink_to_fit(v)`** after building, using a counting pass or the
  estimate `segments ≈ k·cells`.  Net ~0.8 MB → **~0.5 MB** (util → ~90 %).
  Effort S–M (exposing `reserve`/`shrink_to_fit` is a language addition
  that benefits every vector consumer, not just the crystal — see
  PERFORMANCE.md P5 "reserve builtin" follow-up).
- **M3 — indexed mesh / shared vertices (diminishing returns, higher
  effort).**  Segments are line endpoints; if ridge junctions share
  endpoints, a vertex-array + index-array layout dedupes them.  Uncertain
  payoff (depends on sharing) and it complicates per-segment `cell_ix` /
  GPU-growth keying.  Lowest priority.

The parallel struct-of-arrays layout is already optimal (no per-element
padding) and the per-rebuild spatial-index hash is transient (freed each
build) — so element width (M1) and slack (M2) are the only real levers.

### Efficiency for the world-building generalization (TOP priority)

This routine is the prototype for a CLASS of grid→geometry algorithms —
wall placement, edge/corner rounding, ridge/feature detection — that the
world builder needs.  Those run on bigger grids (hundreds–thousands of
cells) and add MORE per-cell work, so the pipeline must be efficient as a
reusable primitive BEFORE the complex patterns land.  Measured state
(interpreter, 2026-05-21): the build is **O(N)** — flat ~6–9 µs/segment
across all sizes/patterns, segments ∝ cells, `µs/hex²` coefficient falls
monotonically (PF2 removed the O(N²)).  Memory is bounded (P5/P6).  What
remains, in priority order:

- **I1 — incremental / dirty-region update (the decisive lever).**  A
  full rebuild is O(N): ~90 ms at 500 cells, ~180 ms at 1000 (interp),
  fine for the demo (≤100 cells, PF1 rebuilds only on paint) but wasteful
  at world scale where a single edit (place one wall, flip one cell)
  affects only a LOCAL neighbourhood — the changed cell plus cells within
  the neighbour-query radius (≤2 rings here).  Make edits **O(affected
  cells) ≈ O(1)** instead of O(N):
  1. keep the spatial index (`cidx`) PERSISTENT across edits (today it is
     rebuilt O(N) every call, lines 660-663);
  2. on a cell change, re-emit only the segments OWNED by the dirty cell
     and its ≤2-ring neighbours — the `cell_ix` field already attributes
     each segment to its owning cell, so the data structure already
     supports find-and-replace by cell;
  3. maintain a dirty-set; rebuild only those cells' segment ranges.
  This is the single biggest win at world scale and the structure every
  later pattern reuses.  Effort: M (the cell_ix attribution + persistent
  index are already in place; needs a dirty-set + per-cell segment-range
  bookkeeping).
- **I2 — cut the per-cell constant factor.**  Each cell does several
  independent hash gathers (`nbr_colors_idx` ×2, `cell_h_at`, 6 axes ×
  `MAX_MAIN_HEXES` probes) → ~15–20 hash lookups/cell.  A single combined
  **1-ring/2-ring neighbour gather per cell** (read each neighbour once,
  reuse for colours / height / nearest-older) removes the redundant
  probes.  Also profile on **`--native`** (the deployment target; the
  ~7 µs/seg above is the interpreter).  Effort: S–M.
- **I3 — extract a reusable grid→mesh primitive.**  Generalise the
  pipeline into: (1) persistent coord→cell spatial index, (2) per-cell
  neighbour gather, (3) pluggable cell CLASSIFICATION (line/blob/edge/
  corner — where wall-placement & edge-rounding rules slot in), (4) per-
  cell geometry emission keyed by `cell_ix`, (5) dirty-set incremental
  rebuild (I1).  Wall placement and edge rounding then become new
  classification+emission rules over the same 1–2 + 4–5.  Effort: M
  (mostly refactor once I1/I2 land).
- **M1/M2** (above) — element-width + slack; orthogonal, compound with I1.

Recommended order: **I1 → I2 → M1 → I3**, with M1 takeable any time as a
quick standalone win.

### Scaling to huge crystals — can it, and where does it crash?

The revised algorithm makes the **segment count ~linear** in cell count
(no more fork explosion), so a single large crystal builds fine:
block-500 = 12738 segments, block-1000 = 25896 segments, each builds
once with no crash.  But scaling the *live* demo to hundreds of cells
hits three walls — and one is a real crash, found 2026-05-21:

1. **O(N²) rebuild TIME** (slowdown, not crash).  block: 100 = 36 ms,
   200 = 203 ms, 300 = 385 ms, ~1 s at 500.  Each *paint* on a
   several-hundred-cell crystal is a multi-hundred-ms hitch (PF1 already
   limits rebuilds to paints, but a busy tap stream still stutters).
   Fixed by PF2 (O(N) rebuild).
2. **Memory blowup under sustained rebuilding → OOM crash.**  The
   `mesh.field += [x]` appends do **not** pre-reserve capacity, so a
   large mesh churns the allocator arena hard: one 100 000-element
   vector built by `+=` peaks at ~171 MB RSS (vs ~1.6 MB of data, ~100×
   overhead).  Rebuilding a block-500 mesh 30× peaks at **7.6 GB RSS**;
   a growing ramp to block-800 hit **11 GB and was OOM-killed** (single
   builds at those sizes are fine — it is the *sustained* re-allocation
   across many rebuilds that accumulates; PF1 limits rebuilds to paints,
   so a busy edit stream on a large crystal is the trigger).  No
   store-level leak (the stores are freed; it is glibc arena retention of
   the append churn).  This is the concrete "crash by scale".  Fixed by
   **PF4, upgraded from nice-to-have to crash-prevention** (preallocate /
   reuse the mesh vectors instead of growing them from empty each build).
   A loft-level fix — `vector` capacity reservation on `+=` / a
   `reserve(n)` builtin — would also help every consumer.
3. **`CrystalState` unbounded growth** (the CPU `update_state` aging
   path — now AVOIDED by the demo, see § Growth animation).  If ever
   re-enabled, `update_state` appends a birth record per (centre,
   direction) main and never prunes, and `lookup_main_birth` is a linear
   scan, so the state vector grows ~6 × cells and aging cost becomes
   O(N²).  The demo no longer uses this path (growth is GPU-side), so it
   is moot for the demo; bound it (cap records to live cells, index by
   cell key) before any consumer turns CPU aging back on at scale.

**Bottom line:** yes, the demo can scale to large crystals — the segment
explosion is solved and PF1 (rebuild only on paint) is already in place,
so steady-state is cheap.  The remaining risks are at *edit time* on a
large crystal: each paint is an O(N²) hitch (PF2) and, without PF4
(preallocate), a busy edit stream churns the allocator into an OOM.  With
PF2 + PF4 added, edits stay cheap and it scales to thousands of cells.

**Leak validation (2026-05-21):** the per-frame path is leak-free on
both backends, **on both the original and the revised algorithm** —
`LOFT_NATIVE_LEAK_CHECK` clean over the full ramp (all 4 patterns × 12
sizes), plus 3000 block-50 builds, ring/90 × 100 builds, 2000 snapshot
builds, and 150 interp builds, all with no leaked stores.  (Relies on the
@P297/@P298 struct-returning-call free fixes landed the same session —
without them the per-frame `cm = crystal_segments_aged(…)` temporary
would have leaked.)

## Growth animation — GPU-side (shipped 2026-05-21)

The crystal **grows in gradually** when a cell is painted, instead of
popping in fully — but **without** any per-frame CPU rebuild, so PF1
caching stays intact.  The growth runs entirely in the vertex shader:

- `audience_crystal::CrystalMesh` carries `cell_ix` (the owning cell per
  segment), so the projector can attribute each beam to the cell whose
  paint produced it.
- Each `Cell` records `birth_frame` (the render frame it was painted, in
  the same clock as the loop's `frames`).  `snapshot_cells` returns a
  `Snapshot{snap, births}` with a placement-ordered `births` array.
- `build_crystal_vbo` emits **stride-14** vertices: pos + normal + rgba +
  the owning cell's centre (bloom anchor, precomputed once) + birth.  The
  graphics cdylib's `gl_upload_vertices` wires those as attribute
  locations 3 (`vec3` anchor) and 4 (`float` birth) when `stride ≥ 14`.
- `CRYSTAL_VERT`: `age = clamp((uNow − aBirth) / uGrowth, 0, 1)` with an
  ease-out, then `pos = aCenter + age · (aPos − aCenter)`.  Each cell's
  crystal blooms out from its centre over `GROWTH_FRAMES` (≈1.5 s).  Only
  the `uNow` uniform advances per frame; the mesh is rebuilt only on
  paint (PF1).  The fade layer switches back to `cell_shader` (stride 10).

This sidesteps both the per-paint O(N²) rebuild AND the CPU-aging
`CrystalState` growth (§ Scaling point 3) — the geometry the shader
animates is the cached static mesh.  Tunables: `GROWTH_FRAMES` and the
ease curve / anchor in `CRYSTAL_VERT` (`projector.loft`).

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
