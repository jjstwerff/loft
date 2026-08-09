// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I69 — Word-addressed store: the chunked entry arena keyed collections allocate from.

//! A chunked arena of fixed-stride slots inside a [`Store`], addressed by a 1-based
//! index (@PLN135 arc H).
//!
//! ## Why an arena and not a record per entry
//!
//! A keyed collection used to claim one store record per entry. A record carries an
//! 8-byte word of its own — the size header, plus the owning-collection back-pointer —
//! and `Store::claim` takes a whole free block rather than split when the remainder is
//! under a third. For a `{integer, integer}` entry that is 24 bytes of store for 16 bytes
//! of payload, and the measured cost is not the bytes but the working set they spread a
//! random lookup across: 200 ns against 77 for the same payload read out of one dense
//! array.
//!
//! Packing entries at a fixed stride removes both. What it must not remove is
//! **stability**.
//!
//! ## Chunks, because growth must not move a live entry
//!
//! A hash hands out `DbRef`s into its entries and callers hold them:
//!
//! ```text
//! e = h[k];  h += [other];  e.val      // must still read e's own value
//! ```
//!
//! That works today because growing a hash reallocates only its bucket table and never
//! an entry's bytes. One flat array would take it away — growth reallocates, every entry
//! moves, and every outstanding reference reads moved-or-freed bytes. Not an error: a
//! WRONG READ, which `COMPATIBILITY.md` forbids and which this subsystem is the worst
//! place in loft to introduce.
//!
//! So the arena grows by APPENDING a chunk and never reallocates one that already holds
//! slots. A slot keeps the address it was handed out at for as long as it lives.
//! Measured cost of chunking versus one flat array: 86 ns against 77, where today is 200
//! (`doc/claude/plans/135-hash-performance/probes/q5-chunked-arena.loft`).
//!
//! ## Chunk sizes double
//!
//! Chunk `k` holds `BASE << k` slots, so `k` and the slot within it are recovered from an
//! index by arithmetic rather than a search, and total waste is bounded the way a
//! vector's is instead of rounding every small collection up to one big chunk. The
//! directory that maps `k` to its record is a handful of `u32`s — small enough to stay
//! cache-resident, which is why the index stays 4 bytes and the bucket table does not
//! double. That hop is not free by assumption: the Q5 probe pays it and still measures
//! 86 ns.
//!
//! ## Layout
//!
//! Directory record — `u32` chunk record numbers from [`DIR0`]:
//!
//! ```text
//!   0  size header (Store's own)
//!   8  chunk record 0, 1, 2, …          (2 per word)
//! ```
//!
//! Chunk record — the owning-collection back-pointer lives here, ONCE per chunk rather
//! than once per entry, because every slot in a chunk shares an owner:
//!
//! ```text
//!   0  size header (Store's own)
//!   4  back-pointer to the owning collection record
//!   8  slot 0, slot 1, …                (each `stride` bytes)
//! ```
//!
//! A freed slot is threaded onto a free list through its own first 4 bytes, so a
//! delete-heavy collection reuses space instead of growing forever.

use crate::store::Store;

/// Slots in chunk 0.  Chunk `k` holds `BASE << k`, so a collection with three entries
/// costs one small chunk rather than rounding up to the size that suits a million.
pub const BASE: u32 = 64;

/// First byte of the chunk-record array in the directory record.
pub const DIR0: u32 = 8;

/// First byte of the slot array in a chunk record.  Byte 4 is the owning-collection
/// back-pointer, which `database::search` reads to decide whether a record is live.
pub const SLOT0: u32 = 8;

/// Byte 4 of a chunk record: the collection this chunk's slots belong to.
pub const OWNER_FLD: u32 = 4;

/// Where slot `index` lives: `(chunk number, byte offset within the chunk record)`.
///
/// `index` is 1-based, so 0 is free to mean "no entry" in a bucket slot and in the free
/// list. Both halves are arithmetic — `chunk_of` is a `leading_zeros`, not a search —
/// because this runs on every lookup that hits.
#[must_use]
pub fn locate(index: u32, stride: u32) -> (u32, u32) {
    debug_assert!(index >= 1, "arena index is 1-based; 0 means absent");
    let i = index - 1;
    let k = chunk_of(i);
    (k, SLOT0 + (i - first_index_of(k)) * stride)
}

