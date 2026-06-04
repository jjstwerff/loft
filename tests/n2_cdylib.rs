// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 Arc N / N2 — the auto-generated native-library cdylib must COMPILE
//! and be DISPATCHABLE from an interpreted script.
//!
//! `native_lib::generate_cdylib_lib_rs` produces a cdylib `lib.rs` from a
//! library's scalar-dispatchable functions (the `--native` program + export
//! wrappers).  Two milestones:
//!
//! 1. `generated_cdylib_compiles_and_exports_scalar_symbol` — the generated
//!    source actually builds as a `cdylib` against `libloft.rlib`, and the export
//!    symbol (`loft_n_double`) is present.
//! 2. `dispatches_scalar_call_into_generated_cdylib` — an interpreted script that
//!    declares `double` as `#native "loft_n_double"` and calls `double(21)`
//!    dispatches into the generated cdylib and gets `42` — the full
//!    interpret→native store-ABI round trip for the scalar slice.  (Auto-deriving
//!    the `#native` decl from `use <lib>` is N3 policy; here it's written by hand
//!    to isolate the dispatch mechanism.)

extern crate loft;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

mod common;
use common::cached_default;

// load_all / wire_native_fns mutate process-global registries (NATIVE_REGISTRY,
// STUB_SYMBOLS, LOADED_LIBS); byte_code calls set_stub_symbols.  Serialise both
// tests so one's stub set can't clobber the other's between wire and execute.
static TEST_LOCK: Mutex<()> = Mutex::new(());

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
fn extra_externs(deps: &Path) -> Vec<(String, PathBuf)> {
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

/// Platform cdylib filename for `stem`.
fn cdylib_name(stem: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{stem}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else {
        format!("lib{stem}.so")
    }
}

/// Compile `src` as a cdylib against `libloft.rlib`, mirroring the `--native`
/// rustc invocation.  Panics (keeping the source for inspection) on failure.
fn compile_cdylib(src: &str, stem: &str, tmp: &Path, rlib: &Path, deps: &Path) -> PathBuf {
    let rs = tmp.join(format!("{stem}.rs"));
    std::fs::write(&rs, src).unwrap();
    let so = tmp.join(cdylib_name(stem));
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
    for (name, path) in extra_externs(deps) {
        args.push("--extern".into());
        args.push(format!("{name}={}", path.display()));
    }
    let out = Command::new("rustc")
        .args(&args)
        .output()
        .expect("invoke rustc");
    assert!(
        out.status.success(),
        "cdylib compile FAILED. source at {}\n--- rustc stderr (tail) ---\n{}",
        rs.display(),
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(so.exists(), "cdylib output should exist");
    so
}

/// Build a cdylib from `lib_src` (a pure library — no `main`) exporting the
/// scalar-dispatchable function `fn_name` as `loft_n_<fn_name>`.  Returns
/// (so_path, tmp_dir), or None when the rlib/rustc/stdlib aren't available
/// (test skips).
fn build_scalar_lib_cdylib(stem: &str, lib_src: &str, fn_name: &str) -> Option<(PathBuf, PathBuf)> {
    let (rlib, deps) = find_loft_rlib()?;
    if Command::new("rustc").arg("--version").output().is_err() {
        println!("skip: rustc unavailable");
        return None;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n2_{stem}_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(lib_src, "lib", false);
    loft::scopes::check(&mut p.data);

    let scalar = loft::native_gate::scalar_dispatchable(&p.data);
    let fn_nr = p.data.def_nr(&format!("n_{fn_name}"));
    assert!(
        scalar.contains(&fn_nr),
        "{fn_name} should be scalar-dispatchable"
    );
    let export: std::collections::HashSet<u32> = std::iter::once(fn_nr).collect();

    // Compile to bytecode to populate the database (the type schema codegen reads).
    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);

    let src = loft::native_lib::generate_cdylib_lib_rs(&p.data, &state.database, &export);
    let want = format!("pub extern \"C\" fn loft_n_{fn_name}");
    assert!(
        src.contains(&want),
        "the export wrapper for {fn_name} must be present"
    );

    let so = compile_cdylib(&src, stem, &tmp, &rlib, &deps);
    Some((so, tmp))
}

/// The canonical `double` library used by the compile + dispatch milestones.
fn build_double_cdylib(stem: &str) -> Option<(PathBuf, PathBuf)> {
    build_scalar_lib_cdylib(
        stem,
        "pub fn double(x: integer) -> integer { x * 2 }",
        "double",
    )
}

#[test]
fn generated_cdylib_compiles_and_exports_scalar_symbol() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_double_cdylib("loft_n2_compile") else {
        return;
    };
    assert!(so.exists(), "cdylib output should exist");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Run a SCRIPT that declares `native_decl` (a `#native "loft_…"` import of a
/// cdylib symbol) and calls it from `main`, dispatching into `so`.  Panics if the
/// dispatch fails (stub panic) or the assert in `source` fails.
fn run_dispatch(so: &Path, native_decl: &str, source: &str) {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    let has_errors = p.diagnostics.lines().iter().any(|l| l.starts_with("Error"));
    assert!(!has_errors, "diagnostics: {:?}", p.diagnostics.lines());
    scopes::check(&mut p.data);

    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    // Load the generated cdylib (zero-registration: resolved via dlsym) and wire
    // the auto-marshal dispatcher.
    extensions::load_all(&mut state, vec![so.to_string_lossy().into_owned()]);
    extensions::wire_native_fns(&mut state, &p.data);

    state.execute_argv("main", &p.data, &[]);
}

#[test]
fn dispatches_scalar_call_into_generated_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_double_cdylib("loft_n2_dispatch") else {
        return;
    };

    // The SCRIPT: declare `double` as a native import of the cdylib symbol, then
    // call it.  This is exactly what N3 will auto-generate from `use <lib>`.
    let native_decl = "pub fn double(x: integer) -> integer not null;\n#native \"loft_n_double\"\n";
    let source = r#"
fn main() {
    assert(double(21) == 42, "double(21) should dispatch to native and return 42, got {double(21)}")
}
"#;
    run_dispatch(&so, native_decl, source);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A scalar-signature function whose *body* allocates on the heap (builds and
/// sums a `vector<integer>`).  The per-call `Stores` cell in the export wrapper
/// backs that internal allocation — no store reference crosses the boundary, so
/// the scalar slice already covers it.  This is the reach the slice unlocks: any
/// scalar-in/scalar-out function regardless of internal heap use.
#[test]
fn dispatches_internal_heap_scalar_fn() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let lib_src = "pub fn sum_first_n(n: integer) -> integer {\n\
                   \x20   v: vector<integer> = [];\n\
                   \x20   for i in 0..n { v += [i]; }\n\
                   \x20   total = 0;\n\
                   \x20   for x in v { total += x; }\n\
                   \x20   total\n\
                   }";
    let Some((so, tmp)) = build_scalar_lib_cdylib("loft_n2_heap", lib_src, "sum_first_n") else {
        return;
    };

    let native_decl =
        "pub fn sum_first_n(n: integer) -> integer not null;\n#native \"loft_n_sum_first_n\"\n";
    // 0 + 1 + … + 9 = 45.
    let source = r#"
fn main() {
    assert(sum_first_n(10) == 45, "sum_first_n(10) should be 45, got {sum_first_n(10)}")
}
"#;
    run_dispatch(&so, native_decl, source);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression: loading TWO zero-registration cdylibs in sequence must not trip
/// the "unregistered via loft_register_v1" guard.  Before the per-library
/// `uses_v1` fix, the first lib's dlsym-inserted symbol left `NATIVE_REGISTRY`
/// non-empty, so the second lib's `wire_native_fns` false-positived (the guard
/// keyed on "registry non-empty" as a proxy for "a v1 lib loaded").  The C71
/// model auto-loads many zero-registration cdylibs, so this must hold.
#[test]
fn two_zero_registration_cdylibs_dont_trip_v1_guard() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so1, tmp1)) = build_scalar_lib_cdylib(
        "loft_n2_guard_a",
        "pub fn triple(x: integer) -> integer { x * 3 }",
        "triple",
    ) else {
        return;
    };
    let Some((so2, tmp2)) = build_scalar_lib_cdylib(
        "loft_n2_guard_b",
        "pub fn quad(x: integer) -> integer { x * 4 }",
        "quad",
    ) else {
        return;
    };

    run_dispatch(
        &so1,
        "pub fn triple(x: integer) -> integer not null;\n#native \"loft_n_triple\"\n",
        "fn main() { assert(triple(7) == 21, \"triple(7) should be 21, got {triple(7)}\") }",
    );
    // The second zero-registration cdylib: before the fix this panicked with the
    // "not registered via loft_register_v1" guard.
    run_dispatch(
        &so2,
        "pub fn quad(x: integer) -> integer not null;\n#native \"loft_n_quad\"\n",
        "fn main() { assert(quad(5) == 20, \"quad(5) should be 20, got {quad(5)}\") }",
    );

    let _ = std::fs::remove_dir_all(&tmp1);
    let _ = std::fs::remove_dir_all(&tmp2);
}
