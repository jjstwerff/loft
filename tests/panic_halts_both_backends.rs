// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `panic("msg")` must halt, report, and exit non-zero — on BOTH backends.
//!
//! It did not.  The native generator emits a real body only for the builtins it
//! special-cases; everything else with no `#rust` template falls through to
//! `fn n_x(..) {}` — an empty stub.  `n_panic` fell through, so on `--native`, which is
//! loft's DEFAULT backend, `panic()` printed nothing, halted nothing, and exited 0, while
//! `--interpret` printed the error and exited 1.  A four-line program was enough to show
//! it, and it silently defeated `loft-libs-net`'s `server::listen`, whose fatal-on-failed-
//! bind is the whole point of the call.
//!
//! Setting `had_fatal` the way the interpreter's `native.rs::n_panic` does is NOT enough
//! here: the generated `main` inspects that flag only after `n_main` returns, by which
//! point the program has run to completion past the panic.  Native has to report and exit
//! at the call site, which is what `RuntimeError::report_and_exit` does.
//!
//! Three separate properties, because each can regress alone and each hid part of the
//! original bug: the process STOPS, it exits NON-ZERO, and it SAYS why.

use std::path::PathBuf;
use std::process::Command;

fn run(backend: &str, src: &str, tag: &str) -> (i32, String, String) {
    let dir = std::env::temp_dir().join(format!("loft_panic_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("p.loft");
    std::fs::write(&path, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args([backend, path.to_str().unwrap()])
        .env("LOFT_TIMEOUT", "120")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const SRC: &str = "fn main() {\n  println(\"before\");\n  panic(\"halt-marker\");\n  \
                   println(\"AFTER-PANIC\");\n}\n";

/// `rustc` is needed for the `--native` leg; skip cleanly where it is absent, like the
/// other native suites.
fn have_rustc() -> bool {
    Command::new("rustc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
        && PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/release")
            .exists()
}

#[test]
fn panic_halts_reports_and_exits_nonzero_on_both_backends() {
    let mut legs = vec![("--interpret", run("--interpret", SRC, "i"))];
    if have_rustc() {
        legs.push(("--native", run("--native", SRC, "n")));
    } else {
        println!("panic_halts...: --native leg skipped (no rustc)");
    }

    for (backend, (code, stdout, stderr)) in &legs {
        // 1. It STOPS.  This is the property that actually matters and the one that was
        //    broken: native ran the whole program, printing the line after the panic.
        assert!(
            !stdout.contains("AFTER-PANIC"),
            "[{backend}] execution continued past panic() — it is not halting.\n\
             stdout: {stdout:?}"
        );
        assert!(
            stdout.contains("before"),
            "[{backend}] the program did not run at all; this test is not measuring what \
             it thinks.\nstdout: {stdout:?}"
        );
        // 2. It exits NON-ZERO.  Native exited 0, so a script or CI step would have read
        //    a fatal panic as success.
        assert_ne!(
            *code, 0,
            "[{backend}] panic() exited 0 — a caller cannot tell it failed.\n\
             stderr: {stderr}"
        );
        // 3. It SAYS why.  An exit code with no message leaves nothing to act on.
        assert!(
            stderr.contains("halt-marker"),
            "[{backend}] the panic message never reached stderr.\nstderr: {stderr}"
        );
    }

    // Parity: the two backends must agree, not merely each be non-vacuous.  The renderer
    // is shared (`RuntimeError::to_diag_entry` + `render_entry_pretty`), so the reported
    // line is identical; comparing it is what pins the sharing in place.
    if legs.len() == 2 {
        let line_of = |s: &str| {
            s.lines()
                .find(|l| l.contains("panic:"))
                .unwrap_or("<no panic line>")
                .to_string()
        };
        let (i_code, _, i_err) = &legs[0].1;
        let (n_code, _, n_err) = &legs[1].1;
        assert_eq!(
            line_of(i_err),
            line_of(n_err),
            "backends render panic() differently"
        );
        assert_eq!(i_code, n_code, "backends disagree on panic()'s exit code");
    }
}

/// `assert` was NOT part of the same defect — it is special-cased in the generator and
/// always halted on native.  Pinned here so a future consolidation of the two builtins
/// cannot quietly regress the one that worked while fixing the one that did not.
#[test]
fn assert_still_halts_on_both_backends() {
    let src = "fn main() {\n  println(\"before\");\n  assert(1 == 2, \"assert-marker\");\n  \
               println(\"AFTER-ASSERT\");\n}\n";
    let mut legs = vec![("--interpret", run("--interpret", src, "ai"))];
    if have_rustc() {
        legs.push(("--native", run("--native", src, "an")));
    }
    for (backend, (code, stdout, stderr)) in &legs {
        assert!(
            !stdout.contains("AFTER-ASSERT"),
            "[{backend}] execution continued past a failed assert\nstdout: {stdout:?}"
        );
        assert_ne!(*code, 0, "[{backend}] failed assert exited 0");
        assert!(
            stderr.contains("assert-marker"),
            "[{backend}] the assert message never reached stderr.\nstderr: {stderr}"
        );
    }
}
