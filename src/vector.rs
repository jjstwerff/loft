// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use crate::keys;
use crate::keys::{Content, DbRef, Key};
use crate::store::Store;
use std::cmp::Ordering;

/// Checked vector position — `8 + index * size` using u64 to detect overflow.
#[inline]
fn checked_vec_pos(index: u32, size: u32) -> u32 {
    let pos = u64::from(index) * u64::from(size) + 8;
    u32::try_from(pos)
        .unwrap_or_else(|_| panic!("Vector position overflow: index={index} size={size}"))
}

/// Checked vector capacity — `(count * size + 15) / 8` using u64.
#[inline]
fn checked_vec_cap(count: u32, size: u32) -> u32 {
    let bytes = u64::from(count) * u64::from(size) + 15;
    u32::try_from(bytes / 8)
        .unwrap_or_else(|_| panic!("Vector capacity overflow: count={count} size={size}"))
}

// TODO change slice to its own vector on updating it
pub fn insert_vector(db: &DbRef, size: u32, index: i32, stores: &mut [Store]) -> DbRef {
    let len = length_vector(db, stores);
    let real = if index < 0 { index + len as i32 } else { index };
    if real < 0 || real > len as i32 {
        return DbRef {
            store_nr: db.store_nr,
            rec: 0,
            pos: 0,
        };
    }
    let store = keys::mut_store(db, stores);
    let mut vec_rec = store.get_int(db.rec, db.pos) as u32;
    let new_length;
    if vec_rec == 0 {
        // claim a new array with minimal 11 elements
        vec_rec = store.claim(checked_vec_cap(11, size));
        store.set_int(db.rec, db.pos, vec_rec as i32);
        new_length = 1;
    } else {
        new_length = len + 1;
        let new_vec = store.resize(vec_rec, checked_vec_cap(new_length, size));
        if new_vec != vec_rec {
            store.set_int(db.rec, db.pos, new_vec as i32);
            vec_rec = new_vec;
        }
        store.copy_block(
            new_vec,
            checked_vec_pos(real as u32, size) as isize,
            new_vec,
            checked_vec_pos(real as u32 + 1, size) as isize,
            (len as isize - real as isize) * size as isize,
        );
    }
    store.set_int(vec_rec, 4, new_length as i32);
    DbRef {
        store_nr: db.store_nr,
        rec: vec_rec,
        pos: checked_vec_pos(real as u32, size),
    }
}

/**
Claim more space in a vector to allow for new records. Return the next reference after the last
records though do not increase the length yet as we might want to iterate the vector before the
actual change.
*/
/// O8.1a: Pre-allocate a vector record with capacity for `count` elements.
/// Sets the vector pointer and length=0.  Subsequent `vector_append` calls
/// will find enough space and never call `store.resize`.
pub fn pre_alloc_vector(db: &DbRef, count: u32, elem_size: u32, stores: &mut [Store]) {
    let store = keys::mut_store(db, stores);
    if db.rec == 0 {
        return;
    }
    let vec_rec = store.get_int(db.rec, db.pos) as u32;
    if vec_rec != 0 {
        return; // already allocated — don't overwrite
    }
    // Match vector_append's minimum of 11 elements to avoid OOB on remove/shift.
    let alloc_count = count.max(11);
    let words = checked_vec_cap(alloc_count, elem_size);
    let new_rec = store.claim(words);
    store.set_int(db.rec, db.pos, new_rec as i32);
    store.set_int(new_rec, 4, 0); // length = 0
}

pub fn vector_append(db: &DbRef, size: u32, stores: &mut [Store]) -> DbRef {
    let store = keys::mut_store(db, stores);
    if db.rec == 0 {
        return DbRef {
            store_nr: db.store_nr,
            rec: 0,
            pos: 0,
        };
    }
    let mut vec_rec = store.get_int(db.rec, db.pos) as u32;
    let pos = if vec_rec == 0 {
        // new array
        vec_rec = store.claim(checked_vec_cap(11, size)); // minimal 11 elements
        store.set_int(db.rec, db.pos, vec_rec as i32);
        store.set_int(vec_rec, 4, 0); // initial length
        0
    } else {
        let length = store.get_int(vec_rec, 4) as u32;
        let new_vec = store.resize(vec_rec, checked_vec_cap(length + 1, size));
        if new_vec != vec_rec {
            store.set_int(db.rec, db.pos, new_vec as i32);
            vec_rec = new_vec;
        }
        length
    };
    DbRef {
        store_nr: db.store_nr,
        rec: vec_rec,
        pos: checked_vec_pos(pos, size),
    }
}

