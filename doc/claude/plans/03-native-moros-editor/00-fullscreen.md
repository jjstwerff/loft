<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 0 — Fullscreen support in `gl_create_window`

**Status:** open.

## Scope

Add fullscreen mode to the native GL windowing API without breaking
existing callers.

## API widening (additive, backward-compatible)

Add a new native fn alongside the existing `gl_create_window`:

```loft
// Opens a borderless fullscreen window at the primary monitor's
// native resolution.  Returns true on success.  The window_width /
// window_height reported via `gl_window_width` / `gl_window_height`
// reflect the actual monitor size.
pub fn gl_create_fullscreen_window(title: text) -> boolean;
#native "loft_gl_create_fullscreen_window"
```

Pre-existing `gl_create_window(w, h, title) -> boolean` stays
unchanged — brick-buster and every other example keeps working.

Decision: separate fn rather than a 4th parameter because loft's
`#native` fn signatures are immutable across callers (no default
args), and a separate fn makes call sites self-documenting.

## Rust implementation (`lib/graphics/native/src/lib.rs`)

Add:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_create_fullscreen_window(
    title_ptr: *const u8,
    title_len: usize,
) -> bool {
    let title = unsafe { loft_ffi::text(title_ptr, title_len) };
    match window::create_gl_state_fullscreen(title) {
        Ok(state) => { /* same as windowed case */ }
        Err(e)    => { eprintln!(...); false }
    }
}
```

In `lib/graphics/native/src/window.rs`, a sibling to
`create_gl_state`:

```rust
pub fn create_gl_state_fullscreen(title: &str) -> Result<GlState, String> {
    // WindowAttributes::default()
    //   .with_title(title)
    //   .with_transparent(false)
    //   .with_fullscreen(Some(Fullscreen::Borderless(None)))
    // — then proceed exactly like create_gl_state.
}
```

`Borderless(None)` means "fullscreen on the current monitor" —
simpler than `Exclusive(mode)` which needs a VideoMode handle.

Refactor the shared body of the two fns into a private helper taking
`WindowAttributes` so both call sites share the glutin config /
surface creation logic.  Avoids drift between windowed and
fullscreen code paths.

Export `loft_gl_create_fullscreen_window` from the symbol table at
the bottom of `lib/graphics/native/src/lib.rs` (the same array
`loft_gl_create_window` is in).

## JS implementation (`lib/graphics/js/loft-gl.js`)

The browser `--html` build already runs fullscreen-capable via
`canvas.requestFullscreen()`.  Add a stub mapping `gl_create_fullscreen_window`
to `gl_create_window(screen.width, screen.height, title)` plus a
`document.documentElement.requestFullscreen()` call.  Browser
fullscreen requires a user gesture; the stub prints a console note
if invoked outside one.

## Test plan

1. Extend `lib/graphics/examples/` with a tiny `00-smoke-fullscreen.loft`
   that opens a fullscreen window and closes on Esc.  **Not added as
   an automated test** because fullscreen needs a display server;
   CI headless tests can't assert it.  Instead, include it as a
   manual-verification recipe in the phase doc.

2. Automated regression: `cargo build --release` of
   `lib/graphics/native/` must pass.  `tests/html_wasm.rs` stays
   green (browser path unchanged for pre-existing callers).

3. Manual: run `00-smoke-fullscreen.loft` on dev machine.  Expect a
   fullscreen black window for ~1 second then Esc-quit.

## Acceptance

- [ ] `loft_gl_create_fullscreen_window` symbol exported from native.
- [ ] `gl_create_fullscreen_window` declared in `lib/graphics/src/graphics.loft`.
- [ ] JS stub added.
- [ ] 00-smoke-fullscreen.loft compiles + runs.
- [ ] Pre-existing `gl_create_window` callers (brick-buster, etc.)
      still compile and run.
- [ ] `cargo test --release` full suite green.

## Rollback

Single commit.  Revert reverts all three (native, loft, JS).  No
cross-phase coupling.
