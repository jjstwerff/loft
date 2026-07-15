// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN106 B3 — Android GL backend for `lib/graphics`.
//!
//! The desktop backend (`window.rs`) creates a glutin+winit window with a desktop
//! GL 3.3 context, synchronously. Android differs in exactly the surface layer: the
//! window (`ANativeWindow`) is provided by the OS and only exists after the activity
//! resumes, and the context is GLES 3.0. Everything above the surface — every
//! `gl::*` draw/shader/texture call in `lib.rs`/`shader.rs`/`text.rs` — is unchanged,
//! because GLES 3.0 is the same API the loft website already targets (WebGL2), so a
//! program that runs on the website runs here without changes.
//!
//! This mirrors the website model ("attach to a surface the environment gives you")
//! rather than the desktop one ("create a window"): raw EGL on `app.native_window()`,
//! `gl::load_with(eglGetProcAddress)` to fill the same `gl` bindings the shared code
//! calls, and android-activity's own event pump for `loft_gl_poll_events`.
//!
//! **The AndroidApp seam.** The OS entry (`ANativeActivity_onCreate` → `android_main`)
//! lives in loft's generated `.so`, not here, so loft's emitted entry hands us the
//! `AndroidApp` via [`loft_gl_android_set_app`] before the program's `main` runs. The
//! type crosses soundly because loft links this crate as a unified rlib on Android, so
//! `android-activity` is one instance (see `src/android.rs`).

use super::GlState;
use android_activity::input::{InputEvent, KeyAction, Keycode, MotionAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use std::ffi::{CString, c_char, c_void};
use std::sync::Mutex;
use std::time::Duration;

// ── Raw EGL / GLES (the NDK's libEGL / libGLESv2) ───────────────────────────
type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglContext = *mut c_void;
type EglSurface = *mut c_void;

#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetDisplay(display_id: *mut c_void) -> EglDisplay;
    fn eglInitialize(dpy: EglDisplay, major: *mut i32, minor: *mut i32) -> u32;
    fn eglBindAPI(api: u32) -> u32;
    fn eglChooseConfig(
        dpy: EglDisplay,
        attrib_list: *const i32,
        configs: *mut EglConfig,
        config_size: i32,
        num_config: *mut i32,
    ) -> u32;
    fn eglCreateContext(
        dpy: EglDisplay,
        config: EglConfig,
        share: EglContext,
        attrib_list: *const i32,
    ) -> EglContext;
    fn eglCreateWindowSurface(
        dpy: EglDisplay,
        config: EglConfig,
        win: *mut c_void,
        attrib_list: *const i32,
    ) -> EglSurface;
    fn eglDestroySurface(dpy: EglDisplay, surface: EglSurface) -> u32;
    fn eglMakeCurrent(dpy: EglDisplay, draw: EglSurface, read: EglSurface, ctx: EglContext) -> u32;
    fn eglSwapBuffers(dpy: EglDisplay, surface: EglSurface) -> u32;
    fn eglGetProcAddress(procname: *const c_char) -> *const c_void;
    fn eglGetError() -> i32;
}

const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_WINDOW_BIT: i32 = 0x0004;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES2_BIT: i32 = 0x0004; // also advertises ES3-capable configs
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_DEPTH_SIZE: i32 = 0x3025;
const EGL_NONE: i32 = 0x3038;
const EGL_CONTEXT_MAJOR_VERSION: i32 = 0x3098;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_NO_SURFACE: EglSurface = std::ptr::null_mut();

/// The `AndroidApp`, handed over by loft's generated entry before the program runs.
static ANDROID_APP: Mutex<Option<AndroidApp>> = Mutex::new(None);

/// loft's emitted `android_main` calls this (once) with `&app as *const c_void`
/// before invoking the program's `main`, so `loft_gl_create_window` has the
/// `AndroidApp` to pump events and reach `native_window()`. Sound because loft links
/// this crate as a unified rlib on Android — one `android-activity`, one `AndroidApp`
/// type — see `src/android.rs`. A no-op if the pointer is null.
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_android_set_app(app_ptr: *const c_void) {
    if app_ptr.is_null() {
        return;
    }
    let app = unsafe { &*(app_ptr as *const AndroidApp) };
    *ANDROID_APP.lock().unwrap() = Some(app.clone());
}

