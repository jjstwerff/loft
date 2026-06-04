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

/// @PLAN54 G2/M6 — a stable string identifying THIS binary build, for the
/// whole-program cache **manifest**.  Mirrors the version inputs of
/// [`stdlib_cache_key`] (format / version / rebuild-id / target / features) so a
/// binary upgrade invalidates a stale program bundle: a bundle written by one
/// build must never be loaded by another (its baked store layout / codegen may
/// differ, which `Store::is_store_file`'s fixed magic does NOT catch).
///
/// Also folds in the running executable's modification time ([`binary_signature_tag`])
/// so an **uncommitted** compiler rebuild invalidates bundles too — [`BUILD_ID`]
/// is the git HEAD hash, which does not change across uncommitted edits, leaving
/// a parser/scopes fix under development at risk of a stale warm-load (see the
/// plan's "Debugging-iteration cost + dev-safety caveat").
#[must_use]
pub fn build_signature() -> String {
    format!(
        "v{CACHE_FORMAT_VERSION}|{LOFT_VERSION}|{BUILD_ID}|{}|{}|{}",
        target_triple(),
        feature_signature(),
        binary_signature_tag(),
    )
}

/// A tag for the *running binary's own build*, folded into [`build_signature`].
///
/// The executable's modification time changes on every rebuild (cargo rewrites
/// the binary), so mixing it in makes any rebuild — committed or not —
/// invalidate program bundles, closing the gap [`BUILD_ID`] (git HEAD) leaves
/// open for uncommitted dev builds.  Best-effort: returns `""` when the exe path
/// or its mtime is unavailable, so the signature gracefully falls back to the
/// [`BUILD_ID`]-only behaviour rather than panicking.
#[must_use]
fn binary_signature_tag() -> String {
    let Ok(exe) = std::env::current_exe() else {
        return String::new();
    };
    let Ok(meta) = std::fs::metadata(&exe) else {
        return String::new();
    };
    let Ok(mtime) = meta.modified() else {
        return String::new();
    };
    match mtime.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("{}.{}", d.as_secs(), d.subsec_nanos()),
        Err(_) => String::new(),
    }
}

/// @PLAN54 G2 / track 1 — whether the whole-program startup cache is active for
/// this run.  **Default ON** (the 3–3.6× warm-start win, no longer hidden behind
/// an opt-in flag), with three overrides; see [`cache_decision`] for the policy.
#[must_use]
pub fn program_cache_enabled() -> bool {
    fn is_set(name: &str) -> bool {
        std::env::var_os(name).is_some_and(|v| !v.is_empty())
    }
    cache_decision(
        is_set("LOFT_NO_CACHE"),
        is_set("LOFT_PROGRAM_CACHE"),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    )
}

/// The cache-enable policy as a pure function of its three signals (so it is
/// unit-testable without mutating process-global env).  Precedence, first match
/// wins:
/// 1. `no_cache` (`LOFT_NO_CACHE` set) → **off** — the explicit kill switch.
/// 2. `program_cache` (`LOFT_PROGRAM_CACHE` set) → **on** — explicit force,
///    overriding the cargo-context default below (the cache's own tests use it).
/// 3. `under_cargo` (`CARGO_MANIFEST_DIR` present) → **off** — running inside a
///    Cargo build / `cargo run` / `cargo test`.  This keeps the compiler-debug
///    loop (dev-safety caveat) and the whole integration-test suite from
///    writing/reading bundles, with no per-test wiring.
/// 4. otherwise → **on** — the default-on win for installed / real invocations.
#[must_use]
fn cache_decision(no_cache: bool, program_cache: bool, under_cargo: bool) -> bool {
    if no_cache {
        return false;
    }
    if program_cache {
        return true;
    }
    !under_cargo
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

/// The on-disk path for the stdlib bundle keyed by `key` (@PLAN54 D2b — the
/// store-format `.store` bundle written by `ir_store::save_bundle`).
///
/// Uses `$XDG_CACHE_HOME/loft/` (or `$HOME/.cache/loft/`), falling back to
/// the system temp dir if neither is set.  The filename embeds the full
/// 64-hex key so distinct builds / feature-sets / stdlib-content never
/// collide.
#[must_use]
pub fn stdlib_cache_path(key: &[u8; 32]) -> std::path::PathBuf {
    cache_base_dir().join(format!("stdlib-{}.store", hex32(key)))
}

/// The loft cache directory: `$XDG_CACHE_HOME/loft/` (or `$HOME/.cache/loft/`),
/// falling back to the system temp dir.
#[must_use]
fn cache_base_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("loft")
}

