// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `loft check` answers a PERSON; `--check --native` under `LOFT_CHECK_ARTIFACT`
//! answers the live-reload host.
//!
//! One `println!` used to serve both. The machine form — `ok <src> <artifact>`, where
//! the artifact is a content-addressed entry under `.loft/cache/` — is what
//! `live_dispatch::spawn_build` parses to find the build it just asked for (@PLN18
//! 08-S4). It was also what somebody typing the reference's own `loft check hello.loft`
//! saw: an absolute path they had just typed, and an internal cache path they cannot
//! act on. The chapter documents the output as `ok`.
//!
//! The live-reload suite does not cover the protocol — with the env var removed from
//! `spawn_build`, `tests/engine_host_reload.rs` stays green — so the machine half is
//! guarded here or nowhere.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A directory of its own per case: `check` writes a `.loft/` cache beside the source.
fn case_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_check_line_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("case dir");
    std::fs::write(
        dir.join("hello.loft"),
        "fn main() { println(\"hello, world!\"); }\n",
    )
    .expect("source");
    dir
}

#[test]
fn a_person_running_check_is_told_ok_and_nothing_else() {
    for (name, args) in [
        ("native", vec!["check", "hello.loft"]),
        ("interpret", vec!["--interpret", "--check", "hello.loft"]),
    ] {
        let dir = case_dir(name);
        let out = Command::new(loft_bin())
            .args(&args)
            .current_dir(&dir)
            .env_remove("LOFT_CHECK_ARTIFACT")
            .output()
            .expect("loft check");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            stdout.trim(),
            "ok",
            "`loft {}` should answer a person with `ok`; it said {stdout:?}",
            args.join(" ")
        );
    }
}

#[test]
fn the_live_host_still_gets_the_source_and_artifact_it_parses() {
    let dir = case_dir("artifact");
    let out = Command::new(loft_bin())
        .args(["--check", "--native", "hello.loft"])
        .current_dir(&dir)
        .env("LOFT_CHECK_ARTIFACT", "1")
        .output()
        .expect("loft --check --native");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().find(|l| l.starts_with("ok ")).unwrap_or("");

    // `live_dispatch::spawn_build` strips exactly `ok <src> ` and takes the rest.
    //
    // The source is named under BOTH spellings of the case directory, because the driver
    // reports the path it resolved and the test built its own from `temp_dir()`. On macOS
    // those differ: `/var` is a symlink to `/private/var`, so the driver says
    // `/private/var/folders/…/hello.loft` where this test had `/var/folders/…/hello.loft`
    // and the prefix matched nothing. On Windows `temp_dir()` is the 8.3 short form
    // (`C:\Users\RUNNER~1\…`) and a bare `canonicalize` is the verbatim form
    // (`\\?\C:\Users\runneradmin\…`) — neither is what the driver prints. The driver's own
    // spelling is `portable_path::plain_canonical`, so the resolved candidate is built the
    // same way, and whichever of the two the driver used is accepted.
    let candidates = [
        dir.join("hello.loft"),
        loft::portable_path::plain_canonical(&dir).join("hello.loft"),
    ];
    let artifact = candidates
        .iter()
        .find_map(|src| line.strip_prefix(&format!("ok {} ", src.display())))
        .unwrap_or_else(|| {
            panic!("the live host parses `ok <src> <artifact>`; the driver said {line:?}")
        })
        .trim();
    assert!(
        !artifact.is_empty(),
        "the artifact field is empty — the live host would rebuild into nothing"
    );
    assert!(
        std::path::Path::new(artifact).is_absolute(),
        "the artifact path must be absolute; got {artifact:?}"
    );
}
