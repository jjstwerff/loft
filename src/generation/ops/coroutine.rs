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
/// `args`: `[gen, value_size?]`.  `value_size` packs two fields:
/// - low byte  = byte size of the yielded value
/// - high byte = channel tag:
///   - 0 = legacy per-type channel — dispatch on byte size:
///     8 → `i64` / 1 → `bool` / 12 → `DbRef` (@P326) /
///     16 → text / else → `i32`
///   - 1 = unified `next_into(stores, &mut [i64; N])` channel (@P327 native)
///     — allocate a stack `[i64; N]` buffer, call `coroutine_next_into`,
///     and rebuild the consumer's tuple from buffer slots
pub struct OpCoroutineNextEmitter;

impl OpEmitter for OpCoroutineNextEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let Some(gen_val) = args.first() {
            let gen_code = ctx.output.generate_expr_buf(gen_val)?;
            let raw_value_size = if let Some(Value::Int(n)) = args.get(1) {
                *n
            } else {
                4 // fallback: i32
            };
            let channel = (raw_value_size >> 8) & 0xFF;
            let byte_size = raw_value_size & 0xFF;
            if channel == 1 {
                // @P327 native — unified channel, tuple-of-i64 rebuild.
                // Buffer size = ceil(byte_size / 8) i64 slots; emit an
                // inline block that allocates the buffer, calls
                // `coroutine_next_into`, and rebuilds the consumer's
                // tuple from the buffer slots.
                let slots = (byte_size as usize).div_ceil(8);
                write!(
                    ctx.w,
                    "{{ let mut _loft_yield_buf: [i64; {slots}] = [0; {slots}]; \
                     loft::codegen_runtime::coroutine_next_into({gen_code}, stores, &mut _loft_yield_buf); \
                     ("
                )?;
                for i in 0..slots {
                    if i > 0 {
                        write!(ctx.w, ", ")?;
                    }
                    write!(ctx.w, "_loft_yield_buf[{i}]")?;
                }
                write!(ctx.w, ") }}")?;
                return Ok(());
            }
            if channel == 2 {
                // @P328 native — unified channel, fn-ref rebuild.  Native
                // fn-ref repr is `(u32 d_nr, DbRef closure)` (see
                // `generation/mod.rs::rust_type` for `Type::Function`).
                // The state machine packs (d_nr, closure) into 2 i64
                // slots; the consumer unpacks to rebuild the tuple:
                //   slot[0]: low 32 bits = d_nr; bits 32..48 = closure.store_nr
                //   slot[1]: low 32 = closure.rec; high 32 = closure.pos
                write!(
                    ctx.w,
                    "{{ let mut _loft_yield_buf: [i64; 2] = [0; 2]; \
                     loft::codegen_runtime::coroutine_next_into({gen_code}, stores, &mut _loft_yield_buf); \
                     ((_loft_yield_buf[0] as u32), DbRef {{ \
                       store_nr: ((_loft_yield_buf[0] >> 32) & 0xFFFF) as u16, \
                       rec: _loft_yield_buf[1] as u32, \
                       pos: (_loft_yield_buf[1] >> 32) as u32 \
                     }}) }}"
                )?;
                return Ok(());
            }
            match byte_size {
                8 => write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_i64({gen_code}, stores)"
                )?,
                1 => write!(
                    ctx.w,
                    "(loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) != 0)"
                )?,
                // size_of::<DbRef>() == 12 — Reference-yielding generator (@P326).
                12 => write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_dbref({gen_code}, stores)"
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
