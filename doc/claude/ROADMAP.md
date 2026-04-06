---
render_with_liquid: false
---
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and fix paths: [PLANNING.md](PLANNING.md).

**Project goal:** browser games that anyone can play via a shared link.
Native OpenGL is supported for desktop enthusiasts.  Server/multiplayer
comes after the single-player browser experience works.

**Effort:** XS = Tiny · S = Small · M = Medium · MH = Med–High · H = High · VH = Very High

**Design:** ✓ = detailed design in place · ~ = partial/outline · — = needs design

**Maintenance rule:** When an item is completed, remove it from this file entirely.
Do not keep completed items — the ROADMAP tracks only what remains to be done.
Completed work belongs in CHANGELOG.md (user-facing) and git history (implementation).

---

## 0.8.4 — Renderer + WebGL + first playable game

The 0.8.4 milestone delivers the core promise: a loft game running in a browser.

### Theme 1: Unified renderer (native works → WebGL plugs in)

| ID     | Title                                                  | E  | Design | Source                     |
|--------|--------------------------------------------------------|----|--------|----------------------------|
| R1     | `render.loft` — Renderer struct, built-in PBR shader   | M  | ✓      | RENDERER.md                |
| R2     | Shadow pass + color pass in `render.frame()`           | M  | ✓      | RENDERER.md                |
| R3     | `render.run()`, `elapsed()`, `destroy()`               | S  | ✓      | RENDERER.md                |
| R4     | Update examples 11, 19 to use renderer                 | S  | ✓      | examples/                  |

### Theme 2: WebGL backend

| ID     | Title                                                  | E  | Design | Source                     |
|--------|--------------------------------------------------------|----|--------|----------------------------|
| GL6.1  | Canvas element + WebGL2 context (web-sys + `#native`)  | M  | ✓      | WEB_EXAMPLES.md            |
| GL6.2  | Shader compile + link (WebGL2 API)                     | S  | ✓      | WEB_EXAMPLES.md            |
| GL6.3  | Buffer upload + draw + requestAnimationFrame           | S  | ✓      | WEB_EXAMPLES.md            |
| GL6.4  | Texture upload from Canvas (WebGL)                     | S  | ✓      | WEB_EXAMPLES.md            |
| GL6.5  | Shader version patching (330 core → 300 es)            | S  | ✓      | WEB_EXAMPLES.md            |
| GL6.6  | Keyboard + mouse input via DOM events                  | S  | ~      | WEB_EXAMPLES.md            |

### Theme 3: Game infrastructure

| ID     | Title                                                  | E  | Design | Source                     |
|--------|--------------------------------------------------------|----|--------|----------------------------|
| G1     | Sprite sheet loading (atlas texture + UV rect lookup)  | S  | ~      | render.loft                |
| G2     | Sprite drawing (billboarded quads in 3D or 2D overlay) | S  | ~      | render.loft                |
| G3     | Tilemap rendering (grid-based 2D, batched draw)        | M  | —      |                            |
| G4     | 2D collision detection (AABB + circle)                 | S  | —      |                            |
| G5     | Audio: sound effect playback (Web Audio + native)      | S  | —      |                            |
| G6     | Audio: background music with crossfade                 | S  | —      | G5                         |
| G7     | First playable demo game (simple, proves the pipeline) | M  | —      | R1, GL6.1, G1              |

### Theme 4: Web deployment

| ID     | Title                                                  | E  | Design | Source                     |
|--------|--------------------------------------------------------|----|--------|----------------------------|
| W1     | WASM build + `compile_and_run()` in browser            | M  | ✓      | WASM.md, WEB_IDE.md M1     |
| W1.1   | Single-file HTML export (`loft --html game.loft`)      | M  | ~      |                            |
| GAL.1  | Example gallery build script + index.html              | S  | ✓      | WEB_EXAMPLES.md            |
| GAL.2  | Per-example pages with source + live WebGL             | M  | ✓      | WEB_EXAMPLES.md            |

### Theme 5: Moros — hex RPG scene editor (first real application)

Moros is a browser-based tabletop RPG toolkit: a hex-grid scene editor and
3D renderer.  It is the first real application built on loft's graphics and
WASM stack, validating the full pipeline (loft → WASM → WebGL → browser).

Design documents:
- `../moros/doc/claude/LOFT_LIBRARIES.md` — package APIs and type specs
- `../moros/doc/claude/SCENE_EDITOR_PLAN.md` — 6-phase implementation plan
- `../moros/doc/claude/SCENE_MAP.md` — hex/chunk data format
- `../moros/doc/claude/SCENE_MAP_RENDER.md` — 3D geometry pseudocode
- `../moros/doc/claude/OPEN_ISSUES.md` — designs for all open items

