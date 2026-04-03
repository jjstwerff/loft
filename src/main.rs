// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![warn(clippy::pedantic)]

#[macro_use]
pub mod diagnostics;
mod calc;
mod codegen_runtime;
mod compile;
mod const_eval;
mod data;
mod database;
mod extensions;
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
mod native_utils;
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
mod test_runner;
mod tree;
mod typedef;
mod variables;
mod vector;
#[cfg(feature = "wasm")]
mod wasm;

use crate::diagnostics::Level;
use crate::native_utils::{
    default_artifact_path, is_output_path, loft_lib_dir, loft_lib_dir_for, project_dir,
};
use crate::state::State;
use crate::test_runner::run_tests;
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
    println!("  --tests file.loft             run all tests in a single file");
    println!("  --tests file.loft::name       run a single test function");
    println!("  --tests file.loft::{{a,b}}      run specific test functions");
    println!(
        "  --tests --native              like --tests but compile each file to native Rust and"
    );
    println!("                                run the binary (skips @EXPECT_FAIL tests)");
    println!("  --no-warnings                 suppress warnings in --tests output");
    println!();
    println!("Subcommands:");
    println!("  test [target]                 run package tests (requires loft.toml in cwd)");
    println!("                                test         — run all tests in tests/");
    println!("                                test draw    — run tests/draw.loft");
    println!("                                test draw::f — run a single test function");
    println!("  install [path]                install a package to ~/.loft/lib/ for global use");
    println!("                                install .    — install package in current dir");
    println!("                                install /p   — install package at /p");
    println!("  doc [path]                    generate HTML documentation for a package");
    println!("                                doc          — generate docs for package in cwd");
    println!("                                doc lib/pkg  — generate docs for lib/pkg");
    println!("                                output: <pkg>/doc/*.html");
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
/// PKG.2: Install a local package to ~/.loft/lib/<name>/.
///
/// Reads loft.toml from `pkg_path`, copies src/*.loft and loft.toml to
/// the user's library directory.  The package is then available via `use <name>;`.
fn install_package(pkg_path: &std::path::Path) {
    let manifest_file = pkg_path.join("loft.toml");
    if !manifest_file.exists() {
        println!("loft install: no loft.toml found in {}", pkg_path.display());
        std::process::exit(1);
    }
    let manifest =
        manifest::read_manifest(manifest_file.to_str().unwrap_or("loft.toml")).unwrap_or_default();
    // Derive package name from directory name or manifest entry.
    let pkg_name = pkg_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    if pkg_name.is_empty() {
        println!("loft install: cannot determine package name from path");
        std::process::exit(1);
    }
    // Target: ~/.loft/lib/<name>/
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let target = std::path::Path::new(&home)
        .join(".loft")
        .join("lib")
        .join(&pkg_name);
    // Create target directories.
    let target_src = target.join("src");
    if let Err(e) = std::fs::create_dir_all(&target_src) {
        println!("loft install: cannot create {}: {e}", target_src.display());
        std::process::exit(1);
    }
    // Copy loft.toml.
    if let Err(e) = std::fs::copy(&manifest_file, target.join("loft.toml")) {
        println!("loft install: cannot copy loft.toml: {e}");
        std::process::exit(1);
    }
    // Copy src/*.loft files.
    let src_dir = if let Some(entry) = &manifest.entry {
        pkg_path.join(
            std::path::Path::new(entry)
                .parent()
                .unwrap_or(std::path::Path::new("src")),
        )
    } else {
        pkg_path.join("src")
    };
    let mut copied = 0;
    if let Ok(entries) = std::fs::read_dir(&src_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            {
                let dest = target_src.join(entry.file_name());
                if let Err(e) = std::fs::copy(&path, &dest) {
                    println!("loft install: cannot copy {}: {e}", path.display());
                } else {
                    copied += 1;
                }
            }
        }
    }
    // Copy tests/ if present (for `loft test` on installed packages).
    let tests_dir = pkg_path.join("tests");
    if tests_dir.is_dir() {
        let target_tests = target.join("tests");
        let _ = std::fs::create_dir_all(&target_tests);
        if let Ok(entries) = std::fs::read_dir(&tests_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let _ = std::fs::copy(&path, target_tests.join(entry.file_name()));
                }
            }
        }
    }
    println!(
        "installed {pkg_name} ({copied} source files) → {}",
        target.display()
    );
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
            // Optional directory/file: consume next non-flag arg.
            // Skip --native/--no-warnings that may appear between --tests and the path.
            let mut path = ".".to_string();
            while argv
                .get(i)
                .is_some_and(|s| s == "--native" || s == "--no-warnings")
            {
                if argv[i] == "--native" {
                    native_mode = true;
                } else if argv[i] == "--no-warnings" {
                    no_warnings = true;
                }
                i += 1;
            }
            if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                path.clone_from(&argv[i]);
                i += 1;
            }
            tests_dir = Some(path);
        } else if a == "--no-warnings" {
            no_warnings = true;
        } else if a == "--help" || a == "-h" || a == "-?" {
            print_help();
            return;
        } else if a == "test" {
            // PKG.6: `loft test [target]` — run package tests.
            // Detects loft.toml in cwd, adds src/ to lib path, runs --tests tests/.
            let mut test_target = "tests".to_string();
            if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                // `loft test draw` → tests/draw.loft
                // `loft test draw::test_foo` → tests/draw.loft::test_foo
                let arg = &argv[i];
                if arg.contains("::")
                    || std::path::Path::new(arg.as_str())
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
                {
                    test_target = format!("tests/{arg}");
                } else {
                    test_target = format!("tests/{arg}.loft");
                }
                i += 1;
            }
            // Read loft.toml to find src/ directory.
            let manifest_path = std::path::Path::new("loft.toml");
            if manifest_path.exists() {
                let manifest = crate::manifest::read_manifest("loft.toml").unwrap_or_default();
                let entry = manifest.entry.unwrap_or_else(|| "src".to_string());
                let src_dir = std::path::Path::new(&entry)
                    .parent()
                    .unwrap_or(std::path::Path::new("src"));
                let abs_src = std::env::current_dir()
                    .unwrap_or_default()
                    .join(src_dir)
                    .to_string_lossy()
                    .to_string();
                lib_dirs.push(abs_src);
            } else if std::path::Path::new("src").is_dir() {
                let abs_src = std::env::current_dir()
                    .unwrap_or_default()
                    .join("src")
                    .to_string_lossy()
                    .to_string();
                lib_dirs.push(abs_src);
            }
            tests_dir = Some(test_target);
        } else if a == "install" {
            // PKG.2: `loft install [path]` — install a local package to ~/.loft/lib/
            let pkg_path = if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                std::path::PathBuf::from(&argv[i])
            } else {
                std::env::current_dir().unwrap_or_default()
            };
            install_package(&pkg_path);
            return;
        } else if a == "doc" {
            // PKG.8: `loft doc [path]` — generate HTML docs for a package.
            let pkg_path = if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                std::path::PathBuf::from(&argv[i])
            } else {
                std::env::current_dir().unwrap_or_default()
            };
            if let Err(e) = loft::documentation::generate_pkg_docs(&pkg_path) {
                eprintln!("Error generating docs: {e}");
                std::process::exit(1);
            }
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
        let exit_code = run_tests(
            &dir,
            test_dir,
            no_warnings,
            &lib_dirs,
            project.as_deref(),
            native_mode,
        );
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
            if !l.starts_with("Debug: ") {
                println!("{l}");
            }
        }
        if p.diagnostics.level() >= Level::Error {
            std::process::exit(1);
        }
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    compile::byte_code(&mut state, &mut p.data);
    // A7.2: load native extension shared libraries registered during parsing.
    extensions::load_all(&mut state, std::mem::take(&mut p.pending_native_libs));

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
                next_format_count: 0,
                yield_collect: false,
                fn_ref_context: false,
                call_stack_prefix: None,
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
                next_format_count: 0,
                yield_collect: false,
                fn_ref_context: false,
                call_stack_prefix: None,
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
