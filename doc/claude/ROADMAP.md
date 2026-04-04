---
render_with_liquid: false
---
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Roadmap

Items in expected implementation order, grouped by milestone.
Full descriptions and fix paths: [PLANNING.md](PLANNING.md).

**Effort:** XS = Tiny · S = Small · M = Medium · MH = Med–High · H = High · VH = Very High

**Design:** ✓ = detailed design in place · ~ = partial/outline · — = needs design

**Maintenance rule:** When an item is completed, remove it from this file entirely.
Do not keep completed items — the ROADMAP tracks only what remains to be done.
Completed work belongs in CHANGELOG.md (user-facing) and git history (implementation).

---

## 0.8.4 — Package system + stdlib extraction + HTTP + OpenGL

The 0.8.4 milestone has three themes:

1. **Package system** (PKG) — dependencies, native codegen, WASM linking
2. **Stdlib extraction** — move PNG/Image and random out of `default/` into packages
3. **New libraries** — HTTP client, graphics/OpenGL as proper packages

All new libraries are built as **packages** using the format designed in
[PACKAGES.md](PACKAGES.md).

### Completed sprints

- **Sprint 1** — Package infrastructure (PKG.1, PKG.2, PKG.6)
- **Sprint 2** — Manifest deps, native stub replacement
- **Sprint 5** — Graphics foundation (GL0–GL2.3)
- **Sprint 6** — Graphics advanced (GL2.4–GL2.6)
- **Sprint 7** — Package deps + docs (PKG.3, `loft doc`, shapes)
- **Sprint 8** — 3D types + bug fixes (GL4.1–GL4.3, C54, P104)
- **Sprint 9** — Package registry (REG.1–REG.4)
- **Sprint 10** — Language ergonomics (C55, C56, A15, I13)
- **Sprint 11** — Native codegen for packages (PKG.4, PKG.5)

### Remaining sprints

```
Sprint 12: Stdlib extraction (needs PKG.4)
  EXT.1   imaging package (PNG + Image types)
  EXT.2   random package

Sprint 13: HTTP client (needs PKG.4)
  H4.1    HttpResponse struct
  H4.2    http_get/post native (ureq)
  H4.3    headers
  H4.5    tests

Sprint 14: Server library — core (needs Sprint 10 + PKG.2 ✓)
  SRV.1   plain HTTP: routing, middleware pipeline, request/response
  SRV.2   HTTPS with static PEM certificates
  SRV.3   WebSocket support (using I13 for msg in ws)
  SRV.4   Authentication: JWT, session, API key, HTTP Basic
  game_protocol package: GameEnvelope, WsMessage, Msg* structs

Sprint 15: Server library — production + game additions (needs SRV.3)
  SRV.5   ACME / Let's Encrypt automatic certificate provisioning
  SRV.6   CORS, rate limiting, decompression, static files
  SRV.G   Game additions: ws_poll, ws_broadcast, ConnectionRegistry,
           run_game_loop, WASM loading (shared rules.wasm)

Sprint 16: game_client — networking + lobby + loop (needs Sprint 10 + SRV.1)
  GC.1    WebSocket client + GameEnvelope protocol + Dispatcher
  GC.2    Lobby + matchmaking state management
  GC.3    Fixed-timestep game loop (using A15 for loop + render concurrently)
  GC.4    Client-side prediction + reconciliation + state delta sync + ping

Sprint 17: game_client — WASM scripts + shared logic (needs PKG.5 + SRV.G)
  GC.5    WASM script loading: wasm_load/call/verify, Ed25519 signature check
  GC.6    Shared game logic: n_script_* exports, end-to-end Tic-Tac-Toe example

Sprint 18: Graphics native (needs PKG.4)
  GL3     text rendering (fontdue native)
  GL5.1   window + event loop
  GL5.2-5 shaders, VBO, render, texture
  GL6.1-4 WebGL2 equivalents

Sprint 19: Native FFI simplification (needs Sprint 12)
  FFI.1   generic type marshaller — auto pop/push from #native type signature
  FFI.2   generic cdylib loader — scan exports, register in HashMap
  FFI.3   eliminate per-function glue in native.rs and extensions.rs
  FFI.4   doc: "write a Rust function, declare in loft, done"
```

