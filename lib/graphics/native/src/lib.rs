// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native OpenGL support for the graphics package.
//! Uses glutin for window/context and gl for OpenGL bindings.

#![allow(clippy::missing_safety_doc)]

use glutin::prelude::*;
use std::cell::RefCell;
use std::ffi::CString;
use std::time::Duration;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::WindowId;

mod shader;
mod text;
mod window;

// ── Thread-local GL state ───────────────────────────────────────────────

struct GlState {
    window: winit::window::Window,
    surface: glutin::surface::Surface<glutin::surface::WindowSurface>,
    context: glutin::context::PossiblyCurrentContext,
    event_loop: winit::event_loop::EventLoop<()>,
    should_close: bool,
}

thread_local! {
    static GL: RefCell<Option<GlState>> = const { RefCell::new(None) };
}

fn with_gl<R>(f: impl FnOnce(&GlState) -> R) -> Option<R> {
    GL.with(|cell| cell.borrow().as_ref().map(f))
}

fn with_gl_mut<R>(f: impl FnOnce(&mut GlState) -> R) -> Option<R> {
    GL.with(|cell| cell.borrow_mut().as_mut().map(f))
}

// Minimal event handler for pump_app_events
struct JsonApp {
    should_close: bool,
}

impl ApplicationHandler for JsonApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}
    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            self.should_close = true;
        }
    }
}

