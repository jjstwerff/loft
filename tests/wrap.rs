// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

use loft::compile::byte_code;
#[cfg(debug_assertions)]
use loft::compile::show_code;
use loft::data::Data;
use loft::generation::Output;
#[cfg(debug_assertions)]
use loft::log_config::LogConfig;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
use std::collections::HashSet;
#[cfg(debug_assertions)]
use std::fs::File;
use std::io::Error;
#[cfg(debug_assertions)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
mod common;
use common::cached_default;

/// Process-wide lock: prevents any two `wrap` tests from running concurrently.
///
/// Several scripts in `tests/scripts/` are not safe to execute in parallel with
/// themselves or each other — for example `11-files.loft` creates and deletes real
/// files, and `loft_suite` already runs that same file.  Cargo's default test runner
/// would execute `loft_suite` and `files()` (or any other pair) simultaneously,
/// causing races and spurious failures.
///
/// Every public `#[test]` in this file acquires the lock before calling `run_test`,
/// so all wrap tests are serialised within the process.  Cross-process parallelism
/// (e.g. two `cargo test` invocations at once) is the caller's responsibility.
static WRAP_LOCK: Mutex<()> = Mutex::new(());

/// Files in `tests/docs/` that are known to be broken (open issues).
/// `dir` skips these so that all other docs files are still exercised.
/// Remove an entry here once the underlying issue is fixed.
const SUITE_SKIP: &[&str] = &[
    // All previously skipped files now pass (verified 2026-04-03).
];

/// Docs files that are known to fail in `--native-wasm` mode.
const WASM_SKIP: &[&str] = &[
    "19-threading.loft", // todo!(); WASM threading model differs
];