#### Sprint A: Data model + 2D canvas (no loft WebGL needed)

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.1a  | `moros_map` — Hex, Chunk, Map, HexAddress structs      | S  | ✓      |               |
| MO.1b  | `moros_map` — MaterialDef, WallDef, ItemDef palettes   | S  | ✓      | MO.1a         |
| MO.1c  | `moros_map` — SpawnPoint, NpcRoutine, NpcWaypoint      | S  | ✓      | MO.1a         |
| MO.2   | `moros_map` — map_to_json/map_from_json serialization  | S  | ✓      | MO.1a         |
| MO.C1  | `scene-canvas.js` — hex coordinate math + flat render  | M  | ✓      | MO.2          |
| MO.C2  | `scene-canvas.js` — pan/zoom/hit-test                  | S  | ✓      | MO.C1         |
| MO.C3  | `scene-canvas.js` — layer rendering with opacity       | S  | ✓      | MO.C1         |

#### Sprint B: Editor tools (pure JS, no loft WASM needed)

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.E1  | `scene-editor.html` — shell page + toolbar + palettes  | M  | ✓      | MO.C1         |
| MO.E2  | `scene-editor.js` — Select + Paint + Height tools      | M  | ✓      | MO.E1         |
| MO.E3  | `scene-editor.js` — Wall + Item placement tools        | S  | ✓      | MO.E2         |
| MO.E4  | `scene-editor.js` — undo/redo stack (JS-side)          | S  | ✓      | MO.E2         |
| MO.E5  | `scene-editor.js` — localStorage save/load             | S  | ✓      | MO.E2         |

#### Sprint C: Loft backend (needs WASM build)

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.3   | `moros_editor` — hex paint, height, wall, item ops     | M  | ✓      | MO.1a, W1     |
| MO.4   | `moros_editor` — undo/redo stack (loft-side)           | S  | ✓      | MO.3          |
| MO.5a  | `moros_editor` — slope tool                            | S  | ✓      | MO.3          |
| MO.5b  | `moros_editor` — stencil stamping (12 orientations)    | M  | ✓      | MO.3          |
| MO.6   | `moros_editor` — spawn/waypoint management             | S  | ✓      | MO.3          |
| MO.W1  | WASM build: moros_map + moros_editor → .wasm           | S  | ✓      | MO.3, W1      |

#### Sprint D: 3D renderer (needs loft renderer + WebGL)

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.7a  | `moros_render` — hex surface geometry (flat + slope)   | M  | ✓      | MO.1a, R1     |
| MO.7b  | `moros_render` — wall slab geometry (thin + thick)     | M  | ✓      | MO.7a         |
| MO.7c  | `moros_render` — MeshBuilder batching + material sort  | S  | ✓      | MO.7a         |
| MO.8a  | `moros_render` — linear stair steps                    | S  | ✓      | MO.7a         |
| MO.8b  | `moros_render` — spiral newel + grand arc treads        | M  | ✓      | MO.8a         |
| MO.9a  | `moros_render` — camera orbit/pan/zoom                 | S  | ✓      | MO.7a, GL6.6  |
| MO.9b  | `moros_render` — hex picking (screen ray → hex addr)   | M  | ✓      | MO.9a         |
| MO.13  | Developer art — flat-shade colours + procedural swatches| S  | ✓      | MO.7a         |

#### Sprint E: Export + integration

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.10  | `moros_render` — GLB export (scene → file/base64)      | M  | ✓      | MO.7a         |
| MO.W2  | WASM build: moros_render → .wasm                       | S  | ✓      | MO.7a, GL6.1  |
| MO.12a | `scene-editor.html` — wire loft WASM to JS editor      | M  | ✓      | MO.W1, MO.E2  |
| MO.12b | `scene-editor.html` — live 3D preview panel            | M  | ✓      | MO.W2, MO.9a  |
| MO.12c | `scene-editor.html` — GLB export button                | S  | ✓      | MO.10, MO.12a |

### Remaining package/language items

| ID     | Title                                                  | E  | Design | Depends on | Source           |
|--------|--------------------------------------------------------|----|--------|------------|------------------|
| PKG.7  | Lock file (`loft.lock`) for reproducible builds        | S  | ✓      | PKG.3      | manifest.rs      |
| FFI.1  | Generic type marshaller from `#native` signature       | MH | —      | EXT.1      | native.rs        |
| FFI.2  | Generic cdylib loader — scan exports, HashMap          | S  | —      | FFI.1      | extensions.rs    |
| FFI.3  | Eliminate per-function glue in native.rs               | M  | —      | FFI.2      | native.rs        |
| FFI.4  | Docs: zero-boilerplate native function guide           | S  | —      | FFI.3      | EXTERNAL_LIBS.md |

---

## 0.9.0 — Polish + developer experience

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| P2     | REPL / interactive mode                                | M  | ✓      | PLANNING.md § P2 |
| L1     | Error recovery after token failures                    | M  | ✓      | PLANNING.md § L1 |
| W-warn | Developer warnings (Clippy-inspired)                   | M  | —      | see below        |
| AOT    | Auto-compile libraries to native shared libs           | M  | ✓      | PLANNING.md      |
| C52    | Stdlib name clash: warning + `std::` prefix            | M  | ✓      | PLANNING.md      |
| C53    | Match arms: library enums + bare variant names         | M  | ✓      | PLANNING.md      |

