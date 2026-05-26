// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Build-script helper for loft native extension cdylibs.
//!
//! @PLAN12 phase 2 Option A (2026-05-24) — the two duplicated `build.rs`
//! files in `lib/crypto/native/` and `lib/web/native/` were collapsed
//! into this shared crate.  Each library's `build.rs` becomes:
//!
//! ```ignore
//! fn main() {
//!     loft_ffi_build::generate_register_invocation("../loft.toml");
//! }
//! ```
//!
//! …plus a single `[build-dependencies]` row in the library's
//! `native/Cargo.toml`:
//!
//! ```toml
//! [build-dependencies]
//! loft-ffi-build = { path = "../../../loft-ffi-build" }
//! ```
//!
//! Adding a new library is now:
//!
//! 1. `lib/<X>/loft.toml::[native.functions]` rows.
//! 2. `pub unsafe extern "C" fn n_*` bodies in
//!    `lib/<X>/native/src/lib.rs`.
//! 3. The two-line `build.rs` above + one Cargo.toml row.
//!
//! The `loft_register!` symbol list, the extraction-hygiene gate's
//! forbidden list, and the loft compiler's `def.native` /
//! `native_symbols` / `native_symbol_crates` populations are all
//! manifest-driven.

use std::env;
use std::fs;
use std::path::Path;

/// Scan the cdylib crate's own Rust sources for every
/// `extern "C" fn n_<name>` definition and emit a
/// `loft_ffi::loft_register! { … }` invocation listing them to
/// `$OUT_DIR/loft_register_gen.rs`.  The library's `src/lib.rs`
/// `include!`s the generated file at module scope.
///
/// This is the **drift-proof** registration path: the function
/// definitions ARE the source of truth, so adding a
/// `pub unsafe extern "C" fn n_foo(...)` body automatically registers
/// it — there is no hand-maintained symbol list and no manifest table
/// that can fall out of sync.  Supersedes
/// [`generate_register_invocation`] (the manifest-scanning variant).
///
/// `src_dir` is relative to the build script's working directory (the
/// crate root) — typically `"src"`.  Emits
/// `cargo:rerun-if-changed=<file>` for every scanned source so adding
/// or removing a function triggers regeneration.
///
/// # Panics
/// - If `OUT_DIR` is unset (Cargo always sets it for build scripts).
/// - If `src_dir` cannot be read or the generated file cannot be
///   written — both are build-time configuration errors.
pub fn generate_register_from_sources(src_dir: &str) {
    let mut files = Vec::new();
    collect_rs_files(Path::new(src_dir), &mut files);
    files.sort();
    let mut symbols: Vec<String> = Vec::new();
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let content = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("read `{}` for loft_ffi_build: {e}", file.display()));
        for sym in parse_register_symbols_from_rust(&content) {
            if !symbols.contains(&sym) {
                symbols.push(sym);
            }
        }
    }
    symbols.sort();
    let mut out = String::from("loft_ffi::loft_register! {\n");
    for sym in &symbols {
        out.push_str("    ");
        out.push_str(sym);
        out.push_str(",\n");
    }
    out.push_str("}\n");
    let out_dir = env::var_os("OUT_DIR")
        .expect("OUT_DIR not set — `generate_register_from_sources` must run from a build.rs");
    let dest = Path::new(&out_dir).join("loft_register_gen.rs");
    fs::write(&dest, &out).unwrap_or_else(|e| panic!("write `{}`: {e}", dest.display()));
}

/// Recursively collect every `*.rs` file under `dir` into `out`.
fn collect_rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
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

/// Extract every `n_*` symbol defined as `extern "C" fn n_<ident>` in a
/// Rust source string.  Skips `//`-comment lines so a commented-out or
/// documented signature doesn't get registered.  Order-preserving;
/// the caller de-duplicates across files.
#[must_use]
pub fn parse_register_symbols_from_rust(content: &str) -> Vec<String> {
    const MARKER: &str = "extern \"C\" fn ";
    let mut out = Vec::new();
    for line in content.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let Some(idx) = line.find(MARKER) else {
            continue;
        };
        let rest = &line[idx + MARKER.len()..];
        let ident: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if ident.starts_with("n_") {
            out.push(ident);
        }
    }
    out
}

