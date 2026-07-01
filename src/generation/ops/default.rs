// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Default emitter — handles every Op for which no custom emitter is
//! registered.  Dispatches between two byte-identical extractions of
//! pre-phase-09 emission paths:
//!
//! * `def_fn.rust.is_empty()` → `Output::user_fn_call_body` (user-defined
//!   functions and Op stubs that ship as `pub fn` in
//!   `src/codegen_runtime.rs`).
//! * otherwise → `Output::substitute_template_body` (`#rust"…@v0…"`
//!   template substitution).
//!
//! Today's `dispatch.rs` makes the same template-vs-user-fn choice at the
//! call site:
//!
//! ```ignore
//! if def_fn.rust.is_empty() {
//!     self.output_call_user_fn(w, def_fn, vals)
//! } else {
//!     self.output_call_template(w, def_fn, vals)
//! }
//! ```
//!
//! After phase 00 step 0.5, both `output_call_user_fn` and
//! `output_call_template` route through `emit_op`.  Falling through to
//! this default emitter inspects `def_fn.rust` and picks the matching
//! body.  Behaviour is byte-identical for both paths.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

pub struct DefaultEmitter;

impl OpEmitter for DefaultEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if ctx.def_fn.rust().is_empty() {
            ctx.output.user_fn_call_body(&mut *ctx.w, ctx.def_fn, args)
        } else {
            ctx.output
                .substitute_template_body(&mut *ctx.w, ctx.def_fn, args)
        }
    }
}
