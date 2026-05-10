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

The phone is **2D pure orthogonal fixed hex grid** — no
perspective, no 3D crystal mesh, no rotation.  Each hex is drawn
as a flat top-down hex with a fixed orientation.  Filled hexes
take their palette colour; empty hexes are filled with black.
Each hex has a visible outline in a slightly lighter shade
(e.g. dark grey) so the grid lattice stays legible — even on an
entirely empty world the audience member sees a structured
canvas, not a void.

The phone deliberately does **not** render the 3D crystal
animation, growth tilts, or ridge-and-crevice tops.  Those live
on the projector and the desktop client; the phone is the
smallest possible flat paint surface so it stays responsive on
modest hardware and reads instantly.

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

A horizontal row of **9 color hex tiles**, plus reserved index 0
for "empty / open space" (which is not a palette tile — it is
the absence of a colour, i.e. an empty hex on the world).

| Index | Color |
|---|---|
| 0 | Reserved — empty / open space.  Never a palette tile |
| 1 | Red |
| 2 | Green |
| 3 | Blue |
| 4 | Cyan (Green + Blue) |
| 5 | Magenta (Red + Blue) |
| 6 | Yellow (Red + Green) |
| 7 | White |
| 8 | Grey |
| 9 | Brown |

Indices 1-3 are the RGB primaries; 4-6 are the additive mixes;
7-9 round out the palette with white, grey, and brown for
neutrals.