/// Compile a `.loft` file to a WebAssembly binary via the loft codegen + rustc, then
/// optionally run it with `wasmtime`.
///
/// Skips silently (returns `Ok`) if the `wasm32-wasip2` target is not installed or if
/// `rustc` is not found.  Runs the wasm with `wasmtime` if it is in PATH; otherwise
/// only verifies that compilation succeeds.
fn run_wasm_test(entry: &Path) -> std::io::Result<()> {
    let stem = entry
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .replace('-', "_");
    println!("wasm  {entry:?}");

    // Parse
    let source = std::fs::read_to_string(entry)?;
    let expected = expected_warnings(&source);
    let (exp_errors, exp_ann_warns) = expected_annotations(&source);
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    let start_def = p.data.definitions();
    p.parse(&entry.to_string_lossy(), false);
    for l in p.diagnostics.lines() {
        println!("{l}");
    }
    if !p.diagnostics.is_empty() {
        check_diagnostics(
            &p.diagnostics.lines(),
            &expected,
            &exp_errors,
            &exp_ann_warns,
        )?;
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    let main_nr = p.data.def_nr("n_main");
    let entry_defs: Vec<u32> = if main_nr < end_def {
        vec![main_nr]
    } else {
        (start_def..end_def).collect()
    };

    // Generate Rust source
    let tmp_rs = std::env::temp_dir().join(format!("loft_wasm_{stem}.rs"));
    {
        let mut f = std::fs::File::create(&tmp_rs)?;
        let mut out = Output {
            data: &p.data,
            stores: &state.database,
            counter: 0,
            indent: 0,
            def_nr: 0,
            declared: HashSet::new(),
            reachable: HashSet::new(),
            loop_stack: Vec::new(),
            next_format_count: 0,
            yield_collect: false,
            fn_ref_context: false,
            call_stack_prefix: None,
            wasm_browser: false,
        };
        out.output_native_reachable(&mut f, start_def, end_def, &entry_defs)?;
    }

    // Compile for wasm32-wasip2
    let tmp_wasm = std::env::temp_dir().join(format!("loft_wasm_{stem}.wasm"));
    let mut cmd = std::process::Command::new("rustc");
    cmd.arg("--edition=2024")
        .arg("--target")
        .arg("wasm32-wasip2")
        .arg("--crate-type")
        .arg("bin")
        .arg("-O")
        .arg("-o")
        .arg(&tmp_wasm)
        .arg(&tmp_rs);
    // Look for a wasm32-wasip2 loft rlib next to the test binary's deps
    // (only present if the user ran `cargo build --target wasm32-wasip2` first).
    let wasm_rlib = std::env::current_exe().ok().and_then(|exe| {
        // Walk up from target/debug/deps to target/, then into wasm32-wasip2/debug/
        let target_dir = exe.parent()?.parent()?.parent()?;
        let rlib_dir = target_dir.join("wasm32-wasip2").join("debug");
        std::fs::read_dir(&rlib_dir)
            .ok()?
            .filter_map(|e| e.ok())
            .find(|e| {
                let n = e.file_name();
                let s = n.to_string_lossy();
                s.starts_with("libloft") && s.ends_with(".rlib")
            })
            .map(|e| (e.path(), rlib_dir))
    });
    if let Some((rlib, deps_dir)) = wasm_rlib {
        cmd.arg("--extern")
            .arg(format!("loft={}", rlib.display()))
            .arg("-L")
            .arg(&deps_dir);
    }
    let compile_out = cmd.output();
    let _ = std::fs::remove_file(&tmp_rs);
    let compile_out = match compile_out {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("  rustc not found — skipping wasm test for {stem}");
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    if !compile_out.status.success() {
        let stderr = String::from_utf8_lossy(&compile_out.stderr);
        // wasm32-wasip2 target not installed → skip gracefully
        if stderr.contains("target may not be installed") || stderr.contains("can't find crate") {
            println!("  wasm32-wasip2 target or loft wasm rlib not available — skipping {stem}");
            let _ = std::fs::remove_file(&tmp_wasm);
            return Ok(());
        }
        eprintln!("rustc (wasm) failed for {stem}:\n{stderr}");
        let _ = std::fs::remove_file(&tmp_wasm);
        return Err(Error::from(std::io::ErrorKind::Other));
    }

    // Run with wasmtime if available
    match std::process::Command::new("wasmtime")
        .arg(&tmp_wasm)
        .status()
    {
        Ok(s) => {
            let _ = std::fs::remove_file(&tmp_wasm);
            if !s.success() {
                eprintln!("wasmtime failed for {stem} (exit {:?})", s.code());
                return Err(Error::from(std::io::ErrorKind::Other));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("  wasmtime not found — compiled ok, skipping run for {stem}");
            let _ = std::fs::remove_file(&tmp_wasm);
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_wasm);
            return Err(e);
        }
    }
    Ok(())
}

/// Run every `.loft` file in `tests/docs/` in alphabetical order, skipping
/// files listed in `SUITE_SKIP` (known broken; tracked as open issues).
/// These files also serve as user-facing documentation (HTML generation via `@NAME`/`@TITLE`).
#[test]
fn dir() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/docs")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    for entry in files {
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        if SUITE_SKIP.iter().any(|s| *s == name.as_ref()) {
            println!("skip {entry:?} (known issue — see SUITE_SKIP)");
            continue;
        }
        run_test(entry, false, true)?;
    }
    Ok(())
}

/// Compile every `.loft` file in `tests/docs/` to WebAssembly (wasm32-wasip2),
/// skipping files listed in `WASM_SKIP`.
///
/// Skips silently if `rustc` is not in PATH or the `wasm32-wasip2` target is not
/// installed.  Runs the resulting `.wasm` with `wasmtime` if it is in PATH; otherwise
/// only verifies compilation succeeds.
#[test]
fn wasm_dir() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/docs")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    for entry in files {
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        if WASM_SKIP.iter().any(|s| *s == name.as_ref()) {
            println!("skip {entry:?} (wasm skip list — see WASM_SKIP)");
            continue;
        }
        run_wasm_test(&entry)?;
    }
    Ok(())
}

/// Run every `.loft` file in `tests/scripts/` in alphabetical order.
/// These are standalone loft programs that exercise compiler and interpreter features.
/// Scripts may use `fn main()` or `fn test_*()` entry points — `run_test`
/// discovers and executes all zero-parameter user functions automatically.
/// To run a single file use the individual test functions below, e.g.:
///   cargo test --test wrap integers
#[test]
fn loft_suite() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/scripts")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    // Scripts with dedicated #[ignore] wrappers are skipped here to keep
    // loft_suite green while the feature is under development.
    let skip: HashSet<&str> = ignored_scripts();
    for entry in files {
        let name = entry
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if skip.contains(name.as_str()) {
            println!("skip {entry:?} (has dedicated #[ignore] test)");
            continue;
        }
        run_test(entry, false, false)?;
    }
    Ok(())
}

