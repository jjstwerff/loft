// Copyright (c) 2022 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Hold all definitions
//! Those are the combinations of types, records, and routines.
//! Many definitions can hold fields of their own, a routine
//! has parameters that behave very similarly to fields.

// These structures are rather inefficient right now, but they are the basis
// for a far more efficient database design later.
#![allow(dead_code)]

use crate::diagnostics::{Diagnostics, Level, diagnostic_format};
use crate::keys::Key;
use crate::lexer::Lexer;

// Re-export Position so external consumers (tests, integrations) can
// construct / pattern-match positions without depending on the private
// `lexer` module.
pub use crate::lexer::Position;
use crate::variables::Function;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{Debug, Display, Formatter};
use std::io::{Result, Write};

static OPERATORS: &[&str] = &[
    "OpAdd", "OpMin", "OpMul", "OpDiv", "OpRem", "OpPow", "OpNot", "OpBitNot", "OpLand", "OpLor",
    "OpEor", "OpSLeft", "OpSRight", "OpEq", "OpNe", "OpLt", "OpLe", "OpGt", "OpGe", "OpAppend",
    "OpConv", "OpCast",
];

pub static I32: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32, false);

/// Full-width integer (post-2c, 8 bytes, i64 range up to u32::MAX bound).
/// Produced by the parser when it sees `long`, `integer limit(..., > i32::MAX)`,
/// or an integer literal whose magnitude exceeds i32::MAX.  At rest: i64.
///
/// Phase 2c round 10c — replaces the former `Type::Long` variant.  The `max`
/// field can't hold full i64::MAX (it's u32), so u32::MAX is used as a
/// "wide" sentinel; all downstream code just observes "max - min >= 256"
/// and picks 8-byte storage.
pub static I64: Type = Type::Integer(i32::MIN + 1, u32::MAX, false);

#[derive(Debug, PartialEq, Clone)]
pub struct Block {
    pub name: &'static str,
    pub operators: Vec<Value>,
    pub result: Type,
    pub scope: u16,
    /// Bytes to pre-claim for small variables (≤ 8 B) at block entry via `OpReserveFrame`.
    /// Computed by `assign_slots`; 0 until then.
    pub var_size: u16,
}

/// A value that can be assigned to attributes on a definition of instance
#[derive(Debug, PartialEq, Clone)]
pub enum Value {
    Null,
    /// Line number inside the source file
    Line(u32),
    Int(i32),
    /// Enum value and database type
    Enum(u8, u16),
    Boolean(bool),
    /// A range
    Float(f64),
    Long(i64),
    Single(f32),
    Text(String),
    /// Call an outside routine with values.
    Call(u32, Vec<Value>),
    /// Call a function through a runtime function reference stored in a local variable.
    CallRef(u16, Vec<Value>),
    /// Call a closure function that allows access to the original stack
    // CCall(Box<Value>, Vec<Value>),
    /// Block with steps and last variable claimed before it.
    Block(Box<Block>),
    /// A block that will be inserted in the outer block and thus not form its own scope.
    /// A block that will be inserted in the outer block and thus not form its own scope.
    Insert(Vec<Value>),
    /// Read variable or parameter from stack (nr relative to current function start).
    Var(u16),
    /// Set a variable with an expressions
    Set(u16, Box<Value>),
    // / Read a variable from the closure stack instead of the current function
    // CVar(u32),
    // / Set a closure variable outside the current function
    // CSet(u32, Box<Value>),
    /// Return from a routine with optionally a Value
    Return(Box<Value>),
    /// Break out of the n-th loop
    Break(u16),
    /// Break out of the n-th loop with a value
    BreakWith(u16, Box<Value>),
    /// Continue the n-th loop
    Continue(u16),
    /// Conditional statement
    If(Box<Value>, Box<Value>, Box<Value>),
    /// Loop through the block till Break is encountered
    Loop(Box<Block>),
    // / Closure function value with a def-nr and
    // Closure(u32, u32),
    /// Drop the returned value of a call
    Drop(Box<Value>),
    /// An iterator (name, create, next, `extra_init`)
    /// `extra_init` is `Value::Null` for non-text loops, or `v_set(index_var`, 0) for text loops.
    Iter(u16, Box<Value>, Box<Value>, Box<Value>),
    /// Key structure
    Keys(Vec<Key>),
    /// T1.2: Tuple literal — elements are evaluated left-to-right onto contiguous stack slots.
    Tuple(Vec<Value>),
    // T1.4: Read element idx of tuple variable var_nr.
    TupleGet(u16, u16),
    // T1.4: Write value to element idx of tuple variable var_nr.
    TuplePut(u16, u16, Box<Value>),
    // CO1.3c: Yield a value from a generator function.
    Yield(Box<Value>),
    // Construct a 16-byte fn-ref on the stack: push d_nr (4B via OpConstInt)
    // then push the closure DbRef (12B via OpVarRef of clos_var_nr). No new opcode.
    FnRef(i32, u16, Box<Type>),
    /// Parallel { arm1; arm2; } — each arm runs concurrently.
    Parallel(Vec<Value>),
}

