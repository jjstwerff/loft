// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Running a file with no `fn main()` invokes the auto test-runner. It must:
//   1. run only `test_*()` functions (the `--tests` convention), NOT arbitrary
//      zero-parameter functions, and
//   2. never PANIC — in particular it must not collect a used library's
//      `#native` zero-param host imports (e.g. graphics' `gl_swap_buffers`) and
//      try to execute them, which hit a `def(u32::MAX)` "Unknown definition"
//      panic in `State::execute_argv`.
// A `#native`-declared zero-param void function stands in for that host-import
// shape without needing a real library; the `execute_argv` guard makes even a
// missing entry a clean message, not a crash.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft")
}

#[test]
fn no_main_runs_only_test_fns_and_never_panics() {
    let loft = loft_bin();
    if !loft.exists() {
        eprintln!("SKIP no_main_runs_only_test_fns: target/release/loft not built");
        return;
    }
    let dir = std::env::temp_dir().join("loft_no_main_runner");
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("t.loft");
    // A `#native` zero-param void fn (host-import shape, no body), a non-test
    // helper, and a real test — only `test_ok` should run.
    std::fs::write(
        &src,
        "fn host_thing();\n\
         #native \"loft_test_host_thing\"\n\
         \n\
         fn helper() { print(\"HELPER_RAN\"); }\n\
         fn test_ok() { print(\"TEST_RAN\"); }\n",
    )
    .expect("write source");

    let out = Command::new(&loft)
        .args(["--interpret"])
        .arg(&src)
        .output()
        .expect("invoke loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !stderr.contains("Unknown definition") && !stderr.contains("panicked"),
        "auto test-runner PANICKED (regression: collected a non-runnable fn).\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("TEST_RAN") || stderr.contains("TEST_RAN"),
        "the `test_ok` function did not run.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains("HELPER_RAN") && !stderr.contains("HELPER_RAN"),
        "a non-`test_` function was auto-run (filter too broad).\nstdout: {stdout}"
    );
}
