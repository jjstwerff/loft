// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Coroutine Op emitters (N8b — native stackful generators).
//!
//! Migrated out of `dispatch.rs::output_call_inner` to keep that match under
//! the `dispatch_op_arm_budget` ratchet.  Both are byte-identical
//! pass-throughs to the `loft::codegen_runtime::coroutine_*` runtime helpers;
//! like the original arms they emit NOTHING when no generator argument is
//! present (no `DefaultEmitter` fallback).

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

/// `OpCoroutineNext` emitter — advance a native coroutine and read its yield.
///
/// `args`: `[gen, value_size?]`.  `value_size` selects the runtime reader and
/// the result cast: 8 → `i64`, 1 → `bool`, 16 (`size_of::<&str>()`) → text,
/// else `i32` (the fallback when arg 1 is absent is 4 → i32).
pub struct OpCoroutineNextEmitter;

impl OpEmitter for OpCoroutineNextEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let Some(gen_val) = args.first() {
            let gen_code = ctx.output.generate_expr_buf(gen_val)?;
            let value_size = if let Some(Value::Int(n)) = args.get(1) {
                *n
            } else {
                4 // fallback: i32
            };
            match value_size {
                8 => write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_i64({gen_code}, stores)"
                )?,
                1 => write!(
                    ctx.w,
                    "(loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) != 0)"
                )?,
                // size_of::<&str>() == 16 — text-yielding generator.
                16 => write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_text({gen_code}, stores)"
                )?,
                _ => write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) as i32"
                )?,
            }
        }
        Ok(())
    }
}

/// `OpCoroutineExhausted` emitter — N8b.2 test whether a native coroutine is
/// exhausted.  `args`: `[gen]` (the generator `DbRef` expression).
pub struct OpCoroutineExhaustedEmitter;

impl OpEmitter for OpCoroutineExhaustedEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let Some(gen_val) = args.first() {
            let gen_code = ctx.output.generate_expr_buf(gen_val)?;
            write!(
                ctx.w,
                "loft::codegen_runtime::coroutine_is_exhausted({gen_code})"
            )?;
        }
        Ok(())
    }
}
