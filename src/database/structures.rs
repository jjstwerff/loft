// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Structure allocation, initialization, field get/set, parsing operations.

use crate::database::{Field, Parts, Stores};
use crate::keys::DbRef;
use crate::store::Store;
use crate::vector;
use crate::{hash, keys, tree};
use std::collections::HashSet;

/// Walker-native diagnostic for `walk_parsed_into` failures.
///
/// `at` is a byte offset into the original input; `path` is the
/// dotted-key / `[index]` path to the failing node.  `format.rs`
/// converts these into the user-visible `"line N:M path:X"` shape
/// using `crate::json::line_col_of`.
pub(super) struct WalkErr {
    pub at: usize,
    pub path: Vec<String>,
}

impl Stores {
    /**
    # Panics
    When requesting a record on a non-structure
    */
    pub fn record_new(&mut self, data: &DbRef, parent_tp: u16, field: u16) -> DbRef {
        let tp = if field == u16::MAX {
            // This case is when the top level is a data-structure
            parent_tp
        } else {
            self.field_type(parent_tp, field)
        };
        let d = self.field_ref(data, parent_tp, field);
        match self.types[tp as usize].parts {
            Parts::Sorted(c, _) => {
                vector::sorted_new(&d, u32::from(self.size(c)), &mut self.allocations)
            }
            Parts::Vector(c) => {
                vector::vector_append(&d, u32::from(self.size(c)), &mut self.allocations)
            }
            Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Index(c, _, _)
            | Parts::Spacial(c, _) => {
                let rec = self.claim(&d, 1 + ((u32::from(self.size(c)) + 7) >> 3));
                self.store_mut(&rec).set_int(rec.rec, 4, data.rec as i32);
                rec
            }
            _ => panic!(
                "Cannot add to none-structure '{}'",
                self.types[tp as usize].name
            ),
        }
    }

    /**
    # Panics
    When the implementation is not yet written
    */
    pub fn record_finish(&mut self, data: &DbRef, rec: &DbRef, parent_tp: u16, field: u16) {
        let tp = if field == u16::MAX {
            // This case is when the top level is a data-structure
            parent_tp
        } else {
            self.field_type(parent_tp, field)
        };
        let d = self.field_ref(data, parent_tp, field);
        self.insert_record(&d, rec, tp);
        if field != u16::MAX
            && let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                self.types[parent_tp as usize].parts.clone()
        {
            let f = &fields[field as usize];
            let o = &f.other_indexes;
            if !o.is_empty() && o[0] != u16::MAX {
                for fld_nr in o {
                    let o = self.field_ref(data, parent_tp, *fld_nr);
                    self.insert_record(&o, rec, fields[*fld_nr as usize].content);
                }
            }
        }
    }

    pub(super) fn field_ref(&self, data: &DbRef, parent_tp: u16, field: u16) -> DbRef {
        if field == u16::MAX {
            *data
        } else if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[parent_tp as usize].parts
        {
            DbRef {
                store_nr: data.store_nr,
                rec: data.rec,
                pos: data.pos + u32::from(fields[field as usize].position),
            }
        } else {
            *data
        }
    }

    pub(super) fn insert_record(&mut self, data: &DbRef, rec: &DbRef, tp: u16) {
        match self.types[tp as usize].parts.clone() {
            Parts::Vector(_) => {
                vector::vector_finish(data, &mut self.allocations);
            }
            Parts::Sorted(c, _) => {
                let size = u32::from(self.size(c));
                vector::sorted_finish(
                    data,
                    size,
                    &self.types[tp as usize].keys,
                    &mut self.allocations,
                );
            }
            Parts::Array(_) => {
                let reference = vector::vector_append(data, 4, &mut self.allocations);
                self.store_mut(data)
                    .set_int(reference.rec, reference.pos, rec.rec as i32);
                vector::vector_finish(data, &mut self.allocations);
            }
            Parts::Hash(_, _) => hash::add(
                data,
                rec,
                &mut self.allocations,
                &self.types[tp as usize].keys,
            ),
            Parts::Index(_, _, _) => tree::add(
                data,
                rec,
                self.fields(tp),
                &mut self.allocations,
                &self.types[tp as usize].keys,
            ),
            Parts::Ordered(_, _) => {
                vector::ordered_finish(
                    data,
                    rec,
                    &self.types[tp as usize].keys,
                    &mut self.allocations,
                );
            }
            Parts::Spacial(_, _) => panic!("Not implemented"),
            _ => (),
        }
    }

