// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Pre-evaluation pass: hoist complex subexpressions into temp variables
//! before emitting the main code.  This avoids nested borrow conflicts in
//! generated Rust code where `stores` would be borrowed mutably twice.

use crate::data::{Type, Value};
use std::io::Write;

use super::{Output, PreEvalEntry, narrow_int_cast, sanitize};

impl Output<'_> {
    /// Use this instead of emitting an argument block when the block exists only to pass a
    /// local text variable by mutable reference. Returns the variable index so the call site
    /// can emit `&mut var_<name>` without generating a spurious empty block expression.
    pub(super) fn create_stack_var(&self, v: &Value) -> Option<u16> {
        // Direct OpCreateStack call on a variable (text or numeric by-ref): `fn f(x: &T)` called as `f(v)`.
        // The parser wraps the argument as Value::Call("OpCreateStack", [Value::Var(n)]).
        // output_call emits nothing for OpCreateStack, so we must intercept here and emit
        // `&mut var_<name>` instead.
        if let Value::Call(d_nr, args) = v
            && self.data.def(*d_nr).name == "OpCreateStack"
            && let [Value::Var(nr)] = args.as_slice()
        {
            return Some(*nr);
        }
        let Value::Block(bl) = v else { return None };
        // Handle DbRef-stack refs: Type::Reference with OpCreateStack ops.
        if let Type::Reference(_, vars) = &bl.result {
            let [vr] = vars.as_slice() else { return None };
            let only_create_stack = bl
                .operators
                .iter()
                .filter(|op| !matches!(op, Value::Line(_)))
                .all(|op| matches!(op, Value::Call(d_nr, _) if self.data.def(*d_nr).name == "OpCreateStack"));
            return only_create_stack.then_some(*vr);
        }
        None
    }

    /// Fix the "hoisted return value" pattern inserted by `scopes::free_vars`.
    ///
    /// When a function returns early (`return expr`) and has local text/ref variables
    /// that need cleanup, `scopes::free_vars` transforms the return into:
    ///   `[expr, OpFreeText(v)…, Return(Null)]`
    /// so the interpreter can push `expr` onto the stack before freeing locals and returning.
    ///
    /// In native Rust code, `OpFreeText` is a no-op (Rust drops automatically), so the
    /// pattern degenerates to `expr; return ();` which drops the return value and fails to
    /// compile when the function return type is not void.
    ///
    /// This method detects the pattern in a slice of block operators and returns a patched
    /// copy where `Return(Null)` is replaced by `Return(expr)` and `expr` is removed from
    /// its earlier position.
    pub(super) fn patch_hoisted_returns<'a>(
        &self,
        ops: &'a [Value],
    ) -> std::borrow::Cow<'a, [Value]> {
        let fn_returned = &self.data.def(self.def_nr).returned;
        if matches!(fn_returned, Type::Void) {
            return std::borrow::Cow::Borrowed(ops);
        }
        // Quick check: is there any Return(Null) at all?
        if !ops
            .iter()
            .any(|op| matches!(op, Value::Return(v) if **v == Value::Null))
        {
            return std::borrow::Cow::Borrowed(ops);
        }
        let is_free_op = |op: &Value| {
            if let Value::Call(d, _) = op {
                let name = &self.data.def(*d).name;
                name == "OpFreeText" || name == "OpFreeRef"
            } else {
                false
            }
        };
        let mut result: Vec<Value> = ops.to_vec();
        // Process all Return(Null) occurrences (usually just one).
        let mut search_from = 0;
        while let Some(ret_pos) = result[search_from..]
            .iter()
            .position(|op| matches!(op, Value::Return(v) if **v == Value::Null))
            .map(|p| p + search_from)
        {
            // Find the nearest preceding expression that is not a free-op, Line, or Return.
            let expr_pos = result[..ret_pos]
                .iter()
                .rposition(|op| !matches!(op, Value::Line(_)) && !is_free_op(op));
            if let Some(idx) = expr_pos {
                let expr = result.remove(idx);
                // ret_pos shifted by -1 because we removed one element before it.
                let actual_ret = ret_pos - 1;
                result[actual_ret] = Value::Return(Box::new(expr));
                search_from = actual_ret + 1;
            } else {
                search_from = ret_pos + 1;
            }
        }
        std::borrow::Cow::Owned(result)
    }

    /// Use this to detect sub-expressions that would cause a double-borrow of `stores`
    /// if left inline and must therefore be hoisted into `let _preN` bindings.
    /// Returns true if the named native Op function uses `stores` in its special-case emit code.
    /// These functions need pre-eval treatment to avoid double-borrow of `stores` when they
    /// appear as arguments inside other stores-using calls.
    fn op_uses_stores(name: &str) -> bool {
        matches!(
            name,
            "OpNewRecord"
                | "OpFinishRecord"
                | "OpGetRecord"
                | "OpIterate"
                | "OpDatabase"
                | "OpCopyRecord"
                | "OpSizeofRef"
                | "OpStep"
                | "OpRemove"
                | "OpHashRemove"
                | "OpAppendCopy"
                | "OpFormatDatabase"
                | "OpFormatStackDatabase"
        )
    }

    fn needs_pre_eval(&self, v: &Value) -> bool {
        match v {
            Value::Call(d_nr, vals) => {
                let def = self.data.def(*d_nr);
                // User-defined functions (rust template is empty AND have loft code body)
                // always need pre-eval to avoid double-borrow.
                if def.rust.is_empty() && def.code != Value::Null {
                    true
                } else if def.rust.contains("stores") {
                    // Template fns that use `stores` can cause double-borrow when nested
                    // inside another stores-using call; treat them as needing pre-eval.
                    true
                } else if def.rust.is_empty() && Self::op_uses_stores(&def.name) {
                    // Native Op functions whose special-case emit code passes `stores`
                    // also cause double-borrow when nested inside other stores-using calls.
                    true
                } else if def.rust.is_empty()
                    && def.code == Value::Null
                    && !def.name.starts_with("Op")
                {
                    // User-fn stubs (no rust template, no loft body, not a built-in Op)
                    // are emitted as todo!() but still take `&mut Stores` — pre-eval
                    // them to avoid double-borrow when they appear as nested arguments.
                    true
                } else {
                    vals.iter().any(|a| self.needs_pre_eval(a))
                }
            }
            // CallRef dispatches via match to user functions that take &mut Stores.
            // Block, Insert, and Iter contain statements that use stores.
            Value::Block(_) | Value::CallRef(_, _) | Value::Insert(_) | Value::Iter(..) => true,
            Value::If(test, t, f) => {
                self.needs_pre_eval(test) || self.needs_pre_eval(t) || self.needs_pre_eval(f)
            }
            Value::Drop(v) => self.needs_pre_eval(v),
            _ => false,
        }
    }

    /// Use this when you need the generated text of an expression for substitution or comparison,
    /// rather than writing it directly to the output stream.
    pub(super) fn generate_expr_buf(&mut self, v: &Value) -> std::io::Result<String> {
        let mut buf = std::io::BufWriter::new(Vec::new());
        self.output_code_inner(&mut buf, v)?;
        Ok(String::from_utf8(buf.into_inner()?).unwrap())
    }

    /// Use this to identify all sub-expressions in `v` that must be hoisted before the enclosing
    /// expression to prevent simultaneous `&mut Stores` borrows.
    /// Returns `(var_name, expr_code)` pairs ordered innermost-first so each pre-eval
    /// can safely reference earlier ones.
    pub(super) fn collect_pre_evals(&mut self, v: &Value) -> std::io::Result<Vec<PreEvalEntry>> {
        let mut result = Vec::new();
        self.collect_pre_evals_inner(v, &mut result)?;
        Ok(result)
    }

    /// Use this as the recursive worker for `collect_pre_evals`.
    /// Splitting from the wrapper keeps the result `Vec` allocated once, and the pre-eval
    ///  counter is globally unique within a block.
    fn collect_pre_evals_inner(
        &mut self,
        v: &Value,
        result: &mut Vec<PreEvalEntry>,
    ) -> std::io::Result<()> {
        // Recurse into wrapper nodes so nested Call nodes inside Set/Drop/If are found.
        if let Value::Set(_, rhs) = v {
            return self.collect_pre_evals_inner(rhs, result);
        }
        if let Value::Drop(inner) | Value::Return(inner) = v {
            return self.collect_pre_evals_inner(inner, result);
        }
        if let Value::If(test, true_v, false_v) = v {
            self.collect_pre_evals_inner(test, result)?;
            self.collect_pre_evals_inner(true_v, result)?;
            return self.collect_pre_evals_inner(false_v, result);
        }
        // CallRef dispatches to user functions — same hoisting rules as user-defined Call.
        // The closure arg appears once per candidate match arm (all arms receive the same
        // allocation block), so use replace_all=true to substitute every occurrence.
        if let Value::CallRef(_, args) = v {
            for arg in args {
                let needs_pre = self.create_stack_var(arg).is_none()
                    && (matches!(arg, Value::Block(_) | Value::Insert(_))
                        || self.needs_pre_eval(arg));
                if needs_pre {
                    let name = format!("_pre_{}", self.counter);
                    self.counter += 1;
                    self.rewrite_code(result, arg, name, true)?;
                } else {
                    self.collect_pre_evals_inner(arg, result)?;
                }
            }
            return Ok(());
        }
        if let Value::Call(d_nr, vals) = v {
            let def_fn = self.data.def(*d_nr);
            if def_fn.rust.is_empty() {
                // User-defined function: pre-eval any Block or nested user-fn arguments
                // (both cause double-borrow of stores if left inline).
                for arg in vals {
                    let needs_pre = self.create_stack_var(arg).is_none()
                        && (matches!(arg, Value::Block(_) | Value::Insert(_))
                            || self.needs_pre_eval(arg));
                    if needs_pre {
                        let name = format!("_pre_{}", self.counter);
                        self.counter += 1;
                        self.rewrite_code(result, arg, name, false)?;
                    } else {
                        self.collect_pre_evals_inner(arg, result)?;
                    }
                }
            } else {
                // Template function: pre-eval Block args (they may use stores) and,
                // when multiple user-fn args exist, pre-eval those too to avoid
                // double-borrow of stores.
                let block_count = vals.iter().filter(|a| matches!(a, Value::Block(_))).count();
                let user_fn_count = vals.iter().filter(|a| self.needs_pre_eval(a)).count();
                // Also pre-eval any arg whose template placeholder appears more than once
                // (e.g., `#rust"!@v1.is_nan() && ... @v1 ..."` expands @v1 twice, causing
                // double-borrow when @v1 is a user-fn call returning stores-backed data).
                let has_dup_param = def_fn.attributes.iter().enumerate().any(|(i, a)| {
                    let placeholder = format!("@{}", a.name);
                    i < vals.len()
                        && def_fn.rust.matches(placeholder.as_str()).count() > 1
                        && self.needs_pre_eval(&vals[i])
                });
                let template_uses_stores = def_fn.rust.contains("stores");
                let needs_pre_eval_args = block_count > 0
                    || user_fn_count > 1
                    || (template_uses_stores && user_fn_count > 0)
                    || has_dup_param;
                if needs_pre_eval_args {
                    for (arg_idx, arg) in vals.iter().enumerate() {
                        let is_block = matches!(arg, Value::Block(_));
                        let is_multi_user_fn = user_fn_count > 1 && self.needs_pre_eval(arg);
                        let is_stores_conflict = template_uses_stores && self.needs_pre_eval(arg);
                        let is_dup = if arg_idx < def_fn.attributes.len() {
                            let placeholder = format!("@{}", def_fn.attributes[arg_idx].name);
                            def_fn.rust.matches(placeholder.as_str()).count() > 1
                                && self.needs_pre_eval(arg)
                        } else {
                            false
                        };
                        if is_block || is_multi_user_fn || is_stores_conflict || is_dup {
                            let name = format!("_pre_{}", self.counter);
                            self.counter += 1;
                            self.rewrite_code(result, arg, name, is_dup)?;
                        } else {
                            self.collect_pre_evals_inner(arg, result)?;
                        }
                    }
                } else {
                    for arg in vals {
                        self.collect_pre_evals_inner(arg, result)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Use this to register one pre-eval binding: generate the expression text with inner
    /// pre-evals already substituted, then push `(name, code)` onto `result`.
    fn rewrite_code(
        &mut self,
        result: &mut Vec<PreEvalEntry>,
        arg: &Value,
        name: String,
        replace_all: bool,
    ) -> std::io::Result<()> {
        // Collect inner pre-evals first, so the pre-eval code itself
        // is free of double borrows.
        let decl_clone = self.declared.clone();
        let start_idx = result.len();
        self.collect_pre_evals_inner(arg, result)?;
        // Propagate replace_all flag: if this pre-eval is a dup-param (replace_all=true),
        // all its inner pre-evals must also use replace_all so that progressive substitution
        // correctly transforms all N occurrences of the dup arg in the outer expression.
        if replace_all {
            for entry in &mut result[start_idx..] {
                entry.4 = true;
            }
        }
        let inner_pre_evals = result[start_idx..].to_vec();
        // Save counter state before generating the expression text;
        // output_block will restore to this value before output_code_with_subst
        // so the block inner pre-eval names (_pre_N) match in both passes.
        let counter_before_gen = self.counter;
        let raw_code = self.generate_expr_buf(arg)?;
        let substituted = if inner_pre_evals.is_empty() {
            raw_code
        } else {
            let mut s = raw_code;
            for (pre_name, pre_code, _, _, inner_replace_all) in &inner_pre_evals {
                if *inner_replace_all {
                    // Dup-param inner pre-eval: the arg code appears multiple times
                    // in the binding code (template expanded @v1 twice), replace all.
                    s = s.replace(pre_code.as_str(), pre_name.as_str());
                } else {
                    // Normal inner pre-eval: appears once, use replace-first.
                    s = s.replacen(pre_code.as_str(), pre_name.as_str(), 1);
                }
            }
            s
        };
        // When the argument type is a narrow integer (u8/u16/i8/i16), the Rust binding
        // would have a narrow type.  Pre-eval bindings must have type i32 so they
        // compare correctly against i32 expressions.  Compute a separate bind_code that
        // wraps the expression with `as i32`; the match_code (used for substitution)
        // is left unchanged so string replacement in the outer code still works.
        let bind_code = if !substituted.is_empty() && substituted != "()" {
            if let Some(tp) = self.infer_type(arg) {
                if narrow_int_cast(&tp).is_some() {
                    format!("({substituted}) as i32")
                } else if matches!(tp, Type::Text(_))
                    && matches!(arg, Value::Call(d, _) if
                        matches!(self.data.def(*d).returned, Type::Text(_))
                        && self.data.def(*d).rust.is_empty()
                        && !self.data.def(*d).name.starts_with("Op"))
                {
                    // Text-returning user fn calls produce `Str`; callees
                    // expect `&str`.  Deref at the binding site.
                    format!("&*({substituted})")
                } else {
                    substituted.clone()
                }
            } else {
                substituted.clone()
            }
        } else {
            substituted.clone()
        };
        if !substituted.is_empty() && substituted != "()" {
            result.push((
                name,
                substituted,
                bind_code,
                counter_before_gen,
                replace_all,
            ));
        }
        self.declared = decl_clone;
        Ok(())
    }

    /// Use this instead of `output_code_inner` when `pre_evals` is non-empty.
    /// Without substitution the same expression would be emitted twice, causing a second
    /// mutable borrow of `stores`.
    #[allow(clippy::too_many_lines)]
    pub(super) fn output_code_with_subst(
        &mut self,
        w: &mut dyn Write,
        v: &Value,
        pre_evals: &[(String, String, String, u32, bool)],
    ) -> std::io::Result<()> {
        if pre_evals.is_empty() {
            self.output_code_inner(w, v)?;
            return Ok(());
        }
        // For If expressions, apply substitution structurally rather than via string
        // replacement on the full generated text.  String-level substitution on the full
        // if-else tree corrupts the `let _pre_N = …;` declarations that inner Block
        // branches emit for their own operators: those declarations contain the same
        // raw code strings (e.g. `get_int_8_code`) as the outer pre-evals, so a
        // replacen call intended for the outer condition accidentally replaces the inner
        // declaration, making the inner variable a stale alias of the outer pre-eval.
        //
        // The structural fix: apply substitution only to the *condition* part of the If
        // (and recursively to any else-if conditions); emit Block branches directly via
        // `output_code_inner`, which calls `output_block` and manages their own
        // pre-evals internally.
        if let Value::If(test, true_v, false_v) = v {
            // Check exact match first: if this entire If expression equals a pre-eval
            // binding, emit the name.  Save/restore counter and declared so the check
            // pass does not corrupt state for the real structural emission below.
            let saved_counter = self.counter;
            let saved_declared = self.declared.clone();
            let mut check_buf = std::io::BufWriter::new(Vec::new());
            self.output_code_inner(&mut check_buf, v)?;
            let full_code = String::from_utf8(check_buf.into_inner()?).unwrap();
            self.counter = saved_counter;
            self.declared = saved_declared;
            for (name, pre_code, _, _, _) in pre_evals {
                if full_code == *pre_code {
                    write!(w, "{name}")?;
                    return Ok(());
                }
            }
            return self.output_if_with_subst(w, test, true_v, false_v, pre_evals);
        }
        // For calls to user-defined functions, apply substitution structurally per
        // argument.  String-level substitution on the full call text fails when a
        // `Value::Block` argument emits counter-dependent inner pre-eval names that
        // differ between the collect pass (high counter stored in pre_evals) and
        // the regeneration pass (counter reset to counter_before).  Without structural
        // handling the block is emitted inline, causing a double `&mut stores` borrow.
        //
        // Built-in opcodes (names starting with "Op") are handled by special-case
        // logic in `output_call` and must NOT be intercepted here; they fall through
        // to string-level substitution below.
        if let Value::Call(d_nr, vals) = v {
            let def_fn = self.data.def(*d_nr);
            if def_fn.rust.is_empty() && !def_fn.name.starts_with("Op") {
                // Full-expression match: if this entire call equals a pre-eval, emit the name.
                let saved_counter = self.counter;
                let saved_declared = self.declared.clone();
                let mut check_buf = std::io::BufWriter::new(Vec::new());
                self.output_code_inner(&mut check_buf, v)?;
                let full_code = String::from_utf8(check_buf.into_inner()?).unwrap();
                self.counter = saved_counter;
                self.declared = saved_declared;
                for (name, pre_code, _, _, _) in pre_evals {
                    if full_code == *pre_code {
                        write!(w, "{name}")?;
                        return Ok(());
                    }
                }
                // Structural emission: emit each argument with per-arg substitution.
                let fn_name = def_fn.name.clone();
                write!(w, "{fn_name}(stores")?;
                for (idx, val) in vals.iter().enumerate() {
                    write!(w, ", ")?;
                    if let Some(vr) = self.create_stack_var(val) {
                        let vname = sanitize(self.data.def(self.def_nr).variables.name(vr));
                        write!(w, "&mut var_{vname}")?;
                    } else {
                        let matched = self.try_subst_pre_eval(w, val, pre_evals)?;
                        if !matched {
                            // Emit the argument with string-level substitution.
                            let saved_c = self.counter;
                            let saved_d = self.declared.clone();
                            let mut buf = std::io::BufWriter::new(Vec::new());
                            self.output_code_inner(&mut buf, val)?;
                            let mut arg_code = String::from_utf8(buf.into_inner()?).unwrap();
                            self.counter = saved_c;
                            self.declared = saved_d;
                            for (pname, pcode, _, _, replace_all) in pre_evals {
                                if *replace_all {
                                    arg_code = arg_code.replace(pcode.as_str(), pname.as_str());
                                } else {
                                    arg_code = arg_code.replacen(pcode.as_str(), pname.as_str(), 1);
                                }
                            }
                            // set fn_ref_context for fn-ref parameter evaluation.
                            let param_is_fnref = idx < self.data.def(*d_nr).attributes.len()
                                && matches!(
                                    self.data.def(*d_nr).attributes[idx].typedef,
                                    Type::Function(_, _, _)
                                );
                            let param_is_routine = idx < self.data.def(*d_nr).attributes.len()
                                && matches!(
                                    self.data.def(*d_nr).attributes[idx].typedef,
                                    Type::Routine(_)
                                );
                            if param_is_fnref && matches!(val, Value::Int(_)) {
                                write!(
                                    w,
                                    "({arg_code} as u32, loft::keys::DbRef {{ store_nr: u16::MAX, rec: 0, pos: 0 }})"
                                )?;
                            } else if param_is_routine && matches!(val, Value::Int(_)) {
                                write!(w, "{arg_code} as u32")?;
                            } else {
                                write!(w, "{arg_code}")?;
                            }
                        }
                    }
                }
                write!(w, ")")?;
                // Add narrow-int cast if the user function returns a narrow int type.
                if narrow_int_cast(&self.data.def(*d_nr).returned).is_some() {
                    write!(w, " as i32")?;
                }
                return Ok(());
            }
        }
        let mut buf_check = std::io::BufWriter::new(Vec::new());
        self.output_code_inner(&mut buf_check, v)?;
        let code = String::from_utf8(buf_check.into_inner()?).unwrap();
        for (name, pre_code, _, _, _) in pre_evals {
            if code == *pre_code {
                write!(w, "{name}")?;
                return Ok(());
            }
        }
        let mut result = code;
        for (name, pre_code, _, _, replace_all) in pre_evals {
            if *replace_all {
                // Dup-param: the same arg code appears multiple times in a template
                // expansion; replace all occurrences so the pre-eval is used everywhere.
                result = result.replace(pre_code.as_str(), name.as_str());
            } else {
                // Normal pre-eval: use replace-first so that identical code strings
                // inside nested block pre-eval declarations are NOT substituted.
                // Multiple pre-evals with the same binding code (one per usage site)
                // are generated by the caller and each replaces exactly one occurrence.
                result = result.replacen(pre_code.as_str(), name.as_str(), 1);
            }
        }
        write!(w, "{result}")?;
        Ok(())
    }

    /// Use this to emit an `if`/`else` expression with pre-eval substitution applied
    /// structurally: the condition receives substitution, Block branches are emitted
    /// directly (they handle their own pre-evals via `output_block`), and non-Block
    /// branches (else-if chains) receive substitution recursively.
    fn output_if_with_subst(
        &mut self,
        w: &mut dyn Write,
        test: &Value,
        true_v: &Value,
        false_v: &Value,
        pre_evals: &[(String, String, String, u32, bool)],
    ) -> std::io::Result<()> {
        write!(w, "if ")?;
        let b_true = matches!(*true_v, Value::Block(_));
        let b_false = matches!(*false_v, Value::Block(_));
        // Condition: apply substitution (this is exactly what the pre-evals are for).
        self.output_code_with_subst(w, test, pre_evals)?;
        if b_true {
            write!(w, " ")?;
        } else {
            write!(w, " {{")?;
        }
        self.indent += u32::from(!b_true);
        if b_true {
            // Block branch: manages its own pre-evals, no outer substitution needed.
            self.output_code_inner(w, true_v)?;
        } else {
            self.output_code_with_subst(w, true_v, pre_evals)?;
        }
        self.indent -= u32::from(!b_true);
        if let Value::Block(_) = *true_v {
            write!(w, " else ")?;
        } else {
            write!(w, "}} else ")?;
        }
        if !b_false {
            write!(w, "{{")?;
        }
        self.indent += u32::from(!b_false);
        if matches!(false_v, Value::Null)
            && let Some(tp) = self.infer_type(true_v)
        {
            Self::write_typed_null(w, &tp)?;
        } else if b_false {
            // Block branch: manages its own pre-evals, no outer substitution needed.
            self.output_code_inner(w, false_v)?;
        } else {
            // Non-block false branch (else-if chain or leaf): apply substitution.
            self.output_code_with_subst(w, false_v, pre_evals)?;
        }
        self.indent -= u32::from(!b_false);
        if !b_false {
            write!(w, "}}")?;
        }
        Ok(())
    }

    /// Try to match `val` against one of the pre-eval bindings by regenerating `val`
    /// at the counter state stored when that pre-eval was collected.  If a match is
    /// found the pre-eval name is written to `w` and `Ok(true)` is returned.
    ///
    /// This is used by the structural `Value::Call` handler in `output_code_with_subst`
    /// to match block-typed arguments whose inner counter-dependent names differ between
    /// the collect pass (high counter) and the regeneration pass (reset counter).
    fn try_subst_pre_eval(
        &mut self,
        w: &mut dyn Write,
        val: &Value,
        pre_evals: &[(String, String, String, u32, bool)],
    ) -> std::io::Result<bool> {
        for (pre_name, pre_code, _, pre_counter, _) in pre_evals {
            let saved_counter = self.counter;
            let saved_declared = self.declared.clone();
            let saved_indent = self.indent;
            self.counter = *pre_counter;
            let mut check_buf = std::io::BufWriter::new(Vec::new());
            let _ = self.output_code_inner(&mut check_buf, val);
            let arg_code =
                String::from_utf8(check_buf.into_inner().unwrap_or_default()).unwrap_or_default();
            self.counter = saved_counter;
            self.declared = saved_declared;
            self.indent = saved_indent;
            if arg_code == *pre_code {
                write!(w, "{pre_name}")?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Use this to determine whether a value produces no Rust result (type `()`).
    /// Needed by `output_block` to find the last non-void expression that should be the
    /// block's return value.
    pub(super) fn is_void_value(&self, v: &Value) -> bool {
        match v {
            // N8a.2: TuplePut is an assignment statement (void), not a return expression.
            Value::Null
            | Value::Drop(_)
            | Value::Set(_, _)
            | Value::Line(_)
            | Value::TuplePut(_, _, _) => true,
            Value::If(_, _, false_v) => matches!(**false_v, Value::Null),
            Value::Call(d_nr, _) => {
                let def = self.data.def(*d_nr);
                matches!(def.returned, Type::Void)
            }
            Value::Block(bl) => matches!(bl.result, Type::Void),
            _ => false,
        }
    }
}
