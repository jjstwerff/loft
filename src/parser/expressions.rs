// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use super::{Level, Parser, Parts, Type, Value, diagnostic_format, v_block, v_if, v_loop, v_set};

/// Returns true if `val` contains a `Set(r, _)` node at any depth.
/// Used to find which block statement first assigns an inline-ref temporary.
/// Returns true if `val` contains a `Set(r, _)` node at any depth.
/// Used to find which block statement first assigns an inline-ref temporary.
///
/// The match is exhaustive over all current `Value` variants so that adding a new
/// compound variant without updating this function is a **compile error** rather than
/// a silent miss that would insert the null-init at the wrong position (A15).
fn inline_ref_set_in(val: &Value, r: u16, depth: usize) -> bool {
    if depth > 1000 {
        return false;
    }
    match val {
        // Compound variants — recurse into sub-expressions.
        Value::Set(v, inner) => *v == r || inline_ref_set_in(inner, r, depth + 1),
        Value::Call(_, args) | Value::CallRef(_, args) => {
            args.iter().any(|a| inline_ref_set_in(a, r, depth + 1))
        }
        Value::Block(bl) | Value::Loop(bl) => bl
            .operators
            .iter()
            .any(|a| inline_ref_set_in(a, r, depth + 1)),
        Value::Insert(ops) => ops.iter().any(|a| inline_ref_set_in(a, r, depth + 1)),
        Value::If(cond, then_val, else_val) => {
            inline_ref_set_in(cond, r, depth + 1)
                || inline_ref_set_in(then_val, r, depth + 1)
                || inline_ref_set_in(else_val, r, depth + 1)
        }
        Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
            inline_ref_set_in(inner, r, depth + 1)
        }
        Value::Iter(_, a, b, c) => {
            inline_ref_set_in(a, r, depth + 1)
                || inline_ref_set_in(b, r, depth + 1)
                || inline_ref_set_in(c, r, depth + 1)
        }
        Value::Tuple(elems) => elems.iter().any(|a| inline_ref_set_in(a, r, depth + 1)),
        Value::TuplePut(_, _, inner) => inline_ref_set_in(inner, r, depth + 1),
        // Leaf variants — cannot contain a Set node.
        Value::Null
        | Value::Int(_)
        | Value::Enum(_, _)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Var(_)
        | Value::Line(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _) => false,
    }
}

