// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Core IR-to-Rust emission: translates `Value` IR nodes into Rust source.

use crate::data::{Block, Context, IntegerSpec, Type, Value};
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
            // Phase 09 phase 00 step 0.7 — synthetic raw-expression
            // passthrough used by fn-ref dispatch to thread pre-evaluated
            // `let _farg_N` bindings through `output_call_user_fn`.  Emit
            // the string verbatim with no transformation.
            Value::RawExpr(s) => write!(w, "{s}")?,
            Value::Text(txt) => {
                // Use debug format to produce a properly escaped Rust string literal.
                write!(w, "{txt:?}")?;
                if self.tuple_text_to_string {
                    // Inside a `(String, String, …)` variable assignment:
                    // wrap the `&str` literal so it fits a `String`-typed
                    // tuple slot.  Argument-context tuples (function calls
                    // taking `(&str, &str)`) clear this flag, so they keep
                    // the borrowed-string form.
                    write!(w, ".to_string()")?;
                }
            }
            Value::Long(v) => write!(w, "{v}_i64")?,
            Value::Int(v) => {
                if self.fn_ref_context {
                    // in fn-ref context (if-else branch), emit tuple.
                    write!(
                        w,
                        "({v}_i32 as u32, loft::keys::DbRef {{ store_nr: u16::MAX, rec: 0, pos: 0 }})"
                    )?;
                } else if self.i32_literal_context {
                    // tp-number / field-index / flag-enum slot: runtime
                    // still expects i32.
                    write!(w, "{v}_i32")?;
                } else {
                    write!(w, "{v}_i64")?;
                }
            }
            Value::Enum(v, _) => write!(w, "{v}_u8")?,
            Value::Boolean(v) => write!(w, "{v}")?,
            Value::Float(v) => write!(w, "{v}_f64")?,
            Value::Single(v) => write!(w, "{v}_f32")?,
            Value::Null => write!(w, "()")?,
            // P198 / DX-source-map: emit a `// loft:<file>:<line>`
            // comment so rustc errors on generated Rust code can be
            // traced back to the originating loft source line.  The
            // comment is on its own line, ahead of the next emitted
            // statement.  File is implicit (per-function) so the
            // comment uses the current def's source path.
            Value::Line(line) => {
                let file = self.data.def(self.def_nr).position.file.replace('\n', "");
                writeln!(w, "// loft:{file}:{line}")?;
            }
            Value::Break(n) => {
                if *n == 0 || self.loop_stack.is_empty() {
                    write!(w, "break")?;
                } else {
                    let idx = self.loop_stack.len().saturating_sub(*n as usize + 1);
                    write!(w, "break 'l{}", self.loop_stack[idx])?;
                }
            }
            Value::BreakWith(n, val) => {
                if *n == 0 || self.loop_stack.is_empty() {
                    write!(w, "break ")?;
                } else {
                    let idx = self.loop_stack.len().saturating_sub(*n as usize + 1);
                    write!(w, "break 'l{} ", self.loop_stack[idx])?;
                }
                self.output_code_inner(w, val)?;
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
                if self.coroutine_persistent_vars.contains(var) {
                    // P224: read from the coroutine struct field.
                    if matches!(variables.tp(*var), Type::Text(_)) {
                        write!(w, "&self.var_{var_name}")?;
                    } else {
                        write!(w, "self.var_{var_name}")?;
                    }
                } else if variables.is_argument(*var) {
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
                if matches!(**val, Value::Null) && *returned != Type::Void {
                    write!(w, "return {}", super::default_native_value(returned))?;
                } else if let Value::If(test, true_v, false_v) = &**val {
                    self.pre_declare_branch_vars(w, true_v, false_v)?;
                    let returns_text = matches!(returned, Type::Text(_));
                    let narrow = narrow_int_cast(returned);
                    let wrap_text = returns_text;
                    write!(w, "return ")?;
                    if wrap_text {
                        write!(w, "Str::new(")?;
                    } else if narrow.is_some() {
                        write!(w, "(")?;
                    }
                    self.output_if_inner(w, test, true_v, false_v, true)?;
                    if wrap_text {
                        write!(w, ")")?;
                    } else if let Some(cast) = narrow {
                        write!(w, ") as {cast}")?;
                    }
                } else {
                    let returns_text = matches!(returned, Type::Text(_));
                    let narrow = narrow_int_cast(returned);
                    let inner_already_str = matches!(
                        &**val,
                        Value::Call(d, _) if (*d as usize) < self.data.definitions.len()
                            && matches!(self.data.def(*d).returned, Type::Text(_))
                            && self.data.def(*d).rust.is_empty()
                            && self.data.def(*d).native.is_empty()
                            && !self.data.def(*d).name.starts_with("Op")
                    );
                    let wrap_text = returns_text && !inner_already_str;
                    // T1.8a's tuple-of-text return path was retired by
                    // Plan-14 phase 07 (P234 runtime closure).  Function
                    // returns of `Type::Tuple(elems)` with any
                    // lifetime-bearing element (Text, Reference, etc.)
                    // are now rewritten in
                    // `src/parser/definitions.rs::parse_function` to
                    // `Type::Reference(__tuple<…>)` and the body's tail
                    // tuple literal becomes a synthetic-struct
                    // construction sequence.  So `Value::Return(Tuple)`
                    // with text elements is unreachable from any
                    // user-written tuple-of-text return — the
                    // `tuple_text_to_string` save/set/restore that lived
                    // here became dead code and was removed.
                    // (`output_set`'s analogous handling at
                    // dispatch.rs:295-359 stays — it serves LOCAL
                    // tuple-with-text variables, which the rewrite does
                    // not touch.)
                    // P205 (plan-09 phase 07): if the function returns
                    // Type::Text but has no `Type::RefVar(Type::Text(_))`
                    // attribute (no proper work buffer set up by
                    // `text_return`), `Str::new(<local_String>)` would
                    // dangle.  Route through `stores.scratch` instead so
                    // the value's backing String lives as long as `stores`.
                    // Note: text_return doesn't set the `hidden` flag (only
                    // ref_return does), so we don't filter on `a.hidden`.
                    let needs_p205_scratch = wrap_text && {
                        let def = self.data.def(self.def_nr);
                        !def.attributes.iter().any(|a| {
                            matches!(a.typedef, Type::RefVar(ref t) if matches!(**t, Type::Text(_)))
                        })
                    };
                    write!(w, "return ")?;
                    if needs_p205_scratch {
                        write!(w, "{{ stores.scratch.push((")?;
                    } else if wrap_text {
                        write!(w, "Str::new(")?;
                    } else if narrow.is_some() {
                        write!(w, "(")?;
                    }
                    // P238: when the function's return type is a tuple
                    // with text element(s) and the body's return value is
                    // a `Value::Tuple` literal, set `tuple_text_to_string`
                    // so each text element gets a `.to_string()` wrap to
                    // fit the `(String, …)` slot.  The parser's
                    // tuple-of-text → synthetic-struct rewrite (Plan-14
                    // phase 07 / P234) does NOT fire for generic
                    // monomorphisations: the source fn was parsed with T
                    // as a generic struct, so the return type at parse
                    // time was `(T, T)` with no Text elements; the
                    // rewrite trigger missed.  At monomorphisation time
                    // the type becomes `(String, String)` but the body
                    // is still a plain `Value::Tuple`.
                    let returns_text_tuple = matches!(returned, Type::Tuple(elems)
                        if elems.iter().any(|e| matches!(e, Type::Text(_))));
                    let prev_tuple_text = self.tuple_text_to_string;
                    if returns_text_tuple && matches!(&**val, Value::Tuple(_)) {
                        self.tuple_text_to_string = true;
                    }
                    self.output_code_inner(w, val)?;
                    self.tuple_text_to_string = prev_tuple_text;
                    if needs_p205_scratch {
                        write!(
                            w,
                            ").to_string()); Str::new(stores.scratch.last().unwrap()) }}"
                        )?;
                    } else if wrap_text {
                        write!(w, ")")?;
                    } else if let Some(cast) = narrow {
                        write!(w, ") as {cast}")?;
                    }
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
                    let elem_is_text = matches!(self.infer_type(e), Some(Type::Text(_)));
                    self.output_code_inner(w, e)?;
                    // Wrap text-returning element with `.to_string()` so it
                    // fits a `String`-typed tuple slot.  The outer
                    // `tuple_text_to_string` flag is set by `set_var` when
                    // the destination tuple has at least one Text element.
                    // Argument-context tuples (function calls taking
                    // `(&str, …)`) clear this flag, so they keep the
                    // borrowed-string form.  Skip when the value is
                    // already a `Value::Text` literal — its own emit
                    // arm appends `.to_string()` directly via the same
                    // flag, and we'd otherwise double-wrap.
                    if elem_is_text && self.tuple_text_to_string && !matches!(e, Value::Text(_)) {
                        write!(w, ".to_string()")?;
                    }
                }
                write!(w, ")")?;
            }
            Value::TupleGet(var, idx) => {
                // N8a.2: use the variable's declared name (like all other emitters),
                // not its internal number.  Before this fix `var_0.0` was emitted even
                // when the parameter was declared as `var_pair`.
                let variables = &self.data.def(self.def_nr).variables;
                let name = sanitize(variables.name(*var));
                // For text-typed tuple elements of a Variable-context
                // tuple, the field is `String`; consumers expecting
                // `&str` need a borrow prefix.  Mirrors the plain text
                // local case at line ~132 (`&var_name`).  Skip for
                // arguments — the variable's tuple element types in
                // Argument context emit as `&str` already.
                let elem_is_text = match variables.tp(*var) {
                    Type::Tuple(elems) => elems
                        .get(*idx as usize)
                        .is_some_and(|e| matches!(e, Type::Text(_))),
                    _ => false,
                };
                let is_arg = variables.is_argument(*var);
                if elem_is_text && !is_arg {
                    write!(w, "&var_{name}.{idx}")?;
                } else {
                    write!(w, "var_{name}.{idx}")?;
                }
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
                    if self.yield_collect_text {
                        write!(w, ").to_string())")?;
                    } else {
                        write!(w, ") as i64)")?;
                    }
                } else {
                    write!(w, "yield ")?;
                    self.output_code_inner(w, inner)?;
                }
            }
            // C39/C47: FnRef emits a (u32, DbRef) tuple.
            // If closure_var (w) is a real variable, pass its DbRef; otherwise null sentinel.
            Value::FnRef(d_nr, closure_var, _) => {
                let clos_name = if *closure_var == u16::MAX {
                    None
                } else {
                    let variables = &self.data.def(self.def_nr).variables;
                    Some(sanitize(variables.name(*closure_var)))
                };
                if let Some(name) = clos_name {
                    write!(w, "({d_nr}_u32, var_{name})")?;
                } else {
                    write!(
                        w,
                        "({d_nr}_u32, loft::keys::DbRef {{ store_nr: u16::MAX, rec: 0, pos: 0 }})"
                    )?;
                }
            }
            Value::Parallel(_) => {
                // Native codegen for parallel {} is not yet supported.
                write!(w, "/* parallel {{}} — not supported in native codegen */")?;
            }
            Value::FnRefDnr(v_nr) => {
                // P215: project the d_nr from a fn-ref var.  `var_<name>`
                // is the (u32, DbRef) tuple in native rust_type;
                // `.0 as i64` widens to match the integer ABI expected
                // by `OpSetInt4`'s template.
                let var_name = sanitize(self.data.def(self.def_nr).variables.name(*v_nr));
                write!(w, "(var_{var_name}.0 as i64)")?;
            }
            // Plan-07 phase 1 — Span is transparent in native emit.
            Value::Span(b) => self.output_code_inner(w, &b.1)?,
            // Plan-06 spine step 3 — ParFor native codegen lands in
            // step 3b.  Until then, emit a placeholder comment so the
            // file at least compiles when it accidentally appears in
            // a reachable definition.
            Value::ParFor(_) => {
                write!(
                    w,
                    "/* par_for(...) — native codegen lands in spine step 3b */"
                )?;
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
        let (param_types, ret_type) = if let Type::Function(p, r, _) = &fn_type {
            (p.clone(), *r.clone())
        } else {
            // Not a function type — fall back to debug print.
            write!(w, "{:?}", crate::data::Value::CallRef(v_nr, args.to_vec()))?;
            return Ok(());
        };
        // P227: parser appends ONE work-buffer arg for text-returning
        // fn-ref calls.  The candidate filter compares against the
        // user-visible param count, not raw `args.len()`.
        let is_text_return_match = matches!(ret_type, Type::Text(_));
        let user_arg_match = if is_text_return_match && args.len() > param_types.len() {
            args.len() - 1
        } else {
            args.len()
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
            // closure-capturing lambdas have a hidden __closure param as the last
            // attribute. The closure is injected explicitly at the call site (in arg_exprs),
            // so total arg count must equal the full attribute count.
            let has_closure = def.attributes.last().is_some_and(|a| a.name == "__closure");
            // P227: hidden-attribute detection is TYPE-based, not name-based.
            // Text-return work-buffers ride as `Type::RefVar(Type::Text(_))`
            // attributes that the parser names after the user-visible variable
            // they shadow (e.g. `a` for `a = "first: {n}"; a`) — a name-prefix
            // check (`starts_with("__")`) would miss these and reject otherwise
            // matching candidates.  Closure records remain detected by the
            // exact `__closure` name (its typedef is plain `DbRef`).
            let visible_attrs: Vec<&crate::data::Attribute> = def
                .attributes
                .iter()
                .filter(|a| {
                    !matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Text(_)))
                        && a.name != "__closure"
                })
                .collect();
            if visible_attrs.len() != user_arg_match {
                continue;
            }
            let params_match = visible_attrs
                .iter()
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
        // Phase 09 phase 00 step 0.7 — fn-ref dispatch routes each
        // candidate arm through `output_call_user_fn` (which dispatches
        // via `emit_op`), so a custom emitter registered for any
        // candidate target is honoured even when the call comes via
        // fn-ref dispatch.
        //
        // Args evaluate exactly once across all arms (correctness for
        // side-effecting expressions, plus the original "avoid
        // double-borrow of `stores`" concern).  Hoist them into Rust
        // `let _farg_N` bindings inside a wrapping block, then pass
        // synthetic `Value::RawExpr("_farg_N")` args to each per-arm
        // call.  `output_code_inner`'s `RawExpr` arm emits the binding
        // name verbatim, so the per-arm code reads
        // `fn_name(cell, _farg_0, _farg_1, …, closure_expr)` —
        // semantically the same shape as the pre-step-0.7 direct
        // emission, just routed through emit_op.
        //
        // `output_call_user_fn` iterates over the candidate's
        // `def_fn.attributes` and emits one arg per attribute.  When
        // `has_closure`, the candidate's last attribute is the
        // synthetic `__closure` (a `DbRef`); we append a RawExpr arg
        // for the closure expression so attribute count and arg count
        // line up.
        // P227: for text-returning fn-refs, the parser appends ONE
        // work-buffer arg to `args` (a `Value::Block` evaluating to
        // `&mut String` referencing a caller-function-scope work-text
        // variable).  Split it off so candidate matching uses the
        // user-visible args only; pass the work-buffer via `_farg_<n>`
        // to any candidate whose attribute list has a `RefVar(Text)`
        // hidden attr.  Function-scope lifetime means the lambda's
        // returned `Str` borrows a buffer that lives long enough for
        // the outer assignment / format consumer to read it — no
        // block-scope buffer, no per-arm `.to_string()` clone.
        let is_text_return = matches!(ret_type, Type::Text(_));
        let user_arg_count = if is_text_return && args.len() > param_types.len() {
            args.len() - 1
        } else {
            args.len()
        };
        let work_buf_idx = if is_text_return && args.len() > user_arg_count {
            Some(args.len() - 1)
        } else {
            None
        };
        write!(w, "{{ ")?;
        for (i, arg) in args.iter().enumerate() {
            let expr = self.generate_expr_buf(arg)?;
            write!(w, "let _farg_{i} = {expr}; ")?;
        }
        // Look up the closure work-var for this fn-ref variable (if any).
        let closure_var_nr = self.data.def(self.def_nr).variables.closure_var_of(v_nr);
        let closure_expr: String = if let Some(clos_nr) = closure_var_nr {
            // Same-scope closure: pass the local ___clos_N variable.
            let clos_name = sanitize(self.data.def(self.def_nr).variables.name(clos_nr));
            format!("var_{clos_name}")
        } else {
            // Cross-scope closure — pass .1 from the fn-ref tuple.
            format!("var_{var_name}.1")
        };
        let work_buf_expr: String = if let Some(idx) = work_buf_idx {
            format!("_farg_{idx}")
        } else {
            // No parser-allocated work-buffer (e.g. fn-ref type isn't
            // text-return, or call site predates Step 2).  Fall back to
            // a block-scope buffer; arms that never write to it are
            // unaffected.  This branch should be unreachable when ret
            // is text after Step 2, but kept as a defensive default.
            String::new()
        };
        // match on .0 (d_nr) of the (u32, DbRef) fn-ref tuple.
        write!(w, "match var_{var_name}.0 {{")?;
        for (d_nr, _fn_name, has_closure) in &candidates {
            write!(w, " {d_nr}_u32 => ")?;
            // P227: text-return arms wrap each call result with
            // `.to_string()` so heterogeneous candidate Rust signatures
            // (some return `Str`, some return `String`) collapse to a
            // uniform `String` for the match expression.  Both `Str`
            // and `String` implement `ToString` (via `Display` / blanket
            // impl), so this works for any text-typed candidate.  No
            // `stores.scratch` usage — the produced `String` is owned
            // by the match arm and naturally drops when the outer
            // expression has consumed it.
            if is_text_return {
                write!(w, "(")?;
            }
            // Build synthetic args matching this candidate's attribute list.
            // The candidate's attrs are interleaved: user params, then
            // hidden `RefVar(Text)` work-buffers, then `__closure` (if
            // has_closure).  Detection is type-based for work-buffers,
            // name-based only for `__closure`.
            let candidate_def = self.data.def(*d_nr);
            let mut synthetic: Vec<Value> = Vec::with_capacity(candidate_def.attributes.len());
            let mut user_idx = 0_usize;
            for a in &candidate_def.attributes {
                if matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Text(_)))
                {
                    if work_buf_expr.is_empty() {
                        // Defensive — shouldn't happen for text-returning
                        // candidate without parser-supplied buffer.
                        synthetic.push(Value::RawExpr("&mut String::new()".to_string()));
                    } else {
                        synthetic.push(Value::RawExpr(work_buf_expr.clone()));
                    }
                } else if a.name == "__closure" {
                    synthetic.push(Value::RawExpr(closure_expr.clone()));
                } else {
                    synthetic.push(Value::RawExpr(format!("_farg_{user_idx}")));
                    user_idx += 1;
                }
            }
            let _ = has_closure;
            // Route through output_call_user_fn → emit_op → custom emitter
            // (or DefaultEmitter::user_fn_call_body when no emitter is
            // registered for this candidate).
            self.output_call_user_fn(w, candidate_def, &synthetic)?;
            if is_text_return {
                write!(w, ").to_string()")?;
            }
            write!(w, ",")?;
        }
        write!(
            w,
            " _ => unreachable!(\"invalid fn-ref: {{}} in {var_name}\", var_{var_name}.0) }} }}"
        )?;
        Ok(())
    }

    /// Use this to emit an `if/else` expression. Handles whether branches are bare
    /// blocks (no extra braces needed) or single expressions (braces required).
    /// Infer the result type of an expression for generating typed null defaults.
    pub(super) fn infer_type(&self, v: &Value) -> Option<Type> {
        match v {
            Value::Int(_) => Some(Type::Integer(IntegerSpec::signed32())),
            Value::Long(_) => Some(crate::data::I64.clone()),
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

    /// Emit a value in an i32-literal context — any `Value::Int`
    /// descendant emits as `_i32` instead of the post-2c `_i64`
    /// default.  Use at tp-number, field-index, and flag-enum
    /// argument slots where the runtime signature is still i32.
    pub(super) fn emit_i32_slot(&mut self, w: &mut dyn Write, val: &Value) -> std::io::Result<()> {
        let saved = self.i32_literal_context;
        self.i32_literal_context = true;
        let r = self.output_code_inner(w, val);
        self.i32_literal_context = saved;
        r
    }

    /// Emit a typed null sentinel for the given type.
    pub(super) fn write_typed_null(w: &mut dyn Write, tp: &Type) -> std::io::Result<()> {
        match tp {
            Type::Character => write!(w, "i32::MIN"),
            Type::Integer(_) => write!(w, "i64::MIN"),
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
        self.output_if_inner(w, test, true_v, false_v, false)
    }

    fn output_if_inner(
        &mut self,
        w: &mut dyn Write,
        test: &Value,
        true_v: &Value,
        false_v: &Value,
        pre_declared: bool,
    ) -> std::io::Result<()> {
        if !pre_declared {
            self.pre_declare_branch_vars(w, true_v, false_v)?;
        }
        if let Value::Insert(ops) = test
            && ops.len() >= 2
        {
            for op in &ops[..ops.len() - 1] {
                self.output_code_inner(w, op)?;
                writeln!(w, ";")?;
                self.indent(w)?;
            }
            write!(w, "if ")?;
            self.output_code_inner(w, &ops[ops.len() - 1])?;
        } else {
            write!(w, "if ")?;
            self.output_code_inner(w, test)?;
        }
        let b_true = matches!(*true_v, Value::Block(_));
        let b_false = matches!(*false_v, Value::Block(_));
        if b_true {
            write!(w, " ")?;
        } else {
            write!(w, " {{")?;
        }
        self.indent += u32::from(!b_true);
        // save/restore fn_ref_context — Call arguments inside the branch
        // must NOT inherit it (OpDatabase int args would be misinterpreted).
        let saved_ctx = self.fn_ref_context;
        self.output_code_inner(w, true_v)?;
        self.fn_ref_context = saved_ctx;
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

    fn pre_declare_branch_vars(
        &mut self,
        w: &mut dyn Write,
        true_v: &Value,
        false_v: &Value,
    ) -> std::io::Result<()> {
        let mut t_vars: Vec<u16> = Vec::new();
        let mut f_vars: Vec<u16> = Vec::new();
        Self::collect_set_vars(true_v, &mut t_vars);
        Self::collect_set_vars(false_v, &mut f_vars);
        let variables = &self.data.def(self.def_nr).variables;
        for &v in &t_vars {
            if f_vars.contains(&v) && !self.declared.contains(&v) {
                let name = sanitize(variables.name(v));
                let tp_str = rust_type(variables.tp(v), &Context::Variable);
                let default = default_native_value(variables.tp(v));
                writeln!(w, "let mut var_{name}: {tp_str} = {default};")?;
                self.indent(w)?;
                self.declared.insert(v);
            }
        }
        Ok(())
    }

    fn collect_set_vars(val: &Value, result: &mut Vec<u16>) {
        match val {
            Value::Set(v, inner) => {
                if !result.contains(v) {
                    result.push(*v);
                }
                Self::collect_set_vars(inner, result);
            }
            Value::Block(bl) => {
                for op in &bl.operators {
                    Self::collect_set_vars(op, result);
                }
            }
            Value::If(c, t, f) => {
                Self::collect_set_vars(c, result);
                Self::collect_set_vars(t, result);
                Self::collect_set_vars(f, result);
            }
            Value::Insert(ops) => {
                for op in ops {
                    Self::collect_set_vars(op, result);
                }
            }
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::collect_set_vars(a, result);
                }
            }
            Value::Drop(inner) | Value::Return(inner) => {
                Self::collect_set_vars(inner, result);
            }
            _ => {}
        }
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
        // Plan-06 phase 4d: fn-ref field read emits the (u32, DbRef)
        // native tuple form directly.  The block carries two ops —
        // `OpGetInt4(ref, fld)` (returns i64) and `OpNullRefSentinel()`
        // (returns DbRef) — which the interpreter pushes sequentially
        // onto the eval stack to form the 20-byte fn-ref slot.  Native
        // can't use a generic Rust block here because the last
        // expression's type would be `DbRef`, not `(u32, DbRef)`; and
        // the `i64` from OpGetInt4 needs an explicit `as u32` cast to
        // match the fn-ref tuple's first slot.
        if bl.name == "fn_ref_field_read" && bl.operators.len() == 2 {
            write!(w, "((")?;
            self.output_code_inner(w, &bl.operators[0])?;
            write!(w, ") as u32, ")?;
            self.output_code_inner(w, &bl.operators[1])?;
            write!(w, ")")?;
            return Ok(());
        }
        writeln!(
            w,
            "{{ //{}_{}: {}",
            bl.name,
            bl.scope,
            bl.result
                .show(self.data, &self.data.def(self.def_nr).variables)
        )?;
        // Inject shadow call stack instrumentation if set by output_function().
        if let Some(prefix) = self.call_stack_prefix.take() {
            writeln!(w, "{prefix}")?;
        }
        let is_void_block = matches!(bl.result, Type::Void);
        let is_text_result = wrap_text && matches!(bl.result, Type::Text(_));
        // Fix "hoisted return value" pattern from scopes::free_vars before iterating.
        // This replaces [expr, OpFreeText…, Return(Null)] with [OpFreeText…, Return(expr)]
        // so native code emits `return expr` rather than a dropped `expr` + `return ()`.
        // also patch Type::Never blocks (unconditional return with cleanup).
        // also patch `Type::Text` blocks when the enclosing function is
        // a bounded-generic T-stub (name like `t_<len><Type>_<method>`).
        // Their IR is produced by template specialisation and shows the same
        // `[Call, OpFreeText(work), Return(Null)]` pattern at the top of the
        // body block — without the patch, native codegen emits the Call as a
        // discarded statement and returns STRING_NULL.
        let fn_name = &self.data.def(self.def_nr).name;
        let is_t_stub_text_body = matches!(bl.result, Type::Text(_)) && fn_name.starts_with("t_");
        // Any text-returning block whose body contains the B5-L3
        // `Set(__ret_N, call); ...; Return(Var(__ret_N))` temp-transfer
        // pattern must also go through `patch_hoisted_returns` so the
        // collapse pass can rewrite it to `return call(...)` — otherwise
        // the local `String` ret-temp drops at function exit and the
        // returned `Str` raw ptr dangles
        // (`tests/scripts/86-interfaces.loft::if_label`).
        let has_ret_temp = matches!(bl.result, Type::Text(_))
            && bl.operators.iter().any(|op| {
                matches!(op.unspan(), Value::Set(v, _) if
                    self.data.def(self.def_nr).variables.name(*v).starts_with("__ret_")
                    && self.data.def(self.def_nr).variables.is_skip_free(*v))
            });
        let patched_ops;
        let operators: &[Value] = if is_void_block
            || matches!(bl.result, Type::Never)
            || is_t_stub_text_body
            || has_ret_temp
        {
            patched_ops = self.patch_hoisted_returns(&bl.operators);
            &patched_ops
        } else {
            &bl.operators
        };
        // Native-only ref-return tail-call capture (87-store-leaks).
        // See pre_eval.rs::detect_ref_tail_capture for the pattern.  At the
        // call index we emit `let __native_tail_ret: DbRef = <call>;`; at
        // the Return(Null) index we emit `return __native_tail_ret;`
        // instead of the null-sentinel `return DbRef { store_nr: u16::MAX, … }`.
        let tail_capture = self.detect_ref_tail_capture(bl, operators);
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
            && return_idx.is_some_and(|i| matches!(operators[i].unspan(), Value::Return(_)));
        for (vnr, v) in operators.iter().enumerate() {
            // DX-source-map: surface line comments at the
            // statement-list level so rustc errors map back to .loft
            // source.  Without this, only Value::Line nodes inside an
            // expression context get rendered (rare in practice).
            if let Value::Line(line) = v {
                let file = self.data.def(self.def_nr).position.file.replace('\n', "");
                self.indent(w)?;
                writeln!(w, "// loft:{file}:{line}")?;
                continue;
            }
            // Ref-return tail-call capture: `return __native_tail_ret;` in
            // place of the Return(Null)'s null-sentinel emission.  No pre_evals
            // — the Return(Null) itself references no vars.
            if let Some((_, ret_idx)) = tail_capture
                && vnr == ret_idx
            {
                self.indent(w)?;
                writeln!(w, "return __native_tail_ret;")?;
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
                if matches!(v.unspan(), Value::Return(_)) {
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
                let is_tail_capture_call =
                    tail_capture.is_some_and(|(call_idx, _)| vnr == call_idx);
                // When OpCreateStack is the tail expression of a non-void block, the
                // op itself emits nothing at runtime (it's a stack-slot no-op), but
                // the block must return the mutable reference.  Emit `&mut var_<name>`
                // directly rather than delegating to output_call which writes nothing.
                if is_return_expr
                    && let Value::Call(d_nr, args) = v.unspan()
                    && self.data.def(*d_nr).name == "OpCreateStack"
                    && let [Value::Var(nr)] = args.as_slice()
                {
                    let vname = sanitize(self.data.def(self.def_nr).variables.name(*nr));
                    writeln!(w, "&mut var_{vname}")?;
                } else {
                    // A `Value::Return(...)` already emits its own `return …`
                    // (typed for the function signature), so wrapping it in
                    // `Str::new(...)` would produce `Str::new(return Str::new(X))`
                    // which fails Rust type-check.  Same reasoning for narrow
                    // int casts: the return statement carries the right type.
                    //
                    // P208 (plan-17 phase 01 follow-up): the same redundancy
                    // applies when the value is a `Value::Block` whose tail
                    // expression is a `Value::Return` (recursively).  The
                    // inner Return handles its own scratch.push wrap; the
                    // outer wrap_result wrap then surrounds an unreachable
                    // expression (the Block's tail has type `!`), which
                    // rustc rejects with E0282 because `to_string()` can't
                    // be inferred on the never type.  Walk through Blocks
                    // and Spans to detect tail-Return.
                    fn tail_is_return(v: &Value) -> bool {
                        match v.unspan() {
                            Value::Return(_) => true,
                            Value::Block(bl) => bl.operators.last().is_some_and(tail_is_return),
                            _ => false,
                        }
                    }
                    let value_is_return = tail_is_return(v);
                    let wrap_result = is_return_expr && is_text_result && !value_is_return;
                    // Iterator-next blocks (name "iter next" / "sorted iter next")
                    // return their element value OR `i64::MIN` as the
                    // end-of-iteration sentinel.  Wrapping the result in
                    // `as u16` / `as u8` for narrow element types truncates
                    // `i64::MIN` to `0`, destroying the sentinel — the
                    // subsequent `!op_conv_bool_from_int(var_x)` break check
                    // compares `0 != i64::MIN` → true, inverted to false,
                    // never breaking.  `for x in vector<u16>` then loops
                    // forever printing `x=0`.  Skip the narrow cast for
                    // iterator-next blocks so `i64::MIN` survives intact;
                    // the consuming variable assignment applies its own
                    // `as i64` widening which is a no-op for i64 values.
                    let is_iter_next = bl.name.contains("iter next");
                    let narrow_cast = if is_return_expr && !value_is_return && !is_iter_next {
                        narrow_int_cast(&bl.result)
                    } else {
                        None
                    };
                    // P205 (plan-09 phase 07): when this function returns
                    // Type::Text but has NO `Type::RefVar(Type::Text(_))`
                    // attribute (i.e. text_return didn't add a proper
                    // work buffer — happens for bounded-generic
                    // specialisations and a few other text-return paths),
                    // a plain `Str::new(<value>)` wrap captures a borrow
                    // into a local String that drops at function return,
                    // dangling the returned `Str`'s raw pointer.  Route
                    // through `stores.scratch` instead so the value's
                    // backing String lives as long as `stores` does.
                    // Note: text_return doesn't set the `hidden` flag (only
                    // ref_return does), so we don't filter on `a.hidden`.
                    let needs_p205_scratch = wrap_result
                        && {
                            let def = self.data.def(self.def_nr);
                            matches!(def.returned, Type::Text(_))
                            && !def.attributes.iter().any(|a| {
                                matches!(a.typedef, Type::RefVar(ref t) if matches!(**t, Type::Text(_)))
                            })
                        };
                    if is_tail_capture_call {
                        write!(w, "let __native_tail_ret: DbRef = ")?;
                    } else if needs_p205_scratch {
                        // Move the String into stores.scratch, then return
                        // a Str pointing into the scratch entry.  The
                        // `(value).to_string()` coerces &str / String /
                        // Str all into an owned String.
                        write!(w, "{{ stores.scratch.push((")?;
                    } else if wrap_result {
                        write!(w, "Str::new(")?;
                    } else if narrow_cast.is_some() {
                        write!(w, "(")?;
                    }
                    self.indent += 1;
                    self.output_code_with_subst(w, v, &pre_evals)?;
                    self.indent -= 1;
                    if needs_p205_scratch {
                        write!(
                            w,
                            ").to_string()); Str::new(stores.scratch.last().unwrap()) }}"
                        )?;
                    } else if wrap_result {
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