### W-warn — Developer warnings

| Warning | Example |
|---------|---------|
| Comparison always true/false | `x >= 0` when x is `integer not null` |
| Unnecessary parentheses | `if (x > 0) { ... }` |
| Empty loop/if body | `for x in v { }` |
| Shadowed variable in same scope | `x = 1; x = "hello"` (type change) |
| Unused import | `use lib;` but no `lib::` references |
| Identical if/else branches | `if c { x } else { x }` |
| Division by literal zero | `x / 0` |

---

## 1.0.0 — IDE + multiplayer

### Web IDE

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| W2     | Editor shell (CodeMirror 6 + Loft grammar)             | M  | ✓      | WEB_IDE.md M2    |
| W3     | Symbol navigation (go-to-def, find-usages)             | M  | ✓      | WEB_IDE.md M3    |
| W4     | Multi-file projects (IndexedDB)                        | M  | ✓      | WEB_IDE.md M4    |
| W5     | Docs & examples browser                                | M  | ✓      | WEB_IDE.md M5    |
| W6     | Export/import ZIP + PWA offline                         | M  | ✓      | WEB_IDE.md M6    |

### Multiplayer (server + client)

| ID     | Title                                                  | E  | Design | Source              |
|--------|--------------------------------------------------------|----|--------|---------------------|
| SRV.1  | Plain HTTP routing + middleware                        | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.2  | HTTPS with static PEM certificates                     | S  | ✓      | WEB_SERVER_LIB.md   |
| SRV.3  | WebSocket support                                      | S  | ✓      | WEB_SERVER_LIB.md   |
| SRV.4  | Authentication: JWT, session, API key                  | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.5  | ACME / Let's Encrypt automatic certs                   | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.6  | CORS, rate limiting, static files                      | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.G  | Game loop: ws_poll, broadcast, ConnectionRegistry      | M  | ✓      | WEB_SERVER_LIB.md   |
| GC.1   | WebSocket client + GameEnvelope protocol               | M  | ✓      | GAME_CLIENT_LIB.md  |
| GC.2   | Lobby + matchmaking                                    | S  | ✓      | GAME_CLIENT_LIB.md  |
| GC.3   | Fixed-timestep game loop                               | S  | ✓      | GAME_CLIENT_LIB.md  |
| GC.4   | Client-side prediction + reconciliation                | M  | ✓      | GAME_CLIENT_LIB.md  |
| GC.5   | WASM script loading + Ed25519 verification             | M  | ✓      | GAME_CLIENT_LIB.md  |
| GC.6   | Shared game logic + Tic-Tac-Toe demo                   | M  | ✓      | GAME_CLIENT_LIB.md  |

---

## 1.1+ — Backlog

| ID     | Title                                                  | E  | Design | Source              |
|--------|--------------------------------------------------------|----|--------|---------------------|
| C57    | Route decorator syntax (`@get`, `@post`, `@ws`)       | H  | ✓      | SERVER_FEATURES.md  |
| W1.14  | WASM Tier 2: Web Worker pool; `par()` parallelism     | VH | ✓      | WASM.md             |
| I13    | Iterator protocol (`for msg in ws` via `fn next`)     | MH | ✓      | SERVER_FEATURES.md  |
| I12    | Interfaces: factory methods (`fn zero() -> Self`)     | S  | ✓      | INTERFACES.md       |
| A12    | Lazy work-variable initialization                      | M  | ✓      | PLANNING.md         |
| O2     | Stack raw pointer cache                                | M  | ✓      | PLANNING.md         |
| A4     | Spatial index operations                               | M  | ✓      | PLANNING.md         |
| O4     | Native: direct-emit local collections                  | M  | ✓      | PLANNING.md         |
| O5     | Native: omit `stores` from pure functions              | M  | ✓      | PLANNING.md         |

---

## Deferred indefinitely

| ID    | Title                                              | Notes                                     |
|-------|----------------------------------------------------|-------------------------------------------|
| O1    | Superinstruction peephole rewriting                | Opcode table full (254/256)               |
| P4    | Bytecode cache (`.loftc`)                          | Superseded by native codegen              |
| A7.4  | Central package registry                           | Ecosystem must exist first                |

---

**Design documents:**

| Area | Document |
|---|---|
| Renderer abstraction | [RENDERER.md](RENDERER.md) |
| Web gallery + unified GL | [WEB_EXAMPLES.md](WEB_EXAMPLES.md) |
| Graphics implementation | [OPENGL_IMPL.md](OPENGL_IMPL.md) |
| Package system | [PACKAGES.md](PACKAGES.md) |
| WASM architecture | [WASM.md](WASM.md) |
| Web IDE | [WEB_IDE.md](WEB_IDE.md) |
| Server library | [WEB_SERVER_LIB.md](WEB_SERVER_LIB.md) |
| Game client library | [GAME_CLIENT_LIB.md](GAME_CLIENT_LIB.md) |