/// Read the package manifest at `manifest_path` (relative to the
/// crate's `build.rs` working directory — typically `"../loft.toml"`
/// for a sibling-`native/` layout), walk its `[native.functions]`
/// table, and emit a `loft_ffi::loft_register! { … }` invocation to
/// `$OUT_DIR/loft_register_gen.rs`.  The library's `src/lib.rs`
/// `include!`s the generated file at module scope.
///
/// Also emits `cargo:rerun-if-changed=<manifest_path>` so manifest
/// edits trigger regeneration on the next build.
///
/// # Panics
///
/// - If `OUT_DIR` is not set (always set by Cargo when running a
///   build script — the panic is a sanity check, not a recoverable
///   case).
/// - If `manifest_path` cannot be read.  Misconfigured paths are
///   programmer errors, not user errors at runtime.
/// - If the generated file cannot be written.  Same rationale.
pub fn generate_register_invocation(manifest_path: &str) {
    println!("cargo:rerun-if-changed={manifest_path}");
    let content = fs::read_to_string(manifest_path)
        .unwrap_or_else(|e| panic!("read `{manifest_path}` for loft_ffi_build: {e}"));
    let symbols = parse_native_functions(&content);
    let mut out = String::from("loft_ffi::loft_register! {\n");
    for sym in &symbols {
        out.push_str("    ");
        out.push_str(sym);
        out.push_str(",\n");
    }
    out.push_str("}\n");
    let out_dir = env::var_os("OUT_DIR")
        .expect("OUT_DIR not set — `generate_register_invocation` must run from a build.rs");
    let dest = Path::new(&out_dir).join("loft_register_gen.rs");
    fs::write(&dest, &out)
        .unwrap_or_else(|e| panic!("write `{}`: {e}", dest.display()));
}

/// Walk `loft.toml`-style content line-by-line and collect the values
/// of every entry inside `[native.functions]`.  Mirrors the minimal
/// section-scanner in `src/manifest.rs::read_manifest` so build.rs
/// callers have no loft compiler dependency (which would be circular —
/// `loft` depends on `loft-ffi`, and `loft-ffi-build` is a sibling of
/// `loft-ffi`, kept TOML-parser-free for the same reason).
///
/// Comment lines (`#`) and blank lines are ignored.  String quotes
/// around values are stripped.  Anything outside `[native.functions]`
/// is skipped.
///
/// Exposed `pub` so callers can drive the parsing themselves if they
/// want to validate or transform the symbol list before generation —
/// the `generate_register_invocation` wrapper covers the common case.
#[must_use]
pub fn parse_native_functions(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed == "[native.functions]";
            continue;
        }
        if !in_section || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((_, value)) = trimmed.split_once('=') {
            let sym = value.trim().trim_matches('"');
            if !sym.is_empty() {
                out.push(sym.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::parse_native_functions;

    #[test]
    fn parses_native_functions_section() {
        let manifest = r#"
[package]
name = "crypto"

[native]
crate = "loft-crypto"

# Comment before section
[native.functions]
sha256_native = "n_sha256"
# Inline comment
hmac_sha256_native = "n_hmac_sha256"

[other]
ignored = "n_should_not_appear"
"#;
        let syms = parse_native_functions(manifest);
        assert_eq!(syms, vec!["n_sha256", "n_hmac_sha256"]);
    }

    #[test]
    fn empty_section_returns_empty() {
        let manifest = "[native.functions]\n";
        assert!(parse_native_functions(manifest).is_empty());
    }

    #[test]
    fn no_section_returns_empty() {
        let manifest = "[package]\nname = \"x\"\n";
        assert!(parse_native_functions(manifest).is_empty());
    }

    use super::parse_register_symbols_from_rust;

    #[test]
    fn scans_extern_c_fn_definitions() {
        let src = r#"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_load_png(store: LoftStore, p: *const u8) -> bool { todo!() }

#[no_mangle]
pub extern "C" fn n_save_png(image: LoftRef) -> bool { false }

// pub unsafe extern "C" fn n_commented_out(x: i64) -> i64  -- must be skipped
fn helper_not_extern() {}
pub fn also_not_extern() {}
"#;
        assert_eq!(
            parse_register_symbols_from_rust(src),
            vec!["n_load_png", "n_save_png"]
        );
    }

    #[test]
    fn ignores_non_n_prefixed_and_non_extern() {
        let src = "pub extern \"C\" fn other_sym() {}\nfn n_not_extern() {}\n";
        assert!(parse_register_symbols_from_rust(src).is_empty());
    }
}
