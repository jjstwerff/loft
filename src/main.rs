// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![warn(clippy::pedantic)]

#[macro_use]
pub mod diagnostics;
mod calc;
mod compile;
mod data;
mod database;
mod fill;
mod formatter;
mod generation;
mod hash;
mod keys;
mod lexer;
mod log_config;
mod logger;
mod manifest;
mod native;
mod ops;
mod parallel;
mod parser;
mod platform;
#[cfg(feature = "png")]
mod png_store;
mod scopes;
mod stack;
mod state;
mod store;
mod tree;
mod typedef;
mod variables;
mod vector;

use crate::state::State;
use std::collections::HashSet;
use std::env;
use std::sync::{Arc, Mutex};

fn print_help() {
    println!("usage: loft [options] <file>");
    println!("       loft --tests [dir]");
    println!();
    println!("Options:");
    println!("  --version                     print version information");
    println!("  -h, --help, -?                print this help message");
    println!(
        "  --path <dir>                  directory containing the default/ library (default: binary location)"
    );
    println!(
        "  --project <dir>               run the script as if launched from <dir>; file I/O is"
    );
    println!(
        "                                sandboxed there and its lib/ sub-directory is searched"
    );
    println!(
        "                                for 'use' imports (useful when the script lives in /tmp)"
    );
    println!("  --lib <dir>                   add <dir> to the 'use' import search path; may be");
    println!("                                repeated for multiple directories");
    println!("  --log-conf <path>             use this log config file instead of the default");
    println!(
        "  --production                  enable production mode (panic/assert log instead of abort)"
    );
    println!(
        "  --generate-log-config [path]  write a documented config file with defaults and exit"
    );
    println!(
        "  --format <file>               format file in-place (use - to read stdin/write stdout)"
    );
    println!("  --format-check <file>         exit 1 if file is not in canonical format");
    println!("  --native                      compile to native Rust via rustc and run");
    println!("  --native-release              like --native but emit only reachable functions and");
    println!("                                compile with rustc -O (optimised build)");
    println!("  --native-emit [out.rs]        write generated Rust source and exit");
    println!("                                (default: .loft/<script>.rs beside the script)");
    println!("  --native-wasm [out.wasm]      compile to WebAssembly (wasm32-wasip2)");
    println!("                                (default: .loft/<script>.wasm beside the script)");
    println!(
        "  --tests [dir]                 discover and run fn test*() functions in .loft files"
    );
    println!("                                recursively (default dir: current directory)");
    println!("  --no-warnings                 suppress warnings in --tests output");
}

fn handle_generate_log_config(path_opt: Option<&str>) {
    let content = logger::generate_config();
    match path_opt {
        Some(path) => {
            if let Err(e) = std::fs::write(path, content) {
                println!("Error writing config to '{path}': {e}");
                std::process::exit(1);
            }
            println!("Log config written to: {path}");
        }
        None => {
            print!("{content}");
        }
    }
}

