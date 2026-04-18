// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Display/debug formatting: `show`, `show_value`, `dump` functions.

use crate::database::{DumpDb, Field, Parts, ShowDb, Stores};
use crate::keys::{self, DbRef};
use crate::store::Store;
use crate::vector;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter, Write as _};

/// Render a walker / unified-parser failure as `"line N:M path:X"`.
///
/// Replaces the legacy `parse_key` / `show_key` machinery — the
/// path is now collected by the walker as it descends, and the
/// (line, col) pair comes straight from `crate::json::line_col_of`.
fn format_walk_err(text: &str, at: usize, path: &[String]) -> String {
    let (line, col) = crate::json::line_col_of(text, at);
    let mut out = format!("line {line}:{col} path:");
    for (i, seg) in path.iter().enumerate() {
        if i > 0 && !seg.starts_with('[') {
            out.push('.');
        }
        out.push_str(seg);
    }
    out
}

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
        let p_rec = self.store(db).get_u32_raw(db.rec, 4);
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
                            let rec = self.store(db).get_u32_raw(data.rec, data.pos);
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
                                        self.store(db).get_u32_raw(sub_data.rec, sub_data.pos);
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
    ///
    /// P54-U phase 2: routes through the unified
    /// `crate::json::parse_with(text, Dialect::Lenient)` + the
    /// schema-driven [`Stores::walk_parsed_into`] walker.  On
    /// unified-path failure (syntax error OR schema/shape
    /// mismatch the walker doesn't yet cover) the call falls
    /// back to the legacy hand-rolled scanner so the
    /// `"line N:M path:X"` error shape stays consistent with
    /// existing tests and tooling.  The fallback will be
    /// removed once the walker's coverage is proven across the
    /// full test matrix.
    pub fn parse(&mut self, text: &str, tp: u16, result: &DbRef) -> Option<String> {
        self.try_parse_unified(text, tp, result).err()
    }

    // Used for testing, returns the interpreted data or the error path on problems.
    pub fn parse_message(&mut self, text: &str, tp: u16) -> String {
        let db = self.database(u32::from(self.types[tp as usize].size));
        self.store_mut(&db).set_u32_raw(db.rec, 4, u32::from(tp));
        match self.try_parse_unified(text, tp, &db) {
            Ok(()) => {
                let mut s = String::new();
                self.show(&mut s, &db, tp, false);
                s
            }
            Err(msg) => msg,
        }
    }

    /// Run the unified parse-then-walk path and translate any
    /// failure into the user-visible `"line N:M path:X"` shape.
    ///
    /// The unified parser ([`crate::json::parse_with`]) handles
    /// syntax-level errors (returns `ParseError` with byte offset),
    /// and the schema walker ([`Stores::walk_parsed_into`]) handles
    /// shape / type mismatches against the loft type definition
    /// (returns `WalkErr` with byte offset + dotted path).
    /// Both feed into the same `"line N:M path:X"` format that
    /// `tests/data_structures.rs::record` asserts.
    fn try_parse_unified(&mut self, text: &str, tp: u16, result: &DbRef) -> Result<(), String> {
        let parsed = crate::json::parse_with(text, crate::json::Dialect::Lenient)
            .map_err(|e| format_walk_err(text, e.byte_offset, &[]))?;
        let mut path: Vec<String> = Vec::new();
        self.walk_parsed_into(&parsed, tp, tp, u16::MAX, result, &mut path)
            .map_err(|e| format_walk_err(text, e.at, &e.path))
    }

    /**
    Get the command line arguments into a vector
    # Panics
    When the OS provided incorrect arguments (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_arguments(&mut self) -> DbRef {
        // P168: always return the curated script-level args.  The old
        // `fallback to std::env::args_os()` leaked the binary path +
        // loft CLI flags when `user_args` was empty (i.e. when the
        // script was invoked with no arguments) — P131's filter only
        // ran through the `user_args` path.  `user_args` is the
        // authoritative list; an empty one is a correct result.
        let args = self.user_args.clone();
        self.text_vector(&args)
    }

    /// Build a `vector<text>` from an explicit string slice.
    #[must_use]
    pub fn text_vector(&mut self, args: &[String]) -> DbRef {
        let vec = self.database(4);
        self.store_mut(&vec).set_u32_raw(vec.rec, vec.pos, 0);
        for v in args {
            let elm = vector::vector_append(&vec, 4, &mut self.allocations);
            let s = self.store_mut(&vec).set_str(v.as_str());
            self.store_mut(&vec).set_u32_raw(elm.rec, elm.pos, s);
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
        self.store_mut(&vec).set_u32_raw(vec.rec, vec.pos, 0);
        #[cfg(not(feature = "wasm"))]
        for t in std::env::vars_os() {
            let name = t.0.to_str().unwrap();
            let value = t.1.to_str().unwrap();
            let elm = vector::vector_append(&vec, size, &mut self.allocations);
            let n = self.store_mut(&vec).set_str(name);
            let v = self.store_mut(&vec).set_str(value);
            self.store_mut(&vec).set_u32_raw(elm.rec, elm.pos, n);
            self.store_mut(&vec).set_u32_raw(elm.rec, elm.pos + 4, v);
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
    #[cfg(not(feature = "wasm"))]
    pub fn os_variable(name: &str) -> crate::keys::Str {
        if let Some(v) = std::env::var_os(name) {
            crate::keys::Str::new(v.to_str().unwrap())
        } else {
            crate::keys::Str::new("")
        }
    }

    /**
    Get the value of an environment variable (WASM stub — always returns empty).
    */
    #[must_use]
    #[cfg(feature = "wasm")]
    pub fn os_variable(name: &str) -> crate::keys::Str {
        let val = crate::wasm::host_env_variable(name);
        crate::keys::Str::new(&val)
    }

    /**
    Get the current directory
    # Panics
    When the OS provided incorrect variable values (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_directory(s: &mut String) -> crate::keys::Str {
        s.clear();
        #[cfg(not(feature = "wasm"))]
        if let Ok(v) = std::env::current_dir() {
            *s += v.to_str().unwrap();
        }
        #[cfg(feature = "wasm")]
        {
            *s = crate::wasm::host_fs_cwd();
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
        #[cfg(not(feature = "wasm"))]
        if let Some(v) = dirs::home_dir() {
            *s += v.to_str().unwrap();
        }
        #[cfg(feature = "wasm")]
        {
            *s = crate::wasm::host_fs_user_dir();
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
        #[cfg(not(feature = "wasm"))]
        if let Ok(v) = std::env::current_exe() {
            *s += v.to_str().unwrap();
        }
        #[cfg(feature = "wasm")]
        {
            *s = crate::wasm::host_fs_program_dir();
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
        #[cfg(not(feature = "wasm"))]
        {
            let mut s = String::new();
            if let Ok(v) = std::env::current_dir() {
                s += v.to_str().unwrap();
            }
            s
        }
        #[cfg(feature = "wasm")]
        crate::wasm::host_fs_cwd()
    }

    /// Native-codegen variant of `os_home` that returns an owned `String`.
    ///
    /// # Panics
    /// Panics if the home directory path contains non-UTF-8 characters.
    #[must_use]
    pub fn os_home_native() -> String {
        #[cfg(not(feature = "wasm"))]
        {
            let mut s = String::new();
            if let Some(v) = dirs::home_dir() {
                s += v.to_str().unwrap();
            }
            s
        }
        #[cfg(feature = "wasm")]
        crate::wasm::host_fs_user_dir()
    }

    /// Native-codegen variant of `os_executable` that returns an owned `String`.
    ///
    /// # Panics
    /// Panics if the executable path contains non-UTF-8 characters.
    #[must_use]
    pub fn os_executable_native() -> String {
        #[cfg(not(feature = "wasm"))]
        {
            let mut s = String::new();
            if let Ok(v) = std::env::current_exe() {
                s += v.to_str().unwrap();
            }
            s
        }
        #[cfg(feature = "wasm")]
        crate::wasm::host_fs_program_dir()
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
            let text_nr = self.store().get_u32_raw(self.rec, self.pos);
            if text_nr == 0 || text_nr >= self.store().capacity_words() {
                write!(s, "<bad-text:{text_nr}>").unwrap();
            } else {
                let text_val = self.store().get_str(text_nr);
                s.push('\"');
                s.push_str(text_val);
                s.push('\"');
            }
        } else if self.known_type == 6 {
            let i = self.store().get_u32_raw(self.rec, self.pos);
            if i != u32::MAX
                && let Some(ch) = char::from_u32(i)
            {
                write!(s, "'{ch}'").unwrap();
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
                Parts::Struct(st) => {
                    self.write_struct(s, st, indent);
                }
                Parts::EnumValue(_, st) => {
                    // P159: wrap struct-enum variant in a discriminant
                    // object so JSON round-trip can identify the variant.
                    // Output: {"VariantName":{fields}} in JSON mode.
                    if self.json {
                        let variant_name = &self.stores.types[self.known_type as usize].name;
                        write!(s, "{{\"{variant_name}\":").unwrap();
                        self.write_struct(s, st, indent);
                        s.push('}');
                    } else {
                        self.write_struct(s, st, indent);
                    }
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

    #[allow(dead_code)]
    fn write_hash(&self, s: &mut String, content: u16, indent: u16, data: &DbRef, complex: bool) {
        let mut map = BTreeMap::new();
        let mut pos = i32::MAX;
        let rec = self.stores.store_nr(self.store).get_u32_raw(data.rec, data.pos);
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
            let rec = self.stores.store_nr(self.store).get_i32_raw(rec, pos as u32);
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

// ─── DumpDb: structured debug dump with references and limits ────────────────

impl Stores {
    /// Produce a structured debug dump string showing store/record references.
    /// Multi-line with indentation when `compact` is false.
    #[must_use]
    pub fn dump_data(&self, db: &DbRef, tp: u16, max_depth: u16, max_elements: u16) -> String {
        let mut s = String::new();
        DumpDb {
            stores: self,
            store: db.store_nr,
            rec: db.rec,
            pos: db.pos,
            known_type: tp,
            max_depth,
            max_elements,
            compact: false,
        }
        .write(&mut s, 0, 0);
        s
    }

    /// Compact single-line dump for inline trace output.
    #[must_use]
    pub fn dump_compact(&self, db: &DbRef, tp: u16, max_depth: u16, max_elements: u16) -> String {
        let mut s = String::new();
        DumpDb {
            stores: self,
            store: db.store_nr,
            rec: db.rec,
            pos: db.pos,
            known_type: tp,
            max_depth,
            max_elements,
            compact: true,
        }
        .write(&mut s, 0, 0);
        s
    }
}

impl DumpDb<'_> {
    fn store(&self) -> &Store {
        let r = DbRef {
            store_nr: self.store,
            rec: 0,
            pos: 0,
        };
        self.stores.store(&r)
    }

    fn sep(&self, s: &mut String, level: u16) {
        if self.compact {
            s.push(' ');
        } else {
            s.push('\n');
            for _ in 0..level {
                s.push_str("  ");
            }
        }
    }

    /// Write the dump to string `s` at the given indent level and depth.
    pub fn write(&self, s: &mut String, indent: u16, depth: u16) {
        if self.rec == 0 {
            s.push_str("null");
            return;
        }
        // Guard: ensure the record is within the store's buffer before reading.
        let store = self.store();
        if u64::from(self.rec) * 8 + u64::from(self.pos) + 8 > store.byte_capacity() {
            write!(s, "<oob:rec={},pos={}>", self.rec, self.pos).unwrap();
            return;
        }
        match self.known_type {
            0 => write!(s, "{}", self.store().get_int(self.rec, self.pos)).unwrap(), // integer
            1 => write!(s, "{}l", self.store().get_long(self.rec, self.pos)).unwrap(), // long
            2 => write!(s, "{}f", self.store().get_single(self.rec, self.pos)).unwrap(), // single
            3 => write!(s, "{}", self.store().get_float(self.rec, self.pos)).unwrap(), // float
            4 => s.push_str(if self.store().get_byte(self.rec, self.pos, 0) == 0 {
                "false"
            } else {
                "true"
            }),
            5 => {
                // text
                let text_nr = self.store().get_u32_raw(self.rec, self.pos);
                if text_nr == 0 || text_nr >= self.store().capacity_words() {
                    write!(s, "<bad-text:{text_nr}>").unwrap();
                } else {
                    let text_val = self.store().get_str(text_nr);
                    write!(s, "\"{}\"", text_val.replace('"', "\\\"")).unwrap();
                }
            }
            6 => {
                // character
                let i = self.store().get_u32_raw(self.rec, self.pos);
                if let Some(ch) = char::from_u32(i) {
                    write!(s, "'{ch}'").unwrap();
                } else {
                    write!(s, "'?{i}'").unwrap();
                }
            }
            tp if (tp as usize) < self.stores.types.len() => {
                self.write_typed(s, indent, depth);
            }
            tp => write!(s, "?type({tp})").unwrap(),
        }
    }

    fn write_typed(&self, s: &mut String, indent: u16, depth: u16) {
        match &self.stores.types[self.known_type as usize].parts.clone() {
            Parts::Enum(vals) => {
                let v = self.store().get_byte(self.rec, self.pos, 0);
                let name = if v <= 0 {
                    "null"
                } else if (v as usize - 1) < vals.len() {
                    &vals[v as usize - 1].1
                } else {
                    "?"
                };
                s.push_str(name);
                let tp_nr = if v <= 0 || (v as usize - 1) >= vals.len() {
                    u16::MAX
                } else {
                    vals[v as usize - 1].0
                };
                if tp_nr != u16::MAX
                    && let Parts::EnumValue(_, st) = &self.stores.types[tp_nr as usize].parts
                {
                    s.push(' ');
                    self.write_struct(s, st, indent, depth);
                }
            }
            Parts::Struct(st) | Parts::EnumValue(_, st) => {
                self.write_struct(s, st, indent, depth);
            }
            Parts::Vector(tp)
            | Parts::Sorted(tp, _)
            | Parts::Array(tp)
            | Parts::Ordered(tp, _)
            | Parts::Index(tp, _, _) => {
                self.write_list(s, *tp, indent, depth);
            }
            Parts::Hash(_, _) | Parts::Spacial(_, _) => {
                // Hash and Spacial don't support sequential next() — show count only.
                let data = DbRef {
                    store_nr: self.store,
                    rec: self.rec,
                    pos: self.pos,
                };
                let len = vector::length_vector(&data, &self.stores.allocations);
                write!(s, "#{}.? [{len} items]", self.store).unwrap();
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
                write!(s, "?base({})", self.known_type).unwrap();
            }
        }
    }

    fn write_struct(&self, s: &mut String, fields: &[Field], indent: u16, depth: u16) {
        // Show store:record reference
        write!(s, "#{}.{}", self.store, self.rec).unwrap();
        if depth >= self.max_depth {
            s.push_str(" {...}");
            return;
        }
        s.push_str(" {");
        let mut first = true;
        for fld in fields {
            if fld.name == "enum" || fld.name.starts_with('#') {
                continue;
            }
            if self.stores.is_null(
                self.store(),
                self.rec,
                self.pos + u32::from(fld.position),
                fld.content,
            ) {
                continue;
            }
            if !first {
                s.push(',');
            }
            first = false;
            self.sep(s, indent + 1);
            s.push_str(&fld.name);
            s.push_str(": ");
            DumpDb {
                stores: self.stores,
                store: self.store,
                rec: self.rec,
                pos: self.pos + u32::from(fld.position),
                known_type: fld.content,
                max_depth: self.max_depth,
                max_elements: self.max_elements,
                compact: self.compact,
            }
            .write(s, indent + 1, depth + 1);
        }
        self.sep(s, indent);
        s.push('}');
    }

    fn write_list(&self, s: &mut String, content: u16, indent: u16, depth: u16) {
        let data = DbRef {
            store_nr: self.store,
            rec: self.rec,
            pos: self.pos,
        };
        // Show the vector record reference
        let vec_rec = if data.rec > 0 {
            self.store().get_u32_raw(data.rec, data.pos)
        } else {
            0
        };
        write!(s, "#{}.{}", self.store, vec_rec).unwrap();
        if depth >= self.max_depth {
            let len = vector::length_vector(&data, &self.stores.allocations);
            write!(s, " [{len} items...]").unwrap();
            return;
        }
        s.push_str(" [");
        let mut pos = i32::MAX;
        let mut count: u16 = 0;
        loop {
            if data.rec == 0 {
                break;
            }
            let rec = self.stores.next(&data, &mut pos, self.known_type);
            if rec.rec == 0 {
                break;
            }
            if count >= self.max_elements {
                self.sep(s, indent + 1);
                let remaining =
                    vector::length_vector(&data, &self.stores.allocations) as u16 - count;
                write!(s, "...{remaining} more").unwrap();
                break;
            }
            if count > 0 {
                s.push(',');
            }
            self.sep(s, indent + 1);
            DumpDb {
                stores: self.stores,
                store: self.store,
                rec: rec.rec,
                pos: rec.pos,
                known_type: content,
                max_depth: self.max_depth,
                max_elements: self.max_elements,
                compact: self.compact,
            }
            .write(s, indent + 1, depth + 1);
            count += 1;
        }
        self.sep(s, indent);
        s.push(']');
    }
}
