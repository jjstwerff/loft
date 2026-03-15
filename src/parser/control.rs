// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use super::{
    DefType, I32, Level, Parser, Position, Type, Value, diagnostic_format, merge_dependencies,
    v_block, v_if, v_loop, v_set,
};

impl Parser {
    // <block> ::= '}' | <expression> {';' <expression} '}'
    pub(crate) fn parse_block(&mut self, context: &str, val: &mut Value, result: &Type) -> Type {
        if let Value::Var(v) = val
            && let Type::Reference(r, _) = self.vars.tp(*v).clone()
            && context == "block"
        {
            // We actually scan a record here instead of a block of statement
            self.parse_object(r, val);
            return Type::Reference(r, Vec::new());
        }
        self.lexer.token("{");
        if self.lexer.has_token("}") {
            *val = v_block(Vec::new(), Type::Void, "empty block");
            return Type::Void;
        }
        let mut t = Type::Void;
        let mut l = Vec::new();
        loop {
            let line = self.lexer.pos().line;
            if line > self.line {
                if matches!(l.last(), Some(Value::Line(_))) {
                    l.pop();
                }
                l.push(Value::Line(line));
                self.line = line;
            }
            if self.lexer.has_token(";") {
                continue;
            }
            if self.lexer.peek_token("}") {
                break;
            }
            let mut n = Value::Null;
            t = self.expression(&mut n);
            if let Value::Insert(ls) = n {
                Self::move_insert_elements(&mut l, ls);
                t = Type::Void;
            } else if t != Type::Void && (self.lexer.peek_token(";") || *result == Type::Void) {
                l.push(Value::Drop(Box::new(n)));
            } else {
                l.push(n);
            }
            if self.lexer.peek_token("}") {
                break;
            }
            t = Type::Void;
            match l.last() {
                Some(Value::If(_, _, _) | Value::Loop(_) | Value::Block(_)) => (),
                _ => {
                    if !self.lexer.token(";") {
                        break;
                    }
                }
            }
        }
        self.lexer.token("}");
        if matches!(l.last(), Some(Value::Line(_))) {
            l.pop();
        }
        if matches!(t, Type::RefVar(_)) {
            let mut code = l.pop().unwrap().clone();
            self.un_ref(&mut t, &mut code);
            l.push(code);
        }
        t = self.block_result(context, result, &t, &mut l);
        *val = v_block(l, t.clone(), "block");
        t
    }

    pub(crate) fn un_ref(&mut self, t: &mut Type, code: &mut Value) {
        if let Type::RefVar(tp) = t.clone() {
            self.convert(code, t, &tp);
            *t = *tp;
            for on in t.depend() {
                *t = t.depending(on);
            }
        }
    }

    pub(crate) fn move_insert_elements(l: &mut Vec<Value>, elms: Vec<Value>) {
        for el in elms {
            if let Value::Insert(ls) = el {
                Self::move_insert_elements(l, ls);
            } else {
                l.push(el);
            }
        }
    }

    pub(crate) fn block_result(
        &mut self,
        context: &str,
        result: &Type,
        t: &Type,
        l: &mut [Value],
    ) -> Type {
        let mut tp = t.clone();
        if *result != Type::Void && !matches!(*result, Type::Unknown(_)) {
            let last = l.len() - 1;
            let ignore = *t == Type::Void && matches!(l[last], Value::Return(_));
            if !self.convert(&mut l[last], t, result) && !ignore {
                self.validate_convert(context, t, result);
            }
            tp = result.clone();
        }
        if let Type::Text(ls) = t {
            self.text_return(ls);
        } else if let Type::Reference(_, ls) | Type::Vector(_, ls) = t {
            self.ref_return(ls);
        }
        tp
    }

