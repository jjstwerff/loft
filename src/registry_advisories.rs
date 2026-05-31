// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN12 Phase 6.7 — security advisory channel.
//!
//! Sibling to `index.json` in the registry, signed by the same
//! Ed25519 key.  Small + fast-refresh (24 h TTL) — surfaces
//! CVEs against published packages without waiting for the full
//! `index.json` refresh.
//!
//! Schema (see [`security.md`](../doc/claude/lib_plans/12-library-extraction/security.md)):
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "updated": "2026-05-31T12:00:00Z",
//!   "retention_days": 90,
//!   "advisories": [
//!     {
//!       "id": "GHSA-xxxx-yyyy-zzzz",
//!       "packages": [{"name": "web", "affected": ">=0.1.0, <0.1.2", "fixed_in": "0.1.2"}],
//!       "severity": "security_critical",
//!       "summary": "TLS bypass in ws_client_connect",
//!       "published": "2026-05-30T08:00:00Z",
//!       "references": ["https://github.com/loft-lang/loft-libs-net/security/advisories/..."]
//!     }
//!   ]
//! }
//! ```
//!
//! `"package": "loft"` entries cover the loft binary itself —
//! same schema; same classifier.  See
//! [`lib-plan 30 § Phase 30.4`](../doc/claude/lib_plans/future/30-loft-distribution/README.md)
//! for the binary-side flow.

#![cfg(feature = "registry")]

use crate::json::{Parsed, parse as parse_json};
use crate::registry_index;

/// Severity tier carried by each advisory.  Drives the loft-binary
/// classifier output ([`Classification::action`]):
/// - `SecurityCritical` → refuse to run.
/// - `SecurityHigh` → warn loudly; non-zero exit only under
///   `--strict-security`.
/// - `SecurityLow` / `Bug` → one-line warning per run.
/// - `Deprecated` → one-line note (suppressed by daily cadence
///   state once implemented).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    SecurityCritical,
    SecurityHigh,
    SecurityLow,
    Bug,
    Deprecated,
}

impl Severity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SecurityCritical => "security_critical",
            Self::SecurityHigh => "security_high",
            Self::SecurityLow => "security_low",
            Self::Bug => "bug",
            Self::Deprecated => "deprecated",
        }
    }

    /// Rank for "worst severity" reductions in `loft audit` exit-code
    /// computation.  Higher = worse.
    #[must_use]
    pub fn rank(self) -> u8 {
        match self {
            Self::Deprecated => 1,
            Self::Bug | Self::SecurityLow => 2,
            Self::SecurityHigh => 3,
            Self::SecurityCritical => 4,
        }
    }
}

fn parse_severity(s: &str) -> Result<Severity, String> {
    match s {
        "security_critical" => Ok(Severity::SecurityCritical),
        "security_high" => Ok(Severity::SecurityHigh),
        "security_low" => Ok(Severity::SecurityLow),
        "bug" => Ok(Severity::Bug),
        "deprecated" => Ok(Severity::Deprecated),
        other => Err(format!("unknown severity `{other}`")),
    }
}

/// A package + version-range pair affected by an advisory.
#[derive(Debug, Clone)]
pub struct AdvisoryPackage {
    pub name: String,
    /// loft.toml-style range expression (`>=0.1.0, <0.1.2`).
    pub affected: String,
    /// Version the fix landed in (informational; printed to the
    /// user as "fix: <pkg> >= <ver>").  `None` when no fix is yet
    /// published.
    pub fixed_in: Option<String>,
}

/// A single advisory row.
#[derive(Debug, Clone)]
pub struct Advisory {
    /// GHSA / CVE / similar identifier — opaque string used as
    /// the cross-reference key from [`crate::registry_index`]'s
    /// per-version typed `status.advisory` field.
    pub id: String,
    /// Packages (one or more) affected by this advisory.
    pub packages: Vec<AdvisoryPackage>,
    pub severity: Severity,
    pub summary: String,
    /// ISO-8601 UTC timestamp.
    pub published: String,
    /// External URLs (NVD, GHSA, GitHub Security Advisory, etc.).
    pub references: Vec<String>,
}

/// Parsed `advisories.json`.
#[derive(Debug, Clone)]
pub struct AdvisoryFeed {
    pub schema_version: u32,
    pub updated: String,
    pub retention_days: u32,
    pub advisories: Vec<Advisory>,
}