/// Scripts that have a dedicated `#[test] #[ignore]` wrapper.
/// Removed once the feature lands and the #[ignore] is dropped.
fn ignored_scripts() -> HashSet<&'static str> {
    HashSet::from([
        // Requires lib_dirs (graphics, math, yield_test) — tested via leak.rs instead.
        "85-yield-resume.loft",
    ])
}

macro_rules! script_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() -> std::io::Result<()> {
            let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            run_test(PathBuf::from($path), false, false)
        }
    };
}

script_test!(integers, "tests/scripts/01-integers.loft");
script_test!(floats, "tests/scripts/02-floats.loft");
script_test!(text, "tests/scripts/03-text.loft");
script_test!(booleans, "tests/scripts/04-booleans.loft");
script_test!(enums, "tests/scripts/05-enums.loft");
script_test!(structs, "tests/scripts/06-structs.loft");
script_test!(control_flow, "tests/scripts/07-control-flow.loft");
script_test!(functions, "tests/scripts/08-functions.loft");
script_test!(lambdas, "tests/scripts/09-lambdas.loft");
script_test!(vectors, "tests/scripts/11-vectors.loft");
script_test!(collections, "tests/scripts/12-collections.loft");
script_test!(map_filter_reduce, "tests/scripts/13-map-filter-reduce.loft");
script_test!(formatting, "tests/scripts/14-formatting.loft");
script_test!(min_max_clamp, "tests/scripts/17-min-max-clamp.loft");
script_test!(math_functions, "tests/scripts/18-math-functions.loft");
script_test!(files, "tests/scripts/19-files.loft");
#[test]
fn binary() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/scripts/20-binary.loft"), false, false)
}
script_test!(binary_ops, "tests/scripts/21-binary-ops.loft");
script_test!(script_threading, "tests/scripts/22-threading.loft");
script_test!(stress, "tests/scripts/37-stress.loft");
script_test!(single_type, "tests/scripts/52-single.loft");

// S16a: field-name overlap between two plain structs in the same file.
// Both structs share a field name `val` at different byte offsets.
// Exercises sorted lookup, range query, full iteration, and index range query.
// Confirmed working — field offsets are type-scoped in determine_keys().
script_test!(
    field_overlap_structs,
    "tests/scripts/23-field-overlap-structs.loft"
);

// S16a: field-name overlap involving struct-enum variants in the same file.
// Scenario A: two struct-enum variants share field `score` at different offsets.
// Scenario B: a plain struct and a struct-enum variant share field `key`.
script_test!(
    field_overlap_enum_struct,
    "tests/scripts/24-field-overlap-enum-struct.loft"
);

// S16b: range queries on sorted<EnumVariant[field]>
// Fixed: index_type() now returns Type::Reference(variant_def_nr) instead of
// Type::Enum(parent, true) so for_type() and field access work correctly.
script_test!(
    sorted_enum_variant_range,
    "tests/scripts/25-sorted-enum-variant-range.loft"
);

// Logging functions compile and run as no-ops without a log.conf.
script_test!(logging_script, "tests/scripts/53-logging.loft");

// Implicit type widening in mixed integer/long/float expressions.
script_test!(auto_convert, "tests/scripts/54-auto-convert.loft");

// stack_trace() introspection returns frames from nested calls.
script_test!(stack_trace_script, "tests/scripts/55-stack-trace.loft");

