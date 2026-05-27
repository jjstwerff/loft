<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Universal hex-world editor — architecture reference

The durable design content for the universal-editor +
moros-extraction arc.  [`README.md`](README.md) carries
status + the forward path; this file is the **why** and the
**how**.

---

## Table of contents

- [Vision](#vision)
- [Audience model](#audience-model)
- [Library architecture](#library-architecture)
- [Slice-based extraction strategy](#slice-based-extraction-strategy)
- [Per-package outline](#per-package-outline)
- [Per-phase details](#per-phase-details)
- [Moros backward-compatibility](#moros-backward-compatibility)
- [Design principles](#design-principles)
- [Test discipline](#test-discipline)
- [Open questions](#open-questions)
- [Why this plan matters](#why-this-plan-matters)
- [See also](#see-also)

---

## Vision

**One editor, many games.**

The loft suite already provides the low-level pieces — GL,
canvas software-rasterizer, hex math (scattered), JSON
serialisation, a 2D physics shape lib, gridmesh, package
format.  What it does NOT provide today is a coherent
**editor + content-pipeline + runtime substrate** that
multiple games can sit on top of.

Today, moros has its own editor (in `lib/moros_editor` +
`lib/moros_sim`), its own map shape (in `lib/moros_map`),
its own render path (in `lib/moros_render`).  Dryopea
reimplements parts of all three because moros's packages
have moros-specific assumptions baked into their names and
API.  A future indie consumer building a hex-world game
would face the same wall.

This plan extracts the **genuinely game-agnostic substrate**
out of moros's packages, names it neutrally, and exposes
**per-game hooks** for the rest (palette indices, item
kinds, wall semantics, render overlays, editor input
bindings, etc.).  The result:

- Moros consumes the neutral substrate + provides its own
  moros-specific layers on top.
- Dryopea consumes the same substrate + provides its own
  dryopea-specific layers on top.
- A future indie picks up the substrate and builds a third
  game on it without re-deriving the editor.

The editor is **the connecting tissue**: a game-agnostic
authoring tool that any hex-world game inherits.

## Audience model

Three audiences benefit from this plan in different ways.
Their needs shape design decisions.

### 1. Moros — the first partner

Moros is **already running on its own code**.  This plan must
**not break moros** at any point.  Every slice extraction
keeps moros functional; moros's existing tests provide the
regression net.  Moros benefits passively from each
extraction: the shared lib is sharper than the moros-only
version because dryopea + later consumers stress-test it.

### 2. Dryopea — the second consumer

Dryopea is **mid-development** with its own plan slice
(`dryopea/plans/future/06-editor-stencil-pipeline/`) that
explicitly relies on this extraction.  Dryopea's plan 06
phases consume slices L1-L6 as they land.  Dryopea's
integration is where bugs surface — moros uses the code
one way, dryopea uses it differently, and the difference
exposes assumptions.

### 3. Indie / strike-path users — the third audience

Eventually, **a starting dev or indie team builds a new
hex-world game on the suite** without ever touching moros
or dryopea internals.  They pick up `hex_grid` +
`hex_map` + `hex_stencil` + `hex_render` + `hex_editor`
+ `hex_entity`, register their own palette and item
kinds, and ship.  This is the strategic unlock — see
dryopea plan 06 § Who this serves for the strike-path
framing.

**Design implication:** the substrate must be usable
without reading moros source.  API documentation +
example consumer + onboarding tutorial.  Phase L7 covers
this explicitly.

## Library architecture

Six packages, in a clean layered stack:

```
                  ┌──────────────────────┐
                  │   hex_entity (L6)    │  ← runtime: baked-mesh
                  │  baked-mesh runtime  │     entities, composition,
                  │   composition,pivots │     pivot animation
                  └──────────┬───────────┘
                             │
            ┌────────────────┼───────────────────┐
            │                │                   │
            ▼                ▼                   ▼
  ┌────────────────┐ ┌──────────────────┐ ┌─────────────────┐
  │  hex_editor (L5)│ │  hex_render (L3) │ │ hex_stencil (L4)│
  │ tools+undo+UI  │ │ mesh emitters +  │ │ format + stamp  │
  │                │ │ 3D camera        │ │ save / load     │
  └────────┬───────┘ └─────────┬────────┘ └────────┬────────┘
           │                   │                    │
           └───────┬───────────┴────────────────────┘
                   ▼
              ┌──────────────────┐
              │   hex_map (L2)   │  ← data shape: multi-layer map,
              │  multi-layer,    │     paint verbs, palette,
              │  paint verbs     │     JSON round-trip
              └────────┬─────────┘
                       ▼
              ┌──────────────────┐
              │   hex_grid (L1)  │  ← pure math: axial coords,
              │  axial flat-top  │     distance, line walks,
              │  pure math       │     world conversions
              └──────────────────┘
```

Dependencies flow downward.  `hex_grid` is leaf (no internal
deps); `hex_entity` is root (depends on render + map +
stencil).  `hex_editor` is a special case — it depends on
map + stencil + render + a UI layer (probably `moros_ui`'s
generalised successor, named TBD in L5).

### What lives outside this stack (existing libs that stay)

- **`graphics`** — GL bindings + Canvas software rasterizer.
  `hex_render` depends on it.
- **`gridmesh`** — generic mesh utility.  `hex_render` may
  use it.
- **`shapes`** — 2D geometry (Rect, Circle, overlap).
  Used by `hex_editor` for hit-test / selection.
- **`world`** — legacy package; status unclear; possibly
  consumed by `lib/world_*` or maybe dead code.  Audit
  during L0.

## Slice-based extraction strategy

**Don't do a big-bang extraction.**  Moros code is
rough-but-unit-tested; lifting all of it into shared libs
in one PR invites a long cleanup tail.  Instead, **each
phase extracts the smallest useful slice**:

1. **Lift the slice verbatim** from moros into the new
   shared package (literal copy, same source for now).
2. **Inherit the moros tests** as the baseline coverage.
3. **Adjust moros to consume the new package** instead of
   its in-tree copy.  Moros keeps working.
4. **Have the second consumer (dryopea or other) integrate
   the same package.**  Bugs surface — different palette
   indices, different wall conventions, different stencil
   sizes, different cy-axis usage.
5. **Each bug fix lands in the shared lib's source.**  Add
   a regression test driven by the consumer's use case.
   Run both moros's and the new consumer's tests.
6. **Iterate until the slice is solid**, then move to the
   next slice.

This is the dogfood loop applied to library extraction.
**The first consumer (moros) provides safety net + initial
correctness; the second consumer (dryopea) drives quality
improvement; the third consumer (indie) gets a hardened
substrate.**

### Slice ordering

Order phases by **smallest deliverable that's useful to a
consumer**, not by moros's internal coupling:

- **L1 `hex_grid`** first — leaf dependency, smallest scope,
  easiest to verify.  Both moros and dryopea instantly
  consume.
- **L2 `hex_map`** next — the data shape.  Most of moros
  builds on top.  Largest extraction; boundary decisions
  about palette / item / wall conventions.
- **L3 `hex_render`** parallel-or-after L2 — mesh emitters.
  Can extract while L2 settles since mesh emitters mostly
  operate on the new `Map` shape from L2.
- **L4 `hex_stencil`** after L2 + L3 — stencil format
  builds on the map shape + uses render to bake.
- **L5 `hex_editor`** after L2 + L4 — editor tools depend
  on paint verbs + stencil format.  Largest API-design
  surface (per-game hook registration).
- **L6 `hex_entity`** after L3 + L4 — entity runtime
  consumes baked meshes from stencils.  Mostly greenfield.
- **L7 documentation** — final pass; assumes L1-L6 stable.

Phases L2 + L3 can run in parallel after L1.  L4 + L5 + L6
queue serially behind their deps.

## Per-package outline

### `hex_grid` — pure math (L1)

**Public API surface:**

```loft
// Axial flat-top hex.  Pure data — no map, no rendering.
pub struct Hex { q: integer, r: integer }

// World-space conversion (matches axial flat-top convention).
pub fn hex_to_world(h: Hex) -> (float, float);
pub fn world_to_hex(x: float, y: float) -> Hex;

// Distance + line-walking.
pub fn hex_distance(a: Hex, b: Hex) -> integer;
pub fn hex_line(a: Hex, b: Hex) -> vector<Hex>;     // inclusive

// Disc enumeration.
pub fn visible_hexes(centre: Hex, radius: integer) -> vector<Hex>;

// Cube-coord helpers (for line-walking, rounding).
pub fn cube_round_axial(qf: float, rf: float) -> Hex;

// Hex direction helpers (6 cardinal directions).
pub enum HexDir { East, NorthEast, NorthWest, West, SouthWest, SouthEast }
pub fn hex_neighbor(h: Hex, dir: HexDir) -> Hex;
```

**Source material:** scattered across moros_map + moros_render
+ dryopea's `src/world.loft`.  Consolidate into a clean
single-purpose package.

**Sizing constants** (HEX_DIAMETER, HEX_FLAT_TO_FLAT) are
**not** in `hex_grid` — they belong to the consuming game's
config (moros uses one scale, dryopea another, an indie a
third).  The package exposes `pub fn flat_top_factors() ->
(float, float, float)` for the inverse constants the
math needs but takes the diameter as a parameter where
relevant.

### `hex_map` — data shape + paint verbs (L2)

**Public API surface (sketch):**

```loft
// One cell in the multi-layer map.
pub struct HexCell {
    material:  integer not null,    // index into game's palette
    height:    integer not null,    // game-defined units
    item:      integer not null,    // 0 = none; otherwise game's item id
    walls:     integer not null,    // bit-packed: 6 edges
    rotation:  integer not null,    // 0-31; item rotation + flags
}

// Multi-layer chunk (cy = vertical layer index).
pub struct Chunk {
    cells: hash<HexCell[q, r, cy]>,
}

// Sparse multi-chunk map (or single chunk for finite games).
pub struct Map {
    chunks: hash<Chunk[cx, cz]>,
}

// Paint verbs (mutating).
pub fn paint_material(m: &Map, q, r, cy, material);
pub fn set_height(m: &Map, q, r, cy, height);
pub fn place_item(m: &Map, q, r, cy, item, rotation);
pub fn remove_item(m: &Map, q, r, cy);
pub fn set_wall(m: &Map, q, r, cy, dir: HexDir, wall: integer);

// Reads.
pub fn get_cell(m: const Map, q, r, cy) -> HexCell;
pub fn has_chunk(m: const Map, q, r, cy) -> boolean;

// Slope shaping.
pub fn slope_path(m: &Map, q1, r1, q2, r2, cy, h_start, h_end);
pub fn slope_band(m: &Map, q1, r1, q2, r2, cy, h_start, h_end, width);

// JSON serialisation.
pub fn map_to_json(m: Map) -> text;
pub fn map_from_json(json: text) -> Map;
```

**Per-game customisation** lives outside the package:

- The **palette** (material 0 = sea or grass or whatever the
  game says).
- The **item registry** (item 5 = NPC spawn in moros vs sap
  tree in dryopea).
- The **wall semantics** (which directions are climbable for
  which entity types).
- The **height units** (cm? mm? hex-units?).

The package treats these as **opaque integers**.  Each game
registers its own meanings.

**Source material:** lib/moros_map (most of it directly).

### `hex_render` — mesh emitters + 3D camera (L3)

**Public API surface (sketch):**

```loft
// Mesh emitters — generate triangle geometry from map data.
pub fn emit_hex_surface(m: &Mesh, q, r, cy, height);
pub fn emit_wall_quad(m: &Mesh, c1: Vec3, c2: Vec3, floor_y, ceil_y);
pub fn emit_slope_face(m: &Mesh, c1: Vec3, c2: Vec3, y_low, y_high);
pub fn emit_thick_flat_wall(m: &Mesh, c1, c2, thickness, height, ...);
pub fn emit_thick_curved_wall(m: &Mesh, c0, c1, c2, thickness, height, ...);
pub fn emit_linear_stair(m: &Mesh, c1, c2, h_low, h_high, ...);
pub fn emit_spiral_stair(m: &Mesh, centre, radius, h_low, h_high, turns, ...);
pub fn emit_box(m: &Mesh, min_corner: Vec3, max_corner: Vec3);
pub fn emit_cylinder_post(m: &Mesh, base, height, radius, segments);
pub fn emit_item_placeholder(m: &Mesh, kind: text, pos: Vec3, scale: float);

// Whole-map mesh.
pub fn emit_map_mesh(m: Map, mesh: &Mesh, opts: RenderOpts);

// 3D camera.
pub struct RenderCamera { ... }
pub fn camera_default() -> RenderCamera;
pub fn camera_orbit(c: &RenderCamera, d_azimuth_deg, d_elevation_deg);
pub fn camera_zoom(c: &RenderCamera, delta);
pub fn camera_pan(c: &RenderCamera, dx, dy, dz);
pub fn camera_follow(c: &RenderCamera, target_pos: Vec3, facing_rad: float);
pub fn camera_view_matrix(c: const RenderCamera) -> Mat4;
pub fn camera_projection_matrix(c: const RenderCamera, aspect: float) -> Mat4;
pub enum CameraMode { Follow, Overview, OverTheShoulder }
```

**Per-game customisation:**

- **Material → colour mapping** is a game-provided callback
  (`fn(material_index) -> integer rgba`).
- **Wall geometry parameters** (thickness, taper, etc.) are
  game-tunable.
- **Slope unit interpretation** depends on game's height
  semantics.

**Source material:** lib/moros_render + lib/wall.loft.

### `hex_stencil` — stencil format + stamp + save/load (L4)

**Public API surface (sketch):**

```loft
// One cell in a stencil (subset of HexCell — relative coords).
pub struct StencilCell {
    dq:        integer,
    dr:        integer,
    dcy:       integer,
    material:  integer,
    height:    integer,
    item:      integer,
    walls:     integer,
    rotation:  integer,
}

// Stencil definition.
pub struct StencilDef {
    name:      text,
    category:  text,            // game-customisable: "house", "robot", "tree"
    cells:     vector<StencilCell>,
    footprint: (integer, integer, integer, integer),  // q_min, r_min, q_max, r_max
}

// Stamp a stencil into a map.
pub enum StampMode { Overwrite, Underlay, Merge }
pub fn stencil_stamp(m: &Map, st: StencilDef, q, r, cy, mode: StampMode);
pub fn stencil_stamp_rotated(m: &Map, st: StencilDef, q, r, cy, rotation: integer, mode);

// Save a region as a stencil (extract from existing map).
pub fn stencil_save(m: const Map, name, category, q1, r1, q2, r2, cy) -> StencilDef;

// JSON round-trip.
pub fn stencil_to_json(st: StencilDef) -> text;
pub fn stencil_from_json(json: text) -> StencilDef;

// Built-in stencils (moros's set; could move to game-specific later).
pub fn stencil_house_small() -> StencilDef;
pub fn stencil_flat() -> StencilDef;
pub fn stencil_spiral_stair() -> StencilDef;
```

**Source material:** the stencil_* family in
lib/moros_editor/src/moros_editor.loft (lines ~250-400).

### `hex_editor` — editor tools + undo + UI (L5)

**Public API surface (sketch):**

```loft
// Undo / redo.
pub struct UndoStack { ... }
pub fn undo_empty() -> UndoStack;
pub fn paint_material_with_undo(s: &UndoStack, m: &Map, q, r, cy, material);
pub fn set_height_with_undo(...);
pub fn set_wall_with_undo(...);
pub fn slope_path_with_undo(...);
pub fn stencil_stamp_with_undo(s: &UndoStack, m: &Map, st: StencilDef, q, r, cy, mode);

pub fn undo_pop(s: &UndoStack, m: &Map) -> boolean;
pub fn redo(s: &UndoStack, m: &Map) -> boolean;
pub fn batch_begin(s: &UndoStack);
pub fn batch_end(s: &UndoStack);

// Editor tools (brush, line, area, stencil-stamp).
pub enum EditorTool { Brush, Line, Area, StencilStamp, Picker, Eraser }
pub struct EditorState {
    tool:           EditorTool,
    active_layer:   integer,    // cy axis
    active_material: integer,    // game's palette index
    active_item:    integer,
    active_stencil: text,       // stencil name
    rotation:       integer,
    // ... + cursor state, drag state, hover state
}

// Input dispatch (game registers its own hotkey bindings).
pub fn editor_input_apply(s: &EditorState, m: &Map, u: &UndoStack, raw: RawInput, hooks: GameHooks);

// Game registers callbacks for game-specific dispatch:
pub struct GameHooks {
    palette_size:       integer,             // how many materials
    item_count:         integer,             // how many item kinds
    is_drivable_wall:   fn(wall_kind: integer, entity_kind: integer) -> boolean,
    material_swatch:    fn(material: integer) -> Canvas,
    render_overlay:     fn(cv: &Canvas, map: const Map, cam: RenderCamera) -> void,
    // ... extensible per game
}
```

**Source material:** lib/moros_editor + lib/moros_sim/tools.loft +
relevant parts of lib/moros_ui (panel, widgets, font).

This is the **largest API-design phase** because of the
per-game hook surface.  Expect L5 to take longer than
earlier slices.

### `hex_entity` — baked-mesh entity runtime (L6)

**Public API surface (sketch):**

```loft
// A baked mesh (output of stencil bake).
pub struct BakedMesh {
    vertices:   vector<Vec3>,
    triangles:  vector<integer>,
    materials:  vector<integer>,
    pivots:     vector<NamedPivot>,
}

pub struct NamedPivot {
    name:       text,
    position:   Vec3,
    axis:       Vec3,
}

// Bake a stencil into a standalone mesh.
pub fn stencil_bake(st: StencilDef, opts: BakeOpts) -> BakedMesh;
pub fn baked_mesh_load(path: text) -> BakedMesh;
pub fn baked_mesh_save(bm: BakedMesh, path: text);

// Entity composition.
pub struct EntityManifest {
    base_mesh:       text,                  // path to baked mesh
    child_meshes:    vector<ChildMount>,
}
pub struct ChildMount {
    pivot_name:      text,                  // name on parent
    child_mesh:      text,                  // path to child mesh
    behaviour:       MountBehaviour,
}
pub enum MountBehaviour {
    Fixed,
    TrackTarget,
    OscillateRate { period_s: float, amplitude_rad: float },
    PlayerControlled,
}

// Runtime entity.
pub struct Entity {
    manifest:        EntityManifest,
    position:        Vec3,
    orientation:     float,
    child_state:     vector<float>,         // per-pivot angle, etc.
}

pub fn entity_spawn(manifest: EntityManifest, pos: Vec3) -> Entity;
pub fn entity_tick(e: &Entity, dt: float, ctx: TickContext);
pub fn entity_render(e: const Entity, mesh: &Mesh, cam: const RenderCamera);
```

**Source material:** mostly greenfield.  Moros has mesh
emitters and the camera, but no entity-composition or
movable-baked-mesh runtime.  L6 is the first phase that
*adds* substantial new code rather than re-homing existing
code.

### Future packages (not in this plan, flagged for awareness)

- `hex_jointed` — leg / multi-joint animation for organic
  units (per dryopea plan 06 S4).  Sketched as deferred;
  lands when an organic-unit consumer emerges.
- `hex_particle` — particle systems for elementals / fire
  / water effects.  Different rendering path entirely; may
  not belong in the `hex_*` family at all.

## Per-phase details

### Phase L0 — Architecture spike

**Goal:** lock the package names, lock the dependency graph,
lock the naming conventions for per-game hooks.

**Activities:**

- Read actual moros source bodies (not just public-API
  signatures) to identify moros-specific assumptions that
  need decoupling.  Catalogue:
  - Hard-coded material indices.
  - Hard-coded item types.
  - Wall direction conventions.
  - Height unit assumptions.
  - JSON schema specifics (versioning, field names).
- Decide naming: `hex_grid` / `hex_map` / `hex_render` /
  `hex_stencil` / `hex_editor` / `hex_entity`.  Or
  `buildkit_*`?  `world_*`?  `tile_*`?  See § Open
  questions.
- Sketch the per-game `GameHooks` contract for L5.
- Update this REFERENCE.md with concrete decisions.
- File P-issues for moros-side cleanup needed before L1
  (if any).

**Deliverable:** updated REFERENCE.md + a `00-l0-spike.md`
phase file recording decisions made.

**Effort:** S (one session, mostly reading + decisions).

### Phase L1 — `hex_grid` extraction

**Goal:** smallest possible useful package — pure math.

**Activities:**

- Create `lib/hex_grid/` package.
- Copy hex math from moros_map + moros_render + dryopea's
  src/world.loft.
- Reconcile naming + signatures.  One canonical
  `hex_to_world` (taking diameter as a parameter or
  exposing inverse factors).
- Move tests where they exist.
- Adapt moros to consume `hex_grid` (remove its in-tree
  copies; `use hex_grid` instead).
- Confirm moros tests pass.
- Have dryopea consume `hex_grid` — collapse its
  `src/world.loft` to a thin wrapper or remove entirely.
- Bug-hunt at dryopea integration.

**Deliverable:** `lib/hex_grid/` package; moros consuming;
dryopea consuming; both projects' tests green.

**Effort:** S (small extraction, big win).

### Phase L2 — `hex_map` extraction

**Goal:** the data shape + paint verbs + JSON serialisation.

**Activities:**

- Create `lib/hex_map/` package.
- Lift moros_map's Hex / Chunk / Map structs verbatim;
  rename for neutrality (`HexCell` instead of `Hex`
  perhaps, since `Hex` is now a coord struct in
  `hex_grid`).
- Lift paint verbs (`map_paint_material`, `set_height`,
  `place_item`, `set_wall`, etc.).
- Lift slope-shaping helpers.
- Lift JSON serialisation.  **Watch the loft JSON-cast
  bug** (≥8 fields with a `vector<Struct>` hangs the cast
  per dryopea's QUESTIONS_FOR_LOFT).  May need to use
  field-by-field serialisation instead of `:j` formatter
  until the loft bug ships.
- Adapt moros to consume; confirm moros tests green.
- Have dryopea consume; convert dryopea's `painted.loft`
  to use `hex_map::Map` instead of `PaintedWorld`.
- Bug-hunt at dryopea integration.
- Decide palette / item / wall meaning is per-game (not in
  this package) — file follow-up issues for any moros code
  that hard-codes them.

**Deliverable:** `lib/hex_map/` package; moros consuming;
dryopea consuming; multi-layer painting works end-to-end.

**Effort:** MH (substantial extraction; lots of boundary
work).

### Phase L3 — `hex_render` extraction

**Goal:** mesh emitters + 3D camera.

**Activities:**

- Create `lib/hex_render/` package.
- Lift mesh emitters from moros_render + the standalone
  `lib/wall.loft` (folding the latter into the new
  package).
- Lift the RenderCamera + orbit/zoom/pan/follow.
- Lift the `emit_thick_flat_wall` + `emit_thick_curved_wall`
  + stair family.
- Per-game material → colour mapping moves to a callback
  parameter.
- Adapt moros + dryopea + plan 02 (dryopea solver-validation
  viewer) consumers.
- Bug-hunt.

**Deliverable:** `lib/hex_render/` package; moros + dryopea
both consuming; plan 02 unblocked.

**Effort:** M-MH (large code volume but cleaner boundary
than L2).

### Phase L4 — `hex_stencil` extraction

**Goal:** stencil format + stamp + save/load.

**Activities:**

- Create `lib/hex_stencil/` package.
- Lift `StencilHex` / `StencilDef` / `stencil_stamp` /
  `stencil_save` / `stencil_to_json` / `stencil_from_json`
  from moros_editor.
- Lift the built-in stencils (`stencil_house_small`,
  `stencil_flat`, `stencil_spiral_stair`) — possibly move
  these to a game-side example library since they're
  moros-flavoured.
- Adapt moros + dryopea (plan 06 S2 consumes this directly).
- Bug-hunt — especially around the rotation logic when
  dryopea's hex orientation conventions differ from
  moros's.

**Deliverable:** `lib/hex_stencil/` package; dryopea plan
06 S2 unblocked.

**Effort:** M (medium extraction; rotation logic is the
risk area).

### Phase L5 — `hex_editor` extraction

**Goal:** editor tools + undo + UI.  Largest API-design
surface.

**Activities:**

- Create `lib/hex_editor/` package.
- Lift UndoStack + paint-with-undo family from
  moros_editor.
- Lift editor tools (brush, line, area, stencil-stamp,
  picker, eraser) from moros_sim/tools.loft + moros_editor.
- **Design the `GameHooks` contract** — what callbacks
  does the editor need from the game?  Material count,
  item count, swatch rendering, custom overlay rendering,
  custom hotkey bindings.  Per § Open questions.
- Lift relevant UI pieces from moros_ui (panel, widgets,
  font, editor_panel).  Generalise — moros_ui's panels
  are moros-flavoured.
- Adapt moros + dryopea (plan 06 S5 / hex_editor adoption).
- Bug-hunt — UI work tends to reveal mouse-event handling
  edge cases.

**Deliverable:** `lib/hex_editor/` package; dryopea's
`main.loft` simplified to use the shared editor +
dryopea-specific GameHooks.

**Effort:** H (largest single phase; per-game hook design
adds risk).

### Phase L6 — `hex_entity` greenfield

**Goal:** baked-mesh runtime + composition + pivots.

**Activities:**

- Create `lib/hex_entity/` package — mostly new code.
- Implement the stencil bake-to-static-mesh path.
- Implement mesh composition (parent + child meshes with
  named pivots).
- Implement entity runtime (spawn, tick, render).
- Define MountBehaviour enum + the simple behaviours
  (Fixed, TrackTarget, Oscillate, PlayerControlled).
- Worked example: a baked tower (base) + rotating top
  (child) tracking a moving target.
- Worked example: a baked robot (chassis) + sensor head
  (child) oscillating.
- dryopea plan 06 S3 consumes this.

**Deliverable:** `lib/hex_entity/` package; first non-
magenta-cuboid entity appears in dryopea play.

**Effort:** MH (greenfield but well-scoped; the test of
"does the worked example look right" anchors it).

### Phase L7 — Onboarding documentation + indie examples

**Goal:** the substrate is **usable without reading moros
or dryopea source**.

**Activities:**

- Write a tutorial under `doc/claude/UNIVERSAL_EDITOR.md`
  (or similar) covering:
  - Vision + audience model.
  - Library architecture diagram.
  - "How to build a hex-world game in 200 lines" walk-through.
  - Per-game `GameHooks` registration walk-through.
  - Palette / item registry registration.
- Build a worked example — `lib/hex_world_example/` —
  showing the substrate at minimum-consumer scale.  Maybe
  a "demo game" with 1 unit kind + 1 tower kind + 1
  enemy kind + a 5×5 map.  Ships alongside the substrate;
  shows indie devs what a minimal game looks like.
- Update `loft/ROADMAP.md` if the substrate becomes a
  named pillar of the suite.
- Update `loft/CHANGELOG.md` for the public release.

**Deliverable:** documentation + worked example; the
strike-path indie audience can pick the substrate up
without prior context.

**Effort:** M (writing + worked-example development).

## Moros backward-compatibility

**Hard constraint: moros must work at every phase boundary.**

The extraction is sliced exactly so moros consumes the new
package immediately after each phase ships.  If moros
breaks during extraction, the phase did too much in one
step.

### Coordination mechanisms

- **Each phase's PR pair** — one PR in loft (create the
  shared package + adapt moros to consume), one PR in moros
  (if moros lives in a separate repo) to confirm green
  tests on the new dependency.
- **Moros tests as the regression net** — they run on every
  loft change that touches a moros-consumed package.  CI
  config decides this.
- **Per-game hooks land progressively** — moros doesn't
  have to register hooks for things it doesn't use.  The
  hook surface starts at minimum-viable (the bits dryopea
  needs) and grows organically.

### What's in scope for "moros works"

- Moros's existing tests pass.
- Moros's saved maps load.
- Moros's editor opens, paints, undoes, stencils, renders
  3D, JSON-saves.
- Moros's runtime renders + ticks + responds to input.

### What's NOT in scope

- Moros's UI looking *unchanged* — moros's UI may visually
  shift slightly when it consumes the generalised
  `hex_editor` UI primitives.  Functionality preserved;
  cosmetic delta acceptable.
- Moros's exact package layout in its own repo — moros's
  internal organisation may need adjustment to consume the
  new packages cleanly.  That's moros's PR to make.

## Design principles

### Generic over specific

**The shared package is opaque about game-specific
semantics.**  It moves cells, holds materials, places items,
serialises JSON.  It does NOT know what material 5 means.

The right test: a third game built tomorrow on the same
substrate must NOT inherit moros's or dryopea's specific
choices.  If `hex_map` has a `is_water(material) ->
boolean` helper, that's wrong — water is a game-specific
concept.

### Per-game hooks, not per-game subclasses

**The substrate exposes callbacks the game provides; it
doesn't expose a base class the game extends.**

Why: loft doesn't have classical inheritance; the natural
extensibility surface is fn-typed parameters.  A consumer
constructs a `GameHooks { palette_swatch: my_swatch_fn,
... }` and passes it where needed.

Benefit: each game's hooks are colocated with the game's
state; no diamond-inheritance ambiguity; no override
discovery problem.

### Opaque integer IDs

**Materials, items, walls, NPC kinds all flow through the
substrate as `integer`.**

The substrate doesn't know what material 5 is.  It serialises
"5" to JSON; the game decodes "5" via its palette table on
load.  This decouples the substrate from any game's specific
encoding.

Trade-off: a save file from moros loaded by dryopea would
yield nonsense (material 5 means different things).  This
is intentional — saves are per-game, not per-substrate.
The substrate provides JSON shape; the game owns meaning.

### Multi-layer is first-class, not opt-in

**The substrate has `cy: integer` in every cell coordinate
from L2 onward.**

Why: retrofit cost is high.  Better to have games that
don't use it pass `cy = 0` everywhere than to bolt it on
later.  Moros uses it; dryopea will use it (plan 06 S1);
indie consumers will use it for any non-flat-ground game.

### Tests live with the package

**Each shared package ships with the moros tests that
exercise it**, lifted into the new package's test directory
where they apply.

When dryopea integrates and finds bugs, **new tests land in
the shared package**, driven by dryopea's case.  Moros gets
those new tests for free.

The accumulation of tests across consumers is one of the
big wins of extraction.

## Test discipline

### Per-package test layout

```
lib/<name>/
  src/
  tests/
    <name>.loft        # core unit tests
    <name>_moros.loft  # behaviours moros specifically relies on
    <name>_dryopea.loft # behaviours dryopea specifically relies on
    fixtures/          # shared test data
```

The split by-consumer is useful for triage: if
`<name>_dryopea.loft` regresses after a refactor, the bug
is in code that handles dryopea's specific patterns.

### Integration tests live in consumers

Each consuming package (moros, dryopea) keeps its own
end-to-end tests in its own repo / test directory.  Those
exercise the substrate at the integration layer; the shared
package's tests stay focused on the substrate.

### Coverage discipline

Aim for:

- **Smoke coverage** (the consumer can call the public API
  without crashing) — table-stakes.
- **Round-trip coverage** for JSON + stencil save/load —
  what goes in comes out unchanged.
- **Multi-consumer divergence coverage** — when moros's
  pattern and dryopea's pattern diverge on a behaviour,
  both branches have tests.

## Open questions

| # | Question | Resolution path |
|---|---|---|
| O1 | **Package naming** — `hex_*` vs `buildkit_*` vs `world_*` vs other.  `hex_*` is descriptive but constrains to hex grids; `buildkit_*` signals "general-purpose for hex-world games"; `world_*` collides with the existing `lib/world` package. | L0 decision; lean `hex_*` for descriptiveness. |
| O2 | **`hex_grid` and `hex_map` separation** — could fold into one package.  Pro: simpler; Con: leaf-dep purity gone. | L0 decision. |
| O3 | **Built-in stencils** (`stencil_house_small` etc.) — keep in `hex_stencil` as starter content, or move to a `hex_stencil_examples` aux package, or pure moros-side? | L4 decision. |
| O4 | **UI generalisation** — `moros_ui` has panel + widgets + font + editor_panel.  Some are general; some are moros-specific.  Granularity of `hex_editor`'s UI dependency? | L5 design. |
| O5 | **Per-game hook surface** — how big does `GameHooks` get?  Minimum-viable is small (4-5 callbacks); a too-rich one becomes a maintenance burden. | L5 design; iterate via dryopea's integration. |
| O6 | **Where does `lib/world` (legacy) fit?**  Voxel-shaped storage.  Currently consumed by something we haven't audited. | L0 audit. |
| O7 | **Save-format versioning** — JSON schemas evolve.  How does `hex_map` handle older saves? | L2 design; lean "include version field; refuse with error if too old; accept if forward-compat fields are unknown" per loft's @P366 lenient-ignore precedent. |
| O8 | **Lib_plans/12 coordination** — packages extracted by this plan get published via the registry per plan 12's process.  Timing? | Track in plan 12's status; this plan's packages join as they ship. |
| O9 | **Naming overlap with `lib/wall.loft`** — single-file legacy.  Fold into `hex_render` or `hex_map`? | L3 absorbs lib/wall.loft. |
| O10 | **Insect / organic units** — dryopea plan 06 S4 needs joint / leg movement.  Lives in `hex_jointed` or expands `hex_entity`? | Defer; when L6 ships and the dryopea S4 trigger fires. |

## Why this plan matters

- **It unlocks dryopea plan 06.**  Plan 06's S1 and S2
  phases are explicitly "port what moros has."  Without
  this extraction, plan 06 either reimplements (waste of
  effort + a fork in maintenance) or copy-pastes
  moros code into dryopea (worst of both worlds).
- **It hardens moros's substrate.**  Moros's code is
  rough-but-tested; dryopea integration is the second
  pass that finds the gaps moros never hit.  Moros benefits
  passively when fixes flow back.
- **It unlocks the strike-path audience.**  Per dryopea
  plan 06 § Who this serves, an indie dev shipping a
  hex-world game without an art team is a real audience.
  That audience needs `lib/hex_*` they can pick up without
  understanding moros or dryopea.  This plan provides it.
- **It positions the loft suite as a hex-world game
  engine.**  Not "loft + a game called moros that has nice
  libraries" but "the loft suite, which includes a
  universal hex-world editor and content pipeline."  The
  framing change is significant — it means a future
  github visitor lands on a *toolkit*, not on a *game with
  borrowable parts*.
- **It compounds.**  Once the substrate is in place, each
  new consumer (third game, fourth game, etc.) doesn't
  trigger a fresh round of "port what we had."  They pick
  up the libraries and ship.  The cost of new hex-world
  games goes from "rebuild a third of the engine" to
  "register palette + hooks + start authoring."

## See also

- [`README.md`](README.md) — status + forward path
- [`../../12-library-extraction/`](../../12-library-extraction/README.md) —
  monorepo-to-external extraction process (sibling arc)
- [`../../../ROADMAP.md`](../../../ROADMAP.md) — loft master
  roadmap
- [`../../../PACKAGES.md`](../../../PACKAGES.md) — package format
- [`../../../PKG_REGISTRY.md`](../../../PKG_REGISTRY.md) — registry
- Dryopea's plan 06 README — second-consumer pull
  motivating this extraction
- Dryopea's
  [`docs/SETTING.md`](https://github.com/jjstwerff/dryopea/blob/main/docs/SETTING.md)
  + [`docs/DESIGN.md`](https://github.com/jjstwerff/dryopea/blob/main/docs/DESIGN.md)
  — example of a per-game layer that sits on top of the
  substrate
