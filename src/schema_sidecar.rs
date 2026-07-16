// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
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
use std::path::{Path, PathBuf};

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

// ── The `.dschema` sidecar file lifecycle (durable-store wiring) ─────────────

/// The `.dschema` sidecar path for a store at `store_path` — a SEPARATE file
/// beside the store (mirrors the `.dmeta` integrity sidecar; `x` → `x.dschema`).
/// The store payload is never touched.
#[must_use]
pub fn dschema_path(store_path: &Path) -> PathBuf {
    let mut p = store_path.as_os_str().to_owned();
    p.push(".dschema");
    PathBuf::from(p)
}

impl LayoutIdentity {
    /// Atomically write this identity to the `.dschema` sidecar beside the store
    /// (temp + rename — a torn write never yields a half-file). Call when a
    /// durable store is persisted; the store payload stays byte-identical.
    ///
    /// # Errors
    /// Propagates any `io::Error` creating, writing, syncing, or renaming the
    /// sidecar file.
    pub fn write_beside(&self, store_path: &Path) -> std::io::Result<()> {
        self.write_to(&dschema_path(store_path))
    }

    /// Atomically write this identity's sidecar text to `path` (temp + rename;
    /// creates the parent dir if needed). Shared by `write_beside` (the store
    /// sidecar) and `write_baseline` (the project layout lockfile).
    fn write_to(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write as _;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut tmp = path.to_path_buf();
        let mut name = tmp
            .file_name()
            .map(std::ffi::OsString::from)
            .unwrap_or_default();
        name.push(".tmp");
        tmp.set_file_name(name);
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(self.to_sidecar().as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }
}

/// The verdict of checking a store's `.dschema` against the running program's
/// layout identity — Phase D's load-time gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaVerdict {
    /// No `.dschema` beside the store — a legacy / brand-new store. Nothing to
    /// check; the caller writes one on persist.
    Fresh,
    /// The stored layout matches the running program's — safe RAW handoff.
    Match,
    /// The stored layout differs — serialize-before-handoff / migrate (Phase E)
    /// or reject-and-rebuild. Carries the per-type diff.
    Changed(LayoutDiff),
    /// The `.dschema` is present but unreadable (bad magic / version / format) —
    /// the store's layout is UNKNOWN, so it must not be read raw. Reject.
    Unreadable,
}

impl SchemaVerdict {
    /// True iff the store can be read RAW without migration (`Fresh` / `Match`).
    #[must_use]
    pub fn is_raw_safe(&self) -> bool {
        matches!(self, SchemaVerdict::Fresh | SchemaVerdict::Match)
    }

    /// Map a non-raw-safe verdict to the durable-store corruption reason, so a
    /// consumer routes it through the SAME `on_corruption` rebuild path an
    /// integrity failure uses. `None` for the raw-safe verdicts.
    #[must_use]
    pub fn as_corrupt_reason(&self) -> Option<crate::store::CorruptReason> {
        match self {
            SchemaVerdict::Fresh | SchemaVerdict::Match => None,
            SchemaVerdict::Changed(_) | SchemaVerdict::Unreadable => {
                Some(crate::store::CorruptReason::SchemaMismatch)
            }
        }
    }
}

/// Classify an already-read sidecar TEXT against the running program's identity
/// `current`. Used for a store's REMOTE `.dschema` fetched over HTTP (#522),
/// where there is no local path to hand [`check_file`]. `Unreadable` when `text`
/// is not a valid sidecar; the ABSENT case (no sidecar at all) is the caller's
/// to map to [`SchemaVerdict::Fresh`].
#[must_use]
pub fn verdict_for_sidecar_text(text: &str, current: &LayoutIdentity) -> SchemaVerdict {
    let Some(stored) = LayoutIdentity::from_sidecar(text) else {
        return SchemaVerdict::Unreadable;
    };
    match classify(&stored, current) {
        Handoff::Identical => SchemaVerdict::Match,
        Handoff::Changed(diff) => SchemaVerdict::Changed(diff),
    }
}