#[allow(clippy::too_many_lines)]
fn main() {
    let argv: Vec<String> = env::args_os()
        .skip(1)
        .map(|a| a.to_str().unwrap_or("").to_string())
        .collect();
    let mut i = 0;
    let mut file_name = String::new();
    let mut dir = project_dir();
    let mut project: Option<String> = None;
    let mut lib_dirs: Vec<String> = Vec::new();
    let mut log_conf: Option<String> = None;
    let mut production = false;
    let mut generate_log_config: Option<Option<String>> = None;
    let mut format_mode: Option<(&'static str, String)> = None;
    let mut native_mode = false;
    let mut native_release = false;
    // None  = flag not given
    // Some("") = flag given without explicit path → use .loft/ default
    // Some(path) = explicit output path
    let mut native_emit: Option<String> = None;
    let mut native_wasm: Option<String> = None;
    let mut tests_dir: Option<String> = None;
    let mut no_warnings = false;
    let mut user_args: Vec<String> = Vec::new();

    while i < argv.len() {
        let a = argv[i].as_str();
        i += 1;
        if a == "--version" {
            println!("loft {}", env!("CARGO_PKG_VERSION"));
            return;
        } else if a == "--path" {
            dir.clone_from(&argv[i]);
            i += 1;
        } else if a == "--project" {
            project = Some(argv[i].clone());
            i += 1;
        } else if a == "--lib" {
            lib_dirs.push(argv[i].clone());
            i += 1;
        } else if a == "--log-conf" {
            log_conf = Some(argv[i].clone());
            i += 1;
        } else if a == "--production" {
            production = true;
        } else if a == "--generate-log-config" {
            // Optional path: consume next arg only if it doesn't look like a flag or source file
            let path = if argv.get(i).is_some_and(|s| is_output_path(s)) {
                let p = argv[i].clone();
                i += 1;
                Some(p)
            } else {
                None
            };
            generate_log_config = Some(path);
        } else if a == "--format" {
            let path = argv.get(i).cloned().unwrap_or_default();
            i += 1;
            format_mode = Some(("format", path));
        } else if a == "--format-check" {
            let path = argv.get(i).cloned().unwrap_or_default();
            i += 1;
            format_mode = Some(("check", path));
        } else if a == "--native" {
            native_mode = true;
        } else if a == "--native-release" {
            native_mode = true;
            native_release = true;
        } else if a == "--native-emit" {
            // Optional path: consume next arg only if it looks like an output path
            native_emit = Some(if argv.get(i).is_some_and(|s| is_output_path(s)) {
                let p = argv[i].clone();
                i += 1;
                p
            } else {
                String::new() // sentinel: compute default from file_name later
            });
        } else if a == "--native-wasm" {
            // Optional path: consume next arg only if it looks like an output path
            native_wasm = Some(if argv.get(i).is_some_and(|s| is_output_path(s)) {
                let p = argv[i].clone();
                i += 1;
                p
            } else {
                String::new() // sentinel: compute default from file_name later
            });
        } else if a == "--tests" {
            // Optional directory: consume next arg only if it doesn't look like a flag
            tests_dir = Some(if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                let p = argv[i].clone();
                i += 1;
                p
            } else {
                ".".to_string()
            });
        } else if a == "--no-warnings" {
            no_warnings = true;
        } else if a == "--help" || a == "-h" || a == "-?" {
            print_help();
            return;
        } else if a.starts_with('-') {
            println!("unknown option: {a}");
            println!("usage: loft [options] <file>");
            println!("Try `loft --help` for more information.");
            std::process::exit(1);
        } else if file_name.is_empty() {
            file_name = a.to_string();
        } else {
            user_args.push(a.to_string());
        }
    }
    // Resolve sentinel empty paths to .loft/ defaults now that file_name is known.
    if let Some(ref mut p) = native_wasm
        && p.is_empty()
        && !file_name.is_empty()
    {
        *p = default_artifact_path(&file_name, "wasm")
            .to_str()
            .unwrap_or("out.wasm")
            .to_string();
    }
    if let Some(ref mut p) = native_emit
        && p.is_empty()
        && !file_name.is_empty()
    {
        *p = default_artifact_path(&file_name, "rs")
            .to_str()
            .unwrap_or("out.rs")
            .to_string();
    }

    // Handle --format / --format-check before requiring an input file
    if let Some((mode, path)) = format_mode {
        if path == "-" {
            // stdin → stdout
            use std::io::Read;
            let mut src = String::new();
            std::io::stdin().read_to_string(&mut src).unwrap_or(0);
            print!("{}", formatter::format_source(&src));
        } else if path.is_empty() {
            println!("loft: --{mode} requires a file argument");
            std::process::exit(1);
        } else {
            let src = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    println!("loft: cannot read '{path}': {e}");
                    std::process::exit(1);
                }
            };
            if mode == "check" {
                if !formatter::check_source(&src) {
                    std::process::exit(1);
                }
            } else {
                let formatted = formatter::format_source(&src);
                if let Err(e) = std::fs::write(&path, &formatted) {
                    println!("loft: cannot write '{path}': {e}");
                    std::process::exit(1);
                }
            }
        }
        return;
    }

    // Handle --generate-log-config before requiring an input file
    if let Some(path_opt) = generate_log_config {
        handle_generate_log_config(path_opt.as_deref());
        return;
    }

    // Handle --tests before requiring an input file
    if let Some(ref test_dir) = tests_dir {
        let exit_code = run_tests(&dir, test_dir, no_warnings, &lib_dirs, project.as_deref());
        std::process::exit(exit_code);
    }

    if file_name.is_empty() {
        println!("loft: no input file specified.");
        println!("usage: loft [options] <file>");
        std::process::exit(1);
    }
    // Resolve the script path to absolute before potentially changing directory.
    let abs_file = std::path::Path::new(&file_name)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(&file_name));
    let abs_file = abs_file.to_str().unwrap().to_string();
    // --project: change working directory so file I/O is sandboxed to the project root.
    if let Some(ref proj) = project {
        if let Err(e) = env::set_current_dir(proj) {
            println!("Error: cannot change to project directory '{proj}': {e}");
            std::process::exit(1);
        }
        // Also expose the project's lib/ sub-directory for 'use' imports.
        lib_dirs.insert(
            0,
            std::path::Path::new(proj)
                .join("lib")
                .to_str()
                .unwrap()
                .to_string(),
        );
    }
    let mut p = parser::Parser::new();
    p.lib_dirs = lib_dirs;
    p.parse_dir(&(dir + "default"), true, false).unwrap();
    let start_def = p.data.definitions();
    p.parse(&abs_file, false);
    if !p.diagnostics.is_empty() {
        for l in p.diagnostics.lines() {
            println!("{l}");
        }
        std::process::exit(1);
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    compile::byte_code(&mut state, &mut p.data);

    // WASM codegen pipeline: --native-wasm
    if let Some(ref wasm_out) = native_wasm {
        let wasm_out = if wasm_out.is_empty() {
            default_artifact_path(&abs_file, "wasm")
                .to_str()
                .unwrap_or("out.wasm")
                .to_string()
        } else {
            wasm_out.clone()
        };
        let wasm_out = &wasm_out;
        let end_def = p.data.definitions();
        let rs_path = std::env::temp_dir().join("loft_wasm.rs");
        {
            let mut f = match std::fs::File::create(&rs_path) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!(
                        "loft: cannot write wasm source to '{}': {e}",
                        rs_path.display()
                    );
                    std::process::exit(1);
                }
            };
            let mut out = generation::Output {
                data: &p.data,
                stores: &state.database,
                counter: 0,
                indent: 0,
                def_nr: 0,
                declared: HashSet::new(),
                reachable: HashSet::new(),
                loop_stack: Vec::new(),
            };
            let main_nr = p.data.def_nr("n_main");
            let entry_defs: Vec<u32> = if main_nr < end_def {
                vec![main_nr]
            } else {
                (start_def..end_def).collect()
            };
            if let Err(e) = out.output_native_reachable(&mut f, start_def, end_def, &entry_defs) {
                eprintln!("loft: wasm code generation failed: {e}");
                std::process::exit(1);
            }
        }
        let mut cmd = std::process::Command::new("rustc");
        cmd.arg("--edition=2024")
            .arg("--target")
            .arg("wasm32-wasip2")
            .arg("--crate-type")
            .arg("bin")
            .arg("-O")
            .arg("-o")
            .arg(wasm_out)
            .arg(&rs_path);
        if let Some(lib_dir) = loft_lib_dir_for(Some("wasm32-wasip2")) {
            cmd.arg("--extern")
                .arg(format!("loft={}", lib_dir.join("libloft.rlib").display()));
            cmd.arg("-L").arg(lib_dir.join("deps"));
        }
        let status = cmd.status();
        let _ = std::fs::remove_file(&rs_path);
        match status {
            Ok(s) if s.success() => {}
            Ok(_) => {
                eprintln!(
                    "loft: wasm compilation failed (try --native-emit to inspect the source)"
                );
                std::process::exit(1);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("loft: rustc not found; install the Rust toolchain to use --native-wasm");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("loft: failed to launch rustc: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Native codegen pipeline: --native or --native-emit
    if native_mode || native_emit.is_some() {
        let end_def = p.data.definitions();
        let emit_path = match native_emit.as_deref() {
            None => std::env::temp_dir().join("loft_native.rs"),
            Some("") => default_artifact_path(&abs_file, "rs"),
            Some(p) => std::path::PathBuf::from(p),
        };
        {
            let mut f = match std::fs::File::create(&emit_path) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!(
                        "loft: cannot write native source to '{}': {e}",
                        emit_path.display()
                    );
                    std::process::exit(1);
                }
            };
            let mut out = generation::Output {
                data: &p.data,
                stores: &state.database,
                counter: 0,
                indent: 0,
                def_nr: 0,
                declared: HashSet::new(),
                reachable: HashSet::new(),
                loop_stack: Vec::new(),
            };
            let result = if native_release {
                let main_nr = p.data.def_nr("n_main");
                let entry_defs: Vec<u32> = if main_nr < end_def {
                    vec![main_nr]
                } else {
                    (start_def..end_def).collect()
                };
                out.output_native_reachable(&mut f, start_def, end_def, &entry_defs)
            } else {
                out.output_native(&mut f, 0, end_def)
            };
            if let Err(e) = result {
                eprintln!("loft: native code generation failed: {e}");
                std::process::exit(1);
            }
        }
        if native_emit.is_some() {
            return; // --native-emit: just write the file, don't compile
        }
        // --native / --native-release: compile with rustc and run
        let binary = std::env::temp_dir().join("loft_native_bin");
        let mut cmd = std::process::Command::new("rustc");
        cmd.arg("--edition=2024")
            .arg("-o")
            .arg(&binary)
            .arg(&emit_path);
        if native_release {
            cmd.arg("-O");
        }
        if let Some(lib_dir) = loft_lib_dir() {
            cmd.arg("--extern")
                .arg(format!("loft={}", lib_dir.join("libloft.rlib").display()));
            cmd.arg("-L").arg(lib_dir.join("deps"));
        }
        let status = cmd.status();
        let status = match status {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("loft: rustc not found; install the Rust toolchain to use --native mode");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("loft: failed to launch rustc: {e}");
                std::process::exit(1);
            }
        };
        if !status.success() {
            eprintln!(
                "loft: native compilation failed (codegen bug — try --native-emit to inspect the source)"
            );
            std::process::exit(1);
        }
        let run_status = std::process::Command::new(&binary)
            .args(&user_args)
            .status()
            .unwrap_or_else(|e| {
                eprintln!("loft: failed to run native binary: {e}");
                std::process::exit(1);
            });
        let _ = std::fs::remove_file(&emit_path);
        let _ = std::fs::remove_file(&binary);
        if !run_status.success() {
            std::process::exit(run_status.code().unwrap_or(1));
        }
        return;
    }

    // Initialize the runtime logger
    let conf_path = if let Some(ref cp) = log_conf {
        std::path::PathBuf::from(cp)
    } else {
        // Prefer .loft/log.conf beside the script; fall back to log.conf beside the script.
        let script_dir = std::path::Path::new(&abs_file)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let loft_conf = script_dir.join(".loft").join("log.conf");
        if loft_conf.exists() {
            loft_conf
        } else {
            script_dir.join("log.conf")
        }
    };
    let mut lg = logger::Logger::from_config_file(&conf_path, &abs_file);
    if production {
        lg.config.production = true;
    }
    state.database.logger = Some(Arc::new(Mutex::new(lg)));

    state.execute_argv("main", &p.data, &user_args);
    if state.database.had_fatal {
        std::process::exit(1);
    }
}

