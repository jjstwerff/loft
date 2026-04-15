// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! N8b.1 + N8b.2 + N8b.3: Native coroutine state-machine code generation.
//! Translates loft generator functions (returning `iterator<T>`) into Rust
//! state-machine structs implementing `LoftCoroutine`.
//!
//! Scope:
//! - N8b.1/N8b.2: sequential top-level yields.
//! - N8b.3: `yield from` delegation — the sub-generator is stored directly in
//!   the outer struct as `Option<Box<dyn LoftCoroutine>>` to avoid a `RefCell`
//!   double-borrow when advancing the sub-generator from within the outer
//!   generator's `next_i64` call.

use crate::data::{Context, Type, Value};
use std::io::Write;

use super::{Output, rust_type, sanitize};

/// Derive the generator struct name from the loft function name.
/// `n_count` → `NCountGen`, `n_gen_len` → `NGenLenGen`.
fn gen_struct_name(fn_name: &str) -> String {
    let base = fn_name.strip_prefix("n_").unwrap_or(fn_name);
    let capitalized: String = base
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect();
    format!("N{capitalized}Gen")
}

/// A segment of the coroutine body.
#[derive(Clone)]
enum YieldSegment {
    /// Top-level `yield expr` with preceding statements.
    Simple { pre: Vec<Value>, val: Value },
    /// `yield from sub_gen()` block.
    /// - `pre`: statements before the block (in the outer context)
    /// - `init`: expression that creates the sub-generator (e.g. `n_inner(stores)`)
    /// - `state_idx`: the state number for this segment (used to name the struct field)
    YieldFrom { pre: Vec<Value>, init: Value },
    /// A for-loop body containing yields.  The factory function runs the loop
    /// eagerly and collects all yielded values into a `Vec<i64>` buffer; `next_i64`
    /// just returns items from that buffer.  This covers the common
    /// `for i in range { yield expr; }` pattern without requiring a full
    /// state-machine decomposition of the range-iteration IR.
    ForLoopBody { pre: Vec<Value>, body: Value },
}

/// Try to recognise a `yield from` desugared block.
///
/// The parser desugars `yield from inner()` into exactly:
/// ```text
/// Block {
///   ops: [
///     Set(sub_var, init_expr),
///     Loop { ops: [ Set(item_var, next_call), If(break_test, break_val, Null), Yield(Var(item_var)) ] }
///   ]
/// }
/// ```
/// Returns `init_expr` when matched.
fn detect_yield_from(val: &Value) -> Option<Value> {
    let Value::Block(bl) = val else { return None };
    if bl.operators.len() != 2 {
        return None;
    }
    let Value::Set(sub_var, init_expr) = &bl.operators[0] else {
        return None;
    };
    let Value::Loop(lp) = &bl.operators[1] else {
        return None;
    };
    if lp.operators.len() != 3 {
        return None;
    }
    let Value::Set(item_var, _) = &lp.operators[0] else {
        return None;
    };
    // Third op must be Yield(Var(item_var)).
    if let Value::Yield(yv) = &lp.operators[2]
        && matches!(yv.as_ref(), Value::Var(v) if v == item_var)
    {
        // Only the init expression is needed — sub_var is an internal detail.
        let _ = sub_var;
        Some(*init_expr.clone())
    } else {
        None
    }
}

/// Returns true if `v` contains any `Value::Yield` node at any depth.
fn contains_yield(v: &Value) -> bool {
    match v {
        Value::Yield(_) => true,
        Value::Block(bl) | Value::Loop(bl) => bl.operators.iter().any(contains_yield),
        Value::If(_, t, f) => contains_yield(t) || contains_yield(f),
        Value::Set(_, rhs) | Value::Drop(rhs) | Value::Return(rhs) => contains_yield(rhs),
        _ => false,
    }
}

/// Scan the top-level operators of a function body and build yield segments.
fn collect_segments(ops: &[Value]) -> Vec<YieldSegment> {
    let mut segments = Vec::new();
    let mut pre: Vec<Value> = Vec::new();
    for op in ops {
        // Generator functions written as `fn() -> iterator<T> { for x in ... { yield x } }`
        // get an implicit `Return(<for-block>)` wrap from block_result.  Peek through
        // Return/Insert wrappers so the inner Block-with-yields still becomes a
        // ForLoopBody segment instead of an opaque `pre` statement.
        let inner_op = match op {
            Value::Return(inner) | Value::Drop(inner) => inner.as_ref(),
            _ => op,
        };
        if let Value::Yield(inner) = inner_op {
            segments.push(YieldSegment::Simple {
                pre: std::mem::take(&mut pre),
                val: *inner.clone(),
            });
        } else if let Some(init) = detect_yield_from(inner_op) {
            segments.push(YieldSegment::YieldFrom {
                pre: std::mem::take(&mut pre),
                init,
            });
        } else if matches!(inner_op, Value::Block(_)) && contains_yield(inner_op) {
            // A block (typically a for-loop) that contains yields somewhere inside.
            // Use the eager-collect approach: the factory will run the block and
            // push all yielded values to a Vec<i64>; next_i64 pops from that buffer.
            segments.push(YieldSegment::ForLoopBody {
                pre: std::mem::take(&mut pre),
                body: inner_op.clone(),
            });
        } else {
            pre.push(op.clone());
        }
    }
    segments
}