/// P89 regression: every field name `n_stack_trace` looks up at runtime
/// must exist in the loaded schema.  If `default/04_stacktrace.loft` is
/// edited to rename or remove a field, this test fails immediately
/// instead of silently producing garbage stack-trace records.
#[test]
fn p89_stacktrace_schema_fields_exist() {
    let (_data, db) = cached_default();
    let db = &db;
    let sf_fields = [
        ("StackFrame", "function"),
        ("StackFrame", "file"),
        ("StackFrame", "line"),
        ("StackFrame", "arguments"),
        ("StackFrame", "variables"),
        ("VarInfo", "name"),
        ("VarInfo", "type_name"),
        ("VarInfo", "value"),
        ("BoolVal", "b"),
        ("IntVal", "n"),
        ("LongVal", "n"),
        ("FloatVal", "f"),
        ("SingleVal", "f"),
        ("CharVal", "c"),
        ("TextVal", "t"),
        ("RefVal", "store"),
        ("RefVal", "rec"),
        ("RefVal", "pos"),
        ("OtherVal", "description"),
    ];
    for (ty, field) in sf_fields {
        let tp = db.name(ty);
        assert_ne!(
            tp,
            u16::MAX,
            "schema is missing type {ty} (default/04_stacktrace.loft drift)"
        );
        let pos = db.position(tp, field);
        assert_ne!(
            pos,
            u16::MAX,
            "schema is missing {ty}.{field} (default/04_stacktrace.loft drift)"
        );
    }
}

/// Quick iteration test: run only the final suite file (`16-parser.loft`) without
/// regenerating documentation.  Use this during active development on the parser
/// to get a fast feedback cycle.
#[test]
fn last() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/16-parser.loft"), false, true)
}

/// Run `17-libraries.loft` in isolation; verifies inline-ref chaining (T0-6 fix).
#[test]
fn libraries() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/17-libraries.loft"), false, true)
}

/// Run `19-threading.loft` in isolation; covers `parallel_for_int` and the
/// new compiler-checked `parallel_for` API with `fn` references.
#[test]
fn threading() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/19-threading.loft"), false, true)
}

/// Run `20-logging.loft` in isolation; verifies log_* functions compile and
/// can be called without aborting when no logger is configured.
#[test]
fn logging() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/20-logging.loft"), false, true)
}

/// Debug the run of `13-file.loft` with a full execution trace written to
/// `tests/dumps/13-file.loft.txt`.  Use this to diagnose store-allocation bugs.
#[test]
fn file_debug() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/13-file.loft"), true, true)
}

/// Debug the run of `16-parser.loft` with a full execution trace written to
/// `tests/dumps/16-parser.loft.txt`.  Use this when diagnosing parser regressions.
/// Ignored: the execute_log trace dereferences a &text DbRef whose debug dump
/// reads uninitialised stack memory (the runtime itself works — only the trace
/// formatter segfaults).  See the `<raw:0x1>` in the dump at GetFileText.
/// Run explicitly: cargo test -- parser_debug --ignored
#[test]
#[ignore]
fn parser_debug() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/docs/16-parser.loft"), true, true)
}

/// Verify that `fn main(args: vector<text>)` receives the arguments passed via `execute_argv`.
#[test]
fn main_argv() {
    let code = r#"
fn main(args: vector<text>) {
    assert(len(args) == 3, "expected 3 args");
    assert(args[0] == "hello", "arg 0");
    assert(args[1] == "world", "arg 1");
    assert(args[2] == "foo", "arg 2");
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(code, "main_argv", false);
    for l in p.diagnostics.lines() {
        println!("{l}");
    }
    assert!(p.diagnostics.is_empty(), "parse errors");
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let args = ["hello", "world", "foo"]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    state.execute_argv("main", &p.data, &args);
}

/// T2: Verify `size()` returns Unicode code-point count, not byte length.
#[test]
fn size_text() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/41-size-text.loft"),
        false,
        true,
    )
}

/// A10: Verify field iteration (for f in s#fields).
#[test]
fn field_iteration() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/45-field-iter.loft"),
        false,
        true,
    )
}

/// L2: Verify nested match patterns in field positions.
#[test]
fn nested_match() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/44-nested-match.loft"),
        false,
        true,
    )
}

/// P3: Verify vector aggregates (sum, min_of, max_of, any, all, count_if).
#[test]
fn aggregates() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/43-aggregates.loft"),
        false,
        true,
    )
}

/// L3: Verify FileResult enum for filesystem operations.
#[test]
fn file_result() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/42-file-result.loft"),
        false,
        true,
    )
}

/// P5.2: Verify generic function call-site instantiation.
#[test]
fn generics() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/scripts/48-generics.loft"), false, true)
}

/// L7: Verify init(expr) stored field initialiser with $ reference.
#[test]
fn init_fields() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(
        PathBuf::from("tests/scripts/49-init-fields.loft"),
        false,
        true,
    )
}

