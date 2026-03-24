// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use super::{
    Argument, DefType, Function, HashSet, I32, Level, LexItem, LexResult, Mode, OPERATORS,
    OUTPUT_DEFAULT, OutputState, Parser, Parts, SKIP_TOKEN, SKIP_WIDTH, ToString, Type, Value,
    diagnostic_format, field_id, rename, to_default, v_block, v_if, v_loop, v_set,
};

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
        Value::Return(inner) | Value::Drop(inner) => inline_ref_set_in(inner, r, depth + 1),
        Value::Iter(_, a, b, c) => {
            inline_ref_set_in(a, r, depth + 1)
                || inline_ref_set_in(b, r, depth + 1)
                || inline_ref_set_in(c, r, depth + 1)
        }
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
        | Value::Keys(_) => false,
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
            // In debug builds: auto-lock the stores for every const Reference/Vector argument
            // at the very start of the function body (after work-variable initialisations).
            #[cfg(debug_assertions)]
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

    // <expression> ::= <for> | 'continue' | 'break' | 'return' <return> | '{' <block> | <operators>
    pub(crate) fn expression(&mut self, val: &mut Value) -> Type {
        if self.lexer.has_token("for") {
            self.parse_for(val);
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
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)] // +5 lines from dead-assignment check (T1-9)
    #[allow(clippy::too_many_lines)]
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
        // An untyped null literal (`null` keyword) produces Type::Null with Value::Null.
        // For scalar field assignments (e.g. `self.cur_def = null`), convert it to the
        // appropriate typed null constant (OpConvIntFromNull, OpConvLongFromNull, etc.)
        // so that the stack argument is actually pushed before the set-operator executes.
        // Without this, `generate(Value::Null)` emits nothing, leaving the stack short by
        // one argument and causing the operator to read garbage bytes as the value.
        //
        // Do NOT convert for reference/collection types: `collection[key] = null` must
        // reach towards_set_hash_remove with Value::Null intact so it can emit OpHashRemove.
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
        // A9: materialise an iterator (e.g. v[a..b] slice) into a vector variable.
        // Promotes the LHS variable to Vector<elm_tp> and builds a loop that appends
        // each element in-place; without this Value::Iter reaches codegen and panics.
        if matches!(&s_type, Type::Iterator(_, _))
            && matches!(f_type, Type::Unknown(_) | Type::Vector(_, _))
            && var_nr != u16::MAX
            && matches!(op, "=" | "+=")
        {
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
                // elm_var holds the DbRef returned by OpNewRecord; must be Reference-typed.
                let elm_var = self.unique_elm_var(&lhs_parent_tp, &elm_tp, var_nr);
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
            && matches!(s_type, Type::Integer(_, _))
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
        // L6.2: emit field constraint check after assignment to a constrained field.
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
            if let Some(type_name) = self.lexer.has_identifier()
                && let Some(tp) = self.parse_type(u32::MAX, &type_name, false)
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

    pub(crate) fn assign_text(
        &mut self,
        code: &mut Value,
        tp: &Type,
        to: &Value,
        op: &str,
        var_nr: u16,
    ) {
        if !self.first_pass && var_nr != u16::MAX && self.vars.is_const_param(var_nr) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot modify {} '{}'; remove 'const' or use a local copy",
                self.vars.const_kind(var_nr),
                self.vars.name(var_nr)
            );
        }
        if let Value::Call(_, parms) = to.clone() {
            if op == "=" {
                let mut p = parms.clone();
                p.push(code.clone());
                *code = self.cl("OpSetText", &p);
            } else {
                let mut ls = Vec::new();
                ls.push(v_set(var_nr, to.clone()));
                if let Value::Insert(cd) = code {
                    for c in cd {
                        ls.push(c.clone());
                    }
                } else if *tp == Type::Character {
                    ls.push(self.cl("OpAppendCharacter", &[Value::Var(var_nr), code.clone()]));
                } else {
                    ls.push(self.cl("OpAppendText", &[Value::Var(var_nr), code.clone()]));
                }
                let mut p = parms.clone();
                p.push(Value::Var(var_nr));
                ls.push(self.cl("OpSetText", &p));
                *code = Value::Insert(ls);
            }
        } else if let Value::Insert(ls) = code {
            if op == "=" {
                ls.insert(0, v_set(var_nr, Value::Text(String::new())));
            }
        } else if op == "=" && var_nr != u16::MAX {
            *code = v_set(var_nr, code.clone());
        } else if *tp == Type::Character {
            *code = self.cl("OpAppendCharacter", &[Value::Var(var_nr), code.clone()]);
        } else {
            *code = self.cl("OpAppendText", &[Value::Var(var_nr), code.clone()]);
        }
    }

    pub(crate) fn create_vector(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        if let (Value::Insert(ls), Type::Vector(tp, _)) = (code, f_type) {
            if !self.first_pass && self.vars.is_const_param(var_nr) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot modify {} '{}'; remove 'const' or use a local copy",
                    self.vars.const_kind(var_nr),
                    self.vars.name(var_nr)
                );
            }
            if op == "=" {
                for (s_nr, s) in self.vector_db(tp, var_nr).iter().enumerate() {
                    ls.insert(s_nr, s.clone());
                }
                if ls.is_empty()
                    && !self.first_pass
                    && var_nr != u16::MAX
                    && matches!(f_type, Type::Vector(_, _))
                {
                    ls.push(self.cl("OpClearVector", &[Value::Var(var_nr)]));
                }
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn copy_ref(&mut self, to: &Value, code: &Value, f_type: &Type) -> Value {
        let d_nr = self.data.type_def_nr(f_type);
        let tp = self.data.def(d_nr).known_type;
        // println!("here! f_type:{f_type} pass:{} to:{to:?} at {}", self.first_pass, self.lexer.pos());
        self.cl(
            "OpCopyRecord",
            &[code.clone(), to.clone(), Value::Int(i32::from(tp))],
        )
    }

    /** Mutate current code when it reads a value into writing it. This is needed for assignments.
     */
    pub(crate) fn compute_op_code(
        &mut self,
        op: &str,
        to: &Value,
        val: &Value,
        f_type: &Type,
    ) -> Value {
        if op == "=" {
            val.clone()
        } else if op == ">" {
            self.op("Lt", val.clone(), to.clone(), f_type.clone())
        } else if op == ">=" {
            self.op("Le", val.clone(), to.clone(), f_type.clone())
        } else {
            self.op(rename(op), to.clone(), val.clone(), f_type.clone())
        }
    }

    /// Dispatch an `OpGetX` getter name to the corresponding `OpSetX` setter call.
    pub(crate) fn call_to_set_op(
        &mut self,
        name: &str,
        args: &[Value],
        code: Value,
        op: &str,
    ) -> Value {
        match name {
            "OpGetInt" => self.cl("OpSetInt", &[args[0].clone(), args[1].clone(), code]),
            "OpGetByte" => self.cl(
                "OpSetByte",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            "OpGetEnum" => self.cl("OpSetEnum", &[args[0].clone(), args[1].clone(), code]),
            "OpGetShort" => self.cl(
                "OpSetShort",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            "OpGetLong" => {
                // f#next = pos: seek the file AND update the stored field.
                if args[1] == Value::Int(16)
                    && let Value::Var(v_nr) = &args[0]
                    && self.is_file_var(*v_nr)
                {
                    let seek = self.cl("OpSeekFile", &[args[0].clone(), code.clone()]);
                    let set = self.cl(
                        "OpSetLong",
                        &[args[0].clone(), args[1].clone(), code.clone()],
                    );
                    return Value::Insert(vec![seek, set]);
                }
                self.cl("OpSetLong", &[args[0].clone(), args[1].clone(), code])
            }
            "OpGetFloat" => self.cl("OpSetFloat", &[args[0].clone(), args[1].clone(), code]),
            "OpGetSingle" => self.cl("OpSetSingle", &[args[0].clone(), args[1].clone(), code]),
            "OpGetField" => code,
            "n_get_store_lock" => {
                // d#lock = val — validation enforced in parse_assign before this call.
                self.cl("n_set_store_lock", &[args[0].clone(), code])
            }
            "OpSizeFile" => {
                // f#size = n: delegate to set_file_size which validates format and sign.
                let fn_nr = self.data.def_nr("t_4File_set_file_size");
                if fn_nr == u32::MAX {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "set_file_size is not defined");
                    }
                    Value::Null
                } else {
                    Value::Call(fn_nr, vec![args[0].clone(), code])
                }
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(self.lexer, Level::Error, "Unknown {op} for {name}");
                }
                Value::Null
            }
        }
    }

    pub(crate) fn parse_operators(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
    ) -> Type {
        let mut ls = Vec::new();
        if precedence >= OPERATORS.len() {
            let t = self.parse_part(var_tp, code, parent_tp);
            return t;
        }
        let orig_var = if let Value::Var(nr) = code {
            *nr
        } else {
            u16::MAX
        };
        let mut current_type = self.parse_operators(var_tp, code, parent_tp, precedence + 1);
        loop {
            let mut operator = "";
            for op in OPERATORS[precedence] {
                if self.lexer.has_token(op) {
                    operator = op;
                    break;
                }
            }
            if operator.is_empty() {
                if !ls.is_empty() {
                    if matches!(current_type, Type::Text(_) | Type::Character) {
                        if current_type == Type::Character {
                            // S9: a Character variable cannot serve as an OpAppendText
                            // destination.  Prepend it to the parts list and use an empty
                            // text literal as the first operand so parse_append_text
                            // creates a fresh work text.
                            ls.insert(0, (code.clone(), Type::Character));
                            *code = Value::Text(String::new());
                            return self.parse_append_text(
                                code,
                                &Type::Text(Vec::new()),
                                &ls,
                                u16::MAX,
                            );
                        }
                        return self.parse_append_text(code, &current_type, &ls, orig_var);
                    } else if matches!(current_type, Type::Vector(_, _)) {
                        return self.parse_append_vector(code, &current_type, &ls, orig_var);
                    } else if let Type::RefVar(inner) = &current_type
                        && matches!(**inner, Type::Vector(_, _))
                    {
                        return self.parse_append_vector(code, inner, &ls, orig_var);
                    }
                }
                return current_type;
            }
            self.known_var_or_type(code);
            if operator == "+"
                && matches!(
                    current_type,
                    Type::Text(_) | Type::Character | Type::Vector(_, _)
                )
            {
                let mut second_code = Value::Null;
                let tp = self.parse_operators(var_tp, &mut second_code, parent_tp, precedence + 1);
                ls.push((second_code, tp));
            } else if let Some(value) = self.handle_operator(
                var_tp,
                code,
                parent_tp,
                precedence,
                &mut current_type,
                operator,
            ) {
                return value;
            }
        }
    }

    pub(crate) fn parse_part(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
    ) -> Type {
        let mut t = self.parse_single(var_tp, code, parent_tp);
        while self.lexer.peek_token(".") || self.lexer.peek_token("[") {
            if !self.first_pass && t.is_unknown() && matches!(code, Value::Var(_)) {
                diagnostic!(self.lexer, Level::Error, "Unknown variable");
            }
            if self.lexer.has_token(".") {
                *parent_tp = t.clone();
                t = self.field(code, t);
                // If the method returned an owned ref and more chaining follows, capture
                // it in a work-ref so scopes.rs emits OpFreeRef at end-of-scope.
                // Without this, the store allocated by the callee leaks and the LIFO
                // invariant in database::free() is violated.
                if !self.first_pass
                    && !matches!(code, Value::Var(_))
                    && (self.lexer.peek_token(".") || self.lexer.peek_token("["))
                    && let Type::Reference(d_nr, dep) = &t
                    && dep.is_empty()
                {
                    let d_nr = *d_nr;
                    let w = self.vars.work_refs(&t.clone(), &mut self.lexer);
                    // Mark as inline-ref temp so parse_code inserts its
                    // null-init after the first user statement, ensuring
                    // it appears after user-scope vars in var_order and is
                    // therefore freed before them (LIFO).
                    self.vars.mark_inline_ref(w);
                    let orig = code.clone();
                    *code = v_block(
                        vec![v_set(w, orig), Value::Var(w)],
                        Type::Reference(d_nr, vec![w]),
                        "inline ref",
                    );
                    t = Type::Reference(d_nr, vec![w]);
                }
            } else if self.lexer.has_token("[") {
                t = self.parse_index(code, &t);
                self.lexer.token("]");
            }
        }
        t
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn handle_operator(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
        ctp: &mut Type,
        operator: &str,
    ) -> Option<Type> {
        if operator == "??" {
            // Null-coalescing: `x ?? default` evaluates to `x` if x is not null,
            // otherwise to `default`.  Compiles as: if (x != null_sentinel) { x } else { default }.
            // Note: `x` is evaluated twice for non-trivial expressions (known V1 limitation).
            // Returns None so the outer loop in parse_operators continues, allowing chaining.
            if self.expr_not_null && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "Redundant null coalescing — '{}' is 'not null', default is never used",
                    self.expr_not_null_name,
                );
            }
            self.expr_not_null = false;
            let lhs_type = ctp.clone();
            let mut rhs = Value::Null;
            let rhs_type = self.parse_operators(var_tp, &mut rhs, parent_tp, precedence + 1);
            self.known_var_or_type(&rhs);
            if matches!(lhs_type, Type::Null) {
                // LHS is an untyped null literal: always use the RHS.
                *code = rhs;
                *ctp = rhs_type;
            } else {
                if !self.convert(&mut rhs, &rhs_type, &lhs_type) && !self.first_pass {
                    self.can_convert(&rhs_type, &lhs_type);
                }
                let lhs = code.clone();
                // Use boolean truthiness (!is_null) instead of `!= null` comparison.
                // For floats, `!= NaN` is always true after NaN-guard fix; boolean
                // conversion (`conv_bool_from_float`) correctly detects NaN as falsy.
                let mut null_check = code.clone();
                self.convert(&mut null_check, &lhs_type, &Type::Boolean);
                *code = v_if(null_check, lhs, rhs);
                *ctp = lhs_type;
            }
        } else if operator == "as" {
            self.expr_not_null = false;
            if let Some(tps) = self.lexer.has_identifier() {
                let Some(tp) = self.parse_type(u32::MAX, &tps, false) else {
                    diagnostic!(self.lexer, Level::Error, "Expect type");
                    return Some(Type::Null);
                };
                if !self.convert(code, ctp, &tp) && !self.cast(code, ctp, &tp) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown cast from {} to {tps}",
                        &ctp.name(&self.data),
                    );
                }
                let mut rt = tp;
                for d in ctp.depend() {
                    rt = rt.depending(d);
                }
                return Some(rt);
            }
            diagnostic!(self.lexer, Level::Error, "Expect type after as");
        } else if operator == "or" || operator == "||" {
            self.expr_not_null = false;
            self.boolean_operator(code, ctp, precedence, true);
            *ctp = Type::Boolean;
        } else if operator == "and" || operator == "&&" {
            self.expr_not_null = false;
            self.boolean_operator(code, ctp, precedence, false);
            *ctp = Type::Boolean;
        } else if operator == "=="
            || operator == "!="
            || operator == "<"
            || operator == "<="
            || operator == ">"
            || operator == ">="
        {
            let lhs_not_null = self.expr_not_null;
            let lhs_not_null_name = self.expr_not_null_name.clone();
            self.expr_not_null = false;
            let mut second_code = Value::Null;
            let tp = parent_tp.clone();
            *parent_tp = ctp.clone();
            let second_type =
                self.parse_operators(var_tp, &mut second_code, parent_tp, precedence + 1);
            self.known_var_or_type(&second_code);
            if !self.first_pass && (operator == "==" || operator == "!=") {
                if second_type == Type::Null && lhs_not_null {
                    let always = if operator == "==" { "false" } else { "true" };
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        "Redundant null check — '{lhs_not_null_name}' is 'not null', comparison is always {always}",
                    );
                } else if *ctp == Type::Null && self.expr_not_null {
                    let always = if operator == "==" { "false" } else { "true" };
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        "Redundant null check — '{}' is 'not null', comparison is always {always}",
                        self.expr_not_null_name,
                    );
                }
            }
            self.expr_not_null = false;
            if operator == ">" {
                *ctp = self.call_op(
                    code,
                    "<",
                    &[second_code, code.clone()],
                    &[second_type, ctp.clone()],
                );
            } else if operator == ">=" {
                *ctp = self.call_op(
                    code,
                    "<=",
                    &[second_code, code.clone()],
                    &[second_type, ctp.clone()],
                );
            } else {
                *ctp = self.call_op(
                    code,
                    operator,
                    &[code.clone(), second_code],
                    &[ctp.clone(), second_type],
                );
            }
            *parent_tp = tp;
        } else {
            self.expr_not_null = false;
            let mut second_code = Value::Null;
            let second_type =
                self.parse_operators(var_tp, &mut second_code, parent_tp, precedence + 1);
            self.known_var_or_type(&second_code);
            if !self.first_pass
                && (operator == "/" || operator == "%")
                && (matches!(second_code, Value::Int(0)) || matches!(second_code, Value::Long(0)))
            {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "{} by constant zero — result is always null",
                    if operator == "/" {
                        "Division"
                    } else {
                        "Modulo"
                    }
                );
            }
            *ctp = self.call_op(
                code,
                operator,
                &[code.clone(), second_code],
                &[ctp.clone(), second_type],
            );
        }
        None
    }

    pub(crate) fn parse_append_vector(
        &mut self,
        code: &mut Value,
        tp: &Type,
        parts: &[(Value, Type)],
        orig_var: u16,
    ) -> Type {
        let mut ls = Vec::new();
        let rec_tp = if let Type::Vector(cont, _) = tp {
            i32::from(self.data.def(self.data.type_def_nr(cont)).known_type)
        } else {
            i32::MIN
        };
        let var_nr = if orig_var == u16::MAX {
            let vec = self.create_unique("vec", tp);
            let elm_tp = tp.content();
            for l in self.vector_db(&elm_tp, vec) {
                ls.push(l);
            }
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(vec), code.clone(), Value::Int(rec_tp)],
            ));
            vec
        } else if let Value::Insert(elms) = code {
            for e in elms {
                ls.push(e.clone());
            }
            orig_var
        } else if matches!(self.vars.tp(orig_var), Type::RefVar(t) if matches!(**t, Type::Vector(_, _)))
        {
            // RefVar(Vector): append directly without an identity Set(v, Var(v)).
            // find_written_vars detects the write via the OpAppendVector in the parts loop.
            orig_var
        } else {
            ls.push(v_set(orig_var, code.clone()));
            orig_var
        };
        for (val, _) in parts {
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(var_nr), val.clone(), Value::Int(rec_tp)],
            ));
        }
        if orig_var == u16::MAX {
            let res = self.vars.tp(var_nr).clone();
            ls.push(Value::Var(var_nr));
            *code = v_block(ls, res.clone(), "Append Vector");
            return res;
        }
        *code = Value::Insert(ls);
        Type::Rewritten(Box::new(tp.clone()))
    }

    pub(crate) fn parse_append_text(
        &mut self,
        code: &mut Value,
        tp: &Type,
        parts: &[(Value, Type)],
        orig_var: u16,
    ) -> Type {
        let mut ls = Vec::new();
        let var_nr = if orig_var == u16::MAX {
            let v = self.vars.work_text(&mut self.lexer);
            if matches!(self.vars.tp(v), Type::RefVar(_)) {
                ls.push(self.cl("OpClearStackText", &[Value::Var(v)]));
                ls.push(self.cl("OpAppendStackText", &[Value::Var(v), code.clone()]));
            } else if tp == &Type::Character {
                ls.push(self.cl("OpClearText", &[Value::Var(v)]));
                ls.push(self.cl("OpAppendCharacter", &[Value::Var(v), code.clone()]));
            } else {
                ls.push(self.cl("OpClearText", &[Value::Var(v)]));
                ls.push(self.cl("OpAppendText", &[Value::Var(v), code.clone()]));
            }
            v
        } else if matches!(self.vars.tp(orig_var), Type::RefVar(_)) {
            ls.push(self.cl("OpAppendStackText", &[Value::Var(orig_var), code.clone()]));
            orig_var
        } else {
            ls.push(self.cl("OpAppendText", &[Value::Var(orig_var), code.clone()]));
            orig_var
        };
        for (val, tp) in parts {
            if matches!(self.vars.tp(var_nr), Type::RefVar(_)) {
                if *tp == Type::Character {
                    ls.push(self.cl("OpAppendStackCharacter", &[Value::Var(var_nr), val.clone()]));
                } else {
                    ls.push(self.cl("OpAppendStackText", &[Value::Var(var_nr), val.clone()]));
                }
            } else if *tp == Type::Character {
                ls.push(self.cl("OpAppendCharacter", &[Value::Var(var_nr), val.clone()]));
            } else {
                ls.push(self.cl("OpAppendText", &[Value::Var(var_nr), val.clone()]));
            }
        }
        let tp = Type::Text(vec![var_nr]);
        if orig_var == u16::MAX || var_nr != orig_var {
            // A new work text was created (either no orig_var, or orig_var was a
            // Character variable) — wrap in a Block so the work text appears on the stack.
            ls.push(Value::Var(var_nr));
            *code = v_block(ls, tp.clone(), "Add text");
            return tp;
        }
        *code = Value::Insert(ls);
        Type::Rewritten(Box::new(tp))
    }

    /// Rewrite boolean operators into an `IF` statement to prevent the calculation of the second
    /// expression when it is unneeded.
    pub(crate) fn boolean_operator(
        &mut self,
        code: &mut Value,
        tp: &Type,
        precedence: usize,
        is_or: bool,
    ) {
        if !self.convert(code, tp, &Type::Boolean) && !self.first_pass {
            self.can_convert(tp, &Type::Boolean);
        }
        let mut second_code = Value::Null;
        let mut parent_tp = Type::Unknown(0);
        let second_type = self.parse_operators(
            &Type::Unknown(0),
            &mut second_code,
            &mut parent_tp,
            precedence + 1,
        );
        self.known_var_or_type(&second_code);
        if !self.convert(&mut second_code, &second_type, &Type::Boolean) && !self.first_pass {
            self.can_convert(&second_type, &Type::Boolean);
        }
        *code = v_if(
            code.clone(),
            if is_or {
                Value::Boolean(true)
            } else {
                second_code.clone()
            },
            if is_or {
                second_code
            } else {
                Value::Boolean(false)
            },
        );
    }

    // <single> ::= '!' <expression> |
    //              '(' <expression> ')' |
    //              <vector> |
    //              'if' <if> |
    //              <identifier:var> |
    //              <number> | <float> | <cstring> |
    //              'true' | 'false' | 'null'
    pub(crate) fn parse_single(
        &mut self,
        var_tp: &Type,
        val: &mut Value,
        parent_tp: &mut Type,
    ) -> Type {
        if self.lexer.has_token("!") {
            let t = self.parse_part(var_tp, val, parent_tp);
            let arg = val.clone();
            self.call_op(val, "Not", &[arg], &[t])
        } else if self.lexer.has_token("-") {
            let t = self.parse_part(var_tp, val, parent_tp);
            let arg = val.clone();
            self.call_op(val, "Min", &[arg], &[t])
        } else if self.lexer.has_token("(") {
            let t = self.expression(val);
            self.lexer.token(")");
            t
        } else if self.lexer.peek_token("{") {
            self.parse_block("block", val, &Type::Unknown(0))
        } else if self.lexer.has_token("[") {
            self.parse_vector(var_tp, val, parent_tp)
        } else if self.lexer.has_token("if") {
            self.parse_if(val)
        } else if self.lexer.has_token("match") {
            self.parse_match(val)
        } else if self.lexer.has_token("fn") {
            if self.lexer.peek_token("(") {
                self.parse_lambda(val)
            } else {
                // S11: function references use the bare name, not 'fn name'.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Use the function name directly, without 'fn' prefix"
                );
                self.parse_fn_ref(val)
            }
        } else if self.lexer.has_token("||") {
            // Zero-parameter short lambda: || { body } — `||` already consumed, no closing `|`
            self.parse_lambda_short(val, false)
        } else if self.lexer.has_token("|") {
            // Short lambda with parameters: |x: T, …| { body } — opening `|` consumed
            self.parse_lambda_short(val, true)
        } else if self.lexer.has_token("sizeof") {
            self.lexer.token("(");
            self.parse_size(val)
        } else if self.lexer.has_token("type_name") {
            self.lexer.token("(");
            self.parse_type_name(val)
        } else if self.lexer.has_token("assert") {
            self.lexer.token("(");
            self.parse_intrinsic_call(val, "assert")
        } else if self.lexer.has_token("panic") {
            self.lexer.token("(");
            self.parse_intrinsic_call(val, "panic")
        } else if let Some(name) = self.lexer.has_identifier() {
            self.parse_var(val, &name, parent_tp)
        } else if self.lexer.has_token("$") {
            self.parse_var(val, "$", parent_tp)
        } else if let Some(nr) = self.lexer.has_integer() {
            *val = Value::Int(nr as i32);
            I32.clone()
        } else if let Some(nr) = self.lexer.has_long() {
            *val = Value::Long(nr as i64);
            Type::Long
        } else if let Some(nr) = self.lexer.has_float() {
            *val = Value::Float(nr);
            Type::Float
        } else if let Some(nr) = self.lexer.has_single() {
            *val = Value::Single(nr);
            Type::Single
        } else if let Some(s) = self.lexer.has_cstring() {
            self.parse_string(val, &s)
        } else if let Some(nr) = self.lexer.has_char() {
            *val = Value::Int(nr as i32);
            Type::Character
        } else if self.lexer.has_token("true") {
            *val = Value::Boolean(true);
            Type::Boolean
        } else if self.lexer.has_token("false") {
            *val = Value::Boolean(false);
            Type::Boolean
        } else if self.lexer.has_token("null") {
            *val = Value::Null;
            Type::Null
        } else {
            Type::Unknown(0)
        }
    }

    // <fn-ref> ::= 'fn' <identifier>
    // Produces a Type::Function value whose runtime representation is the
    // definition number (d_nr) of the named function stored as an i32.
    pub(crate) fn parse_fn_ref(&mut self, code: &mut Value) -> Type {
        let Some(name) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Expect function name after fn");
            }
            return Type::Unknown(0);
        };
        // Try user function (n_<name>) first, then fall back to bare name.
        let d_nr = {
            let prefixed = format!("n_{name}");
            let nr = self.data.def_nr(&prefixed);
            if nr == u32::MAX {
                self.data.def_nr(&name)
            } else {
                nr
            }
        };
        if d_nr == u32::MAX {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Unknown function '{name}'");
            }
            return Type::Unknown(0);
        }
        if !self.first_pass && !matches!(self.data.def_type(d_nr), DefType::Function) {
            diagnostic!(self.lexer, Level::Error, "'{name}' is not a function");
            return Type::Unknown(0);
        }
        *code = Value::Int(d_nr as i32);
        self.data.def_used(d_nr);
        let n_args = self.data.attributes(d_nr);
        let arg_types: Vec<Type> = (0..n_args).map(|a| self.data.attr_type(d_nr, a)).collect();
        let ret_type = self.data.def(d_nr).returned.clone();
        Type::Function(arg_types, Box::new(ret_type))
    }

    // <lambda> ::= 'fn' '(' [<params>] ')' ['->' <type>] '{' <body> '}'
    // Produces Type::Function; runtime representation is d_nr as i32, same as fn-ref.
    pub(crate) fn parse_lambda(&mut self, code: &mut Value) -> Type {
        let lambda_name = format!("__lambda_{}", self.lambda_counter);
        self.lambda_counter += 1;
        let stored_name = format!("n_{lambda_name}");

        let outer_context = self.context;
        let outer_vars = std::mem::replace(
            &mut self.vars,
            Function::new(&lambda_name, &self.lexer.pos().file),
        );
        let outer_loop = self.in_loop;
        self.in_loop = false;

        self.lexer.token("(");
        let mut arguments = Vec::new();
        self.parse_arguments(&lambda_name, &mut arguments);
        self.lexer.token(")");

        self.context = if self.first_pass {
            self.data.add_fn(&mut self.lexer, &lambda_name, &arguments)
        } else {
            self.data.def_nr(&stored_name)
        };
        if self.context == u32::MAX {
            self.context = outer_context;
            self.vars = outer_vars;
            self.in_loop = outer_loop;
            return Type::Unknown(0);
        }
        let d_nr = self.context;

        // Parse optional return type annotation.
        let result = if self.lexer.has_token("->") {
            if let Some(type_name) = self.lexer.has_identifier() {
                self.parse_type(d_nr, &type_name, true)
                    .unwrap_or(Type::Void)
            } else {
                Type::Void
            }
        } else {
            Type::Void
        };
        if self.first_pass {
            self.data.set_returned(d_nr, result);
        }

        self.vars
            .append(&mut self.data.definitions[d_nr as usize].variables);
        for (a_nr, a) in arguments.iter().enumerate() {
            if self.first_pass {
                let v_nr = self.create_var(&a.name, &a.typedef);
                if v_nr != u16::MAX {
                    self.vars.become_argument(v_nr);
                    self.var_usages(v_nr, false);
                }
            } else {
                self.change_var_type(a_nr as u16, &a.typedef);
            }
        }

        self.parse_code();
        self.data.op_code(d_nr);
        self.data.definitions[d_nr as usize]
            .variables
            .append(&mut self.vars);

        self.context = outer_context;
        self.vars = outer_vars;
        self.in_loop = outer_loop;

        self.data.def_used(d_nr);
        *code = Value::Int(d_nr as i32);
        let n_args = self.data.attributes(d_nr);
        let arg_types: Vec<Type> = (0..n_args).map(|a| self.data.attr_type(d_nr, a)).collect();
        let ret_type = self.data.def(d_nr).returned.clone();
        Type::Function(arg_types, Box::new(ret_type))
    }

    // <short-lambda> ::= '||' ['->' type] block              (expect_close=false)
    //                  | '|' [param {',' param}] '|' ['->' type] block  (expect_close=true)
    // param ::= ident [':' type]
    // `expect_close` is true when the opening `|` was consumed (params may follow);
    // false when `||` was consumed (zero params, no closing `|`).
    // Types are inferred from `lambda_hint` (set by the call-site parser) when omitted.
    // Produces Type::Function; runtime representation is d_nr as i32, same as fn-ref.
    #[allow(clippy::too_many_lines)] // single context save/restore spans the whole body; splitting would need unsafe borrowing
    pub(crate) fn parse_lambda_short(&mut self, code: &mut Value, expect_close: bool) -> Type {
        let lambda_name = format!("__lambda_{}", self.lambda_counter);
        self.lambda_counter += 1;
        let stored_name = format!("n_{lambda_name}");

        // Capture hint types before entering the new context.
        let hint_params_ret = self.lambda_hint.clone();
        let hint_params: Vec<Type> = if let Type::Function(pts, _) = &hint_params_ret {
            pts.clone()
        } else {
            Vec::new()
        };

        // Parse parameter list from `|p1 [: T], p2 [: T], …|`.
        // When expect_close=false (`||` was consumed), there are no params and no closing `|`.
        let mut param_names: Vec<String> = Vec::new();
        let mut param_types: Vec<Type> = Vec::new();
        if expect_close {
            while !self.lexer.peek_token("|") && !self.lexer.peek_token("{") {
                let Some(pname) = self.lexer.has_identifier() else {
                    break;
                };
                let idx = param_names.len();
                let tp = if self.lexer.has_token(":") {
                    // S10: type annotations are not allowed in |x| short-form lambdas.
                    // Use the long form fn(x: type) -> ret { body } instead.
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Type annotations are not allowed in |x| lambdas — \
                         use fn({pname}: <type>) -> <ret> {{ ... }} instead"
                    );
                    // Consume the type token so parsing can continue.
                    let _ = self.lexer.has_identifier();
                    // Infer from hint to keep parsing viable.
                    hint_params.get(idx).cloned().unwrap_or(Type::Unknown(0))
                } else {
                    // Infer from hint.
                    hint_params.get(idx).cloned().unwrap_or(Type::Unknown(0))
                };
                param_names.push(pname);
                param_types.push(tp);
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("|"); // consume closing `|`
        }

        // Build Argument list for function registration.
        let arguments: Vec<Argument> = param_names
            .iter()
            .zip(param_types.iter())
            .map(|(n, t)| Argument {
                name: n.clone(),
                typedef: t.clone(),
                default: Value::Null,
                constant: false,
            })
            .collect();

        // Error on second pass for any parameter whose type is still Unknown.
        if !self.first_pass {
            for a in &arguments {
                if a.typedef.is_unknown() {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot infer type for lambda parameter '{}'; pass the lambda where the expected type is known, or use fn(name: <type>) -> <ret> {{{{ ... }}}}",
                        a.name
                    );
                }
            }
        }

        let outer_context = self.context;
        let outer_vars = std::mem::replace(
            &mut self.vars,
            Function::new(&lambda_name, &self.lexer.pos().file),
        );
        let outer_loop = self.in_loop;
        self.in_loop = false;

        self.context = if self.first_pass {
            self.data.add_fn(&mut self.lexer, &lambda_name, &arguments)
        } else {
            self.data.def_nr(&stored_name)
        };
        if self.context == u32::MAX {
            self.context = outer_context;
            self.vars = outer_vars;
            self.in_loop = outer_loop;
            return Type::Unknown(0);
        }
        let d_nr = self.context;

        // S10: return-type annotations are not allowed in |x| short-form lambdas.
        let has_arrow = self.lexer.has_token("->");
        let result = if has_arrow {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Return-type annotations are not allowed in |x| lambdas — \
                 use fn(…) -> <ret> {{ ... }} instead"
            );
            if let Some(type_name) = self.lexer.has_identifier() {
                self.parse_type(d_nr, &type_name, true)
                    .unwrap_or(Type::Void)
            } else {
                Type::Void
            }
        } else if let Type::Function(_, ret) = &hint_params_ret {
            *ret.clone()
        } else {
            Type::Void
        };
        if self.first_pass {
            // On first pass, hint is unavailable — store Void when no annotation.
            self.data.set_returned(
                d_nr,
                if has_arrow {
                    result.clone()
                } else {
                    Type::Void
                },
            );
        } else if !result.is_unknown() && !matches!(result, Type::Void) {
            // On second pass, force-update the return type from hint or annotation.
            self.data.definitions[d_nr as usize].returned = result.clone();
        }

        self.vars
            .append(&mut self.data.definitions[d_nr as usize].variables);
        for (a_nr, a) in arguments.iter().enumerate() {
            if self.first_pass {
                let v_nr = self.create_var(&a.name, &a.typedef);
                if v_nr != u16::MAX {
                    self.vars.become_argument(v_nr);
                    self.var_usages(v_nr, false);
                }
            } else {
                self.change_var_type(a_nr as u16, &a.typedef);
                // Force-update the data definition with the inferred type.
                // `set_attr_type` panics on non-unknown, so write directly.
                // (First pass stored Unknown(0); typedef.rs may have resolved that to a
                // concrete type before the second pass, so we can't rely on is_unknown().)
                if !a.typedef.is_unknown() {
                    self.data.definitions[d_nr as usize].attributes[a_nr].typedef =
                        a.typedef.clone();
                }
            }
        }

        self.parse_code();
        self.data.op_code(d_nr);
        self.data.definitions[d_nr as usize]
            .variables
            .append(&mut self.vars);

        self.context = outer_context;
        self.vars = outer_vars;
        self.in_loop = outer_loop;

        self.data.def_used(d_nr);
        *code = Value::Int(d_nr as i32);
        let n_args = self.data.attributes(d_nr);
        let arg_types: Vec<Type> = (0..n_args).map(|a| self.data.attr_type(d_nr, a)).collect();
        let ret_type = self.data.def(d_nr).returned.clone();
        Type::Function(arg_types, Box::new(ret_type))
    }

    // <for-vector> ::= 'for' <id> 'in' <range> ['if' <cond>] '{' <expr> '}'
    // Implements [for n in range { body }] vector comprehensions.
    #[allow(clippy::too_many_arguments)] // parser helper threading IR-construction params alongside &mut self; no sensible grouping reduces the count
    pub(crate) fn parse_vector_for(
        &mut self,
        vec: u16,
        elm: u16,
        in_t: &mut Type,
        val: &mut Value,
        is_var: bool,
        is_field: bool,
        block: bool,
        parent_tp: &Type,
    ) -> Type {
        let Some(id) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect variable after for");
            return Type::Null;
        };
        self.lexer.token("in");
        let loop_nr = self.vars.start_loop();
        let mut expr = Value::Null;
        let mut in_type = self.parse_in_range(&mut expr, &Value::Null, &id);
        let mut fill = Value::Null;
        if matches!(in_type, Type::Vector(_, _)) {
            let vec_var = self.create_unique("vector", &in_type);
            in_type = in_type.depending(vec_var);
            fill = v_set(vec_var, expr);
            expr = Value::Var(vec_var);
        }
        let var_tp = self.for_type(&in_type);
        let (iter_var, pre_var) = if matches!(in_type, Type::Text(_)) {
            let pos_var = self.create_var(&format!("{id}#next"), &I32);
            self.vars.defined(pos_var);
            let index_var = self.create_var(&format!("{id}#index"), &I32);
            self.vars.defined(index_var);
            (pos_var, Some(index_var))
        } else {
            let iv = self.create_var(&format!("{id}#index"), &I32);
            self.vars.defined(iv);
            (iv, None)
        };
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
        // Parse body as an expression-returning block: [for n in range { expr }]
        let mut body = Value::Null;
        let body_type = self.parse_block("for", &mut body, &Type::Unknown(0));
        *in_t = body_type.clone();
        self.in_loop = in_loop;
        self.vars.finish_loop(loop_nr);
        // Finalise vector element type (same as parse_vector post-loop)
        let struct_tp = Type::Vector(Box::new(in_t.clone()), parent_tp.depend());
        if !is_field {
            self.vars
                .change_var_type(vec, &struct_tp, &self.data, &mut self.lexer);
            self.data.vector_def(&mut self.lexer, in_t);
        }
        let tp = Type::Vector(Box::new(in_t.clone()), parent_tp.depend());
        if self.first_pass {
            return tp;
        }
        // Second pass: build the append-in-loop bytecode.
        self.build_comprehension_code(
            vec,
            elm,
            in_t,
            &in_type,
            &var_tp,
            for_var,
            for_next,
            pre_var,
            fill,
            create_iter,
            if_step,
            body,
            val,
            is_var,
            is_field,
            block,
            tp,
        )
    }

    /// Build the second-pass bytecode for a `[for ... { body }]` vector comprehension.
    // parser helper threading IR-construction params alongside &mut self; no sensible grouping reduces the count
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_comprehension_code(
        &mut self,
        vec: u16,
        elm: u16,
        in_t: &Type,
        in_type: &Type,
        var_tp: &Type,
        for_var: u16,
        for_next: Value,
        pre_var: Option<u16>,
        fill: Value,
        create_iter: Value,
        if_step: Value,
        body: Value,
        val: &mut Value,
        is_var: bool,
        is_field: bool,
        block: bool,
        mut tp: Type,
    ) -> Type {
        // Per-iteration: OpNewRecord / set_field / OpFinishRecord pattern.
        let ed_nr = self.data.type_def_nr(in_t);
        let known = Value::Int(i32::from(
            if ed_nr == u32::MAX || self.data.def(ed_nr).known_type == u16::MAX {
                0
            } else {
                self.database.vector(self.data.def(ed_nr).known_type)
            },
        ));
        let fld = Value::Int(i32::from(u16::MAX));
        let comp_var = self.create_unique("comp", in_t);
        let mut lp = vec![for_next];
        if !matches!(in_type, Type::Iterator(_, _)) {
            let mut test_for = Value::Var(for_var);
            self.convert(&mut test_for, var_tp, &Type::Boolean);
            test_for = self.cl("OpNot", &[test_for]);
            lp.push(v_if(
                test_for,
                v_block(vec![Value::Break(0)], Type::Void, "break"),
                Value::Null,
            ));
        }
        if if_step != Value::Null {
            lp.push(v_if(if_step, Value::Null, Value::Continue(0)));
        }
        lp.push(v_set(comp_var, body));
        lp.push(v_set(
            elm,
            self.cl(
                "OpNewRecord",
                &[Value::Var(vec), known.clone(), fld.clone()],
            ),
        ));
        lp.push(self.set_field(ed_nr, usize::MAX, 0, Value::Var(elm), Value::Var(comp_var)));
        lp.push(self.cl(
            "OpFinishRecord",
            &[Value::Var(vec), Value::Var(elm), known, fld],
        ));
        let mut for_steps: Vec<Value> = Vec::new();
        if fill != Value::Null {
            for_steps.push(fill);
        }
        if let Some(idx_var) = pre_var {
            for_steps.push(v_set(idx_var, Value::Int(0)));
        }
        for_steps.push(create_iter);
        for_steps.push(v_loop(lp, "For comprehension"));
        let mut ls: Vec<Value> = Vec::new();
        if block {
            ls.extend(self.vector_db(in_t, vec));
            // After vector_db, vec's type carries the db dependency.  Propagate that
            // into tp so that (a) the block's result type keeps the db alive until the
            // block exits, and (b) the caller receives the correct Vector<T,[db]> type,
            // preventing scopes from emitting a redundant OpFreeRef for the result variable.
            if let Type::Vector(elem, _) = &tp {
                tp = Type::Vector(elem.clone(), self.vars.tp(vec).depend().clone());
            }
        }
        ls.extend(for_steps);
        if self.vector_needs_db(vec, in_t, is_var) {
            let db = self.insert_new(vec, elm, in_t, &mut ls);
            self.vars.depend(vec, db);
        } else if !is_field && !is_var && *val != Value::Null {
            ls.insert(0, v_set(vec, val.clone()));
        }
        if !is_var && !is_field {
            ls.push(Value::Var(vec));
        }
        *val = if block || (!is_var && !is_field) {
            v_block(ls, tp.clone(), "Vector comprehension")
        } else {
            Value::Insert(ls)
        };
        tp
    }

    /**
    Fill a structure (vector) with values. This can be done in different situations:
    - On a new variable, this creates a variable pointing to a structure with the vector.
    - As a stand-alone expression, this creates a new structure of type vector.
    - On an existing variable, this fills (or replaces) the vector with more elements.
    - On a field inside a structure, this fills any data structure with more elements.
    */
    // <vector> ::= '[' <expr> [ ';' <size-expr>]{ ',' <expr> [ ';' <size-expr> } ']'
    pub(crate) fn parse_vector(
        &mut self,
        var_tp: &Type,
        val: &mut Value,
        parent_tp: &Type,
    ) -> Type {
        let assign_tp = var_tp.content();
        let is_field = self.is_field(val);
        let is_var = matches!(val, Value::Var(_));
        if self.lexer.has_token("]") {
            return if is_var {
                *val = Value::Insert(vec![]);
                Type::Rewritten(Box::new(var_tp.clone()))
            } else if is_field {
                // Empty `[]` on a struct field: the field is already zero-initialized by
                // OpDatabase; there is nothing to emit.  Wrapping the OpGetField result in
                // Value::Insert would leave a dangling 12-byte DbRef on the expression stack.
                *val = Value::Insert(vec![]);
                var_tp.clone()
            } else {
                *val = Value::Insert(vec![val.clone()]);
                var_tp.clone()
            };
        }
        let block = !is_field && !matches!(val, Value::Var(_));
        let vec = if is_field {
            u16::MAX
        } else if let Value::Var(nr) = val {
            *nr
        } else {
            self.create_unique(
                "vec",
                &Type::Vector(Box::new(assign_tp.clone()), parent_tp.depend()),
            )
        };
        let mut in_t = assign_tp.clone();
        let mut res = Vec::new();
        let elm = self.unique_elm_var(parent_tp, &assign_tp, vec);
        // Handle [for n in range [if cond] { body }] vector comprehension
        if self.lexer.peek_token("for") {
            self.lexer.has_token("for");
            let tp =
                self.parse_vector_for(vec, elm, &mut in_t, val, is_var, is_field, block, parent_tp);
            self.lexer.token("]");
            return tp;
        }
        if let Some(early) = self.collect_vector_items(elm, &mut in_t, &mut res) {
            return early;
        }
        // convert parts to the common type
        if in_t == Type::Null {
            return in_t;
        }
        let struct_tp = Type::Vector(Box::new(in_t.clone()), parent_tp.depend());
        if !is_field {
            self.vars
                .change_var_type(vec, &struct_tp, &self.data, &mut self.lexer);
            self.data.vector_def(&mut self.lexer, &in_t);
        }
        let tp = Type::Vector(Box::new(in_t.clone()), parent_tp.depend());
        let (tp, ls) =
            self.build_vector_list(val, parent_tp, elm, vec, &res, &in_t, tp, is_var, is_field);
        self.lexer.token("]");
        if block {
            *val = v_block(ls, tp.clone(), "Vector");
        } else {
            *val = Value::Insert(ls);
        }
        tp
    }

    /// Parse comma-separated vector items inside `[...]`, returning an early error type on failure.
    pub(crate) fn collect_vector_items(
        &mut self,
        elm: u16,
        in_t: &mut Type,
        res: &mut Vec<Value>,
    ) -> Option<Type> {
        loop {
            if let Some(value) = self.parse_item(elm, in_t, res) {
                return Some(value);
            }
            if self.lexer.has_token(";")
                && let Some(value) = self.parse_multiply(res)
            {
                return Some(value);
            }
            if !self.lexer.has_token(",") {
                break;
            }
            if self.lexer.peek_token("]") {
                break;
            }
        }
        None
    }

    /// Build the instruction list for a parsed vector literal; returns `(tp, ls)`.
    #[allow(clippy::too_many_arguments)] // parser helper threading IR-construction params alongside &mut self; no sensible grouping reduces the count
    pub(crate) fn build_vector_list(
        &mut self,
        val: &mut Value,
        parent_tp: &Type,
        elm: u16,
        vec: u16,
        res: &[Value],
        in_t: &Type,
        mut tp: Type,
        is_var: bool,
        is_field: bool,
    ) -> (Type, Vec<Value>) {
        let mut ls = Vec::new();
        // Only create a fresh database record here when the variable has no existing
        // one (dep is empty).  For `v += [...]` the variable already has a dep from
        // the initial `=` assignment; calling vector_db again would reset v to an
        // empty record and discard the existing elements.  create_vector handles
        // the `=` re-assignment case by calling vector_db unconditionally.
        if self.vars.tp(vec).depend().is_empty() {
            ls.extend(self.vector_db(in_t, vec));
        }
        ls.extend(self.new_record(val, parent_tp, elm, vec, res, in_t));
        if !self.first_pass
            && vec != u16::MAX
            && !self.vars.is_argument(vec)
            && self.vector_needs_db(vec, in_t, is_var)
        {
            let db = self.insert_new(vec, elm, in_t, &mut ls);
            self.vars.depend(vec, db);
            tp = tp.depending(db);
        } else if !is_field && !is_var && *val != Value::Null {
            ls.insert(0, v_set(vec, val.clone()));
        }
        if !is_var && !is_field {
            ls.push(Value::Var(vec));
            for d in self.vars.tp(vec).depend() {
                tp = tp.depending(d);
            }
        }
        (tp, ls)
    }

    pub(crate) fn vector_needs_db(&self, vec: u16, in_t: &Type, is_var: bool) -> bool {
        is_var
            && *in_t != Type::Void
            && self.vars.tp(vec).depend().is_empty()
            && !matches!(self.vars.tp(vec), Type::RefVar(_))
            // Argument vectors already have a caller-provided backing store; do not
            // allocate a local __vdb_N store that would be freed before the return.
            && !self.vars.is_argument(vec)
    }

    pub(crate) fn unique_elm_var(&mut self, parent_tp: &Type, assign_tp: &Type, vec: u16) -> u16 {
        let c_tp = parent_tp.content();
        let was = Type::Reference(
            if c_tp.is_unknown() {
                0
            } else {
                self.data.type_def_nr(&c_tp)
            },
            parent_tp.depend(),
        );
        let elm = self.create_unique(
            "elm",
            if let Type::Reference(_, _) = assign_tp {
                assign_tp
            } else {
                &was
            },
        );
        self.vars.depend(elm, vec);
        for on in parent_tp.depend() {
            self.vars.depend(elm, on);
        }
        elm
    }

    pub(crate) fn parse_multiply(&mut self, res: &mut Vec<Value>) -> Option<Type> {
        let mut code = Value::Null;
        let tp = self.parse_operators(&Type::Unknown(0), &mut code, &mut Type::Null, 0);
        if !matches!(tp, Type::Integer(_, _)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect a number as the object multiplier"
            );
            return Some(Type::Unknown(0));
        }
        res.push(Value::Return(Box::new(code)));
        None
    }

    // <item> ::== ['for' | <expr> ]
    pub(crate) fn parse_item(
        &mut self,
        elm: u16,
        in_t: &mut Type,
        res: &mut Vec<Value>,
    ) -> Option<Type> {
        let mut p = Value::Var(elm);
        let mut t = if self.lexer.has_token("for") {
            //self.iter_for(&mut p)
            diagnostic!(
                self.lexer,
                Level::Error,
                "For inside a vector is not yet implemented"
            );
            return Some(Type::Unknown(0));
        } else {
            let mut parent_tp = Type::Null;
            self.parse_operators(&Type::Unknown(0), &mut p, &mut parent_tp, 0)
        };
        if let Type::Rewritten(tp) = in_t {
            *in_t = *tp.clone();
        }
        if let Type::Rewritten(tp) = t {
            t = *tp.clone();
        }
        if in_t.is_unknown() {
            *in_t = t.clone();
        }
        if t.is_unknown() {
            t = in_t.clone();
        }
        if let (Type::Reference(t_nr, _), Type::Reference(in_nr, _)) = (&t, &in_t.clone())
            && let (Type::Enum(t_e, true, _), Type::Enum(in_e, true, _)) = (
                &self.data.def(*t_nr).returned,
                &self.data.def(*in_nr).returned,
            )
            && *t_e == *in_e
        {
            *in_t = Type::Enum(*t_e, true, Vec::new());
        } else if !self.convert(&mut p, &t, in_t) {
            // double conversion check: can't become in_t or vice versa
            if self.convert(&mut p, in_t, &t) {
                *in_t = t.clone();
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "No common type {} for vector {}",
                    t.name(&self.data),
                    in_t.name(&self.data)
                );
            }
        }
        if let Type::Enum(td_nr, true, _) = t
            && let Value::Enum(enum_nr, _) = &p
            && self.lexer.peek_token("{")
        {
            let mut ls = Vec::new();
            self.parse_enum_field(&mut ls, Value::Var(elm), td_nr, 0, *enum_nr);
            ls.push(p.clone());
            p = Value::Insert(ls);
        }
        res.push(p.clone());
        None
    }

    pub(crate) fn is_field(&self, val: &Value) -> bool {
        if let Value::Call(o, _) = *val {
            o == self.data.def_nr("OpGetField")
        } else {
            false
        }
    }

    pub(crate) fn new_record_field_op(&mut self, val: &Value, parent_tp: &Type, op: &str) -> Value {
        if let Value::Call(_, ps) = val {
            let parent = self.data.def(self.data.type_def_nr(parent_tp)).known_type;
            let field_nr = if let Value::Int(pos) = ps[1] {
                self.database.field_nr(parent, pos)
            } else {
                0
            };
            if op == "OpNewRecord" {
                self.cl(
                    "OpNewRecord",
                    &[
                        ps[0].clone(),
                        Value::Int(i32::from(parent)),
                        Value::Int(i32::from(field_nr)),
                    ],
                )
            } else {
                self.cl(
                    "OpFinishRecord",
                    &[
                        ps[0].clone(),
                        Value::Var(0), // placeholder, caller replaces with Value::Var(elm)
                        Value::Int(i32::from(parent)),
                        Value::Int(i32::from(field_nr)),
                    ],
                )
            }
        } else {
            Value::Null
        }
    }

    pub(crate) fn new_record(
        &mut self,
        val: &mut Value,
        parent_tp: &Type,
        elm: u16,
        vec: u16,
        res: &[Value],
        in_t: &Type,
    ) -> Vec<Value> {
        let mut ls = Vec::new();
        let is_field = self.is_field(val);
        let ed_nr = self.data.type_def_nr(in_t);
        assert_ne!(
            ed_nr,
            u32::MAX,
            "Unknown type {} at {}",
            in_t.name(&self.data),
            self.lexer.pos()
        );
        for p in res {
            let known = Value::Int(i32::from(
                if ed_nr == u32::from(u16::MAX) || self.data.def(ed_nr).known_type == u16::MAX {
                    0
                } else {
                    self.database.vector(self.data.def(ed_nr).known_type)
                },
            ));
            if let Value::Return(multiply) = p {
                let to = if let Value::Call(_, ps) = val {
                    ps[0].clone()
                } else {
                    Value::Var(vec)
                };
                ls.push(self.cl("OpAppendCopy", &[to, *multiply.clone(), known]));
                continue;
            }
            let fld = Value::Int(i32::from(u16::MAX));
            let app_v = if is_field {
                self.new_record_field_op(val, parent_tp, "OpNewRecord")
            } else {
                self.cl(
                    "OpNewRecord",
                    &[Value::Var(vec), known.clone(), fld.clone()],
                )
            };
            ls.push(v_set(elm, app_v));
            if let Type::Reference(inner_nr, _) = in_t {
                if let Value::Insert(steps) = p {
                    // Inline struct initialization: the steps already write fields into elm.
                    for l in steps {
                        ls.push(l.clone());
                    }
                } else {
                    // Source is a variable, field access, or function call — the struct bytes
                    // must be explicitly copied into the new element slot.
                    let type_nr = if self.first_pass {
                        Value::Int(i32::from(u16::MAX))
                    } else {
                        Value::Int(i32::from(self.data.def(*inner_nr).known_type))
                    };
                    ls.push(self.cl("OpCopyRecord", &[p.clone(), Value::Var(elm), type_nr]));
                }
            } else if let Value::Insert(steps) = p {
                for l in steps {
                    ls.push(l.clone());
                }
            } else {
                ls.push(self.set_field(ed_nr, usize::MAX, 0, Value::Var(elm), p.clone()));
            }
            let finish = if is_field {
                let mut finish_v = self.new_record_field_op(val, parent_tp, "OpFinishRecord");
                // Replace placeholder Var(0) with the actual elm variable.
                if let Value::Call(_, ref mut args) = finish_v
                    && args.len() >= 2
                {
                    args[1] = Value::Var(elm);
                }
                finish_v
            } else {
                self.cl(
                    "OpFinishRecord",
                    &[Value::Var(vec), Value::Var(elm), known, fld],
                )
            };
            ls.push(finish);
        }
        ls
    }

    pub(crate) fn vector_db(&mut self, assign_tp: &Type, vec: u16) -> Vec<Value> {
        if self.first_pass || vec == u16::MAX || self.vars.is_argument(vec) {
            Vec::new()
        } else {
            let mut ls = Vec::new();
            let vec_def = self.data.vector_def(&mut self.lexer, assign_tp);
            let db = self
                .vars
                .work_vec_db(&Type::Reference(vec_def, Vec::new()), &mut self.lexer);
            self.vars.depend(vec, db);
            let tp = self.data.def(vec_def).known_type;
            debug_assert_ne!(
                tp,
                u16::MAX,
                "Undefined type {} at {}",
                self.data.def(vec_def).name,
                self.lexer.pos()
            );
            ls.push(self.cl("OpDatabase", &[Value::Var(db), Value::Int(i32::from(tp))]));
            // Reference to the vector field.
            ls.push(v_set(vec, self.get_field(vec_def, 0, Value::Var(db))));
            // Write 0 into this reference.
            ls.push(self.set_field(vec_def, 0, 0, Value::Var(db), Value::Int(0)));
            ls
        }
    }

    pub(crate) fn insert_new(
        &mut self,
        vec: u16,
        elm: u16,
        in_t: &Type,
        ls: &mut Vec<Value>,
    ) -> u16 {
        // determine the element size by the resulting type
        let vec_def = self.data.vector_def(&mut self.lexer, in_t);
        // Use work_vec_db (separate __vdb_N counter) so that these calls do NOT
        // consume __ref_N counter slots.  Both vector_db and insert_new contribute
        // to the __vdb_N namespace; at any given vector site exactly one of them
        // runs per pass (vector_db is guarded by !first_pass; insert_new is called
        // on first pass when vector_db has not yet created a dep, but on second pass
        // vector_needs_db returns false after vector_db ran, so insert_new is
        // skipped).  The __ref_N counter is reserved exclusively for add_defaults
        // and other return-value work-refs, ensuring ref_return can match the same
        // name across both passes.
        let db = self
            .vars
            .work_vec_db(&Type::Reference(vec_def, Vec::new()), &mut self.lexer);
        self.vars.depend(elm, db);
        self.vars.depend(vec, db);
        let known = Value::Int(i32::from(self.data.def(vec_def).known_type));
        ls.insert(0, self.cl("OpDatabase", &[Value::Var(db), known]));
        // Reference to the vector field.
        ls.insert(1, v_set(vec, self.get_field(vec_def, 0, Value::Var(db))));
        // Write 0 into this reference.
        ls.insert(
            2,
            self.set_field(vec_def, 0, 0, Value::Var(db), Value::Int(0)),
        );
        db
    }

    pub(crate) fn type_info(&self, in_t: &Type) -> Value {
        Value::Int(i32::from(self.get_type(in_t)))
    }

    pub(crate) fn get_type(&self, in_t: &Type) -> u16 {
        if self.first_pass {
            return u16::MAX;
        }
        match in_t {
            Type::Integer(min, _) => match in_t.size(false) {
                1 if *min == 0 => self.database.name("byte"),
                1 => self.database.name(&format!("byte<{min},false>")),
                2 => self.database.name(&format!("short<{min},false>")),
                _ => self.database.name("integer"),
            },
            Type::Character => self.database.name("integer"),
            Type::Long => self.database.name("long"),
            Type::Float => self.database.name("float"),
            Type::Single => self.database.name("single"),
            Type::Text(_) => self.database.name("text"),
            Type::Reference(r, _) | Type::Enum(r, _, _) => self.data.def(*r).known_type,
            Type::Hash(tp, key, _) => {
                let mut name = "hash<".to_string() + &self.data.def(*tp).name + "[";
                self.database
                    .field_name(self.data.def(*tp).known_type, key, &mut name);
                self.database.name(&name)
            }
            Type::Sorted(tp, key, _) => {
                let mut name = "sorted<".to_string() + &self.data.def(*tp).name + "[";
                field_id(key, &mut name);
                let r = self.database.name(&name);
                if r == u16::MAX {
                    name = "ordered<".to_string() + &self.data.def(*tp).name + "[";
                    field_id(key, &mut name);
                }
                self.database.name(&name)
            }
            Type::Index(tp, key, _) => {
                let mut name = "index<".to_string() + &self.data.def(*tp).name + "[";
                field_id(key, &mut name);
                let r = self.database.name(&name);
                if r == u16::MAX {
                    name = "index<".to_string() + &self.data.def(*tp).name + "[";
                    field_id(key, &mut name);
                }
                self.database.name(&name)
            }
            Type::Vector(tp, _) => {
                let elem_tp = self.get_type(tp);
                let vec_name = if elem_tp == u16::MAX {
                    "vector".to_string()
                } else {
                    format!("vector<{}>", self.database.types[elem_tp as usize].name)
                };
                self.database.name(&vec_name)
            }
            _ => u16::MAX,
        }
    }

    // <children> ::=
    #[allow(clippy::too_many_lines)]
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
                    let vec_val = code.clone();
                    let mut list = vec![vec_val];
                    let mut types = vec![t.clone()];
                    let mut m_arg_idx = 1usize;
                    loop {
                        // S10: infer lambda hint from vector element type.
                        if let Type::Vector(elm, _) = &t {
                            let elem = *elm.clone();
                            let hint = match (field.as_str(), m_arg_idx) {
                                ("map", 1) => {
                                    Some(Type::Function(vec![elem.clone()], Box::new(elem)))
                                }
                                ("filter", 1) => {
                                    Some(Type::Function(vec![elem], Box::new(Type::Boolean)))
                                }
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
                    return match field.as_str() {
                        "map" => self.parse_map(code, &list, &types),
                        "filter" => self.parse_filter(code, &list, &types),
                        "reduce" => self.parse_reduce(code, &list, &types),
                        _ => unreachable!(),
                    };
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
            let mut new = self.data.attr_value(dnr, fnr);
            if let Value::Call(_, args) = &mut new {
                args[0] = code.clone();
            }
            *code = new;
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

    // <var> ::= <object> | [ <call> | <var> | <enum> ] <children> }
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
                t = Type::Integer(0, 65536);
                *code = Value::Int(i32::from(tp));
            } else {
                t = self.parse_call(code, source, &nm);
            }
        } else if self.vars.name_exists(name) {
            let index_var = self.vars.var(name);
            if self.lexer.has_token("#") {
                self.var_usages(index_var, true);
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
        } else if self.data.def_nr(name) != u32::MAX {
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
            // S11: try resolving as a bare function reference.
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
                t = Type::Function(arg_types, Box::new(ret_type));
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
            Type::Integer(min, _) => match t.size(false) {
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
                && self.lexer.peek_token("{")
            {
                let tp = self.parse_object(d_nr, code);
                if tp != Type::Unknown(0) {
                    return tp;
                }
            } else if self.data.def_type(d_nr) == DefType::Constant {
                *code = self.data.def(d_nr).code.clone();
                return self.data.def(d_nr).returned.clone();
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
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown variable '{}'",
                    self.vars.name(*nr)
                );
            }
        }
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
    pub(crate) fn parse_in_range_body(
        &mut self,
        expr: &mut Value,
        data: &Value,
        name: &str,
        in_type: Type,
        reverse: bool,
    ) -> Type {
        let incl = self.lexer.has_token("=");
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
                // rev() wrapping a collection (not a range): set the reverse-iterator flag so
                // that fill_iter adds bit 64 into the OpIterate/OpStep `on` byte.
                if matches!(in_type, Type::Sorted(_, _, _) | Type::Index(_, _, _)) {
                    self.reverse_iterator = true;
                } else if !matches!(in_type, Type::Null) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "rev() on a non-range expression must wrap a sorted or index collection"
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
            // L6.2: emit all field constraint checks after construction completes.
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
            if found_fields.contains(&nm) || matches!(tp, Type::Routine(_)) {
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
            list.push(value.clone());
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

    // <if> ::= <expression> '{' <block> [ 'else' ( 'if' <if> | '{' <block> ) ]
}

#[cfg(test)]
mod tests {
    use super::inline_ref_set_in;
    use crate::data::{Block, Type, Value};

    /// S2: `inline_ref_set_in` must return false conservatively when nesting exceeds the limit.
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
