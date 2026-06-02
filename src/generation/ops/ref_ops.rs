// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Reference-lifetime Op emitters.
//!
//! Migrated out of `dispatch.rs::output_call_inner` to keep that match under
//! the `dispatch_op_arm_budget` ratchet.  This family is the most
//! context-aware in the dispatcher: `OpFreeRef` / `OpFreeRefIfDistinct` read
//! per-function variable metadata (`is_skip_free`, the variable's `Type`, its
//! sanitized name) to decide between a no-op, a closure-component free, or a
//! plain free-plus-null-reset — exactly the schema/variable awareness the
//! `#rust` templates can't supply.  The rest (`OpEqRef` / `OpNeRef` null-aware
//! comparison, `OpCopyRecord` / `OpSizeofRef` pass-throughs, the
//! `OpNullRefSentinel` literal) round out the family.
//!
//! All emitters reproduce their original arm BYTE-FOR-BYTE, including emitting
//! NOTHING on an argument-shape mismatch (no `DefaultEmitter` fallback).

use super::{EmitCtx, OpEmitter};
use crate::data::{Type, Value};
use std::io;

/// `OpFreeRef` — free a heap-owned reference and reset its variable to the null
/// sentinel.  `args`: `[db]`.  Three cases, decided from variable metadata:
///   - a `skip_free` variable (shares a slot with an owner) → emit `()`;
///   - an fn-ref (`Type::Function`) → free only its closure component when set;
///   - otherwise → `OpFreeRef(cell, <db>, "var")` plus a `store_nr = u16::MAX`
///     reset when the operand is a variable.
pub struct OpFreeRefEmitter;

impl OpEmitter for OpFreeRefEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [db_val] = args {
            // S34/S35: skip_free variables share a slot with an outer variable
            // that already owns the record; suppressing their OpFreeRef
            // prevents a double-free.
            if let Value::Var(v) = db_val
                && ctx
                    .output
                    .data
                    .def(ctx.output.def_nr)
                    .variables
                    .is_skip_free(*v)
            {
                write!(ctx.w, "()")?;
                return Ok(());
            }
            // free the closure component of fn-ref (u32, DbRef) variables.
            // Non-capturing lambdas have store_nr = u16::MAX (null sentinel).
            if let Value::Var(v) = db_val
                && matches!(
                    ctx.output.data.def(ctx.output.def_nr).variables.tp(*v),
                    Type::Function(_, _, _)
                )
            {
                let vn = format!(
                    "var_{}",
                    super::super::sanitize(
                        ctx.output.data.def(ctx.output.def_nr).variables.name(*v)
                    )
                );
                write!(
                    ctx.w,
                    "if {vn}.1.store_nr != u16::MAX {{ \
                     OpFreeRef(cell,{vn}.1, \"{vn}.1\"); \
                     {vn}.1.store_nr = u16::MAX }}"
                )?;
                return Ok(());
            }
            let var_name = if let Value::Var(v) = db_val {
                format!(
                    "var_{}",
                    super::super::sanitize(
                        ctx.output.data.def(ctx.output.def_nr).variables.name(*v)
                    )
                )
            } else {
                String::new()
            };
            write!(ctx.w, "OpFreeRef(cell,")?;
            ctx.emit(db_val)?;
            write!(ctx.w, ", \"{var_name}\")")?;
            // Reset variable to null sentinel after free.
            if let Value::Var(_) = db_val {
                write!(ctx.w, "; {var_name}.store_nr = u16::MAX")?;
            }
        }
        Ok(())
    }
}

/// `OpStoreTag` — Plan-57 store-identity gate.  `args`: `[db, tag]`.  Stamps the
/// allocation-site `tag` on the store, mirroring the interpreter's `store_tag`.
/// Emitted only under `LOFT_STORE_TAG`; absent from normal builds.
pub struct OpStoreTagEmitter;

