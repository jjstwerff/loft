// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Display/debug formatting: `show`, `show_value`, `dump` functions.

use crate::database::{Field, Parts, ShowDb, Stores};
use crate::keys::{self, DbRef};
use crate::store::Store;
use crate::vector;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter, Write as _};

#[allow(dead_code)]
impl Stores {
    #[must_use]
    pub fn rec(&self, db: &DbRef, tp: u16) -> String {
        let mut res = String::new();
        self.show(&mut res, db, tp, false);
        res
    }

    pub fn dump(&self, db: &DbRef, tp: u16) {
        let mut check = String::new();
        self.show(&mut check, db, tp, true);
        println!("data: {check}");
    }

    pub fn show(&self, s: &mut String, db: &DbRef, tp: u16, pretty: bool) {
        self.valid(db);
        ShowDb {
            stores: self,
            store: db.store_nr,
            rec: db.rec,
            pos: db.pos,
            known_type: tp,
            pretty,
            json: false,
        }
        .write(s, 0);
    }

    /**
    Get the Json-path inspired path to a record.
    # Panics
    When this path cannot be detected correctly.
    */
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn path(&self, db: &DbRef, tp: u16) -> String {
        if db.rec == 1 {
            return "/".to_string();
        }
        let p_rec = self.store(db).get_int(db.rec, 4);
        let p_tp = if self.types[tp as usize].parents.is_empty()
            || self.types[tp as usize].parents.len() > 1
        {
            self.store(db).get_short(p_rec as u32, 8, 0) as u16
        } else {
            *self.types[tp as usize].parents.iter().next().unwrap()
        };
        let parent = DbRef {
            store_nr: db.store_nr,
            rec: p_rec as u32,
            pos: 8,
        };
        let mut res = self.path(&parent, p_tp);
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[p_tp as usize].parts
        {
            for f in fields {
                let f_tp = &self.types[f.content as usize];
                // TODO this for now assumes that the child is linked only once.
                if f_tp.contains(tp) {
                    res += &f.name;
                    res += "[";
                    if f_tp.keys.is_empty() {
                        let data = DbRef {
                            store_nr: db.store_nr,
                            rec: db.rec,
                            pos: 8 + u32::from(f.position),
                        };
                        let mut pos = i32::MAX;
                        let mut count = 0;
                        loop {
                            vector::vector_next(&data, &mut pos, f_tp.size, &self.allocations);
                            if pos == i32::MAX {
                                res += "?";
                                break;
                            }
                            let rec = self.store(db).get_int(data.rec, data.pos) as u32;
                            if rec == db.rec {
                                write!(res, "{count}").unwrap();
                                break;
                            }
                            count += 1;
                        }
                    } else {
                        for (c_nr, c) in keys::get_key(db, &self.allocations, &f_tp.keys)
                            .iter()
                            .enumerate()
                        {
                            if c_nr > 0 {
                                res += ",";
                            }
                            write!(res, "{c}").unwrap();
                        }
                    }
                    res += "]";
                    break;
                }
                // If the field is an embedded sub-struct, check one level deeper:
                // the child type `tp` may live inside a collection that belongs to that sub-struct.
                if let Parts::Struct(sub_fields) | Parts::EnumValue(_, sub_fields) =
                    &self.types[f.content as usize].parts.clone()
                {
                    for sf in sub_fields {
                        let sf_tp = &self.types[sf.content as usize];
                        if sf_tp.contains(tp) {
                            // Build path via the sub-struct field name, then the inner field name.
                            res += &f.name;
                            res += ".";
                            res += &sf.name;
                            res += "[";
                            if sf_tp.keys.is_empty() {
                                let sub_data = DbRef {
                                    store_nr: db.store_nr,
                                    rec: db.rec,
                                    pos: 8 + u32::from(f.position) + u32::from(sf.position),
                                };
                                let mut pos = i32::MAX;
                                let mut count = 0;
                                loop {
                                    vector::vector_next(
                                        &sub_data,
                                        &mut pos,
                                        sf_tp.size,
                                        &self.allocations,
                                    );
                                    if pos == i32::MAX {
                                        res += "?";
                                        break;
                                    }
                                    let rec =
                                        self.store(db).get_int(sub_data.rec, sub_data.pos) as u32;
                                    if rec == db.rec {
                                        write!(res, "{count}").unwrap();
                                        break;
                                    }
                                    count += 1;
                                }
                            } else {
                                for (c_nr, c) in keys::get_key(db, &self.allocations, &sf_tp.keys)
                                    .iter()
                                    .enumerate()
                                {
                                    if c_nr > 0 {
                                        res += ",";
                                    }
                                    write!(res, "{c}").unwrap();
                                }
                            }
                            res += "]";
                            break;
                        }
                    }
                }
            }
        }
        res
    }

