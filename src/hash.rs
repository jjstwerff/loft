// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

use crate::keys;
use crate::keys::{Content, DbRef, Key};
use crate::store::Store;
use std::cmp::Ordering;

// Bucket-record layout (word-addressed; each field is a byte offset within
// the record `claim`).  Word 0 is the size/length header, word 1 holds the
// per-hash seed, and the bucket array starts at word 2:
//
//   fld 0  : size header (word count = `room`; doubles as data, see
//            `Store::record_words`)
//   fld 4  : `LEN_FLD`  — live-entry count (u32)
//   fld 8  : `SEED_FLD` — per-hash hash seed (u64, low half at 8, high at 12)
//   fld 16 : `BUCKET0`  — first bucket slot (u32 record-numbers, 2 per word)
//
// The seed makes a persisted hash portable: it is stored WITH the buckets,
// so a reader re-derives the same bucket for every key (see
// `keys::seeded_hasher`).  `elms = (room - 2) * 2` — two words (header +
// seed) are reserved before the bucket array.
const LEN_FLD: u32 = 4;
const SEED_FLD: u32 = 8;
const BUCKET0: u32 = 16;

/// Read the per-hash seed stored in bucket record `claim`.
fn read_seed(store: &Store, claim: u32) -> u64 {
    let lo = u64::from(store.get_u32_raw(claim, SEED_FLD));
    let hi = u64::from(store.get_u32_raw(claim, SEED_FLD + 4));
    lo | (hi << 32)
}

/// Write the per-hash seed into bucket record `claim`.
fn write_seed(store: &mut Store, claim: u32, seed: u64) {
    store.set_u32_raw(claim, SEED_FLD, (seed & 0xFFFF_FFFF) as u32);
    store.set_u32_raw(claim, SEED_FLD + 4, (seed >> 32) as u32);
}

pub fn add(hash: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let mut claim = keys::store(hash, stores).get_u32_raw(hash.rec, hash.pos);
    let length = if claim == 0 {
        // First insert: claim 10 words so the bucket array (room - 2 words,
        // 2 slots/word) starts at 16 slots — matching the old 9-word/16-slot
        // table now that words 0 and 1 hold the header and the seed.
        claim = keys::mut_store(hash, stores).claim(10);
        keys::mut_store(hash, stores).zero_fill(claim);
        // Seed the new table and store the seed with its buckets, so any
        // reader (including a different process) re-derives identical buckets.
        let seed = keys::fresh_seed();
        write_seed(keys::mut_store(hash, stores), claim, seed);
        keys::mut_store(hash, stores).set_u32_raw(hash.rec, hash.pos, claim);
        0
    } else {
        keys::store(hash, stores).get_u32_raw(claim, LEN_FLD)
    };
    let room = keys::store(hash, stores).record_words(claim);
    // Grow at load factor 0.75 (= 0.75·elms).  The `+ 2` counts the two
    // reserved words (header + seed) before the bucket array: rehash when
    // `length >= 1.5·(room - 2) = 0.75·elms`.  (Was `+ 1` when only the
    // header word was reserved.)
    if (length * 2 / 3) + 2 >= room {
        let new_claim = keys::mut_store(hash, stores).claim(room * 2 - 1);
        keys::mut_store(hash, stores).zero_fill(new_claim);
        rehash_into(hash, claim, new_claim, stores, keys);
        install_table(hash, claim, new_claim, stores);
        claim = new_claim;
    }
    hash_set(claim, rec, stores, keys);
    keys::mut_store(rec, stores).set_u32_raw(claim, LEN_FLD, length + 1);
    // hash_validate(hash, key, stores, keys);
}

