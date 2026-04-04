---
render_with_liquid: false
---
# OpenGL / Drawing Library — Implementation Checklist

Ordered implementation steps, each small enough to write and verify independently.
Priority: **2D canvas → GLB file export → OpenGL desktop → WebGL browser**.

GLB is tackled first because it needs no GPU, no window, and no platform SDK — just
file I/O, which makes every step scriptable and diff-able.

### Why this project matters for loft optimization

This library is a **real-world performance target** for the loft interpreter.  The
workloads are chosen specifically because they stress areas where interpreters struggle:

| Workload | Stress pattern |
|---|---|
| `wu_line` on 1024×1024 canvas | Inner loop over ~2M pixel blends; integer arithmetic at high frequency |
| Bezier subdivision | Recursive float calls; many small struct allocations |
| `scanline_fill` | Per-row vector append + sort + fill; tests collection performance |
| GLB BIN assembly | Large `vector<u8>` append in a tight loop; tests vector growth |
| Matrix math per frame | Dense float arithmetic; tests `single` operation throughput |
| `draw_text_box` wrapping | String iteration + repeated `measure_text` calls |

Each phase should be **timed and recorded in [PERFORMANCE.md](PERFORMANCE.md)** when
completed.  If a phase is unacceptably slow, the bottleneck is identified and fixed in
the interpreter before the next phase begins — this is the primary mechanism for
ensuring loft is fast enough for real compute workloads.

---

## Implementation status

**Phases 0–2 were implemented as a pure-loft package** in `lib/graphics/` rather than
as Rust-native code in `src/drawing.rs`.  The design doc below was the original plan;
the actual implementation diverged:

| Design step | Actual implementation | Location |
|---|---|---|
| Phase 0 scaffolding | `lib/graphics/loft.toml` + `src/graphics.loft` | `lib/graphics/` |
| Phase 1 Canvas | `Canvas` struct, `rgba()`/`rgb()`, `get/set_pixel`, `clear`, `blend` | `graphics.loft` |
| Phase 2.1–2.3 lines/rect | `hline`, `vline`, `fill_rect`, `draw_rect` | `graphics.loft` |
| Phase 2.4 Bresenham | `draw_line()` — pure loft, all octants | `graphics.loft` |
| Phase 2.6 circle | `draw_circle()`, `fill_circle()` — midpoint algorithm | `graphics.loft` |
| Phase 2.8–2.9 ellipse | `draw_ellipse()` — midpoint two-region | `graphics.loft` |
| Phase 2.5 AA line | `draw_aa_line()` — Wu algorithm with blend_pixel | `graphics.loft` |
| Phase 2.7–2.9 Bezier | `draw_bezier()` — adaptive de Casteljau subdivision | `graphics.loft` |
| Phase 2.10 fill | `fill_triangle()` — scanline with vertex sort | `graphics.loft` |
| fill_ellipse | `fill_ellipse()` — midpoint + hline | `graphics.loft` |
| Phase 2.11 AA fill boundary | Not yet implemented |  |
| Phase 2.13 Dashed/Dotted | Not yet implemented |  |
| Phase 4.1–4.3 3D types | `math.loft` (Vec2/3/4, Mat4), `mesh.loft` (Vertex/Triangle/Mesh), `scene.loft` (Material/Node/Camera/Scene) | `lib/graphics/src/` |
| GL5 single-mesh GLB | `glb::save_glb(mesh, path)` — binary GLB 2.0 writer | `glb.loft` |
| GL6 scene GLB | `glb::save_scene_glb(scene, path)` — multi-mesh with materials | `glb.loft` |
| GL7 glTF compliance | material on primitive (not node); node transform matrix | `glb.loft`, `scene.loft` |
| GL8 mesh primitives | Complete 6-face cube; `plane(w, d)`; `sphere(r, slices, stacks)` | `mesh.loft` |

