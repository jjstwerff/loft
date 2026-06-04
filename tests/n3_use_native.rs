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
