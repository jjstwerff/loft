// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Minimal `loft.toml` manifest reader for external package support (T2-11).
//! Phase 1: pure-loft package layout.  A simple line-scanner is sufficient
//! for the three fields needed here; no full TOML parser is required.

/// Content of a library's `loft.toml` manifest file.
#[derive(Debug, Default)]
pub struct Manifest {
    /// Package name from `[package] name = "..."`.
    pub name: Option<String>,
    /// Package version from `[package] version = "..."`.
    pub version: Option<String>,
    /// Entry `.loft` file path, relative to the package root.
    /// Defaults to `src/<name>.loft` when absent.
    pub entry: Option<String>,
    /// Interpreter version requirement from the `[package]` section,
    /// e.g. `">=1.0"`.  `None` means no constraint.
    pub loft_version: Option<String>,
    /// A7.2: native shared-library stem from `[library] native = "..."`.
    /// `None` for pure-loft packages.  The interpreter resolves this to the
    /// platform-correct filename (`lib<stem>.so` / `.dylib` / `.dll`).
    pub native: Option<String>,
    /// PKG.3: package dependencies from `[dependencies]` section.
    /// Key = package name, value = version requirement or path.
    pub dependencies: Vec<(String, String)>,
}

/// Read and parse a `loft.toml` file at `path`.
/// Returns `None` when the file does not exist or cannot be read.
#[must_use]
pub fn read_manifest(path: &str) -> Option<Manifest> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut manifest = Manifest::default();
    let mut section = String::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            match (section.as_str(), key) {
                ("package", "name") => manifest.name = Some(value.to_string()),
                ("package", "version") => manifest.version = Some(value.to_string()),
                ("package", "loft") => manifest.loft_version = Some(value.to_string()),
                ("library", "entry") => manifest.entry = Some(value.to_string()),
                ("library", "native") => manifest.native = Some(value.to_string()),
                ("dependencies", _) => {
                    manifest
                        .dependencies
                        .push((key.to_string(), value.to_string()));
                }
                _ => {}
            }
        }
    }
    Some(manifest)
}

/// Check whether the `required` version constraint is satisfied by `current`.
/// Only `>=X.Y` and `>=X.Y.Z` forms are supported (Phase 1 scope).
/// Returns `true` when `current >= required_version` or `required` is empty.
#[must_use]
pub fn check_version(required: &str, current: &str) -> bool {
    if required.is_empty() {
        return true;
    }
    let req = required.strip_prefix(">=").unwrap_or(required);
    version_ge(current, req)
}

/// Returns `true` when semantic version `a >= b`.
fn version_ge(a: &str, b: &str) -> bool {
    fn parse(s: &str) -> (u32, u32, u32) {
        let mut parts = s.splitn(3, '.');
        let major = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    }
    parse(a) >= parse(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "loft_manifest_test_{}_{}.toml",
            name,
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn parses_loft_version_requirement() {
        let p = write_temp("ver", "[package]\nloft = \">=1.0\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.loft_version.as_deref(), Some(">=1.0"));
    }

    #[test]
    fn parses_custom_entry() {
        let p = write_temp("entry", "[library]\nentry = \"src/mylib.loft\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.entry.as_deref(), Some("src/mylib.loft"));
    }

    #[test]
    fn version_current_passes() {
        assert!(check_version(">=0.1", "0.1.0"));
        assert!(check_version(">=1.0", "1.2.3"));
        assert!(check_version(">=0.1.0", "0.1.0"));
        assert!(check_version("", "0.1.0"));
    }

    #[test]
    fn parses_package_name_and_version() {
        let p = write_temp(
            "pkg",
            "[package]\nname = \"graphics\"\nversion = \"0.2.1\"\nloft = \">=0.8\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.name.as_deref(), Some("graphics"));
        assert_eq!(m.version.as_deref(), Some("0.2.1"));
        assert_eq!(m.loft_version.as_deref(), Some(">=0.8"));
    }

    #[test]
    fn parses_dependencies() {
        let p = write_temp(
            "deps",
            "[dependencies]\nmath = \">=0.2\"\nutils = \"../utils\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.dependencies.len(), 2);
        assert_eq!(m.dependencies[0], ("math".to_string(), ">=0.2".to_string()));
        assert_eq!(
            m.dependencies[1],
            ("utils".to_string(), "../utils".to_string())
        );
    }

    #[test]
    fn version_too_high_fails() {
        assert!(!check_version(">=2.0", "1.9.9"));
        assert!(!check_version(">=1.1", "1.0.0"));
        assert!(!check_version(">=1.0.1", "1.0.0"));
    }
}
