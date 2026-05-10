<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 36 — Audience-driven generative-art demo (development plan)

## Status

Open.  Scoped 2026-05-09 to support an upcoming local-meetup talk
to game creators + art enthusiasts.  Sibling presentation plan at
[`../../../presentations/audience-generative-art/`](../../../presentations/audience-generative-art/)
owns the talk shape, slides, and audience-participation flow.
This plan owns the **development work** the demo needs.

## Goal

Build the development pieces for an audience-participatory
plant/crystal-growth simulation on a hex world: audience members
seed growth from their phones / laptops via a single shared URL;
choose colors that bias growth direction; native projector view
auto-cameras to recent activity; a loft generative script
(live-tweakable) drives the simulation.

The work upstream-feeds three pre-existing plans
(`lib_plans/08-server`, `lib_plans/13-scriptable-scenes`,
`plans/23-event-loop`) by sharpening their scope to a concrete
deliverable; the demo's WebSocket primitives are already shipped
in `lib/server` + `lib/web`, so the work is application code on
top of them, not library extension.

## Sub-arcs

| # | File | Builds | Effort |
|---|---|---|---|
| 0 | [00-audience-browser-page.md](00-audience-browser-page.md) | Pure HTML/JS smartphone client — 9×7 hex world view (with movement-zone outer rings), 9-color palette, clear + jump-to-active controls, tap/swipe paint, WebSocket | S |
| 1 | [01-server-state.md](01-server-state.md) | Loft server: hold world state, drive tick loop, broadcast world deltas + active-player signals.  Builds on shipped `lib/server` multi-client WS API | M |
| 2 | [02-generation-script.md](02-generation-script.md) | Loft generative script: plant/crystal growth from seeds + neighbor color blending biased by direction-color votes.  2-3 variants for round-to-round switching.  THE STAR of the demo | M |
| 3 | [03-projector-view.md](03-projector-view.md) | Native loft beamer client: subscribe to server, render full hex world, auto-camera follows activity heat field, presenter hotkey overrides | M |
| 4 | [04-hosting.md](04-hosting.md) | Public URL reachable from venue WiFi (VPS / hotspot / ngrok / cloudflared).  Operational, not code | XS |
| 5 | [05-rehearsal-and-backup.md](05-rehearsal-and-backup.md) | One full dry run on demo hardware; record both demos as fallback for catastrophic failure | XS |

Phases 0, 1, 2 land in parallel — they share only the seed-event
schema (sketched on paper first).  Phase 3 depends on phase 1
(server-driven state).  Phase 4 unblocks public testing.  Phase 5
depends on everything else working.

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

## World shape — 3D crystal mesh

The world the projector renders is **not flat** — and it is
**not a closed surface** either.  The mesh is built from filled
hexes only; empty hexes are background lattice with no geometry.
Each filled crystal's height grows monotonically with its filled-
neighbor count (lone filled hex = lowest crystal point, fully-
surrounded = highest plateau).  Two filled hexes one cell apart
form a small **bridge** by each extending laterally toward the
other and meeting over the empty cell between them.  A line of
filled hexes forms a continuous **ridge**.

Both crystals and bridges are **sparse / skeletal** — faceted
geometry with visible negative space.  The camera can see
*through* a bridge to crystals behind, and through the gaps of
any single crystal to what is past it.  Aesthetic: quartz cluster
or open lattice, not solid lava hill.

Each crystal's mesh is built from triangles, **each triangle one
solid palette colour with hard edges to its neighbours** — no
gradients, no shader-side blending.  Most triangles take the
cell's own colour; some take a 1-away neighbour's colour; a few
take a 2-away tile's.  At projection distance the eye fuses the
mosaic into a perceived colour-bleed across cluster boundaries,
but the underlying geometry is always solid-colour triangles
with hard edges.  The cell's own colour reads unambiguously at a
glance.

When a new hex becomes filled, the crystal **grows over ~5
seconds** in the direction extruded from the cluster of older
nearby hexes toward the new one.

