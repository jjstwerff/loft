// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Type resolution and field layout.
//!
//! After the parser's first pass declares all types, this module resolves
//! forward references, computes field sizes and offsets, and initialises
//! database store schemas.  Called between parser pass 1 and pass 2.
//!
//! Key entry points:
//! - [`actual_types`] — resolve forward type references, detect cycles,
//!   compute field positions via [`crate::calc::calculate_positions`].
//! - [`fill_all`] — allocate database stores for each struct/enum and
//!   write the type schema into `Stores`.
//! - [`complete_definition`] — finalise a single definition's field layout.

use crate::data::{Data, DefType, I32, Type, Value};
use crate::database::Stores;
use crate::diagnostics::Level;
use crate::lexer::Lexer;

/// Set the correct type and initial size in definitions.
/// This will not factor in the space for attributes for records
/// as we still need to analyze the actual use of records.
pub fn complete_definition(_lexer: &mut Lexer, data: &mut Data, d_nr: u32) {
    match data.def(d_nr).name.as_str() {
        "vector" => {
            data.set_returned(d_nr, Type::Vector(Box::new(Type::Unknown(0)), Vec::new()));
            data.definitions[d_nr as usize].known_type = 7;
        }
        "integer" => {
            data.set_returned(d_nr, I32.clone());
            data.definitions[d_nr as usize].known_type = 0;
        }
        "float" => {
            data.set_returned(d_nr, Type::Float);
            data.definitions[d_nr as usize].known_type = 3;
        }
        "single" => {
            data.set_returned(d_nr, Type::Single);
            data.definitions[d_nr as usize].known_type = 2;
        }
        "text" => {
            data.set_returned(d_nr, Type::Text(Vec::new()));
            data.definitions[d_nr as usize].known_type = 5;
        }
        "boolean" => {
            data.set_returned(d_nr, Type::Boolean);
            data.definitions[d_nr as usize].known_type = 4;
        }
        "enumerate" => {
            data.set_returned(d_nr, Type::Enum(0, false, Vec::new()));
        }
        "function" => {
            data.set_returned(d_nr, Type::Routine(d_nr));
        }
        "character" => {
            data.set_returned(d_nr, Type::Character);
            data.definitions[d_nr as usize].known_type = 6;
        }
        "radix" | "hash" | "reference" | "index" => {
            data.set_returned(d_nr, Type::Reference(d_nr, Vec::new()));
        }
        "keys_definition" => {
            data.set_returned(d_nr, Type::Keys);
            data.definitions[d_nr as usize].known_type = 8;
        }
        _ => {}
    }
}

fn copy_unknown_fields(data: &mut Data, d: u32) {
    for nr in 0..data.attributes(d) {
        if let Type::Unknown(was) = data.attr_type(d, nr) {
            data.set_attr_type(d, nr, data.def(was).returned.clone());
        } else if let Type::Vector(content, dep) = &data.attr_type(d, nr)
            && let Type::Unknown(was) = **content
            && was != 0
        {
            let c = Box::new(data.def(was).returned.clone());
            data.set_attr_type(d, nr, Type::Vector(c, dep.clone()));
        }
    }
}

