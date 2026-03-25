// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use super::{DefType, I32, Level, Parser, Parts, Type, Value, diagnostic_format, v_block, v_set};

// Field access, indexing, and iterator operations.

impl Parser {
    pub(crate) fn field(&mut self, code: &mut Value, tp: Type) -> Type {
        if let Type::Unknown(_) = tp {
            diagnostic!(self.lexer, Level::Error, "Field of unknown variable");
            return tp;
        }
        let mut t = tp;
        let Some(field) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect a field name");
            return t;
        };
        let enr = self.data.type_elm(&t);
        if enr == u32::MAX {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown type {}",
                t.show(&self.data, &self.vars)
            );
            return Type::Unknown(0);
        }
        let e_size = i32::from(self.database.size(self.data.def(enr).known_type));
        if let Type::RefVar(tp) = t {
            t = *tp;
        }
        let dnr = self.data.type_def_nr(&t);
        if matches!(t, Type::Vector(_, _)) && self.vector_operations(code, &field, e_size) {
            return Type::Void;
        }
        let fnr = self.data.attr(dnr, &field);
        if fnr == usize::MAX {
            if self.first_pass && self.lexer.has_token("(") {
                self.skip_remaining_args();
            } else if !self.first_pass {
                // For polymorphic enums, this field may be in a struct (not the enum itself).
                if let Type::Enum(enum_d_nr, true, _) = &t
                    && let Some((found_d_nr, found_fnr)) =
                        self.find_poly_enum_field(*enum_d_nr, &field)
                {
                    let dep = t.depend();
                    t = self.data.attr_type(found_d_nr, found_fnr);
                    for on in dep {
                        t = t.depending(on);
                    }
                    if let Value::Var(nr) = code {
                        t = t.depending(*nr);
                    }
                    *code = self.get_field(found_d_nr, found_fnr, code.clone());
                    self.data.attr_used(found_d_nr, found_fnr);
                    return t;
                }
                // map/filter/reduce as method syntax on vectors:
                // v.map(fn) → map(v, fn)
                if matches!(t, Type::Vector(_, _))
                    && matches!(field.as_str(), "map" | "filter" | "reduce")
                    && self.lexer.has_token("(")
                {
                    return self.parse_vector_method(code, &t, &field);
                }
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown field {}.{field}",
                    self.data.def(dnr).name
                );
                // Consume a trailing `(…)` to avoid cascading parse errors.
                if self.lexer.has_token("(") {
                    self.skip_remaining_args();
                }
            }
            return Type::Unknown(0);
        }
        if let Type::Routine(r_nr) = self.data.attr_type(dnr, fnr) {
            if self.lexer.has_token("(") {
                t = self.parse_method(code, r_nr, t.clone());
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect call of method {}.{}",
                    self.data.def(dnr).name,
                    self.data.attr_name(dnr, fnr)
                );
            }
        } else if self.data.def(dnr).attributes[fnr].constant {
            let expr = self.data.attr_value(dnr, fnr);
            *code = Self::replace_record_ref(expr, &code.clone());
            let dep = t.depend();
            t = self.data.attr_type(dnr, fnr);
            for on in dep {
                t = t.depending(on);
            }
        } else {
            let dep = t.depend();
            t = self.data.attr_type(dnr, fnr);
            for on in dep {
                t = t.depending(on);
            }
            if let Value::Var(nr) = code {
                t = t.depending(*nr);
            }
            *code = self.get_field(dnr, fnr, code.clone());
        }
        self.data.attr_used(dnr, fnr);
        t
    }

    /// Consume remaining function call arguments after `(` has already been consumed.
    /// Handle `v.map(fn)` / `v.filter(fn)` / `v.reduce(fn)` method syntax.
    fn parse_vector_method(&mut self, code: &mut Value, t: &Type, method: &str) -> Type {
        let mut list = vec![code.clone()];
        let mut types = vec![t.clone()];
        let mut m_arg_idx = 1usize;
        loop {
            if let Type::Vector(elm, _) = t {
                let elem = *elm.clone();
                let hint = match (method, m_arg_idx) {
                    ("map", 1) => Some(Type::Function(vec![elem.clone()], Box::new(elem))),
                    ("filter", 1) => Some(Type::Function(vec![elem], Box::new(Type::Boolean))),
                    ("reduce", 1) => Some(Type::Function(
                        vec![elem.clone(), elem.clone()],
                        Box::new(elem),
                    )),
                    _ => None,
                };
                if let Some(h) = hint {
                    self.lambda_hint = h;
                }
            }
            let mut p = Value::Null;
            let pt = self.expression(&mut p);
            self.lambda_hint = Type::Unknown(0);
            list.push(p);
            types.push(pt);
            m_arg_idx += 1;
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        match method {
            "map" => self.parse_map(code, &list, &types),
            "filter" => self.parse_filter(code, &list, &types),
            "reduce" => self.parse_reduce(code, &list, &types),
            _ => unreachable!(),
        }
    }

    pub(crate) fn skip_remaining_args(&mut self) {
        loop {
            if self.lexer.peek_token(")") {
                break;
            }
            let mut p = Value::Null;
            self.expression(&mut p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
    }

    /// Search for `field` in the variant structs of a polymorphic enum.
    /// Returns `(variant_d_nr, attr_nr)` if found.
    pub(crate) fn find_poly_enum_field(&self, enum_d_nr: u32, field: &str) -> Option<(u32, usize)> {
        for a_nr in 0..self.data.attributes(enum_d_nr) {
            let a_name = self.data.attr_name(enum_d_nr, a_nr);
            let variant_d_nr = self.data.def_nr(&a_name);
            if variant_d_nr == u32::MAX {
                continue;
            }
            if !matches!(self.data.def_type(variant_d_nr), DefType::EnumValue) {
                continue;
            }
            let f = self.data.attr(variant_d_nr, field);
            if f != usize::MAX {
                return Some((variant_d_nr, f));
            }
        }
        None
    }

    pub(crate) fn vector_operations(&mut self, code: &mut Value, field: &str, e_size: i32) -> bool {
        if field == "remove" {
            self.lexer.token("(");
            let (tps, ls) = self.parse_parameters();
            let mut cd = ls[0].clone();
            // validate types
            if tps.len() != 1 || !self.convert(&mut cd, &tps[0], &I32) {
                diagnostic!(self.lexer, Level::Error, "Invalid index in remove");
            }
            *code = self.cl("OpRemoveVector", &[code.clone(), Value::Int(e_size), cd]);
            true
        } else {
            false
        }
    }

    pub(crate) fn parse_index(&mut self, code: &mut Value, tp: &Type) -> Type {
        let mut t = tp.clone();
        let mut p = Value::Null;
        self.un_ref(&mut t, &mut p);
        let mut elm_type = self.index_type(&t);
        for on in t.depend() {
            elm_type = elm_type.depending(on);
        }
        /*let nr = if self.types.exists("$") {
            self.types.var_nr("$")
        } else {
            self.create_var("$".to_string(), elm_type.clone())
        };
        self.data.definitions[self.context as usize].variables[nr as usize].uses = 0;
         */
        if let Type::Vector(etp, _) = &t {
            if let Some(value) = self.parse_vector_index(code, &elm_type, etp) {
                return value;
            }
        } else if matches!(t, Type::Text(_)) {
            let index_t = if self.lexer.peek_token("..") {
                p = Value::Int(0);
                I32.clone()
            } else {
                self.expression(&mut p)
            };
            if self.parse_text_index(code, &mut p, &index_t) == Type::Character {
                elm_type = Type::Character;
            }
        } else if let Type::Hash(el, keys, _) | Type::Spacial(el, keys, _) = &t {
            let mut key_types = Vec::new();
            for k in keys {
                key_types.push(self.data.attr_type(*el, self.data.attr(*el, k)).clone());
            }
            self.parse_key(code, &t, &key_types);
        } else if let Type::Sorted(el, keys, _) | Type::Index(el, keys, _) = &t {
            let mut key_types = Vec::new();
            for (k, _) in keys {
                key_types.push(self.data.attr_type(*el, self.data.attr(*el, k)).clone());
            }
            self.parse_key(code, &t, &key_types);
        } else {
            // index_type() already emitted a diagnostic; consume the inner expression
            // so that the caller can still parse the closing `]` without cascading errors.
            let mut p = Value::Null;
            self.expression(&mut p);
        }
        elm_type
    }

    pub(crate) fn index_type(&mut self, t: &Type) -> Type {
        if let Type::Vector(v_t, _) = t {
            *v_t.clone()
        } else if let Type::Sorted(d_nr, _, _)
        | Type::Hash(d_nr, _, _)
        | Type::Index(d_nr, _, _)
        | Type::Spacial(d_nr, _, _) = t
        {
            self.data.def(*d_nr).returned.clone()
        } else if matches!(t, Type::Text(_)) {
            t.clone()
        } else if let Type::RefVar(tp) = t {
            *tp.clone()
        } else if t.is_unknown() {
            // First pass: type not yet resolved; suppress error until second pass.
            Type::Unknown(0)
        } else {
            diagnostic!(self.lexer, Level::Error, "Indexing a non vector");
            Type::Unknown(0)
        }
    }

    pub(crate) fn parse_vector_index(
        &mut self,
        code: &mut Value,
        elm_type: &Type,
        etp: &Type,
    ) -> Option<Type> {
        let mut p = Value::Null;
        let index_t = self.parse_in_range(&mut p, code, "$");
        let elm_td = self.data.type_elm(etp);
        let known = self.data.def(elm_td).known_type;
        let elm_size = i32::from(self.database.size(known));
        if let Value::Iter(var, init, next, extra_init) = p {
            if matches!(*next, Value::Block(_)) {
                let mut op = self.cl(
                    "OpGetVector",
                    &[code.clone(), Value::Int(elm_size), *next.clone()],
                );
                if self.database.is_base(known) || self.database.is_linked(known) {
                    op = self.get_val(etp, true, 0, op);
                }
                *code = Value::Iter(
                    var,
                    init,
                    Box::new(v_block(vec![op], etp.clone(), "Vector Index")),
                    extra_init,
                );
                return Some(Type::Iterator(
                    Box::new(elm_type.clone()),
                    Box::new(Type::Null),
                ));
            }
            diagnostic!(self.lexer, Level::Error, "Malformed iterator in IR");
            return None;
        }
        if !self.first_pass && !self.convert(&mut p, &index_t, &I32) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Invalid index type {} on vector",
                index_t.show(&self.data, &self.vars)
            );
        }
        *code = self.cl("OpGetVector", &[code.clone(), Value::Int(elm_size), p]);
        if self.database.is_base(known) || self.database.is_linked(known) {
            *code = self.get_val(etp, true, 0, code.clone());
        }
        None
    }

    pub(crate) fn parse_text_index(
        &mut self,
        code: &mut Value,
        p: &mut Value,
        index_t: &Type,
    ) -> Type {
        if !self.convert(p, index_t, &I32) {
            diagnostic!(self.lexer, Level::Error, "Invalid index on string");
        }
        let mut other = Value::Null;
        if self.lexer.has_token("..") {
            let incl = self.lexer.has_token("=");
            if self.lexer.peek_token("]") {
                *code = self.cl(
                    "OpGetTextSub",
                    &[code.clone(), p.clone(), Value::Int(i32::MAX)],
                );
            } else {
                let ot_type = self.expression(&mut other);
                if !self.convert(&mut other, &ot_type, &I32) {
                    diagnostic!(self.lexer, Level::Error, "Invalid index on string",);
                }
                if incl {
                    other = self.cl("OpAddInt", &[other.clone(), Value::Int(1)]);
                }
                *code = self.cl("OpGetTextSub", &[code.clone(), p.clone(), other]);
            }
            Type::Text(Vec::new())
        } else {
            *code = self.cl("OpTextCharacter", &[code.clone(), p.clone()]);
            Type::Character
        }
    }

    pub(crate) fn parse_key(&mut self, code: &mut Value, typedef: &Type, key_types: &[Type]) {
        let mut p = Value::Null;
        let index_t = self.expression(&mut p);
        if !self.convert(&mut p, &index_t, &key_types[0]) {
            diagnostic!(self.lexer, Level::Error, "Invalid index key");
        }
        let known = if self.first_pass {
            Value::Null
        } else {
            self.type_info(typedef)
        };
        let mut nr = 1;
        let mut key = Vec::new();
        key.push(p);
        if key_types.len() > 1 {
            while self.lexer.has_token(",") {
                if nr >= key_types.len() {
                    diagnostic!(self.lexer, Level::Error, "Too many key values on index");
                    break;
                }
                let mut ex = Value::Null;
                let ex_t = self.expression(&mut ex);
                if !self.convert(&mut ex, &ex_t, &key_types[nr]) {
                    diagnostic!(self.lexer, Level::Error, "Invalid index key");
                }
                key.push(ex);
                nr += 1;
            }
        }
        if self.lexer.has_token("..") {
            let inclusive = self.lexer.has_token("=");
            let iter = self.create_unique("iter", &Type::Long);
            let mut ls = Vec::new();
            if !self.first_pass {
                self.fill_iter(&mut ls, code, typedef, true, inclusive);
                ls.push(Value::Int(nr as i32));
                ls.append(&mut key);
            }
            let mut n = Value::Null;
            self.expression(&mut n);
            if !self.convert(&mut n, &index_t, &key_types[0]) {
                diagnostic!(self.lexer, Level::Error, "Invalid index key");
            }
            key.push(n);
            let mut nr = 1;
            if key_types.len() > 1 {
                while self.lexer.has_token(",") {
                    if nr >= key_types.len() {
                        diagnostic!(self.lexer, Level::Error, "Too many key values on index");
                        break;
                    }
                    let mut ex = Value::Null;
                    let ex_t = self.expression(&mut ex);
                    if !self.convert(&mut ex, &ex_t, &key_types[nr]) {
                        diagnostic!(self.lexer, Level::Error, "Invalid index key");
                    }
                    key.push(ex);
                    nr += 1;
                }
            }
            ls.push(Value::Int(nr as i32));
            ls.append(&mut key);
            let start = v_set(iter, self.cl("OpIterate", &ls));
            let mut ls = vec![Value::Var(iter)];
            self.fill_iter(&mut ls, code, typedef, false, inclusive);
            *code = Value::Iter(
                u16::MAX,
                Box::new(start),
                Box::new(v_block(
                    vec![self.cl("OpStep", &ls)],
                    typedef.clone(),
                    "Iterate keys",
                )),
                Box::new(Value::Null),
            );
        } else {
            let mut ls = vec![code.clone(), known.clone(), Value::Int(nr as i32)];
            ls.append(&mut key);
            *code = self.cl("OpGetRecord", &ls);
            if matches!(typedef, Type::Hash(_, _, _)) && nr < key_types.len() {
                diagnostic!(self.lexer, Level::Error, "Too few key fields");
            }
        }
    }

    pub(crate) fn fill_iter(
        &mut self,
        ls: &mut Vec<Value>,
        code: &mut Value,
        typedef: &Type,
        add_keys: bool,
        inclusive: bool,
    ) {
        let known = self.get_type(typedef);
        if known == u16::MAX {
            return;
        }
        let mut on;
        let arg;
        match self.database.types[known as usize].parts {
            Parts::Index(_, _, _) => {
                on = 1;
                arg = self.database.fields(known);
            }
            Parts::Sorted(tp, _) => {
                on = 2;
                arg = self.database.size(tp);
            }
            Parts::Ordered(_, _) => {
                on = 3;
                arg = 4;
            }
            Parts::Hash(_, _) => {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot iterate a hash directly — a hash has no stable element order, \
so #index and #remove are not supported; \
pair the hash with a vector to iterate in insertion order"
                );
                return;
            }
            _ => {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot iterate; expected vector, sorted, index, text, or range"
                );
                return;
            }
        }
        if inclusive {
            on += 128;
        }
        if self.reverse_iterator {
            on += 64;
            // Do not reset here — `iterator()` calls fill_iter twice and resets after both.
        }
        ls.push(code.clone());
        ls.push(Value::Int(i32::from(on)));
        ls.push(Value::Int(i32::from(arg)));
        // For Index (on & 63 == 1): store the type index so OpRemove can call
        // database.fields(tp) and database.remove(..., tp) with the correct type.
        // For all other collection types, arg IS the db_tp used by OpRemove.
        let loop_db_tp = if on & 63 == 1 { known } else { arg };
        self.vars.set_loop(on, loop_db_tp, code);
        if add_keys {
            ls.push(Value::Keys(
                self.database.types[known as usize].keys.clone(),
            ));
        }
    }
}
