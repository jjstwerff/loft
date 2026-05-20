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