/// Read a recorded identity from `path` and compare it with the running
/// program's identity `current`. Shared by [`check_beside`] (a store's
/// `.dschema`) and [`check_against_baseline`] (the project layout lockfile).
///
/// # Errors
/// Propagates an `io::Error` reading the file (other than not-found → `Fresh`).
pub fn check_file(path: &Path, current: &LayoutIdentity) -> std::io::Result<SchemaVerdict> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SchemaVerdict::Fresh),
        Err(e) => return Err(e),
    };
    Ok(verdict_for_sidecar_text(&text, current))
}

/// Read the `.dschema` sidecar beside `store_path` and compare it with the
/// running program's identity `current`. A changed layout is DETECTED here —
/// never a silent raw handoff across a layout change (the @PLN97 gap). Also the
/// safety gate for a REMOTE store read (#522): a working-set loader must reject
/// a remote store whose layout identity differs, never range-read it raw.
///
/// # Errors
/// Propagates an `io::Error` reading the sidecar (other than not-found, which
/// yields [`SchemaVerdict::Fresh`]).
pub fn check_beside(store_path: &Path, current: &LayoutIdentity) -> std::io::Result<SchemaVerdict> {
    check_file(&dschema_path(store_path), current)
}

// ── Phase F — the compiler migration aid (developer surface) ────────────────
//
// The programmer just edits their structs; the compiler diffs the current layout
// against a committed BASELINE and, on the actionable state (a reshape, or an
// add∧drop it can't tell from a rename), aids them: a diagnostic + a scaffolded
// migration outline. Add-only / drop-only changes are handled silently (lenient
// default / ignore). The handoff machinery stays invisible.

/// The project's committed layout baseline — `.loft/layout.lock` beside
/// `loft.toml`. The compiler diffs the current type table against it to detect a
/// schema change; `accept` (a filled migration) rewrites it.
#[must_use]
pub fn baseline_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".loft").join("layout.lock")
}

impl LayoutIdentity {
    /// Write this identity as the project layout baseline (the "accept" flow —
    /// call after the programmer has run/confirmed the migration).
    ///
    /// # Errors
    /// Propagates any `io::Error` writing the lockfile.
    pub fn write_baseline(&self, project_dir: &Path) -> std::io::Result<()> {
        self.write_to(&baseline_path(project_dir))
    }
}

/// Diff the running program's layout `current` against the committed baseline.
///
/// # Errors
/// Propagates an `io::Error` reading the lockfile (other than not-found → the
/// first build, `Fresh`).
pub fn check_against_baseline(
    project_dir: &Path,
    current: &LayoutIdentity,
) -> std::io::Result<SchemaVerdict> {
    check_file(&baseline_path(project_dir), current)
}

/// The human diagnostic for a schema change against the baseline. Empty string
/// when nothing changed; a quiet NOTE for a pure add-only / drop-only change
/// (handled automatically); a WARNING that migration may be needed for the
/// actionable state (a reshape, or an add∧drop).
#[must_use]
pub fn describe_change(diff: &LayoutDiff) -> String {
    use std::fmt::Write as _;
    if diff.added.is_empty() && diff.dropped.is_empty() && diff.changed.is_empty() {
        return String::new();
    }
    let list = |v: &[String]| v.join(", ");
    if diff.is_actionable() {
        let mut s =
            String::from("warning: the store layout changed in a way that may need migration:\n");
        if !diff.changed.is_empty() {
            let _ = writeln!(s, "  reshaped: {}", list(&diff.changed));
        }
        if !diff.added.is_empty() {
            let _ = writeln!(s, "  added:    {}", list(&diff.added));
        }
        if !diff.dropped.is_empty() {
            let _ = writeln!(s, "  dropped:  {}", list(&diff.dropped));
        }
        s.push_str("  a migration outline was generated — fill the backfill stub(s).");
        s
    } else if diff.dropped.is_empty() {
        // add-only — automatic on load.
        format!(
            "note: schema added {} (auto-defaulted on load)",
            list(&diff.added)
        )
    } else {
        // drop-only — automatic on load.
        format!(
            "note: schema dropped {} (auto-ignored on load)",
            list(&diff.dropped)
        )
    }
}

