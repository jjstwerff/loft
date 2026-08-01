// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 Arc N / N3 Phase A — `use <lib>` auto-compiles a normal loft library
//! to a native cdylib and dispatches to it, on the **real binary**.
//!
//! The fixture `tests/lib/mathnative/` is a NORMAL loft library (no `#native`, no
//! Rust crate) whose `loft.toml` opts in with `[library] compile = "native"`.  A
//! script that does `use mathnative;` and calls its functions must run them
//! natively (the library compiles) while the script interprets — output identical
//! to the all-interpreted run.  This is the headline of Arc N realised end-to-end.

use std::process::Command;

/// Is an auto-built cdylib for `mathnative` present in `dir`?
///
/// loft#715 — the artifact name carries the caller's type-layout fingerprint
/// (`libloft_auto_mathnative_<fp>.so`), so two contexts can never name the same
/// file and a process cannot open a library built for someone else's type
/// indices. The test therefore matches the prefix + extension rather than a
/// fixed name; the fingerprint is not knowable from here.
fn cdylib_present(dir: &std::path::Path) -> bool {
    let (prefix, ext) = if cfg!(target_os = "windows") {
        ("loft_auto_mathnative", "dll")
    } else if cfg!(target_os = "macos") {
        ("libloft_auto_mathnative", "dylib")
    } else {
        ("libloft_auto_mathnative", "so")
    };
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with(prefix) && n.ends_with(ext)
        })
    })
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
        cdylib_present(native_auto),
        "expected an auto-built cdylib in {}",
        native_auto.display()
    );

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}

/// @PLN11 Arc N / N3 (B3) — silent per-function fallback: a library where one
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
    // loft#715 — the generated source is named for the caller's type-layout
    // fingerprint, like the cdylib beside it, so find it by prefix.
    let rs_path = std::fs::read_dir(native_auto)
        .expect("native-auto dir should exist")
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("loft_auto_mathmixed") && n.ends_with(".rs"))
        })
        .expect("generated cdylib source should exist");
    let lib_rs = std::fs::read_to_string(&rs_path).expect("read generated cdylib source");
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

