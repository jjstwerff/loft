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
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &self.types[db as usize].parts
        {
            let f = &fields[key as usize];
            return match f.content {
                0 | 6 => Content::Long(i64::from(
                    store.get_int(rec.rec, rec.pos + u32::from(f.position)),
                )),
                1 => Content::Long(store.get_long(rec.rec, rec.pos + u32::from(f.position))),
                2 => Content::Single(store.get_single(rec.rec, rec.pos + u32::from(f.position))),
                3 => Content::Float(store.get_float(rec.rec, rec.pos + u32::from(f.position))),
                4 => Content::Long(i64::from(store.get_byte(
                    rec.rec,
                    rec.pos + u32::from(f.position),
                    0,
                ))),
                5 => Content::Str(crate::keys::Str::new(
                    store.get_str(store.get_int(rec.rec, rec.pos + u32::from(f.position)) as u32),
                )),
                _ => {
                    if let Parts::Enum(_) = self.types[f.content as usize].parts {
                        Content::Long(i64::from(store.get_byte(
                            rec.rec,
                            rec.pos + u32::from(f.position),
                            0,
                        )))
                    } else {
                        panic!(
                            "Unknown key type {} of {}.{}",
                            self.types[f.content as usize].name,
                            self.types[db as usize].name,
                            f.name
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
                v as i32,
                &self.allocations,
            )
        } else {
            DbRef {
                store_nr: data.store_nr,
                rec: if data.rec == 0 || self.store(data).get_int(data.rec, 4) == 0 {
                    0
                } else {
                    self.store(data).get_int(data.rec, 0) as u32
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
                v as i32,
                &self.allocations,
            );
            DbRef {
                store_nr: res.store_nr,
                rec: if res.rec == 0 {
                    0
                } else {
                    self.store(&res).get_int(res.rec, res.pos) as u32
                },
                pos: 8,
            }
        } else {
            DbRef {
                store_nr: data.store_nr,
                rec: if data.rec == 0 || self.store(data).get_int(data.rec, 4) == 0 {
                    0
                } else {
                    let rec = self.store(data).get_int(data.rec, 0) as u32;
                    self.store(data).get_int(rec, 8) as u32
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
                        rec: self.store(data).get_int(data.rec, data.pos) as u32,
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
                let sorted_rec = self.store(data).get_int(data.rec, data.pos) as u32;
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
                        rec: self.store(data).get_int(sorted_rec, 8 + pos * 4) as u32,
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
            _ => panic!("Incorrect search"),
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
                let mut res = Vec::new();
                if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                    &self.types[*c as usize].parts
                {
                    for (k, _) in key {
                        res.push(fields[*k as usize].content);
                    }
                }
                res
            }
            Parts::Hash(c, key) => {
                let mut res = Vec::new();
                if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                    &self.types[*c as usize].parts
                {
                    for k in key {
                        res.push(fields[*k as usize].content);
                    }
                }
                res
            }
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
                let r = self.store(data).get_int(data.rec, data.pos) as u32;
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
                let r = self.store(data).get_int(data.rec, data.pos) as u32;
                DbRef {
                    store_nr: data.store_nr,
                    rec: self.store(data).get_int(r, *pos as u32) as u32,
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
            _ => panic!("Undefined iterate on '{}'", self.types[db as usize].name),
        }
    }

    pub(super) fn db_ref(&self, data: &DbRef, pos: i32, r: u32) -> DbRef {
        DbRef {
            store_nr: data.store_nr,
            rec: if pos == i32::MAX {
                0
            } else {
                self.store(data).get_int(r, pos as u32) as u32
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
                self.store(data).get_int(data.rec, data.pos) as u32
            },
            pos: pos as u32,
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
                    ((rec.pos - 8) / size) as i32,
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
            _ => panic!("Incorrect search"),
        }
    }

    // Output the hash content and validate its content.
    #[allow(dead_code)]
    pub(super) fn hash_dump(&mut self, hash_ref: &DbRef, db: u16, keys: &[u16]) {
        let claim = self.store(hash_ref).get_int(hash_ref.rec, hash_ref.pos) as u32;
        let length = self.store(hash_ref).get_int(claim, 4) as u32;
        let room = self.store(hash_ref).get_int(claim, 0) as u32;
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
            let rec = self.store(hash_ref).get_int(claim, 8 + i * 4) as u32;
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
    use super::*;
    use crate::database::Stores;
    use crate::keys::{Content, DbRef};

    /// S3: find() with a non-collection type must panic with a diagnostic message.
    #[test]
    #[ignore]
    #[should_panic(expected = "find called on non-collection type")]
    fn find_non_collection_panics() {
        let stores = Stores::new();
        let data = DbRef { store_nr: 0, rec: 0, pos: 0 };
        stores.find(&data, 0, &[Content::Long(0)]);
    }

    /// S3: remove() with a non-collection type must panic with a diagnostic message.
    #[test]
    #[ignore]
    #[should_panic(expected = "remove called on non-collection type")]
    fn remove_non_collection_panics() {
        let mut stores = Stores::new();
        let data = DbRef { store_nr: 0, rec: 0, pos: 0 };
        let rec = DbRef { store_nr: 0, rec: 0, pos: 0 };
        stores.remove(&data, &rec, 0);
    }
}
