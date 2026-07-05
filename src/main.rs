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

use loft::base64;
use loft::compile;
use loft::data;
use loft::diagnostics;
pub mod diagnostic_render;
use loft::extensions;
use loft::formatter;
use loft::generation;
use loft::log_config;
use loft::logger;
use loft::manifest;
mod native_utils;
use loft::parser;
use loft::platform;
use loft::scopes;
use loft::state;
mod test_runner;

use crate::native_utils::{
    build_script_native_lib_dirs, default_artifact_path, deps_dir_of, html_wasm_import_modules_ok,
    is_output_path, loft_lib_dir, loft_lib_dir_for, project_dir,
};
use crate::test_runner::run_tests;
use loft::diagnostics::Level;
use loft::state::State;
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
    println!("                                for headless/WASI (wasmtime); NOT the browser build");
    println!(
        "  --html [out.html]             compile to a self-contained browser page (the browser"
    );
    println!(
        "                                target: WebGL2 canvas + keyboard/mouse, println output;"
    );
    println!(
        "                                default: <script>.html) — guide: doc/claude/WEB_APPS.md"
    );
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
    println!("  repl                          start the interactive REPL (also: bare `loft`)");
    println!("                                resumes your last session automatically;");
    println!("                                --fresh starts clean (ignores the saved session)");
    println!("  debug <file>:<line>           run <file> under the debugger, breaking at <line>");
    println!(
        "                                (inspect/edit/step the live frame; :help at the prompt)"
    );
    println!("  check <file>                  same as --check <file>");
    println!(
        "  sandbox-check <file>          report the @PLN86 sandbox admission verdict and STOP"
    );
    println!("                                (Admitted / Rejected + diagnostics; never executes)");
    println!("  test [target]                 run package tests (requires loft.toml in cwd)");
    println!("                                test         — run all tests in tests/");
    println!("                                test draw    — run tests/draw.loft");
    println!("                                test draw::f — run a single test function");
    println!("  install [target]              install a package to ~/.loft/lib/ for global use");
    println!("                                install .        — install package in current dir");
    println!("                                install /p       — install package at /p");
    println!("                                install name     — download latest from registry");
    println!("                                install name@v   — download specific version");
    println!("  pin <script.loft>             pin every registry library the script uses");
    println!("                                writes <script>.loft.lock next to the script;");
    println!("                                subsequent runs use the pinned versions");
    println!("  list-installed                list every registry package installed locally");
    println!("                                (from ~/.loft/registry/), annotated with sha256");
    println!("                                + size + index status (active / yanked / orphan)");
    println!("  api [name]                    discover library APIs without leaving the shell:");
    println!("                                api            — list libraries reachable from here");
    println!("                                api <name>     — print its public API surface");
    println!(
        "                                api --registry [--refresh] — the whole installable catalog"
    );
    println!("                                (pub signatures + doc comments, bodies stripped)");
    println!("  audit                         check every installed package against the");
    println!("                                advisory feed; exit 0 if clean, 1 if any low/bug,");
    println!("                                2 if any high, 3 if any security_critical");
    println!("  update [pkg]                  refresh lockfile to latest active versions");
    println!("                                that satisfy each dep's loft.toml range;");
    println!("                                --dry-run reports changes without writing;");
    println!("                                --check exits 1 if any updates are available");
    println!("  bundle export [opts] <dir>    write a self-contained offline bundle of");
    println!("                                registry packages (--all or --packages X,Y,Z);");
    println!("                                ship via USB/scp; import on the target machine");
    println!("  bundle import <dir>           install a bundle into ~/.loft/registry/");
    println!("                                (verifies sha256 + signature per artifact)");
    println!("  publish [pkg]                 author helper: repackage locally + verify the");
    println!("                                GitHub release exists + emit the index.json");
    println!("                                entry to paste into a registry PR");
    println!("  new <name> [opts]             scaffold a fresh library: loft.toml + src/ +");
    println!("                                tests/ + README; --native adds native/ skeleton;");
    println!("                                --chunk adds .github/workflows/library-ci.yml");
    println!("  yank <pkg>@<ver> --severity   author helper: draft a registry yank PR — emits");
    println!("    <tier> --advisory <id>      the typed `status` entry for index.json + the");
    println!("    --summary <text>            advisories.json row, cross-referenced");
    println!("    --affected <range>");
    println!("    --fixed-in <ver>");
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
    println!("  build-native [pkg-dir]        build the package's native cdylib for this host");
    println!("                                + print its path, triple, and loft-ffi fp, so CI");
    println!(
        "                                can publish it as a prebuilt/<triple>/ binary (@PLN21)"
    );
    println!("  search [query]                client-side search of the package registry");
    println!(
        "                                matches name / description / categories (case-insensitive);"
    );
    println!("                                no query lists all; --json for machine output");
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
        if let Some(p) = loft::manifest::extract_path_dep(value) {
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
        let Some(manifest) = loft::manifest::read_manifest(manifest_path.to_str().unwrap_or(""))
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

/// PKG.REG R8 — `loft search [query]`: client-side filter against the cached
/// index (reuses `loft install`'s fetch/verify path via `install::load_index`).
/// Ranks hits exact-name > name-prefix > description/category; marks a package
/// whose latest version declares lazy-load `triggers` with `⚡auto-use`; and
/// prints a `loft install` hint under each hit when a query is given.  `json`
/// emits the same result set as a JSON array for tooling.
#[cfg(feature = "registry")]
fn search_registry(query: &str, json: bool) {
    use loft::install::InstallOptions;
    use loft::registry_index;

    let opts = InstallOptions {
        allow_unsigned: true,
        ..Default::default()
    };
    let loaded = match loft::install::load_index_reporting(&opts) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("loft search: {e}");
            std::process::exit(1);
        }
    };
    if loaded.stale_fallback {
        eprintln!("loft search: registry unreachable — showing cached index");
    }
    let index = loaded.index;

    // S8 — the stdlib's function-level surface, extracted from the binary's
    // EMBEDDED `default/*.loft` at search time (one source of truth shared with
    // the WASM runtime, no disk dependency): stdlib functions appear as hits
    // identical in shape to a library's, and never bloat the fetched index.
    let stdlib: Vec<registry_index::ApiItem> = loft::stdlib_sources::STDLIB_SOURCES
        .iter()
        .flat_map(|(_, content)| loft::documentation::extract_api_items(content))
        .collect();

    let q = query.to_ascii_lowercase();
    let results = registry_index::search_results(&index, &stdlib, &q);

    if json {
        println!(
            "{}",
            loft::json::to_json_string(&search_results_json(&results))
        );
        return;
    }

    if results.is_empty() {
        println!("No packages or functions match `{query}`.");
        return;
    }
    let querying = !q.is_empty();
    for r in &results {
        // S9 — package header (stdlib reads "built in", no version/install).
        if r.is_stdlib {
            println!("stdlib (built in)");
        } else {
            let marker = if r.auto_use { " ⚡auto-use" } else { "" };
            let mut line = format!("{} {}{marker}", r.name, r.version);
            if let Some(d) = r.description.as_deref().filter(|d| !d.is_empty()) {
                line.push_str(" — ");
                line.push_str(d);
            }
            if !r.categories.is_empty() {
                line.push_str(&format!("  ({})", r.categories.join(", ")));
            }
            println!("{line}");
        }
        // The matching functions: what it does (doc) + how to call it (sig).
        for item in &r.fns {
            println!("    {}", item.sig);
            // Display only the first line of the (now full) doc paragraph; the
            // whole paragraph stays searchable and travels in `--json`.
            if let Some(summary) = item.doc.lines().next().filter(|l| !l.is_empty()) {
                println!("        {summary}");
            }
        }
        // How to get it.
        if querying {
            if r.is_stdlib {
                println!("    built in — use directly, no install");
            } else {
                println!("    → loft install {}", r.name);
            }
        }
    }
}

/// One `loft search --json` record (S9): everything an agent needs to decide
/// and call — `source` (`stdlib`|`registry`), `package`, `version`, `signature`
/// (null for a metadata-only hit), one-line `doc`, and `get` (the install line,
/// or "built in" for the stdlib).
#[cfg(feature = "registry")]
fn search_record(
    source: &str,
    name: &str,
    version: &str,
    signature: loft::json::Parsed,
    doc: loft::json::Parsed,
    get: &str,
) -> loft::json::Parsed {
    use loft::json::Parsed;
    Parsed::Object(vec![
        ("source".to_string(), 0, Parsed::Str(source.to_string())),
        ("package".to_string(), 0, Parsed::Str(name.to_string())),
        ("version".to_string(), 0, Parsed::Str(version.to_string())),
        ("signature".to_string(), 0, signature),
        ("doc".to_string(), 0, doc),
        ("get".to_string(), 0, Parsed::Str(get.to_string())),
    ])
}

/// Build the `loft search --json` payload (S9): a FLAT array of function-level
/// records (see [`search_record`]).  Replaces the S0–S5 per-package shape: each
/// matching function is its own record carrying its package context; a
/// metadata-only hit contributes one `signature: null` record (description as
/// `doc`) so nothing is dropped.
#[cfg(feature = "registry")]
fn search_results_json(results: &[loft::registry_index::SearchResult]) -> loft::json::Parsed {
    use loft::json::Parsed;
    let mut arr: Vec<Parsed> = Vec::new();
    for r in results {
        let source = if r.is_stdlib { "stdlib" } else { "registry" };
        let get = if r.is_stdlib {
            "built in — use directly".to_string()
        } else {
            format!("loft install {}", r.name)
        };
        if r.fns.is_empty() {
            let doc = r.description.clone().map_or(Parsed::Null, Parsed::Str);
            arr.push(search_record(
                source,
                &r.name,
                &r.version,
                Parsed::Null,
                doc,
                &get,
            ));
        } else {
            for item in &r.fns {
                arr.push(search_record(
                    source,
                    &r.name,
                    &r.version,
                    Parsed::Str(item.sig.clone()),
                    Parsed::Str(item.doc.clone()),
                    &get,
                ));
            }
        }
    }
    Parsed::Array(arr)
}

/// The `api` array for one source dir, as `[{ "sig": …, "doc": … }, …]` — the
/// shared shape the index `api` field carries and `loft api --json` emits, so
/// the registry CI can re-derive it from source and reject a pasted mismatch
/// (S7-CI).  Used by `loft publish` (the entry) and `loft api --json`.
#[cfg(feature = "registry")]
fn api_items_json(items: &[loft::registry_index::ApiItem]) -> loft::json::Parsed {
    use loft::json::Parsed;
    Parsed::Array(
        items
            .iter()
            .map(|item| {
                Parsed::Object(vec![
                    ("sig".to_string(), 0, Parsed::Str(item.sig.clone())),
                    ("doc".to_string(), 0, Parsed::Str(item.doc.clone())),
                ])
            })
            .collect(),
    )
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
        lock_path: None,
    };
    let loaded = match loft::install::load_index_reporting(&opts) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("loft info: {e}");
            std::process::exit(1);
        }
    };
    if loaded.stale_fallback {
        eprintln!("loft info: registry unreachable — showing cached index");
    }
    let index = loaded.index;

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

/// PKG.STUB — resolve a library name to a readable package directory, trying
/// the places a consumer's `use` would be served from: an explicit path,
/// project-local `lib/<name>/`, user `~/.loft/lib/<name>/`, then the newest
/// installed registry copy.
#[cfg(feature = "registry")]
fn api_resolve_pkg_dir(name: &str) -> Option<std::path::PathBuf> {
    if name.contains('/') || name == "." {
        let direct = std::path::PathBuf::from(name);
        return direct.join("loft.toml").exists().then_some(direct);
    }
    let project = std::path::PathBuf::from("lib").join(name);
    if project.join("loft.toml").exists() {
        return Some(project);
    }
    let user = loft_home().join("lib").join(name);
    if user.join("loft.toml").exists() {
        return Some(user);
    }
    let mut hits: Vec<(Vec<u32>, std::path::PathBuf)> = loft::registry_index::installed_packages()
        .into_iter()
        .filter(|(n, _, _)| n == name)
        .map(|(_, v, p)| (numeric_version(&v), p))
        .collect();
    hits.sort();
    hits.pop().map(|(_, p)| p)
}

/// `~/.loft/` honouring `LOFT_HOME` (the same base `registry_index::cache_dir`
/// resolves against).
#[cfg(feature = "registry")]
fn loft_home() -> std::path::PathBuf {
    std::env::var_os("LOFT_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".loft")
}

/// Order-comparable version key; non-numeric parts sort as 0.
#[cfg(feature = "registry")]
fn numeric_version(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// PKG.STUB — `loft api`: agent-facing library discovery.
///
/// - `loft api` lists every library discoverable from the cwd — project
///   dependencies (`loft.toml`), installed registry packages, user libraries —
///   each with the directory its source lives in.
/// - `loft api <name>` (or a package path) prints that library's public API
///   surface: `pub` signatures + doc comments, bodies stripped.
#[cfg(feature = "registry")]
fn api_command(target: Option<&str>) {
    if let Some(name) = target {
        let Some(dir) = api_resolve_pkg_dir(name) else {
            eprintln!(
                "loft api: no library `{name}` found (project lib/, ~/.loft/lib, installed registry packages)."
            );
            eprintln!(
                "  `loft api` lists what is discoverable; `loft search {name}` queries the registry."
            );
            std::process::exit(1);
        };
        match loft::documentation::render_pkg_api_text(&dir) {
            Ok(text) => print!("{text}"),
            Err(e) => {
                eprintln!("loft api: cannot read {}: {e}", dir.display());
                std::process::exit(1);
            }
        }
        return;
    }

    // No name: one listing of everything reachable from here.
    let manifest_path = std::path::Path::new("loft.toml");
    if manifest_path.exists() {
        let manifest = loft::manifest::read_manifest("loft.toml").unwrap_or_default();
        println!("== project dependencies (loft.toml) ==");
        if manifest.dependencies.is_empty() {
            println!("  (none)");
        }
        for (name, _constraint) in &manifest.dependencies {
            match api_resolve_pkg_dir(name) {
                Some(dir) => println!("  {name}  {}", dir.display()),
                None => println!("  {name}  NOT INSTALLED — run `loft install`"),
            }
        }
    }
    let installed = loft::registry_index::installed_packages();
    println!("== installed registry packages ==");
    if installed.is_empty() {
        println!("  (none)");
    }
    for (name, version, path) in installed {
        println!("  {name} {version}  {}", path.display());
    }
    let user_lib = loft_home().join("lib");
    if let Ok(read) = std::fs::read_dir(&user_lib) {
        println!("== user libraries ({}) ==", user_lib.display());
        for ent in read.filter_map(Result::ok) {
            if ent.path().join("loft.toml").exists() {
                println!(
                    "  {}  {}",
                    ent.file_name().to_string_lossy(),
                    ent.path().display()
                );
            }
        }
    }
    println!();
    println!("`loft api <name>` prints a library's public API (signatures + doc comments).");
}

/// `loft api --registry` — list the whole installable catalog from the registry
/// index (so an agent can discover what EXISTS, not just what's installed).
/// Mirrors `loft search`'s trust posture (`allow_unsigned: true`): a *missing*
/// signature is tolerated, but an *invalid* one still hard-fails.
#[cfg(feature = "registry")]
fn api_registry_catalog(refresh: bool) {
    // The catalog is cached with a 1-hour TTL; `--refresh` forces a re-fetch so an
    // agent sees registry changes (new packages, fixed descriptions) immediately
    // rather than waiting out the TTL.
    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh,
        offline: false,
        allow_prerelease: false,
        lock_path: None,
    };
    match loft::install::load_index(&opts) {
        Ok(index) => print!("{}", loft::registry_index::render_catalog(&index)),
        Err(e) => {
            eprintln!("loft api --registry: {e}");
            std::process::exit(1);
        }
    }
}

/// PKG.STUB — write `.loft/api/<name>.api` stubs (the public surface of every
/// locked dependency) under `project_dir`.  Called by the lockfile-writing
/// commands, so stub freshness rides the same trigger as `loft.lock` itself:
/// agents exploring the project tree see the dependency APIs without leaving
/// it.  Best-effort: a missing install or unreadable package skips silently —
/// the stub layer must never break an install.
#[cfg(feature = "registry")]
fn write_api_stubs(lock_path: &std::path::Path, project_dir: &std::path::Path) {
    let Ok(Some(lock)) = loft::lockfile::read_lockfile(lock_path) else {
        return;
    };
    if lock.packages.is_empty() {
        return;
    }
    let api_dir = project_dir.join(".loft").join("api");
    if std::fs::create_dir_all(&api_dir).is_err() {
        return;
    }
    let mut written = 0u32;
    for p in &lock.packages {
        let dir = loft::registry_index::extract_dir(&p.name, &p.version);
        if !dir.join("loft.toml").exists() {
            continue;
        }
        if let Ok(text) = loft::documentation::render_pkg_api_text(&dir) {
            if std::fs::write(api_dir.join(format!("{}.api", p.name)), text).is_ok() {
                written += 1;
            }
        }
    }
    if written > 0 {
        println!(
            "wrote {written} API stub(s) to {} (agent-readable; commit them)",
            api_dir.display()
        );
    }

    // Alongside the per-dep stubs, write the registry CATALOG (_available.api) so
    // an agent reading `.loft/api/` sees not just what the project depends on but
    // everything else it could `loft install`.  Best-effort + cache-first: if the
    // index can't load (offline / invalid signature) just skip — the install
    // already succeeded, and discovery is a convenience, never a blocker.
    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        lock_path: None,
    };
    if let Ok(index) = loft::install::load_index(&opts) {
        let _ = std::fs::write(
            api_dir.join("_available.api"),
            loft::registry_index::render_catalog(&index),
        );
    }
}

/// @PLAN12 Phase 6.6 — `loft list-installed` enumerates packages
/// in `~/.loft/registry/` and annotates each with its sha256 +
/// size + index status (active / yanked / orphan-from-index).
///
/// Pure query — touches no network, writes nothing.  Useful for
/// "what's in my cache?" investigations + as a starting point for
/// `loft audit` once Phase 6.7's advisory channel ships.
#[cfg(feature = "registry")]
fn list_installed() {
    use loft::install::InstallOptions;
    use loft::registry_index;
    use std::path::PathBuf;

    let cache = registry_index::cache_dir();
    if !cache.exists() {
        println!("No registry cache at {}.", cache.display());
        return;
    }

    let entries: Vec<(String, String, PathBuf)> = registry_index::installed_packages();

    if entries.is_empty() {
        println!(
            "No registry packages installed (cache: {}).",
            cache.display()
        );
        return;
    }

    // Try to load the cached index so we can show sha256 + size +
    // status.  If the index isn't cached (cold loft binary), skip
    // the annotations but still list the dirs.
    let opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: true, // never hit the network for this query
        allow_prerelease: false,
        lock_path: None,
    };
    let index = loft::install::load_index(&opts).ok();

    println!("Installed packages (in {}):", cache.display());
    for (name, version, path) in &entries {
        let on_disk_bytes = dir_size_bytes(path).unwrap_or(0);
        let mut sha = String::new();
        let mut status_tag = String::new();
        if let Some(idx) = &index {
            if let Some(pkg) = idx.packages.get(name) {
                if let Some(v) = pkg.versions.get(version) {
                    sha.clone_from(&v.sha256);
                    if pkg.yanked.iter().any(|y| y == version) {
                        status_tag.push_str(" [YANKED]");
                    }
                } else {
                    status_tag.push_str(" [version not in current index]");
                }
            } else {
                status_tag.push_str(" [orphan: package not in current index]");
            }
        }
        let sha_short: String = sha.chars().take(12).collect();
        let sha_disp = if sha_short.is_empty() {
            "(unknown sha)".to_string()
        } else {
            format!("sha256:{sha_short}…")
        };
        println!(
            "  {name} {version}{status_tag} — {} bytes on disk, {sha_disp}",
            on_disk_bytes
        );
    }
    println!("{} package(s) total", entries.len());
}