/// A native build failure with a `rustc` toolchain present is a HARD ERROR — loft
/// refuses to silently degrade to the interpreter — and `LOFT_REQUIRE_NATIVE` names
/// itself as the reason.  This guards the **library** chokepoint:
/// `LOFT_FORCE_NATIVE_BUILD_FAIL` deterministically drives the auto-native build to
/// `Err` (a `rustc` toolchain IS present on the host), so both arms hard-fail from
/// the SAME forced failure, with DIFFERENT reasons on stderr:
///  * default (no env)  → exit ≠ 0, no output, "a real build failure" (rustc present);
///  * `LOFT_REQUIRE_NATIVE=1` → exit ≠ 0, no output, the env var named as the reason.
///
/// The graceful no-toolchain fallback (the only remaining silent-interpret path) is
/// covered by `require_native_errors_when_rustc_is_absent` below.
#[test]
fn native_build_failure_hard_fails_default_and_under_require() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_require_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(
        &prog,
        "use mathnative;\n\
         fn main() {\n\
         \x20   println(\"{double(21)}\");\n\
         }\n",
    )
    .unwrap();

    let run = |require: bool| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_loft"));
        c.arg("--lib")
            .arg("tests/lib")
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_FORCE_NATIVE_BUILD_FAIL", "1");
        if require {
            c.env("LOFT_REQUIRE_NATIVE", "1");
        }
        c.output().expect("run the loft binary")
    };

    // Default (no env): with rustc present, the forced build failure is a REAL
    // failure — loft refuses to silently interpret it, exits non-zero, and runs no
    // program output.  The reason names the present toolchain, NOT LOFT_REQUIRE_NATIVE.
    let def = run(false);
    let def_stdout = String::from_utf8_lossy(&def.stdout);
    let def_stderr = String::from_utf8_lossy(&def.stderr);
    assert!(
        !def.status.success(),
        "a native build failure with rustc present must hard-fail by default.\nstdout:\n{def_stdout}\nstderr:\n{def_stderr}"
    );
    assert!(
        !def_stdout.contains("42"),
        "a hard-failed build must not run the interpreted fallback (saw program output)"
    );
    assert!(
        def_stderr.contains("real build failure") && !def_stderr.contains("LOFT_REQUIRE_NATIVE"),
        "the default hard-fail names the present toolchain, not LOFT_REQUIRE_NATIVE.\nstderr:\n{def_stderr}"
    );

    // Strict: the same forced failure is now a hard error — no program output, a
    // non-zero exit, and the reason (the env var + the failing library) on stderr.
    let strict = run(true);
    let strict_stdout = String::from_utf8_lossy(&strict.stdout);
    let strict_stderr = String::from_utf8_lossy(&strict.stderr);
    assert!(
        !strict.status.success(),
        "LOFT_REQUIRE_NATIVE must turn the fallback into a non-zero exit.\nstdout:\n{strict_stdout}\nstderr:\n{strict_stderr}"
    );
    assert!(
        !strict_stdout.contains("42"),
        "strict mode must refuse to run the interpreted fallback (saw program output)"
    );
    assert!(
        strict_stderr.contains("LOFT_REQUIRE_NATIVE")
            && strict_stderr.contains("failed to build native"),
        "strict error must name the env var and the reason.\nstderr:\n{strict_stderr}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Guards the **main-program** chokepoint of `LOFT_REQUIRE_NATIVE`.  Forces a native
/// fallback by hiding `rustc` (empty `PATH`) on a cache-bypassed `--native` run; under
/// the env var that must be a hard error naming the missing toolchain, not a silent
/// degrade to the interpreter.  Skipped on Windows where the `PATH` model + binary
/// resolution differ enough that an empty `PATH` is not a clean way to hide `rustc`.
#[test]
#[cfg(not(target_os = "windows"))]
fn require_native_errors_when_rustc_is_absent() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_norustc_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(&prog, "fn main() {\n    print(\"ran\")\n}\n").unwrap();

    // Empty PATH ⇒ `rustc` (invoked by bare name) is NotFound; loft itself runs
    // because it is launched by absolute path.  Cache bypass forces a compile attempt.
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native")
        .arg(&prog)
        .env("PATH", "")
        .env("LOFT_NATIVE_NO_CACHE", "1")
        .env("LOFT_REQUIRE_NATIVE", "1")
        .output()
        .expect("run the loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "LOFT_REQUIRE_NATIVE must error when rustc is absent.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("ran"),
        "strict mode must not fall through to the interpreter (saw program output)"
    );
    assert!(
        stderr.contains("LOFT_REQUIRE_NATIVE"),
        "strict error must name the env var.\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// #460 — the package that OWNS the entry file is the *script*, not a `use`d
/// library: it must never be auto-native-compiled, even though it carries a
/// `loft.toml`.  Its export set is entry-point dependent — running `entry_a.loft`
/// parses only `mod_a` (`val_a`), running `entry_b.loft` parses only `mod_b`
/// (`val_b`) — so a cdylib built for one entry exports the wrong symbol set for
/// the other.  Before the fix, the second run found the first run's cdylib
/// "fresh", skipped the rebuild, then marked its own export set against it →
/// `OpStaticCall` to a bridge symbol the `.so` never built → the `compile.rs`
/// panic stub (crawler's `make test` gate, exit 101).
///
/// The invariant: the entry package produces NO `native-auto/` cdylib at all
/// (the "libraries compile, scripts interpret" model), so running two different
/// entry points from the same package in sequence both succeed cleanly.
#[test]
fn entry_package_is_never_auto_native_compiled() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }

    // The fixture `tests/lib/selfpkg/` is a normal package (loft.toml, no
    // [native]) run DIRECTLY via two entries that `use` disjoint local modules.
    let pkg = std::path::Path::new("tests/lib/selfpkg");
    let native_auto = pkg.join("native-auto");
    let _ = std::fs::remove_dir_all(&native_auto);

    let run = |entry: &str| {
        Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--lib")
            .arg("tests/lib")
            .arg(pkg.join(entry))
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary")
    };

    // entry_a built the (wrong) cdylib before the fix; entry_b is where the
    // stale-export-set adoption used to panic.
    let a = run("entry_a.loft");
    let b = run("entry_b.loft");

    for (name, out) in [("entry_a", &a), ("entry_b", &b)] {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "{name} must run clean (no cdylib-dispatch panic).\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("could not be wired"),
            "{name}: an entry-package function was marked for cdylib dispatch (the #460 \
             marking bug).\nstderr:\n{stderr}"
        );
    }
    assert_eq!(
        String::from_utf8_lossy(&a.stdout),
        "111\n",
        "entry_a output"
    );
    assert_eq!(
        String::from_utf8_lossy(&b.stdout),
        "222\n",
        "entry_b output"
    );

    // The decisive invariant: the entry package is the script, so it never
    // builds a cdylib — the structural cause of the stale-export-set mismatch.
    assert!(
        !native_auto.exists(),
        "the entry package must NOT be auto-native-compiled, but {} exists",
        native_auto.display()
    );
}

