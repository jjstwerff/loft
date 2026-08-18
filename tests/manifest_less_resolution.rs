// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN143 — manifest-less resolution: a program may omit its dependency declaration.
//!
//! A bare script (no `loft.toml`, no sidecar lock) means *the newest release, re-decided
//! every run*. Today the ergonomics already work — `lib_path`'s last probe auto-installs —
//! but a run LEAVES SOMETHING BEHIND that changes what the next run resolves, which is the
//! defect this plan closes.
//!
//! **The invariant:** nothing a program produces by RUNNING may change which version a
//! later run resolves. A version is fixed only by an explicit act (`loft install`,
//! `loft update`, `loft pin`), and that act writes exactly one declaration, beside the
//! thing it governs.
//!
//! Hermetic throughout: `LOFT_HOME` points at a per-test cache and `LOFT_REGISTRY_URL` at
//! a path that cannot be fetched, so every cell measures resolution rather than the
//! network.

#![cfg(feature = "registry")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

/// A private `~/.loft` whose registry cache holds `index.json` for one package, with the
/// signature beside it decided by `sig`.
///
/// The index is written FRESH, so `index_stale` is false and no fetch is attempted — the
/// cached-index branch is the one under test, and it verifies like every other branch.
fn fake_registry(tag: &str, sig: Option<&str>) -> PathBuf {
    let home = std::env::temp_dir().join(format!("loft_pln143_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let cache = home.join(".loft/registry");
    write(
        &cache.join("index.json"),
        r#"{"schema_version":1,"packages":{"probepkg":{"description":"probe",
           "versions":[{"semver":"0.1.0","url":"http://127.0.0.1:1/probepkg-0.1.0.tar.gz",
           "sha256":"0000000000000000000000000000000000000000000000000000000000000000",
           "deps":{}}]}}}"#,
    );
    if let Some(s) = sig {
        write(&cache.join("index.json.sig"), s);
    }
    home
}

/// Run `script` from `dir` against a private registry cache.
fn run_in(home: &Path, dir: &Path, script: &str) -> String {
    let out = Command::new(loft_bin())
        .args(["--interpret", script])
        .env("LOFT_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        // Unreachable: a cell that reached the real registry would not be measuring
        // resolution, and would fail differently on a machine with no network.
        .env("LOFT_REGISTRY_URL", "http://127.0.0.1:1/index.json")
        .env("LOFT_TIMEOUT", "90")
        .current_dir(dir)
        .output()
        .expect("spawn loft");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// @PLN143 arc A — the auto-install path does not waive the index signature.
///
/// `allow_unsigned` was `true` there, mirroring `loft install`. That is defensible while
/// auto-install is a fallback nobody relies on; this plan makes it the DEFAULT path a bare
/// `use` takes, and widening the blast radius while leaving the bootstrap window open must
/// not be two commits in that order.
///
/// An INVALID signature was already a hard failure everywhere — what this pins is the
/// MISSING one, which is the case `allow_unsigned` actually waived.
///
/// ⚠ The assertion keys on the REFUSAL MESSAGE, not on "the run failed". Both
/// configurations fail here — the fixture's tarball is unreachable by construction — so
/// "it did not resolve" cannot tell them apart, and a first version of this test that
/// asserted only that passed with arc A REVERTED. The two were measured:
///
///   with arc A:    `registry index signature is malformed or missing; pass --allow-unsigned`
///   without arc A: `no version of `probepkg` satisfies constraint `*``
///
/// i.e. without the arc the run gets PAST the signature and fails later, on the index's
/// own content. That difference is the whole claim, so it is what is asserted.
#[test]
fn arc_a_auto_install_refuses_an_unsigned_index() {
    let home = fake_registry("unsigned", None);
    let dir = home.join("proj");
    write(
        &dir.join("s.loft"),
        "use probepkg;\nfn main() { println(\"resolved\"); }\n",
    );
    let all = run_in(&home, &dir, "s.loft");
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        all.contains("signature is malformed or missing"),
        "an unsigned index must be REFUSED by the signature gate on the auto-install \
         path — reaching any later failure means the gate was waived (@PLN143 arc A):\n{all}"
    );
    assert!(
        !all.contains("resolved"),
        "and nothing may resolve through it:\n{all}"
    );
}

/// The control that makes the cell above a comparison rather than an anecdote: the same
/// cache with a signature PRESENT must get past the signature gate and fail later, on the
/// index content instead. If this ever starts reporting a signature refusal, the gate has
/// begun rejecting something it was never meant to.
#[test]
fn arc_a_a_signed_index_gets_past_the_signature_gate() {
    // Malformed rather than valid: `verify_or_explain` treats malformed and missing as
    // the same waivable class, so this cell isolates PRESENCE — which is all it needs to
    // show the failure moving.
    let home = fake_registry("signed", Some("not-a-real-signature"));
    let dir = home.join("proj");
    write(
        &dir.join("s.loft"),
        "use probepkg;\nfn main() { println(\"resolved\"); }\n",
    );
    let all = run_in(&home, &dir, "s.loft");
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        !all.contains("resolved"),
        "the fixture's package cannot be downloaded, so nothing should resolve:\n{all}"
    );
}
