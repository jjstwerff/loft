// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Parse scripts and create internal code from it.
//! Including type checking.
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::data::{
    Argument, Context, Data, DefType, I32, Type, Value, to_default, v_block, v_if, v_loop, v_set,
};
use crate::database::{Parts, Stores};
use crate::diagnostics::{Diagnostics, Level, diagnostic_format};
use crate::lexer::{LexItem, LexResult, Lexer, Link, Mode, Position};
use crate::variables::{Function, size as var_size};
use crate::{manifest, scopes, typedef};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::fs::{File, metadata, read_dir};
use std::io::Write;
use std::string::ToString;
use typedef::complete_definition;

/**
The number of defined reserved text worker variables. A worker variable is needed when
two texts are added or a formatting text is used, and the result is used as a parameter to a call.
These are reused when possible. However, when calculating a text, a new text expression
is used a next worker variable is needed.
This number indicated the depth of these expressions, not the number of these expressions in a
function.
*/
// The parser holds several independent boolean mode flags (in_loop, default, first_pass,
// reverse_iterator) that each track a distinct parse phase or context.  Combining them into
// an enum or state machine would add complexity without benefit.
#[allow(clippy::struct_excessive_bools)]
pub struct Parser {
    pub todo_files: Vec<(String, u16)>,
    /// All definitions
    pub data: Data,
    pub database: Stores,
    /// The lexer on the current text file
    pub lexer: Lexer,
    /// Are we currently allowing break/continue statements?
    in_loop: bool,
    /// The current file number that is being parsed
    file: u32,
    pub diagnostics: Diagnostics,
    default: bool,
    /// The definition that is currently parsed (function or struct)
    context: u32,
    /// Extra library directories for 'use' resolution (from --lib / --project flags)
    pub lib_dirs: Vec<String>,
    /// Is this the first pass on parsing:
    /// - Do not assume that all struct / enum types are already parsed.
    /// - Define variables, try to determine their type (can become clear from later code).
    /// - Claim working text variables for expressions that gather text data outside variables.
    /// - Links between memory allocations (text, stores) their type knows the variable numbers.
    /// - Move variables to a lower scope if an expression still links to their content.
    /// - Determine mutations to stores and administer these in arguments.
    ///
    /// The second pass:
    /// - Creates code, assumes that all types are known.
    first_pass: bool,
    /// Set by `parse_in_range` when `rev(collection)` (without a `..` range) is parsed.
    /// Consumed by `fill_iter` to add the reverse bit (64) into the `on` byte of OpIterate/OpStep.
    reverse_iterator: bool,
    vars: Function,
    /// Last seen line inside the source code, an increase inserts it in the internal code.
    line: u32,
}

// Operators ordered on their precedence
static OPERATORS: &[&[&str]] = &[
    &["??"],
    &["||", "or"],
    &["&&", "and"],
    &["==", "!=", "<", "<=", ">", ">="],
    &["|"],
    &["^"],
    &["&"],
    &["<<", ">>"],
    &["-", "+"],
    &["*", "/", "%"],
    &["as"],
];

static SKIP_TOKEN: [&str; 8] = ["}", ".", "<", ">", "^", "+", "-", "#"];
static SKIP_WIDTH: [&str; 10] = ["}", ".", "x", "X", "o", "b", "e", "j", "d", "f"];

pub(crate) struct OutputState<'a> {
    pub(crate) radix: i32,
    pub(crate) width: Value,
    pub(crate) token: &'a str,
    pub(crate) plus: bool,
    pub(crate) note: bool,
    pub(crate) dir: i32,
    pub(crate) float: bool,
}

impl OutputState<'_> {
    pub(crate) fn db_format(&self) -> i32 {
        i32::from(self.note) + if self.radix < 0 { 2 } else { 0 }
    }
}

pub(crate) const OUTPUT_DEFAULT: OutputState = OutputState {
    radix: 10,
    width: Value::Int(0),
    token: " ",
    plus: false,
    note: false,
    dir: -1,
    float: false,
};

// Sub-modules
pub(super) mod builtins;
pub(super) mod collections;
pub(super) mod control;
pub(super) mod definitions;
pub(super) mod expressions;

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

fn is_op(name: &str) -> bool {
    name.len() >= 3 && name.starts_with("Op") && name.chars().nth(2).unwrap().is_uppercase()
}

/// Validate function, attribute, value, and field names
fn is_lower(name: &str) -> bool {
    for c in name.chars() {
        if c.is_uppercase() {
            return false;
        }
    }
    true
}

#[allow(dead_code)]
/// Used to validate constant names
fn is_upper(name: &str) -> bool {
    for c in name.chars() {
        if c.is_lowercase() {
            return false;
        }
    }
    true
}

/// Validate type, enum, enum values and struct names
fn is_camel(name: &str) -> bool {
    let c = name.chars().next().unwrap();
    if c.is_lowercase() {
        return false;
    }
    for c in name.chars() {
        if c == '_' {
            return false;
        }
    }
    true
}

impl Parser {
    #[must_use]
    pub fn new() -> Self {
        Parser {
            todo_files: Vec::new(),
            data: Data::new(),
            database: Stores::new(),
            lexer: Lexer::default(),
            in_loop: false,
            file: 1,
            diagnostics: Diagnostics::new(),
            default: false,
            context: u32::MAX,
            first_pass: true,
            reverse_iterator: false,
            vars: Function::new("", "none"),
            line: 0,
            lib_dirs: Vec::new(),
        }
    }

