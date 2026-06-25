// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN86 — the sandbox, end-to-end through the `loft` binary: a program's
//! `[sandbox]` policy loads from `loft.toml`, a violating sandboxed program is
//! rejected at load with an actionable error, a clean one is admitted and runs, and
//! a clean sandboxed program also runs on `--native` (admission is backend-agnostic —
//! the forced interpret-only was dropped, an admitted script is fault-free on both).

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Write `prog.loft` + `loft.toml` into a fresh temp dir, run the binary (the
/// stdlib is found relative to the workspace root), return `(success, stderr)`.
fn run(name: &str, prog: &str, toml: &str, native: bool) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("loft_sbcli_{}_{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("prog.loft"), prog).unwrap();
    std::fs::write(dir.join("loft.toml"), toml).unwrap();
    let mut cmd = Command::new(loft_bin());
    if native {
        cmd.arg("--native");
    }
    cmd.arg("--timeout")
        .arg("60")
        .arg(dir.join("prog.loft"))
        .current_dir(workspace_root());
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const POLICY: &str = "[sandbox]\nmod = [\"fn:scripted\"]\n[profile.mod]\nallow_libs = [\"code\"]\n";

/// A sandboxed def reaching `mtime` (fs#read) under a policy that grants only
/// `code` is rejected, with the actionable error naming the symbol + group + fix.
#[test]
fn sandboxed_capability_violation_is_rejected() {
    let prog = "fn scripted() -> integer { mtime(\"x\") }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("viol", prog, POLICY, false);
    assert!(!ok, "a violating sandboxed program must be rejected");
    assert!(
        err.contains("admission violation")
            && err.contains("mtime")
            && err.contains("fs#read")
            && err.contains("fix:"),
        "stderr: {err}"
    );
}

/// A clean sandboxed program (only `code`-library arithmetic) is admitted and
/// runs (force-interpreted).
#[test]
fn sandboxed_clean_program_is_admitted() {
    let prog = "fn scripted() -> integer { 21 + 21 }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("clean", prog, POLICY, false);
    assert!(
        ok,
        "a clean sandboxed program must be admitted + run; stderr: {err}"
    );
}

/// A clean sandboxed program runs on `--native` too: admission is backend-agnostic
/// (an admitted script is total + fault-free on both backends), so the forced
/// interpret-only was dropped and `--native` is no longer refused.
#[test]
fn sandboxed_clean_program_runs_on_native() {
    let prog = "fn scripted() -> integer { 21 + 21 }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("native", prog, POLICY, true);
    assert!(
        ok,
        "a clean sandboxed program must be admitted + run on --native; stderr: {err}"
    );
    assert!(
        !err.contains("interpret-only"),
        "--native must no longer be refused for sandboxed code: {err}"
    );
}

/// @PLN86 — the program warm-cache must NOT bypass admission.  A whole-program
/// warm load restores the IR without re-parsing, so `def_sandbox` would never form
/// and admission would be skipped; and the cache is keyed by program CONTENT, not
/// policy.  So a program admitted under a permissive policy,
/// then run UNCHANGED under a tightened policy, must still be re-admitted — and
/// rejected.  Forces the program cache on (`LOFT_PROGRAM_CACHE=1`) with an
/// isolated `LOFT_HOME`, so run 1 writes a warm cache that run 2 must ignore.
#[test]
fn warm_program_cache_does_not_bypass_admission() {
    let dir = std::env::temp_dir().join(format!("loft_sbcache_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let prog = dir.join("prog.loft");
    std::fs::write(
        &prog,
        "fn scripted() -> integer { mtime(\"Cargo.toml\") }\n\
         fn main() -> integer { scripted() }\n",
    )
    .unwrap();

    let run = |toml: &str| -> (bool, String) {
        std::fs::write(dir.join("loft.toml"), toml).unwrap();
        let out = Command::new(loft_bin())
            .env("LOFT_PROGRAM_CACHE", "1")
            .env("LOFT_HOME", &home)
            .arg("--timeout")
            .arg("60")
            .arg(&prog)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    // permissive: fs#read granted → admitted, runs, and WARMS the program cache.
    let permissive = "[sandbox]\nmod = [\"fn:scripted\"]\n\
                      [profile.mod]\nallow_libs = [\"code\"]\nallow = [\"fs#read\"]\n";
    let (ok1, err1) = run(permissive);
    assert!(ok1, "permissive run must be admitted and run: {err1}");

    // tightened (fs#read withdrawn): the SAME program must be rejected even though
    // a warm cache now exists — no bypass.
    let restrictive = "[sandbox]\nmod = [\"fn:scripted\"]\n\
                       [profile.mod]\nallow_libs = [\"code\"]\n";
    let (ok2, err2) = run(restrictive);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !ok2,
        "the tightened policy must reject on the warm run (admission not bypassed): {err2}"
    );
    assert!(
        err2.contains("admission violation") && err2.contains("fs#read"),
        "warm-run rejection must name the capability: {err2}"
    );
}

/// A raw field write to heap data in a sandboxed def is rejected (2.4) — mutation
/// must go through a `*.write` op; construction stays fine.
#[test]
fn sandboxed_raw_write_is_rejected() {
    let prog = "struct Ent { health: integer }\n\
                fn scripted(e: Ent) -> integer { e.health = 0; e.health }\n\
                fn main() -> integer { scripted(Ent { health: 5 }) }\n";
    let (ok, err) = run("rawwrite", prog, POLICY, false);
    assert!(!ok, "a raw write to host data must be rejected");
    assert!(
        err.contains("raw write") && err.contains("fix:"),
        "stderr: {err}"
    );
}
