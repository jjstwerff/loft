
# Web Example Gallery & Unified Rendering Design

Design for presenting loft graphics examples as interactive web pages and
unifying the rendering backend across native OpenGL, WebGL, and GLB export.

---

## Project goals

### Primary: browser games for the wider public

Loft's main graphics goal is enabling **small games that run in a browser**.
The audience is the general public — people who click a link and immediately
play, with no install, no toolchain, no GPU driver.  WebGL2 in every modern
browser is the delivery platform.

This means:

- **WebGL is the first-class target.**  Every rendering feature must work in
  the browser.  If something works natively but not in WebGL, it is incomplete.
- **The game loop, input, audio, and rendering all run in WASM+WebGL.**
  The loft interpreter (or native-compiled WASM) drives the game; the browser
  provides the display surface and input events.
- **Share and play in one click.**  A loft game compiles to a single `.html`
  page (or a small set of static files) that anyone can host on GitHub Pages,
  itch.io, or any web server.

### Secondary: native OpenGL for enthusiasts

Desktop rendering via native OpenGL is supported for developers who want:

- **Low-latency, full-screen rendering** without browser overhead.
- **LearnOpenGL-style tutorials** that teach graphics programming concepts
  with direct GL access (the existing 23 examples serve this role).
- **Offline tools** — headless GLB export, procedural texture generation,
  batch rendering pipelines.

Native OpenGL is not a prerequisite for browser games.  The two paths share
the same loft-level API but compile to different backends.

### Design consequence

Every abstraction is designed **WebGL-first, native-compatible**:

| Decision | Rationale |
|---|---|
| Shader version patching (330 → 300 es) | One source, both targets |
| No GL extensions beyond WebGL2 core | Browser must work |
| Frame loop via `render.frame()` not `while true` | Maps to `requestAnimationFrame` |
| Input via `gl.key_pressed()` not OS-specific APIs | Maps to DOM events |
| Audio via a future `audio.loft` module | Maps to Web Audio API |
| Asset loading via virtual filesystem | Maps to fetch + IndexedDB |

---

## Contents

