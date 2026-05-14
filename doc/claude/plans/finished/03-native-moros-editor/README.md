<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 03 — Native Moros editor (OpenGL, windowed + fullscreen)

## Status — DONE 2026-04-22

All seven phases landed.  `make editor-dist` produces a shippable
`dist/moros-editor/` directory; the binary runs from a fresh
location without `loft` on the machine.

The Moros hex scene editor now runs end-to-end as a native OpenGL
program (windowed or fullscreen) from a single `loft --native`
invocation, without the browser + JS shell.

### Phase outcome

| # | Phase | File | Outcome |
|---|---|---|---|
| 0 | Fullscreen support in `gl_create_window` | [00-fullscreen.md](00-fullscreen.md) | ✅ done |
| 1 | Native input API gaps — scroll wheel + expanded key codes | [01-input.md](01-input.md) | ✅ done |
| 2 | Minimal native editor driver (window, camera, WASD, quit) | [02-driver.md](02-driver.md) | ✅ done |
| 3a | `editor_tick` + tool select + paint-on-click | [03-panel.md](03-panel.md) | ✅ done |
| 3b | Panel UI overlay (2D panel after 3D scene, click routing) | [03-panel.md](03-panel.md) | ✅ done — landed with a native codegen fix for the `s.const_refs` / `s.string_from_const_store` gap that previously blocked any loft fn reconstructing constants under `--native` (commit `0abc056`) |
| 4 | Save/load (F5/F9) + fullscreen toggle (F11) | [04-persistence.md](04-persistence.md) | ✅ done |
| 5 | Polish — real dt + `gl_create_window` failure hint | [05-polish.md](05-polish.md) | partial ✅ — see Deferred items below |
| 6 | Standalone compiled application — `make editor-dist` | [06-standalone.md](06-standalone.md) | ✅ done |

### Deferred items (Phase 5 polish)

Not blockers; user-visible quality-of-life items that roll forward
into follow-up work:

- FPS counter overlay
- Window resize aspect handling
- Avatar render in 3D scene
- Hex-pick highlight (mouse-over indication of which hex would be edited)

Not currently roadmap-tracked.  Add a ROADMAP G-tier row if any of
these blocks a downstream demo or user request.

## Shipped surface

A user can today:

1. `cargo build --release` the loft binary.
2. `./target/release/loft --native --path . lib/moros_editor/examples/native_editor.loft`
3. See a 3D map render in an OS window.
4. WASD + mouse-look to navigate.
5. Press 2–6 to select paint / height / stencil / item / wall tools;
   left-click to apply at the avatar's hex.
6. Ctrl+Z / Ctrl+Y to undo / redo.
7. Tab to toggle follow / overview camera.
8. Esc to quit.
9. F5 / F9 to save / load the map.
10. F11 to toggle fullscreen.
11. See the moros_ui Panel as a 2D overlay; click it to switch tools.

Or run `make editor-dist` and ship the resulting `dist/moros-editor/`
directory as a self-contained binary.

## Related

- [`../../ROADMAP.md`](../../../ROADMAP.md) — 0.8.5 Moros editor milestone
  (browser path).  This initiative was additive: the browser editor
  stays; this plan added the native option.
- [`../../USER_FACING.md`](../../../USER_FACING.md) — Native OpenGL
  world demo row references this plan as the OpenGL infrastructure
  that's already shipped.
- `lib/moros_editor/` — the edit operations library.
- `lib/moros_sim/src/editor.loft` — `EditorState`, `editor_tick`,
  `input_from_keys`, `camera_apply_input`.
- `lib/moros_ui/src/editor_click.loft` — `editor_click` for panel
  dispatch.
- `lib/graphics/src/graphics.loft` — `gl_create_window`,
  `gl_poll_events`, `gl_swap_buffers`, `gl_key_pressed`,
  `gl_mouse_*`.
- `lib/graphics/native/src/window.rs` — winit + glutin window
  creation (added the fullscreen variant in Phase 0).
- `lib/graphics/examples/25-brick-buster.loft` — reference for how
  a complete native loft + GL game is structured.
- `lib/moros_editor/examples/native_editor.loft` — the entry point
  this plan wired up.