/// Give `hash` a bucket table large enough to hold `count` entries without rehashing,
/// so filling it does not repeatedly rebuild the table (@PLN135 arc C).
///
/// Capacity only: it never changes which records are present, nor their order, nor
/// what `len` answers — a table sized for `count` behaves exactly like one that grew
/// into that size, because the seed and therefore every bucket is carried across.
/// A `count` the current table already covers does nothing, so calling it twice, or
/// with too small a number, is safe.
///
/// The size solves [`add`]'s own growth condition: `add` rebuilds when
/// `(length * 2 / 3) + 2 >= room`, so `room` must exceed that for `length == count`.
pub fn reserve(hash: &DbRef, count: i64, stores: &mut [Store], keys: &[Key]) {
    // A negative or absurd count asks for nothing; a table so large its word count
    // overflows a `u32` cannot be claimed at all.  Both mean "leave it alone" — this
    // is a hint, and a hint never fails the program.
    let Ok(count) = u64::try_from(count) else {
        return;
    };
    let Ok(want) = u32::try_from((count * 2 / 3) + 3) else {
        return;
    };
    let claim = keys::store(hash, stores).get_u32_raw(hash.rec, hash.pos);
    if claim != 0 && keys::store(hash, stores).record_words(claim) >= want {
        return;
    }
    let new_claim = keys::mut_store(hash, stores).claim(want);
    keys::mut_store(hash, stores).zero_fill(new_claim);
    if claim == 0 {
        // No table yet: this IS the first allocation, so mint the seed `add` would
        // have minted on the first insert.
        let seed = keys::fresh_seed();
        write_seed(keys::mut_store(hash, stores), new_claim, seed);
    } else {
        rehash_into(hash, claim, new_claim, stores, keys);
    }
    install_table(hash, claim, new_claim, stores);
}

/// Point `hash` at `new_claim` and give `old_claim` back to the store.
///
/// The two are one step: a bucket table that is no longer the hash's table is
/// unreachable, and a claim nothing can reach is a leak. Both replacement sites — `add`'s
/// growth and [`reserve`] on a non-empty hash — used to do only the first half, so every
/// doubling stranded its predecessor. A grown 1M-entry hash carried 49.3 MB where the
/// identical content pre-sized carried 33.0, `store_reclaim` recovered none of it (the
/// blocks are CLAIMED, not free), and `store_persist_bind` wrote the dead tables to disk.
///
/// The order is load-bearing and is why this is one function rather than a line at each
/// site: repoint FIRST, free second. `Store::delete` repurposes the block's body as a
/// free-tree node and may coalesce it with its neighbours, so between the free and the
/// repoint the hash's field would name bytes that are already something else.
///
/// `old_claim == 0` is the first-allocation case — there is no predecessor to release.
fn install_table(hash: &DbRef, old_claim: u32, new_claim: u32, stores: &mut [Store]) {
    keys::mut_store(hash, stores).set_u32_raw(hash.rec, hash.pos, new_claim);
    if old_claim != 0 {
        keys::mut_store(hash, stores).delete(old_claim);
    }
}

/// Move every entry of bucket table `from` into the freshly zeroed table `into`,
/// carrying the seed and the live-entry count.
///
/// The seed travels with the buckets because the bucket layout is seed-dependent: a
/// rebuild that minted a new one would place every existing record somewhere else than
/// its own lookup will later look.
fn rehash_into(hash: &DbRef, from: u32, into: u32, stores: &mut [Store], keys: &[Key]) {
    let seed = read_seed(keys::store(hash, stores), from);
    write_seed(keys::mut_store(hash, stores), into, seed);
    let length = keys::store(hash, stores).get_u32_raw(from, LEN_FLD);
    let elms = (keys::store(hash, stores).record_words(from) - 2) * 2;
    let mut move_rec = DbRef {
        store_nr: hash.store_nr,
        rec: 0,
        pos: 0,
    };
    for i in 0..elms {
        let v = keys::store(hash, stores).get_u32_raw(from, BUCKET0 + 4 * i);
        if v == 0 {
            continue;
        }
        move_rec.rec = v;
        move_rec.pos = 8;
        hash_set(into, &move_rec, stores, keys);
    }
    keys::mut_store(hash, stores).set_u32_raw(into, LEN_FLD, length);
}

