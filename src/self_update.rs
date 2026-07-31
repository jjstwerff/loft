// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! @PLN78 step 3 — decide what a self-update WOULD do, without doing any of it.
//!
//! The decision is a pure function of (index, running version, host triple), so it is
//! unit-testable against a synthetic index and needs no network, no release, and no
//! privileges.  That matters more than usual here: the registry does not yet carry a
//! toolchain entry (step 1b, an owner action), so this code cannot be exercised against
//! the real index — proving it against a constructed one is the only way to know it is
//! right BEFORE the entry is published, rather than discovering it afterwards.
//!
//! ## What this pins for step 1b
//!
//! Writing the resolver is what forces the entry's shape to be decided, so the
//! decisions live here as executable expectations rather than prose:
//!
//! * the toolchain is the package named [`TOOLCHAIN_PKG`] (`loft`);
//! * a release is installable on a host only if its `binaries` map has an entry for
//!   that **target triple** — a version that exists but was not built for you is
//!   reported as such, not silently skipped, because "no update available" and "no
//!   build for your platform" send a user to different places;
//! * yanked and prerelease versions are excluded by reusing
//!   [`registry_index::find_best_version`], not by a second rule that could drift;
//! * an update is only ever offered UPWARDS.  Everything else here is a report, but a
//!   downgrade is the one outcome that could hand a user a known-vulnerable release,
//!   so the direction is enforced in the planner rather than at the call site.

use crate::registry_index::{Package, RegistryIndex, compare_semver, find_best_version};

/// The registry package name that carries the loft toolchain itself.
pub const TOOLCHAIN_PKG: &str = "loft";

/// What a self-update would do, decided but not done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// The index carries no toolchain entry at all — today's state, until step 1b.
    /// Distinct from `Current` so the report can say "nothing published yet" rather
    /// than implying the running version was checked and found newest.
    NoEntry,
    /// The running version is the newest installable one.
    Current { version: String },
    /// A newer release exists and was built for this host.
    Available {
        from: String,
        to: String,
        url: String,
        sha256: String,
    },
    /// A newer release exists but not for this target triple.  Reported rather than
    /// treated as "up to date", which would be a lie by omission.
    NoBuildForTarget {
        to: String,
        triple: String,
        built_for: Vec<String>,
    },
}

/// Decide what a self-update from `current` on `triple` would do against `index`.
///
/// Pure: no IO, no clock, no environment.
#[must_use]
pub fn plan(index: &RegistryIndex, current: &str, triple: &str) -> Plan {
    let Some(pkg) = index.packages.get(TOOLCHAIN_PKG) else {
        return Plan::NoEntry;
    };
    plan_for_package(pkg, current, triple)
}

fn plan_for_package(pkg: &Package, current: &str, triple: &str) -> Plan {
    // `"*"` = "any version"; `find_best_version` applies the yanked + prerelease rules,
    // which is why they are not restated here.
    let Some(best) = find_best_version(pkg, "*", false) else {
        return Plan::NoEntry;
    };
    // Upwards only.  A registry that offered an older release — through a rollback, a
    // mistake, or tampering that survived signing — must not be able to walk a user
    // back onto a version an advisory already covers.
    if compare_semver(&best.semver, current) != std::cmp::Ordering::Greater {
        return Plan::Current {
            version: current.to_string(),
        };
    }
    match best.binaries.get(triple) {
        Some(bin) => Plan::Available {
            from: current.to_string(),
            to: best.semver.clone(),
            url: bin.url.clone(),
            sha256: bin.sha256.clone(),
        },
        None => Plan::NoBuildForTarget {
            to: best.semver.clone(),
            triple: triple.to_string(),
            built_for: best.binaries.keys().cloned().collect(),
        },
    }
}

