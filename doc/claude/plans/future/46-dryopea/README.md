<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN46 — dryopea: sci-fi free-build / tower-defence game

**Status:** **STALLED 2026-05-28** — development paused along with
moros and bumper-airplanes ([@PLAN50<!--noindex-->](https://github.com/jjstwerff/loft/tree/bumper_plane/doc/claude/plans/future/50-bumper-airplanes))
until the cross-project library structure is clear.  Three consumers
share substantial library surface (see
[lib_plans/12-library-extraction § Cross-project consumers](../../../lib_plans/12-library-extraction/README.md#cross-project-consumers--moros--dryopea--bumper-airplanes));
picking up consumer work before the shared libraries are factored
guarantees rework.  Resume when the user lifts the stall — Phase 7p
(cross-cutting renames + `lib_plans/NN-physics-2body` +
`lib_plans/NN-particles` slot designs) is the strong signal.

Original status: Future (design drafted 2026-05-21; no code). Depends on
[lib-plan 20 terrain height-map](../../../lib_plans/future/20-terrain-heightmap/README.md)
+ [lib-plan 19 gridmesh](../../../lib_plans/19-gridmesh/README.md) Phase C.
**Likely the FIRST real consumer of both** — it validates the
terrain→mesh→render stack at low *content* cost (algorithmic terrain, small
material palette), so building dryopea's vertical slice doubles as the
acceptance test for those primitives.

## Goal

A non-standard sci-fi tower-defence. At the **start of a match the player
places their core building** — the thing the enemies attack and the player
must defend (lose it = lose). The player rides a **semi-floating vehicle**
(over-the-shoulder 3rd-person camera; it hovers above terrain to avoid
clipping cliff edges), and rather than placing structures directly, **issues
build ORDERS** — towers, walls, bridges — that **NPC workers** then construct
over time. The player reacts in real time: repairing and buffing towers as
enemy waves and a boss approach, and **travelling the landscape** to find
hidden treasures that speed up upgrades. Walls are **≥1 hex wide and
walkable**, so the vehicle can drive along them to reach the core under
attack; **bosses can break walls**, severing those routes and re-opening the
enemy path.

What sets dryopea apart from every other tower-defence is the **scramble
phase**: a base is never a simple win-or-lose. When it's about to be
overrun, the player fires a rocket out of the core building, **evacuating key
components** to start the *next* base with an advantage. The game is a
**run** of bases, strung together by what you manage to carry out — see
[§ The scramble phase](#the-scramble-phase--the-signature-mechanic).

## The scramble phase — the signature mechanic

dryopea's identity. A base is **not** win-or-lose-forever; it's one round of
a longer **run**. When a base is about to be overrun, the player can
**scramble** — fire a rocket out of the core building and evacuate to the
next base.

- **The core building is also the escape rocket.** Defending it gains a
  second meaning: keep it alive long enough to launch. If the core is
  destroyed *before* you scramble, the **run ends** (the true loss). A
  successful scramble is a *tactical retreat*, not a defeat.
- **Salvage is a live tradeoff.** Evacuating a key component takes it with
  you but **disables the tower it came from** — so grabbing salvage weakens
  your remaining defence and *hastens* the overrun. Hold longer for more
  salvage, or launch now to keep what you already have: that tension is the
  core decision of the whole game.
- **Carry-over → the next base starts ahead.** Evacuated components give an
  advantage at the next base (stronger / earlier towers). A run is a
  **sequence of bases**, each a TD round, chained by what you carry out — a
  roguelike structure rather than a single defended base.

**Implications for the build:**
- The game is **multi-level / run-based**: each base is an editor-authored
  level (terrain + spawn points + item markers, per the editor/game split
  below); a run chains them. The cross-base **meta-state is the
  evacuated-component inventory** — the only thing that persists between
  bases.
- This is almost entirely **game logic** (run structure, the launch
  trigger/animation, component inventory, the salvage UI) — it adds little to
  the terrain / gridmesh / flow-field primitives. It's the meta layer on top,
  and the reason the whole engine exists: to make the scramble decision feel
  good.

## Design principle — bounded sessions (the rogue-lite opt-out)

A single base must be a **bounded, one-sitting mission**, and the boundaries
between bases must be **clean save-and-quit points**. This is a first-class
design constraint, not an afterthought — and a deliberate strength:

- **Short sessions are the rogue-lite appeal.** Much of why people love
  rogue-lites is the graceful opt-out: when you're short on time you reach a
  natural stopping point without losing progress. dryopea's **scramble IS that
  exit** — bail out of a failing base, keep your salvage, stop; resume the run
  later. A time-pressured player is *rewarded* for leaving, not punished.
- **Between-base = the save point.** The only cross-base meta-state is the
  evacuated-component inventory (§ scramble), so persisting a run between bases
  is tiny and clean. A base-in-progress need not be resumable mid-mission; the
  run is resumable at **base boundaries**, which keeps save-state trivial.
- **Pace a base to a target sitting.** Tune wave count / build time so a base
  resolves (win, scramble, or loss) within a bounded window. Treat "how long
  is a base" as a **tunable pacing parameter**, not an emergent accident —
  echoing @PLAN36's "sluggish by design" pacing principle (bounded tempo as a
  first-class lever).

## The editor / game split (architectural spine)

**The editor authors only TERRAIN; the running game places everything else.**

| | Authored in the editor (static) | Placed by the game at runtime (dynamic) |
|---|---|---|
| **Owns** | hex **terrain features** (ground types + slope, lib-plan 20) **+ a small set of static gameplay markers** — enemy **spawn points**, findable-**item positions** | **buildings, walls, bridges, towers** + spawned mobs |
| **Tooling** | the moros editor (hex paint + slope solver + the **existing item-layer** placement tool) | dryopea runtime build-order system |
| **Mutability** | baked content; re-saved with the level | mutable override layer; built + destroyed during play |
| **Data** | the solved height field + material per hex + markers via the **existing `h_item` item layer** | a separate structure/override store keyed by hex |

This split is the load-bearing decision:
- **Authored content vs runtime state.** The level file carries the terrain
  AND the static markers (spawn points, item positions); a save game carries
  the runtime structures + live mobs. They never mix, so a level can be
  replayed and structures reset without touching terrain or markers.
- **The editor's game-authoring is small and bounded** — terrain plus a few
  static markers (enemy spawn points, findable-item positions, and probably
  about the most of it). It has **no building/wall tool**: structures are
  placed only at runtime, so the bulk of dryopea-specific systems live in the
  game, not the editor.
- **Markers reuse moros's EXISTING item layer — no new editor work.** Spawn
  points and findable items are just entries in the moros item palette,
  placed onto a hex's `h_item` field with the item-placement tool that's
  already there. They're authored *positions*; the game decides what
  spawns/appears at each. **The more complex stencil machinery
  (`PlaceStencil` / `stencil_stamp`) is NOT needed yet** — single-hex item
  placement covers all the markers dryopea wants for now.
- **Structures are an OVERRIDE LAYER on top of solved terrain** (see lib-plan
  20 § "Built structures are a separate override layer"): a wall is
  raised-terrain (walkable top, steep sides), a bridge is a deck on a higher
  `cy` layer over preserved low ground; both reuse the terrain height +
  slope-face + gridmesh dirty-re-mesh machinery.

## Systems (game-specific scope)

0. **Match setup — place the core.** At the start of a base the player
   places the **core building** (the defend objective): it becomes the
   **flow-field goal** all mobs path toward, and it is **also the escape
   rocket** (see § scramble). Keep it alive long enough to scramble; if it's
   destroyed first, the **run** ends. It is the first runtime structure;
   everything else (towers/walls/bridges) is built to defend it.
1. **Floating vehicle + over-shoulder camera** — a hover controller that
   samples terrain height under the vehicle footprint (`world→hex →
   h_height`, max over the footprint, + clearance) so it rides above terraced
   steps; a 3rd-person follow camera.
2. **Build-order system** — the player marks a placement (tower / wall /
   bridge); NPC workers path to the site and construct it over time (not
   instant). Build-validity gated by `md_slope`/material (no towers on cliffs
   or water).
3. **Structure override layer** — walls (walkable hex-width ramparts),
   bridges (`cy`-layer decks toward other walls), towers; all mutable +
   destructible. Building/destroying = a height-override edit → dirty chunk →
   gridmesh incremental re-mesh.
4. **Multi-level pathing + enemy flow field** — a traversal graph over
   natural ground (slope-gated, `md_slope` = cost) **+ wall tops + bridge
   decks**, connected where adjacent at compatible heights.
   - **Enemy guidance = a flow field ("direction markers"), defined at
     runtime.** Compute a distance-from-goal field (BFS/Dijkstra from the
     **player-placed core building** — the fixed defend goal) over the
     *mob-walkable* graph; each cell's "direction marker" is the descent
     toward the nearest-to-goal neighbour. Mobs just sample the marker at
     their cell → O(1) per mob, scales to a horde with one field. This is the
     canonical TD flow field. **The goal is the (static) core, not the roaming
     vehicle — so the field recomputes only when WALLS change, never per
     frame.**
   - **Walls funnel mobs to gaps automatically.** A wall is a steep raised
     hex → impassable to *ground* mobs by the same `md_slope` gate that blocks
     cliffs (the floating vehicle ignores it — traversal is per-agent-type).
     The distance field routes around the obstacle, so its markers point
     through the **holes** in a wall line — no explicit gap-finding needed;
     it's a property of the field. Building a wall reshapes the markers;
     leaving a gap is how you choose where mobs come through.
   - **Recompute shares the wall dirty-regions.** A wall built or
     boss-broken dirties chunk(s) → the SAME dirty signal that triggers
     gridmesh re-mesh also re-solves the affected region of the flow field +
     re-routes. One dirty event, two consumers (mesh + field).
   - **Vehicle + NPC workers** use point-to-point routes (A* over the same
     graph), not the enemy flow field.
5. **Combat** — enemy waves spawned at the **editor-authored spawn points** +
   a boss that **breaks walls** (→ dirty re-mesh + path re-route); tower
   targeting; reactive player **repair/buff** of towers.
6. **Exploration / economy** — free-roam the terrain for hidden treasures (at
   the **editor-authored item positions**) → faster upgrades.
7. **Scramble + run meta** (the signature mechanic, see § above) — the launch
   trigger when the base is failing; the salvage selection (which components
   to evacuate, disabling their towers as you pull them); the
   evacuated-component **inventory** carried between bases; loading the next
   base with the carry-over advantage; the run win/lose bookkeeping
   (scramble = continue; core destroyed first = run over).

## Phases (vertical-slice first)

| # | Scope | Proves |
|---|---|---|
| **D0** | Terrain consumer — load an editor-authored level, render it via gridmesh Phase C, drive the floating vehicle over it (hover + over-shoulder camera); place the **core building** at a chosen hex. | the lib-plan 20 + gridmesh stack end-to-end (the dryopea-first validation). |
| **D1** | Structure override layer + build orders — mark a wall/tower, NPC worker constructs it, chunk re-meshes; build-validity from `md_slope`. | the override layer + dirty re-mesh on a built structure. |
| **D2** | Flow field + multi-level pathing — distance-from-core field guides mobs (funnels to wall gaps); vehicle + NPC point-to-point routes over ground + wall tops + bridge decks. | the flow field (recompute on wall change) + the traversal graph + bridge `cy`-layer. |
| **D3** | Combat slice — one enemy wave (spawns at authored points, flows to the core) + towers + a boss that breaks a wall (re-mesh + flow-field re-route). | destruction as a runtime dirty-rebuild driver for BOTH mesh + field. |
| **D4** | Economy / exploration — treasures + upgrades. | the loop closes. |
| **D5** | **Scramble + run** (the signature mechanic) — launch trigger, salvage selection (pull components → disable towers), carry-over inventory, load next base with the advantage, run win/lose. | the design THESIS — that the scramble decision is the fun. Build as soon as the single-base loop (D0-D3) is playable. |

Vertical slice (engine) = **D0 + minimal D1 + D2 + D3**: place the core, drive
in, order a wall (mobs funnel to the gap), a wave spawns and flows to the
core, a boss breaks the wall and re-opens the path. That slice exercises every
shared primitive.

**Fun slice = + D5 (minimal):** the single-base loop is only the *substrate*;
the game isn't proven fun until you can scramble out of a failing base and
arrive at the next one ahead. So although D5 builds last (it needs the base
loop first), prototype a minimal scramble the moment D0-D3 is playable —
that's the soonest the core design bet can be tested.

## Dependencies + shared primitives

- **lib-plan 20 terrain height-map** — REQUIRED (the height field dryopea
  drives over). dryopea supplies its own small material palette + drainage
  seeds.
- **lib-plan 19 gridmesh Phase C** — REQUIRED (per-chunk meshing, T4 auto
  slope-faces, dirty incremental re-mesh for wall build/destroy).
- **moros editor** — shared terrain-authoring tool (no dryopea changes; it
  only paints terrain).
- **Likely needs** (open): A*/flow-field pathfinding over the multi-level hex
  graph; an entity/update loop. Evaluate whether the override layer +
  multi-level pathing are dryopea-only or lib-worthy (moros may also gain
  runtime structures later) before building — keep game-specific systems
  (towers, waves, economy) in dryopea, lift genuinely shared mechanics to a
  lib only when a second consumer appears (the gridmesh "toolkit not
  framework" discipline).

## Open questions

1. **Multi-level pathing representation** — how the ground + wall-top +
   bridge-deck graph is stored and queried (per-hex walkable-surface list?
   the `cy`-layer model directly?).
2. **Build-order UI in 3rd person** — how the player targets a hex / line for
   a wall from an over-the-shoulder camera.
3. **Save/level format** — confirm the terrain-content vs structure-state
   split on disk.
4. **Enemy / wave / boss design** — out of scope until the slice works.
5. **Lib vs game boundary** — which of {override layer, multi-level pathing}
   become shared libraries vs stay in dryopea.

## Future expansion — planet-scale enemy economy + multiplayer (addons, post-core)

**Out of scope for the initial game; recorded so the core design doesn't
foreclose it.** The base-hopping run is the foundation; the long-horizon
vision opens it onto a whole planet:

- **The enemy is a faction with an ECONOMY** — robots and/or insects with
  **supply lines, mines, and factories** that *produce* the waves. Instead of
  only defending against waves, players push outward to **disrupt that
  economy** (cut a supply line, take a mine, knock out a factory) to weaken
  what spawns against them.
- **Get a foothold on the planet.** A persistent **planet map** frames the run
  as territorial: bases are positions on it. The early bases sit in **rural
  parts of the planet (easier)**; missions get **much harder** as players push
  toward the enemy's industrial heartland (the supply/mine/factory network).
  So the run's escalating difficulty has an in-world cause — you're nearing
  their production.
- **Multiplayer.** **Multiple players** operate on the same planet,
  coordinating to disrupt the enemy economy and establish footholds.
- **Players pick start spots — including persistent ABANDONED bases.** A run
  doesn't start from nowhere: the player chooses a spot on the planet, and
  those spots can be **bases abandoned earlier — by themselves OR a fellow
  player.** When you scramble out, the base **persists** on the planet with
  whatever you *didn't* evacuate (left-over resources + structures), and it's
  **saved and revisitable**. But it doesn't sit frozen: **mobs encroach over
  time**, so revisiting an abandoned base means inheriting its leftover
  resources (a head start) *and* an enemy presence already gathered nearby (a
  danger). This makes the scramble's "take vs leave" decision ripple across
  the *whole persistent world*: what you leave seeds a future restart — for
  you or an ally — but rots into a mob-infested spot the longer it's left.
  The same risk/reward tension as the scramble, now at planet scale.

**Architectural notes (so the core build leaves room):**
- The per-base game already produced by D0-D5 is the **unit** the planet
  campaign composes — a planet mission is "a base + an economy objective on
  the planet map." Keep base state self-contained (it already is: terrain
  content + structure/run state) so the campaign layer can place + sequence
  bases without reaching inside them.
- **Multiplayer reuses loft's shipped stack** — `lib/server` (multi-client
  WebSocket) + `lib/web` (WS client), the same infrastructure validated by
  @PLAN36 (audience) and the tic-tac-toe plans. The expansion is strategic
  game logic + a planet meta-map on top of proven netcode, not new transport.
- The **enemy economy as a wave SOURCE** generalises the D3 spawn model: waves
  stop being authored set-pieces and become *output of* the economy state
  (disrupt the factory → fewer/weaker waves). The flow field, terrain, and
  scramble all carry over unchanged.
- **Persistent abandoned bases = server-side world state.** An abandoned base
  saves a small snapshot: the un-evacuated structure/run state (already
  self-contained) + a timestamp / encroachment level. The "mobs gathered
  nearby" needn't be a live simulation — it can be *derived on revisit* from
  elapsed time + the base's exposure (cheap, deterministic), so the planet
  doesn't have to tick abandoned bases continuously. Shared revisiting (a
  fellow player's abandoned base) is server-authoritative persistent state on
  the same `lib/server` stack + loft's store — no new persistence layer, just
  per-base snapshots keyed by planet position.

## See also
- [lib-plan 20 terrain height-map](../../../lib_plans/future/20-terrain-heightmap/README.md)
  — the terrain primitive dryopea consumes (+ its § "Built structures are a
  separate override layer" boundary note).
- [lib-plan 19 gridmesh](../../../lib_plans/19-gridmesh/README.md) — per-chunk
  meshing + dirty re-mesh dryopea renders with.
- [@PLAN36 audience-generative-art](../../36-audience-generative-art/README.md)
  — sibling app-plan; the projector's 3rd-person GL camera + GPU mesh pipeline
  are reference material.
- `lib/moros_*` — terrain editor, map, render, sim packages.