impl Parser {
    // <code> = '{' <block> '}'
    /// Parse the code on the last inserted definition.
    /// This way we can use recursion with the definition itself.
    pub(crate) fn parse_code(&mut self) -> Type {
        let mut v = Value::Null;
        let result = if self.context == u32::MAX {
            Type::Void
        } else {
            self.data.def(self.context).returned.clone()
        };
        self.parse_block("return from block", &mut v, &result);
        if let Value::Block(bl) = &mut v {
            let ls = &mut bl.operators;
            for wt in self.vars.work_texts() {
                ls.insert(0, v_set(wt, Value::Text(String::new())));
            }
            for r in self.vars.work_references() {
                if !self.vars.is_argument(r)
                    && self.vars.tp(r).depend().is_empty()
                    && !self.vars.is_inline_ref(r)
                {
                    ls.insert(0, v_set(r, Value::Null));
                }
            }
            // Inline-ref temporaries (parse_part work-refs for chained ref calls):
            // Insert null-init for each temp immediately BEFORE the statement that
            // first assigns it (the statement containing {Set(r, call_result)}).
            // This ensures scan_set encounters them AFTER the body variables whose
            // stores precede theirs (e.g. `p`), so reversed var_order frees the
            // inline-ref temps BEFORE those body variables — satisfying LIFO.
            //
            // For temps used in the same statement we insert in descending var_nr order
            // so that lower var_nrs end up first in ls (allocated first = freed last).
            {
                let inline_refs = self.vars.inline_ref_references();
                // Build (first_use_position, var_nr) pairs.
                let mut insertions: Vec<(usize, u16)> = Vec::new();
                // Fallback position: after the first non-Line-marker stmt in ls.
                let mut fallback = 0usize;
                while fallback < ls.len() && matches!(ls[fallback], Value::Line(_)) {
                    fallback += 1;
                }
                if fallback < ls.len() {
                    fallback += 1;
                }
                for r in &inline_refs {
                    if !self.vars.is_argument(*r) && self.vars.tp(*r).depend().is_empty() {
                        let pos = ls
                            .iter()
                            .position(|stmt| inline_ref_set_in(stmt, *r, 0))
                            .unwrap_or(fallback);
                        insertions.push((pos, *r));
                    }
                }
                // Insert from end to start to avoid index invalidation; within the
                // same position insert higher var_nr first so lower var_nr lands first.
                insertions.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
                for (pos, r) in insertions {
                    ls.insert(pos, v_set(r, Value::Null));
                }
            }
            // Auto-lock the stores for every const Reference/Vector argument at the very
            // start of the function body (after work-variable initialisations).
            // Applies in all build profiles so that writes to const parameters panic in
            // release builds too (S22 — previously guarded by #[cfg(debug_assertions)]).
            if !self.first_pass {
                let n_vars = self.vars.next_var();
                let lock_fn = self.data.def_nr("n_set_store_lock");
                if lock_fn != u32::MAX {
                    let mut inserts = Vec::new();
                    for v_nr in 0..n_vars {
                        if self.vars.is_argument(v_nr)
                            && self.vars.is_const_param(v_nr)
                            && matches!(
                                self.vars.tp(v_nr),
                                Type::Reference(_, _) | Type::Vector(_, _)
                            )
                        {
                            inserts.push(Value::Call(
                                lock_fn,
                                vec![Value::Var(v_nr), Value::Boolean(true)],
                            ));
                        }
                    }
                    // Insert in reverse order so index-0 inserts keep the right sequence.
                    inserts.reverse();
                    for ins in inserts {
                        ls.insert(0, ins);
                    }
                }
            }
        }
        if self.context != u32::MAX && !self.first_pass {
            self.data.definitions[self.context as usize].code = v;
        }
        result
    }

