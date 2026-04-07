// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A7.2 — `cdylib` native extension loader tests.
//!
//! Tests the manifest `native` field, `pending_native_libs` propagation on
//! Parser, and the `extensions::load_all()` dispatch path.

extern crate loft;

use loft::manifest::{Manifest, read_manifest};
use loft::parser::Parser;

mod common;
use common::cached_default;

// ---------------------------------------------------------------------------
// A7.2.1: manifest `native` field is parsed and accessible
// ---------------------------------------------------------------------------

/// A7.2.1: `read_manifest` returns the `native` field from `[library]`.
#[test]
fn manifest_parses_native_field() {
    use std::io::Write;
    let dir = std::env::temp_dir();
    let path = dir.join(format!("loft_a72_test_{}.toml", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(b"[package]\nloft = \">=0.8\"\n\n[library]\nnative = \"loft_myext\"\n")
        .unwrap();
    let m: Manifest = read_manifest(path.to_str().unwrap()).unwrap();
    assert_eq!(m.native.as_deref(), Some("loft_myext"));
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// A7.2.2: Parser accumulates pending_native_libs when a manifest has `native`
// ---------------------------------------------------------------------------

/// A7.2.2: Parser resolves the native library path when a package manifest
/// declares `native = "..."`.  The path is only added to `pending_native_libs`
/// when the pre-built `.so` exists or `auto_build_native` succeeds.
/// The test fixture has no buildable native crate, so the list stays empty —
/// but parsing must still succeed without errors.
#[test]
fn parser_native_pkg_parses_without_error() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_str("use native_pkg;", "test", false);
    // No parse errors — the #native stub is registered even without the .so.
    let has_errors = p.diagnostics.lines().iter().any(|l| l.starts_with("Error"));
    assert!(
        !has_errors,
        "unexpected errors: {:?}",
        p.diagnostics.lines()
    );
}

// ---------------------------------------------------------------------------
// A7.2.3: extensions::load_one registers functions via loft_register_v1
// ---------------------------------------------------------------------------

/// Helper: resolve the test fixture cdylib path.  Returns None if not built.
fn fixture_lib_path() -> Option<String> {
    let path = if cfg!(target_os = "macos") {
        "tests/lib/native_pkg/native/target/release/libloft_native_test.dylib"
    } else if cfg!(windows) {
        "tests/lib/native_pkg/native/target/release/loft_native_test.dll"
    } else {
        "tests/lib/native_pkg/native/target/release/libloft_native_test.so"
    };
    if std::path::Path::new(path).exists() {
        Some(path.to_string())
    } else {
        None
    }
}

/// A7.2.3: `extensions::load_one` loads a cdylib and registers its functions.
///
/// Requires the fixture shared library to be pre-built.
/// Build with: `cd tests/lib/native_pkg/native && cargo build --release`
#[test]
fn load_one_registers_native_functions() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let lib_path = match fixture_lib_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping: fixture cdylib not built — run: cd tests/lib/native_pkg/native && cargo build --release"
            );
            return;
        }
    };

    let native_decl = r#"
pub fn ext_add_one(x: integer) -> integer not null;
#native "loft_ext_add_one"
"#;
    let source = r#"
fn main() {
    assert(ext_add_one(41) == 42, "ext_add_one(41) should be 42, got {ext_add_one(41)}")
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    assert!(
        p.diagnostics.is_empty(),
        "diagnostics: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    // Load the fixture cdylib; it registers "n_ext_add_one" under "loft_ext_add_one".
    extensions::load_all(&mut state, vec![lib_path]);
    extensions::wire_native_fns(&mut state, &p.data);

    state.execute_argv("main", &p.data, &[]);
}

// ---------------------------------------------------------------------------
// A7.2.4: registry takes priority over dlsym — issue #119
// ---------------------------------------------------------------------------

/// A7.2.4: When a cdylib registers `n_fn` under the name `"loft_fn"` via
/// `loft_register_v1`, AND also exports a raw C-ABI `loft_fn` symbol,
/// the registered version must be used — not the dlsym fallback.
///
/// The fixture cdylib exports:
/// - `n_ext_add_one(x) -> x + 1`   (registered as "loft_ext_add_one")
/// - `loft_ext_add_one(x) -> x + 1000`  (raw C-ABI export, dlsym bait)
///
/// If the registry wins: `ext_add_one(41) == 42`.
/// If dlsym wins:         `ext_add_one(41) == 1041`.
#[test]
fn registry_takes_priority_over_dlsym() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let lib_path = match fixture_lib_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping: fixture cdylib not built — run: cd tests/lib/native_pkg/native && cargo build --release"
            );
            return;
        }
    };

    let native_decl = r#"
pub fn ext_add_one(x: integer) -> integer not null;
#native "loft_ext_add_one"
"#;
    // The assertion checks that the registered version (x+1) is called,
    // not the dlsym fallback (x+1000).
    let source = r#"
