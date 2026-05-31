// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! IR store schema + Value-free `Type` JSON — @PLAN28 startup-cache Step 2.
//!
//! Two pieces today, both deliberately Value-free:
//!   * [`register_ir_schema`] (rung S1) — registers the compiler's own IR
//!     types as a `Stores` struct-enum schema, the **same representation
//!     `loft --native` emits for user struct-enums** (NATIVE.md § `output_init`).
//!   * [`type_to_json`] / [`type_from_json`] (Step 2 first slice) — a
//!     hand-written `Type` ↔ JSON round-trip.  `Type` is the Value-free half
//!     of the IR, so this exercises the recursive-enum + string-escape +
//!     `crate::json` machinery without the 34-variant `Value` encoder (the
//!     next slice).  Decode reuses loft's own JSON parser — never serde.
//!
//! ## Status — dead code today
//!
//! Nothing in the live compile path calls into this module yet.  The
//! native↔store bridge (S2/S3) and the cache wiring (S4) land in later
//! passes, each its own green PR.  Absolute `def_nr` / `known_type` indices
//! are stored verbatim — this is the whole-bundle image (internally
//! consistent), not the deferred per-library form.
//!
//! ## Why the schema names are prefixed
//!
//! IR type names are prefixed with [`IR_PREFIX`] (`$ir.`) so they can never
//! collide with a user program's `struct Value` / `struct Type` / etc.  The
//! prefix is not a legal loft identifier.
//!
//! ## Two-phase registration (required for recursion)
//!
//! `Value` embeds `Box<Value>` / `Vec<Value>` and `Block`; `Type` embeds
//! `Box<Type>` / `Vec<Type>`.  A field can only reference a type that already
//! has a number, so registration is two-phase: (1) shells — `enumerate()` /
//! `structure()` every IR type; (2) fields + variants that reference those
//! numbers.  S1 lands shells first.

use crate::data::{IntegerSpec, Type};
use crate::database::Stores;
use crate::json::Parsed;
use std::fmt::Write as _;
use std::num::NonZeroU8;

/// Prefix on every IR schema type name, so they cannot collide with a
/// user program's own `struct Value` etc.  Not a legal loft identifier.
pub const IR_PREFIX: &str = "$ir.";

/// The IR enum / struct type names registered as schema shells.  The two
/// recursive enums (`Value`, `Type`) plus the structs they and
/// `Definition` own.  Order is irrelevant for shell registration — every
/// name simply needs a slot before phase-2 field wiring references it.
const IR_ENUMS: &[&str] = &["Value", "Type"];

const IR_STRUCTS: &[&str] = &[
    // Top-level definition graph.
    "Data",
    "Definition",
    "Attribute",
    "Argument",
    "Block",
    "ParForBody",
    "Function",
    "Variable",
    "IntegerSpec",
    "Position",
    "Key",
    // Small payload shapes used by Type/Value variants that don't map to
    // a single scalar (e.g. the `(field-name, ascending)` pairs in
    // `Type::Sorted` / `Index`).
    "SortKey", // (name: text, ascending: boolean)
];

/// Register the IR schema shells into `stores`.  S1: shells only — each
/// IR type gets a `known_type`, no fields/variants wired yet.  Must be
/// called once on a fresh schema; calling twice panics on the
/// duplicate-name guard in `structure()` (by design — double registration
/// is a bug).
///
/// Returns the number of IR types registered, for the collision test.
pub fn register_ir_schema(stores: &mut Stores) -> usize {
    let mut count = 0;
    for name in IR_ENUMS {
        let qualified = format!("{IR_PREFIX}{name}");
        debug_assert!(
            !stores.has_type(&qualified),
            "IR schema enum {qualified} already registered"
        );
        stores.enumerate(&qualified);
        count += 1;
    }
    for name in IR_STRUCTS {
        let qualified = format!("{IR_PREFIX}{name}");
        debug_assert!(
            !stores.has_type(&qualified),
            "IR schema struct {qualified} already registered"
        );
        stores.structure(&qualified, 0);
        count += 1;
    }
    count
}

