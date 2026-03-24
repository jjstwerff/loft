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

// Lambda and vector expression parsing.

impl Parser {
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
}
