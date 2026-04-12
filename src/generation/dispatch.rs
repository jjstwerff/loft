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
        // P117-native: when a call returns a Reference and the callee has
        // visible Reference params, the returned DbRef may alias a parameter.
        // Deep-copy to prevent aliasing.
        if let (Type::Reference(d_nr, _), Value::Call(fn_nr, args)) = (variables.tp(var), to)
            && self.data.def(*fn_nr).name.starts_with("n_")
            && self.data.def(*fn_nr).code != Value::Null
            && self.data.def(*fn_nr).attributes.iter().any(|a| {
                !a.hidden && matches!(a.typedef, Type::Reference(_, _) | Type::Enum(_, true, _))
            })
        {
            let tp_nr = self.data.def(*d_nr).known_type;
            if !self.declared.contains(&var) {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                writeln!(
                    w,
                    "let mut var_{name}: {tp_str} = stores.null_named(\"var_{name}\");"
                )?;
                self.indent(w)?;
            }
            writeln!(
                w,
                "var_{name} = OpDatabase(stores, var_{name}, {tp_nr}_i32);"
            )?;
            self.indent(w)?;
            // Emit the call into a temporary, then deep-copy.
            write!(w, "{{ let _src = {}(stores", self.data.def(*fn_nr).name)?;
            for arg in args {
                write!(w, ", ")?;
                if let Some(vr) = self.create_stack_var(arg) {
                    let sn = sanitize(variables.name(vr));
                    write!(w, "&mut var_{sn}")?;
                } else {
                    self.output_code_inner(w, arg)?;
                }
            }
            write!(
                w,
                "); OpCopyRecord(stores, _src, var_{name}, {tp_nr}_i32); }}"
            )?;
            return Ok(());
        }
        // When assigning a reference to a reference variable, a pointer copy is not
        // sufficient — emit an OpCopyRecord call for a deep copy.
        // For a first declaration, we also need to allocate a fresh store via
        // OpDatabase(null_named(…)) so the destination has its own record to copy into.
        // For reassignment, the existing destination record is reused in-place.
        if let (Type::Reference(d_nr, _), Value::Var(src)) = (variables.tp(var), to)
            && matches!(variables.tp(*src), Type::Reference(_, _))
        {
            let src_name = sanitize(variables.name(*src));
            let tp_nr = self.data.def(*d_nr).known_type;
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
            writeln!(
                w,
                "var_{name} = OpDatabase(stores, var_{name}, {tp_nr}_i32);"
            )?;
            self.indent(w)?;
            write!(
                w,
                "OpCopyRecord(stores, var_{src_name}, var_{name}, {tp_nr}_i32)"
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
        if let Value::Call(call_dnr, args) = to
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
            write!(w, "{}(stores", def_fn.name)?;
            for (idx, arg) in args.iter().enumerate() {
                write!(w, ", ")?;
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
                // C39: wrap plain Int or If-with-Int values assigned to Function vars.
                let is_fn_ref_var = matches!(variables.tp(var), Type::Function(_, _, _));
                let wrap_fn_ref = is_fn_ref_var && matches!(to, Value::Int(_));
                if wrap_fn_ref {
                    write!(w, "(")?;
                }
                // C39: set fn_ref_context so if-else branches with bare Int
                // values produce (u32, null_DbRef) tuples.  Cleared inside
                // Call argument processing to avoid wrapping OpDatabase args.
                let prev_ctx = self.fn_ref_context;
                if is_fn_ref_var && !wrap_fn_ref {
                    self.fn_ref_context = true;
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
                    // Variable is a narrow integer type (stored as i32), but the RHS expression
                    // (a function returning u16 or an iterator block returning as u16) produces
                    // the narrow type. Add an explicit `as i32` cast.
                    write!(w, " as i32")?;
                } else if let Value::Call(d_nr, _) = to {
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
            Type::Reference(_, dep) | Type::Vector(_, dep) | Type::Enum(_, true, dep) => {
                dep.is_empty()
            }
            _ => false,
        };
        if is_elm || variables.is_inline_ref(var) || !owns_store {
            write!(w, "DbRef {{ store_nr: u16::MAX, rec: 0, pos: 8 }}")?;
        } else {
            let ref_buf_type_id = {
                let var_tp = variables.tp(var).clone();
                if let Type::Vector(elm_tp, _) = &var_tp {
                    let elm_name = elm_tp.name(self.data);
                    self.data.name_type(&format!("main_vector<{elm_name}>"), 0)
                } else {
                    u16::MAX
                }
            };
            if ref_buf_type_id == u16::MAX {
                write!(w, "stores.null_named(\"var_{name}\")")?;
            } else {
                writeln!(w, "stores.null_named(\"var_{name}\");")?;
                self.indent(w)?;
                write!(
                    w,
                    "var_{name} = OpDatabase(stores, var_{name}, {ref_buf_type_id}_i32)"
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
        // C39: clear fn_ref_context inside calls — arguments like OpDatabase's
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
        match name {
            "OpFormatLong" | "OpFormatStackLong" => {
                return self.format_long(w, vals, name == "OpFormatStackLong");
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
                // Emit OpFreeRef(stores, var, "var_name") so LOFT_STORE_LOG shows the loft name.
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
                    // C39: free the closure component of fn-ref (u32, DbRef) variables.
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
                             OpFreeRef(stores, {vn}.1, \"{vn}.1\"); \
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
                    write!(w, "OpFreeRef(stores, ")?;
                    self.output_code_inner(w, db_val)?;
                    write!(w, ", \"{var_name}\")")?;
                    // Reset variable to null sentinel after free.
                    if let Value::Var(_) = db_val {
                        write!(w, "; {var_name}.store_nr = u16::MAX")?;
                    }
                }
                return Ok(());
            }
            "OpCopyRecord" => {
                // Deep copy: copy_block + copy_claims
                if let [ref src, ref dst, ref tp_val] = vals[..] {
                    write!(w, "OpCopyRecord(stores, ")?;
                    self.output_code_inner(w, src)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, dst)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, tp_val)?;
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
                    write!(w, "OpSizeofRef(stores, ")?;
                    self.output_code_inner(w, val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpDatabase" => {
                // OpDatabase modifies its DbRef argument in-place; emit as reassignment.
                if let [ref var_val, ref tp_val] = vals[..] {
                    self.output_code_inner(w, var_val)?;
                    write!(w, " = OpDatabase(stores, ")?;
                    self.output_code_inner(w, var_val)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, tp_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpFormatDatabase" | "OpFormatStackDatabase" => {
                // OpFormatDatabase takes a &mut String as the output buffer.
                if let [ref work_val, ref record_val, ref tp_val, ref fmt_val] = vals[..] {
                    write!(w, "OpFormatDatabase(stores, &mut ")?;
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
                    self.output_code_inner(w, tp_val)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, fmt_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpGetRecord" => {
                // vals: [data, db_tp, count, key1, key2, …]
                // Emit: OpGetRecord(stores, data, db_tp, &[Content::…, …])
                if vals.len() >= 3
                    && let (Value::Int(db_tp), Value::Int(_count)) = (&vals[1], &vals[2])
                {
                    let db_tp = *db_tp;
                    let key_types: Vec<i8> = self
                        .stores
                        .types
                        .get(usize::try_from(db_tp).unwrap_or(0))
                        .map(|t| t.keys.iter().map(|k| k.type_nr).collect())
                        .unwrap_or_default();
                    let key_vals = &vals[3..];
                    write!(w, "OpGetRecord(stores, ")?;
                    self.output_code_inner(w, &vals[0])?;
                    write!(w, ", {db_tp}_i32, &[")?;
                    for (i, key_val) in key_vals.iter().enumerate() {
                        if i > 0 {
                            write!(w, ", ")?;
                        }
                        let type_nr = key_types.get(i).copied().unwrap_or(1);
                        self.emit_content(w, key_val, type_nr)?;
                    }
                    write!(w, "])")?;
                    return Ok(());
                }
            }
            "OpIterate" => {
                // vals: [data, on, arg, Keys(keys), from_count, from_vals…, till_count, till_vals…]
                // Emit: OpIterate(stores, data, on, arg, &[Key{…}], &[Content::…], &[Content::…])
                if vals.len() >= 4
                    && let Value::Keys(keys) = &vals[3]
                {
                    let keys = keys.clone();
                    let rest = &vals[4..];
                    let from_count = if let Some(Value::Int(n)) = rest.first() {
                        usize::try_from(*n).unwrap_or(0)
                    } else {
                        0
                    };
                    let till_start = 1 + from_count;
                    let till_count = if let Some(Value::Int(n)) = rest.get(till_start) {
                        usize::try_from(*n).unwrap_or(0)
                    } else {
                        0
                    };
                    let from_vals = rest.get(1..till_start).unwrap_or(&[]);
                    let till_vals = rest
                        .get(till_start + 1..till_start + 1 + till_count)
                        .unwrap_or(&[]);
                    write!(w, "OpIterate(stores, ")?;
                    self.output_code_inner(w, &vals[0])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[1])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[2])?;
                    write!(w, ", &[")?;
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
                    write!(w, "], &[")?;
                    for (i, v) in from_vals.iter().enumerate() {
                        if i > 0 {
                            write!(w, ", ")?;
                        }
                        let type_nr = keys.get(i).map_or(1, |k| k.type_nr);
                        self.emit_content(w, v, type_nr)?;
                    }
                    write!(w, "], &[")?;
                    for (i, v) in till_vals.iter().enumerate() {
                        if i > 0 {
                            write!(w, ", ")?;
                        }
                        let type_nr = keys.get(i).map_or(1, |k| k.type_nr);
                        self.emit_content(w, v, type_nr)?;
                    }
                    write!(w, "])")?;
                    return Ok(());
                }
            }
            "OpStep" => {
                // vals: [iter_var, data, on, arg]
                // Emit: OpStep(stores, &mut var_iter, data, on, arg)
                if vals.len() == 4 {
                    write!(w, "OpStep(stores, &mut ")?;
                    if let Value::Var(v) = &vals[0] {
                        let name = sanitize(self.data.def(self.def_nr).variables.name(*v));
                        write!(w, "var_{name}")?;
                    } else {
                        self.output_code_inner(w, &vals[0])?;
                    }
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[1])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[2])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[3])?;
                    write!(w, ")")?;
                    return Ok(());
                }
            }
            "OpRemove" => {
                // vals: [state_var, data, on, tp/arg]
                // Emit: OpRemove(stores, &mut var_state, data, on, arg)
                // The state may be i32 (plain vector) or i64 (sorted/tree iterator).
                if vals.len() == 4 {
                    write!(w, "OpRemove(stores, &mut ")?;
                    if let Value::Var(v) = &vals[0] {
                        let name = sanitize(self.data.def(self.def_nr).variables.name(*v));
                        write!(w, "var_{name}")?;
                    } else {
                        self.output_code_inner(w, &vals[0])?;
                    }
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[1])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[2])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[3])?;
                    write!(w, ")")?;
                    return Ok(());
                }
            }
            "n_parallel_for" | "n_parallel_for_light" => {
                // Special-case: replace n_parallel_for(input, elem_sz, ret_sz, threads, fn_d_nr, extras..., n_extra)
                // with n_parallel_for_native(..., |stores, elm| { worker_fn(stores, elm, extras...) as i64 }).
                if vals.len() >= 5
                    && let Value::Int(fn_d_nr) = &vals[4]
                    && *fn_d_nr >= 0
                {
                    let fn_d_nr = (*fn_d_nr).cast_unsigned();
                    let worker_def = self.data.def(fn_d_nr);
                    let worker_name = worker_def.name.clone();
                    let worker_ret = worker_def.returned.clone();
                    // Extra context args: vals[5..len-1], last element is n_extra count.
                    let n_extra = if vals.len() > 6 { vals.len() - 6 } else { 0 };
                    // Emit let-bindings for extra args so they can be captured by the closure.
                    for i in 0..n_extra {
                        write!(w, "{{ let _ex{i} = ")?;
                        self.output_code_inner(w, &vals[5 + i])?;
                        write!(w, "; ")?;
                    }
                    let is_ref = matches!(&worker_ret, Type::Reference(_, _));
                    let par_fn = if matches!(&worker_ret, Type::Text(_)) {
                        "n_parallel_for_text_native"
                    } else if is_ref {
                        "n_parallel_for_ref_native"
                    } else {
                        "n_parallel_for_native"
                    };
                    write!(w, "{par_fn}(stores, ")?;
                    self.output_code_inner(w, &vals[0])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[1])?;
                    write!(w, ", ")?;
                    if is_ref {
                        // For ref mode, pass struct_size and known_type instead of return_size.
                        let (struct_size, known_type) =
                            if let Type::Reference(d_nr, _) = &worker_ret {
                                let kt = self.data.def(*d_nr).known_type;
                                (i32::from(self.stores.size(kt)), i32::from(kt))
                            } else {
                                (0, 0)
                            };
                        write!(w, "{struct_size}, {known_type}, ")?;
                    } else {
                        self.output_code_inner(w, &vals[2])?;
                        write!(w, ", ")?;
                    }
                    self.output_code_inner(w, &vals[3])?;
                    // Build the extra arg list for the worker call inside the closure.
                    let extras = {
                        use std::fmt::Write;
                        let mut s = String::new();
                        for i in 0..n_extra {
                            write!(s, ", _ex{i}").unwrap();
                        }
                        s
                    };
                    // Generate closure with return-type-specific conversion.
                    match &worker_ret {
                        Type::Text(_) => write!(
                            w,
                            ", |stores, elm| {{ let mut _w = String::new(); {worker_name}(stores, elm{extras}, &mut _w); _w }})"
                        )?,
                        Type::Reference(_, _) => write!(
                            w,
                            ", |stores, elm| {{ {worker_name}(stores, elm{extras}) }})"
                        )?,
                        Type::Float | Type::Single => write!(
                            w,
                            ", |stores, elm| {{ {worker_name}(stores, elm{extras}).to_bits() as i64 }})"
                        )?,
                        _ => write!(
                            w,
                            ", |stores, elm| {{ {worker_name}(stores, elm{extras}) as i64 }})"
                        )?,
                    }
                    // Close the let-binding braces.
                    for _ in 0..n_extra {
                        write!(w, " }}")?;
                    }
                    return Ok(());
                }
            }
            _ => {}
        }
        if def_fn.rust.is_empty() {
            self.output_call_user_fn(w, def_fn, vals)
        } else {
            self.output_call_template(w, def_fn, vals)
        }
    }
}
