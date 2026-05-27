// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Build-script helper for loft native extension cdylibs.
//!
//! @PLAN12 — generates the cdylib's `loft_ffi::loft_register! { … }`
//! symbol list from the library's co-located `#native` annotations, so
//! the registration can never drift from the bindings the compiler
//! sees.  Each library's `build.rs` is two lines:
//!
//! ```ignore
//! fn main() {
//!     loft_ffi_build::generate_register_from_loft("../src");
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
//! Adding a native function to a library is exactly TWO edits:
//!
//! 1. the loft signature + a `#native` marker in `lib/<X>/src/*.loft`
//!    (bare → symbol `n_<fn>`; `#native "sym"` → an override), and
//! 2. the `extern "C"` body in `lib/<X>/native/src/`.
//!
//! The `loft_register!` list (this crate), the extraction-hygiene gate's
//! forbidden list, and the compiler's `def.native` are all DERIVED from
//! those `#native` annotations — no `[native.functions]` manifest table.
//!
//! [`generate_register_invocation`] (the older manifest-scanning entry
//! point) is retained for any caller that still declares
//! `[native.functions]`, but the `#native`-annotation path above is the
//! clean default.

use std::env;
use std::fs;
use std::path::Path;

/// Scan the LIBRARY's loft source (`*.loft`) for every `#native`
/// binding and emit a `loft_ffi::loft_register! { … }` invocation to
/// `$OUT_DIR/loft_register_gen.rs`.  The cdylib's `src/lib.rs`
/// `include!`s the generated file at module scope.
///
/// This is the **drift-proof** registration path: the co-located
/// `#native` annotations next to each loft signature ARE the single
/// source of truth (the same source the compiler binds against), so
/// there is no hand-maintained symbol list and no `[native.functions]`
/// manifest table that can fall out of sync.  Symbol resolution mirrors
/// the parser:
///   - bare `#native`          → `n_<fn_name>` (the convention default)
///   - `#native "n_custom"`    → the explicit override string
///
/// `loft_src_dir` is relative to the build script's working directory
/// (the `native/` crate root) — typically `"../src"` for the standard
/// `lib/<X>/{src,native}/` layout.  Emits `cargo:rerun-if-changed` for
/// every scanned `.loft` so adding or removing a binding regenerates.
///
/// # Panics
/// - If `OUT_DIR` is unset (Cargo always sets it for build scripts).
/// - If the generated file cannot be written — a build configuration
///   error.
pub fn generate_register_from_loft(loft_src_dir: &str) {
    generate(loft_src_dir, false);
}

/// Like [`generate_register_from_loft`], but ALSO emits a
/// `loft_ffi::loft_register_bridges!` invocation registering each symbol's
/// generated marshal bridge (`<symbol>__loft_bridge`) — the plan-25 FFI
/// generated-dispatch path.  Use this from a `build.rs` whose `native/src`
/// applies `#[loft_native]` to every exported fn.
///
/// Assumes the **clean binding** (bare `#native`, so the loft symbol equals
/// the Rust fn name, and the bridge is `<symbol>__loft_bridge`).  A remapped
/// `#native "sym"` would generate a bridge ident that does not exist — a
/// compile error that names the missing symbol.
///
/// # Panics
/// Same as [`generate_register_from_loft`].
pub fn generate_register_from_loft_with_bridges(loft_src_dir: &str) {
    generate(loft_src_dir, true);
}

fn generate(loft_src_dir: &str, bridges: bool) {
    let mut files = Vec::new();
    collect_loft_files(Path::new(loft_src_dir), &mut files);
    files.sort();
    let mut symbols: Vec<String> = Vec::new();
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };
        for sym in parse_register_symbols_from_loft(&content) {
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
    if bridges {
        out.push_str("loft_ffi::loft_register_bridges! {\n");
        for sym in &symbols {
            out.push_str("    \"");
            out.push_str(sym);
            out.push_str("\" => ");
            out.push_str(sym);
            out.push_str("__loft_bridge,\n");
        }
        out.push_str("}\n");
    }
    let out_dir = env::var_os("OUT_DIR")
        .expect("OUT_DIR not set — `generate_register_from_loft` must run from a build.rs");
    let dest = Path::new(&out_dir).join("loft_register_gen.rs");
    fs::write(&dest, &out).unwrap_or_else(|e| panic!("write `{}`: {e}", dest.display()));
}

/// Recursively collect every `*.loft` file under `dir` into `out`.
fn collect_loft_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_loft_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "loft") {
            out.push(path);
        }
    }
}

/// Extract every native symbol bound by a `#native` annotation in a loft
/// source string.  Tracks the most recent `fn <name>` so a BARE
/// `#native` resolves to `n_<name>` (the convention default); an
/// explicit `#native "sym"` yields `sym`.  Mirrors the parser's
/// resolution (`src/parser/definitions.rs`).
#[must_use]
pub fn parse_register_symbols_from_loft(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut last_fn: Option<String> = None;
    for line in content.lines() {
        let t = line.trim();
        let decl = t.strip_prefix("pub ").unwrap_or(t);
        if let Some(after) = decl.strip_prefix("fn ") {
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                last_fn = Some(name);
            }
        } else if let Some(rest) = t.strip_prefix("#native") {
            let rest = rest.trim();
            let sym = if rest.is_empty() {
                last_fn.as_ref().map(|n| format!("n_{n}"))
            } else {
                Some(rest.trim_matches('"').to_string())
            };
            if let Some(sym) = sym {
                out.push(sym);
            }
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
    fs::write(&dest, &out).unwrap_or_else(|e| panic!("write `{}`: {e}", dest.display()));
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

    use super::parse_register_symbols_from_loft;

    #[test]
    fn bare_native_defaults_to_fn_name() {
        let src = "\
fn load_png(path: text, image: Image) -> boolean;
#native
";
        assert_eq!(parse_register_symbols_from_loft(src), vec!["n_load_png"]);
    }

    #[test]
    fn explicit_native_is_the_override_symbol() {
        // A `_native` wrapper backing a differently-named symbol, and a
        // non-`n_` symbol (graphics' `loft_*` convention).
        let src = "\
fn save_png_native(image: const Image, path: text) -> boolean;
#native \"n_save_png\"
pub fn create_window(w: integer, h: integer) -> integer;
#native \"loft_gl_create_window\"
";
        assert_eq!(
            parse_register_symbols_from_loft(src),
            vec!["n_save_png", "loft_gl_create_window"]
        );
    }

    #[test]
    fn mixes_bare_and_explicit_and_ignores_plain_fns() {
        let src = "\
pub fn pure_helper(x: integer) -> integer { x * 2 }
fn tcp_listen(port: integer) -> integer;
#native
fn ws_upgrade_native() -> integer;
#native \"n_ws_upgrade\"
";
        assert_eq!(
            parse_register_symbols_from_loft(src),
            vec!["n_tcp_listen", "n_ws_upgrade"]
        );
    }
}