pub fn vector_finish(db: &DbRef, stores: &mut [Store]) {
    if db.rec == 0 {
        return;
    }
    let store = keys::mut_store(db, stores);
    let vec_rec = store.get_int(db.rec, db.pos) as u32;
    let length = store.get_int(vec_rec, 4);
    store.set_int(vec_rec, 4, length + 1);
}

pub fn sorted_new(db: &DbRef, size: u32, stores: &mut [Store]) -> DbRef {
    // Keep an extra record between the current and the new one.
    // This is needed to allow to create a new open space to move the new record to.
    let store = keys::mut_store(db, stores);
    let mut sorted_rec = store.get_int(db.rec, db.pos) as u32;
    // Claim a record at the back of the current structure or create a new structure.
    if sorted_rec == 0 {
        sorted_rec = store.claim(checked_vec_cap(12, size));
        store.set_int(db.rec, db.pos, sorted_rec as i32);
        // Set initial length to 0
        store.set_int(sorted_rec, 4, 0);
        // return the first record
        DbRef {
            store_nr: db.store_nr,
            rec: sorted_rec,
            pos: 8,
        }
    } else {
        let length = store.get_int(sorted_rec, 4) as u32;
        let new_sorted = store.resize(sorted_rec, checked_vec_cap(length + 2, size));
        if new_sorted != sorted_rec {
            store.set_int(db.rec, db.pos, new_sorted as i32);
            sorted_rec = new_sorted;
        }
        // return the last record inside the allocation
        DbRef {
            store_nr: db.store_nr,
            rec: sorted_rec,
            pos: checked_vec_pos(length + 1, size),
        }
    }
}

pub fn sorted_finish(sorted: &DbRef, size: u32, keys: &[Key], stores: &mut [Store]) {
    let sorted_rec = keys::store(sorted, stores).get_int(sorted.rec, sorted.pos) as u32;
    let length = keys::store(sorted, stores).get_int(sorted_rec, 4) as u32;
    if length == 0 {
        // we do not have to reorder the first inserted record; set length to 1
        keys::mut_store(sorted, stores).set_int(sorted_rec, 4, 1);
        return;
    }
    let latest_pos = checked_vec_pos(length + 1, size);
    let rec = DbRef {
        store_nr: sorted.store_nr,
        rec: sorted_rec,
        pos: latest_pos,
    };
    let key = keys::get_key(&rec, stores, keys);
    let (pos, _) = sorted_find(sorted, true, size as u16, stores, keys, &key);
    let store = keys::mut_store(sorted, stores);
    let end_pos = length;
    if pos < end_pos {
        // create space to write the new record to
        store.copy_block(
            sorted_rec,
            checked_vec_pos(pos, size) as isize,
            sorted_rec,
            checked_vec_pos(pos + 1, size) as isize,
            ((end_pos - pos) * size) as isize,
        );
    }
    // move last record to the found correct position
    store.copy_block(
        sorted_rec,
        latest_pos as isize,
        sorted_rec,
        checked_vec_pos(pos, size) as isize,
        size as isize,
    );
    store.set_int(sorted_rec, 4, (length + 1) as i32);
}

