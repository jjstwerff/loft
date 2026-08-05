// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

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

use crate::coroutine_layout::{YieldSlot, tuple_kinds};
use crate::data::{Context, Type, Value};
use std::io::Write;

use super::{Output, rust_type, sanitize};

/// Producer side of the layout-driven yield codec (`coroutine_layout`).  Write
/// one yielded tuple element of kind `kind` into the `dest` transport buffer at
/// `slot`.  The consumer's [`yield_slot_read`] is the exact mirror — both are
/// derived from the same `YieldSlot`, so they agree by construction.
pub(crate) fn yield_slot_write(
    w: &mut dyn Write,
    kind: YieldSlot,
    slot: usize,
    expr: &str,
) -> std::io::Result<()> {
    match kind {
        YieldSlot::Int | YieldSlot::CharI32 | YieldSlot::Routine => {
            writeln!(w, "                dest[{slot}] = ({expr}) as i64;")
        }
        YieldSlot::Bool => writeln!(w, "                dest[{slot}] = (({expr}) as u8) as i64;"),
        // f64::to_bits → u64 / f32::to_bits → u32, both zero-extend to i64.
        YieldSlot::F64 | YieldSlot::F32 => {
            writeln!(
                w,
                "                dest[{slot}] = (({expr}).to_bits()) as i64;"
            )
        }
        YieldSlot::Ref => {
            let s1 = slot + 1;
            writeln!(
                w,
                "                {{ let _r = {expr}; \
                 dest[{slot}] = _r.store_nr as i64; \
                 dest[{s1}] = ((_r.rec as u64) | ((_r.pos as u64) << 32)) as i64; }}"
            )
        }
    }
}

/// Consumer side of the layout-driven yield codec — the exact mirror of
/// [`yield_slot_write`].  Returns the Rust expression that reconstructs a
/// tuple element of kind `kind` from the `_loft_yield_buf` transport buffer at
/// `slot`.
#[must_use]
pub(crate) fn yield_slot_read(kind: YieldSlot, slot: usize) -> String {
    let s1 = slot + 1;
    match kind {
        YieldSlot::Int => format!("_loft_yield_buf[{slot}]"),
        YieldSlot::CharI32 => format!("(_loft_yield_buf[{slot}] as i32)"),
        YieldSlot::Bool => format!("(_loft_yield_buf[{slot}] != 0)"),
        YieldSlot::F64 => format!("f64::from_bits(_loft_yield_buf[{slot}] as u64)"),
        YieldSlot::F32 => format!("f32::from_bits(_loft_yield_buf[{slot}] as u32)"),
        YieldSlot::Routine => format!("(_loft_yield_buf[{slot}] as u32)"),
        YieldSlot::Ref => format!(
            "DbRef {{ store_nr: (_loft_yield_buf[{slot}] as u16), \
             rec: (_loft_yield_buf[{s1}] as u64 as u32), \
             pos: (((_loft_yield_buf[{s1}] as u64) >> 32) as u32) }}"
        ),
    }
}

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
    let Value::Block(bl) = val.unspan() else {
        return None;
    };
    if bl.operators.len() != 2 {
        return None;
    }
    let Value::Set(sub_var, init_expr) = bl.operators[0].unspan() else {
        return None;
    };
    let Value::Loop(lp) = bl.operators[1].unspan() else {
        return None;
    };
    if lp.operators.len() != 3 {
        return None;
    }
    let Value::Set(item_var, _) = lp.operators[0].unspan() else {
        return None;
    };
    // Third op must be Yield(Var(item_var)).
    if let Value::Yield(yv) = lp.operators[2].unspan()
        && matches!(yv.as_ref().unspan(), Value::Var(v) if v == item_var)
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
    v.any_node(&mut |n| matches!(n, Value::Yield(_)))
}