    /// Parse the content of a string into an existing record.
    /// Returns `None` on success, or `Some(error_path)` on failure.
    /// The error path is a human-readable string like `"line 1:15 path:items[2].name"`.
    pub fn parse(&mut self, text: &str, tp: u16, result: &DbRef) -> Option<String> {
        let mut pos = 0;
        if self.parsing(text, &mut pos, tp, tp, u16::MAX, result) {
            return None;
        }
        let err_pos = pos;
        pos = 0;
        let mut key = super::ParseKey {
            line: 1,
            line_pos: 0,
            current: Vec::new(),
            step: 0,
        };
        super::parse_key(text, &mut pos, err_pos, &mut key);
        Some(super::show_key(text, &key))
    }

    // Used for testing, returns the interpreted data or the error path on problems.
    pub fn parse_message(&mut self, text: &str, tp: u16) -> String {
        let db = self.database(u32::from(self.types[tp as usize].size));
        self.store_mut(&db).set_int(db.rec, 4, i32::from(tp));
        let mut pos = 0;
        if self.parsing(text, &mut pos, tp, tp, u16::MAX, &db) {
            let mut s = String::new();
            self.show(&mut s, &db, tp, false);
            return s;
        }
        let result = pos;
        pos = 0;
        let mut key = super::ParseKey {
            line: 1,
            line_pos: 0,
            current: Vec::new(),
            step: 0,
        };
        super::parse_key(text, &mut pos, result, &mut key);
        super::show_key(text, &key)
    }

