// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Default emitter — falls through to today's `#rust"…@v0…"` template
//! substitution path via `Output::substitute_template_body`.
//!
//! When no custom emitter is registered for an Op, `emit_op` lands here.
//! This emitter delegates to `Output::substitute_template_body` (the
//! byte-identical extraction of the pre-phase-09 `output_call_template`
//! body).  No behaviour change for any call site.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

pub struct DefaultTemplateEmitter;

impl OpEmitter for DefaultTemplateEmitter {
    fn emit(
        &self,
        ctx: &mut EmitCtx<'_, '_>,
        args: &[Value],
    ) -> io::Result<()> {
        ctx.output
            .substitute_template_body(&mut *ctx.w, ctx.def_fn, args)
    }
}