/// Discover and run callable functions in `.loft` files under `root_dir`.
///
/// Every zero-parameter user function (and single-`vector<text>` functions when
/// `@ARGS` supplies positional arguments) is treated as a test entry point.
/// Each function runs with a fresh `State` so tests cannot leak heap/store
/// state into each other.
///
/// ## File annotations (header comments)
///
/// ```text
/// // @ARGS: --lib path --production arg1 arg2
/// // @EXPECT_ERROR: substring
/// // @EXPECT_FAIL: substring          (file-level — applies to all fns)
/// ```
///
/// `@EXPECT_FAIL` on the line immediately before a `fn` applies to that
/// function only:
///
/// ```text
/// // @EXPECT_FAIL: out of bounds
/// fn test_bad() { ... }
/// ```
///
/// `@ARGS` supports the same flags as the main CLI (`--lib`, `--project`,
/// `--production`, `--log-conf`); remaining positional tokens are passed as
/// `argv` to the loft program, so scripts that accept `fn main(args:
/// vector<text>)` work like normal UNIX commands.
///
/// Returns exit code: 0 if all tests pass, 1 if any fail.
#[allow(clippy::too_many_lines)]
fn run_tests(
    default_dir: &str,
    root_dir: &str,
    no_warnings: bool,
    lib_dirs: &[String],
    project: Option<&str>,
) -> i32 {
    use crate::data::DefType;
    use std::collections::BTreeMap;

    struct FileResult {
        tests: Vec<(String, bool, Option<String>)>, // (fn_name, passed, fail_msg)
        warnings: Vec<String>,
        errors: Vec<String>,
    }

    // ── Annotations parsed from `// @` header comments ──────────────
    #[derive(Default)]
    struct Annotations {
        /// File-level `@IGNORE` — skip the entire file.
        ignore_file: bool,
        /// Per-function `@IGNORE`: `fn_name` → true.
        ignore_fn: std::collections::HashSet<String>,
        /// File-level `@EXPECT_ERROR` substrings — parse errors containing any pass.
        expect_errors: Vec<String>,
        /// Per-function `@EXPECT_ERROR`: `fn_name` → required substrings.
        expect_errors_fn: std::collections::HashMap<String, Vec<String>>,
        /// File-level `@EXPECT_WARNING` substrings — all must appear in warnings.
        expect_warnings: Vec<String>,
        /// Per-function `@EXPECT_WARNING`: `fn_name` → required substrings.
        expect_warnings_fn: std::collections::HashMap<String, Vec<String>>,
        /// File-level `@EXPECT_FAIL` substrings — every function is expected to
        /// panic with a message containing one of these.
        expect_fail_file: Vec<String>,
        /// Per-function `@EXPECT_FAIL`: `fn_name` → required substrings.
        expect_fail_fn: std::collections::HashMap<String, Vec<String>>,
        /// Extra --lib dirs from @ARGS.
        extra_lib_dirs: Vec<String>,
        /// --project from @ARGS.
        project: Option<String>,
        /// --production from @ARGS.
        production: bool,
        /// --log-conf from @ARGS.
        log_conf: Option<String>,
        /// Positional arguments from @ARGS (passed as argv).
        user_args: Vec<String>,
    }

    /// Scan the raw source for `// @` annotations.  Only comments before the
    /// first non-comment, non-blank line (or before a `fn`/`struct`/`enum`
    /// definition) are considered file-level.  A `// @EXPECT_FAIL` on the
    /// line directly before a `fn <name>` binds to that function.
    fn parse_annotations(src: &str) -> Annotations {
        let mut ann = Annotations::default();
        // Pending annotations not yet bound to a function.
        let mut pending_fail: Vec<String> = Vec::new();
        let mut pending_error: Vec<String> = Vec::new();
        let mut pending_warning: Vec<String> = Vec::new();
        let mut pending_ignore = false;
        // True until we see the first fn/struct/enum definition.
        let mut in_header = true;

        for line in src.lines() {
            let trimmed = line.trim();

            // Check for fn definition — bind pending annotations.
            if trimmed.starts_with("fn ") {
                in_header = false;
                if let Some(name) = trimmed
                    .strip_prefix("fn ")
                    .and_then(|s| s.split(&['(', ' ', '{'][..]).next())
                {
                    let name = name.trim();
                    if !name.is_empty() {
                        if !pending_fail.is_empty() {
                            ann.expect_fail_fn
                                .entry(name.to_string())
                                .or_default()
                                .append(&mut pending_fail);
                        }
                        if !pending_error.is_empty() {
                            ann.expect_errors_fn
                                .entry(name.to_string())
                                .or_default()
                                .append(&mut pending_error);
                        }
                        if !pending_warning.is_empty() {
                            ann.expect_warnings_fn
                                .entry(name.to_string())
                                .or_default()
                                .append(&mut pending_warning);
                        }
                        if pending_ignore {
                            ann.ignore_fn.insert(name.to_string());
                        }
                    }
                }
                pending_fail.clear();
                pending_error.clear();
                pending_warning.clear();
                pending_ignore = false;
                continue;
            }

            // Struct/enum definitions end the header.
            if trimmed.starts_with("struct ") || trimmed.starts_with("enum ") {
                in_header = false;
                pending_ignore = false;
                pending_fail.clear();
                pending_error.clear();
                pending_warning.clear();
                continue;
            }

            // Only process // comments.
            let Some(comment) = trimmed.strip_prefix("//") else {
                // Non-comment, non-blank line.
                pending_fail.clear();
                pending_error.clear();
                pending_warning.clear();
                pending_ignore = false;
                continue;
            };
            let comment = comment.trim();

            if let Some(rest) = comment.strip_prefix("@EXPECT_ERROR:") {
                let sub = rest.trim();
                if !sub.is_empty() {
                    pending_error.push(sub.to_string());
                }
            } else if let Some(rest) = comment.strip_prefix("@EXPECT_WARNING:") {
                let sub = rest.trim();
                if !sub.is_empty() {
                    pending_warning.push(sub.to_string());
                }
            } else if let Some(rest) = comment.strip_prefix("@EXPECT_FAIL:") {
                let sub = rest.trim();
                if !sub.is_empty() {
                    pending_fail.push(sub.to_string());
                }
            } else if comment.starts_with("@IGNORE") {
                if in_header {
                    ann.ignore_file = true;
                } else {
                    pending_ignore = true;
                }
            } else if let Some(rest) = comment.strip_prefix("@ARGS:") {
                parse_args_annotation(rest.trim(), &mut ann);
            }
        }
        // Any pending annotations not followed by a fn → file-level.
        ann.expect_fail_file.append(&mut pending_fail);
        ann.expect_errors.append(&mut pending_error);
        ann.expect_warnings.append(&mut pending_warning);
        if pending_ignore {
            ann.ignore_file = true;
        }
        ann
    }

    /// Parse the token list after `@ARGS:` using the same flag convention as
    /// the main CLI.  Unknown flags are silently ignored so that future flags
    /// don't break old test files.
    fn parse_args_annotation(s: &str, ann: &mut Annotations) {
        let tokens: Vec<&str> = s.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            let t = tokens[i];
            i += 1;
            if t == "--lib" {
                if let Some(&dir) = tokens.get(i) {
                    ann.extra_lib_dirs.push(dir.to_string());
                    i += 1;
                }
            } else if t == "--project" {
                if let Some(&dir) = tokens.get(i) {
                    ann.project = Some(dir.to_string());
                    i += 1;
                }
            } else if t == "--production" {
                ann.production = true;
            } else if t == "--log-conf" {
                if let Some(&path) = tokens.get(i) {
                    ann.log_conf = Some(path.to_string());
                    i += 1;
                }
            } else if t.starts_with('-') {
                // Unknown flag — skip (and consume a value arg if present).
                if tokens.get(i).is_some_and(|s| !s.starts_with('-')) {
                    i += 1;
                }
            } else {
                // Positional argument — this and all remaining tokens are user args.
                ann.user_args.push(t.to_string());
                for &rest in &tokens[i..] {
                    ann.user_args.push(rest.to_string());
                }
                break;
            }
        }
    }

    // Recursively collect .loft files grouped by directory.
    fn collect_loft_files(
        dir: &std::path::Path,
        out: &mut BTreeMap<String, Vec<std::path::PathBuf>>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files = Vec::new();
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip hidden directories and .loft artifact dirs
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if !name.starts_with('.') {
                    subdirs.push(path);
                }
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            {
                files.push(path);
            }
        }
        files.sort();
        if !files.is_empty() {
            let dir_key = dir.to_string_lossy().to_string();
            out.insert(dir_key, files);
        }
        subdirs.sort();
        for sub in subdirs {
            collect_loft_files(&sub, out);
        }
    }

    fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
        if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else {
            "unknown panic".to_string()
        }
    }

    /// Check whether `msg` satisfies the expected-fail substrings for `fn_name`.
    /// Returns true when the panic message contains at least one required
    /// substring (file-level or per-function).
    fn matches_expect_fail(ann: &Annotations, fn_name: &str, msg: &str) -> bool {
        // Per-function annotations take priority.
        if let Some(subs) = ann.expect_fail_fn.get(fn_name) {
            return subs.iter().any(|s| msg.contains(s.as_str()));
        }
        // Fall back to file-level.
        if !ann.expect_fail_file.is_empty() {
            return ann
                .expect_fail_file
                .iter()
                .any(|s| msg.contains(s.as_str()));
        }
        false
    }

    // Suppress the default panic hook output ("thread 'main' panicked at ...").
    // All panics inside the runner are caught by catch_unwind; we extract the
    // message from the payload and report it ourselves in the test summary.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let root = std::path::Path::new(root_dir);
    if !root.is_dir() {
        std::panic::set_hook(prev_hook);
        println!("loft: --tests directory '{root_dir}' does not exist");
        return 1;
    }

    let mut dirs: BTreeMap<String, Vec<std::path::PathBuf>> = BTreeMap::new();
    collect_loft_files(root, &mut dirs);

    if dirs.is_empty() {
        std::panic::set_hook(prev_hook);
        println!("loft: no .loft files found in '{root_dir}'");
        return 1;
    }

    // Build the project lib path once, if --project was supplied on the CLI.
    let project_lib: Option<String> = project.map(|proj| {
        std::path::Path::new(proj)
            .join("lib")
            .to_str()
            .unwrap_or("")
            .to_string()
    });

    let mut total_pass = 0u32;
    let mut total_fail = 0u32;
    let mut total_files = 0u32;
    let mut dir_summaries: Vec<(String, u32, u32)> = Vec::new(); // (dir, pass, fail)

    for (dir_path, files) in &dirs {
        let mut dir_pass = 0u32;
        let mut dir_fail = 0u32;

        for file_path in files {
            let abs_file = file_path
                .canonicalize()
                .unwrap_or_else(|_| file_path.clone())
                .to_str()
                .unwrap_or("")
                .to_string();
            let display_name = file_path.to_string_lossy();

            // Read the raw source to extract annotations before parsing.
            let source = match std::fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(e) => {
                    println!("  FAIL  {display_name}  (cannot read: {e})");
                    dir_fail += 1;
                    total_files += 1;
                    continue;
                }
            };
            let ann = parse_annotations(&source);
            if ann.ignore_file {
                continue; // silently skip ignored files
            }
            let has_expect_error = !ann.expect_errors.is_empty();

            // Build parser with CLI lib_dirs + @ARGS lib dirs.
            let mut p = parser::Parser::new();
            p.lib_dirs = lib_dirs.to_vec();
            if let Some(ref pl) = project_lib {
                p.lib_dirs.insert(0, pl.clone());
            }
            for extra in &ann.extra_lib_dirs {
                p.lib_dirs.push(extra.clone());
            }
            if let Some(ref proj) = ann.project {
                p.lib_dirs.insert(
                    0,
                    std::path::Path::new(proj)
                        .join("lib")
                        .to_str()
                        .unwrap_or("")
                        .to_string(),
                );
            }
            if p.parse_dir(&(default_dir.to_string() + "default"), true, false)
                .is_err()
            {
                println!("  FAIL  {display_name}  (cannot load default library)");
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            let start_def = p.data.definitions();
            let parse_ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                p.parse(&abs_file, false);
            }));
            if let Err(payload) = parse_ok {
                let msg = panic_message(&*payload);
                if has_expect_error && ann.expect_errors.iter().any(|s| msg.contains(s.as_str())) {
                    println!("  ok    {display_name}  (expected parse error)");
                    total_files += 1;
                    dir_pass += 1;
                } else {
                    println!("  FAIL  {display_name}  (parse panic: {msg})");
                    dir_fail += 1;
                    total_files += 1;
                }
                continue;
            }

            // Collect diagnostics.
            let mut file_result = FileResult {
                tests: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
            };
            for line in p.diagnostics.lines() {
                if line.starts_with("Warning:") {
                    file_result.warnings.push(line.clone());
                } else {
                    file_result.errors.push(line.clone());
                }
            }

            let has_fn_errors = !ann.expect_errors_fn.is_empty();
            let has_fn_warnings = !ann.expect_warnings_fn.is_empty();
            let all_warnings = file_result.warnings.join("\n");

            // Per-function @EXPECT_ERROR: consume errors matching each function's
            // expected substrings.  Track which functions had their errors satisfied.
            let mut fn_error_pass: Vec<String> = Vec::new();
            let mut fn_error_fail: Vec<String> = Vec::new();
            if has_fn_errors {
                for fn_name in ann.expect_errors_fn.keys() {
                    if file_result.errors.is_empty() {
                        fn_error_fail.push(fn_name.clone());
                    } else {
                        // The file has errors.  Substring validation happens
                        // via the unexpected_errors filter below — any error
                        // not matching ANY annotation is rejected there.
                        fn_error_pass.push(fn_name.clone());
                    }
                }
            }

            // Per-function @EXPECT_WARNING: same logic.
            let mut fn_warning_pass: Vec<String> = Vec::new();
            let mut fn_warning_fail: Vec<String> = Vec::new();
            if has_fn_warnings {
                for (fn_name, subs) in &ann.expect_warnings_fn {
                    let matched = subs.iter().all(|s| all_warnings.contains(s.as_str()));
                    if matched {
                        fn_warning_pass.push(fn_name.clone());
                    } else {
                        fn_warning_fail.push(fn_name.clone());
                    }
                }
            }

            // Determine which errors are "unexpected" — not matched by any per-function
            // or file-level annotation.
            let unexpected_errors: Vec<&String> = if has_fn_errors || has_expect_error {
                file_result
                    .errors
                    .iter()
                    .filter(|e| {
                        // Consumed by a per-function annotation?
                        let fn_consumed = ann
                            .expect_errors_fn
                            .values()
                            .any(|subs| subs.iter().any(|s| e.contains(s.as_str())));
                        // Consumed by file-level annotation?
                        let file_consumed =
                            ann.expect_errors.iter().any(|s| e.contains(s.as_str()));
                        !fn_consumed && !file_consumed
                    })
                    .collect()
            } else {
                file_result.errors.iter().collect()
            };

            if !unexpected_errors.is_empty() {
                for e in &unexpected_errors {
                    println!("  {e}");
                }
                if !no_warnings {
                    for w in &file_result.warnings {
                        println!("  {w}");
                    }
                }
                println!("  FAIL  {display_name}  (parse errors)");
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            // File-level @EXPECT_ERROR: if set but no errors matched, fail.
            if has_expect_error && file_result.errors.is_empty() {
                println!("  FAIL  {display_name}  (expected parse error but file parsed cleanly)");
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            // Per-function @EXPECT_ERROR that expected errors but none appeared.
            if !fn_error_fail.is_empty() && file_result.errors.is_empty() {
                println!(
                    "  FAIL  {display_name}  (expected errors for: {})",
                    fn_error_fail.join(", ")
                );
                dir_fail += 1;
                total_files += 1;
                continue;
            }

            // Check @EXPECT_WARNING (file-level): all substrings must match.
            let has_expect_warning = !ann.expect_warnings.is_empty();
            if has_expect_warning {
                let all_matched = ann
                    .expect_warnings
                    .iter()
                    .all(|sub| all_warnings.contains(sub.as_str()));
                if !all_matched {
                    let missing: Vec<&str> = ann
                        .expect_warnings
                        .iter()
                        .filter(|sub| !all_warnings.contains(sub.as_str()))
                        .map(String::as_str)
                        .collect();
                    for w in &file_result.warnings {
                        println!("  {w}");
                    }
                    println!(
                        "  FAIL  {display_name}  (expected warning not found: {})",
                        missing.join(", ")
                    );
                    dir_fail += 1;
                    total_files += 1;
                    continue;
                }
            }
            // Per-function @EXPECT_WARNING failures.
            if !fn_warning_fail.is_empty() {
                println!(
                    "  FAIL  {display_name}  (expected warnings not found for: {})",
                    fn_warning_fail.join(", ")
                );
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            if !no_warnings && !has_expect_warning && !has_fn_warnings {
                for w in &file_result.warnings {
                    println!("  {w}");
                }
            }

            // If the file has errors that were all expected (file-level or
            // per-function), report the passes and skip execution — the
            // compiler can't produce valid bytecode for a file with errors.
            if !file_result.errors.is_empty() {
                // All errors consumed → success.
                total_files += 1;
                for name in &fn_error_pass {
                    file_result.tests.push((name.clone(), true, None));
                    dir_pass += 1;
                }
                for name in &fn_warning_pass {
                    if !fn_error_pass.contains(name) {
                        file_result.tests.push((name.clone(), true, None));
                        dir_pass += 1;
                    }
                }
                let fn_names: Vec<&str> = file_result
                    .tests
                    .iter()
                    .map(|(n, _, _)| n.as_str())
                    .collect();
                let fn_list = fn_names.join(", ");
                let count = file_result.tests.len();
                println!(
                    "  ok    {display_name}  ({count} expected error{}: {fn_list})",
                    if count == 1 { "" } else { "s" }
                );
                continue;
            }
            // File-level @EXPECT_ERROR set but no errors at all → fail.
            if has_expect_error {
                println!("  FAIL  {display_name}  (expected parse error but file parsed cleanly)");
                dir_fail += 1;
                total_files += 1;
                continue;
            }
            // Per-function @EXPECT_ERROR but no errors at all → fail.
            if has_fn_errors && fn_error_fail.is_empty() && fn_error_pass.is_empty() {
                println!("  FAIL  {display_name}  (expected errors but file parsed cleanly)");
                dir_fail += 1;
                total_files += 1;
                continue;
            }

            // Find callable entry points: zero-parameter user functions, plus
            // single-vector-parameter functions when @ARGS provides argv.
            let has_user_args = !ann.user_args.is_empty();
            let mut test_fns: Vec<(u32, String)> = Vec::new();
            for d_nr in start_def..p.data.definitions() {
                let def = p.data.def(d_nr);
                if !matches!(def.def_type, DefType::Function) {
                    continue;
                }
                // Only user functions (n_<name>); skip generated lambdas.
                if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
                    continue;
                }
                // Skip standard library / operators.
                if def.position.file.starts_with("default/")
                    || def.position.file.starts_with("default\\")
                {
                    continue;
                }
                // Zero parameters — always a test entry point.
                // Single vector<…> parameter — entry point when @ARGS provides argv.
                let attrs = &def.attributes;
                let callable = attrs.is_empty()
                    || (has_user_args
                        && attrs.len() == 1
                        && matches!(attrs[0].typedef, crate::data::Type::Vector(_, _)));
                if !callable {
                    continue;
                }
                let user_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
                test_fns.push((d_nr, user_name.to_string()));
            }

            if test_fns.is_empty() {
                // No callable functions found; skip this file silently.
                continue;
            }

            // Scope analysis — wrap in catch_unwind so a panic here doesn't
            // kill the entire runner.
            let scopes_ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                scopes::check(&mut p.data);
            }));
            if let Err(payload) = scopes_ok {
                let msg = panic_message(&*payload);
                println!("  FAIL  {display_name}  (scope check panic: {msg})");
                dir_fail += 1;
                total_files += 1;
                continue;
            }

            // Save the checked Data and raw Stores so each test function gets a
            // fresh State.  Stores::clone() preserves the type schema but resets
            // runtime allocations — State::new + compile::byte_code reinitialise
            // everything, giving each function a clean heap.
            let clean_data = p.data;
            let clean_db = p.database;

            total_files += 1;

            for (_, fn_name) in &test_fns {
                // Per-function @IGNORE: skip without running.
                if ann.ignore_fn.contains(fn_name.as_str()) {
                    file_result
                        .tests
                        .push((fn_name.clone(), true, Some("ignored".to_string())));
                    continue;
                }
                // Per-function @EXPECT_ERROR: already counted, don't execute.
                if ann.expect_errors_fn.contains_key(fn_name.as_str()) {
                    continue;
                }
                let fn_name_owned = fn_name.clone();
                let user_args = ann.user_args.clone();
                let production = ann.production;
                let log_conf = ann.log_conf.clone();

                // Build a fresh State + bytecode for every function so tests
                // within a file cannot leak heap/store state into each other.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut data_copy = clean_data.clone();
                    let mut state = State::new(clean_db.clone());
                    compile::byte_code(&mut state, &mut data_copy);

                    // Set up logger if @ARGS requested --production or --log-conf.
                    if production || log_conf.is_some() {
                        let lg = if let Some(ref conf) = log_conf {
                            let cp = std::path::PathBuf::from(conf);
                            logger::Logger::from_config_file(&cp, &abs_file)
                        } else {
                            logger::Logger::production()
                        };
                        let mut lg = lg;
                        if production {
                            lg.config.production = true;
                        }
                        state.database.logger =
                            Some(std::sync::Arc::new(std::sync::Mutex::new(lg)));
                    }

                    state.execute_argv(&fn_name_owned, &data_copy, &user_args);
                }));

                // Evaluate pass/fail, respecting @EXPECT_FAIL annotations.
                // A function "should fail" when it has a per-function
                // @EXPECT_FAIL, or when a file-level @EXPECT_FAIL applies
                // (and no per-function annotation overrides it).
                let should_fail = ann.expect_fail_fn.contains_key(fn_name.as_str())
                    || (!ann.expect_fail_file.is_empty()
                        && !ann.expect_fail_fn.contains_key(fn_name.as_str()));
                let (passed, fail_msg) = match result {
                    Ok(()) => {
                        if should_fail {
                            (
                                false,
                                Some("expected panic but function returned cleanly".to_string()),
                            )
                        } else {
                            (true, None)
                        }
                    }
                    Err(payload) => {
                        let msg = panic_message(&*payload);
                        if should_fail && matches_expect_fail(&ann, fn_name, &msg) {
                            (true, None) // expected failure — pass
                        } else {
                            (false, Some(msg))
                        }
                    }
                };

                file_result.tests.push((fn_name.clone(), passed, fail_msg));
                if passed {
                    dir_pass += 1;
                } else {
                    dir_fail += 1;
                }
            }

            // Per-file summary line.
            let ignored_count = file_result
                .tests
                .iter()
                .filter(|(_, _, m)| m.as_deref() == Some("ignored"))
                .count();
            let pass_count = file_result
                .tests
                .iter()
                .filter(|(_, p, m)| *p && m.as_deref() != Some("ignored"))
                .count();
            let fail_count = file_result.tests.len() - pass_count - ignored_count;
            let fn_names: Vec<&str> = file_result
                .tests
                .iter()
                .filter(|(_, _, m)| m.as_deref() != Some("ignored"))
                .map(|(n, _, _)| n.as_str())
                .collect();
            let fn_list = fn_names.join(", ");
            if fail_count == 0 {
                let ignore_note = if ignored_count > 0 {
                    format!(", {ignored_count} ignored")
                } else {
                    String::new()
                };
                println!(
                    "  ok    {display_name}  ({pass_count} fn{}{ignore_note}: {fn_list})",
                    if pass_count == 1 { "" } else { "s" }
                );
            } else {
                for (name, passed, msg) in &file_result.tests {
                    if !passed {
                        if let Some(m) = msg {
                            println!("  FAIL  {display_name}::{name}  —  {m}");
                        } else {
                            println!("  FAIL  {display_name}::{name}");
                        }
                    }
                }
                println!("  FAIL  {display_name}  ({fail_count} failed, {pass_count} passed)");
            }
        }

        // Per-directory summary.
        if dir_pass + dir_fail > 0 {
            dir_summaries.push((dir_path.clone(), dir_pass, dir_fail));
            total_pass += dir_pass;
            total_fail += dir_fail;
        }
    }

    // Restore the default panic hook.
    std::panic::set_hook(prev_hook);

    // Final summary.
    println!();
    if dir_summaries.len() > 1 {
        for (dir_path, pass, fail) in &dir_summaries {
            if *fail == 0 {
                println!("  {dir_path}: {pass} passed");
            } else {
                println!("  {dir_path}: {fail} failed, {pass} passed");
            }
        }
        println!();
    }

    let total = total_pass + total_fail;
    if total_fail == 0 {
        println!(
            "test result: ok. {total_pass} passed; {total_files} file{}",
            if total_files == 1 { "" } else { "s" }
        );
        0
    } else {
        println!(
            "test result: FAILED. {total_fail} failed; {total_pass} passed; {total} total; {total_files} file{}",
            if total_files == 1 { "" } else { "s" }
        );
        1
    }
}

