<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# lib-plan 20 — terrain: slope-based height-map generation

**Status:** ✅ **CLOSED 2026-08-26 — SUPERSEDED, not delivered** (@PLN71,
`status:declined`). `hex_terrain` 0.1.1 ships the need by another model
(`tt_rise`/`tt_steep`, priority-flood hydrology, window independence); the
Eikonal/painted-slope design below was never built. Measured at closure:
`md_slope` has one hit in the whole library ecosystem and it is prose in
`hex_world/MAPFILE.md`, and `emit_slope_face` — which T4 was to auto-call —
is in no shipped library. Kept as a closure record; read
`loft-libs-world/CONVERGENCE.md` before reviving any of it.

**Original status:** Future (design drafted 2026-05-21, no code yet). A
**game-agnostic** height-FIELD producer shared by two games — **dryopea**
(sci-fi free-build / tower-defence, likely first) and **moros** (RPG). It
feeds [lib-plan 19 gridmesh](../19-gridmesh/README.md) Phase C meshing
(gridmesh consumes a height field; this plan generates it).

## Why

Setting the height of every tile by hand is **tedious and error-prone**.
moros today offers only manual height tools — `RaiseHex`/`LowerHex`,
`slope_path`/`slope_band` (linear interpolation between two hand-picked
endpoints). Sculpting a believable landscape one hex at a time does not
scale and is hard to keep consistent.

Instead, let the artist paint **ground TYPES that carry a slope**, pin one
**low point** (a road leading out, a waterway, a sea shore), and have an
algorithm *compute* where the hills, cliffs, valleys and waterfalls are.
The artist controls *steepness per terrain type* + *where the water drains*;
the heights fall out deterministically.

## Consumers — two games, one primitive

The solver is **not a moros-only feature** — it is a shared height-field
primitive for two games that both need believable hill sides without
hand-sculpting, which is exactly why it's a library plan, not game code:

- **dryopea** ([@PLAN46](../../plans/49-dryopea/README.md)) —
  sci-fi free-build / tower-defence. **Likely the FIRST consumer** (and so
  the first validation target). The player rides in a
  **semi-floating vehicle** (over-the-shoulder 3rd-person camera), issues
  build ORDERS for towers/walls that **NPC workers** construct, repairs/buffs
  towers reactively as enemies/a boss approach, and **travels the landscape**
  for hidden treasures (faster upgrades). "Less 3D sculpting" means **less
  hand-authored content** (paint broad terrain types → the solver generates
  the hills; no bespoke set-pieces), NOT a simpler renderer — the
  ground-level moving camera drives the **full** gridmesh Phase-C pipeline
  (per-chunk meshing + frustum culling + LOD + slope faces). Validate against
  dryopea first because its *authoring* is small (a handful of materials +
  one drain), so it exercises the whole stack at low content cost.
- **moros** — RPG. The richer *content* consumer: full rolling-hill / cliff /
  river / waterfall terrain hand-painted in the moros editor, feeding the
  same gridmesh per-chunk world mesh.

Same algorithm, same `md_slope`/`md_drop` data; the games differ only in
which terrain materials they stock and how dense their drainage networks
are. Keep the solver game-agnostic (toolkit, not framework) — each game
supplies its own palette + seeds, mirroring the gridmesh "consumer supplies
the rule" discipline. Build/validate order: **dryopea first → moros next.**

### The slope field is gameplay substrate, not just cosmetics (dryopea)

`md_slope` is a **static per-material attribute** (an input), so it serves
THREE jobs at once — and two of them don't even need the solver to have run:

1. **Height** — the solver integrates `md_slope` outward into terrain
   geometry (this plan's primary, *derived* output → `h_height`).
2. **Build-validity** — towers/walls can't be placed on too-steep ground or
   water; gate placement on `md_slope[h_material]` (+ material flags) read
   straight off the palette. A static threshold; no solver involvement.
3. **Path cost** — vehicle / NPC builders / enemies / boss move over the
   terrain; steep = slow or impassable. `md_slope[h_material]` is the
   movement-cost field for pathfinding. Also static.

So painting one slope number per material drives terrain shape, what's
buildable, AND how things move — the reason it belongs on the material, not
as a separate hand-set layer.

Two further dryopea-specific notes:
- **Stepped/terraced terrain is fine — by design.** The semi-floating vehicle
  hovers at `local terrain max + clearance`, so it never clips a cliff edge
  ("prevent height crashes"). dryopea is therefore comfortable with the
  **discrete-Dijkstra staircase** as a sci-fi terraced aesthetic — **FMM
  smoothing (T5) is a moros nicety, not a dryopea blocker.**
- **Vehicle hover query** — the only new terrain read dryopea needs is
  "height under the vehicle footprint," i.e. `world→hex → h_height` (+ a
  small max over the footprint hexes). Cheap; no new structure.

## The model — slope is gradient, height is accumulated climb

The key reframe: a ground type's **slope is local steepness (gradient
magnitude), not "how tall this tile is."**

