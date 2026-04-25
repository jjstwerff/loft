// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    Context, I32, Level, LexItem, OutputState, Parser, Parts, Type, Value, diagnostic_format,
    v_block, v_if, v_loop, v_set, var_size,
};

impl Parser {
    pub(crate) fn iter_text(
        &mut self,
        code: &mut Value,
        iter_var: u16,
        pre_var: Option<u16>,
    ) -> Value {
        // iter_var is {id}#next — the post-advance byte position (loop driver).
        // pre_var  is {id}#index — saved to the start position of the current char.
        let index_var = pre_var.unwrap();
        let res_var = self
            .vars
            .unique("for_result", &Type::Character, &mut self.lexer);
        let l = self.cl("OpLengthCharacter", &[Value::Var(res_var)]);
        let next = vec![
            // Save current position as #index before advancing.
            v_set(index_var, Value::Var(iter_var)),
            v_set(
                res_var,
                self.cl("OpTextCharacter", &[code.clone(), Value::Var(iter_var)]),
            ),
            v_set(iter_var, self.cl("OpAddInt", &[Value::Var(iter_var), l])),
            Value::Var(res_var),
        ];
        // Initialise the loop driver at the outer scope.
        // The caller must separately initialise index_var at the same scope level.
        *code = v_set(iter_var, Value::Int(0));
        v_block(next, Type::Character, "for text next")
    }

