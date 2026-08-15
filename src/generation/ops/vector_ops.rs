// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// loft#885 — indexed reads through a loop-invariant vector header

//! The two element-address ops, emitted against a header the enclosing loop already
//! derived when [`crate::generation::hoist`] proved the loop writes no store.
//!
//! Both emitters fall back to the `#rust` template the moment there is no header for the
//! vector being read, which is every read outside such a loop — so the shape that changes
//! is exactly the one the analysis vouched for.
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
