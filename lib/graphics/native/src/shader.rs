// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Shader compilation and linking.

use std::ffi::CString;

fn compile_shader(src: &str, shader_type: u32) -> Result<u32, String> {
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
        unsafe { gl::GetProgramInfoLog(program, len, std::ptr::null_mut(), buf.as_mut_ptr().cast()) };
        let msg = String::from_utf8_lossy(&buf).to_string();
        unsafe { gl::DeleteProgram(program) };
        Err(format!("Program link error: {msg}"))
    } else {
        Ok(program)
    }
}
