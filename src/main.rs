// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![warn(clippy::pedantic)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::match_same_arms,
    clippy::collapsible_if,
    clippy::redundant_closure,
    clippy::used_underscore_binding,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::single_match_else,
    clippy::if_not_else,
    clippy::implicit_hasher,
    clippy::unnecessary_wraps,
    clippy::semicolon_if_nothing_returned,
    clippy::uninlined_format_args,
    clippy::let_underscore_untyped,
    clippy::must_use_candidate,
    clippy::option_if_let_else,
    clippy::manual_let_else,
    clippy::redundant_closure_for_method_calls,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::map_unwrap_or,
    clippy::format_push_string,
    clippy::map_entry
)]

#[macro_use]
pub mod diagnostics;
mod base64;
mod cache;
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
mod sha256;
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
    println!(
        "  --interpret                   run in interpreter/bytecode mode (native is default)"
    );
    println!("  --native                      compile to native Rust via rustc and run (default)");
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
    println!(
        "  --check                       parse and compile only; report errors without running
                                can be combined with --native to also verify rustc compilation"
    );
    println!();
    println!("Subcommands:");
    println!("  check <file>                  same as --check <file>");
    println!("  test [target]                 run package tests (requires loft.toml in cwd)");
    println!("                                test         — run all tests in tests/");
    println!("                                test draw    — run tests/draw.loft");
    println!("                                test draw::f — run a single test function");
    println!("  install [target]              install a package to ~/.loft/lib/ for global use");
    println!("                                install .        — install package in current dir");
    println!("                                install /p       — install package at /p");
    println!("                                install name     — download latest from registry");
    println!("                                install name@v   — download specific version");
    println!("  registry <subcommand>         manage the local package registry");
    println!(
        "                                sync             — pull latest registry from source URL"
    );
    println!(
        "                                check            — report updates, deprecations, yanks"
    );
    println!("                                list             — browse all packages in registry");
    println!("                                list --installed — show only installed packages");
    println!("  generate [path]               generate Rust stubs for #native declarations");
    println!("                                writes native/src/generated.rs in the package");
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

/// REG.2: Install a package from the registry by name (optionally with `@version`).
#[cfg(feature = "registry")]
fn install_from_registry(arg: &str) {
    use loft::registry;

    // Parse name[@version].
    let (name, version) = if let Some((n, v)) = arg.split_once('@') {
        (n, Some(v))
    } else {
        (arg, None)
    };

    // Find and read registry file.
    let Some(reg_path) = registry::registry_path() else {
        eprintln!(
            "loft install: no registry file found.\n  \
             Run 'loft registry sync' to download the package registry.\n  \
             Or set LOFT_REGISTRY to a local registry file path."
        );
        std::process::exit(1);
    };
    let (entries, _) = registry::read_registry(reg_path.to_str().unwrap_or(""));

    // Find matching entry.
    let Some(entry) = registry::find_package(&entries, name, version) else {
        let available: Vec<&str> = entries
            .iter()
            .filter(|e| e.name == name && !e.is_yanked())
            .map(|e| e.version.as_str())
            .collect();
        if available.is_empty() {
            eprintln!("loft install: package '{name}' not found in registry.");
        } else {
            eprintln!(
                "loft install: package '{name}@{}' not found in registry.\n  Available versions: {}",
                version.unwrap_or("?"),
                available.join(", ")
            );
        }
        std::process::exit(1);
    };

    if entry.is_yanked() && version.is_some() {
        eprintln!(
            "warning: {name}@{} is yanked ({})",
            entry.version,
            entry.status_slug()
        );
    }

    // Check if already installed.
    let lib = registry::lib_dir();
    let installed_toml = lib.join(name).join("loft.toml");
    if installed_toml.exists()
        && let Ok(content) = std::fs::read_to_string(&installed_toml)
    {
        let installed_ver = extract_toml_version(&content);
        if installed_ver == entry.version {
            println!(
                "loft install: {name} {} is already installed.",
                entry.version
            );
            return;
        }
    }

    // Download and extract.
    let tmp = std::env::temp_dir().join("loft_install");
    let _ = std::fs::create_dir_all(&tmp);
    match registry::download_and_extract(entry, &tmp) {
        Ok(pkg_root) => {
            install_package(&pkg_root);
            // Clean up temp.
            let _ = std::fs::remove_dir_all(&tmp);
        }
        Err(e) => {
            eprintln!("loft install: {e}");
            let _ = std::fs::remove_dir_all(&tmp);
            std::process::exit(1);
        }
    }
}

#[cfg(not(feature = "registry"))]
fn install_from_registry(arg: &str) {
    eprintln!(
        "loft install: registry support is not compiled in.\n  \
         Rebuild with: cargo build --features registry\n  \
         Trying to install: {arg}"
    );
    std::process::exit(1);
}

/// Extract version string from `loft.toml` content.
fn extract_toml_version(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("version") {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('=') {
                return rest.trim().trim_matches('"').to_string();
            }
        }
    }
    String::new()
}

