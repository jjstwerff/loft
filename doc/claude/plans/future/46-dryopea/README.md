<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN46 — dryopea: sci-fi free-build / tower-defence game

**Status:** Future (design drafted 2026-05-21; no code). Depends on
[lib-plan 20 terrain height-map](../../../lib_plans/future/20-terrain-heightmap/README.md)
+ [lib-plan 19 gridmesh](../../../lib_plans/19-gridmesh/README.md) Phase C.
**Likely the FIRST real consumer of both** — it validates the
terrain→mesh→render stack at low *content* cost (algorithmic terrain, small
material palette), so building dryopea's vertical slice doubles as the
acceptance test for those primitives.

## Goal

A non-standard sci-fi tower-defence. The player rides a **semi-floating
vehicle** (over-the-shoulder 3rd-person camera; it hovers above terrain to
avoid clipping cliff edges), and rather than placing structures directly,
**issues build ORDERS** — towers, walls, bridges — that **NPC workers** then
construct over time. The player reacts in real time: repairing and buffing
towers as enemy waves and a boss approach, and **travelling the landscape**
to find hidden treasures that speed up upgrades. Walls are **≥1 hex wide and
walkable**, so the vehicle can drive along them to reach a base under attack;
**bosses can break walls**, severing those routes.

## The editor / game split (architectural spine)

**The editor authors only TERRAIN; the running game places everything else.**

| | Authored in the editor (static) | Placed by the game at runtime (dynamic) |
|---|---|---|
| **Owns** | hex **terrain features** — ground types + slope (lib-plan 20) | **buildings, walls, bridges, towers** |
| **Tooling** | the moros editor (hex paint + slope solver) | dryopea runtime build-order system |
| **Mutability** | baked content; re-solved only when the level is edited | mutable override layer; built + destroyed during play |
| **Data** | the solved height field + material per hex | a separate structure/override store keyed by hex |

This split is the load-bearing decision:
- **Terrain is content, structures are state.** The level file carries the
  authored terrain; a save game carries the runtime structures. They never
  mix, so a level can be replayed and structures reset without touching
  terrain.
- **The editor never needs a building/wall tool** — it only paints terrain.
  Everything dryopea-specific lives in the game runtime.
- **Structures are an OVERRIDE LAYER on top of solved terrain** (see lib-plan
  20 § "Built structures are a separate override layer"): a wall is
  raised-terrain (walkable top, steep sides), a bridge is a deck on a higher
  `cy` layer over preserved low ground; both reuse the terrain height +
  slope-face + gridmesh dirty-re-mesh machinery.

## Systems (game-specific scope)

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
4. **Multi-level pathing** — a traversal graph over natural ground
   (slope-gated, `md_slope` = cost) **+ wall tops + bridge decks**, connected
   where adjacent at compatible heights. Used by the vehicle, NPC workers, and
   enemies. Edits on build/destroy.
5. **Combat** — enemy waves + a boss that **breaks walls** (→ dirty re-mesh +
   path re-route); tower targeting; reactive player **repair/buff** of towers.
6. **Exploration / economy** — free-roam the terrain for hidden treasures →
   faster upgrades.

## Phases (vertical-slice first)

| # | Scope | Proves |
|---|---|---|
| **D0** | Terrain consumer — load an editor-authored level, render it via gridmesh Phase C, drive the floating vehicle over it (hover + over-shoulder camera). | the lib-plan 20 + gridmesh stack end-to-end (the dryopea-first validation). |
| **D1** | Structure override layer + build orders — mark a wall/tower, NPC worker constructs it, chunk re-meshes; build-validity from `md_slope`. | the override layer + dirty re-mesh on a built structure. |
| **D2** | Multi-level pathing — vehicle + NPC routes over ground + wall tops + bridge decks. | the traversal graph + bridge `cy`-layer. |
| **D3** | Combat slice — one enemy wave + towers + a boss that breaks a wall (re-mesh + re-route). | destruction as a runtime dirty-rebuild driver. |
| **D4** | Economy / exploration — treasures + upgrades. | the loop closes. |

Vertical slice = **D0 + minimal D1 + D3**: drive in, order a wall, NPC builds
it, an enemy breaks it. That slice exercises every shared primitive.

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
