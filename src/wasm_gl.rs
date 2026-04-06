// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! GL6.1–GL6.3: WebGL2 bridge for WASM builds.
//!
//! Each `wgl_*` function matches a `#native "loft_gl_*"` declaration in
//! `lib/graphics/src/graphics.loft`.  Arguments are read from the interpreter
//! stack and forwarded to JavaScript via `host_call("gl_*", &args)`.
//!
//! The JavaScript side (`gallery.html`) provides the actual WebGL2
//! implementation on `globalThis.loftHost`.

use crate::database::Stores;
use crate::keys::{DbRef, Str};

/// Register all WebGL bridge functions, replacing the panic stubs created by
/// `compile::register_native_stubs`.  Call this after `byte_code()`.
pub fn register_wgl_natives(state: &mut crate::state::State) {
    // Window lifecycle
    state.replace_native("loft_gl_create_window", wgl_create_window);
    state.replace_native("loft_gl_poll_events", wgl_poll_events);
    state.replace_native("loft_gl_swap_buffers", wgl_swap_buffers);
    state.replace_native("loft_gl_clear", wgl_clear);
    state.replace_native("loft_gl_destroy_window", wgl_destroy_window);
    // Shaders
    state.replace_native("loft_gl_create_shader", wgl_create_shader);
    state.replace_native("loft_gl_use_shader", wgl_use_shader);
    // Vertex upload + drawing
    state.replace_native("loft_gl_upload_vertices", wgl_upload_vertices);
    state.replace_native("loft_gl_draw", wgl_draw);
    state.replace_native("loft_gl_draw_mode", wgl_draw_mode);
    state.replace_native("loft_gl_draw_elements", wgl_draw_elements);
    state.replace_native("loft_gl_draw_fullscreen_quad", wgl_draw_fullscreen_quad);
    // Uniforms
    state.replace_native("loft_gl_set_mat4", wgl_set_uniform_mat4);
    state.replace_native("loft_gl_set_uniform_float", wgl_set_uniform_float);
    state.replace_native("loft_gl_set_uniform_int", wgl_set_uniform_int);
    state.replace_native("loft_gl_set_uniform_vec3", wgl_set_uniform_vec3);
    // GL state
    state.replace_native("loft_gl_enable", wgl_enable);
    state.replace_native("loft_gl_disable", wgl_disable);
    state.replace_native("loft_gl_blend_func", wgl_blend_func);
    state.replace_native("loft_gl_cull_face", wgl_cull_face);
    state.replace_native("loft_gl_depth_mask", wgl_depth_mask);
    state.replace_native("loft_gl_viewport", wgl_viewport);
    state.replace_native("loft_gl_line_width", wgl_line_width);
    state.replace_native("loft_gl_point_size", wgl_point_size);
    // Framebuffers
    state.replace_native("loft_gl_create_framebuffer", wgl_create_framebuffer);
    state.replace_native("loft_gl_bind_framebuffer", wgl_bind_framebuffer);
    state.replace_native("loft_gl_framebuffer_texture", wgl_framebuffer_texture);
    state.replace_native("loft_gl_create_depth_texture", wgl_create_depth_texture);
    state.replace_native("loft_gl_create_color_texture", wgl_create_color_texture);
    // Textures
    state.replace_native("loft_gl_load_texture", wgl_load_texture);
    state.replace_native("loft_gl_upload_canvas", wgl_upload_canvas);
    state.replace_native("loft_gl_bind_texture", wgl_bind_texture);
    state.replace_native("loft_gl_delete_texture", wgl_delete_texture);
    // Cleanup
    state.replace_native("loft_gl_delete_shader", wgl_delete_shader);
    state.replace_native("loft_gl_delete_vao", wgl_delete_vao);
    state.replace_native("loft_gl_delete_framebuffer", wgl_delete_framebuffer);
    // Input
    state.replace_native("loft_gl_key_pressed", wgl_key_pressed);
    state.replace_native("loft_gl_mouse_x", wgl_mouse_x);
    state.replace_native("loft_gl_mouse_y", wgl_mouse_y);
    state.replace_native("loft_gl_mouse_button", wgl_mouse_button);
    // Text/font and PNG save: leave as panic stubs from register_native_stubs.
    // These are not used in WebGL demos.  If called, the panic message
    // clearly states the function is not available.
}

