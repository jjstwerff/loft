// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLAN12 Phase 6.12 — exercises the mock-registry fixture through
// the parser + classifier code paths.  Verifies the registry-
// resolution machinery (index parse, advisory classify) works
// offline against a file:// URL.

#![cfg(feature = "registry")]

use loft::registry_advisories::{self, Severity};
use loft::registry_index;

fn fixture_path(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mock-registry")
        .join(rel)
}

#[test]
fn mock_index_parses() {
    let content = std::fs::read_to_string(fixture_path("index.json")).unwrap();
    let idx = registry_index::parse_index(&content).expect("parse mock index");
    assert!(idx.packages.contains_key("test_alpha"));
    assert!(idx.packages.contains_key("test_beta"));
    let alpha = &idx.packages["test_alpha"];
    assert_eq!(alpha.yanked, vec!["0.1.0".to_string()]);
    assert_eq!(alpha.versions.len(), 2);
}

#[test]
fn mock_advisories_classify() {
    let content = std::fs::read_to_string(fixture_path("advisories.json")).unwrap();
    let feed = registry_advisories::parse_advisories(&content).expect("parse mock advisories");
    assert_eq!(feed.advisories.len(), 2);

    let critical_hits = registry_advisories::classify("test_alpha", "0.1.0", &feed);
    assert_eq!(critical_hits.len(), 1);
    assert_eq!(critical_hits[0].severity, Severity::SecurityCritical);

    let bug_hits = registry_advisories::classify("test_beta", "0.1.0", &feed);
    assert_eq!(bug_hits.len(), 1);
    assert_eq!(bug_hits[0].severity, Severity::Bug);

    // Fixed version: no hits.
    let fixed = registry_advisories::classify("test_alpha", "0.2.0", &feed);
    assert!(fixed.is_empty(), "0.2.0 is the fix; should be silent");
}

#[test]
fn find_best_version_skips_yanked() {
    let content = std::fs::read_to_string(fixture_path("index.json")).unwrap();
    let idx = registry_index::parse_index(&content).unwrap();
    let alpha = &idx.packages["test_alpha"];
    // 0.1.0 is yanked → best is 0.2.0.
    let best =
        registry_index::find_best_version(alpha, "*", false).expect("at least one non-yanked");
    assert_eq!(best.semver, "0.2.0");
}

#[test]
fn file_url_fetches_local_index() {
    // The http_get_bytes file:// branch should let
    // registry_index::fetch_index work against a file:// URL.
    let url = format!("file://{}", fixture_path("index.json").display());
    let fetched = registry_index::fetch_index(&url).expect("file:// fetch");
    assert!(!fetched.content.is_empty());
    let text = std::str::from_utf8(&fetched.content).unwrap();
    let idx = registry_index::parse_index(text).expect("re-parse");
    assert!(idx.packages.contains_key("test_alpha"));
}
