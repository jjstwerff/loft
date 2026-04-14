// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Type definitions and type metadata for the database.

use crate::calc;
use crate::database::{Field, Parts, Stores};
use crate::keys::Content;
use std::collections::HashSet;
use std::fmt::Write as _;

impl Stores {
    /**
    To define the 7 base types of the language.
    */
    pub(super) fn base_type(&mut self, name: &str, size: u8) {
        self.names.insert(name.to_string(), self.types.len() as u16);
        self.types
            .push(Type::new(name, Parts::Base, u16::from(size)));
    }

    /**
    Define a new database structure (record).
    # Panics
    when such a structure already exists.
    */
    pub fn structure(&mut self, name: &str, enum_value: i32) -> u16 {
        let num = self.types.len() as u16;
        assert!(
            !self.names.contains_key(name),
            "Double structure type {name}"
        );
        self.names.insert(name.to_string(), num);
        let mut tp = Type::new(
            name,
            if enum_value <= 0 {
                Parts::Struct(Vec::new())
            } else {
                Parts::EnumValue(enum_value as u8, Vec::new())
            },
            u16::MAX,
        );
        tp.align = u8::MAX;
        self.types.push(tp);
        num
    }

    #[must_use]
    pub fn has_type(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    #[allow(dead_code)]
    pub fn set_default(&mut self, tp: u16, f: u16, value: Content) {
        if let Parts::Struct(fld) | Parts::EnumValue(_, fld) = &mut self.types[tp as usize].parts {
            fld[f as usize].default = value;
        }
    }

    /**
    Add a new field to a structure
    # Panics
    When the field has a position outside the structure size or on a non-structure type.
    */
    pub fn field(&mut self, structure: u16, name: &str, content: u16) -> u16 {
        if content == u16::MAX {
            return 0;
        }
        let mut others = Vec::new();
        let mut linked = std::collections::HashMap::new();
        if matches!(
            self.types[content as usize].parts,
            Parts::Struct(_) | Parts::EnumValue(_, _) | Parts::Enum(_)
        ) {
            self.types[content as usize].parents.insert(structure);
        }
        if let Parts::Array(c)
        | Parts::Vector(c)
        | Parts::Sorted(c, _)
        | Parts::Ordered(c, _)
        | Parts::Hash(c, _)
        | Parts::Index(c, _, _)
        | Parts::Spacial(c, _) = self.types[content as usize].parts
        {
            self.types[c as usize].parents.insert(structure);
        }
        if let Parts::Struct(fld) | Parts::EnumValue(_, fld) = &self.types[structure as usize].parts
        {
            // only link fields that are indexing types (sorted, hash, index),
            // not plain vectors. Two vector<integer> fields must NOT be linked —
            // inserting into one must not propagate to the other.
            let is_index_type = matches!(
                self.types[content as usize].parts,
                Parts::Sorted(_, _)
                    | Parts::Ordered(_, _)
                    | Parts::Hash(_, _)
                    | Parts::Index(_, _, _)
            );
            if is_index_type {
                for (f_nr, f) in fld.iter().enumerate() {
                    let fld_content = self.content(f.content);
                    if fld_content != u16::MAX && fld_content == self.content(content) {
                        if others.is_empty() {
                            others.push(u16::MAX);
                        }
                        linked.insert(f_nr as u16, fld.len() as u16);
                    }
                }
            }
        }
        if let Parts::Struct(s) | Parts::EnumValue(_, s) = &mut self.types[structure as usize].parts
        {
            for (f_nr, f) in s.iter_mut().enumerate() {
                if let Some(add) = linked.get(&(f_nr as u16)) {
                    f.other_indexes.push(*add);
                }
            }
            let num = s.len() as u16;
            s.push(Field {
                name: name.to_string(),
                content,
                position: u16::MAX,
                default: crate::keys::Content::Str(crate::keys::Str::new("")),
                other_indexes: others,
            });
            if num > 8
                || self.types[content as usize].complex
                || matches!(
                    self.types[content as usize].parts,
                    Parts::Struct(_) | Parts::EnumValue(_, _)
                )
            {
                self.types[structure as usize].complex = true;
            }
            num
        } else {
            panic!(
                "Adding field {name} to a non structure type {}",
                self.types[structure as usize].name
            );
        }
    }

    #[must_use]
    pub fn content(&self, tp: u16) -> u16 {
        match self.types[tp as usize].parts {
            Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Sorted(c, _)
            | Parts::Index(c, _, _)
            | Parts::Hash(c, _)
            | Parts::Spacial(c, _) => c,
            _ => u16::MAX,
        }
    }

    #[must_use]
    pub fn is_linked(&self, tp: u16) -> bool {
        tp != u16::MAX && self.types[tp as usize].linked
    }

    #[must_use]
    pub fn is_base(&self, tp: u16) -> bool {
        tp != u16::MAX && matches!(self.types[tp as usize].parts, Parts::Base | Parts::Enum(_))
    }

    #[must_use]
    pub fn field_type(&self, rec: u16, fld: u16) -> u16 {
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &self.types[rec as usize].parts
        {
            fields[fld as usize].content
        } else {
            u16::MAX
        }
    }

    /**
    Determine how structures are actually used.
    */
    pub fn finish(&mut self) {
        let mut vectors = HashSet::new();
        let mut linked = HashSet::new();
        for t_nr in 0..self.types.len() {
            if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &self.types[t_nr].parts {
                for f in fields {
                    match self.types[f.content as usize].parts {
                        Parts::Vector(v) | Parts::Sorted(v, _) => vectors.insert(v),
                        Parts::Hash(r, _) | Parts::Spacial(r, _) | Parts::Index(r, _, _) => {
                            linked.insert(r)
                        }
                        _ => false,
                    };
                }
            }
            if let Parts::Sorted(v, _) = &self.types[t_nr].parts {
                vectors.insert(*v);
            }
        }
        let mut in_progress = HashSet::new();
        for t_nr in 0..self.types.len() {
            self.finish_type(&linked, t_nr, &mut in_progress);
        }
        self.determine_keys();
        // self.dump_types();
    }

    pub(super) fn finish_type(
        &mut self,
        linked: &HashSet<u16>,
        t_nr: usize,
        in_progress: &mut HashSet<usize>,
    ) {
        if !matches!(
            self.types[t_nr].parts,
            Parts::Struct(_) | Parts::Enum(_) | Parts::EnumValue(_, _)
        ) || self.types[t_nr].size != u16::MAX
            || in_progress.contains(&t_nr)
        {
            return;
        }
        in_progress.insert(t_nr);
        let mut sizes = Vec::new();
        if let Parts::Enum(values) = self.types[t_nr].parts.clone() {
            let mut size = 1;
            let mut align = 1;
            for value in values {
                if value.0 != u16::MAX {
                    self.finish_type(linked, value.0 as usize, in_progress);
                    if size < self.types[value.0 as usize].size {
                        size = self.types[value.0 as usize].size;
                    }
                    if align < self.types[value.0 as usize].align {
                        align = self.types[value.0 as usize].align;
                    }
                }
            }
            self.types[t_nr].size = size;
            self.types[t_nr].align = align;
        }
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = self.types[t_nr].parts.clone()
        {
            for f in fields {
                let c_nr = f.content as usize;
                if self.types[c_nr].size == u16::MAX && c_nr != t_nr {
                    self.finish_type(linked, c_nr, in_progress);
                }
                sizes.push((self.types[c_nr].size, self.types[c_nr].align));
                if let Parts::Vector(c) = self.types[c_nr].parts
                    && linked.contains(&c)
                {
                    self.types[c as usize].linked = true;
                    self.types[c_nr].parts = Parts::Array(c);
                    self.types[c_nr].name = format!("array<{}>", self.types[c as usize].name);
                }
                if let Parts::Sorted(c, key) = self.types[c_nr].parts.clone()
                    && linked.contains(&c)
                {
                    let mut name = format!("ordered<{}[", self.types[c as usize].name);
                    self.key_name(c, &key, &mut name);
                    self.types[c as usize].linked = true;
                    self.types[c_nr].parts = Parts::Ordered(c, key.clone());
                    self.types[c_nr].name = name;
                }
            }
        }
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &mut self.types[t_nr].parts {
            let mut size = 0;
            let mut alignment = 0;
            if !fields.is_empty() {
                let pos = calc::calculate_positions(
                    &sizes,
                    fields[0].name == "enum",
                    &mut size,
                    &mut alignment,
                );
                for (field_nr, pos) in pos.iter().enumerate() {
                    fields[field_nr].position = *pos;
                }
            }
            self.types[t_nr].size = size;
            self.types[t_nr].align = alignment;
        }
    }

    pub(super) fn determine_keys(&mut self) {
        for t_nr in 0..self.types.len() {
            match self.types[t_nr].parts.clone() {
                Parts::Hash(c, key_fields) => {
                    if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                        &self.types[c as usize].parts.clone()
                    {
                        self.types[t_nr].keys.clear();
                        for key_field in key_fields {
                            let fld = &fields[key_field as usize];
                            let tp = if fld.content > 5 {
                                7
                            } else {
                                1 + fld.content as i8
                            };
                            self.types[t_nr].keys.push(crate::keys::Key {
                                type_nr: tp,
                                position: fld.position,
                            });
                        }
                    }
                }
                Parts::Ordered(c, key_fields)
                | Parts::Sorted(c, key_fields)
                | Parts::Index(c, key_fields, _) => {
                    if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                        &self.types[c as usize].parts.clone()
                    {
                        self.types[t_nr].keys.clear();
                        for (key_field, asc) in &key_fields {
                            let fld = &fields[*key_field as usize];
                            let mut tp = if fld.content > 5 {
                                7
                            } else {
                                1 + fld.content as i8
                            };
                            if !asc {
                                tp = -tp;
                            }
                            self.types[t_nr].keys.push(crate::keys::Key {
                                type_nr: tp,
                                position: fld.position,
                            });
                        }
                    }
                }
                _ => (),
            }
        }
    }

    #[allow(dead_code)]
    pub fn dump_types(&self) {
        for t_nr in 0..self.types.len() {
            print!("{t_nr}:{}", self.show_type(t_nr as u16, true));
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn dump_type(&self, name: &str) -> String {
        for t in 0..self.types.len() {
            if self.types[t].name == name {
                return self.show_type(t as u16, false);
            }
        }
        String::new()
    }

    pub fn vector(&mut self, content: u16) -> u16 {
        let name = if content == u16::MAX {
            "vector".to_string()
        } else {
            format!("vector<{}>", &self.types[content as usize].name)
        };
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types.push(Type::data(&name, Parts::Vector(content)));
            self.names.insert(name, num);
            num
        }
    }

    pub fn hash(&mut self, content: u16, key: &[String]) -> u16 {
        let mut name = "hash<".to_string() + &self.types[content as usize].name + "[";
        let mut key_nrs = Vec::new();
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[content as usize].parts
        {
            for (k_nr, k) in key.iter().enumerate() {
                if k_nr > 0 {
                    name += ",";
                }
                name += k;
                for (f_nr, f) in fields.iter().enumerate() {
                    if f.name == *k {
                        key_nrs.push(f_nr as u16);
                    }
                }
            }
        }
        name += "]>";
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::data(&name, Parts::Hash(content, key_nrs)));
            self.names.insert(name, num);
            num
        }
    }

