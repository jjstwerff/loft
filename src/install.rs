// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! `loft install` orchestration — drives the full install flow
//! described in [PKG_REGISTRY.md § `loft install` flow](../doc/claude/PKG_REGISTRY.md#loft-install-flow).
//!
//! Covers PKG.REG R4 (single-package install), R5 (project-wide
//! install), R6 (`loft update`), R7 (transitive resolution).  The
//! actual CLI plumbing (`loft install` subcommand arg parsing) lives
//! in `main.rs::install_v2`; this module is the engine.
//!
//! Flow:
//!
//! 1. Resolve registry URL (env override or compiled-in default).
//! 2. Fetch `index.json` + `index.json.sig`; verify signature.
//! 3. Parse the index.
//! 4. Resolve the requested package + version (handling
//!    constraints and transitive deps).
//! 5. For each resolved package: download tarball → verify sha256 →
//!    extract to `~/.loft/registry/<pkg>-<version>/`.
//! 6. Write `loft.lock` reflecting the install graph.

#![cfg(feature = "registry")]

use std::path::{Path, PathBuf};

use crate::lockfile::{self, LockFile, LockedPackage, SCHEMA_VERSION};
use crate::registry_index::{self, RegistryIndex, Version, extract_tarball};
use crate::registry_signing::{self, VerifyResult};

/// Knobs the CLI surface passes through.  Mirrors the flags in
/// PKG_REGISTRY.md § CLI surface.
#[allow(clippy::struct_excessive_bools)] // CLI flag bag; bool fields map directly to `--flag` switches
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Bypass signature verification.  Required when
    /// `TRUSTED_PUBLIC_KEYS` is empty (pre-bootstrap state) AND the
    /// user explicitly opts out; refuses to proceed silently
    /// otherwise.
    pub allow_unsigned: bool,
    /// Force re-fetch of the registry index even if the cache TTL
    /// hasn't expired.  Mirrors `--refresh`.
    pub refresh: bool,
    /// Use cache only; fail if a requested package isn't already
    /// cached or the index is stale.
    pub offline: bool,
    /// Accept prerelease versions when resolving constraints.
    pub allow_prerelease: bool,
    /// Override the lockfile path written by `install_one`.
    /// `None` → default cwd/`loft.lock` (existing behaviour).
    /// `Some(path)` → write/merge into that path instead.
    /// Used by `loft pin <script>` (writes `<script>.loft.lock`
    /// next to the script) and by future project-mode walk-up
    /// resolution (writes to the project root's `loft.lock`).
    pub lock_path: Option<PathBuf>,
    /// Install the package(s) but write NO lockfile.  Set when a resolution
    /// originates INSIDE the registry cache — a transitive dep auto-installed
    /// while parsing an already-cached package (`~/.loft/registry/<pkg>/src`).
    /// There is no consumer project to record against, and the only "project
    /// root" the walk-up finds is the cached dependency's own dir, so the
    /// default would write `loft.lock` INTO the immutable cache — harmless on
    /// Unix (the dir is writable) but an ENOENT that aborts the whole resolution
    /// on Windows.  Skipping the write is correct on every platform: the install
    /// still lands, so `use <dep>` resolves.
    pub skip_lockfile: bool,
}

/// High-level outcome printed back to the user.
#[derive(Debug)]
pub struct InstallReport {
    pub installed: Vec<(String, String)>,
    pub skipped_cached: Vec<(String, String)>,
    /// @PLN21 Phase 5 — per-package native surfacing lines (prebuilt
    /// availability for this host + declared runtime libs), rendered by
    /// `format_report` so a user sees whether `use <pkg>` needs a toolchain.
    pub surface: Vec<String>,
}

