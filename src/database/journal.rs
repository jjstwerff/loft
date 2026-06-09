// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN15.J — store change journal.
//!
//! Records the record-level changes a debugger edit makes to the store, so the edit
//! can be **reverted** (undo) and **replayed** (the cross-store transfer that
//! finishes heap live edits).  Design: `doc/claude/plans/15-debugger/STORE_JOURNAL.md`.
//!
//! This first slice captures in-place field **modifies** — a pure byte restore,
//! correct by construction with no allocator interaction: it never claims, frees, or
//! resizes a record, so it cannot corrupt the free-list or move a `DbRef`.  That
//! covers field / element edits (`pt.x = 9`, `v[i] = 5`).  Whole-value
//! materialisation (insert / free replay) hinges on replay-position determinism
//! (probe #2 in the design) and is the next slice.

use crate::database::Stores;

/// One recorded in-place modify of a record region `[off, off+len)`.  `before` and
/// `after` are the raw bytes of that region (the `(rec, off)` addressing matches
/// [`Store::addr`](crate::store::Store::addr)); `before.len() == after.len()`.
#[derive(Debug, Clone)]
pub struct Change {
    /// Which store (`Stores::allocations` index) the record lives in.
    pub store_nr: u16,
    /// The record (word-addressed position within the store).
    pub rec: u32,
    /// Byte offset of the changed region within the record.
    pub off: u32,
    /// The region's bytes before the edit (written back on revert).
    pub before: Box<[u8]>,
    /// The region's bytes after the edit (written on apply / replay).
    pub after: Box<[u8]>,
}

/// An ordered list of the record changes from one edit — revertible (undo) and
/// replayable (redo / transfer).  Built during a debugger edit, then either kept for
/// undo or discarded.
#[derive(Debug, Default)]
pub struct Journal {
    changes: Vec<Change>,
}

impl Journal {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    #[must_use]
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// Snapshot a record region — the `before` an edit captures *before* it writes.
    /// Pair it with [`record_modify`](Self::record_modify) after the write lands.
    #[must_use]
    pub fn snapshot(stores: &Stores, store_nr: u16, rec: u32, off: u32, len: u32) -> Box<[u8]> {
        stores.allocations[store_nr as usize].read_span(rec, off, len)
    }

    /// Record an in-place modify whose write has just happened: `before` is the
    /// region's pre-write bytes (from [`snapshot`](Self::snapshot)) and the `after`
    /// is read from the store's *current* state, so the same region must still be
    /// `before.len()` bytes wide (a modify never resizes — that is the invariant
    /// that keeps this allocator-free).
    pub fn record_modify(
        &mut self,
        stores: &Stores,
        store_nr: u16,
        rec: u32,
        off: u32,
        before: Box<[u8]>,
    ) {
        let after = stores.allocations[store_nr as usize].read_span(rec, off, before.len() as u32);
        self.changes.push(Change {
            store_nr,
            rec,
            off,
            before,
            after,
        });
    }

    /// Replay every change **forward** (redo / cross-store apply): write each
    /// region's `after` bytes, in record order.
    pub fn apply(&self, stores: &mut Stores) {
        for c in &self.changes {
            stores.allocations[c.store_nr as usize].write_span(c.rec, c.off, &c.after);
        }
    }

    /// Replay every change **backward** (undo): write each region's `before` bytes,
    /// in reverse order, so when two edits touch overlapping regions the earlier
    /// `before` wins and the record returns exactly to its pre-edit bytes.
    pub fn revert(&self, stores: &mut Stores) {
        for c in self.changes.iter().rev() {
            stores.allocations[c.store_nr as usize].write_span(c.rec, c.off, &c.before);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    /// A `Stores` with one fresh heap store holding one claimed `words`-word record.
    /// Returns `(stores, store_nr, rec)`.
    fn store_with_record(words: u32) -> (Stores, u16, u32) {
        let mut stores = Stores::default();
        stores.allocations.push(Store::new(256));
        let store_nr = (stores.allocations.len() - 1) as u16;
        let rec = stores.allocations[store_nr as usize].claim(words);
        (stores, store_nr, rec)
    }

    fn read_i64(stores: &Stores, sn: u16, rec: u32, off: u32) -> i64 {
        *stores.allocations[sn as usize].addr::<i64>(rec, off)
    }
    fn write_i64(stores: &mut Stores, sn: u16, rec: u32, off: u32, v: i64) {
        *stores.allocations[sn as usize].addr_mut::<i64>(rec, off) = v;
    }

    /// Undo then redo a single scalar-field edit, exact byte fidelity both ways.
    #[test]
    fn modify_reverts_then_replays() {
        let (mut stores, sn, rec) = store_with_record(4); // 32 bytes: header + 24 data
        write_i64(&mut stores, sn, rec, 8, 42);

        let before = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 99);
        let mut j = Journal::new();
        j.record_modify(&stores, sn, rec, 8, before);

        assert_eq!(read_i64(&stores, sn, rec, 8), 99, "edit took");
        j.revert(&mut stores);
        assert_eq!(
            read_i64(&stores, sn, rec, 8),
            42,
            "revert restores pre-edit"
        );
        j.apply(&mut stores);
        assert_eq!(read_i64(&stores, sn, rec, 8), 99, "apply redoes the edit");
    }

    /// Two edits to the *same* region revert in reverse order back to the original.
    #[test]
    fn overlapping_edits_revert_in_reverse() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 1);

