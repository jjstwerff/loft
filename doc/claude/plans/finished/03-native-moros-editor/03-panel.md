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
**done 2026-04-22** (commit `0abc056`).  `lib/graphics/examples/
moros_editor.loft` now renders the `moros_ui::Panel` as a 2D
overlay after the 3D scene and routes clicks via `editor_click`.
`render_frame` was split into `render_frame_no_swap` (3D pass
only) + wrapper that swaps, so overlays can layer between 3D and
swap without touching existing callers.  Landed with a native
codegen fix for the `s.const_refs` / `s.string_from_const_store`
gap that previously blocked any loft function reconstructing
constants under `--native`.

---

# Phase 3b — Full design

## Goal

Render the `moros_ui::Panel` model as a 2D overlay after the 3D
scene in `lib/graphics/examples/moros_editor.loft`.  Route left-
clicks through `editor_click(state, panel, mx, my)` so panel
widgets claim clicks before the world does — a click on the
"Raise" toolbar button selects that tool instead of painting the
hex underneath.

## What the panel looks like (from `lib/moros_ui/src/panel.loft`)

240-pixel-wide strip on the LEFT of the window:

```
┌── 240 px ──┐
│ None    1  │ ← 6 toolbar buttons, each 32 px tall,
│ Raise   2  │   4 px gap.  btn_selected == true for
│ Lower   3  │   the currently-active tool (darker fill).
│ Stencil 4  │
│ Item    5  │
│ Wall    6  │
├────────────┤ ← 2 px separator, 8 px padding
│ [palette]  │ ← ListBox: variable content depending on
│ [items]    │   current tool.  For Stencil tool: list of
│ …          │   stencil names.  For Item tool: items.
│            │
│            │
│            │
├────────────┤
│ q=3 r=3…   │ ← StatusStrip: single line, q/r/cy/altitude/fps
└────────────┘
```

All positions in pixel coords; origin top-left of the window.
`panel_build(tools, window_w, window_h, q, r, cy, altitude, fps)`
returns a `Panel` struct with pre-computed rects; the driver's
only job is to RENDER it + ROUTE clicks.

## Prerequisite — `render.loft` API refactor

**Problem:** `render::render_frame(r, scene, cam)` currently calls
`graphics::gl_swap_buffers()` before returning.  A 2D overlay has
to render BETWEEN the 3D pass and the swap, so the caller needs to
control the swap.

**Solution — split `render_frame` into a no-swap variant:**

```loft
// In lib/graphics/src/render.loft — add alongside existing
// `render_frame`.  Body is identical except for the final swap.
pub fn render_frame_no_swap(self: Renderer, sc: const scene::Scene,
                            cam: const scene::Camera) -> boolean {
  // … same body as render_frame, minus the closing:
  //   graphics::gl_swap_buffers();
  // Caller swaps itself.
}

// Existing `render_frame` becomes a thin wrapper so every current
// caller (brick-buster, 24-renderer-demo, moros_render examples)
// keeps working without changes.
pub fn render_frame(self: Renderer, sc: const scene::Scene,
                    cam: const scene::Camera) -> boolean {
  if !self.render_frame_no_swap(sc, cam) { return false; }
  graphics::gl_swap_buffers();
  true
}
```

Zero signature change on `render_frame`.  Overlay callers use
`render_frame_no_swap`; single-pass callers keep the convenience.

## Prerequisite — widget-label text textures

Panel widgets have static labels ("None", "Raise", "Lower",
"Stencil", "Item", "Wall", digit hotkeys "1"..."6") that can be
rasterised once at startup.  The status line ("q=… r=… cy=…
y=… FPS…") changes every frame; recreate each frame (with
`gl_delete_texture` on the prior handle to avoid GPU leak).

```loft
// One-time at startup, after `create_renderer`:
font = graphics::gl_load_font("lib/graphics/examples/DejaVuSans-Bold.ttf");
if !font {
    // Fallback search for `dist/moros-editor/assets/` layout:
    font = graphics::gl_load_font("assets/DejaVuSans-Bold.ttf");
}
white = graphics::rgba(230, 230, 230, 255);
lbl_None    = graphics::create_text_texture(font, "None",    16.0, white);
lbl_Raise   = graphics::create_text_texture(font, "Raise",   16.0, white);
// … through lbl_Wall, hk_1..hk_6
```

Per frame (rebuild on change):
```loft
if status_text != prev_status_text {
    if status_tex != 0 { graphics::gl_delete_texture(status_tex); }
    status_tex = graphics::create_text_texture(font, status_text, 14.0, white);
    prev_status_text = status_text;
}
```

This tracks the previous string and skips re-rasterisation when
the status content hasn't changed (typically the case between
keypresses — q/r update only when the avatar crosses a hex
boundary).