- [Example gallery](#example-gallery)
- [Unified rendering abstraction](#unified-rendering-abstraction)
- [Scene-level API](#scene-level-api)
- [Low-level GL abstraction](#low-level-gl-abstraction)
- [Backend implementations](#backend-implementations)
- [Migration path](#migration-path)
- [Implementation plan](#implementation-plan)

---

## Motivation

### Problem 1: Examples are invisible

The 23 graphics examples only run on machines with a display, a Rust toolchain,
and the native graphics library compiled.  There is no way for a casual visitor
to see what loft can do without cloning the repo and building everything.

### Problem 2: Three rendering paths with no shared abstraction

| Path | API | Runs on | State |
|---|---|---|---|
| GLB export | `glb::save_scene_glb()` | headless / any viewer | working |
| Native OpenGL | `gl_create_shader`, `gl_draw`, etc. | desktop with GPU | working |
| WebGL | — | browser | not started |

Each path has its own calling convention.  A program that renders live on
desktop cannot export a GLB, and neither can run in a browser.  Adding WebGL
as a third independent path would triple the maintenance burden.

### Solution

1. A **web gallery** that presents each example as a page with source, description,
   and either a live WebGL canvas or a static screenshot.
2. A **unified rendering abstraction** (`render.loft`) that sits above all three
   backends, so one program can render natively, in WebGL, or export GLB with
   no code changes.

---

## Example gallery

### Structure

```
docs/examples/
  index.html              — gallery index with thumbnails and descriptions
  01-hello-window.html    — per-example page
  02-hello-triangle.html
  ...
  assets/
    screenshots/          — PNG screenshots of each example (generated offline)
    style.css             — shared gallery styling
    gallery.js            — thumbnail grid, search, category filter
```

### Per-example page layout

```
┌─────────────────────────────────────────────────┐
│  ← Back to Gallery          01 - Hello Window   │
├────────────────────────┬────────────────────────┤
│                        │                        │
│   Live WebGL canvas    │   Source code           │
│   (or screenshot if    │   (syntax highlighted,  │
│    no WebGL support)   │    scrollable)          │
│                        │                        │
├────────────────────────┴────────────────────────┤
│  Description (from file header comments)        │
│  Loft constructions used (from header)          │
│  Controls: [Run] [Stop] [Reset] [Export GLB]    │
└─────────────────────────────────────────────────┘
```

### Generation

A build script (`scripts/build-gallery.py` or `.loft`) reads each example file,
extracts the header documentation, generates the HTML pages, and optionally
captures screenshots via headless rendering.

For examples that use the unified rendering API (see below), the WebGL canvas
is live — the loft WASM runtime executes the example in the browser.  For
examples that use low-level GL calls directly, a static screenshot is shown
with a note that native execution is required.

### Index page

Grid of cards, one per example.  Each card shows:
- Thumbnail (screenshot or placeholder)
- Number and title
- One-line description
- Category badge (basics / lighting / textures / advanced / scene)

Categories are derived from the example number ranges:
- 01-04: Basics (window, triangle, shaders, textures)
- 05-09: Transforms & Lighting
- 10: 2D Canvas
- 11: Scene Graph / GLB
- 12-16: Advanced Rendering (lights, depth, blending, culling, shadows)
- 17-19: Post-Processing, PBR, Complete Scene
- 20-23: Textures, Input, Wireframe, Cleanup

---

## Unified rendering abstraction

### Design principles

1. **One program, three outputs** — the same scene description renders natively,
   in WebGL, or exports to GLB without code changes.
2. **Progressive disclosure** — simple scenes use `render.render_loop(scene)`;
   custom effects drop down to `gl.*` calls that work on both native and WebGL.
3. **No shader code in user programs** — built-in PBR shader handles materials,
   lights, and shadows.  Custom shaders are opt-in for advanced users.
4. **Canvas-first for 2D** — 2D drawing stays on `Canvas` with `draw_text`,
   `fill_rect`, etc.  Canvases upload to textures via `gl_upload_canvas`.

### Architecture

```
User code
    │
    ├── Scene API (scene.loft)        ← declarative: meshes, materials, lights
    │       │
    │       ▼
    ├── Renderer (render.loft)        ← drives the render loop, owns shaders/FBOs
    │       │
    │       ▼
    ├── GL abstraction (gl.loft)      ← thin wrapper: same API for native + WebGL
    │       │
    │       ├── Native backend         ← glutin + gl crate (existing)
    │       └── WebGL backend          ← web-sys WebGl2RenderingContext
    │
    └── GLB export (glb.loft)         ← file output, no GPU needed
```

---

## Scene-level API

Reuses the existing types from `scene.loft` — no new types needed.

```loft
use scene;
use render;

fn main() {
  s = scene::Scene { name: "demo" };
  // ... add meshes, materials, nodes, lights, camera ...

  // Option A: render live (native or WebGL, auto-detected)
  r = render::create(800, 600, "Demo");
  r.run(s, cam);
  r.destroy();

  // Option B: export GLB
  glb::save_scene_glb(s, "demo.glb");

  // Option C: both
  r = render::create(800, 600, "Demo");
  for _ in 0..5000 {
    if !r.frame(s, cam) { break }
  }
  r.destroy();
  glb::save_scene_glb(s, "demo.glb");
}
```

### Renderer struct

```loft
pub struct Renderer {
  width: integer not null,
  height: integer not null,
  // Internal: shader handles, shadow FBO, uploaded mesh cache
  pbr_shader: integer not null,
  shadow_shader: integer not null,
  shadow_fbo: integer not null,
  shadow_tex: integer not null,
  shadow_size: integer not null,
  mesh_vaos: vector<integer>,
  mesh_counts: vector<integer>,
  start_time: long not null
}

// Create renderer with window (native) or canvas (WebGL).
pub fn create(width: integer, height: integer, title: text) -> Renderer

// Render one frame. Returns false when close requested.
pub fn frame(self: Renderer, scene: const Scene, camera: const Camera) -> boolean

// Convenience: render loop until close.
pub fn run(self: Renderer, scene: const Scene, camera: const Camera)

// Seconds since creation.
pub fn elapsed(self: Renderer) -> float

// Destroy resources and close window.
pub fn destroy(self: Renderer)
```

### Built-in rendering pipeline

`frame()` executes:

1. **Upload** — on first call, flatten each mesh to VAO (cached in `mesh_vaos`)
2. **Shadow pass** — if scene has directional lights, render depth from light POV
   into shadow FBO using orthographic projection sized to scene bounding box
3. **Color pass** — for each node: bind material uniforms, bind mesh VAO,
   set MVP from camera, draw with PBR shader that samples shadow map
4. **Swap** — `gl_swap_buffers()` (native) or `requestAnimationFrame` return (WebGL)

---

## Low-level GL abstraction

For examples that need custom shaders or multi-pass rendering, a thin `gl`
module wraps both native OpenGL and WebGL2 behind identical loft functions.

### Current native-only functions → unified

| Function | Native impl | WebGL impl |
|---|---|---|
| `gl.create_window(w, h, title)` | glutin + winit | `document.getElementById` + `getContext("webgl2")` |
| `gl.create_shader(vert, frag)` | `glCreateProgram` | `createProgram` |
| `gl.upload_vertices(data, stride)` | `glBufferData` | `bufferData` |
| `gl.draw(vao, count)` | `glDrawArrays` | `drawArrays` |
| `gl.set_uniform_mat4(prog, name, mat)` | `glUniformMatrix4fv` | `uniformMatrix4fv` |
| `gl.set_uniform_vec3(prog, name, x,y,z)` | `glUniform3f` | `uniform3f` |
| `gl.set_uniform_float(prog, name, val)` | `glUniform1f` | `uniform1f` |
| `gl.set_uniform_int(prog, name, val)` | `glUniform1i` | `uniform1i` |
| `gl.bind_texture(tex, unit)` | `glBindTexture` | `bindTexture` |
| `gl.upload_canvas(data, w, h)` | `glTexImage2D` | `texImage2D` |
| `gl.clear(color)` | `glClear` | `clear` |
| `gl.swap_buffers()` | glutin swap | no-op (rAF handles it) |
| `gl.poll_events()` | winit poll | check close flag |
| `gl.destroy_window()` | drop context | no-op |

### Shader differences

| Feature | OpenGL 3.3 | WebGL2 (GLSL ES 3.0) |
|---|---|---|
| Version line | `#version 330 core` | `#version 300 es` |
| Precision | not needed | `precision mediump float;` required |
| Attributes | `in` | `in` (same) |

The renderer auto-prepends the correct version/precision header based on
the active backend.  User-written shaders use `#version 330 core` and the
WebGL backend patches them.

---

## Backend implementations

### Native (existing)

`lib/graphics/native/src/lib.rs` — glutin + gl crate.  Already working.
Functions registered via `#native` annotations in `graphics.loft`.

### WebGL (new)

`lib/graphics/native/src/webgl.rs` (or separate wasm-only crate).

Implementation via `web-sys`:
- `WebGl2RenderingContext` for all GL calls
- `HtmlCanvasElement` for the drawing surface
- `requestAnimationFrame` via `wasm_bindgen::closure::Closure` for the frame loop
- Keyboard/mouse events via `addEventListener` on the canvas

Compile with: `cargo build --target wasm32-unknown-unknown --features webgl`

The `#native` functions in `graphics.loft` compile to either the native or
WebGL implementation based on the target.  From loft's perspective, the API
is identical.

### GLB (existing)

`lib/graphics/src/glb.loft` — pure loft, no GPU.  Already working with
scene graph, materials, lights, and KHR_lights_punctual.

---

## Migration path

### Phase 1: Renderer layer (no WebGL yet)

Build `render.loft` on top of the existing native GL functions.  The renderer
compiles built-in PBR + shadow shaders, manages FBOs, and exposes
`create/frame/run/destroy`.  Examples 11 and 19 become ~20 lines each.

**Deliverable:** `render.loft` + updated examples using `render.run()`.

### Phase 2: Static gallery

Generate `docs/examples/index.html` and per-example pages from the .loft files.
Screenshots captured via headless rendering or manual.  Source code shown with
syntax highlighting.  No live execution yet.

**Deliverable:** Static HTML gallery deployable to GitHub Pages.

### Phase 3: WASM language examples

Wire up the existing `compile_and_run()` WASM entry point to a browser page.
The 30 language examples (tests/docs/) run live in the browser — text output
shown in a console panel.  No graphics yet.

**Deliverable:** Language example pages with live execution.

### Phase 4: WebGL backend

Implement the WebGL2 backend behind the same `gl.*` API.  The renderer and
all examples that use the unified API work in the browser.  Examples that use
raw `gl_*` calls need the version/precision patching.

**Deliverable:** Live WebGL rendering of graphics examples in the gallery.

### Phase 5: Export integration

Add [Export GLB] button to each gallery page — runs the example's scene
construction in WASM, serializes to GLB via `glb.loft`, and triggers a
browser download.

**Deliverable:** Browser-based GLB export for any scene example.

---

## Implementation plan

| Step | What | Files | Depends on |
|---|---|---|---|
| W1 | `render.loft` — Renderer struct, built-in shaders | `lib/graphics/src/render.loft` | — |
| W2 | `render.frame()` — shadow + PBR passes | `render.loft` | W1 |
| W3 | Update 11-scene-graph to use renderer | example | W2 |
| W4 | Gallery build script + index.html | `scripts/`, `docs/examples/` | — |
| W5 | Per-example HTML pages (static) | `docs/examples/` | W4 |
| W6 | Screenshot capture (headless or manual) | `docs/examples/assets/` | W5 |
| W7 | WASM build + language example runner | `ide/`, `src/wasm.rs` | — |
| W8 | WebGL2 backend (`webgl.rs`) | `lib/graphics/native/src/` | — |
| W9 | Shader version patching (330 → 300 es) | `render.loft` or native | W8 |
| W10 | Live WebGL in gallery pages | `docs/examples/` | W8, W5 |
| W11 | GLB export button in gallery | `docs/examples/` | W7 |

Steps W1-W3 deliver the renderer abstraction (desktop-only).
Steps W4-W6 deliver the static gallery (no execution).
Steps W7-W11 deliver the interactive web experience.