// ── Helpers ──────────────────────────────────────────────────────────────────

#[cfg(feature = "wasm")]
fn gl_call(method: &str, args: &js_sys::Array) -> wasm_bindgen::JsValue {
    crate::wasm::host_call_raw(method, args)
}

#[cfg(not(feature = "wasm"))]
#[allow(dead_code, clippy::trivially_copy_pass_by_ref)]
fn gl_call(_method: &str, _args: &()) -> i32 {
    0
}

/// Extract a `vector<single>` (f32 elements) from a DbRef into a JS Float32Array.
#[cfg(feature = "wasm")]
fn extract_f32_vector(stores: &Stores, vref: &DbRef) -> js_sys::Float32Array {
    let allocs = &stores.allocations;
    let store = &allocs[vref.store_nr as usize];
    let v_rec = store.get_int(vref.rec, vref.pos) as u32;
    if v_rec == 0 {
        return js_sys::Float32Array::new_with_length(0);
    }
    let len = store.get_int(v_rec, 4) as u32;
    // Each f32 is 4 bytes, stored starting at offset 8 in the vector record.
    // Element size for single is 1 word (4 bytes).
    let data_start = 8u32; // vector data starts after (rec_ptr=4, len=4)
    let arr = js_sys::Float32Array::new_with_length(len);
    for i in 0..len {
        let val = store.get_single(v_rec + data_start / 4 + i, 0);
        arr.set_index(i, val);
    }
    arr
}

/// Extract a `vector<float>` (f64 elements) from a DbRef into a JS Float32Array
/// (converting f64→f32 for WebGL uniforms).
#[cfg(feature = "wasm")]
fn extract_f64_as_f32_vector(stores: &Stores, vref: &DbRef) -> js_sys::Float32Array {
    let allocs = &stores.allocations;
    let store = &allocs[vref.store_nr as usize];
    let v_rec = store.get_int(vref.rec, vref.pos) as u32;
    if v_rec == 0 {
        return js_sys::Float32Array::new_with_length(0);
    }
    let len = store.get_int(v_rec, 4) as u32;
    // Each f64 is 8 bytes = 2 words.
    let data_start = 8u32;
    let arr = js_sys::Float32Array::new_with_length(len);
    for i in 0..len {
        let word_offset = data_start / 4 + i * 2;
        let val = store.get_float(v_rec + word_offset, 0);
        arr.set_index(i, val as f32);
    }
    arr
}

// ── Window lifecycle ─────────────────────────────────────────────────────────

fn wgl_create_window(stores: &mut Stores, stack: &mut DbRef) {
    let _title = *stores.get::<Str>(stack);
    let _height = *stores.get::<i32>(stack);
    let _width = *stores.get::<i32>(stack);
    // WebGL context is created by JavaScript before WASM runs.
    // Just return true to indicate success.
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&_width.into(), &_height.into(), &_title.str().into());
        let result = gl_call("gl_create_window", &args);
        stores.put(stack, result.as_bool().unwrap_or(true));
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, true);
}

fn wgl_poll_events(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_poll_events", &args);
        stores.put(stack, result.as_bool().unwrap_or(true));
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, true);
}

fn wgl_swap_buffers(_stores: &mut Stores, _stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        gl_call("gl_swap_buffers", &args);
    }
}

fn wgl_clear(_stores: &mut Stores, stack: &mut DbRef) {
    let color = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&color.into());
        gl_call("gl_clear", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = color;
}

fn wgl_destroy_window(_stores: &mut Stores, _stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        gl_call("gl_destroy_window", &args);
    }
}

