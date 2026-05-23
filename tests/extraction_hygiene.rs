// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-12 extraction-hygiene gate.
//!
//! The loft compiler crate (everything under `src/`) must contain
//! **zero** library code — only language core, runtime, codegen, and
//! stdlib symbols.  Libraries (`lib/crypto`, `lib/imaging`, `lib/web`,
//! `lib/random`, …) live in their own per-package native crates
//! (`lib/<X>/native/src/lib.rs`) and are loaded at runtime via the
//! package format.  See
//! [`doc/claude/lib_plans/12-library-extraction/README.md`](../doc/claude/lib_plans/12-library-extraction/README.md)
//! for the stdlib-vs-library boundary table.
//!
//! Two gates here:
//!
//! 1. **Symbol drain** — `forbidden_library_symbols_absent_from_src`:
//!    walks `src/**/*.rs`, fails if any known library `n_*` symbol
//!    name appears.  The list MUST stay in sync with what each
//!    library's `native/` crate exports.
//!
//! 2. **Dep drain** — `forbidden_library_deps_absent_from_main_cargo`:
//!    reads the main crate's `Cargo.toml`, fails if it lists
//!    library-only dependencies (`png`, `ureq`, `rustls`, `fontdue`)
//!    once those libraries' native crates are the sole owners.  This
//!    gate is **scoped**: currently it only enforces deps that have
//!    actually been drained.  Add a row when a library's drain
//!    completes; never add a row for an undrained library (the test
//!    would be wrong, not the code).
//!
//! These gates are CHEAP — pure file reads, no compilation.  Run on
//! every CI cycle.  When a future @P321-style attempt tries to add
//! `n_load_png` to `src/codegen_runtime.rs`, this test fails before
//! the PR can merge.

use std::fs;
use std::path::{Path, PathBuf};

/// Library `n_*` symbols that have been drained out of the compiler
/// crate by plan-12.  An occurrence of any of these in `src/**/*.rs`
/// is a regression — the symbol's native impl lives in
/// `lib/<owner>/native/src/lib.rs`.
///
/// Stdlib symbols (`n_panic`, `n_assert`, `n_log_*`, `n_json_*`,
/// `n_parallel_*`, `n_now`, `n_ticks`, …) are deliberately NOT in this
/// list — they stay in the compiler crate by design.  See the
/// stdlib-vs-library table in the plan README.
const FORBIDDEN_LIBRARY_SYMBOLS: &[(&str, &str)] = &[
    // ── Phase 1a (crypto, 2026-05-23) ─────────────────────────────
    ("n_sha256", "lib/crypto/native"),
    ("n_hmac_sha256", "lib/crypto/native"),
    ("n_hmac_sha256_raw", "lib/crypto/native"),
    ("n_base64_encode", "lib/crypto/native"),
    ("n_base64_decode", "lib/crypto/native"),
    ("n_base64url_encode", "lib/crypto/native"),
    // Add rows here as later phases drain more libraries:
    //   ("n_load_png",  "lib/imaging/native"),   // phase TBD
    //   ("n_save_png",  "lib/imaging/native"),   // phase TBD
    //   ("n_http_do",   "lib/web/native"),       // phase 1b
    //   ("n_http_body", "lib/web/native"),       // phase 1b
    //   ("n_rand",      "lib/random/native"),    // phase TBD
    //   ("n_rand_seed", "lib/random/native"),    // phase TBD
    //   ("n_rand_indices", "lib/random/native"), // phase TBD
];

/// Cargo dependencies that belong to a library's `native/` crate, not
/// the main loft crate.  Currently empty — the `png` / `ureq` / etc.
/// deps still live in `Cargo.toml` because Tier-B libraries
/// (graphics/imaging/random/server) are still linked as workspace
/// members and the cdylib loader hasn't shipped yet.  Add a row when
/// a dep moves out for real.
const FORBIDDEN_MAIN_CRATE_DEPS: &[&str] = &[
    // Add rows when a library's drain removes its dep from the main
    // crate.  Until then this gate is intentionally lenient — the
    // symbol gate above catches the more important regression
    // (compiler-crate Rust code referencing library symbols).
];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn forbidden_library_symbols_absent_from_src() {
    let root = workspace_root();
    let src_dir = root.join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);
    assert!(!files.is_empty(), "no .rs files found under src/");

    let mut violations: Vec<String> = Vec::new();
    for path in &files {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let rel = path
            .strip_prefix(&root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        // Walk by line so we can skip `//`-comment occurrences (a
        // valid description of the drain location can mention the
        // symbol name — see the trailing comment in `src/native.rs`'s
        // NATIVE_TABLE).  Only CODE occurrences are violations.
        for (line_idx, line) in content.lines().enumerate() {
            // Strip after `//`.  Doesn't handle `/* ... */` block
            // comments correctly, but those aren't used in this
            // codebase for symbol-name references.
            let code_part = line.split_once("//").map_or(line, |(c, _)| c);
            for (sym, owner) in FORBIDDEN_LIBRARY_SYMBOLS {
                for (idx, _) in code_part.match_indices(sym) {
                    let prev_ok = idx == 0
                        || !code_part.as_bytes()[idx - 1].is_ascii_alphanumeric()
                            && code_part.as_bytes()[idx - 1] != b'_';
                    let end = idx + sym.len();
                    let next_ok = end >= code_part.len()
                        || (!code_part.as_bytes()[end].is_ascii_alphanumeric()
                            && code_part.as_bytes()[end] != b'_');
                    if !prev_ok || !next_ok {
                        continue;
                    }
                    violations.push(format!(
                        "{rel}:{}: `{sym}` (belongs in {owner})",
                        line_idx + 1
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "\n@PLAN12 extraction-hygiene gate failed — \
         library `n_*` symbol(s) found in the compiler crate (src/):\n  {}\n\n\
         These symbols belong in their library's `native/src/lib.rs` (cdylib).  \
         If you intend to add a NEW library symbol, put it in the library's \
         native crate, not src/.  If a symbol is genuinely stdlib (every \
         loft program needs it), update FORBIDDEN_LIBRARY_SYMBOLS in this \
         test AND the stdlib-vs-library table in `doc/claude/lib_plans/\
         12-library-extraction/README.md`.\n",
        violations.join("\n  ")
    );
}

#[test]
fn forbidden_library_deps_absent_from_main_cargo() {
    let root = workspace_root();
    let cargo = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    // Match a top-level dep line like `png = "0.17"` or `png = {..}` —
    // anchored at start-of-line so feature-gated lines `dep:png` are
    // ignored (the `[features]` section is its own thing).
    let mut violations: Vec<String> = Vec::new();
    for dep in FORBIDDEN_MAIN_CRATE_DEPS {
        let needle_eq = format!("\n{dep} ");
        let needle_eq2 = format!("\n{dep}=");
        if cargo.contains(&needle_eq) || cargo.contains(&needle_eq2) {
            violations.push((*dep).to_string());
        }
    }
    assert!(
        violations.is_empty(),
        "@PLAN12 extraction-hygiene gate failed — main `Cargo.toml` lists \
         library-only dependencies that should live in `lib/<X>/native/Cargo.toml`: {:?}",
        violations
    );
}
