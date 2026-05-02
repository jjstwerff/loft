// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Default emitter — falls through to today's `#rust"…@v0…"` template
//! substitution path.
//!
//! Phase 00 introduces the `OpEmitter` trait and dispatch entry point
//! (`emit_op`); when no custom emitter is registered for an Op, dispatch
//! lands here.  This emitter does NOT (yet) host the substitution logic
//! itself — that still lives in
//! `src/generation/calls.rs::Output::output_call_template`.  Step 0.3 of
//! phase 00 will hoist the substitution into a free function this emitter
//! can call.
//!
//! For now: this emitter is reachable from the dispatch but never invoked
//! because no call site routes through `emit_op` yet.  Steps 0.4-0.7 add
//! the call-site hoists; once they do, the emitter delegates to
//! `output_call_template`'s existing logic via the same `Output` struct.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io::{self, Write};

pub struct DefaultTemplateEmitter;

impl OpEmitter for DefaultTemplateEmitter {
    fn emit(
        &self,
        _ctx: &mut EmitCtx<'_, dyn Write>,
        _args: &[Value],
    ) -> io::Result<()> {
        // Step 0.3 will wire this up to call `substitute_template`
        // (extracted from `output_call_template`).  Until then, this
        // emitter is unreachable — no call site routes through `emit_op`.
        unreachable!(
            "DefaultTemplateEmitter::emit called before phase 00 step 0.3 \
             extraction landed.  Until step 0.3, no call site dispatches \
             through emit_op; reaching this branch indicates an out-of-order \
             commit.  See doc/claude/plans/09-native-runtime-rewrite/00-scaffold.md."
        );
    }
}