Tests:
- `lib/graphics/tests/canvas.loft` — 30 canvas tests. Run: `cargo run --bin loft -- --lib lib/graphics/src --tests lib/graphics/tests/canvas.loft`
- `lib/graphics/tests/math.loft` — 9 math tests.
- `lib/graphics/tests/mesh.loft` — 6 mesh tests (cube, plane, sphere).
- `lib/graphics/tests/scene.loft` — 5 scene graph tests.
- `lib/graphics/tests/glb.loft` — 5 single-mesh GLB tests.
- `lib/graphics/tests/scene_glb.loft` — 9 scene GLB tests (binary structure + JSON content).

The original Rust-native design is preserved below for reference.  Future phases (text,
OpenGL) will likely need native extensions via the `#native` mechanism.

---

## Phase 0 — Project scaffolding (original design)

**0.1** Create `lib/graphics/` with empty placeholder files:
`draw.loft`, `mesh.loft`, `scene.loft`, `texture.loft`, `text.loft`,
`opengl.loft`, `webgl.loft`, `glb.loft`.
_Test: `cargo build` still passes._

**0.2** Add `src/drawing.rs` as an empty module stub; register it in `src/lib.rs`.
_Test: `cargo build` compiles._

**0.3** Add `fontdue = "0.8"` to `Cargo.toml` (unconditional — used by all backends).
_Test: `cargo build` resolves the crate._

**0.4** Add `src/glb.rs` as an empty module stub with a `pub fn write_scene` placeholder
that returns `Err("not implemented")`.
_Test: `cargo build` compiles._

**0.5** Create `tests/graphics/` with an empty `mod.rs` and a single `#[test] fn
placeholder() {}`.
_Test: `cargo test tests::graphics` passes._

---

## Phase 1 — Canvas (2D pixel buffer)

**1.1** Define `Rgba` struct in `draw.loft` with fields `r, g, b, a: u8 not null`.
_Test (loft): construct `Rgba{r:255,g:0,b:0,a:255}`; assert each field equals its value._

**1.2** Define `Canvas` struct with `canvas_size: integer not null` and
`canvas_data: vector<Rgba>`.
_Test: construct `Canvas{canvas_size: 4}` and assert `canvas_size == 4`._

**1.3** Implement `canvas(size: integer) -> Canvas` — fills `canvas_data` with `size*size`
transparent pixels.
_Test: `canvas(4).canvas_data` has length 16; every pixel has `a == 0`._

**1.4** Implement `pixel_at(self: Canvas, x: integer, y: integer) -> Rgba` —
returns transparent on out-of-bounds.
_Test: `canvas(4).pixel_at(10, 10).a == 0`; `canvas(4).pixel_at(-1, 0).a == 0`._

**1.5** Implement `set_pixel(self: &Canvas, x: integer, y: integer, color: Rgba)` —
no-op out-of-bounds.
_Test: set then read back `(1, 2)` matches; `(99, 99)` does not panic._

**1.6** Implement `blend_pixel` (Porter-Duff src-over) in loft.
_Test: blend opaque red over opaque blue → `r == 255, b == 0`.
Test: blend `a=128` red over white → `r > 200, b > 100` (partial mix)._

**1.7** Implement `save_png` as a native op backed by the existing PNG writer in
`src/state/io.rs` (adapt `write_png` to accept a canvas pixel slice).
_Test: write `out/test_canvas.png`; reload with `file("out/test_canvas.png").png()`;
check pixel `(0, 0)` round-trips correctly._

---

## Phase 2 — Vector path rasterization

**2.1** Define `Xy` struct (`x: single not null, y: single not null`) and
`LineStyle` enum (`Straight, Dashed, Dotted`) in `draw.loft`.
_Test: construct each variant._

**2.2** Define `PathNode` enum with only the `Point` variant for now:
`Point { coord: Xy, color: Rgba, size: single }`.
_Test: construct and match on a `Point` node._

