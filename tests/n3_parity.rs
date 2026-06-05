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
//! - **mixed** — the default (Step 3, default-native): the script interprets, every
//!   `use`d library auto-compiles + dispatches over the shared store — no opt-in;
//! - **native** — `--native` compiles the whole program (script + library).
//!
//! A divergence is a real shared-store-ABI / codegen bug caught **before** the
//! choice becomes invisible — which is exactly what made the default-native flip
//! (Step 3) a proven move rather than a hopeful one.  Seeded with `datalib`, whose
//! public API carries the store-touching types (vector/struct/text/enum, args +
//! returns) across the boundary.

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

/// A program exercising every store-touching boundary type in BOTH construction
/// directions: a `pub` library type constructed by NATIVE code and returned (the
/// factories `make_point`/`make_circle`), AND constructed by the INTERPRETER and
/// passed into a native function (`Point {...}` / `Circle {...}`).  Both must
/// round-trip byte-identically; vectors/text exercise the same both ways.
const DATALIB_PROG: &str = "use datalib;\n\
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
     }\n";

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
    std::fs::write(&prog, DATALIB_PROG).unwrap();

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

/// @PLAN54 Arc N / N3 Step 2 — the build-failure fallback.
///
/// *Invariant:* a library that can't compile native **silently interprets** —
/// byte-identical, no `exit`, no `OpStaticCall` to an unbuilt symbol.
///
/// `LOFT_FORCE_NATIVE_BUILD_FAIL=1` simulates a cdylib build failure (e.g. a
/// codegen gap the gate didn't catch).  The program must still run, exit 0, and
/// produce output **byte-identical** to the pure-interpreted reference — proving
/// the build-before-mark flow never marks (so never dispatches) on failure.  No
/// `rustc` is needed: this path deliberately skips the build.
#[test]
fn build_failure_silently_interprets() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_failparity_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(&prog, DATALIB_PROG).unwrap();
    let native_auto = std::path::Path::new("tests/lib/datalib/native-auto");
    let _ = std::fs::remove_dir_all(native_auto);

    let interp = run(&[], &[("LOFT_NO_NATIVE_LIBS", "1")], &prog);
    let forced = run(&[], &[("LOFT_FORCE_NATIVE_BUILD_FAIL", "1")], &prog);

    assert!(
        forced.success,
        "a build failure must NOT exit non-zero — it must silently interpret.\nstderr:\n{}",
        forced.stderr
    );
    assert_eq!(
        forced.stdout, interp.stdout,
        "a build failure must interpret byte-identically to the interpreted reference"
    );
    assert!(
        !native_auto.exists(),
        "no cdylib should have been built when the build is forced to fail"
    );

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}

/// A consumer of `plainlib` — a library that does **not** opt into
/// `compile = "native"`.  Its API is store-touching (text + vector) in both
/// directions, so a shared-store-ABI bug on the default-native path shows up.
const PLAINLIB_PROG: &str = "use plainlib;\n\
     fn main() {\n\
     \x20   println(banner(\"hi\"));\n\
     \x20   println(\"{doubled([1, 2, 3])}\");\n\
     }\n";

/// @PLAN54 Arc N / N3 Step 3 — **default-native**, proven on a library that never
/// opted in.
///
/// *Invariant:* with no annotation and no flag, a `use`d library is a native
/// candidate — it auto-builds a cdylib and dispatches, byte-identical to
/// interpreting it.  `LOFT_NO_NATIVE_LIBS=1` is the interpret escape.
///
/// Asserts both directions of the now-default flip: by DEFAULT `plainlib` (which
/// has no `compile = "native"`) auto-builds + dispatches native; under
/// `LOFT_NO_NATIVE_LIBS` it interprets and builds no cdylib.
#[test]
fn default_native_dispatches_unopted_library() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (default-native build needs it)");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_plainparity_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(&prog, PLAINLIB_PROG).unwrap();
    let native_auto = std::path::Path::new("tests/lib/plainlib/native-auto");

    let interp = run(&[], &[("LOFT_NO_NATIVE_LIBS", "1")], &prog);

    // ESCAPE: `LOFT_NO_NATIVE_LIBS` interprets and builds no cdylib.
    let _ = std::fs::remove_dir_all(native_auto);
    let escaped = run(&[], &[("LOFT_NO_NATIVE_LIBS", "1")], &prog);
    assert!(
        escaped.success,
        "interpret-escape run failed:\n{}",
        escaped.stderr
    );
    assert!(
        !native_auto.exists(),
        "LOFT_NO_NATIVE_LIBS must build no cdylib"
    );

    // DEFAULT: plainlib never opted in, yet default-native auto-builds + dispatches.
    let _ = std::fs::remove_dir_all(native_auto);
    let dflt = run(&[], &[], &prog);
    assert!(
        dflt.success,
        "default-native run failed:\nstdout:\n{}\nstderr:\n{}",
        dflt.stdout, dflt.stderr
    );
    assert_eq!(
        dflt.stdout, interp.stdout,
        "PARITY DIVERGENCE: default-native (un-opted lib) != interpreted"
    );
    assert!(
        native_auto.exists(),
        "default-native must auto-build the cdylib for an un-opted-in library"
    );

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}