fn with_trailing_sep(p: &std::path::Path) -> String {
    let mut s = p.to_str().unwrap_or("").to_string();
    if !s.ends_with('/') && !s.ends_with('\\') {
        s.push(std::path::MAIN_SEPARATOR);
    }
    s
}

/// Return the directory that contains `libloft.rlib` for the given target triple.
/// Pass `None` for the native target, `Some("wasm32-wasip2")` for WASM.
/// Returns `None` when the rlib cannot be located.
fn loft_lib_dir_for(target: Option<&str>) -> Option<std::path::PathBuf> {
    let exe_dir = env::current_exe().ok()?.parent()?.to_path_buf();
    // Dev layout: <project>/target/release/loft  or  <project>/target/debug/loft
    // The wasm rlib lives at <project>/target/wasm32-wasip2/release/
    if let Some(triple) = target {
        // Walk up to find a sibling target/<triple>/release directory.
        let mut dir = exe_dir.clone();
        loop {
            let candidate = dir.join("target").join(triple).join("release");
            if candidate.join("libloft.rlib").exists() {
                return Some(candidate);
            }
            // Installed layout: <prefix>/share/loft/<triple>/
            if dir.file_name().is_some_and(|n| n == "bin") {
                let share = dir.parent()?.join("share").join("loft").join(triple);
                if share.join("libloft.rlib").exists() {
                    return Some(share);
                }
            }
            if !dir.pop() {
                break;
            }
        }
        return None;
    }
    // Native: look next to the binary first (dev build in target/release/).
    if exe_dir.join("libloft.rlib").exists() {
        return Some(exe_dir.clone());
    }
    // Installed as <prefix>/bin/loft — look in <prefix>/share/loft/.
    if exe_dir.file_name()? == "bin" {
        let share = exe_dir.parent()?.join("share").join("loft");
        if share.join("libloft.rlib").exists() {
            return Some(share);
        }
    }
    None
}

