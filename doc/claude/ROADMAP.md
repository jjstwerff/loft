<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and fix paths: [PLANNING.md](PLANNING.md).

**Project goal:** browser games that anyone can play via a shared link.
Native OpenGL is supported for desktop enthusiasts.  Server/multiplayer
comes after the single-player browser experience works.

## Milestone narrative

| Version | Headline                                       |
|---------|------------------------------------------------|
| 0.8.4   | **Awesome Brick Buster** — a game worth sharing    |
| 0.8.5   | **Working Moros editor** — paint hex scenes in the browser |
| 0.9.0   | **Fully working loft language** — feature-complete + verified |
| 1.0.0   | **Totally sure everything works** — IDE + multiplayer + stability contract |

**Effort:** XS = Tiny · S = Small · M = Medium · MH = Med–High · H = High · VH = Very High

**Design:** ✓ = detailed design in place · ~ = partial/outline · — = needs design

**Maintenance rule:** When an item is completed, remove it from this file.
Completed work belongs in CHANGELOG.md and git history.

---

## 0.8.4 — Awesome Brick Buster

**Goal:** ship a Brick Buster game that is fun to play, not just a tech demo. The
current `lib/graphics/examples/25-brick-buster.loft` already has multi-hit bricks,
pickups, particles, combos, multi-ball, level transitions, and a sprite
atlas (G1/G2). 0.8.4 turns it from "playable proof of concept" into
"a game someone would actually want to share with a friend."

### What "awesome" means

| Area      | Today                          | After 0.8.4                                  |
|-----------|--------------------------------|----------------------------------------------|
| Audio     | ~~Silent~~ | **Done** — chiptune synthesis (sfx_beep/chirp/noise/descend/bounce) |
| Levels    | Procedurally generated rows    | Several hand-designed levels with themes     |
| Visuals   | Procedural sprite atlas        | Polished art + screen shake + better particles |
| Sharing   | ~~Run from `cargo run …`~~ | **Done** — `loft --html` single-file export (native WASM) |
| Smoothness| ~~Per-frame store leak workarounds~~ | **Done** — P122 fixed; idiomatic struct APIs work |

### Game infrastructure

| ID    | Title                                                  | E  | Status |
|-------|--------------------------------------------------------|----|--------|
| G3    | Tilemap rendering (grid-based 2D, batched draw)        | M  | Partial — the brick grid + `level_brick(lv,r,c)` dispatcher in Brick Buster is the tilemap for that game; a generic `lib/tilemap` package is still open. |
| ~~G5~~| ~~Audio: sound effect playback~~                       | S  | **Done** — rodio + audio_play_raw + chiptune synthesis |
| ~~G6~~| ~~Audio: background music with crossfade~~             | S  | **Done** — three-track rotating playlist with inter-track silence |
| ~~W1.1~~| ~~Single-file HTML export~~                          | M  | **Done** — native WASM + asyncify + GL bridge |
| G7.P  | 🌐 **Playable Brick Buster** — share link on itch.io        | S  | Ready (--html works) |

### Game polish (`lib/graphics/examples/25-brick-buster.loft`)

| ID    | Title                                                  | E  | Status |
|-------|--------------------------------------------------------|----|--------|
| ~~BK.1~~ | ~~Audio integration~~                              | S  | **Done** — brick/paddle/wall/pickup/life sounds |
| ~~BK.2~~ | ~~Background music~~                               | S  | **Done** — 3 early-Capcom tracks (Heroic / Determined / Calm) rotating with 4-s silences |
| ~~BK.3~~ | ~~Hand-designed levels (5+)~~                      | M  | **Done** — 5 layouts via `level_brick` dispatcher, procedural fallback at 6+ |
| ~~BK.4~~ | ~~Screen shake on brick break + life lost~~        | XS | **Done** — `br_shake_t/mag` applied via painter projection matrix |
| ~~BK.5~~ | ~~Pause menu + restart~~                           | S  | **Done** — P to pause, SPACE to resume |
| ~~BK.6~~ | ~~Title screen + game-over screen~~                | S  | **Done** — state machine with restart |
| ~~BK.7~~ | ~~High-score persistence~~                         | S  | **Done** — `.loft/brickbuster_score.txt` + `HI <n>` HUD row |
| ~~BK.8~~ | ~~Sprite-atlas polish pass~~                       | S  | **Done** — cel-shaded outlines, round ball with velocity-directional squash, hearts, Roman-numeral levels, balloon projectile, fireball after-images |

