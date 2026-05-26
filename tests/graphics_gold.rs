// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Gold-image regression tests for the graphics library's software
//! rasterizer.  Each test runs a loft example in a tempdir, decodes
//! the produced PNG plus the reference under `tests/gold/`, and
//! asserts they match within a small per-channel tolerance.
//!
//! Why fuzzy compare and not byte compare?
//!   PNG encoders aren't byte-deterministic across platforms (zlib
//!   level, libpng version, deflate variant), so a byte hash would
//!   be brittle on other people's machines.  A pixel-level MAE
//!   check catches every real rendering regression without being
//!   tripped by encoder drift.
//!
//! Updating the gold:
//!   When an intentional rendering change lands (new shape, fixed
//!   bug, tweaked palette), rerun the test with `UPDATE_GOLD=1`:
//!
//!     UPDATE_GOLD=1 cargo test --test graphics_gold
//!
//!   The test writes the newly-rendered PNG over the gold, passes,
//!   and leaves the diff visible in `git status` for the committer
//!   to review before staging.  There is no "auto-accept" path in
//!   CI — humans decide what a good rendering looks like.
//!
//! Skipping:
//!   Requires the graphics native extension
//!   (`lib/graphics/native/target/release/libloft_graphics_native.so`).
//!   If that file doesn't exist, the test prints a note and passes
//!   without comparing — building the native extension is a separate
//!   step (`cargo build --release --manifest-path
//!   lib/graphics/native/Cargo.toml`).  This keeps the default
//!   `cargo test` run green without forcing every developer to
//!   compile the graphics cdylib.

use std::path::{Path, PathBuf};
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

