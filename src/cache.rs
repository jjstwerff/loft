// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Startup cache — skip re-parsing the `default/` standard library on
//! every run.
//!
//! Measured (native `--release`, see
//! `doc/claude/plans/deferred/28-const-store/STARTUP_CACHE_PLAN.md`):
//! parsing `default/` is ~90 % of cold-start (~15 ms) while bytecode
//! generation is ~0.5 ms.  So the win is to cache the parser's output
//! (`Data`) keyed on the stdlib content, restore it, and re-run the
//! cheap codegen fresh — rather than serialise bytecode/stores.
//!
//! This module currently provides the **cache key** — the correctness
//! foundation.  The retired Phase D cache (removed in the integer-i64
//! migration) keyed only on the user source and therefore served stale
//! bytecode after a `default/*.loft` edit; the key here folds in the
//! stdlib content, the loft version, the build id, and the active
//! feature set so no such staleness is possible.
//!
//! ## No serde
//!
//! The `Data`/bytecode snapshot is serialised by hand (length-prefixed
//! little-endian, the same approach as the retired Phase D
//! `src/cache.rs`), **not** via `serde`.  serde-derive cannot express
//! the IR cleanly — `&'static str` fields (`Block.name`,
//! `Definition.synthetic`) make the derive inject a `'de: 'static`
//! bound that poisons the whole recursive `Value`/`Type`/`Data` graph,
//! and the `OnceLock` index field is non-derivable.  Hand-rolled
//! encoding sidesteps all of it and lets us skip the rebuildable
//! `HashMap` indices entirely.  See [CODE.md](../doc/claude/CODE.md)
//! § Dependencies — serde is a forbidden dependency project-wide
//! (native builds).

use sha2::{Digest, Sha256};

/// Format-version byte.  Bump whenever the on-disk snapshot layout
/// changes so old caches are rejected rather than misread.
const CACHE_FORMAT_VERSION: u8 = 1;

/// Loft crate version — a release bump invalidates every cache.
const LOFT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build id from `build.rs` (git short HEAD, or epoch-seconds fallback).
/// Invalidates the cache when the interpreter is rebuilt without a
/// version bump — e.g. a parser fix that changes `Data` output.
const BUILD_ID: &str = env!("LOFT_BUILD_ID");

/// The set of compile-time features that change parsed `Data`, the
/// bytecode, or the native registry — and therefore must be part of the
/// cache key.  A cache produced by a `threading` build must not be read
/// by a non-`threading` build.
///
/// Encoded as a stable, comma-joined string of the features that are
/// *enabled* in this build.  Order is fixed (not dependent on cfg
/// evaluation order) so the key is deterministic.
#[must_use]
pub fn feature_signature() -> String {
    let mut feats: Vec<&str> = Vec::new();
    if cfg!(feature = "threading") {
        feats.push("threading");
    }
    if cfg!(feature = "wasm") {
        feats.push("wasm");
    }
    if cfg!(feature = "mmap") {
        feats.push("mmap");
    }
    if cfg!(feature = "png") {
        feats.push("png");
    }
    if cfg!(feature = "native-extensions") {
        feats.push("native-extensions");
    }
    if cfg!(feature = "random") {
        feats.push("random");
    }
    feats.join(",")
}

/// Compute the stdlib cache key: a SHA-256 over every input that can
/// change the compiled standard library.
///
/// Inputs, each length-prefixed so no concatenation ambiguity exists:
/// - [`CACHE_FORMAT_VERSION`] — on-disk layout
/// - [`LOFT_VERSION`] — release version
/// - [`BUILD_ID`] — same-version rebuild discriminator
/// - the target triple — no cross-arch cache reuse
/// - the active [`feature_signature`]
/// - the concatenated `default/*.loft` source bytes (the
///   retirement-bug fix: a stdlib edit changes the key)
///
/// `stdlib_sources` is a slice of `(name, content)` pairs.  The caller
/// passes them in a stable order (the loader collects them sorted);
/// names are included so a rename also invalidates.
#[must_use]
pub fn stdlib_cache_key(stdlib_sources: &[(String, String)]) -> [u8; 32] {
    let mut h = Sha256::new();
    // Each field is fed as (u64 little-endian length, bytes) so two
    // different field boundaries can never hash identically.
    let mut put = |bytes: &[u8]| {
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    };
    put(&[CACHE_FORMAT_VERSION]);
    put(LOFT_VERSION.as_bytes());
    put(BUILD_ID.as_bytes());
    put(target_triple().as_bytes());
    put(feature_signature().as_bytes());
    // Number of stdlib files, then each (name, content).
    h.update((stdlib_sources.len() as u64).to_le_bytes());
    for (name, content) in stdlib_sources {
        let mut field = |bytes: &[u8]| {
            h.update((bytes.len() as u64).to_le_bytes());
            h.update(bytes);
        };
        field(name.as_bytes());
        field(content.as_bytes());
    }
    h.finalize().into()
}

/// The target triple this binary was built for, e.g.
/// `x86_64-unknown-linux-gnu`.  A cache must never be shared across
/// architectures.  Assembled from the standard `cfg` values rather than
/// a build-script env var so it works in every build configuration.
#[must_use]
fn target_triple() -> String {
    format!(
        "{}-{}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::env::consts::FAMILY,
    )
}

