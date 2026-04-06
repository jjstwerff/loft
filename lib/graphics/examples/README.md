# Graphics Examples

23 progressive examples covering the loft graphics library, from basic window
creation to PBR rendering with shadow mapping.  Each example runs as a native
OpenGL window via `./example.loft` (requires `make install` and the native
graphics library).

## Prerequisites

```sh
cd ~/workspace/loft
make install
cd lib/graphics/native && cargo build --release && cd ../../..
cd lib/graphics/examples
```

## Examples

### Basics

| # | File | What it shows |
|---|---|---|
| 01 | `01-hello-window.loft` | Window creation, event loop, clear color |
| 02 | `02-hello-triangle.loft` | First triangle with vertex buffers and shaders |
| 03 | `03-shaders.loft` | Per-vertex colors with GPU interpolation |
| 04 | `04-textures.loft` | Loading `wall.jpg` and UV-mapped texture sampling |

### Transforms & Lighting

| # | File | What it shows |
|---|---|---|
| 05 | `05-transformations.loft` | MVP matrices, rotating cube |
| 06 | `06-coordinate-systems.loft` | Multiple objects with different model matrices |
| 07 | `07-camera.loft` | Orbiting camera using sin/cos |
| 08 | `08-basic-lighting.loft` | Phong lighting: ambient + diffuse + specular |
| 09 | `09-materials.loft` | Different shininess and metallic per object |

### 2D & Scene Graph

| # | File | What it shows |
|---|---|---|
| 10 | `10-2d-canvas.loft` | Software 2D rasterization, outputs PNG (no window) |
| 11 | `11-scene-graph.loft` | Multi-object GLB export with lights (no window) |

### Advanced Rendering

| # | File | What it shows |
|---|---|---|
| 12 | `12-multiple-lights.loft` | Directional + point light with attenuation |
| 13 | `13-depth-testing.loft` | Depth visualization with per-object color and depth fog |
| 14 | `14-blending.loft` | Alpha transparency with depth-sorted rendering |
| 15 | `15-face-culling.loft` | Back-face culling comparison side-by-side |
| 16 | `16-shadow-mapping.loft` | Two-pass depth shadows with orthographic light |
| 17 | `17-post-processing.loft` | Render-to-texture with animated grayscale effect |
| 18 | `18-pbr.loft` | Physically based rendering — 5×5 metallic/roughness grid |
| 19 | `19-complete-scene.loft` | All techniques combined: PBR, shadows, orbiting camera |

### Textures, Input & Utilities

| # | File | What it shows |
|---|---|---|
| 20 | `20-textured-cube.loft` | Procedural checkerboard texture with `draw_text()` label |
| 21 | `21-keyboard-camera.loft` | First-person WASD + arrow key camera controls |
| 22 | `22-wireframe.loft` | Alternative draw modes: lines and points |
| 23 | `23-cleanup.loft` | GPU resource lifecycle: create, use, delete (no window) |

## Examples without a window

- **10-2d-canvas.loft** — outputs `10-canvas-demo.png`, no GPU needed
- **11-scene-graph.loft** — outputs `11-scene.glb`, viewable in Blender or any glTF viewer
- **23-cleanup.loft** — prints resource IDs, validates lifecycle

## Based on LearnOpenGL

Examples 01–19 are adapted from [LearnOpenGL](https://learnopengl.com) by
Joey de Vries (CC BY-NC 4.0).  Each header comment links to the original tutorial.
