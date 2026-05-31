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

use sha2::{Digest, Sha256};

/// Serde helper for `&'static str` fields (e.g. `Block.name`,
/// `Definition.synthetic`).  Serialises as a plain string; on
/// deserialise it `Box::leak`s the owned string to recover a
/// `&'static str`.
///
/// The leak is intentional and bounded: it happens once per cached
/// stdlib load, over the fixed set of compiler-internal block-kind
/// labels and synthesis tags (a few hundred short strings at most),
/// and is reclaimed at process exit.  For a startup-cache load this is
/// a negligible, one-shot cost — the standard pattern for restoring
/// interned `&'static str` from a serialised form.
pub mod static_str {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(s: &&'static str, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<&'static str, D::Error> {
        let owned = String::deserialize(de)?;
        Ok(Box::leak(owned.into_boxed_str()))
    }
}

/// Serde helper for `Option<&'static str>` fields.
pub mod opt_static_str {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(s: &Option<&'static str>, ser: S) -> Result<S::Ok, S::Error> {
        match s {
            Some(v) => ser.serialize_some(v),
            None => ser.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        de: D,
    ) -> Result<Option<&'static str>, D::Error> {
        let owned = Option::<String>::deserialize(de)?;
        Ok(owned.map(|s| &*Box::leak(s.into_boxed_str())))
    }
}

/// Serde helper for `HashMap<K, &'static str>` value fields
/// (e.g. `Function.scope_origins`).
pub mod map_static_str {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::HashMap;
    use std::hash::Hash;

    pub fn serialize<S, K>(m: &HashMap<K, &'static str>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        K: Serialize + Eq + Hash,
    {
        // Serialise as a Vec of (K, String) pairs — order-independent
        // round-trip (the map is rebuilt on load).
        let pairs: Vec<(&K, &str)> = m.iter().map(|(k, v)| (k, *v)).collect();
        pairs.serialize(ser)
    }

    pub fn deserialize<'de, D, K>(de: D) -> Result<HashMap<K, &'static str>, D::Error>
    where
        D: Deserializer<'de>,
        K: serde::de::DeserializeOwned + Eq + Hash,
    {
        let pairs: Vec<(K, String)> = Vec::deserialize(de)?;
        Ok(pairs
            .into_iter()
            .map(|(k, v)| (k, &*Box::leak(v.into_boxed_str())))
            .collect())
    }
}

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
}
