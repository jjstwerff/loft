<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# graphics — 2D canvas + 3D rendering for loft

## Install

```sh
loft install graphics
```

## Surface

### Sub-modules

- `math` — `Vec2`/`Vec3`/`Vec4` + `Mat4` (`mat4_identity` / `mat4_translate` /
  `mat4_scale` / `mat4_mul` / `mat4_transform` / `mat4_perspective` /
  `mat4_look_at` / `mat4_rotate_y` / `mat4_rotate_x` / `mat4_ortho` /
  `mat4_trs`); vector ops `add3`/`sub3`/`scale3`/`dot3`/`cross`/`length3`/`normalize3`.
- `mesh` — `Vertex` / `Triangle` / `Mesh` types + builders (`sphere`, mesh-to-floats).
- `scene` — `Scene` graph with `Node`, `Material`, `Camera`, `Light`.
- `glb` — glTF 2.0 binary (`.glb`) save (`save_glb`, `save_scene_glb`).
- `graphics` (entry) — `Canvas` 2D pixel surface with `set_pixel` / `clear` /
  `blend_pixel` / `fill_rect` / `hline` / `vline` / `draw_rect` / `draw_line` /
  `draw_circle` / `fill_circle` / `fill_ellipse` / `draw_ellipse` / `draw_bezier` /
  `draw_aa_line` / `fill_triangle`; PNG save via `save_png`; OpenGL bindings
  (`gl_create_window` / `gl_create_fullscreen_window` / shaders / VAOs / textures /
  FBOs / `gl_draw` / `gl_clear` / `gl_swap_buffers`); sprite sheets (`SpriteSheet` +
  `draw_sprite`); `Painter2D` for fixed-function 2D draws over GL; SFX helpers
  (`sfx_beep` / `sfx_chirp` / `sfx_descend`).

### Native code

`loft_graphics_native` cdylib backs the GL + PNG + font + audio calls via
`glutin` / `gl` / `winit` / `fontdue` / `png` / `image` / `rodio`.

## Provenance

Extracted from the loft monorepo's `lib/graphics/` 2026-05-31 as part of
[@PLAN12](https://github.com/loft-lang/loft/blob/main/doc/claude/lib_plans/12-library-extraction/README.md)
Phase 5b (`loft-libs-graphics` Stage B — Stage A for graphics + imaging,
then monorepo cleanup).
