<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# B3 spike — GLES renders on the ANativeWindow (PROVEN) + the port design

B3 is the `lib/graphics` Android backend: a GL surface on the Android window so a loft
graphics program renders on-device. This spike proves the **core mechanism** end to end and
scopes the remaining port. It is a standalone android-activity app (no loft, no winit, raw
`#[link]` EGL/GLES) — the smallest thing that could falsify "GLES works on the emulator".

## Proof

`loft_b3_spike` creates an EGL/GLES-3.0 context on `app.native_window()` and clears to
orange every frame. On a headless KVM emulator (x86_64 image, API 34, `-gpu
swiftshader_indirect`):

```
I/loft: b3: EGL/GLES ready — window 320x640, 1 config(s)
I/loft: b3: drew frame 1 (orange)      ... frame 21 ... frame 41
D/EGL_emulation: app_time_stats: ...   (SwiftShader GLES actually running)
```

`adb exec-out screencap -p` → `b3_screen_orange.png`: **center pixel (255,128,0), 99.5% of the
screen that exact orange** (the 0.5% is the status bar). So: EGL-on-ANativeWindow ✓, GLES-3.0
context ✓, emulator SwiftShader renders ✓, `glClear` colour is exact ✓, screencap golden ✓.
This is the B3 `(G)` gate for a solid clear-colour — the surface/context path holds.

## What the full port changes (in `tests/fixtures/libs/graphics/native/`)

The backend is winit 0.30 + glutin 0.32 + `gl`; the same source runs on Android because winit
0.30 has an android-activity backend. Three concrete diffs:

1. **GLES context, not desktop GL.** `window.rs:105` builds
   `ContextApi::OpenGl(Some(Version::new(3,3)))`. Android needs
   `ContextApi::Gles(Some(Version::new(3,0)))`. Pick by target (`cfg!(target_os = "android")`).
   The `gl` crate loads GLES entry points through the same `get_proc_address`, and the
   `gl_*` call surface (clear/draw/shaders/FBOs) is a GLES-3.0 subset already — the design's §5
   GLES-subset risk applies only to any desktop-only call the shaders use (audit at port time).
2. **Surface creation is deferred to `Resumed`/`InitWindow`.** `window.rs:create_gl_state`
   builds the window+surface **synchronously**; on Android the `ANativeWindow` does not exist
   until the first resume. So on Android, `create_gl_state` must build the winit `EventLoop`
   (with the AndroidApp — see seam below), then **`pump_app_events` until the first resume**
   yields a window, and only then create the glutin surface + context. `lib.rs:301`
   `loft_gl_poll_events` already drives the loop with `pump_app_events`, so the steady-state
   pump is reused; only the *initial* create becomes pump-until-resumed.
3. **Suspend/resume recreates the surface.** `TerminateWindow` (app backgrounded) destroys the
   `ANativeWindow` → drop the glutin surface; the next `InitWindow` recreates it against the
   same context. `gl_*` calls made while suspended must no-op (extend the `GL_READY` guard).

## The load-bearing unknown — the AndroidApp seam

winit's android `EventLoop` needs the `AndroidApp` at build time
(`EventLoop::builder().with_android_app(app)`), but loft's programs call
`loft_gl_create_window(w,h,title)` imperatively — they never see the `AndroidApp`. loft's
emitted entry (`src/android.rs` `ANDROID_MAIN_TAIL`) *does* have it. So the seam is: **the
entry hands the `AndroidApp` to `lib/graphics` before the loft program runs.**

Options, cheapest-safe first — decide with a probe next session (design-protocol):

- **(A) Graphics-aware entry.** Make `--native-android`'s runtime-entry descriptor emit a
  *graphics* tail when the program uses `lib/graphics`: the tail owns the winit android event
  loop and calls the loft program's render step from `Resumed`. Cleanest lifecycle fit, but
  couples loft's android backend to a specific library's shape (layering cost).
- **(B) C-ABI init seam.** `lib/graphics` exports `loft_gl_android_init(app)`; the tail calls
  it before `main()`, stashing the app in a `OnceLock`. Keeps the imperative `gl_*` model.
  **Risk to probe:** `AndroidApp` is not `#[repr(C)]`; passing it across the native-package
  cdylib C-ABI boundary is only sound if cargo unified `android-activity` to one version
  across the generated crate *and* `lib/graphics` (likely, both on 0.6, but VERIFY — a second
  instance makes the type ABI-incompatible). A `Box::into_raw` handoff sidesteps by-value ABI.
- **(C) ndk_context.** android-activity populates `ndk_context` (JavaVM + Activity) globally;
  `lib/graphics` could pull the window from the activity via JNI. Heaviest; avoids the seam
  but adds a JNI surface. Not recommended unless (A)/(B) both fail.

Recommendation: **(A)** for a real graphics app (the entry SHOULD own the loop on Android),
with (B) as the fallback if the layering coupling is unacceptable. Either way the `gl_*`
surface and shaders are unchanged — only window/context creation and the entry move.

## Files here (reproduce, don't depend)

`Cargo.toml` + `src/lib.rs` — the raw-EGL spike (build for `x86_64-linux-android` with the NDK
linker via `cargo-config.toml`; package with `../b2-spike/build_apk.sh`, run with
`../b2-spike/run_emulator_test.sh`, then `adb exec-out screencap -p`). `b3_screen_orange.png`
is the committed golden (99.5% `#ff8000`).

## Not this spike (still B3/B4)

Text/shaders/textures on-device (the loft graphics programs exercise the full `gl_*` surface —
port + golden them incrementally), and B4 touch/IME. This spike only fixes the surface path.