fn loft_lib_dir() -> Option<std::path::PathBuf> {
    loft_lib_dir_for(None)
}

/// Return true if `s` looks like an explicit output path rather than a flag or loft source file.
fn is_output_path(s: &str) -> bool {
    !s.starts_with('-')
        && !std::path::Path::new(s)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("loft"))
}

/// Return (and create) the `.loft/` artifact directory beside `script_path`.
/// Falls back to the current directory's `.loft/` if the parent cannot be determined.
fn loft_artifact_dir(script_path: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(script_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let loft_dir = dir.join(".loft");
    let _ = std::fs::create_dir_all(&loft_dir);
    loft_dir
}

/// Return the default output path for a compiled artifact beside `script_path`.
/// `ext` is the file extension without leading dot (e.g. `"wasm"`, `"rs"`).
fn default_artifact_path(script_path: &str, ext: &str) -> std::path::PathBuf {
    let stem = std::path::Path::new(script_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out");
    loft_artifact_dir(script_path).join(format!("{stem}.{ext}"))
}

fn project_dir() -> String {
    let Ok(prog) = env::current_exe() else {
        return String::new();
    };
    let Some(dir) = prog.parent() else {
        return String::new();
    };
    // Strip target/release or target/debug to get the project root.
    if (dir.ends_with("target/release") || dir.ends_with("target\\release"))
        && let Some(root) = dir.parent().and_then(|p| p.parent())
    {
        return with_trailing_sep(root);
    }
    if (dir.ends_with("target/debug") || dir.ends_with("target\\debug"))
        && let Some(root) = dir.parent().and_then(|p| p.parent())
    {
        return with_trailing_sep(root);
    }
    // Installed binary: binary is in <prefix>/bin/, stdlib in <prefix>/share/loft/.
    if dir.ends_with("bin")
        && let Some(prefix) = dir.parent()
    {
        let share_loft = prefix.join("share").join("loft");
        if share_loft.is_dir() {
            return with_trailing_sep(&share_loft);
        }
        return with_trailing_sep(prefix);
    }
    with_trailing_sep(dir)
}
