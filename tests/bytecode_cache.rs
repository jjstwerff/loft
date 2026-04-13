// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! QUALITY Tier 3 #7 — integration tests for the `.loftc` bytecode cache.
//!
//! The cache was shipped in commit `4039490` without any test exercising
//! the hit / miss / invalidation cycle.  These tests close that gap by
//! driving the full CLI end-to-end (via `std::process::Command`) and
//! checking the on-disk `.loftc` against known invariants.
//!
//! Cache key format (see `src/cache.rs`): magic `"LFC1"` + SHA-256 of
//! `VERSION + BUILD_ID + sources`.  When the source changes the SHA-256
//! changes, so a fresh `.loftc` must be written.  When the source is
//! unchanged the cache must be reused (file content stays byte-identical).

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Carve out a per-test temp directory so parallel integration tests
/// don't race on the same `.loft` / `.loftc` pair.
fn fresh_tmp_dir(tag: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("loft_cache_test_{tag}_{}", std::process::id(),));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_loft(source: &std::path::Path) -> std::process::Output {
    Command::new(loft_bin())
        .arg("--interpret")
        .arg(source)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary")
}

/// First run on a freshly-written `.loft` produces a `.loftc` next to
/// it.  The file starts with the `"LFC1"` magic bytes so a reader can
/// tell the format at a glance.
#[test]
fn first_run_writes_loftc_with_magic_header() {
    let dir = fresh_tmp_dir("magic");
    let src = dir.join("prog.loft");
    std::fs::write(&src, "fn main() { println(\"cache-magic\"); }\n").unwrap();

    let out = run_loft(&src);
    assert!(
        out.status.success(),
        "first run should succeed; stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("cache-magic"),
        "program output missing; stdout={:?}",
        String::from_utf8_lossy(&out.stdout)
    );

    let cached = dir.join("prog.loftc");
    assert!(
        cached.exists(),
        ".loftc must be written next to the source on first run"
    );
    let bytes = std::fs::read(&cached).unwrap();
    assert!(
        bytes.starts_with(b"LFC1"),
        ".loftc must begin with the LFC1 magic bytes; got {:?}",
        &bytes[..bytes.len().min(4)]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Two consecutive runs on the same unchanged source must leave the
/// `.loftc` byte-identical.  A regeneration here would mean either (a)
/// the cache key is unstable across runs (non-determinism in source
/// hashing) or (b) the cache-hit path silently falls through to full
/// compilation — both are staleness bugs the Tier-3 #7 ticket flagged.
#[test]
fn second_run_reuses_cache_bytes_unchanged() {
    let dir = fresh_tmp_dir("reuse");
    let src = dir.join("prog.loft");
    std::fs::write(&src, "fn main() { println(\"cache-reuse\"); }\n").unwrap();

    let out1 = run_loft(&src);
    assert!(out1.status.success(), "first run should succeed");
    let cached = dir.join("prog.loftc");
    let bytes1 = std::fs::read(&cached).expect(".loftc should exist after first run");

    let out2 = run_loft(&src);
    assert!(out2.status.success(), "second run should succeed");
    let bytes2 = std::fs::read(&cached).expect(".loftc should still exist");

    assert_eq!(
        bytes1, bytes2,
        ".loftc must be byte-identical across two runs on unchanged source"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Modifying the source must invalidate the cache: the SHA-256 part of
/// the key changes, so `read_cache` fails its key check and the write
/// path overwrites the `.loftc`.  Locks the invalidation half of the
/// hit/miss/invalidation triad.
#[test]
fn source_change_invalidates_and_rewrites_cache() {
    let dir = fresh_tmp_dir("invalidate");
    let src = dir.join("prog.loft");
    std::fs::write(&src, "fn main() { println(\"version-a\"); }\n").unwrap();

    let out1 = run_loft(&src);
    assert!(out1.status.success(), "first run should succeed");
    assert!(
        String::from_utf8_lossy(&out1.stdout).contains("version-a"),
        "expected 'version-a' in first-run stdout"
    );
    let cached = dir.join("prog.loftc");
    let bytes_a = std::fs::read(&cached).expect(".loftc after first run");

    std::fs::write(&src, "fn main() { println(\"version-b-longer\"); }\n").unwrap();

    let out2 = run_loft(&src);
    assert!(out2.status.success(), "run after edit should succeed");
    assert!(
        String::from_utf8_lossy(&out2.stdout).contains("version-b-longer"),
        "expected 'version-b-longer' in post-edit stdout (cache must not mask the new source)"
    );
    let bytes_b = std::fs::read(&cached).expect(".loftc after source edit");

    assert_ne!(
        bytes_a, bytes_b,
        ".loftc must be rewritten when the source changes"
    );
    assert!(
        bytes_b.starts_with(b"LFC1"),
        "rewritten .loftc must still start with magic bytes"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A deleted `.loftc` must be recreated on the next run.  Protects
/// against a regression where `byte_code_with_cache` silently skips
/// writing when the target directory exists but the file doesn't.
#[test]
fn missing_loftc_is_recreated() {
    let dir = fresh_tmp_dir("recreate");
    let src = dir.join("prog.loft");
    std::fs::write(&src, "fn main() { println(\"recreate\"); }\n").unwrap();

    let out1 = run_loft(&src);
    assert!(out1.status.success(), "initial run should succeed");
    let cached = dir.join("prog.loftc");
    assert!(cached.exists(), ".loftc should exist after initial run");

    std::fs::remove_file(&cached).expect("delete .loftc");

    let out2 = run_loft(&src);
    assert!(
        out2.status.success(),
        "run after .loftc deletion should succeed"
    );
    assert!(cached.exists(), ".loftc must be recreated when missing");
    let bytes = std::fs::read(&cached).unwrap();
    assert!(
        bytes.starts_with(b"LFC1"),
        "recreated .loftc must have LFC1 magic"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
