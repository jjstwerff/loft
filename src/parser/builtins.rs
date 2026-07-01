// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent)

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
            let before = self.lexer.peek().position;
            let mut dummy = Value::Null;
            self.expression(&mut dummy);
            // Recovery must always make forward progress.  Consume an argument
            // separator (as consume_call_args does), and bail out if neither the
            // expression nor the comma advanced the lexer — otherwise a token
            // expression() cannot start (e.g. a leading `,`) spins this loop
            // forever instead of recovering to the body.
            let consumed_comma = self.lexer.has_token(",");
            if !consumed_comma && self.lexer.peek().position == before {
                break;
            }
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

        // Resolve the method on the element type.  @PLN25 E2 — a
        // `__nullable<S>` element resolves the method on the DENSE `S`
        // (`t_<len>S_<method>`), not the synthetic enum: the worker takes a
        // dense `S`, and (second pass) a `__par_nullable_w` wrapper adapts the
        // disc-prefixed element to it.  Resolving the dense name in BOTH passes
        // keeps `fn_d_nr != MAX` in pass 1 so `build_parallel_for_ir` emits a
        // `Value::Parallel` (not `Null`) — the block parser's `;`-exemption
        // keys on that, so an unresolved nullable worker otherwise tripped a
        // spurious "Expect token ;" on the statement after the loop.
        let (type_name, nullable_ctx) = match elem_tp {
            Type::Enum(d, _, _) if self.data.def(*d).name().starts_with("__nullable<") => {
                let nm = self.data.def(*d).name().to_string();
                let sname = nm["__nullable<".len()..nm.len() - 1].to_string();
                let struct_d = self.data.def_nr(&sname);
                (sname, Some((*d, struct_d)))
            }
            Type::Reference(d, _) | Type::Enum(d, _, _) => {
                (self.data.def(*d).name().to_string(), None)
            }
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
        let ret_type = self.data.def(d_nr).returned().clone();
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
        // @PLN25 E2 — for a `__nullable<S>` element, wrap the dense-`S` method
        // in `__par_nullable_w` so the disc-prefixed element coerces to a
        // `ref(S)` at the payload offset (gap 2) before dispatch.  First pass
        // returns the raw method d_nr (so the `;`-exemption fires); the wrapper
        // is synthesized only in the second pass, like the `worker(a)` form.
        if let Some((enum_d, struct_d)) = nullable_ctx
            && !self.first_pass
            && struct_d != u32::MAX
            && self.data.attributes(struct_d) > 0
        {
            let w = self.synth_nullable_par_wrapper(
                elem_tp,
                enum_d,
                struct_d,
                d_nr,
                &ret_type,
                &method_name,
            );
            return (w, ret_type);
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
        elem_tp: &Type,
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
        // Skip hidden __ref_* / __rref_* / __work_* parameters (work-refs
        // for text/vector returns) and any attribute marked
        // `hidden: true` by ref_return() (heap-typed return values
        // promoted to a hidden caller arg, e.g. `out: vector<integer>`).
        if !self.first_pass {
            let n_params = (0..self.data.attributes(d_nr))
                .filter(|&a| {
                    !self.data.attr_name(d_nr, a).starts_with("__")
                        && !self.data.def(d_nr).attributes()[a].hidden
                })
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
        let ret_type = self.data.def(d_nr).returned().clone();
        // @PLN25 E2 gap 3 — par over `vector<__nullable<S>>` whose worker takes a
        // dense `S`: the dispatcher hands the worker a ref to the element START
        // (the discriminant @0), but the worker reads dense-`S` offsets, so it
        // reads the disc.  Synthesize a thin wrapper
        // `__par_nullable_w(e: __nullable<S>, ..hidden) -> ret {
        // worker(<e payload>, ..hidden) }` and use IT as the worker func; the
        // body applies the SAME payload offset-ref coercion as a normal
        // `worker(v[i])` call (gap 2) and forwards the worker's params 1...  That
        // mirrored-param forwarding covers BOTH the `ref_return` hidden out-param
        // (struct/text returns) AND user EXTRA context args (`par(c = scale(a,
        // mult), …)`): the wrapper accepts `(e, ..worker-params-1..)` and the par
        // dispatcher supplies the element + the same extra_vals it would pass the
        // bare worker, so the layouts agree (22-threading "context arg" cases pass
        // gate-on both backends).
        // The element type is the inline `Enum(__nullable<S>, true)` for a `vector<S>` par, OR
        // `Reference(__nullable<S>)` for a KEYED par (hash/sorted/index): `materialise_keyed_for_par`
        // builds the temp vector with `Reference(content_d)` element refs.  Both need the wrapper
        // so the worker reads the dense-`S` payload, not the element's discriminant @0.
        let elem_enum_d = match elem_tp {
            Type::Enum(d, true, _) => Some(*d),
            Type::Reference(d, _) if self.data.def(*d).name().starts_with("__nullable<") => {
                Some(*d)
            }
            _ => None,
        };
        if !self.first_pass
            && let Some(enum_d) = elem_enum_d
            && self.data.attributes(d_nr) > 0
            && let Type::Reference(struct_d, _) =
                self.data.def(d_nr).attributes()[0].typedef.clone()
            && self.data.def(enum_d).name
                == format!("__nullable<{}>", self.data.def(struct_d).name())
            && self.data.attributes(struct_d) > 0
        {
            let w_d_nr = self
                .synth_nullable_par_wrapper(elem_tp, enum_d, struct_d, d_nr, &ret_type, first_id);
            return (w_d_nr, ret_type, extra_vals, extra_types);
        }
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

    /// @PLN25 E2 — synthesize `__par_nullable_w(e: __nullable<S>) -> ret {
    /// worker(<e Some-payload offset-coerced to ref(S)>) }`: the par wrapper
    /// that adapts a worker taking a dense `S` to a `vector<__nullable<S>>`
    /// element (whose element ref points at the discriminant @0).  Applies the
    /// SAME payload offset-ref coercion a normal `worker(v[i])` call does
    /// (gap 2).  Shared by both worker forms (`worker(a)` and `a.method()`).
    /// Returns the wrapper's def_nr.
    pub(crate) fn synth_nullable_par_wrapper(
        &mut self,
        elem_tp: &Type,
        enum_d: u32,
        struct_d: u32,
        worker_d_nr: u32,
        ret_type: &Type,
        label: &str,
    ) -> u32 {
        let pos = self.lexer.pos().clone();
        let wname = format!("__par_nullable_w_{}_{}_{label}", pos.line, pos.pos);
        let w_d_nr = self
            .data
            .add_def(&wname, &pos, crate::data::DefType::Function);
        let _ = self
            .data
            .add_attribute(&mut self.lexer, w_d_nr, "e", elem_tp.clone());
        self.data.set_returned(w_d_nr, ret_type.clone());
        let mut wvars = crate::variables::Function::new(&wname, &pos.file);
        let e_var = wvars.add_variable("e", elem_tp, &mut self.lexer);
        wvars.become_argument(e_var);
        wvars.defined(e_var);
        // Single-payload: the dense `S` is the `Some` variant's inline `payload` field, so the
        // offset-ref coercion sub-references `payload` (NOT S's first field, which is no longer
        // a direct `Some` field) — the same form the field-access unwrap (fields.rs) uses.
        let some_d = self.data.variant_of(enum_d, "Some");
        let off = self
            .database
            .position(self.data.def(some_d).known_type(), "payload");
        let coerced = self.get_val(
            &Type::Reference(struct_d, crate::data::Deps::none()),
            false,
            u32::from(off),
            Value::Var(e_var),
            u32::MAX,
        );
        // Mirror the worker's params 1.. onto the wrapper and forward them.  A
        // struct/text-returning worker carries a hidden `ref_return` out-param
        // (added during its own parse); the par dispatcher supplies it when it
        // calls the worker, so the wrapper — which the dispatcher now calls in
        // its place — must accept and forward it, or `generate_call` panics
        // "Too few parameters".  Param 0 (the dense `S`) is replaced by the
        // coerced Some-payload ref.
        let mut call_args = vec![coerced];
        let worker_attrs = self.data.attributes(worker_d_nr);
        for i in 1..worker_attrs {
            let pname = self.data.attr_name(worker_d_nr, i);
            let ptype = self.data.def(worker_d_nr).attributes()[i].typedef.clone();
            // Preserve the `hidden` flag — a struct/text worker's `__retbuf` out-param is
            // hidden, and the native par codegen (generation/ops/parallel.rs) detects the
            // per-element return buffer to ALLOCATE + PASS by filtering on `attr.hidden`.
            // A mirrored-but-unhidden `__retbuf` is skipped there, so the dispatcher calls
            // the wrapper WITHOUT the buffer ("takes 3 arguments but 2 were supplied" native;
            // "8 < 12" interp).
            let src_hidden = self.data.def(worker_d_nr).attributes()[i].hidden;
            let a = self
                .data
                .add_attribute(&mut self.lexer, w_d_nr, &pname, ptype.clone());
            if src_hidden {
                self.data.definitions[w_d_nr as usize].attributes[a].hidden = true;
            }
            let pvar = wvars.add_variable(&pname, &ptype, &mut self.lexer);
            wvars.become_argument(pvar);
            wvars.defined(pvar);
            call_args.push(Value::Var(pvar));
        }
        let body = crate::data::v_block(
            vec![Value::Return(Box::new(Value::Call(worker_d_nr, call_args)))],
            ret_type.clone(),
            "nullable_par_wrapper",
        );
        self.data.definitions[w_d_nr as usize].code = body;
        self.data.definitions[w_d_nr as usize].variables = wvars;
        w_d_nr
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
            self.parse_parallel_worker_fn(&first_id, elem_tp)
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
        let result_ref_type = Type::Reference(ref_d_nr, crate::data::Deps::none());
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
            let known = self.data.def(elm_td).known_type();
            let db_size = i32::from(self.database.size(known));
            if db_size > 0 {
                db_size
            } else {
                i32::from(var_size(&elem_tp, &Context::Argument))
            }
        };
        // A14.5/A14.6: check if the worker qualifies for the light path.
        // Light path: primitive return (not text, not reference), no recursive store alloc.
        // ARC.md A4 (closed 2026-05-07) — `n_parallel_for_light` was
        // retired and `n_parallel_for` is a panic stub.  This
        // user-facing `parallel_for(...)` builtin path has no
        // exerciser in the test corpus today; the `for ... par(...)`
        // clause goes through `build_parallel_for_ir` instead.  If
        // a future caller exercises this path, the panic stub fires
        // with a clear diagnostic pointing back to A4.  Keep the
        // routing wired so def-resolution succeeds even though the
        // landed call panics at runtime.
        let _ = worker_ret_type;
        let par_fn_name = "n_parallel_for";
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
        // Append any extra args (verified count above; types passed through).
        for extra in list.iter().skip(3) {
            augmented.push(extra.clone());
        }
        *val = Value::Call(par_for_d_nr, augmented);
        result_ref_type
    }

    // ARC.md A4 (closed 2026-05-07) — `check_light_eligible` and its
    // 5 call-graph-walk helpers (`has_recursive_allocation`,
    // `fn_allocates_stores`, `count_ref_vars`, `extract_callees`,
    // `collect_callees`) were removed when the light Concat path
    // was retired.  The eligibility check decided whether a worker
    // could skip per-thread store deep-copy by routing through the
    // light pool — irrelevant now that every par worker streams
    // through the Queue family.  ~110 LOC deleted.

    /// ARC.md A5 — user-facing `par_fold(items, init, fold, threads)`
    /// builtin.  Resolves the fold function reference to a d_nr at
    /// parse time and emits a single `Call` to `n_parallel_fold`.
    /// V1 restriction (mirrors the runtime, see
    /// `default/01_code.loft::parallel_fold`): `items` must be
    /// `vector<integer>`, `init` and the worker's accumulator /
    /// row / return types must all be `integer`.  Heterogeneous
    /// types (`vector<T>` + `fn(R, T) -> R`) await a follow-up
    /// when the runtime relaxes its V1 gate.
    ///
    /// Syntax: `par_fold(items, init, fn worker, threads)` —
    /// `fn worker` is the fn-reference form (parser converts to
    /// `Value::Int(d_nr)` with `Type::Function(...)`).
    pub(crate) fn parse_par_fold(
        &mut self,
        val: &mut Value,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        let int_tp = Type::Integer(crate::data::IntegerSpec::wide());
        if self.first_pass {
            return int_tp;
        }
        if list.len() != 4 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "par_fold requires exactly 4 arguments: items, init, fn fold, threads"
            );
            return Type::Unknown(0);
        }
        // V1: items must be vector<integer>.
        let elem_tp = if let Type::Vector(elem, _) = &types[0] {
            (**elem).clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "par_fold: first argument must be a vector<integer>"
            );
            return Type::Unknown(0);
        };
        if !matches!(elem_tp, Type::Integer(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "par_fold (V1): items element type must be integer; got {}",
                elem_tp.name(&self.data)
            );
            return Type::Unknown(0);
        }
        // V1: init must be integer.
        if !matches!(types[1], Type::Integer(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "par_fold (V1): init must be integer; got {}",
                types[1].name(&self.data)
            );
            return Type::Unknown(0);
        }
        // V1: fold must be fn(integer, integer) -> integer.
        let (fn_args, fn_ret) = if let Type::Function(args, ret, _) = &types[2] {
            (args.clone(), (**ret).clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "par_fold: third argument must be a function reference (use `fn <name>`)"
            );
            return Type::Unknown(0);
        };
        if fn_args.len() != 2
            || !matches!(fn_args[0], Type::Integer(_))
            || !matches!(fn_args[1], Type::Integer(_))
            || !matches!(fn_ret, Type::Integer(_))
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "par_fold (V1): fold function must have signature `fn(integer, integer) -> integer`"
            );
            return Type::Unknown(0);
        }
        // threads must be integer.
        if !matches!(types[3], Type::Integer(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "par_fold: threads must be integer; got {}",
                types[3].name(&self.data)
            );
            return Type::Unknown(0);
        }
        let par_fold_d_nr = self.data.def_nr("n_parallel_fold");
        if par_fold_d_nr == u32::MAX {
            diagnostic!(
                self.lexer,
                Level::Error,
                "internal error: n_parallel_fold not found"
            );
            return Type::Unknown(0);
        }
        // Pop order in the runtime (top of stack first):
        //   n_extra → extras... → threads → fold → init → input
        // So the parser pushes (left-to-right) in declared order:
        //   input, init, fold, threads, then `n_extra` (= 0 for V1).
        // The `n_extra` count is emitted explicitly here, mirroring
        // `build_parallel_for_ir` at `src/parser/collections.rs:1902`
        // (the for-par desugar's analogous emit).  Codegen at
        // `src/state/codegen.rs:2007` would also generate any extras
        // beyond the 4 declared params, but the n_extra COUNT itself
        // must come from here.
        *val = Value::Call(
            par_fold_d_nr,
            vec![
                list[0].clone(), // input
                list[1].clone(), // init
                list[2].clone(), // fold (Value::Int(d_nr) — bare fn name)
                list[3].clone(), // threads
                Value::Int(0),   // n_extra (V1: no extras)
            ],
        );
        int_tp
    }
}