Tap a palette tile (1-9) to make it the **active color**.  The
active color shows a selected indicator (border / glow).  At
most one color is active at a time.  The Clear-color box (zone 3)
sets the active color back to "none" — distinct from index 0,
which is the *world's* state for an empty cell.

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
{ "type": "seed",  "x": <q>, "y": <r>, "color": <1..9> }
{ "type": "clear", "x": <q>, "y": <r> }
```

`color` is the palette index (1-9 from the table above).  The
server treats a `clear` event as setting the cell to colour
**0** (empty).  `<q>`, `<r>` are world hex coordinates (axial),
not local view coordinates.  The client translates
view-coordinates → world coordinates using its current pan
offset.

## WebSocket events (server → client)

Two frame types share the same connection:

**JSON text frames** (small / event-shaped):

```json
{ "type": "active_player_signal", "x": q, "y": r }
```

**Binary frames** (bulk world data, packed 4 bytes per cell):

| Frame | Use |
|---|---|
| `world_snapshot` | Sent once on connect.  Full dump of every existing chunk: per-chunk header (`cx`, `cz`, cell-count) followed by the chunk's cell array packed at 4 bytes/cell |
| `world_delta` | Sent every tick (10 Hz).  Per-chunk list of changed cells (`hx`, `hz`, then the 4-byte payload).  Cells set to colour 0 represent decay-removal events |

Each binary blob carries a **session id** in its header.  This
matters most when a new client first connects: the server splits
the initial `world_snapshot` into one blob per chunk, all
sharing the same session id.  The phone client buffers them and
renders the world as a single coherent fade-in instead of each
chunk popping in as it arrives.  Steady-state deltas usually fit
in one blob and don't need grouping; the session id is still in
the header so the same render path handles both.

The phone client decodes the binary frames in JS using
`DataView` / `Uint8Array` — no JSON parse on the hot path.

The client redraws affected hexes from each delta — including
changes made by **other players**, not just its own taps, so
every audience member sees the full collaborative canvas as it
evolves.

`active_player_signal` arrives as a JSON frame when another
player changes colour and triggers the **Jump to active** box's
flash.

## Desktop variant — projector view + input

Desktop / laptop is a **first-class second target** alongside
phone (Q5 resolution 2026-05-10), but it is **not** a re-skin of
the phone client.  The desktop UI shows the **same 3D crystal
animation the projector renders**, with the **flat hex grid
visible on the base plane underneath** the crystals.  Mouse
clicks land on **base-plane hexes** (not on the 3D crystals
themselves); the same `seed` / `clear` / `color_select` events
fire as from the phone client.

In effect the desktop user gets a **personal projector view they
can also paint into** — what they paint appears under their
mouse in 3D, and they see the same world the auditorium is
watching, from the same kind of 3D camera.

### Visual model

| Layer | Rendering |
|---|---|
| **Base plane** | Flat hex lattice — outlined black tiles, always visible (the same lattice the phone client renders flat).  Clicks register against this layer |
| **Crystal layer** | Same 3D crystal mesh + frost aesthetic + ridge-and-crevice tops + per-triangle hard-edge colour mosaic the projector uses.  Renders on top of the base plane |
| **Hover indicator** | When the mouse hovers over a base-plane hex, a faint highlight on that hex shows where a click would land.  Lifts the hex slightly visually so the user can target it through any crystal currently growing on it |

Camera is **user-controlled** on desktop (not auto-camera):
**mouse-drag pans the world**, scroll-wheel zooms, **WASD also
pans** (continuous while held — convenient for one-handed
navigation while the other hand is on the mouse for painting).
No orbit / no free-camera 3D rotation — the view stays in the
same top-down framing as the projector.  No heat-field tracking
— the user steers their own view.

Implementation: standard WASM target via the existing
`loft --html` pipeline (same as the projector binary, just
loaded into a browser page instead of a native window).

### Code reuse with the projector

The desktop client and the projector view share the **same 3D
renderer** (mesh generator + edge classifier + frost geometry
+ camera transform).  Differences:

- Desktop adds a base-plane render pass + a hover indicator +
  click-to-hex picking.
- Desktop uses a user-controlled camera (mouse-drag + WASD pan,
  scroll zoom); projector uses an auto-camera.
- Desktop sends `seed` / `clear` / `color_select` events to the
  server; projector is subscribe-only.

Implementation path: shared loft renderer library compiled to
WASM via the existing `loft --html` pipeline.  Both the desktop
client and the projector are WASM payloads — the desktop is
embedded in an HTML page served alongside the phone client;
the projector binary loads it in a native window
(or natively-linked equivalent).  CI-3 confirms the split when
the renderer is sketched.

### Input model

| Phone | Desktop |
|---|---|
| `pointerdown` / `pointermove` / `pointerup` (touch) | Same `pointer*` events (mouse) on the canvas |
| Tap = single colour placement | Click = single colour placement (hits the hovered base-plane hex) |
| Swipe = paint a line | Click-and-drag = paint a line over base-plane hexes the cursor crosses |
| Swipe starting on own-colour = erase line of own colour | Drag starting on own-colour = erase line of own colour |
| Pan: hold finger in outer ring | Pan: mouse-drag on empty space, OR WASD / arrow keys (continuous while held) |
| (No camera control) | Zoom: scroll wheel.  No orbit / no free-camera 3D — view stays top-down |
| Color pick: tap palette tile | Color pick: click palette tile OR press number key 1-9 |

### Controls overlay

The colour palette is **set into a single panel** that also
contains the action button (jump-to-active).  Picking a colour
and triggering jump-to-active happen in the same screen region
so the eye does not have to travel between them.

| Element | Position on desktop |
|---|---|
| **Palette + action panel** (set-in panel — recessed visual style) | One side of the screen.  Contains: 9 colour tiles + the clear-color box + the jump-to-active button.  Active-player flash plays on the jump-to-active button so it stays in the same panel as the colour pick |
| Camera reset button | Corner of the canvas (resets pan + zoom to the world centre) |

Keyboard shortcuts:

| Key | Effect |
|---|---|
| 1-9 | Pick palette colour |
| 0 / Esc | Clear active colour |
| W / A / S / D / arrows | Pan world (continuous while held) |
| Mouse-drag (on empty space) | Pan world |
| Scroll wheel | Zoom |
| J | Jump to most-recently-active player |
| R | Reset camera |

### Detection + switching

Use a small `userAgent` + viewport-width heuristic on page load:
small viewport + touch-only → phone layout, large viewport +
mouse → desktop layout.  Do not allow live switching after load
(simpler).  Provide a `?layout=phone` / `?layout=desktop` query
param override for rehearsal + testing.

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

- ~~**Color encoding on the wire**~~ — RESOLVED.  Palette
  index 1-9 over the wire; index 0 is reserved for "empty hex"
  in the world state and is never sent in a `seed` event.
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
