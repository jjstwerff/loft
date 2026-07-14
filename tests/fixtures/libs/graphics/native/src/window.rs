// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Window creation + OpenGL context initialization using glutin + winit.

use super::GlState;
#[cfg(not(target_os = "android"))]
use glutin::config::ConfigTemplateBuilder;
#[cfg(not(target_os = "android"))]
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
#[cfg(not(target_os = "android"))]
use glutin::display::GetGlDisplay;
#[cfg(not(target_os = "android"))]
use glutin::prelude::*;
#[cfg(not(target_os = "android"))]
use glutin::surface::SwapInterval;
#[cfg(not(target_os = "android"))]
use glutin_winit::{DisplayBuilder, GlWindow};
#[cfg(not(target_os = "android"))]
use raw_window_handle::HasWindowHandle;
#[cfg(not(target_os = "android"))]
use std::num::NonZeroU32;
#[cfg(not(target_os = "android"))]
use winit::dpi::LogicalSize;
#[cfg(not(target_os = "android"))]
use winit::event_loop::EventLoop;
#[cfg(not(target_os = "android"))]
use winit::window::{Fullscreen, WindowAttributes};

/// Android: attach a GLES-3.0 context to the OS-provided `ANativeWindow`
/// (`android_gl`); `title` is unused (no window chrome). `width`/`height` are a
/// fallback viewport hint — the real size comes from the window.
#[cfg(target_os = "android")]
pub fn create_gl_state(width: u32, height: u32, title: &str) -> Result<GlState, String> {
    let _ = title;
    crate::android_gl::create_gl_state_android(width, height)
}

/// Android: a NativeActivity is always fullscreen, so this is the same attach.
#[cfg(target_os = "android")]
pub fn create_gl_state_fullscreen(title: &str) -> Result<GlState, String> {
    let _ = title;
    crate::android_gl::create_gl_state_android(0, 0)
}

#[cfg(not(target_os = "android"))]
pub fn create_gl_state(width: u32, height: u32, title: &str) -> Result<GlState, String> {
    let attrs = WindowAttributes::default()
        .with_title(title)
        .with_transparent(false)
        .with_inner_size(LogicalSize::new(width, height));
    create_gl_state_with_attrs(attrs, Some((width, height)), false)
}

/// P184-era initiative 03: borderless-fullscreen window.  Defaults to the
/// primary monitor, but honours `LOFT_GL_MONITOR` (a substring matched
/// against monitor connector names, e.g. "HDMI") to target a specific
/// output — useful for putting the demo on a beamer.  The GL viewport is
/// sized from the actual window size after creation.
#[cfg(not(target_os = "android"))]
pub fn create_gl_state_fullscreen(title: &str) -> Result<GlState, String> {
    let attrs = WindowAttributes::default()
        .with_title(title)
        .with_transparent(false);
    create_gl_state_with_attrs(attrs, None, true)
}

