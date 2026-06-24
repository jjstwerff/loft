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

/// An unresolvable `#native` symbol (no cdylib provides it) must surface a LOUD
/// diagnostic at LOAD time — naming the symbol and how to rebuild — not stay
/// silent until a generic panic at first call.  The warning is non-fatal: a
/// declared-but-never-called native still lets the program run (exit 0), so the
/// operator learns which library to rebuild without the program being aborted.
#[test]
fn unresolved_native_warns_at_load_not_at_call() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "loft_unresolved_native_{}.loft",
        std::process::id()
    ));
    // The #native is declared but never called: the load-time warning must fire
    // anyway, and the program must still reach exit 0.
    std::fs::write(
        &path,
        "pub fn ghost_fn(x: integer) -> integer;\n\
         #native \"loft_ghost_nonexistent_symbol\"\n\n\
         fn main() { print(\"ran fine\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "uncalled unresolved native must not abort the program; got exit {:?}\nstderr: {stderr}",
        out.status.code()
    );
    assert!(
        stdout.contains("ran fine"),
        "program body must still run; stdout: {stdout}"
    );
    assert!(
        stderr.contains("did not load") && stderr.contains("loft_ghost_nonexistent_symbol"),
        "expected a load-time diagnostic naming the unresolved symbol; stderr: {stderr}"
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

// ── P171: --native mode OpCopyRecord panicked on 0x8000-tagged tp ─────────
//
// Root cause: `src/codegen_runtime.rs::OpCopyRecord` was missing the 0x8000
// "free source after copy" tag-bit masking that the bytecode equivalent
// (`src/state/io.rs::copy_record`, line 1021) applies.  Any caller setting
// the tag — e.g. `copy_ref` on a struct-returning call's result — caused
// an out-of-bounds panic at `Types::size()` (index 0x805B = 32859 into a
// 124-entry array).  Surfaced by running moros_render's `map_export_glb`
// path under `--native`.  Fix: port the mask + `remove_claims` call +
// free-source branch from the bytecode version.

/// P171: compiling and running `isolated_stair.loft` under `--native` must
/// complete without panic and produce the same output as the interpreter.
/// Guards a native-mode run through `map_export_glb` → `map_build_scene`
/// → OpCopyRecord with the 0x8000 tag set.
#[test]
fn p171_native_copy_record_high_bit_does_not_panic() {
    let script =
        workspace_root().join("tests/fixtures/libs/moros_render/examples/isolated_stair.loft");
    // The script writes `isolated_stair.glb` CWD-relative (portable — Windows
    // has no /tmp), so run it in a temp working dir and read the GLB from
    // there.  Mirrors the moros_glb_cli_end_to_end pattern below.
    let glb_path = std::env::temp_dir().join("isolated_stair.glb");
    let _ = std::fs::remove_file(&glb_path);
    let path_arg = format!("{}/", workspace_root().display());
    let out = Command::new(loft_bin())
        .arg("--native")
        .arg("--path")
        .arg(&path_arg)
        .arg(&script)
        .current_dir(std::env::temp_dir())
        .output()
        .expect("invoke loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Skip if rustc isn't available, the graphics native rlib isn't
    // compiled against the current rustc, or the rlib hasn't been
    // built at all — all three are environment issues, not
    // regressions.  E0514 = rustc version mismatch; E0463 = can't
    // find crate (rlib missing / `auto_build_native` couldn't run on
    // this runner, e.g. missing X11 headers for glutin).
    // E0308 with a `*const i32` vs `*const i64` pointer mismatch is
    // also environmental: the loft binary and `loft_graphics_native`
    // cdylib were built against different integer-width layouts
    // (typically a stale `target/release/loft` from before the i64
    // migration against a fresh cdylib, or vice versa).  A clean
    // rebuild of both crates resolves it; it is never a regression
    // in the code under test.
    // @P229 G2 (Windows LNK1181 windows-targets link-search) was fixed
    // 2026-05-30 in src/native_utils.rs, so the previous LNK1181 skip branch
    // is removed — this test now exercises the native multi-lib link on
    // Windows too.  The remaining skips are genuine toolchain-availability
    // cases (E0514 rustc-version mismatch, E0463 missing rlib, the i32/i64
    // layout-mismatch from a stale cdylib) — never code-under-test bugs.
    if stderr.contains("rustc not found")
        || stderr.contains("E0514")
        || stderr.contains("E0463")
        || (stderr.contains("E0308")
            && stderr.contains("*const i32")
            && stderr.contains("*const i64"))
    {
        eprintln!("SKIP: native toolchain not ready — {stderr}");
        return;
    }
    assert!(
        out.status.success(),
        "native run must exit 0; stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "native run must not panic; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("mesh '1': 96 verts, 48 tris"),
        "output must match interpreter (96-vert default-rise stair); \
         stdout={stdout:?}"
    );
    // Verify the GLB the script wrote has the glTF magic.
    let glb = std::fs::read(&glb_path).expect("GLB written");
    assert_eq!(&glb[0..4], b"glTF", "GLB magic must be 'glTF'");
    let _ = std::fs::remove_file(&glb_path);
}

// ── tail-call ref-return capture with a store-lifetime "lifted" arg ────────
//
// Native-codegen regression (the zero-trust ztserve blocker; fix in
// `src/generation/emit.rs`, the `is_tail_capture_call` branch).  A ref-returning
// tail call captured into `__native_tail_ret` whose argument is an inline
// call-result that the store-lifetime pass LIFTS emitted the lift as a leading
// `{ … };` statement; its own `;` terminated the capture `let` early — binding the
// var to the lift's `()` and detaching the call (rustc E0308 "expected DbRef, found
// ()").  The fix wraps the capture value in a block (lift = statement, call = tail).

/// Native-codegen regression: a captured ref-return tail call with a store-lifetime
/// "lifted" inline-call argument must compile + run under `--native`.  Mirrors the
/// zero-trust `opsurface::handle_write_s` shape `resp_frame(rid, tag, empty_body())`.
#[test]
fn tail_capture_lifted_arg_compiles_native() {
    let script = workspace_root().join("tests/scripts/tail-capture-lifted-arg.loft");
    let out = Command::new(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("invoke loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Skip on toolchain-availability issues (not code-under-test regressions),
    // mirroring p171 above.
    if stderr.contains("rustc not found") || stderr.contains("E0514") || stderr.contains("E0463") {
        eprintln!("SKIP: native toolchain not ready — {stderr}");
        return;
    }
    assert!(
        out.status.success(),
        "native run must exit 0 (the lifted-arg tail capture must not E0308); \
         stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("ok"),
        "expected the script's 'ok' (its assert passed); stdout={stdout:?}"
    );
}

// ── (I-Join): an inferred integer local widens to the join of its writes ───
//
// The #433-residual (formal/types.md was D4).  `arg = b[0]` (vector<u8>) infers
// `arg : u8`; `arg = arg*256 + b[1]` assigns an `integer` that doesn't fit u8.
// Pre-fix `arg` stayed u8 and the wider write errored on both backends (E0308 on
// native for the cbor shape).  An inferred local now takes the join (`integer`);
// an annotated `arg: u8` would still be constrained.

/// (I-Join) regression: an inferred multiply-assigned integer local compiles + runs
/// natively, widened to the join of its writes (not the narrowest first one).
#[test]
fn ijoin_multiply_assigned_widens_native() {
    let script = workspace_root().join("tests/scripts/433-ijoin-multiply-assigned.loft");
    let out = Command::new(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("invoke loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stderr.contains("rustc not found") || stderr.contains("E0514") || stderr.contains("E0463") {
        eprintln!("SKIP: native toolchain not ready — {stderr}");
        return;
    }
    assert!(
        out.status.success(),
        "native run must exit 0 (the inferred local must widen to the join, not narrow); \
         stdout={stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stdout.contains("ok"),
        "expected the script's 'ok' (second(v) == 258); stdout={stdout:?}"
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
    // Use forward slashes in the embedded path so the loft lexer doesn't
    // treat Windows backslashes as escape sequences (`\U`, `\R`, …).
    let path_in_script = bin_path.display().to_string().replace('\\', "/");
    let script = format!(
        "fn main() {{\n  \
            f = file(\"{path_in_script}\");\n  \
            c = f.content();\n  \
            println(\"len={{len(c)}}\");\n  \
            assert(len(c) == 0, \"content should be empty on binary\");\n\
         }}\n"
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

// ── P168: arguments() leaked argv when zero script-level args ────────────
//
// Prior to 2026-04-17, `src/database/format.rs::os_arguments` fell back
// to `std::env::args_os()` when `user_args` was empty, returning the
// binary path + loft CLI flags + script path.  P131's filter only ran
// through the `user_args` path.  Fix: always return `user_args`
// (an empty vector is a correct result).

/// P168: running a loft script with no script-level args must produce
/// `arguments()` == [] — no binary path, no `--interpret`, no script path.
#[test]
fn p168_arguments_empty_when_no_script_args() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p168_args_empty.loft");
    // Script prints each argument; empty vector → no lines, just "count=0".
    std::fs::write(
        &path,
        "fn main() {\n  \
             a = arguments();\n  \
             println(\"count={len(a)}\");\n  \
             for s in a { println(\"  [{s#index}] {s}\"); }\n\
         }\n",
    )
    .expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "expected exit 0; stderr={stderr:?}");
    assert!(
        stdout.contains("count=0"),
        "arguments() should be empty when no script args given; got stdout={stdout:?}"
    );
    // Belt-and-suspenders: make sure the binary path isn't smuggled in.
    assert!(
        !stdout.contains("target/release/loft"),
        "arguments() must not leak the loft binary path; got stdout={stdout:?}"
    );
    assert!(
        !stdout.contains("--interpret"),
        "arguments() must not leak loft CLI flags; got stdout={stdout:?}"
    );
}

// ── P169: lambda-suggestion error message accuracy ───────────────────────
//
// The `|x: T| { ... }` form is rejected ("Type annotations are not
// allowed in |x| lambdas").  The suggested alternative used to include
// `-> <ret>` in the template, misleading users to try `-> void` which
// fails with "Undefined type void" — loft omits the `->` clause for
// void returns.  Fix: updated the suggestion in
// `src/parser/vectors.rs` to make `<ret>` optional and explicitly
// call out `-> void` as invalid.

/// P169: the "Type annotations not allowed in |x|" diagnostic must
/// suggest `fn(x: <type>) { ... }` (no mandatory `-> <ret>`) and warn
/// that `-> void` is not a valid type.
#[test]
fn p169_lambda_suggestion_mentions_omitting_return_type() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p169_lambda_types.loft");
    std::fs::write(&path, "fn main() {\n  _ = |x: integer| { x * 2 };\n}\n")
        .expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    assert!(!out.status.success(), "expected parse error");
    // @P282: loft emits parse diagnostics to STDERR (rustc / clang convention).
    let stdout = String::from_utf8_lossy(&out.stderr);
    // The new suggestion shows `fn(x: <type>) { ... }` without mandatory
    // `-> <ret>`, and calls out `-> void` as invalid.
    assert!(
        stdout.contains("fn(x: <type>) { ... }"),
        "suggestion should be `fn(x: <type>) {{ ... }}`; got stderr={stdout:?}"
    );
    assert!(
        stdout.contains("`-> void` is not a valid type"),
        "suggestion should warn about `-> void`; got stderr={stdout:?}"
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

    let script = workspace_root().join("tests/fixtures/libs/moros_render/examples/moros_glb.loft");
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
    assert_eq!(&bytes[0..4], b"glTF", "GLB should start with 'glTF' magic");
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
    // Forward slashes so Windows backslashes don't become lexer escapes.
    let path_in_script = text_path.display().to_string().replace('\\', "/");
    let script = format!(
        "fn main() {{\n  \
            f = file(\"{path_in_script}\");\n  \
            c = f.content();\n  \
            assert(len(c) > 0, \"content should be non-empty\");\n\
         }}\n"
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

/// DX-source-map — the native-codegen emitter writes
/// `// loft:<file>:<line>` comments above each function header and
/// each statement so rustc errors on the generated Rust code map
/// back to the originating loft source.
#[test]
fn native_emit_includes_loft_source_map() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_source_map_demo.loft");
    let script = "fn add(a: integer, b: integer) -> integer { a + b }\n\
                  fn main() { x = add(1, 2); println(\"{x}\") }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-rust")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "introspect should succeed");
    // Loft's source-map emission canonicalizes paths.  On Windows
    // canonicalize() returns the `\\?\` UNC form; on Linux/macOS
    // it returns the absolute path with symlinks resolved.  Match
    // the test's expectation against the same canonical form.  A
    // simple `.display()` form fails on Windows because the test's
    // path lacks the UNC prefix.
    let canonical = std::fs::canonicalize(&script_path).unwrap_or_else(|_| script_path.clone());
    let path_str = canonical.display().to_string();
    // Function-header comment maps to the .loft source line.
    // Use ends_with-style match (`// loft:{stem-suffix}:1\nfn n_add(`)
    // when the full path comparison fails — robust to canonical
    // path variations across platforms.
    let header_n_add = format!("// loft:{path_str}:1\nfn n_add(");
    let header_n_main = format!("// loft:{path_str}:2\nfn n_main(");
    let stem_n_add = "loft_source_map_demo.loft:1\nfn n_add(".to_string();
    let stem_n_main = "loft_source_map_demo.loft:2\nfn n_main(".to_string();
    assert!(
        stdout.contains(&header_n_add) || stdout.contains(&stem_n_add),
        "expected source-map header above n_add (canonical or stem match); got {stdout}"
    );
    assert!(
        stdout.contains(&header_n_main) || stdout.contains(&stem_n_main),
        "expected source-map header above n_main (canonical or stem match); got {stdout}"
    );
}

/// P196: tuple struct field whose element is a fn-ref must project
/// `.0` from the runtime `(u32, DbRef)` tuple before the OpSetInt4
/// `as i32` cast.  Regression guard: a Var-of-fn-ref-tuple source
/// (which can't be folded to `Value::Int(d_nr)` at parse time)
/// must emit `(i64::from((var.0).0))` — i.e. project u32 d_nr from
/// the tuple-element's `(u32, DbRef)` shape and widen for the
/// template's null-check.  Without the fix the codegen substitutes
/// `var.0 as i32` directly, which rustc rejects with E0605 (non-
/// primitive cast on tuple type) and E0308 on the matching null
/// check `var.0 == i64::MIN`.
#[test]
fn p196_native_codegen_projects_fn_ref_d_nr() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_p196_codegen.loft");
    let script = "struct Pair { v: (fn(integer) -> integer, integer) }\n\
                  fn p_dbl(x: integer) -> integer { x + x }\n\
                  fn build(f: fn(integer) -> integer, n: integer) -> (fn(integer) -> integer, integer) { (f, n) }\n\
                  fn main() { pp = build(p_dbl, 21); p = Pair { v: pp }; }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-rust")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "introspect should succeed");
    // Fix invariant: every set_i32_raw emitted for the fn-ref tuple
    // element widens via `i64::from(...)` of the projected `.0` —
    // not a bare `var.0 as i32` (which rustc rejects on tuple type).
    assert!(
        stdout.contains("i64::from((var___ref_1.0).0)"),
        "expected fn-ref d_nr projection `i64::from((var___ref_1.0).0)`; got:\n{stdout}"
    );
    // And the buggy bare `var___ref_1.0 == i64::MIN` shape must be gone —
    // it would compare a `(u32, DbRef)` tuple to an i64.
    assert!(
        !stdout.contains("(var___ref_1.0) == i64::MIN"),
        "fn-ref tuple field should not be compared to i64::MIN as a bare tuple; got:\n{stdout}"
    );
}

/// DX-diff — `--introspect --diff <baseline>` exits 0 when the
/// baseline matches and 1 when it differs (mirroring `diff -u`'s
/// exit code).  Lets devs answer "did my parser tweak change
/// anything?" with a single command.
#[test]
fn introspect_diff_against_baseline() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_diff_demo.loft");
    let baseline_path = dir.join("loft_diff_baseline.txt");
    let script = "fn main() { println(\"hello\") }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    // Capture baseline.
    let baseline_out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("baseline capture failed");
    std::fs::write(&baseline_path, &baseline_out.stdout).expect("write baseline");

    // Identical inputs → exit 0.
    let same = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg("--diff")
        .arg(&baseline_path)
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("diff (identical) failed");
    assert_eq!(
        same.status.code(),
        Some(0),
        "identical inputs should exit 0; stderr={:?}",
        String::from_utf8_lossy(&same.stderr)
    );

    // Mutate the script with a STRUCTURAL change so the types table
    // differs (string-literal changes alone don't show up in
    // `--show-types`).
    std::fs::write(
        &script_path,
        "fn add(a: integer) -> integer { a + 1 }\nfn main() { println(\"hello\") }\n",
    )
    .expect("rewrite temp script");

    let differs = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg("--diff")
        .arg(&baseline_path)
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("diff (differs) failed");
    assert_eq!(
        differs.status.code(),
        Some(1),
        "differing inputs should exit 1; stdout={:?}",
        String::from_utf8_lossy(&differs.stdout)
    );

    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&baseline_path);
}

