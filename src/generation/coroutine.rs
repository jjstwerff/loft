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
    /// A loop whose body ends in a single unconditional `yield`, lowered LAZILY (CL-9).
    ///
    /// One advance runs one iteration: the loop's cursor and loop-carried locals already
    /// persist in the coroutine struct, so re-entering the state resumes where the last
    /// `yield` left off — which is what `ForLoopBody`'s eager buffer cannot do.  Takes TWO
    /// states: the first runs `setup` once, the second is the iteration.
    ///
    /// - `pre`: statements before the loop's block, at the generator's top level
    /// - `whole`: the unsplit block, kept so a downgrade to `ForLoopBody` is exact
    /// - `setup`: the block's statements BEFORE the loop (the cursor initialisation)
    /// - `body`: the loop's own operators — the header (advance + bound test) and the body,
    ///   with the trailing `yield` still in place; the emitter rewrites that one node
    /// - `post`: the block's statements AFTER the loop (scope-exit frees, implicit return)
    ForLoopLazy {
        pre: Vec<Value>,
        whole: Value,
        setup: Vec<Value>,
        body: Vec<Value>,
        post: Vec<Value>,
    },
}

/// Does this statement END in a `yield`, looking through trailing blocks?
///
/// The test is for an UNCONDITIONAL trailing yield: an `if`-wrapped one resumes at a point
/// that depends on which branch ran, which is axis A3 and stays eager.
fn tail_is_yield(v: &Value) -> bool {
    match v.unspan() {
        Value::Yield(_) => true,
        Value::Block(bl) => bl.operators.last().is_some_and(tail_is_yield),
        _ => false,
    }
}

/// Move a dead temp's scope-exit free ABOVE the `yield` it trails, so the loop still reads
/// as one the lazy lowering admits.
///
/// A loop body that ends `…; yield v; OpFreeText(_tmp)` is what a TEXT FIELD WRITE compiles
/// to — `t.s += "x"` lifts the field into a temp, copies it back with `OpSetText`, and the
/// temp's free is placed at block exit, which lands after the suspend.  [`detect_lazy_for`]
/// then sees a statement after the yield and demotes the WHOLE generator to the eager
/// buffer, whose side effects all happen before the consumer sees anything — where
/// [`coroutines.md`](../../doc/claude/formal/coroutines.md) `(G-Next)` says they interleave.
/// Measured: the same generator with an INTEGER field is lazy on both backends, because
/// `OpSetInt` leaves nothing trailing.  The author wrote no statement after the yield; the
/// compiler did, and the author has no way to remove it.
///
/// Hoisting is sound exactly when the yield does not READ what is being freed: the free
/// then happens one statement earlier along a straight line, still once per iteration.  It
/// is also strictly better for an ABANDONED generator, which used to strand the last
/// iteration's temp.  A yield that reads the temp (`yield t.s`) keeps the eager path.
///
/// Returns the rewritten statement, or `None` when there is nothing to hoist — so a caller
/// can tell "already fine" and "cannot be made fine" apart from "fixed".
fn hoist_trailing_frees(v: &Value, data: &crate::data::Data) -> Option<Value> {
    let Value::Block(bl) = v.unspan() else {
        return None;
    };
    let last = bl.operators.last()?;
    // Descend the same way `tail_is_yield` does, so the two agree about where the tail is.
    if matches!(last.unspan(), Value::Block(_)) {
        let fixed = hoist_trailing_frees(last, data)?;
        let mut bl2 = bl.clone();
        *bl2.operators.last_mut()? = fixed;
        return Some(Value::Block(bl2));
    }
    // The tail must be a run of frees sitting directly after the yield — nothing else.
    let at = bl
        .operators
        .iter()
        .rposition(|op| matches!(op.unspan(), Value::Yield(_)))?;
    if at + 1 == bl.operators.len() {
        return None; // already ends in the yield
    }
    let Value::Yield(yielded) = bl.operators[at].unspan() else {
        return None;
    };
    let mut freed = Vec::new();
    for op in &bl.operators[at + 1..] {
        let var = Output::free_op_var(op, data)?;
        // The one unsound case: the suspended value still names the store being released.
        if yielded.reads_var(var) {
            return None;
        }
        freed.push(op.clone());
    }
    let mut ops = bl.operators[..at].to_vec();
    ops.extend(freed);
    ops.push(bl.operators[at].clone());
    let mut bl2 = bl.clone();
    bl2.operators = ops;
    Some(Value::Block(bl2))
}