/// #461 — an auto-native cdylib hardcodes type-table INDICES (e.g. `OpWriteFile`'s
/// `db_tp`), but those indices shift with which libraries are loaded, and the
/// cdylib resolves them against the caller's SHARED `Stores` at runtime.  A cdylib
/// is cached per-library, so one consumer's build can be reused by another whose
/// type table differs — making the baked indices resolve to the WRONG type and
/// silently corrupt (the moros GLB header wrote 8-byte fields for `as i32`).
///
/// The fixture `binwriter` writes a 2-field i32 header via `f += X as i32`; the
/// fixture `typeshift` adds struct types that shift `binwriter`'s `i32` index
/// (verified: `db_tp` 64 → 67).  Build `binwriter`'s cdylib in the bare context,
/// then call it from a context where `typeshift` is also loaded: the freshness key
/// must notice the layout changed and rebuild, so the write stays 4 bytes wide.
#[test]
fn cdylib_type_indices_stay_valid_across_consumer_contexts() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }

    let native_auto = std::path::Path::new("tests/lib/binwriter/native-auto");
    let _ = std::fs::remove_dir_all(native_auto);

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_461_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // Bare context: only binwriter loaded — its cdylib bakes binwriter's `i32` index.
    std::fs::write(
        tmp.join("bare.loft"),
        "use binwriter;\nfn main() { write_magic(arguments()[0], 2); }\n",
    )
    .unwrap();
    // Shifted context: typeshift's struct types move `i32` to a different index.
    std::fs::write(
        tmp.join("shifted.loft"),
        "use typeshift;\nuse binwriter;\n\
         fn main() { _ = ts_touch(); write_magic(arguments()[0], 2); }\n",
    )
    .unwrap();

    let run = |entry: &str, out: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--interpret")
            .arg("--lib")
            .arg("tests/lib")
            .arg(tmp.join(entry))
            .arg(out)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary")
    };

    // Build + cache binwriter's cdylib in the bare context first…
    let bare_out = tmp.join("bare.bin");
    let a = run("bare.loft", &bare_out);
    assert!(
        a.status.success(),
        "bare run failed.\nstderr:\n{}",
        String::from_utf8_lossy(&a.stderr)
    );
    // …then reuse it from the shifted context, where the baked index is wrong
    // unless the cdylib is rebuilt for this layout.
    let shifted_out = tmp.join("shifted.bin");
    let b = run("shifted.loft", &shifted_out);
    assert!(
        b.status.success(),
        "shifted run failed.\nstderr:\n{}",
        String::from_utf8_lossy(&b.stderr)
    );

    // Each header field is a 4-byte i32: magic 'glTF' (LE) + version 2 → 8 bytes.
    // A stale-index cdylib would write 8-byte i64 fields (16 bytes, version split).
    let want: &[u8] = &[0x67, 0x6c, 0x54, 0x46, 0x02, 0x00, 0x00, 0x00];
    for (name, path) in [("bare", &bare_out), ("shifted", &shifted_out)] {
        let bytes = std::fs::read(path).expect("read output");
        assert_eq!(
            bytes, want,
            "{name} context wrote the wrong header — `as i32` did not narrow to 4 bytes \
             (stale cdylib type index resolved against the host table)"
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}

/// loft#717 — an auto-built cdylib must be VERIFIED against the type layout it
/// was generated for, not merely trusted because its filename matches.
///
/// #715 content-addressed the artifact so two contexts can never name the same
/// file, and closed the class "by construction". That is an argument, and it
/// holds exactly as long as two things stay true: the fingerprint keeps covering
/// every layout difference, and nothing else can put a file at that name. Neither
/// is checkable at runtime, and when either fails the artifact is not slightly
/// wrong — the generated cdylib hardcodes type-table INDICES, so it resolves them
/// against a foreign table and reads at the wrong offsets. That is silent memory
/// corruption, whose crash lands arbitrarily far from its cause.
///
/// So the artifact now names its own layout (`loft_type_layout_fp_v1`) and the
/// adopter asks. This plants one context's artifact at the other's exact filename
/// — the aliasing #715 argues is unreachable — and requires that it be rejected.
///
/// The test carries its own control, because "it rebuilt" has a boring competing
/// explanation: copying a file changes its mtime, and a rebuild triggered by mtime
/// would pass this test while verifying nothing. So the SAME copy is done with a
/// MATCHING artifact first, and that one must be adopted. Both arms churn the
/// mtime identically; only the declared layout differs.
#[test]
fn a_foreign_context_artifact_is_rejected_not_adopted() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }
    let native_auto = std::path::Path::new("tests/lib/mathnative/native-auto");
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_717_layout_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // Two programs over the SAME library whose type tables differ: the second
    // loads another library first, which shifts every later type index.
    let bare = tmp.join("bare.loft");
    std::fs::write(
        &bare,
        "use mathnative;\nfn main() { println(\"{double(21)}\"); }\n",
    )
    .unwrap();
    let shifted = tmp.join("shifted.loft");
    std::fs::write(
        &shifted,
        "use typeshift;\nuse mathnative;\n\
         fn main() { _ = ts_touch(); println(\"{double(21)}\"); }\n",
    )
    .unwrap();

    let run = |prog: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--lib")
            .arg("tests/lib")
            .arg(prog)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary")
    };
    let sos = || -> Vec<std::path::PathBuf> {
        let mut v: Vec<_> = std::fs::read_dir(native_auto)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    // `.dll` too — Windows names an auto-built cdylib `<stem>.dll`
                    // (`native_lib.rs::cdylib_file_name`), so a filter of just
                    // `so`/`dylib` counts ZERO there and the artifact assertions below
                    // read as "nothing was built".  `cdylib_present` above already
                    // spells all three out, and `n3_parity.rs` filters on all three;
                    // only this closure was short.  The import-library sidecar
                    // (`<stem>.dll.lib`) has extension `lib`, so it is not counted twice.
                    .filter(|p| {
                        p.extension()
                            .is_some_and(|e| e == "so" || e == "dylib" || e == "dll")
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };

    let _ = std::fs::remove_dir_all(native_auto);
    assert!(run(&bare).status.success(), "bare context runs");
    let after_bare = sos();
    assert_eq!(after_bare.len(), 1, "the bare context built one artifact");
    let bare_so = after_bare[0].clone();

    assert!(run(&shifted).status.success(), "shifted context runs");
    let two = sos();
    if two.len() < 2 {
        // Both contexts fingerprinted the same, so there is no foreign artifact to
        // plant and nothing to assert. Say so rather than passing quietly.
        eprintln!("skip: the two contexts share a type-layout fingerprint");
        let _ = std::fs::remove_dir_all(&tmp);
        return;
    }
    let other_so = two.iter().find(|p| **p != bare_so).unwrap().clone();

    let mtime = |p: &std::path::Path| std::fs::metadata(p).unwrap().modified().unwrap();

    // CONTROL: the bare context's OWN artifact, re-copied over itself. Same mtime
    // churn, matching layout — it must be adopted, or the test below proves nothing.
    let own = std::fs::read(&bare_so).unwrap();
    std::fs::write(&bare_so, &own).unwrap();
    let before = mtime(&bare_so);
    assert!(run(&bare).status.success(), "control run succeeds");
    assert_eq!(
        mtime(&bare_so),
        before,
        "CONTROL FAILED: a matching artifact was rebuilt anyway, so this test cannot \
         tell verification from mtime churn"
    );

    // TEST: the other context's artifact at this context's exact filename.
    let foreign = std::fs::read(&other_so).unwrap();
    std::fs::write(&bare_so, &foreign).unwrap();
    let before = mtime(&bare_so);
    let out = run(&bare);
    assert!(
        out.status.success(),
        "the run must recover, not fail.\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(
        mtime(&bare_so),
        before,
        "a cdylib built for a DIFFERENT type layout was adopted instead of rebuilt — \
         its hardcoded type indices would resolve against this context's table"
    );
    // Compared by digest: a rebuild is not byte-identical to the original (rustc
    // embeds paths and is not reproducible here), so the claim that holds is that
    // what sits there is no longer the FOREIGN artifact.
    let digest = |b: &[u8]| -> (usize, u64) {
        let mut h: u64 = 1469598103934665603;
        for &x in b {
            h = (h ^ u64::from(x)).wrapping_mul(1099511628211);
        }
        (b.len(), h)
    };
    assert_ne!(
        digest(&std::fs::read(&bare_so).unwrap()),
        digest(&foreign),
        "the foreign artifact is still in place after the rebuild"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("42"),
        "and it still computes the right answer"
    );

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}