fn hash_set(claim: u32, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let index = hash_free_pos(claim, rec, stores, keys);
    keys::mut_store(rec, stores).set_u32_raw(claim, index, rec.rec);
}

fn hash_free_pos(claim: u32, rec: &DbRef, stores: &[Store], keys: &[Key]) -> u32 {
    let room = keys::store(rec, stores).record_words(claim);
    let elms = (room - 2) * 2;
    let seed = read_seed(keys::store(rec, stores), claim);
    let hash_val = keys::hash(rec, stores, keys, seed);
    let mut index = (hash_val % u64::from(elms)) as u32;
    for _ in 0..elms {
        if keys::store(rec, stores).get_u32_raw(claim, BUCKET0 + index * 4) == 0 {
            break;
        }
        index += 1;
        if index >= elms {
            index = 0;
        }
    }
    BUCKET0 + index * 4
}

/// Return the 0-based slot index in `claim` that currently holds `rec.rec`.
fn hash_rec_pos(claim: u32, rec: &DbRef, stores: &[Store], keys: &[Key]) -> u32 {
    let room = keys::store(rec, stores).record_words(claim);
    let elms = (room - 2) * 2;
    let seed = read_seed(keys::store(rec, stores), claim);
    let hash_val = keys::hash(rec, stores, keys, seed);
    let mut index = (hash_val % u64::from(elms)) as u32;
    for _ in 0..elms {
        if keys::store(rec, stores).get_u32_raw(claim, BUCKET0 + index * 4) == rec.rec {
            break;
        }
        index += 1;
        if index >= elms {
            index = 0;
        }
    }
    index
}

#[must_use]
pub fn find(hash_ref: &DbRef, stores: &[Store], keys: &[Key], key: &[Content]) -> DbRef {
    let store = &stores[hash_ref.store_nr as usize];
    let claim = store.get_u32_raw(hash_ref.rec, hash_ref.pos);
    let mut record = DbRef {
        store_nr: hash_ref.store_nr,
        rec: 0,
        pos: 0,
    };
    if claim == 0 {
        return record;
    }
    let room = store.record_words(claim);
    if room == 0 {
        return record;
    }
    let elms = (room - 2) * 2;
    let seed = read_seed(store, claim);
    let hash_val = keys::key_hash(key, seed);
    let mut index = (hash_val % u64::from(elms)) as u32;
    let mut rec_pos = store.get_u32_raw(claim, BUCKET0 + index * 4);
    // @PLN135 arc B — a probe asks only *is this the key*, about the SAME key every
    // time, so the `(Content, type_nr)` match belongs outside the loop.  `fast_key`
    // resolves the field offset and the value once; the loop then reads the field
    // directly.  Same hash, same bucket order, same answer — measured at ~10 ns of a
    // ~33 ns cache-resident lookup on 1M `integer` keys, and it pays on INSERT too
    // (dedup runs one `find` per insert).  A compound key, or a width `fast_key` does
    // not list, answers `None` and takes the general loop below.
    if let Some(fast) = keys::fast_key(keys, key) {
        for _ in 0..elms {
            if rec_pos == 0 {
                record.rec = 0;
                record.pos = 0;
                break;
            }
            if fast.matches(store, rec_pos) {
                record.rec = rec_pos;
                record.pos = 8;
                break;
            }
            index += 1;
            if index >= elms {
                index = 0;
            }
            rec_pos = store.get_u32_raw(claim, BUCKET0 + index * 4);
        }
        return record;
    }
    'Record: for _ in 0..elms {
        if rec_pos == 0 {
            record.rec = 0;
            record.pos = 0;
            break;
        }
        record.rec = rec_pos;
        record.pos = 8;
        if keys::key_compare(key, &record, stores, keys) != Ordering::Equal {
            index += 1;
            if index >= elms {
                index = 0;
            }
            rec_pos = store.get_u32_raw(claim, BUCKET0 + index * 4);
            continue 'Record;
        }
        break;
    }
    record
}