**2.3** Define `Draw` struct (`nodes: vector<PathNode>, fill: Rgba, line: LineStyle`).
_Test: build a two-node `Draw` and assert `len(nodes) == 2`._

**2.4** In Rust (`src/drawing.rs`): implement integer Bresenham line rasterizer between
two canvas points. No anti-aliasing yet.
_Test (Rust unit): horizontal line from `(0,0)` to `(7,0)` sets exactly 8 pixels._

**2.5** Add `pub fn draw(self: &Canvas, path: Draw)` as a native op calling `render_path`.
Wire only the `Straight` two-point case.
_Test (loft): draw a red horizontal line; check pixel at the midpoint is red._

**2.6** Upgrade to Xiaolin Wu anti-aliased line rasterizer.
_Test: diagonal line has non-zero-alpha pixels on both sides of the ideal line._

**2.7** Add `Curve` variant to `PathNode`:
`Curve { coord: Xy, color: Rgba, size: single, in_handle: Xy, out_handle: Xy }`.
_Test: construct a `Curve` node; pattern-match on it._

**2.8** Implement adaptive cubic Bezier subdivision in Rust (split until flatness
threshold, then Bresenham the segments).
_Test (Rust unit): curve from `(0,0)` to `(100,0)` with symmetric up-handles passes
through its midpoint at approximately `(50, max_deflection)` — pixel row not empty._

**2.9** Extend `render_path` to handle `Curve` nodes.
_Test (loft): draw a two-node Bezier; save PNG; verify pixels appear along the arc._

**2.10** Implement scanline fill in Rust: collect edge crossings per row, sort, fill
between pairs.
_Test (Rust unit): square path `(0,0)→(10,0)→(10,10)→(0,10)→(0,0)` fills all 121
interior pixels._

**2.11** Anti-alias fill boundary: pixels on the path boundary get partial alpha
proportional to coverage fraction.
_Test: boundary pixel alpha is strictly between 0 and 255._

**2.12** Wire fill into `render_path` when `fill.a > 0`.
_Test (loft): filled red circle has solid pixels at centre, soft edge at rim._

**2.13** Implement `Dashed` and `Dotted` stroke patterns (segment length table in Rust).
_Test (loft): dashed line has alternating drawn and skipped segments visible in PNG._

---

## Phase 3 — 2D primitives

**3.1** Implement `rect(x, y, w, h, fill, stroke, stroke_size, style) -> Draw` in Rust.
_Test (loft): filled blue rect; pixel at `(x+1, y+1)` is blue; pixel at `(x-1, y)` is
transparent._

**3.2** Implement `ellipse(cx, cy, rx, ry, fill, stroke, stroke_size, style) -> Draw`.
_Test: centre pixel has fill colour; pixel at `(cx + rx + 5, cy)` is transparent._

**3.3** Implement `rounded_rect(x, y, w, h, r, fill, stroke, stroke_size, style) -> Draw`.
_Test: corner pixels at `(x, y)` are anti-aliased (alpha < 255, > 0); centre is solid._

**3.4** Implement `arrow(ax, ay, bx, by, head_size, fill, stroke, stroke_size, style) -> Draw`.
_Test: pixels exist at both the shaft midpoint and the arrowhead cluster near `(bx, by)`._

---

## Phase 4 — Text rasterization

**4.1** In Rust: load a TTF from bytes using `fontdue::Font::from_bytes`; rasterize the
glyph `'A'` at 24 px; assert coverage bitmap is non-empty.
_Test (Rust unit test in `src/drawing.rs`)._

**4.2** Add a global font registry (`Vec<fontdue::Font>`) to the interpreter state;
implement `load_font(path: text) -> Font` as a native op.
_Test (loft): `load_font("assets/Inter-Regular.ttf").font_id >= 0`._

**4.3** Bundle a minimal fallback font (Noto Sans Latin subset) via `include_bytes!`;
implement `default_font() -> Font` — always succeeds.
_Test (loft): `default_font().font_id >= 0` with no font files present._