/// PKG.6a: Generate Rust stubs for all `#native` declarations in a package.
///
/// Reads the package's `.loft` entry file, finds all `#native "symbol"`
/// declarations, and emits a Rust source file with the correct C-ABI
/// signatures plus `todo!()` bodies.
fn generate_native_stubs(pkg_path: &std::path::Path) {
    use crate::data::{DefType, Type};

    let toml_path = pkg_path.join("loft.toml");
    if !toml_path.exists() {
        eprintln!("Error: no loft.toml in {}", pkg_path.display());
        std::process::exit(1);
    }
    let manifest = match crate::manifest::read_manifest(&toml_path.to_string_lossy()) {
        Some(m) => m,
        None => {
            eprintln!("Error: cannot read {}", toml_path.display());
            std::process::exit(1);
        }
    };
    let entry = manifest
        .entry
        .as_deref()
        .map(|e| pkg_path.join(e))
        .unwrap_or_else(|| {
            let name = manifest.name.as_deref().unwrap_or("lib");
            pkg_path.join(format!("src/{name}.loft"))
        });
    if !entry.exists() {
        eprintln!("Error: entry file {} not found", entry.display());
        std::process::exit(1);
    }

    // Parse just enough to read definitions.
    let abs = std::fs::canonicalize(&entry).unwrap_or_else(|_| entry.clone());
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();
    let default_dir = dir.join("../default");
    let default_str = if default_dir.exists() {
        default_dir.to_string_lossy().to_string()
    } else {
        "default".to_string()
    };

    let mut p = parser::Parser::new();
    if let Some(src_dir) = entry.parent() {
        p.lib_dirs.push(src_dir.to_string_lossy().to_string());
    }
    // Load default definitions so types are known.
    let _ = p.parse_dir(&default_str, true, false);
    p.parse(&abs.to_string_lossy(), false);

    // Collect #native declarations.
    let mut stubs: Vec<String> = Vec::new();
    // Map struct name → (d_nr, fields) for generating field offset constants.
    let mut struct_field_mods: std::collections::HashMap<
        String,
        (u32, Vec<(String, usize, Type)>),
    > = std::collections::HashMap::new();
    for d_nr in 0..p.data.definitions() {
        let def = p.data.def(d_nr);
        if def.native.is_empty() || !matches!(def.def_type, DefType::Function) {
            continue;
        }
        let sym = &def.native;

        let mut c_params: Vec<String> = Vec::new();
        let mut body_lines: Vec<String> = Vec::new();
        let mut param_names: Vec<String> = Vec::new();

        for attr in &def.attributes {
            let name = &attr.name;
            match &attr.typedef {
                Type::Integer(_, _, _) | Type::Character => {
                    c_params.push(format!("{name}: i32"));
                    param_names.push(name.clone());
                }
                Type::Long => {
                    c_params.push(format!("{name}: i64"));
                    param_names.push(name.clone());
                }
                Type::Float => {
                    c_params.push(format!("{name}: f64"));
                    param_names.push(name.clone());
                }
                Type::Single => {
                    c_params.push(format!("{name}: f32"));
                    param_names.push(name.clone());
                }
                Type::Boolean => {
                    c_params.push(format!("{name}: bool"));
                    param_names.push(name.clone());
                }
                Type::Text(_) => {
                    c_params.push(format!("{name}_ptr: *const u8, {name}_len: usize"));
                    body_lines.push(format!(
                        "    let {name} = unsafe {{ loft_ffi::text({name}_ptr, {name}_len) }};"
                    ));
                    param_names.push(name.clone());
                }
                Type::Enum(_, false, _) => {
                    // Simple enum (tag only) — passed as u8.
                    c_params.push(format!("{name}: u8"));
                    param_names.push(name.clone());
                }
                Type::Reference(_, _)
                | Type::Vector(_, _)
                | Type::Enum(_, true, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Spacial(_, _, _) => {
                    let type_name = p.data.type_name_str(&attr.typedef);
                    c_params.push(format!("{name}: loft_ffi::LoftRef /* {type_name} */"));
                    param_names.push(name.clone());
                }
                other => {
                    let type_name = p.data.type_name_str(other);
                    c_params.push(format!(
                        "{name}: () /* {type_name} — not supported in native */"
                    ));
                    param_names.push(name.clone());
                }
            }
        }

        // Return type classification: text, ref, or scalar.
        enum RetKind {
            None,
            Scalar(String),
            Text,
            Ref(String),
        }
        let ret_type_name = p.data.type_name_str(&def.returned);
        let ret_kind = match &def.returned {
            Type::Void | Type::Null => RetKind::None,
            Type::Integer(_, _, _) | Type::Character => RetKind::Scalar(" -> i32".into()),
            Type::Long => RetKind::Scalar(" -> i64".into()),
            Type::Float => RetKind::Scalar(" -> f64".into()),
            Type::Single => RetKind::Scalar(" -> f32".into()),
            Type::Boolean => RetKind::Scalar(" -> bool".into()),
            Type::Text(_) => RetKind::Text,
            Type::Enum(_, false, _) => RetKind::Scalar(" -> u8".into()),
            Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Enum(_, true, _)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Spacial(_, _, _) => {
                RetKind::Ref(format!(" -> loft_ffi::LoftRef /* {ret_type_name} */"))
            }
            _ => RetKind::Scalar(format!(
                " -> () /* {ret_type_name} — not supported in native */"
            )),
        };
        let ret_ty = match &ret_kind {
            RetKind::None => String::new(),
            RetKind::Scalar(s) | RetKind::Ref(s) => s.clone(),
            RetKind::Text => " -> loft_ffi::LoftStr".into(),
        };
        let has_return = !matches!(ret_kind, RetKind::None);

        let has_text_param = def
            .attributes
            .iter()
            .any(|a| matches!(a.typedef, Type::Text(_)));
        let has_ref_param = def.attributes.iter().any(|a| {
            matches!(
                a.typedef,
                Type::Reference(_, _)
                    | Type::Vector(_, _)
                    | Type::Enum(_, true, _)
                    | Type::Sorted(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Spacial(_, _, _)
            )
        });
        let has_ref_ret = matches!(
            def.returned,
            Type::Reference(_, _)
                | Type::Vector(_, _)
                | Type::Enum(_, true, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Spacial(_, _, _)
        );

        // If any param or return is a Ref, prepend LoftStore as first C-ABI param.
        if has_ref_param || has_ref_ret {
            c_params.insert(0, "store: loft_ffi::LoftStore".to_string());
        }

        let needs_unsafe = has_text_param || has_ref_param || has_ref_ret;
        let unsafe_kw = if needs_unsafe { "unsafe " } else { "" };

        // Collect struct types referenced as params for field offset generation.
        for attr in &def.attributes {
            if let Type::Reference(d_nr, _) = &attr.typedef {
                let struct_def = p.data.def(*d_nr);
                if !struct_def.attributes.is_empty() {
                    let sname = struct_def.name.to_lowercase();
                    if !struct_field_mods.contains_key(&sname) {
                        let mut fields = Vec::new();
                        for (i, f) in struct_def.attributes.iter().enumerate() {
                            fields.push((f.name.clone(), i, f.typedef.clone()));
                        }
                        struct_field_mods.insert(sname, (*d_nr, fields));
                    }
                }
            }
        }

        // Format parameter list — wrap if longer than 90 chars.
        let params_joined = c_params.join(", ");
        let sig_line = format!("pub {unsafe_kw}extern \"C\" fn {sym}({params_joined}){ret_ty}");
        let params_str = if sig_line.len() > 95 && c_params.len() > 1 {
            format!("\n    {},\n", c_params.join(",\n    "))
        } else {
            params_joined
        };

        let mut stub = format!(
            "#[unsafe(no_mangle)]\npub {unsafe_kw}extern \"C\" fn {sym}({params_str}){ret_ty} {{\n"
        );
        for line in &body_lines {
            stub.push_str(line);
            stub.push('\n');
        }

        let args = param_names.join(", ");
        match &ret_kind {
            RetKind::Text => {
                stub.push_str(&format!(
                    "    let result: String = todo!(\"implement {sym}({args})\");\n"
                ));
                stub.push_str("    loft_ffi::ret(result)\n");
            }
            RetKind::Ref(_) => {
                stub.push_str(&format!(
                    "    let result: loft_ffi::LoftRef = todo!(\"implement {sym}({args})\");\n"
                ));
                stub.push_str("    result\n");
            }
            _ if has_return => {
                stub.push_str(&format!("    todo!(\"implement {sym}({args})\")\n"));
            }
            _ if !param_names.is_empty() => {
                stub.push_str(&format!("    todo!(\"implement {sym}({args})\")\n"));
            }
            _ => {}
        }
        stub.push_str("}\n");
        stubs.push(stub);
    }

    if stubs.is_empty() {
        println!("No #native declarations found.");
        return;
    }

    // Generate field offset modules for referenced struct types.
    let mut field_modules = String::new();
    for (sname, (_d_nr, fields)) in &struct_field_mods {
        // Compute store-side sizes: in the store, text/ref/vector are 4-byte record refs.
        let sizes: Vec<(u16, u8)> = fields
            .iter()
            .map(|(_, _, tp)| match tp {
                Type::Long | Type::Float => (8u16, 8u8),
                Type::Integer(_, _, _) | Type::Character | Type::Single => (4, 4),
                Type::Boolean | Type::Enum(_, false, _) => (1, 1),
                // In the store, text/ref/vector/collections are stored as 4-byte record refs.
                Type::Text(_)
                | Type::Reference(_, _)
                | Type::Vector(_, _)
                | Type::Enum(_, true, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Spacial(_, _, _) => (4, 4),
                _ => (4, 4), // fallback
            })
            .collect();
        let mut total_size = 0u16;
        let mut alignment = 0u8;
        let positions =
            crate::calc::calculate_positions(&sizes, false, &mut total_size, &mut alignment);

        field_modules.push_str(&format!("/// Field offsets for struct `{sname}`.\n"));
        field_modules.push_str(&format!(
            "/// Record size: {total_size} bytes ({} words).\n",
            total_size.div_ceil(8)
        ));
        field_modules.push_str("#[allow(dead_code)]\n");
        field_modules.push_str(&format!("pub mod {sname}_fields {{\n"));
        for (i, (fname, _, tp)) in fields.iter().enumerate() {
            let offset = positions[i];
            let type_comment = match tp {
                Type::Integer(_, _, _) => "integer",
                Type::Long => "long",
                Type::Float => "float",
                Type::Single => "single",
                Type::Boolean => "boolean",
                Type::Text(_) => "text (record ref)",
                Type::Reference(_, _) => "struct ref",
                Type::Vector(_, _) => "vector ref",
                _ => "other",
            };
            let upper = fname.to_uppercase();
            field_modules.push_str(&format!(
                "    pub const {upper}: u16 = {offset}; // {type_comment}\n"
            ));
        }
        field_modules.push_str("}\n\n");
    }

    let mut output = String::from(
        "// Auto-generated by `loft generate`. Fill in the todo!() bodies.\n\
         // Functions with text parameters use loft_ffi helpers.\n\
         // Struct field offsets are in *_fields modules.\n\n\
         #![allow(clippy::missing_safety_doc)]\n\n",
    );
    if stubs.iter().any(|s| s.contains("loft_ffi")) {
        output.push_str("// Add to Cargo.toml: loft-ffi = { path = \"../../../loft-ffi\" }\n\n");
    }

    // Emit field offset modules first.
    output.push_str(&field_modules);

    for (i, stub) in stubs.iter().enumerate() {
        if i > 0 {
            output.push('\n');
        }
        output.push_str(stub);
    }

    let out_dir = pkg_path.join("native/src");
    if out_dir.exists() {
        let out_file = out_dir.join("generated.rs");
        std::fs::write(&out_file, &output).unwrap_or_else(|e| {
            eprintln!("Error writing {}: {e}", out_file.display());
            std::process::exit(1);
        });
        println!("Wrote {} stubs to {}", stubs.len(), out_file.display());
    } else {
        print!("{output}");
    }
}

/// REG.3/REG.4: Handle `loft registry <subcommand>`.
fn handle_registry(argv: &[String], i: &mut usize) {
    let sub = if argv.get(*i).is_some_and(|s| !s.starts_with('-')) {
        *i += 1;
        argv[*i - 1].as_str()
    } else {
        ""
    };

    match sub {
        "sync" => registry_sync(),
        "check" => registry_check(),
        "list" => {
            let installed_only = argv.get(*i).is_some_and(|s| s == "--installed");
            registry_list(installed_only);
        }
        _ => {
            eprintln!("usage: loft registry <sync|check|list>");
            std::process::exit(1);
        }
    }
}

/// REG.3: Download the latest registry from the source URL.
fn registry_sync() {
    use loft::registry;

    // Determine source URL.
    let existing_source = registry::registry_path().and_then(|p| {
        let (_, src) = registry::read_registry(p.to_str().unwrap_or(""));
        src
    });
    let url = registry::source_url(existing_source.as_deref());

    eprintln!("syncing registry from {url} ...");

    // Download to a temp file first, then validate and move.
    let dst = registry::default_registry_path();
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = dst.with_extension("tmp");

    #[cfg(feature = "registry")]
    {
        if let Err(e) = registry::download_file(&url, &tmp) {
            eprintln!("loft registry sync: {e}\n  local registry is unchanged.");
            std::process::exit(1);
        }
    }
    #[cfg(not(feature = "registry"))]
    {
        let _ = url;
        let _ = tmp;
        eprintln!("loft registry sync: registry feature not compiled in.");
        std::process::exit(1);
    }

    // Validate content.
    let content = match std::fs::read_to_string(&tmp) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("loft registry sync: cannot read downloaded file: {e}");
            let _ = std::fs::remove_file(&tmp);
            std::process::exit(1);
        }
    };
    if let Err(e) = registry::validate_registry_content(&content) {
        eprintln!(
            "loft registry sync: invalid registry content: {e}\n  local registry is unchanged."
        );
        let _ = std::fs::remove_file(&tmp);
        std::process::exit(1);
    }

    // Move into place.
    if let Err(e) = std::fs::rename(&tmp, &dst) {
        eprintln!("loft registry sync: cannot write {}: {e}", dst.display());
        let _ = std::fs::remove_file(&tmp);
        std::process::exit(1);
    }

    let (entries, _) = registry::parse_registry(&content);
    let (pkgs, versions) = registry::registry_stats(&entries);
    let today = chrono_date();
    println!("registry synced: {pkgs} packages, {versions} versions  ({today})");
}

/// REG.4: Compare installed packages against the registry.
fn registry_check() {
    use loft::registry;

    let Some(reg_path) = registry::registry_path() else {
        eprintln!(
            "loft registry check: no registry file found.\n  \
             Run 'loft registry sync' to download the package registry."
        );
        std::process::exit(1);
    };
    let (entries, _) = registry::read_registry(reg_path.to_str().unwrap_or(""));
    let (pkgs, versions) = registry::registry_stats(&entries);

    // Staleness warning.
    if let Some(warning) = registry::staleness_warning(&reg_path) {
        eprintln!("{warning}");
    }
    let age_str = registry_age_str(&reg_path);
    println!("registry: {pkgs} packages, {versions} versions  ({age_str})");
    println!();

    let lib = registry::lib_dir();
    let installed = registry::installed_packages(&lib);

    if installed.is_empty() {
        println!("no packages installed.");
        println!("\nnew packages in registry: {pkgs}");
        println!("  run 'loft registry list' to browse");
        return;
    }

    println!("installed packages ({}):", installed.len());
    let mut yanked_count = 0;
    for (name, version) in &installed {
        let status = registry::classify(&entries, name, version);
        match status {
            registry::PackageStatus::Yanked { entry } => {
                println!(
                    "  {name:<12} {version:<8} YANKED      {} — run: loft install {name}",
                    entry.status_slug()
                );
                yanked_count += 1;
            }
            registry::PackageStatus::Deprecated { entry, .. } => {
                println!(
                    "  {name:<12} {version:<8} deprecated  {} — run: loft install {name}",
                    entry.status_slug()
                );
            }
            registry::PackageStatus::Outdated { latest } => {
                println!(
                    "  {name:<12} {version:<8} outdated    → {} — run: loft install {name}",
                    latest.version
                );
            }
            registry::PackageStatus::Current => {
                println!("  {name:<12} {version:<8} current");
            }
            registry::PackageStatus::Unknown => {
                println!("  {name:<12} {version:<8} (not in registry)");
            }
        }
    }

    let not_installed = pkgs.saturating_sub(installed.len());
    if not_installed > 0 {
        println!("\nnew packages in registry not installed: {not_installed}");
        println!("  run 'loft registry list' to browse");
    }

    if yanked_count > 0 {
        println!(
            "\n{yanked_count} security issue{} — yanked packages must be updated.",
            if yanked_count == 1 { "" } else { "s" }
        );
        std::process::exit(1);
    } else if installed.iter().all(|(name, version)| {
        matches!(
            registry::classify(&entries, name, version),
            registry::PackageStatus::Current
        )
    }) {
        println!("\nall installed packages are up to date.");
    }
}

/// `loft registry list [--installed]`
fn registry_list(installed_only: bool) {
    use loft::registry;

    let Some(reg_path) = registry::registry_path() else {
        eprintln!(
            "loft registry list: no registry file found.\n  \
             Run 'loft registry sync' to download the package registry."
        );
        std::process::exit(1);
    };
    let (entries, _) = registry::read_registry(reg_path.to_str().unwrap_or(""));
    let lib = registry::lib_dir();
    let installed = registry::installed_packages(&lib);

    let names = registry::package_names(&entries);

    println!(
        "{:<12} {:<28} {:<12} status",
        "name", "versions", "installed"
    );
    println!("{:-<12} {:-<28} {:-<12} {:-<20}", "", "", "", "");

    for name in &names {
        let versions = registry::package_versions(&entries, name);
        let inst_ver = installed
            .iter()
            .find(|(n, _)| n == name)
            .map_or("\u{2014}", |(_, v)| v.as_str());
        if installed_only && inst_ver == "\u{2014}" {
            continue;
        }
        let ver_str: Vec<&str> = versions.iter().map(|e| e.version.as_str()).collect();
        // Determine status column.
        let status = if inst_ver == "\u{2014}" {
            String::new()
        } else if let Some(e) = versions.iter().find(|e| e.version == inst_ver) {
            if e.is_yanked() {
                format!("YANKED ({inst_ver})")
            } else if e.is_deprecated() {
                "deprecated".to_string()
            } else {
                let latest = registry::find_package(&entries, name, None);
                if latest.is_some_and(|l| l.version != inst_ver) {
                    "outdated".to_string()
                } else {
                    String::new()
                }
            }
        } else {
            String::new()
        };
        println!(
            "{:<12} {:<28} {:<12} {}",
            name,
            ver_str.join("  "),
            inst_ver,
            status
        );
    }
}

/// Get a simple date string without pulling in the chrono crate.
fn chrono_date() -> String {
    // Use file modification time of a temp file as a proxy for "now".
    let tmp = std::env::temp_dir().join(".loft_date_probe");
    let _ = std::fs::write(&tmp, "");
    let date = std::fs::metadata(&tmp)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
            let secs = dur.as_secs();
            // Simple date calculation from unix timestamp.
            let days = secs / 86400;
            let (year, month, day) = days_to_ymd(days);
            Some(format!("{year}-{month:02}-{day:02}"))
        })
        .unwrap_or_else(|| "unknown date".to_string());
    let _ = std::fs::remove_file(&tmp);
    date
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Human-readable age of the registry file.
fn registry_age_str(path: &std::path::Path) -> String {
    let age = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs() / 86400);
    match age {
        Some(0) => "synced today".to_string(),
        Some(1) => "synced 1 day ago".to_string(),
        Some(d) => format!("synced {d} days ago"),
        None => "sync date unknown".to_string(),
    }
}