/// The single auto-built cdylib under a package's `native-auto/`, if any.
fn single_cdylib(native_auto: &Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(native_auto)
        .ok()?
        .flatten()
        .find_map(|e| {
            let p = e.path();
            let is_lib = p
                .extension()
                .is_some_and(|x| x == "so" || x == "dylib" || x == "dll");
            is_lib.then_some(p)
        })
}

/// @PLAN54 N3 Step 4 — **dev-interpret-on-edit**.
///
/// *Invariant:* a library's first use builds eagerly (native), but **editing** it
/// makes the next run **interpret** the new code with **no `rustc`** (instant loop);
/// once editing settles, a run rebuilds and it is native again.
///
/// Proven on a writable copy of a library across three runs:
/// 1. first run eager-builds the cdylib and dispatches native (`v1`);
/// 2. after an edit, the next run prints the NEW code (`v2`) — so it interpreted,
///    not dispatched the stale cdylib — and the cdylib file is **untouched** (no
///    rebuild, no `rustc`);
/// 3. with the source now unchanged since run 2, the next run **rebuilds** (the
///    cdylib mtime changes) and is native again.
#[test]
fn editing_a_library_interprets_then_rebuilds_when_stable() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (Step 4 needs the native build)");
        return;
    }

    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_n3_edit_{pid}"));
    let _ = std::fs::remove_dir_all(&root);
    let libdir = root.join("lib");
    let pkg = libdir.join("edlib");
    std::fs::create_dir_all(pkg.join("src")).unwrap();
    std::fs::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"edlib\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
         [library]\nentry = \"src/edlib.loft\"\n",
    )
    .unwrap();
    let src = pkg.join("src").join("edlib.loft");
    std::fs::write(&src, "pub fn greet() -> text { \"v1\" }\n").unwrap();
    let prog = root.join("main.loft");
    std::fs::write(&prog, "use edlib;\nfn main() { println(greet()); }\n").unwrap();
    let native_auto = pkg.join("native-auto");

    // Run the binary against this editable lib dir (no `tests/lib`).
    let run_edit = || -> Run {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("spawn loft binary");
        Run {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    };

    // 1. First use → eager build → native.
    let r1 = run_edit();
    assert!(r1.success, "run 1 failed:\n{}", r1.stderr);
    assert_eq!(r1.stdout.trim(), "v1");
    let so = single_cdylib(&native_auto).expect("run 1 must eager-build the cdylib");
    let mtime1 = std::fs::metadata(&so).unwrap().modified().unwrap();

    // Edit the library (sleep first so the new source mtime is unambiguously newer
    // than the just-built cdylib, even on a coarse-granularity filesystem).
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&src, "pub fn greet() -> text { \"v2\" }\n").unwrap();

    // 2. Edit run → interpret the NEW code, NO rebuild.
    let r2 = run_edit();
    assert!(r2.success, "run 2 failed:\n{}", r2.stderr);
    assert_eq!(
        r2.stdout.trim(),
        "v2",
        "an edit must take effect immediately — interpreted, not the stale cdylib"
    );
    let mtime2 = std::fs::metadata(&so).unwrap().modified().unwrap();
    assert_eq!(
        mtime1, mtime2,
        "the edit run must NOT rebuild the cdylib (no `rustc` per save)"
    );

    // 3. Source unchanged since run 2 → rebuild → native again.
    let r3 = run_edit();
    assert!(r3.success, "run 3 failed:\n{}", r3.stderr);
    assert_eq!(r3.stdout.trim(), "v2");
    let mtime3 = std::fs::metadata(&so).unwrap().modified().unwrap();
    assert_ne!(
        mtime2, mtime3,
        "once editing settles, the next run rebuilds the cdylib → native"
    );

    let _ = std::fs::remove_dir_all(&root);
}