    pub fn vector_add(&mut self, db: &DbRef, o_db: &DbRef, known: u16) {
        let o_length = vector::length_vector(o_db, &self.allocations);
        if o_length == 0 {
            // The other vector has no data
            return;
        }
        // Snapshot the source record number BEFORE any resize: if `db` and `o_db` share the
        // same backing store the resize inside `vector_append` / `vector_set_size` may
        // reallocate the vector and invalidate `o_rec`.  Reading it after the resize would
        // reference freed memory, silently producing corrupt data.
        let o_rec = keys::store(o_db, &self.allocations).get_int(o_db.rec, o_db.pos) as u32;
        let size = u32::from(self.size(known));
        // If source and destination share the same backing vector record, copy source elements
        // to a local buffer first so the resize cannot invalidate the source pointer.
        let same_vec = db.store_nr == o_db.store_nr && o_rec != 0 && {
            let dest_rec = keys::store(db, &self.allocations).get_int(db.rec, db.pos) as u32;
            dest_rec == o_rec
        };
        let snapshot: Vec<u8> = if same_vec {
            let store = keys::store(o_db, &self.allocations);
            let byte_len = o_length as usize * size as usize;
            (0..byte_len)
                .map(|i| *store.addr::<u8>(o_rec, 8 + i as u32))
                .collect()
        } else {
            Vec::new()
        };
        let new_db = vector::vector_append(db, size, &mut self.allocations);
        let append_pos = new_db.pos;
        // Claim more than 1 record if needed for the actual copy.
        self.vector_set_size(db, o_length, size);
        // P153: `vector_set_size` may have relocated the destination record.
        // `new_db.rec` captured from `vector_append` is stale after relocation;
        // re-read the current rec from the field slot (which `vector_set_size`
        // keeps up to date) before we use it for the byte copy.  Element
        // offset (`append_pos`) is layout-stable across relocation.
        let dest_rec = keys::store(db, &self.allocations).get_int(db.rec, db.pos) as u32;
        let new_db = DbRef {
            store_nr: db.store_nr,
            rec: dest_rec,
            pos: append_pos,
        };
        if same_vec {
            // Write from the pre-resize snapshot; `new_db.rec` is already the correct
            // (possibly reallocated) destination record after `vector_set_size`.
            let store = keys::mut_store(db, &mut self.allocations);
            for (i, &byte) in snapshot.iter().enumerate() {
                *store.addr_mut::<u8>(new_db.rec, new_db.pos + i as u32) = byte;
            }
        } else if db.store_nr == o_db.store_nr {
            // Re-read o_rec after resize in case it moved (non-self-append same-store case).
            let o_rec = keys::store(o_db, &self.allocations).get_int(o_db.rec, o_db.pos) as u32;
            keys::mut_store(db, &mut self.allocations).copy_block(
                o_rec,
                8,
                new_db.rec,
                new_db.pos as isize,
                o_length as isize * size as isize,
            );
        } else {
            let o_store: &Store;
            let db_store: &mut Store;
            // These stores are actually two different data structures. However, there is no easier
            // way to tell the rust type system this.
            unsafe {
                o_store = keys::store(o_db, &*std::ptr::from_ref::<[Store]>(&self.allocations));
                db_store = keys::mut_store(
                    db,
                    &mut *std::ptr::from_mut::<[Store]>(&mut self.allocations),
                );
            }
            o_store.copy_block_between(
                o_rec,
                8,
                db_store,
                new_db.rec,
                new_db.pos as isize,
                o_length as isize * size as isize,
            );
        }
        // After the raw byte copy, slot indices for text and sub-structure fields in each
        // appended element still point into the source store.  Deep-copy those claims so
        // that the destination owns independent copies and is not affected when the source
        // vector is freed.
        for i in 0..o_length {
            self.copy_claims(
                &DbRef {
                    store_nr: o_db.store_nr,
                    rec: o_rec,
                    pos: 8 + size * i,
                },
                &DbRef {
                    store_nr: db.store_nr,
                    rec: new_db.rec,
                    pos: new_db.pos + size * i,
                },
                known,
            );
        }
    }