/// Match between a (name, version) tuple and an advisory entry.
#[derive(Debug, Clone)]
pub struct Classification {
    pub package: String,
    pub version: String,
    pub advisory_id: String,
    pub severity: Severity,
    pub summary: String,
    pub fixed_in: Option<String>,
    pub references: Vec<String>,
}

/// Parse the JSON-serialised advisory feed.
///
/// # Errors
/// Surfaces JSON parse errors + missing required fields with the
/// path that failed.
pub fn parse_advisories(content: &str) -> Result<AdvisoryFeed, String> {
    let parsed = parse_json(content).map_err(|e| format!("JSON parse error: {e:?}"))?;
    let Parsed::Object(root) = parsed else {
        return Err("advisories.json: top-level must be an object".to_string());
    };
    let mut schema_version: Option<u32> = None;
    let mut updated: Option<String> = None;
    let mut retention_days: Option<u32> = None;
    let mut advisories: Vec<Advisory> = Vec::new();
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
            "retention_days" => {
                if let Parsed::Number(n) = v {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        retention_days = Some(*n as u32);
                    }
                }
            }
            "advisories" => {
                let Parsed::Array(arr) = v else {
                    return Err("advisories.json: `advisories` must be an array".to_string());
                };
                for entry in arr {
                    advisories.push(parse_advisory(entry)?);
                }
            }
            _ => {
                // Forward-compat: silently ignore unknown top-level keys.
            }
        }
    }
    let schema_version = schema_version
        .ok_or_else(|| "advisories.json: missing `schema_version`".to_string())?;
    if schema_version != 1 {
        return Err(format!(
            "advisories.json: schema_version {schema_version} unsupported — upgrade loft"
        ));
    }
    Ok(AdvisoryFeed {
        schema_version,
        updated: updated.unwrap_or_default(),
        retention_days: retention_days.unwrap_or(90),
        advisories,
    })
}