/// @PLAN12 Phase 6.8 — knob set for the `loft update` command.
#[cfg(feature = "registry")]
#[derive(Debug, Default, Clone)]
struct UpdateOpts {
    /// Specific package name; `None` updates every entry in the
    /// lockfile.
    target: Option<String>,
    /// Compute + print the diff without writing the lockfile.
    dry_run: bool,
    /// Exit non-zero if any updates are available (CI gate).
    /// Implies dry_run.
    check_only: bool,
}

/// @PLAN12 Phase 6.8 — `loft update` driver.
///
/// Walks the project's (or cwd's) `loft.lock`, looks up each
/// package in the registry index, picks the highest active
/// non-yanked version that satisfies the corresponding range
/// from `loft.toml` (if present; otherwise the lockfile-pinned
/// version is treated as exact and never updated), and — unless
/// in dry-run/check mode — calls `install_one` to fetch + extract
/// + merge into the lockfile.
///
/// Exit codes:
/// - 0  → up-to-date (no updates needed or all updates applied
///   successfully).
/// - 1  → updates available (`--check`) OR install failure.
/// - 2  → no lockfile to update.
#[cfg(feature = "registry")]
fn update_packages(opts: &UpdateOpts) -> i32 {
    use loft::install::{InstallOptions, install_one};
    use loft::lockfile;
    use loft::manifest;
    use loft::registry_index;

    // Find the project root or fall back to cwd.  Mirror the
    // parser's walk-up logic (Phase 6.6) — start at cwd, walk up
    // looking for loft.toml.
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("loft update: cwd: {e}");
            return 1;
        }
    };
    let project_root = find_project_root_from(&cwd);
    let lock_dir = project_root.as_ref().unwrap_or(&cwd);
    let lock_path = lock_dir.join("loft.lock");
    if !lock_path.exists() {
        eprintln!(
            "loft update: no loft.lock at {} — nothing to update.",
            lock_path.display()
        );
        eprintln!(
            "  Run `loft install <pkg>` first (or `loft pin <script>` for one-file scripts)."
        );
        return 2;
    }
    let lock = match lockfile::read_lockfile(&lock_path) {
        Ok(Some(l)) => l,
        Ok(None) => {
            eprintln!("loft update: lockfile empty");
            return 2;
        }
        Err(e) => {
            eprintln!("loft update: cannot read {}: {e}", lock_path.display());
            return 1;
        }
    };
    if lock.packages.is_empty() {
        eprintln!("loft update: lockfile has no packages.");
        return 0;
    }

    // Read project loft.toml deps so we know the version range for
    // each entry.  Transitive deps (in lockfile but not in toml)
    // default to "*" (any non-yanked).
    let toml_deps: std::collections::HashMap<String, String> = project_root
        .as_ref()
        .and_then(|root| manifest::read_manifest(root.join("loft.toml").to_str().unwrap_or("")))
        .map(|m| {
            m.dependencies
                .into_iter()
                .filter(|(_, v)| manifest::extract_path_dep(v).is_none())
                .collect()
        })
        .unwrap_or_default();

    // Load index (offline-respecting, allow_unsigned for the
    // bootstrap window).
    let install_opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: std::env::var("LOFT_OFFLINE").is_ok(),
        allow_prerelease: false,
        lock_path: Some(lock_path.clone()),
    };
    let index = match loft::install::load_index(&install_opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft update: {e}");
            return 1;
        }
    };

    let dry = opts.dry_run || opts.check_only;
    let mut updates_available = false;
    let mut install_failures: Vec<String> = Vec::new();
    let mut diff: Vec<String> = Vec::new();

    for entry in &lock.packages {
        if let Some(t) = &opts.target {
            if t != &entry.name {
                continue;
            }
        }
        let pkg = match index.packages.get(&entry.name) {
            Some(p) => p,
            None => {
                diff.push(format!(
                    "  {pkg} {ver} — not in current index (orphan; skipped)",
                    pkg = entry.name,
                    ver = entry.version
                ));
                continue;
            }
        };
        let constraint = toml_deps
            .get(&entry.name)
            .cloned()
            .unwrap_or_else(|| "*".to_string());
        let Some(best) = registry_index::find_best_version(pkg, &constraint, false) else {
            diff.push(format!(
                "  {pkg} {ver} — no version satisfies range `{constraint}` (skipped)",
                pkg = entry.name,
                ver = entry.version
            ));
            continue;
        };
        if best.semver == entry.version {
            // Already on the highest satisfying version.
            continue;
        }
        // Higher OR lower (e.g. rollback after yank) — both are
        // "updates" in the sense of "lockfile would change."
        updates_available = true;
        diff.push(format!(
            "  {pkg} {old} → {new}",
            pkg = entry.name,
            old = entry.version,
            new = best.semver
        ));
        if !dry {
            match install_one(&entry.name, Some(&best.semver), &install_opts) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("  FAILED {} {}: {e}", entry.name, best.semver);
                    install_failures.push(entry.name.clone());
                }
            }
        }
    }

    if diff.is_empty() {
        if let Some(t) = &opts.target {
            println!("loft update {t}: already on the highest satisfying version.");
        } else {
            println!(
                "loft update: all {} packages up-to-date.",
                lock.packages.len()
            );
        }
        return 0;
    }

    if opts.check_only {
        println!("loft update --check: updates available:");
        for line in &diff {
            println!("{line}");
        }
        return 1;
    }
    if opts.dry_run {
        println!("loft update --dry-run: would update:");
        for line in &diff {
            println!("{line}");
        }
        return 0;
    }

    println!("loft update:");
    for line in &diff {
        println!("{line}");
    }
    if !install_failures.is_empty() {
        eprintln!(
            "loft update: {} package(s) failed to install",
            install_failures.len()
        );
        return 1;
    }
    let _ = updates_available;
    0
}

/// One path representation everywhere (@P296 / #460): on Windows,
/// `fs::canonicalize` returns an extended-length `\\?\D:\…` verbatim path,
/// while the rest of the pipeline (library `use` resolution, the #460
/// entry-package skip, `def.position().file` prefix checks) builds and
/// compares plain paths.  A verbatim path never equals or prefix-matches its
/// plain twin (`VerbatimDisk` vs `Disk` components), so every canonicalized
/// path entering the shared path space must shed the prefix.  No-op on
/// Linux/macOS; only the `\\?\D:\…` disk form is stripped, not verbatim-UNC
/// (`\\?\UNC\…`), which has no plain equivalent.
fn strip_verbatim_disk(path: String) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        rest.to_string()
    } else {
        path
    }
}

/// Walk up from `start` looking for the nearest directory that
/// contains a `loft.toml`.  Returns `None` when reaching the
/// filesystem root with no match.  Mirrors
/// `parser::find_project_root` but reusable from main.rs
/// (which can't reach into the parser's static helper).
#[cfg(feature = "registry")]
fn find_project_root_from(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let abs = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let mut cur = abs.as_path();
    loop {
        if cur.join("loft.toml").exists() {
            return Some(cur.to_path_buf());
        }
        let parent = cur.parent()?;
        if parent == cur {
            return None;
        }
        cur = parent;
    }
}

/// @PLAN12 Phase 6.11 — `loft bundle export <outdir>` writes a
/// self-contained directory of registry artifacts (index +
/// advisories + tarballs) that can be carried via USB / scp and
/// imported on an air-gapped machine via `loft bundle import`.
///
/// Layout:
/// ```
/// <outdir>/
/// ├── index.json + index.json.sig
/// ├── advisories.json + advisories.json.sig  (when present)
/// ├── packages/
/// │   ├── <pkg>-<ver>.tar.gz
/// │   └── ...
/// └── manifest.json   (bundle metadata: timestamp, source URL,
///                      loft binary version, package list)
/// ```
///
/// Selection:
/// - `--all` (or omit both flags) → every package in index.
/// - `--packages X,Y,Z` → just those (transitive deps not yet
///   auto-resolved; transitive harvest is a follow-up if useful).
#[cfg(feature = "registry")]
fn bundle_export(outdir: &str, packages: Option<&[String]>, all: bool) -> i32 {
    use loft::install::InstallOptions;
    use loft::registry_index;
    use std::path::Path;

    let out = Path::new(outdir);
    if let Err(e) = std::fs::create_dir_all(out.join("packages")) {
        eprintln!("loft bundle export: cannot create {}: {e}", out.display());
        return 1;
    }

    let opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        lock_path: None,
    };
    let index = match loft::install::load_index(&opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft bundle export: {e}");
            return 1;
        }
    };

    // Pick which packages to export.
    let want: Vec<String> = if all || packages.is_none() {
        index.packages.keys().cloned().collect()
    } else {
        packages.unwrap_or(&[]).to_vec()
    };
    if want.is_empty() {
        eprintln!("loft bundle export: nothing to export (empty package list)");
        return 1;
    }

    // Copy index.json + sig.
    let (idx_path, sig_path, _) = registry_index::index_paths();
    if idx_path.exists() {
        if let Err(e) = std::fs::copy(&idx_path, out.join("index.json")) {
            eprintln!("loft bundle export: copy index.json: {e}");
            return 1;
        }
    }
    if sig_path.exists() {
        let _ = std::fs::copy(&sig_path, out.join("index.json.sig"));
    }
    // Advisories (optional — registry may not host one yet).
    let (adv_path, adv_sig_path) = loft::registry_advisories::advisories_paths();
    if adv_path.exists() {
        let _ = std::fs::copy(&adv_path, out.join("advisories.json"));
        if adv_sig_path.exists() {
            let _ = std::fs::copy(&adv_sig_path, out.join("advisories.json.sig"));
        }
    }

    // For each requested package: pick its latest active version,
    // ensure the tarball is downloaded, copy into bundle/packages/.
    let mut exported: Vec<(String, String)> = Vec::new();
    for name in &want {
        let pkg = match index.packages.get(name) {
            Some(p) => p,
            None => {
                eprintln!("  warning: {name} not in current index — skipped");
                continue;
            }
        };
        let Some(ver) = registry_index::find_best_version(pkg, "*", false) else {
            eprintln!("  warning: {name} has no installable version — skipped");
            continue;
        };
        let tarball = format!("{name}-{}.tar.gz", ver.semver);
        let dest = out.join("packages").join(&tarball);
        // Download to the bundle's packages/.
        eprintln!("[bundle] fetching {} {}", name, ver.semver);
        let bytes = match registry_index::download_tarball(&ver.url, &dest) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  FAILED: {e}");
                return 1;
            }
        };
        if let Err(e) = registry_index::verify_sha256(&bytes, &ver.sha256) {
            eprintln!("  FAILED: {e}");
            return 1;
        }
        exported.push((name.clone(), ver.semver.clone()));
    }

    // Write manifest.json — bundle metadata for the import side.
    let manifest_pkgs: String = exported
        .iter()
        .map(|(n, v)| format!("    {{\"name\":\"{n}\",\"version\":\"{v}\"}}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let registry_url = registry_index::registry_url();
    let loft_version = env!("CARGO_PKG_VERSION");
    let manifest = format!(
        "{{\n  \"schema_version\": 1,\n  \"created\": \"{}\",\n  \"registry_url\": \"{}\",\n  \"loft_version\": \"{}\",\n  \"packages\": [\n{}\n  ]\n}}\n",
        chrono_iso8601_utc(),
        registry_url,
        loft_version,
        manifest_pkgs
    );
    if let Err(e) = std::fs::write(out.join("manifest.json"), manifest) {
        eprintln!("loft bundle export: write manifest: {e}");
        return 1;
    }

    println!(
        "[bundle] exported {} package(s) to {}",
        exported.len(),
        out.display()
    );
    0
}

/// @PLAN12 Phase 6.11 — `loft bundle import <indir>` installs a
/// previously-exported bundle into `~/.loft/registry/`.
///
/// Steps per artifact:
/// 1. Copy `index.json` + `.sig` (sig kept; loader verifies via
///    `allow_unsigned` for the bootstrap window).
/// 2. Copy `advisories.json` + `.sig` (when present).
/// 3. For each `packages/<pkg>-<ver>.tar.gz`:
///    - Verify sha256 matches the index entry.
///    - Extract to `~/.loft/registry/<pkg>-<ver>/`.
/// 4. Print summary.
#[cfg(feature = "registry")]
fn bundle_import(indir: &str) -> i32 {
    use loft::registry_index;
    use std::path::Path;

    let inp = Path::new(indir);
    let bundle_index = inp.join("index.json");
    if !bundle_index.exists() {
        eprintln!("loft bundle import: {} has no index.json", inp.display());
        return 1;
    }

    // Read + parse the bundle's index so we know the per-tarball sha256.
    let idx_bytes = match std::fs::read(&bundle_index) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("loft bundle import: read index: {e}");
            return 1;
        }
    };
    let idx_text = match std::str::from_utf8(&idx_bytes) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("loft bundle import: index not UTF-8: {e}");
            return 1;
        }
    };
    let index = match registry_index::parse_index(idx_text) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft bundle import: parse index: {e}");
            return 1;
        }
    };

    // Copy index + sig + (optional) advisories into the cache.
    let cache = registry_index::cache_dir();
    if let Err(e) = std::fs::create_dir_all(&cache) {
        eprintln!("loft bundle import: cannot create {}: {e}", cache.display());
        return 1;
    }
    let _ = std::fs::copy(&bundle_index, cache.join("index.json"));
    let bundle_sig = inp.join("index.json.sig");
    if bundle_sig.exists() {
        let _ = std::fs::copy(&bundle_sig, cache.join("index.json.sig"));
    }
    let bundle_adv = inp.join("advisories.json");
    if bundle_adv.exists() {
        let _ = std::fs::copy(&bundle_adv, cache.join("advisories.json"));
        let bundle_adv_sig = inp.join("advisories.json.sig");
        if bundle_adv_sig.exists() {
            let _ = std::fs::copy(&bundle_adv_sig, cache.join("advisories.json.sig"));
        }
    }

    // Extract each tarball; verify sha256 against the index entry.
    let pkg_dir = inp.join("packages");
    let read = match std::fs::read_dir(&pkg_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("loft bundle import: read {}: {e}", pkg_dir.display());
            return 1;
        }
    };
    let mut imported: Vec<(String, String)> = Vec::new();
    for ent in read.filter_map(Result::ok) {
        let path = ent.path();
        let fname = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if !fname.ends_with(".tar.gz") {
            continue;
        }
        let stem = &fname[..fname.len() - ".tar.gz".len()];
        // Split last `-<digit>` boundary for (name, version).
        let bytes = stem.as_bytes();
        let mut at: Option<usize> = None;
        let mut idx = 1;
        while idx < bytes.len() {
            if bytes[idx - 1] == b'-' && bytes[idx].is_ascii_digit() {
                at = Some(idx - 1);
                break;
            }
            idx += 1;
        }
        let Some(split) = at else { continue };
        let (name, rest) = stem.split_at(split);
        let version = rest.trim_start_matches('-');

        // Look up sha256 in the imported index.
        let Some(pkg) = index.packages.get(name) else {
            eprintln!("  skip {fname}: not in bundle's index");
            continue;
        };
        let Some(ver) = pkg.versions.get(version) else {
            eprintln!("  skip {fname}: version not in bundle's index");
            continue;
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  read {fname}: {e}");
                return 1;
            }
        };
        if let Err(e) = registry_index::verify_sha256(&bytes, &ver.sha256) {
            eprintln!("  sha256 MISMATCH for {fname}: {e}");
            return 1;
        }
        if let Err(e) = registry_index::extract_tarball(&path, &cache) {
            eprintln!("  extract {fname}: {e}");
            return 1;
        }
        imported.push((name.to_string(), version.to_string()));
    }

    if imported.is_empty() {
        eprintln!(
            "loft bundle import: no packages found in {}",
            pkg_dir.display()
        );
        return 1;
    }
    println!(
        "[bundle] imported {} package(s) into {}",
        imported.len(),
        cache.display()
    );
    for (n, v) in &imported {
        println!("  {n} {v}");
    }
    0
}

/// Minimal ISO-8601 UTC timestamp.  Avoid pulling in a date crate;
/// loft already does its own time formatting elsewhere via
/// `std::time::SystemTime`.  Format: `YYYY-MM-DDTHH:MM:SSZ`.
#[cfg(feature = "registry")]
fn chrono_iso8601_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since 1970-01-01 (Unix epoch).
    let day = secs / 86_400;
    let sod = secs % 86_400;
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let second = sod % 60;
    // Convert day count to Y/M/D via the standard algorithm.
    // Cribbed from https://howardhinnant.github.io/date_algorithms.html
    let z = day as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z",
        y = y,
        m = m,
        d = d,
        hour = hour,
        minute = minute,
        second = second
    )
}

/// @PLAN12 Phase 6.7a — `loft yank` author helper.
///
/// Closes the security loop on the author side.  Phase 6.7
/// ships the consumer-side classifier (loft binary checks
/// `~/.loft/registry/advisories.json` against installed
/// versions and refuses/warns per severity); 6.7a provides
/// the CLI an author runs to FILE an advisory.
///
/// Emits the two edits the registry PR needs:
///
/// 1. `index.json` — adds the typed `status` field to the
///    affected version entry.
/// 2. `advisories.json` — appends a row with cross-referenced
///    `id`, `packages[]`, `severity`, `summary`, `published`.
///
/// Auto-PR-open (clone registry + splice in the edits +
/// `gh pr create`) is the next iteration; MVP emits the two
/// blocks ready for paste into a manual PR.
#[cfg(feature = "registry")]
fn yank_package(
    target: &str,
    severity: Option<&str>,
    advisory: Option<&str>,
    summary: Option<&str>,
    affected: Option<&str>,
    fixed_in: Option<&str>,
) -> i32 {
    let Some((name, version)) = target.split_once('@') else {
        eprintln!("loft yank: target must be `<pkg>@<version>` (got `{target}`)");
        return 1;
    };
    if name.is_empty() || version.is_empty() {
        eprintln!("loft yank: target must be `<pkg>@<version>` (got `{target}`)");
        return 1;
    }
    let severity = match severity {
        Some(s) => match s {
            "security_critical" | "security_high" | "security_low" | "bug" | "deprecated" => s,
            other => {
                eprintln!(
                    "loft yank: --severity must be one of \
                     security_critical / security_high / security_low / bug / deprecated \
                     (got `{other}`)"
                );
                return 1;
            }
        },
        None => {
            eprintln!("loft yank: --severity required");
            return 1;
        }
    };
    let Some(advisory) = advisory else {
        eprintln!("loft yank: --advisory <id> required (e.g. GHSA-xxxx-yyyy-zzzz)");
        return 1;
    };
    let Some(summary) = summary else {
        eprintln!("loft yank: --summary <text> required");
        return 1;
    };
    let affected_range = affected.unwrap_or(version);
    let published = chrono_iso8601_utc();

    println!("# 1) Edit `index.json` — set the typed `status` on the affected version.");
    println!("# Replace the existing `\"{version}\": {{ ... }}` entry for `{name}` with:");
    println!();
    println!("\"{version}\": {{");
    println!("  // ...existing fields (url, sha256, size, loft, subpath, deps, published)...");
    println!("  \"status\": {{");
    println!("    \"kind\": \"yanked\",");
    println!("    \"severity\": \"{severity}\",");
    println!("    \"advisory\": \"{advisory}\",");
    println!("    \"summary\": {}", escape_json_string(summary));
    println!("  }}");
    println!("}}");
    println!();
    println!("# 2) Edit `advisories.json` — append the cross-referenced row.");
    println!("# Insert at the top of the `\"advisories\": [ ... ]` array:");
    println!();
    println!("{{");
    println!("  \"id\": \"{advisory}\",");
    println!(
        "  \"packages\": [{{\"name\": \"{name}\", \"affected\": \"{affected_range}\"{}}}],",
        if let Some(fix) = fixed_in {
            format!(", \"fixed_in\": \"{fix}\"")
        } else {
            String::new()
        }
    );
    println!("  \"severity\": \"{severity}\",");
    println!("  \"summary\": {},", escape_json_string(summary));
    println!("  \"published\": \"{published}\",");
    println!("  \"references\": []");
    println!("}}");
    println!();
    println!("# Then:");
    println!("#   git checkout -b yank-{name}-{version}");
    println!("#   $EDITOR index.json   # apply edit 1");
    println!("#   $EDITOR advisories.json   # apply edit 2");
    println!("#   git commit -am 'yank: {name} {version} ({advisory})'");
    println!("#   gh pr create --title 'yank: {name} {version}' --body \"<rationale>\"");
    0
}