    pub fn vector_set_size(&mut self, db: &DbRef, adding: u32, size: u32) {
        let store = keys::mut_store(db, &mut self.allocations);
        let mut vec_rec = store.get_int(db.rec, db.pos) as u32;
        let length = store.get_int(vec_rec, 4) as u32;
        if adding > 1 {
            let new_vec = store.resize(vec_rec, ((length + adding) * size + 15) / 8);
            if new_vec != vec_rec {
                store.set_int(db.rec, db.pos, new_vec as i32);
                // P153: track the relocation so the length write below lands
                // in the current record instead of the freed one.
                vec_rec = new_vec;
            }
        }
        store.set_int(vec_rec, 4, length as i32 + adding as i32);
    }

    /// P54-U phase 2: walk a [`crate::json::Parsed`] tree into
    /// the record at `to`, dispatching on the target type's
    /// [`Parts`] variant.  Returns `true` on success, `false`
    /// on a shape/type mismatch that can't be recovered.
    ///
    /// This is the schema-driven counterpart to the parser-side
    /// `crate::json::parse_with(text, Dialect::Lenient)` — the
    /// parser stays schema-free; all type dispatch lives here.
    /// Replaces the hand-rolled `parsing` scanner in the
    /// legacy `text → struct` path (still kept for the
    /// transition; see § P54-U in doc/claude/QUALITY.md).
    #[allow(clippy::ptr_arg)] // path needs push/pop, slice not enough
    pub(super) fn walk_parsed_into(
        &mut self,
        parsed: &crate::json::Parsed,
        tp: u16,
        rec_tp: u16,
        field: u16,
        to: &DbRef,
        path: &mut Vec<String>,
    ) -> Result<(), WalkErr> {
        // `null` at any target position resets to the type's
        // default sentinel — mirrors the legacy scanner's
        // first-line behaviour and keeps round-tripping correct.
        if matches!(parsed, crate::json::Parsed::Null) {
            self.set_default_value(tp, to);
            return Ok(());
        }
        match self.types[tp as usize].parts.clone() {
            Parts::Base => self.walk_primitive_into(parsed, tp, to, path),
            Parts::Sorted(c, _)
            | Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Spacial(c, _)
            | Parts::Index(c, _, _) => {
                let crate::json::Parsed::Array(items) = parsed else {
                    return Err(WalkErr {
                        at: 0,
                        path: path.clone(),
                    });
                };
                for (idx, item) in items.iter().enumerate() {
                    path.push(format!("[{idx}]"));
                    let res = self.record_new(to, rec_tp, field);
                    self.walk_parsed_into(item, c, c, u16::MAX, &res, path)?;
                    self.record_finish(to, &res, rec_tp, field);
                    path.pop();
                }
                Ok(())
            }
            Parts::Struct(object) | Parts::EnumValue(_, object) => {
                self.walk_parsed_struct(parsed, tp, to, &object, path)
            }
            Parts::Enum(fields) => {
                // Three accepted shapes:
                //   - `Parsed::Str("Tag")` / `Parsed::Ident(Tag)` — unit variant
                //   - `Parsed::Object([("Tag", _, Object(payload))])` — variant
                //     with payload, emitted by the Lenient parser for the
                //     `Tag { field: value, ... }` input shape (matches the
                //     legacy scanner's struct-enum-variant behaviour).
                let (name, payload) = match parsed {
                    crate::json::Parsed::Str(s) | crate::json::Parsed::Ident(s) => {
                        (s.as_str(), None)
                    }
                    crate::json::Parsed::Object(entries) if entries.len() == 1 => {
                        (entries[0].0.as_str(), Some(&entries[0].2))
                    }
                    _ => {
                        return Err(WalkErr {
                            at: 0,
                            path: path.clone(),
                        });
                    }
                };
                let mut enum_tp = u16::MAX;
                let val = if name == "null" {
                    0
                } else {
                    let mut v = 1;
                    for (f_nr, f) in fields.iter().enumerate() {
                        if f.1 == name {
                            v = f_nr as i32 + 1;
                            enum_tp = f.0;
                            break;
                        }
                    }
                    v
                };
                self.store_mut(to).set_byte(to.rec, to.pos, 0, val);
                // Variant-with-payload: if the parser gave us an Object
                // and the variant's EnumValue sub-type exists (size > 1),
                // recurse into the walker so the payload fields land in
                // the same slot as the discriminant byte.
                if let Some(body) = payload
                    && enum_tp != u16::MAX
                    && self.types[enum_tp as usize].size > 1
                {
                    return self.walk_parsed_into(body, enum_tp, rec_tp, field, to, path);
                }
                Ok(())
            }
            Parts::Byte(from, _null) => {
                let crate::json::Parsed::Number(n) = parsed else {
                    return Err(WalkErr {
                        at: 0,
                        path: path.clone(),
                    });
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to).set_byte(to.rec, to.pos, from, *n as i32);
                Ok(())
            }
            Parts::Short(from, _null) => {
                let crate::json::Parsed::Number(n) = parsed else {
                    return Err(WalkErr {
                        at: 0,
                        path: path.clone(),
                    });
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to)
                    .set_short(to.rec, to.pos, from, *n as i32);
                Ok(())
            }
        }
    }