## Main-loop structure (with panel)

```
fn main() {
  rnd = render::create_renderer(…);
  painter = graphics::create_painter_2d(WINDOW_W as float, WINDOW_H as float);
  font    = graphics::gl_load_font(…);
  labels  = bake_static_labels(font);  // one-time rasterise
  status_tex = 0;
  status_text = "";

  // … editor state setup …

  for _ in 0..LOOP_MAX {
    // Poll + InputState + editor_tick … (Phase 3a logic, unchanged)

    // ── NEW: rising-edge click goes through editor_click first ──
    if left_edge {
      panel = editor_panel(st, WINDOW_W, WINDOW_H, current_fps);
      hit = editor_click(st, panel, mx, my);
      // editor_click has already mutated tools + (on UhWorld) run
      // tool_apply with undo push.  `needs_rebuild` flips when
      // the world was hit.
      if hit is UhWorld { needs_rebuild = true; }
    }

    // Rebuild scene if needed … (Phase 3a logic)

    // ── 3D pass — NO SWAP ──
    if !render::render_frame_no_swap(rnd, scn, cam) { break; }

    // ── 2D overlay ──
    //   1. Disable depth test so widgets overwrite the 3D pass.
    //   2. Enable blend for semi-transparent panel background.
    //   3. Rebuild Panel + (if text changed) status_tex.
    //   4. Draw panel widgets.
    //   5. Restore depth / blend state.

    graphics::gl_disable(graphics::GL_DEPTH_TEST);
    graphics::gl_enable(graphics::GL_BLEND);
    graphics::gl_blend_func(
        graphics::BLEND_SRC_ALPHA,
        graphics::BLEND_ONE_MINUS_SRC_ALPHA,
    );
    panel = editor_panel(st, WINDOW_W, WINDOW_H, current_fps);
    render_panel(painter, font, labels, &status_tex, &status_text, panel);
    graphics::gl_enable(graphics::GL_DEPTH_TEST);
    graphics::gl_disable(graphics::GL_BLEND);

    // ── Swap ──
    graphics::gl_swap_buffers();
  }
}
```

## `render_panel` helper

Iterates the Panel model and draws each sub-structure.  All
positions are pixel coordinates (Painter2D's ortho projection has
the origin top-left).

```loft
fn render_panel(
    painter: const Painter2D,
    font: integer,
    labels: PanelLabels,
    status_tex_ref: &integer,
    status_text_ref: &text,
    p: Panel,
) {
  // 1. Panel background — semi-transparent dark rectangle.
  draw_rect_at(painter,
      p.p_rect.r_x as float, p.p_rect.r_y as float,
      p.p_rect.r_w as float, p.p_rect.r_h as float,
      0.08, 0.09, 0.12, 0.85);

  // 2. Toolbar buttons — fill + label + hotkey label.
  for b in p.p_toolbar {
    // Fill: lighter when selected, darker otherwise.
    bg_r = if b.btn_selected { 0.25 } else { 0.15 };
    bg_g = if b.btn_selected { 0.45 } else { 0.18 };
    bg_b = if b.btn_selected { 0.55 } else { 0.22 };
    draw_rect_at(painter,
        b.btn_rect.r_x as float, b.btn_rect.r_y as float,
        b.btn_rect.r_w as float, b.btn_rect.r_h as float,
        bg_r, bg_g, bg_b, 1.0);
    // Hotkey "1".."6" on the left; label centred.  Use the
    // pre-baked text textures from `labels`.
    hk_tex = labels.hotkeys[b.btn_id];
    lbl_tex = labels.labels[b.btn_id];
    // Hotkey at button's left edge, 10 px in.
    draw_texture_at(painter, hk_tex,
        (b.btn_rect.r_x + 10) as float, (b.btn_rect.r_y + 8) as float,
        14.0, 16.0);
    // Label at 40 px in.
    draw_texture_at(painter, lbl_tex,
        (b.btn_rect.r_x + 40) as float, (b.btn_rect.r_y + 8) as float,
        70.0, 16.0);
  }

  // 3. List box — background + per-item text.
  draw_rect_at(painter,
      p.p_list.lb_rect.r_x as float, p.p_list.lb_rect.r_y as float,
      p.p_list.lb_rect.r_w as float, p.p_list.lb_rect.r_h as float,
      0.05, 0.06, 0.09, 1.0);
  // Per-item texture — render on demand.  Caching pass in a
  // follow-up; for the MVP we re-rasterise each item every frame
  // (~10 items x 1 line each = 10 text textures).  Cost is
  // measurable but acceptable for 60 FPS.
  y = p.p_list.lb_rect.r_y + 4 - p.p_list.lb_scroll;
  for (idx, item) in p.p_list.lb_items {
    if idx == p.p_list.lb_selected {
      draw_rect_at(painter,
          p.p_list.lb_rect.r_x as float, y as float,
          p.p_list.lb_rect.r_w as float, p.p_list.lb_item_height as float,
          0.20, 0.35, 0.55, 1.0);
    }
    item_tex = graphics::create_text_texture(font, item, 13.0, white);
    draw_texture_at(painter, item_tex,
        (p.p_list.lb_rect.r_x + 6) as float, y as float,
        (p.p_list.lb_rect.r_w - 12) as float,
        p.p_list.lb_item_height as float);
    graphics::gl_delete_texture(item_tex);  // no caching yet
    y += p.p_list.lb_item_height;
  }

  // 4. Status strip.
  draw_rect_at(painter,
      p.p_status.ss_rect.r_x as float, p.p_status.ss_rect.r_y as float,
      p.p_status.ss_rect.r_w as float, p.p_status.ss_rect.r_h as float,
      0.03, 0.04, 0.06, 1.0);
  if p.p_status.ss_text != *status_text_ref {
    if *status_tex_ref != 0 { graphics::gl_delete_texture(*status_tex_ref); }
    *status_tex_ref = graphics::create_text_texture(
        font, p.p_status.ss_text, 13.0, white);
    *status_text_ref = p.p_status.ss_text;
  }
  draw_texture_at(painter, *status_tex_ref,
      (p.p_status.ss_rect.r_x + 8) as float,
      (p.p_status.ss_rect.r_y + 5) as float,
      (p.p_status.ss_rect.r_w - 16) as float,
      14.0);
}
```