/// `--show-types --trace` emits a per-expression type tape that
/// makes dep-propagation flow visible at every chaining step
/// (`.field`, `.tuple_idx`, `[idx]`, `(args)`).  Designed so a
/// future P197-class bug shows up as a missing `[host]` suffix
/// on an intermediate type, not just the eventual return.
#[test]
fn introspect_show_types_trace_renders_per_expression() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_trace_demo.loft");
    let script = "struct A { v: (text, text) }\n\
                  fn first() -> text {\n  \
                      a = A { v: (\"hello\", \"world\") };\n  \
                      a.v.0\n\
                  }\n\
                  fn main() { println(\"{first()}\") }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg("--trace")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "introspect should succeed");
    assert!(
        stdout.contains("trace (per-expression types):"),
        "expected trace section header; got {stdout}"
    );
    // The per-step tape shows the tuple's element types each carry
    // the host's dep AFTER the `.v` step — this is the line that
    // would have read `(text, text)` (no `[a]`) before the P197 fix.
    assert!(
        stdout.contains("(text[\"a\"], text[\"a\"])"),
        "expected `a.v` step to render `(text[\"a\"], text[\"a\"])` \
         (each tuple element carries dep on host `a`); got {stdout}"
    );
    // And the final `.0` extraction preserves the dep.
    assert!(
        stdout.contains("text[\"a\"]"),
        "expected final `.0` step to render `text[\"a\"]`; got {stdout}"
    );
}

