<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 03 — Native Moros editor (OpenGL, windowed + fullscreen)

**Status:** open — Phase 0 ready to start.

**Goal:** the Moros hex scene editor runs end-to-end as a native
OpenGL program (windowed or fullscreen) from a single
`loft --native` invocation, without the browser + JS shell.

## Context

The Moros editor currently has:

- A complete loft-side library (`lib/moros_editor/`, `lib/moros_map/`,
  `lib/moros_render/`, `lib/moros_ui/`, `lib/moros_sim/`) with
  paint / height / wall / item / stencil / slope / undo primitives,
  a `map_to_json` / `map_from_json` round-trip, 3D scene building,
  GLB export, and a panel UI model.
- A browser shell (JS / HTML) that lives in `../moros/` — not part of
  this repository.
- No native runnable entry point.  `lib/moros_render/examples/*` are
  GLB-export CLI tools; none open a GL window.  `editor_tick` +
  `editor_click` + `gl_create_window` + `gl_poll_events` exist as
  building blocks but nothing wires them together.
- No fullscreen support — `gl_create_window(w, h, title)` has no
  fullscreen parameter; `lib/graphics/native/src/window.rs` never
  calls `WindowAttributes::with_fullscreen`.

This plan wires the building blocks into a runnable native editor
and extends the graphics API just enough to support fullscreen and
the input surface the editor needs.

## Ground rule

Per `doc/claude/plans/README.md`, this plan introduces **no
regressions**.  Every existing `lib/graphics/examples/*` must
continue to run; the existing 6 `tests/html_wasm.rs` tests must
stay green; the 7 `p184_vector_*` tests and all other existing
suites stay green.  API widenings are additive (new functions, new
constants, new variants) — no signature changes on already-shipped
`gl_create_window` etc.

## Phases

| # | Phase | File | Status | Blocks |
|---|---|---|---|---|
| 0 | Fullscreen support in `gl_create_window` | [00-fullscreen.md](00-fullscreen.md) | open | 2 |
| 1 | Native input API gaps — scroll wheel + expanded key codes | [01-input.md](01-input.md) | open | 2 |
| 2 | Minimal native editor driver (window, camera, WASD, quit) | [02-driver.md](02-driver.md) | open | 3 |
| 3a | `editor_tick` + tool select + paint-on-click | [03-panel.md](03-panel.md) | ✅ done 2026-04-22 — commit pending | — |
| 3b | Panel UI overlay (2D panel after 3D scene, click routing) | [03-panel.md](03-panel.md) | open | 4 |
| 4 | Save/load (F5/F9) + fullscreen toggle (F11) | [04-persistence.md](04-persistence.md) | open | 5 |
| 5 | Polish — FPS counter, resize, error diagnostics, avatar render, hex-pick highlight | [05-polish.md](05-polish.md) | open | 6 |
| 6 | Standalone compiled application — `make native-editor` produces a shippable `dist/moros-editor/` directory; binary runs from a fresh location without `loft` on the machine | [06-standalone.md](06-standalone.md) | open | — |

Phases 0–2 ship a minimum-viable native editor (window, camera, WASD
navigation, tool-1-through-6 select, Esc to quit).  Phases 3–5 bring
it to feature parity with what the browser shell provides.  Phase 6
makes the result a shippable standalone binary — a self-contained
`dist/moros-editor/` directory a user can run without `loft` on
their machine.

## Non-goals

- **Parity with the browser shell's UI polish.**  The panel's visual
  layout lives in `../moros/` JS — the native editor renders the
  moros_ui Panel model directly, which is simpler (fewer tools
  visible, less text).
- **GLB export as a live workflow.**  Users can already export via
  `loft --interpret lib/moros_render/examples/demo_village.loft
  out.glb` — no need to re-implement the pipeline inside the editor
  for this round.
- **Multiplayer / collaborative editing.**  Deferred to 1.0+.
- **Asset import UI.**  Material / item palettes stay integer-keyed
  as they are today.

## Success criteria

A user can:
1. `cargo build --release` the loft binary.
2. `./target/release/loft --native --path . lib/moros_editor/examples/native_editor.loft`.
3. See a 3D map render in an OS window.
4. WASD + mouse-look to navigate around.
5. Press 2–6 to select paint / height / stencil / item / wall tools;
   left-click to apply at the avatar's hex.
6. Ctrl+Z / Ctrl+Y to undo / redo.
7. Tab to toggle follow/overview camera.
8. Esc to quit.

Phases 3–5 add panel visibility, save/load, and fullscreen toggle.

## Related

- `doc/claude/ROADMAP.md` — 0.8.5 Moros editor milestone (browser path).
  This initiative is additive — the browser editor stays; this adds a
  native option.
- `lib/moros_editor/` — the edit operations library.
- `lib/moros_sim/src/editor.loft` — `EditorState`, `editor_tick`,
  `input_from_keys`, `camera_apply_input`.
- `lib/moros_ui/src/editor_click.loft` — `editor_click` for panel
  dispatch.
- `lib/graphics/src/graphics.loft` — `gl_create_window`,
  `gl_poll_events`, `gl_swap_buffers`, `gl_key_pressed`,
  `gl_mouse_*`.
- `lib/graphics/native/src/window.rs` — winit + glutin window
  creation.
- `lib/graphics/examples/25-brick-buster.loft` — reference for how a
  complete native loft+GL game is structured.