pub fn remove(hash_ref: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    if rec.rec == 0 {
        return;
    }
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    let length = keys::store(hash_ref, stores).get_u32_raw(claim, LEN_FLD);
    if length == 0 {
        return;
    }
    let room = keys::store(hash_ref, stores).record_words(claim);
    let elms = (room - 2) * 2;
    let seed = read_seed(keys::store(hash_ref, stores), claim);
    // Find the slot holding rec and zero it (create the hole).
    let mut hole = hash_rec_pos(claim, rec, stores, keys);
    keys::mut_store(hash_ref, stores).set_u32_raw(claim, BUCKET0 + hole * 4, 0);
    // Walk forward from hole+1 and pull each element back if its probe distance
    // to the hole is shorter than its probe distance to its current slot.
    // Stop at the first empty slot (all probe chains end at one).
    let mut idx = (hole + 1) % elms;
    for _ in 0..elms {
        let val = keys::store(hash_ref, stores).get_u32_raw(claim, BUCKET0 + idx * 4);
        if val == 0 {
            break;
        }
        let next = DbRef {
            store_nr: hash_ref.store_nr,
            rec: val,
            pos: 8,
        };
        let ideal = (keys::hash(&next, stores, keys, seed) % u64::from(elms)) as u32;
        // Move if probe distance to hole is shorter than probe distance to idx.
        let d_hole = (hole + elms - ideal) % elms;
        let d_idx = (idx + elms - ideal) % elms;
        if d_hole < d_idx {
            keys::mut_store(hash_ref, stores).set_u32_raw(claim, BUCKET0 + hole * 4, val);
            keys::mut_store(hash_ref, stores).set_u32_raw(claim, BUCKET0 + idx * 4, 0);
            hole = idx;
        }
        idx = (idx + 1) % elms;
    }
    keys::mut_store(hash_ref, stores).set_u32_raw(claim, LEN_FLD, length - 1);
}

/**
Check the allocations and structure of the hash table.
# Panics
When the structure is not correctly filled
*/
/// Count the live records in a hash table.
///
/// Walks the bucket array (same loop as `records()` but counting
/// instead of collecting).  O(room) where `room` is the bucket array
/// length, typically ~1.5× the live-record count.  Returns 0 for an
/// uninitialised hash (no claim allocated yet).
///
/// Powers `len(h)` for `hash<T[key]>` (P192).
#[must_use]
pub fn count(hash_ref: &DbRef, stores: &[Store]) -> u32 {
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return 0;
    }
    let room = keys::store(hash_ref, stores).record_words(claim);
    let elms = (room - 2) * 2;
    let mut total: u32 = 0;
    for i in 0..elms {
        if keys::store(hash_ref, stores).get_u32_raw(claim, BUCKET0 + i * 4) != 0 {
            total += 1;
        }
    }
    total
}

/// Byte size of a hash's bucket table — the full table, holes included
/// (@PLN110 `size`).
///
/// The hash's own allocation is the bucket array: `elms` slots, each a 4-byte
/// `u32` record-id (an empty slot is a hole — a zero rec-id — and still counts,
/// because open addressing's spare capacity IS the format). Allocation-local:
/// the pointed-to entry records live in separate allocations and are NOT
/// counted. Excludes the two reserved header words (record header + seed),
/// mirroring `size(vector)` counting content, not the length prefix. Returns 0
/// for an uninitialised hash (no claim allocated yet).
#[must_use]
pub fn table_bytes(hash_ref: &DbRef, stores: &[Store]) -> u32 {
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return 0;
    }
    let room = keys::store(hash_ref, stores).record_words(claim);
    let elms = (room - 2) * 2;
    elms * 4
}

