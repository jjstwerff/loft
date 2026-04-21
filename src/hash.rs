// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use crate::keys;
use crate::keys::{Content, DbRef, Key};
use crate::store::Store;
use std::cmp::Ordering;

pub fn add(hash: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let mut claim = keys::store(hash, stores).get_u32_raw(hash.rec, hash.pos);
    let length = if claim == 0 {
        claim = keys::mut_store(hash, stores).claim(9);
        keys::mut_store(hash, stores).zero_fill(claim);
        keys::mut_store(hash, stores).set_u32_raw(hash.rec, hash.pos, claim);
        0
    } else {
        keys::store(hash, stores).get_u32_raw(claim, 4)
    };
    let room = *keys::store(hash, stores).addr::<i32>(claim, 0) as u32;
    let elms = (room - 1) * 2;
    if (length * 2 / 3) + 1 >= room {
        // rehash
        let mut move_rec = DbRef {
            store_nr: hash.store_nr,
            rec: 0,
            pos: 0,
        };
        let new_claim = keys::mut_store(hash, stores).claim(room * 2 - 1);
        keys::mut_store(hash, stores).zero_fill(new_claim);
        for i in 0..elms {
            let v = keys::store(hash, stores).get_u32_raw(claim, 8 + 4 * i);
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
    keys::mut_store(rec, stores).set_u32_raw(claim, 4, length + 1);
    // hash_validate(hash, key, stores, keys);
}

fn hash_set(claim: u32, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let index = hash_free_pos(claim, rec, stores, keys);
    keys::mut_store(rec, stores).set_u32_raw(claim, index, rec.rec);
}

fn hash_free_pos(claim: u32, rec: &DbRef, stores: &[Store], keys: &[Key]) -> u32 {
    let room = *keys::store(rec, stores).addr::<i32>(claim, 0) as u32;
    let elms = (room - 1) * 2;
    let hash_val = keys::hash(rec, stores, keys);
    let mut index = (hash_val % u64::from(elms)) as u32;
    for _ in 0..elms {
        if keys::store(rec, stores).get_u32_raw(claim, 8 + index * 4) == 0 {
            break;
        }
        index += 1;
        if index >= elms {
            index = 0;
        }
    }
    8 + index * 4
}

/// Return the 0-based slot index in `claim` that currently holds `rec.rec`.
fn hash_rec_pos(claim: u32, rec: &DbRef, stores: &[Store], keys: &[Key]) -> u32 {
    let room = *keys::store(rec, stores).addr::<i32>(claim, 0) as u32;
    let elms = (room - 1) * 2;
    let hash_val = keys::hash(rec, stores, keys);
    let mut index = (hash_val % u64::from(elms)) as u32;
    for _ in 0..elms {
        if keys::store(rec, stores).get_u32_raw(claim, 8 + index * 4) == rec.rec {
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
    let room = *store.addr::<i32>(claim, 0) as u32;
    if room == 0 {
        return record;
    }
    let elms = (room - 1) * 2;
    let hash_val = keys::key_hash(key);
    let mut index = (hash_val % u64::from(elms)) as u32;
    let mut rec_pos = store.get_u32_raw(claim, 8 + index * 4);
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
            rec_pos = store.get_u32_raw(claim, 8 + index * 4);
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
    let length = keys::store(hash_ref, stores).get_u32_raw(claim, 4);
    if length == 0 {
        return;
    }
    let room = *keys::store(hash_ref, stores).addr::<i32>(claim, 0) as u32;
    let elms = (room - 1) * 2;
    // Find the slot holding rec and zero it (create the hole).
    let mut hole = hash_rec_pos(claim, rec, stores, keys);
    keys::mut_store(hash_ref, stores).set_u32_raw(claim, 8 + hole * 4, 0);
    // Walk forward from hole+1 and pull each element back if its probe distance
    // to the hole is shorter than its probe distance to its current slot.
    // Stop at the first empty slot (all probe chains end at one).
    let mut idx = (hole + 1) % elms;
    for _ in 0..elms {
        let val = keys::store(hash_ref, stores).get_u32_raw(claim, 8 + idx * 4);
        if val == 0 {
            break;
        }
        let next = DbRef {
            store_nr: hash_ref.store_nr,
            rec: val,
            pos: 8,
        };
        let ideal = (keys::hash(&next, stores, keys) % u64::from(elms)) as u32;
        // Move if probe distance to hole is shorter than probe distance to idx.
        let d_hole = (hole + elms - ideal) % elms;
        let d_idx = (idx + elms - ideal) % elms;
        if d_hole < d_idx {
            keys::mut_store(hash_ref, stores).set_u32_raw(claim, 8 + hole * 4, val);
            keys::mut_store(hash_ref, stores).set_u32_raw(claim, 8 + idx * 4, 0);
            hole = idx;
        }
        idx = (idx + 1) % elms;
    }
    keys::mut_store(hash_ref, stores).set_u32_raw(claim, 4, length - 1);
}

/**
Check the allocations and structure of the hash table.
# Panics
When the structure is not correctly filled
*/
// C60 Step 1a: this primitive is used from tests today; the stdlib
// `hash_records` wrapper in Step 2 will be its first production caller.
#[allow(dead_code)]
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
    let room = *keys::store(hash_ref, stores).addr::<i32>(claim, 0) as u32;
    let elms = (room - 1) * 2;
    let mut out = Vec::new();
    for i in 0..elms {
        let rec = keys::store(hash_ref, stores).get_u32_raw(claim, 8 + i * 4);
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
    let length = keys::store(hash_ref, stores).get_u32_raw(claim, 4);
    let room = *keys::store(hash_ref, stores).addr::<i32>(claim, 0) as u32;
    let elms = (room - 1) * 2;
    let mut record = DbRef {
        store_nr: hash_ref.store_nr,
        rec: 0,
        pos: 0,
    };
    let mut l = 0;
    for i in 0..elms {
        let rec = keys::store(hash_ref, stores).get_u32_raw(claim, 8 + i * 4);
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
