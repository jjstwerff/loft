//! B3 spike — prove GLES renders on the Android ANativeWindow.
//!
//! Standalone android-activity app (no loft, no winit): on the first InitWindow
//! event it creates an EGL/GLES-3.0 context on `app.native_window()`, clears to a
//! distinctive orange, and swaps — every frame. We screencap the emulator and check
//! the pixel is orange. This de-risks EGL-on-ANativeWindow + the emulator's
//! SwiftShader GLES + the Resumed/InitWindow lifecycle before porting lib/graphics.
//!
//! Raw `#[link]` EGL/GLES (the NDK's libEGL/libGLESv2), so there is no cross-compile
//! build-script to fight — glutin does this same EGL dance internally in the port.

use android_activity::{AndroidApp, MainEvent, PollEvent};
use std::ffi::c_void;
use std::time::Duration;

// ── Raw EGL / GLES bindings (NDK libEGL / libGLESv2) ────────────────────────
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
    fn eglMakeCurrent(
        dpy: EglDisplay,
        draw: EglSurface,
        read: EglSurface,
        ctx: EglContext,
    ) -> u32;
    fn eglSwapBuffers(dpy: EglDisplay, surface: EglSurface) -> u32;
    fn eglGetError() -> i32;
}

#[link(name = "GLESv2")]
unsafe extern "C" {
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);
    fn glViewport(x: i32, y: i32, w: i32, h: i32);
    fn glFinish();
}

const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_WINDOW_BIT: i32 = 0x0004;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES2_BIT: i32 = 0x0004; // also advertises ES3-capable configs
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_NONE: i32 = 0x3038;
const EGL_CONTEXT_MAJOR_VERSION: i32 = 0x3098;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;

struct Gl {
    display: EglDisplay,
    surface: EglSurface,
}

fn init_gl(app: &AndroidApp) -> Result<Gl, String> {
    let win = app.native_window().ok_or("no native window yet")?;
    let (w, h) = (win.width(), win.height());
    unsafe {
        let display = eglGetDisplay(std::ptr::null_mut());
        if display.is_null() {
            return Err("eglGetDisplay returned NO_DISPLAY".into());
        }
        if eglInitialize(display, std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
            return Err(format!("eglInitialize failed (0x{:x})", eglGetError()));
        }
        eglBindAPI(EGL_OPENGL_ES_API);
        let cfg_attribs: [i32; 13] = [
            EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
            EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE, 8,
            EGL_GREEN_SIZE, 8,
            EGL_BLUE_SIZE, 8,
            EGL_ALPHA_SIZE, 8,
            EGL_NONE,
        ];
        let mut config: EglConfig = std::ptr::null_mut();
        let mut num_config: i32 = 0;
        if eglChooseConfig(display, cfg_attribs.as_ptr(), &mut config, 1, &mut num_config) == 0
            || num_config == 0
        {
            return Err(format!("eglChooseConfig found none (0x{:x})", eglGetError()));
        }
        let ctx_attribs: [i32; 3] = [EGL_CONTEXT_MAJOR_VERSION, 3, EGL_NONE];
        let context = eglCreateContext(display, config, std::ptr::null_mut(), ctx_attribs.as_ptr());
        if context.is_null() {
            return Err(format!("eglCreateContext failed (0x{:x})", eglGetError()));
        }
        let win_ptr = win.ptr().as_ptr().cast();
        let surface = eglCreateWindowSurface(display, config, win_ptr, std::ptr::null());
        if surface.is_null() {
            return Err(format!("eglCreateWindowSurface failed (0x{:x})", eglGetError()));
        }
        if eglMakeCurrent(display, surface, surface, context) == 0 {
            return Err(format!("eglMakeCurrent failed (0x{:x})", eglGetError()));
        }
        glViewport(0, 0, w as i32, h as i32);
        log::info!("b3: EGL/GLES ready — window {w}x{h}, {num_config} config(s)");
        Ok(Gl { display, surface })
    }
}

fn draw(gl: &Gl) {
    unsafe {
        // Distinctive orange so a screencap can't be confused with black clears.
        glClearColor(1.0, 0.5, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        eglSwapBuffers(gl.display, gl.surface);
    }
}

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
extern "C" fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("loft"),
    );
    log::info!("b3: android_main reached");
    let mut gl: Option<Gl> = None;
    let mut quit = false;
    let mut frames = 0u32;
    while !quit {
        app.poll_events(Some(Duration::from_millis(100)), |event| match event {
            PollEvent::Main(MainEvent::InitWindow { .. }) => {
                log::info!("b3: InitWindow");
                match init_gl(&app) {
                    Ok(g) => gl = Some(g),
                    Err(e) => log::error!("b3: init_gl failed: {e}"),
                }
            }
            PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                log::info!("b3: TerminateWindow");
                gl = None;
            }
            PollEvent::Main(MainEvent::Destroy) => quit = true,
            _ => {}
        });
        if let Some(g) = &gl {
            draw(g);
            frames += 1;
            if frames % 20 == 1 {
                log::info!("b3: drew frame {frames} (orange)");
            }
        }
    }
    log::info!("b3: exiting");
}
