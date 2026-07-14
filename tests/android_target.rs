// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN-android B1 — the `--native-android` build target (src/android.rs).
//
// The design's invariant (doc/claude/plans/android-build-target.md): a loft build
// target is a *descriptor* over ONE target-agnostic generated-Rust core, so the
// same source `--native` compiles for the host cross-compiles to a bionic AArch64
// `.so` with only toolchain/entry/packaging differences — no Android codegen path.
// B0 confirmed the core cross-compiles; this checks the loft-driven pipeline
// (emit → cross-build runtime rlib → link cdylib) end to end.
//
// The build test needs the Android NDK, which the normal CI images lack, so it is
// a SEPARATE, opt-in leg gated on `ANDROID_NDK_HOME` (like the daily Windows leg,
// never a required check).  The error-path test always runs — it is cheap and
// guards that the flag exists and fails with an actionable message.

use std::io::Write;
use std::process::Command;

fn write_probe(name: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("loft_android_target");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(name);
    std::fs::File::create(&path)
        .expect("create probe")
        .write_all(src.as_bytes())
        .expect("write probe");
    path
}

/// True if `path` is an ELF64 shared object for AArch64 (`e_machine == 183`).
/// Reads only the header, so it needs no `readelf`/NDK tool on the test host.
fn is_aarch64_elf(path: &std::path::Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    // ELF magic + 64-bit class + little-endian, then e_machine (offset 18, u16 LE).
    bytes.len() > 19
        && &bytes[0..4] == b"\x7fELF"
        && bytes[4] == 2 // ELFCLASS64
        && bytes[5] == 1 // ELFDATA2LSB
        && u16::from_le_bytes([bytes[18], bytes[19]]) == 183 // EM_AARCH64
}

/// `--native-android` on a non-trivial program (struct + fn + for-loop + string
/// interpolation) produces a real AArch64 shared object.  Skipped unless
/// `ANDROID_NDK_HOME` is set, so a normal `cargo test` run never needs an NDK.
#[test]
fn android_native_so_is_aarch64_elf() {
    if std::env::var_os("ANDROID_NDK_HOME").is_none()
        && std::env::var_os("ANDROID_NDK_ROOT").is_none()
    {
        eprintln!("skipping: ANDROID_NDK_HOME not set (opt-in Android build leg)");
        return;
    }
    let src = r#"
struct Point { x: integer, y: integer }
fn dist2(p: Point) -> integer { p.x * p.x + p.y * p.y }
fn main() {
  total = 0;
  for i in 0..5 {
    p = Point { x: i, y: i + 1 };
    total = total + dist2(p);
  }
  print("total={total}\n");
}
"#;
    let probe = write_probe("prog.loft", src);
    let out_so = std::env::temp_dir()
        .join("loft_android_target")
        .join("prog.so");
    let _ = std::fs::remove_file(&out_so);
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native-android")
        .arg(&out_so)
        .arg(&probe)
        // The runtime-rlib cross-build can take a minute cold; give it room.
        .env("LOFT_TIMEOUT", "600")
        .output()
        .expect("spawn loft");
    assert!(
        out.status.success(),
        "--native-android failed:\nstdout:{}\nstderr:{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        is_aarch64_elf(&out_so),
        "output {} is not an AArch64 ELF shared object\nstderr:{}",
        out_so.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Without an NDK, `--native-android` fails fast with a message that names the env
/// var to set — not an opaque `rustc`/`cc` error deep in the build.
#[test]
fn android_without_ndk_reports_actionable_error() {
    let probe = write_probe("noNdk.loft", "fn main() { print(\"hi\\n\"); }");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native-android")
        .arg(&probe)
        .env_remove("ANDROID_NDK_HOME")
        .env_remove("ANDROID_NDK_ROOT")
        .output()
        .expect("spawn loft");
    assert!(
        !out.status.success(),
        "expected --native-android to fail without an NDK"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ANDROID_NDK_HOME"),
        "error should name ANDROID_NDK_HOME; got:\n{stderr}"
    );
}