    /// Parse the content of a given file.
    /// - filename: the file to parse
    /// - default: parsing system definitions
    /// # Panics
    /// With filesystem problems.
    pub fn parse(&mut self, filename: &str, default: bool) -> bool {
        self.default = default;
        self.vars.logging = false;
        self.lexer.switch(filename);
        self.first_pass = true;
        self.data.reset();
        self.parse_file();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            self.data.reset();
            self.lexer.switch(filename);
            self.parse_file();
        }
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// Parse all .loft files found in a directory tree in alphabetical ordering.
    /// # Errors
    /// With filesystem problems.
    pub fn parse_dir(&mut self, dir: &str, default: bool, debug: bool) -> std::io::Result<()> {
        let paths = read_dir(dir)?;
        let mut files: BTreeSet<String> = BTreeSet::new();
        for path in paths {
            let p = path?;
            let own_file = p
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"));
            let file_name = p.path().to_string_lossy().to_string();
            let data = metadata(&file_name)?;
            if own_file || data.is_dir() {
                files.insert(file_name);
            }
        }
        for f in files {
            let types = self.database.types.len();
            let from = self.data.definitions();
            let data = metadata(&f)?;
            if data.is_dir() {
                self.parse_dir(&f, default, debug)?;
            } else if !self.parse(&f, default) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}", self.diagnostics),
                ));
            }
            scopes::check(&mut self.data);
            if debug {
                self.output(&f, types, from)?;
            }
        }
        Ok(())
    }

    fn output(&mut self, f: &str, types: usize, from: u32) -> std::io::Result<()> {
        let file = f.rsplit(['/', '\\']).next().unwrap_or(f);
        let to = format!("tests/dumps/{file}.txt");
        let _ = std::fs::create_dir_all("tests/dumps");
        if let Ok(mut w) = File::create(to.clone()) {
            let to = self.database.types.len();
            for tp in types..to {
                writeln!(w, "Type {tp}:{}", self.database.show_type(tp as u16, true))?;
            }
            for d_nr in from..self.data.definitions() {
                if self.data.def(d_nr).code == Value::Null {
                    continue;
                }
                write!(w, "{} ", self.data.def(d_nr).header(&self.data, d_nr))?;
                let mut vars = Function::copy(&self.data.def(d_nr).variables);
                self.data
                    .show_code(&mut w, &mut vars, &self.data.def(d_nr).code, 0, false)?;
                writeln!(w, "\n")?;
            }
        } else {
            diagnostic!(self.lexer, Level::Error, "Could not write: {to}");
        }
        Ok(())
    }

    /// Only parse a specific string, only useful for parser tests.
    #[allow(dead_code)]
    pub fn parse_str(&mut self, text: &str, filename: &str, logging: bool) {
        self.first_pass = true;
        self.default = false;
        self.vars.logging = logging;
        self.lexer.parse_string(text, filename);
        self.data.reset();
        self.parse_file();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            self.diagnostics.fill(self.lexer.diagnostics());
            return;
        }
        self.data.reset();
        self.lexer.parse_string(text, filename);
        self.first_pass = false;
        self.parse_file();
        self.diagnostics.fill(self.lexer.diagnostics());
    }

    // ********************
    // * Helper functions *
    // ********************

    /// Get an iterator.
    /// The iterable expression is in *code.
    /// Creating the iterator will be in *code afterward.
    /// Return the next expression; with `Value::None` the iterator creation was impossible.
    fn convert(&mut self, code: &mut Value, is_type: &Type, should: &Type) -> bool {
        if is_type.is_equal(should) {
            return true;
        }
        // Struct-literal inline constructors are typed as Rewritten(Reference(...)); strip
        // the wrapper so method calls chained on the constructor are accepted correctly.
        if let Type::Rewritten(inner) = is_type {
            return self.convert(code, inner, should);
        }
        if let (Type::Reference(ref_tp, _), Type::Enum(enum_tp, true, _)) = (is_type, should) {
            for a in &self.data.def(*enum_tp).attributes {
                if a.name == self.data.def(*ref_tp).name {
                    return true;
                }
            }
        }
        if let Type::RefVar(ref_tp) = is_type
            && self.convert(code, ref_tp, should)
        {
            return true;
        }
        if let Type::RefVar(ref_tp) = should
            && ref_tp.is_equal(is_type)
        {
            *code = self.cl("OpCreateStack", std::slice::from_ref(code));
            return true;
        }
        let mut check_type = is_type;
        let r = Type::Reference(self.data.def_nr("reference"), Vec::new());
        let e = Type::Enum(0, false, Vec::new());
        if let Type::Vector(_nr, _) = is_type {
            if let Type::Vector(v, _) = should
                && v.is_unknown()
            {
                return true;
            }
        } else if let Type::Reference(_, _) = is_type {
            if matches!(*should, Type::Reference(0, _)) {
                return true;
            }
            check_type = &r;
        } else if let Type::Enum(_, false, _) = is_type {
            if *should == e {
                return true;
            }
            check_type = &e;
        }
        for &dnr in self.data.get_possible("OpConv", &self.lexer) {
            if self.data.def(dnr).name.ends_with("FromNull") {
                if *is_type == Type::Null {
                    if self.data.def(dnr).returned == *should {
                        *code = Value::Call(dnr, vec![]);
                        return true;
                    } else if matches!(self.data.def(dnr).returned, Type::Reference(_, _))
                        && let Type::Reference(_, _) = *should
                    {
                        *code = Value::Call(dnr, vec![]);
                        return true;
                    }
                }
            } else if self.data.attributes(dnr) > 0
                && self.data.attr_type(dnr, 0).is_equal(check_type)
                && self.data.def(dnr).returned == *should
            {
                *code = Value::Call(dnr, vec![code.clone()]);
                return true;
            }
        }
        false
    }

    /// Cast a type to another type when possible
    /// Returns false when impossible.
    fn cast(&mut self, code: &mut Value, is_type: &Type, should: &Type) -> bool {
        if self.first_pass {
            return true;
        }
        let mut should_nr = self.data.type_def_nr(should);
        if let Type::Vector(c_tp, _) = should {
            let c_nr = self.data.type_def_nr(c_tp);
            let tp = self.database.vector(self.data.def(c_nr).known_type);
            should_nr = self.data.check_vector(c_nr, tp, self.lexer.pos());
        }
        let should_kt = if should_nr == u32::MAX {
            u16::MAX
        } else {
            self.data.def(should_nr).known_type
        };
        let is_nr = self.data.type_def_nr(is_type);
        let is_kt = if is_nr == u32::MAX {
            u16::MAX
        } else {
            self.data.def(is_nr).known_type
        };
        if let Type::Reference(tp, _) = should
            && self.data.def(*tp).returned.is_equal(is_type)
            && matches!(is_type, Type::Enum(_, true, _))
        {
            let get_e = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
            let get = self.cl("OpConvIntFromEnum", &[get_e]);
            if let Value::Enum(nr, _) = self.data.def(*tp).attributes[0].value {
                *code = v_if(
                    self.cl("OpEqInt", &[get, Value::Int(i32::from(nr))]),
                    code.clone(),
                    self.cl("OpConvRefFromNull", &[]),
                );
            }
            return true;
        }
        if matches!(is_type, Type::Text(_))
            && matches!(should, Type::Enum(_, true, _) | Type::Reference(_, _))
        {
            *code = self.cl(
                "OpCastVectorFromText",
                &[code.clone(), Value::Int(i32::from(should_kt))],
            );
            return true;
        }
        for &dnr in self.data.get_possible("OpCast", &self.lexer) {
            if self.data.attributes(dnr) == 1
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned.is_same(should)
            {
                if let Type::Enum(tp, false, _) = should {
                    *code = Value::Call(
                        dnr,
                        vec![
                            code.clone(),
                            Value::Int(i32::from(self.data.def(*tp).known_type)),
                        ],
                    );
                } else {
                    *code = Value::Call(dnr, vec![code.clone()]);
                }
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned.is_same(should)
                && should_kt != u16::MAX
            {
                *code = Value::Call(dnr, vec![code.clone(), Value::Int(i32::from(should_kt))]);
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned.is_same(should)
                && is_kt != u16::MAX
            {
                *code = Value::Call(dnr, vec![code.clone(), Value::Int(i32::from(is_kt))]);
                return true;
            }
        }
        false
    }

    /// Validate that two types are equal
    fn can_convert(&mut self, test_type: &Type, should: &Type) -> bool {
        if *test_type != *should && !test_type.is_unknown() {
            if let Type::RefVar(tp) = should
                && tp.is_equal(test_type)
            {
                return true;
            }
            if let (Type::Enum(_e, _, _), Type::Enum(o, _, _)) = (test_type, should)
                && self.data.def(*o).name == "enumerate"
            {
                return true;
            }
            if let (Type::Reference(r_nr, _), Type::Enum(e_nr, true, _)) = (test_type, should)
                && e_nr == r_nr
            {
                return true;
            }
            if let (Type::Enum(t, false, _), Type::Enum(s, false, _)) = (test_type, should)
                && *t == *s
            {
                return true;
            }
            if let (Type::Enum(_, false, _), Type::Integer(_, _)) = (test_type, should) {
                return true;
            }
            if let Type::Reference(r, _) = should
                && *r == self.data.def_nr("reference")
                && let Type::Reference(_, _) = test_type
            {
                return true;
            }
            false
        } else {
            true
        }
    }

    fn validate_convert(&mut self, context: &str, test_type: &Type, should: &Type) {
        if !self.first_pass && !self.can_convert(test_type, should) {
            let res = self.lexer.peek();
            specific!(
                &mut self.lexer,
                &res,
                Level::Error,
                "{} should be {} on {context}",
                test_type.name(&self.data),
                should.name(&self.data)
            );
        }
    }

    /// Search for definitions with the given name and call that with the given parameters.
    fn call(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        // Create a new list of parameters based on the current ones
        // We still need to know the types.
        let d_nr = if self.default && is_op(name) {
            self.data.def_nr(name)
        } else {
            self.data.find_fn(
                source,
                name,
                if types.is_empty() || types[0] == Type::Null {
                    &Type::Unknown(0)
                } else {
                    &types[0]
                },
            )
        };
        if d_nr != u32::MAX {
            self.call_nr(code, d_nr, list, types, true)
        } else if self.first_pass && !self.default {
            Type::Unknown(0)
        } else {
            diagnostic!(self.lexer, Level::Error, "Unknown function {name}");
            Type::Unknown(0)
        }
    }

    fn single_op(&mut self, op: &str, f: Value, t: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f], &[t]);
        code
    }

    fn conv_op(&mut self, op: &str, f: Value, n: Value, f_tp: Type, n_tp: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f, n], &[f_tp, n_tp]);
        code
    }

    fn op(&mut self, op: &str, f: Value, n: Value, t: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f, n], &[t.clone(), t]);
        code
    }

    fn get_field(&mut self, d_nr: u32, f_nr: usize, code: Value) -> Value {
        let tp = self.data.attr_type(d_nr, f_nr);
        let pos = if f_nr == usize::MAX {
            0
        } else {
            let nm = self.data.attr_name(d_nr, f_nr);
            self.database.position(self.data.def(d_nr).known_type, &nm)
        };
        self.get_val(
            &tp,
            self.data.attr_nullable(d_nr, f_nr),
            u32::from(pos),
            code,
        )
    }

    fn get_val(&mut self, tp: &Type, nullable: bool, pos: u32, code: Value) -> Value {
        let p = Value::Int(pos as i32);
        match tp {
            Type::Integer(min, _) => {
                let s = tp.size(nullable);
                if s == 1 {
                    self.cl("OpGetByte", &[code, p, Value::Int(*min)])
                } else if s == 2 {
                    self.cl("OpGetShort", &[code, p, Value::Int(*min)])
                } else {
                    self.cl("OpGetInt", &[code, p])
                }
            }
            Type::Enum(_, false, _) => self.cl("OpGetEnum", &[code, p]),
            Type::Boolean => {
                let val = self.cl("OpGetByte", &[code, p, Value::Int(0)]);
                self.cl("OpEqInt", &[val, Value::Int(1)])
            }
            Type::Long => self.cl("OpGetLong", &[code, p]),
            Type::Float => self.cl("OpGetFloat", &[code, p]),
            Type::Single => self.cl("OpGetSingle", &[code, p]),
            Type::Text(_) => self.cl("OpGetText", &[code, p]),
            Type::Hash(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::Spacial(_, _, _)
            | Type::Index(_, _, _)
            | Type::Enum(_, true, _)
            | Type::Vector(_, _) => self.cl("OpGetField", &[code, p, self.type_info(tp)]),
            Type::Reference(_, _) => {
                // This should only count for OpGetVector
                if let Value::Call(_, _) = code {
                    self.cl("OpGetRef", &[code, p])
                } else {
                    self.cl("OpGetField", &[code, p, self.type_info(tp)])
                }
            }
            _ => panic!(
                "Get not implemented on '{}' at {}",
                tp.name(&self.data),
                self.lexer.pos()
            ),
        }
    }

    fn set_field(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
    ) -> Value {
        let tp = self.data.attr_type(d_nr, f_nr);
        let nm = self.data.attr_name(d_nr, f_nr);
        let pos = self.database.position(self.data.def(d_nr).known_type, &nm);
        let pos_val = Value::Int(if f_nr == usize::MAX {
            i32::from(d_pos)
        } else {
            i32::from(pos + d_pos)
        });
        match tp {
            Type::Integer(min, _) => {
                let m = Value::Int(min);
                let s = tp.size(self.data.attr_nullable(d_nr, f_nr));
                if s == 1 {
                    self.cl("OpSetByte", &[ref_code, pos_val, m, val_code])
                } else if s == 2 {
                    self.cl("OpSetShort", &[ref_code, pos_val, m, val_code])
                } else {
                    self.cl("OpSetInt", &[ref_code, pos_val, val_code])
                }
            }
            Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Spacial(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::Character => self.cl("OpSetInt", &[ref_code, pos_val, val_code]),
            Type::Reference(inner_tp, _) => {
                // The value is a 12-byte DbRef; OpSetInt would only read 4 bytes of it.
                // Copy the struct bytes into the embedded field instead.
                let type_nr = if self.first_pass {
                    Value::Int(i32::from(u16::MAX))
                } else {
                    Value::Int(i32::from(self.data.def(inner_tp).known_type))
                };
                let field_ref = self.cl("OpGetField", &[ref_code, pos_val, type_nr.clone()]);
                self.cl("OpCopyRecord", &[val_code, field_ref, type_nr])
            }
            Type::Enum(_, false, _) => self.cl("OpSetEnum", &[ref_code, pos_val, val_code]),
            Type::Enum(nr, true, _) => self.cl(
                "OpCopyRecord",
                &[
                    val_code,
                    ref_code,
                    Value::Int(i32::from(self.data.def(nr).known_type)),
                ],
            ),
            Type::Boolean => {
                let v = v_if(val_code, Value::Int(1), Value::Int(0));
                self.cl("OpSetByte", &[ref_code, pos_val, Value::Int(0), v])
            }
            Type::Long => self.cl("OpSetLong", &[ref_code, pos_val, val_code]),
            Type::Float => self.cl("OpSetFloat", &[ref_code, pos_val, val_code]),
            Type::Single => self.cl("OpSetSingle", &[ref_code, pos_val, val_code]),
            Type::Text(_) => self.cl("OpSetText", &[ref_code, pos_val, val_code]),
            _ => {
                if self.first_pass {
                    Value::Null
                } else {
                    panic!(
                        "Set not implemented on {}/{} at {}",
                        self.data.attr_name(d_nr, f_nr),
                        self.data.attr_type(d_nr, f_nr).name(&self.data),
                        self.lexer.pos()
                    )
                }
            }
        }
    }

    fn cl(&mut self, op: &str, list: &[Value]) -> Value {
        let d_nr = self.data.def_nr(op);
        if d_nr == u32::MAX {
            diagnostic!(self.lexer, Level::Error, "Call to unknown {op}");
            Value::Null
        } else {
            Value::Call(d_nr, list.to_vec())
        }
    }

    /// Try to find a matching defined operator. There can be multiple possible definitions for each operator.
    fn call_op(&mut self, code: &mut Value, op: &str, list: &[Value], types: &[Type]) -> Type {
        let mut possible = Vec::new();
        for pos in self
            .data
            .get_possible(&format!("Op{}", rename(op)), &self.lexer)
        {
            possible.push(*pos);
        }
        for pos in possible {
            let tp = self.call_nr(code, pos, list, types, false);
            if tp != Type::Null {
                // We cannot compare two different types of enums, both will be integers in the same range
                if let (Some(Type::Enum(f, _, _)), Some(Type::Enum(s, _, _))) =
                    (types.first(), types.get(1))
                    && f != s
                {
                    break;
                }
                return tp;
            }
        }
        if types.len() > 1 {
            specific!(
                self.lexer,
                &self.lexer.peek(),
                Level::Error,
                "No matching operator '{op}' on '{}' and '{}'",
                types[0].name(&self.data),
                types[1].name(&self.data)
            );
        } else {
            specific!(
                self.lexer,
                &self.lexer.peek(),
                Level::Error,
                "No matching operator {op} on {}",
                types[0].name(&self.data)
            );
        }
        Type::Unknown(0)
    }

    /// Call a specific definition
    fn call_nr(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        list: &[Value],
        types: &[Type],
        report: bool,
    ) -> Type {
        let mut all_types = Vec::from(types);
        if self.data.def_type(d_nr) == DefType::Dynamic {
            for a_nr in 0..self.data.attributes(d_nr) {
                let Type::Routine(r_nr) = self.data.attr_type(d_nr, a_nr) else {
                    panic!("Incorrect Dynamic function {}", self.data.def(d_nr).name);
                };
                if self.data.attr_type(r_nr, 0).is_equal(&types[0]) {
                    return self.call_nr(code, r_nr, list, types, report);
                }
            }
            diagnostic!(
                self.lexer,
                Level::Error,
                "No matching function {}",
                self.data.def(d_nr).name
            );
        } else if !matches!(self.data.def_type(d_nr), DefType::Function) {
            if report {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown definition {}",
                    self.data.def(d_nr).name
                );
            }
            return Type::Null;
        }
        let mut actual: Vec<Value> = Vec::new();
        if !types.is_empty() {
            if list.len() > self.data.attributes(d_nr) {
                if report {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Too many parameters for {}",
                        self.data.def(d_nr).name
                    );
                }
                return Type::Null;
            }
            for (nr, a_code) in list.iter().enumerate() {
                let tp = self.data.attr_type(d_nr, nr);
                if let Some(actual_type) = types.get(nr) {
                    let mut actual_code = a_code.clone();
                    // When encountered a subtype reference, find the actual corresponding type
                    if let (Type::Vector(to_tp, _), Type::Vector(a_tp, _)) = (&tp, actual_type)
                        && a_tp.is_unknown()
                        && !to_tp.is_unknown()
                    {
                        self.change_var(&actual_code, &tp);
                        actual.push(actual_code);
                        continue;
                    }
                    if actual_type.is_unknown()
                        && let Type::Vector(_, _) = &tp
                    {
                        self.change_var(&actual_code, &tp);
                        actual.push(actual_code);
                        continue;
                    }
                    if let (Type::Integer(_, _), Type::Enum(_, true, _)) = (&tp, actual_type) {
                        // An enum with a structure is normally a reference to the data.
                        // But for compares we can expect to be a constant Enum value.
                        let cd = if matches!(actual_code, Value::Enum(_, _)) {
                            actual_code
                        } else {
                            self.cl("OpGetEnum", &[actual_code, Value::Int(0)])
                        };
                        actual.push(self.cl("OpConvIntFromEnum", &[cd]));
                        continue;
                    }
                    if !self.convert(&mut actual_code, actual_type, &tp) {
                        if report {
                            let context =
                                format!("call to {}", self.data.def(d_nr).original_name());
                            self.validate_convert(&context, actual_type, &tp);
                        } else if !self.can_convert(actual_type, &tp) {
                            return Type::Null;
                        }
                    }
                    actual.push(actual_code);
                }
            }
        }
        self.add_defaults(d_nr, &mut actual, &mut all_types);
        let tp = self.call_dependencies(d_nr, &all_types);
        *code = Value::Call(d_nr, actual);
        tp
    }

    // Gather depended on variables from arguments of the given called routine.
    fn call_dependencies(&mut self, d_nr: u32, types: &[Type]) -> Type {
        let tp = self.data.def(d_nr).returned.clone();
        if let Type::Text(d) = tp {
            Type::Text(Self::resolve_deps(types, &d))
        } else if let Type::Vector(to, d) = tp {
            Type::Vector(to, Self::resolve_deps(types, &d))
        } else if let Type::Sorted(to, key, d) = tp {
            Type::Sorted(to, key, Self::resolve_deps(types, &d))
        } else if let Type::Hash(to, key, d) = tp {
            Type::Hash(to, key, Self::resolve_deps(types, &d))
        } else if let Type::Index(to, key, d) = tp {
            Type::Index(to, key, Self::resolve_deps(types, &d))
        } else if let Type::Spacial(to, key, d) = tp {
            Type::Spacial(to, key, Self::resolve_deps(types, &d))
        } else if let Type::Reference(to, d) = tp {
            Type::Reference(to, Self::resolve_deps(types, &d))
        } else {
            tp
        }
    }

    fn resolve_deps(types: &[Type], d: &[u16]) -> Vec<u16> {
        let mut dp = HashSet::new();
        for ar in d {
            if *ar as usize >= types.len() {
                continue;
            }
            if let Type::Text(ad)
            | Type::Vector(_, ad)
            | Type::Sorted(_, _, ad)
            | Type::Hash(_, _, ad)
            | Type::Index(_, _, ad)
            | Type::Spacial(_, _, ad)
            | Type::Reference(_, ad) = &types[*ar as usize]
            {
                for a in ad {
                    dp.insert(*a);
                }
            }
        }
        Vec::from_iter(dp)
    }

    fn add_defaults(&mut self, d_nr: u32, actual: &mut Vec<Value>, all_types: &mut Vec<Type>) {
        if actual.len() < self.data.attributes(d_nr) {
            // Insert the default values for not given attributes
            for a_nr in actual.len()..self.data.attributes(d_nr) {
                let default = self.data.def(d_nr).attributes[a_nr].value.clone();
                let tp = self.data.attr_type(d_nr, a_nr);
                if let Type::Vector(content, _) = &tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    let vr = self.vars.work_refs(&tp, &mut self.lexer);
                    self.data.vector_def(&mut self.lexer, content);
                    all_types.push(Type::Vector(content.clone(), vec![vr]));
                    actual.push(Value::Var(vr));
                } else if let Type::Reference(content, _) = tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    let vr = self.vars.work_refs(&tp, &mut self.lexer);
                    all_types.push(Type::Reference(content, vec![vr]));
                    actual.push(Value::Var(vr));
                } else if let Type::RefVar(vtp) = &tp {
                    let mut ls = Vec::new();
                    let vr = if matches!(**vtp, Type::Text(_)) {
                        let wv = self.vars.work_text(&mut self.lexer);
                        if default != Value::Null
                            && if let Value::Text(t) = &default {
                                !t.is_empty()
                            } else {
                                true
                            }
                        {
                            ls.push(self.cl("OpAppendText", &[Value::Var(wv), default]));
                        }
                        wv
                    } else {
                        panic!("Unexpected reference type {}", vtp.name(&self.data));
                    };
                    ls.push(self.cl("OpCreateStack", &[Value::Var(vr)]));
                    actual.push(v_block(
                        ls,
                        Type::Reference(self.data.def_nr("reference"), vec![vr]),
                        "default ref",
                    ));
                    all_types.push(tp.clone());
                } else {
                    actual.push(default);
                    all_types.push(tp.clone());
                }
            }
        }
    }
    // ********************
    // * Parser functions *
    // ********************

    /// Parse data from the current lexer.
    fn parse_file(&mut self) {
        let start_def = self.data.definitions();
        while self.lexer.has_token("use") {
            if let Some(id) = self.lexer.has_identifier() {
                if self.data.use_exists(&id) {
                    self.lexer.token(";");
                    continue;
                }
                let f = self.lib_path(&id);
                if std::path::Path::new(&f).exists() {
                    let cur = &self.lexer.pos().file;
                    self.todo_files.push((cur.clone(), self.data.source));
                    self.data.use_add(&id);
                    self.lexer.switch(&f);
                } else {
                    diagnostic!(self.lexer, Level::Error, "Included file {id} not found");
                }
            }
        }
        self.file += 1;
        self.line = 0;
        loop {
            self.lexer.has_token("pub");
            if self.lexer.diagnostics().level() == Level::Fatal
                || (!self.parse_enum()
                    && !self.parse_typedef()
                    && !self.parse_function()
                    && !self.parse_struct()
                    && !self.parse_constant())
            {
                break;
            }
        }
        let res = self.lexer.peek();
        if res.has != LexItem::None && self.lexer.diagnostics().level() != Level::Fatal {
            if self.lexer.peek_token("use") {
                diagnostic!(
                    self.lexer,
                    Level::Fatal,
                    "use statements must appear before all definitions"
                );
            } else {
                diagnostic!(self.lexer, Level::Fatal, "Syntax error");
            }
        }
        typedef::actual_types(
            &mut self.data,
            &mut self.database,
            &mut self.lexer,
            start_def,
        );
        typedef::fill_all(&mut self.data, &mut self.database, start_def);
        self.database.finish();
        self.enum_fn();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            return;
        }
        // Parse all files left in the todo_files list, as they are halted to parse a use file.
        while let Some((t, s)) = self.todo_files.pop() {
            self.lexer.switch(&t);
            self.data.source = s;
            self.parse_file();
        }
    }

    fn lib_path(&mut self, id: &String) -> String {
        // - a source file, the lib directory in the project (project-supplied)
        let mut f = format!("lib/{id}.loft");
        if !std::path::Path::new(&f).exists() {
            f = format!("{id}.loft");
        }
        // Clone the file path so it is owned; slices of it won't borrow `self`,
        // allowing &mut self calls (lib_path_manifest) later in this method.
        let cur_script = self.lexer.pos().file.clone();
        let cur_dir = if let Some(p) = cur_script.rfind('/') {
            &cur_script[0..p]
        } else {
            ""
        };
        let base_dir = if cur_dir.contains("/tests/") {
            &cur_dir[..cur_dir.find("/tests/").unwrap()]
        } else {
            ""
        };
        // - a lib directory relative to the current directory
        if !cur_dir.is_empty() && !std::path::Path::new(&f).exists() {
            f = format!("{cur_dir}/lib/{id}.loft");
        }
        // - a lib directory relative to the base directory when inside /tests/
        if !base_dir.is_empty() && !std::path::Path::new(&f).exists() {
            f = format!("{base_dir}/lib/{id}.loft");
        }
        // - a directory with the same name of the current script
        if !std::path::Path::new(&f).exists() {
            f = format!("{}/{id}.loft", &cur_script[0..cur_script.len() - 5]);
        }
        // - extra library directories from --lib / --project command-line flags (single-file)
        if !std::path::Path::new(&f).exists() {
            for l in &self.lib_dirs {
                let candidate = format!("{l}/{id}.loft");
                if std::path::Path::new(&candidate).exists() {
                    f = candidate;
                    break;
                }
            }
        }
        // step 7c: packaged layout (<dir>/<id>/src/<id>.loft) in lib_dirs
        if !std::path::Path::new(&f).exists() {
            let lib_dirs = self.lib_dirs.clone();
            for l in &lib_dirs {
                if let Some(entry) = self.lib_path_manifest(l, id) {
                    f = entry;
                    break;
                }
            }
        }
        // - a user-defined lib directory (externally downloaded), single-file
        if !std::path::Path::new(&f).exists()
            && let Some(v) = env::var_os("LOFT_LIB")
        {
            let libs = v.to_str().unwrap().to_string();
            for l in libs.split(':') {
                let candidate = format!("{l}/{id}.loft");
                if std::path::Path::new(&candidate).exists() {
                    f = candidate;
                    break;
                }
            }
        }
        // step 7d: packaged layout in LOFT_LIB
        if !std::path::Path::new(&f).exists()
            && let Some(v) = env::var_os("LOFT_LIB")
        {
            let libs = v.to_str().unwrap().to_string();
            for l in libs.split(':') {
                if let Some(entry) = self.lib_path_manifest(l, id) {
                    f = entry;
                    break;
                }
            }
        }
        // - the current directory (beside the parsed file)
        if !cur_dir.is_empty() && !std::path::Path::new(&f).exists() {
            f = format!("{cur_dir}/{id}.loft");
        }
        // - the base directory when inside /tests/
        if !base_dir.is_empty() && !std::path::Path::new(&f).exists() {
            f = format!("{base_dir}/{id}.loft");
        }
        f
    }

    /// Check whether `<dir>/<id>` contains a valid loft package layout.
    /// Reads `loft.toml` when present and validates the interpreter version
    /// requirement.  Emits a fatal diagnostic on version mismatch.
    /// Returns `Some(entry_path)` when the layout exists and the version passes,
    /// `None` otherwise.
    fn lib_path_manifest(&mut self, dir: &str, id: &str) -> Option<String> {
        let pkg_dir = format!("{dir}/{id}");
        if !std::path::Path::new(&pkg_dir).is_dir() {
            return None;
        }
        let manifest_path = format!("{pkg_dir}/loft.toml");
        let entry = if std::path::Path::new(&manifest_path).exists() {
            let m = manifest::read_manifest(&manifest_path)?;
            if let Some(ref req) = m.loft_version {
                let current = env!("CARGO_PKG_VERSION");
                if !manifest::check_version(req, current) {
                    diagnostic!(
                        self.lexer,
                        Level::Fatal,
                        "Package '{id}' requires loft {req} but interpreter is {current}"
                    );
                    return None;
                }
            }
            m.entry.map_or_else(
                || format!("{pkg_dir}/src/{id}.loft"),
                |e| format!("{pkg_dir}/{e}"),
            )
        } else {
            format!("{pkg_dir}/src/{id}.loft")
        };
        if std::path::Path::new(&entry).exists() {
            Some(entry)
        } else {
            None
        }
    }

    // Determine if there need to be special enum functions that call enum_value variants.
    pub fn create_var(&mut self, name: &str, var_type: &Type) -> u16 {
        if self.context == u32::MAX {
            return u16::MAX;
        }
        self.vars.add_variable(name, var_type, &mut self.lexer)
    }

    fn create_unique(&mut self, name: &str, var_type: &Type) -> u16 {
        self.vars.unique(name, var_type, &mut self.lexer)
    }

    fn var_usages(&mut self, vnr: u16, plus: bool) {
        if vnr == u16::MAX {
            return;
        }
        if plus {
            self.vars.in_use(vnr, true);
        } else if self.vars.uses(vnr) > 0 {
            self.vars.in_use(vnr, false);
        }
    }

    /// After parsing a function body, check that each `&` (`RefVar`) argument is actually
    /// mutated somewhere in the body. If not, emit a compile error suggesting to drop the `&`.
    fn check_ref_mutations(&mut self, arguments: &[Argument]) {
        let code = self.data.def(self.context).code.clone();
        let mut written: HashSet<u16> = HashSet::new();
        find_written_vars(&code, &self.data, &mut written);
        for (a_nr, a) in arguments.iter().enumerate() {
            if matches!(a.typedef, Type::RefVar(_))
                && !a.constant
                && !written.contains(&(a_nr as u16))
            {
                let src = self.vars.var_source(a_nr as u16);
                self.lexer.to(src);
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Parameter '{}' has & but is never modified; remove the &",
                    a.name
                );
            }
        }
    }

    // <function> ::= 'fn' <identifier> '(' <attributes> ] [ '->' <type> ] (';' <rust> | <code>)
    pub fn null(&mut self, tp: &Type) -> Value {
        match tp {
            Type::Integer(_, _) | Type::Character => self.cl("OpConvIntFromNull", &[]),
            Type::Boolean => self.cl("OpConvBoolFromNull", &[]),
            Type::Enum(tp, _, _) => self.cl(
                "OpConvEnumFromNull",
                &[Value::Int(i32::from(self.data.def(*tp).known_type))],
            ),
            Type::Long => self.cl("OpConvLongFromNull", &[]),
            Type::Float => self.cl("OpConvFloatFromNull", &[]),
            Type::Single => self.cl("OpConvSingleFromNull", &[]),
            Type::Text(_) => self.cl("OpConvTextFromNull", &[]),
            Type::RefVar(tp) if matches!(**tp, Type::Text(_)) => self.cl("OpConvTextFromNull", &[]),
            Type::Reference(_, _) => self.cl("OpConvRefFromNull", &[]),
            _ => Value::Null,
        }
    }

    // For now, assume that returned texts are always related to internal variables
}

