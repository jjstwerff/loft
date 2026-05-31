<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# LAVITION.md — engine brand + architecture + library model

[lavition](https://github.com/lavition/lavition) is the universal editor
built on top of the loft language stack.  It edits hex-grid worlds with
walls, materials, items, and heights, supports multi-level layering,
stencil-driven authoring, and detection of round / curved structures.
This doc captures the brand layering with loft, the engine's
architecture, and the library model that organises lavition's plugins.

For the design vision (12-direction buildings, 24-direction walls,
curve detection, vertical layers, dual-use stencils) see the
[`lavition/lavition` README](https://github.com/lavition/lavition).

## Brand layering — loft vs lavition

Two distinct brand identities, by tier:

| Tier | Brand | Role | GitHub org |
|---|---|---|---|
| Language | **loft** | embedded scripting language + interpreter + standard libraries | [`loft-lang`](https://github.com/loft-lang) |
| Engine | **lavition** | universal editor product built on loft | [`lavition`](https://github.com/lavition) |

**Why two brands.** loft is the technical layer — a language with its own
identity, registry, and ecosystem of libraries.  lavition is the
user-facing product — the editor a game developer downloads, the brand
indie devs discover via Google.  Keeping them separate lets each evolve
on its own schedule without collateral churn.

**A game's relationship to both:**

- *Runtime:* depends on `loft` + the `loft-libs-*` packages it imports.
  No lavition dependency at runtime.  A built game ships its own binary
  containing the loft runtime + the libraries it links.
- *Authoring time:* uses lavition's editor to compose worlds, place items,
  edit walls.  The editor writes data files the game then reads at
  runtime via the same `loft-libs-world` primitives.

So games are *runnable without lavition*, *authorable with it*.

## Naming principle — brand in metadata, symbols stay descriptive

The brand belongs where users find the product: org URLs, repo names,
package homepages, plugin manifests, docs hero, marketing.  The brand
does NOT belong in the names users read inside script code.

| Place | Pattern | Example |
|---|---|---|
| GitHub org | brand-prefixed | `github.com/loft-lang/`, `github.com/lavition/` |
| Repo / chunk | brand-prefixed | `loft-libs-world`, `lavition/editor` |
| Crates.io crate | brand-prefixed where coupled | `lavition_core` |
| **`use X;` in `.loft` scripts** | **descriptive only — NO brand prefix** | `use hexworld;`, `use hex_walls;`, `use terrain;` |
| Plugin manifests (lavition-internal) | short descriptive | `[plugin] name = "hex_editor"` |

Same model as cargo: `use serde;` is descriptive; the brand
(`serde-rs/serde`) lives in the repo URL.  A cold reader encountering
`use hexworld;` knows what the library is from the name alone; no need
to also know what engine the script targets.

## Library model

Lavition's library landscape splits into four kinds:

### 1. Data primitives (loft ecosystem; consumed at runtime by games)

Live in `loft-lang/loft-libs-*`.  Bare descriptive names.  Games
import these and ship them in their runtime binary.

**Finalized names — the `hex_*` family.**  Naming intent: each library
covers ONE data axis of the hex world.  The `hex_*` prefix marks the
family + leaves room for parallel `voxel_*` / `tile_*` families later
without naming collisions.

| Library | Purpose | Chunk | Status |
|---|---|---|---|
| `hexworld` | sparse 32×32 chunked hex grid + addressing + save/load | loft-libs-world (planned) | exists as `lib/world/` in monorepo; rename + extract is next-up |
| `hex_walls` | wall segment data (24 sub-hex directions) + curve detection + read APIs | loft-libs-world | currently folded into `lib/world/src/wall.loft`; split + extract after hexworld |
| `hex_terrain` | per-hex heightmap + material palette + slope rules | loft-libs-world | NEW — needs design + ship |
| `hex_items` | item-instance data (placement, orientation among 12 directions, animation state, vertical layer) | loft-libs-world | NEW — needs design + ship |
| `particles` | ribbon trails + point-burst particles | loft-libs-world | planned slot ([`lib_plans/future/27-particles/`](lib_plans/future/27-particles/)) |
| `physics_2body` | rigid-body collision + integrator | loft-libs-world | planned slot ([`lib_plans/future/26-physics-2body/`](lib_plans/future/26-physics-2body/)) |
| `graphics` | 2D canvas + 3D rendering + OpenGL bindings | loft-libs-graphics | shipped 0.1.0 |
| `imaging` | PNG load/save + pixel manipulation | loft-libs-graphics | shipped 0.1.0 |
| `shapes` / `gridmesh` | geometry primitives | loft-libs-graphics | shipped |

These libraries know nothing about lavition.  They're equally usable by
a pure-loft CLI tool, a game built on lavition, or a third-party engine
that wants the same data primitives.

### 2. Engine core (lavition org; required by every plugin)

Live in `lavition/<repo>`.  Brand-prefixed crate names because they only
make sense inside lavition.

| Crate | Purpose |
|---|---|
| `lavition_core` | plugin host + UI framework + input router + history/undo stack + selection abstraction + viewport |
| `lavition_render` | editor viewport rendering (grid overlays, selection halos, stencil previews) — builds on `graphics` |
| `lavition_asset_pipeline` | import / export / file watching / hot reload |
| `lavition_stencil` | shared stencil format (capture, serialise, apply at building scale or item scale) |

### 3. Editor plugins (lavition org; pluggable, one per data type)

Live in `lavition/<plugin-name>` repos.  Each plugin knows how to author
ONE data primitive.  Pluggable so games extend with their own data
types without modifying core.

| Plugin | Authors which data | Tools |
|---|---|---|
| `lavition_hex_editor` | `hex_walls` | paint segment, erase, snap to corners, curve detection toggle |
| `lavition_terrain_paint` | `terrain` heightmap + materials | brush, raise/lower, material palette, smooth |
| `lavition_item_placer` | `items` | place, rotate, animate, smaller-scale stencil-as-item |
| `lavition_layer_panel` | vertical layers | basement / ground / 2nd / roof toggle, free-placement layer creation |
| `lavition_stencil_library` | reusable stencils | save current selection as stencil, browse + stamp |
| `lavition_curve_detector` | derived view of `hex_walls` | smooth-render preview, threshold tuning |

Plugin contract (sketch):

```toml
# lavition.toml in each plugin repo
[plugin]
name        = "hex_editor"
edits       = ["hex_walls"]
loft_deps   = ["hex_walls"]
core_deps   = ["lavition_core"]

[tools]
paint = "src/tool_paint.rs::PaintTool"
erase = "src/tool_erase.rs::EraseTool"

[widgets]
toolbar     = "src/widget_toolbar.rs::Toolbar"
layer_panel = "src/widget_layer.rs::LayerPanel"
```

### 4. Engine binary (lavition org; the product users download)

Lives in `lavition/lavition` (the meta-repo currently holds the design
intent; the binary will land here or in a sibling `lavition/engine`
repo when the core stabilises).

The binary:
- Bundles `lavition_core` + `lavition_render` + `lavition_asset_pipeline`.
- Loads plugins from a configurable plugin dir + the bundled defaults.
- Embeds `loft` as the scripting layer for plugin scripts + project
  automation.
- Ships per-platform installers + a portable archive.

### 5. Game-internal libraries (separate user orgs; NOT in either ecosystem)

Live in each game's repo under that game's owner.  Project-prefixed
descriptive names.  These are not extracted because they encode
game-specific data shapes (moros's faction model, dryopea's mission
graph) that have no value to other games.

| Pattern | Example |
|---|---|
| `<project>_<thing>` | `moros_map`, `moros_sim`, `moros_render`, `dryopea_*` (planned) |

## Hex-family libraries — per-library narrative

Sketches of each `hex_*` data library covering: what data it owns,
which lavition plugin authors it, which game roles consume it, and an
API outline.  These are the source material for the eventual
`lavition.io/docs/<lib>/` pages (which own the "lavition + generic-term"
search space — see [Discoverability](#discoverability--the-practical-reason-for-brand-visibility)).

### `hexworld` — the addressing primitive

**Owns:** the hex grid itself.  Sparse 32×32 chunked storage so a 5000-hex
world is mostly empty chunks; load/save round-trips the whole structure.

**Lavition pairing:** loaded by `lavition_core` as the canonical world
coordinate system; every plugin reads/writes against `hexworld`
addresses (`HexId { col, row }` or chunk-relative
`(ChunkId, sub_col, sub_row)`).

**Game roles:**
- Runtime address space for the game's world data.
- Save/load primitive (the engine reads `world_save` / `world_load`
  state from this library).
- Iteration helper (`hexes_in_chunk`, `neighbors_of(hex)`,
  `bbox_of_chunks(region)`).

**API sketch:**
```loft
pub struct HexId { col: integer not null, row: integer not null }
pub struct ChunkId { x: integer not null, y: integer not null }
pub struct ChunkedWorld { /* sparse storage */ }

pub fn world_new(name: text) -> ChunkedWorld
pub fn world_save(w: const ChunkedWorld, path: text) -> boolean
pub fn world_load(path: text) -> ChunkedWorld
pub fn neighbors_of(h: HexId) -> vector<HexId>     // 6 neighbors
pub fn chunk_of(h: HexId) -> ChunkId
pub fn hexes_in_chunk(c: ChunkId) -> iterator<HexId>
```

### `hex_walls` — linear features at sub-hex resolution

**Owns:** wall / cliff / road segments between or along hex edges.  Each
hex edge has 4 sub-segments per side (so 6 edges × 4 sub-segments = 24
orientations per hex).  Segments carry material, height range, and
vertical layer.

**Lavition pairing:** authored by the `lavition_hex_editor` plugin.
The plugin's curve-detection pass reads `hex_walls` data and renders
chains of straight segments as smooth shapes for preview + final render.

**Game roles:**
- Collision data for the physics layer (`physics_2body` reads
  `hex_walls` to know what's solid).
- Render input for the game's wall renderer (read-only iteration over
  visible segments).
- Pathfinding input (segments block movement between hexes).

**API sketch:**
```loft
pub struct WallSegment {
  hex: hexworld::HexId not null,
  orientation: integer not null,   // 0..23
  layer: integer not null,         // basement/ground/2nd/roof/free
  material: integer not null,
  height_min: float,
  height_max: float
}

pub fn walls_at(w: const ChunkedWorld, h: hexworld::HexId) -> vector<WallSegment>
pub fn walls_between(w: const ChunkedWorld, a: hexworld::HexId, b: hexworld::HexId) -> vector<WallSegment>
pub fn add_wall(w: ChunkedWorld, seg: WallSegment)
pub fn remove_wall(w: ChunkedWorld, hex: hexworld::HexId, orientation: integer)

// Curve detection — derived view, no separate storage.
pub fn detect_curves(w: const ChunkedWorld, layer: integer) -> vector<CurveChain>
```

### `hex_terrain` — heights + materials per hex

**Owns:** per-hex height + material palette.  Slope rules between
adjacent hexes so the renderer knows whether to draw a cliff (handled
as walls) or a smooth ramp.  Material palette indexed by integer so
storage per hex is one byte for material + one for height.

**Lavition pairing:** authored by the `lavition_terrain_paint` plugin
(brush, raise/lower, palette swap, smoothing).

**Game roles:**
- Surface render (the game's ground renderer reads height + material
  per hex and meshes the visible region).
- Movement cost (steeper slopes = more cost; material affects speed).
- Spawn site selection (palette type → spawnable items / NPCs).

**API sketch:**
```loft
pub struct HexTerrain {
  height: integer,                     // 0..255, mapped to world Z by game
  material: integer not null           // index into MaterialPalette
}

pub struct MaterialPalette {
  materials: vector<MaterialDef> not null
}

pub fn terrain_at(w: const ChunkedWorld, h: hexworld::HexId) -> HexTerrain
pub fn set_terrain(w: ChunkedWorld, h: hexworld::HexId, t: HexTerrain)
pub fn slope_between(w: const ChunkedWorld, a: hexworld::HexId, b: hexworld::HexId) -> float
pub fn palette_of(w: const ChunkedWorld) -> MaterialPalette
```

### `hex_items` — placed instances

**Owns:** item instances anchored at one of the 12 building-placement
directions per hex.  Each item carries type, orientation, optional
animation state, and the vertical layer it lives on (including
"free placement" layers that don't participate in collision).

**Lavition pairing:** authored by the `lavition_item_placer` plugin.
Stencils placed as items (smaller render scale) write into this library.

**Game roles:**
- World population at runtime (the game iterates `items_in_region` per
  frame to render + tick animations).
- Interaction targets (item type → interaction script).
- Save/load (items round-trip with the rest of the world via
  `hexworld::world_save`).

**API sketch:**
```loft
pub struct HexItem {
  hex: hexworld::HexId not null,
  direction: integer not null,        // 0..11 (12-direction placement)
  item_type: integer not null,
  orientation: integer,                // optional rotation around vertical axis
  layer: integer not null,             // basement/ground/2nd/roof/free
  anim_state: integer                  // optional animation track + frame
}

pub fn items_at(w: const ChunkedWorld, h: hexworld::HexId) -> vector<HexItem>
pub fn items_in_region(w: const ChunkedWorld, region: ChunkRegion) -> iterator<HexItem>
pub fn place_item(w: ChunkedWorld, item: HexItem)
pub fn remove_item(w: ChunkedWorld, hex: hexworld::HexId, direction: integer)
pub fn tick_animations(w: ChunkedWorld, dt_ms: integer)
```

### Cross-cutting: `physics_2body` and `particles`

Not hex-prefixed because they're general-purpose primitives consumable
by any spatial library (`hex_walls`, future `voxel_walls`, 2D
collision games).  See their plan slots:
[`lib_plans/future/26-physics-2body/`](lib_plans/future/26-physics-2body/),
[`lib_plans/future/27-particles/`](lib_plans/future/27-particles/).

## Next library work — execution order

The `loft-libs-world` chunk is the next chunk to ship.  Order is
foundation-first: hexworld blocks everything downstream because every
other lib references its addressing.

Each sub-step is a self-contained Stage A → Stage B mini-cycle (same
pattern Phase 5b just ran: tarball + chunk-repo + registry PR +
consumer migration + monorepo cleanup).

| # | Step | Effort | Depends on |
|---|---|---|---|
| W.1 | Rename `lib/world` → `lib/hexworld` in monorepo + update consumer loft.tomls + path-deps + `src/wasm.rs` `include_str!` paths.  Pre-extraction churn, all internal. | S | — |
| W.2 | Extract `hexworld` Stage A → Stage B (publish to `loft-libs-world/hexworld`, swap monorepo `lib/moros_*` consumers to registry deps, remove `lib/hexworld/`). | M | W.1 + lavition design clarification (data shape stable enough to ship 0.1.0) |
| W.3 | Split `hex_walls` out of `lib/hexworld/src/wall.loft` into its own monorepo library `lib/hex_walls/`.  Defines the API boundary between the addressing primitive and the wall data. | M | W.2 |
| W.4 | Extract `hex_walls` Stage A → Stage B. | M | W.3 + curve-detection pass design |
| W.5 | Design + implement `hex_terrain` as a new monorepo library `lib/hex_terrain/` (heightmap + materials).  Migrate moros's existing terrain code if any. | MH | W.2 (uses hexworld addressing) |
| W.6 | Extract `hex_terrain` Stage A → Stage B. | M | W.5 |
| W.7 | Design + implement `hex_items` as a new monorepo library `lib/hex_items/`. | MH | W.2 + W.3 (item placement uses 12 directions, layer model shared with hex_walls) |
| W.8 | Extract `hex_items` Stage A → Stage B. | M | W.7 |
| W.9 | Ship `physics_2body` (from existing plan slot) to the same chunk.  Reads hex_walls for collision geometry. | MH | W.4, plan-26 design |
| W.10 | Ship `particles` (from existing plan slot) to the same chunk.  Independent. | M | plan-27 design |

**Total:** ~10 sub-phases, each self-contained.  Realistic shipping
cadence: 1-2 sub-phases per session, so the full `loft-libs-world`
chunk lands across 5-10 sessions.  After W.10, lavition can start
implementing its plugins against the stable data layer.

**Branch model:** continue the established pattern — one cross-theme
branch (`doc-updates` or successor) accumulates the work; a PR opens
per chunk milestone (e.g. when hexworld + hex_walls both ship → PR
"`loft-libs-world` foundation"; when terrain + items ship → next PR;
etc.).  The 6-PR-per-game-data-lib model would be too much PR overhead.

## Discoverability — the practical reason for brand visibility

The brand isn't visible in metadata for marketing reasons.  It's visible
because **descriptive symbol names are not searchable on their own.**  A
user who encounters `use hexworld;` in a script and googles "hexworld"
gets Roblox games, Civilization map packs, and 2D puzzle clones — not
our library.  This is the same problem Python's `requests` has:
"requests documentation" returns garbage; the canonical query is
"python requests".

We accept the tradeoff (script readability over search ergonomics) but
mitigate the cost on multiple fronts:

### 1. Names unique enough that search has a fighting chance

`hexworld` > `world` (less collision).  `hex_walls` > `walls`.
`terrain` is borderline; `particles` is OK; `physics_2body` is unique.
The bad offenders in the current library set are short generic words —
`shapes`, `graphics`, `imaging`, `web`, `server`, `math`, `mesh`,
`scene`.  For these the SEO collision is severe; consider a soft
rebrand-via-prefix at the next major-version boundary (see
[§ Long-term rename candidates](#long-term-rename-candidates) below).

### 2. The package registry is the authoritative discovery layer

`loft-lang/registry` indexes every library with description + homepage
+ category.  Same role crates.io plays for Rust — a user who doesn't
remember a library's name browses the registry by category.

- `loft-lang.org/libraries/` (catalog HTML, planned) — human-facing
  index.
- [`doc/library-catalog.md`](library-catalog.md) (already shipped via
  Phase 6.15's `scripts/gen_library_catalog.py`) — markdown version.
- `loft search <term>` (planned, on the registry MVP roadmap) —
  CLI-side fuzzy search.

A canonical query like "loft library catalog" or "loft package
registry" should rank high; the catalog itself then routes users to the
specific library's docs.

### 3. In-package branding loud (once they find it)

Every published library's `README.md` opens with the ecosystem context:

> # hexworld — chunked hex grid for the loft language
>
> Part of the [loft](https://github.com/jjstwerff/loft) ecosystem;
> works standalone and with the [lavition](https://github.com/lavition)
> editor.

Once a user lands on the package (via any path — search, registry,
catalog, blog post), the brand is unmistakable.  No requirement that
the symbol carry the brand.

### 4. IDE / editor tooltips supply context the symbol omits

When lavition's editor (or any LSP-aware loft IDE plugin) hovers over
`use hexworld;`, the tooltip shows:

```
hexworld v0.1.0
chunked hex grid + addressing + save/load
loft-lang/loft-libs-world
docs:  https://loft-lang.org/libraries/hexworld
```

The symbol stays bare in source; the IDE provides the brand context.
Same pattern IntelliJ uses for bare Java imports (hover on `List` →
"java.util.List from rt.jar").

### 5. Lavition's docs own the "lavition + generic-term" search space

The discoverability target isn't "googling `hexworld` finds us" (it
won't, against Roblox + Civ + a dozen other things).  The target is
**"googling `lavition world` or `lavition wall` lands on our docs
directly."**  That query is achievable with normal SEO because
"lavition" is unique on Google — combine it with any common word and
the result space is small enough we can own it.

Concrete structure for the lavition docs site:

```
lavition.io/docs/                     overview
lavition.io/docs/world                ← hexworld + lavition editor integration
lavition.io/docs/wall                 ← hex_walls + hex_editor plugin
lavition.io/docs/terrain              ← terrain + terrain_paint plugin
lavition.io/docs/items                ← items + item_placer plugin
lavition.io/docs/stencils             ← stencil format + library + stamping
lavition.io/docs/layers               ← vertical-layer model
```

Each page covers the editor narrative + the underlying loft library
(`hexworld`, `hex_walls`, etc.) and links out to the library's own
docs (`loft-lang.org/libraries/hexworld/`).  The brand is in the URL +
page title + meta description; the library symbol stays bare in code.

So a user can find the right documentation via three paths:

- **"lavition world" / "lavition wall"** → directly hits the
  brand-disambiguated lavition docs page.  Brand goes in the search,
  not the symbol.
- **"loft library catalog"** → hits the registry / catalog page,
  browses by category to find `hexworld`.
- **Hover in IDE** → tooltip resolves the bare symbol to its docs URL.

### 6. Cross-linking between loft + lavition + game project sites

- `lavition`'s homepage: "Built on the [loft](https://github.com/jjstwerff/loft) language."
- `loft`'s homepage: "Primary editor: [lavition](https://github.com/lavition).  Also usable from CLI, web, or any other host."
- Each game's README points at both.

So a user landing on any one site can navigate to the others within
one click.  Brand context discoverable through the link graph, not
required in source.

### Long-term rename candidates

For existing generic-name libraries that have severe SEO collision,
consider soft rebrand at the next major-version boundary.  Not urgent;
not all at once.

| Current | Better candidates | Notes |
|---|---|---|
| `graphics` | `loft_graphics`, `lgfx` | "graphics" alone is uncoupled from any specific ecosystem |
| `math` | `loft_math`, `linalg`, `vecmath` | rename to a less collision-prone domain term |
| `mesh` | `loft_mesh`, `geomesh` | overlaps with `mesh.js`, `Mesh` (many engines), Apple's Mesh framework |
| `scene` | `loft_scene`, `scenegraph` | overlaps with countless "scene" libraries |
| `shapes` | `loft_shapes`, `geo_shapes` | borderline; manageable with good SEO |
| `web` | `loft_web` | extremely generic; current size of the lib is small enough that a rename is cheap |
| `server` | `loft_server` | same |

These are not action items today — current 0.x versions stay as-is.
But at 1.0 it's worth a single rename PR per affected library, with a
deprecation period publishing both names for one minor release.

### What this section is NOT

This isn't a recommendation to brand-prefix EVERY symbol.  Unique-enough
names (`hexworld`, `hex_walls`, `terrain`, `particles`, `physics_2body`,
`gridmesh`, `imaging`, `arguments`, `crypto`, `random`) stay bare —
they're already searchable enough that the mitigations above carry the
discoverability load.  The prefix is only for short generic words
where SEO is genuinely broken without it.

## Migration / rename pending

- `lib/world` (currently in monorepo, planned for `loft-libs-world`
  extraction): rename to `hexworld` to disambiguate from voxel / tile /
  BSP "world" expectations.
- `walls` data (currently folded into `lib/world/`): split out as
  `hex_walls` matching the `hexworld` pairing.
- `lib_plans/future/24-universal-editor/` plan: keep the design content,
  retitle to reference lavition as the engine the plan delivers.
- 6 residual `lav` / `Lavition` references in the loft tree (pre-rename
  leftovers): keep — they reference the original codename for the
  language, which is now reused as the engine brand.  No harm, no
  cleanup needed.

## Anti-renames

- **Don't rename `loft` to anything else.**  It's a real shipped artifact
  with published packages, a registry, and consumer commitments.  loft
  stays loft; lavition is built *on* loft.
- **Don't add brand prefixes to data libraries.**  `use hexworld;` is
  cleaner and more portable than `use lavition_hexworld;` (which would
  falsely imply engine coupling).

## See also

- [`lavition/lavition`](https://github.com/lavition/lavition) — engine
  meta-repo with design vision + roadmap pointer.
- [`lib_plans/future/24-universal-editor/`](lib_plans/future/24-universal-editor/)
  — the original design plan for the universal editor (predates the
  lavition brand; design content still authoritative).
- [`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction/)
  — the multi-phase plan for getting libraries into `loft-lang/loft-libs-*`
  (the substrate lavition consumes).
- [`PACKAGES.md`](PACKAGES.md) + [`PKG_REGISTRY.md`](PKG_REGISTRY.md)
  — how loft libraries are packaged + distributed.
