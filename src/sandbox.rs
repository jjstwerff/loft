// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN86 step 1.1 — the sandbox **policy model**: capability-group allow-lists
//! plus the coarse bans, and the `loft.toml` `[sandbox]` / `[profile.*]` parser
//! that builds them.
//!
//! Nothing here executes — it is policy DATA the compile-time admission walk
//! reads (`SandboxProfile::allows` is the central capability check used by the
//! group-membership admission, @PLN86 step 2.3).  Deny-by-default: a capability
//! group is permitted only when an `allow_caps` entry is a dotted-segment prefix
//! of it.

use std::collections::HashMap;

/// A named admission profile: which capability **groups** a sandboxed script may
/// reach, plus the always-on coarse bans (never native/rustc, never a `[native]`
/// cdylib — both RCE surfaces).  Defaults are the safe ones: deny every
/// capability, interpret-only, no FFI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxProfile {
    /// Allowed capability-group prefixes (e.g. `"game"`, `"collections.read"`).
    /// Empty = deny everything.
    pub allow_caps: Vec<String>,
    /// Sandboxed code is always interpreted, never compiled to native via rustc
    /// (generating + compiling Rust on the host is RCE by construction).
    pub interpret_only: bool,
    /// Sandboxed code may never load a `[native]` cdylib.
    pub native_ffi: bool,
}

impl Default for SandboxProfile {
    fn default() -> Self {
        SandboxProfile {
            allow_caps: Vec::new(),
            interpret_only: true,
            native_ffi: false,
        }
    }
}

impl SandboxProfile {
    /// True iff a symbol tagged with capability `group` is allowed by this
    /// profile.  **Deny-by-default**: an empty / untagged group is never allowed,
    /// and a group is allowed only when some `allow_caps` entry is a
    /// dotted-segment prefix of it — so `"game"` allows `"game.read"` but NOT
    /// `"gameover"`, and `"game.read"` is exact (does not match `"game.write"`).
    #[must_use]
    pub fn allows(&self, group: &str) -> bool {
        if group.is_empty() {
            return false;
        }
        self.allow_caps.iter().any(|a| cap_prefix_match(a, group))
    }
}

/// Dotted-segment prefix match: `allow` matches `group` iff `group == allow` or
/// `group` continues `allow` at a `.` boundary.  So `"game"` ⊇ `"game.read"` but
/// not `"gameover"`; `"game.read"` is exact.
#[must_use]
pub fn cap_prefix_match(allow: &str, group: &str) -> bool {
    if allow.is_empty() {
        return false;
    }
    group == allow
        || (group.len() > allow.len()
            && group.starts_with(allow)
            && group.as_bytes()[allow.len()] == b'.')
}

/// The whole sandbox configuration from a program's `loft.toml`: the named
/// profiles plus the designations mapping source selectors (file globs /
/// `fn:<name>`) to a profile.  The host owns this — a script cannot designate
/// itself (@PLN86 §1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxConfig {
    /// profile name -> profile
    pub profiles: HashMap<String, SandboxProfile>,
    /// (selector, profile-name) — which sources compile under which profile.
    /// A selector is a file glob (`"mods/**/*.loft"`) or `"fn:<name>"`.
    pub designations: Vec<(String, String)>,
}

impl SandboxConfig {
    /// True when this config designates any sandboxed source at all.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.designations.is_empty()
    }

    /// The profile a selector maps to, if designated.
    #[must_use]
    pub fn profile_for_selector(&self, selector: &str) -> Option<&SandboxProfile> {
        self.designations
            .iter()
            .find(|(s, _)| s == selector)
            .and_then(|(_, p)| self.profiles.get(p))
    }
}

/// Parse the `[sandbox]` + `[profile.*]` sections of a `loft.toml` body into a
/// [`SandboxConfig`].  Mirrors `manifest::read_manifest`'s hand-rolled line
/// reader (loft.toml is not full TOML); unknown sections/keys are ignored so the
/// sandbox config coexists with the package manifest in one file.
///
/// - `[sandbox]` — `profile-name = ["selector", …]` adds a designation per
///   selector.
/// - `[profile.<name>]` — `allow_caps = ["g", …]`, `backend = "interpret"`,
///   `native_ffi = false`.
#[must_use]
pub fn parse_sandbox_config(content: &str) -> SandboxConfig {
    let mut config = SandboxConfig::default();
    let mut section = String::new();
    for line in content.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_string();
            // A named profile section pre-registers the profile so an empty
            // `[profile.x]` (deny-all) still exists to be designated.
            if let Some(p) = section.strip_prefix("profile.") {
                config.profiles.entry(p.to_string()).or_default();
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if section == "sandbox" {
            // `profile = ["selector", …]` — each selector designates that profile.
            for selector in parse_str_list(value) {
                config.designations.push((selector, key.to_string()));
            }
        } else if let Some(name) = section.strip_prefix("profile.") {
            let profile = config.profiles.entry(name.to_string()).or_default();
            match key {
                "allow_caps" => profile.allow_caps = parse_str_list(value),
                "backend" => profile.interpret_only = unquote(value) == "interpret",
                "native_ffi" => profile.native_ffi = value.trim() == "true",
                _ => {}
            }
        }
    }
    config
}

