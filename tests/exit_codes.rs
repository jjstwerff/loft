// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Binary-level exit-code tests for L7.
//!
//! These tests invoke the compiled `loft` binary via `std::process::Command` so
//! they can verify the OS exit code — something the library-level test harness
//! cannot do.  The binary must be rebuilt (`cargo test` does this automatically
//! for integration tests).

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A program with no diagnostics must run and exit 0.
/// 46-caveats.loft is a clean caveat regression suite that should print "caveats: all ok".
#[test]
fn warning_only_program_exits_zero() {
    let script = workspace_root().join("tests/scripts/46-caveats.loft");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "expected exit 0 for warnings-only program, got {:?}; stdout={stdout:?}; stderr={stderr:?}",
        out.status.code()
    );
    assert!(
        stdout.contains("caveats: all ok"),
        "expected 'caveats: all ok' in output; got {stdout:?}"
    );
}

/// A program with a genuine parse error must exit non-zero.
#[test]
fn parse_error_exits_nonzero() {
    // Write a minimal syntax-error script to a temp file.
    let dir = std::env::temp_dir();
    let path = dir.join("loft_l7_test_parse_error.loft");
    std::fs::write(&path, "fn main() { x = 1\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    assert!(
        !out.status.success(),
        "expected non-zero exit for parse-error program, got exit 0"
    );
}

// ── P131: Loft CLI forwards script-level arguments (FIXED) ─────────────────
//
// `src/main.rs` now treats every token after the script path — including
// `--*` ones — as a script argument that is appended to `user_args` and
// forwarded to the script's `arguments()`. An explicit `--` separator is
// also accepted and skipped. The script must run cleanly when invoked
// with extra script-level arguments.
#[test]
fn p131_cli_forwards_script_dashdash_arg() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p131_args_test.loft");
    std::fs::write(&path, "fn main() { println(\"ran\"); }\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .arg("--mode")
        .arg("glb")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "expected exit 0 with --mode forwarded; stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("ran"),
        "expected script body to run; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// Explicit `--` separator must also be accepted (and consumed) before
/// script arguments.
#[test]
fn p131_cli_explicit_dashdash_separator() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p131_sep_test.loft");
    std::fs::write(&path, "fn main() { println(\"ran\"); }\n").expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .arg("--")
        .arg("--mode")
        .arg("glb")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    assert!(
        out.status.success(),
        "expected exit 0 with `--` separator; stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// P131: `arguments()` must return only the script-level arguments,
/// not the loft binary name or loft CLI flags like `--interpret`.
#[test]
fn p131_arguments_returns_only_script_args() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p131_arguments_content.loft");
    // Print each argument on its own line so we can inspect them.
    std::fs::write(&path, "fn main() { for a in arguments() { println(a) } }\n")
        .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .arg("--mode")
        .arg("glb")
        .arg("extra")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "expected exit 0; stderr={stderr:?}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["--mode", "glb", "extra"],
        "arguments() should return only script-level args, not loft flags; got: {lines:?}"
    );
}

// ── W1.1: --html produces a self-contained HTML file ──────────────────────

/// W1.1: `--html` must produce a valid HTML file with embedded WASM.
/// Requires the `wasm32-unknown-unknown` rustup target — skipped in CI
/// environments where the target is not installed.
#[test]
fn w1_1_html_export_produces_file() {
    let dir = std::env::temp_dir();
    let src = dir.join("loft_w1_1_test.loft");
    let out = dir.join("loft_w1_1_test.html");
    std::fs::write(&src, "fn main() { println(\"html-ok\"); }\n").unwrap();
    let result = Command::new(loft_bin())
        .arg("--html")
        .arg(&out)
        .arg(&src)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&src);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    if stderr.contains("wasm32-unknown-unknown") && stderr.contains("not be installed") {
        eprintln!("SKIP: wasm32-unknown-unknown target not installed");
        return;
    }
    assert!(
        result.status.success(),
        "expected --html to succeed; stdout={stdout:?}; stderr={stderr:?}"
    );
    let html = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    assert!(
        html.contains("<!DOCTYPE html>"),
        "HTML should start with doctype"
    );
    assert!(
        html.contains("loft_start"),
        "HTML should reference loft_start entry point"
    );
    assert!(
        html.contains("buildLoftImports"),
        "HTML should contain the GL bridge"
    );
    // WASM binary is embedded as base64 — file should be substantial
    assert!(
        html.len() > 5000,
        "HTML too small ({} bytes) — WASM likely missing",
        html.len()
    );
}

// ── P166: file().content() on a binary file must surface a warning ────────
//
// Root-cause data-loss bug: prior to the 2026-04-17 fix,
// `file("x.glb").content()` silently returned "" on any file whose bytes
// failed UTF-8 decode — `src/state/io.rs::get_file_text`'s `read_to_string`
// failure path called `buf.clear()` with no log.  Fix: emit an actionable
// stderr warning on `ErrorKind::InvalidData` so the user sees the misuse
// the first time it runs, with a pointer at the `#format = LittleEndian;
// #read(n)` idiom.