pub fn ordered_finish(sorted: &DbRef, rec: &DbRef, keys: &[Key], stores: &mut [Store]) {
    let rec_ref = sorted_new(sorted, 4, stores);
    let sorted_rec = keys::store(sorted, stores).get_int(sorted.rec, sorted.pos) as u32;
    let length = keys::store(sorted, stores).get_int(sorted_rec, 4) as u32;
    if length == 0 {
        // we do not have to reorder the first inserted record, set length to 1
        keys::mut_store(sorted, stores).set_int(sorted_rec, 4, 1);
        keys::mut_store(sorted, stores).set_int(sorted_rec, rec_ref.pos, rec.rec as i32);
        return;
    }
    let key = keys::get_key(rec, stores, keys);
    let pos = ordered_find(sorted, true, stores, keys, &key).0;
    let latest_pos = 8 + length * 4;
    if latest_pos > pos {
        keys::mut_store(sorted, stores).copy_block(
            sorted_rec,
            8 + pos as isize * 4,
            sorted_rec,
            12 + pos as isize * 4,
            (latest_pos - pos * 4) as isize,
        );
    }
    keys::mut_store(&rec_ref, stores).set_int(sorted_rec, 8 + pos * 4, rec.rec as i32);
    keys::mut_store(sorted, stores).set_int(sorted_rec, 4, 1 + length as i32);
}

#[must_use]
pub fn length_vector(db: &DbRef, stores: &[Store]) -> u32 {
    if db.rec == 0 || db.pos == 0 {
        return 0;
    }
    let store = keys::store(db, stores);
    let v_rec = store.get_int(db.rec, db.pos) as u32;
    if v_rec == 0 {
        0
    } else {
        store.get_int(v_rec, 4) as u32
    }
}

pub fn clear_vector(db: &DbRef, stores: &mut [Store]) {
    let store = keys::mut_store(db, stores);
    let v_rec = store.get_int(db.rec, db.pos) as u32;
    if v_rec != 0 {
        // Only set size of the vector to 0
        // TODO when the main path to a separate allocated objects: remove these
        // TODO lower string reference counts where needed
        store.set_int(v_rec, 4, 0);
    }
}

#[must_use]
pub fn get_vector(db: &DbRef, size: u32, from: i32, stores: &[Store]) -> DbRef {
    #[cfg(debug_assertions)]
    if db.store_nr != u16::MAX {
        debug_assert!(
            !stores[db.store_nr as usize].free,
            "get_vector: use-after-free on store {} (rec={} pos={})",
            db.store_nr, db.rec, db.pos
        );
    }
    let store = keys::store(db, stores);
    if from == i32::MIN {
        return DbRef {
            store_nr: db.store_nr,
            rec: 0,
            pos: 0,
        };
    }
    let v_rec = store.get_int(db.rec, db.pos) as u32;
    let l = length_vector(db, stores);
    let f = if from < 0 { from + l as i32 } else { from };
    if f < 0 || f >= l as i32 {
        DbRef {
            store_nr: db.store_nr,
            rec: 0,
            pos: 0,
        }
    } else {
        DbRef {
            store_nr: db.store_nr,
            rec: v_rec,
            pos: checked_vec_pos(f as u32, size),
        }
    }
}

pub fn remove_vector(db: &DbRef, size: u32, index: i32, stores: &mut [Store]) -> bool {
    let len = i64::from(length_vector(db, stores));
    let store = keys::mut_store(db, stores);
    let vec_rec = store.get_int(db.rec, db.pos) as u32;
    let i = if index < 0 {
        i64::from(index) + len
    } else {
        i64::from(index)
    };
    if i >= len || i < 0 || vec_rec == 0 {
        return false;
    }
    if len - i > 1 {
        store.copy_block(
            vec_rec,
            checked_vec_pos(i as u32 + 1, size) as isize,
            vec_rec,
            checked_vec_pos(i as u32, size) as isize,
            (len as isize - i as isize) * size as isize,
        );
    }
    store.set_int(vec_rec, 4, len as i32 - 1);
    true
}