- A **hill** is not a tall tile — it is a *region* of gentle-slope tiles far
  enough from the drain that the climb adds up.
- A **cliff / mountain** is a *band* of steep **rock** tiles where the climb
  jumps in one or few steps.
- **Fields** are nearly-flat (slope ≈ 0); **house bases / floors** are dead
  flat (slope = 0) — so building pads sit level.
- A **river** is a low corridor whose own tile types encode its descent:
  **meander** (≈ flat), **normal current** (small drop), **rapids** (medium
  drop), **waterfall** (big drop).

Formally this is the **Eikonal equation** `|∇h| = slope(type)` with the
drainage seed as the boundary condition, solved outward. The discrete
solver on the hex grid is **multi-source Dijkstra**, where the cost
accumulated along a path is height climbed. This is the standard
"terrain-from-hydrology" reconstruction — well-trodden, not speculative.

> **Why min-path (Dijkstra):** a tile's height is the *gentlest climb* from
> the nearest drain. Min-path keeps valleys (low-slope corridors) low and
> pushes height up only where the cheapest route must cross steep terrain.
> A rock band ringing a flat area becomes a **plateau on a cliff**: every
> interior route climbs the rock once, then stays flat → flat top, steep
> wall, straight out of the algorithm.

## Fit with the moros data model (no schema rewrite)

The Explore pass (2026-05-21) confirmed moros already has the pieces; the
only data addition is **a slope attribute on the material palette**.

| Need | Already present | To add |
|---|---|---|
| Height output | `Hex.h_height` (unbounded `integer`), already extruded by `hex_to_world` / `build_hex_meshes` (`lib/moros_render`) | none — the solver writes it via `map_set_height` |
| Ground type | `Hex.h_material` → `MaterialDef` palette, with `md_category` ("terrain"/"water") and `md_tint_r/g/b` (the "colored ground hexes" are free) | **`md_slope`** (rise units per hex step) + **`md_drop`** (for water tiles) |
| Neighbours incl. cross-chunk | `map_get_hex(q,r,cy)` (negatives + sparse chunks → zero) + `neighbour_dq/dr` (6 dirs) | none |
| What it replaces | `slope_path` / `slope_band` (manual linear-interp tools) | the solver supersedes them |

`MaterialDef` lives at `lib/moros_map/src/palette.loft`; the per-hex height
read/write is `map_get_hex` / `map_set_height` (`lib/moros_map/src/moros_map.loft`).

Stock terrain materials to seed the palette: `field` (slope ≈ 0), `grass`
(gentle), `rock` (steep — cliffs/mountains), `floor`/`pad` (slope 0), and
water variants `meander` / `current` / `rapids` / `waterfall` (carry
`md_drop`, not `md_slope`).

## The algorithm (the whole thing)

```
# inputs:  material palette with md_slope (rise/step) + md_drop (water),
#          seeds = drainage network tiles (shore / road / river) + one
#          pinned outlet height (the global low point, h = 0).
# output:  Hex.h_height for every reachable hex.

# 1. Drainage-network pass — turn the river/road/shore into seed heights.
#    Pin the outlet at h = 0.  BFS each water/road chain from its outlet
#    UPSTREAM; each step adds the tile's drop:
#        meander ≈ 0,  current = small,  rapids = medium,  waterfall = big.
#    Result: every network tile is a seed carrying its own
#    (rising-upstream) height.

# 2. Multi-source Dijkstra over the land — integrate slope outward.
#    PQ := all seed tiles at their pinned heights.
#    pop lowest h; for each of 6 neighbours nb (map_get_hex — crosses chunks):
#        climb = 0.5 * (md_slope[cur] + md_slope[nb]) * hex_spacing
#        if h[cur] + climb < h[nb]:
#            h[nb] = h[cur] + climb;  push nb
#    write round(h[nb]) into Hex.h_height (map_set_height).
```

O(N log N), deterministic (break ties by hex index for reproducibility).

**Single low seed is the MVP** — exactly the artist's "start point."
Multi-source (shore + roads + river) is the *same algorithm for free* and
gives richer terrain: back-valleys behind a ridge get their own nearby
drain instead of rising forever.

How it composes:
- **Waterfall** = big `md_drop`: the river height jumps there, so land
  beside it is forced into a tall transition — paint **rock** alongside and
  the gorge appears automatically.
- **Meandering valley** = low-`md_slope` water corridor: Dijkstra routes the
  lowest heights along it; hills rise on both banks.
- **Crater / interior basin** (lower than its rim): place an interior seed
  (a pond / sink) at a pinned low height — the multi-source solve handles it.

## Phases

