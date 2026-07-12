// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 C1 commit 4 — the STRICT, tier-aware API diff: two canonical surfaces
//! ([`crate::api_surface::Member`]) → a [`Verdict`]. **Identical-or-added is the whole
//! rule:** every existing public symbol present byte-for-byte + additions-only → `Superset`
//! (a drop-in); ANY change to an existing public symbol → `Break`, naming it. No "compatible
//! widening" grace — that IS the strictness.
//!
//! Tier-aware, via **sealed inlining.** A consumer can neither name nor construct a *sealed*
//! type (only read a value's fields through a public signature that returns it), so a sealed
//! type's identity is its SHAPE, not its name. Before comparing, each sealed type's signature
//! is inlined (recursively) into the public signatures that mention it. Then only PUBLIC
//! members are compared, by name:
//! - a **sealed rename** (same shape) leaves every inlined public signature unchanged → no
//!   break — the sealed granularity stays usable without over-flagging;
//! - a **sealed field change** (different shape) flows into every public signature that
//!   returns it → break, named on the public members a consumer actually calls (the break
//!   points C-app later intersects with a consumer's usage).

use crate::api_surface::{Member, Tier};
use std::collections::HashMap;

/// The result of diffing an old surface against a new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A drop-in: every existing public symbol is unchanged; only additions.
    Superset,
    /// A break: the named public symbols were removed or changed.
    Break(Vec<String>),
}

/// Diff `old` against `new`. Additions never break; a removed or changed existing public
/// symbol does (with sealed shapes inlined, so a sealed rename is not a break).
#[must_use]
pub fn diff(old: &[Member], new: &[Member]) -> Verdict {
    let old_pub = inlined_public(old);
    let new_pub = inlined_public(new);
    let new_map: HashMap<(&str, &str), &str> = new_pub
        .iter()
        .map(|(n, k, s)| ((n.as_str(), *k), s.as_str()))
        .collect();

    let mut breaks = Vec::new();
    for (name, kind, sig) in &old_pub {
        match new_map.get(&(name.as_str(), *kind)) {
            None => breaks.push(format!("removed {kind} `{name}`")),
            Some(new_sig) if *new_sig != sig.as_str() => {
                breaks.push(format!("changed {kind} `{name}`"));
            }
            Some(_) => {}
        }
    }
    if breaks.is_empty() {
        Verdict::Superset
    } else {
        Verdict::Break(breaks)
    }
}

/// The PUBLIC members of a surface, each with sealed types inlined into its signature (so a
/// sealed rename is invisible and a sealed field change flows through).
fn inlined_public(surface: &[Member]) -> Vec<(String, &'static str, String)> {
    let sealed: HashMap<&str, &str> = surface
        .iter()
        .filter(|m| m.tier == Tier::Sealed)
        .map(|m| (m.name.as_str(), m.signature.as_str()))
        .collect();
    surface
        .iter()
        .filter(|m| m.tier == Tier::Public)
        .map(|m| (m.name.clone(), m.kind, inline(&m.signature, &sealed, 0)))
        .collect()
}

/// Replace each identifier in `sig` that names a sealed type with that type's (recursively
/// inlined) shape. `depth` bounds recursive/self-referential sealed types.
fn inline(sig: &str, sealed: &HashMap<&str, &str>, depth: u8) -> String {
    if depth > 16 {
        return sig.to_string();
    }
    let mut out = String::new();
    let mut ident = String::new();
    for c in sig.chars() {
        if c.is_alphanumeric() || c == '_' {
            ident.push(c);
        } else {
            push_ident(&ident, sealed, depth, &mut out);
            ident.clear();
            out.push(c);
        }
    }
    push_ident(&ident, sealed, depth, &mut out);
    out
}

fn push_ident(ident: &str, sealed: &HashMap<&str, &str>, depth: u8, out: &mut String) {
    if ident.is_empty() {
        return;
    }
    if let Some(shape) = sealed.get(ident) {
        out.push_str(&inline(shape, sealed, depth + 1));
    } else {
        out.push_str(ident);
    }
}

#[cfg(test)]
mod tests {
    use super::{Verdict, diff};
    use crate::api_surface::{Member, Tier};

    fn pubm(name: &str, kind: &'static str, sig: &str) -> Member {
        Member {
            name: name.to_string(),
            kind,
            tier: Tier::Public,
            signature: sig.to_string(),
        }
    }
    fn sealed(name: &str, kind: &'static str, sig: &str) -> Member {
        Member {
            name: name.to_string(),
            kind,
            tier: Tier::Sealed,
            signature: sig.to_string(),
        }
    }
    fn is_break(v: &Verdict) -> bool {
        matches!(v, Verdict::Break(_))
    }

    #[test]
    fn pure_additions_are_a_superset() {
        let old = vec![pubm("make", "fn", "() -> integer")];
        let new = vec![
            pubm("make", "fn", "() -> integer"),
            pubm("extra", "fn", "(a: integer) -> integer"),
        ];
        assert_eq!(diff(&old, &new), Verdict::Superset);
    }

    #[test]
    fn public_rename_is_a_break() {
        let old = vec![pubm("make", "fn", "() -> integer")];
        let new = vec![pubm("produce", "fn", "() -> integer")];
        assert!(is_break(&diff(&old, &new)), "renaming a public fn breaks");
    }

    #[test]
    fn a_safe_widening_still_breaks() {
        // strict: ANY change to an existing public symbol is a break, no "compatible" grace.
        let old = vec![pubm("f", "fn", "(a: integer) -> integer")];
        let new = vec![pubm("f", "fn", "(a: number) -> integer")];
        assert!(is_break(&diff(&old, &new)), "a widened param still breaks");
    }

    #[test]
    fn sealed_rename_is_not_a_break() {
        // `make` returns a sealed type; rename the sealed type (same shape). The inlined
        // public signature is unchanged, so it is a drop-in.
        let old = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: integer }"),
        ];
        let new = vec![
            pubm("make", "fn", "() -> Gadget"),
            sealed("Gadget", "struct", "{ x: integer }"),
        ];
        assert_eq!(
            diff(&old, &new),
            Verdict::Superset,
            "a sealed rename with an unchanged shape is a drop-in"
        );
    }

    #[test]
    fn sealed_field_change_is_a_break() {
        let old = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: integer }"),
        ];
        let new = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: text }"),
        ];
        assert!(
            is_break(&diff(&old, &new)),
            "a sealed field change flows into the public sig and breaks"
        );
    }

    #[test]
    fn sealed_field_change_names_the_public_break_point() {
        // The break must name the PUBLIC member a consumer calls (`make`), not the sealed type.
        let old = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: integer }"),
        ];
        let new = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: integer, y: text }"),
        ];
        match diff(&old, &new) {
            Verdict::Break(symbols) => {
                assert!(
                    symbols.iter().any(|s| s.contains("make")),
                    "names make: {symbols:?}"
                );
            }
            Verdict::Superset => panic!("expected a break"),
        }
    }

    #[test]
    fn nested_sealed_field_change_breaks_transitively() {
        // `f` returns sealed A; A has a field of sealed B; changing B's field must break `f`.
        let old = vec![
            pubm("f", "fn", "() -> A"),
            sealed("A", "struct", "{ b: B }"),
            sealed("B", "struct", "{ n: integer }"),
        ];
        let new = vec![
            pubm("f", "fn", "() -> A"),
            sealed("A", "struct", "{ b: B }"),
            sealed("B", "struct", "{ n: text }"),
        ];
        assert!(
            is_break(&diff(&old, &new)),
            "a nested sealed field change breaks transitively"
        );
    }
}