**4.4** Define `TextStyle`, `TextAlign`, `TextBaseline`, `TextMetrics` structs in
`text.loft`.
_Test: construct a `TextStyle` and read back `text_size`._

**4.5** Implement `measure_text(str: text, style: TextStyle) -> TextMetrics` in Rust
(sum advances via `fontdue::Font::metrics`, no rasterization).
_Test (loft): `measure_text("Hi", style).tm_width > 0`;
`measure_text("", style).tm_width == 0`._

**4.6** Implement `draw_text` for a single character — rasterize glyph, composite each
coverage byte onto the canvas via `blend_pixel`.
_Test: single `'X'` drawn at `(50, 50)` — at least one pixel in that region has `a > 0`._

**4.7** Extend `draw_text` to multi-character strings with correct horizontal advance.
_Test: `draw_text("AB", …)` — pixels for `'B'` are strictly to the right of pixels for `'A'`._

**4.8** Implement `TextAlign.Center` and `TextAlign.Right` offset calculation.
_Test: centred text at `x=256` on a 512 canvas — leftmost and rightmost glyph pixels
are approximately equidistant from 256._

**4.9** Implement `TextBaseline.Middle` and `TextBaseline.Bottom` vertical offsets.
_Test: middle-baseline text at `y=100` — pixels straddle row 100._

**4.10** Implement `text_spacing` extra advance.
_Test: `text_spacing=10` gives `tm_width` at least 10*(len-1) wider than `text_spacing=0`._

**4.11** Implement `draw_text_box` — call `measure_text` per word, wrap when cumulative
width exceeds `bw`, clip when cumulative height exceeds `bh`.
_Test: long string that does not fit on one line — pixels on two distinct horizontal
bands._

---

## Phase 5 — GLB mesh types (loft structs)

**5.1** Define `Vec2 { u, v }`, `Vec3 { vx, vy, vz }`, `Vec4 { qx, qy, qz, qw }` in
`mesh.loft`.
_Test: construct each; assert field round-trips._

**5.2** Define `Vertex`, `Triangle`, `Mesh` in `mesh.loft`.
_Test: build a single-triangle mesh (3 vertices, 1 triangle); assert `len(mesh_triangles) == 1`._

**5.3** Define `Transform` and `identity_transform()` in `scene.loft`.
_Test: `identity_transform().sx == 1.0f`; `identity_transform().tx == 0.0f`._

**5.4** Define `Camera` and `Light` structs.
_Test: construct with explicit fields; read back `cam_fov`._

**5.5** Define `MeshInstance` and `Scene`.
_Test: build a scene with one mesh, one material, one instance, one light, one camera._

---

## Phase 6 — GLB binary writer (Rust)

**6.1** Rust: implement `serialize_vertices(vertices: &[Vertex]) -> Vec<u8>` — interleaved
`pos f32×3 | normal f32×3 | uv f32×2 | color u8×4` = 36 bytes/vertex.
_Test (Rust unit): 2 vertices → 72 bytes; byte 0..12 match first position._

**6.2** Rust: implement `serialize_indices(triangles: &[Triangle]) -> Vec<u8>` — 3×`u32`
= 12 bytes/triangle.
_Test: 2 triangles → 24 bytes; first 4 bytes equal index `ia` as little-endian u32._

**6.3** Rust: implement `build_accessor_json(buffer_view, count, component_type,
type_str, min, max) -> serde_json::Value`.
_Test: resulting JSON has `"count"` matching input._

**6.4** Rust: implement `build_mesh_json(mesh_idx, accessor_indices) -> Value`.
_Test: JSON contains `"primitives"` array with correct accessor references._

**6.5** Rust: implement `build_node_json(mesh_idx, transform) -> Value` — decompose
`Transform` into `translation`, `rotation` (quaternion from Euler XYZ), `scale`.
_Test: identity transform → `"translation":[0,0,0]`, `"scale":[1,1,1]`._

