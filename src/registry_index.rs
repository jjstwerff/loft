// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

// Included by BOTH the library crate (where install.rs + the search/info
// CLI subcommands use the full API) AND the binary crate (where
// parser/mod.rs::probe_registry_installed routes through lockfile only).
// `allow(dead_code)` silences the binary's view; the lib uses every
// symbol so the attribute is a no-op there.
#![allow(dead_code)]

//! Parsed `registry.json` (the JSON-format file-based registry index
//! described in [PKG_REGISTRY.md](../doc/claude/PKG_REGISTRY.md)).
//!
//! PKG.REG R4 scaffolding.  This module owns:
//!
//! - Schema typestate (`RegistryIndex`, `Package`, `Version`).
//! - Index parsing from JSON via `crate::json::parse`.
//! - Version-constraint resolution (`find_best_version`).
//! - Local cache paths (`cache_dir`, `index_paths`, `extract_dir`).
//! - HTTPS fetcher for the index + the per-tarball download (uses
//!   `ureq`, gated on the `registry` feature).
//!
//! What it does NOT own (R5+):
//!
//! - Driving the install flow (`loft install <name>` → calls into
//!   this module then writes `loft.lock`).
//! - Transitive resolution.
//! - The `loft.lock` reader/writer (lives in `crate::lockfile`).
//! - Signature verification of the index (lives in
//!   `crate::registry_signing`).

#![cfg(feature = "registry")]

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::json::{Parsed, parse as parse_json};

// ── Schema ────────────────────────────────────────────────────────

/// Parsed `registry.json`.
#[derive(Debug, Clone)]
pub struct RegistryIndex {
    pub schema_version: u32,
    pub updated: String,
    pub packages: BTreeMap<String, Package>,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub categories: Vec<String>,
    pub yanked: Vec<String>,
    pub versions: BTreeMap<String, Version>,
}

#[derive(Debug, Clone)]
pub struct Version {
    pub semver: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub loft: String,
    pub deps: BTreeMap<String, String>,
    /// Schema slot — resolver-side support deferred.
    pub conflicts: Vec<String>,
    /// Schema slot — resolver-side support deferred.
    pub replaces: Vec<String>,
    /// Schema slot — resolver-side support deferred.
    pub provides: Vec<String>,
    /// Tier-1 lazy-load triggers — text-method names as `"name:receiver"`
    /// (e.g. `"matches:text"`), derived from the package source at publish time
    /// so a CONSUMER's resolver can map `obj.method()` to this package without
    /// having the source.  Produced by `crate::triggers::derive_triggers`.
    pub triggers: Vec<String>,
    /// Schema slot — pre-built distribution deferred.
    pub binaries: BTreeMap<String, BinaryEntry>,
    pub prerelease: bool,
    pub published: String,
}

#[derive(Debug, Clone)]
pub struct BinaryEntry {
    pub url: String,
    pub sha256: String,
}

// ── Parsing ───────────────────────────────────────────────────────

/// Parse a `registry.json` document into the strongly-typed schema.
///
/// # Errors
///
/// Returns a `String` error on malformed JSON, unsupported
/// schema_version, or missing required fields.  Schema-violation
/// errors are explicit ("missing `<field>` for package X version Y")
/// rather than generic parse errors, because publishers debugging a
/// rejected PR need to know exactly which line to fix.
pub fn parse_index(content: &str) -> Result<RegistryIndex, String> {
    let parsed = parse_json(content).map_err(|e| format!("JSON parse error: {e:?}"))?;
    let Parsed::Object(root) = parsed else {
        return Err("registry.json: top-level must be an object".to_string());
    };
    let mut schema_version: Option<u32> = None;
    let mut updated: Option<String> = None;
    let mut packages: BTreeMap<String, Package> = BTreeMap::new();
    for (k, _, v) in &root {
        match k.as_str() {
            "schema_version" => {
                if let Parsed::Number(n) = v {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        schema_version = Some(*n as u32);
                    }
                }
            }
            "updated" => {
                if let Parsed::Str(s) = v {
                    updated = Some(s.clone());
                }
            }
            "packages" => {
                if let Parsed::Object(pkgs) = v {
                    for (pname, _, pval) in pkgs {
                        let pkg = parse_package(pname, pval)?;
                        packages.insert(pname.clone(), pkg);
                    }
                }
            }
            _ => {
                // Forward-compat: silently ignore unknown top-level keys.
            }
        }
    }
    let schema_version =
        schema_version.ok_or_else(|| "registry.json: missing `schema_version`".to_string())?;
    if schema_version != 1 {
        return Err(format!(
            "registry.json: schema_version {schema_version} unsupported — upgrade loft"
        ));
    }
    Ok(RegistryIndex {
        schema_version,
        updated: updated.unwrap_or_default(),
        packages,
    })
}

