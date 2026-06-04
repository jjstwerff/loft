// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 Arc N / N3 Step 1 — the **parity instrument** (the gate for making the
//! native/interpret choice invisible).
//!
//! *Invariant:* a library run native is **byte-identical** to run interpreted.
//!
//! This harness runs the same program three ways and diffs stdout against the
//! interpreted reference:
//! - **interp** — `LOFT_NO_NATIVE_LIBS=1` forces every `use`d library to interpret;
//! - **mixed** — the default: the script interprets, the `compile = "native"`
//!   library auto-compiles + dispatches over the shared store (the new N3 path);
//! - **native** — `--native` compiles the whole program (script + library).
//!
//! A divergence is a real shared-store-ABI / codegen bug caught **before** the
//! choice becomes invisible — which is exactly what makes default-native (Step 3)
//! a proven flip rather than a hopeful one.  Seeded with `datalib`, whose public
//! API carries the store-touching types (vector/struct/text/enum, args + returns)
//! across the boundary.

use std::path::Path;
use std::process::Command;

struct Run {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Run the loft binary on `prog` with extra `args` + `env`, capturing output.
fn run(args: &[&str], env: &[(&str, &str)], prog: &Path) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("--lib")
        .arg("tests/lib")
        .args(args)
        .arg(prog)
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft binary");
    Run {
        success: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Assert the three modes all succeed and produce byte-identical stdout, against
/// the interpreted reference.  Returns the (shared) stdout for value checks.
fn assert_three_mode_parity(prog: &Path) -> String {
    let interp = run(&[], &[("LOFT_NO_NATIVE_LIBS", "1")], prog);
    let mixed = run(&[], &[], prog);
    let native = run(&["--native"], &[], prog);

    for (name, r) in [("interp", &interp), ("mixed", &mixed), ("native", &native)] {
        assert!(
            r.success,
            "{name} mode failed.\nstdout:\n{}\nstderr:\n{}",
            r.stdout, r.stderr
        );
    }
    assert_eq!(
        mixed.stdout, interp.stdout,
        "PARITY DIVERGENCE: mixed (auto-native lib) != interpreted"
    );
    assert_eq!(
        native.stdout, interp.stdout,
        "PARITY DIVERGENCE: --native != interpreted"
    );
    interp.stdout
}

#[test]
fn datalib_store_touching_types_parity() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (mixed + native modes need it)");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_parity_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    // Exercises every store-touching boundary type in BOTH construction directions:
    // a `pub` library type is constructed by NATIVE code and returned (the factories
    // `make_point`/`make_circle`), AND constructed by the INTERPRETER and passed
    // into a native function (`Point {...}` / `Circle {...}`).  Both must round-trip
    // byte-identically.  (Vectors/text exercise the same both ways.)
    std::fs::write(
        &prog,
        "use datalib;\n\
         fn main() {\n\
         \x20   a = vec_sum([10, 20, 30]);\n\
         \x20   println(\"{a}\");\n\
         \x20   v = range_vec(4);\n\
         \x20   println(\"{vec_sum(v)}\");\n\
         \x20   p = make_point(3, 4);\n\
         \x20   println(\"{point_sum(p)}\");\n\
         \x20   pd = Point { x: 7, y: 8 };\n\
         \x20   println(\"{point_sum(pd)}\");\n\
         \x20   println(\"{shout(\"hi\")}\");\n\
         \x20   c = make_circle(2);\n\
         \x20   println(\"{area(c)}\");\n\
         \x20   cd = Circle { r: 3 };\n\
         \x20   println(\"{area(cd)}\");\n\
         }\n",
    )
    .unwrap();

    // The library's auto-built cdylib lands in its package's git-ignored dir.
    let native_auto = std::path::Path::new("tests/lib/datalib/native-auto");
    let _ = std::fs::remove_dir_all(native_auto);

    let stdout = assert_three_mode_parity(&prog);
    // Sanity-anchor the reference so a "parity holds but all three are wrong"
    // regression is also caught.
    assert_eq!(stdout, "60\n6\n7\n15\nhi!\n12\n27\n", "reference output");

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}
