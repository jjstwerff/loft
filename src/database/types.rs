// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Type definitions and type metadata for the database.

use crate::calc;
use crate::data::IntegerSpec;
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

    /// Debug helper: pretty-print a type's storage layout — overall
    /// size + alignment, plus every field's name, byte position, byte
    /// size, and content type.  Fields are sorted by position so gaps
    /// and overlaps are visually obvious.
    ///
    /// Useful for diagnosing layout bugs (e.g. tree::add writing
    /// at wrong offsets when bookkeeping fields land at non-contiguous
    /// positions per `calc::calculate_positions`'s alignment-aware
    /// reordering).
    ///
    /// For Sorted/Hash/Index/Spacial: also shows the content struct's
    /// layout indented underneath, since those types' bookkeeping
    /// lives inside the content struct.
    #[must_use]
    #[allow(dead_code)]
    pub fn debug_layout(&self, name: &str) -> String {
        for t in 0..self.types.len() {
            if self.types[t].name == name {
                return self.debug_layout_by_nr(t as u16, 0);
            }
        }
        format!("(unknown type: {name})\n")
    }

    /// Same as `debug_layout` but takes a type id directly.
    #[must_use]
    #[allow(dead_code)]
    pub fn debug_layout_by_nr(&self, tp: u16, indent: usize) -> String {
        use std::fmt::Write;
        if tp == u16::MAX || (tp as usize) >= self.types.len() {
            return format!("(unknown type id: {tp})\n");
        }
        let pad = " ".repeat(indent);
        let mut out = String::new();
        let t = &self.types[tp as usize];
        let kind = match &t.parts {
            Parts::Struct(_) => "struct",
            Parts::EnumValue(disc, _) => return self.debug_layout_enumvalue(tp, indent, *disc),
            Parts::Enum(_) => "enum",
            Parts::Vector(_) => "vector",
            Parts::Array(_) => "array",
            Parts::Sorted(_, _) => "sorted",
            Parts::Ordered(_, _) => "ordered",
            Parts::Hash(_, _) => "hash",
            Parts::Index(_, _, _) => "index",
            Parts::Spacial(_, _) => "spacial",
            _ => "<other>",
        };
        let _ = writeln!(
            out,
            "{pad}[{tp}] {} {} (size={}, align={})",
            t.name, kind, t.size, t.align
        );
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &t.parts {
            self.debug_layout_fields(&mut out, indent + 2, fields);
        } else if let Parts::Vector(c)
        | Parts::Array(c)
        | Parts::Sorted(c, _)
        | Parts::Ordered(c, _)
        | Parts::Hash(c, _)
        | Parts::Spacial(c, _) = t.parts
        {
            let _ = writeln!(
                out,
                "{pad}  content → [{c}] {} (size={}, align={})",
                self.types[c as usize].name,
                self.types[c as usize].size,
                self.types[c as usize].align,
            );
            if matches!(
                self.types[c as usize].parts,
                Parts::Struct(_) | Parts::EnumValue(_, _)
            ) {
                out += &self.debug_layout_by_nr(c, indent + 4);
            }
        } else if let Parts::Index(c, _, left_field_nr) = t.parts {
            let _ = writeln!(
                out,
                "{pad}  content → [{c}] {} (size={}, align={}); bookkeeping starts at field index {left_field_nr}",
                self.types[c as usize].name,
                self.types[c as usize].size,
                self.types[c as usize].align,
            );
            // Compute the byte offset where tree::add expects to find
            // RB_LEFT — this is `database.fields(tp)`'s return value.
            if let Parts::Struct(fs) | Parts::EnumValue(_, fs) =
                &self.types[c as usize].parts
            {
                if (left_field_nr as usize) < fs.len() {
                    let left = &fs[left_field_nr as usize];
                    let _ = writeln!(
                        out,
                        "{pad}  tree::add starts at byte offset = 8 + {} (= field[{}] '{}' position) = {}",
                        left.position,
                        left_field_nr,
                        left.name,
                        8 + left.position,
                    );
                }
                out += &self.debug_layout_by_nr(c, indent + 4);
            }
        }
        if !t.keys.is_empty() {
            let _ = writeln!(out, "{pad}  keys:");
            for k in &t.keys {
                let _ = writeln!(
                    out,
                    "{pad}    field {} type {} {}",
                    k.position,
                    k.type_nr.abs(),
                    if k.type_nr < 0 { "desc" } else { "asc" },
                );
            }
        }
        out
    }

    /// Helper for `debug_layout_by_nr` — handles the EnumValue arm
    /// separately so the disc byte is documented.
    fn debug_layout_enumvalue(&self, tp: u16, indent: usize, disc: u8) -> String {
        use std::fmt::Write;
        let pad = " ".repeat(indent);
        let mut out = String::new();
        let t = &self.types[tp as usize];
        let _ = writeln!(
            out,
            "{pad}[{tp}] {} enum-value (disc={disc}, size={}, align={})",
            t.name, t.size, t.align
        );
        if let Parts::EnumValue(_, fields) = &t.parts {
            self.debug_layout_fields(&mut out, indent + 2, fields);
        }
        out
    }

    /// Validate a type's storage layout — detects overlapping fields.
    /// Only `Parts::Enum` (the tagged-union container) legitimately
    /// overlaps its variants; everything else (struct, enum-value,
    /// collection content) must have non-overlapping fields.
    ///
    /// Returns a list of human-readable issues; empty Vec means the
    /// layout is internally consistent.  Recursively checks the
    /// content type for collection kinds.
    ///
    /// Use this after `database.finish()` to catch layout bugs (e.g.
    /// late-mutation that leaves bookkeeping fields at position 0
    /// overlapping user data).
    #[must_use]
    #[allow(dead_code)]
    pub fn validate_layout(&self, name: &str) -> Vec<String> {
        for t in 0..self.types.len() {
            if self.types[t].name == name {
                let mut visited = std::collections::HashSet::new();
                let mut issues = Vec::new();
                self.validate_layout_by_nr(t as u16, &mut visited, &mut issues);
                return issues;
            }
        }
        vec![format!("(unknown type: {name})")]
    }

    /// Walk every registered type and validate its layout.  Returns
    /// a flat list of issues across all types (each line prefixed by
    /// the type name).  Use this after `database.finish()` to catch
    /// layout bugs in any user-defined struct / enum-value /
    /// collection-content type before they corrupt runtime data.
    ///
    /// Skips synthetic database types whose names start with `__`
    /// (currently none, but reserved for the parser-side `__tuple<…>`
    /// shape and similar) and the built-in primitives.
    #[must_use]
    pub fn validate_all_layouts(&self) -> Vec<String> {
        let mut visited = std::collections::HashSet::new();
        let mut issues = Vec::new();
        for tp in 0..self.types.len() {
            let t = &self.types[tp];
            // Skip primitive built-ins (no struct layout to validate)
            // and types whose size is u16::MAX (unlaid-out — likely
            // a parser placeholder that finish_type didn't reach).
            if t.size == u16::MAX {
                continue;
            }
            match t.parts {
                Parts::Struct(_)
                | Parts::EnumValue(_, _)
                | Parts::Enum(_)
                | Parts::Vector(_)
                | Parts::Array(_)
                | Parts::Sorted(_, _)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::Spacial(_, _) => {
                    self.validate_layout_by_nr(tp as u16, &mut visited, &mut issues);
                }
                _ => {}
            }
        }
        // Dedup — recursion can produce the same issue multiple times
        // when a struct is referenced from many places.
        issues.sort();
        issues.dedup();
        issues
    }

    /// Same as `validate_layout` but takes a type id directly.
    /// `visited` prevents infinite recursion through cyclic
    /// references (e.g. struct containing a vector of itself).
    #[allow(dead_code)]
    pub fn validate_layout_by_nr(
        &self,
        tp: u16,
        visited: &mut std::collections::HashSet<u16>,
        issues: &mut Vec<String>,
    ) {
        if tp == u16::MAX || (tp as usize) >= self.types.len() {
            issues.push(format!("type id {tp} out of range"));
            return;
        }
        if !visited.insert(tp) {
            return;
        }
        let t = &self.types[tp as usize];
        match &t.parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                self.check_fields_overlap(tp, fields, issues);
                self.check_fields_within_size(tp, fields, issues);
                // Recurse into each field's content type.
                for f in fields {
                    self.validate_layout_by_nr(f.content, visited, issues);
                }
            }
            Parts::Enum(_) => {
                // Tagged-union variants legitimately overlap.  Each
                // variant is a separate EnumValue type with its own
                // layout — recurse into the children.  We trust the
                // parent's `t.size` to be the max of variant sizes
                // (set in finish_type for Parts::Enum at line 238).
                let children: Vec<u16> = self
                    .types
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, child)| {
                        if let Parts::EnumValue(_, _) = &child.parts {
                            // Heuristic: child belongs to this enum if
                            // its name has the parent's name as prefix
                            // OR if the child appears in `parents`.
                            if child.parents.contains(&tp) {
                                return Some(idx as u16);
                            }
                        }
                        None
                    })
                    .collect();
                for c in children {
                    self.validate_layout_by_nr(c, visited, issues);
                }
            }
            Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Sorted(c, _)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Spacial(c, _) => {
                self.validate_layout_by_nr(*c, visited, issues);
            }
            Parts::Index(c, _, left_field_nr) => {
                // Validate the content struct (which has bookkeeping
                // fields appended).  Also verify the bookkeeping
                // field index is in range and points at a `#left_*`
                // field, since `database.fields(tp)` reads
                // `fields[left_field_nr].position` and a wrong index
                // would silently corrupt tree::add's offsets.
                if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                    &self.types[*c as usize].parts
                {
                    if (*left_field_nr as usize) >= fields.len() {
                        issues.push(format!(
                            "{}: Parts::Index left_field_nr={} out of range (content has {} fields)",
                            t.name,
                            left_field_nr,
                            fields.len()
                        ));
                    } else {
                        let f = &fields[*left_field_nr as usize];
                        if !f.name.starts_with("#left_") {
                            issues.push(format!(
                                "{}: Parts::Index left_field_nr={} points at field '{}' (expected '#left_*')",
                                t.name, left_field_nr, f.name
                            ));
                        }
                        // Bookkeeping must be 3 contiguous fields:
                        // #left (4B), #right (4B), #color (1B).  Their
                        // positions can be anywhere (alignment may
                        // reorder them) but tree::add reads them at
                        // [pos, pos+4, pos+8] — so the LAYOUT must
                        // place them contiguously.
                        if (*left_field_nr as usize + 2) < fields.len() {
                            let l = &fields[*left_field_nr as usize];
                            let r = &fields[*left_field_nr as usize + 1];
                            let cf = &fields[*left_field_nr as usize + 2];
                            if l.position != u16::MAX
                                && r.position != u16::MAX
                                && cf.position != u16::MAX
                            {
                                let mut had = false;
                                if r.position != l.position + 4 {
                                    had = true;
                                    issues.push(format!(
                                        "{}: tree bookkeeping not at expected offsets — '{}'@{} '{}'@{} (tree::add expects right at left+4=int4 but layout has left as {}-byte content '{}')",
                                        t.name,
                                        l.name,
                                        l.position,
                                        r.name,
                                        r.position,
                                        self.types[l.content as usize].size,
                                        self.types[l.content as usize].name,
                                    ));
                                }
                                if cf.position != l.position + 8 {
                                    had = true;
                                    issues.push(format!(
                                        "{}: tree bookkeeping not at expected offsets — '{}'@{} '{}'@{} (tree::add expects color at left+8 but layout disagrees)",
                                        t.name, l.name, l.position, cf.name, cf.position
                                    ));
                                }
                                if had {
                                    issues.push(format!(
                                        "  content layout: {}",
                                        self.layout_summary(*c)
                                    ));
                                }
                            }
                        }
                    }
                }
                self.validate_layout_by_nr(*c, visited, issues);
            }
            _ => {}
        }
    }

    /// Helper — render a type's full layout as a single line, used
    /// to make validate_layout errors self-contained.  Format:
    /// `Score(size=29,align=8){value:integer@0..8, ...}`.
    fn layout_summary(&self, tp: u16) -> String {
        let t = &self.types[tp as usize];
        let mut out = format!("{}(size={},align={})", t.name, t.size, t.align);
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &t.parts {
            out += "{";
            let mut by_pos: Vec<(u16, usize, &Field)> = fields
                .iter()
                .enumerate()
                .map(|(i, f)| (f.position, i, f))
                .collect();
            by_pos.sort_by_key(|(p, i, _)| (*p, *i));
            for (n, (pos, idx, f)) in by_pos.iter().enumerate() {
                if n > 0 {
                    out += ", ";
                }
                let csz = self.types[f.content as usize].size;
                let cname = &self.types[f.content as usize].name;
                let pos_str = if *pos == u16::MAX {
                    "?".to_string()
                } else {
                    pos.to_string()
                };
                let end = if *pos == u16::MAX || csz == u16::MAX {
                    "?".to_string()
                } else {
                    (*pos + csz).to_string()
                };
                out += &format!("[{}]{}:{}@{}..{}", idx, f.name, cname, pos_str, end);
            }
            out += "}";
        }
        out
    }

    /// Helper — detect overlapping field byte ranges.  Each issue
    /// includes the full layout summary so the user can see the
    /// surrounding context without a separate debug_layout dump.
    fn check_fields_overlap(&self, tp: u16, fields: &[Field], issues: &mut Vec<String>) {
        let t_name = self.types[tp as usize].name.clone();
        let mut by_pos: Vec<(u16, &Field)> = fields
            .iter()
            .filter(|f| f.position != u16::MAX)
            .map(|f| (f.position, f))
            .collect();
        by_pos.sort_by_key(|(p, _)| *p);
        let mut had_issue = false;
        for window in by_pos.windows(2) {
            let (a_pos, a) = window[0];
            let (b_pos, b) = window[1];
            let a_end = a_pos as u32 + self.types[a.content as usize].size as u32;
            if (b_pos as u32) < a_end {
                had_issue = true;
                issues.push(format!(
                    "{}: fields '{}' [@{}..{}) and '{}' [@{}..) overlap",
                    t_name, a.name, a_pos, a_end, b.name, b_pos,
                ));
            }
        }
        if had_issue {
            issues.push(format!("  layout: {}", self.layout_summary(tp)));
        }
    }

    /// Helper — verify every field's [pos, pos+size) fits within the
    /// type's reported size.  Issues include the full layout summary.
    fn check_fields_within_size(&self, tp: u16, fields: &[Field], issues: &mut Vec<String>) {
        let t = &self.types[tp as usize];
        if t.size == u16::MAX {
            return;
        }
        let t_name = t.name.clone();
        let t_size = t.size;
        let mut had_issue = false;
        for f in fields {
            if f.position == u16::MAX {
                had_issue = true;
                issues.push(format!(
                    "{}: field '{}' has no position (u16::MAX)",
                    t_name, f.name
                ));
                continue;
            }
            let csz = self.types[f.content as usize].size;
            if csz == u16::MAX {
                continue;
            }
            let end = f.position as u32 + csz as u32;
            if end > t_size as u32 {
                had_issue = true;
                issues.push(format!(
                    "{}: field '{}' [@{}..{}) extends beyond type size {}",
                    t_name, f.name, f.position, end, t_size
                ));
            }
        }
        if had_issue {
            issues.push(format!("  layout: {}", self.layout_summary(tp)));
        }
    }

    /// Helper for `debug_layout_by_nr` — prints fields sorted by
    /// position with content-type size info.  Reveals gaps + overlaps.
    fn debug_layout_fields(&self, out: &mut String, indent: usize, fields: &[Field]) {
        use std::fmt::Write;
        let pad = " ".repeat(indent);
        // Sort by position so layout is visually contiguous.
        let mut by_pos: Vec<(u16, usize, &Field)> = fields
            .iter()
            .enumerate()
            .map(|(i, f)| (f.position, i, f))
            .collect();
        by_pos.sort_by_key(|(p, i, _)| (*p, *i));
        let mut prev_end: i32 = -1;
        for (pos, idx, f) in by_pos {
            let csz = self.types[f.content as usize].size;
            let cname = &self.types[f.content as usize].name;
            // Show gap if this field's start > prev_end (when both
            // positions are real, not the u16::MAX "not laid out" sentinel).
            if prev_end >= 0
                && pos != u16::MAX
                && (pos as i32) > prev_end
            {
                let _ = writeln!(
                    out,
                    "{pad}     ── gap [{}..{}) ({} bytes) ──",
                    prev_end,
                    pos,
                    pos as i32 - prev_end
                );
            }
            let _ = writeln!(
                out,
                "{pad}field[{idx}] '{}' @{} size={} ({})",
                f.name, pos, csz, cname
            );
            if pos != u16::MAX && csz != u16::MAX {
                prev_end = pos as i32 + csz as i32;
            }
        }
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
        // Dedup early.  Post-Category-D native codegen can call `db.index`
        // for the same content/keys combination twice (once in Phase 1a's
        // bare-io registration, once more during struct-field emission).
        // The field appending below must run exactly ONCE per unique
        // index type — otherwise the content struct accumulates stale
        // `#left_N / #right_N / #color_N` triples that push real user
        // fields to unexpected positions and break tree traversal.
        if let Some(nr) = self.names.get(&name) {
            return *nr;
        }
        // P191: bookkeeping fields must be 4-byte ints to match
        // tree::add's hardcoded RB_LEFT=0 / RB_RIGHT=4 offsets, which
        // use set_i32_raw / get_i32_raw exclusively.  Using 8-byte
        // `integer` here makes alignment-aware packing place them 8
        // bytes apart, corrupting tree::add's writes.
        let int4 = self.int(0, false);
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
                content: int4,
                position: 0,
                default: Content::Long(0),
                other_indexes: Vec::new(),
            });
            fields.push(Field {
                name: format!("#right_{nr}"),
                content: int4,
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
        let num = self.types.len() as u16;
        self.types
            .push(Type::new(&name, Parts::Index(content, key_nrs, left), 4));
        self.names.insert(name, num);
        num
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

    /// 2-byte narrow vector-element field type, used for
    /// `vector<u16>` / `vector<i16>` / `vector<integer limit(...) size(2)>`.
    /// Stored raw (no `+1` shift) so `vector_add`'s raw-byte copy works
    /// unchanged.  `i16::MIN` reserved as the null sentinel.  Struct
    /// fields with `u16` / `i16` continue to use `Parts::Short` (the
    /// legacy `+1` encoding with raw=0 null sentinel).
    pub fn short_raw(&mut self, min: i32, nullable: bool) -> u16 {
        let name = format!("short_raw<{min},{nullable}>");
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::ShortRaw(min, nullable), 2));
            self.names.insert(name, num);
            num
        }
    }

    /// 4-byte integer field type, used for `pub type T = integer size(4);`
    /// subtypes (e.g. `i32`).  Stored raw (no +1 shift); `i32::MIN` reserved
    /// as the null sentinel.  Stack values stay 8-byte i64 — narrowing
    /// happens at the field boundary via OpSetInt4/OpGetInt4.
    pub fn int(&mut self, min: i32, nullable: bool) -> u16 {
        let name = format!("int<{min},{nullable}>");
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::Int(min, nullable), 4));
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
            crate::data::Type::Integer(IntegerSpec {
                min: minimum,
                not_null,
                ..
            }) => {
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
                0 => store.get_int(rec, pos) == i64::MIN,
                6 => store.get_u32_raw(rec, pos) == u32::MAX,
                1 => store.get_long(rec, pos) == i64::MIN,
                2 => store.get_single(rec, pos).is_nan(),
                3 => store.get_float(rec, pos).is_nan(),
                4 => store.get_byte(rec, pos, 0) > 1,
                5 => {
                    let text_nr = store.get_u32_raw(rec, pos);
                    text_nr == 0 || store.get_str(text_nr).is_empty()
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
            store.get_u32_raw(rec, pos) == 0
        } else if let Parts::Byte(from, nullable) = &self.types[known_type as usize].parts {
            let v = store.get_byte(rec, pos, *from);
            *nullable && v == 255
        } else if let Parts::Short(from, nullable) = &self.types[known_type as usize].parts {
            let v = store.get_short(rec, pos, *from);
            *nullable && v == 65535
        } else if let Parts::ShortRaw(from, nullable) = &self.types[known_type as usize].parts {
            let v = store.get_i16_raw(rec, pos, *from);
            *nullable && v == i32::MIN
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

    /// Plan-06 phase 2 — true iff `tp` (a struct or struct-enum-variant)
    /// contains any field that owns out-of-line data: text, references,
    /// vectors, hashes, sorted/index/spacial, or nested struct-enums.
    ///
    /// When this returns false, a struct of type `tp` can be safely
    /// `copy_block`-copied across stores without further deep-copy
    /// work — its bytes are self-contained.  Rebase-eligible.
    ///
    /// When true, the struct contains store-local positions or DbRefs
    /// that need translation/deep-copy across the worker→parent
    /// boundary; the caller falls back to `copy_from_worker`.
    ///
    /// For `Parts::Enum` (untyped enum, just a 1-byte discriminant):
    /// returns false — no owned data.
    /// For `Parts::Struct` / `Parts::EnumValue`: true iff any field
    /// is owned (text or DbRef-shaped).
    /// For other Parts (Base, Byte, Int, Vector, etc.): conservative
    /// true since the type itself is owned.
    #[must_use]
    pub fn has_owned_sub_fields(&self, tp: u16) -> bool {
        if tp == u16::MAX {
            return false;
        }
        match &self.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().any(|f| self.is_field_owned(f.content))
            }
            // Untyped enum (Parts::Enum) is a 1-byte discriminant; no
            // owned data.  But a struct-enum's owning type is the
            // parent Enum, whose variants are EnumValue records — so
            // for ANY enum tp passed here, we have to inspect variants.
            Parts::Enum(variants) => variants
                .iter()
                .any(|(v_tp, _)| self.has_owned_sub_fields(*v_tp)),
            // Bare types: conservative.
            _ => true,
        }
    }

    /// Plan-06 phase 2b refinement — per-variant ownership for
    /// struct-enums (DESIGN.md D11c).
    ///
    /// Given a struct-enum's parent type `enum_tp` and a discriminant
    /// byte read from the value, returns whether THAT specific variant
    /// has owned sub-fields.  Used by the per-element copy dispatch
    /// in parallel_execute_and_collect to take the cheap
    /// `copy_from_worker_unowned` path for variants that don't need
    /// the full deep-copy.
    ///
    /// Example: `Verdict { Pass{score:integer}, Fail{reason:text} }`
    /// - has_owned_sub_fields(Verdict) → true (because Fail has text)
    /// - variant_has_owned_sub_fields(Verdict, 0) → false (Pass: integer only)
    /// - variant_has_owned_sub_fields(Verdict, 1) → true (Fail: text)
    ///
    /// Returns `true` (conservative) if `enum_tp` is not a Parts::Enum
    /// or if `disc` is out of range.  This keeps callers safe when the
    /// type is something else.
    #[must_use]
    pub fn variant_has_owned_sub_fields(&self, enum_tp: u16, disc: u8) -> bool {
        if enum_tp == u16::MAX || (enum_tp as usize) >= self.types.len() {
            return true; // conservative
        }
        if let Parts::Enum(variants) = &self.types[enum_tp as usize].parts {
            // Plan-06 D11c per-variant check: each (v_tp, name) pair
            // is one variant; v_tp is the EnumValue type with that
            // variant's fields.  The discriminant indexes into the
            // variants list (0-based).
            if let Some((v_tp, _)) = variants.get(disc as usize) {
                return self.has_owned_sub_fields(*v_tp);
            }
        }
        // Not a Parts::Enum or disc out-of-range → fall back to
        // whole-type ownership.
        self.has_owned_sub_fields(enum_tp)
    }

    /// Internal helper for `has_owned_sub_fields`: classify a single
    /// field's content type.  Primitive integer/float/bool variants
    /// are unowned; everything that owns out-of-line data is owned.
    fn is_field_owned(&self, content: u16) -> bool {
        if content == u16::MAX {
            return false;
        }
        match &self.types[content as usize].parts {
            // text is tp==5 in the seed types; also catch Parts::Base
            // narrow-text shapes.
            Parts::Base if content == 5 => true,
            Parts::Base => false,
            Parts::Byte(_, _) | Parts::Short(_, _) | Parts::Int(_, _) | Parts::ShortRaw(_, _) => {
                false
            }
            Parts::Enum(variants) => {
                // Untyped enum (1-byte disc) is owned-free; struct-enum
                // (variants with payload) is owned because it carries
                // sub-data.
                variants.iter().any(|(v_tp, _)| self.has_owned_sub_fields(*v_tp))
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().any(|f| self.is_field_owned(f.content))
            }
            // Owning types: vector, hash, sorted, index, spacial, array.
            _ => true,
        }
    }

    /// For EnumValue types, return the parent enum's size (which covers
    /// the largest variant).  For all other types, return their own size.
    /// B2-runtime: unit enum variants may have a smaller type size than
    /// the parent enum needs.  The Database opcode must claim enough
    /// space for `set_default_value` to initialize all fields.
    pub fn enum_parent_size(&self, tp: u16) -> u16 {
        if tp == u16::MAX {
            return 0;
        }
        let own_size = self.types[tp as usize].size;
        // Check if any type in the system is an Enum whose variants include tp.
        // If so, use the Enum's size (which is the max of all variants).
        for t in &self.types {
            if let Parts::Enum(variants) = &t.parts {
                for (v_tp, _) in variants {
                    if *v_tp == tp && t.size > own_size {
                        return t.size;
                    }
                }
            }
        }
        own_size
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

#[cfg(test)]
mod layout_tests {
    use super::{Field, Parts, Stores};
    use crate::keys::Content;

    /// Build a clean two-field struct via the public API.
    fn score_struct(s: &mut Stores) -> u16 {
        let int_c = s.name("integer");
        let txt_c = s.name("text");
        let tp = s.structure("Score", 0);
        s.field(tp, "name", txt_c);
        s.field(tp, "value", int_c);
        tp
    }

    #[test]
    fn validate_all_layouts_clean_after_init_returns_no_issues() {
        let s = Stores::new();
        assert!(s.validate_all_layouts().is_empty());
    }

    #[test]
    fn validate_layout_unknown_type_reports_unknown() {
        let s = Stores::new();
        let issues = s.validate_layout("DoesNotExist");
        assert_eq!(issues, vec!["(unknown type: DoesNotExist)".to_string()]);
    }

    #[test]
    fn validate_layout_clean_struct_after_finish_no_issues() {
        let mut s = Stores::new();
        score_struct(&mut s);
        s.finish();
        let issues = s.validate_layout("Score");
        assert!(
            issues.is_empty(),
            "expected no issues for clean Score, got: {issues:?}"
        );
    }

    #[test]
    fn debug_layout_clean_struct_includes_fields_and_size() {
        let mut s = Stores::new();
        score_struct(&mut s);
        s.finish();
        let dump = s.debug_layout("Score");
        assert!(dump.contains("Score"), "dump missing type name: {dump}");
        assert!(dump.contains("name"), "dump missing 'name' field: {dump}");
        assert!(dump.contains("value"), "dump missing 'value' field: {dump}");
        // Both fields must appear with positions (not u16::MAX).
        assert!(
            !dump.contains("@65535"),
            "dump still has unlaid positions: {dump}"
        );
    }

    #[test]
    fn debug_layout_unknown_type_reports_unknown() {
        let s = Stores::new();
        let dump = s.debug_layout("Nope");
        assert!(
            dump.contains("unknown type: Nope"),
            "expected 'unknown type' in: {dump}"
        );
    }

    #[test]
    fn validate_layout_detects_overlapping_fields() {
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("Bad", 0);
        s.field(tp, "a", int_c);
        s.field(tp, "b", int_c);
        s.finish();
        // Force overlap: rewrite both fields to position 0.
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &mut s.types[tp as usize].parts
        {
            fields[0].position = 0;
            fields[1].position = 0;
        }
        let issues = s.validate_layout("Bad");
        assert!(
            issues.iter().any(|i| i.contains("overlap")),
            "expected overlap issue, got: {issues:?}"
        );
        assert!(
            issues.iter().any(|i| i.contains("layout:")),
            "expected layout summary in issues, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_detects_field_beyond_size() {
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("Tiny", 0);
        s.field(tp, "v", int_c);
        s.finish();
        // Shrink the type's reported size below the field's end.
        s.types[tp as usize].size = 4; // integer is 8 bytes, field at @0
        let issues = s.validate_layout("Tiny");
        assert!(
            issues.iter().any(|i| i.contains("extends beyond type size")),
            "expected beyond-size issue, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_detects_field_without_position() {
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("Unlaid", 0);
        s.field(tp, "v", int_c);
        s.finish();
        // Force the field's position back to u16::MAX (simulates a
        // late-mutation that skipped finish_type).
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &mut s.types[tp as usize].parts
        {
            fields[0].position = u16::MAX;
        }
        let issues = s.validate_layout("Unlaid");
        assert!(
            issues.iter().any(|i| i.contains("no position")),
            "expected no-position issue, got: {issues:?}"
        );
    }

    #[test]
    fn validate_all_layouts_index_bookkeeping_after_p191_fix_no_issues() {
        // P191 fix: `database.index` now appends 4-byte int<0,false>
        // bookkeeping fields (`#left_N`, `#right_N`) so they land
        // contiguously at [pos, pos+4, pos+8] and match tree::add's
        // hardcoded RB_LEFT=0, RB_RIGHT=4, RB_FLAG=8 offsets.  This
        // test guards against regressing back to 8-byte `integer`
        // bookkeeping (which would resurface the corruption).
        let mut s = Stores::new();
        let score_tp = score_struct(&mut s);
        let _index_tp = s.index(score_tp, &[("name".to_string(), true)]);
        s.finish();
        let issues = s.validate_all_layouts();
        assert!(
            issues.is_empty(),
            "expected no layout issues after P191 fix, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_index_left_field_nr_out_of_range() {
        let mut s = Stores::new();
        let score_tp = score_struct(&mut s);
        let index_tp = s.index(score_tp, &[("name".to_string(), true)]);
        s.finish();
        // Force `Parts::Index`'s left_field_nr to point past the end.
        if let Parts::Index(_, _, left) = &mut s.types[index_tp as usize].parts {
            *left = 9999;
        }
        let issues = s.validate_layout("index<Score[name]>");
        assert!(
            issues
                .iter()
                .any(|i| i.contains("Parts::Index left_field_nr=9999 out of range")),
            "expected out-of-range issue, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_index_left_field_nr_points_at_wrong_field() {
        let mut s = Stores::new();
        let score_tp = score_struct(&mut s);
        let index_tp = s.index(score_tp, &[("name".to_string(), true)]);
        s.finish();
        // Force `Parts::Index` to point at field 0 (the user `name`
        // field, not `#left_*`).
        if let Parts::Index(_, _, left) = &mut s.types[index_tp as usize].parts {
            *left = 0;
        }
        let issues = s.validate_layout("index<Score[name]>");
        assert!(
            issues.iter().any(|i| i.contains("expected '#left_*'")),
            "expected wrong-field-name issue, got: {issues:?}"
        );
    }

    #[test]
    fn layout_summary_contains_size_align_and_field_byte_ranges() {
        let mut s = Stores::new();
        let tp = score_struct(&mut s);
        s.finish();
        let line = s.layout_summary(tp);
        assert!(line.starts_with("Score("), "expected name prefix: {line}");
        assert!(line.contains("size="), "expected size= in: {line}");
        assert!(line.contains("align="), "expected align= in: {line}");
        assert!(line.contains("name:text@"), "expected name field: {line}");
        assert!(line.contains("value:integer@"), "expected value field: {line}");
        // The byte-range form is `@start..end`.
        assert!(line.contains(".."), "expected ..end in field range: {line}");
    }

    #[test]
    fn validate_all_layouts_skips_unlaid_types() {
        let mut s = Stores::new();
        // Push a struct with size = u16::MAX (unlaid) — validate_all
        // must skip it (no panic, no false positive).
        let tp = s.structure("Unlaid", 0);
        let int_c = s.name("integer");
        s.field(tp, "v", int_c);
        // Don't call finish() — type stays at size = u16::MAX.
        assert_eq!(s.types[tp as usize].size, u16::MAX);
        let issues = s.validate_all_layouts();
        assert!(
            issues.is_empty(),
            "expected no issues for unlaid type, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_enum_variant_overlap_within_variant_detected() {
        // Within a single enum variant, fields must NOT overlap.  The
        // legitimate "overlap" between *different* variants of the
        // same enum is handled by the parent Parts::Enum arm walking
        // each variant separately, so it never compares across them.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let enum_tp = s.enumerate("E");
        let var_tp = s.structure("E_Variant", 1);
        s.field(var_tp, "a", int_c);
        s.field(var_tp, "b", int_c);
        s.types[var_tp as usize].parents.insert(enum_tp);
        s.enum_value(enum_tp, "Variant", var_tp);
        s.finish();
        // Force the two fields inside the variant to overlap.
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &mut s.types[var_tp as usize].parts
        {
            fields[0].position = 0;
            fields[1].position = 0;
        }
        let issues = s.validate_layout("E_Variant");
        assert!(
            issues.iter().any(|i| i.contains("overlap")),
            "expected within-variant overlap issue, got: {issues:?}"
        );
    }

    #[test]
    fn debug_layout_index_includes_bookkeeping_fields() {
        let mut s = Stores::new();
        score_struct(&mut s);
        let score_tp = s.name("Score");
        s.index(score_tp, &[("name".to_string(), true)]);
        s.finish();
        let dump = s.debug_layout("Score");
        assert!(dump.contains("#left_1"), "missing #left_1 in: {dump}");
        assert!(dump.contains("#right_1"), "missing #right_1 in: {dump}");
        assert!(dump.contains("#color_1"), "missing #color_1 in: {dump}");
    }

    #[test]
    fn validate_layout_default_constructed_field_compiles() {
        // Construct a Field directly via struct literal — guards
        // against future reorderings that would break test setup.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("Manual", 0);
        if let Parts::Struct(f) = &mut s.types[tp as usize].parts {
            f.push(Field {
                name: "x".to_string(),
                content: int_c,
                position: 0,
                default: Content::Long(0),
                other_indexes: Vec::new(),
            });
        }
        s.finish();
        let issues = s.validate_layout("Manual");
        assert!(
            issues.is_empty(),
            "expected no issues for Manual struct, got: {issues:?}"
        );
    }
}
