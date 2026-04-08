
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

## Completed (not on roadmap)

Renderer (R1–R4), WASM + playground (W1, W1.P), WebGL bridge (GL6.1–GL6.5),
frame yield (FY.1–FY.3), input (GL6.6), asset loading (GL7.1–GL7.4),
🌐 **live graphics gallery** (GAL.3), language bug fixes (P110–P114).

---

## 0.8.4 — First playable game

### Game infrastructure + first game

| ID     | Title                                                  | E  | Design | Source                     |
|--------|--------------------------------------------------------|----|--------|----------------------------|
| G1     | Sprite sheet loading (atlas texture + UV rect lookup)  | S  | ✓      | GAME_INFRA.md              |
| G2     | Sprite drawing (billboarded quads in 3D or 2D overlay) | S  | ✓      | GAME_INFRA.md              |
| G3     | Tilemap rendering (grid-based 2D, batched draw)        | M  | ✓      | GAME_INFRA.md              |
| G4     | 2D collision detection (AABB + circle)                 | S  | ✓      | GAME_INFRA.md              |
| G5     | Audio: sound effect playback (Web Audio + native)      | S  | ✓      | GAME_INFRA.md              |
| G6     | Audio: background music with crossfade                 | S  | ✓      | GAME_INFRA.md              |
| G7     | First playable demo game (Breakout clone)              | M  | ✓      | GAME_INFRA.md              |
| W1.1   | Single-file HTML export (`loft --html game.loft`)      | M  | ✓      | GAME_INFRA.md              |
| G7.P   | 🌐 **Playable Breakout** — share link on itch.io        | S  | ✓      |                            |

### Moros — hex RPG scene editor

Moros is a browser-based tabletop RPG toolkit: a hex-grid scene editor and
3D renderer.  First real application built on loft's graphics and WASM stack.

Design: `../moros/doc/claude/` — LOFT_LIBRARIES, SCENE_EDITOR_PLAN,
SCENE_MAP, SCENE_MAP_RENDER, OPEN_ISSUES.

#### Sprint A: Data model + 2D canvas

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.1a  | `moros_map` — Hex, Chunk, Map, HexAddress structs      | S  | ✓      |               |
| MO.1b  | `moros_map` — MaterialDef, WallDef, ItemDef palettes   | S  | ✓      | MO.1a         |
| MO.1c  | `moros_map` — SpawnPoint, NpcRoutine, NpcWaypoint      | S  | ✓      | MO.1a         |
| MO.2   | `moros_map` — map_to_json / map_from_json              | S  | ✓      | MO.1a         |
| MO.C1  | `scene-canvas.js` — hex coordinate math + flat render  | M  | ✓      | MO.2          |
| MO.C2  | `scene-canvas.js` — pan/zoom/hit-test                  | S  | ✓      | MO.C1         |
| MO.C3  | `scene-canvas.js` — layer rendering with opacity       | S  | ✓      | MO.C1         |

#### Sprint B: Editor tools

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.E1  | `scene-editor.html` — shell page + toolbar + palettes  | M  | ✓      | MO.C1         |
| MO.E2  | `scene-editor.js` — Select + Paint + Height tools      | M  | ✓      | MO.E1         |
| MO.E3  | `scene-editor.js` — Wall + Item placement tools        | S  | ✓      | MO.E2         |
| MO.E4  | `scene-editor.js` — undo/redo stack                    | S  | ✓      | MO.E2         |
| MO.E5  | `scene-editor.js` — localStorage save/load             | S  | ✓      | MO.E2         |

#### Sprint C: Loft backend

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.3   | `moros_editor` — hex paint, height, wall, item ops     | M  | ✓      | MO.1a         |
| MO.4   | `moros_editor` — undo/redo stack (loft-side)           | S  | ✓      | MO.3          |
| MO.5a  | `moros_editor` — slope tool                            | S  | ✓      | MO.3          |
| MO.5b  | `moros_editor` — stencil stamping (12 orientations)    | M  | ✓      | MO.3          |
| MO.6   | `moros_editor` — spawn/waypoint management             | S  | ✓      | MO.3          |
| MO.W1  | WASM build: moros_map + moros_editor → .wasm           | S  | ✓      | MO.3          |

#### Sprint D: 3D renderer

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.7a  | `moros_render` — hex surface geometry (flat + slope)   | M  | ✓      | MO.1a         |
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
| MO.W2  | WASM build: moros_render → .wasm                       | S  | ✓      | MO.7a         |
| MO.12a | `scene-editor.html` — wire loft WASM to JS editor      | M  | ✓      | MO.W1, MO.E2  |
| MO.12b | `scene-editor.html` — live 3D preview panel            | M  | ✓      | MO.W2, MO.9a  |
| MO.12c | `scene-editor.html` — GLB export button                | S  | ✓      | MO.10, MO.12a |
| MO.P   | 🌐 **Moros scene editor** on GH Pages                   | S  | ✓      | MO.12b         |

