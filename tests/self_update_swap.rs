// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN78 step 4 — replacing the RUNNING loft, on whatever platform CI is.
//
// This was written down as the one thing no test could cover: `apply_bundle` renames
// the target aside and copies in, because a running executable cannot be overwritten on
// Windows but can be renamed, and the unit tests only ever exercised that on ordinary
// files.  "Needs a published release and a Windows box" was wrong — what it needs is a
// loft that is running from the path being replaced, and a test can arrange that by
// running a COPY of the binary out of a temporary installation.
//
// So the Windows leg of the daily matrix now covers the swap for real, and the same
// test guards the Unix path (where unlinking a running binary is legal but leaves the
// process on the old inode) rather than being skipped there.

#![cfg(feature = "registry")]

use std::path::{Path, PathBuf};

fn exe_name() -> &'static str {
    if cfg!(windows) { "loft.exe" } else { "loft" }
}

fn write(path: &Path, body: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    loft::integrity::sha256_hex(bytes)
}

/// Write `SHA256SUMS` covering every file under `root` except itself — the same shape
/// `make-release.sh` produces, because `apply_bundle` refuses a bundle that fails it.
fn write_manifest(root: &Path) {
    let mut lines = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "SHA256SUMS" {
                continue;
            }
            lines.push(format!(
                "{}  {rel}",
                sha256_hex(&std::fs::read(&p).unwrap())
            ));
        }
    }
    lines.sort();
    std::fs::write(root.join("SHA256SUMS"), lines.join("\n") + "\n").unwrap();
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join("loft-self-update-swap")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Replace the binary that is executing the replacement.
///
/// The staged `bin/loft` is deliberately NOT a working executable — the assertion is
/// about the bytes landing, and using real ones would make a silent no-op (the file
/// never replaced) indistinguishable from success.
#[test]
fn self_update_replaces_the_running_binary() {
    let base = scratch("running");
    let install = base.join("install");
    let staged = base.join("staged");

    // A minimal installation, laid out as a release bundle: <root>/bin + <root>/default.
    let running = install.join("bin").join(exe_name());
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_loft"), &running).unwrap();
    write(&install.join("default").join("a.loft"), b"old\n");
    write_manifest(&install);

    // The bundle to install over it.
    const NEW_BINARY: &[u8] = b"REPLACED-BINARY-CONTENT\n";
    write(&staged.join("bin").join(exe_name()), NEW_BINARY);
    write(&staged.join("default").join("a.loft"), b"new\n");
    write_manifest(&staged);

    // Run the COPY, so the process replacing `bin/loft` is running FROM `bin/loft`.
    let out = std::process::Command::new(&running)
        .args(["self-update", "--from"])
        .arg(&staged)
        .output()
        .expect("run the installed loft");

    assert!(
        out.status.success(),
        "self-update --from failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read(&running).unwrap(),
        NEW_BINARY,
        "the running binary must have been replaced, not skipped"
    );
    assert_eq!(
        std::fs::read(install.join("default").join("a.loft")).unwrap(),
        b"new\n",
        "the stdlib must move with the binary — a new bin/loft beside an old default/ \
         is the partial upgrade `verify-self` exists to catch"
    );
}

/// `--dry-run` must change nothing, including on the running binary.  Without this the
/// test above passes just as well for an implementation that always replaces.
#[test]
fn a_dry_run_leaves_the_running_binary_alone() {
    let base = scratch("dry");
    let install = base.join("install");
    let staged = base.join("staged");

    let running = install.join("bin").join(exe_name());
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_loft"), &running).unwrap();
    write(&install.join("default").join("a.loft"), b"old\n");
    write_manifest(&install);
    let before = std::fs::read(&running).unwrap();

    write(&staged.join("bin").join(exe_name()), b"WOULD-REPLACE\n");
    write(&staged.join("default").join("a.loft"), b"new\n");
    write_manifest(&staged);

    let out = std::process::Command::new(&running)
        .args(["self-update", "--dry-run", "--from"])
        .arg(&staged)
        .output()
        .expect("run the installed loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read(&running).unwrap(),
        before,
        "--dry-run must not write"
    );
    assert_eq!(
        std::fs::read(install.join("default").join("a.loft")).unwrap(),
        b"old\n"
    );
}

/// A bundle that contradicts its own manifest is refused, and nothing moves — the
/// truncated-copy case, which is the failure that actually happens.
#[test]
fn a_bundle_that_fails_its_manifest_replaces_nothing() {
    let base = scratch("corrupt");
    let install = base.join("install");
    let staged = base.join("staged");

    let running = install.join("bin").join(exe_name());
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_loft"), &running).unwrap();
    write(&install.join("default").join("a.loft"), b"old\n");
    write_manifest(&install);
    let before = std::fs::read(&running).unwrap();

    write(&staged.join("bin").join(exe_name()), b"NEW\n");
    write(&staged.join("default").join("a.loft"), b"new\n");
    write_manifest(&staged);
    // Corrupt AFTER the manifest is written, so the bundle disagrees with itself.
    write(&staged.join("default").join("a.loft"), b"tampered\n");

    let out = std::process::Command::new(&running)
        .args(["self-update", "--from"])
        .arg(&staged)
        .output()
        .expect("run the installed loft");
    assert!(
        !out.status.success(),
        "a contradicted manifest must be refused"
    );
    assert_eq!(
        std::fs::read(&running).unwrap(),
        before,
        "a refused bundle must leave the running binary untouched"
    );
}