/**
With before this returns index+1 before any matching element.
Otherwise, return the index of the element after.
*/
#[must_use]
pub fn sorted_find(
    sorted: &DbRef,
    before: bool,
    size: u16,
    stores: &[Store],
    keys: &[Key],
    key: &[Content],
) -> (u32, bool) {
    if sorted.rec == 0 {
        return (0, false);
    }
    let store = keys::store(sorted, stores);
    let sorted_rec = store.get_int(sorted.rec, sorted.pos) as u32;
    if sorted_rec == 0 {
        return (0, false);
    }
    let length = store.get_int(sorted_rec, 4) as u32;
    if length == 0 {
        return (0, false);
    }
    let mut result = DbRef {
        store_nr: sorted.store_nr,
        rec: sorted_rec,
        pos: 0,
    };
    let mut left = 0;
    let mut right = length - 1;
    let mut found = false;
    loop {
        let mid = left + (right - left) / 2;
        result.pos = 8 + mid * u32::from(size);
        let cmp = keys::key_compare(key, &result, stores, keys);
        let action = if cmp == Ordering::Equal {
            found = true;
            if before {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            cmp
        };
        if action == Ordering::Less {
            if mid > 0 {
                right = mid - 1;
            } else {
                right = 0;
                left += 1;
            }
        } else {
            left = mid + 1;
        }
        if left > right {
            return (
                if action == Ordering::Greater {
                    mid + 1
                } else {
                    mid
                },
                found,
            );
        }
    }
}

#[must_use]
pub fn ordered_find(
    sorted: &DbRef,
    before: bool,
    stores: &[Store],
    keys: &[Key],
    key: &[Content],
) -> (u32, bool) {
    let store = keys::store(sorted, stores);
    let sorted_rec = store.get_int(sorted.rec, sorted.pos) as u32;
    let length = store.get_int(sorted_rec, 4) as u32;
    let mut result = DbRef {
        store_nr: sorted.store_nr,
        rec: 0,
        pos: 0,
    };
    if sorted_rec == 0 {
        return (0, false);
    }
    let mut found = false;
    let mut left = 0;
    let mut right = length - 1;
    loop {
        let mid = (left + right + 1) >> 1;
        result.rec = store.get_int(sorted_rec, 8 + mid * 4) as u32;
        result.pos = 8;
        let cmp = keys::key_compare(key, &result, stores, keys);
        let action = if cmp == Ordering::Equal {
            found = true;
            if before {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            cmp
        };
        if action == Ordering::Less {
            if mid > 0 {
                right = mid - 1;
            } else {
                right = 0;
                left += 1;
            }
        } else {
            left = mid + 1;
        }
        if left > right {
            return (
                if action == Ordering::Greater {
                    mid + 1
                } else {
                    mid
                },
                found,
            );
        }
    }
}

pub fn vector_next(data: &DbRef, pos: &mut i32, size: u16, stores: &[Store]) {
    let rec = keys::store(data, stores).get_int(data.rec, data.pos) as u32;
    if rec == 0 {
        *pos = i32::MAX;
        return;
    }
    let length = keys::store(data, stores).get_int(rec, 4);
    if *pos == i32::MAX && length != 0 {
        *pos = 8;
    } else if length != 0 && *pos < 8 + (length - 1) * i32::from(size) {
        *pos += i32::from(size);
    } else {
        *pos = i32::MAX;
    }
}

pub fn vector_step(data: &DbRef, pos: &mut i32, stores: &[Store]) {
    let rec = keys::store(data, stores).get_int(data.rec, data.pos) as u32;
    if rec == 0 {
        *pos = i32::MAX;
        return;
    }
    let length = keys::store(data, stores).get_int(rec, 4);
    if *pos == i32::MAX && length != 0 {
        *pos = 0;
    } else if length != 0 && *pos < length - 1 {
        *pos += 1;
    } else {
        *pos = i32::MAX;
    }
}

/// Advance the sorted-vector position one step backwards (reverse iteration).
/// `pos == i32::MAX` or `pos >= length` is the not-yet-started sentinel;
/// the first call sets `pos` to `length - 1` (last element).
/// Returns `i32::MAX` when the iterator has moved past the first element.
pub fn vector_step_rev(data: &DbRef, pos: &mut i32, stores: &[Store]) {
    let rec = keys::store(data, stores).get_int(data.rec, data.pos) as u32;
    if rec == 0 {
        *pos = i32::MAX;
        return;
    }
    let length = keys::store(data, stores).get_int(rec, 4);
    if length == 0 || *pos == i32::MAX || *pos >= length {
        // Not started yet (sentinel) or past the end — begin at the last element.
        *pos = if length == 0 { i32::MAX } else { length - 1 };
    } else if *pos > 0 {
        *pos -= 1;
    } else {
        *pos = i32::MAX; // Passed the beginning.
    }
}

/// Sort a vector of primitive elements in-place (ascending).
/// `elem_size` is the byte size of each element (1, 2, 4, or 8).
/// `is_float` must be true for floating-point types (f32 at size=4, f64 at size=8).
pub fn sort_vector(db: &DbRef, elem_size: u16, is_float: bool, stores: &mut [Store]) {
    let len = length_vector(db, stores) as usize;
    if len < 2 {
        return;
    }
    let store = keys::mut_store(db, stores);
    let v_rec = store.get_int(db.rec, db.pos) as u32;
    if v_rec == 0 {
        return;
    }
    match elem_size {
        1 => {
            let mut vals: Vec<i32> = (0..len)
                .map(|i| store.get_byte(v_rec, 8 + (i as u32), 0))
                .collect();
            vals.sort_unstable();
            for (i, &v) in vals.iter().enumerate() {
                store.set_byte(v_rec, 8 + (i as u32), 0, v);
            }
        }
        2 => {
            let mut vals: Vec<i32> = (0..len)
                .map(|i| store.get_short(v_rec, 8 + (i as u32) * 2, 0))
                .collect();
            vals.sort_unstable();
            for (i, &v) in vals.iter().enumerate() {
                store.set_short(v_rec, 8 + (i as u32) * 2, 0, v);
            }
        }
        4 => {
            if is_float {
                let mut vals: Vec<f32> = (0..len)
                    .map(|i| f32::from_bits(store.get_int(v_rec, 8 + (i as u32) * 4) as u32))
                    .collect();
                vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Greater));
                for (i, &v) in vals.iter().enumerate() {
                    store.set_int(v_rec, 8 + (i as u32) * 4, v.to_bits() as i32);
                }
            } else {
                let mut vals: Vec<i32> = (0..len)
                    .map(|i| store.get_int(v_rec, 8 + (i as u32) * 4))
                    .collect();
                vals.sort_unstable();
                for (i, &v) in vals.iter().enumerate() {
                    store.set_int(v_rec, 8 + (i as u32) * 4, v);
                }
            }
        }
        8 => {
            if is_float {
                let mut vals: Vec<f64> = (0..len)
                    .map(|i| store.get_float(v_rec, 8 + (i as u32) * 8))
                    .collect();
                vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Greater));
                for (i, &v) in vals.iter().enumerate() {
                    store.set_float(v_rec, 8 + (i as u32) * 8, v);
                }
            } else {
                let mut vals: Vec<i64> = (0..len)
                    .map(|i| store.get_long(v_rec, 8 + (i as u32) * 8))
                    .collect();
                vals.sort_unstable();
                for (i, &v) in vals.iter().enumerate() {
                    store.set_long(v_rec, 8 + (i as u32) * 8, v);
                }
            }
        }
        _ => {} // unsupported element size — no-op
    }
}

/// Reverse a vector in-place by swapping elements from the ends toward the middle.
pub fn reverse_vector(db: &DbRef, elem_size: u32, stores: &mut [Store]) {
    let len = length_vector(db, stores);
    if len < 2 {
        return;
    }
    let store = keys::mut_store(db, stores);
    let v_rec = store.get_int(db.rec, db.pos) as u32;
    if v_rec == 0 {
        return;
    }
    let mut buf = vec![0u8; elem_size as usize];
    let mut lo = 0u32;
    let mut hi = len - 1;
    while lo < hi {
        let lo_pos = 8 + lo * elem_size;
        let hi_pos = 8 + hi * elem_size;
        // Copy lo → buf
        for i in 0..elem_size {
            buf[i as usize] = store.get_byte(v_rec, lo_pos + i, 0) as u8;
        }
        // Copy hi → lo
        for i in 0..elem_size {
            let v = store.get_byte(v_rec, hi_pos + i, 0);
            store.set_byte(v_rec, lo_pos + i, 0, v);
        }
        // Copy buf → hi
        for i in 0..elem_size {
            store.set_byte(v_rec, hi_pos + i, 0, i32::from(buf[i as usize]));
        }
        lo += 1;
        hi -= 1;
    }
}
