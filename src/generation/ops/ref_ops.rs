// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

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
                    ctx.output.data.def(ctx.output.def_nr).variables().tp(*v),
                    Type::Function(_, _, _)
                )
            {
                let vn = format!(
                    "var_{}",
                    super::super::sanitize(
                        ctx.output.data.def(ctx.output.def_nr).variables().name(*v)
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
            // @PLN90 #495 — a runtime-Join local's scope-exit free targets the
            // store r actually OWNS (`_own_store_<name>`, NULL once r holds a
            // borrowed view), NOT var_<name> — which on the loop-ran path IS the
            // view, and freeing it would whole-store-free a caller-owned element.
            if let Value::Var(v) = db_val
                && ctx.output.witness_vars.contains(v)
            {
                let nm = super::super::sanitize(
                    ctx.output.data.def(ctx.output.def_nr).variables().name(*v),
                );
                write!(
                    ctx.w,
                    "if _own_store_{nm}.store_nr != u16::MAX {{ \
                     OpFreeRef(cell,_own_store_{nm}, \"var_{nm}(owned)\"); }} \
                     _own_store_{nm}.store_nr = u16::MAX; var_{nm}.store_nr = u16::MAX"
                )?;
                return Ok(());
            }
            let var_name = if let Value::Var(v) = db_val {
                format!(
                    "var_{}",
                    super::super::sanitize(
                        ctx.output.data.def(ctx.output.def_nr).variables().name(*v)
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
                    ctx.output.data.def(ctx.output.def_nr).variables().tp(*v),
                    Type::Function(_, _, _)
                )
            {
                let vn = format!(
                    "var_{}",
                    super::super::sanitize(
                        ctx.output.data.def(ctx.output.def_nr).variables().name(*v)
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
                        ctx.output.data.def(ctx.output.def_nr).variables().name(*v)
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
                        ctx.output.data.def(ctx.output.def_nr).variables().name(*v)
                    )
                )
            } else {
                String::new()
            };
            // Parenthesise both operands: a witness that is a `&` PARAMETER
            // (loft#759) emits as `*var_b`, and `*var_b.store_nr` binds as
            // `*(var_b.store_nr)` — a deref of the `u16` field, which is
            // E0614, not a comparison.
            write!(ctx.w, "if (")?;
            ctx.emit(ph_val)?;
            write!(ctx.w, ").store_nr != (")?;
            ctx.emit(wit_val)?;
            write!(ctx.w, ").store_nr {{ OpFreeRef(cell,")?;
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

/// @PLN87 P2.1 — `OpInitRefSentinel(slot)` sets the slot to the null sentinel
/// (`DbRef::NULL`, `store_nr == u16::MAX`) WITHOUT freeing its prior contents, so
/// a following `OpDatabase` allocates a FRESH store instead of clearing+reusing
/// the current one (`OpDatabase` reuses iff `store_nr != u16::MAX`).  This is the
/// native twin of the interpreter `OpInitRefSentinel` opcode (`state/io.rs`); it
/// is interp-only otherwise, so it had no native emitter before.  `args`: `[slot]`.
pub struct OpInitRefSentinelEmitter;

impl OpEmitter for OpInitRefSentinelEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [slot] = args {
            ctx.emit(slot)?;
            write!(ctx.w, " = DbRef::NULL")?;
        }
        Ok(())
    }
}

/// @PLN87 P2.1 — `OpPutRef(slot, value)` writes a DbRef into `slot` WITHOUT
/// freeing the slot's prior contents — a raw pointer copy (alias).  Used to
/// stash a rebindable heap param's caller-supplied DbRef into its `__orig`
/// witness at function entry.  Native twin of the interpreter `OpPutRef` opcode;
/// interp-only otherwise.  `args`: `[slot, value]`.
pub struct OpPutRefEmitter;

impl OpEmitter for OpPutRefEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [slot, value] = args {
            ctx.emit(slot)?;
            write!(ctx.w, " = ")?;
            ctx.emit(value)?;
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
            // #250: when the copy type is a nested vector, resolve its id at
            // RUNTIME (order-independent) rather than trusting the parser's
            // literal — the two diverge in native at 3+ nesting depth.  The
            // type-id is encoded `tp = id | (0x8000 free-source bit)`.
            if let Value::Int(n) = tp_val {
                let free_bit = n & 0x8000;
                let known = u16::try_from(n & 0x7FFF).unwrap_or(u16::MAX);
                // Only the 2+-deep chains diverge; depth 0/1 keep the literal
                // (the shallow `vector<base>` id is parser↔runtime stable).
                if let Some((depth, base)) = ctx.output.vector_runtime_id_chain(known)
                    && depth >= 2
                {
                    write!(ctx.w, "{{ let _v0 = stores.vector({base});")?;
                    for i in 1..depth {
                        write!(ctx.w, " let _v{i} = stores.vector(_v{});", i - 1)?;
                    }
                    write!(ctx.w, " OpCopyRecord(cell, ")?;
                    ctx.emit(src)?;
                    write!(ctx.w, ", ")?;
                    ctx.emit(dst)?;
                    write!(ctx.w, ", ((_v{} as i32) | {free_bit})) }}", depth - 1)?;
                    return Ok(());
                }
            }
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
