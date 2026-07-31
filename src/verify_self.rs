// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! `loft verify-self` — is this installation the one that was released?
//!
//! @PLN78 step 2.  Read-only: it hashes files and compares, and changes nothing.
//!
//! ## What it can and cannot prove — the distinction the output must keep
//!
//! A release bundle carries two manifests, and both ship INSIDE the bundle they
//! describe:
//!
//! * `stdlib.manifest` — a sha256 per `default/*.loft` plus a `combined` digest over
//!   those lines;
//! * `SHA256SUMS` — a sha256 of every file in the bundle, `bin/loft` included.
//!
//! So they answer **"is this installation intact?"** — not **"is it authentic?"**.
//! Someone who replaced the binary could rewrite the manifest beside it.  That is not
//! a flaw to apologise for, it is the boundary: intactness is what a local check can
//! establish, and it catches the failure modes that actually happen —
//!
//! * a **partial upgrade**: a new `bin/loft` beside an old `default/`.  loft resolves
//!   its stdlib at `<binary-dir>/../default`, so this runs, misbehaves subtly, and
//!   looks like a compiler bug.  `stdlib.manifest` names it in one line.
//! * a truncated or half-written unpack;
//! * an edited stdlib file someone forgot about.
//!
//! Authenticity needs a hash from OUTSIDE the bundle: the signed registry index
//! (@PLN78 step 1b publishes it, `--published` consumes it).  Until that entry
//! exists, this says so rather than implying more than it checked — the reason the
//! output labels every line with what it establishes.

use std::path::{Path, PathBuf};

/// What one check found.  `Skipped` is not a failure: a dev tree legitimately has no
/// release manifest, and saying "not a release bundle" is the useful answer there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Ok(String),
    Failed(String),
    Skipped(String),
}

impl Check {
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(self, Check::Failed(_))
    }
}

