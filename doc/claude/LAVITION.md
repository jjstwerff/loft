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

| Library | Purpose | Chunk |
|---|---|---|
| `hexworld` | sparse 32×32 chunked hex grid + addressing + save/load | loft-libs-world (planned) |
| `hex_walls` | wall segment data (24 sub-hex directions) + read APIs | loft-libs-world |
| `terrain` | heightmap + material palette | loft-libs-world |
| `items` | item-instance data (position, type, rotation, animation state) | loft-libs-world |
| `particles` | ribbon trails + point-burst particles | loft-libs-world |
| `physics_2body` | rigid-body collision + integrator | loft-libs-world |
| `graphics` | 2D canvas + 3D rendering + OpenGL bindings | loft-libs-graphics |
| `imaging` | PNG load/save + pixel manipulation | loft-libs-graphics |
| `shapes` / `gridmesh` | geometry primitives | loft-libs-graphics |

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
