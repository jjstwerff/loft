// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#966 — `loft install` files a package under the name its manifest declares.
//!
//! It used the checkout DIRECTORY's name, so a package whose directory differs from
//! `[package] name` landed in `~/.loft/lib/<directory>` — a name no `use` can reach.
//! `loft api` went on reporting the dependency unresolved, and the copy sat there
//! shadowing nothing useful.
//!
//! ⚠ `~/.loft/lib/<name>` is searched BEFORE the registry cache, so a copy under the
//! RIGHT name is not harmless either — that is loft#667, where a locally installed `web`
//! lost its `wasm/` bridge and shadowed a good published one.  These tests therefore run
//! with `HOME` pointed at a temp directory: a suite that installed into the developer's
//! real `~/.loft/lib` would be re-arming that trap on every run.

use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

/// Build a package in a directory deliberately named something other than the manifest's
/// `[package] name`, run `loft install` in it against a private `HOME`, and return
/// (stdout+stderr, that HOME).
fn install_in_a_mismatched_directory(tag: &str) -> (String, PathBuf) {
    let base = std::env::temp_dir().join(format!("loft_966_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let pkg = base.join("checkout_dir_name");
    std::fs::create_dir_all(&home).expect("mkdir home");
    write(
        &pkg.join("loft.toml"),
        "[package]\nname    = \"instprobe\"\nversion = \"0.1.0\"\nloft    = \">=0.8\"\n\n\
         [library]\nentry = \"src/instprobe.loft\"\n",
    );
    write(
        &pkg.join("src/instprobe.loft"),
        "pub fn hi() -> integer { 1 }\n",
    );

    let out = Command::new(loft_bin())
        .arg("install")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("LOFT_TIMEOUT", "90")
        .current_dir(&pkg)
        .output()
        .expect("spawn loft install");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        home,
    )
}

/// The reported case: manifest name `instprobe`, directory `checkout_dir_name`.
#[test]
fn a_local_install_is_named_by_the_manifest_not_the_directory() {
    let (all, home) = install_in_a_mismatched_directory("name");
    let by_manifest = home.join(".loft/lib/instprobe");
    let by_directory = home.join(".loft/lib/checkout_dir_name");

    assert!(
        by_manifest.is_dir(),
        "expected the install under the manifest's `[package] name`\n{all}"
    );
    // Both halves: a fix that installed under BOTH names would satisfy the first
    // assertion while still leaving the unreachable copy behind.
    assert!(
        !by_directory.exists(),
        "the directory-named copy must not be left behind — nothing can `use` it\n{all}"
    );
    let _ = std::fs::remove_dir_all(home.parent().unwrap());
}

/// `loft api` must not send the reader to a command that does not resolve the dependency
/// it is reporting.  A `{ path = … }` dep needs no install at all — it resolves from the
/// path it names (loft#963) — so an unresolved one means the path is wrong.
#[test]
fn the_api_hint_for_a_broken_path_dep_names_the_path() {
    let base = std::env::temp_dir().join(format!("loft_966_hint_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    write(
        &base.join("loft.toml"),
        "[package]\nname    = \"instprobe\"\nversion = \"0.1.0\"\n\n[library]\n\
         entry = \"src/instprobe.loft\"\n\n[dependencies]\n\
         moros_map = { path = \"../nowhere/moros_map\" }\n",
    );
    write(
        &base.join("src/instprobe.loft"),
        "pub fn hi() -> integer { 1 }\n",
    );

    let out = Command::new(loft_bin())
        .arg("api")
        .env("LOFT_TIMEOUT", "90")
        .current_dir(&base)
        .output()
        .expect("spawn loft api");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&base);

    assert!(
        all.contains("../nowhere/moros_map"),
        "the hint must name the path that did not resolve\n{all}"
    );
    assert!(
        !all.contains("NOT INSTALLED — run `loft install`\n"),
        "a path dep is not fixed by an install\n{all}"
    );
}