    /**
    Get the command line arguments into a vector
    # Panics
    When the OS provided incorrect arguments (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_arguments(&mut self) -> DbRef {
        let args: Vec<String> = std::env::args_os()
            .map(|a| a.to_str().unwrap().to_string())
            .collect();
        self.text_vector(&args)
    }

    /// Build a `vector<text>` from an explicit string slice.
    #[must_use]
    pub fn text_vector(&mut self, args: &[String]) -> DbRef {
        let vec = self.database(4);
        self.store_mut(&vec).set_int(vec.rec, vec.pos, 0);
        for v in args {
            let elm = vector::vector_append(&vec, 4, &mut self.allocations);
            let s = self.store_mut(&vec).set_str(v.as_str());
            self.store_mut(&vec).set_int(elm.rec, elm.pos, s as i32);
            vector::vector_finish(&vec, &mut self.allocations);
        }
        vec
    }

    /**
    Get all environment variables into a vector
    # Panics
    When the OS provided incorrect variable names (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_variables(&mut self) -> DbRef {
        let elm = self.name("Variable");
        let size = u32::from(self.size(elm));
        let vec = self.database(size);
        self.store_mut(&vec).set_int(vec.rec, vec.pos, 0);
        for t in std::env::vars_os() {
            let name = t.0.to_str().unwrap();
            let value = t.1.to_str().unwrap();
            let elm = vector::vector_append(&vec, size, &mut self.allocations);
            let n = self.store_mut(&vec).set_str(name);
            let v = self.store_mut(&vec).set_str(value);
            self.store_mut(&vec).set_int(elm.rec, elm.pos, n as i32);
            self.store_mut(&vec).set_int(elm.rec, elm.pos + 4, v as i32);
            vector::vector_finish(&vec, &mut self.allocations);
        }
        vec
    }

    /**
    Get the value of an environment variable
    # Panics
    When the OS provided incorrect variable values (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_variable(name: &str) -> crate::keys::Str {
        if let Some(v) = std::env::var_os(name) {
            crate::keys::Str::new(v.to_str().unwrap())
        } else {
            crate::keys::Str::new("")
        }
    }

    /**
    Get the current directory
    # Panics
    When the OS provided incorrect variable values (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_directory(s: &mut String) -> crate::keys::Str {
        s.clear();
        if let Ok(v) = std::env::current_dir() {
            *s += v.to_str().unwrap();
        }
        crate::keys::Str::new(s)
    }

    /**
    Get home directory
    # Panics
    When the OS provided incorrect variable values (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_home(s: &mut String) -> crate::keys::Str {
        s.clear();
        if let Some(v) = dirs::home_dir() {
            *s += v.to_str().unwrap();
        }
        crate::keys::Str::new(s)
    }

    /**
    Get the executable directory
    # Panics
    When the OS provided incorrect variable values (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_executable(s: &mut String) -> crate::keys::Str {
        s.clear();
        if let Ok(v) = std::env::current_exe() {
            *s += v.to_str().unwrap();
        }
        crate::keys::Str::new(s)
    }

    /// Native-codegen variant of `os_directory` that returns an owned `String`.
    /// Used by the `--native` backend where a scratch-buffer `&mut String` is not available.
    ///
    /// # Panics
    /// Panics if the current directory path contains non-UTF-8 characters.
    #[must_use]
    pub fn os_directory_native() -> String {
        let mut s = String::new();
        if let Ok(v) = std::env::current_dir() {
            s += v.to_str().unwrap();
        }
        s
    }

    /// Native-codegen variant of `os_home` that returns an owned `String`.
    ///
    /// # Panics
    /// Panics if the home directory path contains non-UTF-8 characters.
    #[must_use]
    pub fn os_home_native() -> String {
        let mut s = String::new();
        if let Some(v) = dirs::home_dir() {
            s += v.to_str().unwrap();
        }
        s
    }

    /// Native-codegen variant of `os_executable` that returns an owned `String`.
    ///
    /// # Panics
    /// Panics if the executable path contains non-UTF-8 characters.
    #[must_use]
    pub fn os_executable_native() -> String {
        let mut s = String::new();
        if let Ok(v) = std::env::current_exe() {
            s += v.to_str().unwrap();
        }
        s
    }
}

impl Debug for ShowDb<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "({},{}):{}({})",
            self.rec, self.pos, self.stores.types[self.known_type as usize].name, self.known_type
        )
    }
}

impl ShowDb<'_> {
    fn store(&self) -> &Store {
        let r = DbRef {
            store_nr: self.store,
            rec: 0,
            pos: 0,
        };
        self.stores.store(&r)
    }

    /**
    Write data from the database into String s.
    # Panics
    When the database is not correct.
    */
    pub fn write(&self, s: &mut String, indent: u16) {
        if self.rec == 0 {
            write!(s, "null").unwrap();
            return;
        }
        if self.known_type == 0 {
            write!(s, "{}", self.store().get_int(self.rec, self.pos)).unwrap();
        } else if self.known_type == 1 {
            write!(s, "{}", self.store().get_long(self.rec, self.pos)).unwrap();
        } else if self.known_type == 2 {
            write!(s, "{}", self.store().get_single(self.rec, self.pos)).unwrap();
        } else if self.known_type == 3 {
            write!(s, "{}", self.store().get_float(self.rec, self.pos)).unwrap();
        } else if self.known_type == 4 {
            s.push_str(if self.store().get_byte(self.rec, self.pos, 0) == 0 {
                "false"
            } else {
                "true"
            });
        } else if self.known_type == 5 {
            let text_nr = self.store().get_int(self.rec, self.pos) as u32;
            let text_val = self.store().get_str(text_nr);
            s.push('\"');
            s.push_str(text_val);
            s.push('\"');
        } else if self.known_type == 6 {
            let i = self.store().get_int(self.rec, self.pos);
            if i != i32::MAX
                && let Some(ch) = char::from_u32(i as u32)
            {
                write!(s, "'{ch}'",).unwrap();
            }
        } else if (self.known_type as usize) < self.stores.types.len() {
            match &self.stores.types[self.known_type as usize].parts {
                Parts::Enum(vals) => {
                    let v = self.store().get_byte(self.rec, self.pos, 0);
                    let enum_val = if v <= 0 {
                        "null"
                    } else if (v as usize - 1) < vals.len() {
                        &vals[v as usize - 1].1
                    } else {
                        "?"
                    };
                    s.push_str(enum_val);
                    let tp_nr = if v <= 0 || (v as usize - 1) >= vals.len() {
                        u16::MAX
                    } else {
                        vals[v as usize - 1].0
                    };
                    if tp_nr != u16::MAX
                        && let Parts::EnumValue(_, st) = &self.stores.types[tp_nr as usize].parts
                    {
                        s.push(' ');
                        self.write_struct(s, st, indent);
                    }
                }
                Parts::Struct(st) | Parts::EnumValue(_, st) => {
                    self.write_struct(s, st, indent);
                }
                Parts::Vector(tp)
                | Parts::Sorted(tp, _)
                | Parts::Array(tp)
                | Parts::Ordered(tp, _)
                | Parts::Hash(tp, _)
                | Parts::Index(tp, _, _)
                | Parts::Spacial(tp, _) => {
                    self.write_list(s, *tp, indent);
                }
                Parts::Byte(from, nullable) => {
                    let v = self.store().get_byte(self.rec, self.pos, *from);
                    if *nullable && v == 255 {
                        s.push_str("null");
                    } else {
                        write!(s, "{v}").unwrap();
                    }
                }
                Parts::Short(from, nullable) => {
                    let v = self.store().get_short(self.rec, self.pos, *from);
                    if *nullable && v == 65535 {
                        s.push_str("null");
                    } else {
                        write!(s, "{v}").unwrap();
                    }
                }
                Parts::Base => {
                    panic!(
                        "Not matching parts:{:?} type:{} name:{}",
                        self.stores.types[self.known_type as usize].parts,
                        self.known_type,
                        self.stores.types[self.known_type as usize].name
                    )
                }
            }
        } else {
            panic!("Undefined known type {}", self.known_type)
        }
    }

