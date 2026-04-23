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
