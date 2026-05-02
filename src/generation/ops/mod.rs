// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Per-Op emitter dispatch (plan 09 scaffold).
//!
//! Native code generation today substitutes `#rust"…@v0…"` templates from
//! `default/*.loft` declarations.  Plan 09 introduces a dispatch layer:
//! every Op-emission call site routes through `emit_op(ctx, name, args)`.
//! When no custom emitter is registered for `name`, dispatch falls through
//! to `DefaultTemplateEmitter`, which performs today's template substitution
//! unchanged.
//!
//! Custom emitters live in `src/generation/ops/<op>.rs` and override the
//! default for Ops that need context-aware emission (field widths, ref
//! flavour, generic bindings) the template can't supply.
//!
//! After phase 00 (this scaffold + step 0.7b's let-bind-on-repeat already
//! shipped), the registry is empty — no Op is intercepted.  The
//! infrastructure is in place for future phases (and external contributors)
//! to opt in per Op.
//!
//! Plan reference: `doc/claude/plans/09-native-runtime-rewrite/00-scaffold.md`.

// Phase 00 step 0.2 ships the dispatch surface without consumers; steps
// 0.4-0.7 wire the call sites in.  Until then, the trait + registry are
// dead code by design.
#![allow(dead_code)]

pub mod default;

use crate::data::{Definition, Value};
use std::io::{self, Write};

/// Context passed to every emitter.  Carries the writer, the Op
/// definition, and references to the rest of the codegen state the
/// emitter may need (resolved via accessor helpers as those are added).
pub struct EmitCtx<'a, W: Write + ?Sized> {
    pub w: &'a mut W,
    pub def_fn: &'a Definition,
}

/// Trait every per-Op emitter implements.  Default emitter dispatches
/// to `#rust` template substitution unchanged.
pub trait OpEmitter: Send + Sync {
    fn emit(
        &self,
        ctx: &mut EmitCtx<'_, dyn Write>,
        args: &[Value],
    ) -> io::Result<()>;
}

/// Dispatch entry point — every Op-emission call site routes here.
///
/// When `name` is registered, calls the custom emitter; otherwise falls
/// through to `DefaultTemplateEmitter` which performs the existing
/// `#rust` template substitution.
pub fn emit_op(
    ctx: &mut EmitCtx<'_, dyn Write>,
    name: &str,
    args: &[Value],
) -> io::Result<()> {
    if let Some(emitter) = registry().get(name) {
        emitter.emit(ctx, args)
    } else {
        default::DefaultTemplateEmitter.emit(ctx, args)
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