### Remaining package/language items

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| PKG.7  | Lock file (`loft.lock`) for reproducible builds        | S  | ✓      | manifest.rs      |
| FFI.1  | Generic type marshaller from `#native` signature       | MH | ✓      | GAME_INFRA.md    |
| FFI.2  | Generic cdylib loader — scan exports, HashMap          | S  | ✓      | GAME_INFRA.md    |
| FFI.3  | Eliminate per-function glue in native.rs               | M  | ✓      | GAME_INFRA.md    |
| FFI.4  | Docs: zero-boilerplate native function guide           | S  | ✓      | GAME_INFRA.md    |

### Game performance

These apply to all deployment targets (native + WASM browser).  Interpreter-only
items (W3 frame-aware dispatch, W4 opcode redesign) are tracked in OPTIMISATIONS.md.

| ID   | Title                                                            | E  | Design | Source           |
|------|------------------------------------------------------------------|----|--------|------------------|
| W.G1 | GL overhead: cache uniform locations + direct wasm_bindgen imports | S  |        | OPTIMISATIONS.md |
| W.G2 | Game object store pooling — plain-data memset pool (S29 already handles general case) | M  |        | OPTIMISATIONS.md |
| W.G3 | `vector<byte>` type — zero-copy pixel/canvas/text transfer       | M  | ✓      | PERFORMANCE.md   |
| W.G4 | Zero-copy vertex upload via WASM memory view (`Float32Array`)    | S  | ✓      | PERFORMANCE.md   |
| V1   | In-place sort/reverse — raw slice, no intermediate Vec           | S  | ✓      | PERFORMANCE.md   |
| V2   | Binary vector bulk write — one claim + memcpy for plain-data     | S  | ✓      | PERFORMANCE.md   |

---

## 0.9.0 — Polish + developer experience

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| P2     | REPL / interactive mode                                | M  | ✓      | PLANNING.md      |
| L1     | Error recovery after token failures                    | M  | ✓      | PLANNING.md      |
| W-warn | Developer warnings (Clippy-inspired)                   | M  | ✓      | GAME_INFRA.md    |
| AOT    | Auto-compile libraries to native shared libs           | M  | ✓      | PLANNING.md      |
| C52    | Stdlib name clash: warning + `std::` prefix            | M  | ✓      | PLANNING.md      |
| C53    | Match arms: library enums + bare variant names         | M  | ✓      | PLANNING.md      |

---

## 1.0.0 — IDE + multiplayer

### Web IDE

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| W2     | Editor shell (CodeMirror 6 + Loft grammar)             | M  | ✓      | WEB_IDE.md       |
| W3     | Symbol navigation (go-to-def, find-usages)             | M  | ✓      | WEB_IDE.md       |
| W4     | Multi-file projects (IndexedDB)                        | M  | ✓      | WEB_IDE.md       |
| W5     | Docs & examples browser                                | M  | ✓      | WEB_IDE.md       |
| W6     | Export/import ZIP + PWA offline                         | M  | ✓      | WEB_IDE.md       |

### Scene scripting

| ID     | Title                                                  | E  | Design | Depends on         |
|--------|--------------------------------------------------------|----|--------|--------------------|
| SC.1   | Scene script API — hooks for hex enter/exit/interact   | M  | ✓      | MO.3, W2           |
| SC.2   | IDE panel in scene editor                              | M  | ✓      | W2, MO.E1          |
| SC.3   | In-browser compile + hot-reload                        | M  | ✓      | W1, SC.1            |
| SC.4   | Script sandbox — limited API                           | S  | ✓      | SC.3               |
| SC.5   | Built-in script templates                              | S  | ✓      | SC.1               |
| SC.6   | Script sharing via scene JSON                          | S  | ✓      | SC.3, MO.2         |
| SC.P   | 🌐 **Scriptable scenes** in browser                     | S  | ✓      | SC.3, MO.P         |

### Multiplayer

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
| MP.P   | 🌐 **Moros multiplayer** — DM + players share live scene | S  | ✓      | hosted server       |

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
| WASM + frame yield | [WASM.md](WASM.md) |
| Game infrastructure | [GAME_INFRA.md](GAME_INFRA.md) |
| Package system | [PACKAGES.md](PACKAGES.md) |
| Web IDE | [WEB_IDE.md](WEB_IDE.md) |
| Server library | [WEB_SERVER_LIB.md](WEB_SERVER_LIB.md) |
| Game client library | [GAME_CLIENT_LIB.md](GAME_CLIENT_LIB.md) |
