<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 0 — Audience browser page

## Status

Open.  Owner of the smartphone-side UX.  Effort: **S**.

## Goal

A single static HTML/JS page each audience member loads in their
phone browser.  It connects via WebSocket to the demo server,
renders a small movable window into the hex world, and lets the
user paint by tapping or swiping.  No build pipeline — one HTML
file with inline CSS/JS so anyone can read the source from the
talk.

## UX layout

The screen is partitioned vertically into three zones.  Total
target height fits an unzoomed phone portrait viewport.

### Zone 1 — World view (top, dominant)

A **9 × 7 roughly-square hex pattern** showing the user's local
window into the much larger shared world.

Each hex has a visible outline.  Empty hexes are filled with
**black**; painted hexes are filled with their palette colour.
The outline is a slightly lighter shade (e.g. dark grey) so the
grid lattice is always legible — even on an entirely empty world,
the audience sees a structured canvas, not a void.

The two outer rings of the hex pattern double as a movement
control:

| Ring | Behaviour when finger held |
|---|---|
| Outermost ring (1 hex thick) | Pans the world **fast** in that direction relative to centre |
| Next-inner ring (1 hex thick) | Pans the world **slightly slower** in that direction |
| Inner 5 × 3 area | Pure paint surface — taps + swipes affect cells |

Movement is continuous while the finger is held; releasing stops
it.  The world is shared across all clients — moving the window
does not consume server bandwidth, only the audience's view
shifts.

### Zone 2 — Color palette (middle)

A horizontal row of **9 color hex tiles**:

- 3 primaries (R / Y / B or R / G / B — locked at CI-1)
- 3 mixes (orange / green / purple — derived from the primaries)
- 1 white
- 1 grey
- 1 brown

Tap a color to make it the **active color**.  The active color
shows a selected indicator (border / glow).  At most one color is
active at a time.

### Zone 3 — Control row (bottom)

Two boxes side by side:

| Box | Behaviour |
|---|---|
| **Clear color** | Deselects the active color.  Subsequent taps in the world view do nothing until a color is reselected.  Useful if a user has stopped contributing |
| **Jump to active** | Moves the world view centre to a hex where another player is currently active.  **Flashes** when other players change colors, signalling there is somewhere to jump to |

## Interactions in the world view

| Gesture | Active color set | Effect |
|---|---|---|
| Tap empty hex | yes | Place active color at that hex (seed event to server) |
| Tap own-color hex | yes | Remove your color from that hex (clear event).  Does **not** affect hexes painted in other colors |
| Tap other-color hex | yes | No effect (cannot overpaint someone else's color) |
| Tap any hex | no (cleared) | No effect |
| Swipe across hexes | yes | Place active color along the line of hexes the finger crosses |
| Swipe starting on own-color hex | yes | **Erase** the line of own color from start to finger release.  Treat as "drag-erase" while continuing to drag |
| Swipe reaches outer ring | (any) | World pans in that direction; swipe continues into newly-revealed hexes — effectively unlimited line length |

The outer-ring movement composes with swipe gestures: a player can
draw arbitrarily long lines by swiping toward an edge and letting
the world scroll under their finger.

## WebSocket events (client → server)

Locked at CI-0 in the parent README.  First cut:

```json
{ "type": "seed",  "x": <q>, "y": <r>, "color": "<palette_id>" }
{ "type": "clear", "x": <q>, "y": <r> }
```

Where `<q>`, `<r>` are world hex coordinates (axial), not local
view coordinates.  The client translates view-coordinates → world
coordinates using its current pan offset.

## WebSocket events (server → client)

```json
{ "type": "world_delta", "cells": [ {"x": q, "y": r, "color": "<id>"} ] }
{ "type": "active_player_signal", "x": q, "y": r }
```

`world_delta` arrives every server tick; client redraws affected
hexes.  `active_player_signal` arrives when another player changes
color and triggers the **Jump to active** box's flash.

## Sub-tasks

| # | Task | Effort |
|---|---|---|
| 0.1 | Static HTML scaffold + responsive layout (zones 1/2/3 fitting phone portrait) | XS |
| 0.2 | Hex grid renderer (canvas or SVG; pick whichever is simpler in plain JS) | S |
| 0.3 | Color palette + active-color state | XS |
| 0.4 | Tap handling — empty / own / other distinction | XS |
| 0.5 | Swipe handling — line draw + drag-erase variant | S |
| 0.6 | Outer-ring movement zones — continuous pan while held | S |
| 0.7 | WebSocket client (open / send / receive / reconnect on drop) | XS |
| 0.8 | Jump-to-active button + flash animation | XS |
| 0.9 | Onboarding self-explanation (no instructions text — UI must be obvious from first glance) | S |

## Open design questions

- **Color encoding on the wire** — palette index (0-8) or hex
  string (`"#ff0000"`)?  Index is smaller; string is
  self-describing for talk-readability.  Recommend palette index.
- **Coordinate system** — axial (q, r) or offset (col, row)?
  Axial is simpler for distance / direction maths in the
  generation script (phase 2).  Recommend axial.
- **Movement cadence** — discrete (one cell per N ms while held)
  or continuous (subpixel pan)?  Discrete is simpler and matches
  the hex grid; continuous looks smoother.  CI-1 decides after
  thumb-test.
- **Touch vs. pointer events** — `pointerdown` / `pointermove` /
  `pointerup` cover both touch and laptop trackpad.  Use those
  unless a specific phone browser misbehaves.

## Deliverable

A single `index.html` (with inlined CSS + JS) that, when served
from any HTTP origin and pointed at a running server, renders
the full UX described above.  Tested on at least 2 different
phone browsers (iOS Safari + Android Chrome).

## See also

- [`README.md`](README.md) — parent plan
- [`01-server-state.md`](01-server-state.md) — what this client
  talks to
- [`03-projector-view.md`](03-projector-view.md) — the projected
  world view that audiences are collectively painting
