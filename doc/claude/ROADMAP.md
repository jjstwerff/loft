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
- **Sprint 5** — Graphics foundation (GL0–GL2.3: canvas, pixel ops, lines, rect, circle, ellipse)
- **Sprint 6** — Graphics advanced (GL2.4–GL2.6: Bezier, AA line, triangle fill, fill_ellipse)

### Remaining sprints

```
Sprint 7: Package deps + docs ✓ (branch sprint-7-pkg-deps-math)
  PKG.3   dependency resolution ✓
  loft doc subcommand ✓
  shapes test package ✓

Sprint 8: 3D types + GLB (branch sprint-8-glb-types)
  GL4.1   Vec3/Mat4 math types ✓
  GL4.2   mesh types ✓
  GL4.3   scene types ✓
  GL4.4   GLB binary writer

Sprint 9: Native codegen for packages
  PKG.4   native codegen --extern
  PKG.5   WASM codegen linking

Sprint 10: Stdlib extraction (needs PKG.4)
  EXT.1   imaging package (PNG + Image types)
  EXT.2   random package

Sprint 11: HTTP client (needs PKG.4)
  H4.1    HttpResponse struct
  H4.2    http_get/post native (ureq)
  H4.3    headers
  H4.5    tests

Sprint 12: Graphics native (needs PKG.4)
  GL3     text rendering (fontdue native)
  GL5.1   window + event loop
  GL5.2-5 shaders, VBO, render, texture
  GL6.1-4 WebGL2 equivalents
```

### Remaining item table

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| EXT.1     | Extract Image/Pixel/PNG to `imaging` package              | M  | ✓      | PKG.1        | default/02_images.loft → pkg  |
| EXT.2     | Extract random to `random` package                        | S  | ✓      | PKG.1        | src/native.rs → pkg           |
| PKG.4     | Native codegen `--extern` for `#native` packages          | M  | ✓      | PKG.1        | generation/mod.rs, main.rs    |
| PKG.5     | WASM codegen with native package wasm rlib                | M  | ✓      | PKG.4        | main.rs                       |
| PKG.7     | Lock file (`loft.lock`) for reproducible builds           | S  | ✓      | PKG.3        | manifest.rs                   |
| H4.1      | HttpResponse struct + ok() in web/src/web.loft            | S  | ✓      | PKG.1        | web/src/web.loft              |
| H4.2      | http_get/post/put/delete in web/native/ (ureq)            | M  | ✓      | H4.1         | web/native/src/lib.rs         |
| H4.3      | Header support (http_get_h, http_post_h)                  | S  | ✓      | H4.2         | web/native/src/lib.rs         |
| H4.5      | Package tests + documentation                             | S  | ✓      | H4.2         | web/tests/                    |
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

**Package system design:** [PACKAGES.md](PACKAGES.md).

**Graphics implementation status:** [OPENGL_IMPL.md](OPENGL_IMPL.md).

---

## 0.9.0 — Standalone executable + developer warnings

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
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
| W2        | Editor shell (CodeMirror 6 + Loft grammar)                | M  | ✓      | W1           | WEB_IDE.md M2                 |
| W3        | Symbol navigation (go-to-def, find-usages)                | M  | ✓      | W1, W2       | WEB_IDE.md M3                 |
| W4        | Multi-file projects (IndexedDB)                           | M  | ✓      | W2           | WEB_IDE.md M4                 |
| W5        | Docs & examples browser                                    | M  | ✓      | W2           | WEB_IDE.md M5                 |
| W6        | Export/import ZIP + PWA offline                             | M  | ✓      | W4           | WEB_IDE.md M6                 |

---

## 1.1+ — Backlog

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
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
