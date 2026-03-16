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
        lib_dirs.insert(0, format!("{proj}/lib"));
    }
    let mut p = parser::Parser::new();
    p.lib_dirs = lib_dirs;
    p.parse_dir(&(dir + "default"), true, false).unwrap();
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

fn project_dir() -> String {
    let direct = if let Ok(prog) = env::current_exe() {
        prog.to_str().unwrap().to_string()
    } else {
        String::new()
    };
    let mut dir = if direct.ends_with("loft") {
        &direct[0..direct.len() - 8]
    } else {
        &direct
    };
    if dir.ends_with("target/release/") {
        dir = &dir[..dir.len() - 15];
    }
    if dir.ends_with("target/debug/") {
        dir = &dir[..dir.len() - 13];
    }
    dir.to_string()
}