/// Resolve forward type references accumulated during parsing.  When
/// `defer_unknown` is `Some`, every `DefType::Unknown` stub is recorded
/// as `(source, def_nr, position)` in the passed-in vec instead of being
/// emitted as a diagnostic — the caller is then responsible for either
/// patching the stub (via `Data::rewrite_unknown_refs`) or surfacing the
/// final "Undefined type" error later.
///
/// The P173 package-mode driver uses this: cyclic intra-package `use`
/// declarations legitimately produce Unknown stubs for cross-file types
/// that will be resolved by `resolve_deferred_unknowns` after both sides
/// of the cycle have registered their definitions.
pub fn actual_types_deferred(
    data: &mut Data,
    database: &mut Stores,
    lexer: &mut Lexer,
    start_def: u32,
    mut defer_unknown: Option<&mut Vec<(u16, u32, crate::data::Position)>>,
) {
    // Determine the actual type of structs regarding their use
    for d in start_def..data.definitions() {
        if matches!(data.def_type(d), DefType::Struct) {
            data.definitions[d as usize].returned = Type::Reference(d, Vec::new());
        }
    }
    for d in start_def..data.definitions() {
        match data.def_type(d) {
            DefType::Unknown => {
                if let Some(buf) = defer_unknown.as_deref_mut() {
                    let def = data.def(d);
                    buf.push((def.source, d, def.position.clone()));
                    continue;
                }
                let name = &data.def(d).name;
                let msg = if name == "string" {
                    "Undefined type 'string' — did you mean 'text'?".to_string()
                } else {
                    format!("Undefined type {name}")
                };
                lexer.pos_diagnostic(Level::Error, &data.def(d).position, &msg);
            }
            DefType::Function => {
                copy_unknown_fields(data, d);
                if let Type::Unknown(was) = data.def(d).returned {
                    data.set_returned(d, data.def(was).returned.clone());
                }
            }
            DefType::Struct => {
                copy_unknown_fields(data, d);
            }
            DefType::Enum => {
                let e_nr = database.enumerate(&data.def(d).name.clone());
                for a in 0..data.attributes(d) {
                    database.value(e_nr, &data.attr_name(d, a), u16::MAX);
                    data.set_attr_value(d, a, Value::Enum(a as u8 + 1, e_nr));
                }
                data.definitions[d as usize].known_type = e_nr;
            }
            DefType::EnumValue if data.attributes(d) > 0 => {
                copy_unknown_fields(data, d);
            }
            _ => {}
        }
    }
}