fn parse_package(name: &str, val: &Parsed) -> Result<Package, String> {
    let Parsed::Object(fields) = val else {
        return Err(format!("package `{name}`: expected object"));
    };
    let mut pkg = Package {
        name: name.to_string(),
        description: None,
        homepage: None,
        categories: Vec::new(),
        yanked: Vec::new(),
        versions: BTreeMap::new(),
    };
    for (k, _, v) in fields {
        match k.as_str() {
            "description" => {
                if let Parsed::Str(s) = v {
                    pkg.description = Some(s.clone());
                }
            }
            "homepage" => {
                if let Parsed::Str(s) = v {
                    pkg.homepage = Some(s.clone());
                }
            }
            "categories" => {
                if let Parsed::Array(arr) = v {
                    pkg.categories = arr
                        .iter()
                        .filter_map(|p| match p {
                            Parsed::Str(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                }
            }
            "yanked" => {
                if let Parsed::Array(arr) = v {
                    pkg.yanked = arr
                        .iter()
                        .filter_map(|p| match p {
                            Parsed::Str(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                }
            }
            "versions" => {
                if let Parsed::Object(vmap) = v {
                    for (semver, _, vval) in vmap {
                        let ver = parse_version(name, semver, vval)?;
                        pkg.versions.insert(semver.clone(), ver);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(pkg)
}

fn parse_version(pkg_name: &str, semver: &str, val: &Parsed) -> Result<Version, String> {
    let Parsed::Object(fields) = val else {
        return Err(format!(
            "package `{pkg_name}` version `{semver}`: expected object"
        ));
    };
    let mut url: Option<String> = None;
    let mut sha256: Option<String> = None;
    let mut size: Option<u64> = None;
    let mut loft: Option<String> = None;
    let mut deps: BTreeMap<String, String> = BTreeMap::new();
    let mut conflicts: Vec<String> = Vec::new();
    let mut replaces: Vec<String> = Vec::new();
    let mut provides: Vec<String> = Vec::new();
    let mut triggers: Vec<String> = Vec::new();
    let mut binaries: BTreeMap<String, BinaryEntry> = BTreeMap::new();
    let mut prerelease = false;
    let mut published: Option<String> = None;
    for (k, _, v) in fields {
        match (k.as_str(), v) {
            ("url", Parsed::Str(s)) => url = Some(s.clone()),
            ("sha256", Parsed::Str(s)) => sha256 = Some(s.clone()),
            ("size", Parsed::Number(n)) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    size = Some(*n as u64);
                }
            }
            ("loft", Parsed::Str(s)) => loft = Some(s.clone()),
            ("deps", Parsed::Object(dmap)) => {
                for (dname, _, dval) in dmap {
                    if let Parsed::Str(s) = dval {
                        deps.insert(dname.clone(), s.clone());
                    }
                }
            }
            ("conflicts", Parsed::Array(a)) => {
                conflicts = a
                    .iter()
                    .filter_map(|p| match p {
                        Parsed::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
            }
            ("replaces", Parsed::Array(a)) => {
                replaces = a
                    .iter()
                    .filter_map(|p| match p {
                        Parsed::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
            }
            ("provides", Parsed::Array(a)) => {
                provides = a
                    .iter()
                    .filter_map(|p| match p {
                        Parsed::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
            }
            ("triggers", Parsed::Array(a)) => {
                triggers = a
                    .iter()
                    .filter_map(|p| match p {
                        Parsed::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
            }
            ("binaries", Parsed::Object(bmap)) => {
                for (triple, _, bval) in bmap {
                    if let Parsed::Object(bfields) = bval {
                        let mut burl: Option<String> = None;
                        let mut bsha: Option<String> = None;
                        for (bk, _, bv) in bfields {
                            match (bk.as_str(), bv) {
                                ("url", Parsed::Str(s)) => burl = Some(s.clone()),
                                ("sha256", Parsed::Str(s)) => bsha = Some(s.clone()),
                                _ => {}
                            }
                        }
                        if let (Some(u), Some(s)) = (burl, bsha) {
                            binaries.insert(triple.clone(), BinaryEntry { url: u, sha256: s });
                        }
                    }
                }
            }
            ("prerelease", Parsed::Bool(b)) => prerelease = *b,
            ("published", Parsed::Str(s)) => published = Some(s.clone()),
            _ => {}
        }
    }
    let url =
        url.ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `url`"))?;
    let sha256 = sha256
        .ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `sha256`"))?;
    let size =
        size.ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `size`"))?;
    let loft =
        loft.ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `loft`"))?;
    let published = published
        .ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `published`"))?;
    Ok(Version {
        semver: semver.to_string(),
        url,
        sha256,
        size,
        loft,
        deps,
        conflicts,
        replaces,
        provides,
        triggers,
        binaries,
        prerelease,
        published,
    })
}

// ── Version resolution ────────────────────────────────────────────

/// Find the best version of `pkg` matching `constraint`.  Skips
/// yanked versions and (unless `allow_prerelease`) prereleases.
///
/// Constraint shorthand:
/// - exact:  `"0.1.0"`        — only `0.1.0`.
/// - caret:  `"^0.1.0"`       — `>=0.1.0, <0.2.0` (cargo / npm shape).
/// - tilde:  `"~0.1.0"`       — `>=0.1.0, <0.2.0` (loosened to match
///   cargo's tilde for 0.x).
/// - range:  `">=0.1, <0.3"`  — comma-separated bounds.
/// - any:    `"*"` or empty   — any non-yanked, non-prerelease.
///
/// Picks the **highest** satisfying version.
#[must_use]
pub fn find_best_version<'a>(
    pkg: &'a Package,
    constraint: &str,
    allow_prerelease: bool,
) -> Option<&'a Version> {
    let yanked: std::collections::HashSet<&str> = pkg.yanked.iter().map(String::as_str).collect();
    let mut best: Option<&Version> = None;
    for ver in pkg.versions.values() {
        if yanked.contains(ver.semver.as_str()) {
            continue;
        }
        if ver.prerelease && !allow_prerelease {
            continue;
        }
        if !satisfies(&ver.semver, constraint) {
            continue;
        }
        if best
            .is_none_or(|b| compare_semver(&ver.semver, &b.semver) == std::cmp::Ordering::Greater)
        {
            best = Some(ver);
        }
    }
    best
}

/// Check whether `version` satisfies `constraint`.
#[must_use]
pub fn satisfies(version: &str, constraint: &str) -> bool {
    let c = constraint.trim();
    if c.is_empty() || c == "*" {
        return true;
    }
    if let Some(rest) = c.strip_prefix('^') {
        let parts: Vec<u32> = rest.split('.').filter_map(|p| p.parse().ok()).collect();
        if parts.len() < 2 {
            return false;
        }
        let lo = format!(
            "{}.{}.{}",
            parts[0],
            parts[1],
            parts.get(2).copied().unwrap_or(0)
        );
        let hi = format!("{}.{}.0", parts[0], parts[1].saturating_add(1));
        return compare_semver(version, &lo) != std::cmp::Ordering::Less
            && compare_semver(version, &hi) == std::cmp::Ordering::Less;
    }
    if let Some(rest) = c.strip_prefix('~') {
        let parts: Vec<u32> = rest.split('.').filter_map(|p| p.parse().ok()).collect();
        if parts.len() < 2 {
            return false;
        }
        let lo = format!(
            "{}.{}.{}",
            parts[0],
            parts[1],
            parts.get(2).copied().unwrap_or(0)
        );
        let hi = format!("{}.{}.0", parts[0], parts[1].saturating_add(1));
        return compare_semver(version, &lo) != std::cmp::Ordering::Less
            && compare_semver(version, &hi) == std::cmp::Ordering::Less;
    }
    if c.contains(',') {
        return c.split(',').all(|p| satisfies(version, p.trim()));
    }
    if let Some(rest) = c.strip_prefix(">=") {
        return compare_semver(version, rest.trim()) != std::cmp::Ordering::Less;
    }
    if let Some(rest) = c.strip_prefix('>') {
        return compare_semver(version, rest.trim()) == std::cmp::Ordering::Greater;
    }
    if let Some(rest) = c.strip_prefix("<=") {
        return compare_semver(version, rest.trim()) != std::cmp::Ordering::Greater;
    }
    if let Some(rest) = c.strip_prefix('<') {
        return compare_semver(version, rest.trim()) == std::cmp::Ordering::Less;
    }
    if let Some(rest) = c.strip_prefix('=') {
        return version == rest.trim();
    }
    // Bare version → exact match.
    version == c
}

/// Compare two semver strings.  Treats unparseable parts as `0`.
#[must_use]
pub fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut parts = s.splitn(3, '.');
        let major = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let patch = parts
            .next()
            .map(|x| x.split('-').next().unwrap_or(x))
            .and_then(|x| x.parse().ok())
            .unwrap_or(0);
        (major, minor, patch)
    };
    parse(a).cmp(&parse(b))
}

// ── Cache + URL helpers ───────────────────────────────────────────

/// Default registry URL.  Overridden by `LOFT_REGISTRY_URL` env var.
pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/loft-lang/registry/main/index.json";

/// Resolve the registry URL from env or default.
#[must_use]
pub fn registry_url() -> String {
    std::env::var("LOFT_REGISTRY_URL").unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_string())
}

/// Local cache root (`~/.loft/registry/`).
#[must_use]
pub fn cache_dir() -> PathBuf {
    // @P332: resolve the home base from `LOFT_HOME` FIRST, falling back to
    // `dirs::home_dir()`.  `dirs::home_dir()` reads `$HOME` on Unix but
    // `USERPROFILE` / the FOLDERID_Profile known folder on Windows — it does
    // NOT honour `$HOME` there — so tests that isolate the registry by setting
    // `HOME=<tmpdir>` leaked into the REAL user profile on Windows, and
    // cross-run caching then routed every install to `skipped_cached`
    // (`install_one` saw a stale `loft.toml`), yielding `installed.len()==0`.
    // `LOFT_HOME` is honoured identically on every platform; production leaves
    // it unset, so behaviour there is unchanged.
    let home = std::env::var_os("LOFT_HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".loft").join("registry")
}

/// Paths for the cached index + signature + fetched-at timestamp.
#[must_use]
pub fn index_paths() -> (PathBuf, PathBuf, PathBuf) {
    let dir = cache_dir();
    (
        dir.join("index.json"),
        dir.join("index.json.sig"),
        dir.join("index.json.fetched_at"),
    )
}

/// Directory where a given `<pkg>-<version>` tarball extracts to.
#[must_use]
pub fn extract_dir(pkg: &str, version: &str) -> PathBuf {
    cache_dir().join(format!("{pkg}-{version}"))
}

// ── HTTPS fetcher ─────────────────────────────────────────────────

/// Result of fetching the registry index.
pub struct FetchedIndex {
    pub content: Vec<u8>,
    pub signature: Vec<u8>,
}

/// Fetch `index.json` + `index.json.sig` from the given URL prefix.
///
/// The URL passed in points at the `index.json` itself; the signature
/// is fetched from the same URL with `.sig` appended.
///
/// # Errors
///
/// Returns a `String` error on HTTP failure, non-200 status, or
/// missing signature file (the latter is only an error when
/// signature verification is required; the caller decides).
pub fn fetch_index(url: &str) -> Result<FetchedIndex, String> {
    let content = http_get_bytes(url).map_err(|e| format!("fetching {url}: {e}"))?;
    let sig_url = format!("{url}.sig");
    let signature = http_get_bytes(&sig_url).unwrap_or_default();
    Ok(FetchedIndex { content, signature })
}

/// Download `url` to the local path `dest`, returning the bytes
/// fetched so the caller can hash them.
///
/// # Errors
///
/// Returns a `String` error on HTTP / IO failure.
pub fn download_tarball(url: &str, dest: &std::path::Path) -> Result<Vec<u8>, String> {
    let bytes = http_get_bytes(url).map_err(|e| format!("downloading {url}: {e}"))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    std::fs::write(dest, &bytes).map_err(|e| format!("writing {}: {e}", dest.display()))?;
    Ok(bytes)
}

/// Verify the SHA-256 of `bytes` against `expected_hex` (lowercase).
/// Returns `Ok(())` on match, `Err` with a clear message otherwise.
///
/// # Errors
///
/// Returns `Err` on mismatch.  Lowercases the computed digest before
/// comparing so casing in the index doesn't cause false negatives.
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in actual {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    if hex.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(format!(
            "sha256 mismatch: expected `{expected_hex}`, got `{hex}`"
        ))
    }
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    // @PLAN12 Phase 6.11 — support `file://` URLs for offline
    // mirrors + bundle-import-served indexes.  Same contract as the
    // HTTP path: return the raw bytes at the URL.
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| format!("file:// read error for {path}: {e}"));
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_mins(1))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|e| format!("HTTP error: {e}"))?;
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status} for {url}"));
    }
    let mut buf = Vec::new();
    use std::io::Read as _;
    response
        .into_reader()
        .take(50 * 1024 * 1024) // 50 MB cap on a single response
        .read_to_end(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    Ok(buf)
}

// ── Tarball extraction ────────────────────────────────────────────

/// Extract a gzipped tarball from `tarball_path` into `dest_parent`.
/// The tarball's top-level directory (e.g. `<pkg>-<version>/`) becomes
/// a subdirectory of `dest_parent`.
///
/// # Errors
///
/// IO errors propagate as `String`.  Missing parent dir is created
/// implicitly.
pub fn extract_tarball(
    tarball_path: &std::path::Path,
    dest_parent: &std::path::Path,
) -> Result<(), String> {
    std::fs::create_dir_all(dest_parent)
        .map_err(|e| format!("create {}: {e}", dest_parent.display()))?;
    let f = std::fs::File::open(tarball_path)
        .map_err(|e| format!("open {}: {e}", tarball_path.display()))?;
    let dec = flate2::read::GzDecoder::new(f);
    let mut ar = tar::Archive::new(dec);
    ar.unpack(dest_parent)
        .map_err(|e| format!("extract {}: {e}", tarball_path.display()))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "schema_version": 1,
        "updated": "2026-05-24T08:00:00Z",
        "packages": {
            "crypto": {
                "description": "SHA-256 etc.",
                "categories": ["crypto"],
                "yanked": ["0.1.0"],
                "versions": {
                    "0.1.0": {
                        "url": "https://example.com/crypto-0.1.0.tar.gz",
                        "sha256": "abc",
                        "size": 100,
                        "loft": ">=0.8",
                        "published": "2026-05-01T00:00:00Z"
                    },
                    "0.1.1": {
                        "url": "https://example.com/crypto-0.1.1.tar.gz",
                        "sha256": "def",
                        "size": 110,
                        "loft": ">=0.8",
                        "deps": {"hash": ">=0.1"},
                        "published": "2026-05-15T00:00:00Z"
                    },
                    "0.2.0-beta": {
                        "url": "https://example.com/crypto-0.2.0-beta.tar.gz",
                        "sha256": "ghi",
                        "size": 120,
                        "loft": ">=0.8",
                        "prerelease": true,
                        "published": "2026-05-20T00:00:00Z"
                    }
                }
            }
        }
    }"#;

    #[test]
    fn parses_triggers_field() {
        let doc = r#"{
            "schema_version": 1,
            "updated": "2026-05-31T00:00:00Z",
            "packages": {
                "regex": {
                    "description": "regex",
                    "categories": [],
                    "yanked": [],
                    "versions": {
                        "0.1.0": {
                            "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                            "triggers": ["matches:text", "regex_find:text"],
                            "published": "2026-05-31T00:00:00Z"
                        }
                    }
                }
            }
        }"#;
        let idx = parse_index(doc).expect("parse");
        let v = &idx.packages["regex"].versions["0.1.0"];
        assert_eq!(
            v.triggers,
            vec!["matches:text".to_string(), "regex_find:text".to_string()]
        );
    }

    #[test]
    fn parses_sample_index() {
        let idx = parse_index(SAMPLE).expect("parse");
        assert_eq!(idx.schema_version, 1);
        let crypto = idx.packages.get("crypto").expect("crypto");
        assert_eq!(crypto.categories, vec!["crypto"]);
        assert_eq!(crypto.yanked, vec!["0.1.0"]);
        assert_eq!(crypto.versions.len(), 3);
        let v_011 = crypto.versions.get("0.1.1").expect("0.1.1");
        assert_eq!(v_011.url, "https://example.com/crypto-0.1.1.tar.gz");
        assert_eq!(v_011.size, 110);
        assert_eq!(v_011.deps.get("hash"), Some(&">=0.1".to_string()));
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let bad = r#"{"schema_version": 999, "updated": "x", "packages": {}}"#;
        assert!(parse_index(bad).is_err());
    }

    #[test]
    fn find_best_version_skips_yanked() {
        let idx = parse_index(SAMPLE).expect("parse");
        let crypto = idx.packages.get("crypto").expect("crypto");
        // 0.1.0 is yanked; 0.1.1 is non-prerelease; 0.2.0-beta is prerelease.
        let best = find_best_version(crypto, "^0.1", false).expect("Some");
        assert_eq!(best.semver, "0.1.1");
    }

    #[test]
    fn find_best_version_allows_prerelease() {
        let idx = parse_index(SAMPLE).expect("parse");
        let crypto = idx.packages.get("crypto").expect("crypto");
        let best = find_best_version(crypto, "*", true).expect("Some");
        assert_eq!(best.semver, "0.2.0-beta");
    }

    #[test]
    fn find_best_version_skips_prerelease_by_default() {
        let idx = parse_index(SAMPLE).expect("parse");
        let crypto = idx.packages.get("crypto").expect("crypto");
        let best = find_best_version(crypto, "*", false).expect("Some");
        assert_eq!(best.semver, "0.1.1");
    }

    #[test]
    fn satisfies_caret() {
        assert!(satisfies("0.1.5", "^0.1.0"));
        assert!(satisfies("0.1.0", "^0.1.0"));
        assert!(!satisfies("0.2.0", "^0.1.0"));
        assert!(!satisfies("0.0.9", "^0.1.0"));
    }

    #[test]
    fn satisfies_range() {
        assert!(satisfies("0.2.5", ">=0.2, <0.3"));
        assert!(!satisfies("0.3.0", ">=0.2, <0.3"));
        assert!(!satisfies("0.1.9", ">=0.2, <0.3"));
    }

    #[test]
    fn satisfies_star_or_empty() {
        assert!(satisfies("99.99.99", "*"));
        assert!(satisfies("99.99.99", ""));
    }

    #[test]
    fn compare_semver_basics() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_semver("1.2.3", "1.2.3"), Equal);
        assert_eq!(compare_semver("1.2.3", "1.2.4"), Less);
        assert_eq!(compare_semver("1.3.0", "1.2.99"), Greater);
        assert_eq!(compare_semver("2.0.0", "1.99.99"), Greater);
    }

    #[test]
    fn cache_dir_uses_dot_loft() {
        let dir = cache_dir();
        assert!(dir.ends_with(".loft/registry"));
    }

    #[test]
    fn extract_dir_format() {
        let p = extract_dir("crypto", "0.1.0");
        assert!(p.ends_with("crypto-0.1.0"));
    }

    #[test]
    fn registry_url_respects_env_override() {
        // Don't actually set the env var (cross-test isolation), just
        // confirm the default fallback shape.
        assert_eq!(
            std::env::var("LOFT_REGISTRY_URL")
                .ok()
                .unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string()),
            registry_url()
        );
    }

    // ── verify_sha256 ────────────────────────────────────────────

    /// `b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9`
    /// is the SHA-256 of `b"hello world"` (well-known test vector).
    const HELLO_WORLD_SHA: &str =
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    #[test]
    fn verify_sha256_accepts_match() {
        assert!(verify_sha256(b"hello world", HELLO_WORLD_SHA).is_ok());
    }

    #[test]
    fn verify_sha256_rejects_mismatch() {
        let result = verify_sha256(
            b"hello world",
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        let err = result.expect_err("should reject");
        assert!(err.contains("sha256 mismatch"), "msg: {err}");
        assert!(err.contains(HELLO_WORLD_SHA), "msg: {err}");
    }

    #[test]
    fn verify_sha256_case_insensitive() {
        // Upper-case hex from a hand-written PR should still match.
        let upper = HELLO_WORLD_SHA.to_ascii_uppercase();
        assert!(verify_sha256(b"hello world", &upper).is_ok());
    }

    #[test]
    fn verify_sha256_rejects_corruption() {
        // Flip one byte → mismatch.
        let mut bytes = b"hello world".to_vec();
        bytes[0] ^= 0x01;
        assert!(verify_sha256(&bytes, HELLO_WORLD_SHA).is_err());
    }
}