/// One `<sha256>  <path>` line of a manifest, as `(path, digest)`.
///
/// Tolerates both `sha256sum` and macOS `shasum -a 256` output (identical format), the
/// `combined  <digest>` trailer `stdlib.manifest` ends with, and blank lines.  Returns
/// the entries and the `combined` digest when present.
#[must_use]
pub fn parse_manifest(text: &str) -> (Vec<(String, String)>, Option<String>) {
    let mut entries = Vec::new();
    let mut combined = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // `sha256sum` writes `<digest>  <path>`; the path may itself contain spaces,
        // so split once and keep the remainder whole.
        let Some((first, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim_start().trim_start_matches('*'); // `*` = binary-mode marker
        if first == "combined" {
            combined = Some(rest.to_string());
        } else {
            entries.push((rest.to_string(), first.to_string()));
        }
    }
    (entries, combined)
}

/// Verify every `<digest>  <path>` entry of `manifest_text` against files under `root`.
///
/// `label` names the manifest in the messages.  A missing file is a failure, not a
/// skip: the manifest says it should be there.
#[must_use]
pub fn check_manifest(root: &Path, manifest_text: &str, label: &str) -> Check {
    let (entries, _) = parse_manifest(manifest_text);
    if entries.is_empty() {
        return Check::Skipped(format!("{label}: no entries"));
    }
    let mut bad: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for (rel, want) in &entries {
        // Never let a manifest path escape the bundle it describes.
        if rel.contains("..") {
            bad.push(format!("{rel} (rejected: path escapes the bundle)"));
            continue;
        }
        let path = root.join(rel);
        match std::fs::read(&path) {
            Ok(bytes) => {
                if crate::integrity::verify_sha256(&bytes, want).is_err() {
                    bad.push(rel.clone());
                }
            }
            Err(_) => missing.push(rel.clone()),
        }
    }
    if bad.is_empty() && missing.is_empty() {
        return Check::Ok(format!("{label}: {} file(s) match", entries.len()));
    }
    let mut parts = Vec::new();
    if !bad.is_empty() {
        parts.push(format!("{} changed: {}", bad.len(), summarise(&bad)));
    }
    if !missing.is_empty() {
        parts.push(format!(
            "{} missing: {}",
            missing.len(),
            summarise(&missing)
        ));
    }
    Check::Failed(format!("{label}: {}", parts.join("; ")))
}

/// First few names, then a count — a wholly-corrupt bundle must not print 50 lines.
fn summarise(names: &[String]) -> String {
    const SHOW: usize = 3;
    if names.len() <= SHOW {
        return names.join(", ");
    }
    format!(
        "{}, and {} more",
        names[..SHOW].join(", "),
        names.len() - SHOW
    )
}

/// The bundle root for a running binary at `exe`: `<binary-dir>/..`, which is where
/// `default/` sits — the same resolution loft uses to find its stdlib, so this checks
/// the stdlib that would actually be LOADED, not one that merely sits nearby.
#[must_use]
pub fn bundle_root(exe: &Path) -> Option<PathBuf> {
    Some(exe.parent()?.parent()?.to_path_buf())
}

/// Run the local (offline) checks for the installation rooted at `root`.
///
/// Returns one `Check` per manifest.  Both absent → a dev tree or a bare binary, and
/// the answer is "not a release bundle", not a failure.
#[must_use]
pub fn local_checks(root: &Path) -> Vec<Check> {
    let mut out = Vec::new();
    for (file, label) in [("stdlib.manifest", "stdlib"), ("SHA256SUMS", "bundle")] {
        let path = root.join(file);
        match std::fs::read_to_string(&path) {
            Ok(text) => out.push(check_manifest(root, &text, label)),
            Err(_) => out.push(Check::Skipped(format!(
                "{label}: no {file} beside the binary — not a release bundle"
            ))),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join("loft-verify-self-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Both manifest dialects and the `combined` trailer parse.
    #[test]
    fn manifest_parses_both_dialects_and_the_combined_trailer() {
        let (e, c) = parse_manifest(
            "abc123  default/01_code.loft\ndef456 *default/02_files.loft\n\ncombined  ff00\n",
        );
        assert_eq!(
            e,
            vec![
                ("default/01_code.loft".to_string(), "abc123".to_string()),
                ("default/02_files.loft".to_string(), "def456".to_string()),
            ]
        );
        assert_eq!(c.as_deref(), Some("ff00"));
    }

    /// An intact bundle passes.
    #[test]
    fn an_intact_bundle_verifies() {
        let d = scratch("intact");
        write(&d, "default/a.loft", "fn main() {}\n");
        let digest = crate::integrity::sha256_hex(b"fn main() {}\n");
        let m = format!("{digest}  default/a.loft\n");
        assert!(matches!(check_manifest(&d, &m, "stdlib"), Check::Ok(_)));
    }

    /// The positive control, and the failure this exists to catch: an edited stdlib
    /// file. A check that cannot fail is not a check.
    #[test]
    fn an_edited_file_fails_and_is_named() {
        let d = scratch("edited");
        write(&d, "default/a.loft", "fn main() {}\n");
        let m = format!(
            "{}  default/a.loft\n",
            crate::integrity::sha256_hex(b"something else\n")
        );
        let Check::Failed(msg) = check_manifest(&d, &m, "stdlib") else {
            panic!("an edited file must FAIL");
        };
        assert!(msg.contains("default/a.loft"), "must name the file: {msg}");
        assert!(msg.contains("changed"), "must say what is wrong: {msg}");
    }

    /// A partial upgrade — the manifest lists a file the installation lost. Reported
    /// as missing rather than silently passing over it.
    #[test]
    fn a_missing_file_fails_as_missing() {
        let d = scratch("missing");
        let m = "00  default/gone.loft\n";
        let Check::Failed(msg) = check_manifest(&d, m, "stdlib") else {
            panic!("a missing file must FAIL");
        };
        assert!(msg.contains("missing"), "{msg}");
        assert!(msg.contains("default/gone.loft"), "{msg}");
    }

    /// A manifest path may not reach outside the bundle it describes.
    #[test]
    fn a_manifest_path_cannot_escape_the_bundle() {
        let d = scratch("escape");
        let m = "00  ../../etc/passwd\n";
        let Check::Failed(msg) = check_manifest(&d, m, "bundle") else {
            panic!("an escaping path must FAIL");
        };
        assert!(msg.contains("escapes the bundle"), "{msg}");
    }

    /// A dev tree has no manifests — "not a release bundle", not a failure.
    #[test]
    fn a_tree_without_manifests_is_skipped_not_failed() {
        let d = scratch("dev");
        let checks = local_checks(&d);
        assert_eq!(checks.len(), 2);
        assert!(
            checks.iter().all(|c| matches!(c, Check::Skipped(_))),
            "{checks:?}"
        );
        assert!(!checks.iter().any(Check::failed));
    }

    /// The stdlib is checked where loft LOADS it from: `<binary-dir>/../default`.
    #[test]
    fn bundle_root_is_the_parent_of_bin() {
        let root = bundle_root(Path::new("/opt/loft/bin/loft")).unwrap();
        assert_eq!(root, Path::new("/opt/loft"));
        assert!(root.join("default").ends_with("loft/default"));
    }
}
