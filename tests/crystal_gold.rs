// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Gold-image regression tests for the @PLAN36 audience-demo crystal
//! pattern.  Each test runs `tools/audience-demo/crystal_render.loft`
//! with a scenario name (single / growth / linked / bridge), reads the
//! produced PNG, and compares against the reference under
//! `tests/golden/crystal/`.
//!
//! Why golden + tolerance, not byte-equal?  PNG encoder output isn't
//! byte-deterministic across libpng / zlib versions, and the canvas
//! rasterizer's anti-aliased lines use f64 trig that may differ in
//! the last ULP on different platforms.  A per-channel-abs tolerance
//! catches every real geometry / colour regression without tripping
//! on encoder drift.
//!
//! Updating the goldens:
//!
//!   UPDATE_GOLD=1 cargo test --test crystal_gold
//!
//! Run once; the four PNGs land in `tests/golden/crystal/`; eyeball
//! the diffs in `git status`; commit if good.
//!
//! Scenarios:
//!   - single   — 1 isolated cell (the canonical 6-ray nucleation)
//!   - growth   — 2 adjacent cells (1 center + 1 supporter)
//!   - linked   — 3 cells in a line (1 center + 2 supporters)
//!   - bridge   — 2 cells with a 1-empty-cell gap (bridge-future case;
//!                today shows two centers reaching but not meeting)

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn update_gold() -> bool {
    std::env::var_os("UPDATE_GOLD").is_some_and(|v| v != "0" && !v.is_empty())
}

fn tempdir() -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("loft-crystal-{pid}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("creating tempdir");
    dir
}

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

/// Invoke `crystal_render.loft` with `<scenario> <out_path>` and assert
/// it exits 0.  Captures combined stdout+stderr for diagnostics.
fn run_render(scenario: &str, out_path: &Path) -> String {
    let root = workspace_root();
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(root.join("tools/audience-demo/crystal_render.loft"))
        .arg(scenario)
        .arg(out_path)
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "crystal_render {scenario} failed: exit={:?}\nstdout={stdout}\nstderr={stderr}",
        out.status.code()
    );
    format!("{stdout}{stderr}")
}

/// Render `scenario` to a fresh tempfile, compare against the matching
/// golden under `tests/golden/crystal/`.  Honors `UPDATE_GOLD=1` to
/// refresh the golden in-place.
fn gold_compare(scenario: &str, max_abs: u32, mean_abs: f64) {
    let root = workspace_root();
    let gold_dir = root.join("tests/golden/crystal");
    std::fs::create_dir_all(&gold_dir).expect("creating golden/crystal dir");
    let gold = gold_dir.join(format!("{scenario}.png"));

    let tmp = tempdir();
    let produced = tmp.join(format!("{scenario}.png"));
    run_render(scenario, &produced);
    assert!(
        produced.exists(),
        "crystal_render {scenario} did not write {}",
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
         run `UPDATE_GOLD=1 cargo test --test crystal_gold` to create it",
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
        "crystal gold mismatch for {scenario}:\n  \
         max_abs    = {} (limit {max_abs})\n  \
         mean_abs   = {:.4} (limit {mean_abs})\n  \
         differing  = {}/{} pixels ({:.2}%)\n  \
         produced   = {}\n  \
         gold       = {}\n  \
         to accept: UPDATE_GOLD=1 cargo test --test crystal_gold",
        diff.max_abs,
        diff.mean_abs,
        diff.differing_pixels,
        diff.total_pixels,
        pct_diff,
        produced.display(),
        gold.display()
    );
}

// Tolerances per channel: the canvas software rasterizer is fully
// deterministic so we keep tight bounds.  max_abs=1 hedges against
// stray rounding in cos/sin (we use f64 trig everywhere); mean_abs
// stays tiny because most pixels are background.

#[test]
fn crystal_single_matches_gold() {
    gold_compare("single", /* max_abs */ 1, /* mean_abs */ 0.05);
}

#[test]
fn crystal_growth_matches_gold() {
    gold_compare("growth", 1, 0.05);
}

#[test]
fn crystal_linked_matches_gold() {
    gold_compare("linked", 1, 0.05);
}

#[test]
fn crystal_linked_centre_first_matches_gold() {
    gold_compare("linked_centre_first", 1, 0.05);
}

#[test]
fn crystal_bridge_matches_gold() {
    gold_compare("bridge", 1, 0.05);
}

// ── Aging animation curve ─────────────────────────────────────────
// The `growth` snap rendered at three ticks against a CrystalState
// where every main was born at tick 0.  Sides at 0 % / 33 % / 100 %
// of full length over GROWTH_TICKS = 3600 ticks.

#[test]
fn crystal_growth_t0_matches_gold() {
    gold_compare("growth_t0", 1, 0.05);
}

#[test]
fn crystal_growth_t1200_matches_gold() {
    gold_compare("growth_t1200", 1, 0.05);
}

#[test]
fn crystal_growth_full_matches_gold() {
    gold_compare("growth_full", 1, 0.05);
}