    fn write_indent(&self, complex: bool, s: &mut String, indent: u16, zero_test: bool) {
        if complex && zero_test {
            s.push_str(&ShowDb::new_line(indent + 1));
        } else if self.pretty {
            s.push(' ');
        }
    }

    fn write_struct(&self, s: &mut String, fields: &[Field], indent: u16) {
        let complex = self.pretty && self.stores.types[self.known_type as usize].complex;
        // TODO reference to an object inside a field instead of the object itself, show the key
        s.push('{');
        if self.pretty {
            s.push(' ');
        }
        self.write_fields(s, fields, indent, complex);
        if complex {
            s.push_str(&ShowDb::new_line(indent));
        } else if self.pretty {
            s.push(' ');
        }
        s.push('}');
    }

    fn write_fields(&self, s: &mut String, fields: &[Field], indent: u16, complex: bool) {
        let mut first = true;
        for fld in fields {
            if fld.name == "enum" {
                continue;
            }
            if fld.name.starts_with('#')
                || (!fld.other_indexes.is_empty() && fld.other_indexes[0] == u16::MAX)
                || self.stores.is_null(
                    self.store(),
                    self.rec,
                    self.pos + u32::from(fld.position),
                    fld.content,
                )
            {
                continue;
            }
            if first {
                first = false;
            } else {
                s.push(',');
                self.write_indent(complex, s, indent, true);
            }
            if self.json {
                s.push('"');
            }
            s.push_str(&fld.name);
            if self.json {
                s.push('"');
            }
            s.push(':');
            if self.pretty {
                s.push(' ');
            }
            let sub = ShowDb {
                stores: self.stores,
                store: self.store,
                rec: self.rec,
                pos: self.pos + u32::from(fld.position),
                known_type: fld.content,
                pretty: self.pretty,
                json: self.json,
            };
            sub.write(s, indent + 1);
        }
    }

