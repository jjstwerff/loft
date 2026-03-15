// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

use loft::compile::byte_code;
#[cfg(debug_assertions)]
use loft::compile::show_code;
#[cfg(debug_assertions)]
use loft::data::Data;
#[cfg(debug_assertions)]
use loft::log_config::LogConfig;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
#[cfg(debug_assertions)]
use std::fs::File;
use std::io::Error;
#[cfg(debug_assertions)]
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

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
        .collect();
    files.sort();
    for entry in files {
        run_test(entry, false, false)?;
    }
    Ok(())
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
script_test!(control_flow, "tests/scripts/05-control-flow.loft");
script_test!(functions, "tests/scripts/06-functions.loft");
script_test!(structs, "tests/scripts/07-structs.loft");
script_test!(enums, "tests/scripts/08-enums.loft");
script_test!(vectors, "tests/scripts/09-vectors.loft");
script_test!(collections, "tests/scripts/10-collections.loft");
script_test!(files, "tests/scripts/11-files.loft");
script_test!(binary, "tests/scripts/12-binary.loft");
script_test!(binary_ops, "tests/scripts/13-binary-ops.loft");
script_test!(formatting, "tests/scripts/14-formatting.loft");
script_test!(script_threading, "tests/scripts/15-threading.loft");
script_test!(stress, "tests/scripts/16-stress.loft");
script_test!(map_filter_reduce, "tests/scripts/17-map-filter-reduce.loft");
script_test!(random, "tests/scripts/18-random.loft");
script_test!(min_max_clamp, "tests/scripts/19-min-max-clamp.loft");
script_test!(math_functions, "tests/scripts/20-math-functions.loft");

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
    p.parse_dir("default", true, false).unwrap();
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

/// Parse, type-check, compile, and execute one `.loft` test file.
///
/// The default library in `default/` is loaded first, then `entry` is parsed on
/// top of it.  Any parse or type errors are printed and immediately fail the
/// test.  On success the bytecode is generated and `main` is called.
///
/// When `debug` is true (debug builds only) a human-readable bytecode dump is
/// written to `tests/dumps/<filename>.txt` and the interpreter emits a full
/// execution trace to that file.  Set `LOFT_DUMP=1` to get the bytecode dump
/// without the execution trace for any non-debug test invocation.
fn run_test(entry: PathBuf, debug: bool, allow_dump: bool) -> std::io::Result<()> {
    println!("run {entry:?}");
    let mut p = Parser::new();
    p.parse_dir("default", true, debug)?;
    #[cfg(debug_assertions)]
    let types = p.database.types.len();
    let path = entry.to_string_lossy().to_string();
    p.parse(&path, false);
    for l in p.diagnostics.lines() {
        println!("{l}");
    }
    if !p.diagnostics.is_empty() {
        return Err(Error::from(std::io::ErrorKind::InvalidData));
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