/// A scaffolded `.loft` migration outline generated from a schema change: the
/// mechanical add/drop parts are noted as handled automatically, and each
/// RESHAPED type gets a `// TODO backfill` stub to fill (map old fields → new).
/// The programmer fills the stubs; `write_baseline` accepts the result.
#[must_use]
pub fn migration_outline(diff: &LayoutDiff) -> String {
    use std::fmt::Write as _;
    let mut s = String::from(
        "// @PLN97 migration outline — generated from a schema change.\n\
         // Add-only / drop-only parts are handled automatically on load; only the\n\
         // reshaped types below need a backfill (map the OLD record to the NEW).\n",
    );
    if !diff.added.is_empty() {
        let _ = writeln!(s, "//   added   (defaulted): {}", diff.added.join(", "));
    }
    if !diff.dropped.is_empty() {
        let _ = writeln!(s, "//   dropped (ignored):   {}", diff.dropped.join(", "));
    }
    s.push_str("\nfn migrate() {\n");
    if diff.changed.is_empty() {
        s.push_str("  // no reshaped types — nothing to backfill.\n");
    } else {
        for name in &diff.changed {
            let _ = writeln!(
                s,
                "  // TODO backfill {name}: read the old {name} fields, write the new ones."
            );
        }
    }
    s.push_str("}\n");
    s
}

