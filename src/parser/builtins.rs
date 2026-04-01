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
        // S23: generator functions (return type iterator<T>) cannot be par() workers.
        // Worker threads do not have access to the main thread's coroutines table.
        // Return u32::MAX + Unknown in both passes so build_parallel_for_ir doesn't
        // type `b` as iterator<T> and downstream body code doesn't produce cascaded errors.
        if matches!(ret_type, Type::Iterator(_, _)) {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "parallel worker '{method_name}' returns {} — \
                     generator functions cannot be used as parallel workers",
                    ret_type.name(&self.data)
                );
            }
            return (u32::MAX, Type::Unknown(0));
        }
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
        // S23: generator functions (return type iterator<T>) cannot be par() workers.
        // Worker threads do not have access to the main thread's coroutines table.
        // Return u32::MAX + Unknown in both passes so build_parallel_for_ir doesn't
        // type `b` as iterator<T> and downstream body code doesn't produce cascaded errors.
        if matches!(ret_type, Type::Iterator(_, _)) {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "parallel worker '{first_id}' returns {} — \
                     generator functions cannot be used as parallel workers",
                    ret_type.name(&self.data)
                );
            }
            return (u32::MAX, Type::Unknown(0), extra_vals, extra_types);
        }
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

    #[allow(clippy::too_many_lines)]
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
        let (worker_arg_types, worker_ret_type) = if let Type::Function(args, ret, _) = &types[0] {
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
        // A14.5/A14.6: check if the worker qualifies for the light path.
        // Light path: primitive return (not text, not reference), no recursive store alloc.
        // A14.5/A14.6: auto-select light path for eligible workers.
        let worker_d_nr = if let Value::Int(d) = &list[0] {
            *d as u32
        } else {
            u32::MAX
        };
        let is_primitive_return =
            !matches!(&worker_ret_type, Type::Text(_) | Type::Reference(_, _));
        let light_m = if is_primitive_return && worker_d_nr != u32::MAX {
            self.check_light_eligible(worker_d_nr)
        } else {
            None
        };

        let (par_fn_name, extra_pool_arg) = if let Some(m) = light_m {
            ("n_parallel_for_light", Some(Value::Int(m as i32)))
        } else {
            ("n_parallel_for", None)
        };
        let par_for_d_nr = self.data.def_nr(par_fn_name);
        if par_for_d_nr == u32::MAX {
            diagnostic!(
                self.lexer,
                Level::Error,
                "internal error: {par_fn_name} not found"
            );
            return Type::Unknown(0);
        }
        // Build augmented call: [input, element_size, return_size, threads, func].
        let mut augmented = vec![
            list[1].clone(),                // input: vector<T>
            Value::Int(elem_size),          // element_size: synthesized
            Value::Int(return_size as i32), // return_size: synthesized
            list[2].clone(),                // threads: integer
            list[0].clone(),                // func: d_nr as integer
        ];
        // pool_m is hardcoded in the native function
        let _ = extra_pool_arg;
        // Append any extra args (verified count above; types passed through).
        for extra in list.iter().skip(3) {
            augmented.push(extra.clone());
        }
        *val = Value::Call(par_for_d_nr, augmented);
        result_ref_type
    }

    /// A14.5: check if a worker function qualifies for the light parallel path.
    /// Returns `Some(M)` (pool stores per worker) if eligible, `None` otherwise.
    /// Eligible = no text return AND no store allocation inside recursive calls.
    pub(crate) fn check_light_eligible(&self, worker_d_nr: u32) -> Option<usize> {
        if worker_d_nr as usize >= self.data.definitions.len() {
            return None;
        }
        // Text return disqualifies — needs special work-buffer handling.
        if matches!(self.data.def(worker_d_nr).returned, Type::Text(_)) {
            return None;
        }
        // Walk the call graph to detect recursive store allocation.
        let mut visited = std::collections::HashSet::new();
        let mut on_stack = std::collections::HashSet::new();
        let mut max_stores = 0usize;
        if self.has_recursive_allocation(worker_d_nr, &mut visited, &mut on_stack, &mut max_stores)
        {
            return None;
        }
        Some(max_stores + 1)
    }

    /// DFS walk of the call graph. Returns true if a cycle contains store allocation.
    fn has_recursive_allocation(
        &self,
        d_nr: u32,
        visited: &mut std::collections::HashSet<u32>,
        on_stack: &mut std::collections::HashSet<u32>,
        max_stores: &mut usize,
    ) -> bool {
        if on_stack.contains(&d_nr) {
            // Cycle detected — check if this function allocates stores.
            return self.fn_allocates_stores(d_nr);
        }
        if visited.contains(&d_nr) {
            return false;
        }
        visited.insert(d_nr);
        on_stack.insert(d_nr);

        // Count reference-type variables in this function (store allocations).
        let ref_count = self.count_ref_vars(d_nr);
        *max_stores = (*max_stores).max(ref_count);

        // Walk all calls in the function body.
        let code = self.data.def(d_nr).code.clone();
        let callees = self.extract_callees(&code);
        for callee in callees {
            if self.has_recursive_allocation(callee, visited, on_stack, max_stores) {
                on_stack.remove(&d_nr);
                return true;
            }
        }
        on_stack.remove(&d_nr);
        false
    }

    /// Check if a function body contains store allocation (`OpDatabase` calls).
    fn fn_allocates_stores(&self, d_nr: u32) -> bool {
        self.count_ref_vars(d_nr) > 0
    }

    /// Count reference-type local variables (each may need a store).
    /// Count locally-allocated reference variables (excluding arguments — those
    /// are passed by the caller and don't allocate new stores).
    fn count_ref_vars(&self, d_nr: u32) -> usize {
        if d_nr as usize >= self.data.definitions.len() {
            return 0;
        }
        let vars = &self.data.def(d_nr).variables;
        (0..vars.next_var())
            .filter(|&v| !vars.is_argument(v) && matches!(vars.tp(v), Type::Reference(_, _)))
            .count()
    }

    /// Extract all direct callee `d_nr`s from a Value tree.
    fn extract_callees(&self, val: &Value) -> Vec<u32> {
        let mut callees = Vec::new();
        self.collect_callees(val, &mut callees);
        callees
    }

    fn collect_callees(&self, val: &Value, out: &mut Vec<u32>) {
        match val {
            Value::Call(d, args) => {
                if *d != u32::MAX && (*d as usize) < self.data.definitions.len() {
                    out.push(*d);
                }
                for a in args {
                    self.collect_callees(a, out);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    self.collect_callees(op, out);
                }
            }
            Value::Set(_, expr) | Value::Return(expr) | Value::Drop(expr) => {
                self.collect_callees(expr, out);
            }
            Value::If(c, t, f) => {
                self.collect_callees(c, out);
                self.collect_callees(t, out);
                self.collect_callees(f, out);
            }
            Value::Insert(ops) => {
                for op in ops {
                    self.collect_callees(op, out);
                }
            }
            _ => {}
        }
    }
}
