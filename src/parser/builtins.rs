// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use super::{Context, DefType, Level, Parser, Type, Value, diagnostic_format, var_size};

impl Parser {
    pub(crate) fn skip_to_parallel_body(&mut self) {
        let mut depth = 1i32;
        loop {
            if self.lexer.peek_token("(") {
                depth += 1;
            } else if self.lexer.peek_token(")") {
                depth -= 1;
                if depth == 0 {
                    self.lexer.has_token(")");
                    break;
                }
            } else if self.lexer.peek_token("") {
                break;
            }
            let mut dummy = Value::Null;
            self.expression(&mut dummy);
        }
        let mut dummy = Value::Null;
        self.parse_block("parallel for", &mut dummy, &Type::Void);
    }

    // Resolve the worker function from a parallel call pattern inside par(...).
    //
    // Recognised forms (elem_var is the name of the loop element, e.g. "a"):
    //   Form 1  func(a)         — global/user function
    //   Form 2  a.method()      — method whose receiver is the element
    //   Form 3  c.method(a)     — captured receiver + element arg (deferred)
    //
    // All tokens for the call are consumed; the caller reads ',' next.
    // On error or first pass returns (u32::MAX, Unknown).
    /// Consume a parenthesised argument list, discarding all tokens.
    pub(crate) fn consume_call_args(&mut self) {
        let mut depth = 1i32;
        loop {
            if self.lexer.peek_token("(") {
                depth += 1;
            } else if self.lexer.peek_token(")") {
                depth -= 1;
                if depth == 0 {
                    self.lexer.has_token(")");
                    break;
                }
            } else if self.lexer.peek_token("") {
                break;
            }
            let mut dummy = Value::Null;
            self.expression(&mut dummy);
            self.lexer.has_token(",");
        }
    }