/// True when a coroutine slot of this type is a Rust `String`.
///
/// @PLN25 — `Optional(τ)` shares τ's storage exactly, and loft's absent text IS a
/// `String` (the `STRING_NULL` sentinel), so a `text?` slot is a `String` slot.  Every
/// site in this file that decides "String or scalar?" reads THIS one predicate, because
/// they have to agree: the struct field, the factory that fills it, the shadow-bind that
/// reads it back, and the yield channel are four views of a single slot.
///
/// loft#1035 — while each site matched `Type::Text` unpeeled, a `text?` PARAMETER was a
/// `String` field (via `rust_type`) that the factory filled with a bare `&str` and the
/// body moved out of `&mut self`: rustc E0308 + E0507, so a generator taking a `text?`
/// did not compile at all on `--native` while `--interpret` ran it.
fn is_text_slot(tp: &Type) -> bool {
    matches!(tp.base(), Type::Text(_))
}

/// The placeholder a lazily-lowered loop's iteration state gives `__y` before running the
/// body.  The `yield` always overwrites it before it is read, so it only has to type-check
/// against the channel this generator answers on.
fn lazy_yield_init(yield_tp: &Type) -> &'static str {
    if is_text_slot(yield_tp) {
        return "String::new()";
    }
    match yield_tp {
        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => "DbRef::NULL",
        _ => "0i64",
    }
}

