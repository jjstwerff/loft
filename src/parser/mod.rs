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
use crate::platform::{other_sep, sep, sep_str};
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
/// Whether a `use lib::...` statement imports all names or a specific subset.
enum ImportSpec {
    Wildcard,
    Names(Vec<String>),
}

/// A pending import queued when `use lib::spec` is parsed.
/// Applied after all definitions in `for_source` are fully parsed.
struct PendingImport {
    for_source: u16,
    lib_source: u16,
    spec: ImportSpec,
}

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
    /// True while parsing an expression inside a format string `{…}`.
    /// Prevents the `v: type = expr` annotation from consuming `:`.
    pub(crate) in_format_expr: bool,
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
    /// Wildcard and selective imports waiting to be applied once the target source is fully parsed.
    pending_imports: Vec<PendingImport>,
    /// Whether the most recently parsed expression is from a `not null` field access.
    /// Set by `get_field`; consumed by `handle_operator` to warn on redundant null checks.
    expr_not_null: bool,
    /// The field name for the most recently parsed `not null` field access (for diagnostics).
    expr_not_null_name: String,
    /// Counter incremented each time a lambda expression is parsed.
    /// Lambda names are `__lambda_N`; the same N is produced on both passes because the counter
    /// advances identically in both passes (same token order → same parse order).
    pub lambda_counter: u32,
    /// Expected `Type::Function(params, ret)` for the argument currently being parsed.
    /// Set by `parse_call` before parsing a function-typed argument so that short-form
    /// lambdas (`|x| { … }`) can infer parameter types from the call-site context.
    /// Cleared to `Type::Unknown(0)` immediately after the argument is parsed.
    pub(crate) lambda_hint: Type,
    /// A10: set by `iter_op` when `#fields` is encountered. Holds the struct `def_nr`.
    /// Checked by `parse_for` to take the unrolling path. Reset after use.
    pub(crate) fields_of: u32,
    /// A5.1: outer-scope variable names and types, populated when parsing a lambda body.
    /// When a variable is not found in the lambda's scope but exists here, it is a capture.
    /// Empty when not inside a lambda.
    pub(crate) capture_context: Vec<(String, Type)>,
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
pub(super) mod fields;
pub(super) mod objects;
pub(super) mod operators;
pub(super) mod vectors;

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
        let mut data = Data::new();
        // Register internal-only functions (i_ prefix) that are never visible to user code.
        // These are resolved by the compiler via data.def_nr("i_...") and mapped to native
        // Rust implementations in native.rs.
        let pos = Position {
            file: String::new(),
            line: 0,
            pos: 0,
        };
        let d = data.add_def("i_parse_errors", &pos, DefType::Function);
        data.definitions[d as usize].returned = Type::Text(Vec::new());
        let d = data.add_def("i_parse_error_push", &pos, DefType::Function);
        data.definitions[d as usize].returned = Type::Void;
        {
            let mut lexer = Lexer::default();
            data.add_attribute(&mut lexer, d, "msg", Type::Text(Vec::new()));
        }
        Parser {
            todo_files: Vec::new(),
            data,
            database: Stores::new(),
            lexer: Lexer::default(),
            in_loop: false,
            in_format_expr: false,
            file: 1,
            diagnostics: Diagnostics::new(),
            default: false,
            context: u32::MAX,
            first_pass: true,
            reverse_iterator: false,
            vars: Function::new("", "none"),
            line: 0,
            lib_dirs: Vec::new(),
            pending_imports: Vec::new(),
            expr_not_null: false,
            expr_not_null_name: String::new(),
            lambda_counter: 0,
            lambda_hint: Type::Unknown(0),
            fields_of: u32::MAX,
            capture_context: Vec::new(),
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
        self.pending_imports.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.parse_file();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            self.data.reset();
            self.lambda_counter = 0;
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
        let f_norm = f.replace(other_sep(), sep_str());
        let file = f_norm.rsplit(sep()).next().unwrap_or(f);
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
        self.lambda_counter = 0;
        self.parse_file();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            self.diagnostics.fill(self.lexer.diagnostics());
            return;
        }
        self.data.reset();
        self.lambda_counter = 0;
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
            if matches!(**ref_tp, Type::Text(_)) {
                // Text → &text: use OpCreateStack for plain variables (write-back),
                // allocate a work-text copy for complex expressions (read-only).
                let orig = std::mem::replace(code, Value::Null);
                if let Value::Var(_) = &orig {
                    *code = self.cl("OpCreateStack", &[orig]);
                } else {
                    let wv = self.vars.work_text(&mut self.lexer);
                    let mut ls = Vec::new();
                    if orig != Value::Text(String::new()) {
                        ls.push(self.cl("OpAppendText", &[Value::Var(wv), orig]));
                    }
                    ls.push(self.cl("OpCreateStack", &[Value::Var(wv)]));
                    *code = v_block(
                        ls,
                        Type::Reference(self.data.def_nr("reference"), vec![wv]),
                        "text_ref",
                    );
                }
            } else {
                *code = self.cl("OpCreateStack", std::slice::from_ref(code));
            }
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
                    if matches!(self.data.def(dnr).returned, Type::Reference(_, _))
                        && let Type::Reference(_, _) = *should
                    {
                        // Use the non-allocating sentinel instead of OpConvRefFromNull so that
                        // null comparisons (`s == null`, `s != null`) do not leak a store.
                        let sentinel_nr = self.data.def_nr("OpNullRefSentinel");
                        *code = Value::Call(sentinel_nr, vec![]);
                        return true;
                    } else if self.data.def(dnr).returned == *should {
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

    /// P5.3: Check if a type is a generic type variable (a dummy struct used as T).
    /// Returns the type variable name if it is, None otherwise.
    pub(crate) fn generic_type_name(&self, tp: &Type) -> Option<&str> {
        if let Type::Reference(d, _) = tp {
            let d = *d as usize;
            if d < self.data.definitions.len()
                && self.data.definitions[d].def_type == DefType::Struct
                && self.data.definitions[d].attributes.is_empty()
                && self.context != u32::MAX
                && self.data.definitions[self.context as usize].def_type == DefType::Generic
            {
                return Some(&self.data.definitions[d].name);
            }
        }
        None
    }

    /// Search for definitions with the given name and call that with the given parameters.
    fn call(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
        list: &[Value],
        types: &[Type],
        named_args: &[(String, Value, Type)],
    ) -> Type {
        // Create a new list of parameters based on the current ones
        // We still need to know the types.
        let mut d_nr = if self.default && is_op(name) {
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
        // P5.2: skip generic templates — they are not callable directly.
        if d_nr != u32::MAX && self.data.def(d_nr).def_type == DefType::Generic {
            d_nr = u32::MAX;
        }
        // P5.2: if no exact match, try generic instantiation.
        if d_nr == u32::MAX && !self.first_pass && !self.default {
            d_nr = self.try_generic_instantiation(name, types);
        }
        if d_nr != u32::MAX {
            self.call_with_named(code, d_nr, list, types, named_args, true)
        } else if self.first_pass && !self.default {
            Type::Unknown(0)
        } else {
            // P5.3: generic-specific error for method calls on T.
            if let Some(tv_name) = types.first().and_then(|t| self.generic_type_name(t)) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "generic type {tv_name}: method call requires a concrete type",
                );
            } else {
                diagnostic!(self.lexer, Level::Error, "Unknown function {name}");
            }
            Type::Unknown(0)
        }
    }

    /// P5.2: Try to instantiate a generic function template for the given call-site types.
    /// Returns the `def_nr` of the instantiated function, or `u32::MAX` if no generic matches.
    fn try_generic_instantiation(&mut self, name: &str, types: &[Type]) -> u32 {
        let generic_name = format!("n_{name}");
        let g_nr = self.data.def_nr(&generic_name);
        if g_nr == u32::MAX || self.data.def(g_nr).def_type != DefType::Generic {
            return u32::MAX;
        }
        if types.is_empty() || types[0].is_unknown() {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot infer type for generic parameter — provide an explicit type annotation"
            );
            return u32::MAX;
        }
        // Find the type variable def_nr and resolve the concrete type T maps to.
        let tv_nr = Self::extract_type_var(&self.data.def(g_nr).attributes[0].typedef);
        if tv_nr == u32::MAX {
            return u32::MAX;
        }
        let concrete =
            Self::resolve_type_var(&self.data.def(g_nr).attributes[0].typedef, tv_nr, &types[0]);
        if concrete.is_unknown() {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot resolve generic type parameter from argument type"
            );
            return u32::MAX;
        }
        // Build the mangled name for the instantiated function.
        let type_nr = self.data.type_def_nr(&concrete);
        let mangled = if type_nr == u32::MAX {
            format!("n_{name}")
        } else {
            format!(
                "t_{}{}_{name}",
                self.data.def(type_nr).name.len(),
                self.data.def(type_nr).name
            )
        };
        // Return existing instantiation if already created.
        let existing = self.data.def_nr(&mangled);
        if existing != u32::MAX {
            return existing;
        }
        // Clone the template data before mutating self.data.
        let tmpl_code = self.data.definitions[g_nr as usize].code.clone();
        let tmpl_returned = self.data.definitions[g_nr as usize].returned.clone();
        let tmpl_attrs: Vec<_> = self.data.definitions[g_nr as usize]
            .attributes
            .iter()
            .map(|a| Argument {
                name: a.name.clone(),
                typedef: Self::substitute_type(a.typedef.clone(), tv_nr, &concrete),
                default: a.value.clone(),
                constant: false,
            })
            .collect();
        let tmpl_vars = self.data.definitions[g_nr as usize].variables.clone();
        let tmpl_pos = self.data.definitions[g_nr as usize].position.clone();
        let new_code = Self::substitute_type_in_value(tmpl_code, tv_nr, &concrete, &self.data);
        let new_returned = Self::substitute_type(tmpl_returned, tv_nr, &concrete);
        // Register the new definition.
        let d_nr = self.data.add_def(&mangled, &tmpl_pos, DefType::Function);
        for a in &tmpl_attrs {
            let a_nr = self
                .data
                .add_attribute(&mut self.lexer, d_nr, &a.name, a.typedef.clone());
            self.data.set_attr_value(d_nr, a_nr, a.default.clone());
        }
        self.data.definitions[d_nr as usize].code = new_code;
        self.data.set_returned(d_nr, new_returned);
        // Copy the variable table with substituted types.
        let mut vars = Function::copy(&tmpl_vars);
        vars.substitute_type(tv_nr, &concrete);
        self.data.definitions[d_nr as usize].variables = vars;
        d_nr
    }

    /// Extract the type variable `def_nr` from a type tree.
    /// Returns the `def_nr` of the first `Reference` that refers to the type variable,
    /// or `u32::MAX` if not found.
    fn extract_type_var(tp: &Type) -> u32 {
        match tp {
            Type::Reference(d, _) => *d,
            Type::Vector(inner, _) => Self::extract_type_var(inner),
            _ => u32::MAX,
        }
    }

    /// Unify a template parameter type with a concrete argument type to extract
    /// what the type variable `tv_nr` resolves to.
    /// E.g. template `vector<T>` + concrete `vector<integer>` → `integer`.
    fn resolve_type_var(template_tp: &Type, tv_nr: u32, concrete_tp: &Type) -> Type {
        match template_tp {
            Type::Reference(d, _) if *d == tv_nr => concrete_tp.clone(),
            Type::Vector(inner, _) => {
                if let Type::Vector(c_inner, _) = concrete_tp {
                    Self::resolve_type_var(inner, tv_nr, c_inner)
                } else {
                    Type::Unknown(0)
                }
            }
            _ => Type::Unknown(0),
        }
    }

    /// Re-resolve a Call target: if the called function's first parameter references
    /// the type variable, look up the correct overload for the concrete type.
    fn re_resolve_call(d_nr: u32, tv_nr: u32, concrete: &Type, data: &Data) -> u32 {
        if d_nr == u32::MAX || d_nr as usize >= data.definitions.len() {
            return d_nr;
        }
        let def = &data.definitions[d_nr as usize];
        if def.attributes.is_empty() {
            return d_nr;
        }
        // Check if any attribute's type references the type variable.
        let has_tv = def
            .attributes
            .iter()
            .any(|a| Self::type_contains_tv(&a.typedef, tv_nr));
        if !has_tv {
            // Also check for Integer(0, tv_nr) patterns — operators sometimes encode
            // type info in the Integer bounds.
            return d_nr;
        }
        // Resolve the concrete first-arg type by substituting tv_nr in the attribute type.
        let concrete_arg =
            Self::substitute_type(def.attributes[0].typedef.clone(), tv_nr, concrete);
        // Extract the user-facing function name from the mangled definition name.
        // Mangled names: "t_<LEN><Type>_<name>" or "n_<name>" or operator names.
        let name = &def.name;
        let fn_name = if let Some(rest) = name.strip_prefix("t_") {
            // Skip the LEN digits and type name, extract name after the underscore.
            if let Some(idx) = rest.find('_') {
                &rest[idx + 1..]
            } else {
                name.as_str()
            }
        } else if let Some(rest) = name.strip_prefix("n_") {
            rest
        } else {
            // Operator name — use as-is for find_fn.
            name.as_str()
        };
        let resolved = data.find_fn(u16::MAX, fn_name, &concrete_arg);
        if resolved != u32::MAX && resolved != d_nr {
            resolved
        } else {
            d_nr
        }
    }

    /// Check if a type references the type variable.
    fn type_contains_tv(tp: &Type, tv_nr: u32) -> bool {
        match tp {
            Type::Reference(d, _) | Type::Unknown(d) => *d == tv_nr,
            Type::Vector(inner, _) => Self::type_contains_tv(inner, tv_nr),
            _ => false,
        }
    }

    /// Substitute all occurrences of `Type::Reference(tv_nr, _)` with `concrete` in a type.
    fn substitute_type(tp: Type, tv_nr: u32, concrete: &Type) -> Type {
        match tp {
            Type::Reference(d, _) if d == tv_nr => concrete.clone(),
            Type::Vector(inner, deps) => Type::Vector(
                Box::new(Self::substitute_type(*inner, tv_nr, concrete)),
                deps,
            ),
            other => other,
        }
    }

    /// Recursively substitute types in a Value IR tree and re-resolve Call targets
    /// whose first parameter references the type variable.
    fn substitute_type_in_value(val: Value, tv_nr: u32, concrete: &Type, data: &Data) -> Value {
        match val {
            Value::Call(d, args) => {
                let new_args: Vec<_> = args
                    .into_iter()
                    .map(|a| Self::substitute_type_in_value(a, tv_nr, concrete, data))
                    .collect();
                // Re-resolve call target if it references the type variable.
                let new_d = Self::re_resolve_call(d, tv_nr, concrete, data);
                Value::Call(new_d, new_args)
            }
            Value::Block(bl) => Value::Block(Box::new(crate::data::Block {
                operators: bl
                    .operators
                    .into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, data))
                    .collect(),
                result: Self::substitute_type(bl.result, tv_nr, concrete),
                name: bl.name,
                scope: bl.scope,
                var_size: bl.var_size,
            })),
            Value::Set(v, expr) => Value::Set(
                v,
                Box::new(Self::substitute_type_in_value(*expr, tv_nr, concrete, data)),
            ),
            Value::Return(expr) => Value::Return(Box::new(Self::substitute_type_in_value(
                *expr, tv_nr, concrete, data,
            ))),
            Value::If(cond, t, f) => Value::If(
                Box::new(Self::substitute_type_in_value(*cond, tv_nr, concrete, data)),
                Box::new(Self::substitute_type_in_value(*t, tv_nr, concrete, data)),
                Box::new(Self::substitute_type_in_value(*f, tv_nr, concrete, data)),
            ),
            Value::Loop(bl) => Value::Loop(Box::new(crate::data::Block {
                operators: bl
                    .operators
                    .into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, data))
                    .collect(),
                result: Self::substitute_type(bl.result, tv_nr, concrete),
                name: bl.name,
                scope: bl.scope,
                var_size: bl.var_size,
            })),
            Value::Drop(expr) => Value::Drop(Box::new(Self::substitute_type_in_value(
                *expr, tv_nr, concrete, data,
            ))),
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, data))
                    .collect(),
            ),
            Value::Iter(name, create, next, extra) => Value::Iter(
                name,
                Box::new(Self::substitute_type_in_value(
                    *create, tv_nr, concrete, data,
                )),
                Box::new(Self::substitute_type_in_value(*next, tv_nr, concrete, data)),
                Box::new(Self::substitute_type_in_value(
                    *extra, tv_nr, concrete, data,
                )),
            ),
            other => other,
        }
    }

    /// Resolve named arguments into positional slots, then delegate to `call_nr`.
    fn call_with_named(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        positional: &[Value],
        pos_types: &[Type],
        named: &[(String, Value, Type)],
        is_method: bool,
    ) -> Type {
        if named.is_empty() {
            return self.call_nr(code, d_nr, positional, pos_types, is_method);
        }
        // Build full argument vector with named args placed at the correct indices.
        let n_params = self.data.attributes(d_nr);
        let mut args = vec![Value::Null; n_params];
        let mut arg_types = vec![Type::Unknown(0); n_params];
        // Place positional args first.
        for (i, (val, tp)) in positional.iter().zip(pos_types.iter()).enumerate() {
            if i < n_params {
                args[i] = val.clone();
                arg_types[i] = tp.clone();
            }
        }
        let pos_count = positional.len();
        // Place named args by looking up parameter names.
        for (name, val, tp) in named {
            let idx = self.data.attr(d_nr, name);
            if idx == usize::MAX {
                if !self.first_pass {
                    diagnostic!(self.lexer, Level::Error, "Unknown parameter '{name}'");
                }
                continue;
            }
            if idx < pos_count {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Parameter '{name}' already provided as positional argument {idx}"
                    );
                }
                continue;
            }
            if args[idx] != Value::Null {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Duplicate named argument '{name}'"
                    );
                }
                continue;
            }
            args[idx] = val.clone();
            arg_types[idx] = tp.clone();
        }
        // Trim trailing Null args — add_defaults will fill them.
        let mut last_provided = args.len();
        while last_provided > 0 && args[last_provided - 1] == Value::Null {
            last_provided -= 1;
        }
        args.truncate(last_provided);
        arg_types.truncate(last_provided);
        self.call_nr(code, d_nr, &args, &arg_types, is_method)
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
        let nullable = self.data.attr_nullable(d_nr, f_nr);
        self.expr_not_null = !nullable;
        if !nullable && f_nr != usize::MAX {
            self.expr_not_null_name = self.data.attr_name(d_nr, f_nr);
        } else {
            self.expr_not_null_name.clear();
        }
        let pos = if f_nr == usize::MAX {
            0
        } else {
            let nm = self.data.attr_name(d_nr, f_nr);
            self.database.position(self.data.def(d_nr).known_type, &nm)
        };
        self.get_val(&tp, nullable, u32::from(pos), code)
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
            _ => {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Field access not supported on type {}",
                    tp.name(&self.data)
                );
                Value::Null
            }
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
        self.set_field_check(d_nr, f_nr, d_pos, ref_code, val_code, true)
    }

    fn set_field_no_check(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
    ) -> Value {
        self.set_field_check(d_nr, f_nr, d_pos, ref_code, val_code, false)
    }

    fn set_field_check(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
        emit_check: bool,
    ) -> Value {
        let tp = self.data.attr_type(d_nr, f_nr);
        let nm = self.data.attr_name(d_nr, f_nr);
        let pos = self.database.position(self.data.def(d_nr).known_type, &nm);
        let pos_val = Value::Int(if f_nr == usize::MAX {
            i32::from(d_pos)
        } else {
            i32::from(pos + d_pos)
        });
        let has_check = emit_check
            && f_nr != usize::MAX
            && !self.first_pass
            && self
                .data
                .def(d_nr)
                .attributes
                .get(f_nr)
                .is_some_and(|a| a.check != Value::Null);
        let ref_for_check = if has_check {
            Some(ref_code.clone())
        } else {
            None
        };
        let set_op = match tp {
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
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot assign to field '{}' of type {}",
                        self.data.attr_name(d_nr, f_nr),
                        self.data.attr_type(d_nr, f_nr).name(&self.data)
                    );
                    Value::Null
                }
            }
        };
        self.emit_field_constraint(set_op, ref_for_check, d_nr, f_nr, &nm)
    }

    /// Wrap a set operation with a constraint assertion if the field has one.
    fn emit_field_constraint(
        &mut self,
        set_op: Value,
        ref_for_check: Option<Value>,
        d_nr: u32,
        f_nr: usize,
        field_name: &str,
    ) -> Value {
        let Some(ref_val) = ref_for_check else {
            return set_op;
        };
        let check = self.data.def(d_nr).attributes[f_nr].check.clone();
        let bound = Self::replace_record_ref(check, &ref_val);
        let msg = if let Value::Text(s) = &self.data.def(d_nr).attributes[f_nr].check_message {
            Value::Text(s.clone())
        } else {
            Value::Text(format!(
                "field constraint failed on {}.{field_name}",
                self.data.def(d_nr).name
            ))
        };
        let assert_dnr = self.data.def_nr("n_assert");
        let pos = self.lexer.pos();
        let assert_call = Value::Call(
            assert_dnr,
            vec![
                bound,
                msg,
                Value::Text(pos.file.clone()),
                Value::Int(pos.line as i32),
            ],
        );
        Value::Insert(vec![set_op, assert_call])
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
        // P5.3: generic-specific error message for operators on T.
        let generic_name = types.iter().find_map(|t| self.generic_type_name(t));
        if let Some(tv_name) = generic_name {
            specific!(
                self.lexer,
                &self.lexer.peek(),
                Level::Error,
                "generic type {tv_name}: operator '{op}' requires a concrete type",
            );
        } else if types.len() > 1 {
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
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Incorrect dynamic function {}",
                        self.data.def(d_nr).name
                    );
                    return Type::Void;
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
        let mut actual = self.process_call_args(d_nr, list, types, &mut all_types, report);
        if actual.is_empty() && !types.is_empty() {
            return Type::Null;
        }
        self.add_defaults(d_nr, &mut actual, &mut all_types);
        let tp = self.call_dependencies(d_nr, &all_types);
        *code = Value::Call(d_nr, actual);
        tp
    }

    /// Convert and validate each positional argument for a call.
    fn process_call_args(
        &mut self,
        d_nr: u32,
        list: &[Value],
        types: &[Type],
        all_types: &mut [Type],
        report: bool,
    ) -> Vec<Value> {
        let mut actual = Vec::new();
        if types.is_empty() {
            return actual;
        }
        if list.len() > self.data.attributes(d_nr) {
            if report {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Too many parameters for {}",
                    self.data.def(d_nr).name
                );
            }
            return actual;
        }
        for (nr, a_code) in list.iter().enumerate() {
            let tp = self.data.attr_type(d_nr, nr);
            let Some(actual_type) = types.get(nr) else {
                continue;
            };
            let mut actual_code = a_code.clone();
            if let (Type::Vector(to_tp, _), Type::Vector(a_tp, _)) = (&tp, actual_type)
                && a_tp.is_unknown()
                && !to_tp.is_unknown()
            {
                self.change_var(&actual_code, &tp);
                actual.push(actual_code);
                continue;
            }
            // empty `[]` literal → create temp vector where parameter type is known.
            if matches!(&actual_code, Value::Insert(ops) if ops.len() <= 1)
                && let Type::Vector(elm_tp, dep) = &tp
            {
                let vec = self.create_unique("vec", &Type::Vector(elm_tp.clone(), dep.clone()));
                let mut ls = self.vector_db(elm_tp, vec);
                ls.push(Value::Var(vec));
                actual.push(v_block(ls, tp.clone(), "empty_vector_arg"));
                all_types[nr] = tp.clone();
                continue;
            }
            if actual_type.is_unknown() && matches!(&tp, Type::Vector(_, _)) {
                self.change_var(&actual_code, &tp);
                actual.push(actual_code);
                continue;
            }
            if let (Type::Integer(_, _), Type::Enum(_, true, _)) = (&tp, actual_type) {
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
                    let context = format!("call to {}", self.data.def(d_nr).original_name());
                    self.validate_convert(&context, actual_type, &tp);
                } else if !self.can_convert(actual_type, &tp) {
                    return Vec::new();
                }
            }
            actual.push(actual_code);
        }
        actual
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
        // When filling extra attrs for a recursive self-call on the second pass, use a
        // separate __rref_N counter so we don't consume __ref_N slots that the outer
        // function's return-value work-ref needs to keep the same name it had on the
        // first pass (allowing ref_return to find the name match instead of adding a
        // new attribute and growing the function's attr count across passes).
        let is_recursive_self = d_nr == self.context && !self.first_pass;
        // Extend to full parameter count so we can fill gaps from named arguments.
        while actual.len() < self.data.attributes(d_nr) {
            actual.push(Value::Null);
            all_types.push(Type::Unknown(0));
        }
        {
            // Fill all missing (Null) parameter slots with defaults.
            for a_nr in 0..self.data.attributes(d_nr) {
                if actual[a_nr] != Value::Null {
                    continue;
                }
                let default = self.data.def(d_nr).attributes[a_nr].value.clone();
                let tp = self.data.attr_type(d_nr, a_nr);
                if let Type::Vector(content, _) = &tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    let vr = if is_recursive_self {
                        self.vars.work_refs_recursive(&tp, &mut self.lexer)
                    } else {
                        self.vars.work_refs(&tp, &mut self.lexer)
                    };
                    self.data.vector_def(&mut self.lexer, content);
                    all_types[a_nr] = Type::Vector(content.clone(), vec![vr]);
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::Reference(content, _) = tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    let vr = if is_recursive_self {
                        self.vars.work_refs_recursive(&tp, &mut self.lexer)
                    } else {
                        self.vars.work_refs(&tp, &mut self.lexer)
                    };
                    all_types[a_nr] = Type::Reference(content, vec![vr]);
                    actual[a_nr] = Value::Var(vr);
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
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unexpected reference type {}",
                            vtp.name(&self.data)
                        );
                        0
                    };
                    ls.push(self.cl("OpCreateStack", &[Value::Var(vr)]));
                    actual[a_nr] = v_block(
                        ls,
                        Type::Reference(self.data.def_nr("reference"), vec![vr]),
                        "default ref",
                    );
                    all_types[a_nr] = tp.clone();
                } else {
                    actual[a_nr] = default;
                    all_types[a_nr] = tp.clone();
                }
            }
        }
    }
    // ********************
    // * Parser functions *
    // ********************

    /// Parse data from the current lexer.
    #[allow(clippy::too_many_lines)] // two-pass parser dispatch — splitting would lose context
    fn parse_file(&mut self) {
        let start_def = self.data.definitions();
        while self.lexer.has_token("use") {
            if let Some(id) = self.lexer.has_identifier() {
                // Parse optional import spec: `::*` for wildcard or `::name1, name2` for selective.
                let spec = if self.lexer.has_token("::") {
                    if self.lexer.has_token("*") {
                        Some(ImportSpec::Wildcard)
                    } else {
                        let mut names = Vec::new();
                        if let Some(name) = self.lexer.has_identifier() {
                            names.push(name);
                            while self.lexer.has_token(",") {
                                if let Some(name) = self.lexer.has_identifier() {
                                    names.push(name);
                                }
                            }
                        }
                        if names.is_empty() {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Expected name or '*' after '::'"
                            );
                            None
                        } else {
                            Some(ImportSpec::Names(names))
                        }
                    }
                } else {
                    None
                };
                if self.data.use_exists(&id) {
                    if let Some(s) = spec {
                        let lib_source = self.data.get_source(&id);
                        self.pending_imports.push(PendingImport {
                            for_source: self.data.source,
                            lib_source,
                            spec: s,
                        });
                    }
                    self.lexer.token(";");
                    continue;
                }
                let f = self.lib_path(&id);
                if std::path::Path::new(&f).exists() {
                    let cur = &self.lexer.pos().file;
                    self.todo_files.push((cur.clone(), self.data.source));
                    self.data.use_add(&id);
                    // spec is consumed (tokens already read); the import will be recorded
                    // when this `use` statement is seen again via todo_files with use_exists=true.
                    drop(spec);
                    self.lexer.switch(&f);
                } else {
                    diagnostic!(self.lexer, Level::Error, "Included file {id} not found");
                }
            }
        }
        // Apply wildcard/selective imports queued for this source now that the while-use loop
        // has resolved all libraries.  Must run before the definitions loop so that imported
        // names are visible when function bodies and type annotations are parsed.
        self.apply_pending_imports();
        self.file += 1;
        self.line = 0;
        loop {
            let is_pub = self.lexer.has_token("pub");
            let before = self.data.definitions();
            if self.lexer.diagnostics().level() == Level::Fatal
                || (!self.parse_enum()
                    && !self.parse_typedef()
                    && !self.parse_function()
                    && !self.parse_struct()
                    && !self.parse_constant())
            {
                break;
            }
            // mark newly created definitions as pub-visible.
            if is_pub {
                for d_nr in before..self.data.definitions() {
                    self.data.def_mut(d_nr).pub_visible = true;
                }
            }
        }
        let res = self.lexer.peek();
        if res.has != LexItem::None && self.lexer.diagnostics().level() != Level::Fatal {
            if self.lexer.peek_token("use") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "use statements must appear before all definitions"
                );
            } else {
                let token = format!("{:?}", res.has);
                diagnostic!(self.lexer, Level::Error, "Syntax error: unexpected {token}");
            }
        }
        typedef::actual_types(
            &mut self.data,
            &mut self.database,
            &mut self.lexer,
            start_def,
        );
        typedef::fill_all(
            &mut self.data,
            &mut self.database,
            &mut self.lexer,
            start_def,
        );
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

    /// Apply all pending imports whose target source matches the currently active source.
    fn apply_pending_imports(&mut self) {
        let cur = self.data.source;
        // Partition: imports targeting `cur` are applied now; others wait for their source.
        let mut to_apply = Vec::new();
        let mut remaining = Vec::new();
        for pi in self.pending_imports.drain(..) {
            if pi.for_source == cur {
                to_apply.push(pi);
            } else {
                remaining.push(pi);
            }
        }
        self.pending_imports = remaining;
        for pi in to_apply {
            match pi.spec {
                ImportSpec::Wildcard => {
                    self.data.import_all(pi.lib_source, cur);
                }
                ImportSpec::Names(names) => {
                    for name in &names {
                        if !self.data.import_name(pi.lib_source, cur, name) {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Name '{name}' not found in library"
                            );
                        }
                    }
                }
            }
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
        // Normalise to the platform separator (sep()) so that rfind / contains
        // use a single token rather than probing for both '/' and '\\'.
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let cur_dir = if let Some(p) = cur_script.rfind(sep()) {
            &cur_script[0..p]
        } else {
            ""
        };
        let tests_infix = format!("{0}tests{0}", sep());
        let base_dir = if cur_dir.contains(tests_infix.as_str()) {
            &cur_dir[..cur_dir.find(tests_infix.as_str()).unwrap()]
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
            for l in env::split_paths(&v) {
                let candidate = l.join(format!("{id}.loft"));
                if candidate.exists() {
                    f = candidate.to_string_lossy().replace(other_sep(), sep_str());
                    break;
                }
            }
        }
        // step 7d: packaged layout in LOFT_LIB
        if !std::path::Path::new(&f).exists()
            && let Some(v) = env::var_os("LOFT_LIB")
        {
            for l in env::split_paths(&v) {
                let l = l.to_string_lossy().replace(other_sep(), sep_str());
                if let Some(entry) = self.lib_path_manifest(&l, id) {
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
    /// Also check for redundant `const` annotations on primitive parameters that are never
    /// written to — the `const` has no effect when the parameter is not modified.
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
            // warn when `const` is used on a primitive parameter that is never
            // written to — the annotation is redundant since the parameter would not
            // have been modified anyway.  Compound types (vector, reference, struct)
            // are exempt: `const` serves as read-only documentation on those.
            let base_tp = if let Type::RefVar(inner) = &a.typedef {
                inner.as_ref()
            } else {
                &a.typedef
            };
            if a.constant
                && !written.contains(&(a_nr as u16))
                && matches!(
                    base_tp,
                    Type::Integer(_, _)
                        | Type::Long
                        | Type::Float
                        | Type::Single
                        | Type::Boolean
                        | Type::Character
                )
            {
                let src = self.vars.var_source(a_nr as u16);
                self.lexer.to(src);
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "Parameter '{}' is const but is never modified; \
                     'const' has no effect on an unmodified primitive parameter",
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
            Type::Reference(_, _) => self.cl("OpNullRefSentinel", &[]),
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
                || def.name == "OpAppendVector"
                || def.name == "OpClearVector"
                || def.name == "OpInsertVector"
                || def.name == "OpRemoveVector";
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