| # | Scope | Effort |
|---|---|---|
| **T1** | `md_slope` + `md_drop` on `MaterialDef`; a handful of stock terrain materials (field / grass / rock / floor / water variants). | XS |
| **T2** | The solver: drainage-network pass + multi-source Dijkstra writing `h_height`. Pure logic, headless, cross-mode unit tests against reference shapes. | M |
| **T3** | Editor mode: paint material type (shows `md_tint` colour) + mark seed hex(es) + a "solve" key → heights set → the **existing 3D view extrudes them** (no new rendering). The separable test the feature was scoped around — **colored ground hexes, no buildings.** | S |
| **T4** | Auto slope-faces: `emit_slope_face` already exists in `moros_render` but is not auto-called between height-differing neighbours — wire it so cliffs render as vertical faces, not stepped tops. (Also a gridmesh Phase-C meshing rule.) | S |
| **T5** | (Later) Incremental re-solve over only the edit-affected region, reusing the gridmesh dirty-chunk machinery; **FMM** (Fast Marching) upgrade if smooth/isotropic terrain is wanted over the discrete-Dijkstra staircase. | M |

## Test / verification (T2 + T3)

Cross-mode (interp + native), against hand-computed reference shapes:
- **Single cone** — one slope type, one seed → radial height ramp.
- **Plateau ringed by cliff** — rock band around flat interior → flat top at
  the cliff height.
- **River valley** — water corridor + grass banks → low channel, rising banks.
- **Waterfall** — a `waterfall` tile mid-river + flanking rock → height jump
  + gorge.

T3 is the visible end-to-end check: paint colours, mark a seed, solve, watch
colored terrain rise in the moros 3D view.

## Limits to design around

- **Unreached tiles** (no path to any seed) stay 0 — flag them in the editor
  so an unconnected region is visible, not a silent flat patch.
- **Hex-Dijkstra is mildly anisotropic** — long even slopes can show a faint
  6-direction "staircase" bias. Fine for a quantized tile world; FMM (T5) is
  the isotropic upgrade.
- **Global pass, not chunk-local** — the drainage network spans chunks, so
  the solve is world-level (run once per edit). Natural two-stage split:
  **global height solve (this plan) → per-chunk meshing (gridmesh Phase C).**
  Incremental re-solve is T5.
- **Integer height** — clamp/scale the accumulated climb so a huge world
  can't overflow a sensible height range.

## Where it fits

- **Feeds** [lib-plan 19 gridmesh](../19-gridmesh/README.md) Phase C: the
  height field is the *input* to `build_chunk_mesh`. This plan is the
  "moros Phase C" precursor the gridmesh DESIGN defers to "its own sub-plan."
- **Reuses** `lib/moros_map` (`MaterialDef`, `map_get_hex`/`map_set_height`,
  chunk addressing), `lib/moros_render` (`hex_to_world`, `emit_slope_face`,
  `neighbour_dq/dr`), `lib/moros_editor` (paint tools, undo).
- **Independent of** buildings/items — this plan computes NATURAL terrain
  only.

### Built structures are a separate override layer (dryopea — out of scope here)

dryopea's player-built **walls** and **bridges** are NOT this plan's concern,
but they're built *entirely on top of* its machinery, so the boundary is
worth pinning:

| Layer | What | Mutable | Reuses |
|---|---|---|---|
| Natural terrain | slope-solved height field (this plan) | re-solved on edit | the solver |
| Built walls | **hex-width walkable ramparts** = runtime height *overrides* on the same field (raised hexes, steep sides, flat top wide enough for the vehicle) | built / destroyed at runtime | terrain height + T4 slope-faces + gridmesh dirty re-mesh |
| Bridges | walkable decks spanning a *low* gap toward another wall — a surface ABOVE preserved low ground | yes | moros's existing `hash<Chunk[cx,cy,cz]>` vertical-`cy`-layer model + multi-level pathing |

Key points for whoever builds the dryopea build-system (its own plan,
[@PLAN46](../../plans/49-dryopea/README.md)):
- A rampart is **raised terrain**, not moros's per-edge `h_wall_*` faces —
  two distinct "wall" concepts; keep the naming separate.
- **Boss-breaks-wall** = remove the height override → revert to natural
  ground → dirty the chunk → gridmesh incremental re-mesh (the live runtime
  driver for the C3 dirty-rebuild work) + traversal-graph re-route.
- Walls/bridges must therefore be a **mutable override layer**, never baked
  into the solved field.
- The static `md_slope` (above) gates where ramparts/towers may be built and
  costs the vehicle/NPC/enemy pathing across natural ground + wall tops +
  bridge decks.

## See also
- [lib-plan 19 gridmesh DESIGN](../19-gridmesh/DESIGN.md) — § 7 moros
  consumer + § 8 step 8 (moros Phase C).
- `lib/moros_map/src/{palette.loft,moros_map.loft,types.loft}` — material
  palette + height accessors + Hex/Chunk structs.
- `lib/moros_render/src/moros_render.loft` — height extrusion + slope faces.
