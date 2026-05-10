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

## Visual model — 3D height field

The projector renders the world as a **3D height field over the
hex grid**, not a flat plane.  This is the spectacle: the room
watches crystals literally *grow upward* as audience input
spreads.

Per-hex height rules:

| Configuration | Height |
|---|---|
| Filled hex with 0 filled neighbors | **Lowest** — a single crystal point |
| Filled hex with all 6 neighbors filled | **Highest** — a peak / plateau |
| Filled hex with N neighbors filled (0 < N < 6) | Monotonically rising with N |
| Empty hex between two filled hexes (gap of 1) | Small **bridge** bump — visible elevation, lower than filled endpoints |
| Empty hex far from any filled | Flat (zero height) |
| Line of filled hexes | Continuous **ridge** of crystals |

Height is derived from the filled-neighbor pattern in the
neighborhood — every hex (filled or empty) gets a height value
suitable for smooth surface rendering.  Empty hexes between
filled ones inherit a fraction of the surrounding height, which
produces the bridge effect naturally.

Per-cell rendering preserves the colour rules from the audience
client:

- **Empty hex**: black fill, slightly lighter outline so the
  hex lattice is always visible.  Empty hexes near filled ones
  pick up some height from the bridge effect but stay black.
- **Painted hex**: filled with the cell's palette colour, same
  outline.  Older cells may desaturate / darken depending on
  the generation variant.
- **Recently-changed hex**: brief highlight pulse (e.g. white
  edge flash, decays over ~500 ms) so the audience sees where
  growth is happening even when the camera is wide.

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
| 3.4 | Per-hex height computation from filled-neighbor pattern (filled + empty hexes both get height; empty inherits fractional height for the bridge effect) | S |
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
- **Height-field formulation — exact math** — simplest: per-hex
  intrinsic height = `f(filled_neighbor_count)` for filled
  hexes, then a smooth surface (Gaussian / cubic-spline-like)
  interpolation over centres so empties between filled hexes
  pick up the bridge effect.  Alternative: explicit "bridge
  weight" rule.  CI-3 picks after seeing both on a fixture.
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