pub fn fill_all(data: &mut Data, database: &mut Stores, lexer: &mut Lexer, start_def: u32) {
    // Detect type cycles before computing sizes.
    for d_nr in start_def..data.definitions() {
        if matches!(data.def_type(d_nr), DefType::Struct) {
            let mut visiting = std::collections::HashSet::new();
            if has_value_cycle(data, d_nr, &mut visiting) {
                lexer.pos_diagnostic(
                    Level::Error,
                    &data.def(d_nr).position,
                    &format!(
                        "Struct '{}' contains itself (directly or indirectly) — use reference<{}> to break the cycle",
                        data.def(d_nr).name,
                        data.def(d_nr).name,
                    ),
                );
            }
        }
    }
    // reject hash-value structs that have a field named `key`.
    // `key` is a reserved pseudo-field for hash iteration (`for kv in h { kv.key }`).
    for d_nr in start_def..data.definitions() {
        if !matches!(data.def_type(d_nr), DefType::Struct) {
            continue;
        }
        for a_nr in 0..data.attributes(d_nr) {
            if let Type::Hash(c_nr, _, _) = data.attr_type(d_nr, a_nr)
                && data.attr(c_nr, "key") != usize::MAX
            {
                lexer.pos_diagnostic(
                    Level::Error,
                    &data.def(c_nr).position,
                    &format!(
                        "Struct '{}' has a field named 'key' which is reserved for hash iteration — rename the field",
                        data.def(c_nr).name,
                    ),
                );
            }
        }
    }
    // Start from 0 (not start_def) so struct-enum variants defined in earlier
    // default library files are processed when later files trigger fill_all.
    // The has_type guard prevents double-processing.  Fixes S14 (PROBLEMS #80).
    // B2-runtime (2026-04-13): Before laying out records, retroactively
    // add a discriminant "enum" field to every unit variant of a mixed
    // struct-enum.  `parse_enum_values` only adds this field inside the
    // `has_token("{")` branch (struct variants), so sibling unit variants
    // have 0 attributes and would produce a size-0 structure — runtime
    // `OpDatabase(db_tp=…)` then panics `Incomplete record` in
    // `Store::claim(size=0)`.  Check the parent's `returned` (set to
    // `Type::Enum(_, true, _)` when ANY variant has braces) rather than
    // the unit-variant child's (which stays `Type::Enum(_, false, _)`).
    let enumerate_d_nr = data.def_nr("enumerate");
    if enumerate_d_nr != u32::MAX {
        for d_nr in 0..data.definitions() {
            if matches!(data.def_type(d_nr), DefType::EnumValue) && data.attributes(d_nr) == 0 {
                let parent = data.def(d_nr).parent;
                if parent != u32::MAX && matches!(data.def(parent).returned, Type::Enum(_, true, _))
                {
                    let discriminant = {
                        let mut v: u8 = 0;
                        for (a_nr, a) in data.def(parent).attributes.iter().enumerate() {
                            if a.name == data.def(d_nr).name {
                                v = a_nr as u8 + 1;
                                break;
                            }
                        }
                        v
                    };
                    data.add_attribute(
                        lexer,
                        d_nr,
                        "enum",
                        Type::Enum(enumerate_d_nr, false, Vec::new()),
                    );
                    let attr_nr = data.def(d_nr).attr_names["enum"];
                    data.set_attr_value(d_nr, attr_nr, Value::Enum(discriminant, u16::MAX));
                }
            }
        }
    }
    // QUALITY B5 fix: register `main_vector<T>` wrapper structs for every
    // `vector<T>` field found on a struct or enum-value.  Parser paths
    // that assign or construct a `vector<T>` already call
    // `data.vector_def(...)`, but **struct-enum variant fields** (e.g.
    // `Node { kids: vector<Tree> }` inside `enum Tree`) go through
    // `parse_enum_values` / `fill_all` without ever hitting a vector
    // assignment site.  Without the wrapper, `gen_set_first_vector_null`'s
    // `data.name_type("main_vector<Tree>")` lookup returns `u16::MAX`
    // and the interpreter emits `OpDatabase(var, db_tp=u16::MAX)` that
    // panics in `Store::claim` as "Incomplete record".  Register the
    // wrappers here, BEFORE the main `fill_database` loop, so the loop
    // then picks them up and assigns a real `known_type`.
    let mut pending: Vec<Type> = Vec::new();
    for d_nr in 0..data.definitions() {
        if !(matches!(data.def_type(d_nr), DefType::Struct)
            || matches!(data.def_type(d_nr), DefType::EnumValue))
        {
            continue;
        }
        for a_nr in 0..data.attributes(d_nr) {
            if let Type::Vector(content, _) = data.attr_type(d_nr, a_nr) {
                let content_tp = *content;
                let wrapper_name = format!("main_vector<{}>", content_tp.name(data));
                if data.def_nr(&wrapper_name) == u32::MAX {
                    pending.push(content_tp);
                }
            }
        }
    }
    for tp in pending {
        data.vector_def(lexer, &tp);
    }
    for d_nr in 0..data.definitions() {
        if ((matches!(data.def_type(d_nr), DefType::EnumValue) && data.attributes(d_nr) > 0)
            || matches!(data.def_type(d_nr), DefType::Struct))
            && !database.has_type(&data.def(d_nr).name)
        {
            fill_database(data, database, d_nr);
        }
    }
}

/// Check if struct `d_nr` contains itself as a value type (not reference) field,
/// directly or through other structs.
fn has_value_cycle(data: &Data, d_nr: u32, visiting: &mut std::collections::HashSet<u32>) -> bool {
    if !visiting.insert(d_nr) {
        return true; // Already visiting this type — cycle found.
    }
    for a_nr in 0..data.attributes(d_nr) {
        let a_type = data.attr_type(d_nr, a_nr);
        // Only recurse into value-typed struct fields (Reference fields are pointers,
        // not inline — they don't cause infinite-size cycles).
        if let Type::Reference(child_nr, _) = &a_type
            && data.def_type(*child_nr) == DefType::Struct
            && has_value_cycle(data, *child_nr, visiting)
        {
            visiting.remove(&d_nr);
            return true;
        }
    }
    visiting.remove(&d_nr);
    false
}