/// Regression tests for documented caveats (CAVEATS.md).
#[test]
fn caveats() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/scripts/46-caveats.loft"), false, true)
}

/// Parse, type-check, compile, and execute one `.loft` test file.
///
/// The default library in `default/` is loaded first, then `entry` is parsed on
/// Extract `// #warn <text>` declarations from a `.loft` source file.
///
/// Each matching comment declares that the script is expected to produce a
/// `Warning:` diagnostic whose message contains `<text>` as a substring.
/// Lines of the form `// #warn Parameter 'x' does not need to be a reference`
/// allow a script that intentionally triggers a warning to still pass.
fn expected_warnings(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            t.strip_prefix("// #warn ").map(|s| s.trim().to_string())
        })
        .collect()
}

/// Validate diagnostics against expected patterns from `// #warn`, `@EXPECT_ERROR`,
/// and `@EXPECT_WARNING` declarations.
///
/// Returns `Ok(())` when every diagnostic matches a declared pattern.
/// Returns `Err` when any diagnostic is unexpected.  All mismatches are printed.
fn check_diagnostics(
    diagnostics: &[String],
    expected_warns: &[String],
    expected_errors: &[String],
    expected_ann_warnings: &[String],
) -> std::io::Result<()> {
    let mut unmatched_warns: Vec<&str> = expected_warns.iter().map(String::as_str).collect();
    let mut unmatched_errors: Vec<&str> = expected_errors.iter().map(String::as_str).collect();
    let mut unmatched_ann_warns: Vec<&str> =
        expected_ann_warnings.iter().map(String::as_str).collect();
    let mut unexpected: Vec<&str> = Vec::new();

    for diag in diagnostics {
        if diag.starts_with("Debug: ") {
            continue;
        } else if diag.starts_with("Warning: ") {
            // Try #warn patterns first (strict — must all match)
            if let Some(pos) = unmatched_warns.iter().position(|pat| diag.contains(*pat)) {
                println!("expected warning matched: {diag}");
                unmatched_warns.remove(pos);
            } else if let Some(pos) = unmatched_ann_warns
                .iter()
                .position(|pat| diag.contains(*pat))
            {
                println!("expected @EXPECT_WARNING matched: {diag}");
                unmatched_ann_warns.remove(pos);
            }
            // Other warnings are not fatal — just log them.
        } else if let Some(pos) = unmatched_errors.iter().position(|pat| diag.contains(*pat)) {
            println!("expected @EXPECT_ERROR matched: {diag}");
            unmatched_errors.remove(pos);
        } else {
            println!("unexpected error: {diag}");
            unexpected.push(diag);
        }
    }
    for pat in &unmatched_warns {
        println!("expected warning not emitted: {pat}");
    }
    // Only fail on unexpected errors or unmatched #warn patterns.
    if unexpected.is_empty() && unmatched_warns.is_empty() {
        Ok(())
    } else {
        Err(Error::from(std::io::ErrorKind::InvalidData))
    }
}

/// Collect the names of all zero-parameter user functions defined in `data`
/// starting from definition `start_def`.  Returns `"n_<name>"` internal names
/// stripped to their user-facing form (e.g. `"main"`, `"test_foo"`).
///
/// This mirrors the discovery logic in `test_runner.rs` so that scripts using
/// `fn test_*()` style entry points are exercised by `cargo test`, not only by
/// `loft --tests`.
fn entry_point_names(data: &Data, start_def: u32) -> Vec<String> {
    use loft::data::DefType;
    let mut names = Vec::new();
    for d_nr in start_def..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
            continue;
        }
        if def.position.file.starts_with("default/") || def.position.file.starts_with("default\\") {
            continue;
        }
        // Only zero-parameter functions are entry points.
        if !def.attributes.is_empty() {
            continue;
        }
        // Skip coroutine generators (return iterator<T>) — they must be called
        // from a for-loop, not as standalone entry points.
        if matches!(def.returned, loft::data::Type::Iterator(_, _)) {
            continue;
        }
        let user_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
        names.push(user_name.to_string());
    }
    names
}