### Language fixes (all completed)

All language blockers for 0.8.4 are resolved:
- **P117–P131** — fixed and verified (store leaks, slot allocation, native codegen,
  CLI args, headless GL safety). See PROBLEMS.md § Fixed.
- **L4** — compile error for literal passed to `&` parameter.
- **L5/L6/S5** — confirmed already fixed.
- **Bytecode cache** — `build.rs` adds git commit hash; rebuilds invalidate stale `.loftc`.

Brick Buster's raw-float workarounds (bitmasks, `aabb_depth_x/y`) can now be replaced
with idiomatic struct-based APIs.

---

## 0.8.5 — Working Moros editor

**Goal:** the Moros hex RPG scene editor runs end-to-end in the browser:
load a map, paint hexes, place walls and items, see a live 3D preview,
export to GLB. Web only — multiplayer comes in 1.0.0.

Design: `../moros/doc/claude/`

### Must-fix blockers (share the Moros editor's code paths)

Step plans for both entries: [QUALITY.md](QUALITY.md).

| ID    | Title                                                           | E  | Source                       |
|-------|-----------------------------------------------------------------|----|------------------------------|
| P137  | `loft --html` browser WASM wedges on first `loft_start` — blocks every WASM-shipped loft program, Moros editor included | M  | PROBLEMS.md #137, QUALITY.md |
| ~~P135~~ | ~~Canvas Y-flip three-way compensation~~ | S | **Done** — upload flip removed + TEX_VERT_2D samples identity V; 2×2 atlas corner guard in `snap_smoke.sh`. Canonical convention locked in OPENGL.md |

### Sprint A–C: Data model + editor + loft backend

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.1a  | `moros_map` — Hex, Chunk, Map, HexAddress structs      | S  | ✓      |               |
| MO.1b  | `moros_map` — MaterialDef, WallDef, ItemDef palettes   | S  | ✓      | MO.1a         |
| MO.1c  | `moros_map` — SpawnPoint, NpcRoutine, NpcWaypoint      | S  | ✓      | MO.1a         |
| MO.2   | `moros_map` — map_to_json / map_from_json              | S  | ✓      | MO.1a         |
| MO.C1  | `scene-canvas.js` — hex coordinate math + flat render  | M  | ✓      | MO.2          |
| MO.C2  | `scene-canvas.js` — pan/zoom/hit-test                  | S  | ✓      | MO.C1         |
| MO.C3  | `scene-canvas.js` — layer rendering with opacity       | S  | ✓      | MO.C1         |
| MO.E1  | `scene-editor.html` — shell + toolbar + palettes       | M  | ✓      | MO.C1         |
| MO.E2  | `scene-editor.js` — Select + Paint + Height tools      | M  | ✓      | MO.E1         |
| MO.E3  | `scene-editor.js` — Wall + Item placement              | S  | ✓      | MO.E2         |
| MO.E4  | `scene-editor.js` — undo/redo stack                    | S  | ✓      | MO.E2         |
| MO.E5  | `scene-editor.js` — localStorage save/load             | S  | ✓      | MO.E2         |
| MO.3   | `moros_editor` — hex paint, height, wall, item ops     | M  | ✓      | MO.1a         |
| MO.4   | `moros_editor` — undo/redo stack (loft-side)           | S  | ✓      | MO.3          |
| MO.5a  | `moros_editor` — slope tool                            | S  | ✓      | MO.3          |
| MO.5b  | `moros_editor` — stencil stamping (12 orientations)    | M  | ✓      | MO.3          |
| MO.6   | `moros_editor` — spawn/waypoint management             | S  | ✓      | MO.3          |
| MO.W1  | WASM build: moros_map + moros_editor → .wasm           | S  | ✓      | MO.3          |

### Sprint D–E: 3D renderer + export

