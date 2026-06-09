// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN15.J — store change journal (two-artifact binary model).
//!
//! Records the record-level changes a debugger edit makes to the store, so the edit
//! can be **reverted** (undo) and **replayed** (the cross-store transfer that
//! finishes heap live edits).  Design: `doc/claude/plans/15-debugger/STORE_JOURNAL.md`.
//!
//! Two artifacts, split by what each is good at:
//!
//! - **blob** — a plain append-only **file** holding the variable-length payload
//!   (the `before`/`after` bytes).  No allocator: a WAL never frees mid-stream, so a
//!   store's machinery would be overhead, and a file is uniform RAM-or-disk (it is
//!   the VirtFS in the browser).
//! - **index** — a [`Store`](crate::store::Store) holding one growing fixed-stride
//!   array of [`Entry`]s.  The array is the store's only occupant, so it appends
//!   without ever relocating, and the store gives RAM-or-mmap for free (mmap is
//!   optional, never forced).
//!
//! **Commit rule:** the payload is appended to the blob *first*, the index entry
//! *last* — so on recovery the index up to its last complete entry is trusted and a
//! half-written blob tail beyond it is ignored.
//!
//! This slice records in-place field **modifies** — a pure byte restore, correct by
//! construction with no allocator interaction (it never claims, frees, or resizes a
//! *target* record), covering field / element edits.  Whole-value insert / free
//! replay is the next slice (probe #2 — replay-position determinism — is confirmed,
//! so it needs no `DbRef` remap).

use crate::database::Stores;
use crate::store::Store;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// `Modify` op tag — an in-place region change.  `Insert`/`Free` (whole-value
/// replay) get `1`/`2` when that slice lands.
const OP_MODIFY: u8 = 0;

/// Bytes per index entry — fixed stride so the array is random-access and the store
/// holds it as one flat vector.
const STRIDE: u32 = 24;

/// Byte offset of the first entry within the index record.  Leaves the 8-byte
/// allocator/header region (`fld 0` = claim size, `fld 4` = the durable entry count)
/// untouched, so the index store is self-describing.
const DATA_OFF: u32 = 8;

/// One recorded change, decoded from its 24-byte little-endian index slot.  For a
/// `Modify`, `before` is the blob bytes at `blob_at` and `after` at `blob_at + len`,
/// each `len` wide; the target region is `(store_nr, rec, off)`.
#[derive(Debug, Clone, Copy)]
struct Entry {
    op: u8,
    store_nr: u16,
    rec: u32,
    off: u32,
    len: u32,
    blob_at: u64,
}

impl Entry {
    /// Pack to the on-disk little-endian slot (portable across runs / machines).
    fn encode(self) -> [u8; STRIDE as usize] {
        let mut b = [0u8; STRIDE as usize];
        b[0] = self.op;
        b[2..4].copy_from_slice(&self.store_nr.to_le_bytes());
        b[4..8].copy_from_slice(&self.rec.to_le_bytes());
        b[8..12].copy_from_slice(&self.off.to_le_bytes());
        b[12..16].copy_from_slice(&self.len.to_le_bytes());
        b[16..24].copy_from_slice(&self.blob_at.to_le_bytes());
        b
    }

    fn decode(b: &[u8]) -> Entry {
        Entry {
            op: b[0],
            store_nr: u16::from_le_bytes([b[2], b[3]]),
            rec: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            off: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            len: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
            blob_at: u64::from_le_bytes([b[16], b[17], b[18], b[19], b[20], b[21], b[22], b[23]]),
        }
    }
}

/// The append-only payload file.  Always a file (never a store), removed on drop for
/// a transient session.
#[derive(Debug)]
struct Blob {
    file: File,
    len: u64,
    path: PathBuf,
    /// Remove the file on drop (a transient debug session); a persisted journal
    /// would keep it.
    transient: bool,
}

impl Blob {
    /// Create a fresh transient blob in the temp dir.
    fn create_temp() -> io::Result<Blob> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("loft_journal_{}_{n}.blob", std::process::id()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        Ok(Blob {
            file,
            len: 0,
            path,
            transient: true,
        })
    }

