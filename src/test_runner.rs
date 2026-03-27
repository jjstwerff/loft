// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Test runner: discover and run callable functions in `.loft` files.

#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(unused_imports)] // Module used from main(), not from test builds.

use crate::compile;
use crate::data::{Data, DefType, Type};
use crate::generation;
use crate::log_config::LogConfig;
use crate::logger;
use crate::native_utils;
use crate::parser::Parser;
use crate::scopes;
use crate::state::State;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Run all zero-parameter functions in `.loft` files under `root_dir` as tests.
/// Supports `@ARGS`, `@EXPECT_ERROR`, and `@EXPECT_FAIL` file annotations.
/// Returns 0 if all pass, 1 if any fail.
#[allow(clippy::too_many_lines)]
pub(crate) fn run_tests(
    default_dir: &str,
    root_dir: &str,
    no_warnings: bool,
    lib_dirs: &[String],
    project: Option<&str>,
    native_mode: bool,
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

    // Parse optional function filter: "file.loft::name" or "file.loft::{a,b}".
    let (path_part, fn_filter): (&str, Option<Vec<String>>) = if let Some(pos) = root_dir.find("::")
    {
        let raw = &root_dir[pos + 2..];
        let names: Vec<String> = if raw.starts_with('{') && raw.ends_with('}') {
            raw[1..raw.len() - 1]
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        } else {
            vec![raw.to_string()]
        };
        (&root_dir[..pos], Some(names))
    } else {
        (root_dir, None)
    };

    let root = std::path::Path::new(path_part);
    let mut dirs: BTreeMap<String, Vec<std::path::PathBuf>> = BTreeMap::new();
    if root.is_file() {
        // Single file mode: run tests in just this file.
        let dir_key = root
            .parent()
            .map_or(".".to_string(), |p| p.to_string_lossy().to_string());
        dirs.insert(dir_key, vec![root.to_path_buf()]);
    } else if root.is_dir() {
        collect_loft_files(root, &mut dirs);
    } else {
        std::panic::set_hook(prev_hook);
        println!("loft: --tests path '{path_part}' does not exist");
        return 1;
    }

    if dirs.is_empty() {
        std::panic::set_hook(prev_hook);
        println!("loft: no .loft files found in '{path_part}'");
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

    // In native mode, ensure libloft.rlib exists and is up to date.
    // `cargo run --bin loft` rebuilds the binary but may skip the library
    // target, leaving native tests linking against stale code.
    // Detect this by comparing source mtimes against the rlib and rebuild
    // automatically when needed.
    if native_mode {
        native_utils::ensure_rlib_fresh();
        if native_utils::loft_lib_dir().is_none() {
            std::panic::set_hook(prev_hook);
            println!(
                "loft: --native requires libloft.rlib; \
                 run `cargo build --lib` first"
            );
            return 1;
        }
    }

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
            let mut p = Parser::new();
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

            // Apply function name filter (from "file.loft::name" syntax).
            if let Some(ref filter) = fn_filter {
                test_fns.retain(|(_, name)| filter.iter().any(|f| name == f));
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

            if native_mode {
                // ── Native mode: generate Rust, compile, run ──────────────
                // Native codegen requires byte_code compilation first.
                let mut native_data = clean_data.clone();
                let mut native_state = State::new(clean_db.clone());
                compile::byte_code(&mut native_state, &mut native_data);
                let native_db = native_state.database;
                // Filter to functions that can run natively (skip @IGNORE,
                // @EXPECT_ERROR, and @EXPECT_FAIL — native can't catch panics).
                let mut native_fns: Vec<(u32, String)> = Vec::new();
                for (d_nr, fn_name) in &test_fns {
                    if ann.ignore_fn.contains(fn_name.as_str()) {
                        file_result.tests.push((
                            fn_name.clone(),
                            true,
                            Some("ignored".to_string()),
                        ));
                        continue;
                    }
                    if ann.expect_errors_fn.contains_key(fn_name.as_str()) {
                        continue;
                    }
                    let should_fail = ann.expect_fail_fn.contains_key(fn_name.as_str())
                        || !ann.expect_fail_file.is_empty();
                    if should_fail {
                        file_result.tests.push((
                            fn_name.clone(),
                            true,
                            Some("skip-native".to_string()),
                        ));
                        continue;
                    }
                    native_fns.push((*d_nr, fn_name.clone()));
                }
                if native_fns.is_empty() {
                    // Nothing to run natively — record as pass with note.
                    if file_result.tests.is_empty() {
                        file_result
                            .tests
                            .push(("(no native tests)".to_string(), true, None));
                    }
                } else {
                    // Generate Rust source.
                    let end_def = native_data.definitions();
                    let main_nr = native_data.def_nr("n_main");
                    let has_main = main_nr < end_def && native_data.def(main_nr).name == "n_main";
                    let entry_defs: Vec<u32> = if has_main {
                        vec![main_nr]
                    } else {
                        native_fns.iter().map(|(d, _)| *d).collect()
                    };
                    let gen_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut buf: Vec<u8> = Vec::new();
                        let mut out = generation::Output {
                            data: &native_data,
                            stores: &native_db,
                            counter: 0,
                            indent: 0,
                            def_nr: 0,
                            declared: HashSet::new(),
                            reachable: HashSet::new(),
                            loop_stack: Vec::new(),
                            next_format_count: 0,
                        };
                        out.output_native_reachable(&mut buf, start_def, end_def, &entry_defs)
                            .expect("native codegen write");
                        // output_native_reachable emits fn main() when n_main
                        // exists.  For test-only files (no n_main) we generate
                        // a main() that calls each test function.
                        if !has_main {
                            use std::io::Write;
                            writeln!(buf, "\nfn main() {{").unwrap();
                            writeln!(buf, "    let mut stores = Stores::new();").unwrap();
                            writeln!(buf, "    init(&mut stores);").unwrap();
                            for (_, name) in &native_fns {
                                writeln!(buf, "    n_{name}(&mut stores);").unwrap();
                            }
                            writeln!(buf, "}}").unwrap();
                        }
                        buf
                    }));
                    let buf = match gen_result {
                        Ok(b) => b,
                        Err(payload) => {
                            let msg = panic_message(&*payload);
                            for (_, fn_name) in &native_fns {
                                file_result.tests.push((
                                    fn_name.clone(),
                                    false,
                                    Some(format!("native codegen panic: {msg}")),
                                ));
                                dir_fail += 1;
                            }
                            // Skip compile+run phases.
                            Vec::new()
                        }
                    };
                    if !buf.is_empty() {
                        let stem = std::path::Path::new(&abs_file)
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .replace('-', "_");
                        let tmp_rs =
                            std::env::temp_dir().join(format!("loft_test_native_{stem}.rs"));
                        let binary =
                            std::env::temp_dir().join(format!("loft_test_native_{stem}_bin"));
                        let key_file =
                            std::env::temp_dir().join(format!("loft_test_native_{stem}_bin.key"));

                        // Write .rs only when content changed (preserves cache).
                        let existing = std::fs::read(&tmp_rs).unwrap_or_default();
                        if existing != buf {
                            let _ = std::fs::write(&tmp_rs, &buf);
                        }

                        // Check binary cache before compiling.
                        let lib_dir = native_utils::loft_lib_dir();
                        let cached = binary.exists()
                            && std::fs::read_to_string(&key_file).is_ok_and(|stored| {
                                stored.trim()
                                    == format!(
                                        "{:016x}",
                                        native_utils::native_cache_key(&buf, lib_dir.as_deref())
                                    )
                            });

                        let compile_ok = if cached {
                            true
                        } else {
                            // Compile with rustc.
                            let mut cmd = std::process::Command::new("rustc");
                            cmd.arg("--edition=2024")
                                .arg("-C")
                                .arg("debuginfo=0")
                                .arg("-C")
                                .arg("opt-level=0")
                                .arg("-o")
                                .arg(&binary)
                                .arg(&tmp_rs);
                            if let Some(ref ld) = lib_dir {
                                cmd.arg("--extern")
                                    .arg(format!("loft={}", ld.join("libloft.rlib").display()));
                                cmd.arg("-L").arg(ld.join("deps"));
                            }
                            let compile_result = cmd.output();
                            let ok = compile_result
                                .as_ref()
                                .map(|o| o.status.success())
                                .unwrap_or(false);
                            if ok {
                                // Write cache key sidecar.
                                let key = native_utils::native_cache_key(&buf, lib_dir.as_deref());
                                let _ = std::fs::write(&key_file, format!("{key:016x}"));
                            } else {
                                let stderr_msg = compile_result.as_ref().ok().map_or_else(
                                    || "rustc not found".to_string(),
                                    |o| {
                                        String::from_utf8_lossy(&o.stderr)
                                            .lines()
                                            .find(|l| l.starts_with("error"))
                                            .unwrap_or("(unknown)")
                                            .to_string()
                                    },
                                );
                                let _ = std::fs::remove_file(&binary);
                                let _ = std::fs::remove_file(&key_file);
                                for (_, fn_name) in &native_fns {
                                    file_result.tests.push((
                                        fn_name.clone(),
                                        false,
                                        Some(format!("native compile: {stderr_msg}")),
                                    ));
                                    dir_fail += 1;
                                }
                            }
                            ok
                        };

                        if compile_ok {
                            // Run the compiled binary.
                            let run_ok = std::process::Command::new(&binary)
                                .status()
                                .map(|s| s.success())
                                .unwrap_or(false);
                            if run_ok {
                                for (_, fn_name) in &native_fns {
                                    file_result.tests.push((fn_name.clone(), true, None));
                                    dir_pass += 1;
                                }
                            } else {
                                for (_, fn_name) in &native_fns {
                                    file_result.tests.push((
                                        fn_name.clone(),
                                        false,
                                        Some("native run failed".to_string()),
                                    ));
                                    dir_fail += 1;
                                }
                            }
                        }
                        // Keep .rs and binary on disk for caching.
                    }
                }
            } else {
                // ── Interpreter mode ──────────────────────────────────────────
                for (_, fn_name) in &test_fns {
                    // Per-function @IGNORE: skip without running.
                    if ann.ignore_fn.contains(fn_name.as_str()) {
                        file_result.tests.push((
                            fn_name.clone(),
                            true,
                            Some("ignored".to_string()),
                        ));
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
                                crate::logger::Logger::from_config_file(&cp, &abs_file)
                            } else {
                                crate::logger::Logger::production()
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
                                    Some(
                                        "expected panic but function returned cleanly".to_string(),
                                    ),
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
            } // end interpreter mode

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
