// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Variable assignment and function call dispatch code generation.

use crate::data::{Context, Type, Value};
use std::io::Write;

use super::calls::contains_op_database;
use super::{Output, default_native_value, narrow_int_cast, rust_type, sanitize};

impl Output<'_> {
    #[allow(clippy::too_many_lines)]
    pub(super) fn output_set(
        &mut self,
        w: &mut dyn Write,
        var: u16,
        to: &Value,
    ) -> std::io::Result<()> {
        let variables = &self.data.def(self.def_nr).variables;
        if variables.is_argument(var)
            && let Type::RefVar(inner) = variables.tp(var)
        {
            if to != &Value::Null {
                let name = sanitize(variables.name(var));
                write!(w, "*var_{name} = ")?;
                self.output_code_inner(w, to)?;
                if matches!(**inner, Type::Text(_)) {
                    write!(w, ".to_string()")?;
                }
            }
            return Ok(());
        }
        let needs_to_string = matches!(variables.tp(var), Type::Text(_));
        let name = sanitize(variables.name(var));
        // P198 — most operators are wrapped in Value::Span by the parser.
        // Unwrap before pattern-matching so the deep-copy emission below
        // fires for Span(Call(...)) / Span(Var(...)) RHS values.  Without
        // this, native codegen falls through to a plain assignment that
        // aliases the parameter's store, breaking the loft invariant that
        // each ref-typed variable owns its own store
        // (tests/scripts/95-alias-copy.loft assert orig.x == 1.0 fails
        // because mutating ac_copy mutates ac_orig too).
        let to_unspanned = to.unspan();
        // when a call returns a Reference and the callee has
        // visible Reference params, the returned DbRef may alias a parameter.
        // Deep-copy to prevent aliasing.
        if let (Some(d_nr), Value::Call(fn_nr, args)) =
            (variables.tp(var).heap_def_nr(), to_unspanned)
            && self.data.def(*fn_nr).name.starts_with("n_")
            && self.data.def(*fn_nr).code != Value::Null
            && self.data.def(*fn_nr).attributes.iter().any(|a| {
                !a.hidden && matches!(a.typedef, Type::Reference(_, _) | Type::Enum(_, true, _))
            })
        {
            let tp_nr = self.data.def(d_nr).known_type;
            if !self.declared.contains(&var) {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                writeln!(
                    w,
                    "let mut var_{name}: {tp_str} = stores.null_named(\"var_{name}\");"
                )?;
                self.indent(w)?;
            }
            writeln!(w, "var_{name} = OpDatabase(cell,var_{name}, {tp_nr}_i32);")?;
            self.indent(w)?;
            // Emit the call into a temporary, then deep-copy.
            // P198 — the inner user-fn call uses the new `cell` ABI; the
            // outer OpCopyRecord wraps `cell` to a fresh `&mut Stores`.
            write!(w, "{{ let _src = {}(cell", self.data.def(*fn_nr).name)?;
            for arg in args {
                write!(w, ", ")?;
                if let Some(vr) = self.create_stack_var(arg) {
                    let sn = sanitize(variables.name(vr));
                    write!(w, "&mut var_{sn}")?;
                } else {
                    self.output_code_inner(w, arg)?;
                }
            }
            write!(w, "); OpCopyRecord(cell,_src, var_{name}, {tp_nr}_i32); }}")?;
            return Ok(());
        }
        // When assigning a reference to a reference variable, a pointer copy is not
        // sufficient — emit an OpCopyRecord call for a deep copy.
        // For a first declaration, we also need to allocate a fresh store via
        // OpDatabase(null_named(…)) so the destination has its own record to copy into.
        // For reassignment, the existing destination record is reused in-place.
        if let (Some(d_nr), Value::Var(src)) = (variables.tp(var).heap_def_nr(), to_unspanned)
            && variables.tp(*src).heap_def_nr().is_some()
        {
            let src_name = sanitize(variables.name(*src));
            let tp_nr = self.data.def(d_nr).known_type;
            if self.declared.contains(&var) {
                // Reassignment: the variable was pre-declared via null_named
                // (Set(var, Null)) at function entry.  OpDatabase below
                // ensures it has a valid allocated record.
            } else {
                self.declared.insert(var);
                let var_tp = variables.tp(var);
                let tp_str = rust_type(var_tp, &Context::Variable);
                // Two statements: null_named and OpDatabase cannot share a &mut stores borrow.
                writeln!(
                    w,
                    "let mut var_{name}: {tp_str} = stores.null_named(\"var_{name}\");"
                )?;
                self.indent(w)?;
            }
            writeln!(w, "var_{name} = OpDatabase(cell,var_{name}, {tp_nr}_i32);")?;
            self.indent(w)?;
            write!(
                w,
                "OpCopyRecord(cell,var_{src_name}, var_{name}, {tp_nr}_i32)"
            )?;
            return Ok(());
        }
        // For text/reference block assignments, pre-declare the variable so that
        // any drop(@var) inside the block (e.g., on break) can reference it.
        if !self.declared.contains(&var) && matches!(to, Value::Block(_)) {
            let var_tp = variables.tp(var);
            if matches!(var_tp, Type::Text(_)) {
                self.declared.insert(var);
                write!(w, "let mut var_{name} = ")?;
                self.output_code_inner(w, to)?;
                if needs_to_string {
                    write!(w, ".to_string()")?;
                }
                return Ok(());
            }
        }
        // S35: Set(var, Insert([stmt1, ..., last_expr])) — hoist all-but-last ops as
        // statements before the declaration, then assign only from the final expression.
        // Without this, the inner Set ops are emitted inline inside an expression context,
        // producing malformed Rust like `let mut var_rv: DbRef = let mut var__read: DbRef = …`.
        if let Value::Insert(ops) = to
            && !ops.is_empty()
        {
            for op in &ops[..ops.len() - 1] {
                self.indent(w)?;
                self.output_code_inner(w, op)?;
                writeln!(w, ";")?;
            }
            self.indent(w)?;
            if self.declared.contains(&var) {
                write!(w, "var_{name} = ")?;
            } else {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                write!(w, "let mut var_{name}: {tp_str} = ")?;
            }
            self.output_code_inner(w, &ops[ops.len() - 1])?;
            return Ok(());
        }
        // Hoist call arguments that mutate stores into temporaries to prevent
        // double-mutable-borrow of `stores` in the call expression.
        if let Value::Call(call_dnr, args) = to.unspan()
            && args.iter().any(|a| contains_op_database(a, self.data))
        {
            let def_fn = self.data.def(*call_dnr);
            let mut hoisted: Vec<Option<String>> = vec![None; args.len()];
            for (idx, arg) in args.iter().enumerate() {
                if contains_op_database(arg, self.data) {
                    let param_tp = if idx < def_fn.attributes.len() {
                        rust_type(&def_fn.attributes[idx].typedef, &Context::Argument)
                    } else {
                        "DbRef".to_string()
                    };
                    let tmp = format!("_harg_{name}_{idx}");
                    write!(w, "let {tmp}: {param_tp} = ")?;
                    self.output_code_inner(w, arg)?;
                    writeln!(w, ";")?;
                    self.indent(w)?;
                    hoisted[idx] = Some(tmp);
                }
            }
            if self.declared.contains(&var) {
                write!(w, "var_{name} = ")?;
            } else {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                write!(w, "let mut var_{name}: {tp_str} = ")?;
            }
            // P199 — user-fn / Op-stub callees take `&UnsafeCell<Stores>`
            // (cell), not `&mut Stores` (stores).  Plan 09 phase 01 added
            // per-fn ABI tagging via `crate::codegen_runtime::abi_of`:
            //   - Cell  → `name(cell, args...)`     (default)
            //   - None  → `name(args...)`           (no implicit Stores)
            let abi = if def_fn.code == Value::Null {
                crate::codegen_runtime::abi_of(&def_fn.name)
            } else {
                crate::codegen_runtime::Abi::Cell
            };
            write!(w, "{}(", def_fn.name)?;
            let mut first_arg = true;
            if matches!(abi, crate::codegen_runtime::Abi::Cell) {
                write!(w, "cell")?;
                first_arg = false;
            }
            for (idx, arg) in args.iter().enumerate() {
                if !first_arg {
                    write!(w, ", ")?;
                }
                first_arg = false;
                if let Some(ref tmp) = hoisted[idx] {
                    write!(w, "{tmp}")?;
                } else if let Some(vr) = self.create_stack_var(arg) {
                    let sn = sanitize(variables.name(vr));
                    write!(w, "&mut var_{sn}")?;
                } else {
                    self.output_code_inner(w, arg)?;
                }
            }
            write!(w, ")")?;
            if needs_to_string {
                write!(w, ".to_string()")?;
            }
            return Ok(());
        }
        if self.declared.contains(&var) {
            write!(w, "var_{name} = ")?;
        } else {
            self.declared.insert(var);
            let var_tp = variables.tp(var);
            let tp_str = rust_type(var_tp, &Context::Variable);
            write!(w, "let mut var_{name}: {tp_str} = ")?;
        }
        if matches!(to, Value::Null) && rust_type(variables.tp(var), &Context::Variable) == "DbRef"
        {
            self.emit_null_dbref(w, var, &name)?;
        } else if to == &Value::Null {
            // Emit the null sentinel for the variable's type, not bare `()`.
            let null_val = default_native_value(variables.tp(var));
            write!(w, "{null_val}")?;
        } else {
            // O7: when this text assignment opens a multi-segment format string,
            // pre-allocate capacity to avoid repeated reallocations.
            if needs_to_string
                && self.next_format_count > 1
                && let Value::Text(initial) = to
            {
                let n = self.next_format_count;
                self.next_format_count = 0;
                let cap = initial.len() + n * 8;
                if initial.is_empty() {
                    write!(w, "String::with_capacity({cap}_usize)")?;
                } else {
                    write!(
                        w,
                        "{{ let mut _s = String::with_capacity({cap}_usize); \
                         _s.push_str({initial:?}); _s }}"
                    )?;
                }
            } else {
                // wrap plain Int or If-with-Int values assigned to Function vars.
                let is_fn_ref_var = matches!(variables.tp(var), Type::Function(_, _, _));
                let wrap_fn_ref = is_fn_ref_var && matches!(to, Value::Int(_));
                if wrap_fn_ref {
                    write!(w, "(")?;
                }
                // set fn_ref_context so if-else branches with bare Int
                // values produce (u32, null_DbRef) tuples.  Cleared inside
                // Call argument processing to avoid wrapping OpDatabase args.
                let prev_ctx = self.fn_ref_context;
                if is_fn_ref_var && !wrap_fn_ref {
                    self.fn_ref_context = true;
                }
                // When assigning to a `(String, …)` tuple variable, the
                // element values that emit as `&str` literals need a
                // `.to_string()` wrap so the tuple's runtime type
                // matches its declared `(String, …)` shape.  Without
                // this the Rust compiler rejects `("a", "b")` against
                // `(String, String)`.  See `Value::Text` in emit.rs.
                let prev_tuple_text = self.tuple_text_to_string;
                if let Type::Tuple(elems) = variables.tp(var)
                    && elems.iter().any(|e| matches!(e, Type::Text(_)))
                {
                    self.tuple_text_to_string = true;
                }
                // When assigning to a String variable from a text-local source,
                // output_code_inner emits `&var_name` (borrow to &str), and
                // appending `.to_string()` yields `&String` not `String`.
                // Detect this case and emit `.clone()` on the owned String directly.
                let text_local_clone = needs_to_string
                    && matches!(to, Value::Var(v) if {
                        let vars = &self.data.def(self.def_nr).variables;
                        !vars.is_argument(*v) && matches!(vars.tp(*v), Type::Text(_))
                    });
                if text_local_clone {
                    if let Value::Var(v) = to {
                        let src_name = sanitize(self.data.def(self.def_nr).variables.name(*v));
                        write!(w, "var_{src_name}.clone()")?;
                    }
                } else {
                    self.output_code_inner(w, to)?;
                }
                self.fn_ref_context = prev_ctx;
                self.tuple_text_to_string = prev_tuple_text;
                if needs_to_string && !text_local_clone {
                    write!(w, ".to_string()")?;
                } else if wrap_fn_ref {
                    write!(
                        w,
                        " as u32, loft::keys::DbRef {{ store_nr: u16::MAX, rec: 0, pos: 0 }})"
                    )?;
                } else if matches!(variables.tp(var), Type::Routine(_))
                    && !matches!(to, Value::Null)
                {
                    write!(w, " as u32")?;
                } else if to != &Value::Null && narrow_int_cast(variables.tp(var)).is_some() {
                    // Variable is a narrow integer type, but the RHS expression
                    // (a function returning u16 or an iterator block returning as u16) produces
                    // the narrow type.  Post-2c: widen to i64 to match the default Integer.
                    write!(w, " as i64")?;
                } else if let Value::Call(d_nr, _) = to.unspan() {
                    // When the variable type and the called function's return type differ
                    // (e.g., multiple parallel-for loops reusing `b` with different worker types),
                    // add a cast so Rust accepts the assignment.
                    let var_tp_str = rust_type(variables.tp(var), &Context::Variable);
                    let ret = &self.data.def(*d_nr).returned;
                    let ret_str = rust_type(ret, &Context::Variable);
                    if ret_str != var_tp_str && !matches!(ret, Type::Void) {
                        write!(w, " as {var_tp_str}")?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Emit a null-initialised `DbRef` variable, matching the interpreter's pre-init order.
    fn emit_null_dbref(&mut self, w: &mut dyn Write, var: u16, name: &str) -> std::io::Result<()> {
        let variables = &self.data.def(self.def_nr).variables;
        let var_raw_name = variables.name(var);
        let is_elm = var_raw_name.starts_with("_elm");
        let owns_store = match variables.tp(var) {
            Type::Reference(_, dep)
            | Type::Vector(_, dep)
            | Type::Enum(_, true, dep)
            | Type::Sorted(_, _, dep)
            | Type::Hash(_, _, dep)
            | Type::Index(_, _, dep)
            | Type::Spacial(_, _, dep) => dep.is_empty(),
            _ => false,
        };
        if is_elm || variables.is_inline_ref(var) || !owns_store {
            write!(w, "DbRef {{ store_nr: u16::MAX, rec: 0, pos: 8 }}")?;
        } else {
            let ref_buf_type_id = {
                let var_tp = variables.tp(var).clone();
                match &var_tp {
                    Type::Vector(elm_tp, _) => {
                        let elm_name = elm_tp.name(self.data);
                        self.data.name_type(&format!("main_vector<{elm_name}>"), 0)
                    }
                    // P188: keyed-collection locals need an OpDatabase
                    // call against the specific keyed-collection type so
                    // the backing store is allocated and the root pointer
                    // is zero-initialised.  Resolves to the same database
                    // type id that struct-field registration uses.
                    Type::Sorted(td, key, _) | Type::Index(td, key, _) => {
                        let c = self.data.def(*td).known_type;
                        if c == u16::MAX {
                            u16::MAX
                        } else {
                            let prefix = match &var_tp {
                                Type::Sorted(_, _, _) => "sorted",
                                Type::Index(_, _, _) => "index",
                                _ => unreachable!(),
                            };
                            let mut name =
                                format!("{prefix}<{}[", self.stores.types[c as usize].name);
                            for (k_nr, (k, asc)) in key.iter().enumerate() {
                                if k_nr > 0 {
                                    name += ",";
                                }
                                if !*asc {
                                    name += "-";
                                }
                                name += k;
                            }
                            name += "]>";
                            self.stores.name(&name)
                        }
                    }
                    Type::Hash(td, key, _) | Type::Spacial(td, key, _) => {
                        let c = self.data.def(*td).known_type;
                        if c == u16::MAX {
                            u16::MAX
                        } else {
                            let prefix = match &var_tp {
                                Type::Hash(_, _, _) => "hash",
                                Type::Spacial(_, _, _) => "spacial",
                                _ => unreachable!(),
                            };
                            let mut name =
                                format!("{prefix}<{}[", self.stores.types[c as usize].name);
                            for (k_nr, k) in key.iter().enumerate() {
                                if k_nr > 0 {
                                    name += ",";
                                }
                                name += k;
                            }
                            name += "]>";
                            self.stores.name(&name)
                        }
                    }
                    _ => u16::MAX,
                }
            };
            if ref_buf_type_id == u16::MAX {
                write!(w, "stores.null_named(\"var_{name}\")")?;
            } else {
                writeln!(w, "stores.null_named(\"var_{name}\");")?;
                self.indent(w)?;
                write!(
                    w,
                    "var_{name} = OpDatabase(cell,var_{name}, {ref_buf_type_id}_i32)"
                )?;
            }
        }
        Ok(())
    }

    /// Use this to dispatch a `Value::Call` to either the user-function or template emitter.
    /// Certain built-in text operations are intercepted here because their generated Rust
    /// differs structurally from both a regular call and a template substitution.
    #[allow(clippy::too_many_lines)] // large opcode dispatch — splitting would lose context
    pub(super) fn output_call(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
        vals: &[Value],
    ) -> std::io::Result<()> {
        // clear fn_ref_context inside calls — arguments like OpDatabase's
        // type number are plain integers, not fn-ref d_nr values.
        let saved_ctx = self.fn_ref_context;
        self.fn_ref_context = false;
        let result = self.output_call_inner(w, def_nr, vals);
        self.fn_ref_context = saved_ctx;
        result
    }

    #[allow(clippy::too_many_lines)]
    fn output_call_inner(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
        vals: &[Value],
    ) -> std::io::Result<()> {
        let def_fn = self.data.def(def_nr);
        let name: &str = &def_fn.name;
        // Phase 09 phase 00 step 0.6: registry-first dispatch.  When a
        // custom emitter is registered for this Op, run it instead of
        // the special-case match arms below.  Today the registry is
        // empty so every Op falls through to the existing dispatch.
        // Future phases register per-Op emitters that take over these
        // emissions one Op at a time (without touching the bulk match).
        if crate::generation::ops::has_custom_emitter(name) {
            let name_owned = name.to_string();
            let mut ctx = crate::generation::ops::EmitCtx {
                w,
                def_fn,
                output: self,
            };
            return crate::generation::ops::emit_op(&mut ctx, &name_owned, vals);
        }
        match name {
            "OpFormatInt" | "OpFormatStackInt" => {
                return self.format_long(w, vals, name == "OpFormatStackInt");
            }
            "OpFormatFloat" | "OpFormatStackFloat" => {
                return self.format_float(w, vals, name == "OpFormatStackFloat");
            }
            "OpFormatSingle" | "OpFormatStackSingle" => {
                return self.format_single(w, vals, name == "OpFormatStackSingle");
            }
            "OpFormatText" | "OpFormatStackText" => return self.format_text(w, vals),
            "OpAppendText" => return self.append_text(w, vals),
            "OpAppendStackText" => {
                write!(w, "*")?;
                return self.append_text(w, vals);
            }
            "OpAppendCharacter" | "OpAppendStackCharacter" => {
                return self.append_character(w, vals);
            }
            "OpClearStackText" | "OpClearText" => return self.clear_stack_text(w, vals),
            "OpClearVector" => return self.clear_vector(w, vals),
            "OpFreeText" | "OpCreateStack" => return Ok(()),
            // N8b.2: advance a native coroutine and return the yielded value.
            // parameters[0] = gen DbRef expression; parameters[1] = Int(value_size).
            "OpCoroutineNext" => {
                if let Some(gen_val) = vals.first() {
                    let gen_code = self.generate_expr_buf(gen_val)?;
                    let value_size = if let Some(Value::Int(n)) = vals.get(1) {
                        *n
                    } else {
                        4 // fallback: i32
                    };
                    match value_size {
                        8 => write!(
                            w,
                            "loft::codegen_runtime::coroutine_next_i64({gen_code}, stores)"
                        )?,
                        1 => write!(
                            w,
                            "(loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) != 0)"
                        )?,
                        _ => write!(
                            w,
                            "loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) as i32"
                        )?,
                    }
                }
                return Ok(());
            }
            // N8b.2: test whether a native coroutine is exhausted.
            // parameters[0] = gen DbRef expression.
            "OpCoroutineExhausted" => {
                if let Some(gen_val) = vals.first() {
                    let gen_code = self.generate_expr_buf(gen_val)?;
                    write!(
                        w,
                        "loft::codegen_runtime::coroutine_is_exhausted({gen_code})"
                    )?;
                }
                return Ok(());
            }
            "OpNullRefSentinel" => {
                write!(w, "DbRef {{ store_nr: u16::MAX, rec: 0, pos: 8 }}")?;
                return Ok(());
            }
            // Null-aware reference equality: treat rec==0 as null regardless of store_nr,
            // matching the bytecode eq_ref/ne_ref implementation.
            "OpEqRef" => {
                if let [v1, v2] = vals {
                    let s1 = self.generate_expr_buf(v1)?;
                    let s2 = self.generate_expr_buf(v2)?;
                    write!(
                        w,
                        "{{let _a={s1};let _b={s2};if _a.rec==0||_b.rec==0{{_a.rec==0&&_b.rec==0}}else{{_a==_b}}}}"
                    )?;
                    return Ok(());
                }
            }
            "OpNeRef" => {
                if let [v1, v2] = vals {
                    let s1 = self.generate_expr_buf(v1)?;
                    let s2 = self.generate_expr_buf(v2)?;
                    write!(
                        w,
                        "{{let _a={s1};let _b={s2};if _a.rec==0||_b.rec==0{{_a.rec!=0||_b.rec!=0}}else{{_a!=_b}}}}"
                    )?;
                    return Ok(());
                }
            }
            "OpFreeRef" => {
                // Emit OpFreeRef(cell,var, "var_name") so LOFT_STORE_LOG shows the loft name.
                // After freeing, reset the variable to null so a subsequent OpDatabase
                // knows to allocate a fresh store rather than reusing the freed one.
                if let [ref db_val] = vals[..] {
                    // S34/S35: skip_free variables share a slot with an outer variable that
                    // already owns the record; suppressing their OpFreeRef prevents a double-free.
                    if let Value::Var(v) = db_val
                        && self.data.def(self.def_nr).variables.is_skip_free(*v)
                    {
                        write!(w, "()")?;
                        return Ok(());
                    }
                    // free the closure component of fn-ref (u32, DbRef) variables.
                    // Non-capturing lambdas have store_nr = u16::MAX (null sentinel).
                    if let Value::Var(v) = db_val
                        && matches!(
                            self.data.def(self.def_nr).variables.tp(*v),
                            Type::Function(_, _, _)
                        )
                    {
                        let vn = format!(
                            "var_{}",
                            sanitize(self.data.def(self.def_nr).variables.name(*v))
                        );
                        write!(
                            w,
                            "if {vn}.1.store_nr != u16::MAX {{ \
                             OpFreeRef(cell,{vn}.1, \"{vn}.1\"); \
                             {vn}.1.store_nr = u16::MAX }}"
                        )?;
                        return Ok(());
                    }
                    let var_name = if let Value::Var(v) = db_val {
                        format!(
                            "var_{}",
                            sanitize(self.data.def(self.def_nr).variables.name(*v))
                        )
                    } else {
                        String::new()
                    };
                    write!(w, "OpFreeRef(cell,")?;
                    self.output_code_inner(w, db_val)?;
                    write!(w, ", \"{var_name}\")")?;
                    // Reset variable to null sentinel after free.
                    if let Value::Var(_) = db_val {
                        write!(w, "; {var_name}.store_nr = u16::MAX")?;
                    }
                }
                return Ok(());
            }
            "OpFreeRefIfDistinct" => {
                // free the placeholder only when its store_nr
                // differs from the witness's.  Emit:
                //   if ph.store_nr != wit.store_nr { OpFreeRef(cell,ph, "ph"); ph.store_nr = u16::MAX }
                // so the fresh-store path still reclaims the orphan
                // and the adoption path leaves both slots alone until
                // the caller's OpFreeRef on the witness fires.
                if let [ref ph_val, ref wit_val] = vals[..] {
                    let ph_name = if let Value::Var(v) = ph_val {
                        format!(
                            "var_{}",
                            sanitize(self.data.def(self.def_nr).variables.name(*v))
                        )
                    } else {
                        String::new()
                    };
                    write!(w, "if ")?;
                    self.output_code_inner(w, ph_val)?;
                    write!(w, ".store_nr != ")?;
                    self.output_code_inner(w, wit_val)?;
                    write!(w, ".store_nr {{ OpFreeRef(cell,")?;
                    self.output_code_inner(w, ph_val)?;
                    write!(w, ", \"{ph_name}\")")?;
                    if let Value::Var(_) = ph_val {
                        write!(w, "; {ph_name}.store_nr = u16::MAX")?;
                    }
                    write!(w, " }}")?;
                }
                return Ok(());
            }
            "OpCopyRecord" => {
                // Deep copy: copy_block + copy_claims
                if let [ref src, ref dst, ref tp_val] = vals[..] {
                    write!(w, "OpCopyRecord(cell,")?;
                    self.output_code_inner(w, src)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, dst)?;
                    write!(w, ", ")?;
                    self.emit_i32_slot(w, tp_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpConvTextFromNull" => {
                write!(w, "loft::state::STRING_NULL")?;
                return Ok(());
            }
            "OpConvRefFromNull" => {
                write!(w, "DbRef {{ store_nr: 0, rec: 0, pos: 0 }}")?;
                return Ok(());
            }
            "OpGetTextSub" => {
                // text[from..till] → &str slice
                if let [ref text_val, ref from_val, ref till_val] = vals[..] {
                    write!(w, "OpGetTextSub(")?;
                    self.output_code_inner(w, text_val)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, from_val)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, till_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpSizeofRef" => {
                if let [ref val] = vals[..] {
                    write!(w, "OpSizeofRef(cell,")?;
                    self.output_code_inner(w, val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpDatabase" => {
                // OpDatabase modifies its DbRef argument in-place; emit as reassignment.
                if let [ref var_val, ref tp_val] = vals[..] {
                    self.output_code_inner(w, var_val)?;
                    write!(w, " = OpDatabase(cell,")?;
                    self.output_code_inner(w, var_val)?;
                    write!(w, ", ")?;
                    self.emit_i32_slot(w, tp_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpFormatDatabase" | "OpFormatStackDatabase" => {
                // OpFormatDatabase takes a &mut String as the output buffer.
                if let [ref work_val, ref record_val, ref tp_val, ref fmt_val] = vals[..] {
                    write!(w, "OpFormatDatabase(cell,&mut ")?;
                    // work_val is Var(nr) — strip the leading & that output_code_inner adds
                    if let Value::Var(nr) = work_val {
                        let variables = &self.data.def(self.def_nr).variables;
                        write!(w, "var_{}", sanitize(variables.name(*nr)))?;
                    } else {
                        self.output_code_inner(w, work_val)?;
                    }
                    write!(w, ", ")?;
                    self.output_code_inner(w, record_val)?;
                    write!(w, ", ")?;
                    self.emit_i32_slot(w, tp_val)?;
                    write!(w, ", ")?;
                    self.emit_i32_slot(w, fmt_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            // Plan 09 phase 04 — `OpGetRecord` and `OpIterate` emission
            // moved to `crate::generation::ops::key_ops::{OpGetRecordEmitter,
            // OpIterateEmitter}` (registered in build_registry).  The
            // phase 00 step 0.6 registry-first guard at the top of
            // `output_call_inner` routes both names to the emitters
            // before reaching this match.
            "OpStep"
                // vals: [iter_var, data, on, arg]
                // Emit: OpStep(cell,&mut var_iter, data, on, arg)
                if vals.len() == 4 => {
                    write!(w, "OpStep(cell,&mut ")?;
                    if let Value::Var(v) = &vals[0] {
                        let name = sanitize(self.data.def(self.def_nr).variables.name(*v));
                        write!(w, "var_{name}")?;
                    } else {
                        self.output_code_inner(w, &vals[0])?;
                    }
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[1])?;
                    write!(w, ", ")?;
                    self.emit_i32_slot(w, &vals[2])?;
                    write!(w, ", ")?;
                    self.emit_i32_slot(w, &vals[3])?;
                    write!(w, ")")?;
                    return Ok(());
                }
            "OpRemove"
                // vals: [state_var, data, on, tp/arg]
                // Emit: OpRemove(cell,&mut var_state, data, on, arg)
                // The state may be i32 (plain vector) or i64 (sorted/tree iterator).
                if vals.len() == 4 => {
                    write!(w, "OpRemove(cell,&mut ")?;
                    if let Value::Var(v) = &vals[0] {
                        let name = sanitize(self.data.def(self.def_nr).variables.name(*v));
                        write!(w, "var_{name}")?;
                    } else {
                        self.output_code_inner(w, &vals[0])?;
                    }
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[1])?;
                    write!(w, ", ")?;
                    self.emit_i32_slot(w, &vals[2])?;
                    write!(w, ", ")?;
                    self.emit_i32_slot(w, &vals[3])?;
                    write!(w, ")")?;
                    return Ok(());
                }
            // Plan 09 phase 03 — `n_parallel_for` / `n_parallel_for_light`
            // emission moved to `crate::generation::ops::parallel::ParallelForEmitter`
            // (registered in build_registry).  The phase 00 step 0.6
            // registry-first guard at the top of `output_call_inner`
            // routes both names to the emitter before reaching this
            // match.  Phase 04 / phase 06 will retire more arms here
            // following the same pattern.
            _ => {}
        }
        if def_fn.rust.is_empty() {
            self.output_call_user_fn(w, def_fn, vals)
        } else {
            self.output_call_template(w, def_fn, vals)
        }
    }
}
