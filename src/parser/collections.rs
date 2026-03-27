// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

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
                    let size = if self.database.is_linked(db_tp) {
                        4
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
                        ref_expr = self.get_field(vec_tp, usize::MAX, ref_expr);
                    }
                    let mut tp = *vtp.clone();
                    for d in dep {
                        tp = tp.depending(*d);
                    }
                    let next = v_block(
                        vec![
                            v_set(
                                iter_var,
                                self.op("Add", i.clone(), Value::Int(1), I32.clone()),
                            ),
                            ref_expr,
                        ],
                        *vtp.clone(),
                        "iter next",
                    );
                    self.vars
                        .set_loop(0, self.data.def(vec_tp).known_type, code);
                    *code = v_set(iter_var, Value::Int(-1));
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
                    let state_var = self.create_var(&iter_state_name, &Type::Long);
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
        if !self.first_pass
            && *val == Value::Null
            && op == "="
            && let Value::Call(get_nr, get_args) = to
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
        // A10: detect #fields for compile-time field iteration.
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
            diagnostic!(self.lexer, Level::Error, "Incorrect # variable on {}", name);
            *t = Type::Unknown(0);
        }
    }

    pub(crate) fn append_data_fp(state: OutputState, fmt: Value) -> (Value, Value, Value) {
        let mut a_width = state.width;
        let mut p_rec = Value::Int(0);
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
            &(start.to_owned() + "Long"),
            &[
                var,
                fmt,
                Value::Int(state.radix),
                state.width,
                Value::Int(i32::from(state.token.as_bytes()[0])),
                Value::Boolean(state.plus),
                Value::Boolean(state.note),
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
        // L8: warn when a format specifier has no effect on the value type.
        if !self.first_pass {
            let is_text = matches!(tp, Type::Text(_));
            let is_bool = matches!(tp, Type::Boolean);
            if state.radix != 10 && (is_text || is_bool) {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "Format specifier has no effect on {}",
                    tp.name(&self.data)
                );
            } else if is_text && state.token == "0" && state.width != Value::Int(0) {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "Zero-padding has no effect on text"
                );
            }
        }
        match tp {
            Type::Integer(_, _, _) => {
                let value = self.cl("OpConvLongFromInt", std::slice::from_ref(format));
                self.append_data_long(list, start, var, value, state);
            }
            Type::Long => {
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
                let (fmt, a_width, p_rec) = Self::append_data_fp(state, format.clone());
                list.push(self.cl(&(start.to_owned() + "Float"), &[var, fmt, a_width, p_rec]));
            }
            Type::Single => {
                let (fmt, a_width, p_rec) = Self::append_data_fp(state, format.clone());
                list.push(self.cl(&(start.to_owned() + "Single"), &[var, fmt, a_width, p_rec]));
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
        let existing_var = self.vars.var(id);
        if !self.first_pass
            && existing_var != u16::MAX
            && self.vars.is_defined(existing_var)
            && !self.vars.var_type(existing_var).is_same(&var_tp)
            && !self.vars.var_type(existing_var).is_unknown()
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "loop variable '{id}' has type {} but was previously used as {}",
                var_tp.name(&self.data),
                self.vars.var_type(existing_var).name(&self.data)
            );
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
            // A10: if #fields was detected, take the compile-time unrolling path.
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
            if matches!(in_type, Type::Vector(_, _)) {
                let vec_var = self.create_unique("vector", &in_type);
                in_type = in_type.depending(vec_var);
                fill = v_set(vec_var, expr);
                expr = Value::Var(vec_var);
            }
            // Optional parallel clause: par(result=worker(elem), threads)
            if let LexItem::Identifier(kw) = &self.lexer.peek().has
                && kw == "par"
            {
                self.lexer.has_identifier(); // consume "par"
                self.parse_parallel_for_loop(code, &id, &in_type, expr, fill, loop_nr);
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
        vec_expr: Value,
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
        let elem_var_nr = self.create_var(elem_var, &elem_tp);
        self.vars.defined(elem_var_nr);
        if matches!(elem_tp, Type::Integer(_, _, _)) {
            self.vars.in_use(elem_var_nr, true);
        }

        // Resolve worker function: consumes the worker call tokens up to the ','.
        let (fn_d_nr, ret_type, extra_vals, _extra_types) =
            self.parse_parallel_worker(elem_var, &elem_tp);

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
        } else if matches!(&ret_type, Type::Reference(_, _)) {
            -1 // sentinel: reference mode — workers return DbRef, main deep-copies
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
        vec_expr: Value,
        threads_expr: Value,
        fill: Value,
        loop_nr: u16,
        extra_args: Vec<Value>,
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
        let b_var = self.create_var(result_name, &b_type);
        self.vars.defined(b_var);
        if matches!(b_type, Type::Integer(_, _, _) | Type::Unknown(_)) {
            self.vars.in_use(b_var, true);
        }
        // Reference return: b_var borrows from the result vector — must not be freed.
        if matches!(ret_type, Type::Reference(_, _)) {
            self.vars.set_skip_free(b_var);
        }

        // Parse the body block.
        self.vars.loop_var(b_var);
        let in_loop = self.in_loop;
        self.in_loop = true;
        let mut block = Value::Null;
        self.parse_block("parallel for", &mut block, &Type::Void);
        let count = self.vars.loop_counter();
        self.in_loop = in_loop;
        self.vars.finish_loop(loop_nr);

        // Build IR only when we have a valid function reference.
        if fn_d_nr == u32::MAX || par_for_d_nr == u32::MAX {
            // Errors already reported; emit nothing useful.
            *code = Value::Null;
            return;
        }

        // parallel_for(input, elem_size, return_size, threads, fn_d_nr, extra1, ..., n_extra)
        // n_extra is pushed LAST so it's on top of the stack for popping first.
        let n_extra = extra_args.len();
        let mut pf_args = vec![
            vec_expr.clone(),
            Value::Int(elem_size),
            Value::Int(return_size),
            threads_expr,
            Value::Int(fn_d_nr as i32),
        ];
        pf_args.extend(extra_args);
        pf_args.push(Value::Int(n_extra as i32));
        let pf_call = Value::Call(par_for_d_nr, pf_args);

        // len(input_vec) — compute once before the loop.
        let len_call = self.cl("OpLengthVector", &[vec_expr]);

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
        let get_call = if matches!(ret_type, Type::Reference(_, _)) {
            get_vec
        } else {
            let vec_tp = self.data.type_def_nr(ret_type);
            self.get_field(vec_tp, usize::MAX, get_vec)
        };
        let b_assign = v_set(b_var, get_call);
        let idx_inc = v_set(
            idx_var,
            self.cl("OpAddInt", &[Value::Var(idx_var), Value::Int(1)]),
        );

        let mut lp = vec![stop, b_assign, block, idx_inc];
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
        let (fn_param_types, fn_ret_type) = if let Type::Function(params, ret) = &types[1] {
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
        // Extract the compile-time d_nr from the fn-ref value (always Value::Int(d_nr)).
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            *d as u32
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: function reference must be a compile-time constant (use fn <name>)"
            );
            return placeholder;
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

        let fill = v_set(vec_copy_var, list[0].clone());

        // Use Value::Call(d_nr, args) directly — avoids a fn_ref_var local variable
        // that would share a stack slot with iter_var (validate_slots violation).
        let body = Value::Call(fn_d_nr, vec![Value::Var(for_var)]);

        self.data.vector_def(&mut self.lexer, &out_elem);

        let tp = result_type.clone();
        // Reset val so build_comprehension_code creates a fresh result vector rather than
        // pre-seeding it with the LHS variable (which would cause a self-reference panic).
        *val = Value::Null;
        self.build_comprehension_code(
            result_vec,
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
    ) -> Result<(Type, u32), Type> {
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
        let (fn_param_types, fn_ret_type) = if let Type::Function(params, ret) = &types[1] {
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
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            *d as u32
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: predicate must be a compile-time constant (use fn <name>)"
            );
            return Err(placeholder);
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

        let fill = v_set(vec_copy_var, list[0].clone());

        // build_comprehension_code: v_if(if_step, null, Continue) → proceed when if_step truthy.
        // Use Value::Call(d_nr, ...) directly — avoids a fn_ref_var local that would conflict.
        let if_step = Value::Call(fn_d_nr, vec![Value::Var(for_var)]);

        let body = Value::Var(for_var);

        self.data.vector_def(&mut self.lexer, &out_elem);

        let tp = result_type.clone();
        // Reset val so build_comprehension_code creates a fresh result vector.
        *val = Value::Null;
        self.build_comprehension_code(
            result_vec,
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

    /// A10: compile-time unroll `for f in s#fields` into one block per field.
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

        let work_checkpoint = self.vars.work_ref();
        for a in 0..num_attrs {
            let attr_name = self.data.attr_name(struct_def_nr, a);
            let attr_type = self.data.attr_type(struct_def_nr, a);

            let variant_name = match &attr_type {
                Type::Boolean => "FvBool",
                Type::Integer(_, _, _) => "FvInt",
                Type::Long => "FvLong",
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
        // Mark work refs as skip_free — they are consumed by the loop var assignment.
        self.vars.clean_work_refs(work_checkpoint);

        if blocks.is_empty() {
            *code = Value::Null;
        } else {
            *code = v_block(blocks, Type::Void, "field_iter");
        }
    }

    /// Compute the in-store byte size of a vector element type.
    pub(crate) fn element_store_size(&self, elm: &Type) -> i32 {
        let elm_td = self.data.type_elm(elm);
        if elm_td != u32::MAX {
            let known = self.data.def(elm_td).known_type;
            let db_size = i32::from(self.database.size(known));
            if db_size > 0 {
                return db_size;
            }
        }
        // Fallback for primitive types
        match elm {
            Type::Integer(_, _, _)
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Text(_) => 4,
            Type::Long | Type::Float => 8,
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
            if !matches!(
                elm.as_ref(),
                Type::Integer(_, _, _) | Type::Long | Type::Float | Type::Single
            ) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "sort is not supported for vector<{}>; use integer, long, float, or single",
                    elm.name(&self.data)
                );
                return Type::Void;
            }
            *val = self.cl("OpSortVector", &[list[0].clone(), self.type_info(elm)]);
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
        if let Type::Function(params, ret) = &types[1] {
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
    #[allow(clippy::type_complexity)]
    fn predicate_loop_scaffold(
        &mut self,
        name: &str,
        list: &[Value],
        types: &[Type],
    ) -> (Vec<Value>, u16, Value) {
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
        (
            preamble,
            for_var,
            v_block(vec![for_next, break_if_done], Type::Void, "iter_step"),
        )
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

        let (preamble, for_var, iter_step) = self.predicate_loop_scaffold("any", list, types);

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

        let loop_body = vec![iter_step, short_circuit];
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

        let (preamble, for_var, iter_step) = self.predicate_loop_scaffold("all", list, types);

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

        let loop_body = vec![iter_step, short_circuit];
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

        let (preamble, for_var, iter_step) = self.predicate_loop_scaffold("count_if", list, types);

        // if pred(elem) { acc += 1 }
        let pred_call = Value::Call(fn_d_nr, vec![Value::Var(for_var)]);
        let inc = v_set(acc, self.cl("OpAddInt", &[Value::Var(acc), Value::Int(1)]));
        let count_step = v_if(pred_call, inc, Value::Null);

        let loop_body = vec![iter_step, count_step];
        let mut stmts = vec![v_set(acc, Value::Int(0))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "count_if"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, I32.clone(), "count_if");
        I32.clone()
    }
}
