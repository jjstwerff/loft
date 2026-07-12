// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Test-backing for the LIBRARY-BACKED doc examples — `tests/docs/14-image.loft`
// (`use imaging`) and `tests/docs/21-random.loft` (`use random`).
//
// The in-process `wrap::dir` / `native::native_dir` doc harnesses can't run these
// (they'd have to provision each package's #native cdylib against the harness's own
// loft-ffi, which they don't — see SUITE_SKIP / NATIVE_SKIP), so those two files are
// skipped there.  Here we drive the REAL `loft` binary as a subprocess instead: it
// resolves + builds each library's cdylib against its OWN loft-ffi and links it by
// C-ABI, exactly as a user's `loft` does — the highest-fidelity check.  We run BOTH
// backends and require the SAME output (the interpret == native master invariant).
//
// NOTE: these `use` a registry package, so a cold run auto-installs it (network) and
// `--native` builds the cdylib (rustc).  Bounded with `LOFT_TIMEOUT`.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn doc(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/docs")
        .join(name)
}

/// Run one doc file on one backend; return trimmed stdout, asserting a clean exit.
fn run(file: &str, native: bool) -> String {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(if native { "--native" } else { "--interpret" })
        .arg(doc(file))
        .env("LOFT_TIMEOUT", "180");
    let backend = if native { "native" } else { "interpret" };
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn loft for {file} ({backend}): {e}"));
    assert!(
        out.status.success(),
        "{file} failed on {backend} (exit {:?}):\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// Both backends must run clean AND agree (interpret == native).
fn both_backends_agree(file: &str) {
    let interp = run(file, false);
    let native = run(file, true);
    assert_eq!(
        interp, native,
        "{file}: interpret and native output diverge\n--- interpret ---\n{interp}\n--- native ---\n{native}"
    );
}

#[test]
fn image_doc_uses_imaging_library() {
    both_backends_agree("14-image.loft");
}

#[test]
fn random_doc_uses_random_library() {
    both_backends_agree("21-random.loft");
}