| ID     | Title                                                  | E  | Design | Depends on    |
|--------|--------------------------------------------------------|----|--------|---------------|
| MO.7a  | `moros_render` — hex surface geometry (flat + slope)   | M  | ✓      | MO.1a         |
| MO.7b  | `moros_render` — wall slab geometry (thin + thick)     | M  | ✓      | MO.7a         |
| MO.7c  | `moros_render` — MeshBuilder batching + material sort  | S  | ✓      | MO.7a         |
| MO.8a  | `moros_render` — linear stair steps                    | S  | ✓      | MO.7a         |
| MO.8b  | `moros_render` — spiral newel + grand arc treads       | M  | ✓      | MO.8a         |
| MO.9a  | `moros_render` — camera orbit/pan/zoom                 | S  | ✓      | MO.7a, GL6.6  |
| MO.9b  | `moros_render` — hex picking (screen ray → hex addr)   | M  | ✓      | MO.9a         |
| MO.10  | `moros_render` — GLB export (scene → file/base64)      | M  | ✓      | MO.7a         |
| MO.13  | Developer art — flat-shade procedural swatches         | S  | ✓      | MO.7a         |
| MO.W2  | WASM build: moros_render → .wasm                       | S  | ✓      | MO.7a         |
| MO.12a | `scene-editor.html` — wire loft WASM to JS editor      | M  | ✓      | MO.W1, MO.E2  |
| MO.12b | `scene-editor.html` — live 3D preview panel            | M  | ✓      | MO.W2, MO.9a  |
| MO.12c | `scene-editor.html` — GLB export button                | S  | ✓      | MO.10, MO.12a |
| MO.P   | 🌐 **Moros scene editor** on GH Pages                   | S  | ✓      | MO.12b        |

---

## 0.9.0 — Fully working loft language

**Goal:** the language itself is feature-complete, well-documented, and
tooling-friendly. No "appears fixed but unverified" bugs in PROBLEMS.md.
Anyone can write loft code in their preferred editor with syntax
highlighting, decent error messages, and a REPL for experimentation.

### Language polish

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| L1     | Error recovery after token failures                    | M  | ✓      | PLANNING.md      |
| P2     | REPL / interactive mode                                | M  | ✓      | PLANNING.md      |
| W-warn | Developer warnings (Clippy-inspired)                   | M  | ✓      | GAME_INFRA.md    |
| AOT    | Auto-compile libraries to native shared libs           | M  | ✓      | PLANNING.md      |
| C52    | Stdlib name clash: warning + `std::` prefix            | M  | ✓      | PLANNING.md      |
| C53    | Match arms: library enums + bare variant names         | M  | ✓      | PLANNING.md      |

### User-biting caveats — all ship in 0.9.0

Each of these is a commitment, not a maybe.  Deferring any of them
makes the "fully working language" label dishonest.

Step plans for both entries: [QUALITY.md](QUALITY.md).

| ID   | Title                                                                     | E  | Source                      |
|------|---------------------------------------------------------------------------|----|-----------------------------|
| C54  | Switch `integer` from i32 to i64 — eliminates the `i32::MIN` null-sentinel trap; `long` becomes a historical alias | L  | CAVEATS.md, QUALITY.md  |
| P54  | First-class `JsonValue` enum (JObject / JArray / JString / JNumber / JBool / JNull) — `json_parse` returns it; `MyStruct.parse` accepts only `JsonValue`; old text-based `json_items` / `json_nested` / `json_long` / `json_float` / `json_bool` surface withdrawn | MH | PROBLEMS.md #54, QUALITY.md |

**Shipped in earlier 0.8.x** (kept here for CHANGELOG readers; delete on 0.9.0 sweep):
- ~~C7/P22~~ — `spacial<T>` diagnostic references 1.1+ timeline
- ~~C60~~ — hash iteration in ascending key order (`quality` branch)
- ~~C61.local~~ — outer-local shadow reject on pass 1
- ~~C61-nested~~ — parse-time reject for `for i { for i { } }`
- ~~P86~~ — real closures; regression guards in `tests/issues.rs` and `tests/parse_errors.rs`
- ~~P91~~ — default-from-earlier-parameter via call-site Var(N) substitution
- ~~P138~~ — `loft --native` prints actionable hint on E0460 + rand_core
- ~~SLOT-VEC3 / #139~~ — `OpReserveFrame` fires when slot sits above TOS