/// The GLES surface + context bound to the current `ANativeWindow`, plus the app we
/// pump for events. `GlState` (on Android) holds one of these.
pub(crate) struct AndroidGl {
    app: AndroidApp,
    display: EglDisplay,
    surface: EglSurface,
    context: EglContext,
    config: EglConfig,
}

/// Create the GLES-3.0 context/surface on the Android window — the `create_gl_state`
/// android path. Pumps the activity until the `ANativeWindow` exists, then sets up
/// EGL and loads the `gl` bindings via `eglGetProcAddress` so all the shared
/// `gl::*` code works unchanged. `width`/`height` are only a fallback viewport hint;
/// the real size comes from the window.
pub(crate) fn create_gl_state_android(width: u32, height: u32) -> Result<GlState, String> {
    let app = ANDROID_APP
        .lock()
        .unwrap()
        .clone()
        .ok_or("android app not set (loft_gl_android_set_app was not called by the entry)")?;

    // The ANativeWindow only exists after the first InitWindow; pump until it does
    // (bounded so a never-resumed app fails instead of hanging).
    let mut window = app.native_window();
    let mut tries = 0;
    while window.is_none() {
        app.poll_events(Some(Duration::from_millis(50)), |_| {});
        window = app.native_window();
        tries += 1;
        if tries > 200 {
            return Err("no ANativeWindow after ~10s (activity never resumed)".into());
        }
    }
    let window = window.unwrap();
    let (vw, vh) = (window.width().max(1) as u32, window.height().max(1) as u32);
    let (vw, vh) = if vw > 1 && vh > 1 {
        (vw, vh)
    } else {
        (width.max(1), height.max(1))
    };

    unsafe {
        let display = eglGetDisplay(std::ptr::null_mut());
        if display.is_null() {
            return Err("eglGetDisplay returned NO_DISPLAY".into());
        }
        if eglInitialize(display, std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
            return Err(format!("eglInitialize failed (0x{:x})", eglGetError()));
        }
        eglBindAPI(EGL_OPENGL_ES_API);
        #[rustfmt::skip]
        let cfg_attribs: [i32; 15] = [
            EGL_SURFACE_TYPE,    EGL_WINDOW_BIT,
            EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE,   8,
            EGL_GREEN_SIZE, 8,
            EGL_BLUE_SIZE,  8,
            EGL_ALPHA_SIZE, 8,
            EGL_DEPTH_SIZE, 16,
            EGL_NONE,
        ];
        let mut config: EglConfig = std::ptr::null_mut();
        let mut num_config: i32 = 0;
        if eglChooseConfig(
            display,
            cfg_attribs.as_ptr(),
            &mut config,
            1,
            &mut num_config,
        ) == 0
            || num_config == 0
        {
            return Err(format!(
                "eglChooseConfig found none (0x{:x})",
                eglGetError()
            ));
        }
        let ctx_attribs: [i32; 3] = [EGL_CONTEXT_MAJOR_VERSION, 3, EGL_NONE];
        let context = eglCreateContext(display, config, std::ptr::null_mut(), ctx_attribs.as_ptr());
        if context.is_null() {
            return Err(format!("eglCreateContext failed (0x{:x})", eglGetError()));
        }
        let win_ptr = window.ptr().as_ptr().cast();
        let surface = eglCreateWindowSurface(display, config, win_ptr, std::ptr::null());
        if surface.is_null() {
            return Err(format!(
                "eglCreateWindowSurface failed (0x{:x})",
                eglGetError()
            ));
        }
        if eglMakeCurrent(display, surface, surface, context) == 0 {
            return Err(format!("eglMakeCurrent failed (0x{:x})", eglGetError()));
        }
        // Fill the `gl` crate's bindings with GLES entry points so every shared
        // `gl::*` call resolves — identical to the desktop `gl::load_with`.
        gl::load_with(|s| {
            let cs = CString::new(s).unwrap();
            eglGetProcAddress(cs.as_ptr()) as *const _
        });
        gl::Enable(gl::DEPTH_TEST);
        gl::Viewport(0, 0, vw as i32, vh as i32);
        gl::ClearColor(0.0, 0.0, 0.0, 1.0);
        gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        eglSwapBuffers(display, surface);

        Ok(GlState {
            android: AndroidGl {
                app,
                display,
                surface,
                context,
                config,
            },
            should_close: false,
            viewport_w: vw,
            viewport_h: vh,
        })
    }
}

