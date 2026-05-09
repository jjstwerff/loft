<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Audience-driven generative art — local meetup demo

## Status

Plan only.  Scoped 2026-05-09.

Target: local meetup of game creators + art enthusiasts.  Demo
concept — audience members influence a **plant / crystal growth**
simulation on a hex map in two ways: **clicking a hex seeds growth**
at that position, and **choosing a color biases the dominant color
in that direction of growth**.  A loft generative script runs the
growth simulation; a native projector view shows the evolving world
in real time.  The spectacle is the **emergent collaborative
patterns**, not any single person's contribution.

The choice between plant-growth aesthetic (organic, branching,
L-system) and crystal-growth aesthetic (faceted, geometric, DLA-
like) is open — both work on the hex grid with the same input
mechanics.  Pick during phase 2 prototyping.

## Goal

Deliver a memorable, audience-participatory creative-coding demo
that showcases what loft uniquely enables for creators:

- **Collaborative shared state** via a single URL — everyone
  contributes, everyone sees the result
- **Generative scripts written in clear loft code** — short,
  readable, live-tweakable
- **Native + browser delivery** from one language and one toolchain

Audience walks away with concrete examples of "I could write
this."

## Demo shape

| Piece | What it does |
|---|---|
| **Projector** | Native viewer in fullscreen, subscribes to server, renders evolving hex world |
| **Audience** | Browser page on phone or laptop — pick a color from a palette, tap hexes to seed plant/crystal growth at that location with that color |
| **Server** | WebSocket; holds seed list + color-direction biases; runs growth simulation each tick; broadcasts world updates |
| **Generation script** | Loft program — plant/crystal growth from seeds, with neighbor-color blending biased by the colors chosen in that direction.  THE STAR of the demo |
| **Presenter** | Tweaks the generation script live between rounds (growth speed, branching factor, color blend rules); rounds reset the world |

## Generation algorithm — plant / crystal growth

The seed list is a set of `{ position, color, planted_at_tick }`
records.  Each simulation tick:

1. For every empty hex adjacent to a "live" cell, decide whether it
   gets filled this tick (probability based on neighbor count +
   growth-rate parameter).
2. If filled, pick its color from the weighted majority of live
   neighbors, biased toward the dominant color in the **direction**
   of the new cell relative to the seed clusters.  This is where
   "audience choosing a color biases the dominant color in that
   direction" enters: the color biases form a vector field that
   pulls growth-color-choices.
3. Optionally: cells "die" / "freeze" after age N — relevant for
   plant aesthetic (older branches lignify, no longer grow) more
   than crystal (freeze on contact).

Plant variant: directional bias creates branching toward concentrated
color votes.  Crystal variant: directional bias creates faceted
boundaries between competing color territories.

The full algorithm lives in phase 2's loft script; phase 2 produces
2-3 variants (plant / crystal / hybrid) so the presenter can switch
between them between rounds.

## Auto-camera (phase 3 detail)

The projector view doesn't pan manually — it follows activity.  Each
recent change (filled hex from a tap or from the growth simulation)
contributes a brief heat-trail at its position.  The camera derives
two things from the heat field:

- **Target position** — centroid of recent activity (last few
  seconds, exponentially weighted)
- **Zoom level** — inverse of activity spread.  All activity in one
  small region → zoom in to fill the projector with that detail.
  Activity spread across the map → zoom out to overview.

Smooth motion: lerp toward target position + zoom each frame; never
snap.  Audience members see their own contribution because the
camera notices where the action is.

Open sub-questions for the auto-camera:

- Zoom limits (min / max)
- Smoothing constant (snappier = more responsive but jittery; slower
  = calmer but the camera can lag interesting moments)
- Idle behaviour — when no recent changes, slowly pan around the map
  to show the full state, or hold last position?
- Override hotkey for the presenter to lock the camera or pin a
  specific area for narration

These are tuning knobs; the framework can ship with defaults and
get adjusted in rehearsal.

## Sub-arcs

| # | Phase | Builds | Status |
|---|---|---|---|
| 0 | Audience browser page | Pure HTML/JS — color-palette picker + `tap → WebSocket seed event` (with chosen color).  Phone-touch friendly | Open |
| 1 | Server: WebSocket + state hold + broadcast | Holds seed list + color-direction bias field; extends `lib/server`'s starter code | Open |
| 2 | Generation script | Loft program — plant/crystal growth from seeds + color-bias-pulled directional growth.  Produces 2-3 variants (plant, crystal, hybrid) for round-to-round variety.  THE STAR of the demo | Open |
| 3 | Native projector view | Subscribe to server world updates + render.  Includes **auto-camera**: tracks recent-change activity, zooms in when changes concentrate in one area, zooms out to overview when changes are spread.  Either a modified `lib/moros_editor` or a slimmer dedicated viewer | Open |
| 4 | Hosting | Deploy server to a public URL (VPS / ngrok / cloudflared / phone hotspot) | Open |
| 5 | Talk content | Slide deck + presenter script + presenter notes (mirror `presentations/par/` structure) | Open |
| 6 | Rehearsal + backup recording | One full dry run on demo hardware; record both demos as fallback | Open |

## Phase ordering

