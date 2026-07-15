// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Gold-image regression tests for the graphics library's software
//! rasterizer.  Each test runs a loft example in a tempdir, decodes
//! the produced PNG plus the reference under `lib/graphics/tests/gold/`,
//! and asserts they match within a small per-channel tolerance.
//!
//! Lives under `lib/graphics/native/tests/` so it travels with the
//! library when `lib/graphics/` extracts to `loft-libs-graphics`.
//! In the monorepo it's invoked via `make test-packages-native` (or
//! `cd lib/graphics/native && cargo test --release`).  In the chunk
//! repo it's invoked by the `library-ci.yml` template's "Rust
//! integration tests (if present)" step.
//!
//! Locating the loft binary:
//!   1. `LOFT_BIN` env var — chunk CI sets this to the just-built loft
//!      (or relies on PATH).
//!   2. Monorepo fallback: `<workspace>/target/release/loft` walked
//!      up from `CARGO_MANIFEST_DIR` (= lib/graphics/native).
//!   3. `loft` on PATH (chunk CI without `LOFT_BIN`).
//!   If none is runnable the test prints a skip note and passes —
//!   matches the original test's "skip if dependencies missing" shape.
//!
//! Why fuzzy compare and not byte compare?
//!   PNG encoders aren't byte-deterministic across platforms (zlib
//!   level, libpng version, deflate variant), so a byte hash would
//!   be brittle on other people's machines.  A pixel-level MAE
//!   check catches every real rendering regression without being
//!   tripped by encoder drift.
//!
//! Updating the gold:
//!   When an intentional rendering change lands, rerun the test with
//!   `UPDATE_GOLD=1`:
//!
//!     UPDATE_GOLD=1 cargo test --release --test gold
//!
//!   The test writes the newly-rendered PNG over the gold, passes,
//!   and leaves the diff visible in `git status` for the committer
//!   to review before staging.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `CARGO_MANIFEST_DIR` of this test = `lib/graphics/native/`.
/// The graphics PACKAGE root is one level up; examples + tests/gold/
/// live there.  In the monorepo, the workspace root is two more
/// levels up (lib/graphics/native → lib/graphics → lib → <root>).
fn graphics_pkg_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("native crate has a parent")
        .to_path_buf()
}

/// Monorepo workspace root if recognisable; otherwise None.
/// Used for path-relative fallbacks; chunk-repo callers should pass
/// `LOFT_BIN=...` instead of relying on this.
fn workspace_root() -> Option<PathBuf> {
    // <root>/lib/graphics/native/ → climb three.
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .parent()?
        .to_path_buf();
    if candidate.join("Cargo.toml").exists() && candidate.join("default").is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Locate a `loft` binary to drive the examples.
fn loft_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LOFT_BIN") {
        let path = PathBuf::from(&p);
        if path.exists() {
            return Some(path);
        }
        eprintln!("LOFT_BIN={p} set but file does not exist");
    }
    if let Some(root) = workspace_root() {
        let p = root.join("target/release/loft");
        if p.exists() {
            return Some(p);
        }
    }
    // Last resort: rely on PATH.  Command::new() will resolve "loft"
    // via PATH; we return a marker that the runner can use.
    Some(PathBuf::from("loft"))
}

fn graphics_native_built() -> bool {
    graphics_pkg_root()
        .join("native/target/release/libloft_graphics_native.so")
        .exists()
        || graphics_pkg_root()
            .join("native/target/release/libloft_graphics_native.dylib")
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

/// Run a loft script under `cwd` and assert it exits 0.
fn run_loft(loft: &Path, script: &Path, cwd: &Path) -> String {
    // --interpret overrides the example's `#!/usr/bin/env -S loft --native`
    // shebang.  Under --native the first invocation falls through to an
    // on-the-fly native compile; nextest's initial try has no cached
    // binary and fails with "failed to run native binary: No such file".
    // --interpret is deterministic across both tries and still exercises
    // the full IR + bytecode + rasterizer path.
    let mut cmd = Command::new(loft);
    cmd.arg("--interpret").arg(script).current_dir(cwd);
    // If running from a non-monorepo location (chunk CI), point loft
    // at the workspace's default/*.loft via --path if we know it.
    if let Some(root) = workspace_root() {
        cmd.arg("--path").arg(&root);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
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

/// Shared driver: runs `script` (relative to the graphics package root),
/// reads the generated PNG, compares against `gold` (under
/// `lib/graphics/tests/gold/`).
fn gold_compare(example: &str, gold_name: &str, max_abs: u32, mean_abs: f64) {
    gold_compare_assets(example, gold_name, &[], max_abs, mean_abs);
}

/// Like `gold_compare`, but first copies each asset (path relative to the
/// graphics package root) into the run's tempdir.
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
             {}/native/target/release/libloft_graphics_native.{{so,dylib}} not built",
            graphics_pkg_root().display()
        );
        return;
    }
    let Some(loft) = loft_bin() else {
        eprintln!("skipping graphics gold test: no loft binary available");
        return;
    };
    let root = graphics_pkg_root();
    let script = root.join(example);
    assert!(script.exists(), "example not found: {}", script.display());
    let gold = root.join("tests/gold").join(gold_name);

    let tmp = tempdir();
    for asset in assets {
        let src = root.join(asset);
        let base = Path::new(asset)
            .file_name()
            .expect("asset path has a filename");
        std::fs::copy(&src, tmp.join(base))
            .unwrap_or_else(|e| panic!("copying asset {}: {e}", src.display()));
    }
    run_loft(&loft, &script, &tmp);
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
         run `UPDATE_GOLD=1 cargo test --release --test gold` to create it",
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
         to accept: UPDATE_GOLD=1 cargo test --release --test gold",
        diff.max_abs,
        diff.mean_abs,
        diff.differing_pixels,
        diff.total_pixels,
        pct_diff,
        produced.display(),
        gold.display()
    );
}

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
        "examples/10-2d-canvas.loft",
        "10-canvas-demo.png",
        /* max_abs  */ 1,
        /* mean_abs */ 0.05,
    );
}

#[test]
fn pixel_roundtrip_matches_gold() {
    gold_compare("examples/gold-pixels.loft", "gold-pixels.png", 0, 0.0);
}

#[test]
fn fill_rect_matches_gold() {
    gold_compare("examples/gold-rect.loft", "gold-rect.png", 0, 0.0);
}

#[test]
fn draw_line_matches_gold() {
    gold_compare("examples/gold-line.loft", "gold-line.png", 0, 0.0);
}

#[test]
fn fill_triangle_matches_gold() {
    gold_compare("examples/gold-triangle.loft", "gold-triangle.png", 0, 0.0);
}

#[test]
fn blend_matches_gold() {
    gold_compare("examples/gold-blend.loft", "gold-blend.png", 0, 0.0);
}

#[test]
fn text_matches_gold() {
    gold_compare_assets(
        "examples/gold-text.loft",
        "gold-text.png",
        &["examples/DejaVuSans-Bold.ttf"],
        /* max_abs  */ 4,
        /* mean_abs */ 0.5,
    );
}