**6.6** Rust: Euler XYZ → quaternion conversion.
_Test: `(0,0,0)` → `(0,0,0,1)`; `(π/2, 0, 0)` → `(0.707,0,0,0.707)` (±0.001)._

**6.7** Rust: implement `build_camera_json(camera) -> Value`.
_Test: perspective camera JSON contains `"yfov"` field._

**6.8** Rust: implement `build_material_json(material, texture_idx) -> Value`.
_Test: material with roughness=0.5 has `"roughnessFactor": 0.5`._

**6.9** Rust: implement `canvas_to_png_bytes(canvas) -> Vec<u8>` — encode RGBA pixels
as in-memory PNG without writing a file.
_Test: decode result with the PNG decoder; pixel `(0,0)` matches input `Rgba`._

**6.10** Rust: implement `build_image_json(buffer_view_idx) -> Value` and
`build_texture_json(image_idx) -> Value`.
_Test: image JSON has `"mimeType":"image/png"`._

**6.11** Rust: implement `build_buffer_view_json(offset, length, target) -> Value`.
_Test: `offset + length` equals next view's offset for two consecutive views._

**6.12** Rust: assemble the full glTF JSON object (`asset`, `scene`, `scenes`, `nodes`,
`meshes`, `accessors`, `bufferViews`, `buffers`, `materials`, `textures`, `images`,
`cameras`).
_Test: resulting JSON string parses without error; `"asset"."version" == "2.0"`._

**6.13** Rust: write GLB binary — magic `0x46546C67`, version `2`, total length,
JSON chunk (`0x4E4F534A`), BIN chunk (`0x004E4942`) with correct lengths and padding.
_Test (Rust unit): first 4 bytes of output are `glTF`; `u32` at offset 8 equals file size._

**6.14** Wire `save_glb(scene, path) -> FileResult` as a loft native op.
_Test (loft): after call, `file("out/test.glb").size > 0`._

**6.15** Run `gltf-validator out/test.glb` (triangle mesh, no texture).
_Test: zero errors, zero warnings._

**6.16** Export a unit cube mesh (12 triangles) with a solid-colour canvas texture.
_Test: `gltf-validator` passes; open in Blender — geometry and texture visible._

**6.17** Export a scene with three instances of the same mesh at different transforms.
_Test: `gltf-validator` passes; Blender shows three distinct positions._

**6.18** Implement `save_mesh_glb(mesh, mat, xf, path) -> FileResult` — single-object
shortcut.
_Test: output is valid GLB with exactly one mesh node._

---

## Phase 7 — GLB with canvas and text textures

**7.1** Define `TextureSource`, `Material` structs in `texture.loft`.
_Test: construct `Material` with a `Drawing` source; read back `mat_roughness`._

**7.2** In Rust: route `TextureSource.Drawing` through `canvas_to_png_bytes` and embed
in the GLB BIN chunk.
_Test: material with canvas texture → validator passes; texture appears in Blender._

**7.3** In Rust: route `TextureSource.Image` by loading the PNG file and embedding its
raw bytes.
_Test: material with PNG file source → validator passes._

**7.4** Draw text onto a canvas, use it as a GLB material texture.
_Test: exported GLB shows readable text on the mesh surface in Blender._

---

## Phase 8 — OpenGL desktop

**8.1** Add `opengl` feature to `Cargo.toml`; gate `glfw` and `gl` behind it.
_Test: `cargo build --features opengl` succeeds; headless build without feature also succeeds._

**8.2** Implement `init_renderer(Backend.OpenGl, w, h)` — create GLFW window and GL 3.3
core context; store in a thread-local renderer state.
_Test: window appears, `poll_events()` returns true, `shutdown()` closes it without crash._

**8.3** Compile vertex shader (position + normal + uv + vertex colour, MVP uniforms).
_Test: `glGetShaderiv(GL_COMPILE_STATUS)` == `GL_TRUE`; no GL error._

