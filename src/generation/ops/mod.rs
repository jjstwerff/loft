// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Per-Op emitter dispatch (plan 09 scaffold).
//!
//! Native code generation today substitutes `#rust"…@v0…"` templates from
//! `default/*.loft` declarations.  Plan 09 introduces a dispatch layer:
//! every Op-emission call site routes through `emit_op(ctx, name, args)`.
//! When no custom emitter is registered for `name`, dispatch falls through
//! to `DefaultEmitter`, which delegates back to
//! `Output::substitute_template_body` (the byte-identical extraction of
//! the original `output_call_template` body).
//!
//! Custom emitters live in `src/generation/ops/<op>.rs` and override the
//! default for Ops that need context-aware emission (field widths, ref
//! flavour, generic bindings) the template can't supply.
//!
//! After phase 00 (this scaffold + step 0.7b's let-bind-on-repeat already
//! shipped + steps 0.4-0.7 hoisting call sites), the registry is empty —
//! no Op is intercepted.  The infrastructure is in place for future phases
//! (and external contributors) to opt in per Op.
//!
//! Plan reference: `doc/claude/plans/09-native-runtime-rewrite/00-scaffold.md`.

// Phase 00 step 0.2 shipped this surface without consumers; step 0.4
// wires the first call site (`output_call_template`).  Future phases
// add more call sites and custom emitters.
#![allow(dead_code)]

pub mod default;

use super::Output;
use crate::data::{Definition, Value};
use std::io::{self, Write};

/// Context passed to every emitter.  Carries the writer, the Op
/// definition, and a back-reference to the codegen state (`Output`)
/// so emitters can call helpers like `substitute_template_body` or
/// `generate_expr_buf` without losing access to per-function state.
///
/// Two lifetime parameters:
///   - `'a` — the borrow scope of EmitCtx itself (the lifetime of the
///     mutable references to the writer and `Output`).
///   - `'b` — `Output`'s data/stores lifetime (longer-lived).
pub struct EmitCtx<'a, 'b> {
    pub w: &'a mut dyn Write,
    pub def_fn: &'a Definition,
    pub output: &'a mut Output<'b>,
}

/// Trait every per-Op emitter implements.  The default emitter
/// dispatches to `#rust` template substitution unchanged.
pub trait OpEmitter: Send + Sync {
    fn emit(
        &self,
        ctx: &mut EmitCtx<'_, '_>,
        args: &[Value],
    ) -> io::Result<()>;
}

/// Dispatch entry point — every Op-emission call site routes here.
///
/// When `name` is registered, calls the custom emitter; otherwise falls
/// through to `DefaultEmitter` which delegates to
/// `Output::substitute_template_body`.
pub fn emit_op(
    ctx: &mut EmitCtx<'_, '_>,
    name: &str,
    args: &[Value],
) -> io::Result<()> {
    if let Some(emitter) = registry().get(name) {
        emitter.emit(ctx, args)
    } else {
        default::DefaultEmitter.emit(ctx, args)
    }
}

/// Registry of custom emitters, keyed by Op name.
///
/// Phase 00 ships an empty registry — every Op falls through to the
/// default template substitution.  Future phases populate this map by
/// inserting boxed trait objects.
fn registry() -> &'static std::collections::HashMap<&'static str, Box<dyn OpEmitter>> {
    static R: std::sync::OnceLock<
        std::collections::HashMap<&'static str, Box<dyn OpEmitter>>,
    > = std::sync::OnceLock::new();
    R.get_or_init(build_registry)
}

fn build_registry() -> std::collections::HashMap<&'static str, Box<dyn OpEmitter>> {
    // Custom emitters register here as future phases add them.
    // Example (when phase 05 lands):
    //     r.insert("OpWriteIntFile", Box::new(op_write_int_file::Emitter));
    std::collections::HashMap::new()
}