/// Emit the struct definition for a coroutine state machine.
fn emit_struct_def(
    w: &mut dyn Write,
    struct_name: &str,
    attrs: &[crate::data::Attribute],
    segments: &[YieldSegment],
) -> std::io::Result<()> {
    writeln!(w, "struct {struct_name} {{")?;
    writeln!(w, "    state: u32,")?;
    for attr in attrs {
        let field_tp = match &attr.typedef {
            Type::Text(_) => "String".to_string(),
            other => rust_type(other, &Context::Variable),
        };
        writeln!(w, "    var_{}: {field_tp},", sanitize(&attr.name))?;
    }
    // N8b.3: one inline sub-generator field per yield-from segment.
    // Stored as `Option<Box<dyn LoftCoroutine>>` to avoid `RefCell` double-borrow
    // when advancing the sub-generator from inside the outer generator's `next_i64`.
    for (idx, seg) in segments.iter().enumerate() {
        if matches!(seg, YieldSegment::YieldFrom { .. }) {
            writeln!(
                w,
                "    sub_{idx}: Option<Box<dyn loft::codegen_runtime::LoftCoroutine>>,"
            )?;
        }
    }
    // ForLoopBody: add a value buffer + index for the eager-collect approach.
    if segments
        .iter()
        .any(|s| matches!(s, YieldSegment::ForLoopBody { .. }))
    {
        writeln!(w, "    __values: Vec<i64>,")?;
        writeln!(w, "    __idx: usize,")?;
    }
    writeln!(w, "}}\n")
}

/// Emit the factory function that allocates and returns a boxed coroutine.
fn emit_factory_fn(
    w: &mut dyn Write,
    fn_name: &str,
    struct_name: &str,
    attrs: &[crate::data::Attribute],
    segments: &[YieldSegment],
) -> std::io::Result<()> {
    // ForLoopBody: the entire factory is emitted by Output::emit_for_body_factory.
    if segments
        .iter()
        .any(|s| matches!(s, YieldSegment::ForLoopBody { .. }))
    {
        return Ok(());
    }
    write!(w, "fn {fn_name}(stores: &mut Stores")?;
    for attr in attrs {
        let arg_tp = rust_type(&attr.typedef, &Context::Argument);
        write!(w, ", var_{}: {arg_tp}", sanitize(&attr.name))?;
    }
    writeln!(w, ") -> Box<dyn loft::codegen_runtime::LoftCoroutine> {{")?;
    writeln!(w, "    let _ = stores;")?;
    writeln!(w, "    Box::new({struct_name} {{")?;
    writeln!(w, "        state: 0,")?;
    for attr in attrs {
        let aname = sanitize(&attr.name);
        match &attr.typedef {
            Type::Text(_) => writeln!(w, "        var_{aname}: var_{aname}.to_string(),")?,
            _ => writeln!(w, "        var_{aname},")?,
        }
    }
    // N8b.3: initialise sub-generator fields to None.
    for (idx, seg) in segments.iter().enumerate() {
        if matches!(seg, YieldSegment::YieldFrom { .. }) {
            writeln!(w, "        sub_{idx}: None,")?;
        }
    }
    writeln!(w, "    }})")?;
    writeln!(w, "}}\n")
}

