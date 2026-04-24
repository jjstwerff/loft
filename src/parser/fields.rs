// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{DefType, I32, Level, Parser, Parts, Type, Value, diagnostic_format, v_block, v_set};

// Field access, indexing, and iterator operations.

impl Parser {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn field(&mut self, code: &mut Value, tp: Type) -> Type {
        if let Type::Unknown(_) = tp {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Field of unknown variable");
            }
            // In the first pass, skip the field name token so parsing continues.
            self.lexer.has_identifier();
            // wrap `code` in Value::Drop so an unresolved field access
            // (e.g. `x.v` where x's type is not yet known on pass 1) is no
            // longer treated as a plain `Value::Var(x)` by downstream
            // assignment processing.  Without this wrapping, `x.v = 99` in
            // a function whose `x = callee()` references a struct-returning
            // fn defined LATER in the file collapses to `x = 99` on pass 1
            // (because `.v` is silently dropped) — which sets x's inferred
            // type to integer.  Pass 2 then sees x = integer and rejects
            // the now-resolved `x = callee()` returning the struct.
            // Wrapping in Drop keeps `code != Value::Var(x)` so
            // `assign_var_nr` returns u16::MAX and `change_var` skips the
            // type update.
            *code = Value::Drop(Box::new(code.clone()));
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
                // Unwrap &vector<T> so map/filter/reduce work on ref params.
                let vec_t = if let Type::RefVar(inner) = &t {
                    inner.as_ref().clone()
                } else {
                    t.clone()
                };
                if matches!(vec_t, Type::Vector(_, _))
                    && matches!(field.as_str(), "map" | "filter" | "reduce")
                    && self.lexer.has_token("(")
                {
                    return self.parse_vector_method(code, &vec_t, &field);
                }
                // I7: bounded method call on generic T — look for a T-stub.
                // Verify the current function's bounds declare this method to
                // prevent T-stubs from unrelated generics leaking in.
                if let Some(_tv_name) = self.generic_type_name(&t) {
                    let stub_nr = self.data.find_fn(u16::MAX, &field, &t);
                    if stub_nr != u32::MAX
                        && self.has_bound_for_method(&field)
                        && self.lexer.has_token("(")
                    {
                        return self.parse_method(code, stub_nr, t.clone());
                    }
                }
                // generic-specific error for field access on T.
                if let Some(tv_name) = self.generic_type_name(&t) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "generic type {tv_name}: field access requires a concrete type",
                    );
                } else {
                    // INC#8 / QUALITY 6c: if a free function `n_<field>` exists
                    // whose first parameter is compatible with the receiver
                    // type, tell the user to call it as a free function
                    // instead of as a method.  The stdlib chooses per
                    // function whether it's `self:` / `both:` / free-only;
                    // readers who don't know that land on "Unknown field
                    // vector.sum_of" without a hint.
                    let free_nr = self.data.def_nr(&format!("n_{field}"));
                    let has_free_hint = free_nr != u32::MAX
                        && !self.data.def(free_nr).attributes.is_empty()
                        && self.data.attr_type(free_nr, 0).is_equal(&t);
                    if has_free_hint {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown field {}.{field} — did you mean the free function `{field}(…)` ? (stdlib declared `{field}` as free-only; see LOFT.md § Methods and function calls)",
                            self.data.def(dnr).name
                        );
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown field {}.{field}",
                            self.data.def(dnr).name
                        );
                    }
                }
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
            // B2-runtime (2026-04-13): `Sig.Idle` on a mixed struct-enum
            // parent resolves `expr` to a bare `Value::Enum(disc, _)` —
            // same type mismatch as the unqualified `Idle` form.  Wrap in
            // the `OpDatabase` + `object_init` record-allocation sequence
            // used by `parse_constant_value` so `var_s: DbRef = …` gets a
            // proper DbRef, not a u8 byte.
            let parent_is_mixed = matches!(self.data.def(dnr).returned, Type::Enum(_, true, _));
            if parent_is_mixed && !self.first_pass && matches!(expr, Value::Enum(_, _)) {
                let variant_name = self.data.attr_name(dnr, fnr);
                let variant_d_nr = self.data.def_nr(&variant_name);
                if variant_d_nr != u32::MAX && self.data.def(variant_d_nr).known_type != u16::MAX {
                    let ret = self.data.def(dnr).returned.clone();
                    let w = self.vars.work_refs(&ret, &mut self.lexer);
                    let known_type = i32::from(self.data.def(variant_d_nr).known_type);
                    let mut list = Vec::new();
                    list.push(crate::data::v_set(w, Value::Null));
                    list.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(known_type)]));
                    self.object_init(
                        &mut list,
                        variant_d_nr,
                        0,
                        &Value::Var(w),
                        &std::collections::HashSet::new(),
                    );
                    list.push(Value::Var(w));
                    // Mirror the unqualified-form path in parser/objects.rs:
                    // the LHS of assignment owns the store (empty dep); the
                    // work-ref is skip_free so it isn't double-freed.  With
                    // `vec![w]` the LHS got `dep=[__ref_N]` which made it a
                    // borrower — nothing freed the store.
                    self.vars.set_skip_free(w);
                    *code =
                        crate::data::v_block(list, Type::Enum(dnr, true, vec![]), "EnumUnitLit");
                    self.data.attr_used(dnr, fnr);
                    return Type::Enum(dnr, true, vec![]);
                }
            }
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
                    ("map", 1) => Some(Type::Function(vec![elem.clone()], Box::new(elem), vec![])),
                    ("filter", 1) => {
                        Some(Type::Function(vec![elem], Box::new(Type::Boolean), vec![]))
                    }
                    ("reduce", 1) => Some(Type::Function(
                        vec![elem.clone(), elem.clone()],
                        Box::new(elem),
                        vec![],
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
            let ret = self.data.def(*d_nr).returned.clone();
            // S16b: struct-enum variants have .returned = Type::Enum(parent, true, []).
            // For collection element access we need Type::Reference(variant_def_nr, [])
            // so that field access and range-query for-loops resolve fields against the
            // variant struct (not the parent enum), and for_type() can map the element type.
            if matches!(ret, Type::Enum(_, true, _)) {
                Type::Reference(*d_nr, Vec::new())
            } else {
                ret
            }
        } else if matches!(t, Type::Text(_)) {
            t.clone()
        } else if let Type::RefVar(tp) = t {
            *tp.clone()
        } else if t.is_unknown() {
            // First pass: type not yet resolved; suppress error until second pass.
            Type::Unknown(0)
        } else {
            // QUALITY 6d: the "Indexing a non vector" message fires for two
            // very different user intents — real misuse of `[..]` on a
            // scalar, and an attempted generic-constructor
            // (`hash<Row[id]>()`, `sorted<Elm[k]>()`) that the language
            // doesn't support.  The second case leaves readers stuck; add
            // a pointer to the struct-literal idiom that *does* work.
            diagnostic!(
                self.lexer,
                Level::Error,
                "Indexing a non vector — keyed collections (hash/sorted/index/spacial) have no generic-constructor expression; declare them as a struct field and initialise via a vector literal: `struct Db {{ h: hash<Row[id]> }}; db = Db {{ h: [Row {{ id: 1 }}] }}`"
            );
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
        // honour narrow vector-element stride when the
        // content Type::Integer carries a forced_size AND Phase 2 would
        // register a direct-encoded narrow type (see
        // `IntegerSpec::vector_narrow_width` — currently 1 and 4 bytes).
        // Shorts stay wide until Phase 4 aligns the `Parts::Short`
        // encoding with raw-byte copies.  Falls back to the
        // bounds-heuristic via `database.size(known_type)` otherwise.
        let elm_size = if let Type::Integer(spec) = etp
            && let Some(n) = spec.vector_narrow_width()
        {
            i32::from(n)
        } else {
            i32::from(self.database.size(known))
        };
        if let Value::Iter(var, init, next, extra_init) = p {
            if matches!(*next, Value::Block(_)) {
                // Linked structs: array stores 4-byte record pointers → use OpVectorRef
                // which internally uses elm_size=4 and dereferences to the actual record.
                // Base/primitive types: array stores inline values → use OpGetVector + get_val.
                let op = if self.database.is_linked(known) {
                    self.cl("OpVectorRef", &[code.clone(), *next.clone()])
                } else {
                    let mut v = self.cl(
                        "OpGetVector",
                        &[code.clone(), Value::Int(elm_size), *next.clone()],
                    );
                    if self.database.is_base(known) {
                        v = self.get_val(etp, true, 0, v, u32::MAX);
                    }
                    v
                };
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
            diagnostic!(self.lexer, Level::Error, "Invalid iterator expression");
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
        // Linked structs: array stores 4-byte record pointers → OpVectorRef dereferences correctly.
        // Base/primitive types: inline data → OpGetVector + get_val reads the primitive value.
        // Plain inline structs: OpGetVector only (field access happens at the next level).
        if self.database.is_linked(known) {
            *code = self.cl("OpVectorRef", &[code.clone(), p]);
        } else {
            *code = self.cl("OpGetVector", &[code.clone(), Value::Int(elm_size), p]);
            if self.database.is_base(known) {
                *code = self.get_val(etp, true, 0, code.clone(), u32::MAX);
            }
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

    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_key(&mut self, code: &mut Value, typedef: &Type, key_types: &[Type]) {
        // detect open-start `col[..hi]` or `col[..]` before parsing expression.
        let open_start = self.lexer.peek_token("..") || self.lexer.peek_token("..=");
        let mut p = Value::Null;
        let _index_t = if open_start {
            Type::Null // from=[] → no lower bound
        } else {
            let t = self.expression(&mut p);
            if !self.convert(&mut p, &t, &key_types[0]) {
                diagnostic!(self.lexer, Level::Error, "Invalid index key");
            }
            t
        };
        let known = if self.first_pass {
            Value::Null
        } else {
            self.type_info(typedef)
        };
        let mut nr = usize::from(!open_start);
        let mut key = Vec::new();
        if !open_start {
            key.push(p);
        }
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
        if self.lexer.has_token("..") || open_start {
            // Consume "..=" if present (open_start already peeked but didn't consume)
            let inclusive = if open_start {
                self.lexer.has_token(".."); // consume the ".."
                self.lexer.has_token("=")
            } else {
                self.lexer.has_token("=")
            };
            let iter = self.create_unique("iter", &crate::data::I64);
            let mut ls = Vec::new();
            if !self.first_pass {
                self.fill_iter(&mut ls, code, typedef, true, inclusive);
                ls.push(Value::Int(nr as i32));
                ls.append(&mut key);
            }
            // open-end — if next token is `]` or `,`, skip upper-bound expression.
            let open_end = self.lexer.peek_token("]") || self.lexer.peek_token(",");
            let mut nr = 0;
            if !open_end {
                let mut n = Value::Null;
                let n_t = self.expression(&mut n);
                if !self.convert(&mut n, &n_t, &key_types[0]) && !self.first_pass {
                    diagnostic!(self.lexer, Level::Error, "Invalid index key");
                }
                key.push(n);
                nr = 1;
            }
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
            // S16b: annotate the step-block with the element type, not the collection type,
            // so that IR dumps and any type-driven passes see the correct element type.
            let elem_type = match typedef {
                Type::Sorted(el, _, dep) | Type::Index(el, _, dep) => {
                    Type::Reference(*el, dep.clone())
                }
                _ => typedef.clone(),
            };
            *code = Value::Iter(
                u16::MAX,
                Box::new(start),
                Box::new(v_block(
                    vec![self.cl("OpStep", &ls)],
                    elem_type,
                    "Iterate keys",
                )),
                Box::new(Value::Null),
            );
        } else if matches!(typedef, Type::Index(_, _, _) | Type::Sorted(_, _, _))
            && key_types.len() > 1
            && nr < key_types.len()
        {
            // partial-key match — rewrite idx[k1] as idx[k1..=k1].
            // Uses the existing inclusive-range iteration path with from=till=key.
            let inclusive = true;
            let iter = self.create_unique("iter", &crate::data::I64);
            let mut ls = Vec::new();
            if !self.first_pass {
                // fill_iter calls set_loop which requires an active loop context.
                let loop_nr = self.vars.start_loop();
                self.fill_iter(&mut ls, code, typedef, true, inclusive);
                self.vars.finish_loop(loop_nr);
                ls.push(Value::Int(nr as i32));
                let from_key = key.clone();
                ls.append(&mut key);
                // till = same key values as from (inclusive prefix match)
                ls.push(Value::Int(nr as i32));
                ls.extend(from_key);
            }
            let start = v_set(iter, self.cl("OpIterate", &ls));
            let mut ls = vec![Value::Var(iter)];
            {
                let loop_nr = self.vars.start_loop();
                self.fill_iter(&mut ls, code, typedef, false, inclusive);
                self.vars.finish_loop(loop_nr);
            }
            let elem_type = match typedef {
                Type::Sorted(el, _, dep) | Type::Index(el, _, dep) => {
                    Type::Reference(*el, dep.clone())
                }
                _ => typedef.clone(),
            };
            *code = Value::Iter(
                u16::MAX,
                Box::new(start),
                Box::new(v_block(
                    vec![self.cl("OpStep", &ls)],
                    elem_type,
                    "Partial key match",
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
                // C60 piece 3 edit C: route hash iteration through
                // Ordered's on=3 code.  Parser has substituted the
                // iterated expression with a `hash_scratch` ref to a
                // u32-stride rec-nr vector in the hash's store (B+A).
                on = 3;
                arg = 4;
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
        // for index collections with a descending primary key, the tree
        // in-order is reversed from user-logical order.  XOR the reverse bit
        // so that step() uses previous() instead of next(), matching user order.
        // When the user also applies rev(), the XOR cancels out.
        let desc_primary = on & 63 == 1
            && !self.database.types[known as usize].keys.is_empty()
            && self.database.types[known as usize].keys[0].type_nr < 0;
        if self.reverse_iterator ^ desc_primary {
            on += 64;
        }
        if self.reverse_iterator {
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