/// Quote + JSON-escape a string.  Minimal — handles `"`, `\\`,
/// `\n`, `\t`.
#[cfg(feature = "registry")]
fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// @PLAN12 — `loft new <name>` scaffolds a fresh loft library
/// package, ready for development + `loft publish`.
///
/// Layout produced:
///
/// ```
/// <name>/
/// ├── loft.toml           — package manifest with [package] + [library]
/// ├── README.md           — placeholder header pointing at the registry
/// ├── release.sh          — one-command release: bump → test → tag → package → GH release
/// ├── src/<name>.loft     — empty entry file with a `pub fn hello()` stub
/// └── tests/
///     └── 01-smoke.loft   — single `test_smoke` exercising the stub
/// ```
///
/// With `--native`: also creates `native/Cargo.toml` + `native/src/lib.rs`
/// + `native/build.rs` for the cdylib bindings.
///
/// With `--chunk`: also creates `.github/workflows/library-ci.yml`
/// using the canonical template from
/// `doc/claude/lib_plans/12-library-extraction/library-ci.yml.example`
/// — for when the dir is becoming a fresh chunk-repo's first
/// library.
///
/// Refuses if `<name>/` already exists.
fn scaffold_library(name: &str, native: bool, chunk: bool) -> i32 {
    use std::io::Write as _;

    // Sanity-check name (lowercase + alphanumeric + underscore;
    // matches loft's identifier rules).
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        eprintln!(
            "loft new: library name must be lowercase ascii + digits + underscore (got `{name}`)"
        );
        return 1;
    }

    let pkg_dir = std::path::PathBuf::from(name);
    if pkg_dir.exists() {
        eprintln!("loft new: `{name}/` already exists; refusing to overwrite");
        return 1;
    }

    // Helper closures.
    let write_file = |rel: &str, content: &str| -> std::io::Result<()> {
        let path = pkg_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(&path)?;
        f.write_all(content.as_bytes())?;
        Ok(())
    };

    // loft.toml — includes [native] declaration when --native.
    let loft_toml = if native {
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
             description = \"One-line description of {name}.\"\n\n\
             [library]\nentry = \"src/{name}.loft\"\nnative = \"loft_{name}\"\n\n\
             [native]\ncrate = \"loft-{name}\"\n\n[dependencies]\n"
        )
    } else {
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
             description = \"One-line description of {name}.\"\n\n\
             [library]\nentry = \"src/{name}.loft\"\n\n[dependencies]\n"
        )
    };

    // src/<name>.loft — placeholder stub.
    let src_loft = format!(
        "// Copyright (c) 2026\n// SPDX-License-Identifier: LGPL-3.0-or-later\n\n\
         // {name} — replace with a one-line description of the library.\n\n\
         // Returns the greeting.  Replace with the library's actual API.\n\
         pub fn hello() -> text {{\n  \"hello from {name}\"\n}}\n"
    );

    // tests/01-smoke.loft — minimal regression guard.
    let test_loft = format!(
        "// Smoke test for {name}.\n\
         use {name};\n\n\
         fn test_hello() {{\n  \
           greeting = {name}::hello();\n  \
           assert(greeting == \"hello from {name}\", \"hello() returned {{greeting ?? \\\"null\\\"}}\");\n\
         }}\n"
    );

    let readme = format!(
        "# {name}\n\n\
         <One-line description of the library.>\n\n\
         ## Install\n\n\
         ```\n\
         loft install {name}\n\
         ```\n\n\
         ## Usage\n\n\
         ```loft\n\
         use {name};\n\n\
         fn main() {{\n  \
           println({name}::hello());\n\
         }}\n\
         ```\n"
    );

    // release.sh — one command to cut a release: bump → test → tag → package →
    // GitHub release, with immutability + deterministic-package gates.  A plain
    // &str (no interpolation): it reads name + version from loft.toml at runtime.
    let release_sh = r#"#!/usr/bin/env bash
# release.sh — cut a release of THIS loft library so the registry can publish it.
# Run from the library directory (the one containing loft.toml).
#
#   ./release.sh            # release the version currently in loft.toml
#   ./release.sh 0.2.0      # set loft.toml to 0.2.0, commit, then release
#
# Reads name + version from loft.toml, runs the test gate + a determinism check,
# commits any version bump, tags <name>-v<version>, pushes the branch + tag,
# packages the tarball, and creates the GitHub release.  Releases are immutable:
# it refuses to re-cut an existing tag — bump the version instead.
#
# After a successful release:
#   * loft-lang family lib -> run scripts/registry_maintain.sh in loft-lang/loft
#     (publishes every stale/missing own lib + signs the registry index).
#   * external lib         -> `loft publish` then open a registry PR
#     (see LIBRARY_AUTHORING.md / REGISTRY_SUBMIT.md).
#
# Env: LOFT=/path/to/loft to use a non-PATH binary; SKIP_NATIVE=1 to skip the
#      `loft --native test` gate (e.g. no Rust toolchain on this machine).
set -euo pipefail
cd "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"

LOFT="${LOFT:-loft}"
sha()   { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
field() { sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*\"\(.*\)\".*/\1/p" loft.toml | head -1; }

[ -f loft.toml ] || { echo "release.sh: no loft.toml here — run from the library dir." >&2; exit 1; }
command -v gh >/dev/null 2>&1 || { echo "release.sh: needs the GitHub CLI (gh)." >&2; exit 1; }

# Push without a username/password prompt by reusing your gh login as git's
# credential helper (HTTPS remotes); harmless on SSH remotes (which use your
# key).  `gh release create` works via gh's API auth, but plain `git push` uses
# git's credential system — without a helper it prompts, and GitHub no longer
# accepts a password there.  GIT_TERMINAL_PROMPT=0 fails fast instead of hanging.
export GIT_TERMINAL_PROMPT=0
gitpush() {
    git -c credential.helper='!gh auth git-credential' push "$@" || {
        echo "release.sh: git push failed — set up auth once with 'gh auth setup-git'," >&2
        echo "             or use an SSH remote (git@github.com:OWNER/REPO.git)." >&2
        exit 1
    }
}
name=$(field name); [ -n "$name" ] || { echo "release.sh: no package name in loft.toml." >&2; exit 1; }

# Optional version bump.
if [ "${1:-}" != "" ]; then
    case "$1" in
        [0-9]*.[0-9]*.[0-9]*) : ;;
        *) echo "release.sh: '$1' is not an x.y.z version." >&2; exit 1 ;;
    esac
    tmp=$(mktemp)
    awk -v v="$1" '!d && /^[[:space:]]*version[[:space:]]*=/ {sub(/"[^"]*"/, "\"" v "\""); d=1} {print}' loft.toml >"$tmp"
    mv "$tmp" loft.toml
fi
ver=$(field version); [ -n "$ver" ] || { echo "release.sh: no version in loft.toml." >&2; exit 1; }
tag="$name-v$ver"
echo "== releasing $name $ver ($tag) =="

# Immutability — never re-cut an existing release; bump the version instead.
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null 2>&1 || gh release view "$tag" >/dev/null 2>&1; then
    echo "release.sh: $tag already exists. Bump first: ./release.sh <new x.y.z>" >&2
    exit 1
fi

# Gate 1 — tests pass (both backends unless SKIP_NATIVE=1).
echo "-- loft test"; "$LOFT" test
if [ "${SKIP_NATIVE:-}" != 1 ]; then echo "-- loft --native test (SKIP_NATIVE=1 to skip)"; "$LOFT" --native test; fi

# Commit the bump FIRST so the tag points at exactly the bytes we package.
if ! git diff --quiet -- loft.toml; then git add loft.toml && git commit -m "release: $name $ver"; fi
git diff --quiet && git diff --cached --quiet || {
    echo "release.sh: uncommitted changes — commit them so the tag matches the release." >&2; exit 1; }

# Gate 2 — packaging is deterministic (two clean builds must hash equal); this is
# the registry's gate-3 reproducible-build invariant, checked locally first.
"$LOFT" package >/dev/null; a=$(sha "$name-$ver.tar.gz"); rm -f "$name-$ver.tar.gz"
"$LOFT" package >/dev/null; b=$(sha "$name-$ver.tar.gz")
[ "$a" = "$b" ] || { echo "release.sh: non-deterministic package ($a != $b)." >&2; rm -f "$name-$ver.tar.gz"; exit 1; }

git tag "$tag"
gitpush origin HEAD
gitpush origin "$tag"
gh release create "$tag" "$name-$ver.tar.gz" --title "$name v$ver" --notes "Release $name $ver."
rm -f "$name-$ver.tar.gz"

echo
echo "released $tag."
echo "  loft-lang family lib -> run scripts/registry_maintain.sh in loft-lang/loft."
echo "  external lib         -> loft publish + open a registry PR."
"#;

    if let Err(e) = (|| -> std::io::Result<()> {
        write_file("loft.toml", &loft_toml)?;
        write_file(&format!("src/{name}.loft"), &src_loft)?;
        write_file("tests/01-smoke.loft", &test_loft)?;
        write_file("README.md", &readme)?;
        write_file("release.sh", release_sh)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let p = pkg_dir.join("release.sh");
            let mut perm = std::fs::metadata(&p)?.permissions();
            perm.set_mode(0o755);
            std::fs::set_permissions(&p, perm)?;
        }
        if native {
            let cargo_toml = format!(
                "[package]\nname = \"loft-{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\nlicense = \"LGPL-3.0-or-later\"\n\n\
                 [lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n\n\
                 [dependencies]\nloft-ffi = \"0.1\"\n\n\
                 [build-dependencies]\nloft-ffi-build = \"0.2\"\n"
            );
            let build_rs =
                "fn main() {\n    loft_ffi_build::generate_register_from_loft(\"../src\");\n}\n";
            let lib_rs = "// Native bindings for the loft library.\n\
                 // Add `#[unsafe(no_mangle)] pub extern \"C\" fn n_<name>(...)` here for each\n\
                 // function whose loft signature is annotated `#native`.\n\n\
                 include!(concat!(env!(\"OUT_DIR\"), \"/loft_register_gen.rs\"));\n";
            write_file("native/Cargo.toml", &cargo_toml)?;
            write_file("native/build.rs", build_rs)?;
            write_file("native/src/lib.rs", lib_rs)?;
        }
        if chunk {
            // Canonical CI YAML — single-package matrix; user edits when adding more.
            let yml = format!(
                "name: library-ci\n\n\
                 on:\n  push:\n    branches: [main]\n  pull_request:\n\n\
                 jobs:\n  test:\n    runs-on: ubuntu-latest\n    \
                 strategy:\n      fail-fast: false\n      matrix:\n        package: [{name}]\n    \
                 steps:\n      - uses: actions/checkout@v4\n\n      \
                 - name: Install mold (loft's pinned linker)\n        \
                 run: sudo apt-get update -y && sudo apt-get install -y mold\n\n      \
                 - name: Clone loft source\n        uses: actions/checkout@v4\n        \
                 with:\n          repository: loft-lang/loft\n          path: loft-src\n\n      \
                 - name: Cache cargo registry + loft build\n        uses: actions/cache@v4\n        \
                 with:\n          path: |\n            ~/.cargo/registry\n            ~/.cargo/git\n            loft-src/target\n          \
                 key: loft-${{{{ hashFiles('loft-src/Cargo.lock') }}}}\n\n      \
                 - name: Build loft\n        working-directory: loft-src\n        \
                 run: |\n          cargo build --release --lib --bin loft\n          \
                 echo \"$PWD/target/release\" >> $GITHUB_PATH\n\n      \
                 - name: Interpreter — loft test\n        working-directory: ${{{{ matrix.package }}}}\n        \
                 env:\n          LOFT_DENY_WARNINGS: ${{{{ hashFiles(format('{{0}}/.allow_warnings', matrix.package)) != '' && '0' || '1' }}}}\n        \
                 run: loft --interpret --tests tests\n\n      \
                 - name: Native — loft --native test\n        working-directory: ${{{{ matrix.package }}}}\n        \
                 env:\n          LOFT_DENY_WARNINGS: ${{{{ hashFiles(format('{{0}}/.allow_warnings', matrix.package)) != '' && '0' || '1' }}}}\n        \
                 run: loft --native --tests tests\n"
            );
            write_file(".github/workflows/library-ci.yml", &yml)?;
        }
        Ok(())
    })() {
        eprintln!("loft new: failed to write scaffolding: {e}");
        // Best-effort cleanup
        let _ = std::fs::remove_dir_all(&pkg_dir);
        return 1;
    }

    println!("Created library `{name}/`:");
    println!("  loft.toml");
    println!("  src/{name}.loft");
    println!("  tests/01-smoke.loft");
    println!("  README.md");
    println!("  release.sh");
    if native {
        println!("  native/Cargo.toml");
        println!("  native/build.rs");
        println!("  native/src/lib.rs");
    }
    if chunk {
        println!("  .github/workflows/library-ci.yml");
    }
    println!();
    println!("Next steps:");
    println!("  cd {name}");
    println!("  loft test           # exercises the smoke test");
    println!("  $EDITOR src/{name}.loft   # add your library's API");
    println!("  ./release.sh        # when ready: test -> tag -> package -> GH release");
    0
}

/// @PLAN12 Phase 6.16 — `loft publish` author helper.
///
/// Closes the publish-by-hand friction.  After the author has
/// tagged + released their library on GitHub (`<pkg>-v<ver>` tag
/// and `gh release create` with the `loft package` tarball as an
/// asset), `loft publish` from the package dir:
///
/// 1. Re-packages locally via `package::package_create` — gets
///    the deterministic sha256 + size + name + version.
/// 2. Detects the chunk repo from `git remote get-url origin`.
/// 3. Verifies the GitHub release exists at `<pkg>-v<ver>` and
///    carries the expected tarball asset (via `gh release view`).
/// 4. Emits the `index.json` entry block, ready to paste into
///    the registry PR.  Includes the auto-generated `url`,
///    `sha256`, `size`, `subpath`, `deps` from the loft.toml.
///
/// `--dry-run` skips the GitHub-release verification (the rest
/// of the flow runs).
///
/// Auto-PR open via `gh pr create` against `loft-lang/registry`
/// is a follow-up; today the author copies the emitted block
/// into a manual PR.
/// Author-side pre-check for `loft publish`: warn when any trigger the package
/// is about to claim is already owned by a DIFFERENT package in the locally
/// cached registry catalog.  The registry CI (`validate.py` gate 4) enforces the
/// same uniqueness as a hard error; this is the early, offline heads-up so the
/// author fixes the clash before opening the PR.  Best-effort: silent when no
/// catalog is cached (nothing to check against) or the index won't parse.
#[cfg(feature = "registry")]
fn warn_trigger_collisions(pkg_name: &str, triggers: &[String]) {
    if triggers.is_empty() {
        return;
    }
    let (idx_path, _, _) = loft::registry_index::index_paths();
    let Ok(content) = std::fs::read_to_string(&idx_path) else {
        return;
    };
    let Ok(index) = loft::registry_index::parse_index(&content) else {
        return;
    };
    let owners = loft::registry_index::trigger_owners(&index);
    for trig in triggers {
        if let Some(owner) = owners.get(trig)
            && owner != pkg_name
        {
            eprintln!(
                "warning: trigger `{trig}` is already claimed by `{owner}` in the registry; the \
                 submission PR will be REJECTED — a method-on-type trigger must be unique. Rename \
                 the method in `{pkg_name}` or drop its `[triggers]` opt-in."
            );
        }
    }
}

#[cfg(feature = "registry")]
fn publish_package(pkg_path: &std::path::Path, dry_run: bool) -> i32 {
    use loft::package;

    let pkg = match package::package_create(pkg_path, None) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("loft publish: package: {e}");
            return 1;
        }
    };
    let tag = format!("{}-v{}", pkg.name, pkg.version);
    let tarball_filename = format!("{}-{}.tar.gz", pkg.name, pkg.version);

    let (org, repo) = match git_remote_org_repo(pkg_path) {
        Some(v) => v,
        None => {
            eprintln!(
                "loft publish: cannot detect GitHub org/repo from `git remote get-url origin`"
            );
            eprintln!("  Run from inside a chunk-repo working tree with an `origin` remote.");
            return 1;
        }
    };
    let release_url =
        format!("https://github.com/{org}/{repo}/releases/download/{tag}/{tarball_filename}");
    let homepage = format!("https://github.com/{org}/{repo}/tree/main/{}", pkg.name);

    if !dry_run {
        if !github_release_has_asset(&org, &repo, &tag, &tarball_filename) {
            eprintln!(
                "loft publish: release `{tag}` not found, OR doesn't carry the asset `{tarball_filename}`."
            );
            eprintln!("  Tag + release first:");
            eprintln!("    git tag {tag}");
            eprintln!("    git push origin {tag}");
            eprintln!(
                "    gh release create {tag} {} --title \"{} v{}\"",
                pkg.tarball.display(),
                pkg.name,
                pkg.version
            );
            eprintln!("  Or pass --dry-run to skip this check.");
            return 1;
        }
    }

    // Emit the index.json entry.  Read deps from loft.toml.
    let manifest_path = pkg_path.join("loft.toml");
    let manifest =
        loft::manifest::read_manifest(manifest_path.to_str().unwrap_or("")).unwrap_or_default();
    let registry_deps: Vec<(String, String)> = manifest
        .dependencies
        .iter()
        .filter(|(_, v)| loft::manifest::extract_path_dep(v).is_none())
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect();
    let published = chrono_iso8601_utc();

    println!("# Paste this entry into `loft-lang/registry/index.json` under");
    println!(
        "# `\"packages\": {{ \"{}\": {{ \"versions\": {{ ... }} }} }}`:",
        pkg.name
    );
    println!();
    println!("\"{}\": {{", pkg.version);
    println!("  \"url\": \"{release_url}\",");
    println!("  \"sha256\": \"{}\",", pkg.sha256);
    println!("  \"size\": {},", pkg.size);
    println!(
        "  \"loft\": \"{}\",",
        manifest.loft_version.unwrap_or_else(|| ">=0.8".to_string())
    );
    println!("  \"subpath\": \"{}\",", pkg.name);
    if registry_deps.is_empty() {
        println!("  \"deps\": {{}},");
    } else {
        let mut deps_lines: Vec<String> = Vec::new();
        for (n, v) in &registry_deps {
            // Strip `{ path = "..." }` -> bare; we already filtered
            // those, so `v` is a plain version string.
            deps_lines.push(format!("    \"{n}\": \"{v}\""));
        }
        println!("  \"deps\": {{");
        for (idx, line) in deps_lines.iter().enumerate() {
            if idx + 1 == deps_lines.len() {
                println!("{line}");
            } else {
                println!("{line},");
            }
        }
        println!("  }},");
    }
    // Tier-1 trigger surface — derived from the package source at publish time
    // (lib_plans/59-lazy-stdlib), so a CONSUMER's resolver can map
    // `obj.method()` to this package without the source.  Emitted only when the
    // package opts in via `[triggers] enabled`; nothing is hand-listed.
    if manifest.trigger_enabled {
        let entry = manifest
            .entry
            .clone()
            .unwrap_or_else(|| format!("src/{}.loft", pkg.name));
        let src = std::fs::read_to_string(pkg_path.join(&entry)).unwrap_or_default();
        let triggers: Vec<String> = loft::triggers::derive_triggers(&src)
            .methods
            .iter()
            .map(|m| format!("{}:{}", m.name, m.receiver))
            .collect();
        warn_trigger_collisions(&pkg.name, &triggers);
        let quoted: Vec<String> = triggers.iter().map(|t| format!("\"{t}\"")).collect();
        println!("  \"triggers\": [{}],", quoted.join(", "));
    }
    // Function-level API surface (S6/S7) — derived from ALL of the package's
    // `src/*.loft` at publish time (the same `parse_pkg_api` extractor `loft api`
    // uses), so `loft search` can surface THIS version's functions without the
    // source.  Emitted automatically (nothing hand-written), the exact mirror of
    // `triggers`; the registry CI re-derives + verifies it from source (S7), so
    // the pasted field is a pure function of the code and cannot drift.
    let api_json = api_items_json(&loft::documentation::pkg_api_items(pkg_path));
    println!("  \"api\": {},", loft::json::to_json_string(&api_json));
    println!("  \"published\": \"{published}\"");
    println!("}}");
    println!();
    println!("# If this is the first version, also add the package block:");
    println!("# \"{}\": {{", pkg.name);
    println!("#   \"description\": \"<one-liner>\",");
    println!("#   \"homepage\": \"{homepage}\",");
    println!("#   \"categories\": [\"<category>\"],");
    println!("#   \"yanked\": [],");
    println!("#   \"versions\": {{ ... the version block above ... }}");
    println!("# }}");
    if dry_run {
        eprintln!("\n[publish] dry-run: GitHub release verification skipped.");
    } else {
        eprintln!("\n[publish] verified release {tag} exists with asset {tarball_filename}");
        eprintln!("[publish] next step: open registry PR with the entry above");
    }
    0
}

