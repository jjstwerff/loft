// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Structure allocation, initialization, field get/set, parsing operations.

use crate::database::{Field, Parts, Stores};
use crate::keys::DbRef;
use crate::store::Store;
use crate::vector;
use crate::{hash, keys, tree};
use std::collections::HashSet;

fn match_token(text: &str, pos: &mut usize, token: u8) -> bool {
    if *pos < text.len() && text.as_bytes()[*pos] == token {
        *pos += 1;
        true
    } else {
        false
    }
}

fn match_empty(text: &str, pos: &mut usize) {
    let mut c = *pos;
    let bytes = text.as_bytes();
    while c < bytes.len() && (bytes[c] == b' ' || bytes[c] == b'\t' || bytes[c] == b'\n') {
        c += 1;
        *pos = c;
    }
}

fn match_null(text: &str, pos: &mut usize) -> bool {
    if text.len() >= *pos + 4 && &text[*pos..*pos + 4] == "null" {
        *pos += 4;
        true
    } else {
        false
    }
}

fn match_boolean(text: &str, pos: &mut usize, value: &mut bool) -> bool {
    if text.len() >= *pos + 4 && &text[*pos..*pos + 4] == "true" {
        *pos += 4;
        *value = true;
        true
    } else if text.len() >= *pos + 5 && &text[*pos..*pos + 5] == "false" {
        *pos += 4;
        *value = false;
        true
    } else {
        false
    }
}

fn skip_integer(text: &str, pos: &mut usize) -> usize {
    let mut c = *pos;
    let bytes = text.as_bytes();
    if c < bytes.len() && bytes[c] == b'-' {
        c += 1;
    }
    while c < bytes.len() && bytes[c] >= b'0' && bytes[c] <= b'9' {
        c += 1;
    }
    c
}

fn match_integer(text: &str, pos: &mut usize, value: &mut i32) -> bool {
    let c = skip_integer(text, pos);
    if c == *pos {
        false
    } else {
        *value = text[*pos..c].parse().unwrap();
        *pos = c;
        true
    }
}

fn match_long(text: &str, pos: &mut usize, value: &mut i64) -> bool {
    let c = skip_integer(text, pos);
    if c == *pos {
        false
    } else {
        *value = text[*pos..c].parse().unwrap();
        *pos = c;
        true
    }
}

fn skip_float(text: &str, pos: &mut usize) -> usize {
    let mut c = *pos;
    let bytes = text.as_bytes();
    if c < bytes.len() && bytes[c] == b'-' {
        c += 1;
    }
    while c < bytes.len()
        && ((bytes[c] >= b'0' && bytes[c] <= b'9') || bytes[c] == b'e' || bytes[c] == b'.')
    {
        c += 1;
        if c < bytes.len() && bytes[c - 1] == b'e' && bytes[c] == b'-' {
            c += 1;
        }
    }
    c
}

fn match_single(text: &str, pos: &mut usize, value: &mut f32) -> bool {
    let c = skip_float(text, pos);
    if c == *pos {
        false
    } else {
        *value = text[*pos..c].parse().unwrap();
        *pos = c;
        true
    }
}

fn match_float(text: &str, pos: &mut usize, value: &mut f64) -> bool {
    let c = skip_float(text, pos);
    if c == *pos {
        false
    } else {
        *value = text[*pos..c].parse().unwrap();
        *pos = c;
        true
    }
}

fn match_identifier(text: &str, pos: &mut usize, value: &mut String) -> bool {
    let mut c = *pos;
    let bytes = text.as_bytes();
    if c < bytes.len()
        && ((bytes[c] >= b'a' && bytes[c] <= b'z')
            || bytes[c] >= b'A' && bytes[c] <= b'Z'
            || bytes[c] == b'_')
    {
        c += 1;
        while c < bytes.len()
            && ((bytes[c] >= b'0' && bytes[c] <= b'9')
                || (bytes[c] >= b'a' && bytes[c] <= b'z')
                || bytes[c] >= b'A' && bytes[c] <= b'Z'
                || bytes[c] == b'_')
        {
            c += 1;
        }
        *value = text[*pos..c].parse().unwrap();
        *pos = c;
        true
    } else {
        false
    }
}

