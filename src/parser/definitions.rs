// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use super::{
    Argument, DefType, Function, HashMap, HashSet, Level, Link, Parser, Position, ToString, Type,
    Value, complete_definition, diagnostic_format, is_camel, is_lower, is_op, is_upper, rename,
    v_block, v_if,
};

impl Parser {
    /// Check whether a type tree contains a reference to a specific definition.
    /// Used to validate that a generic type variable appears in a parameter type.
    fn type_contains_def(tp: &Type, d_nr: u32) -> bool {
        match tp {
            Type::Reference(d, _) | Type::Unknown(d) | Type::Enum(d, _, _) => *d == d_nr,
            Type::Vector(inner, _) => Self::type_contains_def(inner, d_nr),
            _ => false,
        }
    }

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
            self.call(&mut code, u16::MAX, name, &call_args, &call_types, &[]);
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

    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_function(&mut self) -> bool {
        if !self.lexer.has_token("fn") {
            return false;
        }
        let Some(fn_name) = self.parse_fn_name() else {
            return false;
        };
        self.vars = Function::new(&fn_name, &self.lexer.pos().file);
        if !self.default && !is_lower(&fn_name) && !is_op(&fn_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect function names to be in lower case style"
            );
        }
        // P5.1: detect `<T>` type parameter after function name.
        let mut is_generic = false;
        let mut type_var_name = String::new();
        // I4: bound names collected from `<T: A + B>` — resolved to def_nrs in the second pass.
        let mut pending_bounds: Vec<String> = Vec::new();
        if self.lexer.has_token("<") {
            if let Some(tv) = self.lexer.has_identifier() {
                if !is_camel(&tv) && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Type variable '{}' must be CamelCase",
                        tv
                    );
                }
                type_var_name = tv;
                is_generic = true;
                // I4: parse `<T: A + B>` bound list; collect raw names here, resolve in second pass.
                if self.lexer.has_token(":") {
                    loop {
                        if let Some(bound_name) = self.lexer.has_identifier() {
                            pending_bounds.push(bound_name);
                        } else if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Expected interface name in type bound"
                            );
                        }
                        if !self.lexer.has_token("+") {
                            break;
                        }
                    }
                }
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expected type variable name after '<'"
                );
            }
            self.lexer.token(">");
        }
        let mut arguments = Vec::new();
        if self.lexer.token("(") {
            // P5.1: register the type variable as a struct so parse_type
            // resolves it to Reference(d, []).  The definition is never
            // compiled — it only exists for the template's type resolution.
            if is_generic && self.first_pass && self.data.def_nr(&type_var_name) == u32::MAX {
                let tv_nr = self
                    .data
                    .add_def(&type_var_name, self.lexer.pos(), DefType::Struct);
                self.data
                    .set_returned(tv_nr, Type::Reference(tv_nr, Vec::new()));
            }
            if !self.parse_arguments(&fn_name, &mut arguments) {
                return true;
            }
            self.lexer.token(")");
        }
        // P5.1: validate that the type variable appears in the first parameter.
        if is_generic && !arguments.is_empty() {
            let tv_nr = self.data.def_nr(&type_var_name);
            let has_tv = Self::type_contains_def(&arguments[0].typedef, tv_nr);
            if !has_tv && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Type variable {} must appear in the first parameter — \
                     move {} to the first parameter position",
                    type_var_name,
                    type_var_name
                );
            }
        } else if is_generic && arguments.is_empty() && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Generic function must have at least one parameter of type {}",
                type_var_name
            );
        }
        self.context = if self.default && self.first_pass && is_op(&fn_name) {
            self.data.add_op(&mut self.lexer, &fn_name, &arguments)
        } else if self.first_pass {
            let d = self.data.add_fn(&mut self.lexer, &fn_name, &arguments);
            if is_generic && d != u32::MAX {
                self.data.definitions[d as usize].def_type = DefType::Generic;
            }
            d
        } else if self.default && is_op(&fn_name) {
            self.data.def_nr(&fn_name)
        } else {
            self.data.get_fn(&fn_name, &arguments)
        };
        if self.context == u32::MAX {
            return false;
        }
        // I4: resolve pending bound names to interface def_nrs in the second pass.
        if !self.first_pass && !pending_bounds.is_empty() {
            let mut bounds = Vec::new();
            for bname in &pending_bounds {
                let b_nr = self.data.def_nr(bname);
                if b_nr == u32::MAX {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a known interface",
                        bname
                    );
                } else if !matches!(self.data.def_type(b_nr), DefType::Interface) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not an interface — bounds must be interface names",
                        bname
                    );
                } else {
                    bounds.push(b_nr);
                }
            }
            self.data.definitions[self.context as usize].bounds = bounds;
            // I7/I8.1: Create T-parameterized stubs for each bound interface's methods so
            // the body parser can emit `Value::Call(t_stub_nr, ...)` for method/op calls on T.
            // `re_resolve_call` then substitutes these with the concrete type's implementation.
            let tv_nr = self.data.def_nr(&type_var_name);
            let self_nr = self.data.def_nr("Self");
            if tv_nr != u32::MAX && self_nr != u32::MAX {
                let self_prefix = format!("t_{}Self_", "Self".len());
                let iface_nrs: Vec<u32> =
                    self.data.definitions[self.context as usize].bounds.clone();
                for iface_nr in iface_nrs {
                    let children: Vec<u32> = self.data.children_of(iface_nr).collect();
                    for child_nr in children {
                        let child_name = self.data.def(child_nr).name.clone();
                        // Extract method name from interface-scoped stub names:
                        // "__iface_{d_nr}_{method}" → "method"
                        // Also handle legacy "t_4Self_{method}" format.
                        let method_suffix = if let Some(rest) = child_name.strip_prefix("__iface_")
                        {
                            rest.split_once('_')
                                .map_or(rest.to_string(), |(_, m)| m.to_string())
                        } else if child_name.starts_with(&self_prefix) {
                            child_name[self_prefix.len()..].to_string()
                        } else {
                            child_name.clone()
                        };
                        let t_stub_name = format!(
                            "t_{}{}_{}",
                            type_var_name.len(),
                            type_var_name,
                            method_suffix
                        );
                        if self.data.def_nr(&t_stub_name) != u32::MAX {
                            continue; // already created (e.g. multiple bounds share a method)
                        }
                        let attrs_count = self.data.def(child_nr).attributes.len();
                        let t_stub_nr =
                            self.data
                                .add_def(&t_stub_name, self.lexer.pos(), DefType::Function);
                        for a_nr in 0..attrs_count {
                            let a_name = self.data.attr_name(child_nr, a_nr);
                            let a_type = self.data.attr_type(child_nr, a_nr);
                            let new_type = Self::substitute_type(
                                a_type,
                                self_nr,
                                &crate::data::Type::Reference(tv_nr, Vec::new()),
                            );
                            self.data
                                .add_attribute(&mut self.lexer, t_stub_nr, &a_name, new_type);
                        }
                        let ret_type = self.data.def(child_nr).returned.clone();
                        let t_ret_type = Self::substitute_type(
                            ret_type,
                            self_nr,
                            &crate::data::Type::Reference(tv_nr, Vec::new()),
                        );
                        self.data.set_returned(t_stub_nr, t_ret_type.clone());
                        // I9-text: if the interface method returns text, add the hidden
                        // __work_1 parameter that text_return would add for concrete
                        // implementations.  Without this, the call-site argument count
                        // won't match after re_resolve_call substitutes the concrete
                        // text-returning method (which has the hidden param).
                        if matches!(t_ret_type, crate::data::Type::Text(_)) {
                            self.data.add_attribute(
                                &mut self.lexer,
                                t_stub_nr,
                                "__work_1",
                                crate::data::Type::RefVar(Box::new(crate::data::Type::Text(
                                    Vec::new(),
                                ))),
                            );
                        }
                    }
                }
            }
        }
        let mut returned_not_null = false;
        let result = if self.lexer.has_token("->") {
            // Will be the correct def_nr on the second pass
            if let Some(tp) = self.parse_type_full(self.data.def_nr(&fn_name), true) {
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
            // C47.4: reset transient closure state after each function body.
            // Without this, a lambda inside make_adder leaks last_closure_work_var
            // into the next function parsed (main), causing closure_var_of to
            // return a stale value for add5 = make_adder(5).
            self.last_closure_work_var = u16::MAX;
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
    // <native> ::= '#native' <string>   (any file)
    pub(crate) fn parse_rust(&mut self) {
        loop {
            if !self.lexer.peek_token("#") {
                break;
            }
            // Speculatively consume `#`; revert if the annotation is not recognised.
            let link = self.lexer.link();
            self.lexer.has_token("#");
            let id = self.lexer.has_identifier();
            if id == Some("native".to_string()) {
                if let Some(sym) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].native = sym;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect native symbol string");
                }
            } else if self.default && id == Some("rust".to_string()) {
                if let Some(c) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust = c;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust string");
                }
            } else if self.default && id == Some("iterator".to_string()) {
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
            } else {
                // Not a recognised annotation — put the `#` back and stop.
                self.lexer.revert(link);
                break;
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
                if self.lexer.has_keyword("const") {
                    constant = true;
                }
                if let Some(tp) = self.parse_type_full(self.data.def_nr(fn_name), false) {
                    if reference {
                        Type::RefVar(Box::new(tp))
                    } else {
                        tp
                    }
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expecting a type");
                    return true;
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
            if self.lexer.peek_token(")") {
                break;
            }
            if let Some(tp) = self.parse_type_full(d_nr, false) {
                args.push(tp);
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        if self.lexer.has_token("->")
            && let Some(tp2) = self.parse_type_full(d_nr, false)
        {
            r_type = tp2;
        }
        Type::Function(args, Box::new(r_type), vec![])
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
        let mut min = i32::MIN + 1;
        let mut max = i32::MAX as u32;
        if type_name == "integer" {
            let has_limit = self.parse_type_limit(&mut min, &mut max);
            // T1.7: check for `not null` annotation after the integer type
            let not_null = if self.lexer.has_keyword("not") {
                self.lexer.token("null");
                true
            } else {
                false
            };
            if has_limit || not_null {
                return Some(Type::Integer(min, max, not_null));
            }
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

    /// Parse a type expression that may be a tuple `(T1, T2, ...)` or an identifier-based type.
    /// This is the entry point for type positions (return types, parameter types, annotations).
    pub(crate) fn parse_type_full(&mut self, on_d: u32, returned: bool) -> Option<Type> {
        if self.lexer.has_token("(") {
            // Tuple type: (T1, T2, ...)
            let mut types = Vec::new();
            loop {
                if self.lexer.peek_token(")") {
                    break;
                }
                if let Some(tp) = self.parse_type_full(on_d, false) {
                    types.push(tp);
                } else {
                    break;
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token(")");
            if types.len() < 2 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Tuple types require at least 2 elements"
                );
                return types.into_iter().next();
            }
            Some(Type::Tuple(types))
        } else if self.lexer.has_token("fn") {
            Some(self.parse_fn_type(on_d))
        } else if let Some(id) = self.lexer.has_identifier() {
            self.parse_type(on_d, &id, returned)
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
                        // CO1.3c: comma and second type are optional for generators.
                        // iterator<T> = generator yield type; iterator<T, I> = collection iterator.
                        let mut it_tp = Type::Null;
                        if self.lexer.has_token(",") {
                            if let Some(iter) = self.lexer.has_identifier() {
                                if let Some(it) = self.parse_type(on_d, &iter, false) {
                                    self.data.set_referenced(sub_nr, on_d, Value::Null);
                                    it_tp = it;
                                } else {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "Expect an iterator type"
                                    );
                                }
                            } else {
                                diagnostic!(self.lexer, Level::Error, "Expect an iterator type");
                            }
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
            // fix-tvscope: a type variable placeholder (e.g., `T` from generic stdlib
            // functions) blocks user-defined struct of the same name.  Produce a clear
            // diagnostic rather than the confusing "Redefined struct".
            let is_type_var = {
                let ex = &self.data.definitions[d_nr as usize];
                ex.def_type == DefType::Struct
                    && ex.attributes.is_empty()
                    && matches!(&ex.returned, Type::Reference(r, _) if *r == d_nr)
            };
            if is_type_var {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}' is reserved as a generic type variable — choose a different struct name",
                    id
                );
            } else if matches!(
                self.data.definitions[d_nr as usize].returned,
                Type::Unknown(_)
            ) {
                self.data.definitions[d_nr as usize].position = self.lexer.pos().clone();
                self.data.definitions[d_nr as usize].def_type = DefType::Struct;
                self.data.definitions[d_nr as usize].returned = Type::Reference(d_nr, Vec::new());
            } else {
                diagnostic!(self.lexer, Level::Error, "Cannot redefine struct '{id}'");
            }
        }
        let context = self.context;
        self.context = d_nr;
        self.lexer.token("{");
        // #91: collect init field dependency info for circular detection.
        let mut init_deps: Vec<(String, Vec<String>)> = Vec::new();
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
            self.init_field_deps.clear();
            self.parse_field(d_nr, &a_name);
            if !self.init_field_deps.is_empty() {
                init_deps.push((a_name.clone(), self.init_field_deps.clone()));
            }
            if !self.lexer.has_token(",") || self.lexer.peek_token("}") {
                break;
            }
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        // #91: check for circular init dependencies (second pass, all fields known).
        if !self.first_pass {
            self.check_circular_init(&init_deps);
        }
        self.context = context;
        true
    }

    /// I3: parse an `interface` declaration and register it as `DefType::Interface`.
    ///
    /// Syntax: `interface Name { fn method(params) -> type [;] ... }`
    ///
    /// Method signatures are parsed for syntactic correctness (param/return types
    /// resolved against the current scope).  `Self` is a placeholder type that
    /// refers to the concrete satisfying type at instantiation (I6).
    ///
    /// This first-pass implementation registers the interface definition and
    /// verifies syntax; semantic satisfaction checking comes in I5/I6.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_interface(&mut self) -> bool {
        if !self.lexer.has_token("interface") {
            return false;
        }
        let Some(id) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect interface name");
            return true;
        };
        if !is_camel(&id) && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Interface name '{}' must be CamelCase",
                id
            );
        }
        // Register or locate the interface definition.
        let mut d_nr = self.data.def_nr(&id);
        if d_nr == u32::MAX {
            if self.first_pass {
                d_nr = self.data.add_def(&id, self.lexer.pos(), DefType::Interface);
            }
        } else if self.first_pass {
            diagnostic!(self.lexer, Level::Error, "Cannot redefine interface '{id}'");
        }
        // I3: register 'Self' as a type placeholder for method signature parsing.
        // 'Self' resolves to its own definition (like a generic type variable) so
        // that parse_type_full succeeds.  I6 substitutes the concrete satisfying type.
        if self.first_pass && self.data.def_nr("Self") == u32::MAX {
            let self_nr = self.data.add_def("Self", self.lexer.pos(), DefType::Struct);
            self.data
                .set_returned(self_nr, Type::Reference(self_nr, Vec::new()));
        }
        let context = self.context;
        if d_nr != u32::MAX {
            self.context = d_nr;
        }
        if !self.lexer.token("{") {
            self.context = context;
            return true;
        }
        // Parse zero or more method/operator signatures.
        while !self.lexer.peek_token("}") {
            if self.lexer.peek().has == crate::lexer::LexItem::None {
                break;
            }
            // I3.1: `op <token> (params) -> type` desugars to an `OpCamelCase` method stub.
            let method_name = if self.lexer.has_keyword("op") {
                if let crate::lexer::LexItem::Token(tok) = self.lexer.peek().has.clone() {
                    self.lexer.cont();
                    format!("Op{}", rename(&tok))
                } else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Expected operator symbol after 'op' in interface body"
                        );
                    }
                    self.lexer.cont();
                    continue;
                }
            } else {
                if !self.lexer.has_token("fn") {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "Expected 'fn' in interface body");
                    }
                    self.lexer.cont();
                    continue;
                }
                let Some(name) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Expected method name in interface"
                        );
                    }
                    break;
                };
                name
            };
            let mut args = Vec::new();
            if self.lexer.token("(") {
                self.parse_arguments(&method_name, &mut args);
                self.lexer.token(")");
            }
            let return_tp = if self.lexer.has_token("->") {
                self.parse_type_full(d_nr, true)
            } else {
                None
            };
            // I6/I9-stub: register method stubs as children of the interface.
            // Use interface-scoped names (`__iface_{d_nr}_{method}`) to avoid
            // collision when multiple interfaces declare the same operator.
            // `children_of(d_nr)` enumerates them for satisfaction checking;
            // T-stub creation strips the prefix to extract the method name.
            if self.first_pass && d_nr != u32::MAX {
                let stub_name = format!("__iface_{d_nr}_{method_name}");
                if self.data.def_nr(&stub_name) == u32::MAX {
                    let stub_nr =
                        self.data
                            .add_def(&stub_name, self.lexer.pos(), DefType::Function);
                    for a in &args {
                        self.data.add_attribute(
                            &mut self.lexer,
                            stub_nr,
                            &a.name,
                            a.typedef.clone(),
                        );
                    }
                    self.data.definitions[stub_nr as usize].parent = d_nr;
                    if let Some(ref rt) = return_tp {
                        self.data.set_returned(stub_nr, rt.clone());
                    }
                }
            }
            // I5 (phase 1): factory methods (Self in return without self: Self first param)
            // are not yet supported.  Emit a clear diagnostic rather than silently producing
            // wrong code when I6 lands.
            if !self.first_pass {
                let self_nr = self.data.def_nr("Self");
                if self_nr != u32::MAX
                    && let Some(Type::Reference(ret_nr, _)) = &return_tp
                    && *ret_nr == self_nr
                {
                    let has_self_param = args.first().is_some_and(|a| {
                        a.name == "self"
                            && matches!(&a.typedef, Type::Reference(nr, _) if *nr == self_nr)
                    });
                    if !has_self_param {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "factory methods not yet supported: '{}' returns Self without a 'self: Self' parameter",
                            method_name
                        );
                    }
                }
            }
            self.lexer.has_token(";");
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        self.context = context;
        true
    }

    /// #91: DFS cycle detection on init field dependencies.
    fn check_circular_init(&mut self, init_deps: &[(String, Vec<String>)]) {
        let names: HashSet<String> = init_deps.iter().map(|(n, _)| n.clone()).collect();
        for (start, deps) in init_deps {
            let mut visited: Vec<String> = vec![start.clone()];
            let mut stack = deps.clone();
            while let Some(dep) = stack.pop() {
                if dep == *start {
                    visited.push(start.clone());
                    let path = visited.join(" -> ");
                    diagnostic!(self.lexer, Level::Error, "circular init dependency: {path}");
                    break;
                }
                if names.contains(&dep) && !visited.contains(&dep) {
                    visited.push(dep.clone());
                    if let Some((_, subdeps)) = init_deps.iter().find(|(n, _)| *n == dep) {
                        stack.extend(subdeps.clone());
                    }
                }
            }
        }
    }

    // <field> ::= { <field_limit> | 'not' 'null' | <field_default> | 'check' '(' <expr> ')' | <type-id> [ '[' ['-'] <field> { ',' ['-'] <field> } ']' ] } }
    #[allow(clippy::too_many_lines)] // pre-existing length; T1.11a added one branch
    pub(crate) fn parse_field(&mut self, d_nr: u32, a_name: &String) {
        let mut a_type: Type = Type::Unknown(0);
        let mut defined = false;
        let mut value = Value::Null;
        let mut check = Value::Null;
        let mut check_message = Value::Null;
        let mut nullable = true;
        let mut is_computed = false;
        let mut is_init = false;
        loop {
            if self.lexer.has_keyword("not") {
                // This field cannot be null, this allows for 256 values in a byte
                self.lexer.token("null");
                nullable = false;
            }
            {
                let (comp, init) =
                    self.parse_field_default(&mut value, &mut a_type, d_nr, a_name, &mut defined);
                is_computed |= comp;
                is_init |= init;
            }
            if self.lexer.has_token("assert") {
                // assert(condition) or assert(condition, message) on struct fields.
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
            } else if let Some(tp) = self.parse_type_full(d_nr, false) {
                // T1.11a: tuple-typed struct fields are not allowed because tuples are
                // stack-only values that cannot be stored in heap-allocated records.
                if matches!(tp, Type::Tuple(_)) {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "struct field cannot have a tuple type — tuples are stack-only values"
                        );
                    }
                    defined = true; // suppress the generic "needs type" fallback error
                } else {
                    defined = true;
                    a_type = tp;
                }
                break;
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
            if is_init {
                self.data.definitions[d_nr as usize].attributes[a].init = true;
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
            if is_init {
                self.data.definitions[d_nr as usize].attributes[a].init = true;
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

    // <field_default> ::= 'virtual' <value-expr> | 'init' '(' <value-expr> ')'
    //                   | 'default' '(' <value-expr> ')'
    // Returns (is_computed, is_init).
    pub(crate) fn parse_field_default(
        &mut self,
        value: &mut Value,
        a_type: &mut Type,
        _d_nr: u32,
        _a_name: &String,
        defined: &mut bool,
    ) -> (bool, bool) {
        let mut is_computed = false;
        let mut is_init = false;
        if self.lexer.has_keyword("computed") || self.lexer.has_keyword("virtual") {
            is_computed = true;
            // Computed field: calculate on every access, no store space.
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
        if self.lexer.has_keyword("init") {
            is_init = true;
            // L7: init(expr) — stored at creation, writable after. $ allowed.
            // #91: enable dep tracking for circular-init detection.
            self.init_field_tracking = true;
            self.init_field_deps.clear();
            self.lexer.token("(");
            let tp = self.expression(value);
            if a_type.is_unknown() {
                *a_type = tp;
                *defined = true;
            } else {
                self.convert(value, &tp, a_type);
            }
            self.lexer.token(")");
            self.init_field_tracking = false;
        }
        if self.lexer.has_keyword("default") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "default(expr) is removed; use 'computed(expr)' for calculated fields or '= expr' for stored defaults"
            );
            self.lexer.token("(");
            self.expression(value);
            self.lexer.token(")");
        }
        (is_computed, is_init)
    }
}