/// @PLN21 Phase 4 — opportunistically fetch a prebuilt cdylib for THIS host so
/// the package runs with NO Rust toolchain.  Best-effort: any miss (no entry for
/// the host triple, an fp built against a different loft-ffi, offline, or a
/// download/hash failure) silently leaves only the source, which
/// `auto_build_native` compiles on first use.  On success the cdylib lands at
/// `prebuilt/<triple>/lib<stem>.<ext>` + a `.loft-build-fp` sidecar — exactly
/// where `extensions::resolve_native_lib` looks first (Phase 1).
fn fetch_prebuilt(r: &ResolvedPackage, opts: &InstallOptions) -> bool {
    if opts.offline {
        return false;
    }
    let triple = crate::cache::host_triple();
    let Some(bin) = r.version.binaries.get(&triple) else {
        return false;
    };
    // Only a binary built against THIS loft's loft-ffi ABI is compatible.
    if bin.loft_ffi_fp != Some(crate::cache::loft_ffi_fingerprint()) {
        return false;
    }
    let pkg_dir = registry_index::extract_dir(&r.name, &r.version.semver);
    // The cdylib stem is the package manifest's `[library] native` field.
    let Some(stem) = crate::manifest::read_manifest(&pkg_dir.join("loft.toml").to_string_lossy())
        .and_then(|m| m.native)
    else {
        return false;
    };
    let dir = pkg_dir.join("prebuilt").join(&triple);
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let filename = if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(windows) {
        format!("{stem}.dll")
    } else {
        format!("lib{stem}.so")
    };
    let dest = dir.join(&filename);
    let Ok(bytes) = registry_index::download_tarball(&bin.url, &dest) else {
        return false;
    };
    if registry_index::verify_sha256(&bytes, &bin.sha256).is_err() {
        let _ = std::fs::remove_file(&dest);
        return false;
    }
    // Stamp the sidecar so Phase 1's fp-gated resolve accepts the binary.
    crate::cache::write_native_artifact_fingerprint(&dir, crate::cache::loft_ffi_fingerprint());
    true
}

/// @PLN21 Phase 5 — the prebuilt-availability one-liner for a package: given the
/// host triple, the triples that have a published binary, and whether one was
/// just installed.  Pure (no IO) → unit-tested directly.
fn prebuilt_status(host: &str, available: &[String], installed: bool) -> String {
    if installed {
        format!("prebuilt installed for {host} — no Rust toolchain needed")
    } else if available.iter().any(|t| t == host) {
        format!(
            "a {host} prebuilt exists but was built against a different loft-ffi — \
             builds from source on first use"
        )
    } else if available.is_empty() {
        "no prebuilt published — builds from source on first use (needs rustc)".to_string()
    } else {
        format!(
            "no prebuilt for {host} (available: {}) — builds from source on first use (needs rustc)",
            available.join(", ")
        )
    }
}