/// Present the frame — the `loft_gl_swap_buffers` android path.
pub(crate) fn swap(state: &GlState) {
    unsafe {
        // @PLN106 B3 diagnostic: surface the first GL error each frame (once) so
        // GLES-3.0 draw-pipeline problems are visible in logcat.
        let e = gl::GetError();
        if e != 0 {
            use std::sync::atomic::{AtomicBool, Ordering};
            static LOGGED: AtomicBool = AtomicBool::new(false);
            if !LOGGED.swap(true, Ordering::Relaxed) {
                eprintln!("loft-gl: GL error 0x{e:x} during draw");
            }
        }
        eglSwapBuffers(state.android.display, state.android.surface);
    }
}

/// @PLN106 B4 — drain pending touch events into the shared pointer state (`MOUSE_X/Y/BTN` in
/// `lib.rs`, which `loft_gl_mouse_*` read). Touch → mouse: a press/release maps to the left
/// button, moves update the position. Only the action's pointer is tracked, matching the
/// single-cursor desktop model. Must run on the `android_main` thread (an `input_events_iter`
/// invariant) — which `poll` already is.
fn drain_input(app: &AndroidApp) {
    let Ok(mut iter) = app.input_events_iter() else {
        return;
    };
    loop {
        let read = iter.next(|event| match event {
            InputEvent::MotionEvent(motion) => {
                let p = motion.pointer_at_index(motion.pointer_index());
                crate::set_pointer_pos(f64::from(p.x()), f64::from(p.y()));
                match motion.action() {
                    MotionAction::Down | MotionAction::PointerDown => crate::set_pointer_down(true),
                    MotionAction::Up | MotionAction::PointerUp | MotionAction::Cancel => {
                        crate::set_pointer_down(false);
                    }
                    _ => {}
                }
                InputStatus::Handled
            }
            // @PLN106 B4 IME — map a KeyEvent to the shared `KEYS` state so `gl_key_pressed`
            // reads typed keys (hardware keyboard + soft-keyboard keys that emit KeyEvents).
            // Unmapped keys stay Unhandled so the system still handles Back / Volume / etc.
            InputEvent::KeyEvent(key) => match keycode_to_index(key.key_code()) {
                Some(idx) => {
                    crate::set_key(idx, matches!(key.action(), KeyAction::Down));
                    InputStatus::Handled
                }
                None => InputStatus::Unhandled,
            },
            _ => InputStatus::Unhandled,
        });
        if !read {
            break;
        }
    }
}

/// Map an Android [`Keycode`] to the 0-255 index `gl_key_pressed` reads, matching the desktop
/// `key_index` scheme (letters → lowercase ASCII, digits → ASCII, arrows → 128-131, plus the
/// common editing keys). Returns `None` for keys the shared model doesn't cover.
#[allow(clippy::match_same_arms)]
fn keycode_to_index(kc: Keycode) -> Option<u8> {
    let idx: u8 = match kc {
        Keycode::A => b'a',
        Keycode::B => b'b',
        Keycode::C => b'c',
        Keycode::D => b'd',
        Keycode::E => b'e',
        Keycode::F => b'f',
        Keycode::G => b'g',
        Keycode::H => b'h',
        Keycode::I => b'i',
        Keycode::J => b'j',
        Keycode::K => b'k',
        Keycode::L => b'l',
        Keycode::M => b'm',
        Keycode::N => b'n',
        Keycode::O => b'o',
        Keycode::P => b'p',
        Keycode::Q => b'q',
        Keycode::R => b'r',
        Keycode::S => b's',
        Keycode::T => b't',
        Keycode::U => b'u',
        Keycode::V => b'v',
        Keycode::W => b'w',
        Keycode::X => b'x',
        Keycode::Y => b'y',
        Keycode::Z => b'z',
        Keycode::Keycode0 => b'0',
        Keycode::Keycode1 => b'1',
        Keycode::Keycode2 => b'2',
        Keycode::Keycode3 => b'3',
        Keycode::Keycode4 => b'4',
        Keycode::Keycode5 => b'5',
        Keycode::Keycode6 => b'6',
        Keycode::Keycode7 => b'7',
        Keycode::Keycode8 => b'8',
        Keycode::Keycode9 => b'9',
        Keycode::Space => b' ',
        Keycode::Enter | Keycode::NumpadEnter => b'\r',
        Keycode::Tab => 9,
        Keycode::Escape => 27,
        Keycode::Del => 8,      // backspace
        Keycode::DpadUp => 128, // matches desktop key_index arrow codes
        Keycode::DpadDown => 129,
        Keycode::DpadLeft => 130,
        Keycode::DpadRight => 131,
        _ => return None,
    };
    Some(idx)
}