    #[allow(clippy::too_many_lines)] // sorted/index/spacial iterator setup — splitting would lose context
    pub(crate) fn iterator(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
        iter_var: u16,
        pre_var: Option<u16>,
    ) -> Value {
        // unwrap &vector<T> / &sorted<T> so the iterator setup
        // matches the underlying collection type.
        if let Type::RefVar(inner) = is_type {
            return self.iterator(code, inner, should, iter_var, pre_var);
        }
        if let Value::Iter(_, start, next, _) = code.clone() {
            if matches!(*next, Value::Block(_)) {
                *code = *start;
                return *next.clone();
            }
            diagnostic!(self.lexer, Level::Error, "Malformed iterator expression");
            return Value::Null;
        }
        if matches!(*is_type, Type::Text(_)) {
            return self.iter_text(code, iter_var, pre_var);
        }
        // CO1.5a: coroutine iterators (from generator function calls) need
        // a next()-based advance. Detect: the call target returns Iterator.
        if let Type::Iterator(inner, _) = is_type
            && !self.first_pass
            && let Value::Call(d_nr, _) = code
            && matches!(self.data.def(*d_nr).returned, Type::Iterator(_, _))
        {
            let gen_var = self.create_unique("__gen", is_type);
            self.vars.defined(gen_var);
            let gen_expr = code.clone();
            *code = v_set(gen_var, gen_expr);
            let op = self.data.def_nr("OpCoroutineNext");
            let yield_tp = (**inner).clone();
            let value_size = crate::variables::size(&yield_tp, &crate::data::Context::Argument);
            return Value::Call(
                op,
                vec![Value::Var(gen_var), Value::Int(i32::from(value_size))],
            );
        }
        if is_type == should {
            // Non-coroutine pre-existing iterator (sorted/hash/index).
            let orig = code.clone();
            *code = Value::Null;
            return orig;
        }
        if self.first_pass {
            self.reverse_iterator = false;
            return Value::Null;
        }
        if let Type::Iterator(_, _) = should {
            match is_type {
                Type::Vector(vtp, dep) => {
                    let i = Value::Var(iter_var);
                    let vec_tp = self.data.type_def_nr(vtp);
                    let db_tp = self.data.def(vec_tp).known_type;
                    // narrow vector element iteration uses
                    // the forced_size stride so the generated
                    // `vector::get_vector(size, idx)` matches the actual
                    // 1/2/4-byte storage.  Without this, `database.size(db_tp)`
                    // returns 8 (plain `integer`) and reads stray across
                    // element boundaries.
                    let size = if self.database.is_linked(db_tp) {
                        4
                    } else if let Type::Integer(spec) = &**vtp
                        && let Some(n) = spec.vector_narrow_width()
                    {
                        u16::from(n)
                    } else {
                        self.database.size(db_tp)
                    };
                    let mut ref_expr = self.cl(
                        "OpGetVector",
                        &[code.clone(), Value::Int(i32::from(size)), i.clone()],
                    );
                    if let Type::Reference(_, _) = *vtp.clone() {
                        if self.database.is_linked(db_tp) {
                            ref_expr = self.cl("OpVectorRef", &[code.clone(), i.clone()]);
                        }
                    } else {
                        // route through `get_val` with the full
                        // element Type — preserves `IntegerSpec.forced_size`
                        // so narrow vectors dispatch to `OpGetShortRaw` /
                        // `OpGetByte` / `OpGetInt4` via the narrow_vec
                        // split.  Previously via `get_field(vec_tp, MAX)`
                        // which looked up `def(integer).returned` and lost
                        // the forced_size → emitted `OpGetInt` (8 bytes)
                        // into a 2-byte slot, producing off-bytes reads.
                        ref_expr = self.get_val(vtp, false, 0, ref_expr, u32::MAX);
                    }
                    let mut tp = *vtp.clone();
                    for d in dep {
                        tp = tp.depending(*d);
                    }
                    let reverse = self.reverse_iterator;
                    let step = if reverse {
                        // Decrement, but clamp at i32::MIN to prevent negative-index wrap.
                        // When iter reaches -1, set it to i32::MIN so GetVector returns null.
                        let decremented = self.op("Min", i.clone(), Value::Int(1), I32.clone());
                        let cond = self.op("Le", Value::Int(1), i.clone(), I32.clone());
                        v_block(
                            vec![Value::If(
                                Box::new(cond),
                                Box::new(decremented),
                                Box::new(Value::Int(i32::MIN)),
                            )],
                            I32.clone(),
                            "rev step",
                        )
                    } else {
                        self.op("Add", i.clone(), Value::Int(1), I32.clone())
                    };
                    let next = v_block(
                        vec![v_set(iter_var, step), ref_expr],
                        *vtp.clone(),
                        "iter next",
                    );
                    self.vars
                        .set_loop(0, self.data.def(vec_tp).known_type, code);
                    if reverse {
                        // Start at length; the first step gives len-1 (last element).
                        *code = v_set(
                            iter_var,
                            self.cl("OpLengthVector", std::slice::from_ref(code)),
                        );
                    } else {
                        *code = v_set(iter_var, Value::Int(-1));
                    }
                    self.reverse_iterator = false;
                    return next;
                }
                Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Spacial(_, _, _) => {
                    // Derive element type for the block result annotation.
                    let elem_type = match is_type {
                        Type::Sorted(dnr, _, dep)
                        | Type::Index(dnr, _, dep)
                        | Type::Hash(dnr, _, dep)
                        | Type::Spacial(dnr, _, dep) => Type::Reference(*dnr, dep.clone()),
                        _ => Type::Null,
                    };
                    // Create a separate Long variable to hold the packed i64 iterator
                    // state (cur << 32 | finish).  iter_var ({id}#index) remains I32
                    // as the user-visible sequential loop counter.
                    // The state var is named "{loop_name}#iter_state" so that iter_op()
                    // can find it by name when generating #remove.
                    let iter_base = self.vars.name(iter_var);
                    let iter_state_name = format!(
                        "{}#iter_state",
                        iter_base.strip_suffix("#index").unwrap_or(iter_base)
                    );
                    let state_var = self.create_var(&iter_state_name, &crate::data::I64);
                    self.vars.defined(state_var);
                    let mut ls = Vec::new();
                    self.fill_iter(&mut ls, code, is_type, true, true);
                    ls.push(Value::Int(0));
                    ls.push(Value::Int(0));
                    let iter_expr = self.cl("OpIterate", &ls);
                    let mut ls = vec![Value::Var(state_var)];
                    self.fill_iter(&mut ls, code, is_type, false, true);
                    // Reset the reverse flag after both fill_iter calls so the second call
                    // also picks up the bit (fill_iter does not reset it itself).
                    self.reverse_iterator = false;
                    let next_expr = self.cl("OpStep", &ls);
                    let incr = self.op("Add", Value::Var(iter_var), Value::Int(1), I32.clone());
                    let iter_next = v_block(
                        vec![v_set(iter_var, incr), next_expr],
                        elem_type,
                        "sorted iter next",
                    );
                    // Use Insert (not v_block+Void) so that state_var and iter_var are
                    // claimed at the outer For-block scope and their stack slots persist
                    // for the duration of the loop.  A Void block would free them on exit.
                    *code = Value::Insert(vec![
                        v_set(state_var, iter_expr),
                        v_set(iter_var, Value::Int(-1)),
                    ]);
                    return iter_next;
                }
                _ => {
                    // I13: custom iterator protocol — check for fn next(&T) -> Item?
                    let next_d_nr = self.data.find_fn(u16::MAX, "next", is_type);
                    if next_d_nr != u32::MAX {
                        // Store the iterable in a variable so .next() has a stable target.
                        let iter_obj_var = self.create_unique("__iter_obj", is_type);
                        self.vars.defined(iter_obj_var);
                        let obj_expr = code.clone();
                        *code = v_set(iter_obj_var, obj_expr);
                        // The "next" expression is a method call: iter_obj.next()
                        let next_call = Value::Call(next_d_nr, vec![Value::Var(iter_obj_var)]);
                        return next_call;
                    }
                    if self.first_pass {
                        return Value::Null;
                    }
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown iterator type {}",
                        is_type.name(&self.data)
                    );
                }
            }
        }
        Value::Null
    }

    /// Convert a type to another type when possible
    /// Returns false when impossible. However, the other way round might still be possible.
    pub(crate) fn towards_set_hash_remove(
        &mut self,
        to: &Value,
        val: &Value,
        op: &str,
    ) -> Option<Value> {
        if !self.first_pass && *val == Value::Null && op == "=" {
            // Partial-key lookup produces an iteration (Value::Iter), not a single record.
            // Assigning null to an iteration has no defined semantics — require all key fields.
            if matches!(to, Value::Iter(..)) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot assign null to a partial-key lookup — \
                     provide all key fields to remove a single entry"
                );
                return Some(Value::Null);
            }
            if let Value::Call(get_nr, get_args) = to
                && self.data.def(*get_nr).name == "OpGetRecord"
                && let Some(Value::Int(db_tp_val)) = get_args.get(1)
                && (*db_tp_val as usize) < self.database.types.len()
                && matches!(
                    self.database.types[*db_tp_val as usize].parts,
                    Parts::Hash(_, _) | Parts::Index(_, _, _) | Parts::Sorted(_, _)
                )
            {
                let db_tp = *db_tp_val;
                let get_args = get_args.clone();
                let get_rec = self.cl("OpGetRecord", &get_args);
                return Some(self.cl(
                    "OpHashRemove",
                    &[get_args[0].clone(), get_rec, Value::Int(db_tp)],
                ));
            }
        }
        None
    }

    pub(crate) fn towards_set(
        &mut self,
        to: &Value,
        val: &Value,
        f_type: &Type,
        op: &str,
    ) -> Value {
        // Intercept `h[key] = null` → remove the key from hash/index/sorted
        if let Some(result) = self.towards_set_hash_remove(to, val, op) {
            return result;
        }
        if matches!(f_type, Type::Enum(_, true, _) | Type::Reference(_, _))
            && op == "="
            && !matches!(to, Value::Var(_))
        {
            return self.copy_ref(to, val, f_type);
        }
        if matches!(
            *f_type,
            Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Spacial(_, _, _)
        ) {
            if let Value::Var(nr) = to {
                if !self.first_pass && self.vars.is_const_param(*nr) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot modify {} '{}'; remove 'const' or use a local copy",
                        self.vars.const_kind(*nr),
                        self.vars.name(*nr)
                    );
                }
                return v_set(*nr, val.clone());
            }
            // LHS is a field access (e.g. `s.v = fresh`).  Pre-fix this
            // returned bare `val` and the assignment was silently discarded.
            // The full clear-then-append pair lives in parse_assign_op where
            // the RHS type is in scope (so we can avoid emitting OpAppendVector
            // when the RHS is not actually a vector — e.g. `b.data = f#read(...)`
            // where f#read returns text — which would mismatch types in
            // codegen).  Empty literal `[]` is also handled there.
            return val.clone();
        }
        if let Type::RefVar(tp) = f_type
            && matches!(**tp, Type::Vector(_, _) | Type::Sorted(_, _, _))
        {
            if let Value::Var(nr) = to {
                if self.vars.uses(*nr) > 0 {
                    return val.clone();
                }
            } else {
                return val.clone();
            }
        }
        if *f_type == Type::Boolean
            && let Value::Call(_, a) = &to
            && let Value::Call(_, args) = &a[0]
        {
            let conv = Value::If(
                Box::new(val.clone()),
                Box::new(Value::Int(1)),
                Box::new(Value::Int(0)),
            );
            return self.cl(
                "OpSetByte",
                &[args[0].clone(), args[1].clone(), args[2].clone(), conv],
            );
        }
        let code = self.compute_op_code(op, to, val, f_type);
        if let Value::Call(d_nr, args) = &to {
            let name = self.data.def(*d_nr).name.clone();
            let args = args.clone();
            self.call_to_set_op(&name, &args, code, op)
        } else if let Value::Var(nr) = to {
            if !self.first_pass && self.vars.is_const_param(*nr) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot modify {} '{}'; remove 'const' or use a local copy",
                    self.vars.const_kind(*nr),
                    self.vars.name(*nr)
                );
            }
            // This variable was created here and thus not yet used.
            self.var_usages(*nr, false);
            v_set(*nr, code)
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Not implemented operation {op} for type {}",
                    f_type.show(&self.data, &self.vars)
                );
            }
            Value::Null
        }
    }

    /// Compute the RHS value after applying `op` to `to` and `val`.
    pub(crate) fn iter_op_count_or_first(
        &mut self,
        code: &mut Value,
        name: &str,
        t: &mut Type,
        is_first: bool,
    ) {
        let count_var = format!("{name}#count");
        let count = if self.vars.name_exists(&count_var) {
            self.vars.var(&count_var)
        } else {
            self.create_var(&count_var, &I32)
        };
        self.vars.loop_count(count);
        self.vars.defined(count);
        if is_first {
            *code = self.cl("OpEqInt", &[Value::Var(count), Value::Int(0)]);
            *t = Type::Boolean;
        } else {
            *code = Value::Var(count);
            *t = I32.clone();
        }
    }

    #[allow(clippy::too_many_lines)] // iterator operation dispatch — splitting would lose context
    pub(crate) fn iter_op(&mut self, code: &mut Value, name: &str, t: &mut Type, index_var: u16) {
        // File variables handle their own # operations before iterator operations.
        if self.is_file_var(index_var) {
            self.file_op(code, t, index_var);
            return;
        }
        // detect #fields for compile-time field iteration.
        if self.lexer.has_keyword("fields") {
            let var = self.vars.var(name);
            let var_type = if var == u16::MAX {
                Type::Unknown(0)
            } else {
                self.vars.tp(var).clone()
            };
            if let Type::Reference(d, _) = &var_type {
                self.fields_of = *d;
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "#fields requires a struct variable, got {}",
                    var_type.name(&self.data)
                );
            }
            // Set code to the source variable so parse_field_iteration receives it.
            if var != u16::MAX {
                *code = Value::Var(var);
            }
            *t = Type::Void;
            return;
        }
        if self.lexer.has_keyword("index") {
            // For index<T> collections, {name}#index holds an internal B-tree record number,
            // not a sequential 0-based counter.  Reject it at compile time.
            if self.vars.loop_on(index_var) & 63 == 1 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "#index is not supported on index<T> collections \
(it holds an internal record number, not a sequential counter); \
use #count instead"
                );
                *t = Type::Unknown(0);
            } else {
                let i_name = &format!("{name}#index");
                if self.vars.name_exists(i_name) {
                    let v = self.vars.var(i_name);
                    *t = self.vars.tp(v).clone();
                    *code = Value::Var(v);
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Incorrect #index variable on {}",
                        name
                    );
                    *t = Type::Unknown(0);
                }
            }
        } else if self.lexer.has_keyword("next") {
            let n_name = format!("{name}#next");
            if self.vars.name_exists(&n_name) {
                let v = self.vars.var(&n_name);
                *t = self.vars.tp(v).clone();
                *code = Value::Var(v);
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Incorrect #next variable on {} (only valid in text loops)",
                    name
                );
                *t = Type::Unknown(0);
            }
        } else if self.lexer.has_token("break") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot continue outside a loop");
            }
            *code = Value::Break(self.vars.loop_nr(name));
            *t = Type::Void;
        } else if self.lexer.has_token("continue") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot continue outside a loop");
            }
            *code = Value::Continue(self.vars.loop_nr(name));
            *t = Type::Void;
        } else if self.lexer.has_keyword("count") {
            self.iter_op_count_or_first(code, name, t, false);
        } else if self.lexer.has_keyword("first") {
            self.iter_op_count_or_first(code, name, t, true);
        } else if self.lexer.has_keyword("remove") {
            // CO1.5c: #remove on generator iterators is already rejected by the
            // loop_value == Null check below — coroutine for-loops never call set_loop.
            if !self.first_pass && *self.vars.loop_value(index_var) == Value::Null {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}#remove' is only valid on a loop iteration variable (e.g. 'for {} in collection {{ {}#remove }}')",
                    name,
                    name,
                    name
                );
                *t = Type::Void;
                return;
            }
            // C60 Step 9: reject #remove on hash iteration.  The parser
            // substitutes hash iteration with a scratch rec-nr vector
            // (see parse_for, the `{id}#hash_scratch` variable), so
            // #remove would remove from the snapshot, not the hash —
            // silently diverging from the user's intent.
            if !self.first_pass {
                let coll = self.vars.loop_coll_var(index_var);
                if coll != u16::MAX && self.vars.name(coll).contains("hash_scratch") {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "#remove is not supported on hash iteration — the \
                         iterated vector is a sorted snapshot; use \
                         `hash[key] = null` to remove from the hash"
                    );
                    *t = Type::Void;
                    return;
                }
            }
            let on = self.vars.loop_on(index_var);
            let state_name = if on & 63 >= 1 && on & 63 <= 3 {
                let state_key = format!("{name}#iter_state");
                if self.vars.name_exists(&state_key) {
                    state_key
                } else {
                    format!("{name}#index")
                }
            } else {
                format!("{name}#index")
            };
            *code = self.cl(
                "OpRemove",
                &[
                    Value::Var(self.vars.var(&state_name)),
                    self.vars.loop_value(index_var).clone(),
                    Value::Int(i32::from(on)),
                    Value::Int(i32::from(self.vars.loop_db_tp(index_var))),
                ],
            );
            *t = Type::Void;
        } else if self.lexer.has_keyword("lock") {
            // d#lock — read the lock state of the store containing a reference or vector variable.
            // Assignment d#lock = true/false is resolved in towards_set.
            if !self.first_pass
                && !matches!(
                    self.vars.tp(index_var),
                    Type::Reference(_, _) | Type::Vector(_, _)
                )
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "#lock is only valid on reference or vector variables, not on '{}'",
                    name
                );
                *t = Type::Unknown(0);
            } else {
                *code = self.cl("n_get_store_lock", &[Value::Var(index_var)]);
                *t = Type::Boolean;
            }
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown loop attribute '#{name}'; use #index, #count, #first, #last, or #break"
            );
            *t = Type::Unknown(0);
        }
    }

    pub(crate) fn append_data_fp(state: OutputState, fmt: Value) -> (Value, Value, Value) {
        let mut a_width = state.width;
        let mut p_rec = Value::Int(-1); // -1 = no precision specified; 0 = :.0
        if let Value::Float(w) = a_width {
            let s = format!("{w}");
            let mut split = s.split('.');
            a_width = Value::Int(split.next().unwrap().parse::<i32>().unwrap());
            p_rec = Value::Int(split.next().unwrap().parse::<i32>().unwrap());
        }
        if state.float {
            p_rec = a_width;
            a_width = Value::Int(0);
        }
        (fmt, a_width, p_rec)
    }

    pub(crate) fn append_data_long(
        &mut self,
        list: &mut Vec<Value>,
        start: &str,
        var: Value,
        fmt: Value,
        state: OutputState,
    ) {
        list.push(self.cl(
            &(start.to_owned() + "Int"),
            &[
                var,
                fmt,
                Value::Int(state.radix),
                state.width,
                Value::Int(i32::from(state.token.as_bytes()[0])),
                Value::Boolean(state.plus),
                Value::Boolean(state.note),
                Value::Int(state.dir),
            ],
        ));
    }

    pub(crate) fn append_data_text(
        &mut self,
        list: &mut Vec<Value>,
        start: &str,
        var: Value,
        fmt: Value,
        state: OutputState,
    ) {
        list.push(self.cl(
            &(start.to_owned() + "Text"),
            &[
                var,
                fmt,
                state.width,
                Value::Int(state.dir),
                Value::Int(i32::from(state.token.as_bytes()[0])),
            ],
        ));
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn append_data(
        &mut self,
        tp: Type,
        list: &mut Vec<Value>,
        append: u16,
        append_value: u16,
        format: &Value,
        state: OutputState,
    ) {
        let var = Value::Var(append);
        let start = if matches!(self.vars.tp(append), Type::RefVar(_)) {
            "OpFormatStack"
        } else {
            "OpFormat"
        };
        // L9: escalate format-specifier mismatches to compile errors.
        // A specifier that can never have any effect on the value type is always a bug.
        if !self.first_pass {
            let is_text = matches!(tp, Type::Text(_));
            let is_bool = matches!(tp, Type::Boolean);
            if state.radix != 10 && (is_text || is_bool) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Format specifier has no effect on {}",
                    tp.name(&self.data)
                );
            } else if is_text && state.token == "0" && state.width != Value::Int(0) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Zero-padding has no effect on text"
                );
            }
        }
        match tp {
            Type::Integer(_) => {
                self.append_data_long(list, start, var, format.clone(), state);
            }
            Type::Boolean => {
                let value = self.cl("OpCastTextFromBool", std::slice::from_ref(format));
                self.append_data_text(list, start, var, value, state);
            }
            Type::Text(_) => {
                self.append_data_text(list, start, var, format.clone(), state);
            }
            Type::Character => {
                list.push(self.cl("OpAppendCharacter", &[var, format.clone()]));
            }
            Type::Float => {
                let dir = Value::Int(state.dir);
                let (fmt, a_width, p_rec) = Self::append_data_fp(state, format.clone());
                list.push(self.cl(
                    &(start.to_owned() + "Float"),
                    &[var, fmt, a_width, p_rec, dir],
                ));
            }
            Type::Single => {
                let dir = Value::Int(state.dir);
                let (fmt, a_width, p_rec) = Self::append_data_fp(state, format.clone());
                list.push(self.cl(
                    &(start.to_owned() + "Single"),
                    &[var, fmt, a_width, p_rec, dir],
                ));
            }
            Type::Vector(cont, _) => {
                let fmt = format.clone();
                let d_nr = self.data.type_def_nr(&cont);
                let db_tp = self.data.def(d_nr).known_type;
                let vec_tp = if db_tp == u16::MAX {
                    0
                } else {
                    let v = self.database.vector(db_tp);
                    self.data.check_vector(d_nr, v, self.lexer.pos());
                    v
                };
                list.push(self.cl(
                    &(start.to_owned() + "Database"),
                    &[
                        var,
                        fmt,
                        Value::Int(i32::from(vec_tp)),
                        Value::Int(state.db_format()),
                    ],
                ));
            }
            Type::Iterator(vtp, _) => {
                self.append_iter(list, append, append_value, vtp.as_ref(), format, state);
            }
            Type::Reference(d_nr, _) => {
                let fmt = format.clone();
                let db_tp = self.data.def(d_nr).known_type;
                list.push(self.cl(
                    &(start.to_owned() + "Database"),
                    &[
                        var,
                        fmt,
                        Value::Int(i32::from(db_tp)),
                        Value::Int(state.db_format()),
                    ],
                ));
            }
            Type::Enum(d_nr, is_ref, _) => {
                let fmt = format.clone();
                let e_tp = self.data.def(d_nr).known_type;
                if e_tp == u16::MAX || !is_ref {
                    let e_val = self.cl("OpCastTextFromEnum", &[fmt, Value::Int(i32::from(e_tp))]);
                    self.append_data_text(list, start, var, e_val, state);
                } else {
                    list.push(self.cl(
                        &(start.to_owned() + "Database"),
                        &[
                            var,
                            fmt,
                            Value::Int(i32::from(e_tp)),
                            Value::Int(state.db_format()),
                        ],
                    ));
                }
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot format type {}",
                        tp.name(&self.data)
                    );
                }
            }
        }
    }

    pub(crate) fn append_iter(
        &mut self,
        list: &mut Vec<Value>,
        append: u16,
        append_value: u16,
        var_type: &Type,
        value: &Value,
        state: OutputState,
    ) {
        if let Value::Iter(var, init, next, extra_init) = value
            && matches!(**next, Value::Block(_))
        {
            let count = if *var == u16::MAX {
                self.create_unique("count", &I32)
            } else {
                let count_name = format!("{}#count", self.vars.name(*var));
                let c = self.vars.var(&count_name);
                if c == u16::MAX {
                    self.create_var(&count_name, &I32)
                } else {
                    c
                }
            };
            list.push(self.cl("OpAppendText", &[Value::Var(append), Value::str("[")]));
            list.push(*init.clone());
            if !matches!(**extra_init, Value::Null) {
                list.push(*extra_init.clone());
            }
            list.push(v_set(count, Value::Int(0)));
            let mut append_var = append_value;
            if append_value == u16::MAX {
                append_var = self.create_unique("val", var_type);
            }
            let mut steps = Vec::new();
            steps.push(v_set(append_var, *next.clone()));
            steps.push(v_if(
                self.cl("OpLtInt", &[Value::Int(0), Value::Var(count)]),
                self.cl("OpAppendText", &[Value::Var(append), Value::str(",")]),
                Value::Null,
            ));
            steps.push(v_set(
                count,
                self.cl("OpAddInt", &[Value::Var(count), Value::Int(1)]),
            ));
            self.append_data(
                var_type.clone(),
                &mut steps,
                append,
                append_var,
                &Value::Var(append_var),
                state,
            );
            list.push(v_loop(steps, "Append Iter"));
            list.push(self.cl("OpAppendText", &[Value::Var(append), Value::str("]")]));
        }
    }

    // <object> ::= [ <identifier> ':' <expression> { ',' <identifier> ':' <expression> } ] '}'
    /// Parse a single `field: value` entry in an object literal.
    /// Returns `None` if parsing should abort (lexer reverted), `Some(false)` on unknown field,
    /// `Some(true)` on success.
    /// Parse a single `field: value` entry in an object literal.
    /// Returns false if no identifier found or `:` missing (caller handles revert).
    pub(crate) fn parse_for_iter_setup(
        &mut self,
        id: &str,
        in_type: &Type,
        expr: Value,
    ) -> (u16, Option<u16>, u16, Value, Value, Value) {
        let var_tp = self.for_type(in_type);
        // For text loops: {id}#next drives the loop; {id}#index is saved per-iteration.
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
        // error if the loop variable reuses a name with a different type.
        // Same-type reuse is idiomatic in loft (flat variable scoping).
        // `_` is exempt — it's the universal "unused" name and must work
        // across different element types within the same function.
        let existing_var = self.vars.var(id);
        if !self.first_pass
            && id != "_"
            && existing_var != u16::MAX
            && self.vars.is_defined(existing_var)
            && !self.vars.var_type(existing_var).is_same(&var_tp)
            && !self.vars.var_type(existing_var).is_unknown()
            // text_return converts text variables to RefVar(Text) work buffers
            // for the return path.  When a for-loop variable was converted this
            // way, the iterator still writes into it as text — this is correct
            // (the work buffer IS the variable) so suppress the mismatch.
            && !matches!(self.vars.var_type(existing_var), Type::RefVar(_))
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "loop variable '{id}' has type {} but was previously used as {}",
                var_tp.name(&self.data),
                self.vars.var_type(existing_var).name(&self.data)
            );
        }
        // C61: reject two classes of shadow that both produce silent wrong
        // values today:
        //   * Nested same-name loops (`for i { for i { } }`) — the inner
        //     iterator rewrites the outer's `#index` companion, detected
        //     on pass 2 via the active-loop chain.
        //   * Outer-local shadow (`x = 5; for x in …`) — the loop
        //     silently clobbers `x`; detected on pass 1 via the
        //     `was_loop_var` flag.  A plain local's slot has never served
        //     as a loop variable, so the prior binding is unambiguously a
        //     local.
        // Sequential same-name loops stay legal because the prior slot
        // carries `was_loop_var = true`.  `_` is exempt.
        if id != "_" && existing_var != u16::MAX {
            if !self.first_pass && self.vars.is_active_loop_var(existing_var) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "loop variable '{id}' shadows the enclosing loop's '{id}' — \
                     rename the inner loop variable (e.g. inner_{id}); loft does \
                     not support nested same-name loops"
                );
            } else if self.first_pass
                && !self.vars.was_loop_var(existing_var)
                && !self.vars.is_active_loop_var(existing_var)
                && !matches!(self.vars.var_type(existing_var), Type::RefVar(_))
                && (self.vars.var_type(existing_var).is_same(&var_tp)
                    || self.vars.var_type(existing_var).is_unknown())
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "loop variable '{id}' shadows a local named '{id}' — \
                     rename the loop variable (e.g. loop_{id}) or drop the \
                     outer `{id}` if it was a dead placeholder; loft does \
                     not block-scope loop variables"
                );
            }
        }
        let for_var = self.create_var(id, &var_tp);
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
        let iter_next = self.iterator(&mut create_iter, in_type, &it, iter_var, pre_var);
        (iter_var, pre_var, for_var, if_step, create_iter, iter_next)
    }

    pub(crate) fn parse_for(&mut self, code: &mut Value) {
        if let Some(id) = self.lexer.has_identifier() {
            self.lexer.token("in");
            let loop_nr = self.vars.start_loop();
            let mut expr = Value::Null;
            let mut in_type = self.parse_in_range(&mut expr, &Value::Null, &id);
            // if #fields was detected, take the compile-time unrolling path.
            if self.fields_of != u32::MAX {
                let struct_def_nr = self.fields_of;
                self.fields_of = u32::MAX;
                self.vars.finish_loop(loop_nr);
                self.parse_field_iteration(&id, struct_def_nr, &expr, code);
                return;
            }
            let mut fill = Value::Null;
            // For vector loops, the iterator runs on a unique temp copy so that the loop
            // variable does not alias the user-visible collection.  Record the original
            // variable number so that mutation of the original can be detected later.
            let orig_coll_var = if let Value::Var(v) = &expr {
                *v
            } else {
                u16::MAX
            };
            // Save the original collection expression before the vector temp-copy substitution
            // so that is_iterated_value() can match field-access patterns like `db.items`.
            let orig_coll_expr = expr.clone();
            // C60 piece 3 edit B (re-attempt with typed scratch): when
            // iterating a hash, substitute the collection expression
            // with a call to `hash_sorted(h, tp_id)` that builds a
            // u32-stride rec-nr scratch in the hash's own store
            // (edit A).  `in_type` stays Type::Hash so fill_iter hits
            // the Hash arm and emits on=3 (edit C); the empty-bounds
            // guard in iterate on=3 (edit E) handles unbounded.
            //
            // Key fix from prior segfault attempt: type the scratch
            // variable with the hash's actual content def-nr
            // (`Type::Reference(content, dep)`), not `Reference(0)`.
            // Downstream type-size + free-cleanup machinery reads
            // `self.data.def(content)`; passing 0 gave whatever
            // definition happens to sit at index 0 and corrupted
            // stack layout.
            if let Type::Hash(content, _, dep) = in_type.clone() {
                let scratch_tp = Type::Reference(content, dep.clone());
                let scratch_var = self.create_unique("hash_scratch", &scratch_tp);
                let hash_tp_id = self.get_type(&in_type);
                let tp_arg = if hash_tp_id == u16::MAX {
                    0
                } else {
                    i32::from(hash_tp_id)
                };
                let hash_sorted_fn = self.data.def_nr("n_hash_sorted");
                if hash_sorted_fn != u32::MAX {
                    let call = Value::Call(hash_sorted_fn, vec![expr.clone(), Value::Int(tp_arg)]);
                    fill = v_set(scratch_var, call);
                    expr = Value::Var(scratch_var);
                    if !self.first_pass {
                        self.vars.set_type(scratch_var, scratch_tp);
                    }
                }
            }
            if matches!(in_type, Type::Vector(_, _)) {
                let vec_var = self.create_unique("vector", &in_type);
                // On the second pass in_type may carry __vdb_N dependencies that
                // were not present on the first pass (vector_db only runs on pass 2).
                // Update the temp variable's type so that get_free_vars sees the
                // deps and does NOT emit OpFreeRef for the temp — the __vdb_N
                // variable at the outer scope owns the store and will free it.
                if !self.first_pass {
                    self.vars.set_type(vec_var, in_type.clone());
                }
                in_type = in_type.depending(vec_var);
                fill = v_set(vec_var, expr);
                expr = Value::Var(vec_var);
            }
            // Optional parallel clause: par(result=worker(elem), threads)
            if let LexItem::Identifier(kw) = &self.lexer.peek().has
                && kw == "par"
            {
                self.lexer.has_identifier(); // consume "par"
                self.parse_parallel_for_loop(code, &id, &in_type, &expr, fill, loop_nr);
                return;
            }
            // CO1.5: detect coroutine for-loop before parse_for_iter_setup consumes expr.
            let is_coroutine_loop = matches!(&in_type, Type::Iterator(_, _))
                && !self.first_pass
                && matches!(&expr, Value::Call(d, _) if matches!(self.data.def(*d).returned, Type::Iterator(_, _)));
            let (_iter_var, pre_var, for_var, if_step, create_iter, iter_next) =
                self.parse_for_iter_setup(&id, &in_type, expr);
            let var_tp = self.for_type(&in_type);
            // For vector loops: set_loop stores the temp-copy var; override with the
            // original so that `orig += elem` is correctly identified as a mutation.
            if matches!(in_type, Type::Vector(_, _)) {
                if orig_coll_var != u16::MAX {
                    self.vars.set_coll_var(orig_coll_var);
                }
                // Always restore the original collection expression so that
                // is_iterated_value() can match field-access forms like `db.items`.
                self.vars.set_coll_value(orig_coll_expr);
            }
            if !self.first_pass && iter_next == Value::Null {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Need an iterable expression in a for statement"
                );
                return;
            }
            let for_next = v_set(for_var, iter_next);
            self.vars.loop_var(for_var);
            let in_loop = self.in_loop;
            self.in_loop = true;
            let mut block = Value::Null;
            let loop_write_state = self.vars.save_and_clear_write_state();
            self.vars.clear_write_state();
            self.parse_block("for", &mut block, &Type::Void);
            self.vars.restore_write_state(&loop_write_state);
            let count = self.vars.loop_counter();
            self.in_loop = in_loop;
            self.vars.finish_loop(loop_nr);
            let mut for_steps = Vec::new();
            if fill != Value::Null {
                for_steps.push(fill);
            }
            // For text loops, initialise {id}#index at the FOR block scope so its live
            // interval covers the entire loop (not just the inner "for text next" block).
            if let Some(idx_var) = pre_var {
                for_steps.push(v_set(idx_var, Value::Int(0)));
            }
            for_steps.push(create_iter);
            let mut lp = vec![for_next];
            // CO1.5b: coroutine iterators also need the null-check termination.
            if !matches!(in_type, Type::Iterator(_, _)) || is_coroutine_loop {
                let mut test_for = Value::Var(for_var);
                self.convert(&mut test_for, &var_tp, &Type::Boolean);
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
            lp.push(block);
            if count != u16::MAX {
                for_steps.insert(0, v_set(count, Value::Int(0)));
                lp.push(v_set(
                    count,
                    self.cl("OpAddInt", &[Value::Var(count), Value::Int(1)]),
                ));
            }
            for_steps.push(v_loop(lp, "For loop"));
            *code = v_block(for_steps, Type::Void, "For block");
        } else {
            diagnostic!(self.lexer, Level::Error, "Expect variable after for");
        }
    }

    // Desugar `for a in vec par(b = worker(a), N) { body }` into an
    // index-based loop over the `parallel_for` result vector.
    pub(crate) fn parse_parallel_for_loop(
        &mut self,
        code: &mut Value,
        elem_var: &str,
        in_type: &Type,
        vec_expr: &Value,
        fill: Value,
        loop_nr: u16,
    ) {
        // Consume opening '('.
        self.lexer.token("(");

        // Validate: parallel syntax requires a vector input.
        let elem_tp = if let Type::Vector(_, _) = in_type {
            self.for_type(in_type)
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "par(...) requires a vector<T> input, not {}",
                    in_type.name(&self.data)
                );
            }
            self.skip_to_parallel_body();
            self.vars.finish_loop(loop_nr);
            return;
        };

        // Parse: result_name = worker_call , threads )
        let Some(result_name) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect result variable name after 'par('"
                );
            }
            self.skip_to_parallel_body();
            self.vars.finish_loop(loop_nr);
            return;
        };
        if !self.lexer.has_token("=") {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect '=' after result name '{}' in par(...)",
                    result_name
                );
            }
            self.skip_to_parallel_body();
            self.vars.finish_loop(loop_nr);
            return;
        }

        // Create the element variable so the worker call expression can resolve it.
        // (e.g. `calc(a)` needs `a` in scope during parsing even though the body
        // never runs `a` directly — the parallel map handles that.)
        //
        // Plan-04 B.3 follow-up: the body of
        // `for a in items par(b = worker(a), N) { ... a.iv ... }`
        // is parsed against this `elem_var_nr`, but the desugared
        // loop iterates over an `idx` counter and never writes `a` —
        // so the slot allocator would never place it.  `a` is treated
        // as an inline alias for `OpGetVector(items, idx)`, same as
        // `b` → `OpGetVector(results, idx)`.  build_parallel_for_ir
        // performs the actual Var→accessor rewrite after body parse,
        // once `idx_var` exists.
        let elem_var_nr = self.create_var(elem_var, &elem_tp);
        self.vars.defined(elem_var_nr);
        if matches!(elem_tp, Type::Integer(_)) {
            self.vars.in_use(elem_var_nr, true);
        }

        // Resolve worker function: consumes the worker call tokens up to the ','.
        let (fn_d_nr, ret_type, extra_vals, _extra_types) =
            self.parse_parallel_worker(elem_var, &elem_tp);

        // Plan-06 phase 5b proper — par-safety analyser hook lives
        // here.  The analyser (`scopes::is_par_safe` /
        // `scopes::par_unsafe_reason`) is implemented and unit-tested
        // (commits 227acc8, 63ad94d) but is NOT invoked at compile
        // time yet because today's stdlib annotation coverage is
        // ~15 fns out of ~150.  Calling is_par_safe on a worker
        // that legitimately uses unannotated stdlib (set_store_lock,
        // claim, etc. — internals invoked by compiler-generated
        // wrapper code) produces false-positive rejections that
        // would break every existing par() call site.
        //
        // Phase 5b' (after the 5a annotation sweep is comprehensive)
        // re-enables the diagnostic at Level::Error per DESIGN.md D8.
        // Until then the analyser is available via the public API
        // (e.g. for tooling / IDE plugins that want to lint par
        // bodies optimistically).
        let _ = fn_d_nr; // silence unused warning until 5b' enables the check

        // Comma separating worker from thread count.
        self.lexer.token(",");
        let mut threads_expr = Value::Null;
        self.expression(&mut threads_expr);
        // Closing ')'.
        self.lexer.token(")");

        // Compute element size from the return type.
        // return_size =  0 signals text mode to n_parallel_for.
        // return_size = -1 signals reference (struct) mode.
        let return_size: i32 = if matches!(&ret_type, Type::Text(_)) {
            0 // sentinel: text mode — workers collect Strings, main thread stores refs
        } else if matches!(&ret_type, Type::Reference(_, _) | Type::Enum(_, true, _)) {
            // Reference mode — workers return a DbRef into their own
            // store; main deep-copies via copy_from_worker.  Plan-06
            // phase 1 G1: struct-enum returns (Enum variants with
            // payload, e.g. `Verdict::Pass{score}`) are heap-typed
            // (`heap_def_nr().is_some()`) so they share the ref path
            // verbatim.  This closes the size-8 gate for variant payloads.
            -1
        } else {
            let sz = i32::from(var_size(&ret_type, &Context::Argument));
            if !self.first_pass && fn_d_nr != u32::MAX && (sz == 0 || sz > 8) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Parallel worker return type '{}' (size {sz}) is not supported",
                    ret_type.name(&self.data)
                );
            }
            sz.max(1) // fallback to 1 if unknown
        };
        // Use the actual inline element size from the database (e.g. 4 for Score{value:integer},
        // 8 for Range{lo,hi:integer}).  var_size() returns size_of::<DbRef>() for reference types,
        // which is wrong for inline vector element storage.
        let elem_size = {
            let elm_td = self.data.type_elm(&elem_tp);
            let known = self.data.def(elm_td).known_type;
            let db_size = i32::from(self.database.size(known));
            if db_size > 0 {
                db_size
            } else {
                i32::from(var_size(&elem_tp, &Context::Argument))
            }
        };

        self.build_parallel_for_ir(
            code,
            &result_name,
            fn_d_nr,
            &ret_type,
            elem_size,
            return_size,
            vec_expr,
            threads_expr,
            fill,
            loop_nr,
            extra_vals,
            elem_var_nr,
            &elem_tp,
        );
    }

    // parallel_for IR builder; threads unrelated IR params alongside &mut self — no sensible grouping
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn build_parallel_for_ir(
        &mut self,
        code: &mut Value,
        result_name: &str,
        fn_d_nr: u32,
        ret_type: &Type,
        elem_size: i32,
        return_size: i32,
        vec_expr: &Value,
        threads_expr: Value,
        fill: Value,
        loop_nr: u16,
        extra_args: Vec<Value>,
        elem_var: u16,
        elem_tp: &Type,
    ) {
        let ref_d_nr = self.data.def_nr("reference");
        let results_ref_type = Type::Reference(ref_d_nr, Vec::new());
        let par_for_d_nr = self.data.def_nr("n_parallel_for");

        // Create result-reference variable.
        let results_var = self.create_unique("par_results", &results_ref_type);
        self.vars.defined(results_var);

        // Create index variable (b#index).
        let idx_var = self.create_var(&format!("{result_name}#index"), &I32);
        self.vars.defined(idx_var);
        self.vars.in_use(idx_var, true);

        // Create length variable (par_len#N).
        let len_var = self.create_unique("par_len", &I32);
        self.vars.defined(len_var);
        self.vars.in_use(len_var, true);

        // Create the result element variable (b) with the worker's return type.
        // On the first pass fn_d_nr is u32::MAX; use Type::Unknown so that the second
        // pass can update the type to the correct one (Float, Boolean, etc.) via
        // add_variable's "update if unknown" logic.  Using I32 here caused the type
        // to stick as integer even when the worker returns float or boolean.
        let b_type = if matches!(ret_type, Type::Unknown(_)) {
            I32.clone()
        } else if fn_d_nr == u32::MAX {
            // First pass: placeholder — will be replaced on second pass.
            Type::Unknown(u32::MAX)
        } else if let Type::Text(_) = ret_type {
            // Strip worker-internal deps — they reference variables in the worker scope.
            Type::Text(Vec::new())
        } else {
            ret_type.clone()
        };
        // Plan-04 B.3 follow-up v2 (b3-par-inline.md): each par block gets
        // its OWN uniquely-named `b_var` (via `create_unique` → internal
        // name `_<result_name>_<counter>`), so two par blocks sharing the
        // user's loop-variable name can no longer collide on a single
        // `Function::variables` entry.  During body parsing the user's
        // name is aliased to this `b_var` via `set_name` (same mechanism
        // as match-arm field aliases in `control.rs:867`).  After the
        // body parses, every `Value::Var(b_var)` in the body is rewritten
        // to the element-accessor expression (see post-parse rewrite
        // below) — `b` becomes an inline alias rather than a runtime
        // slot, so there is no `Set(b_var, …)`, no `OpPut*`, and no
        // type-width mismatch to drift the stack.
        let b_var = self.create_unique(result_name, &b_type);
        self.vars.defined(b_var);
        let prior_name_target = self.vars.set_name(result_name, b_var);
        if matches!(b_type, Type::Integer(_) | Type::Unknown(_)) {
            self.vars.in_use(b_var, true);
        }

        // Parse the body block.
        self.vars.loop_var(b_var);
        let in_loop = self.in_loop;
        self.in_loop = true;
        // M11-a: flag that we are inside a par() body so that any `yield`
        // encountered during parsing can emit a compile-time error.
        let outer_par = self.in_par_body;
        self.in_par_body = true;
        let mut block = Value::Null;
        self.parse_block("parallel for", &mut block, &Type::Void);
        let count = self.vars.loop_counter();
        self.in_par_body = outer_par;
        self.in_loop = in_loop;
        self.vars.finish_loop(loop_nr);
        // Restore prior `result_name` alias (or remove ours if none).
        match prior_name_target {
            Some(nr) => {
                self.vars.set_name(result_name, nr);
            }
            None => self.vars.remove_name(result_name),
        }

        // Build IR only when we have a valid function reference.
        if fn_d_nr == u32::MAX || par_for_d_nr == u32::MAX {
            // Errors already reported; emit nothing useful.
            *code = Value::Null;
            return;
        }

        // A14.5/A14.6: auto-select light path for eligible workers.
        let is_primitive_return = !matches!(
            ret_type,
            Type::Text(_) | Type::Reference(_, _) | Type::Unknown(_)
        );
        let light_m = if is_primitive_return && fn_d_nr != u32::MAX {
            self.check_light_eligible(fn_d_nr)
        } else {
            None
        };
        let actual_par_d_nr = if light_m.is_some() {
            let d = self.data.def_nr("n_parallel_for_light");
            if d == u32::MAX { par_for_d_nr } else { d }
        } else {
            par_for_d_nr
        };

        // parallel_for(input, elem_size, return_size, threads, fn_d_nr, [pool_m], extra1, ..., n_extra)
        // n_extra is pushed LAST so it's on top of the stack for popping first.
        let n_extra = extra_args.len();
        let mut pf_args = vec![
            vec_expr.clone(),
            Value::Int(elem_size),
            Value::Int(return_size),
            threads_expr,
            Value::Int(fn_d_nr as i32),
        ];
        // pool_m is hardcoded in the native function (avoids stack-ordering complexity)
        let _ = light_m;
        pf_args.extend(extra_args);
        pf_args.push(Value::Int(n_extra as i32));
        let pf_call = Value::Call(actual_par_d_nr, pf_args);

        // len(input_vec) — compute once before the loop.
        let len_call = self.cl("OpLengthVector", std::slice::from_ref(vec_expr));

        let stop_cond = self.cl("OpLeInt", &[Value::Var(len_var), Value::Var(idx_var)]);
        let stop = v_if(
            stop_cond,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );

        // Use OpGetVector + get_field to extract the element from the result
        // vector. This works for all return types (int, long, float, bool, text)
        // without per-type getter functions.
        let result_elem_size = match return_size {
            0 => 4, // text: 4-byte string pointer per element
            -1 => {
                // reference: inline struct size from the database
                let ret_td = self.data.type_def_nr(ret_type);
                let known = self.data.def(ret_td).known_type;
                i32::from(self.database.size(known))
            }
            other => other,
        };
        let get_vec = self.cl(
            "OpGetVector",
            &[
                Value::Var(results_var),
                Value::Int(result_elem_size),
                Value::Var(idx_var),
            ],
        );
        let get_call = if matches!(ret_type, Type::Reference(_, _)) || fn_d_nr == u32::MAX {
            // fn_d_nr == u32::MAX: worker was rejected (e.g. S23 generator check);
            // skip the type-based field access to avoid crashing on Unknown type.
            get_vec
        } else {
            let vec_tp = self.data.type_def_nr(ret_type);
            if vec_tp == u32::MAX {
                // Unsupported return type (e.g. iterator<T> in first pass before S23
                // diagnostic fires): fall back to raw vector access to prevent crash.
                get_vec
            } else {
                self.get_field(vec_tp, usize::MAX, get_vec)
            }
        };
        // Plan-04 B.3 follow-up v2 (b3-par-inline.md): rewrite every
        // `Value::Var(b_var)` in the body with a clone of `get_call`.
        // `b` is no longer a runtime variable — each reference expands
        // inline to the accessor expression.  No `Set(b_var, get_call)`
        // is emitted; the body references ARE the reads.  Under the B.3
        // atomic bundle's slot-aware `OpPut*` dispatch this eliminates
        // the type-width mismatch and the stack drift.
        replace_var_in_ir(&mut block, b_var, &get_call);

        // apply the same inline-alias treatment to the outer
        // iterator variable `a`.  The desugared loop increments `idx`;
        // `a` is logically `items[idx]` on every iteration.  Rewriting
        // `Var(a)` → `OpGetVector(items, elem_size, idx)` (plus
        // `get_field` for non-Reference element types, mirroring the
        // `b` path) means `a` never needs a slot.  Without this the
        // allocator leaves `a` at `stack_pos == u16::MAX` and codegen
        // panics `Incorrect var a[65535] versus N`.
        let a_get_vec = self.cl(
            "OpGetVector",
            &[vec_expr.clone(), Value::Int(elem_size), Value::Var(idx_var)],
        );
        let a_accessor = if matches!(elem_tp, Type::Reference(_, _)) {
            a_get_vec
        } else {
            let elm_td = self.data.type_def_nr(elem_tp);
            if elm_td == u32::MAX {
                a_get_vec
            } else {
                self.get_field(elm_td, usize::MAX, a_get_vec)
            }
        };
        replace_var_in_ir(&mut block, elem_var, &a_accessor);
        let idx_inc = v_set(
            idx_var,
            self.cl("OpAddInt", &[Value::Var(idx_var), Value::Int(1)]),
        );

        let mut lp = vec![stop, block, idx_inc];
        if count != u16::MAX {
            lp.insert(
                3,
                v_set(
                    count,
                    self.cl("OpAddInt", &[Value::Var(count), Value::Int(1)]),
                ),
            );
        }

        let mut for_steps = Vec::new();
        if count != u16::MAX {
            for_steps.push(v_set(count, Value::Int(0)));
        }
        if fill != Value::Null {
            for_steps.push(fill);
        }
        for_steps.push(v_set(len_var, len_call));
        for_steps.push(v_set(results_var, pf_call));
        for_steps.push(v_set(idx_var, Value::Int(0)));
        for_steps.push(v_loop(lp, "Parallel for loop"));
        *code = v_block(for_steps, Type::Void, "Parallel for block");
    }

    // Consume the remaining `par(...)` tokens and then the body block so the
    // parser can recover after an error in the parallel clause.
    // Called after '(' has already been consumed, so this drains to ')'.
    /// Compiler special-case for `map(v: vector<T>, f: fn(T) -> U) -> vector<U>`.
    /// Generates inline bytecode equivalent to `[for elm in v { f(elm) }]`.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_map(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        let placeholder = Type::Vector(Box::new(Type::Unknown(0)), Vec::new());
        // On first pass, return the concrete output vector type derived from the function's
        // return type so that downstream variables (e.g. `r = map(...)`) get the right type
        // and subsequent `for x in r` iterations resolve correctly.
        // We must NOT create unique variables here — only determine the type.
        if self.first_pass {
            // On first pass, infer output element type from the input vector.
            // The lambda return type may not be fully resolved yet; defaulting
            // to the input element type is correct for most cases (e.g. x * 10)
            // and lets downstream code like r[0] type-check.
            if let Type::Vector(elm, _) = &types[0] {
                return Type::Vector(elm.clone(), Vec::new());
            }
            return placeholder;
        }
        if list.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map requires 2 arguments: map(vector, fn f)"
            );
            return placeholder;
        }
        let _in_elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: first argument must be a vector"
            );
            return placeholder;
        };
        let (fn_param_types, fn_ret_type) = if let Type::Function(params, ret, _) = &types[1] {
            (params.clone(), *ret.clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: second argument must be a function reference (use fn <name>)"
            );
            return placeholder;
        };
        if fn_param_types.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: function must take exactly one argument"
            );
            return placeholder;
        }
        // accept both static fn-refs (Value::Int) and fn-ref variables/lambdas.
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            Some(*d as u32)
        } else {
            None // fn-ref variable or lambda — will use CallRef
        };
        // For CallRef path, store the fn-ref value in a local variable.
        let fn_ref_var = if fn_d_nr.is_none() {
            let v = self.create_unique("map_fn", &types[1]);
            self.vars.defined(v);
            Some(v)
        } else {
            None
        };

        let mut in_type = types[0].clone();
        let vec_copy_var = self.create_unique("map_vec", &in_type);
        in_type = in_type.depending(vec_copy_var);

        let iter_var = self.create_unique("map_idx", &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique("map_elm", &var_tp);
        self.vars.defined(for_var);

        let out_elem = fn_ret_type.clone();
        let result_type = Type::Vector(Box::new(out_elem.clone()), Vec::new());
        let result_vec = self.create_unique("map_result", &result_type);
        let elm = self.unique_elm_var(&result_type, &out_elem, result_vec);

        let mut create_iter_code = Value::Var(vec_copy_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter_code, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut fill = v_set(vec_copy_var, list[0].clone());
        // for CallRef path, assign the fn-ref value before the loop.
        if let Some(fv) = fn_ref_var {
            fill = Value::Insert(vec![fill, v_set(fv, list[1].clone())]);
        }

        let body = if let Some(d) = fn_d_nr {
            Value::Call(d, vec![Value::Var(for_var)])
        } else {
            Value::CallRef(fn_ref_var.unwrap(), vec![Value::Var(for_var)])
        };

        self.data.vector_def(&mut self.lexer, &out_elem);

        let tp = result_type.clone();
        // Reset val so build_comprehension_code creates a fresh result vector rather than
        // pre-seeding it with the LHS variable (which would cause a self-reference panic).
        *val = Value::Null;
        self.build_comprehension_code(
            result_vec,
            &Value::Var(result_vec),
            elm,
            &out_elem,
            &in_type,
            &var_tp,
            for_var,
            for_next,
            None,
            fill,
            create_iter_code,
            Value::Null,
            body,
            val,
            false,
            false,
            true,
            tp,
        )
    }

    /// Validate `filter` arguments and extract `(in_elem_type, fn_d_nr)`.
    /// Returns `Err(placeholder)` on validation failure.
    pub(crate) fn parse_filter_validate(
        &mut self,
        list: &[Value],
        types: &[Type],
    ) -> Result<(Type, Option<u32>), Type> {
        let placeholder = Type::Vector(Box::new(Type::Unknown(0)), Vec::new());
        if list.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter requires 2 arguments: filter(vector, fn pred)"
            );
            return Err(placeholder);
        }
        let in_elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: first argument must be a vector"
            );
            return Err(placeholder);
        };
        let (fn_param_types, fn_ret_type) = if let Type::Function(params, ret, _) = &types[1] {
            (params.clone(), *ret.clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: second argument must be a function reference (use fn <name>)"
            );
            return Err(placeholder);
        };
        if fn_param_types.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: predicate must take exactly one argument"
            );
            return Err(placeholder);
        }
        if fn_ret_type != Type::Boolean {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: predicate must return boolean"
            );
            return Err(placeholder);
        }
        // accept both static fn-refs and fn-ref variables/lambdas.
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            Some(*d as u32)
        } else {
            None
        };
        Ok((in_elem_type, fn_d_nr))
    }

    pub(crate) fn parse_filter(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        let placeholder = Type::Vector(Box::new(Type::Unknown(0)), Vec::new());
        // On first pass, return the concrete output type from the input vector's element type.
        if self.first_pass {
            if !types.is_empty()
                && let Type::Vector(elm, _) = &types[0]
            {
                return Type::Vector(elm.clone(), Vec::new());
            }
            return placeholder;
        }
        let (in_elem_type, fn_d_nr) = match self.parse_filter_validate(list, types) {
            Ok(v) => v,
            Err(t) => return t,
        };
        // for CallRef path, store the fn-ref value in a local variable.
        let fn_ref_var = if fn_d_nr.is_none() {
            let v = self.create_unique("filter_fn", &types[1]);
            self.vars.defined(v);
            Some(v)
        } else {
            None
        };

        let mut in_type = types[0].clone();
        let vec_copy_var = self.create_unique("filter_vec", &in_type);
        in_type = in_type.depending(vec_copy_var);

        let iter_var = self.create_unique("filter_idx", &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique("filter_elm", &var_tp);
        self.vars.defined(for_var);

        let out_elem = in_elem_type.clone();
        let result_type = Type::Vector(Box::new(out_elem.clone()), Vec::new());
        let result_vec = self.create_unique("filter_result", &result_type);
        let elm = self.unique_elm_var(&result_type, &out_elem, result_vec);

        let mut create_iter_code = Value::Var(vec_copy_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter_code, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut fill = v_set(vec_copy_var, list[0].clone());
        if let Some(fv) = fn_ref_var {
            fill = Value::Insert(vec![fill, v_set(fv, list[1].clone())]);
        }

        let if_step = if let Some(d) = fn_d_nr {
            Value::Call(d, vec![Value::Var(for_var)])
        } else {
            Value::CallRef(fn_ref_var.unwrap(), vec![Value::Var(for_var)])
        };

        let body = Value::Var(for_var);

        self.data.vector_def(&mut self.lexer, &out_elem);

        let tp = result_type.clone();
        // Reset val so build_comprehension_code creates a fresh result vector.
        *val = Value::Null;
        self.build_comprehension_code(
            result_vec,
            &Value::Var(result_vec),
            elm,
            &out_elem,
            &in_type,
            &var_tp,
            for_var,
            for_next,
            None,
            fill,
            create_iter_code,
            if_step,
            body,
            val,
            false,
            false,
            true,
            tp,
        )
    }

    /// Build ops to construct a struct/struct-enum instance, replicating the IR that
    /// `parse_object` produces. Returns the ops list and the work variable holding the result.
    fn build_object_ops(&mut self, td_nr: u32, fields: &[(usize, Value)]) -> (Vec<Value>, u16) {
        let ret = self.data.def(td_nr).returned.clone();
        let w = self.vars.work_refs(&ret, &mut self.lexer);
        self.data.set_referenced(td_nr, self.context, Value::Null);
        let tp = i32::from(self.data.def(td_nr).known_type);
        let mut list: Vec<Value> = vec![
            v_set(w, Value::Null),
            self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp)]),
        ];
        for &(f_nr, ref val) in fields {
            list.push(self.set_field_no_check(td_nr, f_nr, 0, Value::Var(w), val.clone()));
        }
        (list, w)
    }

    /// Compile-time unroll `for f in s#fields` into one block per field.
    fn parse_field_iteration(
        &mut self,
        loop_var_name: &str,
        struct_def_nr: u32,
        source_expr: &Value,
        code: &mut Value,
    ) {
        let field_def_nr = self.data.def_nr("StructField");
        let field_type = Type::Reference(field_def_nr, Vec::new());
        let loop_var = self.create_var(loop_var_name, &field_type);
        self.vars.defined(loop_var);

        let mut body = Value::Null;
        self.parse_block("fields", &mut body, &Type::Void);

        let num_attrs = self.data.attributes(struct_def_nr);
        let mut blocks: Vec<Value> = Vec::new();

        // work_checkpoint + clean_work_refs removed — see comment at the
        // end of this loop explaining why skip_free must NOT be set here.
        for a in 0..num_attrs {
            let attr_name = self.data.attr_name(struct_def_nr, a);
            let attr_type = self.data.attr_type(struct_def_nr, a);

            let variant_name = match &attr_type {
                Type::Boolean => "FvBool",
                // Post-2c round 10c: wide Type::Integer (former Type::Long)
                // maps to FvLong; narrow range maps to FvInt.
                Type::Integer(s) if s.is_wide() => "FvLong",
                Type::Integer(_) => "FvInt",
                Type::Float => "FvFloat",
                Type::Single => "FvSingle",
                Type::Character => "FvChar",
                Type::Text(_) => "FvText",
                _ => continue,
            };

            let field_read = self.get_field(struct_def_nr, a, source_expr.clone());
            let variant_def_nr = self.data.def_nr(variant_name);
            let disc_val = self.data.def(variant_def_nr).attributes[0].value.clone();

            // Construct FieldValue variant as Value::Insert (flat ops list).
            let (fv_ops, fv_work) =
                self.build_object_ops(variant_def_nr, &[(0, disc_val), (1, field_read)]);
            let fv_insert = Value::Insert(fv_ops);

            // Construct StructField: the FieldValue is passed as Value::Var(fv_work)
            // after the Insert has executed.
            let (sf_ops, sf_work) = self.build_object_ops(
                field_def_nr,
                &[(0, Value::Text(attr_name)), (1, Value::Var(fv_work))],
            );
            let sf_insert = Value::Insert(sf_ops);

            blocks.push(fv_insert);
            blocks.push(sf_insert);
            blocks.push(v_set(loop_var, Value::Var(sf_work)));
            blocks.push(body.clone());
        }
        // do NOT call clean_work_refs here.  The unrolled loop
        // creates 2 work-refs per iteration (FvFloat/etc + StructField)
        // and assigns the latter to loop_var via v_set.  Only the LAST
        // iteration's work-refs feed loop_var; earlier ones are orphaned.
        // Marking them all skip_free prevented get_free_vars from
        // emitting OpFreeRef at scope exit, leaking 1 store per
        // orphaned work-ref (8 stores for a 3-field + 4-field struct).
        // The scan_set var-copy companion (Set(v, Var(src)) path) already
        // strips loop_var's deps so it gets its own OpFreeRef; the
        // work-refs themselves pass get_free_vars's is_work_ref check.

        if blocks.is_empty() {
            *code = Value::Null;
        } else {
            *code = v_block(blocks, Type::Void, "field_iter");
        }
    }

    /// Compute the in-store byte size of a vector element type.
    pub(crate) fn element_store_size(&self, elm: &Type) -> i32 {
        let elm_td = self.data.type_elm(elm);
        // Post-2c: honor size(N) on integer aliases.  Must run before the
        // generic `known_type → database.size(...)` path below, because
        // database.size for the 8-byte integer base returns 8 regardless.
        if matches!(elm, Type::Integer(_))
            && let Some(n) = self.data.forced_size(elm_td)
        {
            return i32::from(n);
        }
        // B5 (2026-04-13): for a mixed struct-enum element type
        // (`Type::Enum(_, true, _)`), the parent enum's `known_type` is
        // a byte-sized enumerate (size 1) — wrong for vector storage,
        // since instances are records.  Use the size of the largest
        // variant's structure type instead.  Without this, recursive
        // struct-enums (`vector<Tree>` inside Tree's own variant) trip
        // `OpDatabase(db_tp=u16::MAX)` panics in `Store::claim`.
        if let Type::Enum(parent_d_nr, true, _) = elm
            && elm_td != u32::MAX
        {
            let mut max_size = 0i32;
            for a_nr in 0..self.data.attributes(*parent_d_nr) {
                let variant_name = self.data.attr_name(*parent_d_nr, a_nr);
                let variant_d_nr = self.data.def_nr(&variant_name);
                if variant_d_nr != u32::MAX {
                    let variant_known = self.data.def(variant_d_nr).known_type;
                    let s = i32::from(self.database.size(variant_known));
                    if s > max_size {
                        max_size = s;
                    }
                }
            }
            if max_size > 0 {
                return max_size;
            }
        }
        if elm_td != u32::MAX {
            let known = self.data.def(elm_td).known_type;
            let db_size = i32::from(self.database.size(known));
            if db_size > 0 {
                return db_size;
            }
        }
        // Fallback for primitive types
        match elm {
            Type::Single | Type::Boolean | Type::Character | Type::Text(_) => 4,
            Type::Integer(_) | Type::Float => 8,
            _ => 12, // DbRef size for reference types
        }
    }

    /// Compiler special-case for `sort(v: vector<T>)`.
    /// Emits `OpSortVector(v, db_tp)` which sorts in-place at runtime, dispatching
    /// on the database element type.
    pub(crate) fn parse_sort(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "sort requires 1 argument: sort(vector)"
            );
            return Type::Void;
        }
        if let Type::Vector(elm, _) = &types[0] {
            if !matches!(elm.as_ref(), Type::Integer(_) | Type::Float | Type::Single) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "sort is not supported for vector<{}>; use integer, long, float, or single",
                    elm.name(&self.data)
                );
                return Type::Void;
            }
            let info = self.type_info(elm);
            *val = self.cl("OpSortVector", &[list[0].clone(), info]);
        } else {
            diagnostic!(self.lexer, Level::Error, "sort requires a vector argument");
        }
        Type::Void
    }

    /// Compiler special-case for `insert(v: vector<T>, idx: integer, elem: T)`.
    /// Emits `OpInsertVector` to create space, then the appropriate `OpSet*` to write the value.
    pub(crate) fn parse_insert(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 3 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "insert requires 3 arguments: insert(vector, index, element)"
            );
            return Type::Void;
        }
        let elm_tp = if let Type::Vector(elm, _) = &types[0] {
            (**elm).clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "insert requires a vector as first argument"
            );
            return Type::Void;
        };
        let elm_size = Value::Int(self.element_store_size(&elm_tp));
        let db_tp = self.type_info(&elm_tp);
        let ed_nr = self.data.type_def_nr(&elm_tp);
        // Create a temp var with dependency on the vector to prevent premature free
        let ref_tp = Type::Reference(ed_nr, types[0].depend());
        let tmp = self.create_unique("ins", &ref_tp);
        if let Value::Var(vec_var) = &list[0] {
            self.vars.depend(tmp, *vec_var);
        }
        // tmp = OpInsertVector(v, elem_size, idx, db_tp)
        let insert_call = self.cl(
            "OpInsertVector",
            &[list[0].clone(), elm_size, list[1].clone(), db_tp],
        );
        let set_val = self.set_field(ed_nr, usize::MAX, 0, Value::Var(tmp), list[2].clone());
        *val = v_block(vec![v_set(tmp, insert_call), set_val], Type::Void, "insert");
        Type::Void
    }

    /// Compiler special-case for `reverse(v: vector<T>)`.
    /// Dispatches to `OpReverseVector` which works for any element type.
    pub(crate) fn parse_reverse(
        &mut self,
        val: &mut Value,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reverse requires 1 argument: reverse(vector)"
            );
            return Type::Void;
        }
        let elm_size = if let Type::Vector(elm, _) = &types[0] {
            self.element_store_size(elm)
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reverse requires a vector argument"
            );
            return Type::Void;
        };
        *val = self.cl("OpReverseVector", &[list[0].clone(), Value::Int(elm_size)]);
        Type::Void
    }

    /// Validate arguments for `any`/`all`/`count_if`: (vector, fn-pred→boolean).
    fn validate_predicate_args(
        &mut self,
        name: &str,
        list: &[Value],
        types: &[Type],
    ) -> Option<(Type, u32)> {
        if list.len() != 2 {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name} requires 2 arguments: {name}(vector, fn pred)"
                );
            }
            return None;
        }
        let elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name}: first argument must be a vector"
                );
            }
            return None;
        };
        if let Type::Function(params, ret, _) = &types[1] {
            if params.len() != 1 && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name}: predicate must take exactly one argument"
                );
            }
            if **ret != Type::Boolean && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name}: predicate must return boolean"
                );
            }
        } else if !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "{name}: second argument must be a function reference (use fn <name>)"
            );
            return None;
        }
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            *d as u32
        } else {
            return None;
        };
        Some((elem_type, fn_d_nr))
    }

    /// Build the iteration preamble shared by `any`/`all`/`count_if`: copies the
    /// vector, creates an iterator, and returns the loop scaffolding.
    fn predicate_loop_scaffold(
        &mut self,
        name: &str,
        list: &[Value],
        types: &[Type],
    ) -> (Vec<Value>, u16, Value, Value) {
        let mut in_type = types[0].clone();
        let vec_var = self.create_unique(&format!("{name}_vec"), &in_type);
        in_type = in_type.depending(vec_var);

        let iter_var = self.create_unique(&format!("{name}_idx"), &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique(&format!("{name}_elm"), &var_tp);
        self.vars.defined(for_var);

        let mut create_iter = Value::Var(vec_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut test_for = Value::Var(for_var);
        self.convert(&mut test_for, &var_tp, &Type::Boolean);
        let not_test = self.cl("OpNot", &[test_for]);
        let break_if_done = v_if(
            not_test,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );

        let preamble = vec![v_set(vec_var, list[0].clone()), create_iter];
        // N8a.4: return for_next and break_if_done as separate values so callers
        // inline them directly in the loop body.  A v_block wrapper would declare
        // `for_var` inside a nested Rust `{ }` block, making it invisible to the
        // short_circuit/count_step expression that follows in native code.
        (preamble, for_var, for_next, break_if_done)
    }

    /// `any(vec, pred)` — true if pred returns true for any element.
    pub(crate) fn parse_any(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Boolean;
        }
        let Some((_, fn_d_nr)) = self.validate_predicate_args("any", list, types) else {
            return Type::Boolean;
        };

        let acc = self.create_unique("any_acc", &Type::Boolean);
        self.vars.defined(acc);

        let (preamble, for_var, for_next, break_if_done) =
            self.predicate_loop_scaffold("any", list, types);

        // if pred(elem) { acc = true; break }
        let pred_call = Value::Call(fn_d_nr, vec![Value::Var(for_var)]);
        let short_circuit = v_if(
            pred_call,
            v_block(
                vec![v_set(acc, Value::Boolean(true)), Value::Break(0)],
                Type::Void,
                "any_hit",
            ),
            Value::Null,
        );

        let loop_body = vec![for_next, break_if_done, short_circuit];
        let mut stmts = vec![v_set(acc, Value::Boolean(false))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "any"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, Type::Boolean, "any");
        Type::Boolean
    }

    /// `all(vec, pred)` — true if pred returns true for every element.
    pub(crate) fn parse_all(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Boolean;
        }
        let Some((_, fn_d_nr)) = self.validate_predicate_args("all", list, types) else {
            return Type::Boolean;
        };

        let acc = self.create_unique("all_acc", &Type::Boolean);
        self.vars.defined(acc);

        let (preamble, for_var, for_next, break_if_done) =
            self.predicate_loop_scaffold("all", list, types);

        // if !pred(elem) { acc = false; break }
        let pred_call = Value::Call(fn_d_nr, vec![Value::Var(for_var)]);
        let not_pred = self.cl("OpNot", &[pred_call]);
        let short_circuit = v_if(
            not_pred,
            v_block(
                vec![v_set(acc, Value::Boolean(false)), Value::Break(0)],
                Type::Void,
                "all_miss",
            ),
            Value::Null,
        );

        let loop_body = vec![for_next, break_if_done, short_circuit];
        let mut stmts = vec![v_set(acc, Value::Boolean(true))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "all"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, Type::Boolean, "all");
        Type::Boolean
    }

    /// `count_if(vec, pred)` — count of elements where pred returns true.
    pub(crate) fn parse_count_if(
        &mut self,
        val: &mut Value,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        if self.first_pass {
            return I32.clone();
        }
        let Some((_, fn_d_nr)) = self.validate_predicate_args("count_if", list, types) else {
            return I32.clone();
        };

        let acc = self.create_unique("cntif_acc", &I32);
        self.vars.defined(acc);

        let (preamble, for_var, for_next, break_if_done) =
            self.predicate_loop_scaffold("count_if", list, types);

        // if pred(elem) { acc += 1 }
        let pred_call = Value::Call(fn_d_nr, vec![Value::Var(for_var)]);
        let inc = v_set(acc, self.cl("OpAddInt", &[Value::Var(acc), Value::Int(1)]));
        let count_step = v_if(pred_call, inc, Value::Null);

        let loop_body = vec![for_next, break_if_done, count_step];
        let mut stmts = vec![v_set(acc, Value::Int(0))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "count_if"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, I32.clone(), "count_if");
        I32.clone()
    }
}