/// Install a single named package (with optional version
/// constraint).  Drives steps 1-6 of the flow above.
///
/// `constraint`:
/// - `None` → highest non-yanked non-prerelease version.
/// - `Some("0.1.0")` → exact pin.
/// - `Some("^0.1")` etc. → semver constraint.
///
/// # Errors
///
/// Returns a `String` error on any failure point.  Errors are
/// composed end-user-readable; the caller writes them to stderr.
pub fn install_one(
    package_name: &str,
    constraint: Option<&str>,
    opts: &InstallOptions,
) -> Result<InstallReport, String> {
    let index = load_index(opts)?;
    let mut graph: Vec<ResolvedPackage> = Vec::new();
    resolve_recursive(
        &index,
        package_name,
        constraint,
        opts,
        &held_versions(opts),
        &mut graph,
    )?;

    let mut report = InstallReport {
        installed: Vec::new(),
        skipped_cached: Vec::new(),
        surface: Vec::new(),
    };

    for r in &graph {
        let dir = registry_index::extract_dir(&r.name, &r.version.semver);
        if dir.join("loft.toml").exists() {
            report
                .skipped_cached
                .push((r.name.clone(), r.version.semver.clone()));
            continue;
        }
        let tarball_path =
            registry_index::cache_dir().join(format!("{}-{}.tar.gz", r.name, r.version.semver));
        let bytes = if opts.offline {
            return Err(format!(
                "package `{}-{}` not cached; offline mode refuses to fetch",
                r.name, r.version.semver
            ));
        } else {
            registry_index::download_tarball(&r.version.url, &tarball_path)?
        };
        registry_index::verify_sha256(&bytes, &r.version.sha256)?;
        extract_tarball(&tarball_path, &registry_index::cache_dir())?;
        // Tarball is consumed — remove it to save space.  The
        // extracted dir is the canonical install.
        let _ = std::fs::remove_file(&tarball_path);
        // @PLN21 Phase 4 — best-effort: grab a host-matching prebuilt cdylib so
        // first use needs no toolchain (silently no-ops when none applies).
        let got_prebuilt = fetch_prebuilt(r, opts);
        // @PLN21 Phase 5 — surface whether `use <pkg>` needs a toolchain, plus
        // any declared runtime system libs.
        let host = crate::cache::host_triple();
        let available: Vec<String> = r.version.binaries.keys().cloned().collect();
        report.surface.push(format!(
            "{} {}: {}",
            r.name,
            r.version.semver,
            prebuilt_status(&host, &available, got_prebuilt)
        ));
        if let Some(m) = crate::manifest::read_manifest(&dir.join("loft.toml").to_string_lossy())
            && !m.runtime_libs.is_empty()
        {
            report.surface.push(format!(
                "  runtime libraries: {}",
                m.runtime_libs.join(", ")
            ));
        }
        report
            .installed
            .push((r.name.clone(), r.version.semver.clone()));
    }

    // A cache-internal resolution installs but records nothing — there is no
    // consumer project here, only the cached dependency whose source triggered
    // this (see `InstallOptions::skip_lockfile`).
    if opts.skip_lockfile {
        return Ok(report);
    }

    // Write lockfile.  When a lockfile already exists (e.g. from a
    // previous `loft install <other>`), MERGE this install's graph
    // into it: new entries overwrite same-named entries, others
    // survive.  This lets `loft install crypto` followed by
    // `loft install random` produce a combined lockfile listing both
    // packages — matches expectations from cargo / npm.
    //
    // Path: `opts.lock_path` if set (used by `loft pin <script>`
    // for the sidecar `<script>.loft.lock`); otherwise cwd's
    // `loft.lock` (default for `loft install`).
    let lock_path = match &opts.lock_path {
        Some(p) => p.clone(),
        None => std::env::current_dir()
            .map_err(|e| format!("cwd: {e}"))?
            .join("loft.lock"),
    };
    let mut lock = match lockfile::read_lockfile(&lock_path) {
        Ok(Some(existing)) => existing,
        _ => lockfile::LockFile {
            schema_version: 1,
            packages: Vec::new(),
        },
    };
    let new_lock = build_lockfile(&graph);
    for new_pkg in new_lock.packages {
        if let Some(existing) = lock.packages.iter_mut().find(|p| p.name == new_pkg.name) {
            *existing = new_pkg;
        } else {
            lock.packages.push(new_pkg);
        }
    }
    lockfile::write_lockfile(&lock_path, &lock).map_err(|e| format!("write lockfile: {e}"))?;

    Ok(report)
}

#[derive(Debug, Clone)]
struct ResolvedPackage {
    name: String,
    version: Version,
}

/// The parsed index plus the one signal a read-only discovery command needs:
/// whether a network fetch was attempted and FAILED, so the index was served
/// from a possibly-stale cache as a fallback.  Distinct from a fresh cache hit
/// or explicit `--offline` (neither means "unreachable").
pub struct LoadedIndex {
    pub index: RegistryIndex,
    pub stale_fallback: bool,
}

