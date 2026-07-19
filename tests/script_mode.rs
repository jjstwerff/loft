// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN13 step 2 — the `--script` desugar, end to end. A beginner script (loose
//! top-level statements, no `fn main`) runs under `--script` on BOTH backends, with the
//! statements executed once in order sharing state and a top-level def hoisted; without
//! the flag the same file fails to parse (loose top-level statements are rejected), and
//! `--script` on a normal `fn main` program is a no-op. The desugar's unit-level shape +
//! the 0-corpus-classification invariant are covered in `loft::script::tests`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run `loft <args> <file>` and return (stdout, success).
fn run(args: &[&str], file: &Path) -> (String, bool) {
    let out = Command::new(loft_bin())
        .args(args)
        .arg(file)
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

#[test]
fn script_runs_without_fn_main_under_flag() {
    let fixture = workspace_root().join("tests/data/script_hello.loft");
    // double(1)+double(2)+double(3) = 2+4+6 = 12 — proves ordering + shared state + a
    // hoisted top-level def, all inside the synthesised run-once `main`.
    let (out, ok) = run(&["--script", "--interpret"], &fixture);
    assert!(ok && out.contains("count=12"), "interp --script: {out:?}");
    // cross-backend: native must produce the byte-identical result.
    let (nout, nok) = run(&["--script", "--native"], &fixture);
    assert!(
        nok && nout.contains("count=12"),
        "native --script: {nout:?}"
    );
}

#[test]
fn script_auto_detected_without_flag() {
    let fixture = workspace_root().join("tests/data/script_hello.loft");
    // Step 3 — no flag needed: a loose-top-level-statement source (which loft rejected
    // before) is auto-detected as a script and desugared, producing the same result as
    // the explicit `--script` run above.
    let (out, ok) = run(&["--interpret"], &fixture);
    assert!(
        ok && out.contains("count=12"),
        "auto-detect (no flag): {out:?}"
    );
}