/// The host target triple this binary was built for — the key a toolchain entry's
/// `binaries` map is looked up by.
///
/// Composed from the compile-time target rather than probed at runtime: the question is
/// "which build am I", and a running binary knows that exactly.
#[must_use]
pub fn host_triple() -> String {
    // `env!("TARGET")` is not set for ordinary cargo builds, so compose it from the
    // cfg values cargo does define.
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    match os {
        "macos" => format!("{arch}-apple-darwin"),
        "windows" => format!("{arch}-pc-windows-msvc"),
        // Linux ships as musl in the release matrix; `env` differs per build, and the
        // entry is keyed by what was published.
        _ => format!("{arch}-unknown-{os}-gnu"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry_index::{BinaryEntry, Package, RegistryIndex, Version};
    use std::collections::BTreeMap;

    fn version(semver: &str, triples: &[&str]) -> Version {
        let mut binaries = BTreeMap::new();
        for t in triples {
            binaries.insert(
                (*t).to_string(),
                BinaryEntry {
                    url: format!("https://example/loft-{semver}-{t}.zip"),
                    sha256: format!("hash-{semver}-{t}"),
                    loft_ffi_fp: None,
                },
            );
        }
        Version {
            semver: semver.to_string(),
            url: String::new(),
            sha256: String::new(),
            size: 0,
            loft: "*".to_string(),
            api_compatible_with: None,
            data_compatible_with: None,
            deps: BTreeMap::new(),
            conflicts: Vec::new(),
            replaces: Vec::new(),
            provides: Vec::new(),
            triggers: Vec::new(),
            binaries,
            api: Vec::new(),
            prerelease: false,
            published: "2026-07-21T00:00:00Z".to_string(),
        }
    }

    fn index(versions: Vec<Version>, yanked: Vec<&str>) -> RegistryIndex {
        let mut vmap = BTreeMap::new();
        for v in versions {
            vmap.insert(v.semver.clone(), v);
        }
        let mut packages = BTreeMap::new();
        packages.insert(
            TOOLCHAIN_PKG.to_string(),
            Package {
                name: TOOLCHAIN_PKG.to_string(),
                description: None,
                homepage: None,
                categories: Vec::new(),
                yanked: yanked.into_iter().map(str::to_string).collect(),
                versions: vmap,
            },
        );
        RegistryIndex {
            schema_version: 1,
            updated: String::new(),
            packages,
            skipped: Vec::new(),
        }
    }

    const T: &str = "x86_64-unknown-linux-gnu";

    /// Today's state, and it must be distinguishable from "you are up to date" —
    /// nothing has been published, so nothing was checked.
    #[test]
    fn an_index_without_a_toolchain_entry_is_not_up_to_date() {
        let empty = RegistryIndex {
            schema_version: 1,
            updated: String::new(),
            packages: BTreeMap::new(),
            skipped: Vec::new(),
        };
        assert_eq!(plan(&empty, "2026.7.2", T), Plan::NoEntry);
    }

    /// The ordinary offer.
    #[test]
    fn a_newer_release_built_for_this_host_is_offered() {
        let idx = index(
            vec![version("2026.7.2", &[T]), version("2026.8.0", &[T])],
            vec![],
        );
        let Plan::Available {
            from,
            to,
            url,
            sha256,
        } = plan(&idx, "2026.7.2", T)
        else {
            panic!("a newer release must be offered");
        };
        assert_eq!((from.as_str(), to.as_str()), ("2026.7.2", "2026.8.0"));
        assert!(url.contains("2026.8.0"), "{url}");
        assert_eq!(sha256, format!("hash-2026.8.0-{T}"));
    }

    /// Calendar versions must order as versions, not as text — "2026.10.0" is newer
    /// than "2026.9.0" even though it sorts earlier as a string.
    #[test]
    fn calendar_versions_order_numerically() {
        let idx = index(
            vec![version("2026.9.0", &[T]), version("2026.10.0", &[T])],
            vec![],
        );
        let Plan::Available { to, .. } = plan(&idx, "2026.9.0", T) else {
            panic!("2026.10.0 is newer than 2026.9.0");
        };
        assert_eq!(to, "2026.10.0");
    }

    /// Running the newest: no offer.
    #[test]
    fn the_newest_version_reports_current() {
        let idx = index(
            vec![version("2026.7.2", &[T]), version("2026.8.0", &[T])],
            vec![],
        );
        assert_eq!(
            plan(&idx, "2026.8.0", T),
            Plan::Current {
                version: "2026.8.0".to_string()
            }
        );
    }

    /// The direction guard: a registry offering only OLDER releases must never walk a
    /// user backwards, which is the one outcome that could restore a vulnerable build.
    #[test]
    fn an_older_registry_never_downgrades() {
        let idx = index(vec![version("2026.6.0", &[T])], vec![]);
        assert_eq!(
            plan(&idx, "2026.7.2", T),
            Plan::Current {
                version: "2026.7.2".to_string()
            },
            "a lower published version must not be offered as an update"
        );
    }

    /// A yanked newest is skipped — and the rule is `find_best_version`'s, not a copy.
    #[test]
    fn a_yanked_newest_is_not_offered() {
        let idx = index(
            vec![
                version("2026.7.2", &[T]),
                version("2026.8.0", &[T]),
                version("2026.9.0", &[T]),
            ],
            vec!["2026.9.0"],
        );
        let Plan::Available { to, .. } = plan(&idx, "2026.7.2", T) else {
            panic!("the newest non-yanked release must be offered");
        };
        assert_eq!(to, "2026.8.0", "the yanked 2026.9.0 must be skipped");
    }

    /// A release that exists but was not built for this host is its own answer:
    /// "no update" and "no build for your platform" send a user to different places.
    #[test]
    fn a_release_without_a_build_for_this_target_says_so() {
        let idx = index(
            vec![
                version("2026.7.2", &[T]),
                version("2026.8.0", &["aarch64-apple-darwin"]),
            ],
            vec![],
        );
        let Plan::NoBuildForTarget {
            to,
            triple,
            built_for,
        } = plan(&idx, "2026.7.2", T)
        else {
            panic!("must distinguish a missing build from being up to date");
        };
        assert_eq!(to, "2026.8.0");
        assert_eq!(triple, T);
        assert_eq!(built_for, vec!["aarch64-apple-darwin".to_string()]);
    }

    /// The triple is the key the entry is looked up by, so its shape is part of the
    /// contract with step 1b: `<arch>-<vendor>-<os>`.
    #[test]
    fn host_triple_has_the_published_shape() {
        let t = host_triple();
        assert!(t.split('-').count() >= 3, "{t}");
        assert!(t.starts_with(std::env::consts::ARCH), "{t}");
    }
}