/// Fetch, verify, and parse the registry index — the single loader every
/// registry-reading command shares (`install`, `search`, `info`, `update`,
/// `pin`).  Honours `LOFT_REGISTRY_URL`; serves the cached
/// `~/.loft/registry/index.json` when it is fresh (1-hour TTL) or when
/// `opts.offline`, otherwise re-fetches and re-caches.  Signature outcomes are
/// surfaced per `opts.allow_unsigned` (see `verify_or_explain`).
///
/// A failed network fetch is a hard error here (the install path wants fresh
/// data); read-only commands that prefer a stale cache over failing use
/// [`load_index_reporting`].
///
/// # Errors
/// Returns `Err` when the index cannot be fetched or read, fails signature
/// verification (subject to `opts.allow_unsigned`), or is not valid JSON.
pub fn load_index(opts: &InstallOptions) -> Result<RegistryIndex, String> {
    load_index_inner(opts, false).map(|l| l.index)
}

/// Like [`load_index`], but reports cache-vs-fresh AND, when the network fetch
/// fails, falls back to a usable cached index instead of erroring (with
/// `stale_fallback = true`).  Used by `loft search` / `loft info` so discovery
/// still works when the registry is unreachable.
///
/// # Errors
/// Returns `Err` when a network fetch fails and there is no cached index to
/// fall back to, or the index fails signature verification or parsing.
pub fn load_index_reporting(opts: &InstallOptions) -> Result<LoadedIndex, String> {
    load_index_inner(opts, true)
}

fn load_index_inner(
    opts: &InstallOptions,
    fallback_on_fetch_failure: bool,
) -> Result<LoadedIndex, String> {
    let url = registry_index::registry_url();
    let (idx_path, sig_path, _) = registry_index::index_paths();
    let (content_bytes, stale_fallback): (Vec<u8>, bool) = if opts.offline {
        let content = std::fs::read(&idx_path).map_err(|e| {
            format!(
                "offline mode: no cached index ({}): {e}",
                idx_path.display()
            )
        })?;
        (content, false)
    } else if opts.refresh || index_stale(&idx_path) {
        match registry_index::fetch_index(&url) {
            Ok(fetched) => {
                // Atomic-ish cache update.
                if let Some(parent) = idx_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&idx_path, &fetched.content)
                    .map_err(|e| format!("cache index: {e}"))?;
                if !fetched.signature.is_empty() {
                    let _ = std::fs::write(&sig_path, &fetched.signature);
                }
                // Verify signature unless explicitly waived.
                let sig_bytes = if fetched.signature.is_empty() {
                    std::fs::read(&sig_path).unwrap_or_default()
                } else {
                    fetched.signature.clone()
                };
                verify_or_explain(&fetched.content, &sig_bytes, opts)?;
                (fetched.content, false)
            }
            Err(fetch_err) => {
                // The fetch failed.  Discovery commands fall back to a cached
                // index if one exists; the install path surfaces the error.
                let cached = fallback_on_fetch_failure
                    .then(|| std::fs::read(&idx_path).ok())
                    .flatten();
                match cached {
                    Some(content) => {
                        let sig = std::fs::read(&sig_path).unwrap_or_default();
                        verify_or_explain(&content, &sig, opts)?;
                        (content, true)
                    }
                    None => return Err(fetch_err),
                }
            }
        }
    } else {
        let sig = std::fs::read(&sig_path).unwrap_or_default();
        let content = std::fs::read(&idx_path).map_err(|e| format!("read cached index: {e}"))?;
        verify_or_explain(&content, &sig, opts)?;
        (content, false)
    };
    let text = std::str::from_utf8(&content_bytes)
        .map_err(|e| format!("index is not valid UTF-8: {e}"))?;
    let index = registry_index::parse_index(text)?;
    Ok(LoadedIndex {
        index,
        stale_fallback,
    })
}

fn verify_or_explain(content: &[u8], sig: &[u8], opts: &InstallOptions) -> Result<(), String> {
    let result = registry_signing::verify_index(content, sig);
    match result {
        VerifyResult::Valid => Ok(()),
        VerifyResult::NoTrustRoot => {
            if opts.allow_unsigned {
                Ok(())
            } else {
                Err(
                    "registry index unsigned and this loft binary has no embedded trust root; \
                     pass --allow-unsigned to proceed at your own risk, or install a loft \
                     release with the registry trust root embedded"
                        .to_string(),
                )
            }
        }
        VerifyResult::Invalid => Err("registry index signature INVALID — refusing to install; \
             this is a hard failure even with --allow-unsigned (the signature \
             exists but doesn't verify against any known key)"
            .to_string()),
        VerifyResult::MalformedSignature => {
            if opts.allow_unsigned {
                Ok(())
            } else {
                Err("registry index signature is malformed or missing; \
                     pass --allow-unsigned to proceed at your own risk"
                    .to_string())
            }
        }
    }
}