// ─── Type JSON round-trip (Step 2 first slice — Value-free) ──────────────────
//
// `Type` is the Value-free half of the IR: every variant carries indices
// (`u32` def_nr / `u16` dep entries), `Box<Type>` / `Vec<Type>` recursion, or
// `(String, bool)` / `String` key lists — never a `Value`.  Encoding is a
// tagged object `{ "k": "<variant>", … }`; decode dispatches on the `"k"` tag.
// Indices are stored verbatim (whole-bundle image).

/// JSON-escape `s` into `out` as a quoted string (mirrors the database
/// generator's `write_json_escaped`; kept local for this Value-free slice).
fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Encode a `Vec<u16>` dep list as a JSON array of numbers.
fn write_u16_list(out: &mut String, list: &[u16]) {
    out.push('[');
    for (i, n) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{n}");
    }
    out.push(']');
}

/// Encode a `Vec<(String, bool)>` sort-key list (`Sorted` / `Index`).
fn write_sort_keys(out: &mut String, keys: &[(String, bool)]) {
    out.push('[');
    for (i, (name, asc)) in keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        write_str(out, name);
        let _ = write!(out, ",\"asc\":{asc}}}");
    }
    out.push(']');
}

/// Encode a `Vec<String>` name list (`Spacial` / `Hash`).
fn write_str_list(out: &mut String, names: &[String]) {
    out.push('[');
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_str(out, name);
    }
    out.push(']');
}

/// Encode an `IntegerSpec` inline (`forced` is 0 when `forced_size` is None).
fn write_integer_spec(out: &mut String, spec: &IntegerSpec) {
    let forced = spec.forced_size.map_or(0u8, NonZeroU8::get);
    let _ = write!(
        out,
        "{{\"min\":{},\"max\":{},\"not_null\":{},\"forced\":{}}}",
        spec.min, spec.max, spec.not_null, forced
    );
}

/// Serialise a `Type` to a JSON string.  Value-free; recurses through
/// `Box<Type>` / `Vec<Type>`.
#[must_use]
pub fn type_to_json(ty: &Type) -> String {
    let mut out = String::new();
    write_type(&mut out, ty);
    out
}