/// P166: reading a non-UTF-8 file via .content() must emit a stderr warning
/// containing the phrase "non-UTF-8 bytes" along with the file size and a
/// pointer at the binary-read idiom.
#[test]
fn p166_content_on_binary_file_warns() {
    let dir = std::env::temp_dir();
    let bin_path = dir.join("loft_p166_binary.bin");
    // Non-UTF-8 bytes: 0xFF and 0xFE are invalid UTF-8 start bytes.
    std::fs::write(&bin_path, [0xFFu8, 0xFE, 0xFD, 0xFC, 0xFB]).expect("write temp binary file");

    let script_path = dir.join("loft_p166_script.loft");
    let script = format!(
        "fn main() {{\n  \
            f = file(\"{}\");\n  \
            c = f.content();\n  \
            println(\"len={{len(c)}}\");\n  \
            assert(len(c) == 0, \"content should be empty on binary\");\n\
         }}\n",
        bin_path.display()
    );
    std::fs::write(&script_path, &script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&bin_path);
    let _ = std::fs::remove_file(&script_path);

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "program should still exit 0 (empty string is valid); stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stderr.contains("non-UTF-8 bytes"),
        "expected 'non-UTF-8 bytes' warning in stderr; got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("5 bytes in file"),
        "warning should include the actual file size; got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("#format = LittleEndian"),
        "warning should name the correct binary-read idiom; got stderr={stderr:?}"
    );
}

// ── 6a.18: moros_glb CLI tool end-to-end ──────────────────────────────────

/// Phase 6a.18 — the `moros_glb` CLI example reads a map JSON and writes
/// a GLB.  This verifies the full loft-level pipeline: JSON parse → Map →
/// build_hex_meshes → save_scene_glb, driven from a standalone script via
/// `arguments()`.
#[test]
fn moros_glb_cli_end_to_end() {
    let dir = std::env::temp_dir();
    let json_path = dir.join("loft_moros_glb_input.json");
    let glb_path = dir.join("loft_moros_glb_output.glb");
    // Minimal map with one material in the palette.
    let map_json = r#"{
        "m_name": "cli_test",
        "m_chunks": [],
        "m_material_palette": [
            {"md_name": "stone", "md_category": "terrain", "md_stair_kind": "",
             "md_texture": 0, "md_tint_r": 120, "md_tint_g": 120, "md_tint_b": 120,
             "md_walkable": 1, "md_swimmable": 0, "md_climbable": 0,
             "md_slippery": 0, "md_loud": 0}
        ],
        "m_wall_palette": [],
        "m_item_palette": [],
        "m_spawns": [],
        "m_routines": []
    }"#;
    std::fs::write(&json_path, map_json).expect("write map JSON");
    let _ = std::fs::remove_file(&glb_path);

    let script = workspace_root().join("lib/moros_render/examples/moros_glb.loft");
    let path_flag = format!("{}/", workspace_root().display());
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--path")
        .arg(&path_flag)
        .arg(&script)
        .arg(&json_path)
        .arg(&glb_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "CLI should exit 0; stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("wrote"),
        "CLI should print 'wrote <path>'; got stdout={stdout:?}"
    );
    assert!(
        glb_path.exists(),
        "GLB file should be written at {}",
        glb_path.display()
    );
    // Read the first 4 bytes and verify 'glTF' magic (LE bytes).
    let bytes = std::fs::read(&glb_path).expect("read GLB");
    let _ = std::fs::remove_file(&json_path);
    let _ = std::fs::remove_file(&glb_path);
    assert!(
        bytes.len() >= 12,
        "GLB should have at least the 12-byte header; got {} bytes",
        bytes.len()
    );
    assert_eq!(
        &bytes[0..4],
        b"glTF",
        "GLB should start with 'glTF' magic"
    );
    // Version is bytes 4..8, little-endian u32; must be 2.
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    assert_eq!(version, 2, "GLB version should be 2");
}

/// P166: reading a valid UTF-8 text file via .content() must NOT emit the
/// warning — the signal is strictly on decode failure, not on all binary
/// opens.
#[test]
fn p166_content_on_text_file_no_warning() {
    let dir = std::env::temp_dir();
    let text_path = dir.join("loft_p166_text.txt");
    std::fs::write(&text_path, "hello world\n").expect("write temp text file");

    let script_path = dir.join("loft_p166_text_script.loft");
    let script = format!(
        "fn main() {{\n  \
            f = file(\"{}\");\n  \
            c = f.content();\n  \
            assert(len(c) > 0, \"content should be non-empty\");\n\
         }}\n",
        text_path.display()
    );
    std::fs::write(&script_path, &script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&text_path);
    let _ = std::fs::remove_file(&script_path);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "text-file read should succeed");
    assert!(
        !stderr.contains("non-UTF-8 bytes"),
        "text file should not trigger the P166 warning; got stderr={stderr:?}"
    );
}