/// Parse `git remote get-url origin` output for the github
/// org + repo.  Handles `https://github.com/<org>/<repo>(.git)?`
/// and `git@github.com:<org>/<repo>(.git)?` shapes.  Returns
/// `None` when the remote isn't a github URL.
#[cfg(feature = "registry")]
fn git_remote_org_repo(pkg_path: &std::path::Path) -> Option<(String, String)> {
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(pkg_path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8(out.stdout).ok()?.trim().to_string();
    let stripped = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("git@github.com:"))?
        .strip_suffix(".git")
        .unwrap_or_else(|| {
            url.strip_prefix("https://github.com/")
                .or_else(|| url.strip_prefix("git@github.com:"))
                .unwrap_or("")
        })
        .to_string();
    let mut parts = stripped.splitn(2, '/');
    let org = parts.next()?.to_string();
    let repo = parts.next()?.trim_end_matches(".git").to_string();
    if org.is_empty() || repo.is_empty() {
        return None;
    }
    Some((org, repo))
}

/// Check whether a GitHub release at `<tag>` exists and carries
/// an asset named `<asset>` (the deterministic tarball).  Uses
/// the `gh` CLI; returns false if `gh` is missing or the API
/// call fails.
#[cfg(feature = "registry")]
fn github_release_has_asset(org: &str, repo: &str, tag: &str, asset: &str) -> bool {
    let out = std::process::Command::new("gh")
        .args([
            "release",
            "view",
            tag,
            "--repo",
            &format!("{org}/{repo}"),
            "--json",
            "assets",
        ])
        .output();
    let Ok(out) = out else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let body = String::from_utf8(out.stdout).unwrap_or_default();
    // Simple substring check — JSON parse would be more
    // principled but `gh release view --json assets` returns
    // `"name":"<asset>"` lines we can match directly.
    body.contains(&format!("\"name\":\"{asset}\""))
}

/// @PLAN12 Phase 6.7 — `loft audit` walks every installed package
/// in `~/.loft/registry/` and classifies each against the cached
/// advisory feed.  Exit code reflects worst severity found:
/// - 0 → clean (no matches)
/// - 1 → at least one low / bug / deprecated match
/// - 2 → at least one security_high match
/// - 3 → at least one security_critical match
///
/// Refreshes the advisory feed if stale (24h TTL) UNLESS
/// `LOFT_OFFLINE=1` is set, in which case it falls back to the
/// cached copy (or warns + returns code 0 if no cache).
///
/// Pure deep scan — no installs, no writes.  The natural
/// companion to `loft list-installed`.
#[cfg(feature = "registry")]
fn audit_installed() -> i32 {
    use loft::registry_advisories::{self, LoadOptions, Severity};

    let offline = std::env::var("LOFT_OFFLINE").is_ok();
    let opts = LoadOptions {
        allow_unsigned: true,
        offline,
        refresh: false,
    };
    let feed = match registry_advisories::load_or_fetch(&opts) {
        Ok(Some(f)) => f,
        Ok(None) => {
            if offline {
                eprintln!(
                    "[audit] advisory feed not cached and offline; can't audit (exit 0 as no-evidence)"
                );
            } else {
                eprintln!(
                    "[audit] advisory feed unavailable (registry may not host one yet); nothing to check"
                );
            }
            return 0;
        }
        Err(e) => {
            eprintln!("loft audit: {e}");
            return 4;
        }
    };

    // Enumerate cached installs (same logic as list-installed).
    let cache = loft::registry_index::cache_dir();
    let read = match std::fs::read_dir(&cache) {
        Ok(r) => r,
        Err(_) => {
            println!("No registry cache; nothing to audit.");
            return 0;
        }
    };
    let mut entries: Vec<(String, String)> = Vec::new();
    for ent in read.filter_map(Result::ok) {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let dirname = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let mut split: Option<usize> = None;
        let bytes = dirname.as_bytes();
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i - 1] == b'-' && bytes[i].is_ascii_digit() {
                split = Some(i - 1);
                break;
            }
            i += 1;
        }
        let Some(at) = split else { continue };
        let (name, rest) = dirname.split_at(at);
        let version = rest.trim_start_matches('-').to_string();
        if !name.is_empty() && !version.is_empty() {
            entries.push((name.to_string(), version));
        }
    }
    entries.sort();

    let mut all_classifications = Vec::new();
    for (name, version) in &entries {
        let hits = registry_advisories::classify(name, version, &feed);
        for hit in hits {
            all_classifications.push(hit);
        }
    }

    if all_classifications.is_empty() {
        println!(
            "{} installed package(s) audited against {} advisory entries — clean",
            entries.len(),
            feed.advisories.len()
        );
        return 0;
    }

    // Compute worst severity → exit code.
    let mut worst = 0u8;
    println!(
        "Audit found {} advisory match(es) across {} installed package(s):",
        all_classifications.len(),
        entries.len()
    );
    for hit in &all_classifications {
        worst = worst.max(hit.severity.rank());
        let prefix = match hit.severity {
            Severity::SecurityCritical => "ERROR",
            Severity::SecurityHigh => "WARN",
            Severity::SecurityLow | Severity::Bug => "NOTE",
            Severity::Deprecated => "INFO",
        };
        println!(
            "  [{prefix}] {pkg} {ver}  {sev}  {summary}",
            pkg = hit.package,
            ver = hit.version,
            sev = hit.severity.as_str(),
            summary = hit.summary,
        );
        println!("         advisory: {}", hit.advisory_id);
        if let Some(fix) = &hit.fixed_in {
            println!(
                "         fix: {pkg} >= {fix} (run `loft install {pkg}@{fix}`)",
                pkg = hit.package
            );
        }
        for r in &hit.references {
            println!("         reference: {r}");
        }
    }

    // Worst severity → exit code:
    //   Deprecated (rank 1) → 1
    //   Low / Bug (rank 2)  → 1
    //   High (rank 3)       → 2
    //   Critical (rank 4)   → 3
    match worst {
        1 | 2 => 1,
        3 => 2,
        4 => 3,
        _ => 0,
    }
}

/// Total size in bytes of a directory's contents (recursive).
/// Used by `list-installed`; cheap enough for the typical
/// ~/.loft/registry/<pkg>-<ver>/ layout (a few hundred files at
/// most per package).  Returns None on read errors so callers
/// can degrade to "size unknown" rather than failing the whole
/// listing.
#[cfg(feature = "registry")]
fn dir_size_bytes(dir: &std::path::Path) -> Option<u64> {
    let mut total: u64 = 0;
    let mut stack: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let read = std::fs::read_dir(&d).ok()?;
        for ent in read.filter_map(Result::ok) {
            let p = ent.path();
            let m = match ent.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if m.is_dir() {
                stack.push(p);
            } else if m.is_file() {
                total += m.len();
            }
        }
    }
    Some(total)
}

/// @PLAN12 Phase 6.6 — `loft pin <script>` writes a sidecar
/// `<script>.loft.lock` next to the script.  Subsequent runs of
/// that script resolve registry libraries via the sidecar (see
/// `src/parser/mod.rs::probe_sidecar_lockfile`), so the script
/// becomes reproducible regardless of cwd or registry drift.
///
/// Implementation: scans the script source for `use <name>;`
/// declarations, installs each name that exists in the registry
/// catalog (calls `install_one` with `lock_path` redirected to
/// the sidecar), and prints a summary.  Imports that aren't
/// registry packages (path-deps, sibling packages, stdlib) are
/// ignored — only registry libraries need pinning; everything
/// else either lives in the workspace or is part of loft itself.
#[cfg(feature = "registry")]
fn pin_script(script: &str) {
    use loft::install::{InstallOptions, install_one};
    use loft::registry_index;
    use std::path::PathBuf;

    let script_path = PathBuf::from(script);
    if !script_path.exists() {
        eprintln!("loft pin: script `{}` not found", script_path.display());
        std::process::exit(1);
    }
    let source = match std::fs::read_to_string(&script_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("loft pin: cannot read `{}`: {e}", script_path.display());
            std::process::exit(1);
        }
    };

    // Extract `use <name>;` declarations.  Simple line-scan rather
    // than a full parser pass — we want to pin even scripts that
    // have parse errors elsewhere.  Loft `use` syntax forms:
    //   use foo;
    //   use foo::bar;
    //   use foo::{a, b};
    //   use foo::*;
    // All start with `use <ident>` after stripping leading whitespace
    // + skipping comment lines.
    let mut uses: Vec<String> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        // Skip comments and blank lines.
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("use ") else {
            continue;
        };
        // Take the leading identifier (alphanumeric + underscore).
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() || uses.contains(&name) {
            continue;
        }
        uses.push(name);
    }

    if uses.is_empty() {
        eprintln!(
            "loft pin: no `use` declarations found in `{}`",
            script_path.display()
        );
        std::process::exit(1);
    }

    // Sidecar path next to the script.
    let mut sidecar = script_path.clone();
    let sidecar_name = format!(
        "{}.lock",
        script_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("script.loft")
    );
    sidecar.set_file_name(sidecar_name);

    // Load index once so we can filter out non-registry names
    // (path-deps + stdlib + sibling packages) before invoking
    // install_one — saves a network hit + a confusing error.
    let opts_for_index = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        lock_path: None,
    };
    let index = match loft::install::load_index(&opts_for_index) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft pin: {e}");
            std::process::exit(1);
        }
    };

    let opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        lock_path: Some(sidecar.clone()),
    };

    let mut pinned: Vec<(String, String)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for name in &uses {
        if !index.packages.contains_key(name) {
            skipped.push(name.clone());
            continue;
        }
        match install_one(name, None, &opts) {
            Ok(report) => {
                for (n, v) in report.installed.iter().chain(report.skipped_cached.iter()) {
                    if !pinned.iter().any(|(pn, _)| pn == n) {
                        pinned.push((n.clone(), v.clone()));
                    }
                }
            }
            Err(e) => {
                eprintln!("loft pin: install `{name}` failed: {e}");
                std::process::exit(1);
            }
        }
    }

    if pinned.is_empty() {
        eprintln!(
            "loft pin: no registry libraries in `{}` (only sibling / stdlib uses)",
            script_path.display()
        );
        std::process::exit(1);
    }

    println!("Pinned to {}:", sidecar.display());
    for (n, v) in &pinned {
        println!("  {n} {v}");
    }
    if !skipped.is_empty() {
        println!("Not from registry (left unresolved):");
        for n in &skipped {
            println!("  {n}");
        }
    }
    println!("{} library(ies) pinned", pinned.len());
    // PKG.STUB — stubs ride the sidecar lockfile, landing next to the script.
    let script_dir = script_path.parent().map_or_else(
        || std::path::PathBuf::from("."),
        std::path::Path::to_path_buf,
    );
    write_api_stubs(&sidecar, &script_dir);
    // Keep registry_index in the symbol table so the cfg above
    // doesn't drop the import.
    let _ = registry_index::cache_dir();
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
    use loft::data::{DefType, Type};

    let toml_path = pkg_path.join("loft.toml");
    if !toml_path.exists() {
        eprintln!("Error: no loft.toml in {}", pkg_path.display());
        std::process::exit(1);
    }
    let manifest = match loft::manifest::read_manifest(&toml_path.to_string_lossy()) {
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

/// @PLN12 phase 04 — resolve the stdlib dir and run the interactive REPL, then
/// exit.  Used by `loft repl` and by a bare `loft` with no file/subcommand.
fn start_repl() -> ! {
    // `--fresh` (anywhere on the command line): discard the saved session so
    // this launch starts clean.  Handled here by an env scan so the flag needs
    // no thread-through the arg loop, and works for both `loft repl --fresh` and
    // a bare `loft --fresh`.  @PLN12 REPL.S — text-replay auto-resume.
    if std::env::args().any(|a| a == "--fresh") {
        if let Some(path) = loft::repl::session_file_path() {
            loft::repl::ReplSession::clear_session(&path);
        }
    }
    let base = project_dir();
    let default_dir = std::path::Path::new(&base).join("default");
    let stdlib = if default_dir.exists() {
        default_dir.to_string_lossy().into_owned()
    } else {
        "default".to_string()
    };
    let stdin = std::io::stdin();
    let mut stderr = std::io::stderr();
    let code = match loft::repl::run_repl(&stdlib, stdin.lock(), &mut stderr) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("loft repl: {e}");
            1
        }
    };
    std::process::exit(code);
}

/// Collect every `--lib <dir>` import path from `args`, canonicalised (so a relative dir
/// resolves against the launch cwd) and de-duplicated.  Shared by the debugger's `--rpc`
/// and `--serve` servers so the debugged file can `use` libraries — the `use`-resolution
/// the normal run path's `--lib` handling already gives a plain `loft <file>` run.
fn collect_lib_dirs(args: &[String]) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--lib" {
            let raw = &args[i + 1];
            let abs = std::fs::canonicalize(raw)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| raw.clone());
            if !dirs.contains(&abs) {
                dirs.push(abs);
            }
            i += 1;
        }
        i += 1;
    }
    dirs
}

/// Parse a wasm-bridge crate's `Cargo.toml` text, returning each non-`loft`
/// `[dependencies]` entry as `(crate_ident, full TOML line)`.  The `loft`
/// dependency is dropped on purpose (#446): the `--html` driver links the SHARED
/// prebuilt loft via `--extern`, never through this manifest — so the bridge's
/// `loft = { path = "../../../loft" }` (which does not resolve for a
/// registry-installed package) must never reach cargo.  Only inline `[dependencies]`
/// entries are recognised (a `[dependencies.foo]` sub-table is not), matching the
/// shape every shipped bridge manifest uses.
fn bridge_nonloft_deps(cargo_text: &str) -> Vec<(String, String)> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in cargo_text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('[') {
            in_deps = rest.starts_with("dependencies]");
        } else if in_deps && !t.is_empty() && !t.starts_with('#') {
            if let Some(k) = t.split('=').next().map(str::trim) {
                if !k.is_empty() && k != "loft" {
                    deps.push((k.replace('-', "_"), t.to_string()));
                }
            }
        }
    }
    deps
}

/// Build a throwaway "deps-only" `Cargo.toml` whose `[dependencies]` are exactly
/// `nonloft_deps` (each full TOML line copied verbatim).  Building this crate
/// produces the bridge's non-`loft` dependency rlibs for wasm32 WITHOUT cargo ever
/// resolving the bridge manifest's `loft` path dep (#446).  The empty `src/lib.rs`
/// uses none of the deps, but cargo compiles every declared dependency regardless,
/// so every dep rlib still lands in the crate's deps dir.
fn synth_bridge_deps_manifest(nonloft_deps: &[(String, String)]) -> String {
    let mut manifest = String::from(
        "[package]\nname = \"loft_html_bridge_deps\"\nversion = \"0.0.0\"\n\
         edition = \"2021\"\n\n[lib]\ncrate-type = [\"rlib\"]\n\n[dependencies]\n",
    );
    for (_, line) in nonloft_deps {
        manifest.push_str(line);
        manifest.push('\n');
    }
    manifest
}

/// @PLN16 M5a — `loft debug <file>:<line>`: load the file, break at `file:line`, run
/// `main()` to the breakpoint, and drop into the interactive `(dbg)` prompt.  Reads its
/// `<file>:<line>` argument by scanning the command line (so it needs no thread-through
/// the arg loop), the same way `start_repl` reads `--fresh`.
fn run_file_debugger() -> ! {
    let args: Vec<String> = std::env::args().collect();
    let base = project_dir();
    let default_dir = std::path::Path::new(&base).join("default");
    let stdlib = if default_dir.exists() {
        default_dir.to_string_lossy().into_owned()
    } else {
        "default".to_string()
    };
    // Explicit `--lib <dir>` import paths so the debugged file can `use` libraries (not
    // just the stdlib) — the same `use`-resolution the normal run path has.
    let lib_dirs = collect_lib_dirs(&args);
    // @PLN16 M5d phase 2 — `loft debug --rpc`: the NDJSON wire-protocol server on stdio.
    // The file is supplied by the `launch` request, so no `<file>:<line>` target is
    // needed; the protocol owns stdout, program output rides `output` events.
    if args.iter().any(|a| a == "--rpc") {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        let code = match loft::rpc::run_rpc(&stdlib, &lib_dirs, stdin.lock(), &mut stdout) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("loft debug --rpc: {e}");
                1
            }
        };
        std::process::exit(code);
    }
    // @PLN16 M5e slice 1 — `loft debug <file> --serve [--port <n>]`: the browser IDE
    // foundation (HTTP shell + WebSocket protocol).  The file is the `debug` target;
    // default port 8770 (distinct from the plan-35 viewer's 8765).
    if args.iter().any(|a| a == "--serve") {
        let port = args
            .iter()
            .position(|a| a == "--port")
            .and_then(|p| args.get(p + 1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(8770);
        let file = args
            .iter()
            .position(|a| a == "debug")
            .and_then(|p| args.get(p + 1))
            .filter(|a| !a.starts_with('-'));
        let Some(file) = file else {
            eprintln!("usage: loft debug <file> --serve [--port <n>]");
            std::process::exit(2);
        };
        let code = match loft::serve::run_serve(&stdlib, &lib_dirs, port, file) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("loft debug --serve: {e}");
                1
            }
        };
        std::process::exit(code);
    }
    let target = args
        .iter()
        .position(|a| a == "debug")
        .and_then(|p| args.get(p + 1));
    let Some(target) = target else {
        eprintln!("usage: loft debug <file>:<line>  (or: loft debug --rpc / --serve)");
        std::process::exit(2);
    };
    // `rsplit_once` so a path that itself contains a colon (e.g. a Windows drive) keeps
    // it; the last `:` separates the line number.
    let Some((file, line_s)) = target.rsplit_once(':') else {
        eprintln!("usage: loft debug <file>:<line>  (missing `:<line>`)");
        std::process::exit(2);
    };
    let Ok(line) = line_s.parse::<u32>() else {
        eprintln!("loft debug: not a line number: {line_s:?}");
        std::process::exit(2);
    };
    let stdin = std::io::stdin();
    let mut stderr = std::io::stderr();
    let code = match loft::repl::run_file_debug(&stdlib, file, line, stdin.lock(), &mut stderr) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("loft debug: {e}");
            1
        }
    };
    std::process::exit(code);
}

