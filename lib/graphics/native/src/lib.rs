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

// ── Store-aware GL functions (use LoftStore to read loft vectors) ─────────

/// Upload a vector<single> as a vertex buffer. Returns VAO handle.
/// stride = floats per vertex (3=pos, 6=pos+normal, 10=pos+normal+color).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_gl_upload_vertices(
    store: loft_ffi::LoftStore,
    data: loft_ffi::LoftRef,
    stride: i32,
) -> i32 {
    let count = unsafe { store.vector_len(&data) } as u32;
    let n_vertices = count / stride as u32;
    let data_ptr = unsafe { store.vector_data_ptr(&data) } as *const f32;
    let mut vao = 0u32;
    let mut vbo = 0u32;
    unsafe { loft_gl_upload_mesh(data_ptr, n_vertices, stride as u32, &mut vao, &mut vbo) };
    vao as i32
}

/// Set a mat4 uniform from a vector<float> (16 elements, column-major).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_gl_set_uniform_mat4(
    store: loft_ffi::LoftStore,
    program: i32,
    name_ptr: *const u8,
    name_len: usize,
    mat: loft_ffi::LoftRef,
) {
    let name = unsafe { loft_ffi::text(name_ptr, name_len) };
    // Mat4.m is stored as vector<float> — 16 f64 values in the store.
    // OpenGL needs f32, so we convert on the fly.
    let count = unsafe { store.vector_len(&mat) };
    if count < 16 {
        return;
    }
    let mut buf = [0.0f32; 16];
    for i in 0..16 {
        let val = unsafe { store.get_float(mat.rec, 8 + i as u32 * 8, 0) };
        buf[i] = val as f32;
    }
    let c_name = std::ffi::CString::new(name).unwrap_or_default();
    unsafe {
        let loc = gl::GetUniformLocation(program as u32, c_name.as_ptr());
        if loc >= 0 {
            gl::UniformMatrix4fv(loc, 1, gl::FALSE, buf.as_ptr());
        }
    }
}