/// Collect crate names → rlib paths from a deps directory
/// (e.g. `libfoo-<hash>.rlib` → `("foo", "/path/to/libfoo-<hash>.rlib")`).
fn rlibs_in_dir(dir: &std::path::Path) -> std::collections::HashMap<String, std::path::PathBuf> {
    let mut map = std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"))
            {
                let fname = entry.file_name().to_string_lossy().to_string();
                if let Some(rest) = fname.strip_prefix("lib") {
                    if let Some(dash_pos) = rest.rfind('-') {
                        map.insert(rest[..dash_pos].to_string(), path);
                    }
                }
            }
        }
    }
    map
}

/// PKG.4/PKG.5: add `--extern` flags to a rustc command for native package rlibs.
/// When `target` is `Some("wasm32-wasip2")`, looks for WASM rlibs in `prebuilt/wasm32-wasip2/`;
/// otherwise looks for native rlibs in `native/target/release/`.
///
/// Uses `-L dependency=` for the native package's deps so deep transitive deps
/// resolve. For any crate that also appears in loft's own deps, adds an explicit
/// `--extern name=<loft's copy>` so rustc uses a single copy, avoiding
/// StableCrateId collisions.
fn add_native_extern_flags(
    cmd: &mut std::process::Command,
    data: &data::Data,
    target: Option<&str>,
    loft_deps_dir: Option<&std::path::Path>,
) {
    let loft_rlibs = loft_deps_dir.map(|d| rlibs_in_dir(d)).unwrap_or_default();

    for (crate_name, pkg_dir) in &data.native_packages {
        // Look for the compiled rlib in the package's native crate output.
        let rlib_name = format!("lib{}.rlib", crate_name.replace('-', "_"));
        let rlib_path = if let Some(tgt) = target {
            // WASM: check prebuilt first, then native/target/<target>/release/
            let prebuilt = std::path::PathBuf::from(pkg_dir)
                .join("prebuilt")
                .join(tgt)
                .join(&rlib_name);
            if prebuilt.exists() {
                prebuilt
            } else {
                std::path::PathBuf::from(pkg_dir)
                    .join("native/target")
                    .join(tgt)
                    .join("release")
                    .join(&rlib_name)
            }
        } else {
            // Native: check native/target/release/
            std::path::PathBuf::from(pkg_dir)
                .join("native/target/release")
                .join(&rlib_name)
        };
        if rlib_path.exists() {
            let extern_name = crate_name.replace('-', "_");
            cmd.arg("--extern")
                .arg(format!("{}={}", extern_name, rlib_path.display()));
            // Add the native crate's deps directory so transitive deps (GL, glutin, etc.)
            // resolve. Use `dependency` search scope so these crates are only found as
            // transitive deps of the native crate, not as direct deps.
            let deps_dir = rlib_path.parent().unwrap().join("deps");
            if deps_dir.is_dir() {
                cmd.arg("-L")
                    .arg(format!("dependency={}", deps_dir.display()));
                // Pin any crate that also exists in loft's deps to loft's copy,
                // preventing StableCrateId collisions from duplicate rlibs.
                if !loft_rlibs.is_empty() {
                    let pkg_crates = rlibs_in_dir(&deps_dir);
                    for (dep_name, loft_path) in &loft_rlibs {
                        if pkg_crates.contains_key(dep_name) {
                            cmd.arg("--extern").arg(format!(
                                "{}={}",
                                dep_name,
                                loft_path.display()
                            ));
                        }
                    }
                }
            }
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
    let mut native_mode = true;
    let mut native_release = false;
    // None  = flag not given
    // Some("") = flag given without explicit path → use .loft/ default
    // Some(path) = explicit output path
    let mut native_emit: Option<String> = None;
    let mut native_wasm: Option<String> = None;
    let mut tests_dir: Option<String> = None;
    let mut native_lib_paths: Vec<String> = Vec::new();
    let mut no_warnings = false;
    let mut check_only = false;
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
        } else if a == "--interpret" || a == "--bytecode" {
            native_mode = false;
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
                    // Note: --tests forces interpreter mode; this is a no-op.
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
            native_mode = false; // test runner uses interpreter
        } else if a == "--no-warnings" {
            no_warnings = true;
        } else if a == "--check" || a == "check" {
            check_only = true;
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
            // Read loft.toml to find src/ directory, dependency paths, and native libs.
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
                // Add parent directory so sibling packages (dependencies) are found.
                if !manifest.dependencies.is_empty() {
                    let parent = std::env::current_dir()
                        .unwrap_or_default()
                        .join("..")
                        .canonicalize()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    if !lib_dirs.contains(&parent) {
                        lib_dirs.push(parent);
                    }
                }
                // Register the package's own native lib for loading.
                // Dependency native libs are discovered when the parser
                // processes `use` statements via lib_path_manifest().
                if let Some(ref stem) = manifest.native {
                    let pkg_dir = std::env::current_dir()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let lib_file = crate::extensions::platform_lib_name(stem);
                    let prebuilt = format!("{pkg_dir}/native/{lib_file}");
                    if std::path::Path::new(&prebuilt).exists() {
                        native_lib_paths.push(prebuilt);
                    } else if let Some(built) = crate::extensions::auto_build_native(&pkg_dir, stem)
                    {
                        native_lib_paths.push(built);
                    }
                }
            } else if std::path::Path::new("src").is_dir() {
                let abs_src = std::env::current_dir()
                    .unwrap_or_default()
                    .join("src")
                    .to_string_lossy()
                    .to_string();
                lib_dirs.push(abs_src);
            }
            tests_dir = Some(test_target);
            // Test runner uses the interpreter internally.
            native_mode = false;
        } else if a == "install" {
            let arg = if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                i += 1;
                argv[i - 1].clone()
            } else {
                String::new()
            };
            if arg.is_empty()
                || arg.starts_with('/')
                || arg.starts_with("./")
                || arg.starts_with("../")
                || arg == "."
                || arg.contains('/')
            {
                // Local path install.
                let pkg_path = if arg.is_empty() {
                    std::env::current_dir().unwrap_or_default()
                } else {
                    std::path::PathBuf::from(&arg)
                };
                install_package(&pkg_path);
            } else {
                // Registry install.
                install_from_registry(&arg);
            }
            return;
        } else if a == "registry" {
            handle_registry(&argv, &mut i);
            return;
        } else if a == "generate" {
            // PKG.6a: `loft generate` — emit Rust stubs for #native declarations.
            let pkg_path = if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                std::path::PathBuf::from(&argv[i])
            } else {
                std::env::current_dir().unwrap_or_default()
            };
            generate_native_stubs(&pkg_path);
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
            // P131: once the script path has been seen, treat every later
            // token (including `--*` ones) as a script argument and forward
            // it to the script's `arguments()`. The loft CLI cannot ambiguate
            // its own options from script options after the script path is
            // known. Use of `--` as an explicit forwarding boundary is also
            // supported (an explicit `--` is consumed and skipped).
            if !file_name.is_empty() {
                if a != "--" {
                    user_args.push(a.to_string());
                }
            } else {
                println!("unknown option: {a}");
                println!("usage: loft [options] <file>");
                println!("Try `loft --help` for more information.");
                std::process::exit(1);
            }
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
            &native_lib_paths,
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
    // Auto-detect loft.toml by walking up from the script file's directory.
    // This lets `loft lib/graphics/examples/01.loft` find the graphics package
    // without requiring the user to cd into the package directory first.
    if !abs_file.is_empty() {
        let script_dir = std::path::Path::new(&abs_file).parent();
        if let Some(mut search) = script_dir.map(std::path::Path::to_path_buf) {
            loop {
                let candidate = search.join("loft.toml");
                if candidate.exists() {
                    if let Some(manifest) =
                        manifest::read_manifest(candidate.to_str().unwrap_or("loft.toml"))
                    {
                        // Add the package's src/ directory to lib_dirs.
                        let entry = manifest.entry.unwrap_or_else(|| "src".to_string());
                        let src_dir = std::path::Path::new(&entry)
                            .parent()
                            .unwrap_or(std::path::Path::new("src"));
                        let abs_src = search.join(src_dir).to_string_lossy().to_string();
                        if !lib_dirs.contains(&abs_src) {
                            lib_dirs.push(abs_src);
                        }
                        // Add parent directory so sibling packages (deps) are found.
                        if !manifest.dependencies.is_empty() {
                            if let Ok(parent) = search.join("..").canonicalize() {
                                let ps = parent.to_string_lossy().to_string();
                                if !lib_dirs.contains(&ps) {
                                    lib_dirs.push(ps);
                                }
                            }
                        }
                        // Register native lib path for auto-build/loading.
                        if let Some(ref stem) = manifest.native {
                            let pkg_dir = search.to_string_lossy().to_string();
                            if let Some(so_path) = extensions::auto_build_native(&pkg_dir, stem) {
                                native_lib_paths.push(so_path);
                            }
                        }
                    }
                    // Auto-add lib/ subdirectory for package imports.
                    let lib_dir = search.join("lib");
                    if lib_dir.is_dir() {
                        let ls = lib_dir.to_string_lossy().to_string();
                        if !lib_dirs.contains(&ls) {
                            lib_dirs.push(ls);
                        }
                    }
                    break;
                }
                if !search.pop() {
                    break;
                }
            }
        }
    }

    // Canonicalize library paths so relative --lib dirs resolve correctly
    // regardless of working directory changes during parsing.
    let lib_dirs: Vec<String> = lib_dirs
        .into_iter()
        .map(|d| {
            std::fs::canonicalize(&d)
                .unwrap_or_else(|_| std::path::PathBuf::from(&d))
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let mut p = parser::Parser::new();
    p.lib_dirs = lib_dirs;
    p.parse_dir(&(dir + "default"), true, false).unwrap();
    let start_def = p.data.definitions();
    p.parse(&abs_file, false);
    if !p.diagnostics.is_empty() {
        // Cache source files for source-line display.
        let mut source_cache: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for entry in p.diagnostics.entries() {
            if entry.level == Level::Debug {
                continue;
            }
            println!("{}", entry.to_string_compact());
            // Show the offending source line with a caret.
            if entry.line > 0 && !entry.file.is_empty() {
                let src = source_cache
                    .entry(entry.file.clone())
                    .or_insert_with(|| std::fs::read_to_string(&entry.file).unwrap_or_default());
                if let Some(line_text) = src.lines().nth(entry.line as usize - 1) {
                    let col = entry.col.saturating_sub(1) as usize;
                    println!("  |");
                    println!("{:>4} | {}", entry.line, line_text);
                    println!("     | {:>width$}^", "", width = col);
                }
            }
        }
        if p.diagnostics.level() >= Level::Error {
            std::process::exit(1);
        }
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    // Set source_dir for the source_dir() built-in.
    state.database.source_dir = std::path::Path::new(&abs_file)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    // P131: store script-level arguments so arguments() returns only these.
    state.database.user_args.clone_from(&user_args);
    // Bytecode cache: read source content for the cache key, use .loftc path.
    let source_content = std::fs::read_to_string(&abs_file).unwrap_or_default();
    let cache_file = cache::cache_path(&abs_file);
    let sources = [(abs_file.as_str(), source_content.as_str())];
    compile::byte_code_with_cache(&mut state, &mut p.data, Some(&cache_file), &sources);
    // A7.2: load native extension shared libraries registered during parsing.
    // Also include any native libs discovered via loft.toml auto-detection.
    let mut all_native_libs = std::mem::take(&mut p.pending_native_libs);
    for nlp in &native_lib_paths {
        if !all_native_libs.contains(nlp) {
            all_native_libs.push(nlp.clone());
        }
    }
    extensions::load_all(&mut state, all_native_libs);
    // PKG.5: wire auto-marshalled native functions from loaded cdylibs.
    extensions::wire_native_fns(&mut state, &p.data);

    // --check: parse + compile only, report errors and exit.
    // When combined with --native, fall through to the native pipeline
    // which will compile but not run the binary.
    if check_only && !native_mode && native_emit.is_none() {
        println!("ok {abs_file}");
        return;
    }

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
        let wasm_deps_dir = if let Some(lib_dir) = loft_lib_dir_for(Some("wasm32-wasip2")) {
            cmd.arg("--extern")
                .arg(format!("loft={}", lib_dir.join("libloft.rlib").display()));
            let deps = lib_dir.join("deps");
            cmd.arg("-L").arg(format!("dependency={}", deps.display()));
            Some(deps)
        } else {
            None
        };
        // PKG.5: add --extern flags for native packages (WASM target).
        add_native_extern_flags(
            &mut cmd,
            &p.data,
            Some("wasm32-wasip2"),
            wasm_deps_dir.as_deref(),
        );
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

    // Check rustc availability; fall back to interpreter if not found.
    if native_mode && native_emit.is_none() {
        if let Err(e) = std::process::Command::new("rustc")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            if e.kind() == std::io::ErrorKind::NotFound {
                eprintln!("Warning: rustc not found, falling back to interpreter mode");
            } else {
                eprintln!("Warning: rustc check failed ({e}), falling back to interpreter");
            }
            native_mode = false;
        }
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
            // For test-only files (no fn main()), generate a main() that calls
            // all zero-parameter user functions as test entry points.
            let main_nr = p.data.def_nr("n_main");
            if main_nr >= end_def {
                use std::io::Write;
                let mut test_fns: Vec<(u32, String)> = Vec::new();
                for d_nr in start_def..end_def {
                    let def = p.data.def(d_nr);
                    if !matches!(def.def_type, crate::data::DefType::Function) {
                        continue;
                    }
                    if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
                        continue;
                    }
                    if def.position.file.starts_with("default/") {
                        continue;
                    }
                    let has_user_params = def
                        .attributes
                        .iter()
                        .any(|a| !a.name.starts_with("__work_") && !a.name.starts_with("__ref_"));
                    if has_user_params {
                        continue;
                    }
                    test_fns.push((d_nr, def.name.clone()));
                }
                if !test_fns.is_empty() {
                    let _ = writeln!(f, "\nfn main() {{");
                    let _ = writeln!(f, "    let mut stores = Stores::new();");
                    let _ = writeln!(f, "    init(&mut stores);");
                    for (d_nr, name) in &test_fns {
                        let def = p.data.def(*d_nr);
                        let mut work_args = Vec::new();
                        for (i, attr) in def.attributes.iter().enumerate() {
                            if attr.name.starts_with("__work_") {
                                let wname = format!("_w_{i}");
                                let _ = writeln!(f, "    let mut {wname} = String::new();");
                                work_args.push(format!("&mut {wname}"));
                            } else if attr.name.starts_with("__ref_") {
                                let wname = format!("_r_{i}");
                                let _ = writeln!(
                                    f,
                                    "    let mut {wname} = stores.null_named(\"{wname}\");"
                                );
                                work_args.push(wname.clone());
                            }
                        }
                        if work_args.is_empty() {
                            let _ = writeln!(f, "    {name}(&mut stores);");
                        } else {
                            let _ =
                                writeln!(f, "    {name}(&mut stores, {});", work_args.join(", "));
                        }
                    }
                    let _ = writeln!(f, "}}");
                }
            }
        }
        if native_emit.is_some() {
            return; // --native-emit: just write the file, don't compile
        }
        // --native / --native-release: compile with rustc and run.
        // Cache compiled binaries in .loft/cache/ next to the source file,
        // keyed by a hash of the generated Rust source so recompilation is
        // skipped when the output hasn't changed.
        let source_bytes = std::fs::read(&emit_path).unwrap_or_default();
        let source_hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            source_bytes.hash(&mut h);
            // Include the release flag in the hash so debug/release don't collide.
            native_release.hash(&mut h);
            // Include modification times of native package rlibs and loft's
            // own rlib so the cache invalidates when dependencies are rebuilt.
            if let Some(lib_dir) = loft_lib_dir() {
                if let Ok(meta) = std::fs::metadata(lib_dir.join("libloft.rlib")) {
                    meta.modified().ok().hash(&mut h);
                }
            }
            for (_crate_name, pkg_dir) in &p.data.native_packages {
                let rlib_name = format!("lib{}.rlib", _crate_name.replace('-', "_"));
                let rlib_path = std::path::PathBuf::from(pkg_dir)
                    .join("native/target/release")
                    .join(&rlib_name);
                if let Ok(meta) = std::fs::metadata(&rlib_path) {
                    meta.modified().ok().hash(&mut h);
                }
            }
            format!("{:016x}", h.finish())
        };
        let cache_dir = std::path::Path::new(&abs_file)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(".loft")
            .join("cache");
        let source_stem = std::path::Path::new(&abs_file)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let cached_binary = cache_dir.join(format!("{source_stem}-{source_hash}"));

        // Use cached binary if it exists, otherwise compile and cache.
        let binary = if cached_binary.exists() {
            cached_binary.clone()
        } else {
            let binary = std::env::temp_dir().join("loft_native_bin");
            let mut cmd = std::process::Command::new("rustc");
            cmd.arg("--edition=2024")
                .arg("-o")
                .arg(&binary)
                .arg(&emit_path);
            if native_release {
                cmd.arg("-O");
            }
            let native_deps_dir = if let Some(lib_dir) = loft_lib_dir() {
                cmd.arg("--extern")
                    .arg(format!("loft={}", lib_dir.join("libloft.rlib").display()));
                let deps = lib_dir.join("deps");
                cmd.arg("-L").arg(format!("dependency={}", deps.display()));
                if let Ok(rd) = std::fs::read_dir(&deps) {
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with("libloft_ffi-")
                            && std::path::Path::new(&name)
                                .extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"))
                        {
                            cmd.arg("--extern")
                                .arg(format!("loft_ffi={}", e.path().display()));
                            break;
                        }
                    }
                }
                Some(deps)
            } else {
                None
            };
            // PKG.4: add --extern flags for native packages.
            add_native_extern_flags(&mut cmd, &p.data, None, native_deps_dir.as_deref());
            let status = cmd.status();
            let status = match status {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!(
                        "loft: rustc not found; install the Rust toolchain to use --native mode"
                    );
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
            // Store in cache for next run.
            if std::fs::create_dir_all(&cache_dir).is_ok() {
                // Remove stale cached binaries for THIS source file only.
                let prefix = format!("{source_stem}-");
                if let Ok(entries) = std::fs::read_dir(&cache_dir) {
                    for entry in entries.flatten() {
                        if entry.file_name().to_string_lossy().starts_with(&prefix) {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
                let _ = std::fs::copy(&binary, &cached_binary);
            }
            binary
        };
        let _ = std::fs::remove_file(&emit_path);

        if check_only {
            // --check --native: compile succeeded, report ok and exit.
            println!("ok {abs_file}");
            return;
        }
        let run_status = std::process::Command::new(&binary)
            .args(&user_args)
            .status()
            .unwrap_or_else(|e| {
                eprintln!("loft: failed to run native binary: {e}");
                std::process::exit(1);
            });
        // Clean up temp binary (not the cached copy).
        if binary != cached_binary {
            let _ = std::fs::remove_file(&binary);
        }
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

    let main_nr = p.data.def_nr("n_main");
    if main_nr == u32::MAX {
        // No main() — wrap each zero-parameter user function in a synthetic
        // main() that calls it. This ensures proper scope cleanup: stores
        // allocated by struct-returning functions are freed when the caller's
        // variables go out of scope, before the leak check runs.
        let mut test_names: Vec<String> = Vec::new();
        for d_nr in start_def..p.data.definitions() {
            let def = p.data.def(d_nr);
            if def.name.starts_with("n_")
                && !def.name.starts_with("n___lambda_")
                && matches!(def.def_type, data::DefType::Function)
                && def.attributes.is_empty()
                && matches!(def.returned, data::Type::Void)
                && !def.position.file.starts_with("default/")
            {
                let name = def.name.strip_prefix("n_").unwrap_or(&def.name);
                test_names.push(name.to_string());
            }
        }
        // Build a single main() that calls all test functions in sequence.
        // This gives each call a proper scope for store cleanup.
        let mut calls = String::new();
        for name in &test_names {
            calls.push_str(name);
            calls.push_str("();\n");
        }
        if !calls.is_empty() {
            let wrapper = format!("fn main() {{\n{calls}}}");
            let mut wp = parser::Parser::new();
            wp.data = p.data;
            wp.database = state.database;
            wp.parse_str(&wrapper, "test_wrapper", false);
            scopes::check(&mut wp.data);
            state.database = wp.database;
            compile::byte_code(&mut state, &mut wp.data);
            p.data = wp.data;
            state.execute_argv("main", &p.data, &[]);
        }
    } else if std::env::var("LOFT_LOG").is_ok() {
        let config = log_config::LogConfig::from_env();
        let mut log = std::io::stderr();
        if let Err(e) = state.execute_log(&mut log, "main", &config, &p.data) {
            eprintln!("Execution error: {e}");
            std::process::exit(1);
        }
    } else {
        state.execute_argv("main", &p.data, &user_args);
        // FY.3: native desktop frame loop — gl_swap_buffers sets frame_yield,
        // causing execute_argv to return. Resume until the program finishes.
        while state.database.frame_yield {
            state.resume();
        }
    }
    state.check_store_leaks();
    if state.database.had_fatal {
        std::process::exit(1);
    }
}