/// Plan-08 phase 01 — `--introspect --show-types` emits a per-fn
/// type table where `Type::show()` includes dependency suffixes
/// (e.g. `text["a"]`).  Designed to surface dep-tracking bugs at a
/// glance; the post-P197 fix means a tuple-element text returned
/// from a struct field carries the host as a dep.  This test pins
/// the visible `text["a"]` annotation so any regression in dep
/// propagation through `Type::Tuple` shows up here too.
#[test]
fn introspect_show_types_renders_deps() {
    let dir = std::env::temp_dir();
    let script_path = dir.join("loft_introspect_types_demo.loft");
    let script = "struct A { v: (text, text) }\n\
                  fn first() -> text {\n  \
                      a = A { v: (\"hello\", \"world\") };\n  \
                      a.v.0\n\
                  }\n\
                  fn main() { println(\"{first()}\") }\n";
    std::fs::write(&script_path, script).expect("write temp script");

    let out = Command::new(loft_bin())
        .arg("--introspect")
        .arg("--show-types")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "introspect should succeed");
    assert!(
        stdout.contains("=== types ==="),
        "expected types section header; got {stdout}"
    );
    // The fix for P197 propagates the host (`a`) as a dep through
    // tuple-element text reads to the function's return type.
    // If this assertion fails, the dep propagation in
    // `Type::depending` / `parse_part` regressed.
    assert!(
        stdout.contains("n_first -> text[\"a\"]"),
        "expected `n_first -> text[\"a\"]` (P197 dep propagation); got {stdout}"
    );
}

