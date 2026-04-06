// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Window creation + OpenGL context initialization using glutin + winit.

use super::GlState;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::SwapInterval;
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use std::num::NonZeroU32;
use winit::dpi::LogicalSize;
use winit::event_loop::EventLoop;
use winit::window::WindowAttributes;

pub fn create_gl_state(width: u32, height: u32, title: &str) -> Result<GlState, String> {
    let event_loop = EventLoop::new().map_err(|e| format!("EventLoop: {e}"))?;

    let window_attrs = WindowAttributes::default()
        .with_title(title)
        .with_transparent(false)
        .with_inner_size(LogicalSize::new(width, height));

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

    unsafe {
        gl::Enable(gl::DEPTH_TEST);
        gl::Viewport(0, 0, width as i32, height as i32);
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

    let _ = surface.set_swap_interval(
        &context,
        SwapInterval::Wait(NonZeroU32::new(1).unwrap()),
    );

    Ok(GlState {
        window,
        surface,
        context,
        event_loop,
        should_close: false,
    })
}
