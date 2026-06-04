// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 Arc N / N3 Phase A — `use <lib>` auto-compiles a normal loft library
//! to a native cdylib and dispatches to it, on the **real binary**.
//!
//! The fixture `tests/lib/mathnative/` is a NORMAL loft library (no `#native`, no
//! Rust crate) whose `loft.toml` opts in with `[library] compile = "native"`.  A
//! script that does `use mathnative;` and calls its functions must run them
//! natively (the library compiles) while the script interprets — output identical
//! to the all-interpreted run.  This is the headline of Arc N realised end-to-end.

use std::process::Command;

/// Platform cdylib filename for the auto-built `mathnative` library.
fn cdylib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "loft_auto_mathnative.dll"
    } else if cfg!(target_os = "macos") {
        "libloft_auto_mathnative.dylib"
    } else {
        "libloft_auto_mathnative.so"
    }
}

#[test]
fn use_compile_native_library_dispatches_on_real_binary() {
    // The binary auto-builds the cdylib via rustc; skip where it isn't available.
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_use_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    // A plain script: `use` the library and call its functions — NO `#native`, no
    // execution-mode declaration.  double/add/factorial are normal loft functions.
    std::fs::write(
        &prog,
        "use mathnative;\n\
         fn main() {\n\
         \x20   println(\"{double(21)}\");\n\
         \x20   println(\"{add(3, 4)}\");\n\
         \x20   println(\"{factorial(5)}\");\n\
         }\n",
    )
    .unwrap();

    // The library's auto-built cdylib lands in its package's `native-auto/` dir
    // (git-ignored); start clean so its presence afterwards proves the build ran.
    let native_auto = std::path::Path::new("tests/lib/mathnative/native-auto");
    let _ = std::fs::remove_dir_all(native_auto);

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--lib")
        .arg("tests/lib")
        .arg(&prog)
        .env("LOFT_NO_CACHE", "1") // auto-native programs bypass the program cache anyway
        .output()
        .expect("run the loft binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "loft exited non-zero.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // 21*2=42, 3+4=7, 5!=120 — identical to the all-interpreted result.  Because
    // `def.native` is set, the calls compile to `OpStaticCall`; a correct answer
    // means the bridge was wired (an unwired stub would have panicked instead),
    // i.e. the calls dispatched into the auto-built native cdylib.
    assert_eq!(
        stdout, "42\n7\n120\n",
        "auto-native dispatch produced the wrong output"
    );

    // The cdylib was actually built (the native path was taken, not interpreted).
    assert!(
        native_auto.join(cdylib_name()).exists(),
        "expected the auto-built cdylib at {}",
        native_auto.join(cdylib_name()).display()
    );

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}

/// @PLAN54 Arc N / N3 (B3) — silent per-function fallback: a library where one
/// public function is shared-store-dispatchable (`triple`) and another is not
/// (`apply_inc` calls through a function reference — a `CallRef` the gate
/// conservatively excludes).  The gate splits the library silently — `triple`
/// compiles into the cdylib + dispatches native; `apply_inc` stays interpreted —
/// with no user-facing error and the script calling both alike.
///
/// Also verifies the synthetic-exclusion invariant: `apply_inc`'s nested lambda
/// (`__lambda_N`, made `pub_visible` by the enclosing `pub fn`) is NOT a dispatch
/// target — it is a fn-ref target, not script-callable public API — so no
/// `loft_shared_n___lambda` symbol appears in the cdylib.
#[test]
fn mixed_library_dispatches_native_and_interprets_rest() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_mixed_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(
        &prog,
        "use mathmixed;\n\
         fn main() {\n\
         \x20   println(\"{triple(7)}\");\n\
         \x20   println(\"{apply_inc(10)}\");\n\
         }\n",
    )
    .unwrap();

    let native_auto = std::path::Path::new("tests/lib/mathmixed/native-auto");
    let _ = std::fs::remove_dir_all(native_auto);

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--lib")
        .arg("tests/lib")
        .arg(&prog)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run the loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "loft exited non-zero.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // triple(7)=21 (native), apply_inc(10)=11 (interpreted) — identical to all-interp.
    assert_eq!(stdout, "21\n11\n", "mixed-library output");

    // The gate split the library: the cdylib exports the dispatchable `triple` but
    // NOT `apply_inc` (CallRef → interpreted), and NOT its synthetic lambda.
    let lib_rs = std::fs::read_to_string(native_auto.join("loft_auto_mathmixed.rs"))
        .expect("generated cdylib source should exist");
    assert!(
        lib_rs.contains("loft_shared_n_triple"),
        "triple should have a native bridge"
    );
    assert!(
        !lib_rs.contains("loft_shared_n_apply_inc"),
        "apply_inc (CallRef) must NOT be dispatched native — it stays interpreted"
    );
    assert!(
        !lib_rs.contains("__lambda"),
        "a synthetic lambda must NOT be a native dispatch target"
    );

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}
