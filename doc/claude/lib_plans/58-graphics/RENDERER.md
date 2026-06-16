
# High-Level Renderer Design

> **Status: designed, not scheduled.  Low-level `gl_*` API covers current use cases.**

Design for a renderer layer on top of the low-level GL bindings in `lib/graphics/`.
The existing `gl_*` functions remain as the low-level API for tutorials and custom
rendering; this layer provides the abstraction that production code should use.

---

## Motivation

Every OpenGL example (02-23) repeats the same patterns:

| Pattern | Lines | Files |
|---|---|---|
| Window + render loop + cleanup | 8-12 | 20/23 |
| MVP matrix composition | 3-5 per object | 15/23 |
| Shader creation from inline GLSL | 2-4 | 18/23 |
| Mesh flatten + VAO upload + vertex count | 3 | 18/23 |
| `ticks()` to seconds conversion | 2 | 15/23 |
| Lighting GLSL (ambient + diffuse) | 5-10 in shader | 10/23 |
| Per-object uniform setup | 3-6 per object | 15/23 |
| FBO + depth/color texture setup | 5-9 | 3/23 |

A programmer wanting shadow-mapped lighting currently writes ~160 lines (16-shadow-mapping)
including manual FBO setup, two render passes, light-space matrix math, and 40 lines of
GLSL.  The same result should be expressible in ~20 lines.

---

## Design principles

1. **Scene-driven** -- reuse the existing `Scene`, `Node`, `Material`, `Light`, `Camera`
   types from `scene.loft`.  The same scene struct that exports to GLB also renders live.

2. **Batteries included** -- built-in PBR shader, shadow mapping, and post-processing.
   No GLSL in user code unless they want custom effects.

3. **Low-level still accessible** -- the `gl_*` API stays for tutorials, experiments, and
   custom passes.  The renderer is built on top of it, not instead of it.

4. **Implemented in loft** -- the renderer logic (draw loop, shadow pass, uniform setup)
   is loft code calling `gl_*` natives.  Only the GL FFI stays in Rust.

---

## API surface

### Core types

```loft
// lib/graphics/src/renderer.loft

pub struct Renderer {
  width: integer not null,
  height: integer not null,
  // Internal state (shader handles, FBOs, etc.)
  pbr_shader: integer not null,
  shadow_shader: integer not null,
  shadow_fbo: integer not null,
  shadow_tex: integer not null,
  shadow_size: integer not null,
  // Uploaded GPU resources keyed by scene mesh index
  vaos: vector<integer>,
  vert_counts: vector<integer>
}
```

### Window and lifecycle

```loft
// Create a renderer with a window.  Compiles built-in shaders, allocates
// shadow map FBO.  Returns null on failure (no display).
pub fn create_renderer(width: integer, height: integer, title: text) -> Renderer

// Destroy GPU resources and close the window.
pub fn destroy(self: Renderer)
```

### Rendering

```loft
// Render one frame of the scene.  Handles:
//   1. Shadow pass (if scene has lights)
//   2. Color pass with PBR shading and shadow sampling
//   3. Buffer swap
// Returns false when the window close button is pressed.
pub fn render_frame(self: Renderer, scene: const Scene, camera: const Camera) -> boolean

// Convenience: render loop until window closes.
pub fn render_loop(self: Renderer, scene: const Scene, camera: const Camera)
```

### Scene upload

```loft
// Upload all meshes in the scene to GPU VAOs.  Called automatically
// by render_frame on first use; can be called explicitly to pre-warm.
pub fn upload_scene(self: Renderer, scene: const Scene)
```

### Animation support

```loft
// Seconds since the renderer was created (float, microsecond precision).
pub fn elapsed(self: Renderer) -> float
```

---

## Built-in shaders

The renderer compiles two shader programs at creation time.  The GLSL source
lives as string constants in `renderer.loft` (not in user code).

### Shadow depth pass

Minimal vertex-only shader: transforms vertices by the light's VP matrix.
Fragment shader is empty (depth-only write).

### PBR color pass

Single uber-shader covering the full material range:

- **Vertex**: transforms position, normal, UV; outputs world position,
  normal, UV, and light-space position for shadow lookup.
- **Fragment**:
  - PBR metallic-roughness (Cook-Torrance BRDF)
  - One directional light (shadow caster) plus one point light
    (quadratic attenuation, no shadow).  Second-and-later lights of
    each kind are ignored.  Both light colours are pre-multiplied by
    `Light.intensity` on the CPU side so the shader reads a single
    `vec3` per light.
  - Shadow map sampling with bias (directional only)
  - Ambient term (0.03 * albedo)
  - HDR tone-mapping + gamma correction

  Regression coverage: `tests/golden/{12,16,19,21,24}-*.png`, driven
  by the `LOFT_FAKE_TICKS_US` clock-freeze in `snap_example.sh`.
  A shader change shows up as a per-pixel diff on these captures.

Materials feed directly into uniforms:
`base_color_r/g/b/a`, `metallic`, `roughness` from the existing `Material` struct.

---

## Shadow mapping

Built into the renderer, not a separate user concern:

1. On `create_renderer`: allocate 1024x1024 depth FBO.
2. On `render_frame`: if the scene has any directional light, compute an
   orthographic light-space VP matrix that covers the scene bounding box.
3. Render all opaque geometry to the shadow FBO.
4. Bind shadow texture to unit 1 during the color pass.

The user never touches FBOs, depth textures, or light matrices.

---

## Usage example

### Minimal scene