/// Scan the top-level operators of a function body and build yield segments, plus the TAIL —
/// the operators AFTER the last yield.
///
/// The tail used to be dropped on the floor: `pre` accumulated it and the function returned
/// only `segments`.  So `--native` silently skipped every statement past the final `yield`,
/// where `--interpret` runs them on the `next()` that exhausts the generator.  A `print` after
/// the last yield vanished, and — because a generator's scope-exit `OpFreeRef`s live exactly
/// there — every heap local a generator owned was leaked.  A dropped side effect is a wrong
/// answer, not a missing optimisation, so the tail is now emitted in its own state.
fn collect_segments(ops: &[Value]) -> (Vec<YieldSegment>, Vec<Value>) {
    let mut segments = Vec::new();
    let mut pre: Vec<Value> = Vec::new();
    for op in ops {
        // Generator functions written as `fn() -> iterator<T> { for x in ... { yield x } }`
        // get an implicit `Return(<for-block>)` wrap from block_result.  Peek through
        // Return/Insert wrappers so the inner Block-with-yields still becomes a
        // ForLoopBody segment instead of an opaque `pre` statement.
        let inner_op = match op.unspan() {
            Value::Return(inner) | Value::Drop(inner) => inner.as_ref().unspan(),
            other => other,
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
        } else if matches!(
            inner_op,
            Value::Block(_) | Value::Loop(_) | Value::If(_, _, _)
        ) && contains_yield(inner_op)
        {
            // A block (for-loop), loop (while/loop), or conditional
            // (if/else) that contains yields somewhere inside.  Use
            // the eager-collect approach: the factory will run the
            // construct and push all yielded values to a Vec<i64>;
            // next_i64 pops from that buffer.
            //
            // P210 — `Value::Loop` previously fell through to the
            // `pre` accumulator, so a generator like `fn g() { i = 0;
            // while i < n { yield i; i += 1; } }` produced an empty
            // state machine (every `next_i64` returned
            // COROUTINE_EXHAUSTED).  The for-loop body case worked
            // because its body is `Value::Block`; while-loops were
            // missed.
            //
            // P230 — `Value::If` was likewise missed.  A generator
            // like `if cond { yield x; }` left the conditional in
            // `pre`, and the per-state code emit then walked the IR
            // via `output_code_inner` which hits `Value::Yield` and
            // emits the literal Rust `yield` keyword (only valid in
            // unstable `gen` blocks) → E0627 native rejection.
            // Routing the if-with-yield through the eager-collect
            // factory uses the `yield_collect` mode that emits
            // `__values.push(...)` instead of `yield ...`, mirroring
            // how Block-with-yield was already handled.
            segments.push(YieldSegment::ForLoopBody {
                pre: std::mem::take(&mut pre),
                body: inner_op.clone(),
            });
        } else {
            pre.push(op.clone());
        }
    }
    // The trailing `Return` is the generator's implicit end — the state machine expresses that
    // with its exhausted sentinel, so emitting a Rust `return` of the loft value would answer
    // the wrong thing.  UNWRAP it rather than dropping it: a void tail expression arrives as
    // `return print(…)`, and the call still has to happen.  Dropping the whole node is how the
    // first cut emitted a tail state containing nothing but a line marker.
    while let Some(last) = pre.last().map(|v| v.unspan().clone()) {
        match last {
            Value::Null => {
                pre.pop();
            }
            Value::Return(inner) if matches!(inner.unspan(), Value::Null) => {
                pre.pop();
            }
            Value::Return(inner) => {
                pre.pop();
                pre.push(*inner);
                break;
            }
            _ => break,
        }
    }
    (segments, pre)
}

/// P224: collect coroutine-body locals that need to persist across
/// `next_*` calls.  Variables `Set` in one state and `Var`-read in
/// another would otherwise be scoped to a single match arm and produce
/// E0425 ("cannot find value `var_X` in this scope") at compile time
/// (or, worse, silently lose the value when the next state runs).
///
/// Every non-argument, user-named local of a primitive (Copy), text, or HEAP type.
///
/// P224 skipped the heap types — *"they would need Store-allocation cascade in the
/// factory"* — and that was the bug this doc predicted: `--native` could not compile ANY
/// generator holding a struct, vector or keyed-collection local, whether or not it was ever
/// reassigned (`fn g() -> iterator<integer> { s = P { n: 11 }; yield s.n; yield s.n; }` is
/// enough).  The E0425 named above is exactly what it produced, on every such generator.
///
/// No cascade turned out to be needed.  A heap local is a `DbRef` — `Copy`, and the same
/// shape the `ForLoopBody` value buffer already stores — so the factory initialises it to
/// `DbRef::NULL` and the body's own `OpDatabase` fills it on first use, which is precisely
/// what the in-arm `let mut var_s: DbRef = DbRef::NULL` did before.  The store itself lives
/// in `Stores`, which outlives the coroutine, so nothing is allocated at factory time.
///
/// Compiler-internal `__*` locals (work-text format buffers
/// `__work_*`, yield-from machinery `__yf_*`, vector-literal
/// backing `__vdb_*`, etc.) are excluded — they have their own
/// emission paths (P218 pre-declares `__work_*` at function scope;
/// the eager-collect factory builds `__yf_*` / `__vdb_*` inline)
/// and adding them as struct fields would conflict with those.
fn coroutine_persistent_locals(data: &crate::data::Data, def_nr: u32) -> Vec<(u16, Type)> {
    let var_table = data.def(def_nr).variables();
    let next = var_table.next_var();
    let mut out = Vec::new();
    for v in 0..next {
        if var_table.is_argument(v) {
            continue;
        }
        let name = var_table.name(v);
        if name.starts_with("__") {
            continue;
        }
        let tp = var_table.tp(v);
        // The heap arm mirrors `rust_type`'s DbRef group exactly: every type that lowers
        // to a `DbRef` is storable as a struct field, so the two lists must not drift.
        let suitable = matches!(
            tp,
            Type::Integer(_)
                | Type::Boolean
                | Type::Character
                | Type::Float
                | Type::Single
                | Type::Enum(_, _, _)
                | Type::Text(_)
                | Type::Reference(_, _)
                | Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Index(_, _, _)
        );
        if !suitable {
            continue;
        }
        out.push((v, tp.clone()));
    }
    out
}