/// Lower-case 64-hex of a 32-byte key.
#[must_use]
fn hex32(key: &[u8; 32]) -> String {
    let mut hex = String::with_capacity(64);
    for b in key {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// @PLAN54 arc E — SHA-256 of a file's bytes, or `None` if unreadable.  Used to
/// hash every parsed source for the whole-program bundle's drift manifest.
#[must_use]
pub fn file_hash(path: &str) -> Option<[u8; 32]> {
    let bytes = std::fs::read(path).ok()?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Some(h.finalize().into())
}

/// @PLAN54 arc E — the `(bundle, manifest)` paths for the whole-program cache of
/// the script at `script_abspath`.  Keyed on the script's path so each script
/// gets a stable slot; the manifest (every parsed source + its content hash)
/// detects drift in any input — stdlib, lazily-loaded libs, or the script.
#[must_use]
pub fn program_cache_paths(script_abspath: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let mut h = Sha256::new();
    h.update(script_abspath.as_bytes());
    let key: [u8; 32] = h.finalize().into();
    let base = cache_base_dir();
    let stem = format!("program-{}", hex32(&key));
    (
        base.join(format!("{stem}.store")),
        base.join(format!("{stem}.manifest")),
    )
}

/// @PLAN54 G2 / track 1 — default budget (MiB) for the program-cache directory
/// before eviction kicks in.  ~512 MiB ≈ 70 bundles at the measured ~7 MiB each;
/// overridable via `LOFT_CACHE_MAX_MB`.
const DEFAULT_CACHE_MAX_MB: u64 = 512;

/// The program-cache size budget in bytes (`LOFT_CACHE_MAX_MB` × 1 MiB, default
/// [`DEFAULT_CACHE_MAX_MB`]).  A malformed value falls back to the default.
#[must_use]
fn program_cache_budget_bytes() -> u64 {
    std::env::var("LOFT_CACHE_MAX_MB")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_CACHE_MAX_MB)
        .saturating_mul(1024 * 1024)
}

/// @PLAN54 G2 / track 1 — bound unbounded cache growth.  With the cache default-on
/// each distinct script gets its own `program-<hash>.store` bundle (~7 MiB) and
/// nothing ever removes them; over a long-lived install that grows without limit.
/// After a cold save, prune the program-cache directory back under the budget
/// ([`program_cache_budget_bytes`]) by evicting whole (`.store` + `.manifest`)
/// pairs oldest-first (mtime, which a drift re-save refreshes).  Best-effort.
pub fn prune_program_cache() {
    prune_dir(&cache_base_dir(), program_cache_budget_bytes());
}

/// The eviction core, factored out of [`prune_program_cache`] so it is testable
/// against a temp dir with an explicit budget (no env, no global cache dir).
/// Only `program-*.store` files (and their sibling `.manifest`) are considered;
/// any other cache file (e.g. the stdlib bundle) is left untouched.
fn prune_dir(base: &std::path::Path, budget_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    struct Bundle {
        store: std::path::PathBuf,
        mtime: std::time::SystemTime,
        size: u64,
    }
    let mut bundles: Vec<Bundle> = Vec::new();
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_program_store = path.extension().and_then(|x| x.to_str()) == Some("store")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("program-"));
        if !is_program_store {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let size = meta.len();
        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        total = total.saturating_add(size);
        bundles.push(Bundle {
            store: path,
            mtime,
            size,
        });
    }
    if total <= budget_bytes {
        return;
    }
    bundles.sort_by_key(|b| b.mtime); // oldest first
    for b in &bundles {
        if total <= budget_bytes {
            break;
        }
        let _ = std::fs::remove_file(&b.store);
        let _ = std::fs::remove_file(b.store.with_extension("manifest"));
        total = total.saturating_sub(b.size);
    }
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
        assert!(
            srcs.len() >= 3,
            "expected the stdlib .loft files, got {}",
            srcs.len()
        );
        assert!(
            srcs.iter()
                .all(|(n, _)| std::path::Path::new(n).extension() == Some("loft".as_ref()))
        );
        // Sorted by filename → deterministic key.
        let mut sorted = srcs.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(srcs, sorted, "sources must be returned sorted by filename");
        // 01_code.loft is always present and non-empty.
        assert!(
            srcs.iter()
                .any(|(n, c)| n == "01_code.loft" && !c.is_empty())
        );
    }

    #[test]
    fn collect_stdlib_sources_missing_dir_is_empty() {
        assert!(collect_stdlib_sources("nonexistent-dir-xyz").is_empty());
    }

    #[test]
    fn cache_decision_precedence() {
        // (no_cache, program_cache, under_cargo) → enabled?
        // 1. kill switch wins over everything.
        assert!(!cache_decision(true, true, false));
        assert!(!cache_decision(true, false, true));
        // 2. explicit force-on overrides the cargo-context default.
        assert!(cache_decision(false, true, true));
        // 3. cargo context disables by default (test suite / cargo run).
        assert!(!cache_decision(false, false, true));
        // 4. plain installed invocation → default on.
        assert!(cache_decision(false, false, false));
    }

    #[test]
    fn build_signature_is_deterministic_and_carries_version() {
        let a = build_signature();
        assert_eq!(a, build_signature(), "same build → same signature");
        assert!(a.contains(LOFT_VERSION), "signature pins the crate version");
        // Five '|'-separated fields (format|version|build-id|target|features|binary).
        assert_eq!(a.matches('|').count(), 5, "signature shape: {a}");
    }

    #[test]
    fn prune_dir_evicts_oldest_over_budget() {
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("loft_prune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Three 10-byte bundles (+ manifests); set distinct mtimes oldest→newest.
        let mk = |name: &str, age_secs: u64| {
            let store = dir.join(format!("{name}.store"));
            std::fs::write(&store, b"0123456789").unwrap();
            std::fs::write(dir.join(format!("{name}.manifest")), b"m").unwrap();
            let when = std::time::SystemTime::now() - Duration::from_secs(age_secs);
            std::fs::File::open(&store)
                .unwrap()
                .set_modified(when)
                .unwrap();
        };
        mk("program-aaa", 300); // oldest
        mk("program-bbb", 200);
        mk("program-ccc", 100); // newest
        // A non-program file must be left alone.
        std::fs::write(dir.join("stdlib-zzz.store"), b"keepme").unwrap();

        // Budget 25 bytes: 3×10 = 30 > 25 → evict the single oldest (→ 20 ≤ 25).
        prune_dir(&dir, 25);
        assert!(
            !dir.join("program-aaa.store").exists(),
            "oldest store evicted"
        );
        assert!(
            !dir.join("program-aaa.manifest").exists(),
            "oldest manifest evicted with it"
        );
        assert!(dir.join("program-bbb.store").exists(), "newer bundle kept");
        assert!(dir.join("program-ccc.store").exists(), "newest bundle kept");
        assert!(
            dir.join("stdlib-zzz.store").exists(),
            "non-program cache file untouched"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_path_is_deterministic_and_key_specific() {
        let k1 = stdlib_cache_key(&[("x.loft".into(), "a".into())]);
        let k2 = stdlib_cache_key(&[("x.loft".into(), "b".into())]);
        let p1 = stdlib_cache_path(&k1);
        assert_eq!(p1, stdlib_cache_path(&k1), "same key → same path");
        assert_ne!(p1, stdlib_cache_path(&k2), "different key → different path");
        let name = p1.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("stdlib-") && name.ends_with(".store"));
        assert!(
            name.len() == "stdlib-".len() + 64 + ".store".len(),
            "filename embeds 64-hex key"
        );
    }
}