// ── Uniform helpers ────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_set_uniform_float(
    program: u32, name_ptr: *const u8, name_len: usize, val: f64,
) {
    let name = unsafe { loft_ffi::text(name_ptr, name_len) };
    let c_name = std::ffi::CString::new(name).unwrap_or_default();
    unsafe {
        let loc = gl::GetUniformLocation(program, c_name.as_ptr());
        if loc >= 0 { gl::Uniform1f(loc, val as f32); }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_set_uniform_int(
    program: u32, name_ptr: *const u8, name_len: usize, val: i32,
) {
    let name = unsafe { loft_ffi::text(name_ptr, name_len) };
    let c_name = std::ffi::CString::new(name).unwrap_or_default();
    unsafe {
        let loc = gl::GetUniformLocation(program, c_name.as_ptr());
        if loc >= 0 { gl::Uniform1i(loc, val); }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_set_uniform_vec3(
    program: u32, name_ptr: *const u8, name_len: usize,
    x: f64, y: f64, z: f64,
) {
    let name = unsafe { loft_ffi::text(name_ptr, name_len) };
    let c_name = std::ffi::CString::new(name).unwrap_or_default();
    unsafe {
        let loc = gl::GetUniformLocation(program, c_name.as_ptr());
        if loc >= 0 { gl::Uniform3f(loc, x as f32, y as f32, z as f32); }
    }
}

// ── GL state management ───────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_enable(cap: i32) {
    let gl_cap = match cap {
        1 => gl::DEPTH_TEST,
        2 => gl::BLEND,
        3 => gl::CULL_FACE,
        _ => return,
    };
    unsafe { gl::Enable(gl_cap); }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_disable(cap: i32) {
    let gl_cap = match cap {
        1 => gl::DEPTH_TEST,
        2 => gl::BLEND,
        3 => gl::CULL_FACE,
        _ => return,
    };
    unsafe { gl::Disable(gl_cap); }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_blend_func(src: i32, dst: i32) {
    let map = |v: i32| -> u32 {
        match v {
            0 => gl::ZERO, 1 => gl::ONE,
            2 => gl::SRC_ALPHA, 3 => gl::ONE_MINUS_SRC_ALPHA,
            4 => gl::DST_ALPHA, 5 => gl::ONE_MINUS_DST_ALPHA,
            _ => gl::ONE,
        }
    };
    unsafe { gl::BlendFunc(map(src), map(dst)); }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_cull_face(face: i32) {
    let f = if face == 0 { gl::BACK } else { gl::FRONT };
    unsafe { gl::CullFace(f); }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_depth_mask(write: bool) {
    unsafe { gl::DepthMask(if write { gl::TRUE } else { gl::FALSE }); }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_viewport(x: i32, y: i32, w: i32, h: i32) {
    unsafe { gl::Viewport(x, y, w, h); }
}

// ── Framebuffer objects ───────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_create_framebuffer() -> i32 {
    let mut fbo = 0u32;
    unsafe { gl::GenFramebuffers(1, &mut fbo); }
    fbo as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_bind_framebuffer(fbo: i32) {
    unsafe { gl::BindFramebuffer(gl::FRAMEBUFFER, fbo as u32); }
}

/// Attach a texture as the color (attachment=0) or depth (attachment=1) target.
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_framebuffer_texture(fbo: i32, attachment: i32, tex: i32) {
    let att = if attachment == 0 { gl::COLOR_ATTACHMENT0 } else { gl::DEPTH_ATTACHMENT };
    unsafe {
        gl::BindFramebuffer(gl::FRAMEBUFFER, fbo as u32);
        gl::FramebufferTexture2D(gl::FRAMEBUFFER, att, gl::TEXTURE_2D, tex as u32, 0);
        if attachment == 1 {
            // Depth-only FBO: no color draw/read
            gl::DrawBuffer(gl::NONE);
            gl::ReadBuffer(gl::NONE);
        }
        gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
    }
}

/// Create a depth-only texture (for shadow mapping).
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_create_depth_texture(width: i32, height: i32) -> i32 {
    let mut tex = 0u32;
    unsafe {
        gl::GenTextures(1, &mut tex);
        gl::BindTexture(gl::TEXTURE_2D, tex);
        gl::TexImage2D(
            gl::TEXTURE_2D, 0, gl::DEPTH_COMPONENT as i32,
            width, height, 0,
            gl::DEPTH_COMPONENT, gl::FLOAT, std::ptr::null(),
        );
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        gl::BindTexture(gl::TEXTURE_2D, 0);
    }
    tex as i32
}

/// Create an empty RGBA texture (for render-to-texture / post-processing).
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_create_color_texture(width: i32, height: i32) -> i32 {
    let mut tex = 0u32;
    unsafe {
        gl::GenTextures(1, &mut tex);
        gl::BindTexture(gl::TEXTURE_2D, tex);
        gl::TexImage2D(
            gl::TEXTURE_2D, 0, gl::RGBA as i32,
            width, height, 0,
            gl::RGBA, gl::UNSIGNED_BYTE, std::ptr::null(),
        );
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
        gl::BindTexture(gl::TEXTURE_2D, 0);
    }
    tex as i32
}

/// Draw a fullscreen quad (for post-processing passes). Uses a built-in VAO.
#[unsafe(no_mangle)]
pub extern "C" fn loft_gl_draw_fullscreen_quad() {
    thread_local! {
        static QUAD_VAO: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    let vao = QUAD_VAO.with(|c| {
        let mut v = c.get();
        if v == 0 {
            let verts: [f32; 24] = [
                -1.0, -1.0, 0.0, 0.0,
                 1.0, -1.0, 1.0, 0.0,
                -1.0,  1.0, 0.0, 1.0,
                 1.0, -1.0, 1.0, 0.0,
                 1.0,  1.0, 1.0, 1.0,
                -1.0,  1.0, 0.0, 1.0,
            ];
            unsafe {
                let mut vbo = 0u32;
                gl::GenVertexArrays(1, &mut v);
                gl::GenBuffers(1, &mut vbo);
                gl::BindVertexArray(v);
                gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
                gl::BufferData(gl::ARRAY_BUFFER, 96, verts.as_ptr().cast(), gl::STATIC_DRAW);
                gl::VertexAttribPointer(0, 2, gl::FLOAT, gl::FALSE, 16, std::ptr::null());
                gl::EnableVertexAttribArray(0);
                gl::VertexAttribPointer(1, 2, gl::FLOAT, gl::FALSE, 16, 8 as *const _);
                gl::EnableVertexAttribArray(1);
                gl::BindVertexArray(0);
            }
            c.set(v);
        }
        v
    });
    unsafe {
        gl::BindVertexArray(vao);
        gl::DrawArrays(gl::TRIANGLES, 0, 6);
        gl::BindVertexArray(0);
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
        reg!(b"n_gl_upload_vertices", n_gl_upload_vertices);
        reg!(b"n_gl_set_uniform_mat4", n_gl_set_uniform_mat4);
        // Uniform helpers
        reg!(b"loft_gl_set_uniform_float", loft_gl_set_uniform_float);
        reg!(b"loft_gl_set_uniform_int", loft_gl_set_uniform_int);
        reg!(b"loft_gl_set_uniform_vec3", loft_gl_set_uniform_vec3);
        // GL state
        reg!(b"loft_gl_enable", loft_gl_enable);
        reg!(b"loft_gl_disable", loft_gl_disable);
        reg!(b"loft_gl_blend_func", loft_gl_blend_func);
        reg!(b"loft_gl_cull_face", loft_gl_cull_face);
        reg!(b"loft_gl_depth_mask", loft_gl_depth_mask);
        reg!(b"loft_gl_viewport", loft_gl_viewport);
        // Framebuffer objects
        reg!(b"loft_gl_create_framebuffer", loft_gl_create_framebuffer);
        reg!(b"loft_gl_bind_framebuffer", loft_gl_bind_framebuffer);
        reg!(b"loft_gl_framebuffer_texture", loft_gl_framebuffer_texture);
        reg!(b"loft_gl_create_depth_texture", loft_gl_create_depth_texture);
        reg!(b"loft_gl_create_color_texture", loft_gl_create_color_texture);
        reg!(b"loft_gl_draw_fullscreen_quad", loft_gl_draw_fullscreen_quad);
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
