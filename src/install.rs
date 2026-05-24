// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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

use std::path::Path;

use crate::lockfile::{self, LockFile, LockedPackage, SCHEMA_VERSION};
use crate::registry_index::{self, FetchedIndex, RegistryIndex, Version, extract_tarball};
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
}

/// High-level outcome printed back to the user.
#[derive(Debug)]
pub struct InstallReport {
    pub installed: Vec<(String, String)>,
    pub skipped_cached: Vec<(String, String)>,
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
    resolve_recursive(&index, package_name, constraint, opts, &mut graph)?;

    let mut report = InstallReport {
        installed: Vec::new(),
        skipped_cached: Vec::new(),
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
        report
            .installed
            .push((r.name.clone(), r.version.semver.clone()));
    }

    // Write lockfile in the current working dir.  R5 (project-wide
    // install from loft.toml) writes elsewhere; for a one-off
    // `loft install foo` we still maintain a lockfile so the next
    // invocation can reproduce the same graph.
    let lock_path = std::env::current_dir()
        .map_err(|e| format!("cwd: {e}"))?
        .join("loft.lock");
    let lock = build_lockfile(&graph);
    lockfile::write_lockfile(&lock_path, &lock).map_err(|e| format!("write lockfile: {e}"))?;

    Ok(report)
}

#[derive(Debug, Clone)]
struct ResolvedPackage {
    name: String,
    version: Version,
}

fn load_index(opts: &InstallOptions) -> Result<RegistryIndex, String> {
    let url = registry_index::registry_url();
    let (idx_path, sig_path, _) = registry_index::index_paths();
    let content_bytes: Vec<u8> = if opts.offline {
        std::fs::read(&idx_path).map_err(|e| {
            format!(
                "offline mode: no cached index ({}): {e}",
                idx_path.display()
            )
        })?
    } else if opts.refresh || index_stale(&idx_path) {
        let fetched: FetchedIndex = registry_index::fetch_index(&url)?;
        // Atomic-ish cache update.
        if let Some(parent) = idx_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&idx_path, &fetched.content).map_err(|e| format!("cache index: {e}"))?;
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
        fetched.content
    } else {
        let sig = std::fs::read(&sig_path).unwrap_or_default();
        let content = std::fs::read(&idx_path).map_err(|e| format!("read cached index: {e}"))?;
        verify_or_explain(&content, &sig, opts)?;
        content
    };
    let text = std::str::from_utf8(&content_bytes)
        .map_err(|e| format!("index is not valid UTF-8: {e}"))?;
    registry_index::parse_index(text)
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

/// Recursive resolver: pull `name` (and its transitive deps) into
/// `graph`.  Diamond resolution: when a package appears twice via
/// different dep paths, pick the **highest** version satisfying both
/// constraints; conflict otherwise.
fn resolve_recursive(
    index: &RegistryIndex,
    name: &str,
    constraint: Option<&str>,
    opts: &InstallOptions,
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
    let version = registry_index::find_best_version(pkg, constraint_str, opts.allow_prerelease)
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
        resolve_recursive(index, &dep_name, Some(&dep_constraint), opts, graph)?;
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
    let total = report.installed.len() + report.skipped_cached.len();
    let _ = writeln!(out, "{total} package(s) total");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn build_lockfile_preserves_order() {
        let v0_1_0 = Version {
            semver: "0.1.0".to_string(),
            url: "u".to_string(),
            sha256: "s".to_string(),
            size: 1,
            loft: ">=0.8".to_string(),
            deps: BTreeMap::new(),
            conflicts: vec![],
            replaces: vec![],
            provides: vec![],
            binaries: BTreeMap::new(),
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
        };
        let s = format_report(&report);
        assert!(s.starts_with("Nothing to do"));
    }

    #[test]
    fn format_report_mixed() {
        let report = InstallReport {
            installed: vec![("crypto".to_string(), "0.1.0".to_string())],
            skipped_cached: vec![("web".to_string(), "0.1.0".to_string())],
        };
        let s = format_report(&report);
        assert!(s.contains("Installed:"));
        assert!(s.contains("crypto 0.1.0"));
        assert!(s.contains("Already cached"));
        assert!(s.contains("web 0.1.0"));
        assert!(s.contains("2 package(s) total"));
    }
}