    /// Parallel Form 2: `a.method()` — method call on the element.
    pub(crate) fn parse_parallel_worker_method(
        &mut self,
        elem_var: &str,
        elem_tp: &Type,
    ) -> (u32, Type) {
        if !self.lexer.has_token(".") {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect '.' after '{elem_var}' in parallel clause (use a.method() or func(a))"
                );
            }
            return (u32::MAX, Type::Unknown(0));
        }
        let Some(method_name) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Expect method name after '.'");
            }
            return (u32::MAX, Type::Unknown(0));
        };
        self.lexer.token("(");
        self.lexer.token(")");

        // Resolve the method on the element type.
        let type_name = match elem_tp {
            Type::Reference(d, _) | Type::Enum(d, _, _) => self.data.def(*d).name.clone(),
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Parallel method call (form 2) requires a struct element type, not {}",
                        elem_tp.name(&self.data)
                    );
                }
                return (u32::MAX, Type::Unknown(0));
            }
        };
        // Method internal name: t_<len><TypeName>_<method>
        let internal = format!("t_{}{type_name}_{method_name}", type_name.len());
        let d_nr = {
            let nr = self.data.def_nr(&internal);
            if nr == u32::MAX {
                self.data.def_nr(&method_name)
            } else {
                nr
            }
        };
        if d_nr == u32::MAX {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown method '{method_name}' on type '{type_name}'"
                );
            }
            return (u32::MAX, Type::Unknown(0));
        }
        if !self.first_pass && !matches!(self.data.def_type(d_nr), DefType::Function) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "'{method_name}' is not a function"
            );
            return (u32::MAX, Type::Unknown(0));
        }
        self.data.def_used(d_nr);
        let ret_type = self.data.def(d_nr).returned.clone();
        (d_nr, ret_type)
    }

    /// Parallel Form 1: `func(a)` — global/user function call.
    /// Parse `worker(a, extra1, extra2)` in a parallel clause.
    /// Returns `(fn_d_nr, return_type, extra_arg_values, extra_arg_types)`.
    /// The first argument (the element variable) is skipped; extra args are returned.
    pub(crate) fn parse_parallel_worker_fn(
        &mut self,
        first_id: &str,
    ) -> (u32, Type, Vec<Value>, Vec<Type>) {
        // Resolve function name: try n_<name> first (user function convention).
        let d_nr = {
            let prefixed = format!("n_{first_id}");
            let nr = self.data.def_nr(&prefixed);
            if nr == u32::MAX {
                self.data.def_nr(first_id)
            } else {
                nr
            }
        };
        // Parse the argument list — skip first arg (element), collect extras.
        let mut extra_vals = Vec::new();
        let mut extra_types = Vec::new();
        if self.lexer.has_token("(") {
            // Skip the first argument (element variable reference).
            let mut dummy = Value::Null;
            self.expression(&mut dummy);
            // Collect remaining arguments as extra context args.
            while self.lexer.has_token(",") {
                if self.lexer.peek_token(")") {
                    break;
                }
                let mut val = Value::Null;
                let tp = self.expression(&mut val);
                extra_vals.push(val);
                extra_types.push(tp);
            }
            self.lexer.token(")");
        } else if !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect '(' after function name '{first_id}' in parallel clause"
            );
        }
        if d_nr == u32::MAX {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Unknown function '{first_id}'");
            }
            return (u32::MAX, Type::Unknown(0), extra_vals, extra_types);
        }
        if !self.first_pass && !matches!(self.data.def_type(d_nr), DefType::Function) {
            diagnostic!(self.lexer, Level::Error, "'{first_id}' is not a function");
            return (u32::MAX, Type::Unknown(0), extra_vals, extra_types);
        }
        // Validate extra arg count against function signature.
        // Skip hidden __ref_* / __rref_* / __work_* parameters (work-refs for text/vector returns).
        if !self.first_pass {
            let n_params = (0..self.data.attributes(d_nr))
                .filter(|&a| !self.data.attr_name(d_nr, a).starts_with("__"))
                .count();
            let n_extra = extra_vals.len();
            let expected_extra = if n_params > 0 { n_params - 1 } else { 0 };
            if n_extra != expected_extra {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "parallel_for: wrong number of extra arguments: \
                     worker expects {expected_extra}, got {n_extra}"
                );
            }
        }
        self.data.def_used(d_nr);
        let ret_type = self.data.def(d_nr).returned.clone();
        (d_nr, ret_type, extra_vals, extra_types)
    }

    pub(crate) fn parse_parallel_worker(
        &mut self,
        elem_var: &str,
        elem_tp: &Type,
    ) -> (u32, Type, Vec<Value>, Vec<Type>) {
        let Some(first_id) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect function name or '{elem_var}.method' inside |..|"
                );
            }
            return (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new());
        };

        if first_id == elem_var {
            // ── Form 2: a.method() ────────────────────────────────────────────
            let (d, t) = self.parse_parallel_worker_method(elem_var, elem_tp);
            (d, t, Vec::new(), Vec::new())
        } else if self.lexer.peek_token(".") {
            // ── Form 3: c.method(a) — deferred ───────────────────────────────
            // Consume the rest of the call so parsing can continue.
            self.lexer.has_token(".");
            if self.lexer.has_identifier().is_some() && self.lexer.has_token("(") {
                self.consume_call_args();
            }
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Parallel form '{first_id}.method({elem_var})' (captured receiver) \
                     is not yet supported; define a wrapper function and use func({elem_var}) instead"
                );
            }
            (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new())
        } else {
            // ── Form 1: func(a, extra...) ─────────────────────────────────────
            self.parse_parallel_worker_fn(&first_id)
        }
    }

    pub(crate) fn parse_parallel_for(
        &mut self,
        val: &mut Value,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        let ref_d_nr = self.data.def_nr("reference");
        let result_ref_type = Type::Reference(ref_d_nr, Vec::new());
        if self.first_pass {
            return result_ref_type;
        }
        if list.len() < 3 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "parallel_for requires at least 3 arguments: fn worker, input_vector, threads"
            );
            return Type::Unknown(0);
        }
        let (worker_arg_types, worker_ret_type) = if let Type::Function(args, ret) = &types[0] {
            (args.clone(), (**ret).clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "parallel_for: first argument must be a function reference (use fn <name>)"
            );
            return Type::Unknown(0);
        };
        let elem_tp = if let Type::Vector(elem, _) = &types[1] {
            (**elem).clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "parallel_for: second argument must be a vector"
            );
            return Type::Unknown(0);
        };
        // Compute element size from the return type.
        // return_size = 0 signals text mode to n_parallel_for.
        let return_size: u32 = if matches!(&worker_ret_type, Type::Text(_)) {
            0
        } else {
            let sz = u32::from(var_size(&worker_ret_type, &Context::Argument));
            if sz == 0 || sz > 8 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "parallel_for: worker return type '{}' (size {sz}) is not supported",
                    worker_ret_type.name(&self.data)
                );
                return Type::Unknown(0);
            }
            sz
        };
        // Validate extra arg count matches worker's extra params.
        let n_extra = list.len().saturating_sub(3);
        let n_worker_extra = worker_arg_types.len().saturating_sub(1);
        if n_extra != n_worker_extra {
            diagnostic!(
                self.lexer,
                Level::Error,
                "parallel_for: wrong number of extra arguments: worker expects {n_worker_extra}, got {n_extra}"
            );
            return Type::Unknown(0);
        }
        // Compute element size from T — use the actual inline database size, not the IR size.
        // var_size() returns size_of::<DbRef>() for reference types, which is wrong for inline
        // vector element storage (e.g. Score{value:integer} is 4 bytes inline, not 12).
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
        // Look up internal n_parallel_for.
        let par_for_d_nr = self.data.def_nr("n_parallel_for");
        if par_for_d_nr == u32::MAX {
            diagnostic!(
                self.lexer,
                Level::Error,
                "internal error: n_parallel_for not found"
            );
            return Type::Unknown(0);
        }
        // Build augmented call: [input, element_size, return_size, threads, func].
        // (input first so gather_key reads it, not the integer func d_nr.)
        let mut augmented = vec![
            list[1].clone(),                // input: vector<T>
            Value::Int(elem_size),          // element_size: synthesized
            Value::Int(return_size as i32), // return_size: synthesized
            list[2].clone(),                // threads: integer
            list[0].clone(),                // func: d_nr as integer
        ];
        // Append any extra args (verified count above; types passed through).
        for extra in list.iter().skip(3) {
            augmented.push(extra.clone());
        }
        *val = Value::Call(par_for_d_nr, augmented);
        result_ref_type
    }
}