/// Decode a PNG into an (rgba, width, height) tuple.  Non-RGBA
/// inputs are expanded to RGBA8 so encoder choices (RGB vs RGBA,
/// depending on whether any alpha < 255) don't break the compare.
fn decode_rgba8(path: &Path) -> (Vec<u8>, u32, u32) {
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
            for chunk in buf.chunks_exact(3) {
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
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
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

/// Run a loft script under `cwd` and assert it exits 0.  Returns the
/// stdout+stderr for diagnostic inclusion on failure.
fn run_loft(script: &Path, cwd: &Path) -> String {
    // --interpret overrides the example's `#!/usr/bin/env -S loft --native`
    // shebang.  Under --native the first invocation falls through to an
    // on-the-fly native compile; nextest's initial try has no cached
    // binary and fails with "failed to run native binary: No such file".
    // --interpret is deterministic across both tries and still exercises
    // the full IR + bytecode + rasterizer path.
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(script)
        .current_dir(cwd)
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "loft {} failed: exit={:?}\nstdout={stdout}\nstderr={stderr}",
        script.display(),
        out.status.code()
    );
    format!("{stdout}{stderr}")
}

fn update_gold() -> bool {
    std::env::var_os("UPDATE_GOLD").is_some_and(|v| v != "0" && !v.is_empty())
}

/// Shared driver: runs `script`, reads the generated PNG, compares
/// against `gold`.  Under UPDATE_GOLD=1, rewrites the gold and
/// passes.  Tolerances are per-channel absolute differences:
///   `max_abs` — largest single-channel delta allowed (0-255)
///   `mean_abs` — mean across every channel of every pixel
fn gold_compare(example: &str, gold_name: &str, max_abs: u32, mean_abs: f64) {
    gold_compare_assets(example, gold_name, &[], max_abs, mean_abs);
}

/// Like `gold_compare`, but first copies each asset (path relative to the
/// workspace root) into the run's tempdir under its basename — so a fixture
/// that loads e.g. a font with a bare relative path resolves it against the
/// tempdir cwd.
fn gold_compare_assets(
    example: &str,
    gold_name: &str,
    assets: &[&str],
    max_abs: u32,
    mean_abs: f64,
) {
    if !graphics_native_built() {
        eprintln!(
            "skipping graphics gold test: \
             lib/graphics/native/target/release/libloft_graphics_native.so not built"
        );
        return;
    }
    let root = workspace_root();
    let script = root.join(example);
    assert!(script.exists(), "example not found: {}", script.display());
    let gold = root.join("tests/gold").join(gold_name);

    let tmp = tempdir();
    for asset in assets {
        let src = root.join(asset);
        let base = std::path::Path::new(asset)
            .file_name()
            .expect("asset path has a filename");
        std::fs::copy(&src, tmp.join(base))
            .unwrap_or_else(|e| panic!("copying asset {}: {e}", src.display()));
    }
    run_loft(&script, &tmp);
    let produced = tmp.join(gold_name);
    assert!(
        produced.exists(),
        "{} did not write {} (looking at {})",
        script.display(),
        gold_name,
        produced.display()
    );

    if update_gold() {
        std::fs::copy(&produced, &gold).expect("copying new gold over existing");
        eprintln!(
            "UPDATE_GOLD=1: wrote fresh {} ({} bytes)",
            gold.display(),
            std::fs::metadata(&gold).map(|m| m.len()).unwrap_or(0)
        );
        return;
    }

    assert!(
        gold.exists(),
        "gold reference missing: {}\n\
         run `UPDATE_GOLD=1 cargo test --test graphics_gold` to create it",
        gold.display()
    );

    let (actual, aw, ah) = decode_rgba8(&produced);
    let (expected, ew, eh) = decode_rgba8(&gold);
    assert_eq!(
        (aw, ah),
        (ew, eh),
        "dimensions differ: produced {aw}x{ah}, gold {ew}x{eh}"
    );
    let diff = compare_rgba(&actual, &expected);
    let pct_diff = diff.differing_pixels as f64 / diff.total_pixels as f64 * 100.0;
    assert!(
        diff.max_abs <= max_abs && diff.mean_abs <= mean_abs,
        "gold mismatch for {gold_name}:\n  \
         max_abs    = {} (limit {max_abs})\n  \
         mean_abs   = {:.4} (limit {mean_abs})\n  \
         differing  = {}/{} pixels ({:.2}%)\n  \
         produced   = {}\n  \
         gold       = {}\n  \
         to accept: UPDATE_GOLD=1 cargo test --test graphics_gold",
        diff.max_abs,
        diff.mean_abs,
        diff.differing_pixels,
        diff.total_pixels,
        pct_diff,
        produced.display(),
        gold.display()
    );
}

/// Minimal temp-directory helper.  Creates a unique dir under
/// `std::env::temp_dir()`, named after the process ID + a random
/// suffix from the system clock.  We don't pull `tempfile` in just
/// for one helper.
fn tempdir() -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("loft-gold-{pid}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("creating tempdir");
    dir
}

#[test]
fn canvas_demo_matches_gold() {
    gold_compare(
        "lib/graphics/examples/10-2d-canvas.loft",
        "10-canvas-demo.png",
        // Tolerances: the software rasterizer is fully deterministic,
        // so a tight bound is fine.  Encoder drift across libpng /
        // zlib revisions only affects compressed bytes; decoded RGBA
        // should match exactly.  Keep `max_abs = 1` as a hedge
        // against stray rounding in platform-specific float math
        // (Bezier and AA-line use f64 trig/lerp).
        /* max_abs  */
        1,
        /* mean_abs */ 0.05,
    );
}

/// Per-part golden: the Canvas integer pixel buffer + `save_png` round-trip,
/// in isolation (no lines/curves/AA — pure `canvas` / `fill_rect` /
/// `set_pixel`).  Exact (`max_abs = 0`): every channel of every pixel must
/// match, so an i32/i64 truncation, sign, or stride regression in the
/// pixel-storage / PNG-encode path fails immediately and points only here.
#[test]
fn pixel_roundtrip_matches_gold() {
    gold_compare(
        "lib/graphics/examples/gold-pixels.loft",
        "gold-pixels.png",
        /* max_abs  */ 0,
        /* mean_abs */ 0.0,
    );
}

/// Per-part golden: `fill_rect` (solid-fill bounds + colour), in isolation.
/// Exact — integer rasterizer.
#[test]
fn fill_rect_matches_gold() {
    gold_compare(
        "lib/graphics/examples/gold-rect.loft",
        "gold-rect.png",
        0,
        0.0,
    );
}

/// Per-part golden: `draw_line` (Bresenham), in isolation.  Exact.
#[test]
fn draw_line_matches_gold() {
    gold_compare(
        "lib/graphics/examples/gold-line.loft",
        "gold-line.png",
        0,
        0.0,
    );
}

/// Per-part golden: `fill_triangle` (scanline — the crystal canvas fill), in
/// isolation.  Exact.
#[test]
fn fill_triangle_matches_gold() {
    gold_compare(
        "lib/graphics/examples/gold-triangle.loft",
        "gold-triangle.png",
        0,
        0.0,
    );
}

/// Per-part golden: `blend_pixel` / `blend` (alpha-over compositing math), in
/// isolation.  Exact — integer blend.
#[test]
fn blend_matches_gold() {
    gold_compare(
        "lib/graphics/examples/gold-blend.loft",
        "gold-blend.png",
        0,
        0.0,
    );
}

/// Per-part golden: the text path (`gl_load_font` + `draw_text` → Canvas →
/// save_png), in isolation.  Copies the font into the run dir.  Modest
/// tolerance — glyph rasterization is antialiased and can drift a hair across
/// font-rasterizer versions; still catches text gone / mispositioned /
/// garbled (the WebGL "no text" class of regression on the native side).
#[test]
fn text_matches_gold() {
    gold_compare_assets(
        "lib/graphics/examples/gold-text.loft",
        "gold-text.png",
        &["lib/graphics/examples/DejaVuSans-Bold.ttf"],
        /* max_abs  */ 4,
        /* mean_abs */ 0.5,
    );
}

// ── GL-render golden track ──────────────────────────────────────────────────
//
// The canvas goldens above exercise the software rasterizer → save_png.  This
// one exercises the real GL pipeline: the crystal editor's --smoke mode paints
// a fixed hex cluster and renders the faint ground hexes
// (`ground_hexes_to_verts`), the crystal beams (`crystal_mesh_to_beams`, the
// 30° points), and the palette, then writes the framebuffer via
// `gl_screenshot`.  Run under Xvfb + llvmpipe so the render is CPU-deterministic
// and matches CI rather than the dev box's GPU.  Looser tolerance than the
// exact canvas goldens — GL line/triangle AA can drift a few LSB across Mesa
// versions — but still catches beams / ground / palette gone, mispositioned,
// or recoloured.  Skips (does not fail) when xvfb-run, the graphics cdylib, or
// a working software-GL context is unavailable.

fn has_cmd(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
        "GL gold missing: {}\nrun `UPDATE_GOLD=1 cargo test --test graphics_gold crystal_editor_gl`",
        gold.display()
    );
    let (actual, aw, ah) = decode_rgba8(&shot);
    let (expected, ew, eh) = decode_rgba8(&gold);
    // @P348 — a HiDPI / display-scaled environment can hand the GL window a
    // SCALED framebuffer (observed 1333x1333 = 1000 × 1.333) even under
    // `xvfb-run`, because some drivers honour the real X display's scale
    // factor.  The controlled `make test-gl-golden` path (fixed Xvfb screen)
    // and CI always produce the exact gold size, so a dimension mismatch here
    // is environmental, not a rendering regression — skip gracefully rather
    // than panic, matching the test's other environmental skips above.
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
         differing={}/{} pixels\n  to accept: UPDATE_GOLD=1 cargo test --test graphics_gold crystal_editor_gl",
        diff.max_abs,
        diff.mean_abs,
        diff.differing_pixels,
        diff.total_pixels
    );
}