impl Output<'_> {
    /// Generate the factory call for a sub-generator, WITHOUT the `alloc_coroutine`
    /// wrapper.  The `init` expression is always `Value::Call(inner_fn, args)` for a
    /// generator function; we call the Rust factory directly to get a
    /// `Box<dyn LoftCoroutine>` that we can store inline in the outer struct.
    fn gen_inner_factory(&mut self, init: &Value) -> std::io::Result<String> {
        if let Value::Call(d_nr, args) = init {
            let fn_name = self.data.def(*d_nr).name.clone();
            let mut buf = format!("{fn_name}(stores");
            for arg in args {
                buf += ", ";
                buf += &self.generate_expr_buf(arg)?;
            }
            buf += ")";
            Ok(buf)
        } else {
            // Fallback — should not happen for well-formed yield-from.
            self.generate_expr_buf(init)
        }
    }

    /// Emit the `next_i64` method body for a coroutine state machine.
    fn emit_next_i64(
        &mut self,
        w: &mut dyn Write,
        attrs: &[crate::data::Attribute],
        segments: &[YieldSegment],
        has_yf: bool,
    ) -> std::io::Result<()> {
        writeln!(
            w,
            "    fn next_i64(&mut self, stores: &mut Stores) -> i64 {{"
        )?;
        // N8b.3: wrap in `loop {}` so yield-from states can `continue` to the
        // next state immediately after sub-generator exhaustion.
        if has_yf {
            writeln!(w, "        loop {{")?;
        }
        writeln!(w, "        match self.state {{")?;
        for (state_idx, segment) in segments.iter().enumerate() {
            writeln!(w, "            {state_idx} => {{")?;
            // Shadow-bind parameters.
            for attr in attrs {
                let aname = sanitize(&attr.name);
                match &attr.typedef {
                    Type::Text(_) => writeln!(
                        w,
                        "                let var_{aname}: &str = &self.var_{aname};"
                    )?,
                    _ => writeln!(w, "                let var_{aname} = self.var_{aname};")?,
                }
            }
            match segment {
                YieldSegment::Simple { pre, val } => {
                    for stmt in pre {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "                {stmt_code};")?;
                    }
                    writeln!(w, "                self.state = {};", state_idx + 1)?;
                    let yield_code = self.generate_expr_buf(val)?;
                    writeln!(w, "                return ({yield_code}) as i64;")?;
                }
                YieldSegment::YieldFrom { pre, init } => {
                    for stmt in pre {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "                {stmt_code};")?;
                    }
                    writeln!(w, "                if self.sub_{state_idx}.is_none() {{")?;
                    let factory = self.gen_inner_factory(init)?;
                    writeln!(
                        w,
                        "                    self.sub_{state_idx} = Some({factory});"
                    )?;
                    writeln!(w, "                }}")?;
                    writeln!(
                        w,
                        "                let val = self.sub_{state_idx}.as_mut().unwrap().next_i64(stores);"
                    )?;
                    writeln!(
                        w,
                        "                if val == loft::codegen_runtime::COROUTINE_EXHAUSTED {{"
                    )?;
                    writeln!(w, "                    self.sub_{state_idx} = None;")?;
                    writeln!(w, "                    self.state = {};", state_idx + 1)?;
                    writeln!(w, "                    continue;")?;
                    writeln!(w, "                }}")?;
                    writeln!(w, "                return val;")?;
                }
                YieldSegment::ForLoopBody { .. } => {
                    // Values were collected eagerly in the factory. Just pop from the buffer.
                    writeln!(w, "                if self.__idx < self.__values.len() {{")?;
                    writeln!(w, "                    let v = self.__values[self.__idx];")?;
                    writeln!(w, "                    self.__idx += 1;")?;
                    writeln!(w, "                    return v;")?;
                    writeln!(w, "                }}")?;
                    writeln!(
                        w,
                        "                return loft::codegen_runtime::COROUTINE_EXHAUSTED;"
                    )?;
                }
            }
            writeln!(w, "            }}")?;
        }
        // Exhausted arm.
        if has_yf {
            writeln!(
                w,
                "            _ => return loft::codegen_runtime::COROUTINE_EXHAUSTED,"
            )?;
        } else {
            writeln!(
                w,
                "            _ => loft::codegen_runtime::COROUTINE_EXHAUSTED,"
            )?;
        }
        writeln!(w, "        }}")?;
        if has_yf {
            writeln!(w, "        }}")?; // close loop
        }
        writeln!(w, "    }}")
    }

    /// Emit a loft generator function as a Rust state-machine struct.
    pub(super) fn output_coroutine(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
    ) -> std::io::Result<()> {
        self.start_fn(def_nr);
        let def = self.data.def(def_nr);
        let fn_name = def.name.clone();
        let struct_name = gen_struct_name(&fn_name);

        // Emit a minimal stub for bodyless functions and return early.
        let Value::Block(body_block) = &def.code.clone() else {
            writeln!(w, "struct {struct_name} {{}}")?;
            writeln!(
                w,
                "impl loft::codegen_runtime::LoftCoroutine for {struct_name} {{"
            )?;
            writeln!(
                w,
                "    fn next_i64(&mut self, _stores: &mut Stores) -> i64 \
                 {{ loft::codegen_runtime::COROUTINE_EXHAUSTED }}"
            )?;
            writeln!(w, "}}")?;
            writeln!(
                w,
                "fn {fn_name}(_stores: &mut Stores) -> Box<dyn loft::codegen_runtime::LoftCoroutine> \
                 {{ Box::new({struct_name} {{}}) }}\n"
            )?;
            return Ok(());
        };

        let segments = collect_segments(&body_block.operators);
        let has_yf = segments
            .iter()
            .any(|s| matches!(s, YieldSegment::YieldFrom { .. }));
        let attrs: Vec<_> = def.attributes.clone();

        // ── 1. Struct definition ─────────────────────────────────────────────
        emit_struct_def(w, &struct_name, &attrs, &segments)?;

        // ── 2. impl LoftCoroutine ────────────────────────────────────────────
        writeln!(
            w,
            "impl loft::codegen_runtime::LoftCoroutine for {struct_name} {{"
        )?;
        self.emit_next_i64(w, &attrs, &segments, has_yf)?;
        writeln!(w, "}}\n")?;

        // ── 3. Factory function ──────────────────────────────────────────────
        let def = self.data.def(def_nr);
        let attrs: Vec<_> = def.attributes.clone();
        let has_for_body = segments
            .iter()
            .any(|s| matches!(s, YieldSegment::ForLoopBody { .. }));
        emit_factory_fn(w, &fn_name, &struct_name, &attrs, &segments)?;
        if has_for_body {
            self.emit_for_body_factory(w, &fn_name, &struct_name, &attrs, &segments)?;
        }
        Ok(())
    }

    /// Emit the factory function for a generator that contains for-loop bodies
    /// with yields.  Runs the body eagerly, pushing all yielded values to a Vec.
    fn emit_for_body_factory(
        &mut self,
        w: &mut dyn Write,
        fn_name: &str,
        struct_name: &str,
        attrs: &[crate::data::Attribute],
        segments: &[YieldSegment],
    ) -> std::io::Result<()> {
        write!(w, "fn {fn_name}(stores: &mut Stores")?;
        for attr in attrs {
            let arg_tp = rust_type(&attr.typedef, &Context::Argument);
            write!(w, ", var_{}: {arg_tp}", sanitize(&attr.name))?;
        }
        writeln!(w, ") -> Box<dyn loft::codegen_runtime::LoftCoroutine> {{")?;
        // Declare local copies of params for use in the body.
        for attr in attrs {
            let aname = sanitize(&attr.name);
            match &attr.typedef {
                Type::Text(_) => writeln!(w, "    let var_{aname}: &str = var_{aname};")?,
                _ => writeln!(w, "    let var_{aname} = var_{aname};")?,
            }
        }
        writeln!(w, "    let mut __values: Vec<i64> = Vec::new();")?;
        // Run each for-loop body with yield_collect enabled.
        self.yield_collect = true;
        for seg in segments {
            match seg {
                YieldSegment::ForLoopBody { pre, body } => {
                    for stmt in pre {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "    {stmt_code};")?;
                    }
                    let body_code = self.generate_expr_buf(body)?;
                    writeln!(w, "    {body_code};")?;
                }
                YieldSegment::Simple { pre, val } => {
                    for stmt in pre {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "    {stmt_code};")?;
                    }
                    let val_code = self.generate_expr_buf(val)?;
                    writeln!(w, "    __values.push(({val_code}) as i64);")?;
                }
                YieldSegment::YieldFrom { pre, init } => {
                    // Eagerly drain the sub-generator.
                    for stmt in pre {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "    {stmt_code};")?;
                    }
                    let factory = self.gen_inner_factory(init)?;
                    writeln!(w, "    {{")?;
                    writeln!(w, "        let mut __sub = {factory};")?;
                    writeln!(w, "        loop {{")?;
                    writeln!(w, "            let v = __sub.next_i64(stores);")?;
                    writeln!(
                        w,
                        "            if v == loft::codegen_runtime::COROUTINE_EXHAUSTED {{ break; }}"
                    )?;
                    writeln!(w, "            __values.push(v);")?;
                    writeln!(w, "        }}")?;
                    writeln!(w, "    }}")?;
                }
            }
        }
        self.yield_collect = false;
        writeln!(w, "    Box::new({struct_name} {{")?;
        writeln!(w, "        state: 0,")?;
        for attr in attrs {
            let aname = sanitize(&attr.name);
            match &attr.typedef {
                Type::Text(_) => writeln!(w, "        var_{aname}: var_{aname}.to_string(),")?,
                _ => writeln!(w, "        var_{aname},")?,
            }
        }
        // ForLoopBody: value buffer + index.
        writeln!(w, "        __values,")?;
        writeln!(w, "        __idx: 0,")?;
        writeln!(w, "    }})")?;
        writeln!(w, "}}\n")
    }
}