pub(super) fn match_text(text: &str, pos: &mut usize, value: &mut String) -> bool {
    let mut c = *pos;
    let bytes = text.as_bytes();
    value.clear();
    if c < bytes.len() && (bytes[c] == b'"' || bytes[c] == b'\'') {
        let close = bytes[c];
        c += 1;
        while c < bytes.len() && bytes[c] != close {
            if bytes[c] == b'\\' {
                c += 1;
                if c == bytes.len() {
                    return false;
                }
                if bytes[c] == b'n' {
                    *value += "\n";
                } else if bytes[c] == b't' {
                    *value += "\t";
                } else if bytes[c] == b'\\' {
                    *value += "\\";
                } else if bytes[c] == b'"' {
                    *value += "\"";
                } else if bytes[c] == b'\'' {
                    *value += "\'";
                } else {
                    return false;
                }
            } else {
                let s = c;
                while c < bytes.len() && bytes[c] > 127 {
                    c += 1;
                }
                if c == bytes.len() || bytes[c] == close {
                    return false;
                }
                c += 1;
                *value += &text[s..c];
            }
        }
        if bytes[c] == close {
            *pos = c + 1;
            true
        } else {
            false
        }
    } else {
        false
    }
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
        // Claim more than 1 record if needed for the actual copy.
        self.vector_set_size(db, o_length, size);
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
        let vec_rec = store.get_int(db.rec, db.pos) as u32;
        let length = store.get_int(vec_rec, 4) as u32;
        if adding > 1 {
            let new_vec = store.resize(vec_rec, ((length + adding) * size + 15) / 8);
            if new_vec != vec_rec {
                store.set_int(db.rec, db.pos, new_vec as i32);
            }
        }
        store.set_int(vec_rec, 4, length as i32 + adding as i32);
    }

    pub fn parsing(
        &mut self,
        text: &str,
        pos: &mut usize,
        tp: u16,
        rec_tp: u16,
        field: u16,
        to: &DbRef,
    ) -> bool {
        if match_null(text, pos) {
            self.set_default_value(tp, to);
        }
        match self.types[tp as usize].parts.clone() {
            Parts::Base => {
                if self.parse_simple(text, pos, tp, to) {
                    return true;
                }
            }
            Parts::Sorted(c, _)
            | Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Spacial(c, _)
            | Parts::Index(c, _, _) => {
                match_empty(text, pos);
                if match_token(text, pos, b'[') {
                    match_empty(text, pos);
                    if match_token(text, pos, b']') {
                        return true;
                    }
                    loop {
                        let res = self.record_new(to, rec_tp, field);
                        if !self.parsing(text, pos, c, c, u16::MAX, &res) {
                            return false;
                        }
                        self.record_finish(to, &res, rec_tp, field);
                        match_empty(text, pos);
                        if !match_token(text, pos, b',') {
                            break;
                        }
                        match_empty(text, pos);
                    }
                    match_empty(text, pos);
                    if !match_token(text, pos, b']') {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            Parts::Struct(object) | Parts::EnumValue(_, object) => {
                return self.parse_struct(text, pos, tp, to, &object);
            }
            Parts::Enum(fields) => {
                let mut value = String::new();
                let mut result = match_text(text, pos, &mut value);
                if !result {
                    result = match_identifier(text, pos, &mut value);
                    if !result {
                        return result;
                    }
                }
                let mut enum_tp = u16::MAX;
                let val = if value == "null" {
                    0
                } else {
                    let mut v = 1;
                    for (f_nr, f) in fields.iter().enumerate() {
                        if f.1 == value {
                            v = f_nr as i32 + 1;
                            enum_tp = f.0;
                            break;
                        }
                    }
                    v
                };
                self.store_mut(to).set_byte(to.rec, to.pos, 0, val);
                if enum_tp < u16::MAX && self.types[enum_tp as usize].size > 1 {
                    match_empty(text, pos);
                    if !self.parsing(text, pos, enum_tp, enum_tp, u16::MAX, to) {
                        return false;
                    }
                }
            }
            Parts::Byte(from, _null) => {
                let mut value = 0;
                if !match_integer(text, pos, &mut value) {
                    return false;
                }
                self.store_mut(to).set_byte(to.rec, to.pos, from, value);
            }
            Parts::Short(from, _null) => {
                let mut value = 0;
                if !match_integer(text, pos, &mut value) {
                    return false;
                }
                self.store_mut(to).set_short(to.rec, to.pos, from, value);
            }
        }
        true
    }

    pub(super) fn parse_struct(
        &mut self,
        text: &str,
        pos: &mut usize,
        tp: u16,
        to: &DbRef,
        object: &[Field],
    ) -> bool {
        if match_token(text, pos, b'{') {
            match_empty(text, pos);
            if match_token(text, pos, b'}') {
                return true;
            }
            let fld = if to.rec == 0 { 0 } else { to.pos };
            let rec = if to.rec == 0 {
                let size = self.types[tp as usize].size;
                self.store_mut(to).claim(u32::from(size).div_ceil(8))
            } else {
                to.rec
            };
            let mut found_fields = HashSet::new();
            loop {
                let mut field_name = String::new();
                // Accept both JSON-style "field" and loft-style field names.
                if !match_text(text, pos, &mut field_name)
                    && !match_identifier(text, pos, &mut field_name)
                {
                    return false;
                }
                match_empty(text, pos);
                if !match_token(text, pos, b':') {
                    return false;
                }
                match_empty(text, pos);
                for (f_nr, f) in object.iter().enumerate() {
                    if f.name == field_name {
                        let result = if self.content(f.content) == u16::MAX {
                            let field = DbRef {
                                store_nr: to.store_nr,
                                rec,
                                pos: fld + u32::from(f.position),
                            };
                            self.parsing(text, pos, f.content, tp, f_nr as u16, &field)
                        } else {
                            self.parsing(text, pos, f.content, tp, f_nr as u16, to)
                        };
                        if !result {
                            return false;
                        }
                    }
                }
                found_fields.insert(field_name);
                match_empty(text, pos);
                if !match_token(text, pos, b',') {
                    break;
                }
                match_empty(text, pos);
            }
            match_empty(text, pos);
            if !match_token(text, pos, b'}') {
                return false;
            }
            for f in object {
                if (f.other_indexes.is_empty() || f.other_indexes[0] != u16::MAX)
                    && !found_fields.contains(&f.name)
                    && f.name != "enum"
                {
                    let field = DbRef {
                        store_nr: to.store_nr,
                        rec,
                        pos: to.pos + u32::from(f.position),
                    };
                    self.set_default_value(f.content, &field);
                }
            }
        } else {
            return false;
        }
        true
    }

    pub(super) fn parse_simple(
        &mut self,
        text: &str,
        pos: &mut usize,
        tp: u16,
        to: &DbRef,
    ) -> bool {
        match tp {
            0 | 6 => {
                let mut value = 0;
                if !match_integer(text, pos, &mut value) {
                    return false;
                }
                self.store_mut(to).set_int(to.rec, to.pos, value);
            }
            1 => {
                let mut value = 0;
                if !match_long(text, pos, &mut value) {
                    return false;
                }
                self.store_mut(to).set_long(to.rec, to.pos, value);
            }
            2 => {
                let mut value = 0.0;
                if !match_single(text, pos, &mut value) {
                    return false;
                }
                self.store_mut(to).set_single(to.rec, to.pos, value);
            }
            3 => {
                let mut value = 0.0;
                if !match_float(text, pos, &mut value) {
                    return false;
                }
                self.store_mut(to).set_float(to.rec, to.pos, value);
            }
            4 => {
                let mut value = false;
                if !match_boolean(text, pos, &mut value) {
                    return false;
                }
                self.store_mut(to)
                    .set_byte(to.rec, to.pos, 0, i32::from(value));
            }
            5 => {
                let mut value = String::new();
                if !match_text(text, pos, &mut value) {
                    return false;
                }
                let text_pos = self.store_mut(to).set_str(&value);
                self.store_mut(to).set_int(to.rec, to.pos, text_pos as i32);
            }
            _ => {
                return false;
            }
        }
        true
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
        // P105: if the value is not a valid live record, the data is
        // stored inline (e.g., struct field within a vector element).
        // Fall back to offset addition (get_field behavior).
        // Cost: one range check + one header read (no HashSet lookup).
        if !store.is_valid_record(res) {
            return DbRef {
                store_nr: db.store_nr,
                rec: db.rec,
                pos: db.pos + fld,
            };
        }
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