    /// Append `data`, returning its start offset (the commit-ordering first step).
    fn append(&mut self, data: &[u8]) -> io::Result<u64> {
        let at = self.len;
        self.file.seek(SeekFrom::Start(at))?;
        self.file.write_all(data)?;
        self.len += data.len() as u64;
        Ok(at)
    }

    /// Read `len` bytes at `off`.
    fn read(&mut self, off: u64, len: usize) -> io::Result<Box<[u8]>> {
        let mut buf = vec![0u8; len];
        self.file.seek(SeekFrom::Start(off))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf.into_boxed_slice())
    }
}

impl Drop for Blob {
    fn drop(&mut self) {
        if self.transient {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A store change journal: an index `Store` (the fixed-stride entry array) + a blob
/// file (the payload).  Built during an edit, then kept for undo or discarded.
#[derive(Debug)]
pub struct Journal {
    index: Store,
    /// The single array record within `index`.
    rec: u32,
    /// Number of entries (authoritative; mirrored to the index header for recovery).
    count: u32,
    blob: Blob,
}

impl Journal {
    /// A fresh in-RAM journal with a transient blob file.  The index store stays in
    /// RAM (mmap is optional and not taken here); the blob is removed on drop.
    ///
    /// # Errors
    /// Returns the I/O error if the temp blob file cannot be created.
    pub fn create() -> io::Result<Journal> {
        let mut index = Store::new(64);
        let rec = index.claim(16); // 128 bytes — room for ~5 entries before the first grow
        index.set_u32_raw(rec, 4, 0); // durable entry count starts at 0
        Ok(Journal {
            index,
            rec,
            count: 0,
            blob: Blob::create_temp()?,
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.count as usize
    }

    /// Snapshot a record region — the `before` an edit captures *before* it writes.
    /// Pair with [`record_modify`](Self::record_modify) after the write lands.
    #[must_use]
    pub fn snapshot(stores: &Stores, store_nr: u16, rec: u32, off: u32, len: u32) -> Box<[u8]> {
        stores.allocations[store_nr as usize].read_span(rec, off, len)
    }

    /// Grow the index array record so entry `count` fits, tracking the (possibly
    /// relocated) record handle `resize` returns.
    fn ensure_capacity(&mut self) {
        let needed_words = (DATA_OFF + (self.count + 1) * STRIDE).div_ceil(8);
        let cur_words = (*self.index.addr::<i32>(self.rec, 0)) as u32;
        if needed_words > cur_words {
            self.rec = self.index.resize(self.rec, needed_words.max(cur_words * 2));
        }
    }

    fn push(&mut self, e: Entry) {
        self.ensure_capacity();
        self.index
            .write_span(self.rec, DATA_OFF + self.count * STRIDE, &e.encode());
        self.count += 1;
        self.index.set_u32_raw(self.rec, 4, self.count); // durable count (recovery)
    }

    fn entry(&self, i: u32) -> Entry {
        Entry::decode(
            &self
                .index
                .read_span(self.rec, DATA_OFF + i * STRIDE, STRIDE),
        )
    }

    /// Record an in-place modify whose write has just happened: `before` is the
    /// region's pre-write bytes (from [`snapshot`](Self::snapshot)); the `after` is
    /// read from the store's *current* state, so the region must still be
    /// `before.len()` bytes wide (a modify never resizes — that invariant is what
    /// keeps this allocator-free).  Commit order: blob payload first, index last.
    ///
    /// # Errors
    /// Returns the I/O error if the blob append fails.
    pub fn record_modify(
        &mut self,
        stores: &Stores,
        store_nr: u16,
        rec: u32,
        off: u32,
        before: &[u8],
    ) -> io::Result<()> {
        let len = before.len() as u32;
        let after = stores.allocations[store_nr as usize].read_span(rec, off, len);
        let mut payload = Vec::with_capacity(2 * len as usize);
        payload.extend_from_slice(before);
        payload.extend_from_slice(&after);
        let blob_at = self.blob.append(&payload)?; // FIRST — payload is durable before the entry
        self.push(Entry {
            op: OP_MODIFY,
            store_nr,
            rec,
            off,
            len,
            blob_at,
        });
        Ok(())
    }

    /// Replay every change **forward** (redo / cross-store apply): write each
    /// region's `after` bytes, in record order.
    ///
    /// # Errors
    /// Returns the I/O error if a blob read fails.
    pub fn apply(&mut self, stores: &mut Stores) -> io::Result<()> {
        for i in 0..self.count {
            let e = self.entry(i);
            let after = self
                .blob
                .read(e.blob_at + u64::from(e.len), e.len as usize)?;
            stores.allocations[e.store_nr as usize].write_span(e.rec, e.off, &after);
        }
        Ok(())
    }

    /// Replay every change **backward** (undo): write each region's `before` bytes,
    /// in reverse order, so overlapping edits restore to the exact pre-edit bytes.
    ///
    /// # Errors
    /// Returns the I/O error if a blob read fails.
    pub fn revert(&mut self, stores: &mut Stores) -> io::Result<()> {
        for i in (0..self.count).rev() {
            let e = self.entry(i);
            let before = self.blob.read(e.blob_at, e.len as usize)?;
            stores.allocations[e.store_nr as usize].write_span(e.rec, e.off, &before);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Stores` with one fresh heap store holding one claimed `words`-word record.
    fn store_with_record(words: u32) -> (Stores, u16, u32) {
        let mut stores = Stores::default();
        stores.allocations.push(Store::new(256));
        let store_nr = (stores.allocations.len() - 1) as u16;
        let rec = stores.allocations[store_nr as usize].claim(words);
        (stores, store_nr, rec)
    }

    /// @PLN15.J probe #2 — **replay-position determinism**, the invariant the
    /// whole-value insert/free replay rests on.  Replaying a constructor's inserts
    /// into the live store with **no `DbRef` remap** is sound *iff* `claim` is a pure
    /// function of allocator state: a store cloned from the live store, run through
    /// the same claims, must hand out the same positions, so the recorded inserts
    /// land where their internal `DbRef`s already point.  This drives the same
    /// claim/free/coalesce/grow sequence on two independent stores and asserts the
    /// positions match at every step.  Rust's `HashSet` is randomly seeded per
    /// construction, so if position selection ever leaked the `claims` set's
    /// iteration order, the two runs would diverge here — making this a live guard,
    /// not a tautology.  If a future allocator change breaks determinism, replay
    /// silently breaks and this test is what catches it.
    #[test]
    fn claim_is_deterministic_from_history() {
        fn claim_sequence() -> Vec<u32> {
            let mut s = Store::new(64);
            let mut pos = Vec::new();
            let a = s.claim(4);
            pos.push(a);
            let b = s.claim(2);
            pos.push(b);
            let c = s.claim(8);
            pos.push(c);
            s.delete(b); // hole in the middle
            pos.push(s.claim(2)); // reuse the hole
            s.delete(a);
            s.delete(c); // adjacent frees → coalesce
            pos.push(s.claim(6)); // reuse the coalesced region
            pos.push(s.claim(40)); // force growth past the initial 64 words
            pos.push(s.claim(1));
            pos.push(s.claim(3));
            pos
        }
        assert_eq!(
            claim_sequence(),
            claim_sequence(),
            "claim positions must be a deterministic function of allocator history",
        );
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
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 42);

        let before = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 99);
        let mut j = Journal::create().unwrap();
        j.record_modify(&stores, sn, rec, 8, &before).unwrap();

        assert_eq!(read_i64(&stores, sn, rec, 8), 99, "edit took");
        j.revert(&mut stores).unwrap();
        assert_eq!(
            read_i64(&stores, sn, rec, 8),
            42,
            "revert restores pre-edit"
        );
        j.apply(&mut stores).unwrap();
        assert_eq!(read_i64(&stores, sn, rec, 8), 99, "apply redoes the edit");
    }

    /// Two edits to the *same* region revert in reverse order back to the original.
    #[test]
    fn overlapping_edits_revert_in_reverse() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 1);

        let mut j = Journal::create().unwrap();
        let b1 = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 2);
        j.record_modify(&stores, sn, rec, 8, &b1).unwrap();
        let b2 = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 3);
        j.record_modify(&stores, sn, rec, 8, &b2).unwrap();

        assert_eq!(read_i64(&stores, sn, rec, 8), 3);
        j.revert(&mut stores).unwrap();
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

        let mut j = Journal::create().unwrap();
        let bx = Journal::snapshot(&stores, sn, rec, 8, 8);
        write_i64(&mut stores, sn, rec, 8, 99);
        j.record_modify(&stores, sn, rec, 8, &bx).unwrap();
        let by = Journal::snapshot(&stores, sn, rec, 16, 8);
        write_i64(&mut stores, sn, rec, 16, 88);
        j.record_modify(&stores, sn, rec, 16, &by).unwrap();

        j.revert(&mut stores).unwrap();
        assert_eq!(read_i64(&stores, sn, rec, 8), 10);
        assert_eq!(read_i64(&stores, sn, rec, 16), 20);
    }

    /// A whole-record snapshot/restore (the record-level granularity).
    #[test]
    fn whole_record_snapshot_restores() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 7);
        write_i64(&mut stores, sn, rec, 16, 8);

        let before = Journal::snapshot(&stores, sn, rec, 8, 24);
        write_i64(&mut stores, sn, rec, 8, -1);
        write_i64(&mut stores, sn, rec, 16, -2);
        let mut j = Journal::create().unwrap();
        j.record_modify(&stores, sn, rec, 8, &before).unwrap();

        j.revert(&mut stores).unwrap();
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

        let mut j = Journal::create().unwrap();
        let ba = Journal::snapshot(&stores, sn_a, rec_a, 8, 8);
        write_i64(&mut stores, sn_a, rec_a, 8, 1);
        j.record_modify(&stores, sn_a, rec_a, 8, &ba).unwrap();
        let bb = Journal::snapshot(&stores, sn_b, rec_b, 8, 8);
        write_i64(&mut stores, sn_b, rec_b, 8, 2);
        j.record_modify(&stores, sn_b, rec_b, 8, &bb).unwrap();

        j.revert(&mut stores).unwrap();
        assert_eq!(read_i64(&stores, sn_a, rec_a, 8), 100);
        assert_eq!(read_i64(&stores, sn_b, rec_b, 8), 200);
    }

    /// Enough entries to force the index array to grow (and possibly relocate) past
    /// its initial capacity — every recorded change still reverts exactly.
    #[test]
    fn index_grows_past_initial_capacity() {
        let (mut stores, sn, rec) = store_with_record(34); // 33 data words: 32 i64 fields
        let n: u32 = 32;
        for i in 0..n {
            write_i64(&mut stores, sn, rec, 8 + i * 8, i64::from(i));
        }
        let mut j = Journal::create().unwrap();
        for i in 0..n {
            let off = 8 + i * 8;
            let before = Journal::snapshot(&stores, sn, rec, off, 8);
            write_i64(&mut stores, sn, rec, off, -1);
            j.record_modify(&stores, sn, rec, off, &before).unwrap();
        }
        assert_eq!(j.len(), 32, "32 entries forced a grow");
        j.revert(&mut stores).unwrap();
        for i in 0..n {
            assert_eq!(
                read_i64(&stores, sn, rec, 8 + i * 8),
                i64::from(i),
                "field {i} restored"
            );
        }
    }

    /// An empty journal is a no-op both ways.
    #[test]
    fn empty_journal_is_noop() {
        let (mut stores, sn, rec) = store_with_record(4);
        write_i64(&mut stores, sn, rec, 8, 5);
        let mut j = Journal::create().unwrap();
        assert!(j.is_empty());
        j.apply(&mut stores).unwrap();
        j.revert(&mut stores).unwrap();
        assert_eq!(read_i64(&stores, sn, rec, 8), 5);
    }
}