/// Compile and run a single `.loft` script test.
///
/// Scripts may declare expected compile-time warnings with `// #warn <text>`
/// comments.  Each such comment consumes one matching `Warning:` diagnostic.
/// Unexpected diagnostics (errors or unmatched warnings) fail the test.
///
/// Any parse or type errors are printed and immediately fail the test.
/// On success the bytecode is generated and every zero-parameter user function
/// is called (not just `main`).  This ensures scripts that use `fn test_*()`
/// entry points are also exercised by `cargo test` / `cargo llvm-cov`.
///
/// Each entry-point function is run inside `catch_unwind` so that a failing
/// assert in one function does not abort the remaining functions.  All failures
/// are collected and reported at the end.
///
/// When `debug` is true (debug builds only) a human-readable bytecode dump is
/// written to `tests/dumps/<filename>.txt` and the interpreter emits a full
/// execution trace to that file.  Set `LOFT_DUMP=1` to get the bytecode dump
/// without the execution trace for any non-debug test invocation.
/// Scan source for `// @EXPECT_FAIL` annotations bound to specific functions.
/// Returns a set of function names whose panics should be tolerated.
/// Also returns true if the file has a file-level `@EXPECT_FAIL`.
fn expect_fail_fns(source: &str) -> (HashSet<String>, bool) {
    let mut fns = HashSet::new();
    let mut file_level = false;
    let mut pending = false;
    let mut in_header = true;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("fn ") {
            in_header = false;
            if pending
                && let Some(name) = trimmed
                    .strip_prefix("fn ")
                    .and_then(|s| s.split(&['(', ' ', '{'][..]).next())
            {
                fns.insert(name.to_string());
            }
            pending = false;
            continue;
        }
        if trimmed.starts_with("struct ") || trimmed.starts_with("enum ") {
            in_header = false;
            pending = false;
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix("//") {
            let comment = comment.trim();
            if comment.starts_with("@EXPECT_FAIL") {
                if in_header {
                    file_level = true;
                } else {
                    pending = true;
                }
            }
        } else {
            pending = false;
        }
    }
    if pending {
        file_level = true;
    }
    (fns, file_level)
}

/// Collect all `// @EXPECT_ERROR:` and `// @EXPECT_WARNING:` annotation substrings.
/// These are treated as expected diagnostics and consumed by `check_diagnostics`.
fn expected_annotations(source: &str) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(comment) = trimmed.strip_prefix("//") {
            let comment = comment.trim();
            if let Some(rest) = comment.strip_prefix("@EXPECT_ERROR:") {
                let sub = rest.trim();
                if !sub.is_empty() {
                    errors.push(sub.to_string());
                }
            } else if let Some(rest) = comment.strip_prefix("@EXPECT_WARNING:") {
                let sub = rest.trim();
                if !sub.is_empty() {
                    warnings.push(sub.to_string());
                }
            }
        }
    }
    (errors, warnings)
}

