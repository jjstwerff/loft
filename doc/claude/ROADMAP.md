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

1. **Package system** (PKG) — `#native` dispatch, dependencies, `loft test`
2. **Stdlib extraction** — move PNG/Image and random out of `default/` into packages
3. **New libraries** — HTTP client, graphics/OpenGL as proper packages

All new libraries are built as **packages** using the format designed in
[PACKAGES.md](PACKAGES.md): `src/*.loft` for pure logic, `native/` for
Rust code, `tests/` for test suites.  This proves the package system with
real-world libraries.

### Execution order

The package system (PKG.1) must ship first — everything else depends on it.
Then stdlib extraction (EXT) proves packages work by migrating existing code.
Then new libraries (H4, GL) build on the proven foundation.

```
PKG.1 → PKG.2 → PKG.3   (package infrastructure)
  ↓
EXT.1 → EXT.2            (stdlib extraction — proves packages work)
  ↓
H4.1 → H4.2 → H4.3      (HTTP — first new package with native code)
  ↓
GL0 → GL1 → GL2 → ...    (graphics — largest package)
```

### Stdlib extraction plan

Currently `default/02_images.loft` contains Image/Pixel types and PNG ops,
and `src/png_store.rs` has the native PNG implementation.  Random number
generation is feature-gated in `src/native.rs`.  These move to packages:

| Current location | Package | What moves | What stays in stdlib |
|---|---|---|---|
| `default/02_images.loft` (Image, Pixel, PNG) | `imaging` | Image/Pixel structs, `png()` method, save_png | File I/O types (File, Format, FileResult) |
| `src/png_store.rs` | `imaging` native/ | PNG encode/decode via `png` crate | — |
| `src/native.rs` (rand functions) | `random` | `rand_int`, `rand_float`, `seed` | — |
| `lib/glb.loft` | `graphics` | GLB type definitions | — |

After extraction, `default/02_images.loft` shrinks to only File I/O.
Programs that need images write `use imaging;` — same API, just an import.

### Package table

| Package | Pure loft (`src/`) | Native (`native/`) | WASM bridge |
|---|---|---|---|
| `imaging` | Image/Pixel types | `png` crate encode/decode | host_png bridge |
| `random` | — (all native) | `rand_core` + `rand_pcg` | `Math.random()` via host |
| `web` | HttpResponse struct, header parsing | `ureq` HTTP client | `fetch()` via host_http |
| `graphics` | Rgba, Canvas, blend, line, fill, text layout, GLB writer | `fontdue` glyph raster | — |
| `graphics` (GL) | — | `glutin` + `glow` window/shader | WebGL2 via `web-sys` |

JSON serialisation (`{value:j}`) and deserialisation (`Type.parse(text)`)
stay in stdlib — they're compiler-integrated syntax, not a library.

### Implementation ordering

Numbered sprints — each delivers a testable, shippable increment.
Items within a sprint can be done in any order.