fn main() {
    result = ext_add_one(41);
    assert(result == 42, "Issue #119: expected 42 (registry), got {result} (dlsym fallback used wrong function)");
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    assert!(
        p.diagnostics.is_empty(),
        "diagnostics: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    extensions::load_all(&mut state, vec![lib_path]);
    extensions::wire_native_fns(&mut state, &p.data);

    // If we get here, the registered version (x+1) was wired correctly.
    state.execute_argv("main", &p.data, &[]);
}

// ---------------------------------------------------------------------------
// A7.2.5: guard panics when #native name is missing from registry — issue #119
// ---------------------------------------------------------------------------

/// A7.2.5: When a cdylib uses `loft_register_v1` but a `#native` annotation
/// refers to a symbol that wasn't registered, AND that symbol is found via
/// dlsym, `wire_native_fns` must panic — not silently use the wrong function.
///
/// This test runs as a subprocess to avoid corrupting the global static
/// registries (`NATIVE_REGISTRY`, `STUB_SYMBOLS`, `NATIVE_SIGS`) that are
/// shared across tests in the same process.
#[test]
fn guard_catches_unregistered_dlsym_fallback() {
    if fixture_lib_path().is_none() {
        eprintln!("skipping: fixture cdylib not built");
        return;
    }

    // Run ourselves as a subprocess with a special env var to trigger
    // the inner test logic.
    if std::env::var("LOFT_TEST_GUARD_INNER").is_ok() {
        guard_inner();
        return;
    }

    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(&exe)
        .env("LOFT_TEST_GUARD_INNER", "1")
        .arg("guard_catches_unregistered_dlsym_fallback")
        .arg("--exact")
        .arg("--test-threads=1")
        .arg("--nocapture")
        .output()
        .expect("failed to spawn subprocess");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "subprocess should have panicked but exited successfully"
    );
    assert!(
        stderr.contains("was not registered via loft_register_v1"),
        "expected registration bug panic message, got:\n{stderr}"
    );
}

fn guard_inner() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let lib_path = fixture_lib_path().unwrap();

    // Use a #native name that is NOT registered by loft_register_v1,
    // but IS exported as a raw C-ABI symbol (dlsym will find it).
    let native_decl = r#"
pub fn ext_bad(x: integer) -> integer not null;
#native "loft_ext_unregistered"
"#;
    let source = r#"
fn main() {
    println("{ext_bad(1)}");
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    extensions::load_all(&mut state, vec![lib_path]);
    // This should panic with the registration bug message.
    extensions::wire_native_fns(&mut state, &p.data);
}

// ---------------------------------------------------------------------------
// A7.3: Vector marshalling patterns
// ---------------------------------------------------------------------------

/// Helper: run a loft program with the test native library loaded.
fn run_native_test(native_decl: &str, source: &str) {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let lib_path = match fixture_lib_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping: fixture cdylib not built");
            return;
        }
    };

    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    let has_errors = p.diagnostics.lines().iter().any(|l| l.starts_with("Error"));
    assert!(!has_errors, "diagnostics: {:?}", p.diagnostics.lines());
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    extensions::load_all(&mut state, vec![lib_path]);
    extensions::wire_native_fns(&mut state, &p.data);
    state.execute_argv("main", &p.data, &[]);
}

/// A7.3.1: vector<integer> passed to native function — sum should be correct.
#[test]
fn vec_i32_sum() {
    run_native_test(
        r#"
pub fn ext_vec_sum(data: vector<integer>) -> integer not null;
#native "loft_ext_vec_sum"
"#,
        r#"
fn main() {
    data = [10, 20, 30, 40];
    result = ext_vec_sum(data);
    assert(result == 100, "vec_sum: expected 100, got {result}");
}
"#,
    );
}

/// A7.3.2: vector<single> (f32) passed to native function.
#[test]
fn vec_f32_sum() {
    run_native_test(
        r#"
pub fn ext_vec_sum_f32(data: vector<single>) -> integer not null;
#native "loft_ext_vec_sum_f32"
"#,
        r#"
fn main() {
    data = [1.0f, 2.0f, 3.0f, 4.0f];
    result = ext_vec_sum_f32(data);
    assert(result == 10, "vec_sum_f32: expected 10, got {result}");
}
"#,
    );
}

/// A7.3.3: scalar before vector parameter.
#[test]
fn scalar_before_vec() {
    run_native_test(
        r#"
pub fn ext_offset_sum(offset: integer, data: vector<integer>) -> integer not null;
#native "loft_ext_offset_sum"
"#,
        r#"
fn main() {
    data = [1, 2, 3];
    result = ext_offset_sum(100, data);
    assert(result == 106, "offset_sum: expected 106, got {result}");
}
"#,
    );
}

