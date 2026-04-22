<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 3 — Panel UI overlay (split 2026-04-22)

**Status:** split into two sub-phases.

**Phase 3a** — editor_tick + tool select + paint-on-click — **done
2026-04-22**.  `lib/graphics/examples/moros_editor.loft` now wires
`moros_sim::EditorState`, `input_from_keys`, `editor_tick`, the
`RenderCamera` → `scene::Camera` translation, and a rising-edge
left-click that calls `tool_apply` at the avatar's hex.  Scene
rebuilds + re-uploads after any edit.

**Phase 3b** — panel overlay (this file's remaining scope) —
**open**.  Depends on a render.loft API change to separate the
3D pass from the buffer swap, so a 2D overlay can render on top
before `gl_swap_buffers` fires.

## Phase 3b scope

Render the `moros_ui::Panel` model as a 2D overlay after the 3D
scene, and route clicks through `editor_click` so panel widgets take
priority over world-hit tool application.

Prerequisite: `render.loft::render_frame` currently runs the full
3D pass AND calls `gl_swap_buffers` before returning.  For an
overlay the swap must be deferred:
- Add `render_frame_no_swap(r, scene, cam) -> boolean` OR split
  existing `render_frame` into `render_3d_pass` + `present_frame`.
- The driver calls the 3D pass, then renders the 2D panel with the
  depth test disabled + blending enabled, then calls
  `gl_swap_buffers` itself.
- Existing `render_frame` stays as a convenience wrapper for
  callers that don't need overlays (brick-buster, demo, the
  renderer-demo example).

## Work

1. `lib/moros_editor/examples/native_editor.loft` adds a 2D
   painter + sprite atlas for widget rendering.  Mirrors brick-buster's
   `create_painter_2d` / `create_sprite_sheet` pattern.

2. Text rendering via `graphics::gl_load_font` (DejaVuSans-Bold.ttf
   pulled from `lib/graphics/examples/`).

3. Per frame, after the 3D scene:
   - `p = editor_panel(st, 1024, 768, fps)` — lay out panel model.
   - Iterate `p.widgets` and draw each as a textured quad with its
     label.
   - `active_tool` highlight: draw a border around the
     current-tool widget.

4. Click routing:
   - On left-click frame, call
     `editor_click(st, p, mx, my)`.
   - Expected return: `UhPanel` if a widget was clicked
     (already handled by `route_click` → state mutation); `UhWorld`
     if not, and `editor_click` has already applied the tool.
   - Replace the Phase 2 direct-`tool_apply` shortcut with this path.

## Technical notes

- The panel is laid out in pixel coordinates; the 2D painter uses
  an orthographic projection.  No Z interaction with the 3D scene —
  blit after clearing the depth buffer.
- Text textures are created once per label at window open (labels
  are static: "None", "Raise", "Lower", "Stencil", "Item", "Wall").

## Test plan

- Compile-regression expands: the driver file now `use moros_ui;`.
- Manual: run on dev machine, click the "Raise" widget, see the
  tool highlight move, confirm the avatar-centric left-click path
  no longer fires when the click lands on a widget.

## Acceptance

- [ ] Panel visible in bottom-left or bottom-right of the window.
- [ ] Current-tool widget visibly highlighted.
- [ ] Clicking a widget selects that tool (no world edit).
- [ ] Clicking outside the panel applies the current tool at the
      avatar's hex.
- [ ] 2D overlay renders above 3D regardless of camera orientation.