    /// Schema-driven struct fill from a [`crate::json::Parsed::Object`].
    /// Matches fields by name, recurses into the walker for each
    /// value, default-fills any unmentioned field (mirroring the
    /// legacy scanner's "missing field → default" behaviour).
    #[allow(clippy::ptr_arg)] // path needs push/pop, slice not enough
    fn walk_parsed_struct(
        &mut self,
        parsed: &crate::json::Parsed,
        tp: u16,
        to: &DbRef,
        object: &[Field],
        path: &mut Vec<String>,
    ) -> Result<(), WalkErr> {
        let crate::json::Parsed::Object(entries) = parsed else {
            return Err(WalkErr {
                at: 0,
                path: path.clone(),
            });
        };
        let fld = if to.rec == 0 { 0 } else { to.pos };
        let rec = if to.rec == 0 {
            let size = self.types[tp as usize].size;
            self.store_mut(to).claim(u32::from(size).div_ceil(8))
        } else {
            to.rec
        };
        let mut found_fields: HashSet<&str> = HashSet::new();
        for (name, key_at, value) in entries {
            let mut matched = false;
            for (f_nr, f) in object.iter().enumerate() {
                if f.name == *name {
                    matched = true;
                    path.push(name.clone());
                    let res = if self.content(f.content) == u16::MAX {
                        let slot = DbRef {
                            store_nr: to.store_nr,
                            rec,
                            pos: fld + u32::from(f.position),
                        };
                        self.walk_parsed_into(value, f.content, tp, f_nr as u16, &slot, path)
                    } else {
                        self.walk_parsed_into(value, f.content, tp, f_nr as u16, to, path)
                    };
                    res?;
                    path.pop();
                    break;
                }
            }
            if !matched {
                // An unknown field name in the source is a parse
                // error.  Position the caret at the byte just past
                // the key (the `:` after the name) — matches the
                // legacy `parse_key`/`show_key` shape that
                // `tests/data_structures.rs::record` asserts as
                // `"line 1:7 path:blame"` for input
                // `{blame:"nothing"}`.
                let mut err_path = path.clone();
                err_path.push(name.clone());
                return Err(WalkErr {
                    at: key_at + name.len(),
                    path: err_path,
                });
            }
            found_fields.insert(name.as_str());
        }
        for f in object {
            if (f.other_indexes.is_empty() || f.other_indexes[0] != u16::MAX)
                && !found_fields.contains(f.name.as_str())
                && f.name != "enum"
            {
                let slot = DbRef {
                    store_nr: to.store_nr,
                    rec,
                    pos: fld + u32::from(f.position),
                };
                self.set_default_value(f.content, &slot);
            }
        }
        Ok(())
    }

    /// Schema-driven primitive write.  `tp` is one of the
    /// low-numbered base-type IDs (0 = int32/Reference, 1 = long,
    /// 2 = single, 3 = float, 4 = bool, 5 = text, 6 = Reference).
    #[allow(clippy::ptr_arg)] // path needs push/pop, slice not enough
    fn walk_primitive_into(
        &mut self,
        parsed: &crate::json::Parsed,
        tp: u16,
        to: &DbRef,
        path: &mut Vec<String>,
    ) -> Result<(), WalkErr> {
        let mismatch = || WalkErr {
            at: 0,
            path: path.clone(),
        };
        match tp {
            0 | 6 => {
                let crate::json::Parsed::Number(n) = parsed else {
                    return Err(mismatch());
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to).set_int(to.rec, to.pos, *n as i32);
                Ok(())
            }
            1 => {
                let crate::json::Parsed::Number(n) = parsed else {
                    return Err(mismatch());
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to).set_long(to.rec, to.pos, *n as i64);
                Ok(())
            }
            2 => {
                let crate::json::Parsed::Number(n) = parsed else {
                    return Err(mismatch());
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to).set_single(to.rec, to.pos, *n as f32);
                Ok(())
            }
            3 => {
                let crate::json::Parsed::Number(n) = parsed else {
                    return Err(mismatch());
                };
                self.store_mut(to).set_float(to.rec, to.pos, *n);
                Ok(())
            }
            4 => {
                let crate::json::Parsed::Bool(b) = parsed else {
                    return Err(mismatch());
                };
                self.store_mut(to)
                    .set_byte(to.rec, to.pos, 0, i32::from(*b));
                Ok(())
            }
            5 => {
                // Text accepts only a quoted string — bare
                // identifiers (`Parsed::Ident`) are NOT promoted to
                // text, matching the legacy `match_text` behaviour.
                let crate::json::Parsed::Str(s) = parsed else {
                    return Err(mismatch());
                };
                let text_pos = self.store_mut(to).set_str(s);
                self.store_mut(to).set_int(to.rec, to.pos, text_pos as i32);
                Ok(())
            }
            _ => Err(mismatch()),
        }
    }

