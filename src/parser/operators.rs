// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use super::{
    Level, OPERATORS, Parser, Type, Value, diagnostic_format, rename, v_block, v_if, v_set,
};

// Operator parsing and type dispatch.

impl Parser {
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
                            // a Character variable cannot serve as an OpAppendText
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
                // T1.2: tuple element access — t.0, t.1, etc.
                if let Type::Tuple(ref elems) = t {
                    if let Some(idx) = self.lexer.has_integer() {
                        let idx = idx as usize;
                        if idx >= elems.len() {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Tuple index {idx} out of range — tuple has {} elements",
                                elems.len()
                            );
                            t = Type::Unknown(0);
                        } else {
                            t = elems[idx].clone();
                            // T1.4 will emit proper codegen; for now store index in IR.
                            let tuple_val = code.clone();
                            *code = Value::Call(u32::MAX, vec![tuple_val, Value::Int(idx as i32)]);
                        }
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Tuple element access requires a numeric index (e.g. .0, .1)"
                        );
                    }
                } else {
                    t = self.field(code, t);
                }
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
            // Non-trivial LHS expressions are materialised into a temp to avoid
            // double evaluation (L6 fix).  Simple Var reads are safe without a temp.
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
                if let Value::Var(_) = code {
                    // Simple variable: reading twice is side-effect-free.
                    let lhs = code.clone();
                    let mut null_check = code.clone();
                    self.convert(&mut null_check, &lhs_type, &Type::Boolean);
                    *code = v_if(null_check, lhs, rhs);
                } else {
                    // Non-trivial expression: materialise into a temp to avoid double evaluation.
                    let tmp = self.create_unique("ncc", &lhs_type);
                    let set_tmp = v_set(tmp, code.clone());
                    let mut null_check = Value::Var(tmp);
                    self.convert(&mut null_check, &lhs_type, &Type::Boolean);
                    let if_expr = v_if(null_check, Value::Var(tmp), rhs);
                    *code = v_block(vec![set_tmp, if_expr], lhs_type.clone(), "ncc");
                }
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
}