#[allow(clippy::too_many_lines)]
fn main() {
    // Install SIGSEGV/SIGABRT/SIGBUS handler so crashes print the
    // last-executed opcode before the default handler fires.
    loft::crash_report::install("loft");
    // @PLAN49 T1+T3 — arm the execution-timeout watchdog from the env
    // (`LOFT_TIMEOUT=<secs>`) BEFORE we parse argv.  An explicit
    // `--timeout` later in argv re-arms (no-op — `arm` is idempotent)
    // but the env value is the floor.  MUST be `loft::timeout` (this
    // binary's module instance), not `loft::timeout` (the lib crate's
    // separate copy) — the binary runs its own `crate::` modules
    // (`loft::state::State` etc.), and the `checkpoint_*` call sites in
    // them resolve to `loft::timeout`, so the watchdog + breadcrumb must
    // share that same instance.  Arming `loft::timeout` set a different
    // set of statics the running code never reads.
    loft::timeout::arm(
        loft::timeout::env_timeout_secs(),
        loft::timeout::env_grace_secs(),
    );
    // Plan-07 phase 1 step 1.20 / phase 3 — chain a Rust panic hook
    // that surfaces the loft source position of the offending pc
    // before the default panic message.  Reads the per-thread snapshot
    // published by `State::execute_argv` via `crash_report`.  Falls
    // through to the default hook if no source-span snapshot is
    // active or no entry precedes the offending pc.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let (pc, _op, _fn_d_nr) = loft::crash_report::last_context();
        if pc != u32::MAX
            && let Some(pos) = loft::crash_report::source_loc_for_pc(pc)
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
    // @PLN86 F12 — `loft sandbox-check <file>`: run the admission walk ONLY and print
    // Admitted / Rejected + diagnostics, NEVER execute.  The modder's "will this be
    // allowed?" loop; the policy comes from `loft.toml` like a normal run.
    let mut sandbox_check_mode = false;
    let mut introspect_sections: Vec<loft::introspect::Section> = Vec::new();
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
        // @F48 — the loft CLI (run a program; --interpret / --native, --timeout, --help)
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
        } else if a == "sandbox-check" {
            // @PLN86 F12 — admission-only verdict; never executes (forced interpret
            // path, no codegen).  The program file follows as a positional arg.
            sandbox_check_mode = true;
            native_mode = false;
        } else if a == "--introspect" || a == "introspect" {
            // @PLN12 phase 01: introspection mode (flag or bare subcommand).
            // Default = emit bytecode + Rust + slots + types to stdout.
            // Sub-flags below narrow the section list, redirect per-section
            // output to files, and filter by function name.
            introspect_mode = true;
            native_mode = false;
        } else if a == "--show-bytecode" {
            introspect_sections.push(loft::introspect::Section::Bytecode);
        } else if a == "--bc-roundtrip" {
            introspect_sections.push(loft::introspect::Section::Roundtrip);
        } else if a == "--show-rust" {
            introspect_sections.push(loft::introspect::Section::Rust);
        } else if a == "--show-slots" {
            introspect_sections.push(loft::introspect::Section::Slots);
        } else if a == "--show-types" {
            introspect_sections.push(loft::introspect::Section::Types);
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
        // @F53 — native-binary backend (--native → rustc)
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
        } else if a == "--report-copies" {
            // @PLN90 Step 5 — the user-facing copy report: every UNBOUND structure copy
            // (Avoidable / Forced) with its location, type and fix hint + a rollup. Off by
            // default; a perf lint, never an error (COPY_DIAGNOSTICS.md).
            //
            // SAFETY: as for --dev-soft-halt — set BEFORE any analysis runs; the gate reads it
            // via a OnceLock captured on first call, so no concurrent reads are in flight.
            unsafe {
                std::env::set_var("LOFT_REPORT_COPIES", "1");
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
        // @F54 — browser / WASM target (--html / --native-wasm)
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
            loft::timeout::arm(secs, loft::timeout::env_grace_secs());
        } else if a == "--check" || a == "check" {
            check_only = true;
        } else if a == "--help" || a == "-h" || a == "-?" {
            print_help();
            return;
        } else if a == "repl" || a == "--repl" {
            // @PLN12 phase 04 — interactive `loft>` prompt.  Prompts + errors go
            // to stderr; evaluated results print to stdout (so a terminal sees
            // them and a pipe can capture them).
            start_repl();
        } else if a == "debug" {
            // @PLN16 M5a — `loft debug <file>:<line>` — interpreter-mode debugger on a
            // real source file: break at the line, drop into the interactive `(dbg)`
            // prompt (inspect / edit / step / undo).
            run_file_debugger();
        } else if a == "--fresh" {
            // @PLN12 REPL.S — recognised here only so a bare `loft --fresh`
            // reaches the REPL instead of "unknown option"; `start_repl` reads
            // the flag via an env scan and clears the saved session itself.
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
                let manifest = loft::manifest::read_manifest("loft.toml").unwrap_or_default();
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
                    let lib_file = loft::extensions::platform_lib_name(stem);
                    let prebuilt = format!("{pkg_dir}/native/{lib_file}");
                    if std::path::Path::new(&prebuilt).exists() {
                        native_lib_paths.push(prebuilt);
                    } else if let Some(built) = loft::extensions::auto_build_native(&pkg_dir, stem)
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
        // @F55 — package management (loft install, loft.toml, lockfile)
        } else if a == "install" {
            // Collect flags + positional in any order.
            #[cfg(feature = "registry")]
            let mut install_opts = loft::install::InstallOptions {
                allow_unsigned: true,
                refresh: false,
                offline: false,
                allow_prerelease: false,
                lock_path: None,
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
                    // PKG.STUB — refresh the in-project API stubs alongside
                    // the lockfile this install just wrote.
                    let cwd = std::env::current_dir().unwrap_or_default();
                    write_api_stubs(&cwd.join("loft.lock"), &cwd);
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
                let json = argv[i..].iter().any(|s| s == "--json");
                let query = argv[i..]
                    .iter()
                    .find(|s| !s.starts_with('-'))
                    .cloned()
                    .unwrap_or_default();
                search_registry(&query, json);
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
        } else if a == "api" {
            // PKG.STUB — agent-facing discovery: list reachable libraries, print
            // one library's public surface, or emit it as JSON (`--json`) for the
            // registry CI's `api` re-derive (S7-CI).
            #[cfg(feature = "registry")]
            {
                // First non-flag arg = the package dir (default cwd); robust to
                // flag order (`loft api --json <dir>` or `loft api <dir> --json`).
                let target = argv[i..].iter().find(|s| !s.starts_with('-')).cloned();
                if argv[i..].iter().any(|s| s == "--json") {
                    // The function-level surface as `[{ "sig":…, "doc":… }, …]` —
                    // the registry `validate.py` runs this on the cloned source and
                    // rejects a pasted `api` that disagrees (the no-drift gate).
                    let dir = target.map_or_else(
                        || std::env::current_dir().unwrap_or_default(),
                        std::path::PathBuf::from,
                    );
                    let items = loft::documentation::pkg_api_items(&dir);
                    println!("{}", loft::json::to_json_string(&api_items_json(&items)));
                } else if argv[i..].iter().any(|s| s == "--registry") {
                    let refresh = argv[i..].iter().any(|s| s == "--refresh");
                    api_registry_catalog(refresh);
                } else {
                    api_command(target.as_deref());
                }
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft api: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "bundle" {
            // @PLAN12 Phase 6.11 — offline bundle export/import.
            #[cfg(feature = "registry")]
            {
                let Some(sub) = argv.get(i) else {
                    eprintln!("loft bundle: subcommand required (export | import)");
                    std::process::exit(1);
                };
                i += 1;
                if sub == "export" {
                    let mut packages: Option<Vec<String>> = None;
                    let mut all = false;
                    let mut outdir: Option<String> = None;
                    while i < argv.len() {
                        let a2 = argv[i].as_str();
                        if a2 == "--all" {
                            all = true;
                            i += 1;
                        } else if a2 == "--packages" {
                            i += 1;
                            let Some(list) = argv.get(i) else {
                                eprintln!(
                                    "loft bundle export: --packages requires a comma-separated list"
                                );
                                std::process::exit(1);
                            };
                            packages = Some(
                                list.split(',')
                                    .filter(|s| !s.is_empty())
                                    .map(str::to_string)
                                    .collect(),
                            );
                            i += 1;
                        } else if !a2.starts_with('-') && outdir.is_none() {
                            outdir = Some(a2.to_string());
                            i += 1;
                        } else {
                            eprintln!("loft bundle export: unknown argument `{a2}`");
                            std::process::exit(1);
                        }
                    }
                    let Some(out) = outdir else {
                        eprintln!("loft bundle export: <outdir> required");
                        std::process::exit(1);
                    };
                    let code = bundle_export(&out, packages.as_deref(), all);
                    std::process::exit(code);
                } else if sub == "import" {
                    let Some(indir) = argv.get(i) else {
                        eprintln!("loft bundle import: <indir> required");
                        std::process::exit(1);
                    };
                    let code = bundle_import(indir);
                    std::process::exit(code);
                }
                eprintln!("loft bundle: unknown subcommand `{sub}`");
                std::process::exit(1);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft bundle: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "update" {
            // @PLAN12 Phase 6.8 — refresh lockfile entries to latest
            // active version within each dep's loft.toml range.
            #[cfg(feature = "registry")]
            {
                let mut update_opts = UpdateOpts::default();
                let mut target: Option<String> = None;
                while i < argv.len() {
                    let a2 = argv[i].as_str();
                    if a2 == "--dry-run" {
                        update_opts.dry_run = true;
                        i += 1;
                    } else if a2 == "--check" {
                        update_opts.check_only = true;
                        i += 1;
                    } else if !a2.starts_with('-') && target.is_none() {
                        target = Some(a2.to_string());
                        i += 1;
                    } else {
                        eprintln!("loft update: unknown argument `{a2}`");
                        std::process::exit(1);
                    }
                }
                update_opts.target = target;
                let code = update_packages(&update_opts);
                // PKG.STUB — a real update rewrote the lockfile; refresh the
                // in-project API stubs with it.
                if code == 0 && !update_opts.dry_run && !update_opts.check_only {
                    let cwd = std::env::current_dir().unwrap_or_default();
                    write_api_stubs(&cwd.join("loft.lock"), &cwd);
                }
                std::process::exit(code);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft update: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "yank" {
            // @PLAN12 Phase 6.7a — author-side yank workflow.
            // Emits the index.json + advisories.json edits ready
            // for the registry PR.
            #[cfg(feature = "registry")]
            {
                let mut target: Option<String> = None;
                let mut severity: Option<String> = None;
                let mut advisory: Option<String> = None;
                let mut summary: Option<String> = None;
                let mut affected: Option<String> = None;
                let mut fixed_in: Option<String> = None;
                while i < argv.len() {
                    let a2 = argv[i].as_str();
                    if a2 == "--severity" {
                        i += 1;
                        severity = argv.get(i).cloned();
                        i += 1;
                    } else if a2 == "--advisory" {
                        i += 1;
                        advisory = argv.get(i).cloned();
                        i += 1;
                    } else if a2 == "--summary" {
                        i += 1;
                        summary = argv.get(i).cloned();
                        i += 1;
                    } else if a2 == "--affected" {
                        i += 1;
                        affected = argv.get(i).cloned();
                        i += 1;
                    } else if a2 == "--fixed-in" {
                        i += 1;
                        fixed_in = argv.get(i).cloned();
                        i += 1;
                    } else if !a2.starts_with('-') && target.is_none() {
                        target = Some(a2.to_string());
                        i += 1;
                    } else {
                        eprintln!("loft yank: unknown argument `{a2}`");
                        std::process::exit(1);
                    }
                }
                let Some(target) = target else {
                    eprintln!("loft yank: <pkg>@<version> required");
                    eprintln!(
                        "  Usage: loft yank <pkg>@<ver> --severity <tier> --advisory <id> \\"
                    );
                    eprintln!(
                        "           --summary \"...\" --affected \">=X, <Y\" --fixed-in \"<ver>\""
                    );
                    std::process::exit(1);
                };
                let code = yank_package(
                    &target,
                    severity.as_deref(),
                    advisory.as_deref(),
                    summary.as_deref(),
                    affected.as_deref(),
                    fixed_in.as_deref(),
                );
                std::process::exit(code);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft yank: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "new" {
            // @PLAN12 — `loft new <name>` scaffolds a fresh library
            // package: loft.toml + src/<name>.loft stub + tests/ +
            // canonical library-ci.yml (when --chunk is set).
            let mut native = false;
            let mut chunk = false;
            let mut name: Option<String> = None;
            while i < argv.len() {
                let a2 = argv[i].as_str();
                if a2 == "--native" {
                    native = true;
                    i += 1;
                } else if a2 == "--chunk" {
                    chunk = true;
                    i += 1;
                } else if !a2.starts_with('-') && name.is_none() {
                    name = Some(a2.to_string());
                    i += 1;
                } else {
                    eprintln!("loft new: unknown argument `{a2}`");
                    std::process::exit(1);
                }
            }
            let Some(name) = name else {
                eprintln!("loft new: library name required");
                eprintln!("  Usage: loft new <name> [--native] [--chunk]");
                std::process::exit(1);
            };
            let code = scaffold_library(&name, native, chunk);
            std::process::exit(code);
        } else if a == "publish" {
            // @PLAN12 Phase 6.16 — author-side publish helper.
            // Repackages locally (deterministic), verifies the
            // GitHub release exists with the expected tag + asset,
            // emits the index.json entry ready for the registry PR.
            #[cfg(feature = "registry")]
            {
                let mut dry_run = false;
                let mut pkg_path: Option<String> = None;
                while i < argv.len() {
                    let a2 = argv[i].as_str();
                    if a2 == "--dry-run" {
                        dry_run = true;
                        i += 1;
                    } else if !a2.starts_with('-') && pkg_path.is_none() {
                        pkg_path = Some(a2.to_string());
                        i += 1;
                    } else {
                        eprintln!("loft publish: unknown argument `{a2}`");
                        std::process::exit(1);
                    }
                }
                let dir = pkg_path
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                let code = publish_package(&dir, dry_run);
                std::process::exit(code);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft publish: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "audit" {
            // @PLAN12 Phase 6.7 — explicit deep scan: every cached
            // package vs the advisory feed.  Exit code reflects
            // worst severity.
            #[cfg(feature = "registry")]
            {
                let code = audit_installed();
                std::process::exit(code);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft audit: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "list-installed" {
            // @PLAN12 Phase 6.6 — enumerate ~/.loft/registry/<pkg>-<ver>/
            // dirs, annotate with sha256 + size from the cached index.
            #[cfg(feature = "registry")]
            {
                list_installed();
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!(
                    "loft list-installed: this binary was built without the `registry` feature."
                );
                std::process::exit(1);
            }
        } else if a == "pin" {
            // @PLAN12 Phase 6.6 — write a sidecar lockfile next to
            // a script so subsequent runs use pinned versions
            // regardless of cwd or registry drift.
            #[cfg(feature = "registry")]
            {
                let Some(script) = argv.get(i) else {
                    eprintln!("loft pin: script path required");
                    std::process::exit(1);
                };
                pin_script(script);
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft pin: this binary was built without the `registry` feature.");
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
        } else if a == "build-native" {
            // @PLN21 Phase 4 producer — build a package's native cdylib for THIS
            // host and report the artifact + the loft-ffi fingerprint + the host
            // triple, so CI can publish it as a `prebuilt/<triple>/` registry
            // binary.  Runs NO program (a graphics lib needs no display) — just
            // the cdylib + its fp, the two things `loft install` then needs.
            #[cfg(feature = "registry")]
            {
                let pkg_path = if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                    std::path::PathBuf::from(&argv[i])
                } else {
                    std::env::current_dir().unwrap_or_default()
                };
                // Canonicalize: the auto-native branch resolves the library via
                // `use <name>` and filters its defs by `pkg_str` prefix, so the
                // path the parser opens and `pkg_str` must be one form.
                let pkg_path = std::fs::canonicalize(&pkg_path).unwrap_or(pkg_path);
                let manifest =
                    loft::manifest::read_manifest(&pkg_path.join("loft.toml").to_string_lossy());
                let pkg_str = pkg_path.to_string_lossy().to_string();
                // Machine-readable so a publish script can capture it.  The KEY
                // line differs by native model, because the two cdylibs have
                // different ABI contracts (see the auto-native branch below):
                //   • hand-written links loft-ffi's `#[repr(C)]` surface → valid for
                //     ANY loft on the same loft-ffi → `loft_ffi_fp`;
                //   • auto-compiled `extern crate loft` (statically embeds libloft,
                //     shares repr(Rust) `Stores`/`DbRef` by memory) → valid only for a
                //     byte-identical loft build → `loft_build_fp` + the rustc it used.
                let report = |cdylib: String, stem: String, keys: &[String]| {
                    println!("cdylib: {cdylib}");
                    println!("stem: {stem}");
                    println!("triple: {}", loft::cache::host_triple());
                    for k in keys {
                        println!("{k}");
                    }
                };
                if let Some(stem) = manifest.as_ref().and_then(|m| m.native.clone()) {
                    // Hand-written `native/` crate — links the loft-ffi C ABI, so the
                    // wide loft-ffi key is the correct compatibility gate.
                    let keys = [format!(
                        "loft_ffi_fp: {}",
                        loft::cache::loft_ffi_fingerprint()
                    )];
                    match loft::extensions::auto_build_native(&pkg_str, &stem) {
                        Some(cdylib) => report(cdylib, stem, &keys),
                        None => {
                            eprintln!(
                                "loft build-native: building `{stem}` failed (see the cargo error above)"
                            );
                            std::process::exit(1);
                        }
                    }
                } else if let Some(name) = manifest.and_then(|m| m.name) {
                    // @PLN21 Phase 4b — AUTO-COMPILED native (the default
                    // "libraries compile, scripts interpret" path): parse the
                    // library the `use`-resolution way, then build its auto-native
                    // cdylib (`loft_auto_<dir>`, exporting `loft_shared_<fn>`
                    // wrappers) FROM the loft source — so a pure-loft compute
                    // library also ships a toolchain-free prebuilt.
                    let mut p = parser::Parser::new();
                    // Resolve the library FROM the handed package path (a fresh
                    // checkout, not necessarily registry-installed): its parent dir
                    // lets `use <name>` resolve `<parent>/<name>` (and sibling deps)
                    // BEFORE the registry fallback.  Without it `use <name>` finds a
                    // same-named registry package and `library_export_set` (filtering
                    // by `pkg_str` prefix) sees none of its defs.
                    if let Some(parent) = pkg_path.parent() {
                        p.lib_dirs.push(parent.to_string_lossy().to_string());
                    }
                    let default_dir = format!("{}/default", project_dir());
                    let _ = p.parse_dir(&default_dir, true, false);
                    let tmp = std::env::temp_dir()
                        .join(format!("loft_build_native_{}.loft", std::process::id()));
                    let _ = std::fs::write(&tmp, format!("use {name};\n"));
                    p.parse(&tmp.to_string_lossy(), false);
                    let _ = std::fs::remove_file(&tmp);
                    // Slot/scope analysis the cdylib codegen depends on — the run
                    // path runs this before its auto-native build (main.rs), and
                    // without it codegen emits undeclared locals (e.g. `var_me`).
                    scopes::check(&mut p.data);
                    let export = loft::native_lib::library_export_set(&p.data, &pkg_str);
                    if export.is_empty() {
                        eprintln!(
                            "loft build-native: `{name}` has no native-compilable public functions"
                        );
                        std::process::exit(1);
                    }
                    match loft::native_lib::cached_or_build_shared_cdylib(
                        &p.data,
                        &p.database,
                        &export,
                        &pkg_str,
                    ) {
                        Ok(Some(so)) => {
                            // This cdylib `extern crate loft`s — it statically embeds
                            // libloft and operates on the host's repr(Rust)
                            // `Stores`/`DbRef` by shared memory, so it is valid ONLY
                            // for a byte-identical loft build (the `loft_build_fp` rlib
                            // hash, which already folds in source + rustc).  Reporting
                            // `loft_ffi_fp` here would mislabel it as widely portable —
                            // a corruption-shaped trap for any consumer that gated on
                            // it.  rustc is named too, for human/diagnostic clarity.
                            report(
                                so.to_string_lossy().to_string(),
                                loft::native_lib::auto_cdylib_stem(&pkg_str),
                                &[
                                    format!(
                                        "loft_build_fp: {}",
                                        loft::cache::loft_build_fingerprint()
                                    ),
                                    format!(
                                        "rustc: {}",
                                        option_env!("LOFT_BUILD_RUSTC").unwrap_or("unknown")
                                    ),
                                ],
                            );
                        }
                        Ok(None) => {
                            eprintln!(
                                "loft build-native: `{name}` is being edited (dev-interpret); no cdylib produced"
                            );
                            std::process::exit(1);
                        }
                        Err(e) => {
                            eprintln!(
                                "loft build-native: auto-native build of `{name}` failed: {e}"
                            );
                            std::process::exit(1);
                        }
                    }
                } else {
                    eprintln!(
                        "loft build-native: {} is not a loft package (no [package] name)",
                        pkg_path.display()
                    );
                    std::process::exit(1);
                }
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!(
                    "loft build-native: requires the `registry` feature; rebuild with default features."
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
        loft::timeout::arm(300, loft::timeout::env_grace_secs());
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
        // @PLN12 phase 04 — no file and no subcommand: drop into the interactive
        // REPL (like `python` / `node` / `irb`).  Works for a terminal and for
        // piped input; the banner advertises `:help`.
        start_repl();
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
    // path shares one representation (see `strip_verbatim_disk`).
    let abs_file = strip_verbatim_disk(abs_file);
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
    // regardless of working directory changes during parsing.  Strip the
    // Windows verbatim prefix afterwards: everything downstream of
    // `lib_dirs` — `use` candidates, the package dirs recorded into
    // `pending_native_compile`, `def.position().file` prefix checks — must
    // share `abs_file`'s plain representation.  A verbatim entry here is
    // how the #460 entry-package skip (`entry_path.starts_with(pkg_dir)`)
    // silently missed on Windows: plain entry vs verbatim pkg_dir never
    // prefix-match, so the entry package auto-native-compiled after all.
    let lib_dirs: Vec<String> = lib_dirs
        .into_iter()
        .map(|d| {
            strip_verbatim_disk(
                std::fs::canonicalize(&d)
                    .unwrap_or_else(|_| std::path::PathBuf::from(&d))
                    .to_string_lossy()
                    .into_owned(),
            )
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
    // Step 0 (startup-cache plan): env-gated parse timing (LOFT_TIMING).
    let t_parse_default = std::time::Instant::now();
    let default_dir = std::path::Path::new(&dir).join("default");
    let default_str = default_dir.to_string_lossy().to_string();
    // @PLN11 arc E / D2b / track 1 — the whole-program startup cache mmaps the
    // ENTIRE parsed program (stdlib + lazily-loaded libs + user file) on a
    // repeated unchanged run, skipping all parsing (~3–3.6× faster).  It is now
    // **default-on** (`cache::program_cache_enabled`): off only under
    // `LOFT_NO_CACHE`, or automatically when running inside Cargo (`cargo run` /
    // the test suite — the dev-safety + test-isolation default).  The narrower
    // stdlib cache (`LOFT_STDLIB_CACHE`, D2b) caches `default/` only and engages
    // just when the program cache is off.
    let program_cache_on = loft::cache::program_cache_enabled();
    p.track_sources = program_cache_on;
    // @PLN11 G2/M6 — on a warm hit with LOFT_CODEGEN_STORE, the cache is loaded
    // as a SKELETON (def table only) and the mmap'd bundle store is returned
    // here so codegen reads bodies straight from it (no read_data body rebuild).
    // @PLN86 — load the program's `[sandbox]` policy (loft.toml next to the file)
    // BEFORE the warm-load decision.  A whole-program warm load restores the IR
    // WITHOUT re-parsing, so the per-def designations (`def_sandbox`) would never
    // form and admission — AND the force-interpret guard — would be silently
    // skipped on every warm run; worse, the cache is keyed by program content, not
    // policy, so a TIGHTENED policy would be ignored.  A sandboxed program must
    // therefore always parse fresh: disable the warm-load when a policy is active.
    // The host owns this policy — a script cannot designate itself.
    if let Some(dir) = std::path::Path::new(&abs_file).parent()
        && let Ok(content) = std::fs::read_to_string(dir.join("loft.toml"))
    {
        p.set_sandbox_config(loft::sandbox::parse_sandbox_config(&content));
    }
    let mut warm_store: Option<(loft::database::Stores, loft::keys::DbRef)> = None;
    // #358 — a warm hit returns the def-table index where user definitions
    // start; the cold path derives it from the post-stdlib def count below.
    let warm_user_start = if program_cache_on && !p.sandbox_is_active() {
        loft::startup_cache::warm_load_program(&mut p, &abs_file, &mut warm_store)
    } else {
        None
    };
    let program_warm = warm_user_start.is_some();
    if !program_warm {
        // When building a program cache, parse `default/` fresh so every stdlib
        // file lands in the program's drift manifest; otherwise use the D2b
        // stdlib cache.
        let stdlib_warm =
            !program_cache_on && loft::startup_cache::warm_load_stdlib(&mut p, &default_str);
        if !stdlib_warm {
            if let Err(e) = p.parse_dir(&default_str, true, false) {
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
            if !program_cache_on {
                loft::startup_cache::save_stdlib_cache(&p, &default_str);
            }
        }
    }
    // #358 — on a warm load the def table already holds stdlib + user defs, so
    // `definitions()` would put `start_def` PAST the user fns and the no-`main`
    // test-fn fallback would silently execute nothing; use the persisted boundary.
    let start_def = warm_user_start.unwrap_or_else(|| p.data.definitions());
    if std::env::var("LOFT_TIMING").is_ok() {
        eprintln!(
            "LOFT_TIMING parse_default={:.2}ms ({start_def} defs)",
            t_parse_default.elapsed().as_secs_f64() * 1000.0
        );
    }
    // `--show-types --trace`: enable per-expression type recording
    // BEFORE parsing the user file (parse_dir on default/* already
    // ran without tracing — those are stdlib internals).
    if introspect_mode && introspect_trace {
        p.trace_types = true;
    }
    // @PLN11 arc E — a whole-program warm load already holds every definition;
    // skip parsing the user file (and its lib loads) entirely.  (The `[sandbox]`
    // policy was loaded above, before the warm-load gate, so designations form
    // during this parse — and a sandboxed program is never warm-loaded.)
    if !program_warm {
        p.parse(&abs_file, false);
    }
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
    // @PLN86 F12 — `sandbox-check`: report the admission verdict and STOP, never
    // executing.  The whole point is a no-run "will this be allowed?" surface, so this
    // returns before any codegen/run path below.
    if sandbox_check_mode {
        let errors = p.sandbox_admission_errors();
        if errors.is_empty() {
            if p.has_sandboxed_defs() {
                println!("Admitted: no admission violations.");
                println!("{}", p.sandbox_complexity_report());
            } else {
                println!(
                    "Admitted: no sandboxed code is designated (check the [sandbox] \
                     policy in loft.toml)."
                );
            }
            std::process::exit(0);
        }
        eprintln!("Rejected: {} admission violation(s):", errors.len());
        for e in &errors {
            eprintln!("{e}");
        }
        std::process::exit(1);
    }
    // @PLN86 2.5 — admission: a program that designates sandboxed code is admitted
    // only if it is proven safe at LOAD (capabilities + totality + no-raw-write).
    // Reject with the actionable errors before it ever runs — the contract the modder
    // writes against; a clean compile is the guarantee.  This is BACKEND-AGNOSTIC: an
    // admitted script is total and fault-free on the interpreter AND on `--native`
    // (bounded loops, an acyclic call graph, partial ops total on both — div/mod-zero,
    // OOB, overflow all yield null natively too), so the host keeps its choice of
    // backend.  (A deployment that wants to forbid host-side `rustc` on mod-derived
    // input can opt in to interpret-only per profile; the cdylib-FFI surface is already
    // gated by `native_ffi`.)
    if p.has_sandboxed_defs() {
        let errors = p.sandbox_admission_errors();
        if !errors.is_empty() {
            eprintln!(
                "error: sandboxed code rejected — {} admission violation(s):",
                errors.len()
            );
            for e in &errors {
                eprintln!("{e}");
            }
            std::process::exit(1);
        }
    }
    scopes::check(&mut p.data);
    // @PLN90 Step 5 — the user-facing copy report, emitted ONCE here (the whole program is now
    // loaded + checked) rather than per file-load. Gated on `--report-copies`; a no-op otherwise.
    loft::use_analysis::report_copies(&p.data);
    // @PLN11 Arc N / N3 (Step 2) — auto-compile `use`d libraries that opted in via
    // `[library] compile = "native"`, **build-before-mark**: build each library's
    // cdylib from the post-parse type schema FIRST, then mark its functions native
    // (so `byte_code` emits `OpStaticCall`) ONLY on success.  A build failure (or
    // `LOFT_FORCE_NATIVE_BUILD_FAIL`) leaves the library unmarked → it **silently
    // interprets** — byte-identical, no `exit`, no dispatch to an unbuilt symbol.
    // An auto-native program bypasses the program cache for now (artifact caching is
    // N1/Phase D).
    // `LOFT_NO_NATIVE_LIBS=1` forces ALL `use`d libraries to interpret — the parity
    // reference (a library run native ≡ run interpreted) and a manual escape hatch.
    let native_libs_off = std::env::var_os("LOFT_NO_NATIVE_LIBS").is_some();
    let force_build_fail = std::env::var_os("LOFT_FORCE_NATIVE_BUILD_FAIL").is_some();
    // Crawler / efficiency aid (`LOFT_REQUIRE_NATIVE=1`, the inverse of
    // `LOFT_NO_NATIVE_LIBS`): turn every native→interpreter fallback below into a
    // HARD ERROR that names the reason, so a performance run can never silently run
    // slow interpreted code.  Off by default — the normal warn-and-interpret
    // fallbacks are unchanged.  Enforced at two points (the only places native can
    // degrade to interpretation): the auto-native library loop just below, and the
    // main-program chokepoint past the `'native` block.
    let native_required = std::env::var("LOFT_REQUIRE_NATIVE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let pending_native = if native_libs_off {
        p.pending_native_compile.clear();
        Vec::new()
    } else {
        std::mem::take(&mut p.pending_native_compile)
    };
    let mut auto_native_libs: Vec<String> = Vec::new();
    let mut any_dev_interpret = false;
    // #460 — never auto-native-compile the package that OWNS the entry file: that
    // package is the *script* being run, not a `use`d library, and the model is
    // "libraries compile, scripts interpret".  Its export set is entry-point
    // dependent (only files reachable from THIS entry get parsed), so a cdylib
    // built for one entry (e.g. `selftest.loft` → exports `build_walls`) is wrong
    // for another (`equiptest.loft` → marks `player_new`): the freshness check
    // adopts the stale `.so`, then `mark_exports` marks a symbol it never built →
    // `OpStaticCall` to a missing bridge → the compile.rs panic stub.  Under
    // `--native` these functions compile into the whole-program binary anyway, so
    // excluding them from the cdylib loop costs nothing there.
    let entry_path = std::path::Path::new(&abs_file);
    for pkg_dir in &pending_native {
        if entry_path.starts_with(pkg_dir) {
            continue;
        }
        // #453 — for an `--html` build a `[wasm.bridge]` library builds its WASM
        // bridge (the `--html` branch below), not a native cdylib. Building the
        // native cdylib here is wasted work that FAILS for a browser-only bridge
        // lib: its `#native` symbols route through the bridge, so they have no
        // native implementation (P269). Skip it — the bridge build downstream uses
        // the routes that `register_native_manifest` registered. (Populated only
        // because the `[wasm.bridge]` registration there is now ungated by
        // `[native]`; the two halves of #453 are a pair.)
        if html_out.is_some()
            && p.data
                .wasm_bridge_packages
                .iter()
                .any(|(_, dir)| dir == pkg_dir)
        {
            continue;
        }
        // @PLN18 — `[native] in_binary = true`: the library's natives are
        // registered inside this binary (src/native.rs); a cdylib compile can
        // only fail (the symbols exist nowhere else).  Skip it silently.
        if manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
            .is_some_and(|m| m.native_in_binary)
        {
            continue;
        }
        let export = loft::native_lib::library_export_set(&p.data, pkg_dir);
        if export.is_empty() {
            continue;
        }
        let built = if force_build_fail {
            Err("LOFT_FORCE_NATIVE_BUILD_FAIL".to_string())
        } else {
            loft::native_lib::cached_or_build_shared_cdylib(&p.data, &p.database, &export, pkg_dir)
        };
        match built {
            Ok(Some(so)) => {
                loft::native_lib::mark_exports(&mut p.data, &export);
                auto_native_libs.push(so.to_string_lossy().into_owned());
            }
            Ok(None) => {
                // Dev-interpret-on-edit (Step 4): the library is being actively
                // edited, so it interprets this run — instant loop, no `rustc`, no
                // marking, no warning (this is the intended fast path while you iterate).
                // Do NOT cache the program: a warm load would replay this interpreted
                // image and the "rebuild once editing settles" check would never fire.
                if native_required {
                    eprintln!(
                        "loft: LOFT_REQUIRE_NATIVE is set, but library '{pkg_dir}' is being \
                         edited (dev-interpret) and would run interpreted, not native. \
                         Let its cdylib build (stop editing it), or unset LOFT_REQUIRE_NATIVE."
                    );
                    std::process::exit(1);
                }
                any_dev_interpret = true;
            }
            Err(e) => {
                // The fallback rule: interpreting a library instead of building it
                // native is graceful ONLY when native is impossible here — i.e. there
                // is no Rust toolchain.  When `rustc` IS installed, a failed native
                // build is a REAL failure: silently interpreting it would hand back a
                // partly-interpreted binary (or one whose `#native` functions panic at
                // runtime) while the caller asked for native.  So hard-fail when a
                // toolchain is present (and always under LOFT_REQUIRE_NATIVE); fall back
                // only when there is genuinely no toolchain to build with.
                if native_required || loft::native_lib::rustc_available() {
                    let why = if native_required {
                        "LOFT_REQUIRE_NATIVE is set"
                    } else {
                        "rustc is installed, so this is a real build failure, not a \
                         missing-toolchain fallback"
                    };
                    eprintln!(
                        "loft: library '{pkg_dir}' failed to build native ({e}).\n\
                         {why} — refusing to silently interpret it (that would hand back a \
                         partly-interpreted binary, or one whose #native functions panic \
                         when called).  Fix the library's native build, or run with \
                         --interpret to run interpreted on purpose."
                    );
                    std::process::exit(1);
                }
                eprintln!(
                    "loft: library '{pkg_dir}' has no native build and no Rust toolchain to \
                     build one ({e}); interpreting it.  Install rustc for a native build."
                );
            }
        }
    }
    let has_auto_native = !auto_native_libs.is_empty();
    // @PLN11 G2 / M0 — equivalence harness.  With `LOFT_IR_CHECK` set, assert
    // the store-materialised IR is bit-for-bit identical to the native `Data`
    // before any subsystem is rewired to read from the store.  Opt-in (default
    // off): validates the store-mirror invariant on the *actual* program being
    // run — user code + lazily-loaded libs — not just the stdlib round-trip tests.
    if std::env::var_os("LOFT_IR_CHECK").is_some_and(|v| !v.is_empty()) {
        if let Err(diff) = loft::ir_read::ir_roundtrip_check(&p.data) {
            eprintln!("LOFT_IR_CHECK: store-backed IR diverges from native Data: {diff:?}");
            std::process::exit(1);
        }
        if std::env::var_os("LOFT_TIMING").is_some() {
            eprintln!(
                "LOFT_IR_CHECK: store round-trip == native ({} defs)",
                p.data.definitions()
            );
        }
    }
    // @PLN11 arc E — on a cold run with the program cache enabled, write the
    // whole-program bundle + drift manifest (post-`scopes::check`, so loaded
    // functions carry `done=true` and the baked free-ops).
    // Skip the program cache for auto-native programs: the warm-load path restores
    // the parsed `Data` without re-running manifest detection, so it would have the
    // `def.native` markings but no rebuilt cdylib to wire (Phase D persists this).
    // Also skip when a library took the dev-interpret-on-edit path (Step 4): caching
    // the interpreted image would pin it, so the "rebuild once editing settles" check
    // would never run on a warm load.
    if program_cache_on && !program_warm && !has_auto_native && !any_dev_interpret {
        loft::startup_cache::save_program(&p, &abs_file, start_def);
    }
    // @PLAN28 debug/validation hook — when `LOFT_DUMP_SNAPSHOT=<path>` is set,
    // write the parsed `Data` as the startup-cache JSON snapshot and exit.
    // This is the manual validation path for the quick-loading work: dump the
    // post-parse image, then (later) load it back and diff.  The JSON is the
    // machine format (compact); format it externally if you need to eyeball.
    // No effect unless the env var is set.
    if let Ok(path) = std::env::var("LOFT_DUMP_SNAPSHOT") {
        let json = loft::ir_schema::data_to_json(&p.data);
        if let Err(e) = std::fs::write(&path, &json) {
            eprintln!("loft: failed to write snapshot to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!(
            "loft: wrote startup-cache snapshot ({} bytes, {} defs) to {path}",
            json.len(),
            p.data.definitions()
        );
        std::process::exit(0);
    }
    if std::env::var_os("LOFT_DUMP_TYPES").is_some() {
        p.database.dump_types();
    }
    let mut state = State::new(p.database);
    // Set source_dir for the source_dir() built-in.
    state.database.source_dir = std::path::Path::new(&abs_file)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    // #255 / @PLN9: LOFT_PATHS overrides the path-resolution mode for this run
    // (`program` = re-home relative paths to the program's own directory;
    // `cwd` = the process cwd).  Unset → the parsed default / `#cwd` directive.
    if let Ok(mode) = std::env::var("LOFT_PATHS") {
        state.database.program_relative = mode.eq_ignore_ascii_case("program");
    }
    // store script-level arguments so arguments() returns only these.
    state.database.user_args.clone_from(&user_args);
    // @PLN11 G2/M6 — lower from the warm-loaded mmap store when present.
    // (The auto-native cdylibs were already built + their symbols marked above,
    // build-before-mark, so `byte_code` has emitted `OpStaticCall` only for the
    // libraries that compiled.)
    compile::byte_code_with_store(&mut state, &mut p.data, warm_store.as_ref());
    // load native extension shared libraries registered during parsing.
    // Also include any native libs discovered via loft.toml auto-detection.
    let mut all_native_libs = std::mem::take(&mut p.pending_native_libs);
    for nlp in &native_lib_paths {
        if !all_native_libs.contains(nlp) {
            all_native_libs.push(nlp.clone());
        }
    }
    all_native_libs.extend(auto_native_libs);
    extensions::load_all(&mut state, all_native_libs.clone());
    // PKG.5: wire auto-marshalled native functions from loaded cdylibs.
    extensions::wire_native_fns(&mut state, &p.data);
    // @PLN11 Arc N / N3 — wire the shared-store bridge dispatchers for the
    // auto-native libraries (the `loft_shared_*` symbols), a disjoint set from the
    // hand-written `#native` symbols `wire_native_fns` handles.
    extensions::wire_shared_native_fns(&mut state, &p.data);

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
            let mut out = generation::Output::new(&p.data, &state.database);
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
        if std::env::var("LOFT_KEEP_NATIVE_RS").is_err() {
            let _ = std::fs::remove_file(&rs_path);
        } else {
            eprintln!(
                "loft: wasm source preserved at {} (LOFT_KEEP_NATIVE_RS)",
                rs_path.display()
            );
        }
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
            let mut out = generation::Output::new(&p.data, &state.database);
            out.wasm_browser = true;
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
                let host_deps = deps_dir_of(&host_lib_dir);
                if host_deps.exists() {
                    cmd.arg("-L")
                        .arg(format!("dependency={}", host_deps.display()));
                }
            }
        }
        // lib_plan-29 W1c (2026-05-29): link each used library's wasm
        // bridge crate (declared via `[wasm.bridge]` in `loft.toml`).
        //
        // Build via rustc directly, not `cargo build`: cargo would
        // produce a SECOND copy of `loft` (with a different
        // StableCrateId than the top-level
        // `target/wasm32-unknown-unknown/release/libloft.rlib` the
        // standalone-binary `--extern loft=…` references), and rustc
        // would refuse with "expected DbRef, found DbRef" (two distinct
        // types from two builds of the same source).  By invoking
        // rustc with the SAME `--extern loft=…` + deps search path
        // the standalone build uses, the bridge rlib links against
        // exactly one copy of loft — eliminating the dup.
        let loft_wasm_lib_dir = loft_lib_dir_for(Some("wasm32-unknown-unknown"));
        for (bridge_crate, pkg_dir) in &p.data.wasm_bridge_packages {
            let wasm_dir = std::path::PathBuf::from(pkg_dir).join("wasm");
            let bridge_src = wasm_dir.join("src/lib.rs");
            if !bridge_src.exists() {
                eprintln!(
                    "loft: --html: [wasm.bridge] declared `crate = \"{bridge_crate}\"` \
                     but {} is missing — skipping bridge link",
                    bridge_src.display()
                );
                continue;
            }
            let crate_ident = bridge_crate.replace('-', "_");
            let bridge_rlib = std::env::temp_dir().join(format!("lib{crate_ident}.rlib"));
            // Build-extension (@PLN84 ZT-B): when the bridge crate declares Cargo
            // dependencies beyond `loft` (the vetted dalek/RustCrypto stack the SHARED
            // ed25519/x25519/aes modules use), build those deps for wasm32 to produce
            // their rlibs.  Each DIRECT dep is `--extern`'d on the bridge rustc compile
            // (the `use <crate>` extern-prelude entry — `-L` alone resolves only
            // TRANSITIVE deps), and the deps dir is `-L`'d on both the bridge compile
            // and the main wasm link (transitive symbols).  We consume ONLY the
            // dependency rlibs — the bridge itself is rustc-compiled below against the
            // SHARED prebuilt loft (no duplicate loft), and these deps are
            // loft-independent, so no "two copies of loft" (`expected DbRef, found
            // DbRef`) error arises.
            //
            // #446: we deliberately do NOT `cargo build` the bridge's own
            // `wasm/Cargo.toml`.  It carries a `loft = { path = "../../../loft" }` dep
            // that resolves only in a dev checkout — NOT for a registry-installed
            // package, where the relative path points at the nonexistent `~/.loft/loft`
            // and cargo aborts before building any dep.  That `loft` dep is pure
            // redundancy here: the bridge links the SHARED prebuilt loft via `--extern`
            // (below), never through this manifest.  So we synthesize a throwaway
            // deps-only crate whose `[dependencies]` are exactly the bridge's non-`loft`
            // deps and build THAT: cargo compiles every declared dep (even though the
            // empty lib uses none), producing the identical wasm32 rlibs without ever
            // resolving the manifest's `loft` path dep.
            let wasm_cargo = wasm_dir.join("Cargo.toml");
            // Every non-`loft` [dependencies] entry as (crate_ident, full TOML line).
            let nonloft_deps: Vec<(String, String)> = std::fs::read_to_string(&wasm_cargo)
                .map(|text| bridge_nonloft_deps(&text))
                .unwrap_or_default();
            let nonloft_idents: Vec<String> = nonloft_deps
                .iter()
                .map(|(ident, _)| ident.clone())
                .collect();
            let mut bridge_dep_search: Option<std::path::PathBuf> = None;
            let mut bridge_externs: Vec<(String, std::path::PathBuf)> = Vec::new();
            if !nonloft_deps.is_empty() {
                // Stage the deps-only crate under a per-bridge temp dir, then build it.
                let synth_dir =
                    std::env::temp_dir().join(format!("loft_html_bridge_deps_{crate_ident}"));
                let synth_src = synth_dir.join("src");
                let manifest = synth_bridge_deps_manifest(&nonloft_deps);
                let staged = std::fs::create_dir_all(&synth_src).is_ok()
                    && std::fs::write(synth_dir.join("Cargo.toml"), &manifest).is_ok()
                    && std::fs::write(synth_src.join("lib.rs"), "").is_ok();
                if !staged {
                    eprintln!(
                        "loft: --html: failed to stage wasm-bridge dependency build for {bridge_crate}"
                    );
                    std::process::exit(1);
                }
                let cargo_ok = std::process::Command::new("cargo")
                    .arg("build")
                    .arg("--release")
                    .arg("--target")
                    .arg("wasm32-unknown-unknown")
                    .arg("--manifest-path")
                    .arg(synth_dir.join("Cargo.toml"))
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if !cargo_ok {
                    eprintln!(
                        "loft: --html: failed to cargo-build wasm-bridge dependencies for {bridge_crate}"
                    );
                    std::process::exit(1);
                }
                let deps = synth_dir.join("target/wasm32-unknown-unknown/release/deps");
                if deps.is_dir() {
                    // Resolve each DIRECT dep to its `lib<ident>-<hash>.rlib` for `--extern`.
                    let files: Vec<String> = std::fs::read_dir(&deps)
                        .map(|rd| {
                            rd.flatten()
                                .map(|e| e.file_name().to_string_lossy().into_owned())
                                .collect()
                        })
                        .unwrap_or_default();
                    for ident in &nonloft_idents {
                        let prefix = format!("lib{ident}-");
                        if let Some(f) = files.iter().find(|f| {
                            f.starts_with(&prefix)
                                && std::path::Path::new(f.as_str())
                                    .extension()
                                    .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"))
                        }) {
                            bridge_externs.push((ident.clone(), deps.join(f)));
                        }
                    }
                    bridge_dep_search = Some(deps);
                }
            }
            let mut build = std::process::Command::new("rustc");
            build
                .arg("--edition=2024")
                .arg("--target")
                .arg("wasm32-unknown-unknown")
                .arg("--crate-type")
                .arg("rlib")
                .arg("--crate-name")
                .arg(&crate_ident)
                .arg("-O")
                .arg("-o")
                .arg(&bridge_rlib)
                .arg(&bridge_src);
            if let Some(ref lib_dir) = loft_wasm_lib_dir {
                build
                    .arg("--extern")
                    .arg(format!("loft={}", lib_dir.join("libloft.rlib").display()));
                let deps = lib_dir.join("deps");
                if deps.is_dir() {
                    build
                        .arg("-L")
                        .arg(format!("dependency={}", deps.display()));
                }
                if let Some(host_lib_dir) = loft_lib_dir_for(None) {
                    let host_deps = deps_dir_of(&host_lib_dir);
                    if host_deps.exists() {
                        build
                            .arg("-L")
                            .arg(format!("dependency={}", host_deps.display()));
                    }
                }
            }
            // Build-extension: the bridge crate's own Cargo deps (dalek/RustCrypto).
            // `--extern` each DIRECT dep (the `use <crate>` extern-prelude entry), then
            // `-L` the deps dir for their transitive deps.
            for (ident, rlib) in &bridge_externs {
                build
                    .arg("--extern")
                    .arg(format!("{ident}={}", rlib.display()));
            }
            if let Some(ref deps) = bridge_dep_search {
                build
                    .arg("-L")
                    .arg(format!("dependency={}", deps.display()));
            }
            let status = build.status();
            if !matches!(status, Ok(s) if s.success()) {
                eprintln!(
                    "loft: --html: failed to compile wasm bridge crate {} from {}",
                    bridge_crate,
                    bridge_src.display()
                );
                std::process::exit(1);
            }
            cmd.arg("--extern")
                .arg(format!("{crate_ident}={}", bridge_rlib.display()));
            // Build-extension: link the bridge crate's Cargo deps into the main wasm.
            if let Some(ref deps) = bridge_dep_search {
                cmd.arg("-L").arg(format!("dependency={}", deps.display()));
            }
        }
        let status = cmd.status();
        if std::env::var("LOFT_KEEP_NATIVE_RS").is_err() {
            let _ = std::fs::remove_file(&rs_path);
        } else {
            eprintln!(
                "loft: browser-wasm source preserved at {} (LOFT_KEEP_NATIVE_RS)",
                rs_path.display()
            );
        }
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
                // Asyncify suspend imports (comma-separated module.function):
                //   loft_gl.loft_gl_swap_buffers — the GL frame-yield (games).
                //   loft_web.ws_yield — the WebSocket frame-yield (@PLN84 ZT-C):
                //     a synchronous WS poll loop calls `web::yield_frame()` once
                //     per iteration; under --html that lowers to this import,
                //     which unwinds back to the JS event loop so
                //     `WebSocket.onmessage` can deliver, then resumes.  Naming
                //     it here even when no program yields on it is harmless (the
                //     import just isn't present in the wasm).
                "--pass-arg=asyncify-imports@loft_gl.loft_gl_swap_buffers,loft_web.ws_yield",
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
        // @P321(c) Phase 3a: auto-discover *.png siblings of the entry .loft
        // and embed each as a base64 string under `ctrl.assets[basename]`.
        // Phase 3b's JS preamble decodes them to RGB bytes before
        // `loft_start()` runs; the imaging bridge then looks up by basename.
        // Stays a no-op when no PNGs are adjacent (most --html programs).
        let assets_js = {
            let entry_dir = std::path::Path::new(&abs_file)
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let mut entries: Vec<(String, String)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&entry_dir) {
                let mut pngs: Vec<std::path::PathBuf> = rd
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
                    .collect();
                pngs.sort();
                for p in pngs {
                    let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    let Ok(bytes) = std::fs::read(&p) else {
                        continue;
                    };
                    entries.push((name.to_string(), crate::base64::encode(&bytes)));
                }
            }
            if entries.is_empty() {
                String::from("{}")
            } else {
                let mut s = String::from("{");
                for (i, (name, b64)) in entries.iter().enumerate() {
                    if i > 0 {
                        s.push(',');
                    }
                    // Asset names are filesystem basenames — restrict to
                    // safe chars; reject anything that could break out of
                    // the JS string literal.  PNG suffix already required.
                    let safe = name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
                    if !safe {
                        continue;
                    }
                    s.push_str(&format!("\"{name}\":\"{b64}\""));
                }
                s.push('}');
                s
            }
        };
        let gl_js = include_str!("../doc/loft-gl-wasm.js");
        // @lib_plan-29 W2: concatenate every used library's
        // `[wasm.bridge].host_js` file into the HTML preamble.  Each
        // file pushes a registration callback onto
        // `globalThis.LOFT_WASM_EXTENSIONS`; the dispatch loop below
        // applies each callback to the imports object after
        // `buildLoftImports` returns, so library-specific JS handlers
        // (e.g. `imaging_query`) become part of the wasm imports
        // without the compiler/tooling crate naming them.
        let host_js_extensions = {
            let mut s = String::new();
            for path in &p.data.wasm_bridge_host_js_files {
                match std::fs::read_to_string(path) {
                    Ok(content) => {
                        s.push_str("\n/* === lib_plan-29 W2: host.js from ");
                        s.push_str(path);
                        s.push_str(" === */\n");
                        s.push_str(&content);
                    }
                    Err(e) => {
                        eprintln!(
                            "loft: --html: cannot read [wasm.bridge].host_js file '{path}': {e}"
                        );
                        std::process::exit(1);
                    }
                }
            }
            s
        };
        // Pick the page shell by what the wasm actually imports — minimal by
        // default, bigger only when the program uses it.  A wasm that imports
        // only `loft_io` is pure text I/O, so it ships the tiny engine-less
        // shim (no WebGL2, no asyncify, no canvas).  `loft_gl` (graphics/audio)
        // or a `loft_<lib>` bridge means the program opted into the full engine
        // page.  Unparsable → full page (the shell that satisfies every import).
        let minimal_page = crate::native_utils::html_wasm_import_modules(&wasm_bytes)
            .is_some_and(|mods| mods.iter().all(|m| m == "loft_io"));
        let html = if minimal_page {
            format!(
                r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{title}</title>
<style>body{{margin:0;font:14px/1.5 monospace;background:#111;color:#0f0}}pre{{margin:0;padding:1rem;white-space:pre-wrap;word-break:break-word}}</style>
</head><body><pre id="out"></pre>
<script>
// Minimal engine-less loft page: a small wasm + this tiny shim.  No WebGL2, no
// asyncify, no canvas — only `loft_io` (text out).  JS owns the page; loft is a
// callable module (loft_start builds fresh Stores each call, so JS can invoke
// it per request).  A program that uses graphics/audio/a frame loop gets the
// full engine page instead.
const wasmB64="{wasm_b64}";
const wasmBytes=Uint8Array.from(atob(wasmB64),c=>c.charCodeAt(0));
const out=document.getElementById('out');
const dec=new TextDecoder();
let mem;
// JS -> loft input is a QUEUE: seed it with globalThis.loftInput (a string)
// before this runs, push live messages any time with globalThis.loftPush(msg)
// (e.g. fetch() completions) — each host_input() call pops one message
// (len+copy pairs; the copy pops).  loft -> JS structured messages arrive at
// globalThis.loftOutput(msg) (host_output(); default: console.log) — the
// request/response pattern: loft host_output's a request, JS acts on it and
// loftPush'es the completion.
const enc=new TextEncoder();
const inQ=[];
if(globalThis.loftInput!=null)inQ.push(enc.encode(String(globalThis.loftInput)));
globalThis.loftPush=(m)=>{{inQ.push(enc.encode(String(m)));}};
const imports={{loft_io:{{
  loft_host_print:(ptr,len)=>{{out.textContent+=dec.decode(new Uint8Array(mem.buffer,ptr,len));}},
  loft_host_input_len:()=>inQ.length?inQ[0].length:0,
  loft_host_input_copy:(ptr)=>{{const b=inQ.shift();if(b)new Uint8Array(mem.buffer,ptr,b.length).set(b);}},
  loft_host_output:(ptr,len)=>{{const m=dec.decode(new Uint8Array(mem.buffer,ptr,len));
    if(globalThis.loftOutput)globalThis.loftOutput(m);else console.log("[loft:out]",m);}}
}}}};
WebAssembly.instantiate(wasmBytes,imports).then(r=>{{
  mem=r.instance.exports.memory;
  r.instance.exports.loft_start();
}}).catch(e=>{{out.textContent+="\n[loft] "+e;}});
</script></body></html>"#
            )
        } else {
            format!(
                r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{title}</title>
<style>body{{margin:0;background:#000;display:flex;justify-content:center;align-items:center;height:100vh}}canvas{{display:block}}pre{{color:#0f0;font-size:14px}}</style>
</head><body>
<canvas id="c" tabindex="0" style="display:none"></canvas>
<pre id="out"></pre>
<script>
{gl_js}
{host_js_extensions}
const wasmB64="{wasm_b64}";
const wasmBytes=Uint8Array.from(atob(wasmB64),c=>c.charCodeAt(0));
const canvas=document.getElementById('c');
const output=document.getElementById('out');
let mem;
// @P321(c) Phase 3a: raw base64 of *.png siblings of the entry .loft
// (auto-discovered).  Phase 3b decodes each to RGB bytes and replaces
// the slot with {{width, height, bytes}} before loft_start runs.
const ctrl={{ac:null,assets:{assets_js}}};
const imports=buildLoftImports(canvas,output,()=>mem,ctrl);
// @lib_plan-29 W2: apply each library's host.js-registered extension
// to the imports object (mutates `imports.loft_gl` in place).
for(const reg of (globalThis.LOFT_WASM_EXTENSIONS||[])){{
  try{{reg(imports,ctrl,()=>mem);}}catch(e){{console.error('loft host_js extension failed',e);}}
}}
WebAssembly.instantiate(wasmBytes,imports).then(async r=>{{
  mem=r.instance.exports.memory;
  // @P321(c) Phase 3b: decode base64 PNG assets to RGB bytes before
  // loft_start so the wasm-side imaging bridge looks them up sync.
  ctrl.assets=await decodeLoftAssets(ctrl.assets);
  if(r.instance.exports.asyncify_start_unwind){{
    const ac=new AsyncifyCtrl(r.instance);
    ctrl.ac=ac;
    ac.start('loft_start');
    if(ac.sleeping){{
      // Drive the asyncify resume loop.  A HIDDEN page (headless capture, a
      // backgrounded tab) throttles or fully pauses requestAnimationFrame, so
      // an rAF-only loop stalls at the first suspend (issue #450).  Pump via an
      // unthrottled MessageChannel while the page is hidden, and via rAF while
      // visible so a GL render loop stays vsync-aligned.  schedule() re-checks
      // visibility each tick, so a tab going hidden/visible adapts live.
      const mc=new MessageChannel();
      const pump=()=>{{ if(ac.resume('loft_start'))schedule(); }};
      mc.port1.onmessage=pump;
      const schedule=()=>{{
        if(document.hidden)mc.port2.postMessage(0);
        else requestAnimationFrame(pump);
      }};
      schedule();
    }}
  }}else{{
    r.instance.exports.loft_start();
  }}
}});
</script></body></html>"#
            )
        };
        if let Err(e) = std::fs::write(&html_path, &html) {
            eprintln!("loft: cannot write HTML to '{html_path}': {e}");
            std::process::exit(1);
        }
        let wasm_kb = wasm_bytes.len() / 1024;
        let html_kb = html.len() / 1024;
        let shell = if minimal_page {
            " · minimal engine-less shell"
        } else {
            " · full engine shell"
        };
        println!("wrote {html_path} ({html_kb} KB, WASM {wasm_kb} KB{shell})");
        return;
    }

    // Native codegen pipeline: --native or --native-emit.
    //
    // rustc availability is checked lazily, in the cache-miss compile branch
    // below (the NotFound arm of `cmd.output()`) — NOT up front.  A warm cache
    // hit runs the cached binary and never needs rustc, so an up-front
    // `rustc --version` probe would tax every run with a ~18 ms process spawn
    // for nothing.  The `'native` label lets the lazy check fall back to the
    // interpreter when rustc is genuinely absent on a cache miss.
    //
    // Each fallback `break 'native` below records WHY in `native_fallback_reason`
    // so the chokepoint past the block can report it (warning by default, hard
    // error under `LOFT_REQUIRE_NATIVE`).  `None` after the block means native
    // either ran (and `return`ed) or was never requested.
    let mut native_fallback_reason: Option<String> = None;
    'native: {
        if !(native_mode || native_emit.is_some()) {
            break 'native;
        }
        let end_def = p.data.definitions();
        let emit_path = match native_emit.as_deref() {
            // Default `loft <file>` writes to a per-process tmp file
            // so concurrent invocations (e.g. nextest's parallel test
            // execution) don't race on a single shared
            // `temp_dir/loft_native.rs`.  The PID suffix is a process-
            // local choice; user-visible artefacts pass through
            // `--native-emit <path>` which the user controls.
            None => platform::scratch_dir().join(format!("loft_native_{}.rs", std::process::id())),
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
            let mut out = generation::Output::new(&p.data, &state.database);
            // Host-native backend: link each `#native` package's cdylib by C-ABI
            // (`extern "C"` decls + `.so`), not its rlib — see NATIVE.md
            // § Resolution: separate the API id from the Rust part.  The shared
            // `native_cabi_enabled()` keeps codegen and the linker flags in sync
            // (off on Windows, which stays on the rlib path).
            out.native_cabi = native_utils::native_cabi_enabled();
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
                    if !matches!(def.def_type, loft::data::DefType::Function) {
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
            // Up-front toolchain check (cache miss ⇒ about to compile).  A rustc
            // that differs from the one this loft + its rlib were built with
            // (the LOFT_BUILD_RUSTC stamp from build.rs) can't link the SVH-locked
            // rlib — the post-`rustup update` case.  For a DEFAULT native run,
            // detect it here and fall back to the interpreter WITHOUT the doomed
            // compile.  Cheap: one `rustc --version`, only on a cache miss (warm
            // hits ran the cached binary above) and only for the default path
            // (explicit `--native` proceeds and errors with the rebuild
            // diagnostic).  The lazy post-compile fallback still backstops
            // anything missed here (e.g. a matching rustc but a missing rlib).
            if let Some(reason) = loft::cache::rustc_mismatch() {
                // rustc changed since this loft was built, so the cached runtime
                // rlib is SVH-locked to the old rustc and can't be reused.  In a
                // source checkout, self-heal: rebuild the runtime with the user's
                // rustc and carry on natively — the fresh rlib is picked up by the
                // `loft_lib_dir()` resolution below.  From a bundle there is no
                // source to rebuild, so fall back (default) or error (`--native`).
                let healed = native_utils::loft_source_tree()
                    .is_some_and(|tree| native_utils::rebuild_runtime(&tree, reason));
                if !healed && !native_requested {
                    eprintln!(
                        "Warning: native compilation unavailable ({reason}); falling \
                         back to the interpreter. To restore native, rebuild from source \
                         (`cargo build --release`) — a downloaded release ships no native \
                         runtime and always runs interpreted."
                    );
                    native_fallback_reason = Some(format!(
                        "native compilation unavailable ({reason}); rebuild loft"
                    ));
                    let _ = std::fs::remove_file(&emit_path);
                    break 'native;
                }
                // healed → fresh rlib in place, fall through to the compile.
                // !healed && native_requested → fall through; the compile errors
                // below with the actionable `--native` message.
            }
            // Per-process tmp path — same rationale as the emit_path
            // above: avoids races between concurrent `loft <file>`
            // invocations.  The cached path (`cached_binary` above)
            // is content-addressed (source_hash) and thus safe to
            // share across processes; this fallback is only used
            // when the cache miss; it doesn't need to be content-
            // addressed but does need to be unique per process.
            let scratch = platform::scratch_dir();
            // Layer 2: refuse to start a compile that could overflow a
            // RAM-backed tmpfs and exhaust memory (reclaims loft's own stale
            // artefacts first).  Warn + continue rather than hard-fail; rustc
            // will surface a real ENOSPC if it genuinely can't write.
            if !platform::native_compile_space_ok(&scratch) {
                eprintln!(
                    "loft: warning — low space in {} (set LOFT_TMPFS_MIN_FREE_MB to tune)",
                    scratch.display()
                );
            }
            let binary = scratch.join(format!("loft_native_bin_{}", std::process::id()));
            let mut cmd = std::process::Command::new("rustc");
            cmd.env("TMPDIR", &scratch)
                .arg("--edition=2024")
                .arg("-o")
                .arg(&binary)
                .arg(&emit_path);
            if native_release {
                cmd.arg("-O");
            }
            // Layer 1: strip the linked binary (~36MB → ~1MB; the bulk is
            // debug info from libloft.rlib + std).  Skipped when the user
            // asked for debug info (--native-debug) or set
            // LOFT_NATIVE_KEEP_SYMBOLS=1.
            if !native_debug && platform::native_strip_symbols() {
                cmd.arg("-Cstrip=symbols");
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
                let deps = deps_dir_of(&lib_dir);
                cmd.arg("-L").arg(format!("dependency={}", deps.display()));
                // Pick the `loft_ffi` that `libloft` was built against, NOT the first
                // in dir order: with two copies in `deps/`, naming the wrong one puts
                // a second `loft_ffi` in the link → "colliding StableCrateId" (see
                // `native_lib::loft_ffi_for_libloft`).
                if let Some(ffi) =
                    loft::native_lib::loft_ffi_for_libloft(&lib_dir.join("libloft.rlib"), &deps)
                {
                    cmd.arg("--extern")
                        .arg(format!("loft_ffi={}", ffi.display()));
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
                    // No rustc on a cache miss → fall back to the interpreter
                    // rather than failing.  This is the moment the old up-front
                    // probe used to fire; doing it here means a warm cache hit
                    // never pays for a `rustc --version` spawn.
                    eprintln!("Warning: rustc not found, falling back to interpreter mode");
                    native_fallback_reason = Some("rustc not found".to_string());
                    let _ = std::fs::remove_file(&emit_path);
                    break 'native;
                }
                Err(e) => {
                    eprintln!("Warning: rustc check failed ({e}), falling back to interpreter");
                    native_fallback_reason = Some(format!("rustc could not be launched ({e})"));
                    let _ = std::fs::remove_file(&emit_path);
                    break 'native;
                }
            };
            let status = output.status;
            let stderr_utf8 = String::from_utf8_lossy(&output.stderr);
            // Classify a compile failure caused by the native TOOLCHAIN/cache, not
            // by loft codegen: a stale cached rlib after a `rustc`/`rustup update`
            // (E0514 "compiled by an incompatible version", E0460 "possibly newer
            // version" — the common case, rlibs are SVH-locked to one rustc), the
            // rand_core/cargo-cache staleness, an unresolvable loft/library crate
            // (E0463 — e.g. a distributed bundle ships no rlib), or an rmeta-without-
            // rlib dep (@P229 G3, an unbuilt package).
            let crate_resolution_failure = (stderr_utf8.contains("E0460")
                || stderr_utf8.contains("E0463")
                || stderr_utf8.contains("E0514"))
                && (stderr_utf8.contains("rand_core")
                    || stderr_utf8.contains("possibly newer version of crate")
                    || stderr_utf8.contains("compiled by an incompatible version")
                    || stderr_utf8.contains("can't find crate"));
            let rlib_format_failure =
                stderr_utf8.contains("required to be available in rlib format");
            // Turnkey fallback: a DEFAULT-native run (not an explicit `--native`)
            // that fails ONLY because the native toolchain isn't usable here —
            // loft's cached rlib is stale after a rustc update, or absent in a
            // distributed bundle — degrades to the interpreter so the program still
            // runs.  Keyed on the toolchain/crate failures above, never on arbitrary
            // compile errors, so a genuine codegen bug still surfaces loudly.
            // `--native` stays a hard error (explicit request needs the toolchain
            // set up).  The rustc-not-found (NotFound) arm above is the sibling case.
            if !status.success()
                && !native_requested
                && (crate_resolution_failure || rlib_format_failure)
            {
                eprintln!(
                    "Warning: native toolchain not usable here (cached build stale \
                     after a rustc update, or loft's runtime library unavailable); \
                     falling back to the interpreter. To restore native, rebuild from \
                     source (`cargo build --release`) — a downloaded release ships no \
                     native runtime and always runs interpreted."
                );
                native_fallback_reason = Some(
                    "native toolchain not usable here (cached build stale after a rustc \
                     update, or loft's runtime library unavailable); rebuild loft"
                        .to_string(),
                );
                let _ = std::fs::remove_file(&emit_path);
                break 'native;
            }
            // Relay rustc's own output to the user.
            let _ = std::io::Write::write_all(&mut std::io::stderr(), &output.stderr);
            let _ = std::io::Write::write_all(&mut std::io::stdout(), &output.stdout);
            if !status.success() {
                if crate_resolution_failure || rlib_format_failure {
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
                        "\n--native needs loft's runtime rlib, built with your rustc.\n\n\
                         In a source checkout:  cargo build --release --lib --bin loft\n\
                         (prefix `cargo clean &&` after a rustc update).\n\
                         A downloaded release ships no native runtime — drop `--native` \
                         to run on the interpreter, or build loft from source.\n"
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
        // PLAN51 — also preserve when `LOFT_KEEP_NATIVE_RS=1` is set,
        // so probe runs that panic in the generated Rust leave the file
        // readable for post-mortem inspection.  Used by the Cluster V
        // (native-only) investigation in plans/finished/51-hidden-buffer-
        // aliasing/cluster-V-native-only.md.
        let keep_rs = std::env::var("LOFT_KEEP_NATIVE_RS").is_ok();
        if !native_debug && !keep_rs {
            let _ = std::fs::remove_file(&emit_path);
        } else {
            let reason = if native_debug {
                "--native-debug"
            } else {
                "LOFT_KEEP_NATIVE_RS"
            };
            eprintln!(
                "loft: source preserved at {} ({reason})",
                emit_path.display()
            );
        }

        if check_only {
            // --check --native: compile succeeded, report ok and exit.
            // @PLN18 08-S4 — the artifact path rides the ok line so the
            // background-rebuild host (live_dispatch) can find the build
            // without re-deriving the cache key (which hashes the GENERATED
            // source + rlib mtimes — only this pipeline can compute it).
            // Prefer the DURABLE content-addressed cache path over the
            // per-pid temp the miss branch built into (the consumer is the
            // S5 swap; a temp path is clobbered by the next same-pid run).
            let artifact = if !no_cache && cached_binary.exists() {
                &cached_binary
            } else {
                &binary
            };
            println!("ok {abs_file} {}", artifact.display());
            return;
        }
        // @PLN26 phase 4 — Windows has no RPATH, so a C-ABI-linked native-package
        // DLL must sit beside the binary that loads it; stage it there before the
        // spawn (no-op off Windows / on the rlib path).
        if let Some(dir) = binary.parent() {
            native_utils::stage_native_dlls(dir, &p.data);
        }
        // @PLN18 08-S2 — live-dispatch handoff: the spawned binary's bootstrap
        // re-parses the same sources, so hand it the resolved paths the driver
        // already knows.  Inert unless the binary runs under LOFT_LIVE_FLIP=1;
        // explicit user-set values win.
        let mut cmd = std::process::Command::new(&binary);
        cmd.args(&user_args);
        // @PLN26 follow-up — run the native binary with cwd = source_dir so its
        // raw `std::fs` anchors where its loft `file()` does (the binary bakes
        // `program_relative` + reads source_dir from LOFT_SOURCE_DIR).  Mirrors
        // the interpreter chdir above; gated on the same `program_relative`.
        if state.database.program_relative
            && let Some(dir) = std::path::Path::new(&abs_file).parent()
        {
            cmd.current_dir(dir);
        }
        // The artifact anchors relative paths at its OWN dir (the
        // standalone-bundle rule) — in driver mode that is the cache/tmp
        // dir, not the program's.  Hand the source anchor down so file I/O
        // matches the interpreter; an explicit user value wins.
        if std::env::var("LOFT_SOURCE_DIR").is_err()
            && let Some(dir) = std::path::Path::new(&abs_file).parent()
        {
            cmd.env("LOFT_SOURCE_DIR", dir);
        }
        if std::env::var("LOFT_LIVE_SRC").is_err() {
            cmd.env("LOFT_LIVE_SRC", &abs_file);
        }
        if std::env::var("LOFT_LIVE_STDLIB").is_err() {
            cmd.env("LOFT_LIVE_STDLIB", &default_str);
        }
        if std::env::var("LOFT_LIVE_LIBS").is_err() && !p.lib_dirs.is_empty() {
            cmd.env("LOFT_LIVE_LIBS", p.lib_dirs.join(":"));
        }
        // @PLN18 08-S4 — the background rebuild re-invokes THIS driver.
        if std::env::var("LOFT_LIVE_DRIVER").is_err()
            && let Ok(me) = std::env::current_exe()
        {
            cmd.env("LOFT_LIVE_DRIVER", me);
        }
        let run_status = cmd.status().unwrap_or_else(|e| {
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

    // Reached only by the `'native` fallback above (rustc absent on a cache
    // miss).  A `--check --native` run wants a status, not execution: report
    // ok on a clean parse — the same answer the interpret-mode check gives —
    // rather than falling through and running the program.
    if check_only {
        println!("ok {abs_file}");
        return;
    }

    // Crawler / efficiency aid (`LOFT_REQUIRE_NATIVE`): reaching here with native ON
    // means a fallback above broke out of `'native` — native success `return`s at the
    // end of that block, and `--interpret`/`--bytecode`/the test paths set
    // `native_mode = false`, so `native_mode` still true here is exactly "native was
    // wanted but we're about to interpret".  Under the env var that is a hard error
    // (the per-site warning already named the reason for the default warn path).  The
    // `unwrap_or` is a catch-all so a future fallback that forgets to record a reason
    // still errors loudly rather than degrading silently.
    if native_required && native_mode {
        let reason = native_fallback_reason
            .as_deref()
            .unwrap_or("native execution did not occur (reason not recorded)");
        eprintln!(
            "loft: LOFT_REQUIRE_NATIVE is set, but native execution was unavailable \
             ({reason}); refusing to fall back to the interpreter. \
             Fix the toolchain (e.g. `cargo build --release --lib --bin loft`), \
             or unset LOFT_REQUIRE_NATIVE to allow interpreter fallback."
        );
        std::process::exit(1);
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
        let opts = loft::introspect::Options {
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
        if let Err(e) = loft::introspect::emit_all(&mut p.data, &mut state, end_def, &opts) {
            eprintln!("loft: introspect failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    // @PLN26 follow-up — anchor native file I/O at source_dir: chdir so a native
    // crate's raw `std::fs` resolves a relative path the SAME way loft's
    // `resolve_path` (which joins source_dir) does.  Gated on `program_relative`
    // so a `#cwd` program keeps both at the cwd.  Done here — after parse +
    // native-lib resolution (those ran against the invocation cwd), before user
    // execution; no restore needed (the process exits after the run).  The
    // --native path uses `Command::current_dir` on the spawn instead.
    if state.database.program_relative && !state.database.source_dir.is_empty() {
        let _ = std::env::set_current_dir(&state.database.source_dir);
    }
    if main_nr == u32::MAX && !dump_only {
        // No main() — execute each zero-parameter user `test_*()` function
        // INDIVIDUALLY with a fresh `state.database` per call.  This
        // mirrors `src/test_runner.rs`'s per-fn isolation (line 997's
        // `clean_data.clone() + State::new(clean_db.clone())` pattern) and
        // is what prevents shared-State accumulation of leaked stores
        // from one test compounding into a SIGSEGV in a later test.
        //
        // History: the prior wrapper-synthesis approach (a synthetic
        // `fn main() { test_a(); test_b(); ... }`) ran every test in the
        // SAME `State`, so per-call store-lifetime leaks (the @P377
        // family — `map_get_hex(m: Map, …) -> Hex { return chunk.ck_hexes[idx]; }`
        // shape, deep-slice-borrows the dep-inference can't track) accumulated
        // until `cast_vector_from_text` SIGSEGV'd inside the next JSON cast
        // (@P382).  That synthesis also tripped a CONST_STORE re-lock by
        // calling `compile::byte_code` a second time (@P381, fixed by
        // `compile::byte_code_from`).  Both issues vanish with per-test
        // fresh-State isolation, which also matches the canonical
        // `loft --tests <file>` invocation.
        //
        // `--dump` skips this path — it wants the bytecode of the user
        // functions as parsed, not a synthetic execution.
        let mut test_names: Vec<String> = Vec::new();
        for d_nr in start_def..p.data.definitions() {
            let def = p.data.def(d_nr);
            // The no-`main` fallback runs every zero-param void user function
            // (the #358 contract — see `tests/arc_e_program_cache.rs`).  The one
            // exclusion is `#native` host imports: a used LIBRARY's zero-param
            // host imports (`gl_swap_buffers`, `gl_destroy_window`, …) have no
            // loft body, so running one via `execute_argv` hit a `def(u32::MAX)`
            // "Unknown definition" panic.  `def.native.is_empty()` filters them
            // out — the name is NOT gated (a bare `check_me()` must still run).
            if def.name.starts_with("n_")
                && !def.name.starts_with("n___lambda_")
                && matches!(def.def_type, data::DefType::Function)
                && def.native.is_empty()
                && def.attributes.is_empty()
                && matches!(def.returned, data::Type::Void)
                && !def.position.file.starts_with("default/")
            {
                let name = def.name.strip_prefix("n_").unwrap_or(&def.name);
                test_names.push(name.to_string());
            }
        }
        if !test_names.is_empty() {
            // Per-test isolation pattern (mirrors `src/test_runner.rs:997`):
            // `Stores::clone` only clones the TYPE SCHEMA (allocations + runtime
            // state are reset to empty by design — see `src/database/mod.rs:412`),
            // so each test needs a full re-byte_code on its own freshly-cloned
            // Data + State.  Reset `max` on the clone to 0 because
            // `Stores::clone` preserves `max` from the source but clears
            // `free_bits` — `find_free_slot` would then return the stale `max`
            // value as the first allocation slot, and `State::new`'s initial
            // `db.database(1000)` would panic with `allocations[max]` OOB.
            let clean_data = p.data.clone();
            let clean_db = state.database.clone();
            for name in &test_names {
                let mut data_iter = clean_data.clone();
                let mut db_iter = clean_db.clone();
                db_iter.max = 0;
                let mut state_iter = State::new(db_iter);
                compile::byte_code(&mut state_iter, &mut data_iter);
                // Preserve native-extension wiring across test iterations.
                extensions::load_all(&mut state_iter, all_native_libs.clone());
                extensions::wire_native_fns(&mut state_iter, &data_iter);
                // #303 — the shared-store bridges too: a `loft_shared_*`-marked
                // fn dispatches via `OpStaticCall` in test bodies as well; without
                // this wire every such call hits the panicking stub.
                extensions::wire_shared_native_fns(&mut state_iter, &data_iter);
                state_iter.execute_argv(name, &data_iter, &[]);
            }
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
        // @PLN18 phase 02 — tier-0 live reload (opt-in): watch the program
        // file and hot-swap edited fns into the running State.  The shadow
        // session inherits the RESOLVED stdlib dir — a relative "default"
        // only exists when the cwd happens to be a loft checkout (#346).
        if std::env::var_os("LOFT_LIVE_RELOAD").is_some() {
            loft::live_reload::install(&abs_file, &default_str, &p.lib_dirs, &p.data);
        }
        state.execute_argv("main", &p.data, &user_args);
        // FY.3: native desktop frame loop — gl_swap_buffers sets frame_yield,
        // causing execute_argv to return. Resume until the program finishes.
        while state.database.frame_yield {
            state.resume();
        }
        // The program is over HERE in the frame-yield case — unwire the
        // parallel ctx at its program-scope owner (`resume` deliberately
        // does not: it also serves per-call re-entry, where the standing
        // ctx must survive).
        state.database.parallel_ctx = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    // The crypto bridge manifest verbatim (loft-libs-core crypto/wasm/Cargo.toml,
    // as published in crypto 0.3.3): one `loft` path dep + the dalek/RustCrypto stack.
    const CRYPTO_BRIDGE_CARGO: &str = "\
[package]
name = \"crypto-wasm\"
version = \"0.1.0\"
edition = \"2024\"

[lib]
crate-type = [\"rlib\"]

[dependencies]
loft = { path = \"../../../loft\", default-features = false, features = [\"random\"] }
ed25519-dalek = { version = \"2.1\", default-features = false, features = [\"std\", \"fast\"] }
x25519-dalek  = { version = \"2.0\", default-features = false, features = [\"static_secrets\"] }
aes-gcm       = { version = \"0.10\", default-features = false, features = [\"aes\", \"alloc\"] }
";

    #[test]
    fn bridge_nonloft_deps_excludes_loft_and_keeps_the_rest() {
        let deps = bridge_nonloft_deps(CRYPTO_BRIDGE_CARGO);
        let idents: Vec<&str> = deps.iter().map(|(i, _)| i.as_str()).collect();
        // #446: `loft` must NOT appear — it is the redundant path dep that fails
        // to resolve for a registry-installed package.
        assert!(
            !idents.contains(&"loft"),
            "loft must be excluded, got {idents:?}"
        );
        // The hyphenated crate names are normalised to rlib idents.
        assert_eq!(idents, ["ed25519_dalek", "x25519_dalek", "aes_gcm"]);
        // The full dependency line is preserved verbatim (versions + features).
        assert!(deps[0].1.contains("version = \"2.1\""));
        assert!(deps[2].1.contains("features = [\"aes\", \"alloc\"]"));
    }

    #[test]
    fn synth_manifest_never_contains_loft_path_dep() {
        let deps = bridge_nonloft_deps(CRYPTO_BRIDGE_CARGO);
        let manifest = synth_bridge_deps_manifest(&deps);
        // #446 invariant: the synthesized deps-only manifest carries the non-loft
        // deps but NO `loft` line, so cargo never resolves `../../../loft`.
        assert!(
            !manifest.contains("loft = {"),
            "synth manifest leaked the loft dep:\n{manifest}"
        );
        assert!(!manifest.contains("../../../loft"));
        assert!(manifest.contains("ed25519-dalek = { version = \"2.1\""));
        assert!(manifest.contains("[lib]\ncrate-type = [\"rlib\"]"));
    }

    #[test]
    fn bridge_with_no_extra_deps_yields_empty() {
        // A bridge whose only dep is `loft` (e.g. the web WS bridge) must produce
        // zero non-loft deps, so the driver skips the build-extension entirely.
        let web = "[package]\nname = \"web-wasm\"\n\n[dependencies]\n\
                   loft = { path = \"../../../loft\" }\n";
        assert!(bridge_nonloft_deps(web).is_empty());
    }
}