    // <expression> ::= <for> | 'continue' | 'break' | 'return' | 'yield' | '{' <block> | <operators>
    #[allow(clippy::too_many_lines)]
    pub(crate) fn expression(&mut self, val: &mut Value) -> Type {
        if self.lexer.has_token("for") {
            self.parse_for(val);
            Type::Void
        } else if self.lexer.has_token("while") {
            self.parse_while(val);
            Type::Void
        } else if self.lexer.has_token("continue") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot continue outside a loop");
            }
            *val = Value::Continue(0);
            Type::Void
        } else if self.lexer.has_token("break") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot break outside a loop");
            }
            *val = Value::Break(0);
            Type::Void
        } else if self.lexer.has_token("return") {
            self.parse_return(val);
            Type::Void
        } else if self.lexer.has_token("yield") {
            // CO1.3c: yield expr — only valid inside generator functions.
            let r_type = self.data.def(self.context).returned.clone();
            if !matches!(r_type, Type::Iterator(_, _)) && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "yield is only allowed inside generator functions (return type must be iterator<T>)"
                );
            }
            if self.lexer.has_keyword("from") {
                // CO1.4: yield from sub_gen — desugar to:
                //   __sub = sub; loop { __item = next(__sub); if !__item break; yield __item; }
                let mut sub = Value::Null;
                let sub_type = self.expression(&mut sub);
                if let Type::Iterator(inner, _) = &sub_type {
                    let elem_tp = (**inner).clone();
                    let sub_var = self.create_unique("__yf_sub", &sub_type);
                    self.vars.defined(sub_var);
                    let item_var = self.create_unique("__yf_item", &elem_tp);
                    self.vars.defined(item_var);
                    let op = self.data.def_nr("OpCoroutineNext");
                    let value_size =
                        crate::variables::size(&elem_tp, &crate::data::Context::Argument);
                    let next_call = Value::Call(
                        op,
                        vec![Value::Var(sub_var), Value::Int(i32::from(value_size))],
                    );
                    let mut test = Value::Var(item_var);
                    self.convert(&mut test, &elem_tp, &Type::Boolean);
                    test = self.cl("OpNot", &[test]);
                    let lp = vec![
                        crate::data::v_set(item_var, next_call),
                        crate::data::v_if(
                            test,
                            crate::data::v_block(vec![Value::Break(0)], Type::Void, "break"),
                            Value::Null,
                        ),
                        Value::Yield(Box::new(Value::Var(item_var))),
                    ];
                    let steps = vec![
                        crate::data::v_set(sub_var, sub),
                        crate::data::v_loop(lp, "yield from"),
                    ];
                    *val = crate::data::v_block(steps, Type::Void, "yield from block");
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "yield from requires an iterator expression"
                    );
                }
                Type::Void
            } else {
                let mut v = Value::Null;
                self.expression(&mut v);
                *val = Value::Yield(Box::new(v));
                Type::Void
            }
        } else if self.lexer.peek_token("{") {
            self.parse_block("block", val, &Type::Void)
        } else {
            // `const x = expr` — mark the resulting local variable as const after initialisation.
            let const_decl = self.lexer.has_keyword("const");
            let res = self.parse_assign(val);
            if const_decl && !self.first_pass {
                let v_nr = match val {
                    Value::Set(nr, _) => Some(*nr),
                    Value::Insert(ls) => ls.iter().find_map(|v| {
                        if let Value::Set(nr, _) = v {
                            Some(*nr)
                        } else {
                            None
                        }
                    }),
                    _ => None,
                };
                if let Some(v_nr) = v_nr {
                    self.vars.set_const_param(v_nr);
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "const keyword requires a variable assignment"
                    );
                }
            }
            self.known_var_or_type(val);
            res
        }
    }

    /// L10: `while <cond> { <body> }` desugars to an infinite loop with a break guard.
    ///
    /// The emitted IR is equivalent to:
    ///   loop { if !cond { break }; body }
    pub(crate) fn parse_while(&mut self, code: &mut Value) {
        let mut cond = Value::Null;
        self.expression(&mut cond);
        if !self.first_pass && matches!(cond, Value::Null) {
            diagnostic!(self.lexer, Level::Error, "Expected condition after 'while'");
            return;
        }
        let not_cond = self.cl("OpNot", &[cond]);
        let break_if = v_if(
            not_cond,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );
        let loop_nr = self.vars.start_loop();
        let in_loop = self.in_loop;
        self.in_loop = true;
        let mut body = Value::Null;
        let loop_write_state = self.vars.save_and_clear_write_state();
        self.parse_block("while", &mut body, &Type::Void);
        self.vars.restore_write_state(&loop_write_state);
        self.in_loop = in_loop;
        self.vars.finish_loop(loop_nr);
        *code = v_loop(vec![break_if, body], "while");
    }

    pub(crate) fn change_var(&mut self, code: &Value, tp: &Type) -> bool {
        if let Value::Var(v_nr) = code {
            let mut is_text = matches!(self.vars.tp(*v_nr), Type::Text(_));
            if let Type::RefVar(i) = self.vars.tp(*v_nr)
                && matches!(**i, Type::Text(_))
            {
                is_text = true;
            }
            if !is_text || *tp != Type::Character {
                self.change_var_type(*v_nr, tp);
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn change_var_type(&mut self, v_nr: u16, tp: &Type) {
        let chg = self
            .vars
            .change_var_type(v_nr, tp, &self.data, &mut self.lexer);
        if chg
            && !tp.is_unknown()
            && let Type::Vector(elm, _) = tp
        {
            self.data.vector_def(&mut self.lexer, elm);
        }
    }

    /// Check for iteration-safety violation on `+=` to collections; emit diagnostics.
    pub(crate) fn check_iter_safety(&mut self, to: &Value, f_type: &Type, op: &str) {
        if self.first_pass
            || op != "+="
            || !matches!(
                f_type,
                Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Spacial(_, _, _)
            )
        {
            return;
        }
        if let Value::Var(lhs_nr) = to
            && self.vars.is_iterated_var(*lhs_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot add elements to '{}' while it is being iterated — \
use a separate collection or add after the loop",
                self.vars.name(*lhs_nr)
            );
        } else if !matches!(to, Value::Var(_)) && self.vars.is_iterated_value(to) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot add elements to a collection while it is being iterated — \
use a separate collection or add after the loop"
            );
        }
    }

    /// Validate `d#lock = expr` assignment; returns true if handled (caller should return Void).
    pub(crate) fn validate_lock_assign(&mut self, code: &Value, to: &Value) -> bool {
        if self.first_pass {
            return false;
        }
        let Value::Call(lock_nr, lock_args) = to else {
            return false;
        };
        if self.data.def(*lock_nr).name != "n_get_store_lock" {
            return false;
        }
        if !matches!(code, Value::Boolean(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "d#lock can only be assigned a constant boolean (true or false)"
            );
            return true;
        }
        if matches!(code, Value::Boolean(false))
            && let Some(Value::Var(v_nr)) = lock_args.first()
            && self.vars.is_const_param(*v_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot unlock const variable '{}' via d#lock = false",
                self.vars.name(*v_nr)
            );
            return true;
        }
        false
    }

    /// Apply the operator `op` to an already-parsed LHS and parse the RHS,
    /// then rewrite `code` into the assignment IR. Returns `Type::Void`.
    // threads LHS context (to, f_type, parent_tp, var_nr) alongside op and &mut self
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn parse_assign_op(
        &mut self,
        code: &mut Value,
        op: &str,
        f_type: &Type,
        to: &Value,
        mut parent_tp: Type,
        var_nr: u16,
    ) -> Type {
        self.check_iter_safety(to, f_type, op);
        // Save parent struct type before the RHS parse overwrites parent_tp.
        let lhs_parent_tp = parent_tp.clone();
        let mut s_type = self.parse_operators(f_type, code, &mut parent_tp, 0);
        if let Type::Rewritten(tp) = s_type {
            s_type = *tp;
        }
        // Dead assignment check: after the RHS is parsed (so RHS reads of the
        // variable are already counted), check if the previous write was never read.
        if op == "=" && var_nr != u16::MAX && !self.first_pass && self.vars.exists(var_nr) {
            self.vars.track_write(var_nr, &mut self.lexer);
        }
        // Convert untyped null to typed null for scalar assignments (not collections).
        if s_type == Type::Null
            && op == "="
            && !matches!(
                f_type,
                Type::Reference(_, _)
                    | Type::Enum(_, true, _)
                    | Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
            )
        {
            self.convert(code, &Type::Null, f_type);
        }
        if var_nr == u16::MAX {
            self.validate_write(to, &parent_tp);
        }
        // materialise a collection iterator (e.g. v[a..b] slice) into a vector variable.
        // CO1.3c: skip materialisation for coroutine iterators (second type is Null).
        let is_coroutine_iter = matches!(&s_type, Type::Iterator(_, it) if **it == Type::Null);
        if matches!(&s_type, Type::Iterator(_, _))
            && !is_coroutine_iter
            && matches!(f_type, Type::Unknown(_) | Type::Vector(_, _))
            && var_nr != u16::MAX
            && matches!(op, "=" | "+=")
        {
            self.materialize_iterator(code, &s_type, to, &lhs_parent_tp, var_nr, op);
            return Type::Void;
        }
        self.change_var(to, &s_type);
        if matches!(f_type, Type::Text(_)) {
            self.assign_text(code, &s_type, to, op, var_nr);
            return Type::Void;
        }
        if self.assign_refvar_text(code, f_type, &s_type, op, var_nr) {
            return Type::Void;
        }
        if self.assign_refvar_vector(code, f_type, op, var_nr) {
            return Type::Void;
        }
        if var_nr != u16::MAX && self.create_vector(code, f_type, op, var_nr) {
            return Type::Void;
        }
        // `lhs += other_vec` where both sides are vectors: append all elements
        // in-place via OpAppendVector.
        if !self.first_pass
            && op == "+="
            && let Type::Vector(elm_tp, _) = &f_type.clone()
            && matches!(s_type, Type::Vector(_, _))
            && !matches!(code, Value::Insert(_))
        {
            let rec_tp = i32::from(self.data.def(self.data.type_def_nr(elm_tp)).known_type);
            *code = Value::Insert(vec![self.cl(
                "OpAppendVector",
                &[to.clone(), code.clone(), Value::Int(rec_tp)],
            )]);
            return Type::Void;
        }
        // Scalar `field += elem` where field is a vector field (var_nr == u16::MAX).
        if !self.first_pass
            && var_nr == u16::MAX
            && op == "+="
            && self.is_field(to)
            && let Type::Vector(elm_tp, _) = f_type
            && !matches!(code, Value::Insert(_))
        {
            let elm_tp = (**elm_tp).clone();
            let elm = self.unique_elm_var(&lhs_parent_tp, &elm_tp, u16::MAX);
            let scalar = code.clone();
            let ls = self.new_record(
                &mut to.clone(),
                &lhs_parent_tp,
                elm,
                u16::MAX,
                &[scalar],
                &elm_tp,
            );
            *code = Value::Insert(ls);
            return Type::Void;
        }
        // Auto-convert integer to long for a long-typed LHS assignment.
        if matches!(f_type, Type::Long)
            && matches!(s_type, Type::Integer(_, _, _))
            && op == "="
            && !self.first_pass
        {
            *code = self.cl("OpConvLongFromInt", std::slice::from_ref(code));
        }
        if self.validate_lock_assign(code, to) {
            return Type::Void;
        }
        // For const variables the Insert path (e.g. struct constructor) bypasses
        // towards_set, so check const here before that path can be taken.
        if matches!(code, Value::Insert(_))
            && !self.first_pass
            && var_nr != u16::MAX
            && self.vars.is_const_param(var_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot modify {} '{}'; remove 'const' or use a local copy",
                self.vars.const_kind(var_nr),
                self.vars.name(var_nr)
            );
        }
        if !matches!(code, Value::Insert(_)) {
            *code = self.towards_set(to, code, f_type, &op[0..1]);
        }
        // emit field constraint check after assignment to a constrained field.
        if !self.first_pass
            && let Type::Reference(struct_dnr, _) = &parent_tp
            && let Value::Call(_, to_args) = to
            && to_args.len() >= 2
            && let Value::Int(field_offset) = &to_args[1]
        {
            let sd = *struct_dnr;
            let off = *field_offset;
            // Find the field by matching its database offset.
            for a_nr in 0..self.data.def(sd).attributes.len() {
                let nm = self.data.attr_name(sd, a_nr);
                let fpos = self.database.position(self.data.def(sd).known_type, &nm);
                if i32::from(fpos) == off && self.data.def(sd).attributes[a_nr].check != Value::Null
                {
                    let check = self.data.def(sd).attributes[a_nr].check.clone();
                    let ref_val = to_args[0].clone();
                    let bound = Self::replace_record_ref(check, &ref_val);
                    let msg = match &self.data.def(sd).attributes[a_nr].check_message {
                        Value::Text(s) => Value::Text(s.clone()),
                        _ => Value::Text(format!(
                            "field constraint failed on {}.{nm}",
                            self.data.def(sd).name
                        )),
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
                    *code = Value::Insert(vec![code.clone(), assert_call]);
                    break;
                }
            }
        }
        Type::Void
    }

    // <assign> ::= <operators> [ '=' | '+=' | '-=' | '*=' | '%=' | '/=' <operators> ]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_assign(&mut self, code: &mut Value) -> Type {
        let mut parent_tp = Type::Null;
        let mut f_type = self.parse_operators(&Type::Unknown(0), code, &mut parent_tp, 0);
        if let (Type::RefVar(_), Value::Var(v_nr)) = (&f_type, &code) {
            self.vars.in_use(*v_nr, true);
        }
        // Type annotation: `v: type = expr`
        // Only attempt outside format-string expressions (where `:` is used for
        // format specifiers like `{c:#}`).  Consume `: type` only when `=`
        // follows, confirming this is an annotated declaration.
        if let Value::Var(v_nr) = code
            && self.vars.exists(*v_nr)
            && !self.in_format_expr
            && self.lexer.peek_token(":")
        {
            let lnk = self.lexer.link();
            self.lexer.cont(); // consume ":"
            let mut got_annotation = false;
            if let Some(tp) = self.parse_type_full(u32::MAX, false)
                && self.lexer.peek_token("=")
            {
                self.change_var_type(*v_nr, &tp);
                f_type = tp;
                got_annotation = true;
            }
            if !got_annotation {
                self.lexer.revert(lnk);
            }
        }
        // T1.2: LHS tuple destructuring — (a, b) = expr
        if let Value::Tuple(vars) = code
            && self.lexer.has_token("=")
        {
            let var_nrs: Vec<u16> = vars
                .iter()
                .filter_map(|v| {
                    if let Value::Var(nr) = v {
                        Some(*nr)
                    } else {
                        None
                    }
                })
                .collect();
            if var_nrs.len() != vars.len() {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Tuple destructuring requires plain variable names"
                );
            }
            let mut rhs = Value::Null;
            let rhs_type = self.expression(&mut rhs);
            if let Type::Tuple(ref rhs_elems) = rhs_type {
                if rhs_elems.len() != var_nrs.len() {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Tuple arity mismatch: left has {} names, right has {} elements",
                        var_nrs.len(),
                        rhs_elems.len()
                    );
                }
                // T1.4: create a temp variable for the RHS tuple, then read elements.
                let tmp_tp = rhs_type.clone();
                let tmp = self.vars.work_refs(&tmp_tp, &mut self.lexer);
                if !self.first_pass {
                    self.change_var_type(tmp, &tmp_tp);
                }
                let mut steps = vec![Value::Set(tmp, Box::new(rhs))];
                for (i, &v_nr) in var_nrs.iter().enumerate() {
                    if self.vars.exists(v_nr) {
                        self.vars.defined(v_nr);
                        if i < rhs_elems.len() {
                            self.change_var_type(v_nr, &rhs_elems[i]);
                        }
                    }
                    steps.push(Value::Set(v_nr, Box::new(Value::TupleGet(tmp, i as u16))));
                }
                *code = Value::Insert(steps);
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot destructure a non-tuple value"
                );
            }
            return Type::Void;
        }
        // T1.4-fix-a: tuple element assignment t.0 = expr.
        if let Value::TupleGet(var_nr, idx) = code {
            let var_nr = *var_nr;
            let idx = *idx;
            if self.lexer.has_token("=") {
                let mut rhs = Value::Null;
                self.expression(&mut rhs);
                *code = Value::TuplePut(var_nr, idx, Box::new(rhs));
                return Type::Void;
            }
        }
        let to = code.clone();
        for op in ["=", "+=", "-=", "*=", "%=", "/="] {
            if self.lexer.has_token(op) {
                // Mark the variable as defined only once we have confirmed the `=` token
                // is actually present. Doing this before the token check caused any bare
                // `Value::Var` (e.g. `{cd}` inside a format string) to be marked defined
                // prematurely, hiding the "use before assignment" diagnostic.
                if op == "="
                    && let Value::Var(v_nr) = code
                    && !self.first_pass
                    && self.vars.exists(*v_nr)
                {
                    self.vars.defined(*v_nr);
                }
                let var_nr = self.assign_var_nr(code, op, &f_type, &mut parent_tp);
                // Handle `f += X` for File variables before type-changing logic.
                if op == "+="
                    && self.is_file_var_type(&f_type)
                    && let Value::Var(file_v) = to
                {
                    self.append_to_file(code, file_v);
                    return Type::Void;
                }
                // A5.3: record closure association if the RHS was a capturing lambda.
                if op == "=" && self.last_closure_work_var != u16::MAX && var_nr != u16::MAX {
                    self.closure_vars.insert(var_nr, self.last_closure_work_var);
                    self.last_closure_work_var = u16::MAX;
                }
                return self.parse_assign_op(code, op, &f_type, &to, parent_tp, var_nr);
            }
        }
        *code = to;
        f_type
    }

    pub(crate) fn append_to_text(
        &mut self,
        code: &mut Value,
        op: &str,
        var_nr: u16,
        s_type: &Type,
    ) {
        if !self.first_pass && self.vars.is_const_param(var_nr) && !matches!(code, Value::Insert(_))
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot modify {} '{}'; remove 'const' or use a local copy",
                self.vars.const_kind(var_nr),
                self.vars.name(var_nr)
            );
        }
        if matches!(code, Value::Insert(_)) {
            // nothing
        } else if op == "=" {
            *code = v_set(var_nr, code.clone());
        } else if s_type == &Type::Character {
            *code = self.cl(
                "OpAppendStackCharacter",
                &[Value::Var(var_nr), code.clone()],
            );
        } else {
            *code = self.cl("OpAppendStackText", &[Value::Var(var_nr), code.clone()]);
        }
    }

    pub(crate) fn append_to_file(&mut self, code: &mut Value, file_v: u16) {
        let mut rhs_code = Value::Null;
        let mut unused = Type::Null; // parent_tp, this is normally used to unpack the vector fill
        let mut rhs_type = self.parse_operators(&Type::Unknown(0), &mut rhs_code, &mut unused, 0);
        if let Type::Rewritten(tp) = rhs_type {
            rhs_type = *tp;
        }
        *code = self.write_to_file(file_v, rhs_code, &rhs_type);
    }

    /// Determine the variable number for an assignment target.
    /// For text `+=`, creates a unique temporary variable.
    pub(crate) fn assign_var_nr(
        &mut self,
        code: &mut Value,
        op: &str,
        f_type: &Type,
        parent_tp: &mut Type,
    ) -> u16 {
        if let Value::Var(v_nr) = *code {
            v_nr
        } else if op == "+=" && matches!(f_type, Type::Text(_)) {
            let v = self
                .vars
                .unique("field", &Type::Text(vec![]), &mut self.lexer);
            *code = Value::Var(v);
            *parent_tp = Type::Null;
            v
        } else {
            u16::MAX
        }
    }

    /// Handle assignment into a `RefVar(Text)` target; returns true if handled.
    pub(crate) fn assign_refvar_text(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        s_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        let Type::RefVar(t) = f_type else {
            return false;
        };
        if !matches!(**t, Type::Text(_)) {
            return false;
        }
        self.append_to_text(code, op, var_nr, s_type);
        true
    }

    /// Handle `v += expr` where `v: &vector<T>`; returns true if handled.
    /// NOTE: does NOT intercept `Value::Insert` — bracket-form `[elem]` literals are already
    /// handled by the Insert-expansion in `parse_block` → `OpFinishRecord`.
    pub(crate) fn assign_refvar_vector(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        let Type::RefVar(inner) = f_type else {
            return false;
        };
        let Type::Vector(elm_tp, _) = inner.as_ref() else {
            return false;
        };
        if op != "+=" {
            return false;
        }
        // Bracket-form [elem] and vector comprehensions produce Insert/Block; leave those
        // to the existing parse_block expansion path which uses OpFinishRecord.
        if matches!(code, Value::Insert(_) | Value::Block(_)) {
            return false;
        }
        if self.first_pass {
            return true;
        }
        let rec_tp = i32::from(self.data.def(self.data.type_def_nr(elm_tp)).known_type);
        *code = self.cl(
            "OpAppendVector",
            &[Value::Var(var_nr), code.clone(), Value::Int(rec_tp)],
        );
        true
    }

    pub(crate) fn validate_write(&mut self, to: &Value, parent_tp: &Type) {
        if let Value::Call(_, vars) = to
            && vars.len() > 1
            && let Value::Int(pos) = vars[1]
        {
            let d_nr = self.data.type_def_nr(parent_tp);
            if d_nr != u32::MAX {
                let known = self.data.def(d_nr).known_type;
                if known != u16::MAX
                    && let Parts::Struct(fields) = &self.database.types[known as usize].parts
                {
                    for (f_nr, f) in fields.iter().enumerate() {
                        if f.position == pos as u16 && !self.data.def(d_nr).attributes[f_nr].mutable
                        {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Cannot write to key field {}.{} create a record instead",
                                self.data.def(d_nr).name,
                                f.name
                            );
                        }
                    }
                }
            }
        }
    }

    /// Materialise an iterator (e.g. `v[a..b]` slice) into a vector variable.
    /// Promotes the LHS variable to `Vector<elm_tp>` and builds a loop that appends
    /// each element in-place.
    fn materialize_iterator(
        &mut self,
        code: &mut Value,
        s_type: &Type,
        to: &Value,
        lhs_parent_tp: &Type,
        var_nr: u16,
        op: &str,
    ) {
        let Type::Iterator(elm_tp, _) = s_type.clone() else {
            unreachable!()
        };
        let elm_tp = *elm_tp;
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Vec::new());
        self.change_var(to, &vec_tp);
        if !self.first_pass
            && let Value::Iter(_, init, next, _) = code.clone()
            && matches!(*next, Value::Block(_))
        {
            let ed_nr = self.data.type_def_nr(&elm_tp);
            let known_db = if ed_nr == u32::MAX || self.data.def(ed_nr).known_type == u16::MAX {
                0
            } else {
                self.database.vector(self.data.def(ed_nr).known_type)
            };
            let known = Value::Int(i32::from(known_db));
            let fld = Value::Int(i32::from(u16::MAX));
            let elm_var = self.unique_elm_var(lhs_parent_tp, &elm_tp, var_nr);
            let for_var = self.create_unique("slice_elm", &elm_tp);
            let comp_var = self.create_unique("comp", &elm_tp);
            let for_next = v_set(for_var, *next);
            let mut lp = vec![for_next];
            lp.push(v_set(comp_var, Value::Var(for_var)));
            lp.push(v_set(
                elm_var,
                self.cl(
                    "OpNewRecord",
                    &[Value::Var(var_nr), known.clone(), fld.clone()],
                ),
            ));
            lp.push(self.set_field(
                ed_nr,
                usize::MAX,
                0,
                Value::Var(elm_var),
                Value::Var(comp_var),
            ));
            lp.push(self.cl(
                "OpFinishRecord",
                &[Value::Var(var_nr), Value::Var(elm_var), known, fld],
            ));
            let needs_db = self.vector_needs_db(var_nr, &elm_tp, true);
            let mut stmts = Vec::new();
            if op == "=" && !needs_db {
                stmts.push(self.cl("OpClearVector", &[Value::Var(var_nr)]));
            }
            stmts.push(*init);
            stmts.push(v_loop(lp, "Slice materialise"));
            if needs_db {
                let db = self.insert_new(var_nr, elm_var, &elm_tp, &mut stmts);
                self.vars.depend(var_nr, db);
            }
            *code = Value::Insert(stmts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inline_ref_set_in;
    use crate::data::{Block, Type, Value};

    /// `inline_ref_set_in` must return false conservatively when nesting exceeds the limit.
    #[test]
    fn inline_ref_set_in_depth_limit_returns_false() {
        let mut v: Value = Value::Null;
        for _ in 0..1100 {
            v = Value::Block(Box::new(Block {
                name: "",
                operators: vec![v],
                result: Type::Void,
                scope: 0,
                var_size: 0,
            }));
        }
        // At depth limit, inline_ref_set_in must not overflow the stack.
        let result = inline_ref_set_in(&v, 0, 0);
        assert!(!result, "depth-exceeded should return false conservatively");
    }
}