fn merge_dependencies(a: &Type, b: &Type) -> Type {
    if let (Type::Text(da), Type::Text(db)) = (a, b) {
        let mut d = HashSet::new();
        for v in da {
            d.insert(*v);
        }
        for v in db {
            d.insert(*v);
        }
        Type::Text(d.into_iter().collect())
    } else {
        a.clone()
    }
}

fn field_id(key: &[(String, bool)], name: &mut String) {
    for (k_nr, (k, asc)) in key.iter().enumerate() {
        if k_nr > 0 {
            *name += ",";
        }
        if !asc {
            *name += "-";
        }
        *name += k;
    }
    *name += "]>";
}

/// Collect all `Value::Var` indices reachable anywhere in `val`.
fn collect_vars_in(val: &Value, result: &mut HashSet<u16>) {
    match val {
        Value::Var(v) => {
            result.insert(*v);
        }
        Value::Set(_, body) => collect_vars_in(body, result),
        Value::Call(_, args) => {
            for a in args {
                collect_vars_in(a, result);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for op in &b.operators {
                collect_vars_in(op, result);
            }
        }
        Value::Insert(list) => {
            for item in list {
                collect_vars_in(item, result);
            }
        }
        Value::If(c, t, e) => {
            collect_vars_in(c, result);
            collect_vars_in(t, result);
            collect_vars_in(e, result);
        }
        Value::Return(v) | Value::Drop(v) => collect_vars_in(v, result),
        Value::Iter(_, a, b, c) => {
            collect_vars_in(a, result);
            collect_vars_in(b, result);
            collect_vars_in(c, result);
        }
        _ => {}
    }
}

/// Recursively walk a Value IR tree and collect all variable indices that are written.
/// A variable is considered written if:
/// - It appears as the target of `Value::Set(v, ...)`,
/// - It is passed as a `RefVar`-typed argument to a `Value::Call`, or
/// - It appears anywhere in the first argument of a field-write operator (`OpSet*`),
///   which covers the pattern `v[idx].field = val` where `v: &vector<T>`.
fn find_written_vars(code: &Value, data: &Data, written: &mut HashSet<u16>) {
    match code {
        Value::Set(v, body) => {
            written.insert(*v);
            find_written_vars(body, data, written);
        }
        Value::Call(fn_nr, args) => {
            let def = data.def(*fn_nr);
            let attrs = &def.attributes;
            // Stack-text mutation operators (OpAppendStackText, OpAppendStackCharacter,
            // OpClearStackText) write to their first argument via a const-u16 position.
            // OpAppendVector mutates the vector pointed to by its first Var argument.
            let stack_write = def.name.starts_with("OpAppendStack")
                || def.name.starts_with("OpClearStack")
                || def.name == "OpAppendVector";
            // Field-write and vector-append operators: any Var appearing in the first
            // argument is being mutated (e.g. v[idx].field = val, r += [x]).
            let field_write = def.name.starts_with("OpSet")
                || def.name == "OpNewRecord"
                || def.name == "OpAppendCopy";
            for (i, arg) in args.iter().enumerate() {
                if i < attrs.len()
                    && matches!(attrs[i].typedef, Type::RefVar(_))
                    && let Value::Var(v) = arg
                {
                    written.insert(*v);
                }
                if i == 0
                    && stack_write
                    && let Value::Var(v) = arg
                {
                    written.insert(*v);
                }
                if i == 0 && field_write {
                    collect_vars_in(arg, written);
                }
                find_written_vars(arg, data, written);
            }
        }
        Value::Block(block) | Value::Loop(block) => {
            for item in &block.operators {
                find_written_vars(item, data, written);
            }
        }
        Value::Insert(list) => {
            for item in list {
                find_written_vars(item, data, written);
            }
        }
        Value::If(cond, then, els) => {
            find_written_vars(cond, data, written);
            find_written_vars(then, data, written);
            find_written_vars(els, data, written);
        }
        Value::Return(v) | Value::Drop(v) => {
            find_written_vars(v, data, written);
        }
        Value::Iter(_, create, next, extra) => {
            find_written_vars(create, data, written);
            find_written_vars(next, data, written);
            find_written_vars(extra, data, written);
        }
        _ => {}
    }
}

fn rename(op: &str) -> &str {
    match op {
        "*" => "Mul",
        "+" => "Add",
        "-" => "Min",
        "/" => "Div",
        "&" => "Land",
        "|" => "Lor",
        "^" => "Eor",
        "<<" => "SLeft",
        ">>" => "SRight",
        "==" => "Eq",
        "!=" => "Ne",
        "<" => "Lt",
        "<=" => "Le",
        "%" => "Rem",
        "!" => "Not",
        "+=" => "Append",
        _ => op,
    }
}