    /**
        Write default(null) values on all fields. This should normally only be done while debugging
        as all fields should be set anyway under correctly generated code.
        # Panics
        On inconsistent database definitions.
    */
    pub fn set_default_value(&mut self, tp: u16, rec: &DbRef) {
        if tp <= 6 {
            match tp {
                0 | 6 => {
                    self.store_mut(rec).set_int(rec.rec, rec.pos, i32::MIN);
                }
                1 => {
                    self.store_mut(rec).set_long(rec.rec, rec.pos, i64::MIN);
                }
                2 => {
                    self.store_mut(rec).set_single(rec.rec, rec.pos, f32::NAN);
                }
                3 => {
                    self.store_mut(rec).set_float(rec.rec, rec.pos, f64::NAN);
                }
                4 => {
                    self.store_mut(rec).set_byte(rec.rec, rec.pos, 0, 0);
                }
                5 => {
                    self.store_mut(rec).set_int(rec.rec, rec.pos, 0);
                }
                _ => (),
            }
            return;
        }
        match self.types[tp as usize].parts.clone() {
            Parts::Enum(_) => {
                self.store_mut(rec).set_byte(rec.rec, rec.pos, 0, 0);
            }
            Parts::Byte(_, null) => {
                self.store_mut(rec)
                    .set_byte(rec.rec, rec.pos, 0, if null { 255 } else { 0 });
            }
            Parts::Short(_, null) => {
                self.store_mut(rec)
                    .set_short(rec.rec, rec.pos, 0, if null { 65535 } else { 0 });
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in &fields {
                    if f.name == "type" && f.position == 0 {
                        self.store_mut(rec)
                            .set_short(rec.rec, rec.pos, 0, i32::from(tp));
                        continue;
                    }
                    self.set_default_value(
                        f.content,
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        },
                    );
                }
            }
            Parts::Sorted(_, _)
            | Parts::Ordered(_, _)
            | Parts::Spacial(_, _)
            | Parts::Hash(_, _)
            | Parts::Index(_, _, _)
            | Parts::Array(_)
            | Parts::Vector(_) => {
                self.store_mut(rec).set_int(rec.rec, rec.pos, 0);
            }
            Parts::Base => {
                panic!(
                    "not implemented default {:?}",
                    self.types[tp as usize].parts
                );
            }
        }
    }

    #[must_use]
    pub fn get_ref(&self, db: &DbRef, fld: u32) -> DbRef {
        if db.rec == 0 {
            return DbRef {
                store_nr: db.store_nr,
                rec: 0,
                pos: 0,
            };
        }
        let store = self.store(db);
        let res = store.get_int(db.rec, db.pos + fld) as u32;
        DbRef {
            store_nr: db.store_nr,
            rec: res,
            pos: 8,
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn get_field(db: &DbRef, fld: u32) -> DbRef {
        DbRef {
            store_nr: db.store_nr,
            rec: db.rec,
            pos: db.pos + fld,
        }
    }

    pub fn copy_block(&mut self, from: &DbRef, to: &DbRef, len: u32) {
        unsafe {
            std::ptr::copy(
                self.store(from)
                    .ptr
                    .offset(from.rec as isize * 8 + from.pos as isize),
                self.store_mut(to)
                    .ptr
                    .offset(to.rec as isize * 8 + to.pos as isize),
                len as usize,
            );
        }
    }
}
