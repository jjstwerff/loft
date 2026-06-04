// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 Arc N / N2 — the auto-generated native-library cdylib must COMPILE.
//!
//! `native_lib::generate_cdylib_lib_rs` produces a cdylib `lib.rs` from a
//! library's scalar-dispatchable functions (the `--native` program + export
//! wrappers).  This is the first verifiable milestone of N2's dispatch: the
//! generated source actually builds as a `cdylib` against `libloft.rlib`, and the
//! export symbol (`loft_n_double`) is present.  (Dispatching to it — the
//! end-to-end `double(21)→42` — is the next slice.)

use std::path::PathBuf;
use std::process::Command;

/// Locate `libloft.rlib` + its sibling `deps/` for standalone rustc, matching the
/// feature set of this test binary (mirrors `tests/native.rs::find_loft_rlib`).
fn find_loft_rlib() -> Option<(PathBuf, PathBuf)> {
    let deps = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let rlib = std::fs::read_dir(&deps)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            (n.starts_with("libloft-") || n == "libloft.rlib") && n.ends_with(".rlib")
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?
        .path();
    Some((rlib, deps))
}

/// `--extern name=path` for optional feature deps (random/png) that the generated
/// stdlib code may reference, mirroring `tests/native.rs::collect_extra_externs`.
fn extra_externs(deps: &std::path::Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(deps) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !n.starts_with("lib") || !n.ends_with(".rlib") || n.starts_with("libloft") {
                continue;
            }
            if let Some(stem) = n
                .strip_prefix("lib")
                .and_then(|s| s.rsplit_once('-'))
                .map(|x| x.0)
            {
                out.push((stem.to_string(), e.path()));
            }
        }
    }
    out
}

#[test]
fn generated_cdylib_compiles_and_exports_scalar_symbol() {
    let Some((rlib, deps)) = find_loft_rlib() else {
        println!("skip: libloft.rlib not found");
        return;
    };
    if Command::new("rustc").arg("--version").output().is_err() {
        println!("skip: rustc unavailable");
        return;
    }

    // Parse stdlib + a tiny library with one scalar function.
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n2cdylib_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let lib = tmp.join("lib.loft");
    // A pure library — no `main`, so no `n_main` and no `fn main()` bootstrap is
    // emitted; only the exported function + its transitive deps are generated.
    std::fs::write(&lib, "pub fn double(x: integer) -> integer { x * 2 }\n").unwrap();

    let mut p = loft::parser::Parser::new();
    if p.parse_dir("default", true, false).is_err() {
        println!("skip: default/ stdlib not parseable here");
        return;
    }
    p.parse(&lib.to_string_lossy(), false);
    loft::scopes::check(&mut p.data);
    p.data.namespace_colliding_native_fns();

    // The export set is the LIBRARY's own public scalar-dispatchable functions —
    // here just `double`.  (A cdylib exports the library's public API, not the
    // whole stdlib; the stdlib is a reachable dependency, inlined/emitted as
    // needed.)  Passing the whole stdlib scalar set would try to wrap operator
    // definitions (`OpAddSingle`, …) that `--native` inlines rather than emits.
    let scalar = loft::native_gate::scalar_dispatchable(&p.data);
    let double_nr = p.data.def_nr("n_double");
    assert!(
        scalar.contains(&double_nr),
        "double should be scalar-dispatchable"
    );
    let export: std::collections::HashSet<u32> = std::iter::once(double_nr).collect();

    // Compile to bytecode to populate the database (the type schema codegen reads).
    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);

    // Generate the cdylib source.
    let src = loft::native_lib::generate_cdylib_lib_rs(&p.data, &state.database, &export);
    assert!(
        src.contains("pub extern \"C\" fn loft_n_double"),
        "the export wrapper for double must be present"
    );
    let rs = tmp.join("lib.rs");
    std::fs::write(&rs, &src).unwrap();

    // Compile it as a cdylib against libloft.rlib (mirrors the --native rustc call).
    let so = tmp.join(if cfg!(target_os = "windows") {
        "loft_n2.dll"
    } else if cfg!(target_os = "macos") {
        "libloft_n2.dylib"
    } else {
        "libloft_n2.so"
    });
    let mut args: Vec<String> = vec![
        "--edition=2024".into(),
        "-C".into(),
        "debuginfo=0".into(),
        "-C".into(),
        "opt-level=0".into(),
        "--crate-type".into(),
        "cdylib".into(),
        "-o".into(),
        so.display().to_string(),
        rs.display().to_string(),
        "--extern".into(),
        format!("loft={}", rlib.display()),
        "-L".into(),
        deps.display().to_string(),
    ];
    for (name, path) in extra_externs(&deps) {
        args.push("--extern".into());
        args.push(format!("{name}={}", path.display()));
    }
    let out = Command::new("rustc")
        .args(&args)
        .output()
        .expect("invoke rustc");
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Keep the generated source for inspection on failure.
        panic!(
            "cdylib compile FAILED. source at {}\n--- rustc stderr (tail) ---\n{}",
            rs.display(),
            stderr
                .lines()
                .rev()
                .take(40)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    assert!(so.exists(), "cdylib output should exist");

    let _ = std::fs::remove_dir_all(&tmp);
}
