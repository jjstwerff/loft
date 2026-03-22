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
    println!("  --native-emit <out.rs>        write generated Rust source to <out.rs> and exit");
    println!("  --native-wasm <out.wasm>      compile to WebAssembly (wasm32-wasip2) and write");
    println!("                                the .wasm file to <out.wasm>");
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
    let mut args = env::args_os();
    args.next();
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
    let mut native_emit: Option<String> = None;
    let mut native_wasm: Option<String> = None;
    let mut user_args: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        let a = arg.to_str().unwrap();
        if a == "--version" {
            println!("loft {}", env!("CARGO_PKG_VERSION"));
            return;
        } else if a == "--path" {
            dir = args.next().unwrap().to_str().unwrap().to_string();
        } else if a == "--project" {
            project = Some(args.next().unwrap().to_str().unwrap().to_string());
        } else if a == "--lib" {
            lib_dirs.push(args.next().unwrap().to_str().unwrap().to_string());
        } else if a == "--log-conf" {
            log_conf = Some(args.next().unwrap().to_str().unwrap().to_string());
        } else if a == "--production" {
            production = true;
        } else if a == "--generate-log-config" {
            // Optional path argument: peek at next arg (if it doesn't start with -)
            let next = args.next();
            let path = next.as_ref().and_then(|s| s.to_str()).and_then(|s| {
                if s.starts_with('-') {
                    None
                } else {
                    Some(s.to_string())
                }
            });
            generate_log_config = Some(path);
        } else if a == "--format" {
            let path = args
                .next()
                .map(|s| s.to_str().unwrap().to_string())
                .unwrap_or_default();
            format_mode = Some(("format", path));
        } else if a == "--format-check" {
            let path = args
                .next()
                .map(|s| s.to_str().unwrap().to_string())
                .unwrap_or_default();
            format_mode = Some(("check", path));
        } else if a == "--native" {
            native_mode = true;
        } else if a == "--native-release" {
            native_mode = true;
            native_release = true;
        } else if a == "--native-emit" {
            native_emit = Some(
                args.next()
                    .map(|s| s.to_str().unwrap().to_string())
                    .unwrap_or_default(),
            );
        } else if a == "--native-wasm" {
            native_wasm = Some(
                args.next()
                    .map(|s| s.to_str().unwrap().to_string())
                    .unwrap_or_default(),
            );
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
        if wasm_out.is_empty() {
            eprintln!("loft: --native-wasm requires an output path (e.g. --native-wasm out.wasm)");
            std::process::exit(1);
        }
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
        let emit_path = native_emit.as_deref().map_or_else(
            || std::env::temp_dir().join("loft_native.rs"),
            std::path::PathBuf::from,
        );
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
        // Default: log.conf next to the main loft file
        std::path::Path::new(&abs_file)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("log.conf")
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
            if dir.file_name().map_or(false, |n| n == "bin") {
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