    // <operator> ::= '..' ['='] |
    //                '||' | 'or' |
    //                '&&' | 'and' |
    //                '==' | '!=' | '<' | '<=' | '>' | '>=' |
    //                '|' |
    //                '^' |
    //                '&' |
    //                '<<' | '>>' |
    //                '-' | '+' |
    //                '*' | '/' | '%'
    // <operators> ::= <single>  { '.' <field> | '[' <index> ']' } | <operators> <operator> <operators>
    pub(crate) fn parse_if(&mut self, code: &mut Value) -> Type {
        let mut test = Value::Null;
        let tp = self.expression(&mut test);
        self.convert(&mut test, &tp, &Type::Boolean);
        let mut true_code = Value::Null;
        let mut true_type = self.parse_block("if", &mut true_code, &Type::Unknown(0));
        let mut false_type = Type::Void;
        let mut false_code = Value::Null;
        if self.lexer.has_token("else") {
            if self.lexer.has_token("if") {
                self.parse_if(&mut false_code);
            } else {
                if true_type == Type::Null {
                    true_type = Type::Unknown(0);
                }
                false_type = self.parse_block("else", &mut false_code, &true_type);
                if true_type == Type::Unknown(0) {
                    if let Value::Block(bl) = &mut true_code {
                        let p = bl.operators.len() - 1;
                        bl.operators[p] = self.null(&false_type);
                        bl.result = false_type.clone();
                    }
                    true_type = false_type.clone();
                }
            }
        } else if true_type != Type::Void {
            false_code = v_block(vec![self.null(&true_type)], true_type.clone(), "else");
        }
        *code = v_if(test, true_code, false_code);
        merge_dependencies(&true_type, &false_type)
    }