fn index_stale(idx_path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(idx_path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let Ok(age) = modified.elapsed() else {
        return true;
    };
    age.as_secs() > 60 * 60 // 1-hour TTL per PKG_REGISTRY.md
}

/// What this project already holds, read from its `loft.lock` — the input step 6
/// needs to tell a first install from an upgrade.
///
/// Empty on every path that has no consumer project to speak of: a
/// cache-internal resolution (`skip_lockfile`), a missing or unreadable
/// lockfile.  Empty means unconstrained, so those paths resolve exactly as they
/// did before.
fn held_versions(opts: &InstallOptions) -> std::collections::BTreeMap<String, String> {
    use std::collections::BTreeMap;
    if opts.skip_lockfile {
        return BTreeMap::new();
    }
    let lock_path = match &opts.lock_path {
        Some(p) => p.clone(),
        None => std::env::current_dir()
            .unwrap_or_default()
            .join("loft.lock"),
    };
    match lockfile::read_lockfile(&lock_path) {
        Ok(Some(l)) => l
            .packages
            .into_iter()
            .map(|p| (p.name, p.version))
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// Recursive resolver: pull `name` (and its transitive deps) into
/// `graph`.  Diamond resolution: when a package appears twice via
/// different dep paths, pick the **highest** version satisfying both
/// constraints; conflict otherwise.
fn resolve_recursive(
    index: &RegistryIndex,
    name: &str,
    constraint: Option<&str>,
    opts: &InstallOptions,
    held: &std::collections::BTreeMap<String, String>,
    graph: &mut Vec<ResolvedPackage>,
) -> Result<(), String> {
    if graph.iter().any(|r| r.name == name) {
        // Already resolved — TODO when conflict detection grows, re-
        // check that the existing pin satisfies the new constraint.
        return Ok(());
    }
    let pkg = index
        .packages
        .get(name)
        .ok_or_else(|| format!("package `{name}` not found in registry"))?;
    let constraint_str = constraint.unwrap_or("*");
    // Step 6: if this project already holds a version, re-resolving is an
    // UPGRADE, and an upgrade must not hand it a release that declared it
    // breaks them.  Nothing held → nothing to be a drop-in for, so the fresh
    // install resolves exactly as before.
    let resolved = registry_index::find_compatible_version(
        pkg,
        constraint_str,
        opts.allow_prerelease,
        held.get(name).map(String::as_str),
    );
    for held_back in &resolved.withheld {
        eprintln!(
            "note: `{name}` {} is available but declares a break past the {} you hold \
             (api_compatible_with = {}); staying compatible.  Ask for it by version to take it.",
            held_back.semver,
            held.get(name).map_or("?", String::as_str),
            held_back.api_compatible_with.as_deref().unwrap_or("?"),
        );
    }
    let version = resolved
        .best
        .ok_or_else(|| {
            format!(
                "no version of `{name}` satisfies constraint `{constraint_str}` \
                 (available: {})",
                pkg.versions.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?
        .clone();

    let dep_pairs: Vec<(String, String)> = version
        .deps
        .iter()
        .map(|(n, c)| (n.clone(), c.clone()))
        .collect();
    graph.push(ResolvedPackage {
        name: name.to_string(),
        version,
    });
    for (dep_name, dep_constraint) in dep_pairs {
        resolve_recursive(index, &dep_name, Some(&dep_constraint), opts, held, graph)?;
    }
    Ok(())
}

fn build_lockfile(graph: &[ResolvedPackage]) -> LockFile {
    let mut lock = LockFile {
        schema_version: SCHEMA_VERSION,
        packages: Vec::with_capacity(graph.len()),
    };
    for r in graph {
        lock.packages.push(LockedPackage {
            name: r.name.clone(),
            version: r.version.semver.clone(),
            url: r.version.url.clone(),
            sha256: r.version.sha256.clone(),
            source: "registry".to_string(),
            deps: r.version.deps.keys().cloned().collect(),
        });
    }
    lock
}

/// Auto-install fallback fired by the parser's `use X;` resolution
/// chain (@PLAN12 Phase 6.6).
///
/// Behaviour:
/// - Loads the registry index (uses cached, refreshes if TTL stale).
/// - If `name` is in the catalog, installs the latest active version
///   (same machinery as `loft install <name>`).
/// - Returns `Ok(Some(report))` on a successful install, `Ok(None)`
///   when `name` is NOT in the catalog (so the parser can fall
///   through to remaining resolution strategies), or `Err` on a real
///   failure (network down on a cold cache, signature mismatch,
///   etc.).
///
/// The caller (parser's `probe_auto_install`) decides whether to
/// announce or stay silent based on the return value.
///
/// # Errors
///
/// Surfaces any error from `install_one` — network failure, sig
/// mismatch, tarball corruption, lockfile write failure.
/// `constraint` is the root project's declared version requirement for `name`
/// (e.g. `Some("=0.1.0")`), threaded through so a source-level `use` honours the
/// consumer's pin instead of always resolving the latest release.  `None` keeps
/// the historical behaviour (newest satisfying `*`).
pub fn auto_install_if_in_catalog(
    name: &str,
    constraint: Option<&str>,
    opts: &InstallOptions,
) -> Result<Option<InstallReport>, String> {
    // Load the index FIRST so we can check membership without
    // committing to a network fetch for the tarball.  load_index
    // honours `opts.offline` — if true, only uses cached index.
    let index = load_index(opts)?;
    if !index.packages.contains_key(name) {
        return Ok(None);
    }
    eprintln!("[registry] resolving {name} from registry");
    let report = install_one(name, constraint, opts)?;
    for (n, v) in &report.installed {
        eprintln!("[registry] installed {n} {v}");
    }
    Ok(Some(report))
}

/// Render a human-readable summary of an install.
#[must_use]
pub fn format_report(report: &InstallReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if report.installed.is_empty() && report.skipped_cached.is_empty() {
        return "Nothing to do.\n".to_string();
    }
    if !report.installed.is_empty() {
        let _ = writeln!(out, "Installed:");
        for (n, v) in &report.installed {
            let _ = writeln!(out, "  {n} {v}");
        }
    }
    if !report.skipped_cached.is_empty() {
        let _ = writeln!(out, "Already cached (skipped):");
        for (n, v) in &report.skipped_cached {
            let _ = writeln!(out, "  {n} {v}");
        }
    }
    // @PLN21 Phase 5 — per-package native surfacing (prebuilt availability +
    // declared runtime libs), so a user sees whether `use <pkg>` needs a toolchain.
    if !report.surface.is_empty() {
        let _ = writeln!(out, "Native:");
        for line in &report.surface {
            let _ = writeln!(out, "  {line}");
        }
    }
    let total = report.installed.len() + report.skipped_cached.len();
    let _ = writeln!(out, "{total} package(s) total");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry_index::{Package, RegistryIndex};
    use std::collections::BTreeMap;

    /// Build a minimal `Version` with placeholder URL/sha/size — the
    /// resolver doesn't touch the network-side fields.
    fn ver(semver: &str, deps: &[(&str, &str)]) -> Version {
        let mut d = BTreeMap::new();
        for (n, c) in deps {
            d.insert((*n).to_string(), (*c).to_string());
        }
        Version {
            semver: semver.to_string(),
            url: format!("https://example.com/{semver}.tar.gz"),
            sha256: "0".repeat(64),
            size: 1,
            loft: ">=0.8".to_string(),
            api_compatible_with: None,
            data_compatible_with: None,
            deps: d,
            conflicts: vec![],
            replaces: vec![],
            provides: vec![],
            triggers: vec![],
            binaries: BTreeMap::new(),
            api: vec![],
            prerelease: false,
            published: "p".to_string(),
        }
    }

    fn pkg(name: &str, versions: Vec<Version>) -> Package {
        let mut vmap = BTreeMap::new();
        for v in versions {
            vmap.insert(v.semver.clone(), v);
        }
        Package {
            name: name.to_string(),
            description: None,
            homepage: None,
            categories: vec![],
            yanked: vec![],
            versions: vmap,
        }
    }

    fn index(pkgs: Vec<Package>) -> RegistryIndex {
        let mut pmap = BTreeMap::new();
        for p in pkgs {
            pmap.insert(p.name.clone(), p);
        }
        RegistryIndex {
            schema_version: 1,
            updated: "now".to_string(),
            packages: pmap,
        }
    }

    fn opts() -> InstallOptions {
        InstallOptions {
            allow_unsigned: true,
            refresh: false,
            offline: false,
            allow_prerelease: false,
            skip_lockfile: false,
            lock_path: None,
        }
    }

    // ── resolve_recursive — linear / diamond / cycle / missing ──

    #[test]
    fn resolves_linear_deps_in_order() {
        let idx = index(vec![
            pkg("a", vec![ver("0.1.0", &[("b", "^0.1")])]),
            pkg("b", vec![ver("0.1.0", &[("c", "^0.1")])]),
            pkg("c", vec![ver("0.1.0", &[])]),
        ]);
        let mut graph = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph).unwrap();
        let names: Vec<&str> = graph.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn resolves_diamond_dedups_shared_node() {
        let idx = index(vec![
            pkg("a", vec![ver("0.1.0", &[("b", "^0.1"), ("c", "^0.1")])]),
            pkg("b", vec![ver("0.1.0", &[("d", "^0.1")])]),
            pkg("c", vec![ver("0.1.0", &[("d", "^0.1")])]),
            pkg("d", vec![ver("0.1.0", &[])]),
        ]);
        let mut graph = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph).unwrap();
        let names: Vec<&str> = graph.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "d", "c"]);
        // Each package appears exactly once.
        let unique: std::collections::HashSet<&&str> = names.iter().collect();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn handles_cycle_without_infinite_loop() {
        // a → b → a — pathological but mustn't hang.
        let idx = index(vec![
            pkg("a", vec![ver("0.1.0", &[("b", "^0.1")])]),
            pkg("b", vec![ver("0.1.0", &[("a", "^0.1")])]),
        ]);
        let mut graph = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph).unwrap();
        let names: Vec<&str> = graph.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn rejects_missing_transitive_dep() {
        let idx = index(vec![pkg("a", vec![ver("0.1.0", &[("b", "^0.1")])])]);
        let mut graph = Vec::new();
        let err = resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph)
            .expect_err("should fail on missing transitive");
        assert!(err.contains("`b` not found"), "msg: {err}");
    }

    #[test]
    fn rejects_unsatisfiable_constraint() {
        let idx = index(vec![pkg("a", vec![ver("0.1.0", &[])])]);
        let mut graph = Vec::new();
        let err = resolve_recursive(
            &idx,
            "a",
            Some("^0.2"),
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .expect_err("should fail on unsatisfiable constraint");
        assert!(
            err.contains("satisfies constraint") && err.contains("^0.2"),
            "msg: {err}"
        );
        assert!(err.contains("0.1.0"), "msg: {err}");
    }

    #[test]
    fn rejects_missing_root_package() {
        let idx = index(vec![]);
        let mut graph = Vec::new();
        let err = resolve_recursive(
            &idx,
            "nope",
            None,
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .expect_err("should fail on unknown package");
        assert!(err.contains("`nope` not found"), "msg: {err}");
    }

    #[test]
    fn resolves_highest_version_by_default() {
        let idx = index(vec![pkg(
            "a",
            vec![ver("0.1.0", &[]), ver("0.1.5", &[]), ver("0.1.2", &[])],
        )]);
        let mut graph = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph).unwrap();
        assert_eq!(graph[0].version.semver, "0.1.5");
    }

    #[test]
    fn exact_pin_overrides_highest() {
        // An exact (`=`) constraint pins the OLDER version even when a newer one
        // exists — the feature behind `glb = "=0.1.0"`: pinning is an option, not
        // "always resolve latest".  `auto_install_if_in_catalog` threads this same
        // constraint through from the root project's manifest.
        let idx = index(vec![pkg(
            "a",
            vec![ver("0.1.0", &[]), ver("0.1.1", &[]), ver("0.2.0", &[])],
        )]);
        let mut graph = Vec::new();
        resolve_recursive(
            &idx,
            "a",
            Some("=0.1.0"),
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .unwrap();
        assert_eq!(graph[0].version.semver, "0.1.0");
        // Sanity: without the pin the newest wins.
        let mut g2 = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut g2).unwrap();
        assert_eq!(g2[0].version.semver, "0.2.0");
    }

    #[test]
    fn resolves_specific_pinned_version() {
        let idx = index(vec![pkg("a", vec![ver("0.1.0", &[]), ver("0.1.5", &[])])]);
        let mut graph = Vec::new();
        resolve_recursive(
            &idx,
            "a",
            Some("0.1.0"),
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .unwrap();
        assert_eq!(graph[0].version.semver, "0.1.0");
    }

    #[test]
    fn build_lockfile_preserves_order() {
        let v0_1_0 = Version {
            semver: "0.1.0".to_string(),
            url: "u".to_string(),
            sha256: "s".to_string(),
            size: 1,
            loft: ">=0.8".to_string(),
            api_compatible_with: None,
            data_compatible_with: None,
            deps: BTreeMap::new(),
            conflicts: vec![],
            replaces: vec![],
            provides: vec![],
            triggers: vec![],
            binaries: BTreeMap::new(),
            api: vec![],
            prerelease: false,
            published: "p".to_string(),
        };
        let graph = vec![
            ResolvedPackage {
                name: "crypto".to_string(),
                version: v0_1_0.clone(),
            },
            ResolvedPackage {
                name: "web".to_string(),
                version: v0_1_0,
            },
        ];
        let lock = build_lockfile(&graph);
        assert_eq!(lock.packages.len(), 2);
        assert_eq!(lock.packages[0].name, "crypto");
        assert_eq!(lock.packages[1].name, "web");
        assert_eq!(lock.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn format_report_empty() {
        let report = InstallReport {
            installed: vec![],
            skipped_cached: vec![],
            surface: vec![],
        };
        let s = format_report(&report);
        assert!(s.starts_with("Nothing to do"));
    }

    #[test]
    fn format_report_mixed() {
        let report = InstallReport {
            installed: vec![("crypto".to_string(), "0.1.0".to_string())],
            skipped_cached: vec![("web".to_string(), "0.1.0".to_string())],
            surface: vec![],
        };
        let s = format_report(&report);
        assert!(s.contains("Installed:"));
        assert!(s.contains("crypto 0.1.0"));
        assert!(s.contains("Already cached"));
        assert!(s.contains("web 0.1.0"));
        assert!(s.contains("2 package(s) total"));
    }

    // @PLN21 Phase 5 — the prebuilt-availability one-liner across its branches.
    #[test]
    fn prebuilt_status_branches() {
        use super::prebuilt_status;
        let host = "x86_64-unknown-linux-gnu";
        assert!(prebuilt_status(host, &[], true).contains("no Rust toolchain needed"));
        assert!(prebuilt_status(host, &[], false).contains("no prebuilt published"));
        let other = vec!["aarch64-apple-darwin".to_string()];
        assert!(prebuilt_status(host, &other, false).contains("available: aarch64-apple-darwin"));
        let same = vec![host.to_string()];
        assert!(prebuilt_status(host, &same, false).contains("different loft-ffi"));
    }
}
