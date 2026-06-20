// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Find/search operations.

use crate::database::{Parts, Stores};
use crate::keys::{Content, DbRef};
use crate::vector;
use crate::{hash, keys, tree};
use std::cmp::Ordering;

#[allow(dead_code)]
fn compare(a: &Content, b: &Content) -> Ordering {
    match (a, b) {
        (Content::Long(a), Content::Long(b)) => i64::cmp(a, b),
        (Content::Single(a), Content::Single(b)) => {
            if a > b {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (Content::Float(a), Content::Float(b)) => {
            if a > b {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (Content::Str(a), Content::Str(b)) => str::cmp(a.str(), b.str()),
        _ => panic!("Undefined compare {a:?} vs {b:?}"),
    }
}

impl Stores {
    #[allow(dead_code)]
    pub(super) fn get_key(&self, fld: &DbRef, db: u16, keys: &[(u16, bool)]) -> Vec<Content> {
        let mut key = Vec::new();
        for (k, _) in keys {
            key.push(self.field_content(fld, db, *k));
        }
        key
    }

    #[must_use]
    pub fn fields(&self, tp: u16) -> u16 {
        if let Parts::Index(c, _, f) = self.types[tp as usize].parts {
            // @PLN25 — nullable `index<__nullable<S>>`: the content `c` is the synth enum
            // (`Parts::Enum`, no field list); the LLRB bookkeeping (#left/#right/#color) lives
            // on the `Some` variant's record (where `database.index` appended it).  Resolve
            // through `Some` so the bookkeeping offset is read from the actual stored struct;
            // else `c` (the enum) misses both arms below → `u16::MAX` → `tree::add` writes at
            // 0xFFFF (the `Fld 65543 outside record` OOB on a keyed-clear repopulate).
            let c = self.nullable_some_variant(c).unwrap_or(c);
            if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                &self.types[c as usize].parts
            {
                8 + fields[f as usize].position
            } else {
                u16::MAX
            }
        } else {
            u16::MAX
        }
    }

    #[must_use]
    pub fn keys(&self, tp: u16) -> &[crate::keys::Key] {
        &self.types[tp as usize].keys
    }

    #[allow(dead_code)]
    pub(super) fn field_content(&self, rec: &DbRef, db: u16, key: u16) -> Content {
        let store = self.store(rec);
        // Resolve key field `key` to its (content type, absolute byte position) via the
        // SAME `key_field` chokepoint `determine_keys` bakes from — so a synth
        // `__nullable<S>` element's payload base is added identically here and there.
        if let Some((content, position)) = self.key_field(db, key) {
            let pos = rec.pos + u32::from(position);
            return match content {
                0 => Content::Long(store.get_int(rec.rec, pos)),
                6 => Content::Long(i64::from(store.get_u32_raw(rec.rec, pos))),
                1 => Content::Long(store.get_long(rec.rec, pos)),
                2 => Content::Single(store.get_single(rec.rec, pos)),
                3 => Content::Float(store.get_float(rec.rec, pos)),
                4 => Content::Long(i64::from(store.get_byte(rec.rec, pos, 0))),
                5 => Content::Str(crate::keys::Str::new(
                    store.get_str(store.get_u32_raw(rec.rec, pos)),
                )),
                _ => {
                    if let Parts::Enum(_) = self.types[content as usize].parts {
                        Content::Long(i64::from(store.get_byte(rec.rec, pos, 0)))
                    } else {
                        panic!(
                            "Unknown key type {} (field {key} of {})",
                            self.types[content as usize].name, self.types[db as usize].name,
                        )
                    }
                }
            };
        }
        Content::Long(0)
    }

    /**
    Find a record on a given key.
    # Panics
    When the given database type doesn't support searcher.
    */
    #[must_use]
    pub(super) fn find_vector(&self, data: &DbRef, c: u16, key: &[Content]) -> DbRef {
        if let Content::Long(v) = key[0] {
            vector::get_vector(
                data,
                u32::from(self.types[c as usize].size),
                v,
                &self.allocations,
            )
        } else {
            DbRef {
                store_nr: data.store_nr,
                rec: if data.rec == 0 || self.store(data).get_u32_raw(data.rec, 4) == 0 {
                    0
                } else {
                    self.store(data).get_u32_raw(data.rec, 0)
                },
                pos: 8,
            }
        }
    }

    pub(super) fn find_array(&self, data: &DbRef, c: u16, key: &[Content]) -> DbRef {
        if let Content::Long(v) = key[0] {
            let res = vector::get_vector(
                data,
                u32::from(self.types[c as usize].size),
                v,
                &self.allocations,
            );
            DbRef {
                store_nr: res.store_nr,
                rec: if res.rec == 0 {
                    0
                } else {
                    self.store(&res).get_u32_raw(res.rec, res.pos)
                },
                pos: 8,
            }
        } else {
            DbRef {
                store_nr: data.store_nr,
                rec: if data.rec == 0 || self.store(data).get_u32_raw(data.rec, 4) == 0 {
                    0
                } else {
                    let rec = self.store(data).get_u32_raw(data.rec, 0);
                    self.store(data).get_u32_raw(rec, 8)
                },
                pos: 8,
            }
        }
    }

    /**
    Find a record on a given key.
    # Panics
    When the given database type doesn't support searching.
    */
    #[must_use]
    pub fn find(&self, data: &DbRef, db: u16, key: &[Content]) -> DbRef {
        match &self.types[db as usize].parts {
            Parts::Vector(c) => self.find_vector(data, *c, key),
            Parts::Array(c) => self.find_array(data, *c, key),
            Parts::Sorted(c, _) => {
                let (pos, found) = vector::sorted_find(
                    data,
                    true,
                    self.types[*c as usize].size,
                    &self.allocations,
                    &self.types[db as usize].keys,
                    key,
                );
                if found {
                    DbRef {
                        store_nr: data.store_nr,
                        rec: self.store(data).get_u32_raw(data.rec, data.pos),
                        pos: 8 + pos * u32::from(self.types[*c as usize].size),
                    }
                } else {
                    DbRef {
                        store_nr: data.store_nr,
                        rec: 0,
                        pos: 0,
                    }
                }
            }
            Parts::Ordered(_, _) => {
                let sorted_rec = self.store(data).get_u32_raw(data.rec, data.pos);
                let (pos, found) = vector::ordered_find(
                    data,
                    true,
                    &self.allocations,
                    &self.types[db as usize].keys,
                    key,
                );
                if found {
                    DbRef {
                        store_nr: data.store_nr,
                        rec: self.store(data).get_u32_raw(sorted_rec, 8 + pos * 4),
                        pos: 8,
                    }
                } else {
                    DbRef {
                        store_nr: data.store_nr,
                        rec: 0,
                        pos: 0,
                    }
                }
            }
            Parts::Hash(_, _) => hash::find(data, &self.allocations, self.keys(db), key),
            Parts::Index(rec_nr, _, left_field) => {
                self.find_index(data, *rec_nr, *left_field, db, key)
            }
            Parts::Base
            | Parts::Struct(_)
            | Parts::Enum(_)
            | Parts::EnumValue(_, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Int(_, _)
            | Parts::DbRef
            | Parts::ChildRec(_)
            | Parts::Spacial(_, _) => panic!(
                "find called on non-collection type: {} (db={})",
                self.types[db as usize].name, db
            ),
        }
    }

    pub(super) fn find_index(
        &self,
        data: &DbRef,
        rec_nr: u16,
        left_field: u16,
        db: u16,
        key: &[Content],
    ) -> DbRef {
        // @PLN25 — nullable `index<__nullable<S>>`: `rec_nr` is the synth enum (`Parts::Enum`);
        // resolve to the `Some` variant where the LLRB bookkeeping lives (mirrors `fields()`).
        let rec_nr = self.nullable_some_variant(rec_nr).unwrap_or(rec_nr);
        let left = if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[rec_nr as usize].parts
        {
            8 + fields[left_field as usize].position
        } else {
            u16::MAX
        };
        let rec = tree::find(data, true, left, &self.allocations, self.keys(db), key);
        let mut result = DbRef {
            store_nr: data.store_nr,
            rec,
            pos: 8,
        };
        result.rec = if rec == 0 {
            tree::first(data, left, &self.allocations).rec
        } else {
            tree::next(
                keys::store(&result, &self.allocations),
                &DbRef {
                    store_nr: result.store_nr,
                    rec,
                    pos: u32::from(left),
                },
            )
        };
        let cmp = keys::key_compare(key, &result, &self.allocations, self.keys(db));
        if cmp == Ordering::Equal {
            result
        } else {
            DbRef {
                store_nr: data.store_nr,
                rec: 0,
                pos: 0,
            }
        }
    }

    #[must_use]
    pub fn get_keys(&self, db: u16) -> Vec<u16> {
        match &self.types[db as usize].parts {
            Parts::Vector(_) | Parts::Array(_) => vec![0],
            Parts::Sorted(c, key) | Parts::Ordered(c, key) | Parts::Index(c, key, _) => {
                // Key content TYPES (for `read_key` to pop the right widths).  Route through
                // the `key_field` chokepoint so a synth `__nullable<S>` element resolves the
                // key inside the `Some` payload, matching what `determine_keys` baked.
                key.iter()
                    .filter_map(|(k, _)| self.key_field(*c, *k).map(|(content, _)| content))
                    .collect()
            }
            Parts::Hash(c, key) => key
                .iter()
                .filter_map(|k| self.key_field(*c, *k).map(|(content, _)| content))
                .collect(),
            _ => Vec::new(),
        }
    }

    /**
    Validate the structure in any way possible.
    What is still open to validate:
    - individual allocations inside store size
    - length of vector/sorted/array/ordered stays within allocation
    - when called fully; but allow for single vector:
      - allocations linked together correctly (linked from previous and to next)
      - open space validation
      - references of array/ordered/separate to correct allocations
    # Panics
    When the structure is not correct
    */
    pub fn validate(&mut self, data: &DbRef, db: u16) {
        match self.types[db as usize].parts.clone() {
            Parts::Hash(_, _) => {
                hash::validate(data, &self.allocations, &self.types[db as usize].keys);
            }
            Parts::Index(_, _, fields) => {
                tree::validate(
                    data,
                    fields,
                    &self.allocations,
                    &self.types[db as usize].keys,
                );
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields {
                    self.validate(
                        &DbRef {
                            store_nr: data.store_nr,
                            rec: data.rec,
                            pos: data.pos + u32::from(f.position),
                        },
                        f.content,
                    );
                }
            }
            _ => (),
        }
    }

    /**
    Get the next record given a specific point in a structure.
    # Panics
    When not in a valid structure
    */
    pub(super) fn next(&self, data: &DbRef, pos: &mut i32, db: u16) -> DbRef {
        match &self.types[db as usize].parts {
            Parts::Vector(c) | Parts::Sorted(c, _) => {
                vector::vector_next(data, pos, self.types[*c as usize].size, &self.allocations);
                self.element_reference(data, *pos)
            }
            Parts::Array(_) => {
                vector::vector_next(data, pos, 4, &self.allocations);
                let r = self.store(data).get_u32_raw(data.rec, data.pos);
                self.db_ref(data, *pos, r)
            }
            Parts::Ordered(_, _) => {
                vector::vector_next(data, pos, 4, &self.allocations);
                if *pos == i32::MAX {
                    return DbRef {
                        store_nr: data.store_nr,
                        rec: 0,
                        pos: 0,
                    };
                }
                let r = self.store(data).get_u32_raw(data.rec, data.pos);
                DbRef {
                    store_nr: data.store_nr,
                    rec: self.store(data).get_u32_raw(r, *pos as u32),
                    pos: 8,
                }
            }
            Parts::Index(_, _, _) => {
                if *pos == i32::MAX {
                    let n = tree::first(data, self.fields(db), &self.allocations);
                    *pos = n.rec as i32;
                    return n;
                }
                let store = keys::store(data, &self.allocations);
                let mut rec = DbRef {
                    store_nr: data.store_nr,
                    rec: *pos as u32,
                    pos: u32::from(self.fields(db)),
                };
                let n = tree::next(store, &rec);
                if n == 0 {
                    return DbRef {
                        store_nr: data.store_nr,
                        rec: 0,
                        pos: 0,
                    };
                }
                *pos = n as i32;
                rec.rec = n;
                rec.pos = 8;
                rec
            }
            Parts::Base
            | Parts::Struct(_)
            | Parts::Enum(_)
            | Parts::EnumValue(_, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Int(_, _)
            | Parts::DbRef
            | Parts::ChildRec(_)
            | Parts::Hash(_, _)
            | Parts::Spacial(_, _) => panic!(
                "Undefined iterate on non-collection type: {} (db={})",
                self.types[db as usize].name, db
            ),
        }
    }

    pub(super) fn db_ref(&self, data: &DbRef, pos: i32, r: u32) -> DbRef {
        DbRef {
            store_nr: data.store_nr,
            rec: if pos == i32::MAX {
                0
            } else {
                self.store(data).get_u32_raw(r, pos as u32)
            },
            pos: 8,
        }
    }

    #[must_use]
    pub fn element_reference(&self, data: &DbRef, pos: i32) -> DbRef {
        DbRef {
            store_nr: data.store_nr,
            rec: if pos == i32::MAX {
                0
            } else {
                self.store(data).get_u32_raw(data.rec, data.pos)
            },
            pos: pos as u32,
        }
    }

    /// @P305 — insert-or-replace a value into a keyed collection
    /// (`hash`/`sorted`/`index`), keyed by `value`'s own key field(s).  This
    /// is the runtime of `coll[k] = value`: if a record with `value`'s key
    /// already exists it is removed first (so the key stays unique — dedup),
    /// then `value` is deep-copied into a fresh record and linked in.  Works
    /// uniformly for a LOCAL, struct FIELD, or `&`-param collection because
    /// `coll` is just the resolved collection ref at runtime.  Mirrors the
    /// proven `OpNewRecord` + `OpCopyRecord` + `OpFinishRecord` append
    /// sequence (so it shares its deep-copy + linking machinery) plus a
    /// preceding remove.  `free_source` frees `value`'s store after the copy
    /// when it is a caller temp (the `0x8000` bit, as in `copy_record`).
    pub fn set_keyed(&mut self, coll: &DbRef, value: &DbRef, db: u16, free_source: bool) {
        let content_tp = match self.types[db as usize].parts {
            Parts::Hash(c, _)
            | Parts::Sorted(c, _)
            | Parts::Index(c, _, _)
            | Parts::Ordered(c, _) => c,
            _ => return,
        };
        let keys = self.types[db as usize].keys.clone();
        let key = keys::get_key(value, &self.allocations, &keys);
        let existing = self.find(coll, db, &key);
        if existing.rec != 0 {
            // dedup: free the old record's nested heap, then unlink it.
            self.remove_claims(&existing, content_tp);
            self.remove(coll, &existing, db);
            // A hash record is a SEPARATE store claim, so the unlink above
            // leaves it orphaned — reclaim it (otherwise repeated
            // `coll[k] = v` replaces grow the store unboundedly).  Sorted /
            // index records are inline in the vector / freed by their own
            // `remove`, so no extra delete there.
            if matches!(self.types[db as usize].parts, Parts::Hash(_, _)) {
                self.store_mut(coll).delete(existing.rec);
            }
        }
        // insert: claim a fresh record, deep-copy `value` into it, link by key.
        let new = self.record_new(coll, db, u16::MAX);
        let size = u32::from(self.size(content_tp));
        self.copy_block(value, &new, size);
        self.copy_claims(value, &new, content_tp);
        self.record_finish(coll, &new, db, u16::MAX);
        // @P317 — LOFT_LOG=copy_check: warn if the keyed deep copy changed any
        // nested collection length (before the source-free below).
        if self.copy_check_enabled() {
            self.report_copy_mismatches(value, &new, content_tp, "set_keyed");
        }
        if free_source
            && value.store_nr != coll.store_nr
            && value.store_nr != 0
            && !self.allocations[value.store_nr as usize].free
            && !self.allocations[value.store_nr as usize].read_only
            && !self.allocations[value.store_nr as usize].free_protected
        {
            self.free(value);
        }
    }

    /// @P306 — before linking a new keyed record, drop any EXISTING record
    /// that shares its key so the key stays unique (latest insert wins —
    /// `coll += [entry]` on a keyed collection dedups instead of stacking a
    /// shadowed duplicate).  For hash / index the records are SEPARATE store
    /// claims, so free the old one's nested heap, unlink it, and reclaim its
    /// slot.  (Sorted / ordered records are inline in the vector and the new
    /// one is already appended at the end when their `*_finish` runs, so they
    /// dedup by overwriting the found slot in place there, not here.)
    /// Replace any record already stored under `rec`'s key.
    ///
    /// `secondary` distinguishes a PRIMARY keyed collection (this index OWNS its
    /// records) from a SECONDARY index auto-maintained for a sibling field via
    /// `Field.other_indexes` (the "two views share records" pattern — a
    /// `vector<T>` + `hash<T[k]>` in one struct).  A secondary index must only
    /// UNLINK the stale key→record mapping; the displaced record is still held
    /// by the primary collection (the vector), so freeing it here corrupts that
    /// collection (read-back nulls / `Unknown record`).  This is the
    /// `other_indexes` analogue of the @P305 `keyed_field_is_linked` update-only
    /// path already used by `OpSetKeyed`.
    pub(crate) fn dedup_keyed(
        &mut self,
        data: &DbRef,
        rec: &DbRef,
        db: u16,
        content_tp: u16,
        secondary: bool,
    ) {
        let keys = self.types[db as usize].keys.clone();
        let key = keys::get_key(rec, &self.allocations, &keys);
        let existing = self.find(data, db, &key);
        if existing.rec != 0 && existing.rec != rec.rec {
            self.remove(data, &existing, db);
            if !secondary {
                self.remove_claims(&existing, content_tp);
                self.store_mut(data).delete(existing.rec);
            }
        }
    }

    /**
    Remove a specific record from a structure.
    # Panics
    When not in a structure.
    */
    pub fn remove(&mut self, data: &DbRef, rec: &DbRef, db: u16) {
        match self.types[db as usize].parts.clone() {
            Parts::Sorted(c, _) | Parts::Vector(c) | Parts::Array(c) | Parts::Ordered(c, _) => {
                let size = u32::from(self.types[c as usize].size);
                vector::remove_vector(
                    data,
                    size,
                    i64::from((rec.pos - 8) / size),
                    &mut self.allocations,
                );
            }
            Parts::Hash(_, _) => {
                let keys = self.keys(db).to_vec();
                hash::remove(data, rec, &mut self.allocations, &keys);
            }
            Parts::Index(_, _, _) => {
                let left = self.fields(db);
                let keys = self.keys(db).to_vec();
                tree::remove(data, rec, left, &mut self.allocations, &keys);
            }
            Parts::Base
            | Parts::Struct(_)
            | Parts::Enum(_)
            | Parts::EnumValue(_, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Int(_, _)
            | Parts::DbRef
            | Parts::ChildRec(_)
            | Parts::Spacial(_, _) => panic!(
                "remove called on non-collection type: {} (db={})",
                self.types[db as usize].name, db
            ),
        }
    }

    // Output the hash content and validate its content.
    #[allow(dead_code)]
    pub(super) fn hash_dump(&mut self, hash_ref: &DbRef, db: u16, keys: &[u16]) {
        let claim = self.store(hash_ref).get_u32_raw(hash_ref.rec, hash_ref.pos);
        let length = self.store(hash_ref).get_u32_raw(claim, 4);
        let room = self.store(hash_ref).get_i32_raw(claim, 0) as u32;
        let elms = (room - 1) * 2;
        println!(
            "dump hash length:{length} elms:{elms} {:.2}%",
            100.0 * f64::from(length) / f64::from(elms)
        );
        let mut record = DbRef {
            store_nr: hash_ref.store_nr,
            rec: 0,
            pos: 0,
        };
        let mut l = 0;
        for i in 0..elms {
            let rec = self.store(hash_ref).get_u32_raw(claim, 8 + i * 4);
            if rec != 0 {
                let mut s = String::new();
                record.rec = rec;
                self.show(&mut s, &record, db, false);
                l += 1;
                println!("{i:4}:[{rec}]{s}");
                let mut k = Vec::new();
                for f in keys {
                    k.push(self.field_content(&record, db, *f));
                }
            }
        }
        assert_eq!(length, l, "Incorrect hash length");
    }

    #[allow(dead_code)]
    pub(super) fn compare_key(
        &self,
        rec: &DbRef,
        db: u16,
        keys: &[(u16, bool)],
        key: &[Content],
    ) -> Ordering {
        for (k_nr, k) in key.iter().enumerate() {
            let mut cmp = compare(k, &self.field_content(rec, db, keys[k_nr].0));
            if !keys[k_nr].1 {
                if cmp == Ordering::Less {
                    cmp = Ordering::Greater;
                } else if cmp == Ordering::Greater {
                    cmp = Ordering::Less;
                }
            }
            if cmp != Ordering::Equal {
                return cmp;
            }
        }
        Ordering::Equal
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Stores;
    use crate::keys::{Content, DbRef};

    /// `find()` with a non-collection type must panic with a diagnostic message.
    #[test]
    #[should_panic(expected = "find called on non-collection type")]
    fn find_non_collection_panics() {
        let stores = Stores::new();
        let data = DbRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        };
        let _ = stores.find(&data, 0, &[Content::Long(0)]);
    }

    /// `remove()` with a non-collection type must panic with a diagnostic message.
    #[test]
    #[should_panic(expected = "remove called on non-collection type")]
    fn remove_non_collection_panics() {
        let mut stores = Stores::new();
        let data = DbRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        };
        let rec = DbRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        };
        stores.remove(&data, &rec, 0);
    }
}
