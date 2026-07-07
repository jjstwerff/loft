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
    let elms = (room - 2) * 2;
    // Grow at load factor 0.75 (= 0.75·elms).  The `+ 2` counts the two
    // reserved words (header + seed) before the bucket array: rehash when
    // `length >= 1.5·(room - 2) = 0.75·elms`.  (Was `+ 1` when only the
    // header word was reserved.)
    if (length * 2 / 3) + 2 >= room {
        // rehash
        let mut move_rec = DbRef {
            store_nr: hash.store_nr,
            rec: 0,
            pos: 0,
        };
        let seed = read_seed(keys::store(hash, stores), claim);
        let new_claim = keys::mut_store(hash, stores).claim(room * 2 - 1);
        keys::mut_store(hash, stores).zero_fill(new_claim);
        // Carry the seed across the resize — the bucket layout is
        // seed-dependent, so a rehash must reuse the same seed.
        write_seed(keys::mut_store(hash, stores), new_claim, seed);
        for i in 0..elms {
            let v = keys::store(hash, stores).get_u32_raw(claim, BUCKET0 + 4 * i);
            if v == 0 {
                continue;
            }
            move_rec.rec = v;
            move_rec.pos = 8;
            hash_set(new_claim, &move_rec, stores, keys);
        }
        claim = new_claim;
        keys::mut_store(hash, stores).set_u32_raw(hash.rec, hash.pos, claim);
    }
    hash_set(claim, rec, stores, keys);
    keys::mut_store(rec, stores).set_u32_raw(claim, LEN_FLD, length + 1);
    // hash_validate(hash, key, stores, keys);
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
