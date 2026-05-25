// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Store-refcount Op emitter.
//!
//! Migrated out of `dispatch.rs::output_call_inner` to keep that match under
//! the `dispatch_op_arm_budget` ratchet (per-Op emission belongs in the
//! registry, not the bulk match).
//!
//! `OpIncRc` (P259): increments the referenced store's refcount.  Emitted by
//! `emit_lambda_code` after each `SetDbRef` that captures a heap-owned cell
//! into a closure record's auto-`Reference` attribute.  Like the original arm,
//! it emits NOTHING when the argument shape doesn't match — byte-identical.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

/// `OpIncRc` emitter — `args`: `[db]` → `OpIncRc(cell, <db>)`.
pub struct OpIncRcEmitter;

impl OpEmitter for OpIncRcEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [db_val] = args {
            write!(ctx.w, "OpIncRc(cell,")?;
            ctx.emit(db_val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}