// ── Shaders ──────────────────────────────────────────────────────────────────

fn wgl_create_shader(stores: &mut Stores, stack: &mut DbRef) {
    let frag = *stores.get::<Str>(stack);
    let vert = *stores.get::<Str>(stack);
    #[cfg(feature = "wasm")]
    {
        // GL6.5: patch shader version for WebGL2
        let vert_patched = patch_shader(vert.str());
        let frag_patched = patch_shader(frag.str());
        let args = js_sys::Array::of2(&vert_patched.into(), &frag_patched.into());
        let result = gl_call("gl_create_shader", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i32);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (vert, frag);
        stores.put(stack, -1i32);
    }
}

/// GL6.5: Convert GLSL 330 core → GLSL 300 es for WebGL2.
#[cfg(feature = "wasm")]
fn patch_shader(src: &str) -> String {
    let mut result = src.to_string();
    // Replace version line
    if result.contains("#version 330 core") {
        result = result.replace("#version 330 core", "#version 300 es");
        // Add precision qualifier after version line
        if let Some(pos) = result.find("#version 300 es") {
            let end = pos + "#version 300 es".len();
            let newline_end = result[end..].find('\n').map_or(end, |p| end + p + 1);
            result.insert_str(newline_end, "precision highp float;\n");
        }
    } else if result.contains("#version 330") {
        result = result.replace("#version 330", "#version 300 es");
        if let Some(pos) = result.find("#version 300 es") {
            let end = pos + "#version 300 es".len();
            let newline_end = result[end..].find('\n').map_or(end, |p| end + p + 1);
            result.insert_str(newline_end, "precision highp float;\n");
        }
    }
    result
}

fn wgl_use_shader(_stores: &mut Stores, stack: &mut DbRef) {
    let program = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&program.into());
        gl_call("gl_use_shader", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = program;
}

// ── Vertex upload + drawing ──────────────────────────────────────────────────

fn wgl_upload_vertices(stores: &mut Stores, stack: &mut DbRef) {
    let stride = *stores.get::<i32>(stack);
    let data_ref = *stores.get::<DbRef>(stack);
    #[cfg(feature = "wasm")]
    {
        let data = extract_f32_vector(stores, &data_ref);
        let args = js_sys::Array::of2(&data.into(), &stride.into());
        let result = gl_call("gl_upload_vertices", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i32);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (stride, data_ref);
        stores.put(stack, -1i32);
    }
}

fn wgl_draw(_stores: &mut Stores, stack: &mut DbRef) {
    let vertex_count = *_stores.get::<i32>(stack);
    let vao = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&vao.into(), &vertex_count.into());
        gl_call("gl_draw", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (vao, vertex_count);
}

fn wgl_draw_mode(_stores: &mut Stores, stack: &mut DbRef) {
    let mode = *_stores.get::<i32>(stack);
    let vertex_count = *_stores.get::<i32>(stack);
    let vao = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&vao.into(), &vertex_count.into(), &mode.into());
        gl_call("gl_draw_mode", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (vao, vertex_count, mode);
}

fn wgl_draw_elements(_stores: &mut Stores, stack: &mut DbRef) {
    let mode = *_stores.get::<i32>(stack);
    let index_count = *_stores.get::<i32>(stack);
    let vao = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&vao.into(), &index_count.into(), &mode.into());
        gl_call("gl_draw_elements", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (vao, index_count, mode);
}

fn wgl_draw_fullscreen_quad(_stores: &mut Stores, _stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        gl_call("gl_draw_fullscreen_quad", &args);
    }
}

// ── Uniforms ─────────────────────────────────────────────────────────────────

