// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN12 phase 2 final step (2026-05-24) — generate the
//! `loft_ffi::loft_register!` invocation from
//! `lib/web/loft.toml::[native.functions]` so the symbol list lives in
//! exactly ONE place.  The crate's `src/lib.rs` `include!`s the generated
//! file at module scope.
//!
//! Sibling to `lib/crypto/native/build.rs` — same logic, different
//! manifest path.  The two build scripts could collapse into a shared
//! `loft-ffi-build` crate later if more libraries adopt this pattern;
//! the inline duplication is acceptable while only 2 libraries use it.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let manifest_path = Path::new("..").join("loft.toml");
    println!("cargo:rerun-if-changed=../loft.toml");
    let content =
        fs::read_to_string(&manifest_path).expect("read ../loft.toml relative to native/");
    let symbols = parse_native_functions(&content);
    let mut out = String::from("loft_ffi::loft_register! {\n");
    for sym in &symbols {
        out.push_str("    ");
        out.push_str(sym);
        out.push_str(",\n");
    }
    out.push_str("}\n");
    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR not set");
    let dest = Path::new(&out_dir).join("loft_register_gen.rs");
    fs::write(&dest, out).expect("write loft_register_gen.rs");
}

/// Walk `loft.toml` line-by-line and collect the values of every entry
/// inside `[native.functions]`.  Mirrors the minimal section-scanner in
/// `src/manifest.rs::read_manifest` so build.rs has no loft compiler
/// dependency (which would be circular).
fn parse_native_functions(content: &str) -> Vec<String> {
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