        let mut j = Journal::new();
        let b1 = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 2);
        j.record_modify(&stores, sn, rec, 8, b1);

        let b2 = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 3);
        j.record_modify(&stores, sn, rec, 8, b2);

        assert_eq!(read_i64(&stores, sn, rec, 8), 3);
        j.revert(&mut stores);
        assert_eq!(
            read_i64(&stores, sn, rec, 8),
            1,
            "reverse order restores original"
        );
    }

    /// Two fields of one record edited together revert together.
    #[test]
    fn multi_field_edit_round_trips() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 10);
        write_i64(&mut stores, sn, rec, 16, 20);

        let mut j = Journal::new();
        let bx = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 99);
        j.record_modify(&stores, sn, rec, 8, bx);
        let by = Journal::snapshot(&stores, sn, rec, 16, 8);
        write_i64(&mut stores, sn, rec, 16, 88);
        j.record_modify(&stores, sn, rec, 16, by);

        j.revert(&mut stores);
        assert_eq!(read_i64(&stores, sn, rec, 8), 10);
        assert_eq!(read_i64(&stores, sn, rec, 16), 20);
    }

    /// A whole-record snapshot/restore (the granularity the design favours):
    /// scribble the whole record, revert from a full-record `before`.
    #[test]
    fn whole_record_snapshot_restores() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 7);
        write_i64(&mut stores, sn, rec, 16, 8);

        let before = Journal::snapshot(&stores, sn, rec, 8, 24); // all 3 data words
        write_i64(&mut stores, sn, rec, 8, -1);
        write_i64(&mut stores, sn, rec, 16, -2);
        let mut j = Journal::new();
        j.record_modify(&stores, sn, rec, 8, before);

        j.revert(&mut stores);
        assert_eq!(read_i64(&stores, sn, rec, 8), 7);
        assert_eq!(read_i64(&stores, sn, rec, 16), 8);
    }

    /// Changes spanning two stores apply/revert independently.
    #[test]
    fn cross_store_changes() {
        let (mut stores, sn_a, rec_a) = store_with_record(4);
        stores.allocations.push(Store::new(256));
        let sn_b = (stores.allocations.len() - 1) as u16;
        let rec_b = stores.allocations[sn_b as usize].claim(4);

        write_i64(&mut stores, sn_a, rec_a, 8, 100);
        write_i64(&mut stores, sn_b, rec_b, 8, 200);

        let mut j = Journal::new();
        let ba = Journal::snapshot(&stores, sn_a, rec_a, 8, 8);
        write_i64(&mut stores, sn_a, rec_a, 8, 1);
        j.record_modify(&stores, sn_a, rec_a, 8, ba);
        let bb = Journal::snapshot(&stores, sn_b, rec_b, 8, 8);
        write_i64(&mut stores, sn_b, rec_b, 8, 2);
        j.record_modify(&stores, sn_b, rec_b, 8, bb);

        j.revert(&mut stores);
        assert_eq!(read_i64(&stores, sn_a, rec_a, 8), 100);
        assert_eq!(read_i64(&stores, sn_b, rec_b, 8), 200);
    }

    /// An empty journal is a no-op both ways.
    #[test]
    fn empty_journal_is_noop() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 5);
        let j = Journal::new();
        assert!(j.is_empty());
        j.apply(&mut stores);
        j.revert(&mut stores);
        assert_eq!(read_i64(&stores, sn, rec, 8), 5);
    }
}
