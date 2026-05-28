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
#[macro_use]
mod trace;
mod base64;
mod calc;
mod codegen_runtime;
mod compile;
mod const_eval;
mod crash_report;
mod data;
mod database;
pub mod diagnostic_render;
mod extensions;
mod fill;
mod formatter;
mod generation;
mod hash;
mod introspect;
mod json;
mod keys;
mod lexer;
mod lockfile;
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
#[cfg(feature = "registry")]
mod registry_index;
mod runtime_error;
mod scopes;
mod stack;
mod state;
mod store;
mod test_runner;
mod timeout;
mod tree;
mod typedef;
mod variables;
mod vector;
#[cfg(feature = "wasm")]
mod wasm;

use crate::diagnostics::Level;
use crate::native_utils::{
    build_script_native_lib_dirs, default_artifact_path, html_wasm_import_modules_ok,
    is_output_path, loft_lib_dir, loft_lib_dir_for, project_dir,
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
        "  --timeout <secs>              hard-kill the process after <secs>+grace seconds (PLAN49)"
    );
    println!(
        "                                LOFT_TIMEOUT=<secs> as env equivalent; grace defaults to"
    );
    println!("                                2s, override via LOFT_TIMEOUT_GRACE=<secs>");
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
    println!(
        "  --dump                        compile to bytecode, dump to stderr, and exit (no execution)"
    );
    println!("  --native                      compile to native Rust via rustc and run (default)");
    println!("  --native-release              like --native but emit only reachable functions and");
    println!("                                compile with rustc -O (optimised build)");
    println!(
        "  --native-debug                like --native but compile with -Cdebuginfo=2 (DWARF)"
    );
    println!("  --dev-soft-halt               demote dev-mode runtime raises to log-and-continue");
    println!("                                (also: LOFT_DEV_SOFT_HALT=1) — surfaces every fault");
    println!(
        "                                site in a single run instead of halting on the first"
    );
    println!(
        "                                and preserve the generated .rs on disk; combine with"
    );
    println!("                                --native-release for optimised + debug-info");
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
    println!("  --no-warnings                 suppress warnings (in run mode and --tests output)");
    println!(
        "  --deny-warnings               under --tests/`loft test`, fail any file with an
                                unexpected warning.  LOFT_DENY_WARNINGS=1 as env equivalent.
                                Used by extracted library chunks' CI to lock in cleanliness."
    );
    println!(
        "  --deps[=direct|=transitive]   under `loft test`, also run `loft test` in every
                                dependency directory listed in loft.toml.  Default is
                                =transitive; =direct walks only first-level deps.  Reads
                                path-form deps `{{ path = \"...\" }}`; registry-version deps
                                require a loft.lock (T4 follow-up).  Returns non-zero if
                                the host project's tests OR any dep's tests fail."
    );
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
    println!("  package [path]                build a publishable <pkg>-<version>.tar.gz");
    println!("                                prints sha256 + size + the registry index entry");
    println!("                                (PKG.REG R1 — see doc/claude/PKG_REGISTRY.md)");
    println!("  search [query]                client-side search of the package registry");
    println!(
        "                                matches name / description / categories (case-insensitive)"
    );
    println!("                                (PKG.REG R8)");
    println!(
        "  info <name>                   per-package details (versions, latest, deps, homepage)"
    );
    println!("                                (PKG.REG R8)");
    println!();
    println!("install flags (PKG.REG R4):");
    println!("  --refresh                     force re-fetch the registry index");
    println!("  --offline                     use cache only; fail if anything's missing");
    println!("  --prerelease                  resolve prerelease versions too");
    println!("  --allow-unsigned              proceed even if the index has no valid signature");
    println!("                                (default while no trust root is embedded;");
    println!("                                 flips after src/registry_keys.rs is populated)");
    println!(
        "  --require-signature           refuse to proceed unless the index signature verifies"
    );
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
/// Phase 6t Tier 4 — walk the current project's dep tree and invoke
/// `loft test` in each dep's directory.  Direct mode walks only
/// `manifest.dependencies` of the cwd; transitive mode recurses into
/// every walked dep's own `loft.toml`.  Returns 1 if any dep failed,
/// 0 otherwise.
///
/// Today this resolves PATH dependencies only (manifest `{ path = ".." }`
/// form).  Registry-installed deps live at
/// `~/.loft/registry/<id>-<version>/` but their version lookup needs the
/// lockfile loader, which is the T4 follow-up (per lib_plans/12 Phase
/// 6t Tier 4 step T4); without a lockfile, registry-version deps fall
/// through silently.
fn run_dep_tests(transitive: bool, native_mode: bool) -> i32 {
    use std::collections::{HashSet, VecDeque};
    use std::path::PathBuf;
    let cwd = std::env::current_dir().unwrap_or_default();
    let loft_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("loft"));

    // Resolve a dep name + value to a directory.  Path deps win; we
    // ignore version-only registry refs for now (T4 follow-up).
    fn resolve_dep(name: &str, value: &str, from_pkg: &std::path::Path) -> Option<PathBuf> {
        if let Some(p) = crate::manifest::extract_path_dep(value) {
            let candidate = from_pkg.join(p);
            if candidate.join("loft.toml").exists() {
                return Some(candidate.canonicalize().unwrap_or(candidate));
            }
        }
        // Fallback — sibling directory: from_pkg/../<name>/loft.toml.
        let sibling = from_pkg.join("..").join(name);
        if sibling.join("loft.toml").exists() {
            return Some(sibling.canonicalize().unwrap_or(sibling));
        }
        None
    }

    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(cwd.clone());
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut total_fail: i32 = 0;
    let mut tested = 0usize;

    while let Some(pkg) = queue.pop_front() {
        if !visited.insert(pkg.clone()) {
            continue;
        }
        let manifest_path = pkg.join("loft.toml");
        let Some(manifest) = crate::manifest::read_manifest(manifest_path.to_str().unwrap_or(""))
        else {
            continue;
        };
        for (dep_name, dep_value) in &manifest.dependencies {
            let Some(dep_dir) = resolve_dep(dep_name, dep_value, &pkg) else {
                if pkg == cwd {
                    eprintln!(
                        "  --deps: skipping {dep_name} (no path-dep; registry resolution not yet wired)"
                    );
                }
                continue;
            };
            if visited.contains(&dep_dir) {
                continue;
            }
            if dep_dir.join("tests").is_dir() {
                tested += 1;
                let mut cmd = std::process::Command::new(&loft_bin);
                cmd.arg("test").current_dir(&dep_dir);
                if native_mode {
                    cmd.arg("--native");
                }
                // Suppress warnings inside deps unless the user opts in
                // via LOFT_DENY_WARNINGS_DEPS=1 — the consumer should
                // not be blocked by lint debt inside a dep they don't
                // own.  (Errors still surface via exit code.)
                cmd.arg("--no-warnings");
                let label = dep_dir
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| dep_name.clone());
                println!("  --deps: testing {label}");
                let status = cmd.status();
                let ok = status.as_ref().map(|s| s.success()).unwrap_or(false);
                if !ok {
                    eprintln!("  --deps: FAILED {label}");
                    total_fail += 1;
                }
            }
            if transitive {
                queue.push_back(dep_dir);
            }
        }
    }

    if tested > 0 {
        println!("  --deps: {tested} dep(s) tested, {total_fail} failed");
    } else {
        println!("  --deps: no deps with tests/ directory found");
    }
    i32::from(total_fail > 0)
}

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
///
/// PKG.REG R4 (2026-05-24): the `loft install <name>` entry point in
/// `main` calls [`install_from_registry_with_opts`] directly with the
/// parsed flag bag.  When `LOFT_LEGACY_REGISTRY` is set, that helper
/// delegates to [`install_from_registry_legacy`] (the older
/// text-format flow).
#[cfg(feature = "registry")]
fn install_from_registry_with_opts(args: &[String], opts: &loft::install::InstallOptions) {
    use loft::install::{format_report, install_one};

    if std::env::var("LOFT_LEGACY_REGISTRY").is_ok() {
        for arg in args {
            install_from_registry_legacy(arg);
        }
        return;
    }

    if args.is_empty() {
        eprintln!("loft install: no package name given");
        std::process::exit(1);
    }

    // Process each arg sequentially.  Each `install_one` invocation
    // merges its packages into the cwd's `loft.lock`, so the final
    // lockfile lists every package the user asked for.
    for arg in args {
        let (name, version) = if let Some((n, v)) = arg.split_once('@') {
            (n, Some(v))
        } else {
            (arg.as_str(), None)
        };
        match install_one(name, version, opts) {
            Ok(report) => {
                print!("{}", format_report(&report));
            }
            Err(e) => {
                eprintln!("loft install: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// PKG.REG R8 — `loft search <query>`: client-side filter against
/// the cached index.  Refreshes the index if the cache is stale (TTL
/// reuses `loft install`'s code path).  Output: one line per matching
/// `name X.Y.Z — description` row.
#[cfg(feature = "registry")]
fn search_registry(query: &str) {
    use loft::install::InstallOptions;
    use loft::registry_index;

    let opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
    };
    let index = match loft_install_load_index(&opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft search: {e}");
            std::process::exit(1);
        }
    };

    let q = query.to_ascii_lowercase();
    let mut hits: Vec<&loft::registry_index::Package> = index
        .packages
        .values()
        .filter(|p| {
            let name_match = p.name.to_ascii_lowercase().contains(&q);
            let desc_match = p
                .description
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains(&q);
            let cat_match = p
                .categories
                .iter()
                .any(|c| c.to_ascii_lowercase().contains(&q));
            q.is_empty() || name_match || desc_match || cat_match
        })
        .collect();
    hits.sort_by(|a, b| a.name.cmp(&b.name));

    if hits.is_empty() {
        println!("No packages match `{query}`.");
        return;
    }
    for pkg in hits {
        let latest = registry_index::find_best_version(pkg, "*", false)
            .map(|v| v.semver.clone())
            .unwrap_or_else(|| "(no stable version)".to_string());
        let desc = pkg.description.as_deref().unwrap_or("(no description)");
        println!("{} {latest} — {desc}", pkg.name);
    }
}

/// PKG.REG R8 — `loft info <name>`: full info for one package.
/// Prints homepage, categories, available versions (yanked /
/// prerelease tags inline), deps for the latest.
#[cfg(feature = "registry")]
fn package_info(name: &str) {
    use loft::install::InstallOptions;
    use loft::registry_index;

    let opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
    };
    let index = match loft_install_load_index(&opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft info: {e}");
            std::process::exit(1);
        }
    };

    let Some(pkg) = index.packages.get(name) else {
        eprintln!("loft info: package `{name}` not found in registry");
        std::process::exit(1);
    };
    println!("{name}");
    if let Some(d) = &pkg.description {
        println!("  description: {d}");
    }
    if let Some(h) = &pkg.homepage {
        println!("  homepage:    {h}");
    }
    if !pkg.categories.is_empty() {
        println!("  categories:  {}", pkg.categories.join(", "));
    }
    let latest = registry_index::find_best_version(pkg, "*", false);
    if let Some(v) = latest {
        println!("  latest:      {}", v.semver);
        if !v.deps.is_empty() {
            println!("  deps:");
            for (n, c) in &v.deps {
                println!("    {n} {c}");
            }
        }
    }
    println!("  versions:");
    for ver in pkg.versions.values() {
        let mut tags = Vec::new();
        if pkg.yanked.iter().any(|y| y == &ver.semver) {
            tags.push("yanked");
        }
        if ver.prerelease {
            tags.push("prerelease");
        }
        let tag_str = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        println!("    {}{tag_str}", ver.semver);
    }
}

/// Thin wrapper exposing `install::load_index`-equivalent for the
/// `search` / `info` paths above without making `install::load_index`
/// public (it's an internal helper of the install orchestrator).
/// Re-fetches if cache stale, verifies signature per opts.
#[cfg(feature = "registry")]
fn loft_install_load_index(
    opts: &loft::install::InstallOptions,
) -> Result<loft::registry_index::RegistryIndex, String> {
    use loft::registry_index;
    use loft::registry_signing::{VerifyResult, verify_index};

    let url = registry_index::registry_url();
    let (idx_path, sig_path, _) = registry_index::index_paths();
    let content_bytes: Vec<u8> = if opts.offline {
        std::fs::read(&idx_path).map_err(|e| {
            format!(
                "offline mode: no cached index ({}): {e}",
                idx_path.display()
            )
        })?
    } else {
        let stale = std::fs::metadata(&idx_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_none_or(|age| opts.refresh || age.as_secs() > 60 * 60);
        if stale {
            let fetched = registry_index::fetch_index(&url)?;
            if let Some(parent) = idx_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&idx_path, &fetched.content).map_err(|e| format!("cache index: {e}"))?;
            if !fetched.signature.is_empty() {
                let _ = std::fs::write(&sig_path, &fetched.signature);
            }
            fetched.content
        } else {
            std::fs::read(&idx_path).map_err(|e| format!("read cached index: {e}"))?
        }
    };
    let sig = std::fs::read(&sig_path).unwrap_or_default();
    match verify_index(&content_bytes, &sig) {
        VerifyResult::Valid => {}
        VerifyResult::NoTrustRoot | VerifyResult::MalformedSignature if opts.allow_unsigned => {}
        VerifyResult::Invalid => {
            return Err("index signature INVALID — refusing to load (hard failure)".to_string());
        }
        VerifyResult::NoTrustRoot => {
            return Err(
                "registry index unsigned and this loft binary has no embedded trust root; \
                 pass --allow-unsigned to proceed"
                    .to_string(),
            );
        }
        VerifyResult::MalformedSignature => {
            return Err(
                "registry index signature is malformed; pass --allow-unsigned to proceed"
                    .to_string(),
            );
        }
    }
    let text = std::str::from_utf8(&content_bytes)
        .map_err(|e| format!("index is not valid UTF-8: {e}"))?;
    registry_index::parse_index(text)
}

#[cfg(feature = "registry")]
fn install_from_registry_legacy(arg: &str) {
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
#[cfg(feature = "registry")]
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
                // Post-2c round 10c: wide Type::Integer (former Type::Long) → i64.
                Type::Integer(s) if s.is_wide() => {
                    c_params.push(format!("{name}: i64"));
                    param_names.push(name.clone());
                }
                Type::Integer(_) | Type::Character => {
                    c_params.push(format!("{name}: i32"));
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
            // Post-2c round 10c: wide Type::Integer (former Type::Long) → i64.
            Type::Integer(s) if s.is_wide() => RetKind::Scalar(" -> i64".into()),
            Type::Integer(_) | Type::Character => RetKind::Scalar(" -> i32".into()),
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
    //
    // Offsets and record size come from the SAME canonical struct schema the
    // interpreter and native codegen consult (`Stores::position` /
    // `Stores::size`) — never a layout re-derived here.  A separate
    // size/offset calculation drifts from the real runtime layout (e.g. a
    // plain `integer` field is an 8-byte i64, not 4 bytes, and loft reorders
    // 8-byte fields ahead of 4-byte record refs for alignment), which
    // silently corrupts every native struct read/write.  @P321c.
    let mut field_modules = String::new();
    for (sname, (d_nr, fields)) in &struct_field_mods {
        let struct_tp = p.data.def(*d_nr).known_type;
        let total_size = p.database.size(struct_tp);

        field_modules.push_str(&format!("/// Field offsets for struct `{sname}`.\n"));
        field_modules.push_str(&format!(
            "/// Record size: {total_size} bytes ({} words).\n",
            total_size.div_ceil(8)
        ));
        field_modules.push_str("#[allow(dead_code)]\n");
        field_modules.push_str(&format!("pub mod {sname}_fields {{\n"));
        for (fname, _, tp) in fields {
            let offset = p.database.position(struct_tp, fname);
            // Skip names that aren't real record fields (e.g. methods that
            // leak into the collected attribute list) — `position` returns
            // u16::MAX for them; emitting a `= 65535` const would be bogus.
            if offset == u16::MAX {
                continue;
            }
            let type_comment = match tp {
                Type::Integer(_) => "integer",
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

    // Validate content. (Cfg-gated: under `--no-default-features` the block
    // above exits; gating the rest keeps clippy's `unreachable_code` /
    // `dead_code` quiet without a blanket `#[allow]`.)
    #[cfg(feature = "registry")]
    {
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
#[cfg(feature = "registry")]
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
#[cfg(feature = "registry")]
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

// `rlibs_in_dir` + `add_native_extern_flags` moved to `native_utils.rs` so the
// native test runner (`test_runner.rs`, in the library crate) can link a
// package's `#native` crate identically to the standalone + WASM native
// compiles (LibCI native library gate).

#[allow(clippy::too_many_lines)]
fn main() {
    // Install SIGSEGV/SIGABRT/SIGBUS handler so crashes print the
    // last-executed opcode before the default handler fires.
    crate::crash_report::install("loft");
    // @PLAN49 T1+T3 — arm the execution-timeout watchdog from the env
    // (`LOFT_TIMEOUT=<secs>`) BEFORE we parse argv.  An explicit
    // `--timeout` later in argv re-arms (no-op — `arm` is idempotent)
    // but the env value is the floor.  MUST be `crate::timeout` (this
    // binary's module instance), not `loft::timeout` (the lib crate's
    // separate copy) — the binary runs its own `crate::` modules
    // (`crate::state::State` etc.), and the `checkpoint_*` call sites in
    // them resolve to `crate::timeout`, so the watchdog + breadcrumb must
    // share that same instance.  Arming `loft::timeout` set a different
    // set of statics the running code never reads.
    crate::timeout::arm(
        crate::timeout::env_timeout_secs(),
        crate::timeout::env_grace_secs(),
    );
    // Plan-07 phase 1 step 1.20 / phase 3 — chain a Rust panic hook
    // that surfaces the loft source position of the offending pc
    // before the default panic message.  Reads the per-thread snapshot
    // published by `State::execute_argv` via `crash_report`.  Falls
    // through to the default hook if no source-span snapshot is
    // active or no entry precedes the offending pc.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let (pc, _op, _fn_d_nr) = crate::crash_report::last_context();
        if pc != u32::MAX
            && let Some(pos) = crate::crash_report::source_loc_for_pc(pc)
        {
            eprintln!("  at {}:{}:{}", pos.file, pos.line, pos.pos);
        }
        prev_hook(info);
    }));
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
    // LibCI: `loft test` / `--tests` default to the interpreter, but honour an
    // EXPLICIT `--native` (matching the `--help` docs for `--tests --native`).
    // Tracked separately because the `test`/`--tests` handlers force interpreter
    // unless native was explicitly requested (regardless of arg order).
    let mut native_requested = false;
    let mut native_release = false;
    // Plan-0.8.5 NDB.0 — `--native-debug` flag: pass `-Cdebuginfo=2`
    // to rustc, drop `-O` (unless `--native-release` is also set),
    // and preserve the generated `.rs` on disk so DWARF's `.debug_line`
    // table points at a real file the debugger can show.
    let mut native_debug = false;
    let mut dump_only = false;
    // None  = flag not given
    // Some("") = flag given without explicit path → use .loft/ default
    // Some(path) = explicit output path
    let mut native_emit: Option<String> = None;
    let mut native_wasm: Option<String> = None;
    // Plan-07 phase 2: --errors=compact|pretty CLI flag (overrides
    // LOFT_ERRORS env var).  None = use env-or-default (Pretty).
    let mut error_mode_arg: Option<String> = None;
    let mut html_out: Option<String> = None;
    let mut tests_dir: Option<String> = None;
    // Plan-08 phase 01: --introspect mode collects per-section
    // selectors, output paths, and filters into one Options bundle.
    // The flag itself only toggles the mode; sub-flags accumulate
    // into `introspect_opts` and are flushed into a real
    // `introspect::Options` after argv parsing.
    let mut introspect_mode = false;
    let mut introspect_sections: Vec<crate::introspect::Section> = Vec::new();
    let mut introspect_bytecode_out: Option<String> = None;
    let mut introspect_rust_out: Option<String> = None;
    let mut introspect_slots_out: Option<String> = None;
    let mut introspect_types_out: Option<String> = None;
    let mut introspect_diff_against: Option<String> = None;
    let mut introspect_trace = false;
    let mut introspect_fn_filter: Vec<String> = Vec::new();
    let mut introspect_all_fns = false;
    let mut native_lib_paths: Vec<String> = Vec::new();
    let mut no_warnings = false;
    let mut deny_warnings = false;
    let mut check_only = false;
    // Phase 6t Tier 4 — `--deps[=direct|=transitive]` walks the current
    // project's dep tree and runs `loft test` in each dep.  None = off,
    // Direct = only manifest.dependencies, Transitive = also deps-of-deps.
    let mut test_deps: Option<&'static str> = None;
    let mut user_args: Vec<String> = Vec::new();

    while i < argv.len() {
        let a = argv[i].as_str();
        i += 1;
        if a == "--version" {
            println!("loft {}", env!("CARGO_PKG_VERSION"));
            return;
        } else if a == "--migrate-long" {
            // C54.B migration tool — rewrite `long` type references and
            // `l` literal suffixes to their post-C54.A equivalents.
            let dry_run = argv.get(i).is_some_and(|s| s == "--dry-run");
            if dry_run {
                i += 1;
            }
            let Some(target) = argv.get(i) else {
                eprintln!("usage: loft --migrate-long [--dry-run] <path-or-dir>");
                std::process::exit(1);
            };
            match loft::migrate_long::migrate_path(std::path::Path::new(target), dry_run) {
                Ok((scanned, modified)) => {
                    if dry_run {
                        println!(
                            "migrate-long (dry run): {scanned} file(s) scanned, \
                             {modified} would be rewritten"
                        );
                    } else {
                        println!(
                            "migrate-long: {scanned} file(s) scanned, \
                             {modified} rewritten"
                        );
                    }
                }
                Err(e) => {
                    eprintln!("migrate-long error: {e}");
                    std::process::exit(1);
                }
            }
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
        } else if let Some(rest) = a.strip_prefix("--errors=") {
            // Plan-07 phase 2: --errors=compact|pretty selects the
            // diagnostic renderer.  Pretty is default; compact is
            // single-line for harnesses + CI.
            error_mode_arg = Some(rest.to_string());
        } else if a == "--errors" {
            // Two-arg form: `--errors compact`.
            error_mode_arg = argv.get(i).cloned();
            if error_mode_arg.is_some() {
                i += 1;
            }
        } else if a == "--dump" {
            native_mode = false;
            dump_only = true;
        } else if a == "--introspect" {
            // Plan-08 phase 01: introspection mode.  Default = emit
            // bytecode + Rust + slots to stdout.  Sub-flags below
            // narrow the section list, redirect per-section output
            // to files, and filter by function name.
            introspect_mode = true;
            native_mode = false;
        } else if a == "--show-bytecode" {
            introspect_sections.push(crate::introspect::Section::Bytecode);
        } else if a == "--show-rust" {
            introspect_sections.push(crate::introspect::Section::Rust);
        } else if a == "--show-slots" {
            introspect_sections.push(crate::introspect::Section::Slots);
        } else if a == "--show-types" {
            introspect_sections.push(crate::introspect::Section::Types);
        } else if a == "--bytecode-out" {
            introspect_bytecode_out = argv.get(i).cloned();
            i += 1;
        } else if a == "--rust-out" {
            introspect_rust_out = argv.get(i).cloned();
            i += 1;
        } else if a == "--slots-out" {
            introspect_slots_out = argv.get(i).cloned();
            i += 1;
        } else if a == "--types-out" {
            introspect_types_out = argv.get(i).cloned();
            i += 1;
        } else if a == "--diff" {
            introspect_diff_against = argv.get(i).cloned();
            i += 1;
        } else if a == "--trace" {
            introspect_trace = true;
        } else if a == "--fn" {
            if let Some(name) = argv.get(i) {
                introspect_fn_filter.push(name.clone());
                i += 1;
            }
        } else if a == "--all-fns" {
            introspect_all_fns = true;
        } else if a == "--native" {
            native_mode = true;
            native_requested = true;
        } else if a == "--native-release" {
            native_mode = true;
            native_requested = true;
            native_release = true;
        } else if a == "--native-debug" {
            // NDB.0 — emit DWARF; combine with --native-release if
            // optimised + debug-info is wanted.
            native_mode = true;
            native_requested = true;
            native_debug = true;
        } else if a == "--dev-soft-halt" {
            // Plan-07 phase 4g.3 — demote dev-mode raises to
            // log-and-continue so a single run surfaces every
            // fault site.  Useful for porting / first-pass
            // scripts where the full pattern of breakage
            // matters more than fast loop-back on one site.
            //
            // SAFETY: set_var is unsafe in Rust 2024.  We set
            // the env var BEFORE any State is created (the
            // State::raise check reads via OnceLock that
            // captures the value on first call), so no
            // concurrent reads are in flight.
            unsafe {
                std::env::set_var("LOFT_DEV_SOFT_HALT", "1");
            }
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
        } else if a == "--html" {
            // single-file HTML export with compiled browser WASM.
            html_out = Some(if argv.get(i).is_some_and(|s| is_output_path(s)) {
                let p = argv[i].clone();
                i += 1;
                p
            } else {
                String::new()
            });
        } else if a == "--tests" {
            // Optional directory/file: consume next non-flag arg.
            // Skip --native/--no-warnings/--deny-warnings that may appear between --tests and the path.
            let mut path = ".".to_string();
            while argv
                .get(i)
                .is_some_and(|s| s == "--native" || s == "--no-warnings" || s == "--deny-warnings")
            {
                if argv[i] == "--native" {
                    // LibCI: opt into native test compilation (matches --help).
                    native_requested = true;
                } else if argv[i] == "--no-warnings" {
                    no_warnings = true;
                } else if argv[i] == "--deny-warnings" {
                    deny_warnings = true;
                }
                i += 1;
            }
            if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                path.clone_from(&argv[i]);
                i += 1;
            }
            tests_dir = Some(path);
            // The test runner defaults to the interpreter; an explicit --native
            // (anywhere on the line) opts into native compilation per file.
            if !native_requested {
                native_mode = false;
            }
        } else if a == "--no-warnings" {
            no_warnings = true;
        } else if a == "--deps" {
            test_deps = Some("transitive");
        } else if a == "--deps=direct" {
            test_deps = Some("direct");
        } else if a == "--deps=transitive" {
            test_deps = Some("transitive");
        } else if a == "--deny-warnings" {
            // Lib CI gate: any Warning-level diagnostic on the run becomes
            // a non-zero exit, just like a parse error.  Used by extracted
            // library chunk CIs to prevent regression of clean libraries.
            // Defaults off so existing consumers are unaffected.
            // Env equivalent: LOFT_DENY_WARNINGS=1
            deny_warnings = true;
        } else if a == "--timeout" {
            // @PLAN49 T3 — `--timeout <secs>` arms the watchdog.  The
            // graceful T2 fault (when shipped) fires at `<secs>`; the
            // hard T1 kill at `<secs> + grace` (default 2s, overridable
            // via `LOFT_TIMEOUT_GRACE`).  `0` disables.
            let secs: u64 = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                eprintln!("--timeout requires a non-negative integer (seconds)");
                std::process::exit(2);
            });
            i += 1;
            crate::timeout::arm(secs, crate::timeout::env_grace_secs());
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
            // `loft test` defaults to the interpreter; an explicit `--native`
            // (e.g. `loft --native test draw`) compiles each test file to native
            // Rust and runs it — the LibCI native library gate uses this.
            if !native_requested {
                native_mode = false;
            }
        } else if a == "install" {
            // Collect flags + positional in any order.
            #[cfg(feature = "registry")]
            let mut install_opts = loft::install::InstallOptions {
                allow_unsigned: true,
                refresh: false,
                offline: false,
                allow_prerelease: false,
            };
            let mut positional: Vec<String> = Vec::new();
            while i < argv.len() {
                let a2 = argv[i].as_str();
                if a2 == "--refresh" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.refresh = true;
                    }
                    i += 1;
                } else if a2 == "--offline" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.offline = true;
                    }
                    i += 1;
                } else if a2 == "--prerelease" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.allow_prerelease = true;
                    }
                    i += 1;
                } else if a2 == "--allow-unsigned" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.allow_unsigned = true;
                    }
                    i += 1;
                } else if a2 == "--require-signature" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.allow_unsigned = false;
                    }
                    i += 1;
                } else if !a2.starts_with('-') {
                    positional.push(a2.to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            // The first positional arg decides path-vs-registry mode.
            // Local-path install (`.`, `./`, `../`, `/`, contains `/`)
            // takes exactly one arg.  Registry install accepts N names.
            let first = positional.first().map(|s| s.as_str()).unwrap_or("");
            let is_local_path = first.is_empty()
                || first.starts_with('/')
                || first.starts_with("./")
                || first.starts_with("../")
                || first == "."
                || first.contains('/');
            if is_local_path {
                let pkg_path = if first.is_empty() {
                    std::env::current_dir().unwrap_or_default()
                } else {
                    std::path::PathBuf::from(first)
                };
                install_package(&pkg_path);
            } else {
                // Registry install — multiple names allowed.
                #[cfg(feature = "registry")]
                {
                    install_from_registry_with_opts(&positional, &install_opts);
                }
                #[cfg(not(feature = "registry"))]
                {
                    for name in &positional {
                        install_from_registry(name);
                    }
                }
            }
            return;
        } else if a == "search" {
            // PKG.REG R8: client-side registry search.
            #[cfg(feature = "registry")]
            {
                let query = argv.get(i).cloned().unwrap_or_default();
                search_registry(&query);
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft search: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "info" {
            // PKG.REG R8: per-package info (versions, latest, deps).
            #[cfg(feature = "registry")]
            {
                let Some(name) = argv.get(i) else {
                    eprintln!("loft info: package name required");
                    std::process::exit(1);
                };
                package_info(name);
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft info: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
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
        } else if a == "package" {
            // PKG.REG R1 (PKG_REGISTRY.md): `loft package [path]` — build a
            // gzipped tarball + print SHA-256 + size + the registry-index
            // entry the publisher pastes into loft-lang/registry.
            // Feature-gated on `registry` because tar / flate2 / sha2
            // aren't worth carrying in a no-default-features build.
            #[cfg(feature = "registry")]
            {
                let pkg_path = if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                    // `i += 1` would be dead since we `return` below, but
                    // the arg is consumed for clarity.
                    std::path::PathBuf::from(&argv[i])
                } else {
                    std::env::current_dir().unwrap_or_default()
                };
                match loft::package::package_create(&pkg_path, None) {
                    Ok(out) => {
                        let stdout = std::io::stdout();
                        let mut lock = stdout.lock();
                        if let Err(e) = loft::package::print_summary(&out, &mut lock) {
                            eprintln!("loft package: print summary failed: {e}");
                            std::process::exit(1);
                        }
                    }
                    Err(e) => {
                        eprintln!("loft package: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!(
                    "loft package: this binary was built without the `registry` feature; \
                     rebuild with default features."
                );
                std::process::exit(1);
            }
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
            // once the script path has been seen, treat every later
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
        // @PLAN49 T3 — default the timeout ON under `loft test` / `--tests`.
        // This is the auto-mode case the watchdog exists for: a hung test or a
        // looping compile in the suite can't be killed interactively, so a
        // generous deadline self-kills it with a breadcrumb.  `arm` is
        // idempotent, so an explicit positive `--timeout <secs>` or
        // `LOFT_TIMEOUT=<secs>` (armed earlier in `main`) overrides this
        // default.  300s is far longer than any single test's compile+run,
        // short enough to catch a true infinite loop.
        crate::timeout::arm(300, crate::timeout::env_grace_secs());
        // Env-var equivalent so external CI doesn't need to thread the flag
        // through `loft test` invocations buried in shell loops.
        let deny_warnings = deny_warnings
            || std::env::var("LOFT_DENY_WARNINGS")
                .map(|v| !v.is_empty() && v != "0")
                .unwrap_or(false);
        let exit_code = run_tests(
            &dir,
            test_dir,
            no_warnings,
            deny_warnings,
            &lib_dirs,
            project.as_deref(),
            native_mode,
            &native_lib_paths,
        );
        // Phase 6t Tier 4 — `loft test --deps` walks the current
        // project's transitive (or direct) dep tree and runs `loft test`
        // in each dep's directory.  Failures are reported per-dep; the
        // process exits non-zero if any dep failed OR if the host
        // project's own tests failed.
        let final_code = if let Some(mode) = test_deps {
            let transitive = mode == "transitive";
            let dep_fail = run_dep_tests(transitive, native_mode);
            i32::from(exit_code != 0 || dep_fail != 0)
        } else {
            exit_code
        };
        std::process::exit(final_code);
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
    // @P296-sibling (Windows-only): `canonicalize()` returns an
    // extended-length `\\?\D:\…` verbatim path, but library `use`
    // resolution (`lib_path` / `probe_*`) builds plain paths.  When the
    // entry file carries the verbatim prefix, a `lib::Name` reference in
    // it registers the module under a source derived from the verbatim
    // path while the same module loaded via `use` uses the plain form —
    // the two sources don't dedup → "Dual definition of <lib>" on
    // Windows (crystal_gold CI).  Strip the verbatim-disk prefix so every
    // path shares one representation.  No-op on Linux/macOS (paths never
    // begin with `\\?\`); only the `\\?\D:\…` disk form is stripped, not
    // verbatim-UNC (`\\?\UNC\…`), which has no plain equivalent.
    let abs_file = if let Some(rest) = abs_file.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        rest.to_string()
    } else {
        abs_file
    };
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
    // @P363: join with a path separator (Path::join) instead of string
    // concat.  `project_dir()` returns a trailing-separator path, but
    // `--path <dir>` sets `dir` to the raw CLI argument with no trailing
    // separator — `dir + "default"` then yielded `<dir>default` and the
    // stdlib `default/` dir was never found.  A missing stdlib is a
    // recoverable CLI fault (wrong `--path`, not a corrupt install), so
    // emit a clean actionable diagnostic and exit non-zero rather than
    // unwrapping the NotFound into a panic.
    let default_dir = std::path::Path::new(&dir).join("default");
    if let Err(e) = p.parse_dir(&default_dir.to_string_lossy(), true, false) {
        eprintln!(
            "loft: cannot load standard library from `{}`: {e}",
            default_dir.display()
        );
        eprintln!(
            "  the `default/` library directory was not found under the \
             compiler path.\n  Pass `--path <dir>` pointing at the directory \
             that contains `default/`,\n  or run `loft` from an installed \
             location where the stdlib is bundled."
        );
        std::process::exit(1);
    }
    let start_def = p.data.definitions();
    // `--show-types --trace`: enable per-expression type recording
    // BEFORE parsing the user file (parse_dir on default/* already
    // ran without tracing — those are stdlib internals).
    if introspect_mode && introspect_trace {
        p.trace_types = true;
    }
    p.parse(&abs_file, false);
    if !p.diagnostics.is_empty() {
        // @P282 fix: when `--no-warnings` is set, suppress
        // Warning-level diagnostics entirely so the program's
        // stdout stays free of warning preambles for piped
        // consumers (the loft-native scanner, viewer state
        // emission, anything machine-readable).  Errors still
        // print and still exit non-zero.
        let print_warnings = !no_warnings;
        let has_errors = p.diagnostics.level() >= Level::Error;
        if print_warnings || has_errors {
            let mode =
                crate::diagnostic_render::ErrorMode::from_cli_and_env(error_mode_arg.as_deref());
            match mode {
                crate::diagnostic_render::ErrorMode::Pretty => {
                    // @P282 — diagnostics (warnings + errors) go to STDERR,
                    // matching the rustc / clang convention.  This keeps the
                    // program's STDOUT free for piped consumers (the loft
                    // scanner, viewer state, any machine-readable output).
                    let loader = crate::diagnostic_render::FileSourceLoader::new();
                    if print_warnings {
                        let out = crate::diagnostic_render::render_pretty_all(
                            &p.diagnostics,
                            &loader,
                            crate::diagnostic_render::ColorMode::Auto,
                        );
                        eprint!("{out}");
                    } else {
                        // Errors-only: re-render entry-by-entry so we
                        // can skip Warning levels.  Mirrors render_pretty_all's
                        // shape minus the warning-cascade dedup (which is
                        // moot when no warnings are emitted).
                        for entry in p.diagnostics.entries() {
                            if entry.level >= Level::Error {
                                let s = crate::diagnostic_render::render_entry_pretty(
                                    entry,
                                    &loader,
                                    crate::diagnostic_render::ColorMode::Auto,
                                );
                                eprint!("{s}");
                                eprintln!();
                            }
                        }
                    }
                }
                crate::diagnostic_render::ErrorMode::Compact => {
                    for entry in p.diagnostics.entries() {
                        if entry.level == Level::Debug {
                            continue;
                        }
                        if !print_warnings && entry.level == Level::Warning {
                            continue;
                        }
                        eprintln!("{}", entry.to_string_compact());
                    }
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
    // store script-level arguments so arguments() returns only these.
    state.database.user_args.clone_from(&user_args);
    compile::byte_code(&mut state, &mut p.data);
    // load native extension shared libraries registered during parsing.
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
            // @P379 — qualify native symbols for functions whose name
            // collides across libraries (no-op without a collision; calls
            // resolve by d_nr so the renamed def stays consistent).
            p.data.namespace_colliding_native_fns();
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
                yield_collect_text: false,
                fn_ref_context: false,
                i32_literal_context: false,
                tuple_text_to_string: false,
                coroutine_persistent_vars: HashSet::new(),
                call_stack_prefix: None,
                wasm_browser: false,
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
        native_utils::add_native_extern_flags(
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

    // --html — compile to browser WASM and assemble self-contained HTML.
    if let Some(ref html_path) = html_out {
        let html_path = if html_path.is_empty() {
            default_artifact_path(&abs_file, "html")
                .to_str()
                .unwrap_or("out.html")
                .to_string()
        } else {
            html_path.clone()
        };
        let end_def = p.data.definitions();
        let rs_path = std::env::temp_dir().join("loft_html.rs");
        {
            let mut f = match std::fs::File::create(&rs_path) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("loft: cannot write source to '{}': {e}", rs_path.display());
                    std::process::exit(1);
                }
            };
            // @P379 — qualify native symbols for functions whose name
            // collides across libraries (no-op without a collision; calls
            // resolve by d_nr so the renamed def stays consistent).
            p.data.namespace_colliding_native_fns();
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
                yield_collect_text: false,
                fn_ref_context: false,
                i32_literal_context: false,
                tuple_text_to_string: false,
                coroutine_persistent_vars: HashSet::new(),
                call_stack_prefix: None,
                wasm_browser: true,
            };
            let main_nr = p.data.def_nr("n_main");
            let entry_defs: Vec<u32> = if main_nr < end_def {
                vec![main_nr]
            } else {
                (start_def..end_def).collect()
            };
            if let Err(e) = out.output_native_reachable(&mut f, start_def, end_def, &entry_defs) {
                eprintln!("loft: html code generation failed: {e}");
                std::process::exit(1);
            }
        }
        // Compile to wasm32-unknown-unknown cdylib
        let wasm_path = std::env::temp_dir().join("loft_html.wasm");
        let mut cmd = std::process::Command::new("rustc");
        cmd.arg("--edition=2024")
            .arg("--target")
            .arg("wasm32-unknown-unknown")
            .arg("--crate-type")
            .arg("cdylib")
            .arg("-O")
            .arg("-o")
            .arg(&wasm_path)
            .arg(&rs_path);
        if let Some(lib_dir) = loft_lib_dir_for(Some("wasm32-unknown-unknown")) {
            cmd.arg("--extern")
                .arg(format!("loft={}", lib_dir.join("libloft.rlib").display()));
            let deps = lib_dir.join("deps");
            cmd.arg("-L").arg(format!("dependency={}", deps.display()));
            // W1.1 env fix: libloft.rlib depends on wasm-bindgen, which pulls
            // in the proc-macro crate wasm_bindgen_macro.  Proc-macros are
            // always built for the host (never for wasm32), so rustc needs
            // the *host* deps directory on its search path in addition to
            // the target deps dir.  Without this, compilation fails with:
            //   error[E0463]: can't find crate for `wasm_bindgen_macro`
            // and subsequent errors cascade (every `use loft::...` fails,
            // so `cr_call_push` is reported unfound as a collateral).
            if let Some(host_lib_dir) = loft_lib_dir_for(None) {
                let host_deps = host_lib_dir.join("deps");
                if host_deps.exists() {
                    cmd.arg("-L")
                        .arg(format!("dependency={}", host_deps.display()));
                }
            }
        }
        let status = cmd.status();
        let _ = std::fs::remove_file(&rs_path);
        match status {
            Ok(s) if s.success() => {}
            Ok(_) => {
                eprintln!("loft: browser WASM compilation failed");
                std::process::exit(1);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("loft: rustc not found; install the Rust toolchain to use --html");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("loft: failed to launch rustc: {e}");
                std::process::exit(1);
            }
        }
        // wasm-opt: optimize size + enable asyncify for frame yield.
        // Asyncify lets loft_gl_swap_buffers suspend the WASM execution
        // so the browser can render the frame via requestAnimationFrame.
        let opt_path = std::env::temp_dir().join("loft_html_opt.wasm");
        let final_wasm = if std::process::Command::new("wasm-opt")
            .args([
                // -O / -Oz plus --asyncify strips the host imports
                // (loft_gl.*, loft_io.*) entirely — wasm goes from 25
                // imports to 0 and every GL call runtime-panics as
                // "unreachable executed".  -O1 with the explicit
                // --asyncify pass keeps imports intact while still
                // producing a smaller, asyncify-ready bundle.
                "-O1",
                "--strip-debug",
                "--strip-producers",
                "--asyncify",
                "--pass-arg=asyncify-imports@loft_gl.loft_gl_swap_buffers",
            ])
            .arg("-o")
            .arg(&opt_path)
            .arg(&wasm_path)
            .status()
            .is_ok_and(|s| s.success())
        {
            let _ = std::fs::remove_file(&wasm_path);
            opt_path
        } else {
            // @P337: a missing wasm-opt is NOT a cosmetic "larger output"
            // problem — without the `--asyncify` pass the bundle cannot
            // frame-yield, so the HTML driver runs loft_start() synchronously
            // and any render loop (`for _ in 0..N`) blocks the browser main
            // thread forever ("page times out").  Warn loudly so a hung
            // bundle is never shipped unknowingly.
            eprintln!(
                "WARNING: a required tool ('wasm-opt', from the 'binaryen' \
                 package) is not installed.\n  \
                 Without it this game page will FREEZE the browser tab \
                 (it locks up and never draws).\n  \
                 Install it and rebuild before publishing — e.g. `apt \
                 install binaryen`, or run `make doctor` for the command \
                 for your system."
            );
            wasm_path
        };
        // Assemble HTML
        let wasm_bytes = std::fs::read(&final_wasm).unwrap_or_default();
        let _ = std::fs::remove_file(&final_wasm);
        // @P350: self-validate the emitted wasm BEFORE writing the HTML so a
        // bare `loft --html` never ships the silently-broken "rlib stomp"
        // bundle.  `make wasm` (wasm-pack, feature=wasm) and `--html` write
        // the SAME target/wasm32-unknown-unknown/release/libloft.rlib with
        // incompatible feature sets; if --html links the wasm-bindgen variant
        // the wasm imports `__wbindgen_placeholder__` (35+), which the
        // embedded loft-gl-wasm.js glue (raw loft_gl/loft_io externs only)
        // can't provide → the page fails to instantiate.  A correct --html
        // bundle imports ONLY `loft_gl` + `loft_io`.  Same check as
        // tools/check_html_bundle.mjs, but inline so it guards the bare
        // command, not just `make game`.  (The asyncify/wasm-opt footgun is
        // handled by the loud warning above — it's conditional on whether the
        // program frame-yields, so it must not hard-abort compute-only bundles.)
        if let Err(bad_mods) = html_wasm_import_modules_ok(&wasm_bytes) {
            eprintln!(
                "loft: --html produced a BROKEN bundle — the wasm imports \
                 unexpected module(s) {bad_mods:?}.\n  \
                 The wasm32-unknown-unknown libloft.rlib was built with the \
                 `wasm` (wasm-bindgen) feature — most likely a prior `make \
                 wasm` stomped it (see WASM.md § The rlib-stomp hazard).\n  \
                 Rebuild the rlib in the --html shape, then re-run --html:\n    \
                 cargo build --release --target wasm32-unknown-unknown --lib \
                 --no-default-features --features random\n  \
                 (No HTML was written — a stomped bundle does not instantiate \
                 in the browser.)"
            );
            std::process::exit(1);
        }
        let wasm_b64 = crate::base64::encode(&wasm_bytes);
        let title = std::path::Path::new(&file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Loft Program".to_string());
        let gl_js = include_str!("../doc/loft-gl-wasm.js");
        let html = format!(
            r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{title}</title>
<style>body{{margin:0;background:#000;display:flex;justify-content:center;align-items:center;height:100vh}}canvas{{display:block}}pre{{color:#0f0;font-size:14px}}</style>
</head><body>
<canvas id="c" tabindex="0" style="display:none"></canvas>
<pre id="out"></pre>
<script>
{gl_js}
const wasmB64="{wasm_b64}";
const wasmBytes=Uint8Array.from(atob(wasmB64),c=>c.charCodeAt(0));
const canvas=document.getElementById('c');
const output=document.getElementById('out');
let mem;
const ctrl={{ac:null}};
const imports=buildLoftImports(canvas,output,()=>mem,ctrl);
WebAssembly.instantiate(wasmBytes,imports).then(r=>{{
  mem=r.instance.exports.memory;
  if(r.instance.exports.asyncify_start_unwind){{
    const ac=new AsyncifyCtrl(r.instance);
    ctrl.ac=ac;
    ac.start('loft_start');
    if(ac.sleeping){{
      (function frame(){{
        if(ac.resume('loft_start'))requestAnimationFrame(frame);
      }})();
    }}
  }}else{{
    r.instance.exports.loft_start();
  }}
}});
</script></body></html>"#
        );
        if let Err(e) = std::fs::write(&html_path, &html) {
            eprintln!("loft: cannot write HTML to '{html_path}': {e}");
            std::process::exit(1);
        }
        let wasm_kb = wasm_bytes.len() / 1024;
        let html_kb = html.len() / 1024;
        println!("wrote {html_path} ({html_kb} KB, WASM {wasm_kb} KB)");
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
            // Default `loft <file>` writes to a per-process tmp file
            // so concurrent invocations (e.g. nextest's parallel test
            // execution) don't race on a single shared
            // `temp_dir/loft_native.rs`.  The PID suffix is a process-
            // local choice; user-visible artefacts pass through
            // `--native-emit <path>` which the user controls.
            None => std::env::temp_dir().join(format!("loft_native_{}.rs", std::process::id())),
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
            // @P379 — qualify native symbols for functions whose name
            // collides across libraries (no-op without a collision; calls
            // resolve by d_nr so the renamed def stays consistent).
            p.data.namespace_colliding_native_fns();
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
                yield_collect_text: false,
                fn_ref_context: false,
                i32_literal_context: false,
                tuple_text_to_string: false,
                coroutine_persistent_vars: HashSet::new(),
                call_stack_prefix: None,
                wasm_browser: false,
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
                    // P199 — wrap Stores in UnsafeCell so the native ABI
                    // can pass `&UnsafeCell<Stores>` instead of `&mut Stores`,
                    // eliminating E0499 in nested user-fn calls.  Each
                    // generated function derives its own short-lived
                    // `&mut Stores` from the cell at function entry.
                    let _ = writeln!(
                        f,
                        "    let cell = std::cell::UnsafeCell::new(Stores::new());"
                    );
                    let _ = writeln!(
                        f,
                        "    let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                    );
                    let _ = writeln!(f, "    init(&cell);");
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
                            let _ = writeln!(f, "    {name}(&cell);");
                        } else {
                            let _ = writeln!(f, "    {name}(&cell, {});", work_args.join(", "));
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
            // Include the release + debug flags in the hash so each
            // distinct rustc invocation produces a distinct cached
            // binary.  Without this, switching between `--native`
            // and `--native --native-debug` returns whichever build
            // ran first — debugger sees a stripped binary, or a
            // user accidentally runs an unoptimised build under
            // `--native-release` because the debug build was cached.
            native_release.hash(&mut h);
            native_debug.hash(&mut h);
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

        // P254 — cache-poisoning defense.  Bypass the cache entirely
        // when the user opts out via `LOFT_NATIVE_NO_CACHE=1` (matches
        // the behaviour the workaround in PROBLEMS.md described before
        // this fix landed; documented here so paranoid users have a
        // hard kill switch even if the safety helpers gain a future
        // bug).  We also reject the cache when the cached file fails
        // the safety helpers — symlinked cache, wrong owner uid,
        // group/other-readable, or SUID-set.  In every reject case we
        // recompile (rather than refusing to run) so a poisoned cache
        // doesn't deny the user service; it just costs them a
        // recompile.
        let no_cache = std::env::var("LOFT_NATIVE_NO_CACHE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let cache_usable = !no_cache
            && cached_binary.exists()
            && native_utils::cache_safe_to_execute(&cached_binary);
        if !no_cache && cached_binary.exists() && !cache_usable {
            eprintln!(
                "loft: rejecting suspicious cached binary at {} (P254 — wrong owner, world-writable, symlink, or SUID); recompiling",
                cached_binary.display()
            );
        }

        // Use cached binary if it exists AND passes the safety check;
        // otherwise compile and cache.
        let binary = if cache_usable {
            cached_binary.clone()
        } else {
            // Per-process tmp path — same rationale as the emit_path
            // above: avoids races between concurrent `loft <file>`
            // invocations.  The cached path (`cached_binary` above)
            // is content-addressed (source_hash) and thus safe to
            // share across processes; this fallback is only used
            // when the cache miss; it doesn't need to be content-
            // addressed but does need to be unique per process.
            let binary =
                std::env::temp_dir().join(format!("loft_native_bin_{}", std::process::id()));
            let mut cmd = std::process::Command::new("rustc");
            cmd.arg("--edition=2024")
                .arg("-o")
                .arg(&binary)
                .arg(&emit_path);
            if native_release {
                cmd.arg("-O");
            }
            // NDB.0 — when --native-debug is set, emit DWARF debug
            // info so stock GDB / LLDB can step through the native
            // binary.  Combines with --native-release: the user gets
            // an optimised build with debug info if both flags are
            // present.
            if native_debug {
                cmd.arg("-Cdebuginfo=2");
            }
            // P266 follow-up: each native package's rlib carries a copy
            // of `loft_register_v1` (synthesized by the `loft_ffi::loft_register!`
            // macro for the cdylib's dlopen registration path).  When two or
            // more native packages are pulled into the SAME --native binary
            // (e.g. the viewer pulls lib/web AND lib/server transitively),
            // ld errors with `duplicate symbol: loft_register_v1`.  The
            // binary never calls `loft_register_v1` (it inlines
            // `loft_<crate>::n_…` directly), so the duplicates are
            // functionally harmless.  Tell the linker to merge them
            // (keep the first definition, skip the rest).  This matches
            // the GNU ld / lld semantics for `-z muldefs`.
            //
            // macOS ld64 rejects `--allow-multiple-definition` as an
            // unknown option (the Apple linker has no equivalent
            // surface — duplicate symbols are either silently
            // permitted by symbol kind, or errors).  Skip the flag on
            // macOS; if a future cross-package binary hits a real
            // duplicate-symbol error on macOS, we'll narrow the fix
            // (e.g. weak-link the symbol or dedup the macro emission)
            // rather than re-add a flag the host linker doesn't
            // support.
            #[cfg(not(target_os = "macos"))]
            cmd.arg("-Clink-arg=-Wl,--allow-multiple-definition");
            let native_deps_dir = if let Some(lib_dir) = loft_lib_dir() {
                cmd.arg("--extern")
                    .arg(format!("loft={}", lib_dir.join("libloft.rlib").display()));
                // `loft_lib_dir()` returns either the binary's directory
                // (when libloft.rlib lives next to the binary) or the
                // `deps/` subdirectory (cargo's standard layout — macOS
                // and Windows always; Linux when the canonical sibling
                // is absent).  In the latter case `lib_dir` IS already
                // the deps directory; appending "deps" yields an
                // invalid `target/release/deps/deps` path that rustc
                // can't search, leading to E0463 "can't find crate"
                // for transitive deps like rand_core.
                //
                // Detect the deps-already case via the directory name.
                let deps = if lib_dir.file_name().is_some_and(|n| n == "deps") {
                    lib_dir.clone()
                } else {
                    lib_dir.join("deps")
                };
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
                // Propagate `-L native=` for every build-script `OUT_DIR`
                // that bundles a native lib.  Windows-targets ships
                // `windows.0.48.5.lib` inside its OUT_DIR; without these
                // paths the link step fails with `LNK1181: cannot open
                // input file 'windows.0.48.5.lib'`.
                for out_dir in build_script_native_lib_dirs(&lib_dir) {
                    cmd.arg("-L").arg(format!("native={}", out_dir.display()));
                }
                Some(deps)
            } else {
                None
            };
            // PKG.4: add --extern flags for native packages.
            native_utils::add_native_extern_flags(
                &mut cmd,
                &p.data,
                None,
                native_deps_dir.as_deref(),
            );
            let output = cmd.output();
            let output = match output {
                Ok(o) => o,
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
            // Relay rustc's own output to the user.
            let _ = std::io::Write::write_all(&mut std::io::stderr(), &output.stderr);
            let _ = std::io::Write::write_all(&mut std::io::stdout(), &output.stdout);
            let status = output.status;
            if !status.success() {
                // detect the rand_core / cargo-cache staleness and print a
                // clear recovery hint instead of a generic codegen-bug message.
                let stderr_utf8 = String::from_utf8_lossy(&output.stderr);
                let crate_resolution_failure = (stderr_utf8.contains("E0460")
                    || stderr_utf8.contains("E0463"))
                    && (stderr_utf8.contains("rand_core")
                        || stderr_utf8.contains("possibly newer version of crate")
                        || stderr_utf8.contains("can't find crate"));
                if crate_resolution_failure {
                    // Print the rustc invocation + the deps directory listing
                    // so the diagnostic shows what was actually attempted.
                    // Surfaces the Windows latent issue + any future
                    // platform-specific dep-resolution gaps.
                    eprintln!("\nloft: rustc could not resolve a transitive crate dep.");
                    eprintln!("\nrustc invocation:");
                    let prog = cmd.get_program().to_string_lossy().to_string();
                    eprintln!("  {prog}");
                    for arg in cmd.get_args() {
                        eprintln!("    {}", arg.to_string_lossy());
                    }
                    if let Some(deps) = native_deps_dir.as_ref() {
                        eprintln!("\nDeps directory: {}", deps.display());
                        match std::fs::read_dir(deps) {
                            Ok(rd) => {
                                let mut entries: Vec<_> = rd
                                    .flatten()
                                    .filter_map(|e| {
                                        let n = e.file_name().to_string_lossy().to_string();
                                        let is_rlib = std::path::Path::new(&n)
                                            .extension()
                                            .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"));
                                        if n.contains("rand") || n.contains("loft") || is_rlib {
                                            Some(n)
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                entries.sort();
                                if entries.is_empty() {
                                    eprintln!("  (no rlibs found in deps directory)");
                                } else {
                                    for n in entries.iter().take(40) {
                                        eprintln!("  {n}");
                                    }
                                    if entries.len() > 40 {
                                        eprintln!("  ... ({} more)", entries.len() - 40);
                                    }
                                }
                            }
                            Err(e) => eprintln!("  (cannot read deps directory: {e})"),
                        }
                    } else {
                        eprintln!(
                            "\nNo deps directory was passed to rustc — `loft_lib_dir()` returned None."
                        );
                    }
                    eprintln!(
                        "\nMost likely cause: cached `libloft.rlib` references a different \
                         dependency version than the one now in `target/release/deps/`, \
                         or the deps directory is missing the named crate.\n\n\
                         Fix:  cargo build --release --lib --bin loft\n\
                         Or:   cargo clean && cargo build --release\n"
                    );
                } else {
                    eprintln!(
                        "loft: native compilation failed (codegen bug — try --native-emit to inspect the source)"
                    );
                }
                std::process::exit(1);
            }
            // Store in cache for next run.  P254 — also opt out
            // when LOFT_NATIVE_NO_CACHE=1 is set so paranoid users
            // can avoid leaving a cache file on disk at all.
            if !no_cache && std::fs::create_dir_all(&cache_dir).is_ok() {
                // P254 — tighten cache-dir mode to 0700 so a future
                // attacker can't drop files into our cache (or
                // remove ours from under us).  Repairs pre-existing
                // wider-mode cache directories left over from earlier
                // loft versions.  No-op on non-Unix.
                native_utils::tighten_cache_dir(&cache_dir);
                if !native_utils::cache_dir_safe(&cache_dir) {
                    // Couldn't tighten the directory — bail on the
                    // cache write so we don't write a binary the
                    // next run will reject anyway.  Common case:
                    // the cache dir lives on a network mount whose
                    // server enforces a different mode than we
                    // requested.
                    eprintln!(
                        "loft: cache directory {} has unsafe permissions and could not be tightened; skipping cache write (P254)",
                        cache_dir.display()
                    );
                } else {
                    // Remove stale cached binaries for THIS source file only.
                    let prefix = format!("{source_stem}-");
                    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
                        for entry in entries.flatten() {
                            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                                let _ = std::fs::remove_file(entry.path());
                            }
                        }
                    }
                    if std::fs::copy(&binary, &cached_binary).is_ok() {
                        // P254 — tighten the freshly written binary
                        // to 0700.  std::fs::copy preserves source
                        // mode, which for `/tmp/loft_native_bin_<pid>`
                        // is typically 0644 — wider than we want.
                        native_utils::tighten_cache_binary(&cached_binary);
                    }
                }
            }
            binary
        };
        // NDB.0 — preserve the generated .rs on disk when
        // --native-debug is set so DWARF's `.debug_line` table points
        // at a real file the debugger can show.  Without this, GDB /
        // LLDB show `(no source)` even though debug info is present.
        if !native_debug {
            let _ = std::fs::remove_file(&emit_path);
        } else {
            eprintln!(
                "loft: source preserved at {} (--native-debug)",
                emit_path.display()
            );
        }

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
    // Plan-08 phase 01: --introspect short-circuits everything.
    // Bypass execution; emit bytecode + Rust + slots and exit.
    if introspect_mode {
        let trace_lines = std::mem::take(&mut p.trace_types_lines);
        let opts = crate::introspect::Options {
            sections: introspect_sections.clone(),
            bytecode_out: introspect_bytecode_out.clone(),
            rust_out: introspect_rust_out.clone(),
            slots_out: introspect_slots_out.clone(),
            types_out: introspect_types_out.clone(),
            diff_against: introspect_diff_against.clone(),
            trace_lines,
            fn_filter: introspect_fn_filter.clone(),
            all_fns: introspect_all_fns,
            lib_dirs: Vec::new(),
            install_dir: String::new(),
        };
        let end_def = p.data.definitions();
        if let Err(e) = crate::introspect::emit_all(&mut p.data, &mut state, end_def, &opts) {
            eprintln!("loft: introspect failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    if main_nr == u32::MAX && !dump_only {
        // No main() — wrap each zero-parameter user function in a synthetic
        // main() that calls it. This ensures proper scope cleanup: stores
        // allocated by struct-returning functions are freed when the caller's
        // variables go out of scope, before the leak check runs.
        //
        // `--dump` skips this wrap-and-execute path — it wants to see the
        // bytecode of the user functions as parsed, not a synthetic caller
        // (and the synthetic caller may itself panic on buggy user code,
        // aborting before the dump is written).
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
    } else if dump_only {
        // --dump: compile to bytecode, dump to stderr, exit (no execution).
        // Respects LOFT_LOG for extra detail (e.g. LOFT_LOG=variables --dump).
        let config = if std::env::var("LOFT_LOG").is_ok() {
            log_config::LogConfig::from_env()
        } else {
            log_config::LogConfig::static_only()
        };
        let mut log = std::io::stderr();
        let _ = state.dump_bytecode(&mut log, &config, &mut p.data);
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
    // Plan-07 phase 4 — render typed runtime errors through the
    // phase-2 pretty renderer.  `panic("msg")`, failed `assert`, and
    // every fault-site opcode populate `state.database.runtime_error`;
    // pulling it out here avoids a borrow conflict with the renderer's
    // loader and keeps the existing `had_fatal` exit path intact.
    //
    // Skip `check_store_leaks` when a runtime error halted execution:
    // the abrupt halt skips scope-exit cleanup so owned vectors / texts
    // remain held, but those leaks are EXPECTED and the warning would
    // bury the error message users actually care about.  Keep the
    // leak check for clean exits where it still surfaces real bugs.
    let runtime_err = state.database.runtime_error.take();
    if runtime_err.is_none() {
        state.check_store_leaks();
    }
    if let Some(err) = runtime_err {
        let entry = err.to_diag_entry();
        let loader = crate::diagnostic_render::FileSourceLoader::new();
        let color = crate::diagnostic_render::ColorMode::Auto;
        let rendered = crate::diagnostic_render::render_entry_pretty(&entry, &loader, color);
        eprint!("{rendered}");
        // Plan-07 phase 4g.1 / 4g.2 slice 1 — render the
        // call-chain captured at raise time after the typed-
        // error block.  Innermost first so the eye lands on
        // the function the fault fired in; chevron points
        // outward to indicate the call sequence.  Top-level
        // (single-frame) chains are skipped; the source
        // location already names the function in spirit via
        // its file:line:col.
        if err.call_chain.len() > 1 {
            let trimmed: Vec<&str> = err
                .call_chain
                .iter()
                .map(String::as_str)
                .take(5) // top 5 frames; rest summarised
                .collect();
            eprintln!("  in fn {}() ← called from", trimmed[0]);
            for name in &trimmed[1..] {
                eprintln!("        fn {name}()");
            }
            if err.call_chain.len() > 5 {
                eprintln!("        … ({} more frames)", err.call_chain.len() - 5);
            }
        }
    }
    if state.database.had_fatal {
        std::process::exit(1);
    }
}