/// C60 Step 1: collect every live record's record-number from a hash.
///
/// Walks the hash's internal bucket array — the same traversal pattern
/// as `validate`, but appending each nonzero slot into a vector instead
/// of asserting.  Returned order is internal bucket order (unspecified
/// but stable for a given hash state) — callers that need a
/// user-visible ordering sort the result afterwards.
///
/// Runs in O(room) time where `room` is the bucket array length,
/// typically around 1.5× the live-record count.
#[must_use]
pub fn records(hash_ref: &DbRef, stores: &[Store]) -> Vec<u32> {
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    if claim == 0 {
        return Vec::new();
    }
    let room = keys::store(hash_ref, stores).record_words(claim);
    let elms = (room - 2) * 2;
    let mut out = Vec::new();
    for i in 0..elms {
        let rec = keys::store(hash_ref, stores).get_u32_raw(claim, BUCKET0 + i * 4);
        if rec != 0 {
            out.push(rec);
        }
    }
    out
}

/// C60 Step 2: collect every live record sorted by the hash's key.
///
/// Ascending on each key field, with `-` prefix flipping the direction
/// per-field — the existing `keys::compare` helper handles multi-field
/// lexicographic order and the descending bit for us, so one call
/// covers Steps 2 / 6 / 7 of the plan in CAVEATS.md C60.
///
/// Inefficient by design: walks the whole bucket array (Step 1) then
/// sorts the collected references in O(n log n).  Suitable for the
/// small hashes that scripting code typically iterates; users with a
/// tight loop over a large hash should pair the hash with a `vector`
/// or `sorted` for amortised traversal.
///
/// # Panics
///
/// Panics if `keys::compare` encounters a key field type it cannot
/// compare — same invariant as the existing `hash::find` path and
/// not reachable from valid loft source.
#[must_use]
#[allow(dead_code)]
pub fn records_sorted(hash_ref: &DbRef, stores: &[Store], keys: &[Key]) -> Vec<u32> {
    let mut recs = records(hash_ref, stores);
    // Build DbRefs once so the comparator doesn't re-materialise them.
    // Records in a hash all live in the same store and share the same
    // schema offset (pos=8 is the record body, matching what
    // `validate` uses internally).
    let store_nr = hash_ref.store_nr;
    recs.sort_by(|a, b| {
        let ra = DbRef {
            store_nr,
            rec: *a,
            pos: 8,
        };
        let rb = DbRef {
            store_nr,
            rec: *b,
            pos: 8,
        };
        keys::compare(&ra, &rb, stores, keys)
    });
    recs
}

/// Validate the bucket structure of a hash — each live slot's record
/// must `find` back to the same rec-nr, and the stored length must
/// match the number of nonzero slots.
///
/// # Panics
///
/// Panics via `assert_eq!` when the bucket structure is inconsistent
/// (a slot whose key does not round-trip through `find`, or a stored
/// length that does not match the actual live-slot count).  Used as a
/// debug-time structural invariant check; callers should never hit a
/// panic here in production.
pub fn validate(hash_ref: &DbRef, stores: &[Store], keys: &[Key]) {
    let claim = keys::store(hash_ref, stores).get_u32_raw(hash_ref.rec, hash_ref.pos);
    let length = keys::store(hash_ref, stores).get_u32_raw(claim, LEN_FLD);
    let room = keys::store(hash_ref, stores).record_words(claim);
    let elms = (room - 2) * 2;
    let mut record = DbRef {
        store_nr: hash_ref.store_nr,
        rec: 0,
        pos: 0,
    };
    let mut l = 0;
    for i in 0..elms {
        let rec = keys::store(hash_ref, stores).get_u32_raw(claim, BUCKET0 + i * 4);
        if rec != 0 {
            record.rec = rec;
            record.pos = 8;
            l += 1;
            let key = keys::get_key(&record, stores, keys);
            assert_eq!(
                find(hash_ref, stores, keys, &key).rec,
                rec,
                "Incorrect entry"
            );
        }
    }
    assert_eq!(length, l, "Incorrect hash length");
}