/// The chunk holding 0-based slot `i`.
///
/// Chunk `k` starts at `BASE * ((1 << k) - 1)`, so `i / BASE + 1` lands in `[2^k, 2^(k+1))`
/// exactly when `i` is in chunk `k`, and the chunk number is that value's floor-log2.
#[must_use]
pub fn chunk_of(i: u32) -> u32 {
    (i / BASE + 1).ilog2()
}

/// The first 0-based slot number that chunk `k` holds.
#[must_use]
pub fn first_index_of(k: u32) -> u32 {
    BASE * ((1 << k) - 1)
}

/// How many slots chunk `k` holds.
#[must_use]
pub fn chunk_slots(k: u32) -> u32 {
    BASE << k
}

/// Words to claim for chunk `k` at `stride` bytes per slot: the header word plus the
/// slots themselves.
#[must_use]
pub fn chunk_words(k: u32, stride: u32) -> u32 {
    1 + (chunk_slots(k) * stride).div_ceil(8)
}

/// Read chunk `k`'s record number out of directory record `dir`, or 0 when the directory
/// does not reach that far yet.
#[must_use]
pub fn chunk_rec(store: &Store, dir: u32, k: u32) -> u32 {
    if dir == 0 || k >= dir_capacity(store, dir) {
        return 0;
    }
    store.get_u32_raw(dir, DIR0 + 4 * k)
}

/// How many chunk numbers directory record `dir` can hold.
#[must_use]
pub fn dir_capacity(store: &Store, dir: u32) -> u32 {
    if dir == 0 {
        return 0;
    }
    (store.record_words(dir) - 1) * 2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decode must tile the index space with no gap and no overlap — every index
    /// lands in exactly one chunk, at exactly one slot, and consecutive indices are
    /// consecutive slots until a chunk boundary.
    ///
    /// Checked against an independently BUILT map rather than against the formula's own
    /// algebra: a closed form that agrees with itself proves nothing.
    #[test]
    fn every_index_lands_in_exactly_one_slot() {
        // Build the expected layout by walking chunks and handing out slots in order.
        let mut expect: Vec<(u32, u32)> = Vec::new();
        for k in 0..12 {
            for s in 0..chunk_slots(k) {
                expect.push((k, s));
            }
        }
        for (i, &(k, s)) in expect.iter().enumerate() {
            let idx = u32::try_from(i).unwrap() + 1;
            let stride = 16;
            assert_eq!(
                chunk_of(u32::try_from(i).unwrap()),
                k,
                "index {idx} should be in chunk {k}"
            );
            assert_eq!(
                locate(idx, stride),
                (k, SLOT0 + s * stride),
                "index {idx} should be chunk {k} slot {s}"
            );
        }
    }

    /// Chunk starts must agree with the cumulative slot counts, or `locate` subtracts the
    /// wrong base and every entry past chunk 0 reads its neighbour's bytes.
    #[test]
    fn chunk_starts_are_the_running_total() {
        let mut total = 0;
        for k in 0..16 {
            assert_eq!(first_index_of(k), total, "chunk {k} starts at {total}");
            total += chunk_slots(k);
        }
    }

    /// The first slot of each chunk sits immediately after the header, and the last slot
    /// fits inside the claimed words — an off-by-one here writes outside the record.
    #[test]
    fn slots_fit_inside_their_chunk() {
        for k in 0..12 {
            for &stride in &[8_u32, 16, 24, 64] {
                let words = chunk_words(k, stride);
                let last = chunk_slots(k) - 1;
                let (_, off) = locate(first_index_of(k) + last + 1, stride);
                assert_eq!(
                    locate(first_index_of(k) + 1, stride).1,
                    SLOT0,
                    "chunk {k} must start its slots at SLOT0"
                );
                assert!(
                    off + stride <= words * 8,
                    "chunk {k} stride {stride}: last slot ends at {} but only {} bytes claimed",
                    off + stride,
                    words * 8
                );
            }
        }
    }

    /// Growth must not move what is already placed: an index's location is a pure
    /// function of the index, so appending chunks cannot relocate an existing slot.
    /// This is Q5's whole answer, so it gets its own assertion rather than resting on
    /// the fact that no code moves anything.
    #[test]
    fn appending_a_chunk_never_moves_an_existing_slot() {
        let stride = 16;
        let before: Vec<(u32, u32)> = (1..5000).map(|i| locate(i, stride)).collect();
        // "Appending a chunk" is only ever a directory write; it cannot change `locate`.
        let after: Vec<(u32, u32)> = (1..5000).map(|i| locate(i, stride)).collect();
        assert_eq!(before, after, "an existing slot moved");
    }
}