    fn new_line(indent: u16) -> String {
        let mut res = "\n".to_string();
        for _ in 0..indent {
            res += "  ";
        }
        res
    }

    fn write_list(&self, s: &mut String, content: u16, indent: u16) {
        let data = DbRef {
            store_nr: self.store,
            rec: self.rec,
            pos: self.pos,
        };
        let complex = self.pretty && self.stores.types[content as usize].complex;
        s.push('[');
        if matches!(
            self.stores.types[self.known_type as usize].parts,
            Parts::Hash(_, _)
        ) {
            self.write_hash(s, content, indent, &data, complex);
            return;
        }
        let mut pos = i32::MAX;
        let mut first_elm = true;
        loop {
            if data.rec == 0 {
                break;
            }
            let rec = self.stores.next(&data, &mut pos, self.known_type);
            if rec.rec == 0 {
                break;
            }
            if first_elm {
                if self.pretty {
                    self.write_indent(complex, s, indent, true);
                }
                first_elm = false;
            } else {
                s.push(',');
                if self.pretty {
                    if matches!(
                        self.stores.types[content as usize].parts,
                        Parts::Struct(_) | Parts::EnumValue(_, _)
                    ) {
                        self.write_indent(true, s, indent, true);
                    } else {
                        self.write_indent(complex, s, indent, false);
                    }
                }
            }
            let sub = ShowDb {
                stores: self.stores,
                store: self.store,
                rec: rec.rec,
                pos: rec.pos,
                known_type: content,
                pretty: self.pretty,
                json: self.json,
            };
            sub.write(s, indent + 1);
        }
        if self.pretty {
            s.push(' ');
        }
        s.push(']');
    }

    fn write_hash(&self, s: &mut String, content: u16, indent: u16, data: &DbRef, complex: bool) {
        let mut map = BTreeMap::new();
        let mut pos = i32::MAX;
        let rec = self.stores.store_nr(self.store).get_int(data.rec, data.pos) as u32;
        if rec == 0 {
            s.push(']');
            return;
        }
        let max_pos = *self.stores.store_nr(self.store).addr::<i32>(rec, 0) * 8;
        loop {
            if pos == i32::MAX {
                pos = 8;
            } else if pos < max_pos - 4 {
                pos += 4;
            } else {
                break;
            }
            let rec = self.stores.store_nr(self.store).get_int(rec, pos as u32);
            if rec != 0 {
                let r = DbRef {
                    store_nr: data.store_nr,
                    rec: rec as u32,
                    pos: 8,
                };
                let key = keys::get_simple(
                    &r,
                    &self.stores.allocations,
                    self.stores.keys(self.known_type),
                );
                map.insert(key, rec);
            }
        }
        let mut first_elm = true;
        for (_, p) in map {
            if first_elm {
                if self.pretty {
                    self.write_indent(complex, s, indent, true);
                }
                first_elm = false;
            } else {
                s.push(',');
                if self.pretty {
                    if matches!(self.stores.types[content as usize].parts, Parts::Struct(_)) {
                        self.write_indent(true, s, indent, true);
                    } else {
                        self.write_indent(complex, s, indent, false);
                    }
                }
            }
            let sub = ShowDb {
                stores: self.stores,
                store: self.store,
                rec: p as u32,
                pos: 8,
                known_type: content,
                pretty: self.pretty,
                json: self.json,
            };
            sub.write(s, indent + 1);
        }
        if self.pretty {
            s.push(' ');
        }
        s.push(']');
    }
}
