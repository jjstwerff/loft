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
}