### Remaining item table

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| EXT.1     | Extract Image/Pixel/PNG to `imaging` package              | M  | ✓      | PKG.1        | default/02_images.loft → pkg  |
| EXT.2     | Extract random to `random` package                        | S  | ✓      | PKG.1        | src/native.rs → pkg           |
| PKG.7     | Lock file (`loft.lock`) for reproducible builds           | S  | ✓      | PKG.3        | manifest.rs                   |
| H4.1      | HttpResponse struct + ok() in web/src/web.loft            | S  | ✓      | PKG.1        | web/src/web.loft              |
| H4.2      | http_get/post/put/delete in web/native/ (ureq)            | M  | ✓      | H4.1         | web/native/src/lib.rs         |
| H4.3      | Header support (http_get_h, http_post_h)                  | S  | ✓      | H4.2         | web/native/src/lib.rs         |
| H4.5      | Package tests + documentation                             | S  | ✓      | H4.2         | web/tests/                    |
| SRV.1     | Plain HTTP routing + middleware pipeline (loft layer)     | M  | ✓      | C55, C56     | server/src/                   |
| SRV.2     | HTTPS with static PEM certificates (rustls)               | S  | ✓      | SRV.1        | server/native/                |
| SRV.3     | WebSocket support (I13 enables `for msg in ws`)           | S  | ✓      | SRV.1, I13   | server/src/websocket.loft     |
| SRV.4     | Authentication: JWT, session, API key, HTTP Basic         | M  | ✓      | SRV.1        | server/src/auth.loft          |
| SRV.P     | game_protocol package: GameEnvelope, WsMessage, Msg*      | S  | ✓      |              | game_protocol/src/            |
| SRV.5     | ACME / Let's Encrypt automatic certificate provisioning   | M  | ✓      | SRV.2        | server/native/ (instant-acme) |
| SRV.6     | CORS, rate limiting, decompression, static files          | M  | ✓      | SRV.1        | server/src/middleware.loft    |
| SRV.G     | Game additions: ws_poll, ws_broadcast, ConnectionRegistry, | M  | ✓      | SRV.3, A15   | server/src/game_loop.loft     |
|           |   run_game_loop, server-side WASM loading                 |    |        |              |                               |
| GC.1      | WebSocket client + GameEnvelope protocol + Dispatcher     | M  | ✓      | SRV.P        | game_client/src/              |
| GC.2      | Lobby + matchmaking state management                      | S  | ✓      | GC.1         | game_client/src/lobby.loft    |
| GC.3      | Fixed-timestep game loop (A15 enables loop + render side-by-side) | S | ✓ | GC.2, A15  | game_client/src/loop.loft     |
| GC.4      | Client-side prediction + reconciliation + state sync + ping | M | ✓     | GC.3         | game_client/src/predict.loft  |
| GC.5      | WASM script loading: wasm_load/call/verify, Ed25519       | M  | ✓      | GC.1, PKG.5  | game_client/native/           |
| GC.6      | Shared game logic: n_script_* exports, Tic-Tac-Toe demo  | M  | ✓      | GC.5, SRV.G  | game_client/src/wasm.loft     |
| GL3       | Text rendering (fontdue native + pure loft layout)        | M  | ✓      | GL1, PKG.1   | graphics/src/text.loft        |
| GL4.4     | GLB binary writer (header + JSON chunk + BIN chunk)       | M  | ✓      | GL4.2        | graphics/src/glb.loft         |
| GL4.5     | GLB accessor/bufferView encoding for mesh data            | M  | ✓      | GL4.4        | graphics/src/glb.loft         |
| GL4.6     | GLB material + texture + scene node encoding              | S  | ✓      | GL4.5        | graphics/src/glb.loft         |
| GL5.1     | Window creation + event loop (glutin + #native)           | M  | ✓      | PKG.1        | graphics/native/src/gl.rs     |
| GL5.2     | Shader compile + link + uniform upload                    | S  | ✓      | GL5.1        | graphics/native/src/gl.rs     |
| GL5.3     | VBO/VAO creation from Mesh vertex data                    | S  | ✓      | GL5.2        | graphics/native/src/gl.rs     |
| GL5.4     | Draw call + swap buffers + render loop                    | S  | ✓      | GL5.3        | graphics/native/src/gl.rs     |
| GL5.5     | Texture upload from Canvas pixel buffer                   | S  | ✓      | GL5.4        | graphics/native/src/gl.rs     |
| GL6.1     | Canvas element + WebGL2 context (web-sys + #native)       | M  | ✓      | PKG.5        | graphics/native/src/webgl.rs  |
| GL6.2     | Shader compile + link (WebGL2 API)                        | S  | ✓      | GL6.1        | graphics/native/src/webgl.rs  |
| GL6.3     | Buffer upload + draw call + requestAnimationFrame         | S  | ✓      | GL6.2        | graphics/native/src/webgl.rs  |
| GL6.4     | Texture upload from Canvas pixel buffer (WebGL)           | S  | ✓      | GL6.3        | graphics/native/src/webgl.rs  |
| FFI.1     | Generic type marshaller from `#native` type signature     | MH | —      | EXT.1        | src/native.rs, src/state/codegen.rs |
| FFI.2     | Generic cdylib loader — scan exports, HashMap dispatch    | S  | —      | FFI.1        | src/extensions.rs             |
| FFI.3     | Eliminate per-function glue in native.rs / extensions.rs  | M  | —      | FFI.2        | src/native.rs, src/extensions.rs |
| FFI.4     | Docs: zero-boilerplate native function guide              | S  | —      | FFI.3        | doc/claude/EXTERNAL_LIBS.md   |

**Package system design:** [PACKAGES.md](PACKAGES.md).

**Graphics implementation status:** [OPENGL_IMPL.md](OPENGL_IMPL.md).

---

## 0.9.0 — Standalone executable + library extraction + developer warnings

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| LIB.1     | GitHub Actions release-zip workflow template for lib repos | S  | ✓      | REG.1        | .github/workflows/release.yml |
| LIB.2     | Migrate lib/graphics/ → jjstwerff/loft-graphics repo      | S  | ✓      | LIB.1, REG.4 | lib/graphics/ (removed)       |
| LIB.3     | Migrate lib/shapes/ → jjstwerff/loft-shapes repo          | S  | ✓      | LIB.2        | lib/shapes/ (removed)         |
| LIB.4     | Add graphics + shapes to central registry; verify install  | S  | ✓      | LIB.3, REG.3 | registry.txt                  |
| C55       | Type aliases (`type Handler = fn(Request) -> Response`)   | XS | ✓      |              | SERVER_FEATURES.md § C55      |
| C56       | `?? return expr` null early-exit in handlers              | XS | ✓      |              | SERVER_FEATURES.md § C56      |
| A15       | `parallel { }` structured concurrency block               | M  | ✓      |              | SERVER_FEATURES.md § A15      |
| L1        | Error recovery after token failures                       | M  | ✓      |              | PLANNING.md § L1              |
| A2        | Logger: hot-reload, run-mode, release + debug             | M  | ✓      |              | LOGGER.md                     |
| C52       | Stdlib name clash: warning + `std::` prefix               | M  | ✓      |              | PLANNING.md § C52             |
| C53       | Match arms: library enums + bare variant names            | M  | ✓      |              | PLANNING.md § C53             |
| W-warn    | Developer warnings (Clippy-inspired)                      | M  | —      |              | see below                     |
| AOT       | Auto-compile libraries to native shared libs              | M  | ✓      |              | PLANNING.md § AOT             |
| P2        | REPL / interactive mode                                    | M  | ✓      | L1           | PLANNING.md § P2              |

### W-warn — Developer warnings

Additional warnings to catch common mistakes, inspired by Rust's Clippy:

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

## 1.0.0 — IDE + stability contract

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| I13       | Iterator protocol (`for msg in ws` via `fn next → T?`)   | MH | ✓      | I5+          | SERVER_FEATURES.md § I13      |
| W2        | Editor shell (CodeMirror 6 + Loft grammar)                | M  | ✓      | W1           | WEB_IDE.md M2                 |
| W3        | Symbol navigation (go-to-def, find-usages)                | M  | ✓      | W1, W2       | WEB_IDE.md M3                 |
| W4        | Multi-file projects (IndexedDB)                           | M  | ✓      | W2           | WEB_IDE.md M4                 |
| W5        | Docs & examples browser                                    | M  | ✓      | W2           | WEB_IDE.md M5                 |
| W6        | Export/import ZIP + PWA offline                             | M  | ✓      | W4           | WEB_IDE.md M6                 |

---

## 1.1+ — Backlog

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| C57       | Route decorator syntax (`@get`, `@post`, `@ws`)           | H  | ✓      | C55          | SERVER_FEATURES.md § C57      |
| W1.14     | WASM Tier 2: Web Worker pool; `par()` parallelism         | VH | ✓      | W1.18        | WASM.md — Threading           |
| I12       | Interfaces: factory methods (`fn zero() -> Self`)         | S  | ✓      | I5.1         | INTERFACES.md § Q4/Q6         |
| I8.5      | Interfaces: left-side concrete operand                    | S  | ~      | I8.3         | INTERFACES.md § Phase 1 gaps  |
| A12       | Lazy work-variable initialization                         | M  | ✓      |              | PLANNING.md § A12             |
| O2        | Stack raw pointer cache                                    | M  | ✓      |              | PLANNING.md § O2              |
| A4        | Spatial index operations                                   | M  | ✓      |              | PLANNING.md § A4              |
| O4        | Native: direct-emit local collections                      | M  | ✓      |              | PLANNING.md § O4              |
| O5        | Native: omit `stores` from pure functions                  | M  | ✓      | O4           | PLANNING.md § O5              |

---

## Deferred indefinitely

| ID    | Title                                                     | E  | Notes                                                              |
|-------|-----------------------------------------------------------|----|-------------------------------------------------------------------|
| O1    | Superinstruction peephole rewriting                       | M  | Blocked: opcode table full (254/256 used)                          |
| P4    | Bytecode cache (`.loftc`)                                 | M  | Superseded by native codegen                                       |
| A7.4  | Package registry (central, `loft install <url>`)          | M  | 2.x; ecosystem must exist first                                   |

---

## See also

- [PLANNING.md](PLANNING.md) — Full descriptions and fix paths
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark data and O1–O7 designs
- [DEVELOPMENT.md](DEVELOPMENT.md) — Sprint workflow, branch naming, CI
- [RELEASE.md](RELEASE.md) — Gate criteria per milestone
