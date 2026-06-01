<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# MapFile schema — cross-project contract

Three consumer projects load MapFile JSON: **moros** (RPG, author +
runtime), **dryopea** ([@PLAN46](../../doc/claude/plans/future/46-dryopea/README.md),
sci-fi tower-defence — loads moros-authored maps), and
**bumper-airplanes** ([@PLAN50](../../doc/claude/plans/future/50-bumper-airplanes/README.md),
audience demo — extrudes a moros-authored MapFile into 3D).

This document is the **cross-project contract**: the schema the three
consumers agree on.  Schema changes need a `schema_version` bump and
a backwards-compatible loader for the old version.

## Status

**v1 of the schema is implicitly defined by `lib/moros_map`'s `Map`
struct.**  Per
[lib_plans/12 Phase 7a](../../doc/claude/lib_plans/12-library-extraction/README.md#phase-7a--moros-world-split-cross-project-unlock-appears-monorepo-internal),
the schema migrates to `lib/hex_world/` along with the moros_map fold.
Until that lands:

- The schema **is** what `lib/moros_map/src/moros_map.loft` declares.
- `pub fn hex_world::load_mapfile(path)` and `pub fn hex_world::save_mapfile`
  do not yet exist in `lib/hex_world/` — consumers use
  `moros_map::map_to_json` / `moros_map::map_from_json` directly.
- The migration to `lib/hex_world/` is **blocked on the consumer stall**
  ([memory: project_consumer_stall](../../doc/claude/lib_plans/12-library-extraction/README.md#why-a-separate-plan-from-packagesmd)).

This document captures the schema NOW so the eventual migration is a
mechanical move + import rename, not a re-design.

## v1 schema (the contract)

The MapFile is a JSON document with the structure below.  Field names
mirror the loft struct field names (loft's `{m:j}` format specifier
emits exactly the struct shape).

```jsonc
{
  // Display name — UI label; loaders treat any string as valid.
  "m_name": "untitled",

  // The hex grid, addressed by 32-wide chunks.
  // Each chunk holds 32x32 = 1024 hexes in row-major order
  // (hx * 32 + hz).  Empty chunks (no live cells) MAY be omitted —
  // missing chunks default to all-zero hexes.
  "m_chunks": [
    {
      "ck_cx":    0,         // chunk x-coord (floor-div of world q by 32)
      "ck_cy":    0,         // vertical layer
      "ck_cz":    0,         // chunk z-coord (floor-div of world r by 32)
      "ck_hexes": [          // exactly 1024 entries
        {
          "h_height":        0,   // 0..255 (game-defined units)
          "h_material":      0,   // palette index into m_material_palette
          "h_item":          0,   // palette index into m_item_palette
          "h_item_rotation": 0,   // bits 0-4: rotation (0-23)
                                   // bit  5:   spawn_flag
                                   // bit  6:   waypoint_flag
          "h_wall_n":        0,   // palette index into m_wall_palette
          "h_wall_ne":       0,
          "h_wall_se":       0
        }
        // … 1023 more …
      ]
    }
    // … more chunks …
  ],

  // Material (terrain) palette.  Index 0 is conventionally "void"
  // (transparent / empty).  Consumers free to add their own kinds
  // beyond what moros ships; loaders must preserve unknown fields
  // for round-trip.
  "m_material_palette": [
    {
      "md_name":      "grass",
      "md_color":     "#5fa030ff",
      "md_slope":     0.0,        // lib_plans/20 terrain-heightmap addition
      "md_drop":      0.0,        //   (default 0.0 = flat)
      "md_extrude":   ""          // PLAN50 bumper addition (default empty)
    }
  ],

  // Wall palette.  Hex sides reference these.
  "m_wall_palette": [ /* WallDef objects */ ],

  // Item palette.  Hexes' h_item references these.
  "m_item_palette": [ /* ItemDef objects */ ],

  // Spawn points + NPC waypoints/routines (game-state seed).
  "m_spawn_points":  [ /* SpawnPoint objects */ ],
  "m_npc_routines":  [ /* NpcRoutine objects */ ]
}
```

The exact field shapes for `MaterialDef`, `WallDef`, `ItemDef`,
`SpawnPoint`, `NpcWaypoint`, `NpcRoutine` are defined in
`lib/moros_map/src/{palette,spawn}.loft`.

## Versioning policy

v1 has no `schema_version` field.  **v2 adds one** as a required
top-level field; loaders treating the absence as v1.  Subsequent
versions append fields with sensible defaults; loaders ignore
unknown fields (forward-compat) and preserve them on round-trip
(so an old loader's save doesn't strip new fields a newer producer
added).

Breaking changes (renames, removals, type narrowing) require a
version bump + a per-version loader path in `hex_world::load_mapfile`.
Goal: no breaking change has shipped to date.

## Per-consumer overlays

Each consumer reads the same MapFile and applies its own overlay
logic at load time:

| Consumer | Overlay |
|---|---|
| **moros** | Authoritative — reads everything, including its own spawn/NPC state. |
| **dryopea** | Reads hex grid + materials.  Extends via [lib-plan 20 terrain-heightmap](../../doc/claude/lib_plans/future/20-terrain-heightmap/README.md): the `md_slope` / `md_drop` fields on `MaterialDef` drive the Eikonal solver that computes hex heights from painted slope (replaces hand-set `h_height` for natural terrain). |
| **bumper-airplanes** | Reads hex grid + materials.  Extrudes per `md_extrude` strings into 3D pillars / cliffs / ramps (`wall`, `wall_high`, `hill`, etc.).  Target bumpers live in a sibling `targets.json` keyed to the same hex coords (PLAN50 Open Q #6). |

Overlay fields are **opt-in** and have sensible defaults — moros maps
without `md_slope` (e.g. content authored before that field
existed) load cleanly under all three consumers.

## Migration plan to `hex_world::load_mapfile()`

When the consumer stall lifts and Phase 7a's moros_map fold proceeds:

1. Move `Map` + `Chunk` + `Hex` from `lib/moros_map/src/{moros_map,types}.loft`
   → `lib/hex_world/src/hex_world.loft`.  Field names stay identical so
   existing JSON loads cleanly.
2. Move `MaterialDef` + `WallDef` + `ItemDef` from
   `lib/moros_map/src/palette.loft` → `lib/hex_world/src/palette.loft`
   (new sibling module).
3. Move `SpawnPoint` + `NpcWaypoint` + `NpcRoutine` from
   `lib/moros_map/src/spawn.loft` → `lib/hex_world/src/spawn.loft`.
4. Add `pub fn world::load_mapfile(path: text) -> Map` and
   `pub fn world::save_mapfile(self: Map, path: text) -> FileResult`
   to `lib/hex_world/src/world.loft`.  Loader passes `text as Map`
   under the hood (the `{m:j}` / `text as T` machinery is shipped
   language behaviour).
5. Update `lib/moros_map/src/moros_map.loft` to be a thin shim:
   `pub use world::*;` — keeps every existing `moros_map::Map` call
   site working while consumers migrate.
6. Sweep consumers (`lib/moros_editor`, audience demo, dryopea,
   bumper) to `use world;` and drop the shim once references are
   gone.  Delete `lib/moros_map/` in the same change that finishes
   Phase 7b.

Until step 1 lands, this document is the contract; the *enforcer*
is the moros_map loft code.

## See also

- [lib_plans/12 § Phase 7a](../../doc/claude/lib_plans/12-library-extraction/README.md#phase-7a--moros-world-split-cross-project-unlock-appears-monorepo-internal)
  — drives the eventual move.
- [lib_plans/future/20 terrain-heightmap](../../doc/claude/lib_plans/future/20-terrain-heightmap/README.md)
  — adds `md_slope` + `md_drop` to `MaterialDef`.
- [lib_plans/future/24 universal-editor](../../doc/claude/lib_plans/future/24-universal-editor/README.md)
  — the editor that AUTHORS these files.
- [@PLAN46 dryopea](../../doc/claude/plans/future/46-dryopea/README.md)
  — consumes MapFile + lib-plan 20 height-field overlay.
- [@PLAN50 bumper-airplanes](../../doc/claude/plans/future/50-bumper-airplanes/README.md)
  — consumes MapFile + extrudes via `md_extrude` palette field.