**8.4** Compile fragment shader (texture sample × vertex colour × N-dot-L diffuse + ambient).
_Test: link succeeds; `glGetProgramiv(GL_LINK_STATUS)` == `GL_TRUE`._

**8.5** Implement `upload_mesh(mesh) -> integer` — allocate VAO + VBO + EBO from
`Mesh` data; return handle.
_Test: handle > 0; `glGetError()` == `GL_NO_ERROR` after call._

**8.6** Implement `upload_canvas(canvas) -> integer` — `glTexImage2D` with `GL_RGBA8`
from `canvas_data`.
_Test: handle > 0; no GL error._

**8.7** Implement `upload_png(path) -> integer` — load PNG via existing decoder, upload.
_Test: valid path → handle > 0; bad path → returns null._

**8.8** Implement `release_texture(id)` — `glDeleteTextures`.
_Test: no GL error after release._

**8.9** Compute view matrix from `Camera` (look-at: eye, target, up).
_Test (Rust unit): camera at `(0,0,5)` looking at origin → correct 4×4 matrix values._

**8.10** Compute perspective projection from `cam_fov`, `cam_near`, `cam_far`, aspect ratio.
_Test (Rust unit): standard 90° FOV matrix matches known values._

**8.11** Compute TRS model matrix from `Transform` (same quaternion from step 6.6).
_Test (Rust unit): identity transform → identity 4×4 matrix._

**8.12** Implement `render(scene)` — for each instance: bind VAO + texture, upload MVP
matrix and light uniforms, `glDrawElements`, swap buffers.
_Test: coloured triangle visible in window._

**8.13** Verify canvas-texture material renders correctly in the GL window.
_Test: box mesh with drawn texture shows texture._

**8.14** Implement `poll_events() -> boolean` — GLFW event pump; return false when close
button pressed.
_Test: pressing the window close button exits the render loop._

**8.15** Implement `shutdown()` — destroy GL objects, close GLFW.
_Test: no sanitizer / GL debug callback errors at shutdown._

---

## Phase 9 — WebGL browser

**9.1** Add `webgl` feature; add `web-sys` (with `WebGl2RenderingContext`, `HtmlCanvasElement`)
and `wasm-bindgen` dependencies.
_Test: `cargo build --target wasm32-unknown-unknown --features webgl` succeeds._

**9.2** Add `wasm-pack` build script (`pkg/`); produce `index.html` that loads the WASM
and calls a `loft_main()` export.
_Test: `wasm-pack build --target web` produces `pkg/loft_bg.wasm`._

**9.3** Implement WebGL2 context acquisition from `document.getElementById("loft")`.
_Test: no JS console errors on page load._

**9.4** Port `upload_mesh` to WebGL2 — `createBuffer`, `bufferData` with the same
interleaved vertex layout as GLB/OpenGL.
_Test: no WebGL error after call (`getError() == NO_ERROR`)._

**9.5** Port shaders to GLSL ES 3.0 — add `#version 300 es` and
`precision mediump float;`; input qualifiers `in`/`out` replace `attribute`/`varying`.
_Test: `compileShader` succeeds for both vertex and fragment shaders._

**9.6** Port `upload_canvas` to WebGL2 — `texImage2D` with RGBA byte array.
_Test: no WebGL error._

**9.7** Port MVP + light uniform upload.
_Test: uniforms present (`getUniformLocation` non-null for each)._

**9.8** Port `render(scene)` draw loop using `requestAnimationFrame` via
`wasm_bindgen::closure::Closure`.
_Test: triangle visible in browser window._

**9.9** Port `poll_events()` — use a `js_sys::AtomicBool` flag set by a JS `beforeunload`
listener; return false when set.
_Test: page close / escape key exits the frame loop._

**9.10** Cross-browser smoke test: same scene renders identically in Firefox and Chrome.
_Test: screenshot comparison of both browsers against the OpenGL reference frame._
