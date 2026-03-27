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
    // (PROBLEMS #27 set_int crash and #37 LIFO panic both fixed 2026-03-15)
    // (PROBLEMS #41 inline ref-returning calls leak stores fixed 2026-03-15)
    // (PROBLEMS #42 generate_call size mismatch fixed 2026-03-16)
];

/// Docs files that are known to fail in `--native-wasm` mode.
const WASM_SKIP: &[&str] = &[
    "06-function.loft",  // #77: CallRef not implemented
    "13-file.loft",      // #74: file I/O ops missing; also no WASM filesystem
    "18-locks.loft",     // todo!()
    "19-threading.loft", // todo!(); WASM threading model differs
    "21-random.loft",    // #79: external crate
    "22-time.loft",      // todo!()
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
        return Err(Error::from(std::io::ErrorKind::InvalidData));
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
        // Only run files that have fn main() (01-22 doc/demo scripts).
        // Test-style scripts (fn test_*) are exercised by `loft --tests`.
        .filter(|p| {
            std::fs::read_to_string(p)
                .map(|s| s.contains("\nfn main("))
                .unwrap_or(false)
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
        // C28: slot conflict between rv and _read_34 in n_main — pre-existing slot regression.
        "20-binary.loft",
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
script_test!(random, "tests/scripts/15-random.loft");
script_test!(min_max_clamp, "tests/scripts/17-min-max-clamp.loft");
script_test!(math_functions, "tests/scripts/18-math-functions.loft");
script_test!(files, "tests/scripts/19-files.loft");
#[test]
#[ignore = "C28: slot conflict between rv and _read_34 in n_main — pre-existing slot regression; see CAVEATS.md C28"]
fn binary() -> std::io::Result<()> {
    let _g = WRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    run_test(PathBuf::from("tests/scripts/20-binary.loft"), false, false)
}
script_test!(binary_ops, "tests/scripts/21-binary-ops.loft");
script_test!(script_threading, "tests/scripts/22-threading.loft");
script_test!(stress, "tests/scripts/37-stress.loft");

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
/// Ignored by default — the execution trace takes ~100 s.
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

/// Validate diagnostics against `// #warn` declarations in the script source.
///
/// Returns `Ok(())` when every diagnostic is either:
/// - a `Warning:` line whose message matches a declared `// #warn` pattern, or
/// - absent (no unexpected diagnostics remain).
///
/// Returns `Err` when any diagnostic is unexpected or any declared warning was
/// not emitted.  All mismatches are printed before returning.
fn check_diagnostics(diagnostics: &[String], expected: &[String]) -> std::io::Result<()> {
    let mut unmatched_expected: Vec<&str> = expected.iter().map(String::as_str).collect();
    let mut unexpected: Vec<&str> = Vec::new();

    for diag in diagnostics {
        if diag.starts_with("Warning: ") {
            if let Some(pos) = unmatched_expected
                .iter()
                .position(|pat| diag.contains(*pat))
            {
                println!("expected warning matched: {diag}");
                unmatched_expected.remove(pos);
            } else {
                println!("unexpected warning: {diag}");
                unexpected.push(diag);
            }
        } else {
            println!("unexpected diagnostic: {diag}");
            unexpected.push(diag);
        }
    }
    for pat in &unmatched_expected {
        println!("expected warning not emitted: {pat}");
    }
    if unexpected.is_empty() && unmatched_expected.is_empty() {
        Ok(())
    } else {
        Err(Error::from(std::io::ErrorKind::InvalidData))
    }
}

/// Compile and run a single `.loft` script test.
///
/// Scripts may declare expected compile-time warnings with `// #warn <text>`
/// comments.  Each such comment consumes one matching `Warning:` diagnostic.
/// Unexpected diagnostics (errors or unmatched warnings) fail the test.
///
/// Any parse or type errors are printed and immediately fail the test.
/// On success the bytecode is generated and `main` is called.
///
/// When `debug` is true (debug builds only) a human-readable bytecode dump is
/// written to `tests/dumps/<filename>.txt` and the interpreter emits a full
/// execution trace to that file.  Set `LOFT_DUMP=1` to get the bytecode dump
/// without the execution trace for any non-debug test invocation.
fn run_test(entry: PathBuf, debug: bool, allow_dump: bool) -> std::io::Result<()> {
    println!("run {entry:?}");
    let source = std::fs::read_to_string(&entry)?;
    let expected = expected_warnings(&source);
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    #[cfg(debug_assertions)]
    let types = p.database.types.len();
    let path = entry.to_string_lossy().to_string();
    p.parse(&path, false);
    if !p.diagnostics.is_empty() {
        check_diagnostics(p.diagnostics.lines(), &expected)?;
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    if debug {
        #[cfg(debug_assertions)]
        {
            let config = LogConfig::from_env();
            let mut w = dump_results(entry, &mut p.data, types, &mut state, &config)?;
            state.execute_log(&mut w, "main", &config, &p.data)?;
        }
        #[cfg(not(debug_assertions))]
        state.execute("main", &p.data);
    } else {
        #[cfg(debug_assertions)]
        if allow_dump && std::env::var("LOFT_DUMP").is_ok() {
            let config = LogConfig::from_env();
            let _ = dump_results(entry, &mut p.data, types, &mut state, &config)?;
        }
        state.execute("main", &p.data);
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
