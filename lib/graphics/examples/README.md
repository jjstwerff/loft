# Graphics Examples

24 progressive examples covering the loft graphics library, from basic window
creation to PBR rendering with shadow mapping.  Every example runs natively
via OpenGL and **live in the browser** via WebGL.

**[Run all examples in the browser →](https://jjstwerff.github.io/loft/gallery.html)**

## Prerequisites (native)

```sh
cd ~/workspace/loft
make install
cd lib/graphics/native && cargo build --release && cd ../../..
cd lib/graphics/examples
```

## Examples

### Basics

| # | File | What it shows | Live demo |
|---|---|---|---|
| 01 | `01-hello-window.loft` | Window creation, event loop, clear color | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 02 | `02-hello-triangle.loft` | First triangle with vertex buffers and shaders | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 03 | `03-shaders.loft` | Per-vertex colors with GPU interpolation | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 04 | `04-textures.loft` | Loading `wall.jpg` and UV-mapped texture sampling | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |

### Transforms & Lighting

| # | File | What it shows | Live demo |
|---|---|---|---|
| 05 | `05-transformations.loft` | MVP matrices, rotating cube | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 06 | `06-coordinate-systems.loft` | Multiple objects with different model matrices | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 07 | `07-camera.loft` | Orbiting camera using sin/cos | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 08 | `08-basic-lighting.loft` | Phong lighting: ambient + diffuse + specular | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 09 | `09-materials.loft` | Different shininess and metallic per object | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |

### 2D & Scene Graph

| # | File | What it shows | Live demo |
|---|---|---|---|
| 10 | `10-2d-canvas.loft` | Software 2D rasterization, downloads PNG | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 11 | `11-scene-graph.loft` | Multi-object scene with PBR renderer | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |

### Advanced Rendering

| # | File | What it shows | Live demo |
|---|---|---|---|
| 12 | `12-multiple-lights.loft` | Directional + point light with attenuation | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 13 | `13-depth-testing.loft` | Depth visualization with per-object color and depth fog | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 14 | `14-blending.loft` | Alpha transparency with depth-sorted rendering | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 15 | `15-face-culling.loft` | Back-face culling comparison side-by-side | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 16 | `16-shadow-mapping.loft` | Two-pass depth shadows with orthographic light | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 17 | `17-post-processing.loft` | Render-to-texture with animated grayscale effect | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 18 | `18-pbr.loft` | Physically based rendering — 5×5 metallic/roughness grid | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 19 | `19-complete-scene.loft` | All techniques combined: PBR, shadows, orbiting camera | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |

### Textures, Input & Utilities

| # | File | What it shows | Live demo |
|---|---|---|---|
| 20 | `20-textured-cube.loft` | Procedural texture with `draw_text()` label | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 21 | `21-keyboard-camera.loft` | WASD + mouse drag camera controls | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 22 | `22-wireframe.loft` | Alternative draw modes: lines and points | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |
| 23 | `23-cleanup.loft` | GPU resource lifecycle: create, use, delete | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |

### High-Level Renderer

| # | File | What it shows | Live demo |
|---|---|---|---|
| 24 | `24-renderer-demo.loft` | Scene-driven PBR with shadows — no shader code | [▶ Run](https://jjstwerff.github.io/loft/gallery.html) |

## Based on LearnOpenGL

Examples 01–19 are adapted from [LearnOpenGL](https://learnopengl.com) by
Joey de Vries (CC BY-NC 4.0).  Each header comment links to the original tutorial.