/// The user-defined struct / enum types of a program — the roots of its layout
/// identity. Stdlib types are excluded (the layout closure pulls in whatever
/// base / collection types they reference). Feed these to [`LayoutIdentity::of`]
/// to build the identity the baseline (Phase F) or the `.dschema` (Phase D) records.
#[must_use]
pub fn program_roots(data: &crate::data::Data) -> Vec<u16> {
    use crate::data::DefType;
    (0..data.definitions())
        .map(|d_nr| data.def(d_nr))
        .filter(|def| {
            matches!(def.def_type, DefType::Struct | DefType::Enum)
                // User-declared types have plain-identifier names. Compiler-
                // generated ones (`__tuple<…>`, `main_vector<…>`, closures) carry
                // `__` or a `<…>` instantiation suffix; the layout closure reaches
                // them via the user types that use them, so they aren't roots.
                && !def.name.starts_with("__")
                && !def.name.contains('<')
                && !crate::compile::is_default_file(&def.position.file)
                && def.known_type != u16::MAX
        })
        .map(|def| def.known_type)
        .collect()
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

    /// @PLN97 F9 — the layout identity pins host endianness: two stores with an
    /// identical byte layout but written on opposite-endian hosts must NEVER hand
    /// over raw. Proven without a big-endian machine — the `@endian` line is the
    /// only difference, and even under a (hypothetical) hash COLLISION the dump
    /// compare still refuses the raw handoff and names `@endian` as the change.
    #[test]
    fn endian_flip_is_never_a_raw_handoff() {
        let types = "A\tsize=4\tstruct{x@0:integer}\n";
        let little = id(7, &format!("@endian\tlittle\n{types}"));
        let big = id(7, &format!("@endian\tbig\n{types}")); // same hash ON PURPOSE
        let Handoff::Changed(d) = classify(&little, &big) else {
            panic!("a cross-endian store must never classify Identical");
        };
        assert_eq!(
            d.changed,
            vec!["@endian".to_string()],
            "the endian flip must be visible in the diff"
        );
        assert!(d.added.is_empty() && d.dropped.is_empty());
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

    /// The full `.dschema` lifecycle: write beside a store path, then re-check —
    /// matching identity → raw-safe; a changed layout → detected (not a silent
    /// raw handoff), mapping to `CorruptReason::SchemaMismatch`.
    #[test]
    fn dschema_lifecycle_beside_a_store() {
        use crate::store::CorruptReason;
        let dir = std::env::temp_dir();
        let store = dir.join(format!("loft_dschema_test_{}.store", std::process::id()));
        let side = dschema_path(&store);
        let _ = std::fs::remove_file(&side);

        let a = id(111, "W\tsize=4\tstruct{x@0:integer}\n");
        // No sidecar yet → Fresh (a brand-new / legacy store).
        assert_eq!(check_beside(&store, &a).unwrap(), SchemaVerdict::Fresh);

        // Persist the identity beside the store.
        a.write_beside(&store).unwrap();
        assert!(
            side.exists(),
            "the .dschema sidecar was written beside the store"
        );

        // Same identity → raw-safe handoff.
        let v = check_beside(&store, &a).unwrap();
        assert_eq!(v, SchemaVerdict::Match);
        assert!(v.is_raw_safe() && v.as_corrupt_reason().is_none());

        // A changed layout (a field grew) → detected, NOT read raw.
        let b = id(222, "W\tsize=8\tstruct{x@0:long}\n");
        let v = check_beside(&store, &b).unwrap();
        assert!(matches!(v, SchemaVerdict::Changed(_)) && !v.is_raw_safe());
        assert_eq!(v.as_corrupt_reason(), Some(CorruptReason::SchemaMismatch));

        // A garbage sidecar → Unreadable → reject (layout unknown).
        std::fs::write(&side, "corrupt").unwrap();
        let v = check_beside(&store, &a).unwrap();
        assert_eq!(v, SchemaVerdict::Unreadable);
        assert_eq!(v.as_corrupt_reason(), Some(CorruptReason::SchemaMismatch));

        let _ = std::fs::remove_file(&side);
    }

    fn diff(added: &[&str], dropped: &[&str], changed: &[&str]) -> LayoutDiff {
        let v = |s: &[&str]| s.iter().map(|x| (*x).to_string()).collect();
        LayoutDiff {
            added: v(added),
            dropped: v(dropped),
            changed: v(changed),
        }
    }

    #[test]
    fn describe_change_notes_vs_warns() {
        assert_eq!(describe_change(&diff(&[], &[], &[])), "");
        // add-only / drop-only → a quiet note, no "migration".
        assert!(describe_change(&diff(&["B"], &[], &[])).starts_with("note:"));
        assert!(describe_change(&diff(&[], &["B"], &[])).contains("auto-ignored"));
        // reshape (or add∧drop) → a warning about migration.
        let w = describe_change(&diff(&["C"], &["B"], &["A"]));
        assert!(w.starts_with("warning:") && w.contains("reshaped: A"));
    }

    #[test]
    fn migration_outline_stubs_reshapes() {
        let o = migration_outline(&diff(&["C"], &["B"], &["A", "D"]));
        assert!(o.contains("TODO backfill A") && o.contains("TODO backfill D"));
        assert!(o.contains("added   (defaulted): C") && o.contains("dropped (ignored):   B"));
        // no reshape → nothing to backfill.
        assert!(migration_outline(&diff(&["C"], &[], &[])).contains("nothing to backfill"));
    }

    #[test]
    fn baseline_accept_and_recheck() {
        let dir = std::env::temp_dir().join(format!("loft_baseline_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = id(7, "W\tsize=4\tstruct{x@0:integer}\n");

        // First build — no baseline yet.
        assert_eq!(
            check_against_baseline(&dir, &a).unwrap(),
            SchemaVerdict::Fresh
        );
        // Accept: record the baseline.
        a.write_baseline(&dir).unwrap();
        assert!(baseline_path(&dir).exists());
        // Unchanged → Match; a reshape → Changed (the compiler would aid).
        assert_eq!(
            check_against_baseline(&dir, &a).unwrap(),
            SchemaVerdict::Match
        );
        let b = id(8, "W\tsize=8\tstruct{x@0:long}\n");
        assert!(matches!(
            check_against_baseline(&dir, &b).unwrap(),
            SchemaVerdict::Changed(_)
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