    pub fn spacial(&mut self, content: u16, key: &[String]) -> u16 {
        let mut name = "spacial<".to_string() + &self.types[content as usize].name + "[";
        let key_nrs = self.field_name(content, key, &mut name);
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::data(&name, Parts::Spacial(content, key_nrs)));
            self.names.insert(name, num);
            num
        }
    }

    pub fn field_name(&self, content: u16, key: &[String], name: &mut String) -> Vec<u16> {
        let mut key_nrs = Vec::new();
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[content as usize].parts
        {
            for (k_nr, k) in key.iter().enumerate() {
                if k_nr > 0 {
                    *name += ",";
                }
                *name += k;
                for (f_nr, f) in fields.iter().enumerate() {
                    if f.name == *k {
                        key_nrs.push(f_nr as u16);
                    }
                }
            }
        }
        *name += "]>";
        key_nrs
    }

    #[must_use]
    pub fn field_nr(&self, record: u16, position: i32) -> u16 {
        if record == u16::MAX {
            // Should normally only occur in the first_phase of the parser.
            return 0;
        }
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[record as usize].parts
        {
            for (f_nr, f) in fields.iter().enumerate() {
                if f.position == position as u16 {
                    return f_nr as u16;
                }
            }
        }
        0
    }

    /**
    Keys with field number and ascending flag.
    */
    pub fn sorted(&mut self, content: u16, key: &[(String, bool)]) -> u16 {
        let mut name = "sorted<".to_string() + &self.types[content as usize].name + "[";
        let key_nrs = self.create_key(content, key, &mut name);
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::Sorted(content, key_nrs), 4));
            self.names.insert(name, num);
            num
        }
    }

    pub fn index(&mut self, content: u16, key: &[(String, bool)]) -> u16 {
        let mut name = "index<".to_string() + &self.types[content as usize].name + "[";
        let key_nrs = self.create_key(content, key, &mut name);
        let int_c = self.name("integer");
        let bool_c = self.name("boolean");
        let mut nr = 1;
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[content as usize].parts
        {
            for f in fields {
                if f.name.starts_with("#left_") {
                    nr += 1;
                }
            }
        }
        let left = if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &mut self.types[content as usize].parts
        {
            let left = fields.len();
            fields.push(Field {
                name: format!("#left_{nr}"),
                content: int_c,
                position: 0,
                default: Content::Long(0),
                other_indexes: Vec::new(),
            });
            fields.push(Field {
                name: format!("#right_{nr}"),
                content: int_c,
                position: 0,
                default: Content::Long(0),
                other_indexes: Vec::new(),
            });
            fields.push(Field {
                name: format!("#color_{nr}"),
                content: bool_c,
                position: 0,
                default: Content::Long(0),
                other_indexes: Vec::new(),
            });
            left as u16
        } else {
            u16::MAX
        };
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::Index(content, key_nrs, left), 4));
            self.names.insert(name, num);
            num
        }
    }

    pub(super) fn key_name(&mut self, content: u16, key: &[(u16, bool)], name: &mut String) {
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[content as usize].parts
        {
            for (k_nr, (k, asc)) in key.iter().enumerate() {
                if k_nr > 0 {
                    *name += ",";
                }
                if !*asc {
                    *name += "-";
                }
                *name += &fields[*k as usize].name;
            }
        }
        *name += "]>";
    }

    pub(super) fn create_key(
        &mut self,
        content: u16,
        key: &[(String, bool)],
        name: &mut String,
    ) -> Vec<(u16, bool)> {
        let mut key_nrs = Vec::new();
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[content as usize].parts
        {
            for (k_nr, (k, asc)) in key.iter().enumerate() {
                if k_nr > 0 {
                    *name += ",";
                }
                if !*asc {
                    *name += "-";
                }
                *name += k;
                for (f_nr, f) in fields.iter().enumerate() {
                    if f.name == *k {
                        key_nrs.push((f_nr as u16, *asc));
                    }
                }
            }
        }
        *name += "]>";
        key_nrs
    }

    pub fn byte(&mut self, min: i32, nullable: bool) -> u16 {
        let name = if min == 0 && !nullable {
            "byte".to_string()
        } else {
            format!("byte<{min},{nullable}>")
        };
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::Byte(min, nullable), 1));
            self.names.insert(name, num);
            num
        }
    }

    /**
    Retrieve a defined type number by name.
    # Panics
    When a type name doesn't exist.
    */
    #[must_use]
    pub fn name(&self, name: &str) -> u16 {
        *self.names.get(name).unwrap_or(&u16::MAX)
    }

    pub fn short(&mut self, min: i32, nullable: bool) -> u16 {
        let name = format!("short<{min},{nullable}>");
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::Short(min, nullable), 2));
            self.names.insert(name, num);
            num
        }
    }

    pub fn enumerate(&mut self, name: &str) -> u16 {
        let num = self.types.len() as u16;
        self.types
            .push(Type::new(name, Parts::Enum(Vec::new()), u16::MAX));
        self.names.insert(name.to_string(), num);
        num
    }

    pub fn enum_value(&mut self, enum_tp: u16, value_name: &str, value_tp: u16) {
        // B2 guard: a caller that hasn't yet run type-resolution on the parent
        // enum may pass `u16::MAX` here (known_type unset).  Returning without
        // panicking lets the later passes recover; the missing variant-type
        // link surfaces as a normal type-check error downstream rather than
        // an `index out of bounds` crash in the allocator.
        if enum_tp == u16::MAX || (enum_tp as usize) >= self.types.len() {
            return;
        }
        if let Parts::Enum(variants) = &mut self.types[enum_tp as usize].parts {
            for variant in variants.iter_mut() {
                if variant.1 == value_name {
                    variant.0 = value_tp;
                }
            }
        }
    }

    pub fn db_type(&mut self, tp: &crate::data::Type, data: &crate::data::Data) -> u16 {
        match tp {
            crate::data::Type::Integer(minimum, _, not_null) => {
                let nullable = !not_null;
                let s = tp.size(nullable);
                if s == 1 {
                    self.byte(*minimum, nullable)
                } else if s == 2 {
                    self.short(*minimum, nullable)
                } else {
                    self.name("integer")
                }
            }
            crate::data::Type::Enum(_, false, _) => self.name("byte"),
            _ => data.def(data.type_def_nr(tp)).known_type,
        }
    }

    /**
    Add a value to an enumerated type.
    # Panics
    When adding a value to a non-enumerated variable.
    */
    pub fn value(&mut self, known_type: u16, name: &str, value_type: u16) -> u16 {
        if let Parts::Enum(values) = &mut self.types[known_type as usize].parts {
            let num = values.len() as u16;
            values.push((value_type, name.to_string()));
            num
        } else {
            panic!(
                "Adding a value to a non enum type {}",
                self.types[known_type as usize].name
            );
        }
    }

    #[must_use]
    pub fn enum_val(&self, known_type: u16, value: u8) -> &str {
        if known_type == u16::MAX {
            return "unknown";
        }
        if let Parts::Enum(values) = &self.types[known_type as usize].parts
            && value > 0
            && (value as usize) <= values.len()
        {
            return &values[value as usize - 1].1;
        }
        "null"
    }

    #[must_use]
    pub fn to_enum(&self, known_type: u16, value: &str) -> u8 {
        if let Parts::Enum(values) = &self.types[known_type as usize].parts {
            for (idx, val) in values.iter().enumerate() {
                if val.1 == value {
                    return 1 + idx as u8;
                }
            }
        }
        0u8
    }

    #[must_use]
    pub fn is_null(
        &self,
        store: &crate::store::Store,
        rec: u32,
        pos: u32,
        known_type: u16,
    ) -> bool {
        if rec == 0 {
            return true;
        }
        if known_type < 6 {
            match known_type {
                0 | 6 => store.get_int(rec, pos) == i32::MIN,
                1 => store.get_long(rec, pos) == i64::MIN,
                2 => store.get_single(rec, pos).is_nan(),
                3 => store.get_float(rec, pos).is_nan(),
                4 => store.get_byte(rec, pos, 0) > 1,
                5 => {
                    store.get_int(rec, pos) == 0
                        || store.get_str(store.get_int(rec, pos) as u32).is_empty()
                }
                _ => false,
            }
        } else if let Parts::Enum(_) = &self.types[known_type as usize].parts {
            store.get_byte(rec, pos, 0) == 0
        } else if let Parts::Struct(_) | Parts::EnumValue(_, _) =
            &self.types[known_type as usize].parts
        {
            rec == 0
        } else if let Parts::Vector(_) = &self.types[known_type as usize].parts {
            store.get_int(rec, pos) == 0
        } else if let Parts::Byte(from, nullable) = &self.types[known_type as usize].parts {
            let v = store.get_byte(rec, pos, *from);
            *nullable && v == 255
        } else if let Parts::Short(from, nullable) = &self.types[known_type as usize].parts {
            let v = store.get_short(rec, pos, *from);
            *nullable && v == 65535
        } else {
            false
        }
    }

    #[must_use]
    pub fn size(&self, tp: u16) -> u16 {
        if tp == u16::MAX {
            0
        } else {
            self.types[tp as usize].size
        }
    }

    #[must_use]
    pub fn position(&self, tp: u16, field: &str) -> u16 {
        if tp == u16::MAX {
            u16::MAX
        } else if let Parts::Struct(f) | Parts::EnumValue(_, f) = &self.types[tp as usize].parts {
            for fld in f {
                if fld.name == field {
                    return fld.position;
                }
            }
            u16::MAX
        } else {
            u16::MAX
        }
    }

    #[must_use]
    pub fn is_text_type(&self, tp: u16) -> bool {
        self.names.get("text").copied() == Some(tp)
    }

    pub(super) fn show_fields(&self, pretty: bool, res: &mut String, v: &[Field]) {
        if pretty {
            *res += "\n";
        } else {
            *res += "{";
        }
        for (f_nr, p) in v.iter().enumerate() {
            let name = &self.types[p.content as usize].name;
            if pretty {
                *res += "    ";
            } else if f_nr > 0 {
                *res += ", ";
            }
            write!(res, "{}:{name}[{}]", p.name, p.position).unwrap();
            if !p.other_indexes.is_empty() {
                write!(res, " other {:?}", p.other_indexes).unwrap();
            }
            if let Content::Str(val) = p.default
                && val.len == 0
            {
            } else if let Content::Long(v) = p.default
                && v == 0
            {
            } else {
                write!(res, " default {:?}", p.default).unwrap();
            }
            if pretty {
                *res += "\n";
            }
        }
        if !pretty {
            *res += "}";
        }
    }

    #[must_use]
    pub fn show_type(&self, tp: u16, pretty: bool) -> String {
        if tp > self.types.len() as u16 {
            return format!("Unknown type({tp})");
        }
        let typedef = &self.types[tp as usize];
        let mut res = format!("{}[{}/{}]:", typedef.name, typedef.size, typedef.align);
        if let Parts::EnumValue(nr, _) = typedef.parts {
            write!(res, " EnumValue({nr})").unwrap();
        }
        if !typedef.parents.is_empty() {
            write!(res, " parents [").unwrap();
            for (n, p) in typedef.parents.iter().enumerate() {
                if n > 0 {
                    write!(res, ", ").unwrap();
                }
                write!(res, "{} {p}", self.types[*p as usize].name).unwrap();
            }
            write!(res, "]").unwrap();
        }
        if let Parts::Struct(v) | Parts::EnumValue(_, v) = &typedef.parts {
            self.show_fields(pretty, &mut res, v);
        } else if let Parts::Enum(v) = &typedef.parts {
            if pretty {
                res += "\n";
            } else {
                res += "[";
            }
            for (e_nr, (nr, e)) in v.iter().enumerate() {
                if pretty {
                    res += "    ";
                } else if e_nr > 0 {
                    res += ", ";
                }
                write!(res, "{e}").unwrap();
                if *nr != u16::MAX {
                    write!(res, ":{nr}").unwrap();
                }
                if pretty {
                    res += "\n";
                }
            }
            if !pretty {
                res += "]";
            }
        } else {
            write!(res, "{:?}", &typedef.parts).unwrap();
            if !typedef.keys.is_empty() {
                res += " keys [";
                for k in &typedef.keys {
                    write!(
                        res,
                        "tp:{} desc:{} field:{}, ",
                        k.type_nr.abs(),
                        k.type_nr < 0,
                        k.position
                    )
                    .unwrap();
                }
                res += "]";
            }
            if pretty {
                res += "\n";
            }
        }
        res
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub name: String,
    pub parts: Parts,
    pub keys: Vec<crate::keys::Key>,
    pub(super) parents: std::collections::BTreeSet<u16>,
    pub(super) complex: bool,
    pub(super) linked: bool,
    pub(super) size: u16,
    pub(super) align: u8,
}

impl Type {
    pub(super) fn new(name: &str, parts: Parts, size: u16) -> Type {
        Type {
            name: name.to_string(),
            parts,
            keys: Vec::new(),
            parents: std::collections::BTreeSet::new(),
            complex: false,
            linked: false,
            size,
            align: size as u8,
        }
    }

    pub(super) fn data(name: &str, parts: Parts) -> Type {
        Type {
            name: name.to_string(),
            parts,
            keys: Vec::new(),
            parents: std::collections::BTreeSet::new(),
            complex: true,
            linked: false,
            size: 4,
            align: 4,
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn contains(&self, tp: u16) -> bool {
        match self.parts {
            Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Sorted(c, _)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Index(c, _, _)
            | Parts::Spacial(c, _) => c == tp,
            _ => false,
        }
    }
}
