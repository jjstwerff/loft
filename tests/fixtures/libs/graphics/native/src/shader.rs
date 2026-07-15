// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Shader compilation and linking.

use std::ffi::CString;

/// @PLN106 B3 — down-convert desktop GLSL (`#version 330 core`) to GLSL ES 3.00 for
/// Android's GLES-3.0 context, the same transform the loft website applies for WebGL2
/// (`src/wasm_gl.rs::patch_shader`): swap the version, add the ES-required precision
/// qualifiers, and default `gl_PointSize` if a vertex shader sets only `gl_Position`.
/// Keeping it identical to the website's is what lets a website GL program run on
/// Android unchanged.
#[cfg(target_os = "android")]
fn patch_shader_gles(src: &str) -> String {
    let mut result = src.to_string();
    for from in ["#version 330 core", "#version 330"] {
        if result.contains(from) {
            result = result.replace(from, "#version 300 es");
            if let Some(pos) = result.find("#version 300 es") {
                let end = pos + "#version 300 es".len();
                let nl = result[end..].find('\n').map_or(end, |p| end + p + 1);
                result.insert_str(nl, "precision highp float;\nprecision highp int;\n");
            }
            break;
        }
    }
    if result.contains("gl_Position") && !result.contains("gl_PointSize") {
        result = result.replace("gl_Position =", "gl_PointSize = 4.0; gl_Position =");
    }
    result
}

fn compile_shader(src: &str, shader_type: u32) -> Result<u32, String> {
    #[cfg(target_os = "android")]
    let patched = patch_shader_gles(src);
    #[cfg(target_os = "android")]
    let src = patched.as_str();
    let shader = unsafe { gl::CreateShader(shader_type) };
    let c_src = CString::new(src).map_err(|e| format!("CString: {e}"))?;
    unsafe {
        gl::ShaderSource(shader, 1, &c_src.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);
    }
    let mut success = 0i32;
    unsafe { gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success) };
    if success == 0 {
        let mut len = 0i32;
        unsafe { gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut len) };
        let mut buf = vec![0u8; len as usize];
        unsafe { gl::GetShaderInfoLog(shader, len, std::ptr::null_mut(), buf.as_mut_ptr().cast()) };
        let msg = String::from_utf8_lossy(&buf).to_string();
        unsafe { gl::DeleteShader(shader) };
        Err(format!("Shader compile error: {msg}"))
    } else {
        Ok(shader)
    }
}

pub fn compile_program(vert_src: &str, frag_src: &str) -> Result<u32, String> {
    let vert = compile_shader(vert_src, gl::VERTEX_SHADER)?;
    let frag = compile_shader(frag_src, gl::FRAGMENT_SHADER)?;
    let program = unsafe { gl::CreateProgram() };
    unsafe {
        gl::AttachShader(program, vert);
        gl::AttachShader(program, frag);
        gl::LinkProgram(program);
    }
    let mut success = 0i32;
    unsafe { gl::GetProgramiv(program, gl::LINK_STATUS, &mut success) };
    unsafe {
        gl::DeleteShader(vert);
        gl::DeleteShader(frag);
    }
    if success == 0 {
        let mut len = 0i32;
        unsafe { gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut len) };
        let mut buf = vec![0u8; len as usize];
        unsafe {
            gl::GetProgramInfoLog(program, len, std::ptr::null_mut(), buf.as_mut_ptr().cast())
        };
        let msg = String::from_utf8_lossy(&buf).to_string();
        unsafe { gl::DeleteProgram(program) };
        Err(format!("Program link error: {msg}"))
    } else {
        Ok(program)
    }
}