impl OpEmitter for OpStoreTagEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [db_val, tag_val] = args {
            write!(ctx.w, "OpStoreTag(cell,")?;
            ctx.emit(db_val)?;
            write!(ctx.w, ", ")?;
            ctx.emit(tag_val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}

/// `OpFreeRefTag` — Plan-57 store-identity gate.  `args`: `[db, tag]`.  Byte-for-byte
/// the `OpFreeRef` emission (skip_free → `()`, fn-ref → closure-component free, plain
/// → free + null-reset) but routed through the verifying `OpFreeRefTag` runtime, so a
/// tagged native build behaves exactly like the untagged one plus the tag check.
/// Emitted only under `LOFT_STORE_TAG`.
pub struct OpFreeRefTagEmitter;

impl OpEmitter for OpFreeRefTagEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [db_val, tag_val] = args {
            // skip_free variables share a slot with an owner — suppress, as OpFreeRef.
            if let Value::Var(v) = db_val
                && ctx
                    .output
                    .data
                    .def(ctx.output.def_nr)
                    .variables
                    .is_skip_free(*v)
            {
                write!(ctx.w, "()")?;
                return Ok(());
            }
            // fn-ref: free + verify only the closure component when set.
            if let Value::Var(v) = db_val
                && matches!(
                    ctx.output.data.def(ctx.output.def_nr).variables.tp(*v),
                    Type::Function(_, _, _)
                )
            {
                let vn = format!(
                    "var_{}",
                    super::super::sanitize(
                        ctx.output.data.def(ctx.output.def_nr).variables.name(*v)
                    )
                );
                write!(
                    ctx.w,
                    "if {vn}.1.store_nr != u16::MAX {{ OpFreeRefTag(cell,{vn}.1, "
                )?;
                ctx.emit(tag_val)?;
                write!(ctx.w, "); {vn}.1.store_nr = u16::MAX }}")?;
                return Ok(());
            }
            let var_name = if let Value::Var(v) = db_val {
                format!(
                    "var_{}",
                    super::super::sanitize(
                        ctx.output.data.def(ctx.output.def_nr).variables.name(*v)
                    )
                )
            } else {
                String::new()
            };
            write!(ctx.w, "OpFreeRefTag(cell,")?;
            ctx.emit(db_val)?;
            write!(ctx.w, ", ")?;
            ctx.emit(tag_val)?;
            write!(ctx.w, ")")?;
            // Reset variable to null sentinel after free (mirrors OpFreeRef).
            if let Value::Var(_) = db_val {
                write!(ctx.w, "; {var_name}.store_nr = u16::MAX")?;
            }
        }
        Ok(())
    }
}

/// `OpFreeRefIfDistinct` — free the placeholder only when its `store_nr`
/// differs from the witness's, so the fresh-store path reclaims the orphan and
/// the adoption path leaves both slots alone.  `args`: `[placeholder, witness]`.
pub struct OpFreeRefIfDistinctEmitter;

impl OpEmitter for OpFreeRefIfDistinctEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [ph_val, wit_val] = args {
            let ph_name = if let Value::Var(v) = ph_val {
                format!(
                    "var_{}",
                    super::super::sanitize(
                        ctx.output.data.def(ctx.output.def_nr).variables.name(*v)
                    )
                )
            } else {
                String::new()
            };
            write!(ctx.w, "if ")?;
            ctx.emit(ph_val)?;
            write!(ctx.w, ".store_nr != ")?;
            ctx.emit(wit_val)?;
            write!(ctx.w, ".store_nr {{ OpFreeRef(cell,")?;
            ctx.emit(ph_val)?;
            write!(ctx.w, ", \"{ph_name}\")")?;
            if let Value::Var(_) = ph_val {
                write!(ctx.w, "; {ph_name}.store_nr = u16::MAX")?;
            }
            write!(ctx.w, " }}")?;
        }
        Ok(())
    }
}

/// `OpCopyRecord` — deep copy (`copy_block` + `copy_claims`).
/// `args`: `[src, dst, tp]` → `OpCopyRecord(cell, <src>, <dst>, <tp>_i32)`.
pub struct OpCopyRecordEmitter;

impl OpEmitter for OpCopyRecordEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [src, dst, tp_val] = args {
            write!(ctx.w, "OpCopyRecord(cell,")?;
            ctx.emit(src)?;
            write!(ctx.w, ", ")?;
            ctx.emit(dst)?;
            write!(ctx.w, ", ")?;
            ctx.emit_i32_slot(tp_val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}

/// `OpSizeofRef` — record size of a reference.  `args`: `[val]`.
pub struct OpSizeofRefEmitter;

impl OpEmitter for OpSizeofRefEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [val] = args {
            write!(ctx.w, "OpSizeofRef(cell,")?;
            ctx.emit(val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}
