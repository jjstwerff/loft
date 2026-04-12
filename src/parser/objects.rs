// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    DefType, HashSet, I32, Level, LexItem, LexResult, Mode, OUTPUT_DEFAULT, OutputState, Parser,
    SKIP_TOKEN, SKIP_WIDTH, ToString, Type, Value, diagnostic_format, to_default, v_block, v_if,
    v_set,
};

// Variable resolution, struct construction, and object parsing.

impl Parser {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_var(&mut self, code: &mut Value, name: &str, parent_tp: &mut Type) -> Type {
        // '$' refers to the current record in struct field default expressions
        if name == "$" && matches!(self.data.def_type(self.context), DefType::Struct) {
            *code = Value::Var(0);
            return Type::Reference(self.context, Vec::new());
        }
        let mut source = u16::MAX;
        let nm = if self.lexer.has_token("::") {
            source = self.data.get_source(name);
            if let Some(id) = self.lexer.has_identifier() {
                id
            } else {
                diagnostic!(self.lexer, Level::Error, "Expecting identifier after ::");
                name.to_string()
            }
        } else {
            name.to_string()
        };
        // vector<T>.parse(text) — parse a JSON array into a vector of T.
        if nm == "vector" && self.lexer.has_token("<") {
            if let Some(elem_name) = self.lexer.has_identifier() {
                let elem_d_nr = self.data.def_nr(&elem_name);
                self.lexer.token(">");
                if self.lexer.has_token(".") && self.lexer.has_keyword("parse") {
                    if elem_d_nr != u32::MAX {
                        return self.parse_vector_parse(elem_d_nr, code);
                    }
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown type '{elem_name}' in vector<{elem_name}>.parse()"
                        );
                    }
                    return Type::Unknown(0);
                }
            }
            // Not a vector<T>.parse() — cannot recover tokens, report error.
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expected '.parse(' after vector<T>"
                );
            }
            return Type::Unknown(0);
        }
        let mut t = self.parse_constant_value(code, source, &nm);
        if t != Type::Null {
            return t;
        }
        if self.lexer.has_token("(") {
            if name == "sizeof" {
                t = self.parse_size(code);
            } else if name == "type_name" {
                t = self.parse_type_name(code);
            } else if name == "typedef" {
                let mut p = Value::Null;
                let et = self.expression(&mut p);
                self.lexer.token(")");
                let tp = self.data.def(self.data.type_def_nr(&et)).known_type;
                t = Type::Integer(0, 65536, false);
                *code = Value::Int(i32::from(tp));
            } else {
                t = self.parse_call(code, source, &nm);
            }
        } else if self.closure_param != u16::MAX
            && !self.first_pass
            && self.data.def(self.context).closure_record != u32::MAX
            && self
                .data
                .attr(self.data.def(self.context).closure_record, name)
                != usize::MAX
        {
            // A5.3/A5.4: redirect captured variable reads to closure record field.
            let closure_d_nr = self.data.def(self.context).closure_record;
            let fnr = self.data.attr(closure_d_nr, name);
            *code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
            t = self.data.attr_type(closure_d_nr, fnr);
            // closure record is a struct — add __closure as dep so the
            // store allocation stays alive while derived text/references are in use.
            t = t.depending(self.closure_param);
        } else if self.vars.name_exists(name) {
            let index_var = self.vars.var(name);
            // on pass 2, if a variable has Unknown type, it may be a pass-1
            // placeholder for a forward-declared function. Try fn-ref resolution.
            if !self.first_pass && self.vars.tp(index_var).is_unknown() {
                let prefixed = format!("n_{nm}");
                let fn_d_nr = self.data.def_nr(&prefixed);
                if fn_d_nr != u32::MAX && matches!(self.data.def_type(fn_d_nr), DefType::Function) {
                    // Suppress "never read" warning on the pass-1 placeholder.
                    self.var_usages(index_var, true);
                    *code = Value::Int(fn_d_nr as i32);
                    self.data.def_used(fn_d_nr);
                    let n_args = self.data.attributes(fn_d_nr);
                    let arg_types: Vec<Type> = (0..n_args)
                        .map(|a| self.data.attr_type(fn_d_nr, a))
                        .collect();
                    let ret_type = self.data.def(fn_d_nr).returned.clone();
                    return Type::Function(arg_types, Box::new(ret_type), vec![]);
                }
            }
            if self.lexer.has_token("#") {
                self.var_usages(index_var, true);
                if self.lexer.has_keyword("errors") {
                    // s#errors — return the parse errors from the last Type.parse() call.
                    let fn_nr = self.data.def_nr("i_parse_errors");
                    if fn_nr != u32::MAX {
                        *code = Value::Call(fn_nr, vec![]);
                        t = Type::Text(Vec::new());
                    }
                    return t;
                }
                self.iter_op(code, name, &mut t, index_var);
            } else if let Value::Var(into) = code {
                let v_nr = self.vars.var(name);
                if matches!(self.vars.tp(v_nr), Type::Text(_)) {
                    t = self.vars.tp(v_nr).clone();
                } else {
                    t = self.vars.tp(v_nr).depending(v_nr);
                }
                self.var_usages(v_nr, true);
                if let Type::Reference(d_nr, _) = self.vars.tp(*into)
                    && let Type::Reference(vd_nr, _) = self.vars.tp(v_nr)
                    && d_nr == vd_nr
                {
                    // Don't create OpCopyRecord here: generate_set handles the copy when
                    // value=Var(src). Using Var(v_nr) directly lets method calls like
                    // `d = c.double()` pass c as `self` without the broken CopyRecord-as-self
                    // pattern that was causing garbage store_nr crashes (Issue 1).
                    let d_nr = *d_nr;
                    let into_var = *into;
                    self.vars.make_independent(into_var, v_nr);
                    *code = Value::Var(v_nr);
                    return Type::Reference(d_nr, Vec::new());
                }
                *code = Value::Var(v_nr);
            } else {
                let v_nr = self.vars.var(name);
                t = self.vars.tp(v_nr).depending(v_nr);
                self.var_usages(v_nr, true);
                *code = Value::Var(v_nr);
            }
        } else if let Some((_cname, ctype)) = self
            .capture_context
            .iter()
            .find(|(n, _)| n == name)
            .cloned()
        {
            // record the capture for closure record synthesis.
            if !self.captured_names.iter().any(|(n, _)| n == name) {
                self.captured_names.push((name.to_string(), ctype.clone()));
            }
            // if we have a closure parameter (second pass), emit field read
            // from the closure record.  Otherwise create a placeholder variable.
            // if we have a closure parameter (second pass), emit field read.
            let closure_d_nr = if self.closure_param == u16::MAX || self.first_pass {
                u32::MAX
            } else {
                self.data.def(self.context).closure_record
            };
            let fnr = if closure_d_nr == u32::MAX {
                usize::MAX
            } else {
                self.data.attr(closure_d_nr, name)
            };
            if fnr == usize::MAX {
                // First pass, no closure param, or field not found — placeholder variable.
                let v_nr = self.create_var(name, &ctype);
                self.var_usages(v_nr, true);
                t = ctype;
                *code = Value::Var(v_nr);
            } else {
                *code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
                t = self.data.attr_type(closure_d_nr, fnr);
                // closure record is a struct — add __closure as dep.
                t = t.depending(self.closure_param);
            }
        } else if self.data.def_nr(name) != u32::MAX
            && (!self.lexer.peek_token("=") || self.lexer.peek_token("=="))
        {
            let dnr = self.data.def_nr(name);
            if self.data.def_type(dnr) == DefType::Enum {
                t = self.data.def(dnr).returned.clone();
            } else if self.data.def_type(dnr) == DefType::EnumValue {
                t = Type::Enum(self.data.def(dnr).parent, true, Vec::new());
            } else {
                t = Type::Null;
            }
        } else if matches!(self.data.def_type(self.context), DefType::Struct)
            && self.data.attr(self.context, name) != usize::MAX
        {
            let fnr = self.data.attr(self.context, name);
            *code = self.get_field(self.context, fnr, Value::Var(0));
            t = self.data.attr_type(self.context, fnr);
        } else if let Type::Enum(enr, _, _) = parent_tp
            && let Some(a_nr) = self.data.def(*enr).attr_names.get(name)
        {
            *code = self.data.attr_value(*enr, *a_nr);
            t = parent_tp.clone();
        } else {
            // try resolving as a bare function reference.
            // On the first pass, only do this when the identifier is NOT followed
            // by '=' (assignment position), so that `double = 5` still creates a
            // local variable that shadows the function name.
            let fn_d_nr = {
                let prefixed = format!("n_{nm}");
                let nr = self.data.def_nr(&prefixed);
                if nr == u32::MAX {
                    self.data.def_nr(&nm)
                } else {
                    nr
                }
            };
            if fn_d_nr != u32::MAX && matches!(self.data.def_type(fn_d_nr), DefType::Function) {
                if self.lexer.peek_token("=") && !self.lexer.peek_token("==") {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot redefine function '{nm}' as a variable"
                    );
                }
                *code = Value::Int(fn_d_nr as i32);
                self.data.def_used(fn_d_nr);
                let n_args = self.data.attributes(fn_d_nr);
                let arg_types: Vec<Type> = (0..n_args)
                    .map(|a| self.data.attr_type(fn_d_nr, a))
                    .collect();
                let ret_type = self.data.def(fn_d_nr).returned.clone();
                t = Type::Function(arg_types, Box::new(ret_type), vec![]);
            } else if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Unknown variable '{}'", name);
                t = Type::Unknown(0);
            } else {
                *code = Value::Var(self.create_var(name, &Type::Unknown(0)));
                t = Type::Unknown(0);
            }
        }
        t
    }

    pub(crate) fn is_file_var(&self, var_nr: u16) -> bool {
        let file_def = self.data.def_nr("File");
        matches!(self.vars.tp(var_nr), Type::Reference(d, _) if *d == file_def)
    }

    pub(crate) fn file_op(&mut self, code: &mut Value, t: &mut Type, var_nr: u16) {
        self.vars.in_use(var_nr, true);
        if self.lexer.has_keyword("format") {
            let file_ref = Value::Var(var_nr);
            *code = self.cl("OpGetEnum", &[file_ref, Value::Int(32)]);
            let fmt_def = self.data.def_nr("Format");
            *t = Type::Enum(fmt_def, false, Vec::new());
        } else if self.lexer.has_keyword("exists") {
            let file_ref = Value::Var(var_nr);
            let fmt = self.cl("OpGetEnum", &[file_ref, Value::Int(32)]);
            let fmt_def = self.data.def_nr("Format");
            let enum_tp = Type::Enum(fmt_def, false, Vec::new());
            let ne_val = if let Some(&a_nr) = self.data.def(fmt_def).attr_names.get("NotExists") {
                self.data.attr_value(fmt_def, a_nr)
            } else {
                diagnostic!(self.lexer, Level::Error, "Format.NotExists not found");
                Value::Null
            };
            self.call_op(code, "!=", &[fmt, ne_val], &[enum_tp.clone(), enum_tp]);
            *t = Type::Boolean;
        } else if self.lexer.has_keyword("size") {
            *code = self.cl("OpSizeFile", &[Value::Var(var_nr)]);
            *t = Type::Long;
        } else if self.lexer.has_keyword("index") {
            // Read the current field at offset 8
            *code = self.cl("OpGetLong", &[Value::Var(var_nr), Value::Int(8)]);
            *t = Type::Long;
        } else if self.lexer.has_keyword("next") {
            // Read the next field at offset 16
            *code = self.cl("OpGetLong", &[Value::Var(var_nr), Value::Int(16)]);
            *t = Type::Long;
        } else if self.lexer.has_keyword("read") {
            self.lexer.token("(");
            let mut n_code = Value::Null;
            self.expression(&mut n_code);
            self.lexer.token(")");
            // Determine read type from optional "as T"
            let (read_type, db_tp) = if self.lexer.has_token("as") {
                if let Some(type_name) = self.lexer.has_identifier() {
                    let tp = self
                        .parse_type(u32::MAX, &type_name, false)
                        .unwrap_or(Type::Text(vec![]));
                    if let Type::Reference(d_nr, _) = &tp
                        && let Some(field) = Self::first_collection_field(*d_nr, &self.data)
                    {
                        let tname = self.data.def(*d_nr).name.clone();
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "read_file: '{}' has collection field '{}'; use a plain struct for serialisation",
                            tname,
                            field
                        );
                    }
                    self.ensure_io_type(&tp.clone());
                    let id = self.get_type(&tp);
                    (tp, id)
                } else {
                    let text_tp = Type::Text(vec![]);
                    let id = self.get_type(&text_tp);
                    (text_tp, id)
                }
            } else {
                let text_tp = Type::Text(vec![]);
                let id = self.get_type(&text_tp);
                (text_tp, id)
            };
            let mut ls = Vec::new();
            let temp_var = if let Type::Text(_) = read_type {
                self.vars.work_text(&mut self.lexer)
            } else {
                let t = self.vars.unique("read", &read_type, &mut self.lexer);
                ls.push(v_set(t, self.null(&read_type)));
                t
            };
            let var_ref = self.cl("OpCreateStack", &[Value::Var(temp_var)]);
            ls.push(self.cl(
                "OpReadFile",
                &[
                    Value::Var(var_nr),
                    var_ref,
                    n_code,
                    Value::Int(i32::from(db_tp)),
                ],
            ));
            ls.push(Value::Var(temp_var));
            *code = v_block(ls, read_type.clone(), "reading file");
            *t = read_type;
        } else {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Unknown # operation on File");
            }
            *t = Type::Unknown(0);
        }
    }

    pub(crate) fn is_file_var_type(&self, tp: &Type) -> bool {
        let file_def = self.data.def_nr("File");
        matches!(tp, Type::Reference(d, _) if *d == file_def)
    }

    /// Ensure byte/short integer types used in file I/O are registered in the database.
    pub(crate) fn ensure_io_type(&mut self, t: &Type) {
        match t {
            Type::Integer(min, _, _) => match t.size(false) {
                1 => {
                    self.database.byte(*min, false);
                }
                2 => {
                    self.database.short(*min, false);
                }
                _ => {}
            },
            Type::Vector(tp, _) => {
                let tp = tp.clone();
                self.ensure_io_type(&tp);
            }
            _ => {}
        }
    }

    /// Return the name of the first collection-type field in `d_nr`, or `None`.
    /// Collection fields (sorted/index/hash/spacial) cannot be serialised by the binary
    /// file I/O routines; callers should emit a compile-time error when this returns `Some`.
    fn first_collection_field(d_nr: u32, data: &super::Data) -> Option<String> {
        for a in &data.def(d_nr).attributes {
            if matches!(
                a.typedef,
                Type::Sorted(..) | Type::Index(..) | Type::Hash(..) | Type::Spacial(..)
            ) {
                return Some(a.name.clone());
            }
        }
        None
    }

    pub(crate) fn write_to_file(&mut self, file_var: u16, val: Value, val_type: &Type) -> Value {
        if let Type::Reference(d_nr, _) = val_type
            && let Some(field) = Self::first_collection_field(*d_nr, &self.data)
        {
            let type_name = self.data.def(*d_nr).name.clone();
            diagnostic!(
                self.lexer,
                Level::Error,
                "write_file: '{}' has collection field '{}'; use a plain struct for serialisation",
                type_name,
                field
            );
            return Value::Null;
        }
        let val_type_clone = val_type.clone();
        self.ensure_io_type(&val_type_clone);
        let db_tp = self.get_type(val_type);
        let temp_var = self.vars.unique("wf", val_type, &mut self.lexer);
        for d in val_type.depend() {
            self.vars.depend(temp_var, d);
        }
        let assign = v_set(temp_var, val);
        let var_ref = self.cl("OpCreateStack", &[Value::Var(temp_var)]);
        let write = self.cl(
            "OpWriteFile",
            &[Value::Var(file_var), var_ref, Value::Int(i32::from(db_tp))],
        );
        Value::Insert(vec![assign, write])
    }

    pub(crate) fn parse_constant_value(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
    ) -> Type {
        let mut t;
        let d_nr = if source == u16::MAX {
            self.data.def_nr(name)
        } else {
            self.data.source_nr(source, name)
        };
        if d_nr != u32::MAX {
            self.data.def_used(d_nr);
            t = self.data.def(d_nr).returned.clone();
            if self.data.def_type(d_nr) == DefType::Function {
                t = Type::Routine(d_nr);
            } else if matches!(
                self.data.def_type(d_nr),
                DefType::Struct | DefType::EnumValue
            ) && !matches!(self.data.def(d_nr).returned, Type::Enum(_, false, _))
            {
                if self.lexer.peek_token("{") {
                    let tp = self.parse_object(d_nr, code);
                    if tp != Type::Unknown(0) {
                        return tp;
                    }
                } else if self.lexer.peek_token(".") {
                    // Check for Type.parse(text) without consuming the dot
                    // unless we confirm it's ".parse(".
                    // Consume "." — if parse follows, continue; otherwise this
                    // will fall through to normal parsing which handles Struct.field.
                    self.lexer.cont();
                    if self.lexer.has_keyword("parse") {
                        return self.parse_type_parse(d_nr, code);
                    }
                }
            } else if self.data.def_type(d_nr) == DefType::Constant {
                let const_code = self.data.def(d_nr).code.clone();
                let const_tp = self.data.def(d_nr).returned.clone();
                // vector constants are pre-built in CONST_STORE during
                // byte_code(). Emit OpConstRef + OpCopyRecord to deep-copy
                // from the constant store into a fresh runtime store.
                // On pass 1 const_ref is None but we still emit the same IR
                // shape so create_unique runs on both passes (counter sync).
                if matches!(const_tp, Type::Vector(_, _)) && matches!(const_code, Value::Block(_)) {
                    // Emit a simple Call to OpConstRef. The constant's DbRef
                    // will be deep-copied at the call site — the caller's
                    // gen_set_first_ref_call_copy handles the CopyRecord.
                    *code = self.cl("OpConstRef", &[Value::Int(d_nr as i32)]);
                    return const_tp;
                }
                *code = const_code;
                return const_tp;
            }
            if let Type::Enum(en, _, _) = t {
                for a_nr in 0..self.data.attributes(en) {
                    if self.data.attr_name(en, a_nr) == name {
                        *code = self.data.attr_value(en, a_nr);
                        return t;
                    }
                }
            }
        }
        Type::Null
    }

    pub(crate) fn known_var_or_type(&mut self, code: &Value) {
        if let Value::Var(nr) = code {
            if !self.vars.exists(*nr) {
                return;
            }
            if self.default && matches!(self.vars.tp(*nr), Type::Vector(_, _)) {
                return;
            }
            if !self.first_pass && (self.vars.tp(*nr).is_unknown() || !self.vars.is_defined(*nr)) {
                let name = self.vars.name(*nr).to_string();
                let candidates: Vec<&str> = (0..self.vars.count())
                    .filter(|&v| {
                        v != *nr && self.vars.is_defined(v) && !self.vars.tp(v).is_unknown()
                    })
                    .map(|v| self.vars.name(v))
                    .collect();
                let suggestion = crate::diagnostics::suggest_similar(&name, &candidates);
                if let Some(s) = suggestion {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown variable '{}' — did you mean '{}'?",
                        name,
                        s
                    );
                } else {
                    diagnostic!(self.lexer, Level::Error, "Unknown variable '{}'", name);
                }
            }
        }
    }

    /// `Type.parse(text_expr)` — parse text into a struct record.
    /// Compiles to the same `OpCastVectorFromText` as the `as Type` cast.
    fn parse_type_parse(&mut self, d_nr: u32, code: &mut Value) -> Type {
        self.lexer.token("(");
        let mut text_expr = Value::Null;
        let tp = self.expression(&mut text_expr);
        self.lexer.token(")");
        if !self.first_pass {
            if !matches!(tp, Type::Text(_)) {
                self.convert(&mut text_expr, &tp, &Type::Text(Vec::new()));
            }
            let known_tp = self.data.def(d_nr).known_type;
            *code = self.cl(
                "OpCastVectorFromText",
                &[text_expr, Value::Int(i32::from(known_tp))],
            );
        }
        Type::Reference(d_nr, Vec::new())
    }

    /// Parse `vector<T>.parse(text)` — parse a JSON array into a vector of T.
    /// Returns `Type::Vector(T)` so the result is directly iterable.
    fn parse_vector_parse(&mut self, elem_d_nr: u32, code: &mut Value) -> Type {
        self.lexer.token("(");
        let mut text_expr = Value::Null;
        let tp = self.expression(&mut text_expr);
        self.lexer.token(")");
        let elem_tp = Type::Reference(elem_d_nr, Vec::new());
        let vec_type = Type::Vector(Box::new(elem_tp.clone()), Vec::new());
        if !self.first_pass {
            if !matches!(tp, Type::Text(_)) {
                self.convert(&mut text_expr, &tp, &Type::Text(Vec::new()));
            }
            // Get the database vector type for vector<elem>.
            let elem_kt = self.data.def(elem_d_nr).known_type;
            let vec_kt = self.database.vector(elem_kt);
            let parse_call = self.cl(
                "OpCastVectorFromText",
                &[text_expr, Value::Int(i32::from(vec_kt))],
            );
            // The parse returns a DbRef to the wrapper struct main_vector<T>.
            // Extract the vector field (at position 0) so the result is directly iterable.
            let wrapper_name = format!("main_vector<{}>", self.data.def(elem_d_nr).name);
            let wrapper_d_nr = self.data.def_nr(&wrapper_name);
            if wrapper_d_nr == u32::MAX {
                *code = parse_call;
            } else {
                *code = self.get_field(wrapper_d_nr, 0, parse_call);
            }
        }
        // Ensure the vector def exists for type resolution.
        self.data.vector_def(&mut self.lexer, &elem_tp);
        vec_type
    }

    pub(crate) fn parse_string(&mut self, code: &mut Value, string: &str) -> Type {
        let mut append_value = u16::MAX;
        *code = Value::str(string);
        let mut var = u16::MAX;
        let mut list = vec![];
        if self.lexer.mode() == Mode::Formatting {
            // Define a new variable to append to
            var = self.vars.work_text(&mut self.lexer);
            list.push(v_set(var, code.clone()));
        }
        while self.lexer.mode() == Mode::Formatting {
            self.lexer.set_mode(Mode::Code);
            let mut format = Value::Null;
            let saved_in_fmt = self.in_format_expr;
            self.in_format_expr = true;
            let mut tp = if self.lexer.has_token("for") {
                self.iter_for(&mut format, &mut append_value)
            } else {
                self.expression(&mut format)
            };
            self.in_format_expr = saved_in_fmt;
            self.un_ref(&mut tp, &mut format);
            if !self.first_pass && tp.is_unknown() {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Incorrect expression in string was {tp:?}"
                );
                return Type::Void;
            }
            self.lexer.set_mode(Mode::Formatting);
            let mut state = OUTPUT_DEFAULT;
            let mut token = "0".to_string();
            if self.lexer.has_token(":") {
                if let LexResult {
                    has: LexItem::Token(t),
                    position: _pos,
                } = self.lexer.peek()
                {
                    let st: &str = &t;
                    if !SKIP_TOKEN.contains(&st) {
                        token.clear();
                        token += &t;
                        state.token = &token;
                        self.lexer.cont();
                    }
                }
                self.string_states(&mut state);
                let LexResult {
                    has: h,
                    position: _pos,
                } = self.lexer.peek();
                if match h {
                    LexItem::Token(st) | LexItem::Identifier(st) => {
                        let s: &str = &st;
                        !SKIP_WIDTH.contains(&s)
                    }
                    LexItem::Integer(_, _) | LexItem::Float(_) => true,
                    _ => false,
                } {
                    if let LexResult {
                        has: LexItem::Integer(_, true),
                        position: _pos,
                    } = self.lexer.peek()
                    {
                        state.token = "0";
                    }
                    self.lexer.set_mode(Mode::Code);
                    self.expression(&mut state.width);
                    self.lexer.set_mode(Mode::Formatting);
                }
                state.radix = self.get_radix();
            }
            self.append_data(tp, &mut list, var, append_value, &format, state);
            if let Some(text) = self.lexer.has_cstring() {
                if !text.is_empty() {
                    let call = if matches!(self.vars.tp(var), Type::RefVar(_)) {
                        "OpAppendStackText"
                    } else {
                        "OpAppendText"
                    };
                    list.push(self.cl(call, &[Value::Var(var), Value::str(&text)]));
                }
            } else {
                diagnostic!(self.lexer, Level::Error, "Formatter error");
                return Type::Void;
            }
        }
        if var < u16::MAX {
            list.push(Value::Var(var));
            *code = v_block(list, Type::Text(vec![var]), "Formatted string");
            Type::Text(vec![var])
        } else {
            Type::Text(Vec::new())
        }
    }

    pub(crate) fn string_states(&mut self, state: &mut OutputState) {
        if self.lexer.has_token("<") {
            state.dir = -1;
        } else if self.lexer.has_token("^") {
            state.dir = 0;
        } else if self.lexer.has_token(">") {
            state.dir = 1;
        }
        if self.lexer.has_token("+") {
            state.plus = true;
        }
        if self.lexer.has_token("#") {
            // show 0x 0b or 0o in front of numbers when applicable
            state.note = true;
        }
        if self.lexer.has_token(".") {
            state.float = true;
        }
    }

    pub(crate) fn get_radix(&mut self) -> i32 {
        if let Some(id) = self.lexer.has_identifier() {
            if id.to_lowercase() == "j" || id.to_lowercase() == "json" {
                -1
            } else if id == "x" || id == "X" {
                16
            } else if id == "b" {
                2
            } else if id == "o" {
                8
            } else if id == "e" {
                1
            } else if id == "d" || id == "f" {
                10
            } else {
                diagnostic!(self.lexer, Level::Error, "Unexpected formatting type: {id}");
                10
            }
        } else {
            10
        }
    }

    // Iterator for
    // <for> ::= <identifier> 'in' <range> '{' <block>
    pub(crate) fn iter_for(&mut self, val: &mut Value, append_value: &mut u16) -> Type {
        if let Some(id) = self.lexer.has_identifier() {
            // Create {id}#index first (always needed, regardless of type).
            let index_var = self.create_var(&format!("{id}#index"), &I32);
            self.vars.defined(index_var);
            self.lexer.token("in");
            let loop_nr = self.vars.start_loop();
            let mut expr = Value::Null;
            let in_type = self.parse_in_range(&mut expr, &Value::Null, &id);
            // For text loops: {id}#next drives the loop; {id}#index is saved per-iteration.
            let (iter_var, pre_var) = if matches!(in_type, Type::Text(_)) {
                let pos_var = self.create_var(&format!("{id}#next"), &I32);
                self.vars.defined(pos_var);
                (pos_var, Some(index_var))
            } else {
                (index_var, None)
            };
            let var_tp = self.for_type(&in_type);
            *append_value = self.create_unique("val", &Type::Unknown(0));
            let for_var = self.create_var(&id, &var_tp);
            self.vars.defined(for_var);
            let if_step = if self.lexer.has_token("if") {
                let mut if_expr = Value::Null;
                self.expression(&mut if_expr);
                if_expr
            } else {
                Value::Null
            };
            let mut create_iter = expr;
            let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
            let iter_next = self.iterator(&mut create_iter, &in_type, &it, iter_var, pre_var);
            if !self.first_pass && iter_next == Value::Null {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Need an iterable expression in a for statement"
                );
                return Type::Null;
            }
            let for_next = v_set(for_var, iter_next);
            self.vars.loop_var(for_var);
            let in_loop = self.in_loop;
            self.in_loop = true;
            let mut block = Value::Null;
            let format_type = self.parse_block("for", &mut block, &Type::Unknown(0));
            self.change_var_type(*append_value, &format_type);
            self.in_loop = in_loop;
            let mut lp = vec![for_next];
            if !matches!(in_type, Type::Iterator(_, _)) {
                lp.push(v_if(
                    self.single_op("!", Value::Var(for_var), var_tp.clone()),
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            }
            if if_step != Value::Null {
                lp.push(v_if(if_step, Value::Null, Value::Continue(0)));
            }
            let result_tp = if let Value::Block(bl) = &block {
                bl.result.clone()
            } else {
                var_tp.clone()
            };
            lp.push(block);
            let tp = Type::Iterator(Box::new(format_type), Box::new(Type::Null));
            // For text loops, extra_init holds v_set(index_var, 0) which must be emitted at
            // the same scope level as the iterator init (outside the loop) so the slot
            // assigner sees {id}#index as live across the entire loop body.
            let extra_init = if let Some(idx_var) = pre_var {
                Box::new(v_set(idx_var, Value::Int(0)))
            } else {
                Box::new(Value::Null)
            };
            *val = Value::Iter(
                for_var,
                Box::new(create_iter),
                Box::new(v_block(lp, result_tp, "Iter For")),
                extra_init,
            );
            self.vars.finish_loop(loop_nr);
            return tp;
        }
        diagnostic!(self.lexer, Level::Error, "Expect variable after for");
        Type::Null
    }

    // range ::= rev(<expr> '..' ['='] <expr>) | <expr> [ '..' ['='] <expr> ]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_in_range_body(
        &mut self,
        expr: &mut Value,
        data: &Value,
        name: &str,
        in_type: Type,
        reverse: bool,
    ) -> Type {
        let incl = self.lexer.has_token("=");
        // O8.5: capture range bounds for const-unroll detection.
        self.last_range_from = Some(expr.clone());
        let mut till = Value::Null;
        let till_tp = if self.lexer.peek_token("]") {
            till = if *data == Value::Null {
                Value::Int(i32::MAX)
            } else {
                self.cl("OpLengthVector", std::slice::from_ref(data))
            };
            in_type.clone()
        } else {
            self.expression(&mut till)
        };
        // O8.5: store till value (adjusted for inclusive ranges).
        if incl {
            // 0..=9 means till is 9, but the range includes 9.
            // Store till+1 so the unroller can use from..till_exclusive.
            if let Value::Int(t) = &till {
                self.last_range_till = Some(Value::Int(t + 1));
            } else {
                self.last_range_till = None;
            }
        } else {
            self.last_range_till = Some(till.clone());
        }
        let ivar = if name == "$" {
            self.create_unique("index", &in_type.clone())
        } else {
            self.create_var(&format!("{name}#index"), &in_type)
        };
        let mut ls = Vec::new();
        let test = if reverse {
            if incl {
                ls.push(v_set(
                    ivar,
                    v_if(
                        self.single_op("!", Value::Var(ivar), in_type.clone()),
                        till,
                        self.conv_op(
                            "-",
                            Value::Var(ivar),
                            Value::Int(1),
                            in_type.clone(),
                            I32.clone(),
                        ),
                    ),
                ));
            } else {
                ls.push(v_if(
                    self.single_op("!", Value::Var(ivar), in_type.clone()),
                    v_set(ivar, till),
                    Value::Null,
                ));
                ls.push(v_set(
                    ivar,
                    self.conv_op(
                        "-",
                        Value::Var(ivar),
                        Value::Int(1),
                        in_type.clone(),
                        I32.clone(),
                    ),
                ));
            }
            self.conv_op(
                "<",
                Value::Var(ivar),
                expr.clone(),
                in_type.clone(),
                till_tp,
            )
        } else {
            ls.push(v_set(
                ivar,
                v_if(
                    self.single_op("!", Value::Var(ivar), in_type.clone()),
                    expr.clone(),
                    self.conv_op(
                        "+",
                        Value::Var(ivar),
                        Value::Int(1),
                        in_type.clone(),
                        I32.clone(),
                    ),
                ),
            ));
            self.conv_op(
                if incl { "<" } else { "<=" },
                till,
                Value::Var(ivar),
                till_tp,
                in_type.clone(),
            )
        };
        ls.push(v_if(test, Value::Break(0), Value::Null));
        ls.push(Value::Var(ivar));
        *expr = Value::Iter(
            u16::MAX,
            Box::new(v_set(ivar, self.null(&in_type))),
            Box::new(v_block(ls, in_type.clone(), "Iter range")),
            Box::new(Value::Null),
        );
        if reverse {
            self.lexer.token(")");
            self.reverse_iterator = false;
        }
        Type::Iterator(Box::new(in_type), Box::new(Type::Null))
    }

    pub(crate) fn parse_in_range(&mut self, expr: &mut Value, data: &Value, name: &str) -> Type {
        let mut reverse = false;
        if let LexItem::Identifier(rev) = self.lexer.peek().has
            && &rev == "rev"
        {
            self.lexer.has_identifier();
            self.lexer.token("(");
            reverse = true;
            // set the reverse flag BEFORE parsing the inner expression so that
            // rev(col[lo..hi]) passes the flag through parse_key → fill_iter.
            self.reverse_iterator = true;
        }
        let in_type = if self.lexer.peek_token("..") || self.lexer.peek_token("..=") {
            // Open-start range: treat missing start as 0.
            *expr = Value::Int(0);
            I32.clone()
        } else {
            self.expression(expr)
        };
        if !self.lexer.has_token("..") {
            if reverse {
                // if the inner expression was a subscript that already produced
                // a range iterator (parse_key consumed the `..`), the Value::Iter is
                // ready with the reverse flag — just consume ')' and return.
                if matches!(expr, Value::Iter(_, _, _, _)) {
                    self.lexer.token(")");
                    self.reverse_iterator = false;
                    return in_type;
                }
                // rev() wrapping a bare collection (not a range subscript).
                if matches!(
                    in_type,
                    Type::Sorted(_, _, _) | Type::Index(_, _, _) | Type::Vector(_, _)
                ) {
                    // reverse_iterator stays set; consumed and reset by iterator()
                } else if !matches!(in_type, Type::Null) {
                    self.reverse_iterator = false;
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "rev() on a non-range expression must wrap a sorted, index, or vector collection"
                    );
                }
                self.lexer.token(")");
            }
            return in_type;
        }
        self.parse_in_range_body(expr, data, name, in_type, reverse)
    }

    pub(crate) fn parse_object_field(
        &mut self,
        td_nr: u32,
        code: &mut Value,
        list: &mut Vec<Value>,
        found_fields: &mut HashSet<String>,
    ) -> bool {
        // Accept both bare identifiers and JSON-style quoted strings as field names.
        let field = if let Some(id) = self.lexer.has_identifier() {
            id
        } else if let Some(s) = self.lexer.has_cstring() {
            s
        } else {
            return false;
        };
        if !self.lexer.has_token(":") {
            return false;
        }
        let nr = self.data.attr(td_nr, &field);
        if nr == usize::MAX {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown field {}.{field}",
                self.data.def(td_nr).name
            );
        } else {
            let td = self.data.attr_type(td_nr, nr);
            let pos = self
                .database
                .position(self.data.def(td_nr).known_type, &field);
            found_fields.insert(field.clone());
            let mut value = if let Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Spacial(_, _, _)
            | Type::Enum(_, true, _)
            | Type::Index(_, _, _) = td
            {
                list.push(self.cl(
                    "OpSetInt",
                    &[code.clone(), Value::Int(i32::from(pos)), Value::Int(0)],
                ));
                self.cl(
                    "OpGetField",
                    &[
                        code.clone(),
                        Value::Int(i32::from(pos)),
                        self.type_info(&td),
                    ],
                )
            } else {
                Value::Null
            };
            let mut parent_tp = Type::Reference(td_nr, Vec::new());
            if let Value::Var(v) = code {
                parent_tp = parent_tp.depending(*v);
            }
            let exp_tp = self.parse_operators(&td, &mut value, &mut parent_tp, 0);
            self.handle_field(td_nr, code, list, &field, &mut value, &exp_tp);
        }
        true
    }

    pub(crate) fn parse_object(&mut self, td_nr: u32, code: &mut Value) -> Type {
        let link = self.lexer.link();
        if !self.lexer.has_token("{") {
            self.lexer.revert(link);
            return Type::Unknown(0);
        }
        let mut list = Vec::new();
        let mut new_object = false;
        let work = self.vars.work_ref();
        if let Value::Var(v_nr) = code {
            let var_tp = self.vars.tp(*v_nr).clone();
            let type_matches =
                var_tp.is_unknown() || matches!(&var_tp, Type::Reference(d, _) if *d == td_nr);
            if self.vars.is_independent(*v_nr) && type_matches {
                if !self.vars.is_argument(*v_nr) {
                    list.push(v_set(*v_nr, Value::Null));
                }
                self.data.set_referenced(td_nr, self.context, Value::Null);
                let tp = i32::from(self.data.def(td_nr).known_type);
                list.push(self.cl("OpDatabase", &[Value::Var(*v_nr), Value::Int(tp)]));
            } else if !type_matches && !self.first_pass {
                // LHS variable already has an incompatible type (e.g. integer from a prior
                // pass). Fall through to new_object so the struct gets a fresh work ref and
                // the result is a proper Value::Block — not a Value::Insert — which can be
                // used safely as a method-call argument.
                new_object = true;
                self.data.set_referenced(td_nr, self.context, Value::Null);
                let ret = &self.data.def(td_nr).returned;
                let w = self.vars.work_refs(ret, &mut self.lexer);
                let tp = i32::from(self.data.def(td_nr).known_type);
                list.push(v_set(w, Value::Null));
                list.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp)]));
                *code = Value::Var(w);
            }
        } else if !self.first_pass && !self.is_field(code) {
            new_object = true;
            self.data.set_referenced(td_nr, self.context, Value::Null);
            let ret = &self.data.def(td_nr).returned;
            let w = self.vars.work_refs(ret, &mut self.lexer);
            let tp = i32::from(self.data.def(td_nr).known_type);
            list.push(v_set(w, Value::Null));
            list.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp)]));
            *code = Value::Var(w);
        }
        let mut found_fields = HashSet::new();
        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            if !self.parse_object_field(td_nr, code, &mut list, &mut found_fields) {
                self.lexer.revert(link);
                self.vars.clean_work_refs(work);
                return Type::Unknown(0);
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("}");
        if !self.first_pass {
            self.object_init(&mut list, td_nr, 0, code, &found_fields);
            // emit all field constraint checks after construction completes.
            let assert_dnr = self.data.def_nr("n_assert");
            for a_nr in 0..self.data.def(td_nr).attributes.len() {
                let check = self.data.def(td_nr).attributes[a_nr].check.clone();
                if check != Value::Null {
                    let bound = Self::replace_record_ref(check, code);
                    let nm = self.data.attr_name(td_nr, a_nr);
                    let msg = match &self.data.def(td_nr).attributes[a_nr].check_message {
                        Value::Text(s) => Value::Text(s.clone()),
                        _ => Value::Text(format!(
                            "field constraint failed on {}.{nm}",
                            self.data.def(td_nr).name
                        )),
                    };
                    let pos = self.lexer.pos();
                    list.push(Value::Call(
                        assert_dnr,
                        vec![
                            bound,
                            msg,
                            Value::Text(pos.file.clone()),
                            Value::Int(pos.line as i32),
                        ],
                    ));
                }
            }
        }
        if new_object && let Value::Var(v) = code {
            list.push(Value::Var(*v));
            *code = v_block(list, Type::Reference(td_nr, vec![*v]), "Object");
            Type::Reference(td_nr, Vec::new())
        } else {
            *code = Value::Insert(list);
            Type::Rewritten(Box::new(Type::Reference(td_nr, Vec::new())))
        }
    }

    /// Recursively replace `Value::Var(0)` (the record placeholder used in field default
    /// expressions) with the actual record reference from the calling context.
    pub(crate) fn replace_record_ref(val: Value, record: &Value) -> Value {
        match val {
            Value::Var(0) => record.clone(),
            Value::Call(nr, args) => Value::Call(
                nr,
                args.into_iter()
                    .map(|a| Self::replace_record_ref(a, record))
                    .collect(),
            ),
            Value::If(cond, t, f) => Value::If(
                Box::new(Self::replace_record_ref(*cond, record)),
                Box::new(Self::replace_record_ref(*t, record)),
                Box::new(Self::replace_record_ref(*f, record)),
            ),
            Value::Block(bl) => Value::Block(Box::new(crate::data::Block {
                name: bl.name,
                operators: bl
                    .operators
                    .into_iter()
                    .map(|v| Self::replace_record_ref(v, record))
                    .collect(),
                result: bl.result,
                scope: bl.scope,
                var_size: 0,
            })),
            Value::Set(v, inner) => {
                Value::Set(v, Box::new(Self::replace_record_ref(*inner, record)))
            }
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|v| Self::replace_record_ref(v, record))
                    .collect(),
            ),
            Value::Return(inner) => {
                Value::Return(Box::new(Self::replace_record_ref(*inner, record)))
            }
            Value::Drop(inner) => Value::Drop(Box::new(Self::replace_record_ref(*inner, record))),
            other => other,
        }
    }

    // fill the not mentioned fields with their default value
    pub(crate) fn object_init(
        &mut self,
        list: &mut Vec<Value>,
        td_nr: u32,
        pos: u16,
        code: &Value,
        found_fields: &HashSet<String>,
    ) {
        for aid in 0..self.data.attributes(td_nr) {
            let tp = self.data.attr_type(td_nr, aid);
            let nm = self.data.attr_name(td_nr, aid);
            let fld = self.database.position(self.data.def(td_nr).known_type, &nm);
            // Skip computed fields (not stored) and already-provided fields.
            if found_fields.contains(&nm)
                || matches!(tp, Type::Routine(_))
                || self.data.def(td_nr).attributes[aid].constant
            {
                continue;
            }
            let mut default = self.data.attr_value(td_nr, aid);
            if let Type::Reference(tp, _) = tp
                && default == Value::Null
            {
                self.object_init(list, tp, pos + fld, code, &HashSet::new());
                continue;
            } else if default == Value::Null {
                default = to_default(&tp, &self.data);
            } else {
                default = Self::replace_record_ref(default, code);
            }
            list.push(self.set_field_no_check(td_nr, aid, pos, code.clone(), default));
        }
    }

    pub(crate) fn handle_field(
        &mut self,
        td_nr: u32,
        code: &mut Value,
        list: &mut Vec<Value>,
        field: &str,
        value: &mut Value,
        exp_tp: &Type,
    ) {
        let nr = self.data.attr(td_nr, field);
        let td = self.data.attr_type(td_nr, nr);
        if matches!(
            td,
            Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Spacial(_, _, _)
                | Type::Index(_, _, _)
        ) {
            // Issue #120: for vector fields assigned from a bare variable
            // (e.g. `BigBox { data: d }`), parse_operators overwrites the
            // field ref with Var(d) — no copy operation is generated.
            // Emit OpAppendVector to deep-copy the source vector into the
            // struct's field so the data is independent of the source store.
            if let Type::Vector(ref content, _) = td {
                if !self.first_pass && matches!(value, Value::Var(_)) {
                    let pos = self
                        .database
                        .position(self.data.def(td_nr).known_type, field);
                    let elem_tp = self.data.def(self.data.type_def_nr(content)).known_type;
                    let vec_tp = self.database.vector(elem_tp);
                    let field_ref = self.cl(
                        "OpGetField",
                        &[
                            code.clone(),
                            Value::Int(i32::from(pos)),
                            Value::Int(i32::from(vec_tp)),
                        ],
                    );
                    list.push(self.cl(
                        "OpAppendVector",
                        &[field_ref, value.clone(), Value::Int(i32::from(elem_tp))],
                    ));
                } else {
                    list.push(value.clone());
                }
            } else {
                list.push(value.clone());
            }
        } else if let Value::Insert(ops) = value {
            for o in ops {
                list.push(o.clone());
            }
        } else {
            if !self.convert(value, exp_tp, &td) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot write {} on field {}.{field}:{}",
                    td.show(&self.data, &self.vars),
                    self.data.def(td_nr).name,
                    exp_tp.show(&self.data, &self.vars)
                );
            }
            list.push(self.set_field_no_check(td_nr, nr, 0, code.clone(), value.clone()));
        }
    }

    pub(crate) fn parse_enum_field(
        &mut self,
        list: &mut Vec<Value>,
        into: Value,
        d_nr: u32,
        pos: u16,
        enum_nr: u8,
    ) {
        let e_nr = self
            .data
            .def_nr(&self.data.def(d_nr).attributes[enum_nr as usize - 1].name);
        let tp = self.data.def(e_nr).returned.clone();
        let v = self.create_unique("enum", &tp);
        let mut cd = if pos != 0 {
            list.push(v_set(
                v,
                self.cl("OpGetField", &[into, Value::Int(i32::from(pos))]),
            ));
            Value::Var(v)
        } else {
            into.clone()
        };
        self.parse_object(e_nr, &mut cd);
        if let Value::Insert(ls) = &cd {
            for l in ls {
                list.push(l.clone());
            }
        }
    }
}
