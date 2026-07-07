// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN97 Phase D — the schema-description sidecar: a store's self-describing
//! layout identity.
//!
//! loft's store is ONE format — the durable file is bit-for-bit identical to the
//! in-memory store. This module does NOT touch that payload. It produces a
//! separate, self-describing descriptor (the `.dschema` sidecar) written BESIDE
//! the store, so a later build can read what layout the store was written under
//! and decide the reload/persistence **handoff**: hand over the raw store when
//! the layout is unchanged, or serialize-before-handoff / migrate when it moved
//! ([README](../doc/claude/plans/97-layout-contract/README.md)).
//!
//! The version/identity is the RUNNING program's live type table (the in-memory
//! source of truth); the sidecar merely records it.

use crate::database::Stores;

/// The self-describing layout identity of a store: the compact
/// [`layout_algo_hash`](Stores::layout_algo_hash) (the quick identical-check) +
/// the per-type storage-layout dump (the basis for the change diff). This is the
/// content of the `.dschema` sidecar — the store payload is never touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutIdentity {
    /// Stable hash of the storage layout — changes iff the byte layout changes.
    pub layout_hash: u64,
    /// The per-type storage-layout dump (`Stores::layout_dump`) — one line per
    /// type, keyed by name; the basis for the added/dropped/changed diff.
    pub dump: String,
}

const SIDECAR_MAGIC: &str = "LOFT-DSCHEMA";
const SIDECAR_VERSION: u32 = 1;

impl LayoutIdentity {
    /// Build the identity from the live type table (the in-memory source of
    /// truth) for `roots` and every type they reference.
    #[must_use]
    pub fn of(stores: &Stores, roots: &[u16]) -> LayoutIdentity {
        LayoutIdentity {
            layout_hash: stores.layout_algo_hash(roots),
            dump: stores.layout_dump(roots),
        }
    }

    /// Serialize to the `.dschema` sidecar text — a SEPARATE file beside the
    /// store (the only on-disk addition; the payload stays byte-identical).
    #[must_use]
    pub fn to_sidecar(&self) -> String {
        format!(
            "{SIDECAR_MAGIC} v{SIDECAR_VERSION}\nhash={}\n--\n{}",
            self.layout_hash, self.dump
        )
    }

    /// Parse a `.dschema` sidecar. `None` on a bad magic / unknown version /
    /// malformed content — the caller treats that as an incompatible store
    /// (reject-and-rebuild), never a silent misread.
    #[must_use]
    pub fn from_sidecar(text: &str) -> Option<LayoutIdentity> {
        let mut parts = text.splitn(3, '\n');
        let (magic, ver) = parts.next()?.split_once(" v")?;
        if magic != SIDECAR_MAGIC || ver.parse::<u32>().ok()? != SIDECAR_VERSION {
            return None;
        }
        let layout_hash = parts.next()?.strip_prefix("hash=")?.parse::<u64>().ok()?;
        let dump = parts.next()?.strip_prefix("--\n")?.to_string();
        Some(LayoutIdentity { layout_hash, dump })
    }
}

/// The per-type layout diff between a stored identity (OLD) and the running
/// program's identity (NEW), keyed by type name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayoutDiff {
    /// Type names present in NEW only.
    pub added: Vec<String>,
    /// Type names present in OLD only.
    pub dropped: Vec<String>,
    /// Type names in both whose storage layout differs (a reshape).
    pub changed: Vec<String>,
}

impl LayoutDiff {
    /// The **actionable** state (Phase F): a diff that reshapes a type, or that
    /// carries BOTH an add and a drop, cannot be told from a rename / a data
    /// migration — so the compiler must surface it (diagnostic + migration
    /// outline) rather than silently defaulting/dropping. A pure add-only or
    /// drop-only diff is handled automatically (lenient add / drop).
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        !self.changed.is_empty() || (!self.added.is_empty() && !self.dropped.is_empty())
    }
}

