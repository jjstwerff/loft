// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! N8b.1: Native coroutine state-machine code generation.
//! Translates loft generator functions (returning `iterator<T>`) into Rust
//! state-machine structs implementing `LoftCoroutine`.
//!
//! Scope (N8b.1 + N8b.2): sequential top-level yields only; no loops with
//! yields inside them (those are deferred to N8b.3 — yield from).

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

/// Split the top-level operators of a function body at `Value::Yield` nodes.
///
/// Returns `(pre_statements, yield_value)` pairs — one per yield.
/// Statements that appear before a yield are collected as `pre_statements`.
/// Any statements after the last yield (the function tail) are not emitted
/// in N8b.1 scope, because all test generators end immediately after the
/// last yield.
fn collect_yield_segments(ops: &[Value]) -> Vec<(Vec<Value>, Value)> {
    let mut segments = Vec::new();
    let mut pre: Vec<Value> = Vec::new();
    for op in ops {
        if let Value::Yield(inner) = op {
            segments.push((pre.clone(), *inner.clone()));
            pre.clear();
        } else {
            pre.push(op.clone());
        }
    }
    segments
}

impl Output<'_> {
    /// Emit a loft generator function as a Rust state-machine struct.
    ///
    /// Produces:
    /// 1. `struct {Name}Gen { state: u32, var_param: T, … }` — stores parameters.
    /// 2. `impl LoftCoroutine for {Name}Gen` — `next_i64` match dispatch.
    /// 3. `fn {fn_name}(stores, params) -> Box<dyn LoftCoroutine>` — factory.
    pub(super) fn output_coroutine(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
    ) -> std::io::Result<()> {
        self.start_fn(def_nr);
        let def = self.data.def(def_nr);
        let fn_name = def.name.clone();
        let struct_name = gen_struct_name(&fn_name);

        // Collect segments only when a body is present.
        let body_block = match &def.code {
            Value::Block(bl) => bl.clone(),
            _ => {
                // No body — emit a stub factory returning an exhausted generator.
                writeln!(w, "struct {struct_name} {{}}")?;
                writeln!(w, "impl loft::codegen_runtime::LoftCoroutine for {struct_name} {{")?;
                writeln!(
                    w,
                    "    fn next_i64(&mut self, _stores: &mut Stores) -> i64 \
                     {{ loft::codegen_runtime::COROUTINE_EXHAUSTED }}"
                )?;
                writeln!(w, "}}")?;
                write!(w, "fn {fn_name}(_stores: &mut Stores) -> Box<dyn loft::codegen_runtime::LoftCoroutine>")?;
                writeln!(w, " {{ Box::new({struct_name} {{}}) }}\n")?;
                return Ok(());
            }
        };

        let segments = collect_yield_segments(&body_block.operators);

        // ── 1. Struct definition ─────────────────────────────────────────────
        writeln!(w, "struct {struct_name} {{")?;
        writeln!(w, "    state: u32,")?;
        for attr in &def.attributes {
            // Parameters are stored as owned types: text → String, others unchanged.
            let field_tp = match &attr.typedef {
                Type::Text(_) => "String".to_string(),
                other => rust_type(other, &Context::Variable),
            };
            writeln!(w, "    var_{}: {field_tp},", sanitize(&attr.name))?;
        }
        writeln!(w, "}}\n")?;

        // ── 2. impl LoftCoroutine ────────────────────────────────────────────
        writeln!(
            w,
            "impl loft::codegen_runtime::LoftCoroutine for {struct_name} {{"
        )?;
        writeln!(
            w,
            "    fn next_i64(&mut self, stores: &mut Stores) -> i64 {{"
        )?;
        writeln!(w, "        match self.state {{")?;

        for (state_idx, (pre_stmts, yield_val)) in segments.iter().enumerate() {
            writeln!(w, "            {state_idx} => {{")?;

            // Shadow-bind parameters so existing output_code_inner logic works.
            // In the struct, text params are `String`; bind as `&str` to match
            // the original function's argument context (is_argument returns true).
            for attr in &def.attributes {
                let aname = sanitize(&attr.name);
                match &attr.typedef {
                    Type::Text(_) => writeln!(
                        w,
                        "                let var_{aname}: &str = &self.var_{aname};"
                    )?,
                    _ => writeln!(w, "                let var_{aname} = self.var_{aname};")?,
                }
            }

            // Emit pre-yield statements.
            for stmt in pre_stmts {
                let stmt_code = self.generate_expr_buf(stmt)?;
                writeln!(w, "                {stmt_code};")?;
            }

            // Advance the state counter before returning.
            writeln!(w, "                self.state = {};", state_idx + 1)?;

            // Return the yielded value, cast to i64.
            let yield_code = self.generate_expr_buf(yield_val)?;
            writeln!(w, "                return ({yield_code}) as i64;")?;
            writeln!(w, "            }}")?;
        }

        // Exhausted arm: any state beyond the last yield.
        writeln!(
            w,
            "            _ => loft::codegen_runtime::COROUTINE_EXHAUSTED,"
        )?;
        writeln!(w, "        }}")?;
        writeln!(w, "    }}")?;
        writeln!(w, "}}\n")?;

        // ── 3. Factory function ──────────────────────────────────────────────
        write!(w, "fn {fn_name}(stores: &mut Stores")?;
        for attr in &def.attributes {
            let arg_tp = rust_type(&attr.typedef, &Context::Argument);
            write!(w, ", var_{}: {arg_tp}", sanitize(&attr.name))?;
        }
        writeln!(
            w,
            ") -> Box<dyn loft::codegen_runtime::LoftCoroutine> {{"
        )?;
        writeln!(w, "    let _ = stores;")?;
        writeln!(w, "    Box::new({struct_name} {{")?;
        writeln!(w, "        state: 0,")?;
        for attr in &def.attributes {
            let aname = sanitize(&attr.name);
            match &attr.typedef {
                Type::Text(_) => writeln!(w, "        var_{aname}: var_{aname}.to_string(),")?,
                _ => writeln!(w, "        var_{aname},")?,
            }
        }
        writeln!(w, "    }})")?;
        writeln!(w, "}}\n")
    }
}
