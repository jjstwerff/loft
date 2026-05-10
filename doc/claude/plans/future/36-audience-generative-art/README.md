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
| 1 | [01-server-state.md](01-server-state.md) | Loft server: hold world state, age cells, run automatic decay (older cells removed; filled-neighbour lease extends life so shrinking starts at the edges), broadcast world deltas + active-player signals.  No autonomous growth — placement is pure direct painting from audience taps | M |
| ~~2~~ | ~~02-generation-script.md~~ | DROPPED — closed by the 2026-05-10 growth-model decision (pure direct painting; no autonomous growth).  Renderer-side ridge / edge classification covers what would have been the "plant / crystal aesthetic" generator | — |
| 3 | [03-projector-view.md](03-projector-view.md) | Native loft beamer client: subscribe to server, render full hex world (frost-style 3D crystal mesh + edge-detected plant/crystal aesthetic), auto-camera follows activity heat field, presenter hotkey overrides | M |
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

Reference aesthetic: **ice forming on a cold window pane** —
dendritic / fern-like silhouettes with a central spine, feathery
secondary branches, and needle tips.  Mostly negative space.
The camera can see *through* a bridge to crystals behind, and
through the gaps of any single crystal to what is past it.

Crystal **tops are never flat**.  The mesh-generation algorithm
produces ridge lines at even height (the frost spines) flanked
by triangles that slope into shallow crevices (between
branches).  Across many adjacent filled hexes, ridge lines
preferentially connect into longer fern-like features.

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
nearby hexes toward the new one.  Direction is derived
client-side from comparing this cell's age with its neighbours'
ages, so no per-cell direction metadata travels on the wire.

### Three views, three roles

The same world is rendered three different ways depending on
who is looking:

| View | Renderer | Camera | Hex grid visible | 3D crystals visible |
|---|---|---|---|---|
| **Projector** (the spectacle) | WASM 3D | Auto, follows activity | NO — pure dark stage | YES |
| **Desktop client** | WASM 3D (same renderer) | User-controlled (mouse-drag + WASD pan, scroll zoom) | YES — base plane under the crystals | YES |
| **Phone client** | Plain HTML/JS 2D | Pan via outer-ring touch | YES — the entire view IS the grid | NO — pure orthogonal flat hexes |

The phone deliberately does not need the WASM renderer at all,
keeping it responsive on modest hardware.  Desktop and projector
share the renderer; the desktop adds base-plane render +
click-to-paint, the projector adds auto-camera + hides the base
lattice.

### Tempo philosophy — sluggish by design

The whole demo is **deliberately sluggish**: 10 Hz server tick
(not 20+); ~5 second crystal growth animation per placed cell;
~2 second aesthetic-swing window; gentle decay over tens of
seconds; placed cells don't snap on instantly, removed cells
don't disappear instantly.  This is **a feature, not a bug**.

Many audience members tapping at once would otherwise overwhelm
the spectacle — bursts of input would produce visual chaos no
single person could parse.  The slow tempo absorbs input rate
spikes into a flowing visible river of growth + decay.  Each
audience member sees their input land but with enough delay and
animation that the result reads as part of the collective
rather than as a private input → output reaction.

Concrete consequences this principle locks in:

- Server tick rate stays at 10 Hz (not faster).
- Growth animation stays at ~5 seconds (not snappier).
- Aesthetic-swing window stays at ~2 seconds (not 500 ms).
- Direct-paint actions don't get optimistic local echo —
  the lag IS the desired pacing.

If a tuning question later asks "should X be faster / snappier?"
the default answer is **no, lean into sluggish** unless
rehearsal proves the lag has crossed from atmospheric into
disconnected.

### Self-cleaning lifecycle

Together with the decay rule, the system **self-cleans its
data structures over time**: cells expire, chunks empty out and
get garbage-collected, in-flight growth/decay animations finish
themselves.  No manual cleanup is needed during the talk and no
persistence layer is needed after.  The server can run
indefinitely without operator intervention; a crash-recovery
restart from blank produces the same end-state the decay rule
would have produced anyway given enough time.

### World data layout

The world reuses the **`lib/moros_map` chunk pattern** — sparse
32×32 chunks at integer chunk coordinates, with the same
`chunk_idx_32` / `hex_idx_32` addressing helpers (which handle
negative-coordinate floor-division correctly).  A chunk is
created on first non-empty write at coordinates inside it; a
chunk is **removed when all 1024 of its cells are empty**.