/// Collect the `default/` stdlib sources as `(filename, content)` pairs,
/// sorted by filename for a deterministic cache key.  Reads every `*.loft`
/// file directly under `default_dir` (non-recursive — the stdlib is flat).
///
/// Returns an empty vec on any read error; the caller treats that as
/// "cannot cache" and falls back to a cold parse.
#[must_use]
pub fn collect_stdlib_sources(default_dir: &str) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(default_dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("loft") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };
        out.push((name, content));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The on-disk path for the stdlib snapshot keyed by `key`.
///
/// Uses `$XDG_CACHE_HOME/loft/` (or `$HOME/.cache/loft/`), falling back to
/// the system temp dir if neither is set.  The filename embeds the full
/// 64-hex key so distinct builds / feature-sets / stdlib-content never
/// collide.
#[must_use]
pub fn stdlib_cache_path(key: &[u8; 32]) -> std::path::PathBuf {
    let mut hex = String::with_capacity(64);
    for b in key {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("loft");
    base.join(format!("stdlib-{hex}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<(String, String)> {
        vec![
            ("01_code.loft".to_string(), "fn a() {}".to_string()),
            ("02_files.loft".to_string(), "fn b() {}".to_string()),
        ]
    }

    #[test]
    fn key_is_deterministic() {
        // Identical inputs → identical key, across repeated calls.
        let a = stdlib_cache_key(&sample());
        let b = stdlib_cache_key(&sample());
        assert_eq!(a, b, "same inputs must yield the same key");
    }

    #[test]
    fn key_changes_on_stdlib_content() {
        // The retirement bug: a stdlib edit MUST change the key.
        let base = stdlib_cache_key(&sample());
        let mut edited = sample();
        edited[0].1.push_str(" // tweak");
        assert_ne!(
            base,
            stdlib_cache_key(&edited),
            "editing default/*.loft content must invalidate the cache"
        );
    }

    #[test]
    fn key_changes_on_stdlib_name() {
        let base = stdlib_cache_key(&sample());
        let mut renamed = sample();
        renamed[0].0 = "01_core.loft".to_string();
        assert_ne!(
            base,
            stdlib_cache_key(&renamed),
            "renaming a stdlib file must invalidate the cache"
        );
    }

    #[test]
    fn key_changes_on_file_count() {
        let base = stdlib_cache_key(&sample());
        let mut more = sample();
        more.push(("03_text.loft".to_string(), "fn c() {}".to_string()));
        assert_ne!(
            base,
            stdlib_cache_key(&more),
            "adding a stdlib file must invalidate the cache"
        );
    }

    #[test]
    fn key_is_order_sensitive() {
        // Reordering files is a different stdlib (load order matters for
        // parse); the key must reflect it.
        let base = stdlib_cache_key(&sample());
        let mut swapped = sample();
        swapped.swap(0, 1);
        assert_ne!(
            base,
            stdlib_cache_key(&swapped),
            "stdlib file order is part of the key"
        );
    }

    #[test]
    fn boundary_shift_changes_key() {
        // Moving a byte across the name/content boundary must not
        // collide (length-prefixing guarantees this).
        let a = stdlib_cache_key(&[("ab".to_string(), "c".to_string())]);
        let b = stdlib_cache_key(&[("a".to_string(), "bc".to_string())]);
        assert_ne!(a, b, "field boundaries must not be ambiguous");
    }

    #[test]
    fn feature_signature_is_stable() {
        // Two calls in the same build must agree (deterministic order).
        assert_eq!(feature_signature(), feature_signature());
        // The default native build enables threading; sanity-check the
        // signature is non-empty there so the key actually varies by
        // build config.
        #[cfg(feature = "threading")]
        assert!(feature_signature().contains("threading"));
    }

    #[test]
    fn collect_stdlib_sources_reads_default_dir_sorted() {
        // The real default/ dir relative to the crate root.
        let srcs = collect_stdlib_sources("default");
        assert!(srcs.len() >= 3, "expected the stdlib .loft files, got {}", srcs.len());
        assert!(srcs.iter().all(|(n, _)| std::path::Path::new(n).extension() == Some("loft".as_ref())));
        // Sorted by filename → deterministic key.
        let mut sorted = srcs.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(srcs, sorted, "sources must be returned sorted by filename");
        // 01_code.loft is always present and non-empty.
        assert!(srcs.iter().any(|(n, c)| n == "01_code.loft" && !c.is_empty()));
    }

    #[test]
    fn collect_stdlib_sources_missing_dir_is_empty() {
        assert!(collect_stdlib_sources("nonexistent-dir-xyz").is_empty());
    }

    #[test]
    fn cache_path_is_deterministic_and_key_specific() {
        let k1 = stdlib_cache_key(&[("x.loft".into(), "a".into())]);
        let k2 = stdlib_cache_key(&[("x.loft".into(), "b".into())]);
        let p1 = stdlib_cache_path(&k1);
        assert_eq!(p1, stdlib_cache_path(&k1), "same key → same path");
        assert_ne!(p1, stdlib_cache_path(&k2), "different key → different path");
        let name = p1.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("stdlib-") && name.ends_with(".json"));
        assert!(
            name.len() == "stdlib-".len() + 64 + ".json".len(),
            "filename embeds 64-hex key"
        );
    }
}