/// The reload/persistence handoff decision, comparing a stored identity (OLD,
/// from the sidecar) with the running program's identity (NEW).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handoff {
    /// Byte layout unchanged — hand over the RAW store (zero-copy, no migration).
    Identical,
    /// Layout changed — serialize-before-handoff. The diff drives Phase E
    /// (Drop&Add / migration) and Phase F (the compiler aid).
    Changed(LayoutDiff),
}

/// Compare OLD (from the sidecar) with NEW (the running program). Equal hash +
/// dump → [`Handoff::Identical`] (the raw-handoff fast path); otherwise a
/// structured per-type diff. This is the handoff decision variable @PLN97 exists
/// to make sound — never a silent raw handoff across a changed layout.
#[must_use]
pub fn classify(old: &LayoutIdentity, new: &LayoutIdentity) -> Handoff {
    if old.layout_hash == new.layout_hash && old.dump == new.dump {
        return Handoff::Identical;
    }
    let index = |d: &str| -> std::collections::BTreeMap<String, String> {
        d.lines()
            .filter_map(|l| {
                l.split_once('\t')
                    .map(|(n, r)| (n.to_string(), r.to_string()))
            })
            .collect()
    };
    let (o, n) = (index(&old.dump), index(&new.dump));
    let mut diff = LayoutDiff::default();
    for (name, row) in &n {
        match o.get(name) {
            None => diff.added.push(name.clone()),
            Some(orow) if orow != row => diff.changed.push(name.clone()),
            _ => {}
        }
    }
    for name in o.keys() {
        if !n.contains_key(name) {
            diff.dropped.push(name.clone());
        }
    }
    Handoff::Changed(diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(hash: u64, dump: &str) -> LayoutIdentity {
        LayoutIdentity {
            layout_hash: hash,
            dump: dump.to_string(),
        }
    }

    #[test]
    fn identical_when_hash_and_dump_match() {
        let a = id(42, "A\tsize=4\tstruct{}\nB\tsize=8\tstruct{}\n");
        assert_eq!(classify(&a, &a.clone()), Handoff::Identical);
    }

    #[test]
    fn detects_added_dropped_changed() {
        let old = id(1, "A\tsize=4\tstruct{x@0:integer}\nB\tsize=8\tstruct{}\n");
        // A reshaped (a field grew), B dropped, C added.
        let new = id(2, "A\tsize=8\tstruct{x@0:long}\nC\tsize=4\tstruct{}\n");
        let Handoff::Changed(d) = classify(&old, &new) else {
            panic!("expected Changed");
        };
        assert_eq!(d.added, vec!["C".to_string()]);
        assert_eq!(d.dropped, vec!["B".to_string()]);
        assert_eq!(d.changed, vec!["A".to_string()]);
        assert!(d.is_actionable(), "a reshape is actionable");
    }

    #[test]
    fn add_only_and_drop_only_are_not_actionable() {
        let base = "A\tsize=4\tstruct{}\n";
        let add_only = classify(
            &id(1, base),
            &id(2, "A\tsize=4\tstruct{}\nB\tsize=4\tstruct{}\n"),
        );
        let drop_only = classify(
            &id(2, "A\tsize=4\tstruct{}\nB\tsize=4\tstruct{}\n"),
            &id(1, base),
        );
        match (add_only, drop_only) {
            (Handoff::Changed(a), Handoff::Changed(d)) => {
                assert!(!a.is_actionable() && !a.added.is_empty() && a.dropped.is_empty());
                assert!(!d.is_actionable() && !d.dropped.is_empty() && d.added.is_empty());
            }
            _ => panic!("expected Changed"),
        }
    }

    #[test]
    fn sidecar_round_trips() {
        let a = id(0xdead_beef, "A\tsize=4\tstruct{x@0:integer}\n");
        assert_eq!(LayoutIdentity::from_sidecar(&a.to_sidecar()), Some(a));
    }

    #[test]
    fn bad_sidecar_rejects() {
        assert!(LayoutIdentity::from_sidecar("garbage").is_none());
        assert!(LayoutIdentity::from_sidecar("LOFT-DSCHEMA v999\nhash=1\n--\n").is_none());
        assert!(LayoutIdentity::from_sidecar("OTHER v1\nhash=1\n--\n").is_none());
    }
}