#[cfg_attr(not(debug_assertions), allow(unused_variables, unused_mut))]
fn run_test(entry: PathBuf, debug: bool, allow_dump: bool) -> std::io::Result<()> {
    println!("run {entry:?}");
    let source = std::fs::read_to_string(&entry)?;
    let expected = expected_warnings(&source);
    let (exp_errors, exp_ann_warns) = expected_annotations(&source);
    let (expect_fail, file_level_fail) = expect_fail_fns(&source);
    let _has_expected_errors = !exp_errors.is_empty();
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    #[cfg(debug_assertions)]
    let types = p.database.types.len();
    let start_def = p.data.definitions();
    let path = entry.to_string_lossy().to_string();
    p.parse(&path, false);
    let had_errors = !p.diagnostics.is_empty()
        && p.diagnostics
            .lines()
            .iter()
            .any(|l| !l.starts_with("Warning:") && !l.starts_with("Debug:"));
    if !p.diagnostics.is_empty() {
        check_diagnostics(
            &p.diagnostics.lines(),
            &expected,
            &exp_errors,
            &exp_ann_warns,
        )?;
    }
    // Only skip execution when the file actually has unresolved parse errors.
    // If @EXPECT_ERROR annotations exist but the errors are gone (bug fixed),
    // proceed to execution so the fix can be verified.
    if had_errors {
        println!("  ok (errors consumed)");
        return Ok(());
    }
    // Scope check and bytecode generation can panic on compiler bugs.
    // When the file has @EXPECT_FAIL annotations, tolerate the panic.
    let compile_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scopes::check(&mut p.data);
        let mut state = State::new(p.database);
        byte_code(&mut state, &mut p.data);
        (state, p.data)
    }));
    let (mut state, mut p_data) = match compile_result {
        Ok(pair) => {
            if file_level_fail {
                println!("  FIXED {path} (was @EXPECT_FAIL, now compiles)");
            }
            pair
        }
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else {
                "unknown panic".to_string()
            };
            if file_level_fail || !expect_fail.is_empty() {
                println!("  expected compile fail {path} — {msg}");
                return Ok(());
            }
            std::panic::resume_unwind(payload);
        }
    };

    // Discover all zero-parameter user functions as entry points.
    let all_fns = entry_point_names(&p_data, start_def);
    assert!(
        !all_fns.is_empty(),
        "no entry-point functions found in {}",
        path
    );
    // If `main` exists, run only `main` (it calls helpers internally).
    // Otherwise run all zero-param functions (fn test_* style).
    let fns = if all_fns.contains(&"main".to_string()) {
        vec!["main".to_string()]
    } else {
        all_fns
    };

    #[cfg(debug_assertions)]
    if allow_dump && std::env::var("LOFT_DUMP").is_ok() {
        let config = LogConfig::from_env();
        let _ = dump_results(entry.clone(), &mut p_data, types, &mut state, &config)?;
    }

    if debug {
        #[cfg(debug_assertions)]
        {
            let config = LogConfig::from_env();
            let mut w = dump_results(entry, &mut p_data, types, &mut state, &config)?;
            for name in &fns {
                state.execute_log(&mut w, name, &config, &p_data)?;
            }
        }
        #[cfg(not(debug_assertions))]
        for name in &fns {
            state.execute(name, &p_data);
        }
    } else {
        // Run each function with catch_unwind so one failure doesn't abort the rest.
        let mut failures: Vec<String> = Vec::new();
        for name in &fns {
            if std::env::var("LOFT_TEST_VERBOSE").is_ok() {
                eprintln!("  running {path}::{name}");
            }
            let should_fail = file_level_fail || expect_fail.contains(name.as_str());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                state.execute(name, &p_data);
            }));
            let msg_from = |payload: &Box<dyn std::any::Any + Send>| -> String {
                if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = payload.downcast_ref::<&str>() {
                    (*s).to_string()
                } else {
                    "unknown panic".to_string()
                }
            };
            match result {
                Ok(()) if should_fail => {
                    // Bug was fixed — the @EXPECT_FAIL annotation can be removed.
                    println!("  FIXED {path}::{name} (was @EXPECT_FAIL, now passes)");
                }
                Ok(()) => {} // passed as expected
                Err(payload) if should_fail => {
                    println!("  expected fail {path}::{name} — {}", msg_from(&payload));
                }
                Err(payload) => {
                    let msg = msg_from(&payload);
                    println!("  FAIL {path}::{name} — {msg}");
                    failures.push(format!("{name}: {msg}"));
                }
            }
        }
        if !failures.is_empty() {
            return Err(Error::other(format!(
                "{} of {} functions failed in {path}: {}",
                failures.len(),
                fns.len(),
                failures.join("; ")
            )));
        }
        // Check for store leaks after all test functions have run.
        state.check_store_leaks();
    }
    Ok(())
}

/// Write a debug snapshot of a compiled test to `tests/dumps/<filename>.txt`.
///
/// Writes every type definition introduced by the test file (i.e., types beyond
/// those already present in the default library), followed by the full bytecode
/// listing produced by `show_code`.  Returns the open file so the caller can
/// append an execution trace if needed.
#[cfg(debug_assertions)]
fn dump_results(
    entry: PathBuf,
    data: &mut Data,
    types: usize,
    state: &mut State,
    config: &LogConfig,
) -> Result<File, Error> {
    let filename = entry.file_name().unwrap_or_default().to_string_lossy();
    let mut w = File::create(format!("tests/dumps/{filename}.txt"))?;
    for tp in types..state.database.types.len() {
        writeln!(
            &mut w,
            "Type {tp}:{}",
            state.database.show_type(tp as u16, true)
        )?;
    }
    show_code(&mut w, state, data, config)?;
    Ok(w)
}
