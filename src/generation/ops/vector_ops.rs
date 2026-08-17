// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
// loft#885 — indexed reads through a loop-invariant vector header

//! Indexed reads emitted against a header the enclosing loop already derived, once
//! [`crate::generation::hoist`] proved the loop writes no store.
//!
//! Two element-address ops (`v[i]` as an address, raising and nullable) plus the scalar
//! getters, which fuse the address and the load into one call rather than building a `DbRef`
//! between them. Every emitter falls back to the `#rust` template the moment there is no
//! header for the vector being read, which is every read outside such a loop — so the shape
//! that changes is exactly the one the analysis vouched for.
//!
//! The runtime helpers keep the fast path to an in-range index and route everything else
//! (negative, out of range, `i64::MIN`, a null or empty vector) back into `get_vector` /
//! `vec_get_or_raise_runtime`. That is deliberate: the answers those cases give — and the
//! `IndexOutOfBounds` / `NegativeIndex` raise the non-nullable form owes — keep one
//! definition rather than two that can drift.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

/// The `&(vector)` operand plus the header local, or `None` when this read is not covered.
fn header_for<'a>(ctx: &'a EmitCtx<'_, '_>, arg: &Value) -> Option<&'a str> {
    match arg.unspan() {
        Value::Var(v) => ctx.output.active_vec_header(*v),
        _ => None,
    }
}

/// `LOFT_HOIST_VERIFY=1` picks the checking monomorphisation.
fn verify(ctx: &EmitCtx<'_, '_>) -> &'static str {
    if ctx.output.hoist_verify {
        "true"
    } else {
        "false"
    }
}

/// `OpGetInt` / `OpGetSingle` / `OpGetFloat` — a scalar read of `v[i]` inside a loop that
/// hoisted `v`'s header, emitted as ONE load: the bounds test, then the value.
///
/// Everything the pair used to do between those two — build the element `DbRef`, test its
/// `rec` against the null element, resolve the store from it again, and re-check
/// `rec != 0 && valid(..)` inside the getter — is decided by the bounds test already.
/// Worth ~3.2× on top of the header hoist alone — more than the hoist itself, because the
/// second store resolution it removes costs more than the arithmetic it saves
/// (loft#885 stage 2; PERFORMANCE.md § what the fusion is worth). `LOFT_NO_ELEM_FUSE=1`
/// emits the unfused form, which is the middle rung of that measurement.
///
/// Anything else (no header, an expression instead of a variable for the vector, a getter
/// with a different shape) emits the `#rust` template unchanged.
pub struct FusedElementReadEmitter;

impl OpEmitter for FusedElementReadEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let Some(fused) = ctx.output.fused_element_read(ctx.def_fn.name(), args) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let Some(header) = ctx.output.active_vec_header(fused.var) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let (header, ty, absent) = (header.to_string(), fused.rust_type, fused.absent);
        let verify = verify(ctx);
        write!(
            ctx.w,
            "vector::get_elem_hoisted::<{ty}, {verify}>(&{header}, &("
        )?;
        ctx.emit(fused.vector)?;
        write!(ctx.w, "), (")?;
        ctx.emit(fused.size)?;
        write!(ctx.w, ") as u32, ")?;
        ctx.emit(fused.index)?;
        write!(ctx.w, ", (")?;
        ctx.emit(fused.fld)?;
        write!(ctx.w, ") as u32, {absent}, &stores.allocations)")
    }
}

/// `OpGetVectorNullable` — `v[i]` where an out-of-range index answers the null element
/// (for-loop iteration depends on that null as its end signal).  `args`:
/// `[vector, elem_size, index]`.
pub struct OpGetVectorNullableEmitter;

impl OpEmitter for OpGetVectorNullableEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let (Some(header), [vec_val, size_val, index_val]) =
            (args.first().and_then(|a| header_for(ctx, a)), args)
        else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let header = header.to_string();
        let verify = verify(ctx);
        write!(
            ctx.w,
            "vector::get_vector_hoisted::<{verify}>(&{header}, &("
        )?;
        ctx.emit(vec_val)?;
        write!(ctx.w, "), (")?;
        ctx.emit(size_val)?;
        write!(ctx.w, ") as u32, ")?;
        ctx.emit(index_val)?;
        write!(ctx.w, ", &stores.allocations)")
    }
}

/// `OpGetVector` — user-facing `v[i]`, which RAISES on an out-of-range or negative index.
/// `args`: `[vector, elem_size, index]`.
///
/// The receiver and the index are bound to locals before the call for the same reason the
/// template does it (@P321d / @P338): the fallback takes `&mut stores`, and a nested index
/// or a checked index expression would otherwise still be evaluating its own borrow when
/// that one is taken (E0499).
pub struct OpGetVectorEmitter;

impl OpEmitter for OpGetVectorEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let (Some(header), [vec_val, size_val, index_val]) =
            (args.first().and_then(|a| header_for(ctx, a)), args)
        else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let header = header.to_string();
        let verify = verify(ctx);
        write!(ctx.w, "{{let __vr = ")?;
        ctx.emit(vec_val)?;
        write!(ctx.w, "; let __vi = ")?;
        ctx.emit(index_val)?;
        write!(
            ctx.w,
            "; stores.vec_get_hoisted_or_raise_runtime::<{verify}>(&{header}, &__vr, ("
        )?;
        ctx.emit(size_val)?;
        write!(ctx.w, ") as u32, __vi)}}")
    }
}