/// Plan-04 B.3 follow-up v2: recursively walk `val` and replace every
/// `Value::Var(target)` with a clone of `replacement`.  Used by
/// `build_parallel_for_ir` to inline-expand the par loop variable `b`
/// to its element-accessor expression, so that `b` is a parse-time
/// alias rather than a runtime slot.  See
/// `doc/claude/plans/finished/04-slot-assignment-redesign/b3-par-inline.md`.
fn replace_var_in_ir(val: &mut Value, target: u16, replacement: &Value) {
    match val {
        Value::Var(v) if *v == target => {
            *val = replacement.clone();
        }
        Value::Var(_)
        | Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Text(_)
        | Value::Enum(_, _)
        | Value::Line(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _)
        | Value::FnRef(_, _, _)
        | Value::Null => {}
        Value::Call(_, args)
        | Value::CallRef(_, args)
        | Value::Insert(args)
        | Value::Tuple(args)
        | Value::Parallel(args) => {
            for a in args.iter_mut() {
                replace_var_in_ir(a, target, replacement);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &mut bl.operators {
                replace_var_in_ir(op, target, replacement);
            }
        }
        Value::Set(_, body)
        | Value::Return(body)
        | Value::BreakWith(_, body)
        | Value::Drop(body)
        | Value::TuplePut(_, _, body)
        | Value::Yield(body) => {
            replace_var_in_ir(body, target, replacement);
        }
        Value::If(cond, t, f) => {
            replace_var_in_ir(cond, target, replacement);
            replace_var_in_ir(t, target, replacement);
            replace_var_in_ir(f, target, replacement);
        }
        Value::Iter(_, a, b, c) => {
            replace_var_in_ir(a, target, replacement);
            replace_var_in_ir(b, target, replacement);
            replace_var_in_ir(c, target, replacement);
        }
    }
}