## Click routing

Phase 3a bypassed the panel (no panel existed) and called
`tool_apply` directly on any left-click.  Phase 3b replaces that
shortcut with `editor_click`:

```loft
// Phase 3a (delete):
if left_edge && st.es_tools.ts_current != ToolKind.None {
  batch_begin(st.es_undo);
  undo_push(st.es_undo, st.es_map, …);
  tool_apply(st.es_player.pl_pos, st.es_overlay_cy, st.es_map, st.es_tools);
  batch_end(st.es_undo);
  needs_rebuild = true;
}

// Phase 3b (add):
if left_edge {
  panel = editor_panel(st, WINDOW_W, WINDOW_H, current_fps);
  hit = editor_click(st, panel, mx, my);
  // UhWorld → editor_click already ran tool_apply internally.
  // UhToolButton → route_click mutated tools.ts_current.
  // UhListItem   → route_click mutated tools.ts_selected_*.
  // UhNone       → click landed in panel gap; no-op.
  if hit is UhWorld { needs_rebuild = true; }
}
```

`editor_click` internally calls `route_click` (panel hit-test +
tool state mutation) and, on world hit, pushes an undo entry and
calls `tool_apply` — see `lib/moros_ui/src/editor_click.loft`.
That's the exact logic Phase 3a inlined; Phase 3b delegates.

## Struct-enum matching syntax

`UiHit` is a struct-enum with field variants
(`UhToolButton { tb_id: integer }`, `UhListItem { li_idx:
integer }`).  Match via `if hit is UhVariantName { field1, field2 }`:

```loft
if hit is UhToolButton { tb_id } { println("tool {tb_id}"); }
if hit is UhWorld { println("world"); }
```

The driver only needs the `UhWorld` case (to flag `needs_rebuild`);
the other cases are handled internally by `route_click`.

## `labels` helper

```loft
pub struct PanelLabels {
  hotkeys: vector<integer>,   // 6 entries, idx = btn_id
  labels:  vector<integer>,   // 6 entries, idx = btn_id
}

fn bake_static_labels(font: integer) -> PanelLabels {
  white = graphics::rgba(230, 230, 230, 255);
  hk: vector<integer> = [];
  lb: vector<integer> = [];
  names = ["None", "Raise", "Lower", "Stencil", "Item", "Wall"];
  for i in 0..6 {
    hk += [graphics::create_text_texture(font, "{i + 1}", 12.0, white)];
    lb += [graphics::create_text_texture(font, names[i], 14.0, white)];
  }
  PanelLabels { hotkeys: hk, labels: lb }
}
```

## Font resolution

Three candidate paths, tried in order until one loads:

1. `./assets/DejaVuSans-Bold.ttf` — the dist layout
   (`make editor-dist` copies the font here).
2. `lib/graphics/examples/DejaVuSans-Bold.ttf` — the development
   layout (running `loft --native` from the repo root).
3. `lib/graphics/examples/DejaVuSans-Bold.ttf` resolved relative
   to the script path — catches running from a different cwd.

If none load, print a diagnostic and run without labels (widget
rectangles still render; just no text).  Graceful fallback rather
than hard-exit so users can debug path issues without losing the
3D view.

