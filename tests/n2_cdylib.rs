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

/// Build a cdylib from `lib_src` (a pure library — no `main`) exporting the
/// **shared-store-dispatchable** function `fn_name` as `loft_shared_n_<fn_name>`
/// (the `*mut Stores` bridge, for a non-scalar value crossing the boundary).
/// Returns (so_path, tmp_dir), or None when the toolchain isn't available.
fn build_shared_lib_cdylib(stem: &str, lib_src: &str, fn_name: &str) -> Option<(PathBuf, PathBuf)> {
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

    let shared = loft::native_gate::shared_store_dispatchable(&p.data);
    let fn_nr = p.data.def_nr(&format!("n_{fn_name}"));
    assert!(
        shared.contains(&fn_nr),
        "{fn_name} should be shared-store-dispatchable"
    );
    let export: std::collections::HashSet<u32> = std::iter::once(fn_nr).collect();

    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);

    let src = loft::native_lib::generate_shared_cdylib_lib_rs(&p.data, &state.database, &export);
    let want = format!("pub extern \"C\" fn loft_shared_n_{fn_name}");
    assert!(
        src.contains(&want),
        "the shared bridge for {fn_name} must be present"
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

/// The store-touching slice's library: `vec_sum` takes a `vector<integer>` (a
/// non-scalar value crossing the boundary) and returns its sum.
const VEC_SUM_LIB: &str = "pub fn vec_sum(data: vector<integer>) -> integer {\n\
                           \x20   total = 0;\n\
                           \x20   for x in data { total += x; }\n\
                           \x20   total\n\
                           }";

/// N2 store-touching slice, milestone 1: the shared-store bridge (`*mut Stores`
/// + `LibArg` ABI) for a `vector<integer>`-arg function must COMPILE.
#[test]
fn generated_shared_cdylib_compiles() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_c", VEC_SUM_LIB, "vec_sum")
    else {
        return;
    };
    assert!(so.exists(), "shared cdylib output should exist");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Run a SCRIPT that declares `native_decl` (a `#native "loft_shared_…"` import
/// of an auto-generated shared-store bridge) and calls it from `main`,
/// dispatching into `so` over the shared `*mut Stores` ABI.
fn run_shared_dispatch(so: &Path, native_decl: &str, source: &str) {
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

    extensions::load_all(&mut state, vec![so.to_string_lossy().into_owned()]);
    extensions::wire_shared_native_fns(&mut state, &p.data);

    state.execute_argv("main", &p.data, &[]);
}

/// N2 store-touching slice, milestone 2: dispatch a `vector<integer>` argument
/// into the generated shared-store bridge end-to-end.  The interpreter builds the
/// vector in its store; the bridge shares that store by pointer; the `--native`
/// body sums it — `vec_sum([10,20,30]) == 60`.  The raw stack `DbRef` crosses
/// unchanged (no deref) and resolves correctly in the shared store.
#[test]
fn dispatches_vector_arg_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_d", VEC_SUM_LIB, "vec_sum")
    else {
        return;
    };

    let native_decl = "pub fn vec_sum(data: vector<integer>) -> integer not null;\n#native \"loft_shared_n_vec_sum\"\n";
    let source = r#"
fn main() {
    d = [10, 20, 30];
    assert(vec_sum(d) == 60, "vec_sum([10,20,30]) should be 60, got {vec_sum(d)}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A function that ALLOCATES a `vector<integer>` and returns it — the non-scalar
/// *return* case.  The `--native` body allocates in the shared store; the
/// returned `DbRef` must be valid back in the interpreter, which then iterates it.
const RANGE_VEC_LIB: &str = "pub fn range_vec(n: integer) -> vector<integer> {\n\
                             \x20   v: vector<integer> = [];\n\
                             \x20   for i in 0..n { v += [i]; }\n\
                             \x20   v\n\
                             }";

/// N2 store-touching slice, milestone 3: a non-scalar *return* crosses the
/// boundary.  `range_vec(4)` allocates `[0,1,2,3]` in the SHARED store inside the
/// native body and returns its `DbRef`; the interpreter sums it to 6.
///
/// `--native` returns a vector via a **hidden trailing destination `DbRef`**
/// (`Attribute::hidden`, appended by `ref_return`) that the caller pre-allocates
/// (`stores.null_named` + `OpDatabase(<type_id>)`).  The shared bridge wrapper
/// allocates that destination itself, so the script-side `#native` forward-decl
/// still models only the public param `n`.  Proves native-made store allocations
/// are valid back in the interpreter.
#[test]
fn dispatches_vector_return_from_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_r", RANGE_VEC_LIB, "range_vec")
    else {
        return;
    };

    let native_decl = "pub fn range_vec(n: integer) -> vector<integer> not null;\n#native \"loft_shared_n_range_vec\"\n";
    let source = r#"
fn main() {
    v = range_vec(4);
    total = 0;
    for x in v { total += x; }
    assert(total == 6, "sum of range_vec(4) = 0+1+2+3 should be 6, got {total}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A function taking a struct `reference` (schema-DEPENDENT — the native body
/// reads `p.x`/`p.y` at `db.finish()`-computed field offsets).
const POINT_SUM_LIB: &str = "struct Point { x: integer, y: integer }\n\
                             pub fn point_sum(p: Point) -> integer { p.x + p.y }";

/// N2 store-touching probe: a struct `reference` ARG crosses the boundary.  This
/// is the first SCHEMA-DEPENDENT case — the library cdylib and the interpreter,
/// built from SEPARATE `Data`, must assign `Point` the same type id + field
/// offsets for the shared `DbRef` to read correctly.  `point_sum(Point{3,4}) == 7`.
#[test]
fn dispatches_struct_arg_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_s", POINT_SUM_LIB, "point_sum")
    else {
        return;
    };

    let native_decl = "struct Point { x: integer, y: integer }\npub fn point_sum(p: Point) -> integer not null;\n#native \"loft_shared_n_point_sum\"\n";
    let source = r#"
fn main() {
    p = Point { x: 3, y: 4 };
    r = point_sum(p);
    assert(r == 7, "point_sum of x=3 y=4 should be 7, got {r}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A function that constructs and RETURNS a struct.  Unlike a vector return,
/// `--native` does NOT use a hidden destination — the body allocates the record
/// fresh and returns its `DbRef` (`n_make_point(cell, a, b) -> DbRef`).
const MAKE_POINT_LIB: &str = "struct Point { x: integer, y: integer }\n\
                              pub fn make_point(a: integer, b: integer) -> Point {\n\
                              \x20   Point { x: a, y: b }\n\
                              }";

/// N2 store-touching: a struct `reference` RETURN crosses the boundary.  The
/// native body allocates a `Point` in the shared store and returns its `DbRef`;
/// the interpreter reads its fields.  `make_point(3,4)` → `p.x*10 + p.y == 34`.
#[test]
fn dispatches_struct_return_from_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) =
        build_shared_lib_cdylib("loft_n2_shared_sr", MAKE_POINT_LIB, "make_point")
    else {
        return;
    };

    let native_decl = "struct Point { x: integer, y: integer }\npub fn make_point(a: integer, b: integer) -> Point not null;\n#native \"loft_shared_n_make_point\"\n";
    let source = r#"
fn main() {
    p = make_point(3, 4);
    r = p.x * 10 + p.y;
    assert(r == 34, "make_point(3,4) fields should give 34, got {r}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A function taking a `text` arg (passed as `&str` — ptr+len borrowed from the
/// shared store) and returning a scalar.
const STR_LEN_LIB: &str = "pub fn str_len(s: text) -> integer {\n\
                           \x20   n = 0;\n\
                           \x20   for c in s { n += 1; }\n\
                           \x20   n\n\
                           }";

/// N2 store-touching: a `text` ARG crosses the boundary.  `--native` takes a
/// `text` parameter as `&str` (ptr+len), not a `DbRef`; the bridge borrows the
/// store-backed bytes for the call.  `str_len("hello") == 5`.
#[test]
fn dispatches_text_arg_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_t", STR_LEN_LIB, "str_len")
    else {
        return;
    };

    let native_decl =
        "pub fn str_len(s: text) -> integer not null;\n#native \"loft_shared_n_str_len\"\n";
    let source = r#"
fn main() {
    n = str_len("hello");
    assert(n == 5, "str_len('hello') should be 5, got {n}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A function RETURNING `text` — `--native` uses the `text_return` `&mut String`
/// work buffer (hidden param) and returns a `Str`.  The bridge owns a local
/// `String`, then copies the result into the shared store's scratch.
const SHOUT_LIB: &str = "pub fn shout(s: text) -> text {\n\
                         \x20   s + \"!\"\n\
                         }";

/// N2 store-touching: a `text` RETURN crosses the boundary.  The native body
/// builds the result in a work `String`; the bridge copies it into the shared
/// store's scratch so it survives back in the interpreter.  `shout("hi") == "hi!"`.
#[test]
fn dispatches_text_return_from_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_tr", SHOUT_LIB, "shout") else {
        return;
    };

    let native_decl = "pub fn shout(s: text) -> text not null;\n#native \"loft_shared_n_shout\"\n";
    let source = r#"
fn main() {
    r = shout("hi");
    assert(r == "hi!", "shout('hi') should be 'hi!', got {r}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Plain (tag-only) enum: `--native` represents it as a `u8` tag (both arg and
/// return — no hidden dest), riding in the `LibArg` scalar slot.
const DIR_LIB: &str = "enum Direction { North, East, South, West }\n\
                       pub fn dir_code(d: Direction) -> integer {\n\
                       \x20   match d { North => 0, East => 1, South => 2, West => 3 }\n\
                       }\n\
                       pub fn dir_from(n: integer) -> Direction {\n\
                       \x20   if n == 1 { East } else { North }\n\
                       }";

/// N2 store-touching: a plain `enum` crosses the boundary as a `u8` tag, both as
/// an ARG (`dir_code(South) == 2`) and a RETURN (`dir_from(1) == East`, verified
/// by round-tripping through `dir_code`).
#[test]
fn dispatches_plain_enum_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // dir_code: enum arg, scalar return.
    let Some((so_c, tmp_c)) = build_shared_lib_cdylib("loft_n2_enum_c", DIR_LIB, "dir_code") else {
        return;
    };
    let decl_c = "enum Direction { North, East, South, West }\npub fn dir_code(d: Direction) -> integer not null;\n#native \"loft_shared_n_dir_code\"\n";
    run_shared_dispatch(
        &so_c,
        decl_c,
        "fn main() { c = dir_code(South); assert(c == 2, \"dir_code(South) should be 2, got {c}\") }",
    );
    let _ = std::fs::remove_dir_all(&tmp_c);

    // dir_from: scalar arg, enum return — verify by feeding it back to dir_code.
    let Some((so_f, tmp_f)) = build_shared_lib_cdylib("loft_n2_enum_f", DIR_LIB, "dir_from") else {
        return;
    };
    let decl_f = "enum Direction { North, East, South, West }\npub fn dir_from(n: integer) -> Direction not null;\n#native \"loft_shared_n_dir_from\"\n";
    run_shared_dispatch(
        &so_f,
        decl_f,
        "fn main() { d = dir_from(1); r = match d { East => 1, _ => 0 }; assert(r == 1, \"dir_from(1) should be East, got tag {r}\") }",
    );
    let _ = std::fs::remove_dir_all(&tmp_f);
}

/// Data enum (variants carrying fields): `--native` represents it as a `DbRef`
/// (like a struct), both arg and return — no hidden dest.
const SHAPE_LIB: &str = "enum Shape { Circle { r: integer }, Rect { w: integer, h: integer } }\n\
                         pub fn area(s: Shape) -> integer {\n\
                         \x20   match s { Circle { r } => r * r * 3, Rect { w, h } => w * h }\n\
                         }\n\
                         pub fn make_rect(w: integer, h: integer) -> Shape {\n\
                         \x20   Rect { w: w, h: h }\n\
                         }";

/// N2 store-touching: a data `enum` crosses the boundary as a `DbRef`, both as an
/// ARG (`area(Circle{r:2}) == 12`) and a RETURN (`make_rect(3,4)` then `area` → 12),
/// the latter allocated by native code in the shared store.
#[test]
fn dispatches_data_enum_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // area: data-enum arg, scalar return.
    let Some((so_a, tmp_a)) = build_shared_lib_cdylib("loft_n2_denum_a", SHAPE_LIB, "area") else {
        return;
    };
    let decl_a = "enum Shape { Circle { r: integer }, Rect { w: integer, h: integer } }\npub fn area(s: Shape) -> integer not null;\n#native \"loft_shared_n_area\"\n";
    run_shared_dispatch(
        &so_a,
        decl_a,
        "fn main() { s = Circle { r: 2 }; a = area(s); assert(a == 12, \"area(Circle r=2) should be 12, got {a}\") }",
    );
    let _ = std::fs::remove_dir_all(&tmp_a);

    // make_rect: data-enum RETURN (allocated in the shared store), read via area.
    let Some((so_m, tmp_m)) = build_shared_lib_cdylib("loft_n2_denum_m", SHAPE_LIB, "make_rect")
    else {
        return;
    };
    let decl_m = "enum Shape { Circle { r: integer }, Rect { w: integer, h: integer } }\npub fn make_rect(w: integer, h: integer) -> Shape not null;\n#native \"loft_shared_n_make_rect\"\n";
    run_shared_dispatch(
        &so_m,
        decl_m,
        "fn main() { s = make_rect(3, 4); r = match s { Rect { w, h } => w * h, _ => 0 }; assert(r == 12, \"make_rect(3,4) area should be 12, got {r}\") }",
    );
    let _ = std::fs::remove_dir_all(&tmp_m);
}
