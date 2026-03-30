// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Core IR-to-Rust emission: translates `Value` IR nodes into Rust source.

use crate::data::{Block, Context, Type, Value};
use std::io::Write;

use super::text::count_format_ops;
use super::{Output, default_native_value, narrow_int_cast, rust_type, sanitize};

impl Output<'_> {
    /// Central recursive dispatch from a `Value` node to its Rust representation.
    /// All emit functions ultimately call this; complex variants are delegated to
    /// dedicated helpers to keep each match arm concise.
    #[allow(clippy::too_many_lines)]
    pub(super) fn output_code_inner(
        &mut self,
        w: &mut dyn Write,
        code: &Value,
    ) -> std::io::Result<()> {
        match code {
            Value::Text(txt) => {
                // Use debug format to produce a properly escaped Rust string literal.
                write!(w, "{txt:?}")?;
            }
            Value::Long(v) => write!(w, "{v}_i64")?,
            Value::Int(v) => write!(w, "{v}_i32")?,
            Value::Enum(v, _) => write!(w, "{v}_u8")?,
            Value::Boolean(v) => write!(w, "{v}")?,
            Value::Float(v) => write!(w, "{v}_f64")?,
            Value::Single(v) => write!(w, "{v}_f32")?,
            Value::Null => write!(w, "()")?,
            Value::Line(_) => {}
            Value::Break(n) => {
                if *n == 0 || self.loop_stack.is_empty() {
                    write!(w, "break")?;
                } else {
                    let idx = self.loop_stack.len().saturating_sub(*n as usize + 1);
                    write!(w, "break 'l{}", self.loop_stack[idx])?;
                }
            }
            Value::Continue(n) => {
                if *n == 0 || self.loop_stack.is_empty() {
                    write!(w, "continue")?;
                } else {
                    let idx = self.loop_stack.len().saturating_sub(*n as usize + 1);
                    write!(w, "continue 'l{}", self.loop_stack[idx])?;
                }
            }
            Value::Drop(v) => self.output_code_inner(w, v)?,
            Value::Insert(ops) => {
                for (vnr, v) in ops.iter().enumerate() {
                    self.indent(w)?;
                    self.indent += 1;
                    self.output_code_inner(w, v)?;
                    self.indent -= 1;
                    if vnr < ops.len() - 1 {
                        writeln!(w, ";")?;
                    } else {
                        writeln!(w)?;
                    }
                }
            }
            Value::Block(bl) => self.output_block(w, bl, false)?,
            Value::Loop(lp) => {
                self.loop_stack.push(lp.scope);
                writeln!(w, "'l{}: loop {{ //{}_{}", lp.scope, lp.name, lp.scope)?;
                for v in &lp.operators {
                    self.indent(w)?;
                    self.indent += 1;
                    self.output_code_inner(w, v)?;
                    self.indent -= 1;
                    writeln!(w, ";")?;
                }
                self.indent(w)?;
                write!(w, "}} /*{}_{}*/", lp.name, lp.scope)?;
                self.loop_stack.pop();
            }
            Value::Set(var, to) => self.output_set(w, *var, to)?,
            Value::Var(var) => {
                let variables = &self.data.def(self.def_nr).variables;
                let var_name = sanitize(variables.name(*var));
                if variables.is_argument(*var) {
                    if let Type::RefVar(inner) = variables.tp(*var) {
                        // By-ref argument: variable holds &mut T — dereference to read value.
                        if matches!(**inner, Type::Text(_)) {
                            // Text RefVar: deref &mut String to &str via &*
                            write!(w, "&*var_{var_name}")?;
                        } else {
                            write!(w, "*var_{var_name}")?;
                        }
                    } else if matches!(variables.tp(*var), Type::Text(_)) {
                        // Text params are `&str` — already a reference, no prefix needed.
                        write!(w, "var_{var_name}")?;
                    } else {
                        write!(w, "var_{var_name}")?;
                    }
                } else if matches!(variables.tp(*var), Type::Text(_)) {
                    // Text locals are `String` — add `&` to coerce to `&str`.
                    write!(w, "&var_{var_name}")?;
                } else {
                    write!(w, "var_{var_name}")?;
                }
            }
            Value::If(test, true_v, false_v) => self.output_if(w, test, true_v, false_v)?,
            Value::Call(def_nr, vals) => {
                self.output_call(w, *def_nr, vals)?;
            }
            Value::Return(val) => {
                let returned = &self.data.def(self.def_nr).returned;
                let returns_text = matches!(returned, Type::Text(_));
                let narrow = narrow_int_cast(returned);
                write!(w, "return ")?;
                if returns_text {
                    write!(w, "Str::new(")?;
                } else if narrow.is_some() {
                    write!(w, "(")?;
                }
                self.output_code_inner(w, val)?;
                if returns_text {
                    write!(w, ")")?;
                } else if let Some(cast) = narrow {
                    write!(w, ") as {cast}")?;
                }
            }
            Value::Keys(keys) => {
                write!(w, "&[")?;
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        write!(w, ", ")?;
                    }
                    write!(
                        w,
                        "Key {{ type_nr: {}, position: {} }}",
                        k.type_nr, k.position
                    )?;
                }
                write!(w, "]")?;
            }
            Value::CallRef(v_nr, args) => {
                self.output_call_ref(w, *v_nr, args)?;
            }
            Value::Iter(..) => write!(w, "{code:?}")?,
            Value::Tuple(elems) => {
                write!(w, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(w, ", ")?;
                    }
                    self.output_code_inner(w, e)?;
                }
                write!(w, ")")?;
            }
            Value::TupleGet(var, idx) => {
                // N8a.2: use the variable's declared name (like all other emitters),
                // not its internal number.  Before this fix `var_0.0` was emitted even
                // when the parameter was declared as `var_pair`.
                let variables = &self.data.def(self.def_nr).variables;
                let name = sanitize(variables.name(*var));
                write!(w, "var_{name}.{idx}")?;
            }
            Value::TuplePut(var, idx, val) => {
                // N8a.2: emit the actual element assignment instead of the `= ...` stub.
                // TuplePut is a void statement (assignment); is_void_value returns true for it
                // so the block emitter adds `;` and does not treat it as the return value.
                let variables = &self.data.def(self.def_nr).variables;
                let name = sanitize(variables.name(*var));
                write!(w, "var_{name}.{idx} = ")?;
                self.output_code_inner(w, val)?;
            }
            Value::Yield(inner) => {
                if self.yield_collect {
                    // Inside a ForLoopBody factory: push to the collector instead.
                    write!(w, "__values.push((")?;
                    self.output_code_inner(w, inner)?;
                    write!(w, ") as i64)")?;
                } else {
                    write!(w, "yield ")?;
                    self.output_code_inner(w, inner)?;
                }
            }
        }
        Ok(())
    }

    /// Emit a call through a fn-ref variable (`Value::CallRef`).
    /// The variable `v_nr` holds a `u32` definition number at runtime.
    /// We enumerate all reachable definitions with a matching signature and
    /// generate a `match` dispatch.
    fn output_call_ref(
        &mut self,
        w: &mut dyn Write,
        v_nr: u16,
        args: &[Value],
    ) -> std::io::Result<()> {
        let variables = &self.data.def(self.def_nr).variables;
        let var_name = sanitize(variables.name(v_nr));
        let fn_type = variables.tp(v_nr).clone();
        let (param_types, ret_type) = if let Type::Function(p, r) = &fn_type {
            (p.clone(), *r.clone())
        } else {
            // Not a function type — fall back to debug print.
            write!(w, "{:?}", crate::data::Value::CallRef(v_nr, args.to_vec()))?;
            return Ok(());
        };
        // Collect all definitions with a matching signature.
        // Only include native-callable functions (n_ / t_ prefix) in the reachable set;
        // bytecode ops (Op* prefix) are never callable via fn-refs in native mode.
        let n_defs = self.data.definitions();
        // (d_nr, fn_name, has_closure): has_closure=true when the last attribute is __closure.
        let mut candidates: Vec<(u32, String, bool)> = Vec::new();
        for d in 0..n_defs {
            if !self.reachable.is_empty() && !self.reachable.contains(&d) {
                continue;
            }
            let def = self.data.def(d);
            if !matches!(def.def_type, crate::data::DefType::Function) {
                continue;
            }
            // Exclude bytecode ops (Op* prefix) — they are not callable in native mode.
            if def.name.starts_with("Op") {
                continue;
            }
            // A5.6g: closure-capturing lambdas have a hidden __closure param as the last
            // attribute. The closure is injected explicitly at the call site (in arg_exprs),
            // so total arg count must equal the full attribute count.
            let has_closure = def
                .attributes
                .last()
                .map_or(false, |a| a.name == "__closure");
            let visible_attr_count = if has_closure {
                def.attributes.len() - 1
            } else {
                def.attributes.len()
            };
            // Total attribute count must equal total args (closure included for closures).
            if def.attributes.len() != args.len() {
                continue;
            }
            // Compare visible parameter types only (Type::Function excludes __closure).
            let params_match = def
                .attributes
                .iter()
                .take(visible_attr_count)
                .zip(param_types.iter())
                .all(|(a, expected)| {
                    rust_type(&a.typedef, &Context::Argument)
                        == rust_type(expected, &Context::Argument)
                });
            if !params_match {
                continue;
            }
            if rust_type(&def.returned, &Context::Result) != rust_type(&ret_type, &Context::Result)
            {
                continue;
            }
            candidates.push((d, def.name.clone(), has_closure));
        }
        // Evaluate args into pre-eval bindings to avoid double-borrow.
        let mut arg_exprs: Vec<String> = Vec::new();
        for arg in args {
            let expr = self.generate_expr_buf(arg)?;
            arg_exprs.push(expr);
        }
        // Generate a match dispatch on the fn-ref variable.
        write!(w, "match var_{var_name} {{")?;
        for (d_nr, fn_name, _has_closure) in &candidates {
            write!(w, " {d_nr}_u32 => {fn_name}(stores")?;
            for expr in &arg_exprs {
                write!(w, ", {expr}")?;
            }
            write!(w, "),")?;
        }
        write!(
            w,
            " _ => unreachable!(\"invalid fn-ref: {{}} in {var_name}\", var_{var_name}) }}"
        )?;
        Ok(())
    }

    /// Use this to emit an `if/else` expression. Handles whether branches are bare
    /// blocks (no extra braces needed) or single expressions (braces required).
    /// Infer the result type of an expression for generating typed null defaults.
    pub(super) fn infer_type(&self, v: &Value) -> Option<Type> {
        match v {
            Value::Int(_) => Some(Type::Integer(i32::MIN + 1, i32::MAX as u32, false)),
            Value::Long(_) => Some(Type::Long),
            Value::Float(_) => Some(Type::Float),
            Value::Single(_) => Some(Type::Single),
            Value::Boolean(_) => Some(Type::Boolean),
            Value::Text(_) => Some(Type::Text(Vec::new())),
            Value::Enum(_, tp) => Some(Type::Enum(u32::from(*tp), false, Vec::new())),
            Value::Var(nr) => Some(self.data.def(self.def_nr).variables.tp(*nr).clone()),
            Value::Call(d, _) => {
                let ret = &self.data.def(*d).returned;
                (*ret != Type::Void).then(|| ret.clone())
            }
            Value::Block(bl) => (bl.result != Type::Void).then(|| bl.result.clone()),
            Value::If(_, t, _) => self.infer_type(t),
            _ => None,
        }
    }

    /// Emit a typed null sentinel for the given type.
    pub(super) fn write_typed_null(w: &mut dyn Write, tp: &Type) -> std::io::Result<()> {
        match tp {
            Type::Integer(_, _, _) | Type::Character => write!(w, "i32::MIN"),
            Type::Long => write!(w, "i64::MIN"),
            Type::Float => write!(w, "f64::NAN"),
            Type::Single => write!(w, "f32::NAN"),
            Type::Boolean => write!(w, "false"),
            Type::Text(_) => write!(w, "loft::state::STRING_NULL"),
            Type::Enum(_, false, _) => write!(w, "255_u8"),
            Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Enum(_, true, _) => {
                write!(w, "DbRef {{ store_nr: u16::MAX, rec: 0, pos: 8 }}")
            }
            _ => write!(w, "()"),
        }
    }

    pub(super) fn output_if(
        &mut self,
        w: &mut dyn Write,
        test: &Value,
        true_v: &Value,
        false_v: &Value,
    ) -> std::io::Result<()> {
        write!(w, "if ")?;
        let b_true = matches!(*true_v, Value::Block(_));
        let b_false = matches!(*false_v, Value::Block(_));
        self.output_code_inner(w, test)?;
        if b_true {
            write!(w, " ")?;
        } else {
            write!(w, " {{")?;
        }
        self.indent += u32::from(!b_true);
        self.output_code_inner(w, true_v)?;
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
        // When the else branch is Null and the true branch returns a value,
        // emit a typed null sentinel instead of () to match the true branch type.
        if matches!(false_v, Value::Null)
            && let Some(tp) = self.infer_type(true_v)
        {
            Self::write_typed_null(w, &tp)?;
        } else {
            self.output_code_inner(w, false_v)?;
        }
        self.indent -= u32::from(!b_false);
        if !b_false {
            write!(w, "}}")?;
        }
        Ok(())
    }

    /// Use this to emit a scoped sequence of operators with an optional return value.
    /// This is the most involved emitter because blocks must handle three interacting concerns:
    /// 1. **Pre-evaluation hoisting** — sub-expressions that would double-borrow `stores`
    ///    are lifted into `let _preN` bindings before the enclosing expression.
    /// 2. **Return-value tracking** — when void operators trail the last non-void expression,
    ///    that expression is captured into `let _ret` first, then yielded at the end.
    /// 3. **String conversion** — a text-typed block may receive a `Str` from a field read;
    ///    `.to_string()` converts it to an owned `String`.
    #[allow(clippy::too_many_lines)]
    pub(super) fn output_block(
        &mut self,
        w: &mut dyn Write,
        bl: &Block,
        wrap_text: bool,
    ) -> std::io::Result<()> {
        writeln!(
            w,
            "{{ //{}_{}: {}",
            bl.name,
            bl.scope,
            bl.result
                .show(self.data, &self.data.def(self.def_nr).variables)
        )?;
        let is_void_block = matches!(bl.result, Type::Void);
        let is_text_result = wrap_text && matches!(bl.result, Type::Text(_));
        // Fix "hoisted return value" pattern from scopes::free_vars before iterating.
        // This replaces [expr, OpFreeText…, Return(Null)] with [OpFreeText…, Return(expr)]
        // so native code emits `return expr` rather than a dropped `expr` + `return ()`.
        let patched_ops;
        let operators: &[Value] = if is_void_block {
            patched_ops = self.patch_hoisted_returns(&bl.operators);
            &patched_ops
        } else {
            &bl.operators
        };
        // When the block expects a non-void result but trailing operator(s) are
        // void (drops, if-without-else, etc.), find the last non-void operator
        // and capture its value before the trailing void ops run.
        let last_op_idx = operators.len().saturating_sub(1);
        let return_idx = if is_void_block || operators.is_empty() {
            None
        } else {
            operators.iter().rposition(|v| !self.is_void_value(v))
        };
        let has_trailing_void = return_idx.is_some_and(|i| i < last_op_idx);
        // If the captured "return value" is a Return(…) expression, it diverges —
        // we emit it directly and skip the `_ret` tail.
        let return_value_is_return = has_trailing_void
            && return_idx.is_some_and(|i| matches!(operators[i], Value::Return(_)));
        for (vnr, v) in operators.iter().enumerate() {
            if matches!(v, Value::Line(_)) {
                continue;
            }
            // O7: pre-compute format-segment count so that text assignments at the
            // start of a format-string block (Set(var, Text)) and OpClearStackText/
            // OpClearText can emit a with_capacity hint when ≥ 2 format/append ops follow.
            self.next_format_count = match v {
                Value::Set(var, boxed)
                    if matches!(**boxed, Value::Text(_))
                        && matches!(
                            self.data.def(self.def_nr).variables.tp(*var),
                            crate::data::Type::Text(_)
                        ) =>
                {
                    count_format_ops(operators, vnr + 1, self.data)
                }
                Value::Call(d, _) => {
                    let name = &self.data.def(*d).name;
                    if name == "OpClearStackText" || name == "OpClearText" {
                        count_format_ops(operators, vnr + 1, self.data)
                    } else {
                        0
                    }
                }
                _ => 0,
            };
            // Collect pre-evaluations needed for this operator (to avoid double
            // mutable borrow of stores when user-defined functions are nested).
            // NOTE: indent is incremented here to match the level used in
            // output_code_with_subst below, so multi-line block pre_codes match.
            let counter_before = self.counter;
            self.indent += 1;
            let pre_evals = self.collect_pre_evals(v)?;
            self.indent -= 1;
            let counter_after_collect = self.counter;
            for (name, _, bind_code, _, _) in &pre_evals {
                self.indent(w)?;
                writeln!(w, "let {name} = {bind_code};")?;
            }
            // Restore counter to the value it had when the pre-eval code was generated
            // so that output_code_with_subst regenerates the same inner _pre_N names
            // as those stored in the pre-eval strings (counter desync fix).
            let restore_counter = pre_evals
                .iter()
                .map(|(_, _, _, c, _)| *c)
                .max()
                .unwrap_or(self.counter);
            self.counter = restore_counter;
            self.indent(w)?;
            // Restore counter so the buffer-check pass in output_code_with_subst
            // produces the same counter values as collect_pre_evals did above.
            self.counter = counter_before;
            if has_trailing_void && return_idx == Some(vnr) {
                // If the captured "return value" is itself a Return(…) expression,
                // emitting `let _ret = return expr;` produces an unreachable `_ret`
                // binding of type `!` that fails a later `_ret as T` cast.
                // Emit the return directly instead; the function exits here.
                if matches!(v, Value::Return(_)) {
                    self.indent += 1;
                    self.output_code_with_subst(w, v, &pre_evals)?;
                    self.indent -= 1;
                    writeln!(w, ";")?;
                    // All remaining operators are unreachable — skip trailing void tail.
                    // (We break here; the loop over subsequent ops continues but they
                    //  are free-ops which emit nothing harmful under allow(unreachable_code).)
                } else {
                    write!(w, "let _ret = ")?;
                    self.indent += 1;
                    self.output_code_with_subst(w, v, &pre_evals)?;
                    self.indent -= 1;
                    writeln!(w, ";")?;
                }
            } else {
                let is_return_expr =
                    !is_void_block && !has_trailing_void && return_idx == Some(vnr);
                // When OpCreateStack is the tail expression of a non-void block, the
                // op itself emits nothing at runtime (it's a stack-slot no-op), but
                // the block must return the mutable reference.  Emit `&mut var_<name>`
                // directly rather than delegating to output_call which writes nothing.
                if is_return_expr
                    && let Value::Call(d_nr, args) = v
                    && self.data.def(*d_nr).name == "OpCreateStack"
                    && let [Value::Var(nr)] = args.as_slice()
                {
                    let vname = sanitize(self.data.def(self.def_nr).variables.name(*nr));
                    writeln!(w, "&mut var_{vname}")?;
                } else {
                    let wrap_result = is_return_expr && is_text_result;
                    let narrow_cast = if is_return_expr {
                        narrow_int_cast(&bl.result)
                    } else {
                        None
                    };
                    if wrap_result {
                        write!(w, "Str::new(")?;
                    } else if narrow_cast.is_some() {
                        write!(w, "(")?;
                    }
                    self.indent += 1;
                    self.output_code_with_subst(w, v, &pre_evals)?;
                    self.indent -= 1;
                    if wrap_result {
                        write!(w, ")")?;
                    } else if let Some(cast) = narrow_cast {
                        write!(w, ") as {cast}")?;
                    }
                    if is_return_expr {
                        writeln!(w)?;
                    } else {
                        writeln!(w, ";")?;
                    }
                }
            }
            // Restore counter to the state after collect_pre_evals so the next
            // operator gets fresh, non-conflicting pre-eval names.
            self.counter = counter_after_collect;
        }
        if has_trailing_void && !return_value_is_return {
            self.indent(w)?;
            if is_text_result {
                writeln!(w, "Str::new(_ret)")?;
            } else if let Some(cast) = narrow_int_cast(&bl.result) {
                writeln!(w, "_ret as {cast}")?;
            } else {
                writeln!(w, "_ret")?;
            }
        } else if !is_void_block && return_idx.is_none() {
            // Non-void block with all-void operators (e.g. dynamic dispatch where all code
            // paths use explicit `return`).  Emit a typed default so Rust accepts the
            // function signature; this line is unreachable at runtime.
            self.indent(w)?;
            if is_text_result {
                writeln!(w, "Str::new(loft::state::STRING_NULL)")?;
            } else if let Some(cast) = narrow_int_cast(&bl.result) {
                writeln!(w, "0 as {cast}")?;
            } else {
                writeln!(w, "{}", default_native_value(&bl.result))?;
            }
        }
        self.indent(w)?;
        write!(
            w,
            "}} /*{}_{}: {}*/",
            bl.name,
            bl.scope,
            bl.result
                .show(self.data, &self.data.def(self.def_nr).variables)
        )?;
        Ok(())
    }
}
