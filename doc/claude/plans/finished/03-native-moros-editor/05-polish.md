<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 5 — Polish

**Status:** partial ✅ 2026-04-22 — real `dt` and
`gl_create_window`-failure diagnostics landed.  Remaining items
(window resize aspect, avatar render, hex-pick highlight) deferred
to a follow-up initiative.  **Depends on:** Phase 4.

## Items

1. **Real `dt` from a monotonic clock** ✅ done 2026-04-22.
   Per-frame `dt = (ticks() - prev) / 1e6`, clamped to 50 ms to
   survive window-drag stalls on some WMs, falls back to 1/60 on
   ticks jitter or the first frame.  Drives `editor_tick` for
   consistent avatar movement independent of frame rate.

2. **`gl_create_window`-failure diagnostics** ✅ done 2026-04-22.
   Driver now checks `rnd.width == 0` (the default empty-Renderer
   shape) and prints a 3-bullet actionable message enumerating the
   common root causes — no display server, OpenGL 3.3 not
   supported, missing X11 / Wayland dev headers at build time.

3. **FPS counter** — deferred.  Needs the panel from Phase 3b to
   show fps to the user.  Real `dt` (item 1) is the foundation;
   EMA + panel render is the follow-up.

4. **Window resize aspect-ratio handling** — deferred.  Requires a
   `render.loft` API change: `render_frame` reads `self.width /
   self.height` for the projection aspect, but those fields don't
   update when the window resizes.  Adding `render::set_viewport(r,
   w, h)` + wiring `gl_poll_events` resize notifications into it
   is a render-library concern that touches every existing native
   GL example, not editor-specific.

5. **Avatar render** — deferred.  Needs a cylinder / capsule mesh
   + node at `st.es_player.pl_pos`.  `moros_render::emit_cylinder_post`
   exists and could be reused, but scene rebuild semantics (the
   avatar mesh isn't part of `build_hex_meshes`) need one of: a
   second scene, a manual `add_node` after `map_build_scene`, or a
   scene-delta API.  Follow-up.

6. **Hex-pick highlight** — deferred.  Needs screen-to-world
   picking: `moros_render::camera_ray_dir` + `ray_plane_y_intersect`
   + `world_to_hex`.  Primitives exist; wiring into the driver +
   overlay rendering is ~50 lines but interacts with Phase 3b's
   2D pass.

## Acceptance

- [x] `gl_create_window` returning false prints a human hint.
- [x] `dt` updates per frame from `ticks()`.
- [ ] FPS counter updates in the panel.  (Deferred — needs 3b.)
- [ ] Resizing the window doesn't distort the 3D projection.
      (Deferred — render.loft refactor.)
- [ ] Avatar is visible.  (Deferred — mesh integration.)
- [ ] Current-hex highlight overlays the scene.  (Deferred —
      interacts with 3b.)

## Deferred

Ray-cast mouse picking (click-anywhere-in-the-world, not
avatar-centric) — this needs camera-aware screen-to-world via
`moros_render::camera_ray_dir` + `ray_plane_y_intersect`, which
exist today but haven't been wired into `editor_click`.  Ship the
highlight first; full pick-and-apply routing is its own follow-up
initiative.