    // <for> ::= <identifier> 'in' <expression> [ 'par' '(' <id> '=' <worker> ',' <threads> ')' ] '{' <block>
    //
    // The optional parallel clause `par(b=worker(a), N)` desugars to a parallel map
    // followed by an index-based loop over the results.  Three worker call forms
    // are supported — see `parse_parallel_for_loop` for details.
    /// Set up iterator variables for a for-loop header and return
    /// `(iter_var, pre_var, for_var, if_step, create_iter, iter_next)`.
    pub(crate) fn for_type(&mut self, in_type: &Type) -> Type {
        if let Type::Vector(t_nr, dep) = &in_type {
            let mut t = *t_nr.clone();
            if let Type::Enum(nr, true, _) = t {
                t = Type::Reference(nr, vec![]);
            }
            for d in dep {
                t = t.depending(*d);
            }
            t
        } else if let Type::Sorted(dnr, _, dep) | Type::Index(dnr, _, dep) = &in_type {
            Type::Reference(*dnr, dep.clone())
        } else if let Type::Iterator(i_tp, _) = &in_type {
            if **i_tp == Type::Null {
                I32.clone()
            } else {
                *i_tp.clone()
            }
        } else if let Type::Text(_) = in_type {
            Type::Character
        } else if let Type::Reference(_, _) | Type::Integer(_, _) | Type::Long = in_type {
            in_type.clone()
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown in expression type {}",
                    in_type.name(&self.data)
                );
            }
            Type::Null
        }
    }

    pub(crate) fn text_return(&mut self, ls: &[u16]) {
        if let Type::Text(cur) = &self.data.definitions[self.context as usize].returned {
            let mut dep = cur.clone();
            for v in ls {
                let n = self.vars.name(*v);
                let tp = self.vars.tp(*v);
                // skip related variables that are already attributes
                if let Some(a) = self.data.def(self.context).attr_names.get(n) {
                    if !dep.contains(&(*a as u16)) {
                        dep.push(*a as u16);
                    }
                    continue;
                }
                if matches!(tp, Type::Text(_)) {
                    // create a new attribute with this name
                    let a = self.data.add_attribute(
                        &mut self.lexer,
                        self.context,
                        n,
                        Type::RefVar(Box::new(Type::Text(Vec::new()))),
                    );
                    self.vars.become_argument(*v);
                    dep.push(a as u16);
                    self.vars
                        .set_type(*v, Type::RefVar(Box::new(Type::Text(Vec::new()))));
                } else {
                    let a = self
                        .data
                        .add_attribute(&mut self.lexer, self.context, n, tp.clone());
                    self.vars.become_argument(*v);
                    dep.push(a as u16);
                }
            }
            self.data.definitions[self.context as usize].returned = Type::Text(dep);
        }
    }

    pub(crate) fn ref_return(&mut self, ls: &[u16]) {
        let ret = self.data.definitions[self.context as usize]
            .returned
            .clone();
        if let Type::Vector(_, cur) | Type::Reference(_, cur) = &ret {
            let mut dep = cur.clone();
            for v in ls {
                let n = self.vars.name(*v);
                // skip related variables that are already attributes
                if let Some(a) = self.data.def(self.context).attr_names.get(n) {
                    if !dep.contains(&(*a as u16)) {
                        dep.push(*a as u16);
                    }
                    continue;
                }
                // create a new attribute with this name
                let a = self
                    .data
                    .add_attribute(&mut self.lexer, self.context, n, ret.clone());
                self.vars.become_argument(*v);
                dep.push(a as u16);
            }
            self.data.definitions[self.context as usize].returned = match ret {
                Type::Vector(it, _) => Type::Vector(it, dep),
                Type::Reference(td, _) => Type::Reference(td, dep),
                _ => unreachable!("ref_return called with non-Vector/Reference return type"),
            };
        }
    }

    // <return> ::= [ <expression> ]
    pub(crate) fn parse_return(&mut self, val: &mut Value) {
        // validate if there is a defined return value
        let mut v = Value::Null;
        let r_type = self.data.def(self.context).returned.clone();
        if !self.lexer.peek_token(";") && !self.lexer.peek_token("}") {
            let t = self.expression(&mut v);
            if r_type == Type::Void {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect no expression after return"
                );
                *val = Value::Return(Box::new(Value::Null));
                return;
            }
            if t == Type::Null {
                v = self.null(&r_type);
            } else if !self.convert(&mut v, &t, &r_type) {
                self.validate_convert("return", &t, &r_type);
            }
            if let Type::Text(ls) = t {
                self.text_return(&ls);
            }
        } else if !self.first_pass && r_type != Type::Void {
            diagnostic!(self.lexer, Level::Error, "Expect expression after return");
        }
        *val = Value::Return(Box::new(v));
    }

    // <call> ::= [ <expression> { ',' <expression> } ] ')'
    pub(crate) fn parse_call_diagnostic(
        &mut self,
        val: &mut Value,
        name: &str,
        list: &[Value],
        types: &[Type],
        call_pos: &Position,
    ) -> Type {
        if name == "assert" {
            let mut test = list[0].clone();
            self.convert(&mut test, &types[0], &Type::Boolean);
            let message = if list.len() > 1 {
                list[1].clone()
            } else {
                Value::str("assert failure")
            };
            if self.first_pass {
                *val = Value::Null;
                return Type::Void;
            }
            let d_nr = self.data.def_nr("n_assert");
            *val = Value::Call(
                d_nr,
                vec![
                    test,
                    message,
                    Value::str(&call_pos.file),
                    Value::Int(call_pos.line as i32),
                ],
            );
            Type::Void
        } else if name == "panic" {
            let message = if list.is_empty() {
                Value::str("panic")
            } else {
                list[0].clone()
            };
            if self.first_pass {
                *val = Value::Null;
                return Type::Void;
            }
            let d_nr = self.data.def_nr("n_panic");
            *val = Value::Call(
                d_nr,
                vec![
                    message,
                    Value::str(&call_pos.file),
                    Value::Int(call_pos.line as i32),
                ],
            );
            Type::Void
        } else {
            // log_info / log_warn / log_error / log_fatal
            let message = if list.is_empty() {
                Value::str("")
            } else {
                list[0].clone()
            };
            if self.first_pass {
                *val = Value::Null;
                return Type::Void;
            }
            let fn_name = format!("n_{name}");
            let d_nr = self.data.def_nr(&fn_name);
            *val = Value::Call(
                d_nr,
                vec![
                    message,
                    Value::str(&call_pos.file),
                    Value::Int(call_pos.line as i32),
                ],
            );
            Type::Void
        }
    }

    pub(crate) fn parse_call(&mut self, val: &mut Value, source: u16, name: &str) -> Type {
        let call_pos = self.lexer.pos().clone();
        let mut list = Vec::new();
        let mut types = Vec::new();
        if self.lexer.has_token(")") {
            // Check for zero-argument fn-ref call
            if self.vars.name_exists(name) {
                let v_nr = self.vars.var(name);
                if let Type::Function(param_types, ret_type) = self.vars.tp(v_nr).clone()
                    && param_types.is_empty()
                {
                    if !self.first_pass {
                        self.var_usages(v_nr, true);
                        *val = Value::CallRef(v_nr, vec![]);
                    }
                    return *ret_type;
                }
            }
            return self.call(val, source, name, &list, &Vec::new());
        }
        loop {
            let mut p = Value::Null;
            let t = self.expression(&mut p);
            types.push(t);
            list.push(p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        if matches!(
            name,
            "assert" | "panic" | "log_info" | "log_warn" | "log_error" | "log_fatal"
        ) {
            return self.parse_call_diagnostic(val, name, &list, &types, &call_pos);
        }
        if name == "parallel_for" {
            return self.parse_parallel_for(val, &list, &types);
        }
        if name == "map" {
            return self.parse_map(val, &list, &types);
        }
        if name == "filter" {
            return self.parse_filter(val, &list, &types);
        }
        if name == "reduce" {
            return self.parse_reduce(val, &list, &types);
        }
        // If the name refers to a fn-ref variable, emit a dynamic call through it.
        if self.vars.name_exists(name) {
            let v_nr = self.vars.var(name);
            if let Type::Function(param_types, ret_type) = self.vars.tp(v_nr).clone() {
                if !self.first_pass {
                    if list.len() != param_types.len() {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Function reference '{name}' expects {} argument(s), got {}",
                            param_types.len(),
                            list.len()
                        );
                        return *ret_type;
                    }
                    let mut converted = list.clone();
                    for (i, expected) in param_types.iter().enumerate() {
                        self.convert(&mut converted[i], &types[i], expected);
                    }
                    self.var_usages(v_nr, true);
                    *val = Value::CallRef(v_nr, converted);
                }
                return *ret_type;
            }
        }
        self.call(val, source, name, &list, &types)
    }

    // Validate and rewrite a user-friendly `parallel_for(fn f, vec, threads)` call
    // into a `Value::Call(n_parallel_for_d_nr, [input, elem_size, return_size, threads, func])`.
    //
    // The parser intercepts calls by name "parallel_for" before normal overload
    // resolution.  Compile-time checks performed here:
    // - First arg must be `Type::Function(args, ret)` (produced by `fn <name>` expression).
    // - Second arg must be `Type::Vector(T, _)`.
    // - Worker's first parameter must be a reference to T (type checked by name).
    // - Return type must be a primitive: integer, long, float, or boolean.
    // - Extra arg count must match the worker's extra parameters (args[1..]).
    /// Compiler special-case for `reduce(v: vector<T>, init: U, f: fn(U, T) -> U) -> U`.
    /// Generates inline bytecode equivalent to a left-fold over the vector.
    pub(crate) fn parse_reduce(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            // On first pass, return the accumulator type (second arg) if available.
            if types.len() >= 2 {
                return types[1].clone();
            }
            return Type::Unknown(0);
        }
        if list.len() != 3 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce requires 3 arguments: reduce(vector, init, fn f)"
            );
            return Type::Unknown(0);
        }
        let _in_elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: first argument must be a vector"
            );
            return Type::Unknown(0);
        };
        let acc_type = types[1].clone();
        let (fn_param_types, _fn_ret_type) = if let Type::Function(params, ret) = &types[2] {
            (params.clone(), *ret.clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: third argument must be a function reference (use fn <name>)"
            );
            return Type::Unknown(0);
        };
        if fn_param_types.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: function must take exactly two arguments (accumulator, element)"
            );
            return Type::Unknown(0);
        }
        // Extract the compile-time d_nr from the fn-ref value (always Value::Int(d_nr)).
        let fn_d_nr = if let Value::Int(d) = &list[2] {
            *d as u32
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: function must be a compile-time constant (use fn <name>)"
            );
            return Type::Unknown(0);
        };

        let acc_var = self.create_unique("reduce_acc", &acc_type);
        self.vars.defined(acc_var);

        let mut in_type = types[0].clone();
        let vec_copy_var = self.create_unique("reduce_vec", &in_type);
        in_type = in_type.depending(vec_copy_var);

        let iter_var = self.create_unique("reduce_idx", &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique("reduce_elm", &var_tp);
        self.vars.defined(for_var);

        let mut create_iter_code = Value::Var(vec_copy_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter_code, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut test_for = Value::Var(for_var);
        self.convert(&mut test_for, &var_tp, &Type::Boolean);
        let not_test = self.cl("OpNot", &[test_for]);
        let break_if_null = v_if(
            not_test,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );

        // Use Value::Call(d_nr, ...) directly — no fn_ref_var local needed.
        let fold_step = v_set(
            acc_var,
            Value::Call(fn_d_nr, vec![Value::Var(acc_var), Value::Var(for_var)]),
        );

        let loop_body = vec![for_next, break_if_null, fold_step];

        *val = v_block(
            vec![
                v_set(acc_var, list[1].clone()),
                v_set(vec_copy_var, list[0].clone()),
                create_iter_code,
                v_loop(loop_body, "reduce loop"),
                Value::Var(acc_var),
            ],
            acc_type.clone(),
            "reduce",
        );
        acc_type
    }

    // <size> ::= ( <type> | <var> ) ')'
    pub(crate) fn parse_size(&mut self, val: &mut Value) -> Type {
        let mut found = false;
        let lnk = self.lexer.link();
        if let Some(id) = self.lexer.has_identifier() {
            let d_nr = self.data.def_nr(&id);
            if d_nr != u32::MAX && self.data.def_type(d_nr) != DefType::EnumValue {
                if !self.first_pass && self.data.def_type(d_nr) == DefType::Unknown {
                    found = true;
                } else if let Some(tp) = self.parse_type(u32::MAX, &id, false) {
                    found = true;
                    if !self.first_pass {
                        *val = Value::Int(i32::from(
                            self.database
                                .size(self.data.def(self.data.type_elm(&tp)).known_type),
                        ));
                    }
                }
            }
        }
        if !found {
            let mut drop = Value::Null;
            self.lexer.revert(lnk);
            let tp = self.expression(&mut drop);
            let e_tp = self.data.type_elm(&tp);
            if e_tp != u32::MAX {
                found = true;
                if matches!(tp, Type::Enum(_, true, _) | Type::Reference(_, _)) && !self.first_pass
                {
                    // Polymorphic enum or reference: size depends on runtime variant.
                    *val = self.cl("OpSizeofRef", &[drop]);
                } else {
                    *val = Value::Int(i32::from(
                        self.database.size(self.data.def(e_tp).known_type),
                    ));
                }
            }
        }
        if !self.first_pass && !found {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect a variable or type after sizeof"
            );
        }
        self.lexer.token(")");
        I32.clone()
    }

    // <call> ::= [ <expression> { ',' <expression> } ] ')'
    pub(crate) fn parse_method(&mut self, val: &mut Value, md_nr: u32, on: Type) -> Type {
        let mut list = vec![val.clone()];
        let mut types = vec![on];
        if self.lexer.has_token(")") {
            return self.call_nr(val, md_nr, &list, &types, true);
        }
        loop {
            let mut p = Value::Null;
            let t = self.expression(&mut p);
            types.push(t);
            list.push(p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        self.call_nr(val, md_nr, &list, &types, true)
    }

    pub(crate) fn parse_parameters(&mut self) -> (Vec<Type>, Vec<Value>) {
        let mut list = vec![];
        let mut types = vec![];
        if self.lexer.has_token(")") {
            return (types, list);
        }
        loop {
            let mut p = Value::Null;
            types.push(self.expression(&mut p));
            list.push(p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        (types, list)
    }
}