### Compilation cache and constant store

The `.loftc` bytecode cache and `CONST_STORE` are implemented (Phase A + D).
Remaining work must land in 0.9.0 to avoid stability risk in later milestones.
Design: [CONST_STORE.md](CONST_STORE.md).

| ID     | Title                                                  | E  | Design | Source           |
|--------|--------------------------------------------------------|----|--------|------------------|
| CS.B   | mmap cache loading (native)                            | S  | ✓      | CONST_STORE.md   |
| CS.C1  | Serialize `Data` struct to binary                      | MH | ~      | CONST_STORE.md   |
| CS.C2  | `build.rs` pre-compile stdlib to `.loftc`              | M  | ✓      | CONST_STORE.md   |
| CS.C3  | WASM: `include_bytes!` stdlib cache, skip re-parse     | S  | ✓      | CONST_STORE.md   |

CS.B becomes worthwhile after CS.C2 produces a larger cache file. CS.C1 is
the prerequisite for CS.C2/C3 — it requires serializing `Definition`,
`Value`, `Type`, `Attribute`, and `Function` (recursive enums, ~2K lines
in `data.rs`). Hand-written binary serialization preferred over serde to
avoid adding serde to the default feature set.

### Developer experience

| ID    | Title                                                  | E  | Design | Source           |
|-------|--------------------------------------------------------|----|--------|------------------|
| SH.1  | TextMate grammar for `.loft` syntax highlighting       | S  | ✓      | DX.md            |
| SH.2  | VS Code extension (syntax + snippets + run task)       | S  | ✓      | DX.md            |
| DX.1  | Quick-start `examples/` directory                      | XS | ✓      | DX.md            |
| DX.2  | CI: add package tests + native tests to workflow       | XS | ✓      | DX.md            |

### Package and FFI tooling

| ID    | Title                                                  | E  | Design | Source           |
|-------|--------------------------------------------------------|----|--------|------------------|
| PKG.7 | Lock file (`loft.lock`) for reproducible builds        | S  | ✓      | manifest.rs      |
| FFI.1 | Generic type marshaller from `#native` signature       | MH | ✓      | GAME_INFRA.md    |
| FFI.2 | Generic cdylib loader — scan exports, HashMap          | S  | ✓      | GAME_INFRA.md    |
| FFI.3 | Eliminate per-function glue in native.rs               | M  | ✓      | GAME_INFRA.md    |
| FFI.4 | Docs: zero-boilerplate native function guide           | S  | ✓      | GAME_INFRA.md    |

### CLI fixes that improved during 0.8.4

These were closed during today's release-mode push but belong to the
language-polish narrative.

| ID   | Title                                                  | Status |
|------|--------------------------------------------------------|--------|
| P126 | `-1` tail expression after `if { return; }`            | ✓ closed |
| P128 | File-scope constants reject type annotations           | ✓ closed |
| P131 | CLI consumes script-level arguments                    | ✓ closed |
| P132 | Release-mode coroutine-iterator-character hang         | ✓ closed |

---

## 1.0.0 — Totally sure everything works

**Goal:** the stability contract. Anyone can write, run, and share a
program — terminal or browser — and trust that it will keep working.
Ship the IDE, ship multiplayer, and prove the language is bulletproof
with hands-on testing on every supported platform.

### IDE + multiplayer must-haves

| ID    | Title                                                  | E  | Design | Source           |
|-------|--------------------------------------------------------|----|--------|------------------|
| W2    | Editor shell (CodeMirror 6 + Loft grammar)             | M  | ✓      | WEB_IDE.md       |
| W3    | Symbol navigation (go-to-def, find-usages)             | M  | ✓      | WEB_IDE.md       |
| W4    | Multi-file projects (IndexedDB)                        | M  | ✓      | WEB_IDE.md       |
| W5    | Docs & examples browser                                | M  | ✓      | WEB_IDE.md       |
| W6    | Export/import ZIP + PWA offline                        | M  | ✓      | WEB_IDE.md       |

### Scene scripting

