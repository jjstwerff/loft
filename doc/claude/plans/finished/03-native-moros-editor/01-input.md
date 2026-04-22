<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 1 — Native input API gaps

**Status:** open.

## Scope

Fill the two gaps between what `moros_sim::editor_tick` /
`input_from_keys` need and what `lib/graphics/src/graphics.loft`
exposes natively:

1. Scroll wheel (`camera_apply_input` uses `input.in_wheel` for camera
   zoom).
2. Expanded key constants — the editor needs digits 1–6, Tab, O, R,
   F, Z, Y, `[`, `]`, and F5/F9/F11 for phase 4.  Today only
   W/A/S/D/Q/E/Space/Esc/arrows/Shift/Ctrl are exposed.

## Part A — scroll wheel

New native fn:

```loft
// Returns the accumulated scroll-wheel delta (in ticks, positive = up)
// since the last call.  Resets on read.  Returns 0 when no scroll
// occurred this frame.
pub fn gl_mouse_wheel() -> integer;
#native "loft_gl_mouse_wheel"
```

Rust side — `lib/graphics/native/src/lib.rs`:

```rust
thread_local! {
    static WHEEL_ACCUM: Cell<i64> = const { Cell::new(0) };
}

// In the winit event handler (MouseWheel arm), accumulate:
// LineDelta(_, y) => WHEEL_ACCUM.update(|w| w + y as i64)
// PixelDelta(p)   => WHEEL_ACCUM.update(|w| w + (p.y / LINE_HEIGHT) as i64)

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_mouse_wheel() -> i64 {
    WHEEL_ACCUM.with(|c| { let v = c.get(); c.set(0); v })
}
```

JS side — `loft-gl.js`: accumulate on the canvas `wheel` event;
return + reset.

## Part B — expanded key constants

Add to `lib/graphics/src/graphics.loft` (after the existing KEY_*
block):

```loft
// Digits (for tool-select 1-6).
KEY_0 = 48; KEY_1 = 49; KEY_2 = 50; KEY_3 = 51; KEY_4 = 52;
KEY_5 = 53; KEY_6 = 54; KEY_7 = 55; KEY_8 = 56; KEY_9 = 57;

// Editor hotkeys.
KEY_TAB       = 9;
KEY_O         = 111;
KEY_R         = 114;
KEY_F         = 102;
KEY_Z         = 122;
KEY_Y         = 121;
KEY_LBRACKET  = 91;
KEY_RBRACKET  = 93;

// Function keys for save/load/fullscreen-toggle.
KEY_F5  = 134;
KEY_F9  = 138;
KEY_F11 = 140;
```

These use Latin-1/ASCII code points where they exist; the F-key
range starts at 134 (after the existing 132 / 133 shift/ctrl
constants).

Rust side: `lib/graphics/native/src/input.rs` (or wherever
`winit::keyboard::NamedKey` / `Key::Character` is mapped) extends
the match for:

- `Character(c)` for digits + letters (map to their ASCII point).
- `NamedKey::Tab` → 9.
- `NamedKey::F5` / `NamedKey::F9` / `NamedKey::F11` → 134/138/140.

JS side: `loft-gl.js` handles `event.code` / `event.key` the same
way.

## Test plan

1. `cargo build --release` clean.

2. A smoke loft script that polls each new key and prints when
   pressed — manually verified on dev machine.  No automated test
   (requires real keyboard input).

3. Regression: existing smoke scripts (brick-buster uses only
   KEY_A/D/LEFT/RIGHT/ESCAPE) keep working — those constants don't
   change.

## Acceptance

- [ ] `gl_mouse_wheel()` returns non-zero after a scroll event and
      zero on the next call.
- [ ] New KEY_* constants added; values don't collide with existing
      ones.
- [ ] Native + JS backends handle the new events.
- [ ] Pre-existing native GL examples still run.

## Non-goals

- Raw input devices (gamepad, stylus).
- Non-Latin keyboard layouts — the digit/letter mapping is
  ASCII-centric, matching the rest of the existing KEY_* set.
- Key repeat rate / timing — editor hotkeys are edge-triggered,
  not repeat-sensitive.

## Rollback

Single commit.  All additions; reverting doesn't affect any
existing caller.