/// @P367 regression: `loft --tests` must report a test FAILED (and exit
/// non-zero) when an `assert(false)` / `panic` / divide-by-zero fired inside it
/// — these set a typed runtime fault and halt WITHOUT a Rust panic (the C66
/// path), so the runner previously scored them PASSED.
#[test]
fn tests_runner_fails_on_assert_and_fault() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p367_fault.loft");
    std::fs::write(
        &path,
        "fn test_bad_assert() { assert(false, \"boom\"); }\n\
         fn test_panic() { panic(\"kapow\"); }\n\
         fn test_ok() { assert(1 == 1, \"fine\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--tests")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "expected non-zero exit when a test asserts/panics; stdout={stdout}"
    );
    assert!(
        stdout.contains("FAILED") && stdout.contains("2 failed") && stdout.contains("1 passed"),
        "expected 2 failed / 1 passed; got {stdout}"
    );
    assert!(
        stdout.contains("assertion failed: boom") && stdout.contains("panic: kapow"),
        "expected the fault messages in the FAIL lines; got {stdout}"
    );
}

/// @P367 companion: a `@EXPECT_FAIL` test whose intentional fault fires must
/// still PASS (and the file exit 0) — the fix must not break expected-fail.
#[test]
fn tests_runner_expect_fail_still_passes() {
    let dir = std::env::temp_dir();
    let path = dir.join("loft_p367_expectfail.loft");
    std::fs::write(
        &path,
        "// @EXPECT_FAIL: boom\nfn test_intentional() { assert(false, \"boom\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--tests")
        .arg(&path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "expected exit 0 for an @EXPECT_FAIL intentional fault; stdout={stdout}"
    );
    assert!(
        stdout.contains("1 passed"),
        "expected the @EXPECT_FAIL test to pass; got {stdout}"
    );
}

