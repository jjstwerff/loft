<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4 — Save/load + fullscreen toggle (follow-up)

**Status:** open.  **Depends on:** Phase 3.

## Scope

Add three file-level and window-level controls:

- **F5** — save current `Map` to `./moros_map.json` via
  `moros_map::map_to_json`.
- **F9** — load `./moros_map.json` via `moros_map::map_from_json`,
  replacing `st.es_map`.  Clears the undo stack.
- **F11** — toggle fullscreen.

## F5 / F9

```loft
if pressed.contains("f5") {
  f = file("./moros_map.json");
  f.clear();
  f += map_to_json(st.es_map);
}
if pressed.contains("f9") {
  f = file("./moros_map.json");
  t = f.content();
  if t.len() > 0 {
    new_m = map_from_json(t);
    if ! is_null(new_m) {
      st.es_map = new_m;
      st.es_undo = undo_empty();
    }
  }
}
```

`map_to_json` + `map_from_json` exist and round-trip today (MO.2,
tested in `lib/moros_map/tests/`).

## F11 fullscreen toggle

Trickier — `gl_create_fullscreen_window` creates a fullscreen
window, but we need to TOGGLE an existing window.  Options:

### Option A (simpler): restart the GL state

Destroy current window + context, create a new one in the opposite
mode.  Every GL resource (textures, VAOs, fonts) must be re-created.
Heavyweight; state loss for anything the driver has cached.

### Option B (preferred): native-side toggle

Add a new native fn:

```loft
pub fn gl_set_fullscreen(on: boolean);
#native "loft_gl_set_fullscreen"
```

Rust side calls `window.set_fullscreen(Some(Fullscreen::Borderless(None)))`
on `on=true`, `None` on `on=false`.  winit supports this at runtime
without destroying the context; GL resources survive.

Option B is the right answer — cheaper at runtime, no resource
re-creation.

## Test plan

- F5 writes valid JSON (already tested by existing
  `map_to_json` round-trip in `lib/moros_map/tests/`).
- F9 round-trips (save, mutate, load, compare) — can be scripted
  WITHOUT a window via `loft --interpret`:

```loft
// tests/moros_save_load_roundtrip.loft
fn main() {
  m = map_empty();
  map_paint_material(m, 1, 1, 0, 7);
  file("/tmp/t.json").clear();
  file("/tmp/t.json") += map_to_json(m);
  m2 = map_from_json(file("/tmp/t.json").content());
  assert(map_get_hex(m2, 1, 1, 0).h_material == 7, "round-trip");
}
```

Fullscreen toggle — manual verification only.

## Acceptance

- [ ] F5 saves to `./moros_map.json`.
- [ ] F9 loads the same file, replacing state.  Undo stack cleared.
- [ ] F11 toggles between windowed 1024×768 and fullscreen.
- [ ] Save/load round-trip test green.
- [ ] `gl_set_fullscreen` added to native + JS backends (JS uses
      `document.exitFullscreen()` / `requestFullscreen()`).