```
Sprint 1: Package infrastructure
  PKG.1   #native interpreter dispatch
  PKG.6   loft test for packages
  PKG.2   loft install (local)

Sprint 2: Prove packages work — extract stdlib
  EXT.1   imaging package (PNG + Image types)
  EXT.2   random package

Sprint 3: Package dependency & codegen
  PKG.3   dependency resolution
  PKG.4   native codegen --extern
  PKG.5   WASM codegen linking
  PKG.7   lock file

Sprint 4: HTTP client
  H4.1    HttpResponse struct
  H4.2    http_get/post native (ureq)
  H4.3    headers
  H4.4    WASM bridge (fetch)
  H4.5    tests

Sprint 5: Graphics foundation ✓ (branch sprint-3-graphics-foundation)
  GL0     package scaffolding ✓
  GL1     Canvas + pixel ops ✓
  GL2.1   lines + rect ✓
  GL2.2   Bresenham line ✓
  GL2.3   circle + ellipse ✓

Sprint 6: Graphics advanced ✓ (branch sprint-6-graphics-advanced)
  GL2.4   Bezier curves ✓
  GL2.5   triangle fill ✓
  GL2.6   Wu AA line ✓
  fill_ellipse ✓
  GL3     text rendering (fontdue) — deferred (needs native extension)
  GL4.1   math types — deferred to Sprint 7
  GL4.2   mesh types — deferred to Sprint 7

Sprint 7: GLB export
  GL4.3   scene types
  GL4.4   GLB writer
  GL4.5   accessor encoding
  GL4.6   material encoding

Sprint 8: OpenGL + WebGL
  GL5.1   window + event loop
  GL5.2   shaders
  GL5.3   VBO/VAO
  GL5.4   render loop
  GL5.5   texture upload

Sprint 9: WebGL + test server
  GL6.1   WebGL2 context
  GL6.2   WebGL shaders
  GL6.3   WebGL draw + rAF
  GL6.4   WebGL texture
  TS      test server
```

