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

## Visual model

Same hex grid as the audience client but at much greater extent.
The visible window is a function of the auto-camera's centre +
zoom; cells outside the window are not rendered.

Per-cell rendering matches the client (consistency for the
audience):

- **Empty hex**: filled with black, outlined in a slightly
  lighter colour (e.g. dark grey) so the grid lattice is always
  visible even on a blank canvas.
- **Painted hex**: filled with the cell's palette colour, same
  outline.  Older cells may be slightly desaturated or darkened
  if the generation variant uses age-based fading.
- **Recently-changed hex**: brief highlight pulse (white edge
  flash, decays over ~500 ms) so the audience can see *where*
  growth is happening even when the camera is wide.

The constant outline + black background pattern means the room
always sees the world structure (a hex lattice) even before any
seeds are placed — it's not just a void on the wall.

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
| 3.2 | Hex grid renderer at projection resolution (target 1920x1080+) | M |
| 3.3 | Camera transform — world coordinates → screen pixels with pan + zoom | S |
| 3.4 | Heat-field tracker — accumulate events, decay over time | S |
| 3.5 | Camera target derivation — centroid + spread → target + zoom | S |
| 3.6 | Smooth camera motion (lerp + smoothing constant tuning) | S |
| 3.7 | Recently-changed pulse highlight | XS |
| 3.8 | Presenter hotkey handlers | XS |
| 3.9 | Headline overlay (round number, palette legend, optional player count) | S |
| 3.10 | Pre-recorded backup capture — record a 5-min run as fallback video | XS |

## Open design questions

- **Build on `lib/moros_editor` or fresh?** — moros editor
  already renders 3D hex worlds + has camera infrastructure;
  reuse may be faster than rebuild.  But the demo needs 2D top-
  down + auto-camera, which is orthogonal to the editor's
  manual-orbit camera.  CI-3 picks: fork the editor view, or
  write a slimmer dedicated viewer.
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
- **2D top-down or 3D perspective?** — 2D is simpler, easier to
  read on a projector at distance.  3D is showier but expensive.
  Default 2D unless rehearsal shows otherwise.

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