#[allow(dead_code)]
impl Value {
    #[must_use]
    pub fn str(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    #[must_use]
    pub fn is_op(&self, op: u32) -> bool {
        if let Value::Call(func, _) = self {
            return *func == op;
        }
        false
    }
}

#[must_use]
pub fn to_default(tp: &Type, data: &Data) -> Value {
    match tp {
        Type::Boolean => Value::Boolean(false),
        Type::Enum(tp, _, _) => Value::Enum(0, data.def(*tp).known_type),
        Type::Integer(_, _, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Spacial(_, _, _) => Value::Int(0),
        Type::Single => Value::Single(0.0),
        Type::Float => Value::Float(0.0),
        Type::Text(_) => Value::Text(String::new()),
        _ => Value::Null,
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
/// Static type of a parsed expression or variable.
///
/// Several variants carry a `Vec<u16>` **dependency list** (`dep`):
/// - **Empty** → the value is *owned* — freed by `OpFreeRef` at scope exit.
/// - **Non-empty** → the value *borrows* from the parameters listed by
///   attribute index — NOT freed (the caller owns the store).
///
/// This governs the freeing logic in [`crate::scopes`].  See also
/// [`Function::depend`](crate::variables::Function) which adds entries.
pub enum Type {
    /// The type of this parse result is unknown, possibly linked to a yet unknown type (if != 0).
    Unknown(u32),
    /// The type of this result is specifically undefined.
    Null,
    /// Result of a function without return type.
    Void,
    /// Divergent expression (return/break/continue) — compatible with any type.
    Never,
    /// The given definition might hold restrictions on this number.
    /// (minimum, maximum, `not_null`).
    Integer(i32, u32, bool),
    /// A store with the given base record type. (nullable)
    Boolean,
    Float,
    Single,
    Character,
    /// A text with the linked variables.
    Text(Vec<u16>),
    /// Description of the possible keys on a structure (hash, index, spacial, sorted)
    Keys,
    /// An enum value. With definition with enum type itself. With value true it is a reference.
    Enum(u32, bool, Vec<u16>),
    /// A readonly reference to a record instance in a store.
    Reference(u32, Vec<u16>),
    /// A reference to a variable on stack.
    RefVar(Box<Type>),
    /// A dynamic vector of a specific type
    Vector(Box<Type>, Vec<u16>),
    /// A dynamic routine, from a routine definition without code.
    /// The actual code is a routine with this routine as a parent or just a Block for a lambda function.
    Routine(u32),
    /// Iterator with a certain result, the first type is the result per step.
    /// The second is the internal iterator value or `Type::Null` for structure iterator: `(i32,i32)`
    Iterator(Box<Type>, Box<Type>),
    /// An ordered vector on a record, second is the key [field name, ascending]
    Sorted(u32, Vec<(String, bool)>, Vec<u16>),
    /// An index towards other records. The key is [field name, ascending]
    Index(u32, Vec<(String, bool)>, Vec<u16>),
    /// An index towards other records. The second is [field name]
    Spacial(u32, Vec<String>, Vec<u16>),
    /// A hash table towards other records. The second is the hash function per [field name].
    Hash(u32, Vec<String>, Vec<u16>),
    /// A function reference allowing for closures. Argument types, result, and deps.
    /// The dep list tracks ownership of the closure record embedded in the fn-ref slot.
    Function(Vec<Type>, Box<Type>, Vec<u16>),
    /// A rewritten type into append statements (mostly Text or structures)
    Rewritten(Box<Type>),
    /// T1.1: stack-allocated fixed-arity compound type, e.g. `(integer, text)`.
    Tuple(Vec<Type>),
}

impl Type {
    /// Returns the dep list if this is a heap-allocated, store-backed type
    /// (Reference, Vector, or struct-enum with is_ref=true).
    /// Use this instead of manual pattern matches on Reference/Vector/Enum
    /// to avoid forgetting the Enum arm.
    #[must_use]
    pub fn heap_dep(&self) -> Option<&Vec<u16>> {
        match self {
            Type::Reference(_, dep) | Type::Vector(_, dep) | Type::Enum(_, true, dep) => Some(dep),
            _ => None,
        }
    }

    /// True if this type owns a heap store (heap_dep is Some and dep is empty).
    #[must_use]
    pub fn is_heap_owned(&self) -> bool {
        self.heap_dep().is_some_and(Vec::is_empty)
    }

    /// The definition number for struct-like heap types (Reference or struct-enum).
    #[must_use]
    pub fn heap_def_nr(&self) -> Option<u32> {
        match self {
            Type::Reference(d, _) | Type::Enum(d, true, _) => Some(*d),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_unknown(&self) -> bool {
        if let Type::Vector(tp, _) = self {
            return tp.is_unknown();
        }
        matches!(self, Type::Unknown(_)) || matches!(self, Type::Reference(0, _))
    }

    /**
    Return the same type but with an additional variable in the dependency list.
    # Panics
    When this extra variable doesn't exist.
    */
    #[must_use]
    pub fn depending(&self, on: u16) -> Type {
        assert_ne!(on, u16::MAX, "Unknown depended on variable");
        let mut v = vec![on];
        match self {
            Type::Text(dep) => {
                if !v.contains(&on) {
                    v.append(&mut dep.clone());
                }
                Type::Text(v)
            }
            Type::Reference(t, dep) => {
                if !v.contains(&on) {
                    v.append(&mut dep.clone());
                }
                Type::Reference(*t, v)
            }
            Type::Enum(t, is_ref, dep) => {
                if !v.contains(&on) {
                    v.append(&mut dep.clone());
                }
                Type::Enum(*t, *is_ref, v)
            }
            Type::Index(t, keys, dep) => {
                if !v.contains(&on) {
                    v.append(&mut dep.clone());
                    v.append(&mut dep.clone());
                }
                Type::Index(*t, keys.clone(), v)
            }
            Type::Spacial(t, keys, dep) => {
                if !v.contains(&on) {
                    v.append(&mut dep.clone());
                }
                Type::Spacial(*t, keys.clone(), v)
            }
            Type::Hash(t, keys, dep) => {
                if !v.contains(&on) {
                    v.append(&mut dep.clone());
                }
                Type::Hash(*t, keys.clone(), v)
            }
            Type::Sorted(t, keys, dep) => {
                if !v.contains(&on) {
                    v.append(&mut dep.clone());
                }
                Type::Sorted(*t, keys.clone(), v)
            }
            Type::Vector(t, dep) => {
                if !v.contains(&on) {
                    v.append(&mut dep.clone());
                }
                Type::Vector(Box::new(*t.clone()), v)
            }
            Type::Function(params, ret, dep) => {
                if !v.contains(&on) {
                    v.append(&mut dep.clone());
                }
                Type::Function(params.clone(), ret.clone(), v)
            }
            Type::RefVar(tp) => Type::RefVar(Box::new(tp.depending(on))),
            _ => self.clone(),
        }
    }

    #[must_use]
    pub fn depend(&self) -> Vec<u16> {
        let mut v = Vec::new();
        match self {
            Type::Text(dep)
            | Type::Reference(_, dep)
            | Type::Index(_, _, dep)
            | Type::Spacial(_, _, dep)
            | Type::Hash(_, _, dep)
            | Type::Sorted(_, _, dep)
            | Type::Enum(_, _, dep)
            | Type::Vector(_, dep)
            | Type::Function(_, _, dep) => v.append(&mut dep.clone()),
            Type::RefVar(tp) => return tp.depend(),
            _ => {}
        }
        v
    }

    #[must_use]
    pub fn content(&self) -> Type {
        match self {
            Type::Index(tp, _, dep)
            | Type::Spacial(tp, _, dep)
            | Type::Hash(tp, _, dep)
            | Type::Sorted(tp, _, dep) => Type::Reference(*tp, dep.clone()),
            Type::Vector(tp, _) => *tp.clone(),
            Type::RefVar(tp) => tp.content(),
            _ => Type::Unknown(0),
        }
    }

    #[must_use]
    pub fn is_same(&self, other: &Type) -> bool {
        self == other
            || (matches!(self, Type::Enum(_, _, _)) && matches!(other, Type::Enum(_, _, _)))
            || (matches!(self, Type::Reference(_, _)) && matches!(other, Type::Reference(_, _)))
            || (matches!(self, Type::Vector(_, _)) && matches!(other, Type::Vector(_, _)))
            || (matches!(self, Type::Integer(_, _, _)) && matches!(other, Type::Integer(_, _, _)))
            || (matches!(self, Type::Text(_)) && matches!(other, Type::Text(_)))
    }

    #[must_use]
    pub fn is_equal(&self, other: &Type) -> bool {
        match (self, other) {
            (Type::RefVar(s), Type::RefVar(o)) => return s.is_equal(o),
            (Type::Enum(s, s_tp, _), Type::Enum(o, o_tp, _)) => return *s == *o && *s_tp == *o_tp,
            (Type::Reference(r, _), Type::Reference(o, _)) => return r == o,
            (Type::Vector(r, _), Type::Vector(o, _)) => return r.is_equal(o),
            (Type::Hash(r, rf, _), Type::Hash(o, of, _))
            | (Type::Spacial(r, rf, _), Type::Spacial(o, of, _)) => return r == o && rf == of,
            (Type::Sorted(r, rf, _), Type::Sorted(o, of, _))
            | (Type::Index(r, rf, _), Type::Index(o, of, _)) => return r == o && rf == of,
            (Type::Function(sp, sr, _), Type::Function(op, or, _)) => {
                return sp.len() == op.len()
                    && sp.iter().zip(op.iter()).all(|(a, b)| a.is_equal(b))
                    && sr.is_equal(or);
            }
            // T1.7: tuple equality ignores `not_null` on Integer elements (runtime type is same).
            (Type::Tuple(se), Type::Tuple(oe)) => {
                return se.len() == oe.len()
                    && se.iter().zip(oe.iter()).all(|(a, b)| a.is_equal(b));
            }
            _ => {}
        }
        self == other
            || (matches!(self, Type::Integer(_, _, _)) && matches!(other, Type::Integer(_, _, _)))
            || (matches!(self, Type::Text(_)) && matches!(other, Type::Text(_)))
    }

    #[must_use]
    pub fn size(&self, nullable: bool) -> u8 {
        if let Type::Integer(min, max, _) = self {
            let c_min = i64::from(*min);
            let c_max = i64::from(*max);
            if c_max - c_min < 256 || (nullable && c_max - c_min == 256) {
                1
            } else if c_max - c_min < 65536 || (nullable && c_max - c_min == 65536) {
                2
            } else {
                // Phase 2c: integer is stored as 8-byte i64 by default.
                8
            }
        } else {
            0
        }
    }

    #[must_use]
    pub fn name(&self, data: &Data) -> String {
        match self {
            Type::Rewritten(tp) => tp.name(data),
            Type::RefVar(tp) => format!("&{}", tp.name(data)),
            Type::Enum(t, _, _) | Type::Reference(t, _) => data.def(*t).name.clone(),
            Type::Text(_) => "text".to_string(),
            Type::Vector(tp, _) if matches!(tp as &Type, Type::Unknown(_)) => "vector".to_string(),
            Type::Vector(tp, _) => format!("vector<{}>", tp.name(data)),
            Type::Sorted(tp, key, _) => {
                format!("sorted<{},{key:?}>", data.def(*tp).name)
            }
            Type::Hash(tp, key, _) => format!("hash<{},{key:?}>", data.def(*tp).name),
            Type::Index(tp, key, _) => format!("index<{},{key:?}>", data.def(*tp).name),
            Type::Spacial(tp, key, _) => {
                format!("spacial<{},{key:?}>", data.def(*tp).name)
            }
            Type::Routine(tp) => format!("fn {}[{tp}]", data.def(*tp).name),
            _ => self.to_string(),
        }
    }

    #[must_use]
    pub fn show(&self, data: &Data, vars: &Function) -> String {
        match self {
            Type::RefVar(tp) => format!("&{}", tp.show(data, vars)),
            Type::Enum(t, false, _) => data.def(*t).name.clone(),
            Type::Reference(t, dep) | Type::Enum(t, true, dep) => {
                format!("ref({}){}", data.def(*t).name, Self::dep_var(dep, vars))
            }
            Type::Vector(tp, dep) if matches!(tp as &Type, Type::Unknown(_)) => {
                format!("vector{}", Self::dep_var(dep, vars))
            }
            Type::Vector(tp, dep) => format!(
                "vector<{}>{}",
                tp.show(data, vars),
                Self::dep_var(dep, vars)
            ),
            Type::Sorted(tp, key, dep) => {
                format!(
                    "sorted<{},{key:?}>{}",
                    data.def(*tp).name,
                    Self::dep_var(dep, vars)
                )
            }
            Type::Hash(tp, key, dep) => format!(
                "hash<{},{key:?}>{}",
                data.def(*tp).name,
                Self::dep_var(dep, vars)
            ),
            Type::Index(tp, key, dep) => format!(
                "index<{},{key:?}>{}",
                data.def(*tp).name,
                Self::dep_var(dep, vars)
            ),
            Type::Spacial(tp, key, dep) => {
                format!(
                    "spacial<{},{key:?}>{}",
                    data.def(*tp).name,
                    Self::dep_var(dep, vars)
                )
            }
            Type::Routine(tp) => format!("fn {}[{tp}]", data.def(*tp).name),
            Type::Text(dep) if dep.is_empty() => "text".to_string(),
            Type::Text(dep) => format!("text{}", Self::dep_var(dep, vars)),
            _ => self.to_string(),
        }
    }

    fn dep_var(dep: &Vec<u16>, vars: &Function) -> String {
        let mut ls = BTreeSet::new();
        for d in dep {
            ls.insert(vars.name(*d).to_string());
        }
        let mut res = Vec::new();
        for v in ls {
            res.push(v);
        }
        if res.is_empty() {
            String::new()
        } else {
            format!("{res:?}")
        }
    }

    #[must_use]
    pub fn argument(&self, data: &Data, d_nr: u32) -> String {
        match self {
            Type::Reference(t, link) if link.is_empty() => data.def(*t).name.clone(),
            Type::Reference(t, link) => {
                format!("{}{:?}", data.def(*t).name, Self::dep_att(data, d_nr, link))
            }
            Type::Text(dep) if dep.is_empty() => "text".to_string(),
            Type::Text(dep) => format!("text{:?}", Self::dep_att(data, d_nr, dep)),
            _ => {
                let d = data.def(d_nr);
                self.show(data, &Function::new(&d.name, &d.position.file))
            }
        }
    }

    fn dep_att(data: &Data, d_nr: u32, dep: &Vec<u16>) -> Vec<String> {
        let mut ls = BTreeSet::new();
        for d in dep {
            ls.insert(data.def(d_nr).attributes[*d as usize].name.clone());
        }
        let mut res = Vec::new();
        for v in ls {
            res.push(v);
        }
        res
    }
}

// ── T1.1 — Tuple element layout helpers ─────────────────────────────────────

/// Stack width in bytes of a single element type.
/// Uses the same sizing as `variables::size(tp, &Context::Argument)`.
#[must_use]
pub fn element_size(t: &Type) -> usize {
    match t {
        Type::Boolean | Type::Enum(_, false, _) => 1,
        Type::Single | Type::Function(_, _, _) | Type::Character => 4,
        Type::Integer(_, _, _) | Type::Float => 8,
        Type::Text(_) => std::mem::size_of::<crate::keys::Str>(),
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Spacial(_, _, _)
        | Type::Enum(_, true, _) => std::mem::size_of::<crate::keys::DbRef>(),
        Type::Tuple(elems) => {
            element_offsets(elems).last().map_or(0, |&off| off)
                + elems.last().map_or(0, element_size)
        }
        _ => 0,
    }
}

/// Byte offset of each element in a tuple-like layout.
/// Element *i* starts at `offsets[i]`; total size is `offsets[last] + element_size(last)`.
#[must_use]
pub fn element_offsets(types: &[Type]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(types.len());
    let mut pos: usize = 0;
    for t in types {
        offsets.push(pos);
        pos += element_size(t);
    }
    offsets
}

/// `(offset, index)` pairs for elements that need cleanup on scope exit
/// (text, reference, vector, collection, struct-enum).
#[must_use]
pub fn owned_elements(types: &[Type]) -> Vec<(usize, usize)> {
    let offsets = element_offsets(types);
    let mut result = Vec::new();
    for (i, t) in types.iter().enumerate() {
        match t {
            Type::Text(_)
            | Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Spacial(_, _, _)
            | Type::Enum(_, true, _) => {
                result.push((offsets[i], i));
            }
            _ => {}
        }
    }
    result
}

impl Display for Type {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Integer(min, max, _) if *min == i32::MIN + 1 && *max == i32::MAX as u32 => {
                f.write_str("integer")
            }
            Type::Integer(min, max, _) if *min == 0 && *max == 256 => f.write_str("byte"),
            Type::Integer(min, max, _) => f.write_str(&format!("integer({min}, {max})")),
            Type::Vector(tp, link) if matches!(tp as &Type, Type::Unknown(_)) => {
                f.write_str(&format!("vector#{link:?}"))
            }
            _ => f.write_str(&format!("{self:?}").to_lowercase()),
        }
    }
}

#[derive(Debug)]
pub struct Argument {
    pub name: String,
    pub typedef: Type,
    pub default: Value,
    pub constant: bool,
}

#[derive(Clone)]
#[allow(clippy::struct_excessive_bools)] // four independent property flags (mutable/constant/nullable/primary); an enum would add indirection without clarity
pub struct Attribute {
    /// Name of the attribute for this definition
    pub name: String,
    pub typedef: Type,
    /// This attribute is mutable.
    pub mutable: bool,
    /// Only return the default on this field.
    pub constant: bool,
    /// L7: init(expr) field — stored at creation, writable after. `$` allowed.
    pub init: bool,
    /// This attribute is allowed to be null in the substructure.
    pub nullable: bool,
    /// This attribute is holding the primary reference of its records.
    primary: bool,
    /// Hidden return-mechanism parameter added by `text_return` or `ref_return`.
    /// Not a user-declared parameter — should be excluded from dep propagation.
    pub hidden: bool,
    /// The initial value of this attribute if it is not given.
    pub value: Value,
    /// A constraint expression checked on every field write.
    /// Parsed from `assert(expr)` or `assert(expr, message)` in field definitions.
    pub check: Value,
    /// Optional message for a failed constraint check.
    pub check_message: Value,
    /// Post-2c: when the field's declared type was an integer alias with a
    /// `size(N)` annotation (e.g. `i32`), this holds the alias def_nr so
    /// `fill_database` / codegen can consult `forced_size(alias_nr)`.  `0`
    /// means "no alias" — fall back to the limit()-based heuristic.
    pub alias_d_nr: u32,
}

impl Debug for Attribute {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{}:{}", self.name, &self.typedef))
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum DefType {
    // Not yet known, must be filled in after the first parse pass.
    Unknown,
    // A normal function cannot be defined twice.
    Function,
    // Dynamic function, where all arguments hold references to multiple implementations we can choose
    Dynamic,
    // The possible values are EnumValue definitions in the childs.
    Enum,
    // The parent is the Enum.
    EnumValue,
    // A structure, with possibly conditional fields in the childs.
    Struct,
    // A vector with a unique content (can be a base Type, Struct, Enum or Vector)
    Vector,
    // A type definition, for now only the base types.
    Type,
    // A static constant.
    Constant,
    // A generic function template parameterised by a single type variable.
    // Not compiled until instantiated at a concrete call site (P5).
    Generic,
    // I2: an interface declaration — a named set of required method signatures.
    // Method stubs are stored as attributes on this definition.
    // Used by bounded generics (<T: InterfaceName>) for satisfaction checking (I6).
    Interface,
}

impl Display for DefType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{self:?}"))
    }
}

/// Game definition, the data cannot be changed, there can be instances with differences
#[derive(Clone)]
pub struct Definition {
    pub name: String,
    pub source: u16,
    /// Type of definition.
    pub def_type: DefType,
    /// Parent definition for `EnumValue` or `StructPart`. Initial `u32::MAX`.
    pub parent: u32,
    /// The source file position where this is defined, only allow redefinitions within the same file.
    /// This might eventually also limit access to protected internals.
    pub position: Position,
    /// Allowed attributes
    pub attributes: Vec<Attribute>,
    /// Allowed attributes on name
    pub attr_names: HashMap<String, usize>,
    /// Possible code associated with this definition. The attributes are parameters.
    pub code: Value,
    /// Related type for fields, and the return type for functions
    pub returned: Type,
    /// Whether the return type was declared `not null` (only meaningful for functions)
    pub returned_not_null: bool,
    /// Rust code
    pub rust: String,
    /// Native symbol name for `#native "symbol"` extern dispatch; empty if not native.
    pub native: String,
    /// Interpreter operator code
    pub op_code: u16,
    /// Position inside the generated code
    pub code_position: u32,
    /// Code length for this function
    pub code_length: u32,
    /// Entry in the known types for the database
    pub known_type: u16,
    /// Known variables inside this definition
    pub variables: Function,
    /// Whether this definition was declared with `pub`.
    pub pub_visible: bool,
    /// Definition number of the closure record struct for capturing lambdas.
    /// `u32::MAX` if this function does not capture.
    pub closure_record: u32,
    /// I2: for generic functions — the `def_nr`s of all required interface bounds.
    /// Empty for non-generic or unbounded generic functions.  Multiple bounds (`<T: A + B>`)
    /// are stored as multiple entries; checked for conflicting method signatures at I6.
    pub bounds: Vec<u32>,
    /// DbRef into CONST_STORE for pre-built vector constants.
    /// `None` for non-constant definitions or constants that couldn't be pre-built.
    pub const_ref: Option<crate::keys::DbRef>,
    /// Post-2c: explicit `size(N)` annotation on an integer subtype
    /// (e.g. `pub type i32 = integer size(4);`).  `None` means use the
    /// limit()-based heuristic; `Some(n)` forces the stored-width to n
    /// bytes (n ∈ {1, 2, 4, 8}).
    pub forced_size: Option<u8>,
}

impl Definition {
    #[must_use]
    pub fn is_operator(&self) -> bool {
        matches!(self.def_type, DefType::Function)
            && self.name.len() > 2
            && self.name.starts_with("Op")
            && self.name[2..3]
                .chars()
                .next()
                .unwrap_or_default()
                .is_uppercase()
    }

    #[must_use]
    pub fn original_name(&self) -> String {
        if self.def_type == DefType::Function {
            if self.name.starts_with("t_") {
                if let Ok(nr) = self.name[2..4].parse::<u8>() {
                    self.name[5 + nr as usize..].to_string()
                } else if let Ok(nr) = self.name[2..3].parse::<u8>() {
                    self.name[4 + nr as usize..].to_string()
                } else {
                    self.name[2..].to_string()
                }
            } else {
                self.name[2..].to_string()
            }
        } else {
            self.name.clone()
        }
    }

    #[must_use]
    pub fn header(&self, data: &Data, d_nr: u32) -> String {
        let mut res = "fn ".to_string();
        res += &self.name;
        res += "(";
        for (a_nr, a) in self.attributes.iter().enumerate() {
            if a_nr > 0 {
                res += ", ";
            }
            res += &a.name;
            res += ":";
            res += &a.typedef.argument(data, d_nr);
        }
        res += ")";
        if self.returned != Type::Void {
            res += " -> ";
            res += &self.returned.argument(data, d_nr);
        }
        res
    }
}

#[derive(PartialEq, Debug)]
pub enum Context {
    Argument,
    Reference,
    Result,
    Constant,
    Variable,
}

#[allow(dead_code)]
#[derive(Clone)]
/// The immutable data of a parsed loft program
pub struct Data {
    pub definitions: Vec<Definition>,
    /// Index on definitions on name
    def_names: HashMap<(String, u16), u32>,
    use_names: HashMap<String, u16>,
    /// Current source file
    pub source: u16,
    used_definitions: HashSet<u32>,
    used_attributes: HashSet<(u32, usize)>,
    /// This definition is referenced by a specific definition, the code is used to update this
    referenced: HashMap<u32, (u32, Value)>,
    /// Static data
    statics: Vec<u8>,
    pub(crate) op_codes: u16,
    possible: HashMap<String, Vec<u32>>,
    pub(crate) operators: HashMap<u16, u32>,
    /// PKG.4: native function symbols — loft function name → Rust symbol path.
    /// Populated when packages with `[native.functions]` are loaded.
    /// Keys are the user-facing loft names (e.g. `save_png`), not the internal
    /// `n_save_png` or `t_8graphics_save_png` forms.
    pub native_symbols: HashMap<String, String>,
    /// PKG.4: native package crate directories — (`crate_name`, `pkg_dir`).
    /// Used to construct `--extern` flags for `rustc`.
    pub native_packages: Vec<(String, String)>,
    /// Map from `#native "symbol"` names to the Rust crate that provides them.
    /// Populated when a package declares `[native] crate` in loft.toml.
    /// Used by native codegen to emit `crate::symbol(args)` calls.
    pub native_symbol_crates: HashMap<String, String>,
}

#[must_use]
pub fn v_if(test: Value, t: Value, f: Value) -> Value {
    Value::If(Box::new(test), Box::new(t), Box::new(f))
}

#[must_use]
pub fn v_set(var: u16, value: Value) -> Value {
    Value::Set(var, Box::new(value))
}

#[must_use]
pub fn v_block(operators: Vec<Value>, result: Type, name: &'static str) -> Value {
    Value::Block(Box::new(Block {
        name,
        operators,
        result,
        scope: u16::MAX,
        var_size: 0,
    }))
}

#[must_use]
pub fn v_loop(operators: Vec<Value>, name: &'static str) -> Value {
    Value::Loop(Box::new(Block {
        name,
        operators,
        result: Type::Void,
        scope: u16::MAX,
        var_size: 0,
    }))
}

impl Display for Definition {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", &self.name, &self.def_type)
    }
}

impl Default for Data {
    fn default() -> Self {
        Self::new()
    }
}

struct Into {
    str: String,
}

impl Write for Into {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.str += &String::from_utf8_lossy(buf);
        Ok(self.str.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.write(buf)?;
        Ok(())
    }
}

#[allow(dead_code)]
impl Data {
    #[must_use]
    pub fn new() -> Data {
        Data {
            definitions: Vec::new(),
            def_names: HashMap::new(),
            use_names: HashMap::new(),
            source: 0,
            used_definitions: HashSet::new(),
            used_attributes: HashSet::new(),
            referenced: HashMap::new(),
            statics: Vec::new(),
            op_codes: 0,
            possible: HashMap::new(),
            operators: HashMap::new(),
            native_symbols: HashMap::new(),
            native_packages: Vec::new(),
            native_symbol_crates: HashMap::new(),
        }
    }

    pub fn reset(&mut self) {
        self.use_names.clear();
        self.source = 0;
        self.use_names.insert("std".to_string(), 0);
    }

    #[must_use]
    pub fn get_source(&self, name: &str) -> u16 {
        if let Some(nr) = self.use_names.get(name) {
            *nr
        } else {
            u16::MAX
        }
    }

    #[must_use]
    pub fn use_exists(&self, file: &str) -> bool {
        self.use_names.contains_key(file)
    }

    pub fn use_add(&mut self, short: &str) {
        let n = self.use_names.len() as u16;
        self.use_names.insert(short.to_string(), n);
        self.source = n;
    }

    /// Allow a new attribute on a definition with a specified type.
    pub fn add_attribute(
        &mut self,
        lexer: &mut Lexer,
        on_def: u32,
        name: &str,
        typedef: Type,
    ) -> usize {
        if self.def(on_def).attr_names.contains_key(name) {
            let orig_attr = self.def(on_def).attr_names[name];
            let attr = &self.def(on_def).attributes[orig_attr];
            if attr.typedef.is_unknown() {
                if attr.typedef == typedef {
                    diagnostic!(
                        lexer,
                        Level::Error,
                        "Double attribute '{}.{name}'",
                        self.def(on_def).name
                    );
                } else {
                    diagnostic!(
                        lexer,
                        Level::Error,
                        "Cannot change the type of attribute: {}.{name}",
                        self.def(on_def).name
                    );
                }
            }
            return orig_attr;
        }
        let attr = Attribute {
            name: name.to_string(),
            typedef,
            mutable: true,
            constant: false,
            init: false,
            nullable: true,
            primary: false,
            hidden: false,
            value: Value::Null,
            check: Value::Null,
            check_message: Value::Null,
            alias_d_nr: u32::MAX,
        };
        let next_attr = self.def(on_def).attributes.len();
        let def = &mut self.definitions[on_def as usize];
        def.attr_names.insert(name.to_string(), next_attr);
        def.attributes.push(attr);
        next_attr
    }

    /**
        Add a definitions.
        # Panics
        Will panic if a definition with the same name already exists.
    */
    pub fn add_def(&mut self, name: &str, position: &Position, def_type: DefType) -> u32 {
        let rec = self.definitions();
        assert!(
            !self
                .def_names
                .contains_key(&(name.to_string(), self.source)),
            "Dual definition of {name} at {position}"
        );
        self.def_names.insert((name.to_string(), self.source), rec);
        let new_def = Definition {
            name: name.to_string(),
            source: self.source,
            position: position.clone(),
            def_type,
            parent: u32::MAX,
            attributes: Vec::default(),
            attr_names: HashMap::default(),
            code: Value::Null,
            returned: Type::Unknown(rec),
            returned_not_null: false,
            rust: String::new(),
            native: String::new(),
            op_code: u16::MAX,
            known_type: u16::MAX,
            code_position: 0,
            code_length: 0,
            variables: Function::new(name, &position.file),
            pub_visible: false,
            closure_record: u32::MAX,
            bounds: Vec::new(),
            const_ref: None,
            forced_size: None,
        };
        self.definitions.push(new_def);
        rec
    }

    /**
       Write the `op_code` on operators.
       # Panics
       When too many `op_codes` are written. The byte code can only handle values <256.
    */
    pub fn op_code(&mut self, def_nr: u32) {
        if !self.def(def_nr).is_operator() || self.def(def_nr).op_code != u16::MAX {
            return;
        }
        // Flat table: op_codes 0..N map to OPERATORS[0..N].
        // Bytecode encoding is transparent via fill::emit_op:
        // codes < 255 → 1 byte; codes >= 255 → 2 bytes (255 + offset).
        let max_ops = crate::fill::OPERATORS.len();
        assert!(
            (self.op_codes as usize) < max_ops,
            "Too many defined operators ({} of {max_ops} used). \
             To add more, grow the OPERATORS array in src/fill.rs and regenerate \
             with `cargo test --test issues regen_fill_rs -- --ignored`.",
            self.op_codes
        );
        self.definitions[def_nr as usize].op_code = self.op_codes;
        self.operators.insert(self.op_codes, def_nr);
        self.op_codes += 1;
    }

    #[must_use]
    /// # Panics
    /// When an operator is searched that is currently not known.
    pub fn get_possible(&self, start: &str, lexer: &Lexer) -> &Vec<u32> {
        assert!(
            self.possible.contains_key(start),
            "Unknown operator {start} at {}",
            lexer.pos()
        );
        &self.possible[start]
    }

    #[must_use]
    pub fn definitions(&self) -> u32 {
        self.definitions.len() as u32
    }

    #[must_use]
    pub fn def_referenced(&self, d_nr: u32) -> bool {
        self.referenced.contains_key(&d_nr)
    }

    pub fn set_referenced(&mut self, d_nr: u32, t_nr: u32, change: Value) {
        if d_nr != u32::MAX {
            self.referenced.insert(d_nr, (t_nr, change));
        }
    }

    #[must_use]
    pub fn def_type(&self, d_nr: u32) -> DefType {
        if d_nr == u32::MAX {
            DefType::Unknown
        } else {
            self.def(d_nr).def_type.clone()
        }
    }

    /**
    Set the return type on a definition.
    # Panics
    When the return type was already set before.
    */
    pub fn set_returned(&mut self, d_nr: u32, tp: Type) {
        assert!(
            self.def(d_nr).returned.is_unknown(),
            "Cannot change returned type on [{d_nr}]{} to {} twice was {} at {:?}",
            self.def(d_nr).name,
            self.def(d_nr).returned.name(self),
            tp.name(self),
            self.def(d_nr).position
        );
        self.definitions[d_nr as usize].returned = tp;
    }

    #[must_use]
    pub fn attributes(&self, d_nr: u32) -> usize {
        self.def(d_nr).attributes.len()
    }

    #[must_use]
    pub fn attr(&self, d_nr: u32, name: &str) -> usize {
        if let Some(nr) = self.def(d_nr).attr_names.get(name) {
            *nr
        } else {
            usize::MAX
        }
    }

    #[must_use]
    pub fn attr_name(&self, d_nr: u32, a_nr: usize) -> String {
        if a_nr == usize::MAX {
            "Undefined".to_string()
        } else {
            self.def(d_nr).attributes[a_nr].name.clone()
        }
    }

    #[must_use]
    pub fn attr_type(&self, d_nr: u32, a_nr: usize) -> Type {
        if a_nr == usize::MAX {
            self.def(d_nr).returned.clone()
        } else {
            self.def(d_nr).attributes[a_nr].typedef.clone()
        }
    }

    /**
    Write the type on an attribute of a definition.
    # Panics
    When the type was already set before.
    */
    pub fn set_attr_type(&mut self, d_nr: u32, a_nr: usize, tp: Type) {
        if a_nr == usize::MAX || !self.attr_type(d_nr, a_nr).is_unknown() {
            panic!(
                "Cannot set attribute type {}.{} twice was {} to {}",
                self.def(d_nr).name,
                self.attr_name(d_nr, a_nr),
                self.attr_type(d_nr, a_nr).name(self),
                tp.name(self)
            );
        } else {
            self.definitions[d_nr as usize].attributes[a_nr].typedef = tp;
        }
    }

    #[must_use]
    pub fn attr_value(&self, d_nr: u32, a_nr: usize) -> Value {
        self.def(d_nr).attributes[a_nr].value.clone()
    }

    /**
    Write the default value of an attribute in a definition.
    # Panics
    When the value was already set before.
    */
    pub fn set_attr_value(&mut self, d_nr: u32, a_nr: usize, val: Value) {
        self.definitions[d_nr as usize].attributes[a_nr].value = val;
    }

    #[must_use]
    pub fn attr_check(&self, d_nr: u32, a_nr: u16) -> Value {
        self.def(d_nr).attributes[a_nr as usize].check.clone()
    }

    /**
    Write the check value of an attribute in a definition.
    # Panics
    When the value was already set before.
    */
    pub fn set_attr_check(&mut self, d_nr: u32, a_nr: usize, check: Value) {
        assert_eq!(
            self.def(d_nr).attributes[a_nr].value,
            Value::Null,
            "Cannot set attribute value twice"
        );
        self.definitions[d_nr as usize].attributes[a_nr].check = check;
    }

    #[must_use]
    pub fn attr_nullable(&self, d_nr: u32, a_nr: usize) -> bool {
        if a_nr == usize::MAX {
            return false;
        }
        self.definitions[d_nr as usize].attributes[a_nr].nullable
    }

    pub fn set_attr_nullable(&mut self, d_nr: u32, a_nr: usize, nullable: bool) {
        self.definitions[d_nr as usize].attributes[a_nr].nullable = nullable;
    }

    /**
    Add a new function to the definitions.
    # Panics
    When the return type cannot be parsed.
    */
    pub fn add_fn(&mut self, lexer: &mut Lexer, fn_name: &str, arguments: &[Argument]) -> u32 {
        let mut name = String::new();
        let is_self = !arguments.is_empty() && arguments[0].name == "self";
        let is_both = !arguments.is_empty() && arguments[0].name == "both";
        if is_self || is_both {
            let type_nr = self.type_def_nr(&arguments[0].typedef);
            if type_nr == u32::MAX {
                diagnostic!(
                    lexer,
                    Level::Error,
                    "Unknown type on fn '{fn_name}' argument '{}'",
                    arguments[0].name
                );
            } else {
                name = format!(
                    "t_{}{}_{fn_name}",
                    self.def(type_nr).name.len(),
                    self.def(type_nr).name
                );
            }
        } else {
            name = format!("n_{fn_name}");
        }
        let o_nr = self.def_nr(fn_name);
        if o_nr != u32::MAX && self.def(o_nr).def_type != DefType::Dynamic {
            diagnostic!(
                lexer,
                Level::Error,
                "Cannot redefine '{}' (already defined at {})",
                fn_name.strip_prefix("n_").unwrap_or(fn_name),
                self.def(o_nr).position
            );
        }
        let mut d_nr = self.def_nr(&name);
        if d_nr != u32::MAX {
            diagnostic!(
                lexer,
                Level::Error,
                "Cannot redefine '{}'",
                fn_name.strip_prefix("n_").unwrap_or(fn_name)
            );
            return u32::MAX;
        }
        d_nr = self.add_def(&name, lexer.pos(), DefType::Function);
        for a in arguments {
            let a_nr = self.add_attribute(lexer, d_nr, &a.name, a.typedef.clone());
            self.set_attr_value(d_nr, a_nr, a.default.clone());
            // Note: Argument.constant (the `const` keyword on a parameter) is enforced at the
            // parser level via Variable.const_param — NOT by setting Attribute.mutable = false
            // here. Setting mutable = false for a user-defined function parameter would cause
            // the bytecode generator to skip pushing the argument value onto the stack, breaking
            // all calls to the function. Attribute.constant/mutable semantics are only correct
            // for operator definitions (add_op), where non-mutable params are bytecode constants.
        }
        if is_self || is_both {
            let type_nr = self.type_def_nr(&arguments[0].typedef);
            if self.attr(type_nr, fn_name) != usize::MAX {
                diagnostic!(lexer, Level::Error, "Cannot redefine field {fn_name}",);
                return u32::MAX;
            }
            let a_nr = self.add_attribute(lexer, type_nr, fn_name, Type::Routine(d_nr));
            self.definitions[type_nr as usize].attributes[a_nr].mutable = false;
            self.definitions[type_nr as usize].attributes[a_nr].constant = true;
        }
        if is_both {
            let mut main = self.def_nr(fn_name);
            if main == u32::MAX {
                main = self.add_def(fn_name, lexer.pos(), DefType::Dynamic);
            }
            let type_nr = self.type_def_nr(&arguments[0].typedef);
            assert_ne!(
                type_nr,
                u32::MAX,
                "Unknown type {}: {:?} at {}",
                arguments[0].name,
                arguments[0].typedef,
                lexer.pos()
            );
            let name = &self.def(type_nr).name.clone();
            let a_nr = self.add_attribute(lexer, main, name, Type::Routine(d_nr));
            self.definitions[main as usize].attributes[a_nr].mutable = false;
            self.definitions[main as usize].attributes[a_nr].constant = true;
        }
        d_nr
    }

    #[must_use]
    pub fn get_fn(&self, fn_name: &str, arguments: &[Argument]) -> u32 {
        let is_self = !arguments.is_empty() && arguments[0].name == "self";
        let is_both = !arguments.is_empty() && arguments[0].name == "both";
        if is_self || is_both {
            let type_nr = self.type_def_nr(&arguments[0].typedef);
            let name = format!(
                "t_{}{}_{fn_name}",
                self.def(type_nr).name.len(),
                self.def(type_nr).name
            );
            let struct_source = self.definitions[type_nr as usize].source;
            let d_nr = self.source_nr(struct_source, &name);
            if d_nr == u32::MAX {
                // Method defined outside the struct's source file (e.g., user extends a
                // library type). Fall back to the current parse source.
                self.source_nr(self.source, &name)
            } else {
                d_nr
            }
        } else {
            self.def_nr(&format!("n_{fn_name}"))
        }
    }

    #[must_use]
    pub fn find_fn(&self, source: u16, fn_name: &str, tp: &Type) -> u32 {
        if matches!(tp, Type::Unknown(_)) {
            return self.source_nr(source, &format!("n_{fn_name}"));
        }
        let type_nr = self.type_def_nr(tp);
        if type_nr == u32::MAX {
            // No method dispatch for types like Function; fall back to n_ global.
            return self.source_nr(source, &format!("n_{fn_name}"));
        }
        let name = format!(
            "t_{}{}_{fn_name}",
            self.def(type_nr).name.len(),
            self.def(type_nr).name
        );
        let d_nr = self.source_nr(source, &name);
        if d_nr != u32::MAX {
            return d_nr;
        }
        let d_nr = self.source_nr(source, &format!("n_{fn_name}"));
        if d_nr != u32::MAX {
            return d_nr;
        }
        // I9-prim: fall back to the `possible` operator map for built-in types.
        // Built-in operators use `add_op` (e.g. `OpLtInt`) rather than the method-style
        // `t_7integer_OpLt` convention.  Search `possible[fn_name]` for an operator whose
        // first parameter matches `tp`.
        if let Some(ops) = self.possible.get(fn_name) {
            for &op_nr in ops {
                if !self.def(op_nr).attributes.is_empty() && self.attr_type(op_nr, 0).is_equal(tp) {
                    return op_nr;
                }
            }
        }
        u32::MAX
    }

    /**
    Add a new operator
    # Panics
    When operators are not scanned correctly.
    */
    pub fn add_op(&mut self, lexer: &mut Lexer, fn_name: &str, arguments: &[Argument]) -> u32 {
        let d_nr = self.add_def(fn_name, lexer.pos(), DefType::Function);
        for a in arguments {
            let a_nr = self.add_attribute(lexer, d_nr, &a.name, a.typedef.clone());
            self.definitions[d_nr as usize].attributes[a_nr].mutable = !a.constant;
            self.definitions[d_nr as usize].attributes[a_nr].constant = a.constant;
            self.set_attr_value(d_nr, a_nr, a.default.clone());
        }
        if self.def(d_nr).is_operator() {
            for op in OPERATORS {
                if self.def(d_nr).name.starts_with(op) {
                    if !self.possible.contains_key(*op) {
                        self.possible.insert((*op).to_string(), Vec::new());
                    }
                    self.possible.get_mut(*op).unwrap().push(d_nr);
                }
            }
        }
        d_nr
    }

    /// Get a vector definition. This is a record with a single field pointing towards this vector.
    /// We need this definition as the primary record of a database holding a vector and its child records/vectors.
    pub fn vector_def(&mut self, lexer: &mut Lexer, tp: &Type) -> u32 {
        let fld_tp = Type::Vector(Box::new(tp.clone()), Vec::new());
        let fld = fld_tp.name(self);
        if self.def_nr(&fld) == u32::MAX {
            let d = self.add_def(&fld, lexer.pos(), DefType::Vector);
            self.definitions[d as usize].returned = fld_tp;
            self.definitions[d as usize].parent = self.type_def_nr(tp);
        }
        let name = format!("main_vector<{}>", tp.name(self));
        let d_nr = self.def_nr(&name);
        if d_nr == u32::MAX {
            let vd = self.add_def(&name, lexer.pos(), DefType::Struct);
            // Also register globally (source=0) so other files can find it.
            self.def_names.entry((name.clone(), 0)).or_insert(vd);
            self.add_attribute(
                lexer,
                vd,
                "vector",
                Type::Vector(Box::new(tp.clone()), Vec::new()),
            );
            vd
        } else {
            d_nr
        }
    }

    pub fn check_vector(&mut self, d_nr: u32, vec_tp: u16, pos: &Position) -> u32 {
        let vec_name = format!("vector<{}>", self.def(d_nr).name);
        let mut v_nr = self.def_nr(&vec_name);
        if v_nr == u32::MAX {
            v_nr = self.add_def(&vec_name, pos, DefType::Vector);
            self.definitions[v_nr as usize].parent = d_nr;
        }
        self.definitions[v_nr as usize].known_type = vec_tp;
        v_nr
    }

    /// Get the corresponding number from a definition on name.
    /// This will test both the own source file or the standard library data.
    #[must_use]
    pub fn def_nr(&self, name: &str) -> u32 {
        if let Some(nr) = self.def_names.get(&(name.to_string(), self.source)) {
            *nr
        } else if let Some(nr) = self.def_names.get(&(name.to_string(), 0)) {
            *nr
        } else {
            u32::MAX
        }
    }

    #[must_use]
    pub fn source_nr(&self, source: u16, name: &str) -> u32 {
        if source == u16::MAX {
            return self.def_nr(name);
        }
        let Some(nr) = self.def_names.get(&(name.to_string(), source)) else {
            return u32::MAX;
        };
        *nr
    }

    /** Get the definition by name
    # Panics
    When an unknown definition is requested
    */
    #[must_use]
    pub fn name_type(&self, name: &str, source: u16) -> u16 {
        let nr = if let Some(nr) = self.def_names.get(&(name.to_string(), source)) {
            *nr
        } else if let Some(nr) = self.def_names.get(&(name.to_string(), 0)) {
            *nr
        } else {
            return u16::MAX;
        };
        self.definitions[nr as usize].known_type
    }

    /** Get the definition by name from a given source file
    # Panics
    When an unknown definition is requested
    */
    #[must_use]
    pub fn source_name(&self, source: u16, name: &str) -> &Definition {
        let Some(nr) = self.def_names.get(&(name.to_string(), source)) else {
            panic!("Unknown definition {name}");
        };
        &self.definitions[*nr as usize]
    }

    /// Import all names from `lib_source` into `into_source`.
    /// Names already present in `into_source` (local definitions) are kept unchanged.
    pub fn import_all(&mut self, lib_source: u16, into_source: u16) {
        let names: Vec<(String, u32)> = self
            .def_names
            .iter()
            .filter(|((_, src), def_nr)| {
                *src == lib_source && self.definitions[**def_nr as usize].pub_visible
            })
            .map(|((name, _), &def_nr)| (name.clone(), def_nr))
            .collect();
        for (name, def_nr) in names {
            self.def_names.entry((name, into_source)).or_insert(def_nr);
        }
    }

    /// Import a single name from `lib_source` into `into_source`.
    /// Returns `false` if neither the plain name nor its `n_`-prefixed function
    /// form exists in `lib_source`, so the caller can emit an appropriate error.
    /// Names already present in `into_source` are kept unchanged (local wins).
    pub fn import_name(&mut self, lib_source: u16, into_source: u16, name: &str) -> bool {
        // Functions are stored under the `n_` prefix; try both forms.
        let fn_key = format!("n_{name}");
        let found_plain = self
            .def_names
            .get(&(name.to_string(), lib_source))
            .copied()
            .filter(|&d| self.definitions[d as usize].pub_visible);
        let found_fn = self
            .def_names
            .get(&(fn_key.clone(), lib_source))
            .copied()
            .filter(|&d| self.definitions[d as usize].pub_visible);
        if found_plain.is_none() && found_fn.is_none() {
            return false;
        }
        if let Some(def_nr) = found_plain {
            self.def_names
                .entry((name.to_string(), into_source))
                .or_insert(def_nr);
        }
        if let Some(def_nr) = found_fn {
            self.def_names
                .entry((fn_key, into_source))
                .or_insert(def_nr);
        }
        true
    }

    /// Variant of [`import_all`] that **overwrites** forward-reference stubs
    /// in the target source.  Real local definitions still win (local
    /// precedence), but bindings that currently point to a `DefType::Unknown`
    /// stub are replaced by the imported real definition.
    ///
    /// Used by the P173 package-mode driver's Phase C: when file B creates an
    /// Unknown stub for a type that will come from file A's re-exported
    /// namespace, this variant is what makes B's later references resolve
    /// to A's real definition.
    pub fn import_all_overwrite(&mut self, lib_source: u16, into_source: u16) {
        let names: Vec<(String, u32)> = self
            .def_names
            .iter()
            .filter(|((_, src), def_nr)| {
                *src == lib_source && self.definitions[**def_nr as usize].pub_visible
            })
            .map(|((name, _), &def_nr)| (name.clone(), def_nr))
            .collect();
        for (name, def_nr) in names {
            self.insert_or_replace_stub((name, into_source), def_nr);
        }
    }

    /// Variant of [`import_name`] that overwrites forward-reference stubs.
    /// See [`import_all_overwrite`] for the rationale.  Returns the same
    /// `false` on lookup miss as [`import_name`].
    pub fn import_name_overwrite(&mut self, lib_source: u16, into_source: u16, name: &str) -> bool {
        let fn_key = format!("n_{name}");
        let found_plain = self
            .def_names
            .get(&(name.to_string(), lib_source))
            .copied()
            .filter(|&d| self.definitions[d as usize].pub_visible);
        let found_fn = self
            .def_names
            .get(&(fn_key.clone(), lib_source))
            .copied()
            .filter(|&d| self.definitions[d as usize].pub_visible);
        if found_plain.is_none() && found_fn.is_none() {
            return false;
        }
        if let Some(def_nr) = found_plain {
            self.insert_or_replace_stub((name.to_string(), into_source), def_nr);
        }
        if let Some(def_nr) = found_fn {
            self.insert_or_replace_stub((fn_key, into_source), def_nr);
        }
        true
    }

    /// Insert `def_nr` at `key`, or replace an existing binding when the
    /// existing binding points to a `DefType::Unknown` stub.  Real local
    /// bindings are preserved (local wins over imports).
    fn insert_or_replace_stub(&mut self, key: (String, u16), def_nr: u32) {
        match self.def_names.get(&key) {
            Some(&existing)
                if matches!(
                    self.definitions[existing as usize].def_type,
                    DefType::Unknown
                ) =>
            {
                self.def_names.insert(key, def_nr);
            }
            None => {
                self.def_names.insert(key, def_nr);
            }
            _ => {}
        }
    }

    /// Rewrite every `Type::Unknown(stub_nr)` occurrence in any definition's
    /// `returned` type or attribute typedefs to the resolved type from
    /// `target_def_nr`.  Walks compound types (`Vector`, `RefVar`, `Iterator`,
    /// `Tuple`, `Function`, `Rewritten`) recursively.
    ///
    /// Used by the P173 package-mode driver's Phase C after imports have been
    /// propagated: stub def_nrs created during Phase B's deferred parsing
    /// become resolvable once the real definition is reachable via
    /// `def_names`, and this helper patches every `Type::Unknown(stub_nr)`
    /// pointer to the real type.  Direct mutation of the stored type bypasses
    /// `set_attr_type`'s panic guard, which only accepts replacement when the
    /// outer type is already `Unknown` — we need to patch `Vector<Unknown>`,
    /// `RefVar<Unknown>`, etc., where the outer wrapper is not `Unknown`.
    pub fn rewrite_unknown_refs(&mut self, stub_nr: u32, target_def_nr: u32) {
        let target_type = self.definitions[target_def_nr as usize].returned.clone();
        for d_nr in 0..self.definitions.len() {
            if let Some(new_ret) =
                Self::rewrite_type_opt(&self.definitions[d_nr].returned, stub_nr, &target_type)
            {
                self.definitions[d_nr].returned = new_ret;
            }
            let n_attrs = self.definitions[d_nr].attributes.len();
            for a_nr in 0..n_attrs {
                if let Some(new_ty) = Self::rewrite_type_opt(
                    &self.definitions[d_nr].attributes[a_nr].typedef,
                    stub_nr,
                    &target_type,
                ) {
                    self.definitions[d_nr].attributes[a_nr].typedef = new_ty;
                }
            }
        }
    }

    /// Recursive helper for [`rewrite_unknown_refs`].  Returns
    /// `Some(new_type)` when the subtree contained `Type::Unknown(stub)`
    /// and was rewritten, or `None` when the subtree is unchanged.
    #[allow(clippy::only_used_in_recursion)] // kept as associated fn for clarity
    fn rewrite_type_opt(t: &Type, stub: u32, target: &Type) -> Option<Type> {
        match t {
            Type::Unknown(n) if *n == stub => Some(target.clone()),
            Type::Vector(inner, deps) => Self::rewrite_type_opt(inner, stub, target)
                .map(|new_inner| Type::Vector(Box::new(new_inner), deps.clone())),
            Type::RefVar(inner) => Self::rewrite_type_opt(inner, stub, target)
                .map(|new_inner| Type::RefVar(Box::new(new_inner))),
            Type::Rewritten(inner) => Self::rewrite_type_opt(inner, stub, target)
                .map(|new_inner| Type::Rewritten(Box::new(new_inner))),
            Type::Iterator(step, internal) => {
                let new_step = Self::rewrite_type_opt(step, stub, target);
                let new_internal = Self::rewrite_type_opt(internal, stub, target);
                if new_step.is_none() && new_internal.is_none() {
                    None
                } else {
                    Some(Type::Iterator(
                        Box::new(new_step.unwrap_or_else(|| (**step).clone())),
                        Box::new(new_internal.unwrap_or_else(|| (**internal).clone())),
                    ))
                }
            }
            Type::Tuple(elems) => {
                let mut changed = false;
                let new_elems: Vec<Type> = elems
                    .iter()
                    .map(|e| match Self::rewrite_type_opt(e, stub, target) {
                        Some(new_e) => {
                            changed = true;
                            new_e
                        }
                        None => e.clone(),
                    })
                    .collect();
                if changed {
                    Some(Type::Tuple(new_elems))
                } else {
                    None
                }
            }
            Type::Function(args, ret, deps) => {
                let mut changed = false;
                let new_args: Vec<Type> = args
                    .iter()
                    .map(|a| match Self::rewrite_type_opt(a, stub, target) {
                        Some(new_a) => {
                            changed = true;
                            new_a
                        }
                        None => a.clone(),
                    })
                    .collect();
                let new_ret_opt = Self::rewrite_type_opt(ret, stub, target);
                if changed || new_ret_opt.is_some() {
                    Some(Type::Function(
                        new_args,
                        Box::new(new_ret_opt.unwrap_or_else(|| (**ret).clone())),
                        deps.clone(),
                    ))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /** Get a definition.
    # Panics
    When no definition on that number is found
    */
    #[must_use]
    pub fn def(&self, dnr: u32) -> &Definition {
        assert_ne!(dnr, u32::MAX, "Unknown definition");
        &self.definitions[dnr as usize]
    }

    /// Post-2c: return the explicit `size(N)` annotation on the definition,
    /// if any.  Used by field allocation and sizeof() to honor `pub type i32 =
    /// integer size(4);` — the size overrides the limit()-based heuristic.
    /// Returns `None` when no annotation was provided (use the heuristic).
    #[must_use]
    pub fn forced_size(&self, dnr: u32) -> Option<u8> {
        if dnr == u32::MAX || (dnr as usize) >= self.definitions.len() {
            return None;
        }
        self.definitions[dnr as usize].forced_size
    }

    /// Return the `def_nr`s of all definitions whose `parent` field equals `parent_nr`.
    /// Used by the interface satisfaction checker (I6) to enumerate an interface's method stubs.
    pub fn children_of(&self, parent_nr: u32) -> impl Iterator<Item = u32> + '_ {
        self.definitions
            .iter()
            .enumerate()
            .filter(move |(_, d)| d.parent == parent_nr)
            .map(|(i, _)| i as u32)
    }

    /// # Panics
    /// When no definition on that number is found.
    pub fn def_mut(&mut self, dnr: u32) -> &mut Definition {
        assert_ne!(dnr, u32::MAX, "Unknown definition");
        &mut self.definitions[dnr as usize]
    }

    #[must_use]
    pub fn has_op(&self, op: u16) -> bool {
        self.operators.contains_key(&op)
    }

    #[must_use]
    pub fn operator(&self, op: u16) -> &Definition {
        self.def(self.operators[&op])
    }

    pub fn attr_used(&mut self, d_nr: u32, a_nr: usize) {
        self.used_attributes.insert((d_nr, a_nr));
    }

    pub fn def_used(&mut self, d_nr: u32) {
        self.used_definitions.insert(d_nr);
    }

    #[must_use]
    pub fn type_def_nr(&self, tp: &Type) -> u32 {
        match tp {
            Type::Rewritten(t) => self.type_def_nr(t),
            Type::Integer(_, _, _) => self.source_nr(0, "integer"),
            Type::Boolean => self.source_nr(0, "boolean"),
            Type::Float => self.source_nr(0, "float"),
            Type::Text(_) => self.source_nr(0, "text"),
            Type::Single => self.source_nr(0, "single"),
            Type::Character => self.source_nr(0, "character"),
            Type::Routine(d_nr)
            | Type::Enum(d_nr, _, _)
            | Type::Reference(d_nr, _)
            | Type::Unknown(d_nr) => *d_nr,
            Type::Vector(_, _) => self.source_nr(0, "vector"),
            Type::RefVar(t) if matches!(**t, Type::Reference(_, _)) => self.type_def_nr(t),
            Type::RefVar(_) | Type::Sorted(_, _, _) => self.source_nr(0, "reference"),
            Type::Index(_, _, _) => self.source_nr(0, "index"),
            Type::Hash(_, _, _) => self.source_nr(0, "hash"),
            _ => u32::MAX,
        }
    }

    #[must_use]
    /// Get the definition number for the given type.
    /// # Panics
    /// When no element of a type exists
    pub fn type_elm(&self, tp: &Type) -> u32 {
        match tp {
            Type::Rewritten(t) => self.type_elm(t),
            Type::Integer(_, _, _) => self.source_nr(0, "integer"),
            Type::Boolean => self.source_nr(0, "boolean"),
            Type::Float => self.source_nr(0, "float"),
            Type::Text(_) => self.source_nr(0, "text"),
            Type::Single => self.source_nr(0, "single"),
            Type::Character => self.source_nr(0, "character"),
            Type::Routine(d_nr) | Type::Enum(d_nr, _, _) | Type::Reference(d_nr, _) => *d_nr,
            Type::Vector(tp, _) | Type::RefVar(tp) => {
                if let Type::Reference(td, _) = **tp {
                    td
                } else {
                    self.type_def_nr(tp)
                }
            }
            Type::Sorted(_, _, _) | Type::Index(_, _, _) | Type::Hash(_, _, _) => {
                self.source_nr(0, "reference")
            }
            _ => u32::MAX,
        }
    }

    /// Return a user-facing type name string for use by `type_name()`.
    #[must_use]
    pub fn type_name_str(&self, tp: &Type) -> String {
        match tp {
            Type::Unknown(_) => "unknown".to_string(),
            Type::Null => "null".to_string(),
            Type::Void => "void".to_string(),
            Type::Never => "never".to_string(),
            Type::Integer(min, max, _) if *min == i32::MIN + 1 && *max == i32::MAX as u32 => {
                "integer".to_string()
            }
            Type::Integer(_, _, _) => "integer".to_string(),
            Type::Boolean => "boolean".to_string(),
            Type::Float => "float".to_string(),
            Type::Single => "single".to_string(),
            Type::Character => "character".to_string(),
            Type::Text(_) => "text".to_string(),
            Type::Keys => "keys".to_string(),
            Type::Enum(d_nr, _, _) | Type::Reference(d_nr, _) => self.def(*d_nr).name.clone(),
            Type::RefVar(inner) => format!("&{}", self.type_name_str(inner)),
            Type::Vector(inner, _) => format!("vector<{}>", self.type_name_str(inner)),
            Type::Sorted(d_nr, _, _) => format!("sorted<{}>", self.def(*d_nr).name),
            Type::Index(d_nr, _, _) => format!("index<{}>", self.def(*d_nr).name),
            Type::Hash(d_nr, _, _) => format!("hash<{}>", self.def(*d_nr).name),
            Type::Routine(_) => "fn".to_string(),
            Type::Function(args, ret, _) => {
                let args_s: Vec<String> = args.iter().map(|a| self.type_name_str(a)).collect();
                format!("fn({}) -> {}", args_s.join(", "), self.type_name_str(ret))
            }
            Type::Iterator(inner, _) => format!("iterator<{}>", self.type_name_str(inner)),
            Type::Rewritten(inner) => self.type_name_str(inner),
            Type::Spacial(d_nr, _, _) => format!("spacial<{}>", self.def(*d_nr).name),
            Type::Tuple(elems) => {
                let es: Vec<String> = elems.iter().map(|e| self.type_name_str(e)).collect();
                format!("({})", es.join(", "))
            }
        }
    }

    /**
    Return the rust type for definitions.
    # Panics
    When the rust type cannot be determined.
    */
    #[must_use]
    pub fn rust_type(&self, tp: &Type, context: &Context) -> String {
        if context == &Context::Reference {
            let mut result = String::new();
            result += "&";
            result += &self.rust_type(tp, &Context::Argument);
            return result;
        }
        match tp {
            Type::Integer(from, to, _)
                if i64::from(*to) - i64::from(*from) <= 255 && i64::from(*from) >= 0 =>
            {
                "u8"
            }
            Type::Integer(from, to, _)
                if i64::from(*to) - i64::from(*from) <= 65536 && i64::from(*from) >= 0 =>
            {
                "u16"
            }
            Type::Integer(from, to, _) if i64::from(*to) - i64::from(*from) <= 255 => "i8",
            Type::Integer(from, to, _) if i64::from(*to) - i64::from(*from) <= 65536 => "i16",
            Type::Integer(_, _, _) => "i64",
            Type::Enum(_, false, _) => "u8",
            Type::Text(_) if context == &Context::Variable => "String",
            Type::Text(_) => "Str",
            Type::Boolean => "bool",
            Type::Float => "f64",
            Type::Single => "f32",
            Type::Character => "char",
            Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::RefVar(_)
            | Type::Enum(_, true, _)
            | Type::Index(_, _, _) => "DbRef",
            Type::Routine(_) => "u32",
            Type::Unknown(_) => "??",
            Type::Iterator(_, _) => "Iterator",
            Type::Keys => "&[Key]",
            _ => panic!("Incorrect type {}", tp.name(self)),
        }
        .to_string()
    }

    pub fn find_unused(&self, diagnostics: &mut Diagnostics) {
        for (d_nr, def) in self.definitions.iter().enumerate() {
            if self.used_definitions.contains(&(d_nr as u32)) {
                for (a_nr, attr) in def.attributes.iter().enumerate() {
                    if !self.used_attributes.contains(&(d_nr as u32, a_nr)) {
                        diagnostics.add(
                            Level::Warning,
                            &format!(
                                "Unused field {}.{} at {}",
                                def.name, attr.name, def.position
                            ),
                        );
                    }
                }
            } else {
                diagnostics.add(
                    Level::Warning,
                    &format!("Unused definition {} at {}", def.name, def.position),
                );
            }
        }
    }

    /**
    Dump the internal parse tree to the standard output.
    # Panics
    Will not, this is to internal data structures instead of a file.
    */
    pub fn dump(&self, d_nr: u32) {
        let mut vars = Function::copy(&self.def(d_nr).variables);
        let mut s = Into { str: String::new() };
        self.show_code(&mut s, &mut vars, &self.def(d_nr).code, 0, true)
            .unwrap();
        println!("dump {}", s.str);
    }

    /**
    Dump the internal parse tree to the standard output.
    # Panics
    Will not, this is to internal data structures instead of a file.
    */
    pub fn dump_fn(&self, value: &Value, vars: &Function) {
        let mut vars = Function::copy(vars);
        let mut s = Into { str: String::new() };
        self.show_code(&mut s, &mut vars, value, 0, true).unwrap();
        println!("dump_fn {}", s.str);
    }

    /**
    Dump the internal parse tree to file.
    # Panics
    On incorrect rewritten code
    # Errors
    When the file cannot be written.
    */
    #[allow(clippy::too_many_lines)]
    pub fn show_code(
        &self,
        write: &mut dyn Write,
        vars: &mut Function,
        value: &Value,
        indent: u32,
        start: bool,
    ) -> Result<()> {
        if start {
            for _i in 0..indent {
                write!(write, "  ")?;
            }
        }
        match value {
            Value::Null => write!(write, "null"),
            Value::Int(i) => write!(write, "{i}i32"),
            Value::Enum(e, tp) => write!(write, "{e}u8({tp})"),
            Value::Boolean(true) => write!(write, "true"),
            Value::Boolean(_) => write!(write, "false"),
            Value::Float(f) => write!(write, "{f}f64"),
            Value::Long(l) => write!(write, "{l}i64"),
            Value::Single(f) => write!(write, "{f}f32"),
            Value::Text(t) => write!(write, "\"{t}\""),
            Value::Iter(_, _, _, _) => panic!("Rewrite!"),
            Value::Call(t, ex) => {
                write!(write, "{}(", self.def(*t).name)?;
                for (v_nr, v) in ex.iter().enumerate() {
                    if v_nr > 0 {
                        write!(write, ", ")?;
                    }
                    self.show_code(write, vars, v, indent, false)?;
                }
                write!(write, ")")
            }
            Value::CallRef(v, ex) => {
                write!(write, "fn_ref[{v}](")?;
                for (i, a) in ex.iter().enumerate() {
                    if i > 0 {
                        write!(write, ", ")?;
                    }
                    self.show_code(write, vars, a, indent, false)?;
                }
                write!(write, ")")
            }
            Value::Block(bl) => self.show_block(write, vars, bl, indent),
            Value::Var(v) => write!(write, "{}({})", vars.name(*v), vars.scope(*v)),
            Value::Set(v, to) => {
                if *v == u16::MAX {
                    write!(write, "unknown(??):?? = ")?;
                } else {
                    write!(
                        write,
                        "{}({}):{} = ",
                        vars.name(*v),
                        vars.scope(*v),
                        vars.tp(*v).show(self, vars)
                    )?;
                }
                self.show_code(write, vars, to, indent, false)
            }
            Value::Return(ex) => {
                write!(write, "return ")?;
                self.show_code(write, vars, ex, indent, false)
            }
            Value::Insert(i) => self.show_insert(write, vars, i, indent),
            Value::Break(v) => write!(write, "break({v})"),
            Value::BreakWith(v, expr) => {
                write!(write, "break({v}) ")?;
                self.show_code(write, vars, expr, indent, false)
            }
            Value::Continue(v) => write!(write, "continue({v})"),
            Value::If(test, t, f) => {
                write!(write, "if ")?;
                self.show_code(write, vars, test, indent, false)?;
                write!(write, " ")?;
                self.show_code(write, vars, t, indent, false)?;
                write!(write, " else ")?;
                self.show_code(write, vars, f, indent, false)
            }
            Value::Loop(lp) => self.show_loop(write, vars, lp, indent),
            Value::Drop(v) => {
                write!(write, "drop ")?;
                self.show_code(write, vars, v, indent, false)
            }
            Value::Keys(keys) => {
                write!(write, "&{keys:?}")
            }
            Value::Line(line) => write!(write, "[{line}] "),
            Value::Tuple(elems) => {
                write!(write, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(write, ", ")?;
                    }
                    self.show_code(write, vars, e, indent, false)?;
                }
                write!(write, ")")
            }
            Value::TupleGet(var, idx) => {
                write!(write, "{}.{idx}", vars.name(*var))
            }
            Value::TuplePut(var, idx, val) => {
                write!(write, "{}.{idx} = ", vars.name(*var))?;
                self.show_code(write, vars, val, indent, false)
            }
            Value::Yield(inner) => {
                write!(write, "yield ")?;
                self.show_code(write, vars, inner, indent, false)
            }
            Value::FnRef(d_nr, clos_var, _) => {
                write!(write, "FnRef({d_nr}, {})", vars.name(*clos_var))
            }
            Value::Parallel(arms) => {
                writeln!(write, "parallel {{")?;
                for arm in arms {
                    self.show_code(write, vars, arm, indent + 1, true)?;
                    writeln!(write, ";")?;
                }
                for _i in 0..indent {
                    write!(write, "  ")?;
                }
                write!(write, "}}")
            }
        }
    }

    fn show_block(
        &self,
        write: &mut dyn Write,
        vars: &mut Function,
        bl: &crate::data::Block,
        indent: u32,
    ) -> Result<()> {
        if !bl.operators.is_empty() {
            writeln!(
                write,
                "{{#{}({}):{}",
                bl.name,
                bl.scope,
                bl.result.show(self, vars)
            )?;
            let mut starting = true;
            for val in &bl.operators {
                self.show_code(write, vars, val, indent + 1, starting)?;
                starting = if matches!(val, Value::Line(_)) {
                    false
                } else {
                    writeln!(write, ";")?;
                    true
                };
            }
            for _i in 0..indent {
                write!(write, "  ")?;
            }
            write!(
                write,
                "}}#{}({}):{}",
                bl.name,
                bl.scope,
                bl.result.show(self, vars)
            )?;
        }
        Ok(())
    }

    fn show_loop(
        &self,
        write: &mut dyn Write,
        vars: &mut Function,
        lp: &Block,
        indent: u32,
    ) -> Result<()> {
        writeln!(write, "loop {{#{}_{}", lp.name, lp.scope)?;
        for val in &lp.operators {
            self.show_code(write, vars, val, indent + 1, true)?;
            writeln!(write, ";")?;
        }
        for _i in 0..indent {
            write!(write, "  ")?;
        }
        write!(write, "}}#{}_{}", lp.name, lp.scope)?;
        Ok(())
    }

    fn show_insert(
        &self,
        write: &mut dyn Write,
        vars: &mut Function,
        items: &[Value],
        indent: u32,
    ) -> Result<()> {
        writeln!(write, "{{ !! INSERT")?;
        for v in items {
            self.show_code(write, vars, v, indent + 1, true)?;
            writeln!(write)?;
        }
        for _i in 0..indent {
            write!(write, "  ")?;
        }
        write!(write, "}}")
    }
}

#[test]
fn value_sizes() {
    // Debugging function to validate the sizes of the variants for the Value enum.
    assert_eq!(size_of::<Value>(), 32);
    assert_eq!(size_of::<Vec<Value>>(), 24);
    assert_eq!(size_of::<Box<Value>>(), 8);
    assert_eq!(size_of::<(u8, u32)>(), 8); // Int
    assert_eq!(size_of::<(u8, u8, u16)>(), 4); // Enum
    assert_eq!(size_of::<(u8, f64)>(), 16); // Float
    assert_eq!(size_of::<(u8, String)>(), 32); // Text
    assert_eq!(size_of::<(u8, u32, Vec<Value>)>(), 32); // Call
    assert_eq!(size_of::<(u8, Box<(Vec<Value>, Type, &'static str)>)>(), 16); // Block
    assert_eq!(size_of::<(u8, u16, Box<Value>)>(), 16); // Set
    assert_eq!(size_of::<(u8, Box<Value>, Box<Value>, Box<Value>)>(), 32); // If
    assert_eq!(size_of::<(u8, Box<Value>, Box<Value>)>(), 24); // Iter
}