/// @P368 regression: the divide-by-zero warning must NOT fire when the divisor
/// is a non-zero literal constant (int OR float), but MUST still fire for a
/// variable divisor.  Also: the message must not say "integer division".
///
/// @P368 follow-up (dryopea-surfaced): the warning must ALSO not fire when
/// the dividend is float / single and the divisor is an integer literal
/// (`x / 3` with `x: float`).  The parser wraps the literal in an
/// `OpConvFloatFromInt` cast for type matching; without seeing through that
/// cast, `lit_nonzero` returns None and the warning fires spuriously.  The
/// `e = x / 3` case in this test exercises that arm.
#[test]
fn div_by_literal_constant_no_warning() {
    let dir = std::env::temp_dir();
    let safe = dir.join("loft_p368_safe.loft");
    std::fs::write(
        &safe,
        "fn calc(x: float, c: integer) -> float {\n  \
           a = x / 2.0;\n  b = x / 0.75;\n  d = c / 2;\n  \
           e = x / 3;\n  \
           x + a + b + (d as float) + e\n}\n\
         fn main() { println(\"{calc(10.0, 10)}\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&safe)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("may produce null on divide-by-zero"),
        "literal-constant divisors must not warn; got stderr={stderr}"
    );

    // A variable divisor is genuinely unchecked — the warning MUST still fire,
    // and must read "division" (not "integer division").
    let unsafe_ = dir.join("loft_p368_unsafe.loft");
    std::fs::write(
        &unsafe_,
        "fn calc(c: integer, y: integer) -> integer { c / y }\n\
         fn main() { println(\"{calc(10, 2)}\"); }\n",
    )
    .expect("write temp file");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&unsafe_)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("division may produce null on divide-by-zero"),
        "variable divisor must still warn; got stderr={stderr}"
    );
    assert!(
        !stderr.contains("integer division may produce null"),
        "warning must not say 'integer division'; got stderr={stderr}"
    );
}