Per-cell payload is **4 bytes**: 1 byte colour (0 = empty, 1-9
= palette), 1 byte height (0..255, derived from filled-neighbour
count), 2 bytes age (0..65535 ticks).  No per-cell `(q, r)`,
direction, or planting-author metadata — all derivable from the
chunk address + cell index + neighbour ages.

Detail in [`01-server-state.md` § State model](01-server-state.md#state-model).

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

1. **Plant vs crystal aesthetic** — RESOLVED: they are not
   separate variants.  Both emerge from a single renderer based
   on the local shape of the filled region.  Thin lines (dotted,
   1-wide, 2-wide) read as **plant** with ridges following the
   line tangent and curving smoothly through bends (anticipated
   2 cells before, trailed 2 cells after).  Wider blobs read as
   **crystal** with the default radial pattern.  A single drawn
   line **swings between the two** as audience input continues.
   Detail in [`03-projector-view.md` § Plant vs crystal — emergent
   from filled-region shape](03-projector-view.md#plant-vs-crystal--emergent-from-filled-region-shape).
2. **Color palette size** — RESOLVED at 9: indices 1-3 RGB
   primaries, 4-6 CMY additive mixes, 7-9 white / grey / brown.
   Index 0 is reserved for "empty hex" in the world state and is
   never sent in a `seed` event.  Detail in
   [`00-audience-browser-page.md` § Zone 2](00-audience-browser-page.md#zone-2--color-palette-middle).
3. ~~**Direction-bias mechanic**~~ — MOOT (autonomous growth
   removed).  With pure direct painting, no generation step
   needs colour-direction bias.  Closed by the 2026-05-10
   "growth model" decision.
4. ~~**Round structure**~~ — RESOLVED 2026-05-10: no rounds.
   Demo runs continuously from start of talk to end; the
   automatic decay rule keeps the canvas alive without manual
   resets.  Presenter narrates and shows code at chosen
   moments without segmenting the painting timeline.
5. ~~**Audience platform**~~ — RESOLVED 2026-05-10: both
   phone (portrait touch) and desktop (laptop pointer + larger
   world view) are first-class targets.  Phone is the primary
   layout; desktop is a separate UI tuned for mouse + keyboard.
   Detail in [`00-audience-browser-page.md` § Desktop variant](00-audience-browser-page.md#desktop-variant).
6. ~~**Presenter as a special role**~~ — RESOLVED 2026-05-10:
   no.  Presenter uses the regular phone or desktop client like
   everyone else.  Operational actions (server restart, clear
   canvas if needed) happen out-of-band via SSH or the local
   shell on the server box; nothing is exposed in the audience
   UI.  Avoids a third client surface to build + test.

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
| Q1 plant vs crystal | 2026-05-10 (design review) | Single renderer, both aesthetics emerge from local-shape edge detection | Thin-line cells follow line tangent with curve lookahead 2 cells before/after a bend (plant); wider blobs fall back to default radial pattern (crystal); the swing between aesthetics as audience input changes is part of the spectacle |
| Q2 palette size | 2026-05-10 (design review) | 9 colours, indices 1-9; index 0 = empty | RGB primaries (1-3) + CMY mixes (4-6) + white/grey/brown (7-9) covers spectrum + neutrals; 9 is comfortable for thumb-pick on phone; 0 reserved for world-state "empty hex" so colour and emptiness share one field |
| Q3 direction-bias mechanic | 2026-05-10 (design review) | MOOT — no autonomous growth | Closed by the growth-model decision: pure direct painting needs no per-direction colour bias |
| Q4 round structure | 2026-05-10 (design review) | No rounds — continuous demo from start to end | Automatic decay keeps the canvas alive without manual resets, removing the original reason for round boundaries.  Presenter narrates + reveals code at chosen moments without segmenting the painting timeline.  Simplifies server (no reset RPC), client (no "round X" UI), and presenter flow |
| Q-growth model | 2026-05-10 (design review) | Pure direct painting + automatic age-based decay (no autonomous growth) | Cells appear only from audience taps + swipes.  Server runs no generation simulation.  Decay is an automatic per-tick step: older cells expire and are removed; filled-neighbour count extends the effective lease so decay starts at the edges of clusters and works inward (inverse-growth aesthetic).  Removes Q3 (direction-bias) from scope and reshapes Q4 (round structure) |
| Q-decay tuning | 2026-05-10 (design review) | base_lifetime = 5 minutes (3000 ticks at 10 Hz); lease_per_neighbour ≈ 1 minute; decay_window ≈ 30 seconds (slow shrink) | Sluggish-by-design: no cell decays before 5 minutes; once eligible, removal animates over ~30 s rather than snapping.  An isolated edge cell expires at 5 min + 30 s; a fully-surrounded cell survives ~11 min + 30 s.  Re-validate at first prototype |
| Q5 audience platform | 2026-05-10 (design review) | Both phone (touch) and desktop (pointer + keyboard) are first-class | Phone layout is the primary; desktop is a separate UI optimised for larger world view + mouse + keyboard.  Same WebSocket protocol, same world model — just two layouts.  Doubles input testing surface but lets non-phone audience members participate fully and gives the presenter a usable laptop fallback |
| Q6 presenter special role | 2026-05-10 (design review) | No special role — presenter uses regular phone or desktop client | Operational actions (restart, clear canvas) handled out-of-band via SSH / local shell on the server box.  Avoids a third client surface to build + test; presenter blends in with the audience |
| Q-three-view-roles | 2026-05-10 (design review) | Three distinct view roles — projector (3D, no base grid, auto-camera), desktop (3D + base grid, user camera, paints), phone (2D pure orthogonal grid, no 3D, paints) | Projector is the pure spectacle (no input, no lattice); desktop is the projector renderer + base-plane input; phone is the smallest possible flat paint surface.  Phone does not need the WASM renderer at all |

## Cross-arc dependencies

**WebSocket primitives the demo needs are already shipped** (post
TTT v5):

- `lib/server/src/server.loft` ships multi-client WebSocket
  (`srv.run(on_event)` + `ws_clients_*`, `ws_event_*`) plus
  `send_binary` + `last_opcode` on the single-client path.
- `lib/web/src/web.loft` ships the WebSocket client (`ws_handler`,
  `ws_connect`, `ws_send`, `ws_recv`) plus `send_binary`,
  `last_opcode`, and the `pack_*` / `byte_at` binary helpers.
- `lib/world/src/world.loft` ships the sparse 32×32 World/Chunk/
  Cell + `tick_and_decay`.

Sub-arcs 1 + 3 are application code on top, not library
extensions.

**Hard prereqs (sibling plan):**

- [plan-34 — `lib/server` hardening](../34-server-hardening/README.md)
  items (a), (b), (e):
  - (a) `srv.broadcast_binary()` / `srv.send_binary_to()` — the
    projector's world-snapshot + delta broadcasts depend on this.
    Without it, plan-36 has to inline `n_ws_send_binary` in loft
    (TTT v5 t4 already did this as a workaround; not the right
    long-term shape).
  - (b) Server-side binary recv via `from_utf8_unchecked` — the
    phone client's drag events stay ASCII-safe today, but any
    future client→server binary path (e.g. compact swipe
    encoding) would silently corrupt without this.
  - (e) `srv.run_with_tick(on_event, on_tick, tick_ms)` — the
    1 Hz `active_player_signal` heartbeat needs a tick-driven
    broadcast scaffold.  Without it, the server can only emit
    on event arrival, which breaks the "audience always sees
    fresh activity" property.
- Items (c), (d), (f) of plan-34 are post-launch hardening (the
  30-client soak, panic isolation, observability metrics).
  Useful for production rehearsal; not blocking phase 1.

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
- `plans/22-mutable-closures` + the [TTT v6 retrofit](../32-tic-tac-toe/README.md#tic-tac-toe-v6--ergonomic-retrofit-using-writable-closures)
  — drops the `Reference<T>.inner` ceremony from the server's
  pump callback so the loft snippets shown on stage during the
  "loft snippet highlights" beats read at their best.  If
  plan-22 lands before the talk, plan-36 server uses writable
  closures; if not, it uses `Reference<T>` exactly like TTT v5.
  Demo functions either way; only on-screen elegance differs.

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

**Wire-protocol primitives validated by TTT v5:**

`plans/future/32-tic-tac-toe` § "Tic-tac-toe v5" carries the
binary-frame extension to `lib/server` + `lib/web`, the
session-tagged blob protocol, the N-client routing pattern,
the catch-up recovery handler, and the sluggish-tempo tick-loop
behaviour — each validated with the smallest possible text-mode
test program.  The TTT board uses the same `World` / `Chunk` /
`Cell` data model this plan defines, so primitives proven there
translate to plan-36 with zero protocol glue.

Build TTT v5 first; this plan's phase 1 (server) and phase 0
(phone client binary decoder) become consumers of proven
infrastructure rather than co-developing it.

**Deliberately does NOT depend on:**

- `plans/future/24-multiplayer-editor` — audience client is a
  dumb tap-emitter, not the full moros editor
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