Phases 0, 1, 2 land in parallel — they share only the seed-event
schema (which can be sketched on paper first).  Phase 3 depends
on phase 1 (server-driven state).  Phase 4 unblocks public
testing.  Phases 5 + 6 depend on everything else working.

Recommended sequence: 0 + 1 (parallel) → 2 (parallel with 3) →
3 → 4 → 5 → 6.

The seed-event schema is the only cross-phase contract.  Lock it
in before anyone codes phase 0 or phase 1.  Suggested first cut:

```json
{ "type": "seed", "x": <hex_q>, "y": <hex_r>, "color": "<hex>" }
```

The world-update broadcast (server → projector + audience clients)
schema needs locking too — minimum: a per-tick delta of
`{ filled_hexes: [(x, y, color), ...] }` so the projector renders
the growth without recomputing it.  Phase 1 + 2 + 3 all consume
this; phase 1 produces it.

## Open design questions

1. **Plant vs crystal aesthetic** — both work mechanically but
   produce very different visual character.  Plant = organic
   branching, lignifying older cells, asymmetric growth;
   crystal = faceted territory boundaries, frozen-on-contact,
   geometric symmetry.  Phase 2 prototypes both before talk-deck
   freeze.
2. **Color palette size** — small (4-6 colors, easy thumb-pick on
   phone) vs. large (12-20 colors, more expressive).  Larger
   palettes make the color-bias direction field richer but make
   the audience UI fiddlier on phones.  Recommend 6-8.
3. **Direction-bias mechanic specifics** — does an audience
   member's color choice influence growth ONLY at their seed
   site, or globally as a vector pull on the field?  Local-only
   is simpler to implement + explain; global-pull produces more
   visible "everyone's choices interact" but is harder to predict.
4. **Round structure** — continuous (always-running, no resets) vs.
   timed rounds with reset between?  Timed rounds let presenter
   tweak generation script between rounds and demo "watch the same
   inputs produce different worlds."  Recommend timed rounds for
   the talk's narrative structure.
5. **Audience platform** — phone-touch friendly (optimize for
   thumb taps on small canvas + color palette) vs. laptop-only
   (expect mouse, larger targets)?  Phone-friendly opens broader
   participation but doubles the input testing surface.
6. **Presenter as a special role** — does the presenter get a
   reserved color + extra controls (clear-canvas, change-script,
   pause-generation), or are they just another audience member
   with script edits handled out-of-band on their laptop?

## Cross-arc dependencies

This demo is upstream-feeding for several pre-existing plans —
demo-driven scope sharpens what those plans need anyway:

- **`lib_plans/future/08-server`** — Phase 1 here is a focused
  extension of the server library (WebSocket + state hold +
  broadcast).  Anything built can become reusable lib/server
  primitives for later plans.
- **`lib_plans/future/13-scriptable-scenes`** — Phase 2 here (a
  loft generative script that the presenter tweaks live) is a
  proof-of-concept for the scriptable-scenes hot-reload
  architecture.
- **`plans/future/23-event-loop`** — the WebSocket protocol used
  here is a practical instance of EVENT_PROTOCOL (text-mode v1
  shipped per the plan README).

This demo deliberately does NOT depend on:
- **`plans/future/24-multiplayer-editor`** — full multi-paint moros
  editor.  Too heavy.  Audience uses simpler tap-only client.
- **`plans/future/32-tic-tac-toe`** — different protocol shape.
- **`lib_plans/future/10-game-client`** — audience client is dumb
  (just emits taps), no game-client library needed yet.

## Risks

| Risk | Mitigation |
|---|---|
| Conference WiFi blocks WebSocket / firewalls outbound | Host server on presenter's laptop + phone hotspot; have audience join via local AP if needed |
| Generation script crashes during live tweak | Rehearse 3-5 known-good scripts; presenter switches to a fallback if a tweak crashes |
| Audience > expected concurrent connections | Load-test up to 2× expected count; lightweight WebSocket library |
| Native projector view crashes mid-talk | Pre-recorded video backup of full demo; presenter narrates over it if needed |
| Audience hesitates to participate ("am I supposed to tap?") | Presenter starts the demo by tapping their own phone; "look, my hex appeared.  Now everyone try." |
| Generation script needs language features that don't exist yet | Lock the script's loft surface area early (phase 2 first cut); only use shipped features |

## See also

- [`../par/`](../par/) — sibling presentation (typed-par redesign);
  reference for slide-deck structure + supporting script layout
- [`../../plans/future/24-multiplayer-editor/`](../../plans/future/24-multiplayer-editor/) —
  adjacent plan (full multi-paint moros editor); this demo is a
  simpler subset
- [`../../lib_plans/future/08-server/`](../../lib_plans/future/08-server/) —
  server library this work extends
- [`../../lib_plans/future/13-scriptable-scenes/`](../../lib_plans/future/13-scriptable-scenes/) —
  adjacent plan (scriptable scenes); phase 2 here is its
  proof-of-concept
- `lib/moros_editor/` — existing 3D hex editor (potential basis
  for phase 3 projector view)
- `lib/server/` — existing starter code (phase 1 extends this)
- `lib/graphics/examples/` — existing creative-coding examples
  (potential demo source material)