// ── #333 / C80: undefended div-by-zero is null-and-continue on BOTH backends ──
// E-Uncomp (formal/operational.md): a calculation fault never halts.  `5 / z`
// with `z == 0` yields the null sentinel and execution CONTINUES (exit 0) — the
// interpreter and the compiled native binary must agree.  (Was: both exited 1
// via the raise/NATIVE_FAIL_FAST halt; reversed by C80.)
#[test]
fn issue_333_div_zero_null_continues() {
    let pid = std::process::id();
    let script = std::env::temp_dir().join(format!("loft_i333_{pid}.loft"));
    std::fs::write(
        &script,
        "fn main() {\n  z = 0;\n  a = 5 / z;\n  print(\"reached a={a}\");\n}\n",
    )
    .expect("write script");
    for mode in ["--interpret", "--native"] {
        let out = Command::new(loft_bin())
            .arg(mode)
            .arg(&script)
            .current_dir(workspace_root())
            .output()
            .expect("invoke loft");
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{mode}: expected exit 0 (null-and-continue), got {:?}\nstdout: {stdout}\nstderr: {stderr}",
            out.status.code()
        );
        assert!(
            stdout.contains("reached a=null"),
            "{mode}: execution must continue past the fault with null: {stdout}"
        );
    }
    let _ = std::fs::remove_file(&script);
}