#[cfg(not(target_os = "android"))]
fn create_gl_state_with_attrs(
    mut window_attrs: WindowAttributes,
    initial_viewport: Option<(u32, u32)>,
    fullscreen: bool,
) -> Result<GlState, String> {
    let event_loop = EventLoop::new().map_err(|e| format!("EventLoop: {e}"))?;

    // Open fullscreen on the default monitor first; a specific monitor is
    // selected after creation (winit 0.30 exposes monitor enumeration on the
    // Window, not the EventLoop).
    if fullscreen {
        window_attrs = window_attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
    }

    let config_template = ConfigTemplateBuilder::new();

    let (window, gl_config) = DisplayBuilder::new()
        .with_window_attributes(Some(window_attrs))
        .build(&event_loop, config_template, |configs| {
            // Strongly prefer configs without an alpha channel — an alpha
            // channel makes the compositor treat the window as transparent.
            configs
                .reduce(|a, b| {
                    let a_opaque = a.alpha_size() == 0;
                    let b_opaque = b.alpha_size() == 0;
                    if a_opaque && !b_opaque {
                        return a;
                    }
                    if b_opaque && !a_opaque {
                        return b;
                    }
                    if a.num_samples() > b.num_samples() {
                        a
                    } else {
                        b
                    }
                })
                .unwrap()
        })
        .map_err(|e| format!("DisplayBuilder: {e}"))?;

    let window = window.ok_or("No window created")?;

    // Move the fullscreen window to a named monitor (LOFT_GL_MONITOR
    // substring, e.g. "HDMI") if requested — for putting the demo on a
    // beamer.  Capture the target's size so the viewport / window-size
    // getters use it directly (inner_size lags an async monitor move).
    let mut monitor_size: Option<(u32, u32)> = None;
    if fullscreen
        && let Ok(name) = std::env::var("LOFT_GL_MONITOR")
        && !name.is_empty()
        && let Some(mon) = window
            .available_monitors()
            .find(|m| m.name().map(|n| n.contains(&name)).unwrap_or(false))
    {
        let sz = mon.size();
        monitor_size = Some((sz.width.max(1), sz.height.max(1)));
        window.set_fullscreen(Some(Fullscreen::Borderless(Some(mon))));
    }
    let gl_display = gl_config.display();

    let raw_handle = window
        .window_handle()
        .map_err(|e| format!("window_handle: {e}"))?
        .as_raw();

    let context_attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(Some(raw_handle));

    let not_current = unsafe { gl_display.create_context(&gl_config, &context_attrs) }
        .map_err(|e| format!("create_context: {e}"))?;

    let surface_attrs = window
        .build_surface_attributes(<_>::default())
        .map_err(|e| format!("surface_attrs: {e}"))?;
    let surface = unsafe { gl_display.create_window_surface(&gl_config, &surface_attrs) }
        .map_err(|e| format!("create_surface: {e}"))?;

    let context = not_current
        .make_current(&surface)
        .map_err(|e| format!("make_current: {e}"))?;

    // Load GL function pointers
    gl::load_with(|s| {
        gl_display
            .get_proc_address(&std::ffi::CString::new(s).unwrap())
            .cast()
    });

    // Viewport size: caller's windowed hint, else the selected monitor's
    // size (a moved-to monitor's inner_size lags), else the post-creation
    // inner_size (default-monitor fullscreen at native resolution).
    let (vw, vh) = match (initial_viewport, monitor_size) {
        (Some((w, h)), _) => (w, h),
        (None, Some((w, h))) => (w, h),
        (None, None) => {
            let sz = window.inner_size();
            (sz.width.max(1), sz.height.max(1))
        }
    };

    unsafe {
        gl::Enable(gl::DEPTH_TEST);
        gl::Viewport(0, 0, vw as i32, vh as i32);
        // Clear both front and back buffers to opaque black before the window
        // becomes visible, preventing see-through artifacts on compositors.
        gl::ClearColor(0.0, 0.0, 0.0, 1.0);
        gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
    }
    // Force alpha=1.0 before presenting (same as loft_gl_swap_buffers does).
    unsafe {
        gl::ColorMask(gl::FALSE, gl::FALSE, gl::FALSE, gl::TRUE);
        gl::ClearColor(0.0, 0.0, 0.0, 1.0);
        gl::Clear(gl::COLOR_BUFFER_BIT);
        gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE);
    }
    let _ = surface.swap_buffers(&context);
    unsafe {
        gl::ClearColor(0.0, 0.0, 0.0, 1.0);
        gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        gl::ColorMask(gl::FALSE, gl::FALSE, gl::FALSE, gl::TRUE);
        gl::ClearColor(0.0, 0.0, 0.0, 1.0);
        gl::Clear(gl::COLOR_BUFFER_BIT);
        gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE);
    }
    let _ = surface.swap_buffers(&context);

    let _ = surface.set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()));

    Ok(GlState {
        window,
        surface,
        context,
        event_loop,
        should_close: false,
        viewport_w: vw,
        viewport_h: vh,
    })
}
