// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use super::{
    Argument, DefType, Function, HashMap, HashSet, Level, Link, Parser, Position, ToString, Type,
    Value, complete_definition, diagnostic_format, is_camel, is_lower, is_op, is_upper, v_block,
    v_if,
};

impl Parser {
    pub(crate) fn warn_missing_enum_variants(&mut self, e_nr: u32, nrs: &[usize], name: &str) {
        let implemented: HashSet<u32> = nrs
            .iter()
            .filter_map(|nr| {
                if let Type::Reference(a_nr, _) = self.data.def(*nr as u32).attributes[0].typedef {
                    Some(a_nr)
                } else {
                    None
                }
            })
            .collect();
        let missing: Vec<(String, Position)> = self
            .data
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, v)| v.def_type == DefType::EnumValue && v.parent == e_nr)
            .filter(|(v_nr, _)| !implemented.contains(&(*v_nr as u32)))
            .map(|(_, v)| (v.name.clone(), v.position.clone()))
            .collect();
        for (variant_name, pos) in &missing {
            self.lexer.pos_diagnostic(
                Level::Warning,
                pos,
                &format!("Warning: no implementation of '{name}' for variant '{variant_name}'"),
            );
        }
    }

    pub(crate) fn create_enum_dispatch_fn(&mut self, e_nr: u32, nrs: &[usize]) {
        let from_nr = nrs[0] as u32;
        let name = self.data.def(from_nr).original_name().clone();
        let attrs = self.data.def(from_nr).attributes[1..].to_vec();
        let mut common = attrs.len();
        for nr in &nrs[1..] {
            let mut c = 0;
            for a in &self.data.def(*nr as u32).attributes[1..] {
                for o in &attrs {
                    if a.name == o.name && a.typedef == o.typedef {
                        c += 1;
                    }
                }
            }
            if c < common {
                common = c;
            }
        }
        for nr in nrs {
            if self.data.def(*nr as u32).attributes.len() > common + 1 {
                for a in &self.data.def(*nr as u32).attributes[common + 1..] {
                    if a.value == Value::Null {
                        return;
                    }
                }
            }
        }
        let mut args = Vec::new();
        args.push(Argument {
            name: "self".to_string(),
            typedef: Type::Enum(e_nr, true, vec![]),
            default: Value::Null,
            constant: false,
        });
        for a in &attrs[..common] {
            args.push(Argument {
                name: a.name.clone(),
                typedef: a.typedef.clone(),
                default: a.value.clone(),
                constant: false,
            });
        }
        let fn_nr = self.data.add_fn(&mut self.lexer, &name, &args);
        self.context = fn_nr;
        self.vars = Function::new(&name, &self.data.def(from_nr).position.file);
        self.data
            .set_returned(fn_nr, self.data.def(from_nr).returned.clone());
        for a in &args {
            let v_nr = self.create_var(&a.name, &a.typedef);
            if v_nr != u16::MAX {
                self.vars.become_argument(v_nr);
            }
        }
        // Build forwarding args for extra (non-self) attributes (e.g. RefVar(Text) buffers).
        // Variant calls must write into the dispatcher's own text-buffer argument, not a
        // freshly-allocated work_text that has no stack slot yet.
        let mut extra_call_args: Vec<Value> = Vec::new();
        let mut extra_call_types: Vec<Type> = Vec::new();
        for a in &args[1..] {
            let v = self.vars.var(&a.name);
            if v != u16::MAX {
                extra_call_args.push(Value::Var(v));
                extra_call_types.push(a.typedef.clone());
            }
        }
        let mut ls = Vec::new();
        let get_enum = self.cl("OpGetEnum", &[Value::Var(0), Value::Int(0)]);
        let get_int = self.cl("OpConvIntFromEnum", &[get_enum]);
        self.enum_numbers(
            nrs.to_vec(),
            &name,
            &mut ls,
            &get_int,
            &extra_call_args,
            &extra_call_types,
        );
        ls.push(Value::Null);
        self.data.definitions[fn_nr as usize].code =
            v_block(ls, self.data.def(from_nr).returned.clone(), "dynamic_fn");
        self.data.definitions[self.context as usize].variables = self.vars.clone();
        self.warn_missing_enum_variants(e_nr, nrs, &name);
    }

    pub(crate) fn enum_fn(&mut self) {
        if !self.first_pass {
            return;
        }
        let mut todo = HashMap::new();
        for (d_nr, d) in self.data.definitions.iter().enumerate() {
            if d.def_type != DefType::Function || d.attributes.is_empty() {
                continue;
            }
            if let Type::Reference(e_tp, _) = &d.attributes[0].typedef
                && matches!(self.data.def(*e_tp).returned, Type::Enum(_, true, _))
                && self
                    .data
                    .find_fn(u16::MAX, &d.original_name(), &self.data.def(*e_tp).returned)
                    == u32::MAX
                && let Type::Enum(e_nr, true, _) = self.data.def(*e_tp).returned
            {
                todo.entry(e_nr).or_insert(vec![]).push(d_nr);
            }
        }
        for (e_nr, nrs) in todo {
            self.create_enum_dispatch_fn(e_nr, &nrs);
        }
    }

    pub(crate) fn enum_numbers(
        &mut self,
        nrs: Vec<usize>,
        name: &str,
        ls: &mut Vec<Value>,
        get_int: &Value,
        extra_args: &[Value],
        extra_types: &[Type],
    ) {
        for nr in nrs {
            let d_nr = nr as u32;
            let a_nr = if let Type::Reference(nr, _) = self.data.def(d_nr).attributes[0].typedef {
                nr
            } else {
                0
            };
            let e_nr = if let Value::Enum(nr, _) = self.data.def(a_nr).attributes[0].value {
                nr
            } else {
                0
            };
            let self_type = self.data.def(d_nr).attributes[0].typedef.clone();
            let mut call_args = vec![Value::Var(0)];
            call_args.extend_from_slice(extra_args);
            let mut call_types = vec![self_type];
            call_types.extend_from_slice(extra_types);
            let mut code = Value::Null;
            self.call(&mut code, u16::MAX, name, &call_args, &call_types);
            let ret_call = v_block(
                vec![Value::Return(Box::new(code.clone()))],
                Type::Void,
                "ret",
            );
            ls.push(v_if(
                self.cl("OpEqInt", &[get_int.clone(), Value::Int(i32::from(e_nr))]),
                ret_call,
                Value::Null,
            ));
        }
    }

    /// Parse the `{ Value { fields }, Value, ... }` body of an enum definition.
    /// Returns false if a fatal parse error occurred and parsing should stop.
    pub(crate) fn parse_enum_values(&mut self, d_nr: u32) -> bool {
        let mut nr: u8 = 0;
        loop {
            let Some(value_name) = self.lexer.has_identifier() else {
                diagnostic!(self.lexer, Level::Error, "Expect name in type definition");
                return false;
            };
            if !is_camel(&value_name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect enum values to be in camel case style"
                );
            }
            let v_nr = if self.first_pass {
                let v = self
                    .data
                    .add_def(&value_name, self.lexer.pos(), DefType::EnumValue);
                self.data.definitions[v as usize].parent = d_nr;
                v
            } else {
                self.data.def_nr(&value_name)
            };
            if self.lexer.has_token("{") {
                if self.first_pass {
                    self.data.definitions[d_nr as usize].returned =
                        Type::Enum(d_nr, true, Vec::new());
                    self.data
                        .set_returned(v_nr, Type::Enum(d_nr, true, Vec::new()));
                    self.data.add_attribute(
                        &mut self.lexer,
                        d_nr,
                        &value_name,
                        Type::Enum(d_nr, true, Vec::new()),
                    );
                    self.data.definitions[d_nr as usize].attributes[nr as usize].constant = true;
                    // Enum values start with 1 as 0 is de null/undefined value.
                    self.data
                        .set_attr_value(d_nr, nr as usize, Value::Enum(nr + 1, u16::MAX));
                    // Create an "enum" field inside the new structure
                    let e_attr = self.data.add_attribute(
                        &mut self.lexer,
                        v_nr,
                        "enum",
                        Type::Enum(self.data.def_nr("enumerate"), false, Vec::new()),
                    );
                    // Enum values start with 1 as 0 is de null/undefined value.
                    self.data
                        .set_attr_value(v_nr, e_attr, Value::Enum(nr + 1, u16::MAX));
                }
                loop {
                    let Some(a_name) = self.lexer.has_identifier() else {
                        diagnostic!(self.lexer, Level::Error, "Expect attribute");
                        return true;
                    };
                    if self.first_pass && self.data.attr(v_nr, &a_name) != usize::MAX {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "field `{}` is already declared",
                            a_name
                        );
                    }
                    self.lexer.token(":");
                    self.parse_field(v_nr, &a_name);
                    if !self.lexer.has_token(",") {
                        break;
                    }
                }
                self.lexer.token("}");
            } else if self.first_pass {
                self.data
                    .set_returned(v_nr, Type::Enum(d_nr, false, Vec::new()));
                self.data.add_attribute(
                    &mut self.lexer,
                    d_nr,
                    &value_name,
                    Type::Enum(d_nr, false, Vec::new()),
                );
                self.data.definitions[d_nr as usize].attributes[nr as usize].constant = true;
                // Enum values start with 1 as 0 is de null/undefined value.
                self.data
                    .set_attr_value(d_nr, nr as usize, Value::Enum(nr + 1, u16::MAX));
            } else if self.data.def(d_nr).returned != self.data.def(v_nr).returned {
                self.data.definitions[v_nr as usize].returned =
                    self.data.def(d_nr).returned.clone();
            }
            if !self.lexer.has_token(",") {
                break;
            }
            if nr == 255 {
                self.lexer
                    .diagnostic(Level::Error, "Too many enumerate values");
                break;
            }
            nr += 1;
        }
        true
    }

    // <enum> ::= 'enum' <identifier> '{' <value> {, <value>} '}' [';']
    pub(crate) fn parse_enum(&mut self) -> bool {
        if !self.lexer.has_token("enum") {
            return false;
        }
        let Some(type_name) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect name in type definition");
            return false;
        };
        if !is_camel(&type_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect enum definitions to be in camel case style"
            );
        }
        let mut d_nr = self.data.def_nr(&type_name);
        if d_nr == u32::MAX {
            let pos = self.lexer.pos();
            d_nr = self.data.add_def(&type_name, pos, DefType::Enum);
        } else if self.first_pass && self.data.def_type(d_nr) == DefType::Unknown {
            self.data.definitions[d_nr as usize].def_type = DefType::Enum;
            self.data.definitions[d_nr as usize].position = self.lexer.pos().clone();
        } else if self.first_pass {
            diagnostic!(self.lexer, Level::Error, "Cannot redefine {type_name}");
        }
        if self.first_pass {
            self.data
                .set_returned(d_nr, Type::Enum(d_nr, false, Vec::new()));
        }
        if !self.lexer.token("{") {
            return false;
        }
        if !self.parse_enum_values(d_nr) {
            return false;
        }
        if self.first_pass {
            complete_definition(&mut self.lexer, &mut self.data, d_nr);
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        true
    }

    // <typedef> ::= 'type' <identifier> '=' <type_def> [ 'size' '(' <integer> ')' ] ';'
    pub(crate) fn parse_typedef(&mut self) -> bool {
        if !self.lexer.has_token("type") {
            return false;
        }
        let Some(type_name) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect name in type definition");
            return false;
        };
        if !self.default && !is_camel(&type_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect type definitions to be in camel case style"
            );
        }
        let d_nr = if self.first_pass {
            self.data
                .add_def(&type_name, self.lexer.pos(), DefType::Type)
        } else {
            self.data.def_nr(&type_name)
        };
        if self.lexer.has_token("=") {
            if let Some(type_name) = self.lexer.has_identifier() {
                if let Some(tp) = self.parse_type(d_nr, &type_name, false) {
                    if self.first_pass {
                        self.data.set_returned(d_nr, tp);
                    }
                } else if !self.first_pass {
                    diagnostic!(self.lexer, Level::Error, "'{type_name}' is not a type");
                }
            } else {
                diagnostic!(self.lexer, Level::Error, "Expected a type after =");
            }
        }
        if self.lexer.has_keyword("size") {
            self.lexer.token("(");
            self.lexer.has_integer();
            self.lexer.token(")");
        }
        if self.first_pass {
            complete_definition(&mut self.lexer, &mut self.data, d_nr);
        }
        self.lexer.token(";");
        true
    }

    // <constant>
    pub(crate) fn parse_constant(&mut self) -> bool {
        if let Some(id) = self.lexer.has_identifier() {
            self.lexer.token("=");
            if !is_upper(&id) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect constants to be in upper case style"
                );
            }
            let mut val = Value::Null;
            let tp = self.expression(&mut val);
            if self.first_pass {
                let c_nr = self.data.add_def(&id, self.lexer.pos(), DefType::Constant);
                self.data.set_returned(c_nr, tp);
                self.data.definitions[c_nr as usize].code = val;
            }
            self.lexer.token(";");
            true
        } else {
            false
        }
    }

    /// Read the function name after `fn`.  In user code only identifiers are accepted.
    /// In the default library, `assert` and `panic` are also allowed even though they are
    /// keywords — they remain real functions with call-site file/line injection.
    fn parse_fn_name(&mut self) -> Option<String> {
        if let Some(name) = self.lexer.has_identifier() {
            return Some(name);
        }
        if self.default {
            if self.lexer.has_token("assert") {
                return Some("assert".to_string());
            }
            if self.lexer.has_token("panic") {
                return Some("panic".to_string());
            }
        }
        diagnostic!(
            self.lexer,
            Level::Error,
            "Expect name in function definition"
        );
        None
    }

    pub(crate) fn parse_function(&mut self) -> bool {
        if !self.lexer.has_token("fn") {
            return false;
        }
        let Some(fn_name) = self.parse_fn_name() else {
            return false;
        };
        self.vars = Function::new(&fn_name, &self.lexer.pos().file);
        if !self.default && !is_lower(&fn_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect function names to be in lower case style"
            );
        }
        let mut arguments = Vec::new();
        if self.lexer.token("(") {
            if !self.parse_arguments(&fn_name, &mut arguments) {
                return true;
            }
            self.lexer.token(")");
        }
        self.context = if self.default && self.first_pass && is_op(&fn_name) {
            self.data.add_op(&mut self.lexer, &fn_name, &arguments)
        } else if self.first_pass {
            self.data.add_fn(&mut self.lexer, &fn_name, &arguments)
        } else if self.default && is_op(&fn_name) {
            self.data.def_nr(&fn_name)
        } else {
            self.data.get_fn(&fn_name, &arguments)
        };
        if self.context == u32::MAX {
            return false;
        }
        let mut returned_not_null = false;
        let result = if self.lexer.has_token("->") {
            // Will be the correct def_nr on the second pass
            if let Some(type_name) = self.lexer.has_identifier() {
                let Some(tp) = self.parse_type(self.data.def_nr(&fn_name), &type_name, true) else {
                    // Message
                    return false;
                };
                if self.lexer.has_keyword("not") {
                    self.lexer.token("null");
                    returned_not_null = true;
                }
                tp
            } else {
                // message
                Type::Void
            }
        } else {
            Type::Void
        };
        self.vars
            .append(&mut self.data.definitions[self.context as usize].variables);
        if self.first_pass {
            self.data.set_returned(self.context, result);
            self.data.definitions[self.context as usize].returned_not_null = returned_not_null;
        }
        if !self.lexer.has_token(";") {
            for (a_nr, a) in arguments.iter().enumerate() {
                if self.first_pass {
                    let v_nr = self.create_var(&a.name, &a.typedef);
                    if v_nr != u16::MAX {
                        self.vars.become_argument(v_nr);
                        self.var_usages(v_nr, false);
                    }
                } else {
                    self.change_var_type(a_nr as u16, &a.typedef);
                    if a.constant {
                        self.vars.set_const_param(a_nr as u16);
                    }
                }
            }
            self.parse_code();
            if !self.first_pass {
                self.check_ref_mutations(&arguments);
            }
        }
        if !self.first_pass {
            // Stub functions with an empty body `{ }` and a `self` parameter are intentional
            // skips (e.g. to silence the "no implementation for variant" warning).
            // Don't warn about unused parameters in that case.
            let is_stub = {
                let def = &self.data.definitions[self.context as usize];
                let body_empty = matches!(&def.code, Value::Block(bl) if bl.operators.is_empty());
                let first_is_self = def.attributes.first().is_some_and(|a| a.name == "self");
                body_empty && first_is_self
            };
            if !is_stub {
                self.vars.test_used(&mut self.lexer, &self.data);
            }
        }
        self.lexer.has_token(";");
        self.parse_rust();
        self.data.op_code(self.context);
        self.data.definitions[self.context as usize]
            .variables
            .append(&mut self.vars);
        self.context = u32::MAX;
        true
    }

    // <rust> ::= { '#rust' <string> | '#iterator' <string> <string> }
    pub(crate) fn parse_rust(&mut self) {
        while self.default && self.lexer.has_token("#") {
            let id = self.lexer.has_identifier();
            if id == Some("rust".to_string()) {
                if let Some(c) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust = c;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust string");
                }
            }
            if id == Some("iterator".to_string()) {
                if let Some(init) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust = init;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust init string");
                }
                if let Some(next) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust += "#";
                    self.data.definitions[self.context as usize].rust += &next;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust next string");
                }
            }
        }
    }

    pub(crate) fn parse_arguments(&mut self, fn_name: &str, arguments: &mut Vec<Argument>) -> bool {
        loop {
            if self.lexer.peek_token(")") {
                break;
            }
            let Some(attr_name) = self.lexer.has_identifier() else {
                diagnostic!(self.lexer, Level::Error, "Expect attribute");
                return false;
            };
            if !is_lower(&attr_name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect function attributes to be in lower case style"
                );
            }
            for a in arguments.iter() {
                if attr_name == a.name {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Double attribute '{fn_name}.{attr_name}'"
                    );
                }
            }
            let mut constant = false;
            let mut reference = false;
            let typedef = if self.lexer.has_token(":") {
                if self.lexer.has_token("&") {
                    reference = true;
                }
                // Will be the correct def_nr on the second pass
                if self.lexer.has_token("fn") {
                    self.parse_fn_type(self.data.def_nr(fn_name))
                } else {
                    if self.lexer.has_keyword("const") {
                        constant = true;
                    }
                    if let Some(type_name) = self.lexer.has_identifier() {
                        if let Some(tp) =
                            self.parse_type(self.data.def_nr(fn_name), &type_name, false)
                        {
                            if reference {
                                Type::RefVar(Box::new(tp))
                            } else {
                                tp
                            }
                        } else {
                            if !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "'{type_name}' is not a type"
                                );
                            }
                            Type::Unknown(0)
                        }
                    } else {
                        diagnostic!(self.lexer, Level::Error, "Expecting a type");
                        return true;
                    }
                }
            } else {
                Type::Unknown(0)
            };
            let val = if self.lexer.has_token("=") {
                let mut t = Value::Var(arguments.len() as u16);
                self.expression(&mut t);
                t
            } else {
                Value::Null
            };
            if !self.first_pass
                && typedef.is_unknown()
                && val == Value::Null
                && (!self.default || !matches!(typedef, Type::Vector(_, _)))
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expecting a clear type, found {}",
                    typedef.name(&self.data)
                );
            }
            (*arguments).push(Argument {
                name: attr_name,
                typedef,
                default: val,
                constant,
            });
            if !self.lexer.has_token(",") {
                break;
            }
        }
        true
    }

    pub(crate) fn parse_fn_type(&mut self, d_nr: u32) -> Type {
        let mut r_type = Type::Void;
        let mut args = Vec::new();
        self.lexer.token("(");
        loop {
            if let Some(id) = self.lexer.has_identifier()
                && let Some(tp) = self.parse_type(d_nr, &id, false)
            {
                args.push(tp);
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        if self.lexer.has_token("->")
            && let Some(id) = self.lexer.has_identifier()
            && let Some(tp2) = self.parse_type(d_nr, &id, false)
        {
            r_type = tp2;
        }
        Type::Function(args, Box::new(r_type))
    }

    // <type> ::= <identifier> [::<identifier>] [ '<' ( <sub_type> | <type> ) '>' ] [ <depend> ]
    pub(crate) fn parse_type(
        &mut self,
        on_d: u32,
        type_name: &str,
        returned: bool,
    ) -> Option<Type> {
        let tp_nr = if self.lexer.has_token("::") {
            if let Some(name) = self.lexer.has_identifier() {
                let source = self.data.get_source(type_name);
                self.data.source_nr(source, &name)
            } else {
                diagnostic!(self.lexer, Level::Error, "Expect type from {type_name}");
                return None;
            }
        } else {
            self.data.def_nr(type_name)
        };
        if self.first_pass && tp_nr == u32::MAX && type_name != "spacial" {
            let u_nr = self
                .data
                .add_def(type_name, self.lexer.pos(), DefType::Unknown);
            return Some(Type::Unknown(u_nr));
        }
        if tp_nr != u32::MAX && self.data.def_type(tp_nr) == DefType::Unknown {
            return Some(Type::Unknown(tp_nr));
        }
        let link = self.lexer.link();
        if self.lexer.has_token("<")
            && let Some(value) = self.sub_type(on_d, type_name, link)
        {
            return Some(value);
        }
        let mut dep = Vec::new();
        self.parse_depended(returned, &mut dep);
        let mut min = 0;
        let mut max = 0;
        if type_name == "integer" && self.parse_type_limit(&mut min, &mut max) {
            return Some(Type::Integer(min, max));
        }
        let dt = self.data.def_type(tp_nr);
        if tp_nr != u32::MAX
            && matches!(
                dt,
                DefType::Type | DefType::Enum | DefType::EnumValue | DefType::Struct
            )
        {
            if matches!(dt, DefType::EnumValue)
                || (self.first_pass && matches!(dt, DefType::Struct))
            {
                Some(Type::Reference(tp_nr, dep))
            } else if matches!(self.data.def(tp_nr).returned, Type::Text(_)) {
                Some(Type::Text(dep))
            } else {
                Some(self.data.def(tp_nr).returned.clone())
            }
        } else {
            None
        }
    }

    pub(crate) fn sub_type(&mut self, on_d: u32, type_name: &str, link: Link) -> Option<Type> {
        if let Some(sub_name) = self.lexer.has_identifier() {
            if let Some(tp) = self.parse_type(on_d, &sub_name, false) {
                let sub_nr = if let Type::Unknown(d) = tp {
                    d
                } else {
                    self.data.type_def_nr(&tp)
                };
                let mut fields = Vec::new();
                return Some(match type_name {
                    "index" => {
                        self.parse_fields(true, &mut fields);
                        Type::Index(self.data.type_def_nr(&tp), fields, Vec::new())
                    }
                    "hash" => {
                        self.parse_fields(false, &mut fields);
                        self.data.set_referenced(sub_nr, on_d, Value::Null);
                        let mut f = Vec::new();
                        for (field, _) in fields {
                            f.push(field);
                        }
                        Type::Hash(sub_nr, f, Vec::new())
                    }
                    "vector" => {
                        self.lexer.token(">");
                        Type::Vector(Box::new(tp), Vec::new())
                    }
                    "sorted" => {
                        self.parse_fields(true, &mut fields);
                        Type::Sorted(sub_nr, fields, Vec::new())
                    }
                    "spacial" => {
                        // Consume remaining ", field, ..." tokens up to the closing >.
                        while !self.lexer.has_token(">") {
                            self.lexer.has_token(",");
                            self.lexer.has_identifier();
                        }
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "spacial<T> is not yet implemented; use sorted<T> or index<T> for ordered lookups"
                        );
                        Type::Unknown(0)
                    }
                    "reference" => {
                        self.lexer.token(">");
                        self.data.set_referenced(sub_nr, on_d, Value::Null);
                        Type::Reference(sub_nr, Vec::new())
                    }
                    "iterator" => {
                        self.lexer.token(",");
                        let mut it_tp = Type::Null;
                        if let Some(iter) = self.lexer.has_identifier() {
                            if let Some(it) = self.parse_type(on_d, &iter, false) {
                                self.data.set_referenced(sub_nr, on_d, Value::Null);
                                it_tp = it;
                            } else {
                                diagnostic!(self.lexer, Level::Error, "Expect an iterator type");
                            }
                        } else {
                            diagnostic!(self.lexer, Level::Error, "Expect an iterator type");
                        }
                        self.lexer.token(">");
                        Type::Iterator(Box::new(tp), Box::new(it_tp))
                    }
                    _ => {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Subtype only allowed on structures"
                        );
                        Type::Unknown(0)
                    }
                });
            }
            assert!(self.first_pass, "Incorrect handling of unknown types");
        } else {
            self.lexer.revert(link);
        }
        None
    }

    // <depend> ::= '[' { <field> [ ',' ] } ']'
    pub(crate) fn parse_depended(&mut self, returned: bool, dep: &mut Vec<u16>) {
        if self.default && returned && self.lexer.has_token("[") && self.context != u32::MAX {
            loop {
                if let Some(id) = self.lexer.has_identifier() {
                    if let Some(nr) = self.data.def(self.context).attr_names.get(&id) {
                        dep.push(*nr as u16);
                    } else {
                        diagnostic!(self.lexer, Level::Error, "Unknown field name '{id}'");
                    }
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expected a field name");
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("]");
        }
    }

    pub(crate) fn parse_fields(&mut self, directions: bool, result: &mut Vec<(String, bool)>) {
        self.lexer.token("[");
        loop {
            let desc = self.lexer.has_token("-");
            if !directions && desc {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Structure doesn't support descending fields"
                );
            }
            if let Some(field) = self.lexer.has_identifier() {
                result.push((field, !desc));
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("]");
        self.lexer.token(">");
    }

    // <field_limit> ::= 'limit' '(' [ '-' ] <min-integer> ',' [ '-' ] <max-integer> ')'
    pub(crate) fn parse_type_limit(&mut self, min: &mut i32, max: &mut u32) -> bool {
        if self.lexer.has_keyword("limit") {
            self.lexer.token("(");
            let min_neg = self.lexer.has_token("-");
            if let Some(nr) = self.lexer.has_integer() {
                *min = if min_neg { -(nr as i32) } else { nr as i32 };
            }
            self.lexer.token(",");
            if let Some(nr) = self.lexer.has_integer() {
                *max = nr;
            }
            self.lexer.token(")");
            true
        } else {
            false
        }
    }

    // <struct> = 'struct' <identifier> [ ':' <type> ] '{' <param-id> ':' <field> { ',' <param-id> ':' <field> } '}'
    pub(crate) fn parse_struct(&mut self) -> bool {
        if !self.lexer.has_token("struct") {
            return false;
        }
        let Some(id) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect attribute");
            return true;
        };
        let mut d_nr = self.data.def_nr(&id);
        if d_nr == u32::MAX {
            d_nr = self.data.add_def(&id, self.lexer.pos(), DefType::Struct);
            self.data.definitions[d_nr as usize].returned = Type::Reference(d_nr, Vec::new());
        } else if self.first_pass {
            if let Type::Unknown(_) = self.data.definitions[d_nr as usize].returned {
                self.data.definitions[d_nr as usize].position = self.lexer.pos().clone();
                self.data.definitions[d_nr as usize].def_type = DefType::Struct;
                self.data.definitions[d_nr as usize].returned = Type::Reference(d_nr, Vec::new());
            } else {
                diagnostic!(self.lexer, Level::Error, "Redefined struct {}", id);
            }
        }
        let context = self.context;
        self.context = d_nr;
        self.lexer.token("{");
        loop {
            self.lexer.has_token("pub");
            let Some(a_name) = self.lexer.has_identifier() else {
                diagnostic!(self.lexer, Level::Error, "Expect attribute");
                self.context = context;
                return true;
            };
            if self.first_pass && self.data.attr(d_nr, &a_name) != usize::MAX {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "field `{}` is already declared",
                    a_name
                );
            }
            self.lexer.token(":");
            self.parse_field(d_nr, &a_name);
            if !self.lexer.has_token(",") || self.lexer.peek_token("}") {
                break;
            }
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        self.context = context;
        true
    }

    // <field> ::= { <field_limit> | 'not' 'null' | <field_default> | 'check' '(' <expr> ')' | <type-id> [ '[' ['-'] <field> { ',' ['-'] <field> } ']' ] } }
    pub(crate) fn parse_field(&mut self, d_nr: u32, a_name: &String) {
        let mut a_type: Type = Type::Unknown(0);
        let mut defined = false;
        let mut value = Value::Null;
        let mut check = Value::Null;
        let mut check_message = Value::Null;
        let mut nullable = true;
        let mut is_computed = false;
        loop {
            if self.lexer.has_keyword("not") {
                // This field cannot be null, this allows for 256 values in a byte
                self.lexer.token("null");
                nullable = false;
            }
            is_computed |= self.parse_field_default(&mut value, &mut a_type, d_nr, a_name, &mut defined);
            if self.lexer.has_token("assert") {
                // L6: assert(condition) or assert(condition, message) on struct fields.
                self.lexer.token("(");
                self.expression(&mut check);
                if self.lexer.has_token(",") {
                    self.expression(&mut check_message);
                }
                self.lexer.token(")");
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "CHECK" {
                    // Legacy CHECK syntax — parse and discard for backward compat
                    self.lexer.token("(");
                    let mut p = Value::Null;
                    self.expression(&mut p);
                    if self.lexer.has_token(",") {
                        let mut q = Value::Null;
                        self.expression(&mut q);
                    }
                    self.lexer.token(")");
                } else if let Some(tp) = self.parse_type(d_nr, &id, false) {
                    defined = true;
                    a_type = tp;
                    // '= expr' shorthand for a field default value
                    if self.lexer.has_token("=") {
                        let tp = self.expression(&mut value);
                        if a_type.is_unknown() {
                            a_type = tp;
                        }
                    }
                }
            } else {
                break;
            }
        }
        if !defined {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Attribute {a_name} needs type or definition"
            );
        }
        if self.first_pass {
            let a = self
                .data
                .add_attribute(&mut self.lexer, d_nr, a_name, a_type);
            self.data.set_attr_nullable(d_nr, a, nullable);
            self.data.set_attr_value(d_nr, a, value);
            if is_computed {
                self.data.definitions[d_nr as usize].attributes[a].constant = true;
            }
            if check != Value::Null {
                self.data.definitions[d_nr as usize].attributes[a].check = check;
                self.data.definitions[d_nr as usize].attributes[a].check_message = check_message;
            }
        } else {
            let a = self.data.attr(d_nr, a_name);
            if is_computed {
                self.data.definitions[d_nr as usize].attributes[a].constant = true;
            }
            if value != Value::Null {
                self.data.set_attr_value(d_nr, a, value);
            }
            if check != Value::Null {
                self.data.definitions[d_nr as usize].attributes[a].check = check;
                self.data.definitions[d_nr as usize].attributes[a].check_message = check_message;
            }
        }
    }

    // <field_default> ::= 'virtual' <value-expr> | 'default' '(' <value-expr> ')'
    pub(crate) fn parse_field_default(
        &mut self,
        value: &mut Value,
        a_type: &mut Type,
        _d_nr: u32,
        _a_name: &String,
        defined: &mut bool,
    ) -> bool {
        let mut is_computed = false;
        if self.lexer.has_keyword("computed") || self.lexer.has_keyword("virtual") {
            is_computed = true;
            // Computed field: calculate on every access, no store space.
            // The expression is stored directly in the attribute value and inlined
            // at every access site.  Var(0) references the record (replaced by
            // replace_record_ref at the access site).
            self.lexer.token("(");
            let tp = self.expression(value);
            if a_type.is_unknown() {
                *a_type = tp;
                *defined = true;
            } else {
                self.convert(value, &tp, a_type);
            }
            self.lexer.token(")");
        }
        if self.lexer.has_keyword("default") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "default(expr) is removed; use 'computed(expr)' for calculated fields or '= expr' for stored defaults"
            );
            // Consume the expression to recover parsing.
            self.lexer.token("(");
            self.expression(value);
            self.lexer.token(")");
        }
        is_computed
    }
}