fn parse_advisory(val: &Parsed) -> Result<Advisory, String> {
    let Parsed::Object(fields) = val else {
        return Err("advisory entry: expected object".to_string());
    };
    let mut id: Option<String> = None;
    let mut packages: Vec<AdvisoryPackage> = Vec::new();
    let mut severity: Option<Severity> = None;
    let mut summary: Option<String> = None;
    let mut published: Option<String> = None;
    let mut references: Vec<String> = Vec::new();
    for (k, _, v) in fields {
        match k.as_str() {
            "id" => {
                if let Parsed::Str(s) = v {
                    id = Some(s.clone());
                }
            }
            "packages" => {
                let Parsed::Array(arr) = v else {
                    return Err("advisory `packages` must be an array".to_string());
                };
                for entry in arr {
                    packages.push(parse_advisory_package(entry)?);
                }
            }
            "severity" => {
                if let Parsed::Str(s) = v {
                    severity = Some(parse_severity(s)?);
                }
            }
            "summary" => {
                if let Parsed::Str(s) = v {
                    summary = Some(s.clone());
                }
            }
            "published" => {
                if let Parsed::Str(s) = v {
                    published = Some(s.clone());
                }
            }
            "references" => {
                if let Parsed::Array(arr) = v {
                    for entry in arr {
                        if let Parsed::Str(s) = entry {
                            references.push(s.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let id = id.ok_or_else(|| "advisory: missing `id`".to_string())?;
    let severity = severity.ok_or_else(|| format!("advisory `{id}`: missing `severity`"))?;
    let summary = summary.ok_or_else(|| format!("advisory `{id}`: missing `summary`"))?;
    let published = published.ok_or_else(|| format!("advisory `{id}`: missing `published`"))?;
    if packages.is_empty() {
        return Err(format!("advisory `{id}`: must list at least one affected package"));
    }
    Ok(Advisory {
        id,
        packages,
        severity,
        summary,
        published,
        references,
    })
}

fn parse_advisory_package(val: &Parsed) -> Result<AdvisoryPackage, String> {
    let Parsed::Object(fields) = val else {
        return Err("advisory package entry: expected object".to_string());
    };
    let mut name: Option<String> = None;
    let mut affected: Option<String> = None;
    let mut fixed_in: Option<String> = None;
    for (k, _, v) in fields {
        match k.as_str() {
            "name" => {
                if let Parsed::Str(s) = v {
                    name = Some(s.clone());
                }
            }
            "affected" => {
                if let Parsed::Str(s) = v {
                    affected = Some(s.clone());
                }
            }
            "fixed_in" => {
                if let Parsed::Str(s) = v {
                    fixed_in = Some(s.clone());
                }
            }
            _ => {}
        }
    }
    let name = name.ok_or_else(|| "advisory package: missing `name`".to_string())?;
    let affected = affected
        .ok_or_else(|| format!("advisory package `{name}`: missing `affected` range"))?;
    Ok(AdvisoryPackage {
        name,
        affected,
        fixed_in,
    })
}

/// Classify a single (name, version) tuple against the feed.
///
/// Returns one [`Classification`] per matching advisory — multiple
/// advisories can hit the same package; the caller prints them all
/// (one line each per the design's open-question recommendation).
#[must_use]
pub fn classify(name: &str, version: &str, feed: &AdvisoryFeed) -> Vec<Classification> {
    let mut out: Vec<Classification> = Vec::new();
    for adv in &feed.advisories {
        for pkg in &adv.packages {
            if pkg.name != name {
                continue;
            }
            if !registry_index::satisfies(version, &pkg.affected) {
                continue;
            }
            out.push(Classification {
                package: name.to_string(),
                version: version.to_string(),
                advisory_id: adv.id.clone(),
                severity: adv.severity,
                summary: adv.summary.clone(),
                fixed_in: pkg.fixed_in.clone(),
                references: adv.references.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
  "schema_version": 1,
  "updated": "2026-05-31T12:00:00Z",
  "retention_days": 90,
  "advisories": [
    {
      "id": "GHSA-test-aaaa-bbbb",
      "packages": [{"name": "web", "affected": ">=0.1.0, <0.1.2", "fixed_in": "0.1.2"}],
      "severity": "security_critical",
      "summary": "TLS bypass in ws_client_connect",
      "published": "2026-05-30T08:00:00Z",
      "references": ["https://example.com/advisory"]
    },
    {
      "id": "GHSA-test-cccc-dddd",
      "packages": [{"name": "gridmesh", "affected": ">=0.1.0, <0.1.1", "fixed_in": "0.1.1"}],
      "severity": "bug",
      "summary": "Off-by-one in step_y",
      "published": "2026-05-29T08:00:00Z",
      "references": []
    }
  ]
}"#;

    #[test]
    fn parses_fixture() {
        let feed = parse_advisories(FIXTURE).expect("parse");
        assert_eq!(feed.schema_version, 1);
        assert_eq!(feed.retention_days, 90);
        assert_eq!(feed.advisories.len(), 2);
        assert_eq!(feed.advisories[0].id, "GHSA-test-aaaa-bbbb");
        assert_eq!(feed.advisories[0].severity, Severity::SecurityCritical);
        assert_eq!(feed.advisories[0].packages.len(), 1);
        assert_eq!(feed.advisories[0].packages[0].name, "web");
        assert_eq!(
            feed.advisories[0].packages[0].fixed_in.as_deref(),
            Some("0.1.2")
        );
    }

    #[test]
    fn classifies_affected_version() {
        let feed = parse_advisories(FIXTURE).unwrap();
        let hits = classify("web", "0.1.0", &feed);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].advisory_id, "GHSA-test-aaaa-bbbb");
        assert_eq!(hits[0].severity, Severity::SecurityCritical);
    }

    #[test]
    fn skips_unaffected_version() {
        let feed = parse_advisories(FIXTURE).unwrap();
        let hits = classify("web", "0.1.2", &feed);
        assert!(hits.is_empty(), "0.1.2 is the fix; should be silent");
    }

    #[test]
    fn skips_unrelated_package() {
        let feed = parse_advisories(FIXTURE).unwrap();
        let hits = classify("crypto", "0.1.0", &feed);
        assert!(hits.is_empty());
    }

    #[test]
    fn rejects_missing_severity() {
        let bad = r#"{
            "schema_version": 1,
            "advisories": [{"id":"X","packages":[{"name":"a","affected":">=0"}],
                            "summary":"s","published":"now"}]
        }"#;
        assert!(parse_advisories(bad).is_err());
    }

    #[test]
    fn severity_rank_orders_correctly() {
        assert!(Severity::SecurityCritical.rank() > Severity::SecurityHigh.rank());
        assert!(Severity::SecurityHigh.rank() > Severity::Bug.rank());
        assert!(Severity::Bug.rank() > Severity::Deprecated.rank());
    }
}
