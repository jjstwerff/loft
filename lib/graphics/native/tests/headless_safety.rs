// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! P130: GL functions must not crash when called without a valid GL context.
//! In headless environments `loft_gl_create_window` fails; subsequent GL
//! calls should no-op and return safe defaults instead of calling through
//! null function pointers.

use loft_graphics_native::*;

/// Call a representative cross-section of GL functions without ever
/// creating a window.  Every one must return its default (0, false, or
/// void) without panicking or aborting.
#[test]
fn p130_gl_functions_noop_without_context() {
    // Window/lifecycle — poll and swap should be safe
    assert!(!loft_gl_poll_events());
    loft_gl_swap_buffers();

    // Drawing
    loft_gl_clear(0xFF000000);
    loft_gl_draw(0, 0);
    loft_gl_draw_elements(0, 0, 0);
    loft_gl_draw_mode(0, 0, 0);
    loft_gl_draw_fullscreen_quad();

    // Shaders
    let shader = loft_gl_create_shader(
        b"#version 330\nvoid main(){}\0".as_ptr(),
        27,
        b"#version 330\nvoid main(){}\0".as_ptr(),
        27,
    );
    assert_eq!(shader, 0);
    loft_gl_use_shader(0);
    loft_gl_delete_shader(0);

    // State management
    loft_gl_enable(1);
    loft_gl_disable(2);
    loft_gl_blend_func(2, 3);
    loft_gl_cull_face(0);
    loft_gl_depth_mask(true);
    loft_gl_viewport(0, 0, 800, 600);
    loft_gl_line_width(1.0);
    loft_gl_point_size(1.0);

    // Framebuffers
    assert_eq!(loft_gl_create_framebuffer(), 0);
    loft_gl_bind_framebuffer(0);
    loft_gl_framebuffer_texture(0, 0, 0);
    loft_gl_delete_framebuffer(0);

    // Textures
    assert_eq!(loft_gl_create_depth_texture(256, 256), 0);
    assert_eq!(loft_gl_create_color_texture(256, 256), 0);
    loft_gl_bind_texture(0, 0);
    loft_gl_delete_texture(0);
    assert_eq!(loft_gl_upload_alpha_texture(std::ptr::null(), 0, 0), 0);

    // Uniform setters
    loft_gl_set_uniform_float(0, b"u\0".as_ptr(), 1, 1.0);
    loft_gl_set_uniform_int(0, b"u\0".as_ptr(), 1, 1);
    loft_gl_set_uniform_vec3(0, b"u\0".as_ptr(), 1, 0.0, 0.0, 0.0);

    // Cleanup
    loft_gl_delete_vao(0);

    // Destroy (should be safe even without create)
    loft_gl_destroy_window();
}