| ID    | Title                                                  | E  | Design | Depends on    |
|-------|--------------------------------------------------------|----|--------|---------------|
| SC.1  | Scene script API — hooks for hex enter/exit/interact   | M  | ✓      | MO.3, W2      |
| SC.2  | IDE panel in scene editor                              | M  | ✓      | W2, MO.E1     |
| SC.3  | In-browser compile + hot-reload                        | M  | ✓      | W1, SC.1      |
| SC.4  | Script sandbox — limited API                           | S  | ✓      | SC.3          |
| SC.5  | Built-in script templates                              | S  | ✓      | SC.1          |
| SC.6  | Script sharing via scene JSON                          | S  | ✓      | SC.3, MO.2    |
| SC.P  | 🌐 **Scriptable scenes** in browser                     | S  | ✓      | SC.3, MO.P    |

### Multiplayer

| ID    | Title                                                  | E  | Design | Source              |
|-------|--------------------------------------------------------|----|--------|---------------------|
| SRV.1 | Plain HTTP routing + middleware                        | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.2 | HTTPS with static PEM certificates                     | S  | ✓      | WEB_SERVER_LIB.md   |
| SRV.3 | WebSocket support                                      | S  | ✓      | WEB_SERVER_LIB.md   |
| SRV.4 | Authentication: JWT, session, API key                  | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.5 | ACME / Let's Encrypt automatic certs                   | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.6 | CORS, rate limiting, static files                      | M  | ✓      | WEB_SERVER_LIB.md   |
| SRV.G | Game loop: ws_poll, broadcast, ConnectionRegistry      | M  | ✓      | WEB_SERVER_LIB.md   |
| GC.1  | WebSocket client + GameEnvelope protocol               | M  | ✓      | GAME_CLIENT_LIB.md  |
| GC.2  | Lobby + matchmaking                                    | S  | ✓      | GAME_CLIENT_LIB.md  |
| GC.3  | Fixed-timestep game loop                               | S  | ✓      | GAME_CLIENT_LIB.md  |
| GC.4  | Client-side prediction + reconciliation                | M  | ✓      | GAME_CLIENT_LIB.md  |
| GC.5  | WASM script loading + Ed25519 verification             | M  | ✓      | GAME_CLIENT_LIB.md  |
| GC.6  | Shared game logic + Tic-Tac-Toe demo                   | M  | ✓      | GAME_CLIENT_LIB.md  |
| MP.P  | 🌐 **Moros multiplayer** — DM + players share live scene | S  | ✓      | hosted server       |

### Stability gate (no shortcuts)

The 1.0.0 stability contract requires every item below to be checked off
before tagging — no "appears fixed" exceptions.

- [ ] **PROBLEMS.md** has zero open `**High**` severity entries
- [ ] All `⚠️ Appears fixed but unverified` flags from 0.9.0 have been
      definitively closed via real-world testing (not just regression guards)
- [ ] **valgrind clean** on a debug build of `tests/scripts/50-tuples.loft`
      and the full brick-buster game (`25-brick-buster.loft`) for 5+ minutes of play
- [ ] `make ci` green on Linux, macOS Intel, macOS ARM, Windows
- [ ] All `~~Fixed~~` PROBLEMS.md entries removed (history lives in CHANGELOG.md)
- [ ] `doc/claude/INCONSISTENCIES.md` reviewed: each entry resolved or
      explicitly accepted in LOFT.md / CHANGELOG.md
- [ ] Pre-built binaries on the GitHub release for all four platforms
- [ ] HTML reference and PDF up to date and linked from the release page

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
| Developer experience | [DX.md](DX.md) |
| Game infrastructure | [GAME_INFRA.md](GAME_INFRA.md) |
| Package system | [PACKAGES.md](PACKAGES.md) |
| WASM + frame yield | [WASM.md](WASM.md) |
| Web IDE | [WEB_IDE.md](WEB_IDE.md) |
| Server library | [WEB_SERVER_LIB.md](WEB_SERVER_LIB.md) |
| Game client library | [GAME_CLIENT_LIB.md](GAME_CLIENT_LIB.md) |
| Graphics | [OPENGL_IMPL.md](OPENGL_IMPL.md) |
| Renderer abstraction | [RENDERER.md](RENDERER.md) |