Each sprint is ~2 weeks of effort.  The critical path is:
`PKG.1 → EXT.1 → GL0 → GL1 → GL2 → GL4 → GL5`.
HTTP (Sprint 4) can run in parallel with Graphics (Sprint 5+).

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| EXT       | Stdlib extraction to packages                              | —  | ✓      | PKG.2        | see below                     |
| EXT.1     | ↳ Extract Image/Pixel/PNG to `imaging` package            | M  | ✓      | PKG.1        | default/02_images.loft → pkg  |
| EXT.1.1   |   ↳ Create imaging/src/imaging.loft (types + pure fns)    | S  | ✓      | PKG.1        | imaging/src/imaging.loft      |
| EXT.1.2   |   ↳ Move png_store.rs to imaging/native/src/lib.rs        | S  | ✓      | EXT.1.1      | imaging/native/               |
| EXT.1.3   |   ↳ Strip Image/Pixel/PNG from default/02_images.loft     | S  | ✓      | EXT.1.2      | default/02_images.loft        |
| EXT.1.4   |   ↳ Add `use imaging;` to existing tests, verify pass     | S  | ✓      | EXT.1.3      | tests/docs/14-image.loft      |
| EXT.2     | ↳ Extract random to `random` package                      | S  | ✓      | PKG.1        | src/native.rs → pkg           |
| EXT.2.1   |   ↳ Create random/src/random.loft + native/src/lib.rs     | S  | ✓      | PKG.1        | random/                       |
| EXT.2.2   |   ↳ Remove rand functions from native.rs, update tests    | S  | ✓      | EXT.2.1      | src/native.rs                 |
| H4        | HTTP client (`web` package)                                | —  | ✓      | PKG.1        | WEB_SERVICES.md               |
| H4.1      | ↳ HttpResponse struct + ok() in web/src/web.loft          | S  | ✓      | PKG.1        | web/src/web.loft              |
| H4.2      | ↳ http_get/post/put/delete in web/native/ (ureq)          | M  | ✓      | H4.1         | web/native/src/lib.rs         |
| H4.3      | ↳ Header support (http_get_h, http_post_h)                | S  | ✓      | H4.2         | web/native/src/lib.rs         |
| H4.4      | ↳ WASM bridge: fetch() via host_http                      | S  | ✓      | H4.2         | web/native/src/wasm.rs        |
| H4.5      | ↳ Package tests + documentation                           | S  | ✓      | H4.2         | web/tests/                    |
| GL0       | Graphics package scaffolding                               | S  | ✓      | PKG.2        | graphics/loft.toml + stubs    |
| GL0.1     | ↳ Create graphics/ package dir, loft.toml, dep on imaging | S  | ✓      | PKG.2, EXT.1 | graphics/                     |
| GL0.2     | ↳ Rgba type + basic pixel ops (pure loft)                 | S  | ✓      | GL0.1        | graphics/src/draw.loft        |
| GL1       | Canvas: struct, pixel buffer, blend, clear                 | M  | ✓      | GL0          | graphics/src/draw.loft        |
| GL1.1     | ↳ Canvas struct with width/height/data                    | S  | ✓      | GL0.2        | graphics/src/draw.loft        |
| GL1.2     | ↳ clear(), set_pixel(), get_pixel(), blend_pixel()        | S  | ✓      | GL1.1        | graphics/src/draw.loft        |
| GL1.3     | ↳ Package tests for Canvas basics                         | S  | ✓      | GL1.2        | graphics/tests/draw.loft      |
| GL2       | Drawing primitives                                         | —  | ✓      | GL1          | graphics/src/draw.loft        |
| GL2.1     | ↳ Horizontal/vertical line + axis-aligned rect fill       | S  | ✓      | GL1          | graphics/src/draw.loft        |
| GL2.2     | ↳ Bresenham line (arbitrary angle) + anti-aliased line    | S  | ✓      | GL2.1        | graphics/src/draw.loft        |
| GL2.3     | ↳ Circle + ellipse (midpoint algorithm)                   | S  | ✓      | GL2.1        | graphics/src/primitives.loft  |
| GL2.4     | ↳ Quadratic + cubic Bezier curves                         | M  | ✓      | GL2.2        | graphics/src/draw.loft        |
| GL2.5     | ↳ Scanline polygon fill (convex + concave)                | M  | ✓      | GL2.2        | graphics/src/draw.loft        |
| GL3       | Text rendering                                             | —  | ✓      | GL1, PKG.1   | graphics/src/text.loft        |
| GL3.1     | ↳ Font/GlyphMetrics types (pure loft)                     | S  | ✓      | GL1          | graphics/src/text.loft        |
| GL3.2     | ↳ fontdue native: load_font + glyph_metrics (#native)     | M  | ✓      | GL3.1, PKG.1 | graphics/native/src/font.rs   |
| GL3.3     | ↳ draw_text layout + glyph blit (pure loft)               | M  | ✓      | GL3.2        | graphics/src/text.loft        |
| GL4       | GLB binary export                                          | —  | ✓      | GL1          | graphics/src/glb.loft         |
| GL4.1     | ↳ Vec3/Mat4 types + basic matrix ops (pure loft)          | S  | ✓      |              | graphics/src/math.loft        |
| GL4.2     | ↳ Vertex/Triangle/Mesh structs + mesh builder             | S  | ✓      | GL4.1        | graphics/src/mesh.loft        |
| GL4.3     | ↳ Scene/Camera/Light/Material structs                     | S  | ✓      | GL4.2        | graphics/src/scene.loft       |
| GL4.4     | ↳ GLB binary writer (header + JSON chunk + BIN chunk)     | M  | ✓      | GL4.2        | graphics/src/glb.loft         |
| GL4.5     | ↳ GLB accessor/bufferView encoding for mesh data          | M  | ✓      | GL4.4        | graphics/src/glb.loft         |
| GL4.6     | ↳ GLB material + texture + scene node encoding            | S  | ✓      | GL4.5        | graphics/src/glb.loft         |
| GL5       | OpenGL desktop                                             | —  | ✓      | GL4, PKG.4   | graphics/src/gl.loft          |
| GL5.1     | ↳ Window creation + event loop (glutin + #native)         | M  | ✓      | PKG.1        | graphics/native/src/gl.rs     |
| GL5.2     | ↳ Shader compile + link + uniform upload                  | S  | ✓      | GL5.1        | graphics/native/src/gl.rs     |
| GL5.3     | ↳ VBO/VAO creation from Mesh vertex data                  | S  | ✓      | GL5.2        | graphics/native/src/gl.rs     |
| GL5.4     | ↳ Draw call + swap buffers + render loop                  | S  | ✓      | GL5.3        | graphics/native/src/gl.rs     |
| GL5.5     | ↳ Texture upload from Canvas pixel buffer                 | S  | ✓      | GL5.4        | graphics/native/src/gl.rs     |
| GL6       | WebGL browser                                              | —  | ✓      | GL4, PKG.5   | graphics/src/gl.loft          |
| GL6.1     | ↳ Canvas element + WebGL2 context (web-sys + #native)     | M  | ✓      | PKG.5        | graphics/native/src/webgl.rs  |
| GL6.2     | ↳ Shader compile + link (WebGL2 API)                      | S  | ✓      | GL6.1        | graphics/native/src/webgl.rs  |
| GL6.3     | ↳ Buffer upload + draw call + requestAnimationFrame       | S  | ✓      | GL6.2        | graphics/native/src/webgl.rs  |
| GL6.4     | ↳ Texture upload from Canvas pixel buffer (WebGL)         | S  | ✓      | GL6.3        | graphics/native/src/webgl.rs  |
| PKG       | Package system: dependencies, native, WASM, test suites   | —  | ✓      |              | PACKAGES.md                   |
| PKG.1     | ↳ Connect `#native` to interpreter dispatch               | M  | ✓      |              | extensions.rs, compile.rs     |
| PKG.2     | ↳ `loft install` for local packages                       | M  | ✓      | PKG.1        | main.rs, manifest.rs          |
| PKG.3     | ↳ Package dependencies + transitive resolution            | M  | ✓      | PKG.2        | manifest.rs, parser/mod.rs    |
| PKG.4     | ↳ Native codegen `--extern` for `#native` packages        | M  | ✓      | PKG.1        | generation/mod.rs, main.rs    |
| PKG.5     | ↳ WASM codegen with native package wasm rlib              | M  | ✓      | PKG.4        | main.rs                       |
| PKG.6     | ↳ `loft test` for package test suites                     | S  | ✓      | PKG.2        | test_runner.rs, main.rs       |
| PKG.7     | ↳ Lock file (`loft.lock`) for reproducible builds         | S  | ✓      | PKG.3        | manifest.rs                   |
| TS        | Test server: embedded Rust HTTP server for H4 integration | M  | ✓      |              | PLANNING.md § TS              |
| TS.1      | ↳ `loft serve app.loft` — tiny HTTP server binary         | S  | ✓      |              | src/serve.rs                  |
| TS.2      | ↳ Route dispatch: loft `fn handle(r: Request) -> Response`| M  | ✓      | TS.1         | src/serve.rs                  |
| TS.3      | ↳ H4 integration tests against local test server          | S  | ✓      | TS.2, H4.2   | tests/web/                    |

**Package system design (PKG):**
Full design in [PACKAGES.md](PACKAGES.md).

**Stdlib extraction (EXT):**
Proves the package system by migrating existing code.  EXT.1 (imaging) is the
first real package with native Rust code — validates the full lifecycle before
new libraries are built on top.  After EXT.1, `default/02_images.loft` shrinks
to File I/O only; programs that need images write `use imaging;`.

**Graphics package (GL):**
All GL items now live in `graphics/` package directory.  Depends on `imaging`
package (for PNG) and PKG.1 (for `#native` dispatch).  GL0-GL2 are pure loft;
GL3+ use native code for fonts and GPU access.

Key pieces: `#native` symbol dispatch (PKG.1), `loft install` (PKG.2), transitive
dependency resolution with version checking and diamond-dependency handling (PKG.3),
native codegen linking (PKG.4), WASM variant selection (PKG.5), `loft test` runner
for package test suites with text/binary fixture support (PKG.6), and `loft.lock`
for reproducible builds (PKG.7).

PKG.1 is the foundation — without interpreter dispatch of `#native` symbols, no
native package code runs.  GL5/GL6 (OpenGL/WebGL) depend on PKG.4/PKG.5 for
cross-target linking.

**Test server design (TS):**

A minimal Rust HTTP server (`src/serve.rs`, new binary `loft-serve`) that loads a
loft script and calls a user-defined `fn handle(request: Request) -> Response` for
each incoming HTTP request.  Purpose: provide a local test target for H4 integration
tests without depending on external services.

```
loft serve app.loft --port 8080
```

**How it works:**
1. `src/serve.rs` uses `tiny_http` (small, no async, no tokio) to accept connections.
2. For each request, it constructs a loft `Request` struct (method, path, headers, body)
   and calls `fn handle(r: Request) -> Response` in the loaded loft script.
3. The loft function returns a `Response` struct (status, content_type, body).
4. The Rust server writes the HTTP response back to the client.

**Loft types** (in `default/06_web.loft`):
```loft
struct Request {
    method: text
    path: text
    body: text
}

struct Response {
    status: integer init(200)
    content_type: text init("text/plain")
    body: text
}
```

**Example loft script** (`examples/hello_server.loft`):
```loft
import "web"

fn handle(r: Request) -> Response {
    if r.path == "/hello" {
        Response { body: "Hello, {r.method}!" }
    } else if r.path == "/json" {
        data = MyData { name: "test", value: 42 };
        Response { content_type: "application/json", body: "{data:j}" }
    } else {
        Response { status: 404, body: "not found" }
    }
}
```

**H4 integration test** (`tests/web/`):
```rust
// 1. Start loft-serve with tests/web/test_server.loft on a random port
// 2. Use http_get/http_post from the loft H4 client to hit localhost
// 3. Assert response status, body, content-type
// 4. Shut down server
```

This tests the full round-trip: loft HTTP client → Rust TCP → loft request handler
→ Rust TCP → loft HTTP response parsing.  No external dependencies, fully offline,
deterministic.

**Cargo feature:** `serve` (new, optional) — gates `tiny_http` dependency.
Not included in default features or WASM builds.

---

## 0.9.0 — Standalone executable + developer warnings

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| L1        | Error recovery after token failures                       | M  | ✓      |              | PLANNING.md § L1              |
| A2        | Logger: hot-reload, run-mode, release + debug             | M  | ✓      |              | LOGGER.md                     |
| A2.1      | ↳ Wire hot-reload in log functions                        | S  | ✓      |              | native.rs                     |
| A2.2      | ↳ `is_production()` + `is_debug()` + `RunMode`            | S  | ✓      |              | 01_code.loft                  |
| A2.3      | ↳ `--release` flag + `debug_assert()` elision             | MH | ✓      | A2.2         | control.rs, main.rs           |
| A2.4      | ↳ `--debug` per-type safety logging                       | M  | ✓      | A2.2         | fill.rs, native.rs            |
| C52       | Stdlib name clash: warning + `std::` prefix               | M  | ✓      |              | PLANNING.md § C52             |
| C53       | Match arms: library enums + bare variant names            | M  | ✓      |              | PLANNING.md § C53             |
| W-warn    | Developer warnings (Clippy-inspired)                      | M  | —      |              | see below                     |
| AOT       | Auto-compile libraries to native shared libs for interpreter | — | ✓  |              | PLANNING.md § AOT             |
| AOT.1     | ↳ Detect stale rlib (mtime check src/ vs cached lib)      | S  | ✓      |              | native_utils.rs               |
| AOT.2     | ↳ Generate Rust from library .loft (reuse --native-emit)  | M  | ✓      | AOT.1        | generation/mod.rs             |
| AOT.3     | ↳ Compile rlib with rustc + link against libloft.rlib     | S  | ✓      | AOT.2        | native_utils.rs               |
| AOT.4     | ↳ dlopen compiled rlib in interpreter at runtime          | M  | ✓      | AOT.3, PKG.1 | extensions.rs                 |
| AOT.5     | ↳ Fallback to bytecode when rustc unavailable             | S  | ✓      | AOT.4        | compile.rs                    |
| P2        | REPL / interactive mode                                    | —  | ✓      | L1           | PLANNING.md § P2              |
| P2.1      | ↳ Input completeness detection                            | S  | ✓      |              | new repl.rs                   |
| P2.2      | ↳ Single-statement execution                              | M  | ✓      | P2.1         | main.rs, repl.rs              |
| P2.3      | ↳ Automatic value output                                  | S  | ✓      | P2.2         | repl.rs                       |
| P2.4      | ↳ Error recovery in session                               | M  | ✓      | P2.2, L1     | repl.rs, parser.rs            |

### W-warn — Developer warnings

Loft currently emits 6 warnings.  The following additional warnings would
catch common mistakes earlier, inspired by Rust's compiler and Clippy:

**Existing warnings (0.8.3):**
- Dead assignment — variable overwritten before read
- Variable never read — assigned but never used
- Parameter never read — function parameter unused
- Unreachable code — code after return/break/continue
- Format specifier mismatch — `:x` on text, `:05` on boolean
- Unnecessary const — `const` parameter never modified

**Proposed new warnings (0.9.0):**

| Warning | Rust/Clippy equivalent | Example |
|---------|----------------------|---------|
| Comparison always true/false | `clippy::absurd_extreme_comparisons` | `x >= 0` when x is `integer not null` |
| Unnecessary parentheses | `clippy::unnecessary_parens` | `if (x > 0) { ... }` |
| Empty loop/if body | `clippy::empty_loop` | `for x in v { }` |
| Single-element vector literal | `clippy::single_element_loop` | `for x in [42] { ... }` |
| Shadowed variable in same scope | `clippy::shadow_unrelated` | `x = 1; x = "hello"` (type change) |
| Unused import | `unused_imports` | `use lib;` but no `lib::` references |
| Identical if/else branches | `clippy::if_same_then_else` | `if c { x } else { x }` |
| Division by literal zero | compile-time div-by-zero | `x / 0` |
| Infinite loop without break | `clippy::empty_loop` | `while true { }` without break |
| Stdlib name shadow | C52 | `fn len(t: text)` shadows stdlib |

---

## 1.0.0 — IDE + stability contract

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| W2        | Editor shell (CodeMirror 6 + Loft grammar)                | M  | ✓      | W1           | WEB_IDE.md M2                 |
| W3        | Symbol navigation (go-to-def, find-usages)                | M  | ✓      | W1, W2       | WEB_IDE.md M3                 |
| W4        | Multi-file projects (IndexedDB)                           | M  | ✓      | W2           | WEB_IDE.md M4                 |
| W5        | Docs & examples browser                                    | —  | ✓      | W2           | WEB_IDE.md M5                 |
| W5.1      | ↳ Markdown renderer for doc comments                      | S  | ✓      | W2           | web/docs.ts                   |
| W5.2      | ↳ Sidebar tree: stdlib types + functions                  | M  | ✓      | W5.1         | web/docs.ts                   |
| W5.3      | ↳ Runnable examples (inline editor + run button)          | M  | ✓      | W5.2         | web/docs.ts + web/runner.ts   |
| W6        | Export/import ZIP + PWA offline                             | —  | ✓      | W4           | WEB_IDE.md M6                 |
| W6.1      | ↳ ZIP export: serialize project files to .zip             | S  | ✓      | W4           | web/export.ts                 |
| W6.2      | ↳ ZIP import: unpack .zip into IndexedDB project          | S  | ✓      | W4           | web/export.ts                 |
| W6.3      | ↳ Service worker + manifest.json for offline PWA          | M  | ✓      | W6.1         | web/sw.ts + manifest.json     |

_W2 and W4 can be developed in parallel after W1; W3 and W5 can follow independently._

---

## 1.1+ — Backlog

| ID        | Title                                                     | E  | Design | Depends on   | Source                        |
|-----------|-----------------------------------------------------------|----|--------|--------------|-------------------------------|
| W1.18-6   | Remove `19-threading.loft` from `WASM_SKIP`               | S  | ✓      | W1.18-5      | tests/wrap.rs                 |
| W1.14     | WASM Tier 2: Web Worker pool; `par()` parallelism         | VH | ✓      | W1.18        | WASM.md — Threading           |
| I12       | Interfaces: factory methods (`fn zero() -> Self`) — phase 2 | S | ✓    | I5.1         | INTERFACES.md § Q4/Q6         |
| I8.5      | Interfaces: left-side concrete operand (`concrete op T`)  | S  | ~      | I8.3         | INTERFACES.md § Phase 1 gaps  |
| A12       | Lazy work-variable initialization                         | M  | ✓      |              | PLANNING.md § A12             |
| O2        | Stack raw pointer cache                                    | —  | ✓      |              | PLANNING.md § O2              |
| O2.1      | ↳ Replace Vec<u8> stack with raw *mut u8 + capacity       | M  | ✓      |              | state/mod.rs                  |
| O2.2      | ↳ Cached stack pointer in execute loop (avoid bounds chk) | S  | ✓      | O2.1         | state/mod.rs                  |
| O2.3      | ↳ Benchmark: measure dispatch overhead reduction          | S  | ✓      | O2.2         | benches/                      |
| A4        | Spatial index operations                                   | —  | ✓      |              | PLANNING.md § A4              |
| A4.1      | ↳ Insert + exact lookup                                   | M  | ✓      |              | PLANNING.md § A4 Phase 1      |
| A4.2      | ↳ Bounding-box range query                                | M  | ✓      | A4.1         | PLANNING.md § A4 Phase 2      |
| A4.3      | ↳ Removal                                                 | S  | ✓      | A4.1         | PLANNING.md § A4 Phase 3      |
| A4.4      | ↳ Full iteration                                          | S  | ✓      | A4.2, A4.3   | PLANNING.md § A4 Phase 4      |
| O4        | Native: direct-emit local collections                      | —  | ✓      |              | PLANNING.md § O4              |
| O4.1      | ↳ Detect pure-local vectors (no store escape)             | M  | ✓      |              | generation/mod.rs             |
| O4.2      | ↳ Emit Rust Vec<T> instead of store-backed vector         | M  | ✓      | O4.1         | generation/dispatch.rs        |
| O4.3      | ↳ Emit Rust struct instead of store-backed record         | M  | ✓      | O4.2         | generation/dispatch.rs        |
| O5        | Native: omit `stores` from pure functions                  | —  | ✓      | O4           | PLANNING.md § O5              |
| O5.1      | ↳ Purity analysis: mark functions that don't touch stores | M  | ✓      |              | scopes.rs, generation/mod.rs  |
| O5.2      | ↳ Emit pure functions without `&mut Stores` parameter     | M  | ✓      | O5.1         | generation/mod.rs             |
| O5.3      | ↳ Inline small pure functions at call sites               | M  | ✓      | O5.2         | generation/mod.rs             |

---

## Deferred indefinitely

| ID    | Title                                                     | E  | Notes                                                              |
|-------|-----------------------------------------------------------|----|-------------------------------------------------------------------|
| O1    | Superinstruction peephole rewriting                       | M  | Blocked: opcode table full (254/256 used); requires opcode-space redesign |
| P4    | Bytecode cache (`.loftc`)                                 | M  | Superseded by native codegen                                       |
| A7.4  | Package registry (central, `loft install <url>`)          | M  | 2.x; PKG system ships first; ecosystem must exist                  |

---

## See also

- [PLANNING.md](PLANNING.md) — Full descriptions, fix paths, and effort justifications for every item
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark data and designs for O1–O7
- [DEVELOPMENT.md](DEVELOPMENT.md) — Branch naming, commit sequence, and CI workflow
- [RELEASE.md](RELEASE.md) — Gate criteria each milestone must satisfy before tagging