Full rendering rules + open mesh-shape questions in
[`03-projector-view.md` § Visual model](03-projector-view.md#visual-model--3d-crystal-mesh).
The audience client (phone) stays 2D top-down for input
clarity; the projector is the spectacle that shows the world's
true 3D shape.

## Generation algorithm — plant / crystal growth

The seed list is a set of `{ position, color, planted_at_tick }`
records.  Each simulation tick:

1. For every empty hex adjacent to a "live" cell, decide whether
   it gets filled this tick (probability based on neighbor count
   + growth-rate parameter).
2. If filled, pick its color from the weighted majority of live
   neighbors, **biased toward the dominant color in the direction**
   of the new cell relative to the seed clusters.  Audience color
   choices form a vector field that pulls growth-color decisions.
3. Optionally: cells "die" / "freeze" after age N — relevant for
   plant aesthetic (older branches lignify) more than crystal
   (freeze on contact).

Plant variant: directional bias creates branching toward
concentrated color votes.  Crystal variant: directional bias
creates faceted boundaries between competing color territories.

Phase 2 produces 2-3 variants (plant / crystal / hybrid) so the
presenter can switch between them between rounds.

## Auto-camera (phase 3 detail)

The projector view doesn't pan manually — it follows activity.
Each recent change (filled hex from a tap or from the growth
simulation) contributes a brief heat-trail at its position.  The
camera derives:

- **Target position** — centroid of recent activity (last few
  seconds, exponentially weighted)
- **Zoom level** — inverse of activity spread.  All activity in
  one small region → zoom in; spread across the map → zoom out

Smooth motion: lerp toward target each frame; never snap.

Tuning knobs (defaults + adjust in rehearsal): zoom min/max,
smoothing constant, idle behaviour (slow pan vs hold), presenter
override hotkey to lock the camera.

## Open design questions

1. **Plant vs crystal aesthetic** — both work mechanically but
   produce very different visual character.  Plant = organic
   branching, lignifying older cells, asymmetric growth;
   crystal = faceted territory boundaries, frozen-on-contact,
   geometric symmetry.  Phase 2 prototypes both before talk-deck
   freeze.
2. **Color palette size** — small (4-6 colors, easy thumb-pick on
   phone) vs. large (12-20 colors, more expressive).  Recommend
   6-8.
3. **Direction-bias mechanic** — does an audience member's color
   choice influence growth ONLY at their seed site, or globally
   as a vector pull on the field?  Local-only is simpler;
   global-pull produces more visible "everyone's choices interact"
   but is harder to predict.
4. **Round structure** — continuous (always-running) vs. timed
   rounds with reset between?  Timed rounds let the presenter
   tweak the generation script between rounds and demo "watch the
   same inputs produce different worlds."  Recommend timed
   rounds.
5. **Audience platform** — phone-touch friendly vs. laptop-only?
   Phone-friendly opens broader participation but doubles the
   input testing surface.
6. **Presenter as a special role** — reserved color + extra
   controls (clear-canvas, change-script, pause-generation), or
   just another audience member with script edits handled
   out-of-band?

## Check-in points (regular validation moments)

This plan is built for interactive work — each check-in is a
deliberate pause where the previous phase gets validated and the
next phase's open questions get answered.  Don't skip them; the
open questions exist precisely so they can be answered with
information from the prior phase, not pre-committed.

| Check-in | After phase | What's validated | What's decided |
|---|---|---|---|
| **CI-0** | Before any code | (none — agreement that the schema sketch in Sub-arcs § works) | Lock the seed-event + world-update schemas as the cross-phase contract |
| **CI-1** | Phase 0 (browser page) | Tap → seed event reaches a stub WebSocket on localhost; phone-touch ergonomics tested on 2-3 actual phones | Q5 (audience platform — phone vs laptop), Q2 (palette size after seeing real-thumb-pick) |
| **CI-2** | Phase 1 (server state) | Stub client → server holds state → broadcasts back.  Multi-client test (2-3 connections) | Q4 (round structure: continuous vs timed), since round structure shapes server reset logic |
| **CI-3** | Phase 2 prototype (generation) | Simulated audience taps + generation algorithm produces visuals.  Both plant + crystal variants prototyped before this gate | Q1 (plant vs crystal — pick after SEEING both), Q3 (direction-bias: local vs global, picked from observed visual character) |
| **CI-4** | Phase 3 (projector view + auto-camera) | End-to-end on demo machine: audience client → server → projector renders.  Auto-camera tuning observed in motion | Q6 (presenter special role: do the controls actually feel needed in practice?) |
| **CI-5** | Phase 4 (hosting) | External machine reaches the server via the public URL; latency under acceptable threshold (~200ms round-trip) | (none — pure validation) |
| **CI-6** | Phase 5 (talk content draft) | Slide deck + presenter script reviewed for narrative arc; presentation-side open questions resolved (see [`presentations/audience-generative-art/`](../../../presentations/audience-generative-art/)) | Decisions in sibling presentation plan |
| **CI-7** | Phase 6 (rehearsal) | Full dry run on demo hardware; backup recording tested as fallback path | **Ship or postpone** — explicit go/no-go with the user |

## Decision log

Decisions get recorded here as each check-in resolves them.  The
log carries the resolution + a one-line *why* so the rationale
survives the talk.

| Question | Resolved at | Decision | Why |
|---|---|---|---|
| Q1 plant vs crystal | (pending CI-3) | — | — |
| Q2 palette size | (pending CI-1) | — | — |
| Q3 direction-bias mechanic | (pending CI-3) | — | — |
| Q4 round structure | (pending CI-2) | — | — |
| Q5 audience platform | (pending CI-1) | — | — |
| Q6 presenter special role | (pending CI-4) | — | — |

## Cross-arc dependencies

**No hard dependencies on currently-open ROADMAP plans.**  The
WebSocket primitives the demo needs are already shipped:

- `lib/server/src/server.loft` ships multi-client WebSocket
  (`srv.run(on_event)` + `ws_clients_*`, `ws_event_*`)
- `lib/web/src/web.loft` ships the WebSocket client (`ws_handler`,
  `ws_connect`, `ws_send`, `ws_recv`)

Sub-arcs 1 + 3 are application code on top, not library
extensions.

**Soft dependencies — would benefit but not block:**

- `lib_plans/future/13-scriptable-scenes` — hot-reload of the
  generation script between rounds.  Without it, presenter
  restarts the script (acceptable; the talk script can frame
  "now let me change the rules and re-run").
- `plans/07-error-messages` phases 4-7 — nicer errors if something
  goes wrong on stage.  Not blocking; presenter has rehearsed
  fallback.
- `plans/future/27-developer-experience` DX.1 / DX.3 — talk
  content overlaps with quick-start `examples/` and the "Learn
  loft in 30 minutes" walkthrough.  Can write the talk inline OR
  land both at once.

**Latent risk:** `plans/future/15-closure-validation` phase 03 /
closure-DbRef leak (LIFETIME.md "NOT YET HANDLED").  Generation
script will use closures heavily.  Leak is bounded per closure-
creation, not per tick — a 30-60 minute talk session is fine; an
unattended installation running for hours could accumulate.

**Upstream-feeds (this work sharpens scope for):**

- `lib_plans/future/08-server` — phase 1 patterns (state hold +
  per-tick broadcast) reusable as server primitives
- `lib_plans/future/13-scriptable-scenes` — phase 2 is a
  proof-of-concept for the script architecture
- `plans/future/23-event-loop` — phase 1 is a practical
  EVENT_PROTOCOL instance

**Deliberately does NOT depend on:**

- `plans/future/24-multiplayer-editor` — audience client is a
  dumb tap-emitter, not the full moros editor
- `plans/future/32-tic-tac-toe` — different protocol shape
- `lib_plans/future/10-game-client` — not needed yet

## Risks

| Risk | Mitigation |
|---|---|
| Conference WiFi blocks WebSocket / firewalls outbound | Host server on presenter's laptop + phone hotspot; have audience join via local AP if needed |
| Generation script crashes during live tweak | Rehearse 3-5 known-good scripts; presenter switches to fallback if a tweak crashes |
| Audience > expected concurrent connections | Load-test up to 2× expected count; lightweight WebSocket library |
| Native projector view crashes mid-talk | Pre-recorded video backup of full demo; presenter narrates over it if needed |
| Audience hesitates to participate ("am I supposed to tap?") | Presenter starts the demo by tapping their own phone; "look, my hex appeared.  Now everyone try." |
| Generation script needs language features that don't exist yet | Lock the script's loft surface area early (phase 2 first cut); only use shipped features |

## See also

- [`../../../presentations/audience-generative-art/`](../../../presentations/audience-generative-art/) —
  sibling presentation plan: talk shape, slides, audience flow
- [`../../../presentations/par/`](../../../presentations/par/) —
  reference for slide-deck structure
- [`../24-multiplayer-editor/`](../24-multiplayer-editor/) —
  adjacent plan (full multi-paint moros editor); this demo is a
  simpler subset
- [`../../../lib_plans/future/08-server/`](../../../lib_plans/future/08-server/) —
  server library; this plan's phase 1 sharpens its scope
- [`../../../lib_plans/future/13-scriptable-scenes/`](../../../lib_plans/future/13-scriptable-scenes/) —
  scriptable-scenes; this plan's phase 2 is its proof-of-concept
- `lib/moros_editor/` — existing 3D hex editor (potential basis
  for phase 3 projector view)
- `lib/server/` — shipped multi-client WebSocket support
- `lib/web/` — shipped WebSocket client
- `lib/graphics/examples/` — existing creative-coding examples
