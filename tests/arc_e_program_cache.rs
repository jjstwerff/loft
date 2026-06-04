// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 arc E — the opt-in whole-program startup cache, end-to-end.
//!
//! On `LOFT_PROGRAM_CACHE` a cold run caches the ENTIRE parsed program (stdlib +
//! the script's lazily-loaded libs + user file) keyed on the script path, and a
//! warm run mmaps it and skips ALL parsing.  A drift manifest of every parsed
//! source's content hash invalidates the bundle whenever any input changes.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run the binary on `script`, optionally with the whole-program cache enabled
/// at `cache_dir` (`XDG_CACHE_HOME`).  Returns `(success, stdout)`.
fn run(script: &std::path::Path, cache_dir: Option<&std::path::Path>) -> (bool, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(script)
        .current_dir(workspace_root())
        .env_remove("LOFT_STDLIB_CACHE");
    if let Some(dir) = cache_dir {
        cmd.env("LOFT_PROGRAM_CACHE", "1")
            .env("XDG_CACHE_HOME", dir);
    } else {
        cmd.env_remove("LOFT_PROGRAM_CACHE");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn program_cache_cold_warm_then_drift() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_arce_{pid}.loft"));
    let write = |body: &str| std::fs::write(&script, body).expect("write script");
    write("fn main() {\n  v = [5, 10, 15];\n  print(\"sum={v[0]+v[1]+v[2]}\\n\");\n}\n");
    let cache_dir = tmp.join(format!("loft_arce_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    // 1. Cache off.
    let (ok_off, out_off) = run(&script, None);
    assert!(ok_off, "cache-off run failed: {out_off}");
    assert!(out_off.contains("sum=30"), "off output: {out_off}");

    // 2. Cold — parses then writes a whole-program bundle + manifest.
    let (ok_cold, out_cold) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold run failed: {out_cold}");
    let dir = cache_dir.join("loft");
    let has = |ext: &str| {
        std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some(ext))
    };
    assert!(
        has("store") && has("manifest"),
        "cold run must write bundle + manifest"
    );

    // 3. Warm — mmaps the whole program, skips all parsing.
    let (ok_warm, out_warm) = run(&script, Some(&cache_dir));
    assert!(ok_warm, "warm run failed: {out_warm}");
    assert_eq!(out_off, out_cold, "cold differs from off");
    assert_eq!(out_off, out_warm, "warm differs from off");

    // 4. Drift — edit the script; the manifest hash mismatches → reparse, new output.
    write("fn main() {\n  v = [5, 10, 100];\n  print(\"sum={v[0]+v[1]+v[2]}\\n\");\n}\n");
    let (ok_drift, out_drift) = run(&script, Some(&cache_dir));
    assert!(ok_drift, "drift run failed: {out_drift}");
    assert!(
        out_drift.contains("sum=115"),
        "drift must reparse → 115, got: {out_drift}"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// @PLAN54 G2/M6 — the manifest's build-signature line invalidates a stale
/// bundle on a (simulated) binary upgrade.  Without it, a bundle written by an
/// older build would be warm-loaded by a newer one whose store layout/codegen
/// differs — a silent stale-load.  We simulate the upgrade by rewriting the
/// manifest's `sig ` line; the next run must reparse (cache miss), not load the
/// stale bundle, and still produce correct output.
#[test]
fn manifest_build_signature_invalidates_stale_bundle() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_sig_{pid}.loft"));
    std::fs::write(&script, "fn main() { print(\"answer={6*7}\\n\"); }\n").expect("write script");
    let cache_dir = tmp.join(format!("loft_sig_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    // Cold run primes the bundle + a manifest whose first line is `sig <build>`.
    let (ok_cold, _) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold run failed");
    let dir = cache_dir.join("loft");
    let manifest = std::fs::read_dir(&dir)
        .expect("cache dir")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("manifest"))
        .expect("manifest written");
    let text = std::fs::read_to_string(&manifest).expect("read manifest");
    assert!(
        text.starts_with("sig "),
        "manifest must start with the sig line: {text:?}"
    );

    // Simulate a binary upgrade: clobber the sig line with a different build.
    let body: String = text.lines().skip(1).map(|l| format!("{l}\n")).collect();
    std::fs::write(&manifest, format!("sig STALE-OTHER-BUILD\n{body}")).expect("tamper manifest");

    // Next run must treat it as a cache miss (reparse) and still be correct —
    // NOT warm-load the now-"foreign" bundle.
    let (ok, out) = run(&script, Some(&cache_dir));
    assert!(ok, "run after sig mismatch failed: {out}");
    assert!(
        out.contains("answer=42"),
        "must reparse to correct output: {out}"
    );

    // And it should have re-saved a manifest with THIS build's real signature.
    let restored = std::fs::read_to_string(&manifest).expect("read manifest");
    assert!(
        !restored.contains("STALE-OTHER-BUILD"),
        "a cold reparse should overwrite the stale-sig manifest"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}
