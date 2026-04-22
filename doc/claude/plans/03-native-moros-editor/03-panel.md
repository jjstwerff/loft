<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 3 — Panel UI overlay (follow-up)

**Status:** open.  **Depends on:** Phase 2.

## Scope

Render the `moros_ui::Panel` model as a 2D overlay after the 3D
scene, and route clicks through `editor_click` so panel widgets take
priority over world-hit tool application.

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
