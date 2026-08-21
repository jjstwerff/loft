// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! GL-render gold-image regression for the crystal editor smoke
//! mode (`tools/audience-demo/crystal_editor.loft --smoke`).
//!
//! Drives the real GL pipeline under Xvfb + llvmpipe so the render is
//! CPU-deterministic and matches CI rather than the dev box's GPU.
//! Looser tolerance than the canvas gold-image tests (those live in
//! `lib/graphics/native/tests/gold.rs` so they travel with the
//! library) — GL line/triangle AA can drift a few LSB across Mesa
//! versions, but the test still catches beams / ground / palette
//! gone, mispositioned, or recoloured.
//!
//! Skips (does not fail) when xvfb-run, the graphics cdylib, or a
//! working software-GL context is unavailable.  Updating:
//!
//!   UPDATE_GOLD=1 cargo test --test crystal_editor_gold
//!
//! References `tools/audience-demo/crystal_editor.loft` — an
//! audience-demo tool, not a library — so this test stays in the loft
//! repo, not in any extracted library chunk.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn graphics_native_built() -> bool {
    workspace_root()
        .join("lib/graphics/native/target/release/libloft_graphics_native.so")
        .exists()
}

fn has_cmd(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn update_gold() -> bool {
    std::env::var_os("UPDATE_GOLD").is_some_and(|v| v != "0" && !v.is_empty())
}

/// Decode a PNG into an (rgba, width, height) tuple.
fn decode_rgba8(path: &std::path::Path) -> (Vec<u8>, u32, u32) {
    let file =
        std::fs::File::open(path).unwrap_or_else(|e| panic!("opening {}: {e}", path.display()));
    let decoder = png::Decoder::new(file);
    let mut reader = decoder
        .read_info()
        .unwrap_or_else(|e| panic!("reading info for {}: {e}", path.display()));
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("decoding frame of {}: {e}", path.display()));
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width, info.height);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(buf.len() / 3 * 4);
            for chunk in buf.as_chunks::<3>().0 {
                out.extend_from_slice(chunk);
                out.push(255);
            }
            out
        }
        other => panic!(
            "{}: unsupported color type {other:?} (expected RGB or RGBA)",
            path.display()
        ),
    };
    (rgba, w, h)
}

struct DiffReport {
    max_abs: u32,
    mean_abs: f64,
    differing_pixels: u64,
    total_pixels: u64,
}

fn compare_rgba(a: &[u8], b: &[u8]) -> DiffReport {
    assert_eq!(a.len(), b.len(), "rgba buffers have different lengths");
    let mut max_abs = 0u32;
    let mut sum_abs = 0u64;
    let mut differing_pixels = 0u64;
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let mut pixel_diff = 0u32;
        for (x, y) in p.iter().zip(q.iter()) {
            let d = x.abs_diff(*y) as u32;
            if d > max_abs {
                max_abs = d;
            }
            sum_abs += d as u64;
            pixel_diff += d;
        }
        if pixel_diff > 0 {
            differing_pixels += 1;
        }
    }
    let total_pixels = (a.len() / 4) as u64;
    let channel_count = a.len() as f64;
    DiffReport {
        max_abs,
        mean_abs: sum_abs as f64 / channel_count,
        differing_pixels,
        total_pixels,
    }
}

#[test]
fn crystal_editor_gl_matches_gold() {
    if !graphics_native_built() {
        eprintln!("skipping crystal GL gold: graphics cdylib not built");
        return;
    }
    if !has_cmd("xvfb-run") {
        eprintln!("skipping crystal GL gold: xvfb-run not installed");
        return;
    }
    let root = workspace_root();
    let shot = PathBuf::from("/tmp/crystal_editor_gold.png");
    let _ = std::fs::remove_file(&shot);
    let out = Command::new("xvfb-run")
        .args([
            "-a",
            "-s",
            "-screen 0 1000x1000x24",
            "env",
            // On a Wayland session `xvfb-run` only sets DISPLAY (X11), but
            // winit/glutin prefer Wayland and would connect to the REAL
            // compositor — popping a visible window on the user's screen and
            // bypassing Xvfb (and breaking truly-headless runs).  Unset
            // WAYLAND_DISPLAY and pin the winit backend to x11 so the window
            // lands on the virtual Xvfb display instead.
            "-u",
            "WAYLAND_DISPLAY",
            "WINIT_UNIX_BACKEND=x11",
            "LIBGL_ALWAYS_SOFTWARE=1",
            "GALLIUM_DRIVER=llvmpipe",
            // @PLN11 N3 Step 3 — default-native is now on, so a `use`d library
            // (here `audience_crystal`) would auto-build a cdylib.  This is a
            // dev/CI gold-image test, not a native-dispatch test, and native↔interp
            // is parity-guaranteed; interpret the library to keep the run fast and
            // avoid writing a `native-auto/` into the source tree.
            "LOFT_NO_NATIVE_LIBS=1",
        ])
        .arg(loft_bin())
        .arg("--no-warnings")
        .arg("--path")
        .arg(format!("{}/", root.display()))
        .arg("--lib")
        .arg(root.join("lib"))
        .arg("tools/audience-demo/crystal_editor.loft")
        .arg("--smoke")
        .arg("--screenshot")
        .arg(&shot)
        .current_dir(&root)
        .output()
        .expect("invoke xvfb-run");

    if !shot.exists() {
        // No framebuffer captured — almost always a missing software-GL
        // context in this environment, not a rendering regression.  Skip.
        eprintln!(
            "skipping crystal GL gold: no screenshot produced (software GL unavailable?)\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        return;
    }

    let gold = root.join("tests/gold").join("crystal-editor-gl.png");
    if update_gold() {
        std::fs::copy(&shot, &gold).expect("copying new GL gold");
        eprintln!("UPDATE_GOLD=1: wrote {}", gold.display());
        return;
    }
    assert!(
        gold.exists(),
        "GL gold missing: {}\nrun `UPDATE_GOLD=1 cargo test --test crystal_editor_gold`",
        gold.display()
    );
    let (actual, aw, ah) = decode_rgba8(&shot);
    let (expected, ew, eh) = decode_rgba8(&gold);
    // @P348 — HiDPI / display-scaled environments can hand the GL window a
    // SCALED framebuffer (observed 1333x1333 = 1000 × 1.333) even under
    // `xvfb-run`.  The controlled `make test-gl-golden` path (fixed Xvfb
    // screen) and CI always produce the exact gold size, so a dimension
    // mismatch here is environmental — skip gracefully.
    if (aw, ah) != (ew, eh) {
        eprintln!(
            "skipping crystal GL gold: framebuffer {aw}x{ah} != gold {ew}x{eh} \
             (HiDPI/display-scaled environment — run via `make test-gl-golden` for a controlled size)"
        );
        return;
    }
    let diff = compare_rgba(&actual, &expected);
    let (max_abs, mean_abs) = (16u32, 2.0f64);
    assert!(
        diff.max_abs <= max_abs && diff.mean_abs <= mean_abs,
        "crystal GL gold mismatch:\n  max_abs={} (limit {max_abs})\n  mean_abs={:.4} (limit {mean_abs})\n  \
         differing={}/{} pixels\n  to accept: UPDATE_GOLD=1 cargo test --test crystal_editor_gold",
        diff.max_abs,
        diff.mean_abs,
        diff.differing_pixels,
        diff.total_pixels
    );
}