/// Emit the struct definition for a coroutine state machine.
#[allow(clippy::too_many_arguments)]
fn emit_struct_def(
    w: &mut dyn Write,
    struct_name: &str,
    attrs: &[crate::data::Attribute],
    segments: &[YieldSegment],
    yield_tp: &Type,
    persistent: &[(u16, Type)],
    data: &crate::data::Data,
    def_nr: u32,
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
    // P224: persistent function-locals as struct fields.
    let var_table = data.def(def_nr).variables();
    for (v, tp) in persistent {
        let n = sanitize(var_table.name(*v));
        let field_tp = match tp {
            Type::Text(_) => "String".to_string(),
            other => rust_type(other, &Context::Variable),
        };
        writeln!(w, "    var_{n}: {field_tp},")?;
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
        let elem_ty = match yield_tp {
            Type::Text(_) => "String",
            // @P326 — Reference / Vector / struct-enum yields are DbRef-shaped.
            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => "DbRef",
            _ => "i64",
        };
        writeln!(w, "    __values: Vec<{elem_ty}>,")?;
        writeln!(w, "    __idx: usize,")?;
    }
    writeln!(w, "}}\n")
}

/// Emit the factory function that allocates and returns a boxed coroutine.
#[allow(clippy::too_many_arguments)]
fn emit_factory_fn(
    w: &mut dyn Write,
    fn_name: &str,
    struct_name: &str,
    attrs: &[crate::data::Attribute],
    segments: &[YieldSegment],
    persistent: &[(u16, Type)],
    data: &crate::data::Data,
    def_nr: u32,
) -> std::io::Result<()> {
    // ForLoopBody: the entire factory is emitted by Output::emit_for_body_factory.
    if segments
        .iter()
        .any(|s| matches!(s, YieldSegment::ForLoopBody { .. }))
    {
        return Ok(());
    }
    write!(w, "fn {fn_name}(cell: &std::cell::UnsafeCell<Stores>")?;
    for attr in attrs {
        let arg_tp = rust_type(&attr.typedef, &Context::Argument);
        write!(w, ", var_{}: {arg_tp}", sanitize(&attr.name))?;
    }
    writeln!(w, ") -> Box<dyn loft::codegen_runtime::LoftCoroutine> {{")?;
    writeln!(w, "    let _ = cell;")?;
    writeln!(w, "    Box::new({struct_name} {{")?;
    writeln!(w, "        state: 0,")?;
    for attr in attrs {
        let aname = sanitize(&attr.name);
        match &attr.typedef {
            Type::Text(_) => writeln!(w, "        var_{aname}: var_{aname}.to_string(),")?,
            _ => writeln!(w, "        var_{aname},")?,
        }
    }
    // P224: initialise persistent locals to default.
    let var_table = data.def(def_nr).variables();
    for (v, tp) in persistent {
        let n = sanitize(var_table.name(*v));
        let init = persistent_default(tp);
        writeln!(w, "        var_{n}: {init},")?;
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

/// P224: default initialiser for a persistent coroutine local.
/// Mirrors `default_native_value` but inlined here so the helper
/// stays usable from the free function `emit_factory_fn`.
fn persistent_default(tp: &Type) -> String {
    match tp {
        Type::Text(_) => "String::new()".to_string(),
        Type::Boolean => "false".to_string(),
        Type::Character => "0_u32".to_string(),
        Type::Float | Type::Single => "0.0_f64".to_string(),
        // A heap local starts as the null reference and the body's own `OpDatabase` fills
        // it — the same initialiser the per-arm `let` used before these became fields.
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Index(_, _, _)
        | Type::Enum(_, true, _) => "DbRef::NULL".to_string(),
        _ => "0_i64".to_string(),
    }
}

impl Output<'_> {
    /// Generate the factory call for a sub-generator, WITHOUT the `alloc_coroutine`
    /// wrapper.  The `init` expression is always `Value::Call(inner_fn, args)` for a
    /// generator function; we call the Rust factory directly to get a
    /// `Box<dyn LoftCoroutine>` that we can store inline in the outer struct.
    fn gen_inner_factory(&mut self, init: &Value) -> std::io::Result<String> {
        if let Value::Call(d_nr, args) = init {
            let fn_name = self.data.def(*d_nr).name().to_string();
            // P199 — the factory now takes `&UnsafeCell<Stores>`; pass the
            // caller's `cell` binding instead of `stores`.
            let mut buf = format!("{fn_name}(cell");
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

    /// Emit the `next_*` method body for a coroutine state machine.
    /// `yield_tp` selects which trait method to override: `next_i64` for
    /// 8-byte-or-less yields, `next_text` for text yields.
    fn emit_next_i64(
        &mut self,
        w: &mut dyn Write,
        attrs: &[crate::data::Attribute],
        segments: &[YieldSegment],
        tail: &[Value],
        has_yf: bool,
        yield_tp: &Type,
    ) -> std::io::Result<()> {
        let is_text = matches!(yield_tp, Type::Text(_));
        // @P326 — Reference-yielding generators override `next_dbref` (not
        // `next_i64`), so a consumer's `coroutine_next_dbref(gen)` reads the
        // yielded DbRef directly.  Mirrors the text branch's `next_text`
        // selection; both keep the default `next_i64` to drain immediately
        // on a wrong-channel call.
        let is_dbref = matches!(
            yield_tp,
            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
        );
        // @P327 / @P328 native — tuple-of-(integer|float) AND fn-ref yields
        // use the unified `next_into(stores, dest: &mut [i64])` channel.
        // Each yield arm writes the yielded value's slots into `dest` and
        // returns `true`; the exhaust arm returns `false`.  The Simple
        // yield arm in the match below knows which packing layout to emit
        // based on the type-specific flags (`is_tuple_into` /
        // `is_fnref_into`).
        // @PLAN16 phase 02 — a tuple whose every element classifies into a
        // transport slot rides the unified `next_into` channel via the
        // layout-driven flatten-walk (`coroutine_layout`).  This is the SAME
        // decision the consumer's channel-1 selection makes (both call
        // `tuple_kinds` on the same `T`), so the two ends never diverge.
        let tkinds = tuple_kinds(yield_tp);
        let is_tuple_into = tkinds.is_some();
        let is_fnref_into = matches!(yield_tp, Type::Function(_, _, _));
        let uses_next_into = is_tuple_into || is_fnref_into;
        let (sig, exhaust, advance, wrap_open, wrap_close) = if is_text {
            (
                "    fn next_text(&mut self, stores: &mut Stores) -> String {",
                "loft::state::STRING_NULL.to_string()",
                "next_text",
                "(",
                ").to_string()",
            )
        } else if is_dbref {
            (
                "    fn next_dbref(&mut self, stores: &mut Stores) -> DbRef {",
                "DbRef::NULL",
                "next_dbref",
                "(",
                ")",
            )
        } else if uses_next_into {
            // `wrap_open` / `wrap_close` aren't used by the Simple yield
            // arm below for this channel — that arm emits its own
            // per-shape writes (see `is_tuple_into` / `is_fnref_into`
            // branches in the match).
            (
                "    fn next_into(&mut self, stores: &mut Stores, dest: &mut [i64]) -> bool {",
                "false",
                "next_into",
                "",
                "",
            )
        } else if matches!(yield_tp, Type::Float | Type::Single) {
            // #401 — float/single yield: store the IEEE bit-pattern via
            // `to_bits`, NOT a numeric `as i64` (which truncates 1.5 → 1).  The
            // consumer's tagged channel (3 = f64, 4 = f32) mirrors this with
            // `from_bits`.  `f64::to_bits` → u64 and `f32::to_bits` → u32 both
            // zero-extend cleanly through `as i64`.
            (
                "    fn next_i64(&mut self, stores: &mut Stores) -> i64 {",
                "loft::codegen_runtime::COROUTINE_EXHAUSTED",
                "next_i64",
                "(",
                ").to_bits() as i64",
            )
        } else {
            (
                "    fn next_i64(&mut self, stores: &mut Stores) -> i64 {",
                "loft::codegen_runtime::COROUTINE_EXHAUSTED",
                "next_i64",
                "(",
                ") as i64",
            )
        };
        writeln!(w, "{sig}")?;
        // P225: when the generator contains any `ForLoopBody` segment,
        // the factory eager-collects EVERY yield (Simple, YieldFrom, and
        // ForLoopBody alike) into `__values`.  In that case the impl
        // collapses to a single pop-from-buffer arm — emitting per-segment
        // arms would re-execute Simple yields a second time, producing
        // duplicates (the original P225 symptom: first Simple yield
        // appeared twice on native, once from state 0's explicit `return`
        // and once from state 1's `__values[0]` pop).
        let has_for_body = segments
            .iter()
            .any(|s| matches!(s, YieldSegment::ForLoopBody { .. }));
        if has_for_body {
            writeln!(
                w,
                "        let cell: &std::cell::UnsafeCell<Stores> = unsafe \
                 {{ &*(stores as *mut Stores as *const std::cell::UnsafeCell<Stores>) }};"
            )?;
            writeln!(w, "        let _ = &cell;")?;
            writeln!(w, "        let _ = stores;")?;
            writeln!(w, "        if self.__idx < self.__values.len() {{")?;
            if is_text {
                writeln!(w, "            let v = self.__values[self.__idx].clone();")?;
            } else {
                // i64 and DbRef are both `Copy`; the `[idx]` indexing
                // returns by value.
                writeln!(w, "            let v = self.__values[self.__idx];")?;
            }
            writeln!(w, "            self.__idx += 1;")?;
            writeln!(w, "            return v;")?;
            writeln!(w, "        }}")?;
            writeln!(w, "        {exhaust}")?;
            writeln!(w, "    }}")?;
            return Ok(());
        }
        // P199 — coroutine state-machine bodies call user fns that take
        // `&UnsafeCell<Stores>` (the new ABI).  The `LoftCoroutine` trait
        // method still receives `&mut Stores`; derive a `cell` view via
        // the safe `repr(transparent)` cast so generated user-fn calls
        // inside the body have `cell` in scope.
        writeln!(
            w,
            "        let cell: &std::cell::UnsafeCell<Stores> = unsafe \
             {{ &*(stores as *mut Stores as *const std::cell::UnsafeCell<Stores>) }};"
        )?;
        writeln!(w, "        let _ = &cell;")?;
        // P218: pre-declare work-text locals (`var___work_*`) at function
        // scope so they are visible from every state arm.  Without this,
        // the IR's function-entry `Set(__work_N, "")` ops get emitted
        // inside state 0's match arm via `let mut var___work_N: String =
        // …`, scoping the binding to arm 0 only.  A subsequent state arm
        // referencing the same buffer (e.g. a yielded format-string that
        // interpolates a parameter) then fails to compile with E0425
        // ("cannot find value `var___work_N` in this scope").  Adding
        // them to `self.declared` keeps the per-state Set ops emitting as
        // assignments (`var___work_N = "".to_string()`) rather than
        // `let mut`, so each state's pre-statements still re-init
        // correctly without redeclaring.
        // P218: pre-declare any text-typed locals at function scope so
        // they are visible from every state arm (`__work_*` format
        // buffers are the canonical case — declared inside state 0's
        // pre-statements as a `let mut`, then referenced from a later
        // state arm where they're out of scope).  Non-argument text
        // locals are safe to pre-declare with `String::new()` because
        // every consumer site that uses them re-initialises before
        // reading.  Adding them to `self.declared` keeps the per-state
        // Set ops emitting as assignments rather than `let mut`.
        let next = self.data.def(self.def_nr).variables().next_var();
        let mut to_predeclare: Vec<u16> = Vec::new();
        {
            let var_table = self.data.def(self.def_nr).variables();
            for v in 0..next {
                if var_table.is_argument(v) {
                    continue;
                }
                if !matches!(var_table.tp(v), Type::Text(_)) {
                    continue;
                }
                if !var_table.name(v).starts_with("__work") {
                    continue;
                }
                to_predeclare.push(v);
            }
        }
        for v in &to_predeclare {
            let name = sanitize(self.data.def(self.def_nr).variables().name(*v));
            writeln!(w, "        let mut var_{name}: String = String::new();")?;
            self.declared.insert(*v);
        }
        // P226: pre-declare vector-literal backing DbRefs (`__vdb_*`) at
        // function scope.  Same scoping family as P218's `__work_*` fix,
        // but for vector literals: each `[...]` expression in the
        // generator body allocates a `__vdb_N` slot for its backing
        // record, and the per-state `let mut var___vdb_N: DbRef = …`
        // declaration scopes it to a single match arm.  A second state
        // arm whose pre-statements reference the same `__vdb_N`
        // (e.g. `yield [1,2,3].len(); yield [10,20].len();` reuses
        // slots across both arms) then fails with E0425.  Pre-declaring
        // at function scope and adding to `self.declared` keeps the
        // per-state Set ops emitting as plain assignments, so each
        // state still re-initialises before use.
        let mut to_predeclare_vdb: Vec<u16> = Vec::new();
        {
            let var_table = self.data.def(self.def_nr).variables();
            for v in 0..next {
                if var_table.is_argument(v) {
                    continue;
                }
                let name = var_table.name(v);
                // @P326 — also pre-declare `__ref_*` work-vars used by
                // inline struct construction (`yield SomeStruct { … }`).
                // Same scoping family as `__vdb_*` and `__work_*` — declared
                // inside one match arm's pre-statements, then referenced
                // from another arm where they're out of scope.  Each
                // Reference-typed yield arm allocates a `__ref_N` slot.
                if !name.starts_with("__vdb") && !name.starts_with("__ref") {
                    continue;
                }
                to_predeclare_vdb.push(v);
            }
        }
        for v in &to_predeclare_vdb {
            let name = sanitize(self.data.def(self.def_nr).variables().name(*v));
            writeln!(
                w,
                "        let mut var_{name}: DbRef = stores.null_named(\"var_{name}\");"
            )?;
            self.declared.insert(*v);
        }
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
                    if is_tuple_into {
                        // @PLAN16 phase 02 — layout-driven flatten-walk.
                        // `val` is a `Value::Tuple([…])`; encode each element
                        // per its `YieldSlot` kind at the running transport
                        // slot.  Slot offsets accumulate by kind width (a `Ref`
                        // takes two), so a tuple of mixed scalar/ref kinds packs
                        // correctly — and the consumer's `yield_slot_read`
                        // mirror unpacks the identical layout.
                        let kinds = tkinds.as_ref().expect("is_tuple_into ⇒ tuple_kinds");
                        if let crate::data::Value::Tuple(elems) = val {
                            let mut slot = 0usize;
                            for (elem, &kind) in elems.iter().zip(kinds.iter()) {
                                let code = self.generate_expr_buf(elem)?;
                                yield_slot_write(w, kind, slot, &code)?;
                                slot += kind.width();
                            }
                        }
                        writeln!(w, "                return true;")?;
                    } else if is_fnref_into {
                        // @P328 native — pack the fn-ref `(u32, DbRef)`
                        // into 2 i64 slots and return `true`.  Layout
                        // mirrors the OpCoroutineNextEmitter rebuild
                        // (channel tag 2):
                        //   dest[0] = (d_nr as i64) | ((store_nr as i64) << 32)
                        //   dest[1] = (rec as i64) | ((pos as i64) << 32)
                        //
                        // Non-capturing fn-ref yields IR-emit as plain
                        // `Value::Int(d_nr)` / `Value::Long(d_nr)` (the
                        // parser drops the closure DbRef when there's
                        // nothing to capture); capturing yields emit as
                        // a Block ending in `Value::FnRef(d, closure_var, _)`
                        // which `generate_expr_buf` materialises as
                        // `(d_u32, var_closure)`.  Detect the
                        // bare-integer non-capturing shape and wrap
                        // with the null-DbRef sentinel so the consumer's
                        // rebuild gets a valid `(u32, DbRef)` either way.
                        let val_un = val.unspan();
                        let is_bare_dnr = matches!(
                            val_un,
                            crate::data::Value::Int(_) | crate::data::Value::Long(_)
                        );
                        let yield_code = self.generate_expr_buf(val)?;
                        if is_bare_dnr {
                            writeln!(
                                w,
                                "                let _f: (u32, DbRef) = (({yield_code}) as u32, loft::keys::DbRef::NULL);"
                            )?;
                        } else {
                            writeln!(w, "                let _f: (u32, DbRef) = ({yield_code});")?;
                        }
                        writeln!(
                            w,
                            "                dest[0] = (_f.0 as i64) | (((_f.1.store_nr as u64) as i64) << 32);"
                        )?;
                        writeln!(
                            w,
                            "                dest[1] = (_f.1.rec as i64) | ((_f.1.pos as i64) << 32);"
                        )?;
                        writeln!(w, "                return true;")?;
                    } else {
                        let yield_code = self.generate_expr_buf(val)?;
                        writeln!(
                            w,
                            "                return {wrap_open}{yield_code}{wrap_close};"
                        )?;
                    }
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
                        "                let val = self.sub_{state_idx}.as_mut().unwrap().{advance}(stores);"
                    )?;
                    writeln!(w, "                if val == {exhaust} {{")?;
                    writeln!(w, "                    self.sub_{state_idx} = None;")?;
                    writeln!(w, "                    self.state = {};", state_idx + 1)?;
                    writeln!(w, "                    continue;")?;
                    writeln!(w, "                }}")?;
                    writeln!(w, "                return val;")?;
                }
                YieldSegment::ForLoopBody { .. } => {
                    // Values were collected eagerly in the factory. Just pop from the buffer.
                    writeln!(w, "                if self.__idx < self.__values.len() {{")?;
                    if is_text {
                        writeln!(
                            w,
                            "                    let v = self.__values[self.__idx].clone();"
                        )?;
                    } else {
                        writeln!(w, "                    let v = self.__values[self.__idx];")?;
                    }
                    writeln!(w, "                    self.__idx += 1;")?;
                    writeln!(w, "                    return v;")?;
                    writeln!(w, "                }}")?;
                    writeln!(w, "                return {exhaust};")?;
                }
            }
            writeln!(w, "            }}")?;
        }
        // The TAIL state — everything after the last yield, run ONCE on the `next()` that
        // exhausts the generator, exactly as the interpreter does.  Advancing `state` past it
        // is what makes it once: a later call falls through to the catch-all below, so the
        // scope-exit `OpFreeRef`s here cannot double-free.
        // A tail statement may only be emitted if every LOCAL it names is reachable from the
        // state machine — a field, or a parameter.  A compiler-internal `__*` local
        // (a closure store, a vector-literal backing record) is deliberately NOT persisted, so
        // it lives in whichever arm declared it and the tail cannot see it.  Skipping just
        // those statements is provably no worse than before, when the whole tail was dropped;
        // emitting them regardless is not, and reintroduced the very E0425 this fixes.
        let reachable = |out: &Self, op: &Value| {
            let vars = out.data.def(out.def_nr).variables();
            let mut ok = true;
            op.walk(&mut |n| {
                if let Value::Var(v) = n
                    && !out.coroutine_persistent_vars.contains(v)
                    && !vars.is_argument(*v)
                {
                    ok = false;
                }
            });
            ok
        };
        let tail: Vec<&Value> = tail.iter().filter(|op| reachable(self, op)).collect();
        if !tail.is_empty() {
            let tail_state = segments.len();
            writeln!(w, "            {tail_state} => {{")?;
            for op in tail {
                write!(w, "                ")?;
                self.output_code_inner(w, op)?;
                writeln!(w, ";")?;
            }
            writeln!(w, "                self.state = {};", tail_state + 1)?;
            if has_yf {
                writeln!(w, "                return {exhaust};")?;
            } else {
                writeln!(w, "                {exhaust}")?;
            }
            writeln!(w, "            }}")?;
        }
        // Exhausted arm.
        if has_yf {
            writeln!(w, "            _ => return {exhaust},")?;
        } else {
            writeln!(w, "            _ => {exhaust},")?;
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
        let fn_name = self.fn_ident(def);
        let struct_name = gen_struct_name(&fn_name);

        // Emit a minimal stub for bodyless functions and return early.
        let Value::Block(body_block) = &def.code().clone() else {
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
                "fn {fn_name}(_cell: &std::cell::UnsafeCell<Stores>) -> Box<dyn loft::codegen_runtime::LoftCoroutine> \
                 {{ Box::new({struct_name} {{}}) }}\n"
            )?;
            return Ok(());
        };

        let (segments, tail) = collect_segments(&body_block.operators);
        let has_yf = segments
            .iter()
            .any(|s| matches!(s, YieldSegment::YieldFrom { .. }));
        let attrs: Vec<_> = def.attributes().to_vec();
        let yield_tp = match def.returned() {
            Type::Iterator(inner, _) => (**inner).clone(),
            other => other.clone(),
        };

        // P224: compute persistent locals once, share across struct + impl + factory.
        let persistent = coroutine_persistent_locals(self.data, def_nr);

        // ── 1. Struct definition ─────────────────────────────────────────────
        emit_struct_def(
            w,
            &struct_name,
            &attrs,
            &segments,
            &yield_tp,
            &persistent,
            self.data,
            def_nr,
        )?;

        // ── 2. impl LoftCoroutine ────────────────────────────────────────────
        // Scope the persistent-vars set AND any "declared" insertions to the
        // impl block — they only govern emission inside `next_i64` /
        // `next_text`.  The factory function (emitted next) re-uses the same
        // `Output` instance and would otherwise inherit stale declared marks
        // for `__yf_*` / `__vdb_*` etc., causing E0425 in the eager-collect
        // factory path.
        let prev_persistent = std::mem::take(&mut self.coroutine_persistent_vars);
        let prev_allocated = std::mem::take(&mut self.coroutine_allocated_vars);
        self.coroutine_persistent_vars = persistent.iter().map(|(v, _)| *v).collect();
        let mut newly_declared = Vec::with_capacity(persistent.len());
        for (v, _) in &persistent {
            if self.declared.insert(*v) {
                newly_declared.push(*v);
            }
        }
        writeln!(
            w,
            "impl loft::codegen_runtime::LoftCoroutine for {struct_name} {{"
        )?;
        self.emit_next_i64(w, &attrs, &segments, &tail, has_yf, &yield_tp)?;
        writeln!(w, "}}\n")?;
        self.coroutine_persistent_vars = prev_persistent;
        self.coroutine_allocated_vars = prev_allocated;
        for v in newly_declared {
            self.declared.remove(&v);
        }

        // ── 3. Factory function ──────────────────────────────────────────────
        let def = self.data.def(def_nr);
        let attrs: Vec<_> = def.attributes().to_vec();
        let has_for_body = segments
            .iter()
            .any(|s| matches!(s, YieldSegment::ForLoopBody { .. }));
        emit_factory_fn(
            w,
            &fn_name,
            &struct_name,
            &attrs,
            &segments,
            &persistent,
            self.data,
            def_nr,
        )?;
        if has_for_body {
            self.emit_for_body_factory(
                w,
                &fn_name,
                &struct_name,
                &attrs,
                &segments,
                &yield_tp,
                &persistent,
                def_nr,
            )?;
        }
        Ok(())
    }

    /// Emit the factory function for a generator that contains for-loop bodies
    /// with yields.  Runs the body eagerly, pushing all yielded values to a Vec.
    #[allow(clippy::too_many_arguments)]
    fn emit_for_body_factory(
        &mut self,
        w: &mut dyn Write,
        fn_name: &str,
        struct_name: &str,
        attrs: &[crate::data::Attribute],
        segments: &[YieldSegment],
        yield_tp: &Type,
        persistent: &[(u16, Type)],
        def_nr: u32,
    ) -> std::io::Result<()> {
        let is_text = matches!(yield_tp, Type::Text(_));
        // @P326 — for-body factory must use the DbRef channel for
        // Reference-yielding generators (the eager-collect buffer is
        // `Vec<DbRef>`, the sub-generator advances via `next_dbref`).
        let is_dbref = matches!(
            yield_tp,
            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
        );
        let (vec_ty, push_wrap_open, push_wrap_close, sub_advance, sub_exhaust) = if is_text {
            (
                "String",
                "(",
                ").to_string()",
                "next_text",
                "loft::state::STRING_NULL",
            )
        } else if is_dbref {
            ("DbRef", "(", ")", "next_dbref", "DbRef::NULL")
        } else if matches!(yield_tp, Type::Float | Type::Single) {
            // #401 — float/single eager-collect buffer stores the IEEE
            // bit-pattern (mirror of the consumer's from_bits channel).
            (
                "i64",
                "(",
                ").to_bits() as i64",
                "next_i64",
                "loft::codegen_runtime::COROUTINE_EXHAUSTED",
            )
        } else {
            (
                "i64",
                "(",
                ") as i64",
                "next_i64",
                "loft::codegen_runtime::COROUTINE_EXHAUSTED",
            )
        };
        write!(w, "fn {fn_name}(cell: &std::cell::UnsafeCell<Stores>")?;
        for attr in attrs {
            let arg_tp = rust_type(&attr.typedef, &Context::Argument);
            write!(w, ", var_{}: {arg_tp}", sanitize(&attr.name))?;
        }
        writeln!(w, ") -> Box<dyn loft::codegen_runtime::LoftCoroutine> {{")?;
        writeln!(
            w,
            "    let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        writeln!(w, "    let _ = &stores;")?;
        // Declare local copies of params for use in the body.
        for attr in attrs {
            let aname = sanitize(&attr.name);
            match &attr.typedef {
                Type::Text(_) => writeln!(w, "    let var_{aname}: &str = var_{aname};")?,
                _ => writeln!(w, "    let var_{aname} = var_{aname};")?,
            }
        }
        // P218: same pre-declaration as `emit_next_i64` — `__work_*`
        // text-format buffers used in the eager-collect body need to
        // be declared in the factory's scope (not first-use-inside-
        // a-loop-body, which scoped them to that block and left them
        // invisible at later assignment sites).  Mark them declared so
        // the IR's per-body Set ops emit as assignments.
        let next = self.data.def(self.def_nr).variables().next_var();
        let mut to_predeclare: Vec<u16> = Vec::new();
        {
            let var_table = self.data.def(self.def_nr).variables();
            for v in 0..next {
                if var_table.is_argument(v) {
                    continue;
                }
                if !matches!(var_table.tp(v), Type::Text(_)) {
                    continue;
                }
                if !var_table.name(v).starts_with("__work") {
                    continue;
                }
                to_predeclare.push(v);
            }
        }
        for v in &to_predeclare {
            let name = sanitize(self.data.def(self.def_nr).variables().name(*v));
            writeln!(w, "    let mut var_{name}: String = String::new();")?;
            self.declared.insert(*v);
        }
        writeln!(w, "    let mut __values: Vec<{vec_ty}> = Vec::new();")?;
        // Run each for-loop body with yield_collect enabled.
        self.yield_collect = true;
        self.yield_collect_text = is_text;
        self.yield_collect_dbref = is_dbref;
        for seg in segments {
            match seg {
                YieldSegment::ForLoopBody { pre, body } => {
                    for stmt in pre {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "    {stmt_code};")?;
                    }
                    // P219: strip trailing Return ops from the body before
                    // emitting.  The factory drives the body purely for its
                    // side effects (yields populate `__values`); the
                    // factory's actual return is `Box::new(struct)` emitted
                    // below.  But the function body's `[Loop, Return(Null)]`
                    // pattern triggers `patch_hoisted_returns` in
                    // `output_block` to coalesce into `[Return(Loop)]`,
                    // which `Value::Return` then emits as
                    // `return 'l4: loop {...}` — invalid because the loop is
                    // unit-typed and the factory expects
                    // `Box<dyn LoftCoroutine>`.  Stripping trailing Return
                    // ops keeps the body as plain statements.
                    let body_for_emit = match body.unspan() {
                        Value::Block(bl) => {
                            let mut bl2 = bl.clone();
                            while bl2
                                .operators
                                .last()
                                .is_some_and(|op| matches!(op.unspan(), Value::Return(_)))
                            {
                                bl2.operators.pop();
                            }
                            Value::Block(bl2)
                        }
                        _ => body.clone(),
                    };
                    let body_code = self.generate_expr_buf(&body_for_emit)?;
                    writeln!(w, "    {body_code};")?;
                }
                YieldSegment::Simple { pre, val } => {
                    for stmt in pre {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "    {stmt_code};")?;
                    }
                    let val_code = self.generate_expr_buf(val)?;
                    writeln!(
                        w,
                        "    __values.push({push_wrap_open}{val_code}{push_wrap_close});"
                    )?;
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
                    writeln!(w, "            let v = __sub.{sub_advance}(stores);")?;
                    writeln!(w, "            if v == {sub_exhaust} {{ break; }}")?;
                    writeln!(w, "            __values.push(v);")?;
                    writeln!(w, "        }}")?;
                    writeln!(w, "    }}")?;
                }
            }
        }
        self.yield_collect = false;
        self.yield_collect_text = false;
        self.yield_collect_dbref = false;
        writeln!(w, "    Box::new({struct_name} {{")?;
        writeln!(w, "        state: 0,")?;
        for attr in attrs {
            let aname = sanitize(&attr.name);
            match &attr.typedef {
                Type::Text(_) => writeln!(w, "        var_{aname}: var_{aname}.to_string(),")?,
                _ => writeln!(w, "        var_{aname},")?,
            }
        }
        // P224: initialise persistent locals to default — same as the
        // non-for-body factory path.  Without this the eager-collect
        // factory builds a `StructName { … }` with the user-attribute
        // fields but omits the persistent-locals fields that
        // `emit_struct_def` declared, producing a Rust E0063 ("missing
        // fields in initializer").
        let var_table = self.data.def(def_nr).variables();
        for (v, tp) in persistent {
            let n = sanitize(var_table.name(*v));
            let init = persistent_default(tp);
            writeln!(w, "        var_{n}: {init},")?;
        }
        // ForLoopBody: value buffer + index.
        writeln!(w, "        __values,")?;
        writeln!(w, "        __idx: 0,")?;
        writeln!(w, "    }})")?;
        writeln!(w, "}}\n")
    }
}
