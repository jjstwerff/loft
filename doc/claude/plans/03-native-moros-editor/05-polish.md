<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 5 — Polish (follow-up)

**Status:** open.  **Depends on:** Phase 4.

## Items

1. **FPS counter** — real `dt` from a monotonic clock, exponential
   moving average displayed in the panel's `fps` field.
   `graphics::n_ticks` already provides millisecond-resolution
   timing on both native and WASM paths.

2. **Window resize handling** — winit fires resize events;
   `gl_poll_events` already updates the viewport, but the driver's
   hardcoded 1024×768 aspect ratio needs to become dynamic.  Read
   `gl_window_width` / `gl_window_height` each frame.

3. **Error diagnostics** — `gl_create_window` → false path should
   print a short actionable message (no DISPLAY, no X11, no
   Wayland, mesa missing, etc).  Today it just returns false.

4. **Avatar render** — draw a simple capsule / cylinder at
   `st.es_player.pl_pos` so the user sees where "here" is.  Placeholder
   geometry is fine; any art replacement is a separate concern.

5. **Hex-pick highlight** — overlay the hex that `world_to_hex`
   resolves under the mouse cursor with a faint outline.  Gives
   visual feedback for where a click would land in world-space
   picking mode (a future tool beyond avatar-centric).

## Acceptance

- [ ] FPS counter updates in the panel.
- [ ] Resizing the window doesn't distort the 3D projection.
- [ ] `gl_create_window` returning false prints a human hint.
- [ ] Avatar is visible.
- [ ] Current-hex highlight overlays the scene.

## Deferred

Ray-cast mouse picking (click-anywhere-in-the-world, not
avatar-centric) — this needs camera-aware screen-to-world via
`moros_render::camera_ray_dir` + `ray_plane_y_intersect`, which
exist today but haven't been wired into `editor_click`.  Ship the
highlight first; full pick-and-apply routing is its own follow-up
initiative.