## Performance budget

- **Per-frame cost** (dominant terms):
  - 1 panel bg rect   (1 draw call)
  - 6 toolbar bg rects + 12 text textures (~20 draw calls,
    static textures — no rebuild)
  - N list-item texts (~10 on average, recreated + destroyed each
    frame — each rasterisation allocates GPU memory;
    ~0.1–0.5 ms per rasterise per text)
  - 1 status text (recreated only when the string changes — typical
    steady-state cost ~0, only fires on avatar movement crossing
    hex boundaries)
  - Total on an idle frame: ~20 draw calls + 1 text raster ≈
    negligible (<1 ms overhead).

- **Active frame** (user panning the list):
  - N ≤ 30 list items at most (max stencils / items per tool).
  - ~30 text rasterisations per frame ≈ 5–15 ms.  At 60 Hz that's
    significant but not catastrophic.  A text-texture cache keyed
    on `(font, size, content)` removes this; kept as a Phase 3b
    follow-up.

## Test plan

1. `loft --native-emit lib/graphics/examples/moros_editor.loft` —
   emit the generated Rust, must parse clean.
2. `loft --native ...` on this headless box — compile must succeed;
   runs, fails at `gl_create_window` as usual.
3. Manual verification on a machine with a display:
   - Run `make native-editor`.
   - Expected: 3D scene + 240-px left panel with 6 tool buttons +
     empty / populated list + status line.
   - Click a tool button → the selected-tool highlight moves,
     status line's hotkey cycles, no map edit fires.
   - Click in the 3D area → current tool applies to the avatar's
     hex, map re-renders.
   - Ctrl-Z rolls back the last click.
   - Status line updates (q/r/cy) when WASD moves the avatar
     across a hex boundary.
4. `cargo test --release --test native` — 5/5 pre-existing tests
   stay green (no render_frame callers were touched; only a new
   `render_frame_no_swap` was added).
5. `cargo test --release --test html_wasm` — 6/6 stay green.

## Acceptance

- [ ] `render::render_frame_no_swap` exists and is called by the
      driver.
- [ ] `render::render_frame` is the unchanged convenience wrapper.
- [ ] Panel background visible at left edge of the window, 240 px
      wide.
- [ ] 6 toolbar buttons visible; active tool has a distinct
      highlight colour.
- [ ] List-box items (stencils / items / etc) visible depending
      on selected tool.
- [ ] Status line updates per frame.
- [ ] Left-click on a tool button changes the active tool without
      modifying the map.
- [ ] Left-click in the 3D area applies the current tool at the
      avatar's hex (unchanged from Phase 3a's behaviour, routed
      through `editor_click` instead of direct `tool_apply`).
- [ ] Pre-existing `native_*` + `html_wasm_*` tests green.

## Scope explicitly deferred

- **Text-texture cache.**  List-item re-rasterisation is a Phase
  3b follow-up once we have baseline numbers.
- **FPS counter actual value in status line.**  Phase 5 landed
  real `dt` per frame; Phase 3b can derive fps = `1 / dt` via
  EMA, but doing it cleanly means passing the fps through
  `editor_panel` (which already takes `fps: integer`).
  Wire-through is trivial; decoupling the EMA windowing is its
  own mini-project.  Ship with `fps = 60` hardcoded for now;
  correct-fps in Phase 5 continuation.
- **Click on panel outside widgets (`UhNone`).**  Currently a
  no-op; that's intentional.  Phase 3b ships without visual
  feedback for an empty click.
- **Keyboard focus / scroll-wheel on the list.**  Scroll is in
  Phase 3a's mouse-wheel handler (camera zoom); redirecting it
  to the list when the mouse hovers the panel needs a mouse-
  hover check.  Deferred.

## Rollback

Single additive commit:
- Add `render_frame_no_swap` — existing `render_frame` keeps
  working.
- Add `render_panel` / `bake_static_labels` / `PanelLabels` in
  the driver.
- Replace the Phase 3a left-click dispatch with `editor_click`.

Reverting removes the overlay rendering and the `editor_click`
routing; the direct Phase 3a `tool_apply` path is restored
verbatim.  No cross-phase state is introduced.

## Estimated implementation size

- `lib/graphics/src/render.loft` — ~25 lines added
  (new `render_frame_no_swap` + convert `render_frame` to
  one-line wrapper).
- `lib/graphics/examples/moros_editor.loft` — ~150 lines added
  (panel rendering helpers + integration).
- `doc/claude/plans/03-native-moros-editor/03-panel.md` — flip
  status to ✅ done on landing.
- README phase table — bump 3b to ✅.

One commit.  Manual dev-box verification required since the
overlay can't be automated-tested without an image-diff rig
(out of scope).