/// A7.3.4: vector between two scalars.
#[test]
fn vec_between_scalars() {
    run_native_test(
        r#"
pub fn ext_sandwich_sum(a: integer, data: vector<integer>, b: integer) -> integer not null;
#native "loft_ext_sandwich_sum"
"#,
        r#"
fn main() {
    data = [10, 20];
    result = ext_sandwich_sum(1, data, 2);
    assert(result == 33, "sandwich_sum: expected 33, got {result}");
}
"#,
    );
}

/// A7.3.5: vector from struct field (indirect reference).
#[test]
fn vec_from_struct_field() {
    run_native_test(
        r#"
pub fn ext_struct_vec_len(data: vector<integer>) -> integer not null;
#native "loft_ext_struct_vec_len"

struct TestBox {
    items: vector<integer>
}
"#,
        r#"
fn main() {
    b = TestBox { items: [1, 2, 3, 4, 5] };
    result = ext_struct_vec_len(b.items);
    assert(result == 5, "struct_vec_len: expected 5, got {result}");
}
"#,
    );
}

/// A7.3.6: vector call inside if block inside loop — issue #120 pattern.
#[test]
fn vec_in_loop_if() {
    run_native_test(
        r#"
pub fn ext_loop_vec_sum(data: vector<integer>) -> integer not null;
#native "loft_ext_loop_vec_sum"
"#,
        r#"
fn main() {
    data = [5, 10, 15];
    total = 0;
    for i in 0..10 {
        if true {
            s = ext_loop_vec_sum(data);
            total += s;
        }
    }
    assert(total == 300, "loop_vec_sum: expected 300, got {total}");
}
"#,
    );
}

/// A7.3.7: vector from struct field of a RETURNED struct.
/// This is the textured-cube pattern: make_texture() returns a Canvas,
/// then gl_upload_canvas(canvas.data, ...) reads the vector.
#[test]
fn vec_from_returned_struct() {
    run_native_test(
        r#"
pub fn ext_struct_vec_len(data: vector<integer>) -> integer not null;
#native "loft_ext_struct_vec_len"

struct TestBox {
    items: vector<integer>
}

fn make_box() -> TestBox {
    TestBox { items: [10, 20, 30, 40, 50, 60, 70, 80] }
}
"#,
        r#"
fn main() {
    b = make_box();
    result = ext_struct_vec_len(b.items);
    assert(result == 8, "returned_struct_vec: expected 8, got {result}");
}
"#,
    );
}

/// A7.3.8: vector from struct field of returned struct, with other calls between.
/// Tests that the store isn't freed/reused between make_box() and ext_struct_vec_len().
#[test]
fn vec_from_returned_struct_with_gap() {
    run_native_test(
        r#"
pub fn ext_add_one(x: integer) -> integer not null;
#native "loft_ext_add_one"

pub fn ext_struct_vec_len(data: vector<integer>) -> integer not null;
#native "loft_ext_struct_vec_len"

struct TestBox {
    items: vector<integer>
}

fn make_box() -> TestBox {
    TestBox { items: [1, 2, 3, 4] }
}
"#,
        r#"
fn main() {
    b = make_box();
    dummy = ext_add_one(0);
    dummy = ext_add_one(1);
    dummy = ext_add_one(2);
    result = ext_struct_vec_len(b.items);
    assert(result == 4, "returned_struct_gap: expected 4, got {result}");
}
"#,
    );
}

/// A7.3.9: vector from returned struct after heavy allocation.
/// Simulates the make_texture() pattern: create a struct with a large vector,
/// do many operations that allocate temporary stores, then return the struct.
#[test]
fn vec_from_returned_struct_heavy() {
    run_native_test(
        r#"
pub fn ext_vec_sum(data: vector<integer>) -> integer not null;
#native "loft_ext_vec_sum"

struct BigBox {
    width: integer,
    height: integer,
    data: vector<integer>
}

fn make_big() -> BigBox {
    w = 4;
    h = 4;
    d: vector<integer> = [];
    for y in 0..h {
        for x in 0..w {
            d += [x + y * w];
        }
    }
    BigBox { width: w, height: h, data: d }
}
"#,
        r#"
fn main() {
    b = make_big();
    assert(b.width == 4, "width: {b.width}");
    assert(b.height == 4, "height: {b.height}");
    result = ext_vec_sum(b.data);
    expected = 0;
    for i in 0..16 { expected += i; }
    assert(result == expected, "heavy: expected {expected}, got {result}");
}
"#,
    );
}

/// A7.3.9 (continued)
#[test]
fn vec_struct_field_in_loop() {
    run_native_test(
        r#"
pub fn ext_loop_vec_sum(data: vector<integer>) -> integer not null;
#native "loft_ext_loop_vec_sum"

struct Container {
    vals: vector<integer>
}
"#,
        r#"
fn main() {
    c = Container { vals: [3, 7] };
    total = 0;
    for i in 0..5 {
        total += ext_loop_vec_sum(c.vals);
    }
    assert(total == 50, "struct_field_loop: expected 50, got {total}");
}
"#,
    );
}