fn wgl_set_uniform_mat4(stores: &mut Stores, stack: &mut DbRef) {
    let mat_ref = *stores.get::<DbRef>(stack);
    let name = *stores.get::<Str>(stack);
    let program = *stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let mat = extract_f64_as_f32_vector(stores, &mat_ref);
        let args = js_sys::Array::of3(&program.into(), &name.str().into(), &mat.into());
        gl_call("gl_set_uniform_mat4", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (program, name, mat_ref);
}

fn wgl_set_uniform_float(stores: &mut Stores, stack: &mut DbRef) {
    let val = *stores.get::<f64>(stack);
    let name = *stores.get::<Str>(stack);
    let program = *stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&program.into(), &name.str().into(), &val.into());
        gl_call("gl_set_uniform_float", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (program, name, val);
}

fn wgl_set_uniform_int(stores: &mut Stores, stack: &mut DbRef) {
    let val = *stores.get::<i32>(stack);
    let name = *stores.get::<Str>(stack);
    let program = *stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&program.into(), &name.str().into(), &val.into());
        gl_call("gl_set_uniform_int", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (program, name, val);
}

fn wgl_set_uniform_vec3(stores: &mut Stores, stack: &mut DbRef) {
    let z = *stores.get::<f64>(stack);
    let y = *stores.get::<f64>(stack);
    let x = *stores.get::<f64>(stack);
    let name = *stores.get::<Str>(stack);
    let program = *stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        args.push(&program.into());
        args.push(&name.str().into());
        args.push(&x.into());
        args.push(&y.into());
        args.push(&z.into());
        gl_call("gl_set_uniform_vec3", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (program, name, x, y, z);
}

// ── GL state ─────────────────────────────────────────────────────────────────

fn wgl_enable(_stores: &mut Stores, stack: &mut DbRef) {
    let cap = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&cap.into());
        gl_call("gl_enable", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = cap;
}

fn wgl_disable(_stores: &mut Stores, stack: &mut DbRef) {
    let cap = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&cap.into());
        gl_call("gl_disable", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = cap;
}

fn wgl_blend_func(_stores: &mut Stores, stack: &mut DbRef) {
    let dst = *_stores.get::<i32>(stack);
    let src = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&src.into(), &dst.into());
        gl_call("gl_blend_func", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (src, dst);
}

fn wgl_cull_face(_stores: &mut Stores, stack: &mut DbRef) {
    let face = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&face.into());
        gl_call("gl_cull_face", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = face;
}

fn wgl_depth_mask(_stores: &mut Stores, stack: &mut DbRef) {
    let write = *_stores.get::<bool>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&write.into());
        gl_call("gl_depth_mask", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = write;
}

fn wgl_viewport(_stores: &mut Stores, stack: &mut DbRef) {
    let h = *_stores.get::<i32>(stack);
    let w = *_stores.get::<i32>(stack);
    let y = *_stores.get::<i32>(stack);
    let x = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        args.push(&x.into());
        args.push(&y.into());
        args.push(&w.into());
        args.push(&h.into());
        gl_call("gl_viewport", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (x, y, w, h);
}

fn wgl_line_width(_stores: &mut Stores, stack: &mut DbRef) {
    let width = *_stores.get::<f64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&width.into());
        gl_call("gl_line_width", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = width;
}

fn wgl_point_size(_stores: &mut Stores, stack: &mut DbRef) {
    let size = *_stores.get::<f64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&size.into());
        gl_call("gl_point_size", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = size;
}

// ── Framebuffers ─────────────────────────────────────────────────────────────

fn wgl_create_framebuffer(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_create_framebuffer", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i32);
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, -1i32);
}

fn wgl_bind_framebuffer(_stores: &mut Stores, stack: &mut DbRef) {
    let fbo = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&fbo.into());
        gl_call("gl_bind_framebuffer", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = fbo;
}

fn wgl_framebuffer_texture(_stores: &mut Stores, stack: &mut DbRef) {
    let tex = *_stores.get::<i32>(stack);
    let attachment = *_stores.get::<i32>(stack);
    let fbo = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&fbo.into(), &attachment.into(), &tex.into());
        gl_call("gl_framebuffer_texture", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (fbo, attachment, tex);
}

fn wgl_create_depth_texture(stores: &mut Stores, stack: &mut DbRef) {
    let height = *stores.get::<i32>(stack);
    let width = *stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&width.into(), &height.into());
        let result = gl_call("gl_create_depth_texture", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i32);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (width, height);
        stores.put(stack, -1i32);
    }
}