/// Recognise the loop shape a single advance can run one iteration of (CL-9 slice 1).
///
/// Returns `(setup, loop_ops, post)` — the statements before the loop, the loop's own
/// operators, and the statements after it.  Everything this REJECTS keeps the eager buffer,
/// so no generator regresses while the remaining axes are still to come:
///
/// * more than one loop, or a yield outside the loop — the state graph is not one cursor;
/// * more than one yield, or one that is not the loop body's last statement (axes A2/A3) —
///   re-entry would have to land at the yield that suspended, which one state cannot encode;
/// * a nested loop (A4) — its `break` would be caught by the single-iteration wrapper this
///   lowering runs the body in, ending the OUTER loop instead of the inner one;
/// * a `continue` — it would leave the iteration without yielding, which the wrapper reads
///   as "the loop ended";
/// * a `return` — it would return from `next()` rather than from the generator.
fn detect_lazy_for(
    val: &Value,
    data: &crate::data::Data,
) -> Option<(Vec<Value>, Vec<Value>, Vec<Value>)> {
    let Value::Block(bl) = val.unspan() else {
        return None;
    };
    let mut loop_at = None;
    for (i, op) in bl.operators.iter().enumerate() {
        if matches!(op.unspan(), Value::Loop(_)) {
            if loop_at.is_some() {
                return None;
            }
            loop_at = Some(i);
        } else if contains_yield(op) {
            return None;
        }
    }
    let at = loop_at?;
    let Value::Loop(lp) = bl.operators[at].unspan() else {
        return None;
    };
    let mut yields = 0usize;
    let mut disqualified = false;
    for op in &lp.operators {
        op.walk(&mut |n| match n {
            Value::Yield(_) => yields += 1,
            Value::Loop(_) | Value::Continue(_) | Value::Return(_) => disqualified = true,
            _ => {}
        });
    }
    if yields != 1 || disqualified {
        return None;
    }
    // A trailing free is the compiler's statement, not the author's — hoist it above the
    // suspend rather than reading it as "a statement after the yield" and giving up.
    let mut body_ops = lp.operators.clone();
    if !body_ops.last().is_some_and(tail_is_yield)
        && let Some(tail) = body_ops.last()
        && let Some(fixed) = hoist_trailing_frees(tail, data)
    {
        *body_ops.last_mut()? = fixed;
    }
    if !body_ops.last().is_some_and(tail_is_yield) {
        return None;
    }
    // The block's trailing `return` is the generator's implicit end, which the state machine
    // expresses with its exhausted sentinel — emitting a Rust `return` of the loft value
    // answers the wrong type from `next()`.  Unwrap it the way `collect_segments` unwraps the
    // tail, keeping any real expression it wrapped so its side effect still happens.
    let mut post = bl.operators[at + 1..].to_vec();
    while let Some(last) = post.last().map(|v| v.unspan().clone()) {
        match last {
            Value::Null => {
                post.pop();
            }
            Value::Return(inner) if matches!(inner.unspan(), Value::Null) => {
                post.pop();
            }
            Value::Return(inner) => {
                post.pop();
                post.push(*inner);
                break;
            }
            _ => break,
        }
    }
    Some((bl.operators[..at].to_vec(), body_ops, post))
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
    // Anything after the loop is the scope exit the handle now carries — `OpFreeRef(sub_var)`,
    // added when a generator handle became a freed heap value (loft#835).  The native lowering
    // owns the sub-generator through `self.sub_N` and never gives it a table entry, so that
    // free has nothing to do here and is dropped; `self.sub_N`'s own cleanup releases it.
    // Matching the op count exactly instead is what silently pushed every `yield from` onto
    // the eager path the moment that free appeared.
    if bl.operators.len() < 2 || bl.operators[2..].iter().any(contains_yield) {
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
fn collect_segments(ops: &[Value], data: &crate::data::Data) -> (Vec<YieldSegment>, Vec<Value>) {
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
            // CL-9 (loft#836): a loop whose body ends in one unconditional `yield` is
            // lowered lazily — one iteration per advance — so its side effects interleave
            // with the consumer's exactly as they do on the interpreter.  Everything else
            // keeps the eager buffer.
            if let Some((setup, body, post)) = detect_lazy_for(inner_op, data) {
                segments.push(YieldSegment::ForLoopLazy {
                    pre: std::mem::take(&mut pre),
                    whole: inner_op.clone(),
                    setup,
                    body,
                    post,
                });
            } else {
                segments.push(YieldSegment::ForLoopBody {
                    pre: std::mem::take(&mut pre),
                    body: inner_op.clone(),
                });
            }
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
    // Laziness is decided for the WHOLE generator, not per loop.  An eager segment makes the
    // factory collect EVERY yield up front and `next()` collapse to a pop-from-buffer arm
    // (P225), which a lazy segment's own states would then run a second time.  So one loop
    // that has to stay eager pulls the rest back with it.
    if segments
        .iter()
        .any(|s| matches!(s, YieldSegment::ForLoopBody { .. }))
    {
        for seg in &mut segments {
            if let YieldSegment::ForLoopLazy { pre, whole, .. } = seg {
                *seg = YieldSegment::ForLoopBody {
                    pre: std::mem::take(pre),
                    body: whole.clone(),
                };
            }
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
        // `__vdb_*` is the exception among the compiler-internal locals: it OWNS the backing
        // store of a vector literal, so it has to survive resumes or the store allocated in
        // one state is orphaned when the next `next_*` call re-declares the local — and the
        // tail's `OpFreeRef(__vdb_N)` then frees a fresh `DbRef::NULL` instead.  `__work_*`
        // (P218, a `String` pre-declared at function scope) and `__yf_*` (built inline by the
        // eager-collect factory) own no store and keep their own emission paths.
        if name.starts_with("__") && !name.starts_with("__vdb") {
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
                | Type::Trie(_, _, _)
                | Type::Index(_, _, _)
        );
        if !suitable {
            continue;
        }
        out.push((v, tp.clone()));
    }
    out
}

/// The generator struct's field name for every persistent local, made unique.
///
/// A generator moves its locals onto ONE struct, so unlike a function body — where a second
/// `let mut var_i` merely shadows the first, and loft refuses the one shape where that would
/// lose a value — two variables spelling the same name declare the field twice and the
/// generated Rust does not compile.  Two `for i in …` loops in one generator are exactly
/// that: each loop declares its own `i` and its own `i#index`, so four distinct variables
/// spell two names (loft#928).
///
/// The first variable to claim a name keeps the bare `var_<name>`, so a generator with no
/// duplicate emits what it emitted before; a later claimant takes the first free
/// `var_<name>__2`, `var_<name>__3`, …  The argument fields seed the used set because they
/// share the struct with the locals.  Testing each candidate against the names already TAKEN,
/// rather than appending the variable number, is what keeps a suffix from colliding with a
/// local the program really did name `i__2`.
///
/// This is the one derivation of a persistent field's name.  The struct definition, both
/// factories, `drop_stores`, and every `self.var_x` read and write in `emit.rs` /
/// `dispatch.rs` / `ref_ops.rs` all read the map it returns, and the map's KEYS are also what
/// says a variable is persistent at all — so "is this a field" and "what is the field called"
/// are one lookup and cannot drift apart.
fn persistent_field_names(
    attrs: &[crate::data::Attribute],
    persistent: &[(u16, Type)],
    vars: &crate::variables::Function,
) -> std::collections::HashMap<u16, String> {
    let mut used: std::collections::HashSet<String> =
        attrs.iter().map(|a| sanitize(&a.name)).collect();
    let mut out = std::collections::HashMap::with_capacity(persistent.len());
    for (v, _) in persistent {
        let base = sanitize(vars.name(*v));
        let mut name = base.clone();
        let mut n = 2;
        while !used.insert(name.clone()) {
            name = format!("{base}__{n}");
            n += 1;
        }
        out.insert(*v, name);
    }
    out
}

/// Emit `drop_stores` — release the heap locals a generator still owns when its handle is
/// freed without the generator having exhausted.
///
/// A generator frees its own locals from the tail of its body, and a consumer that stops
/// early never reaches that tail (loft#835).  Every scope-exit free the generator DOES run
/// nulls its own field (`OpFreeRefEmitter` writes the `store_nr = u16::MAX` reset against
/// `self.var_x` for a persistent local), so freeing exactly the fields that are still set
/// frees each local once and only once.
///
/// Nothing is emitted for a generator with no owned heap local; the trait's no-op default
/// stands.  `Type::Iterator` is included: a nested generator handle routes back through
/// `OpFreeRef`, which frees that coroutine in turn.
fn emit_drop_stores(
    w: &mut dyn Write,
    persistent: &[(u16, Type)],
    fields: &std::collections::HashMap<u16, String>,
    data: &crate::data::Data,
    def_nr: u32,
) -> std::io::Result<()> {
    let vars = data.def(def_nr).variables();
    let owned: Vec<String> = persistent
        .iter()
        .filter(|(v, tp)| {
            vars.owns_store(*v)
                && matches!(
                    tp,
                    Type::Reference(_, _)
                        | Type::Vector(_, _)
                        | Type::Sorted(_, _, _)
                        | Type::Hash(_, _, _)
                        | Type::Radix(_, _, _)
                        | Type::Trie(_, _, _)
                        | Type::Index(_, _, _)
                        | Type::Enum(_, true, _)
                        | Type::Iterator(_, _)
                )
        })
        .map(|(v, _)| fields[v].clone())
        .collect();
    if owned.is_empty() {
        return Ok(());
    }
    writeln!(w, "    fn drop_stores(&mut self, stores: &mut Stores) {{")?;
    for name in &owned {
        writeln!(
            w,
            "        if self.var_{name}.store_nr != u16::MAX {{ \
             loft::codegen_runtime::coroutine_drop_local(stores, self.var_{name}, \"var_{name}\"); \
             self.var_{name}.store_nr = u16::MAX; }}"
        )?;
    }
    writeln!(w, "    }}")
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
    fields: &std::collections::HashMap<u16, String>,
) -> std::io::Result<()> {
    writeln!(w, "struct {struct_name} {{")?;
    writeln!(w, "    state: u32,")?;
    for attr in attrs {
        let field_tp = if is_text_slot(&attr.typedef) {
            "String".to_string()
        } else {
            rust_type(&attr.typedef, &Context::Variable)
        };
        writeln!(w, "    var_{}: {field_tp},", sanitize(&attr.name))?;
    }
    // P224: persistent function-locals as struct fields.
    for (v, tp) in persistent {
        let n = &fields[v];
        let field_tp = if is_text_slot(tp) {
            "String".to_string()
        } else {
            rust_type(tp, &Context::Variable)
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
        let elem_ty = if is_text_slot(yield_tp) {
            "String"
        } else {
            match yield_tp {
                // @P326 — Reference / Vector / struct-enum yields are DbRef-shaped.
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => "DbRef",
                _ => "i64",
            }
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
    fields: &std::collections::HashMap<u16, String>,
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
        if is_text_slot(&attr.typedef) {
            writeln!(w, "        var_{aname}: var_{aname}.to_string(),")?;
        } else {
            writeln!(w, "        var_{aname},")?;
        }
    }
    // P224: initialise persistent locals to default.
    for (v, tp) in persistent {
        let n = &fields[v];
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
    if is_text_slot(tp) {
        return "String::new()".to_string();
    }
    match tp {
        // A heap local starts as the null reference and the body's own `OpDatabase` fills
        // it — the same initialiser the per-arm `let` used before these became fields.
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _)
        | Type::Index(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Iterator(_, _) => "DbRef::NULL".to_string(),
        // Every remaining field type lowers to a Rust NUMBER, so the zero of whatever
        // `rust_type` decided is a value of exactly that type.  Asking it, rather than
        // listing the types a second time here, is the point: the second list had drifted
        // on three arms at once — a `character` field was declared `i32` and initialised
        // `0_u32`, a `single` field `f32` and initialised `0.0_f64`, a `boolean` field `u8`
        // and initialised `false` — so a generator holding a local of any of those types
        // did not compile under `--native` at all, while `--interpret` ran it.
        other => format!("0 as {}", rust_type(other, &Context::Variable)),
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
        let is_text = is_text_slot(yield_tp);
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
                if !is_text_slot(var_table.tp(v)) {
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
                // A `__vdb_*` that became a persistent FIELD must not also get a local of the
                // same name — the local would shadow the field and the frees would miss.
                if self.coroutine_persistent_fields.contains_key(&v) {
                    continue;
                }
                to_predeclare_vdb.push(v);
            }
        }
        // `DbRef::NULL`, NOT `null_named` — the same @P317 rule the ordinary local path
        // already follows (an ordinary function emits `let mut var___ref_1: DbRef =
        // DbRef::NULL`).  `null_named` allocates a real store slot, and this declaration runs
        // on EVERY `next_*` call, so it leaked one store per hidden work-ref per resume — six
        // records for a two-`__ref` generator advanced three times.  Each state re-initialises
        // these before use, so there is nothing for the placeholder to carry.
        //
        // loft#1032 — ask the variable's TYPE rather than assuming `DbRef`.  The `__ref_*`
        // family is named for the Reference-typed yield arms that motivated it, but a
        // generic's hidden return buffer joins the same family and takes the type the
        // monomorph bound: `-> iterator<T>` at `T = integer` gives an INTEGER `__ref_1`,
        // which the hardcoded pair declared `DbRef` and then assigned `0` (E0308, so a
        // generic generator over any scalar did not compile).  `persistent_default` is
        // already the one home for "the zero of whatever `rust_type` decided" — its own
        // doc records a hand-maintained second list drifting on three arms — so this asks
        // it instead of becoming a fourth.  A `__vdb_*` handle is Reference-typed and
        // still gets `DbRef` / `DbRef::NULL` from it, unchanged.
        for v in &to_predeclare_vdb {
            let vars = self.data.def(self.def_nr).variables();
            let name = sanitize(vars.name(*v));
            let tp = vars.tp(*v).clone();
            let tp_str = rust_type(&tp, &Context::Variable);
            let init = persistent_default(&tp);
            writeln!(w, "        let mut var_{name}: {tp_str} = {init};")?;
            self.declared.insert(*v);
        }
        // These, plus `__work_*` above, are in scope for the WHOLE `next_*` body, so the tail
        // state may name them — unlike a local declared inside one match arm.
        let fn_scope: std::collections::HashSet<u16> = to_predeclare
            .iter()
            .chain(to_predeclare_vdb.iter())
            .copied()
            .collect();
        // N8b.3: wrap in `loop {}` so yield-from states can `continue` to the
        // next state immediately after sub-generator exhaustion.
        if has_yf {
            writeln!(w, "        loop {{")?;
        }
        writeln!(w, "        match self.state {{")?;
        // A lazily-lowered loop takes TWO states — its setup, then its iteration — so state
        // numbers are handed out per segment rather than read off the index.
        let mut state_of: Vec<usize> = Vec::with_capacity(segments.len());
        let mut next_state = 0usize;
        for segment in segments {
            state_of.push(next_state);
            next_state += if matches!(segment, YieldSegment::ForLoopLazy { .. }) {
                2
            } else {
                1
            };
        }
        let after_segments = next_state;
        for (seg_idx, segment) in segments.iter().enumerate() {
            let state_idx = state_of[seg_idx];
            writeln!(w, "            {state_idx} => {{")?;
            // Shadow-bind parameters.
            for attr in attrs {
                let aname = sanitize(&attr.name);
                if is_text_slot(&attr.typedef) {
                    writeln!(
                        w,
                        "                let var_{aname}: &str = &self.var_{aname};"
                    )?;
                } else {
                    writeln!(w, "                let var_{aname} = self.var_{aname};")?;
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
                    // Release the sub-generator's own heap locals on the way out, the same
                    // cleanup a handle's scope-exit free performs (loft#835) — this path owns
                    // the sub-generator directly, so nothing else would.
                    writeln!(
                        w,
                        "                    if let Some(mut _s) = self.sub_{state_idx}.take() {{ _s.drop_stores(stores); }}"
                    )?;
                    writeln!(w, "                    self.state = {};", state_idx + 1)?;
                    writeln!(w, "                    continue;")?;
                    writeln!(w, "                }}")?;
                    writeln!(w, "                return val;")?;
                }
                YieldSegment::ForLoopLazy {
                    pre,
                    setup,
                    body,
                    post,
                    ..
                } => {
                    // State 1 of 2 — run the loop's setup ONCE, then hand over to the
                    // iteration state.  The cursor it initialises is a struct field, so it
                    // survives every advance from here on.
                    for stmt in pre.iter().chain(setup.iter()) {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "                {stmt_code};")?;
                    }
                    writeln!(w, "                self.state = {};", state_idx + 1)?;
                    writeln!(w, "                continue;")?;
                    writeln!(w, "            }}")?;
                    // State 2 of 2 — ONE iteration per advance.  The loop's operators run
                    // inside a wrapper that is left two ways: the header's bound test
                    // `break`s it with `__exhausted` still set (the loop is over), and the
                    // trailing `yield` breaks it after capturing the value (`yield_lazy_wrap`
                    // in emit.rs).  That is the whole of the laziness: the next iteration
                    // does not run until the consumer asks for it.
                    writeln!(w, "            {} => {{", state_idx + 1)?;
                    for attr in attrs {
                        let aname = sanitize(&attr.name);
                        if is_text_slot(&attr.typedef) {
                            writeln!(
                                w,
                                "                let var_{aname}: &str = &self.var_{aname};"
                            )?;
                        } else {
                            writeln!(w, "                let var_{aname} = self.var_{aname};")?;
                        }
                    }
                    writeln!(w, "                let mut __exhausted = true;")?;
                    writeln!(
                        w,
                        "                let mut __y = {};",
                        lazy_yield_init(yield_tp)
                    )?;
                    writeln!(w, "                'iter: loop {{")?;
                    let prev_wrap = self.yield_lazy_wrap.take();
                    self.yield_lazy_wrap = Some((wrap_open.to_string(), wrap_close.to_string()));
                    for stmt in body {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "                    {stmt_code};")?;
                    }
                    self.yield_lazy_wrap = prev_wrap;
                    // Falling off the end means an iteration ran without reaching the yield,
                    // which `detect_lazy_for` has already ruled out — the trailing `yield` is
                    // unconditional.  Leaving the wrapper is the safe reading if it ever did.
                    writeln!(w, "                    break 'iter;")?;
                    writeln!(w, "                }}")?;
                    writeln!(w, "                if __exhausted {{")?;
                    for stmt in post {
                        let stmt_code = self.generate_expr_buf(stmt)?;
                        writeln!(w, "                    {stmt_code};")?;
                    }
                    writeln!(w, "                    self.state = {};", state_idx + 2)?;
                    writeln!(w, "                    continue;")?;
                    writeln!(w, "                }}")?;
                    writeln!(w, "                return __y;")?;
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
                    && !out.coroutine_persistent_fields.contains_key(v)
                    && !fn_scope.contains(v)
                    && !vars.is_argument(*v)
                {
                    ok = false;
                }
            });
            ok
        };
        let tail: Vec<&Value> = tail.iter().filter(|op| reachable(self, op)).collect();
        if !tail.is_empty() {
            let tail_state = after_segments;
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

        let (mut segments, tail) = collect_segments(&body_block.operators, self.data);
        let attrs: Vec<_> = def.attributes().to_vec();
        let yield_tp = match def.returned() {
            Type::Iterator(inner, _) => (**inner).clone(),
            other => other.clone(),
        };

        // P224: compute persistent locals once, share across struct + impl + factory.
        let persistent = coroutine_persistent_locals(self.data, def_nr);
        // loft#928: and their field names with them, so every emitter spells a field the
        // same way.  Derived here rather than at each site because a name is only unique
        // relative to the OTHER fields on the struct.
        let fields = persistent_field_names(&attrs, &persistent, self.data.def(def_nr).variables());

        // Two reasons a loop that `detect_lazy_for` accepted still cannot be lowered lazily.
        //
        // The unified `next_into` channel (a tuple or fn-ref yield) writes its value into the
        // caller's transport buffer and answers a bool, so a lazy loop — which hands back ONE
        // value through the channel's wrap — has nothing to return.
        //
        // A DbRef yield (struct / vector / struct-enum) is held back for a different reason.
        // Lowering it lazily makes the VALUES right — the eager collector's aliasing, which
        // the loud `compile_error!` in the yield-collect path names, cannot happen when each
        // yield returns immediately — but the record is built into a `__ref_*` work local that
        // is re-declared on every advance and is not a struct field, so nothing frees it: a
        // three-yield generator run to exhaustion leaked all three records.  Persisting the
        // work-ref is what unlocks this, and it is its own change; refusing to compile is
        // better than building and leaking.
        //
        // And a lazy loop runs its setup in one state and its body in the next, so anything
        // the setup binds has to outlive the advance that bound it, which only a struct FIELD
        // does; a local the setup declares is scoped to its own match arm and the iteration
        // state cannot name it (E0425 — the `yield from` desugaring's sub-generator handle is
        // exactly this).  Rather than widen what counts as persistent, keep the eager buffer.
        //
        // The verdict is all-or-nothing across the generator, for the same reason
        // `collect_segments` decides it that way: one eager segment makes the factory collect
        // EVERY yield and `next()` collapse to a pop-from-buffer arm, which would run a
        // surviving lazy segment's states a second time.
        let persistent_vars: std::collections::HashSet<u16> =
            persistent.iter().map(|(v, _)| *v).collect();
        let channel_can_suspend = tuple_kinds(&yield_tp).is_none()
            && !matches!(
                yield_tp,
                Type::Function(_, _, _)
                    | Type::Reference(_, _)
                    | Type::Vector(_, _)
                    | Type::Enum(_, true, _)
            );
        let setup_is_carried = |setup: &[Value]| {
            let mut carried = true;
            for stmt in setup {
                stmt.walk(&mut |n| {
                    if let Value::Set(v, _) = n
                        && !persistent_vars.contains(v)
                    {
                        carried = false;
                    }
                });
            }
            carried
        };
        let keep_lazy = channel_can_suspend
            && segments.iter().all(|s| match s {
                YieldSegment::ForLoopLazy { setup, .. } => setup_is_carried(setup),
                _ => true,
            });
        if !keep_lazy {
            for seg in &mut segments {
                if let YieldSegment::ForLoopLazy { pre, whole, .. } = seg {
                    *seg = YieldSegment::ForLoopBody {
                        pre: std::mem::take(pre),
                        body: whole.clone(),
                    };
                }
            }
        }
        let segments = segments;

        // The outer `loop {}` in `next_*` is what lets a state hand over to the next one
        // without returning a value.  A lazily-lowered loop needs it for the same reason a
        // `yield from` does: when the loop ends, control must fall through to the states
        // after it rather than answer the consumer.
        let has_yf = segments.iter().any(|s| {
            matches!(
                s,
                YieldSegment::YieldFrom { .. } | YieldSegment::ForLoopLazy { .. }
            )
        });

        // ── 1. Struct definition ─────────────────────────────────────────────
        emit_struct_def(
            w,
            &struct_name,
            &attrs,
            &segments,
            &yield_tp,
            &persistent,
            &fields,
        )?;

        // ── 2. impl LoftCoroutine ────────────────────────────────────────────
        // Scope the persistent-vars set AND any "declared" insertions to the
        // impl block — they only govern emission inside `next_i64` /
        // `next_text`.  The factory function (emitted next) re-uses the same
        // `Output` instance and would otherwise inherit stale declared marks
        // for `__yf_*` / `__vdb_*` etc., causing E0425 in the eager-collect
        // factory path.
        let prev_persistent = std::mem::take(&mut self.coroutine_persistent_fields);
        let prev_allocated = std::mem::take(&mut self.coroutine_allocated_vars);
        self.coroutine_persistent_fields.clone_from(&fields);
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
        emit_drop_stores(w, &persistent, &fields, self.data, def_nr)?;
        writeln!(w, "}}\n")?;
        self.coroutine_persistent_fields = prev_persistent;
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
            &fields,
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
                &fields,
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
        fields: &std::collections::HashMap<u16, String>,
    ) -> std::io::Result<()> {
        let is_text = is_text_slot(yield_tp);
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
            if is_text_slot(&attr.typedef) {
                writeln!(w, "    let var_{aname}: &str = var_{aname};")?;
            } else {
                writeln!(w, "    let var_{aname} = var_{aname};")?;
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
                if !is_text_slot(var_table.tp(v)) {
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
                // Unreachable by construction: this factory is only emitted when some segment
                // stayed eager, and `collect_segments` pulls every lazy segment back to eager
                // in that case (the buffer must hold ALL yields or the state machine runs the
                // lazy ones twice).
                YieldSegment::ForLoopLazy { .. } => {}
            }
        }
        self.yield_collect = false;
        self.yield_collect_text = false;
        self.yield_collect_dbref = false;
        writeln!(w, "    Box::new({struct_name} {{")?;
        writeln!(w, "        state: 0,")?;
        for attr in attrs {
            let aname = sanitize(&attr.name);
            if is_text_slot(&attr.typedef) {
                writeln!(w, "        var_{aname}: var_{aname}.to_string(),")?;
            } else {
                writeln!(w, "        var_{aname},")?;
            }
        }
        // P224: initialise persistent locals to default — same as the
        // non-for-body factory path.  Without this the eager-collect
        // factory builds a `StructName { … }` with the user-attribute
        // fields but omits the persistent-locals fields that
        // `emit_struct_def` declared, producing a Rust E0063 ("missing
        // fields in initializer").
        for (v, tp) in persistent {
            let n = &fields[v];
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