fn fill_database(data: &mut Data, database: &mut Stores, d_nr: u32) {
    if data.def(d_nr).name == "Unknown(0)" {
        return;
    }
    let mut enum_value = 0;
    if let Type::Enum(nr, true, _) = data.def(d_nr).returned {
        for (a_nr, a) in data.def(nr).attributes.iter().enumerate() {
            if a.name == data.def(d_nr).name {
                enum_value = a_nr as i32 + 1;
                break;
            }
        }
    }
    let s_type = database.structure(&data.def(d_nr).name, enum_value);
    data.definitions[d_nr as usize].known_type = s_type;
    if data.def_type(d_nr) == DefType::EnumValue {
        let e_tp = data.def(d_nr).parent;
        let enum_tp = data.def(e_tp).known_type;
        database.enum_value(enum_tp, &data.def(d_nr).name, data.def(d_nr).known_type);
    }
    for a_nr in 0..data.attributes(d_nr) {
        // Computed fields are not stored — skip them in the database layout.
        if data.def(d_nr).attributes[a_nr].constant {
            continue;
        }
        let a_type = data.attr_type(d_nr, a_nr);
        let t_nr = data.type_elm(&a_type);
        let nullable = data.attr_nullable(d_nr, a_nr);
        if t_nr < u32::MAX {
            let tp = match a_type {
                Type::Vector(c_type, _) => {
                    let c_nr = data.type_elm(&c_type);
                    // P156: unresolved vector content — parser already emitted
                    // a diagnostic (constant-shadow, undefined type, etc.).
                    // Skip this attribute rather than panicking so the user
                    // sees the proper error instead of an interpreter crash.
                    if c_nr == u32::MAX {
                        continue;
                    }
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    let tp = database.vector(c_tp);
                    data.check_vector(c_nr, tp, &data.def(d_nr).position.clone());
                    tp
                }
                Type::Integer(minimum, _, not_null) => {
                    let field_nullable = nullable && !not_null;
                    // Post-2c: if the field's alias has a forced size(N)
                    // annotation, prefer it over the limit()-based heuristic.
                    // The alias def_nr was captured in parse_field because
                    // Type::Integer collapses alias names.
                    let alias = data.def(d_nr).attributes[a_nr].alias_d_nr;
                    let s = data
                        .forced_size(alias)
                        .unwrap_or_else(|| a_type.size(field_nullable));
                    if s == 1 {
                        database.byte(minimum, field_nullable)
                    } else if s == 2 {
                        database.short(minimum, field_nullable)
                    } else if s == 4 {
                        database.int(minimum, field_nullable)
                    } else {
                        database.name("integer")
                    }
                }
                Type::Hash(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    set_mutable(data, c_nr, &key_fields);
                    database.hash(c_tp, &key_fields)
                }
                Type::Index(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    set_mutable_directed(data, c_nr, &key_fields);
                    database.index(c_tp, &key_fields)
                }
                Type::Sorted(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    set_mutable_directed(data, c_nr, &key_fields);
                    database.sorted(c_tp, &key_fields)
                }
                Type::Spacial(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    set_mutable(data, c_nr, &key_fields);
                    database.spacial(c_tp, &key_fields)
                }
                Type::Enum(t, _, _) if data.def(t).name == "enumerate" => database.byte(0, false),
                _ => data.def(t_nr).known_type,
            };
            database.field(s_type, &data.attr_name(d_nr, a_nr), tp);
        }
    }
}

fn set_mutable(data: &mut Data, on_d: u32, fields: &[String]) {
    for f in fields {
        let a_nr = data.attr(on_d, f);
        data.definitions[on_d as usize].attributes[a_nr].mutable = false;
    }
}

fn set_mutable_directed(data: &mut Data, on_d: u32, fields: &[(String, bool)]) {
    for f in fields {
        let a_nr = data.attr(on_d, &f.0);
        data.definitions[on_d as usize].attributes[a_nr].mutable = false;
    }
}