/// @PLN106 B4 IME — raise the soft keyboard so its keys arrive as `KeyEvent`s. Uses the
/// `AndroidApp` handed to the crate via `loft_gl_android_set_app`.
pub(crate) fn show_keyboard() {
    if let Some(app) = ANDROID_APP.lock().unwrap().as_ref() {
        app.show_soft_input(true);
    }
}

/// @PLN106 B4 IME — dismiss the soft keyboard.
pub(crate) fn hide_keyboard() {
    if let Some(app) = ANDROID_APP.lock().unwrap().as_ref() {
        app.hide_soft_input(false);
    }
}

/// Pump one round of activity events — the `loft_gl_poll_events` android path.
/// Returns `false` when the app should close. Handles the surface lifecycle
/// (Terminate/Init) so a backgrounded+foregrounded app keeps rendering, and (B4) drains
/// touch input into the shared pointer state.
pub(crate) fn poll(state: &mut GlState) -> bool {
    // Clone the app handle out first so the poll closure can mutate `state`.
    let app = state.android.app.clone();
    let mut close = false;
    let mut resized: Option<(u32, u32)> = None;
    let mut window_lost = false;
    let mut window_gained = false;
    let mut input_available = false;
    app.poll_events(Some(Duration::ZERO), |event| match event {
        PollEvent::Main(MainEvent::Destroy) => close = true,
        PollEvent::Main(MainEvent::TerminateWindow { .. }) => window_lost = true,
        PollEvent::Main(MainEvent::InitWindow { .. }) => window_gained = true,
        PollEvent::Main(MainEvent::WindowResized { .. }) => {
            if let Some(w) = app.native_window() {
                resized = Some((w.width().max(1) as u32, w.height().max(1) as u32));
            }
        }
        PollEvent::Main(MainEvent::InputAvailable) => input_available = true,
        _ => {}
    });

    // @PLN106 B4 — drain pending touch into the shared pointer state (AFTER poll_events, so
    // `input_events_iter` isn't a re-borrow of `app` inside its own poll closure).
    if input_available {
        drain_input(&app);
    }

    if window_lost {
        unsafe {
            eglMakeCurrent(
                state.android.display,
                EGL_NO_SURFACE,
                EGL_NO_SURFACE,
                std::ptr::null_mut(),
            );
            if !state.android.surface.is_null() {
                eglDestroySurface(state.android.display, state.android.surface);
                state.android.surface = std::ptr::null_mut();
            }
        }
    }
    if window_gained && state.android.surface.is_null() {
        if let Some(win) = app.native_window() {
            unsafe {
                let win_ptr = win.ptr().as_ptr().cast();
                let surface = eglCreateWindowSurface(
                    state.android.display,
                    state.android.config,
                    win_ptr,
                    std::ptr::null(),
                );
                if !surface.is_null() {
                    eglMakeCurrent(
                        state.android.display,
                        surface,
                        surface,
                        state.android.context,
                    );
                    state.android.surface = surface;
                    resized = Some((win.width().max(1) as u32, win.height().max(1) as u32));
                }
            }
        }
    }
    if let Some((w, h)) = resized {
        unsafe { gl::Viewport(0, 0, w as i32, h as i32) };
        state.viewport_w = w;
        state.viewport_h = h;
    }
    if close {
        state.should_close = true;
    }
    !state.should_close
}
