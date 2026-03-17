// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Calculate the positions of fields inside a record
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]

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
        "long" => {
            data.set_returned(d_nr, Type::Long);
            data.definitions[d_nr as usize].known_type = 1;
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

pub fn actual_types(data: &mut Data, database: &mut Stores, lexer: &mut Lexer, start_def: u32) {
    // Determine the actual type of structs regarding their use
    for d in start_def..data.definitions() {
        if matches!(data.def_type(d), DefType::Struct) {
            data.definitions[d as usize].returned = Type::Reference(d, Vec::new());
        }
    }
    for d in start_def..data.definitions() {
        match data.def_type(d) {
            DefType::Unknown => {
                lexer.pos_diagnostic(
                    Level::Error,
                    &data.def(d).position,
                    &format!("Error: Undefined type {}", data.def(d).name),
                );
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
            DefType::EnumValue => {
                if data.attributes(d) > 0 {
                    copy_unknown_fields(data, d);
                }
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
                        "Error: Struct '{}' contains itself (directly or indirectly) — use reference<{}> to break the cycle",
                        data.def(d_nr).name,
                        data.def(d_nr).name,
                    ),
                );
            }
        }
    }
    for d_nr in start_def..data.definitions() {
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
        let a_type = data.attr_type(d_nr, a_nr);
        let t_nr = data.type_elm(&a_type);
        let nullable = data.attr_nullable(d_nr, a_nr);
        if t_nr < u32::MAX {
            let tp = match a_type {
                Type::Vector(c_type, _) => {
                    let c_nr = data.type_elm(&c_type);
                    assert_ne!(
                        c_nr,
                        u32::MAX,
                        "Unknown vector {} content type on [{d_nr}]{}.{}",
                        c_type.name(data),
                        data.def(d_nr).name,
                        data.attr_name(d_nr, a_nr)
                    );
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    let tp = database.vector(c_tp);
                    data.check_vector(c_nr, tp, &data.def(d_nr).position.clone());
                    tp
                }
                Type::Integer(minimum, _) => {
                    let s = a_type.size(nullable);
                    if s == 1 {
                        database.byte(minimum, nullable)
                    } else if s == 2 {
                        database.short(minimum, nullable)
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
                Type::Enum(t, _, _) if data.def(t).name == "enumerate" => database.name("byte"),
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