// ── C-ABI exports ───────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_create_window(
    width: u32,
    height: u32,
    title_ptr: *const u8,
    title_len: usize,
) -> bool {
    let title = unsafe { loft_ffi::text(title_ptr, title_len) };
    match window::create_gl_state(width, height, title) {
        Ok(state) => {
            GL.with(|cell| *cell.borrow_mut() = Some(state));
            true
        }
        Err(e) => {
            eprintln!("loft_gl_create_window: {e}");
            false
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_poll_events() -> bool {
    with_gl_mut(|s| {
        let mut handler = JsonApp {
            should_close: false,
        };
        let status = s
            .event_loop
            .pump_app_events(Some(Duration::ZERO), &mut handler);
        if handler.should_close || matches!(status, PumpStatus::Exit(_)) {
            s.should_close = true;
        }
        !s.should_close
    })
    .unwrap_or(false)
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_swap_buffers() {
    with_gl(|s| {
        let _ = s.surface.swap_buffers(&s.context);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_clear(color: u32) {
    let r = ((color >> 24) & 0xFF) as f32 / 255.0;
    let g = ((color >> 16) & 0xFF) as f32 / 255.0;
    let b = ((color >> 8) & 0xFF) as f32 / 255.0;
    let a = (color & 0xFF) as f32 / 255.0;
    unsafe {
        gl::ClearColor(r, g, b, a);
        gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_destroy_window() {
    GL.with(|cell| *cell.borrow_mut() = None);
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_create_shader(
    vert_ptr: *const u8,
    vert_len: usize,
    frag_ptr: *const u8,
    frag_len: usize,
) -> u32 {
    let vert = unsafe { loft_ffi::text(vert_ptr, vert_len) };
    let frag = unsafe { loft_ffi::text(frag_ptr, frag_len) };
    shader::compile_program(vert, frag).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_use_shader(program: u32) {
    unsafe {
        gl::UseProgram(program);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_gl_upload_mesh(
    data_ptr: *const f32,
    n_vertices: u32,
    stride: u32,
    out_vao: *mut u32,
    out_vbo: *mut u32,
) {
    let mut vao = 0u32;
    let mut vbo = 0u32;
    unsafe {
        gl::GenVertexArrays(1, &mut vao);
        gl::GenBuffers(1, &mut vbo);
        gl::BindVertexArray(vao);
        gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
        let byte_size = (n_vertices * stride * 4) as isize;
        gl::BufferData(
            gl::ARRAY_BUFFER,
            byte_size,
            data_ptr.cast(),
            gl::STATIC_DRAW,
        );
        // Position: location 0, 3 floats
        gl::VertexAttribPointer(0, 3, gl::FLOAT, gl::FALSE, (stride * 4) as i32, std::ptr::null());
        gl::EnableVertexAttribArray(0);
        // Normal: location 1, 3 floats at offset 12
        if stride >= 6 {
            gl::VertexAttribPointer(
                1,
                3,
                gl::FLOAT,
                gl::FALSE,
                (stride * 4) as i32,
                (3 * 4) as *const _,
            );
            gl::EnableVertexAttribArray(1);
        }
        // Color: location 2, 4 floats at offset 24
        if stride >= 10 {
            gl::VertexAttribPointer(
                2,
                4,
                gl::FLOAT,
                gl::FALSE,
                (stride * 4) as i32,
                (6 * 4) as *const _,
            );
            gl::EnableVertexAttribArray(2);
        }
        gl::BindVertexArray(0);
        *out_vao = vao;
        *out_vbo = vbo;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_draw(vao: u32, n_vertices: u32) {
    unsafe {
        gl::BindVertexArray(vao);
        gl::DrawArrays(gl::TRIANGLES, 0, n_vertices as i32);
        gl::BindVertexArray(0);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_gl_set_uniform_mat4(
    program: u32,
    name_ptr: *const u8,
    name_len: usize,
    data: *const f32,
) {
    let name = unsafe { loft_ffi::text(name_ptr, name_len) };
    let c_name = CString::new(name).unwrap_or_default();
    unsafe {
        let loc = gl::GetUniformLocation(program, c_name.as_ptr());
        if loc >= 0 {
            gl::UniformMatrix4fv(loc, 1, gl::FALSE, data);
        }
    }
}

// ── Registration ────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_register_v1(
    register: unsafe extern "C" fn(*const u8, usize, *const (), *mut ()),
    ctx: *mut (),
) {
    macro_rules! reg {
        ($name:expr, $fn:ident) => {
            register($name.as_ptr(), $name.len(), $fn as *const (), ctx)
        };
    }
    unsafe {
        reg!(b"loft_gl_create_window", loft_gl_create_window);
        reg!(b"loft_gl_poll_events", loft_gl_poll_events);
        reg!(b"loft_gl_swap_buffers", loft_gl_swap_buffers);
        reg!(b"loft_gl_clear", loft_gl_clear);
        reg!(b"loft_gl_destroy_window", loft_gl_destroy_window);
        reg!(b"loft_gl_create_shader", loft_gl_create_shader);
        reg!(b"loft_gl_use_shader", loft_gl_use_shader);
        reg!(b"loft_gl_upload_mesh", loft_gl_upload_mesh);
        reg!(b"loft_gl_draw", loft_gl_draw);
        reg!(b"loft_gl_set_uniform_mat4", loft_gl_set_uniform_mat4);
        reg!(b"loft_gl_upload_texture", loft_gl_upload_texture);
        reg!(b"loft_gl_bind_texture", loft_gl_bind_texture);
        reg!(b"loft_gl_delete_texture", loft_gl_delete_texture);
        reg!(b"loft_gl_load_font", loft_gl_load_font);
        reg!(b"loft_gl_measure_text", loft_gl_measure_text);
        reg!(b"loft_gl_rasterize_text", loft_gl_rasterize_text);
        reg!(b"loft_gl_free_bitmap", loft_gl_free_bitmap);
    }
}

// ── Text / Font C-ABI exports (GL3) ─────────────────────────────────────

/// Load a font from a file path. Returns font index (>= 0) or -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_load_font(path_ptr: *const u8, path_len: usize) -> i32 {
    let path = unsafe { loft_ffi::text(path_ptr, path_len) };
    match std::fs::read(path) {
        Ok(data) => text::load_font_bytes(&data),
        Err(_) => -1,
    }
}

/// Measure text width in pixels at the given font size.
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_measure_text(
    font_idx: i32,
    text_ptr: *const u8,
    text_len: usize,
    size: f32,
) -> f32 {
    let s = unsafe { loft_ffi::text(text_ptr, text_len) };
    text::measure_text(font_idx, s, size)
}

/// Rasterize text into an alpha bitmap.
/// Caller must free the returned buffer with `loft_gl_free_bitmap`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_gl_rasterize_text(
    font_idx: i32,
    text_ptr: *const u8,
    text_len: usize,
    size: f32,
    out_width: *mut u32,
    out_height: *mut u32,
    out_pixels: *mut *mut u8,
    out_pixels_len: *mut usize,
) {
    let s = unsafe { loft_ffi::text(text_ptr, text_len) };
    let (w, h, mut pixels) = text::rasterize_text(font_idx, s, size);
    unsafe {
        *out_width = w;
        *out_height = h;
        *out_pixels_len = pixels.len();
        *out_pixels = pixels.as_mut_ptr();
    }
    std::mem::forget(pixels);
}

/// Free a bitmap buffer allocated by `loft_gl_rasterize_text`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_gl_free_bitmap(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, len, len) });
    }
}

// ── Texture C-ABI exports (GL5.5) ──────────────────────────────────────

/// Upload an RGBA texture from a pixel buffer. Returns the texture ID.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loft_gl_upload_texture(
    data_ptr: *const u8,
    width: u32,
    height: u32,
) -> u32 {
    let mut tex = 0u32;
    unsafe {
        gl::GenTextures(1, &mut tex);
        gl::BindTexture(gl::TEXTURE_2D, tex);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        gl::TexImage2D(
            gl::TEXTURE_2D, 0, gl::RGBA as i32,
            width as i32, height as i32, 0,
            gl::RGBA, gl::UNSIGNED_BYTE, data_ptr.cast(),
        );
        gl::BindTexture(gl::TEXTURE_2D, 0);
    }
    tex
}

/// Bind a texture to a texture unit for rendering.
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_bind_texture(texture_id: u32, unit: u32) {
    unsafe {
        gl::ActiveTexture(gl::TEXTURE0 + unit);
        gl::BindTexture(gl::TEXTURE_2D, texture_id);
    }
}

/// Delete a texture.
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_delete_texture(texture_id: u32) {
    unsafe {
        gl::DeleteTextures(1, &texture_id);
    }
}