/// Drop a trailing `# comment` (loft.toml line comments).  Naive — fine for the
/// manifest's simple values, which never contain a literal `#`.
fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(head, _)| head)
}

/// Strip surrounding double quotes from a scalar value.
fn unquote(value: &str) -> &str {
    value.trim().trim_matches('"')
}

/// Parse a value that is either a TOML array `["a", "b"]` or a bare/quoted
/// scalar into a trimmed, unquoted, non-empty list.
fn parse_str_list(value: &str) -> Vec<String> {
    let inner = value
        .trim()
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or(value);
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_match_is_dotted_segment() {
        assert!(cap_prefix_match("game", "game")); // exact
        assert!(cap_prefix_match("game", "game.read")); // child
        assert!(cap_prefix_match("game", "game.entity.write")); // grandchild
        assert!(!cap_prefix_match("game", "gameover")); // NOT a string prefix
        assert!(!cap_prefix_match("game.read", "game.write")); // sibling
        assert!(!cap_prefix_match("game.read", "game")); // parent is not allowed by child
        assert!(!cap_prefix_match("", "game")); // empty allow never matches
    }

    #[test]
    fn allows_is_deny_by_default() {
        let p = SandboxProfile {
            allow_caps: vec!["game.read".into(), "math".into(), "collections.read".into()],
            ..SandboxProfile::default()
        };
        assert!(p.allows("game.read"));
        assert!(p.allows("math")); // exact whole-lib
        assert!(p.allows("math.trig")); // child of an allowed prefix
        assert!(p.allows("collections.read"));
        assert!(!p.allows("collections.write")); // sibling — denied
        assert!(!p.allows("fs.read")); // not listed — denied
        assert!(!p.allows("game.write")); // sibling — denied
        assert!(!p.allows("")); // untagged — denied
        // an empty profile denies everything
        assert!(!SandboxProfile::default().allows("math"));
    }

    #[test]
    fn parses_profiles_and_designations() {
        let toml = r#"
[package]
name = "ztgame"

[sandbox]
mod-script = ["mods/**/*.loft", "fn:player_eval"]

[profile.mod-script]
backend = "interpret"
native_ffi = false
allow_caps = ["game.read", "game.write", "math", "collections.read"]
"#;
        let cfg = parse_sandbox_config(toml);
        assert!(cfg.is_active());
        assert_eq!(cfg.designations.len(), 2);
        assert_eq!(
            cfg.designations[0],
            ("mods/**/*.loft".to_string(), "mod-script".to_string())
        );
        assert_eq!(
            cfg.designations[1],
            ("fn:player_eval".to_string(), "mod-script".to_string())
        );
        let p = cfg.profiles.get("mod-script").expect("profile present");
        assert!(p.interpret_only);
        assert!(!p.native_ffi);
        assert!(p.allows("game.write"));
        assert!(p.allows("collections.read"));
        assert!(!p.allows("fs.read")); // not granted
        assert!(!p.allows("collections.write")); // sibling denied
        // resolve a designation to its profile
        assert!(
            cfg.profile_for_selector("mods/**/*.loft")
                .is_some_and(|p| p.allows("math"))
        );
    }

    #[test]
    fn empty_profile_section_registers_a_deny_all_profile() {
        let cfg = parse_sandbox_config("[sandbox]\nlocked = [\"a.loft\"]\n[profile.locked]\n");
        let p = cfg.profiles.get("locked").expect("registered");
        assert!(p.allow_caps.is_empty());
        assert!(!p.allows("game.read")); // deny-all by default
        assert!(p.interpret_only && !p.native_ffi); // safe defaults
    }

    #[test]
    fn no_sandbox_section_is_inactive() {
        let cfg = parse_sandbox_config("[package]\nname = \"x\"\n");
        assert!(!cfg.is_active());
        assert!(cfg.profiles.is_empty());
    }
}
