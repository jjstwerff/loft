# OpenGL / WebGL / GLB Library Design

Design for a loft graphics library covering 2D RGBA drawing, 3D mesh representation,
scene management, and multi-backend rendering (desktop OpenGL, browser WebGL, GLB file export).

---

## Contents

- [Overview](#overview)
- [File layout](#file-layout)
- [2D Drawing — types](#2d-drawing--types)
- [2D Drawing — canvas](#2d-drawing--canvas)
- [2D Drawing — primitives](#2d-drawing--primitives)
- [2D Drawing — rendering](#2d-drawing--rendering)
- [3D Mesh — types](#3d-mesh--types)
- [3D Scene — types](#3d-scene--types)
- [Texture upload](#texture-upload)
- [Rendering backends](#rendering-backends)
- [GLB export](#glb-export)
- [2D Text rendering](#2d-text-rendering)
- [Rust integration notes](#rust-integration-notes)
- [Usage examples](#usage-examples)
- [Implementation constraints](#implementation-constraints)

---

## Overview

The graphics library has two independent layers:

| Layer | Purpose |
|---|---|
| **2D drawing** | Produce RGBA textures from vector paths and primitives, with anti-aliasing |
| **3D rendering** | Upload meshes and textures to a backend, manage a scene with camera |

The two layers connect at the texture boundary: a `Canvas` (2D output) can be uploaded
as a texture for a 3D mesh material.

---

## File layout

Field names must be unique across all structs within a single `.loft` file (see
[QUICK_START.md](QUICK_START.md) § Key gotchas). The graphics library therefore uses
separate files, each with its own field namespace:

```
lib/graphics/
  draw.loft      — Rgba, Xy, PathNode, Draw, LineStyle, Canvas
  mesh.loft      — Vec2, Vec3, Vec4, Vertex, Triangle, Mesh
  scene.loft     — Transform, MeshInstance, Camera, Light, Scene
  texture.loft   — Texture enum, Material
  text.loft      — Font, TextStyle, TextAlign, TextBaseline, TextMetrics
  opengl.loft    — OpGL* operator declarations + render/present functions
  webgl.loft     — OpWebGL* operator declarations + wasm entry points
  glb.loft       — GLB binary writer, save_glb()
```

---

## 2D Drawing — types

### `draw.loft`

```loft
// 32-bit RGBA colour. a == 0 means fully transparent / absent.
pub struct Rgba {
  r: u8 not null,
  g: u8 not null,
  b: u8 not null,
  a: u8 not null
}

// 2D coordinate in canvas pixels (origin top-left, y increases downward).
pub struct Xy {
  x: single not null,
  y: single not null
}

// A node in a vector path.
// Point  — a simple anchor: the stroke passes through coord with the given
//          colour and brush size.  a == 0 in color disables the stroke at
//          this node (useful for "move without drawing").
// Curve  — a cubic-Bezier anchor: same fields as Point plus two tangent
//          handles.  in_handle is the incoming control point (relative to
//          coord); out_handle is the outgoing control point.  The stroke
//          between two adjacent Curve nodes is the cubic Bezier defined by
//          the four control points:
//            prev.coord, prev.coord + prev.out_handle,
//            this.coord + this.in_handle, this.coord
pub enum PathNode {
  Point  { coord: Xy, color: Rgba, size: single },
  Curve  { coord: Xy, color: Rgba, size: single, in_handle: Xy, out_handle: Xy }
}

// Stroke style applied to the path between anchor nodes.
pub enum LineStyle {
  Straight,
  Dashed,
  Dotted
}

// A complete vector path ready to be rendered onto a Canvas.
// nodes  — ordered sequence of PathNode values.
// fill   — interior colour.  a == 0 means no fill (open path or unfilled).
// line   — how gaps are rendered between anchor nodes.
pub struct Draw {
  nodes: vector<PathNode>,
  fill:  Rgba,
  line:  LineStyle
}
```

---

## 2D Drawing — canvas

```loft
// An RGBA pixel buffer of size × size pixels (square).
// All pixels initialise to Rgba{r:0, g:0, b:0, a:0} (fully transparent).
pub struct Canvas {
  canvas_size: integer not null,
  canvas_data: vector<Rgba>
}

// Create a new empty canvas of the given side length.
pub fn canvas(size: integer) -> Canvas {
  c = Canvas { canvas_size: size };
  // fill with transparent pixels
  blank = Rgba { r: 0, g: 0, b: 0, a: 0 };
  for _i in 0..size * size {
    c.canvas_data += [blank];
  }
  c
}

// Read one pixel (bounds-checked; returns transparent on out-of-range).
pub fn pixel_at(self: Canvas, x: integer, y: integer) -> Rgba {
  if x < 0 || y < 0 || x >= self.canvas_size || y >= self.canvas_size {
    return Rgba { r: 0, g: 0, b: 0, a: 0 };
  }
  self.canvas_data[y * self.canvas_size + x]
}

// Write one pixel (no-op on out-of-range).
pub fn set_pixel(self: &Canvas, x: integer, y: integer, color: Rgba) {
  if x >= 0 && y >= 0 && x < self.canvas_size && y < self.canvas_size {
    self.canvas_data[y * self.canvas_size + x] = color;
  }
}

// Alpha-composite src over the canvas pixel at (x, y) using Porter-Duff.
pub fn blend_pixel(self: &Canvas, x: integer, y: integer, src: Rgba) {
  if x < 0 || y < 0 || x >= self.canvas_size || y >= self.canvas_size { return }
  idx = y * self.canvas_size + x;
  dst = self.canvas_data[idx];
  sa = src.a;
  da = dst.a;
  out_a = sa + da * (255 - sa) / 255;
  if out_a == 0 {
    self.canvas_data[idx] = Rgba { r: 0, g: 0, b: 0, a: 0 };
    return
  }
  self.canvas_data[idx] = Rgba {
    r: (src.r * sa + dst.r * da * (255 - sa) / 255) / out_a,
    g: (src.g * sa + dst.g * da * (255 - sa) / 255) / out_a,
    b: (src.b * sa + dst.b * da * (255 - sa) / 255) / out_a,
    a: out_a
  };
}

// Render a Draw onto the canvas (anti-aliased).
// Calls the native rasteriser.
pub fn draw(self: &Canvas, path: Draw);
#rust"drawing::render_path(&mut @self, &@path);"

// Save canvas as a PNG file.
pub fn save_png(self: Canvas, path: text);
#rust"stores.write_png(@path, &@self);"
```

---

## 2D Drawing — primitives

All primitives return a `Draw` value (a path description). Render it onto a `Canvas`
with `canvas.draw(path)`. This separates construction from rasterisation.

```loft
// Ellipse centred at (cx, cy) with half-axes rx, ry.
// fill    — interior colour (a == 0 = no fill).
// stroke  — outline colour and brush size (a == 0 = no outline).
// style   — stroke dash pattern.
pub fn ellipse(cx: single, cy: single, rx: single, ry: single,
               fill: Rgba, stroke: Rgba, stroke_size: single,
               style: LineStyle) -> Draw;
#rust"drawing::ellipse(@cx, @cy, @rx, @ry, &@fill, &@stroke, @stroke_size, @style)"

// Axis-aligned rectangle with top-left at (x, y).
pub fn rect(x: single, y: single, w: single, h: single,
            fill: Rgba, stroke: Rgba, stroke_size: single,
            style: LineStyle) -> Draw;
#rust"drawing::rect(@x, @y, @w, @h, &@fill, &@stroke, @stroke_size, @style)"

// Rectangle with uniformly rounded corners of radius r.
pub fn rounded_rect(x: single, y: single, w: single, h: single, r: single,
                    fill: Rgba, stroke: Rgba, stroke_size: single,
                    style: LineStyle) -> Draw;
#rust"drawing::rounded_rect(@x, @y, @w, @h, @r, &@fill, &@stroke, @stroke_size, @style)"

// Arrow from point a to point b with a filled arrowhead of the given size.
// The shaft uses the stroke colour and style.
pub fn arrow(ax: single, ay: single, bx: single, by: single,
             head_size: single, fill: Rgba, stroke: Rgba, stroke_size: single,
             style: LineStyle) -> Draw;
#rust"drawing::arrow(@ax, @ay, @bx, @by, @head_size, &@fill, &@stroke, @stroke_size, @style)"
```

### Anti-aliasing strategy

Stroke rasterisation uses **coverage-weighted super-sampling**:

1. For each pixel that the path's bounding box touches, compute the analytical
   coverage of the stroke curve segment through that pixel (using the distance from
   the pixel centre to the nearest point on the path, normalised by the brush radius).
2. Use this coverage as the alpha weight when blending via `blend_pixel`.
3. For filled regions (when `fill.a > 0`), use a scanline fill with edge-anti-aliasing
   on boundary pixels (area-of-pixel coverage).

This gives smooth edges without a full MSAA buffer and keeps memory proportional to
the canvas area, not to a supersample factor.

---

## 3D Mesh — types

### `mesh.loft`

```loft
// 2-component float vector (texture coordinates).
pub struct Vec2 {
  u: single not null,
  v: single not null
}

// 3-component float vector.
pub struct Vec3 {
  vx: single not null,
  vy: single not null,
  vz: single not null
}

// 4-component float vector (homogeneous coordinates, quaternions).
pub struct Vec4 {
  qx: single not null,
  qy: single not null,
  qz: single not null,
  qw: single not null
}

// A single vertex in a mesh.
// pos     — world-space position.
// normal  — surface normal (unit vector; null = flat-shaded, computed at upload).
// uv      — texture coordinate [0, 1].
// color   — per-vertex tint; Rgba{255,255,255,255} = no tint.
pub struct Vertex {
  pos:    Vec3,
  normal: Vec3,
  uv:     Vec2,
  color:  Rgba
}

// One triangle, as indices into the parent Mesh's vertex list.
pub struct Triangle {
  ia: integer not null,
  ib: integer not null,
  ic: integer not null
}

// A 3D mesh: lists of vertices and triangles.
// The mesh is topology-agnostic; it can represent any closed or open surface.
pub struct Mesh {
  mesh_vertices:  vector<Vertex>,
  mesh_triangles: vector<Triangle>
}
```

---

## 3D Scene — types

### `scene.loft`

```loft
// Position, orientation (Euler XYZ radians), and scale of a scene object.
pub struct Transform {
  tx: single not null,   // translation x
  ty: single not null,   // translation y
  tz: single not null,   // translation z
  rx: single not null,   // rotation x (radians)
  ry: single not null,   // rotation y (radians)
  rz: single not null,   // rotation z (radians)
  sx: single not null,   // scale x
  sy: single not null,   // scale y
  sz: single not null    // scale z
}

pub fn identity_transform() -> Transform {
  Transform { tx: 0.0f, ty: 0.0f, tz: 0.0f,
              rx: 0.0f, ry: 0.0f, rz: 0.0f,
              sx: 1.0f, sy: 1.0f, sz: 1.0f }
}

// A mesh placed in the scene at a specific transform.
// mesh_ref — index into Scene.meshes (avoids duplicating geometry).
// mat_ref  — index into Scene.materials.
pub struct MeshInstance {
  inst_mesh: integer not null,
  inst_mat:  integer not null,
  inst_xf:   Transform
}

// Perspective camera.
pub struct Camera {
  cam_pos:    Vec3,   // world-space eye position
  cam_target: Vec3,   // look-at point
  cam_up:     Vec3,   // up vector (usually 0,1,0)
  cam_fov:    single, // vertical field of view in radians
  cam_near:   single, // near clip distance
  cam_far:    single  // far clip distance
}

// Point light source.
pub struct Light {
  light_pos:   Vec3,
  light_color: Rgba,
  light_power: single
}

// A complete 3D scene.
pub struct Scene {
  scene_meshes:    vector<Mesh>,
  scene_materials: vector<Material>,
  scene_objects:   vector<MeshInstance>,
  scene_lights:    vector<Light>,
  scene_camera:    Camera
}
```

---

## Texture upload

### `texture.loft`

```loft
// Source for a GPU texture — either a loft Canvas or a PNG file.
pub enum TextureSource {
  Drawing { drawing_canvas: Canvas },
  Image   { image_file: text }
}

// A material applied to a mesh instance.
pub struct Material {
  mat_source:    TextureSource,
  mat_tint:      Rgba,    // multiplied with texture; Rgba{255,255,255,255} = unmodified
  mat_roughness: single,  // 0.0 = mirror, 1.0 = fully diffuse
  mat_metallic:  single   // 0.0 = dielectric, 1.0 = metallic
}

// Upload a Canvas to the GPU and return an opaque texture handle (integer id).
// Must be called from the render thread or before the first frame.
pub fn upload_canvas(canvas: Canvas) -> integer;
#rust"gl::upload_rgba_texture(&@canvas)"

// Upload a PNG file to the GPU. Returns null on load failure.
pub fn upload_png(path: text) -> integer;
#rust"gl::upload_png_texture(@path)"

// Release a previously uploaded texture. The id becomes invalid.
pub fn release_texture(id: integer);
#rust"gl::release_texture(@id);"
```

---

## Rendering backends

The backend is selected once at program start and hidden behind a single `render()`
call. The selection strategy:

| Backend | When active |
|---|---|
| `OpenGl` | Desktop build, GLFW window, OpenGL 3.3 core profile |
| `WebGl`  | WASM build, compiled with `--target wasm32-unknown-unknown`, WebGL 2 |
| `Glb`    | Offline/headless — `render()` writes a `.glb` file instead of drawing |

```loft
pub enum Backend { OpenGl, WebGl, Glb }

// Initialise the rendering context.
// For OpenGl: creates a window of the given width × height.
// For WebGl: attaches to the <canvas id="loft"> element.
// For Glb: no-op (output path is given to save_glb instead).
pub fn init_renderer(backend: Backend, width: integer, height: integer);
#rust"renderer::init(@backend, @width, @height);"

// Upload a mesh to GPU memory. Returns a handle (integer).
pub fn upload_mesh(mesh: Mesh) -> integer;
#rust"renderer::upload_mesh(&@mesh)"

// Upload all scene meshes and materials. Returns upload counts.
pub fn upload_scene(scene: &Scene);
#rust"renderer::upload_scene(&@scene);"

// Render one frame of the scene. For OpenGl/WebGl, swaps buffers.
pub fn render(scene: Scene);
#rust"renderer::render_frame(&@scene);"

// Poll window events; return false when the window has been closed.
pub fn poll_events() -> boolean;
#rust"renderer::poll_events()"

// Tear down the rendering context and release GPU resources.
pub fn shutdown();
#rust"renderer::shutdown();"
```

---

## GLB export

GLB is a binary-packaged version of glTF 2.0. It embeds mesh geometry, materials,
and textures in a single file for interchange with Blender, Unity, Unreal, etc.

```loft
// Write the entire scene as a binary GLB file.
// Textures are embedded as PNG data inside the GLB binary blob.
pub fn save_glb(scene: Scene, path: text) -> FileResult;
#rust"glb::write_scene(&@scene, @path)"

// Write only a single mesh instance (no lights or camera) to GLB.
// Useful for exporting individual props.
pub fn save_mesh_glb(mesh: Mesh, mat: Material, xf: Transform, path: text) -> FileResult;
#rust"glb::write_mesh(&@mesh, &@mat, &@xf, @path)"
```

### GLB internal layout

```
GLB file
├── JSON chunk  — glTF scene graph: nodes, meshes, accessors, materials, samplers
└── BIN chunk   — interleaved binary buffer
    ├── mesh 0: vertices (position f32×3, normal f32×3, uv f32×2, color u8×4) + indices u32
    ├── mesh 1: …
    └── texture images: raw PNG bytes, one per material texture source
```

The exporter builds the JSON chunk from the `Scene` struct, computing byte offsets
for each accessor into the binary buffer.

---

## 2D Text rendering

### Library choice

Text rasterization uses **`fontdue`** (Rust crate) for both native and WebGL backends.

| Property | Detail |
|---|---|
| Crate | `fontdue 0.8` |
| Language | Pure Rust — zero C dependencies |
| WASM | Compiles to `wasm32` unchanged; same code path for native and browser |
| Input | TTF / OTF font bytes |
| Output | Per-glyph 8-bit coverage bitmap + metrics |
| Layout | Sub-pixel positioned advances, kerning via font metrics |

**Why not a separate library for WebGL?**  Because `fontdue` compiles to WASM without
modification, the native and WebGL backends share the identical rasterization path.
There is no need for a JS text library (e.g. `opentype.js`) or the browser Canvas 2D
API.  The only browser-specific text capability foregone is access to system fonts;
loft programs supply their own TTF files.

**Why not FreeType or HarfBuzz?**  Both require C linkage and complicate the WASM
build.  FreeType in particular has no `wasm32` package.  `fontdue` provides comparable
quality with a Rust-native build graph.

**Glyph cache.**  The native Rust layer maintains a per-`Font` LRU cache of rasterized
bitmaps keyed by `(glyph_id, size_px_rounded_to_tenths)`.  Cache misses call
`fontdue::Font::rasterize`.  The cache is hidden from loft; it is an implementation
detail of `src/drawing.rs`.

---

### `text.loft`

```loft
// An opaque handle to a loaded font.
// font_id is an index into the interpreter's internal font registry.
pub struct Font {
  font_id: integer not null
}

// Horizontal alignment of a text run relative to the draw origin.
pub enum TextAlign {
  Left,    // origin is left edge of the text
  Center,  // origin is horizontal centre
  Right    // origin is right edge
}

// Vertical alignment relative to the draw origin.
pub enum TextBaseline {
  Top,     // origin is top of the ascender line
  Middle,  // origin is midpoint between ascender and descender
  Bottom   // origin is bottom of the descender line
}

// Parameters for a text draw call.
pub struct TextStyle {
  text_font:    Font,
  text_size:    single not null,          // font size in pixels
  text_color:   Rgba,
  text_align:   TextAlign,
  text_base:    TextBaseline,
  text_spacing: single not null           // extra advance between glyphs (0 = default)
}

// Measured extents of a text string under a given style.
pub struct TextMetrics {
  tm_width:   single not null,  // total advance width
  tm_height:  single not null,  // ascent + descent
  tm_ascent:  single not null,  // distance from baseline to top of tallest glyph
  tm_descent: single not null   // distance from baseline to bottom of deepest glyph (positive)
}

// Load a font from a TTF or OTF file.
// Returns null if the file does not exist or is not a valid font.
pub fn load_font(path: text) -> Font;
#rust"drawing::load_font(@path)"

// Returns the built-in fallback font (a bundled subset of Noto Sans).
// Always succeeds; useful for quick prototyping without a font file.
pub fn default_font() -> Font;
#rust"drawing::default_font()"

// Measure the pixel extent of str under the given style without drawing anything.
// Use to position text precisely or to wrap lines manually.
pub fn measure_text(str: text, style: TextStyle) -> TextMetrics;
#rust"drawing::measure_text(@str, &@style)"

// Render str onto canvas at (x, y).
// The origin is interpreted according to style.text_align and style.text_base.
// Uses coverage-weighted alpha blending (same anti-aliasing as vector paths).
pub fn draw_text(self: &Canvas, str: text, x: single, y: single, style: TextStyle);
#rust"drawing::render_text(&mut @self, @str, @x, @y, &@style);"

// Render str inside a bounding box with automatic line wrapping.
// Lines are broken at whitespace; words longer than max_width are hard-broken.
// Returns the y coordinate immediately below the last line rendered.
pub fn draw_text_box(self: &Canvas, str: text,
                     bx: single, by: single, bw: single, bh: single,
                     style: TextStyle) -> single;
#rust"drawing::render_text_box(&mut @self, @str, @bx, @by, @bw, @bh, &@style)"
```

---

### Rasterization pipeline

```
load_font(path)
    │  fontdue::Font::from_bytes(ttf_bytes)
    │  stored in font registry → font_id
    ▼
draw_text(canvas, str, x, y, style)
    │
    ├─ for each Unicode scalar in str
    │      look up glyph_id via fontdue layout
    │      query glyph cache (font_id, glyph_id, size)
    │      on miss: fontdue::Font::rasterize → (metrics, Vec<u8> coverage)
    │      advance cursor by metrics.advance_width + text_spacing
    │      for each coverage pixel (cx, cy, alpha):
    │          tinted = Rgba { r, g, b, a: style.color.a * alpha / 255 }
    │          canvas.blend_pixel(px + cx, py + cy, tinted)
    ▼
    canvas pixels updated in-place
```

The sub-pixel advance from `fontdue` is rounded to the nearest integer for canvas
placement; fractional positioning is not attempted (canvas pixels are integers).

---

### Usage example — text on a canvas

```loft
use lib/graphics/draw;
use lib/graphics/text;

fn main() {
  c = canvas(512);

  // dark background
  c.draw(rect(0.0f, 0.0f, 512.0f, 512.0f,
    Rgba{r:20,g:20,b:30,a:255}, Rgba{r:0,g:0,b:0,a:0}, 0.0f, LineStyle.Straight));

  fnt = load_font("assets/Inter-Regular.ttf");

  title_style = TextStyle {
    text_font:    fnt,
    text_size:    48.0f,
    text_color:   Rgba { r: 255, g: 255, b: 255, a: 255 },
    text_align:   TextAlign.Center,
    text_base:    TextBaseline.Top,
    text_spacing: 0.0f
  };

  body_style = TextStyle {
    text_font:    fnt,
    text_size:    20.0f,
    text_color:   Rgba { r: 180, g: 200, b: 220, a: 255 },
    text_align:   TextAlign.Left,
    text_base:    TextBaseline.Top,
    text_spacing: 0.5f
  };

  c.draw_text("Hello loft", 256.0f, 40.0f, title_style);

  m = measure_text("Hello loft", title_style);
  println("title is {m.tm_width:.1} × {m.tm_height:.1} px");

  c.draw_text_box(
    "This text wraps automatically inside the box boundaries.",
    20.0f, 120.0f, 472.0f, 300.0f, body_style);

  c.save_png("output/hello.png");
}
```

---

## Rust integration notes

All native operators follow the existing `#rust "..."` pattern with the `Op`-prefix
naming convention.  The implementation lives in new crate-local modules:

| Module | Content |
|---|---|
| `src/drawing.rs` | 2D path rasteriser (`render_path`, primitive builders) |
| `src/renderer/mod.rs` | Backend dispatcher (`init`, `render_frame`, `poll_events`) |
| `src/renderer/opengl.rs` | OpenGL 3.3 core, GLFW window, GLSL shaders |
| `src/renderer/webgl.rs` | WebGL 2 via `web-sys`; compiled only for `wasm32` target |
| `src/glb.rs` | glTF/GLB binary writer |

External crate dependencies (added to `Cargo.toml`):

| Crate | Purpose |
|---|---|
| `glfw` | Window + OpenGL context (desktop only, feature-gated) |
| `gl` | OpenGL function loader |
| `web-sys` | WebGL 2 API bindings (WASM target only) |
| `gltf` | glTF/GLB serialisation |
| `fontdue` | TTF/OTF rasterization — pure Rust, compiles to WASM unchanged |

Both `glfw` and `web-sys` are gated behind Cargo features so a headless/GLB-only
build has no windowing dependency.

---

## Usage examples

### Draw a rounded button and save as PNG

```loft
use lib/graphics/draw;

fn main() {
  c = canvas(256);

  bg = Rgba { r: 60, g: 120, b: 200, a: 255 };
  border = Rgba { r: 255, g: 255, b: 255, a: 200 };
  none = Rgba { r: 0, g: 0, b: 0, a: 0 };

  c.draw(rounded_rect(8.0f, 8.0f, 240.0f, 80.0f, 12.0f, bg, border, 2.0f, LineStyle.Straight));

  f = file("button.png");
  c.save_png(f.path);
}
```

### Draw a Bezier curve

```loft
fn main() {
  c = canvas(512);

  stroke = Rgba { r: 220, g: 80, b: 80, a: 255 };
  no_fill = Rgba { r: 0, g: 0, b: 0, a: 0 };

  origin = Xy { x: 50.0f,  y: 400.0f };
  p1     = Xy { x: 200.0f, y: 100.0f };
  p2     = Xy { x: 350.0f, y: 450.0f };
  end    = Xy { x: 460.0f, y: 150.0f };

  path = Draw {
    nodes: [
      PathNode.Curve {
        coord: origin, color: stroke, size: 3.0f,
        in_handle: Xy { x: 0.0f, y: 0.0f },
        out_handle: Xy { x: 80.0f, y: -100.0f }
      },
      PathNode.Curve {
        coord: p2, color: stroke, size: 3.0f,
        in_handle: Xy { x: -60.0f, y: 80.0f },
        out_handle: Xy { x: 60.0f, y: -80.0f }
      },
      PathNode.Point { coord: end, color: stroke, size: 3.0f }
    ],
    fill: no_fill,
    line: LineStyle.Straight
  };

  c.draw(path);
  c.save_png("curve.png");
}
```

### Render a 3D scene with a textured box

```loft
use lib/graphics/draw;
use lib/graphics/mesh;
use lib/graphics/scene;
use lib/graphics/texture;
use lib/graphics/opengl;

fn make_box() -> Mesh {
  // ... build 8 vertices, 12 triangles for a unit cube
}

fn main() {
  init_renderer(Backend.OpenGl, 1280, 720);

  // 2D canvas texture
  tex_canvas = canvas(256);
  tex_canvas.draw(rect(0.0f, 0.0f, 256.0f, 256.0f,
    Rgba{r:80,g:160,b:80,a:255}, Rgba{r:0,g:0,b:0,a:0}, 0.0f, LineStyle.Straight));
  tex_canvas.draw(ellipse(128.0f, 128.0f, 60.0f, 60.0f,
    Rgba{r:255,g:220,b:0,a:255}, Rgba{r:0,g:0,b:0,a:0}, 0.0f, LineStyle.Straight));

  mat = Material {
    mat_source:    TextureSource.Drawing { drawing_canvas: tex_canvas },
    mat_tint:      Rgba { r: 255, g: 255, b: 255, a: 255 },
    mat_roughness: 0.6f,
    mat_metallic:  0.0f
  };

  box_mesh = make_box();
  inst = MeshInstance {
    inst_mesh: 0,
    inst_mat:  0,
    inst_xf:   identity_transform()
  };

  cam = Camera {
    cam_pos:    Vec3 { vx: 3.0f, vy: 2.0f, vz: 5.0f },
    cam_target: Vec3 { vx: 0.0f, vy: 0.0f, vz: 0.0f },
    cam_up:     Vec3 { vx: 0.0f, vy: 1.0f, vz: 0.0f },
    cam_fov:    1.0f,
    cam_near:   0.1f,
    cam_far:    100.0f
  };

  light = Light {
    light_pos:   Vec3 { vx: 5.0f, vy: 8.0f, vz: 4.0f },
    light_color: Rgba { r: 255, g: 255, b: 255, a: 255 },
    light_power: 1.0f
  };

  scene = Scene {
    scene_meshes:    [box_mesh],
    scene_materials: [mat],
    scene_objects:   [inst],
    scene_lights:    [light],
    scene_camera:    cam
  };

  upload_scene(&scene);

  while poll_events() {
    render(scene);
  }

  shutdown();
}
```

### Export to GLB

```loft
use lib/graphics/mesh;
use lib/graphics/scene;
use lib/graphics/glb;

fn main() {
  scene = /* ... build scene ... */;
  result = save_glb(scene, "output/scene.glb");
  if !result.ok() {
    println("GLB export failed");
  }
}
```

---

## Implementation constraints

### Field uniqueness per file

Loft enforces that field names are unique across all structs within one source file.
The library avoids collisions by using disambiguating prefixes per struct:

| Struct | Prefix applied | Example fields |
|---|---|---|
| `Vec2` | `u`, `v` (UV) | `u`, `v` |
| `Vec3` | `v` prefix | `vx`, `vy`, `vz` |
| `Vec4` | `q` prefix | `qx`, `qy`, `qz`, `qw` |
| `Transform` | `t/r/s` + axis | `tx`, `ty`, `tz`, `rx`, `ry`, `rz`, `sx`, `sy`, `sz` |
| `Camera` | `cam_` | `cam_pos`, `cam_fov`, … |
| `Light` | `light_` | `light_pos`, `light_color`, … |
| `MeshInstance` | `inst_` | `inst_mesh`, `inst_xf`, … |
| `Scene` | `scene_` | `scene_meshes`, `scene_camera`, … |
| `Canvas` | `canvas_` | `canvas_size`, `canvas_data` |
| `Mesh` | `mesh_` | `mesh_vertices`, `mesh_triangles` |
| `Material` | `mat_` | `mat_source`, `mat_tint`, … |

### `single` vs `float`

All 3D positions, normals, and texture coordinates use `single` (32-bit float) to
match GPU vertex buffer formats and reduce upload bandwidth.  Canvas pixel coordinates
in `Xy` also use `single` to allow sub-pixel anti-aliasing offsets.

### Null sentinels in mesh data

Because mesh coordinates are stored as `single not null`, there is no null sentinel
concern for vertex data.  Index fields (`ia`, `ib`, `ic`, `inst_mesh`, `inst_mat`)
use plain `integer not null` and rely on range validation at upload time.

### Integer vertex indices

GLB and OpenGL both support 32-bit indices (`u32`).  The loft `integer` type
(signed 32-bit) covers all positive index values up to ~2 billion, sufficient for
any practical mesh.

### Backend feature gates

The OpenGL backend links against `libGL` and `glfw`.  The WebGL backend requires a
WASM toolchain.  Both are gated so the GLB-only backend compiles anywhere:

```toml
[features]
opengl  = ["dep:glfw", "dep:gl"]
webgl   = ["dep:web-sys", "dep:wasm-bindgen"]
default = ["opengl"]

[dependencies]
fontdue = "0.8"          # no feature gate — used by both native and WASM builds
```

Loft programs targeting a browser are compiled with `--no-default-features --features webgl`.
