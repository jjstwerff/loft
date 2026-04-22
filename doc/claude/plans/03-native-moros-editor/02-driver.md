<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 2 — Minimal native editor driver

**Status:** open.  **Depends on:** Phase 0 (fullscreen), Phase 1 (input).

**Scope re-framed 2026-04-22:** Phase 2 is now a *native viewer
foundation*, not the full `editor_tick`-wired editor.  The
editor_tick + tool + panel integration shifts to Phase 3 because
it requires a `RenderCamera` ↔ `scene::Camera` adapter layer that's
cleaner to land as its own commit.  Phase 2 proves the end-to-end
native GL pipeline with `loft --native` producing a runnable
window that renders the Moros hex map — the smallest useful native
deliverable.

## Scope

Write the first native `fn main()` that wires:

```
gl_create_window → seed EditorState → event loop {
    poll keys + mouse + wheel
    input_from_keys → InputState
    editor_tick(state, input, dt)
    build scene from state.es_map
    render scene with state.es_camera
    gl_swap_buffers
} → quit on Esc
```

## File layout

New file: `lib/moros_editor/examples/native_editor.loft`.

Follow `lib/graphics/examples/25-brick-buster.loft`'s shape — that
example is the canonical "complete native loft+GL program" reference.

## Pseudocode

```loft
use moros_editor;
use moros_sim;
use moros_render;
use graphics;

fn seed_starter_map() -> Map {
    m = map_empty();
    // A small 7x7 flat grid at cy=0, material 1.
    for q in 0..7 {
      for r in 0..7 {
        map_paint_material(m, q, r, 0, 1);
      }
    }
    m
}

fn main() {
  if !graphics::gl_create_window(1024, 768, "Moros editor") {
    println("failed to create window"); return;
  }

  // Editor state with a small starter map so the user sees something
  // on frame 1 instead of an empty world.
  st = moros_sim::editor_default();
  st.es_map = seed_starter_map();

  prev_held: vector<text> = [];
  prev_mx = 0; prev_my = 0;
  dt = 1.0 / 60.0;  // fixed for now; fps counter is Phase 5

  loop {
    if !graphics::gl_poll_events() { break; }
    if graphics::gl_key_pressed(KEY_ESCAPE) { break; }

    // Build held / pressed-since sets from polled key state.
    held = keys_held_this_frame();   // helper below
    pressed = moros_sim::keys_pressed_since(prev_held, held);

    mx = graphics::gl_mouse_x() as integer;
    my = graphics::gl_mouse_y() as integer;
    mbtn = graphics::gl_mouse_button();
    clicked = (mbtn & 1) != 0;   // left
    rclicked = (mbtn & 2) != 0;  // right

    input = moros_sim::input_from_keys(
      held, pressed, mx, my,
      mx - prev_mx, my - prev_my,
      graphics::gl_mouse_wheel(),
      clicked, rclicked,
      graphics::gl_key_pressed(KEY_CTRL),
    );

    moros_sim::editor_tick(st, input, dt);

    // Build + render scene.
    scene = moros_render::map_build_scene(st.es_map);
    graphics::render_scene(scene, st.es_camera, 1024.0 / 768.0);

    graphics::gl_swap_buffers();

    prev_held = held;
    prev_mx = mx;
    prev_my = my;
  }
}

fn keys_held_this_frame() -> vector<text> {
  k: vector<text> = [];
  if graphics::gl_key_pressed(KEY_W) { k += ["w"]; }
  if graphics::gl_key_pressed(KEY_A) { k += ["a"]; }
  if graphics::gl_key_pressed(KEY_S) { k += ["s"]; }
  if graphics::gl_key_pressed(KEY_D) { k += ["d"]; }
  if graphics::gl_key_pressed(KEY_SPACE) { k += ["space"]; }
  if graphics::gl_key_pressed(KEY_1) { k += ["1"]; }
  if graphics::gl_key_pressed(KEY_2) { k += ["2"]; }
  // … etc for the full editor key set
  k
}
```

## What's out of scope for Phase 2

- **No `editor_tick` / `InputState` wiring.**  Phase 3 adds it.
  Phase 2 drives the camera directly from polled keys.
- **No UI panel overlay.**  Pure 3D scene + camera.  Phase 3.
- **No save/load.**  Phase 4.
- **No tool-apply at world click.**  Phase 3.
- **No fullscreen toggle at runtime.**  Phase 4 adds F11.  Phase 0's
  `gl_create_fullscreen_window` is selectable via an `--fullscreen`
  command-line argument read at boot.
- **No avatar render.**  Phase 5.
- **Fixed `dt`**.  Real `dt` from `ticks()` is Phase 5.

## Test plan

1. Compile the driver: `loft --native --path . lib/moros_editor/examples/native_editor.loft`.
   Must produce a runnable binary without errors.

2. Run it on the dev machine: window opens, 7×7 flat map visible,
   WASD moves avatar, number keys change tool, left-click paints
   under avatar's hex with currently-selected tool, Esc closes.

3. No automated test — native GL needs a display.  A `cargo test`
   can't assert window-level behaviour.  The compile-only check
   (loft --native driver.loft succeeds) is the automated regression
   guard, expressed as:

```rust
// tests/native_editor.rs
#[test]
fn native_editor_example_compiles() {
    // Invokes `loft --native --native-emit /tmp/native_editor.rs
    //         --path . lib/moros_editor/examples/native_editor.loft`
    // and asserts the resulting Rust is non-empty + compiles.
    // Skips if rustc / native target not available.
}
```

This guards against signature drift in `editor_tick` /
`input_from_keys` / `map_build_scene` that would break the
driver.

## Acceptance

- [ ] `lib/moros_editor/examples/native_editor.loft` exists.
- [ ] `loft --native --path . …native_editor.loft` produces a
      running binary that opens a window.
- [ ] Compile-regression test added.
- [ ] ROADMAP updated to note the native editor entry point.

## Known shortcut / deferred work

The MVP calls `tool_apply` directly on world click instead of via
`editor_click` — because Phase 2 has no panel to hit-test against.
Phase 3 migrates the call site to `editor_click` once the panel
overlay exists.