fn wgl_create_color_texture(stores: &mut Stores, stack: &mut DbRef) {
    let height = *stores.get::<i32>(stack);
    let width = *stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&width.into(), &height.into());
        let result = gl_call("gl_create_color_texture", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i32);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (width, height);
        stores.put(stack, -1i32);
    }
}

// ── Textures ─────────────────────────────────────────────────────────────────

fn wgl_load_texture(stores: &mut Stores, stack: &mut DbRef) {
    let _path = *stores.get::<Str>(stack);
    // File-based texture loading not supported in WASM yet
    stores.put(stack, -1i32);
}

fn wgl_upload_canvas(stores: &mut Stores, stack: &mut DbRef) {
    let height = *stores.get::<i32>(stack);
    let width = *stores.get::<i32>(stack);
    let data_ref = *stores.get::<DbRef>(stack);
    #[cfg(feature = "wasm")]
    {
        // Extract vector<integer> as Uint32Array and pass to JS
        let allocs = &stores.allocations;
        let store = &allocs[data_ref.store_nr as usize];
        let v_rec = store.get_int(data_ref.rec, data_ref.pos) as u32;
        let len = if v_rec == 0 {
            0
        } else {
            store.get_int(v_rec, 4) as u32
        };
        let arr = js_sys::Uint32Array::new_with_length(len);
        for i in 0..len {
            let val = store.get_int(v_rec + 2 + i, 0) as u32;
            arr.set_index(i, val);
        }
        let args = js_sys::Array::of3(&arr.into(), &width.into(), &height.into());
        let result = gl_call("gl_upload_canvas", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i32);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (data_ref, width, height);
        stores.put(stack, -1i32);
    }
}

fn wgl_bind_texture(_stores: &mut Stores, stack: &mut DbRef) {
    let unit = *_stores.get::<i32>(stack);
    let tex_id = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&tex_id.into(), &unit.into());
        gl_call("gl_bind_texture", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (tex_id, unit);
}

fn wgl_delete_texture(_stores: &mut Stores, stack: &mut DbRef) {
    let tex_id = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&tex_id.into());
        gl_call("gl_delete_texture", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = tex_id;
}

// ── Cleanup ──────────────────────────────────────────────────────────────────

fn wgl_delete_shader(_stores: &mut Stores, stack: &mut DbRef) {
    let program = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&program.into());
        gl_call("gl_delete_shader", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = program;
}

fn wgl_delete_vao(_stores: &mut Stores, stack: &mut DbRef) {
    let vao = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&vao.into());
        gl_call("gl_delete_vao", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = vao;
}

fn wgl_delete_framebuffer(_stores: &mut Stores, stack: &mut DbRef) {
    let fbo = *_stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&fbo.into());
        gl_call("gl_delete_framebuffer", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = fbo;
}

// ── Input ────────────────────────────────────────────────────────────────────

fn wgl_key_pressed(stores: &mut Stores, stack: &mut DbRef) {
    let key_code = *stores.get::<i32>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&key_code.into());
        let result = gl_call("gl_key_pressed", &args);
        stores.put(stack, result.as_bool().unwrap_or(false));
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = key_code;
        stores.put(stack, false);
    }
}

fn wgl_mouse_x(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_mouse_x", &args);
        stores.put(stack, result.as_f64().unwrap_or(0.0));
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, 0.0f64);
}

fn wgl_mouse_y(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_mouse_y", &args);
        stores.put(stack, result.as_f64().unwrap_or(0.0));
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, 0.0f64);
}

fn wgl_mouse_button(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_mouse_button", &args);
        stores.put(stack, result.as_f64().unwrap_or(0.0) as i32);
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, 0i32);
}