```loft
use mesh;
use math;
use scene;
use renderer;

fn main() {
  s = scene::Scene { name: "demo" };
  s.add_mesh(mesh::plane("floor", 10.0, 10.0));
  s.add_mesh(mesh::cube());
  s.add_material(scene::Material {
    name: "gray", base_color_r: 0.5, base_color_g: 0.5, base_color_b: 0.55,
    base_color_a: 1.0, metallic: 0.0, roughness: 0.8 });
  s.add_material(scene::Material {
    name: "red", base_color_r: 0.9, base_color_g: 0.1, base_color_b: 0.1,
    base_color_a: 1.0, metallic: 0.0, roughness: 0.5 });
  s.add_node(scene::Node {
    name: "floor", mesh_idx: 0, material_idx: 0,
    transform: math::mat4_identity() });
  s.add_node(scene::Node {
    name: "cube", mesh_idx: 1, material_idx: 1,
    transform: math::mat4_translate(0.0, 0.5, 0.0) });
  s.add_light(scene::directional_light("sun", 1.0, 0.95, 0.9, 3.0,
    math::normalize3(math::vec3(-1.0, -2.0, -1.5))));
  cam = scene::camera("main");

  r = renderer::create_renderer(800, 600, "Demo");
  r.render_loop(s, cam);
  r.destroy();
}
```

This gives: PBR shading, directional light with shadows, correct depth testing --
in 25 lines, no GLSL, no manual MVP math, no FBO setup.

### Animated scene

```loft
  r = renderer::create_renderer(800, 600, "Animated");
  for _ in 0..100000 {
    t = r.elapsed();
    // Update node transforms for animation
    s.nodes[1].transform = math::mat4_mul(
      math::mat4_translate(0.0, 0.5, 0.0),
      math::mat4_rotate_y(t));
    if !r.render_frame(s, cam) { break }
  }
  r.destroy();
```

### With keyboard camera (future)

```loft
  r = renderer::create_renderer(800, 600, "FPS");
  cam = scene::camera("player");
  for _ in 0..100000 {
    dt = r.delta_time();
    // Input helpers (future renderer.loft addition)
    cam = renderer::fps_camera_update(cam, dt);
    if !r.render_frame(s, cam) { break }
  }
```

---

## Missing abstractions identified from examples

Beyond the renderer, these helpers would reduce boilerplate across the library:

### 1. `mat4_trs(translate, rotate_y, scale)` -- single-call transform

Most examples compose 2-3 `mat4_mul` calls for translate + rotate + scale.
Add to `math.loft`:

```loft
pub fn mat4_trs(tx: float, ty: float, tz: float, ry: float,
                sx: float, sy: float, sz: float) -> Mat4
```

### 2. Material presets

Most examples construct `Material` structs with 7 fields. Add helpers to `scene.loft`:

```loft
pub fn material_color(name: text, r: float, g: float, b: float) -> Material
pub fn material_metal(name: text, r: float, g: float, b: float, roughness: float) -> Material
```

### 3. `node_at` already exists -- promote it

`scene::node_at(name, mesh_idx, mat_idx, transform)` exists but examples don't use it,
writing the full `Node { ... }` struct instead.  Examples should prefer `node_at`.

### 4. `RenderTarget` abstraction

For examples that do need custom multi-pass rendering (post-processing, deferred),
wrap FBO + textures:

```loft
pub struct RenderTarget {
  fbo: integer not null,
  color_tex: integer not null,
  depth_tex: integer not null,
  width: integer not null,
  height: integer not null
}

pub fn create_render_target(w: integer, h: integer) -> RenderTarget
pub fn bind(self: RenderTarget)
pub fn unbind()
pub fn destroy(self: RenderTarget)
```

### 5. Shader library functions

For examples that write custom GLSL, provide includable functions as loft string
constants so they don't copy-paste the same lighting math:

```loft
// In graphics.loft or a new shaderlib.loft
pub GLSL_PHONG_LIGHTING = `
  vec3 calcPhong(vec3 normal, vec3 lightDir, vec3 viewDir,
                 vec3 color, float ambient, float shininess) {{
    float diff = max(dot(normal, -lightDir), 0.0);
    vec3 reflectDir = reflect(lightDir, normal);
    float spec = pow(max(dot(viewDir, reflectDir), 0.0), shininess);
    return color * (ambient + diff + 0.5 * spec);
  }}
`;
```

Users concatenate: `FRAG_SRC = graphics::GLSL_PHONG_LIGHTING + my_main;`

---

## Implementation plan

| Step | What | Files |
|---|---|---|
| R1 | Add `mat4_trs`, `mat4_ortho` (done) | `math.loft` |
| R2 | Add `material_color`, `material_metal` presets | `scene.loft` |
| R3 | Create `renderer.loft` with `Renderer` struct | new file |
| R4 | Built-in PBR + shadow shaders as string constants | `renderer.loft` |
| R5 | `create_renderer`: window + shader compile + FBO | `renderer.loft` |
| R6 | `upload_scene`: mesh flatten + VAO upload | `renderer.loft` |
| R7 | `render_frame`: shadow pass + color pass + swap | `renderer.loft` |
| R8 | `render_loop`, `elapsed`, `destroy` | `renderer.loft` |
| R9 | Add `RenderTarget` abstraction | `graphics.loft` |
| R10 | GLSL library constants (phong, pbr, shadow) | `shaderlib.loft` |
| R11 | Rewrite 11-scene-graph to also render live | example update |
| R12 | Add new example: `24-renderer-demo.loft` (~25 lines) | new example |

Steps R1-R2 are small helpers.  R3-R8 are the core renderer.  R9-R12 polish.

---

## What stays low-level

The `gl_*` API and the tutorial examples (02-23) stay as-is.  They serve as:

- Learning material (matching LearnOpenGL progression)
- Test coverage for the native GL bindings
- Escape hatch for custom rendering techniques

The renderer is a layer **above** them, not a replacement.