fn write_type(out: &mut String, ty: &Type) {
    match ty {
        Type::Unknown(n) => {
            let _ = write!(out, "{{\"k\":\"Unknown\",\"n\":{n}}}");
        }
        Type::Null => out.push_str("{\"k\":\"Null\"}"),
        Type::Void => out.push_str("{\"k\":\"Void\"}"),
        Type::Never => out.push_str("{\"k\":\"Never\"}"),
        Type::Boolean => out.push_str("{\"k\":\"Boolean\"}"),
        Type::Float => out.push_str("{\"k\":\"Float\"}"),
        Type::Single => out.push_str("{\"k\":\"Single\"}"),
        Type::Character => out.push_str("{\"k\":\"Character\"}"),
        Type::Keys => out.push_str("{\"k\":\"Keys\"}"),
        Type::Integer(spec) => {
            out.push_str("{\"k\":\"Integer\",\"spec\":");
            write_integer_spec(out, spec);
            out.push('}');
        }
        Type::Text(dep) => {
            out.push_str("{\"k\":\"Text\",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Enum(n, is_ref, dep) => {
            let _ = write!(out, "{{\"k\":\"Enum\",\"n\":{n},\"ref\":{is_ref},\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Reference(n, dep) => {
            let _ = write!(out, "{{\"k\":\"Reference\",\"n\":{n},\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::RefVar(inner) => {
            out.push_str("{\"k\":\"RefVar\",\"t\":");
            write_type(out, inner);
            out.push('}');
        }
        Type::Vector(inner, dep) => {
            out.push_str("{\"k\":\"Vector\",\"t\":");
            write_type(out, inner);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Routine(n) => {
            let _ = write!(out, "{{\"k\":\"Routine\",\"n\":{n}}}");
        }
        Type::Iterator(step, inner) => {
            out.push_str("{\"k\":\"Iterator\",\"step\":");
            write_type(out, step);
            out.push_str(",\"inner\":");
            write_type(out, inner);
            out.push('}');
        }
        Type::Sorted(n, keys, dep) => {
            let _ = write!(out, "{{\"k\":\"Sorted\",\"n\":{n},\"keys\":");
            write_sort_keys(out, keys);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Index(n, keys, dep) => {
            let _ = write!(out, "{{\"k\":\"Index\",\"n\":{n},\"keys\":");
            write_sort_keys(out, keys);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Spacial(n, names, dep) => {
            let _ = write!(out, "{{\"k\":\"Spacial\",\"n\":{n},\"names\":");
            write_str_list(out, names);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Hash(n, names, dep) => {
            let _ = write!(out, "{{\"k\":\"Hash\",\"n\":{n},\"names\":");
            write_str_list(out, names);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Function(args, result, dep) => {
            out.push_str("{\"k\":\"Function\",\"args\":[");
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_type(out, a);
            }
            out.push_str("],\"result\":");
            write_type(out, result);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Rewritten(inner) => {
            out.push_str("{\"k\":\"Rewritten\",\"t\":");
            write_type(out, inner);
            out.push('}');
        }
        Type::Tuple(elems) => {
            out.push_str("{\"k\":\"Tuple\",\"elems\":[");
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_type(out, e);
            }
            out.push_str("]}");
        }
    }
}

/// Errors from decoding a `Type` out of a `Parsed` tree.
#[derive(Debug, PartialEq, Eq)]
pub enum TypeDecodeError {
    /// Top-level JSON did not parse, or a sub-node had the wrong JSON shape.
    Shape(String),
    /// The `"k"` tag named a variant we don't know.
    UnknownTag(String),
}

/// Parse a JSON string (via `crate::json::parse`) back into a `Type`.
///
/// # Errors
/// Returns [`TypeDecodeError`] when the JSON is malformed, a field is missing
/// or has the wrong shape, or the `"k"` tag is unrecognised.
pub fn type_from_json(src: &str) -> Result<Type, TypeDecodeError> {
    let parsed = crate::json::parse(src)
        .map_err(|e| TypeDecodeError::Shape(format!("json parse: {e:?}")))?;
    type_from_parsed(&parsed)
}

/// Look up a field in a `Parsed::Object` by name.
fn field<'a>(obj: &'a Parsed, name: &str) -> Result<&'a Parsed, TypeDecodeError> {
    if let Parsed::Object(entries) = obj {
        entries
            .iter()
            .find(|(k, _, _)| k == name)
            .map(|(_, _, v)| v)
            .ok_or_else(|| TypeDecodeError::Shape(format!("missing field '{name}'")))
    } else {
        Err(TypeDecodeError::Shape(format!("expected object for '{name}'")))
    }
}

fn as_u32(p: &Parsed) -> Result<u32, TypeDecodeError> {
    if let Parsed::Number(n) = p {
        Ok(*n as u32)
    } else {
        Err(TypeDecodeError::Shape("expected number".into()))
    }
}

/// Read a signed `i32` (e.g. `IntegerSpec.min`, which can be negative —
/// `i32::MIN + 1` for plain-integer templates).  `as_u32` would clamp a
/// negative `f64` to 0, so decode the float directly.
fn as_i32(p: &Parsed) -> Result<i32, TypeDecodeError> {
    if let Parsed::Number(n) = p {
        Ok(*n as i32)
    } else {
        Err(TypeDecodeError::Shape("expected number".into()))
    }
}

fn as_u16(p: &Parsed) -> Result<u16, TypeDecodeError> {
    Ok(as_u32(p)? as u16)
}

fn as_bool(p: &Parsed) -> Result<bool, TypeDecodeError> {
    if let Parsed::Bool(b) = p {
        Ok(*b)
    } else {
        Err(TypeDecodeError::Shape("expected bool".into()))
    }
}

fn as_str(p: &Parsed) -> Result<String, TypeDecodeError> {
    match p {
        Parsed::Str(s) | Parsed::Ident(s) => Ok(s.clone()),
        _ => Err(TypeDecodeError::Shape("expected string".into())),
    }
}

fn dep_list(p: &Parsed) -> Result<Vec<u16>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(as_u16).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn sort_keys(p: &Parsed) -> Result<Vec<(String, bool)>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items
            .iter()
            .map(|it| Ok((as_str(field(it, "name")?)?, as_bool(field(it, "asc")?)?)))
            .collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn str_list(p: &Parsed) -> Result<Vec<String>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(as_str).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn type_list(p: &Parsed) -> Result<Vec<Type>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(type_from_parsed).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn integer_spec(p: &Parsed) -> Result<IntegerSpec, TypeDecodeError> {
    let min = as_i32(field(p, "min")?)?;
    let max = as_u32(field(p, "max")?)?;
    let not_null = as_bool(field(p, "not_null")?)?;
    let forced = as_u32(field(p, "forced")?)? as u8;
    Ok(IntegerSpec {
        min,
        max,
        not_null,
        forced_size: NonZeroU8::new(forced),
    })
}

fn type_from_parsed(p: &Parsed) -> Result<Type, TypeDecodeError> {
    let tag = as_str(field(p, "k")?)?;
    Ok(match tag.as_str() {
        "Unknown" => Type::Unknown(as_u32(field(p, "n")?)?),
        "Null" => Type::Null,
        "Void" => Type::Void,
        "Never" => Type::Never,
        "Boolean" => Type::Boolean,
        "Float" => Type::Float,
        "Single" => Type::Single,
        "Character" => Type::Character,
        "Keys" => Type::Keys,
        "Integer" => Type::Integer(integer_spec(field(p, "spec")?)?),
        "Text" => Type::Text(dep_list(field(p, "dep")?)?),
        "Enum" => Type::Enum(
            as_u32(field(p, "n")?)?,
            as_bool(field(p, "ref")?)?,
            dep_list(field(p, "dep")?)?,
        ),
        "Reference" => Type::Reference(as_u32(field(p, "n")?)?, dep_list(field(p, "dep")?)?),
        "RefVar" => Type::RefVar(Box::new(type_from_parsed(field(p, "t")?)?)),
        "Vector" => Type::Vector(
            Box::new(type_from_parsed(field(p, "t")?)?),
            dep_list(field(p, "dep")?)?,
        ),
        "Routine" => Type::Routine(as_u32(field(p, "n")?)?),
        "Iterator" => Type::Iterator(
            Box::new(type_from_parsed(field(p, "step")?)?),
            Box::new(type_from_parsed(field(p, "inner")?)?),
        ),
        "Sorted" => Type::Sorted(
            as_u32(field(p, "n")?)?,
            sort_keys(field(p, "keys")?)?,
            dep_list(field(p, "dep")?)?,
        ),
        "Index" => Type::Index(
            as_u32(field(p, "n")?)?,
            sort_keys(field(p, "keys")?)?,
            dep_list(field(p, "dep")?)?,
        ),
        "Spacial" => Type::Spacial(
            as_u32(field(p, "n")?)?,
            str_list(field(p, "names")?)?,
            dep_list(field(p, "dep")?)?,
        ),
        "Hash" => Type::Hash(
            as_u32(field(p, "n")?)?,
            str_list(field(p, "names")?)?,
            dep_list(field(p, "dep")?)?,
        ),
        "Function" => Type::Function(
            type_list(field(p, "args")?)?,
            Box::new(type_from_parsed(field(p, "result")?)?),
            dep_list(field(p, "dep")?)?,
        ),
        "Rewritten" => Type::Rewritten(Box::new(type_from_parsed(field(p, "t")?)?)),
        "Tuple" => Type::Tuple(type_list(field(p, "elems")?)?),
        other => return Err(TypeDecodeError::UnknownTag(other.to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shells_register_without_collision() {
        let mut stores = Stores::new();
        let before = stores.types.len();
        let registered = register_ir_schema(&mut stores);
        let after = stores.types.len();

        // Every IR shell got a distinct new slot — no name collided with a
        // base type or with another IR type (a collision would have
        // panicked in structure(), or silently merged in enumerate()).
        assert_eq!(
            after - before,
            registered,
            "every IR shell must register as a new type"
        );
        assert_eq!(registered, IR_ENUMS.len() + IR_STRUCTS.len());

        // Each registered name is present and prefixed (so it cannot clash
        // with a user type).
        for name in IR_ENUMS.iter().chain(IR_STRUCTS.iter()) {
            let qualified = format!("{IR_PREFIX}{name}");
            assert!(
                stores.has_type(&qualified),
                "IR shell {qualified} should be registered"
            );
        }
    }

    #[test]
    fn prefix_is_not_a_legal_identifier() {
        // The prefix must contain a char that can't start/continue a loft
        // identifier, guaranteeing user code can never name a colliding type.
        assert!(IR_PREFIX.contains('$') && IR_PREFIX.contains('.'));
    }

    // ── Type JSON round-trip (Step 2 first slice) ───────────────────────────

    /// Assert `Type → JSON → Type` is lossless for one value.
    fn round_trip(ty: &Type) {
        let json = type_to_json(ty);
        let back =
            type_from_json(&json).unwrap_or_else(|e| panic!("decode {ty:?} from {json}: {e:?}"));
        assert_eq!(*ty, back, "round-trip mismatch via {json}");
    }

    #[test]
    fn type_round_trip_all_variants() {
        // One representative of every Type variant, including the recursive
        // and key-carrying ones.  Value-free by construction.
        let cases = vec![
            Type::Unknown(7),
            Type::Null,
            Type::Void,
            Type::Never,
            Type::Boolean,
            Type::Float,
            Type::Single,
            Type::Character,
            Type::Keys,
            Type::Integer(IntegerSpec::wide()),
            Type::Integer(IntegerSpec::u8()),
            Type::Integer(IntegerSpec::signed32()),
            Type::Text(vec![]),
            Type::Text(vec![1, 2, 3]),
            Type::Enum(4, true, vec![5]),
            Type::Enum(4, false, vec![]),
            Type::Reference(9, vec![0, 1]),
            Type::RefVar(Box::new(Type::Boolean)),
            Type::Vector(Box::new(Type::Text(vec![])), vec![2]),
            Type::Routine(11),
            Type::Iterator(
                Box::new(Type::Integer(IntegerSpec::wide())),
                Box::new(Type::Null),
            ),
            Type::Sorted(
                3,
                vec![("name".into(), true), ("age".into(), false)],
                vec![1],
            ),
            Type::Index(3, vec![("key".into(), true)], vec![]),
            Type::Spacial(3, vec!["x".into(), "y".into()], vec![1]),
            Type::Hash(3, vec!["id".into()], vec![]),
            Type::Function(
                vec![Type::Integer(IntegerSpec::wide()), Type::Text(vec![])],
                Box::new(Type::Boolean),
                vec![0],
            ),
            Type::Rewritten(Box::new(Type::Text(vec![]))),
            Type::Tuple(vec![Type::Integer(IntegerSpec::wide()), Type::Text(vec![])]),
            // Nested recursion: vector<vector<reference>>.
            Type::Vector(
                Box::new(Type::Vector(Box::new(Type::Reference(2, vec![])), vec![])),
                vec![],
            ),
        ];
        for ty in &cases {
            round_trip(ty);
        }
    }

    #[test]
    fn string_fields_escape_round_trip() {
        // Key names with characters that need JSON escaping must survive.
        let ty = Type::Sorted(
            1,
            vec![("a\"b\\c\nd".into(), true), ("tab\there".into(), false)],
            vec![],
        );
        round_trip(&ty);
    }

    #[test]
    fn unknown_tag_is_an_error() {
        let err = type_from_json("{\"k\":\"Bogus\"}");
        assert_eq!(err, Err(TypeDecodeError::UnknownTag("Bogus".into())));
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(matches!(
            type_from_json("{not json"),
            Err(TypeDecodeError::Shape(_))
        ));
    }
}
