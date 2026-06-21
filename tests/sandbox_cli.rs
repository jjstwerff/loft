// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN86 — the sandbox, end-to-end through the `loft` binary: a program's
//! `[sandbox]` policy loads from `loft.toml`, a violating sandboxed program is
//! rejected at load with an actionable error, a clean one is admitted and runs
//! (force-interpreted), and an explicit `--native` on sandboxed code is refused.

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

/// A sandboxed def reaching `mtime` (fs.read) under a policy that grants only
/// `code` is rejected, with the actionable error naming the symbol + group + fix.
#[test]
fn sandboxed_capability_violation_is_rejected() {
    let prog = "fn scripted() -> integer { mtime(\"x\") }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("viol", prog, POLICY, false);
    assert!(!ok, "a violating sandboxed program must be rejected");
    assert!(
        err.contains("admission violation")
            && err.contains("mtime")
            && err.contains("fs.read")
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

/// An explicit `--native` on a program that designates sandboxed code is refused
/// (sandboxed code is interpret-only: native codegen is RCE + traps).
#[test]
fn sandboxed_native_is_refused() {
    let prog = "fn scripted() -> integer { 21 + 21 }\nfn main() -> integer { scripted() }\n";
    let (ok, err) = run("native", prog, POLICY, true);
    assert!(!ok, "--native on sandboxed code must be refused");
    assert!(err.contains("interpret-only"), "stderr: {err}");
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
