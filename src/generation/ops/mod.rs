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
pub mod forwarding_smoke;
pub mod int_compare;
pub mod key_ops;
pub mod parallel;

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

impl EmitCtx<'_, '_> {
    /// Convenience: emit a `Value` to `self.w`.  Equivalent to
    /// `self.output.output_code_inner(&mut *self.w, v)` but cuts the
    /// reborrow boilerplate at every emitter call site.  Phase 03
    /// learned that real emitters call this pattern many times per
    /// `emit()` body — the wrapper makes those call sites readable.
    pub fn emit(&mut self, v: &Value) -> io::Result<()> {
        self.output.output_code_inner(&mut *self.w, v)
    }

    /// Convenience: emit a `Value` as an i32-typed slot.  Used when
    /// the call-site context expects i32 (tp-numbers, field offsets,
    /// flag-enum slots) and the runtime helper signature is fixed at
    /// i32.  Equivalent to
    /// `self.output.emit_i32_slot(&mut *self.w, v)`.
    pub fn emit_i32_slot(&mut self, v: &Value) -> io::Result<()> {
        self.output.emit_i32_slot(&mut *self.w, v)
    }
}

/// Trait every per-Op emitter implements.  The default emitter
/// dispatches to `#rust` template substitution unchanged.
pub trait OpEmitter: Send + Sync {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()>;
}

/// Dispatch entry point — every Op-emission call site routes here.
///
/// When `name` is registered, calls the custom emitter; otherwise falls
/// through to `DefaultEmitter` which delegates to
/// `Output::substitute_template_body`.
pub fn emit_op(ctx: &mut EmitCtx<'_, '_>, name: &str, args: &[Value]) -> io::Result<()> {
    if let Some(emitter) = registry().get(name) {
        emitter.emit(ctx, args)
    } else {
        default::DefaultEmitter.emit(ctx, args)
    }
}

/// True when a custom emitter is registered for `name`.
///
/// Used by `dispatch.rs::output_call_inner` to give registered emitters
/// priority over the special-case match arms (phase 00 step 0.6).
/// While the registry is empty this always returns false, so the
/// dispatch falls through to today's match unchanged.
pub fn has_custom_emitter(name: &str) -> bool {
    registry().contains_key(name)
}

/// Registry of custom emitters, keyed by Op name.
///
/// Phase 00 ships an empty registry — every Op falls through to the
/// default template substitution.  Future phases populate this map by
/// inserting boxed trait objects.
fn registry() -> &'static std::collections::HashMap<&'static str, Box<dyn OpEmitter>> {
    static R: std::sync::OnceLock<std::collections::HashMap<&'static str, Box<dyn OpEmitter>>> =
        std::sync::OnceLock::new();
    R.get_or_init(build_registry)
}

fn build_registry() -> std::collections::HashMap<&'static str, Box<dyn OpEmitter>> {
    let mut r: std::collections::HashMap<&'static str, Box<dyn OpEmitter>> =
        std::collections::HashMap::new();

    // Plan 09 phase 00 runtime smoke test — register a forwarding emitter
    // for the curated Op list in `forwarding_smoke::FORWARDING_OP_NAMES`.
    // Each entry exercises the registry → emit_op → custom emitter
    // dispatch path with byte-identical output.  When a future phase
    // ships a real (non-forwarding) emitter for one of these Op names,
    // remove the name from FORWARDING_OP_NAMES — that phase's
    // `r.insert(name, …)` below takes priority.
    for op_name in forwarding_smoke::FORWARDING_OP_NAMES {
        r.insert(op_name, Box::new(forwarding_smoke::ForwardingEmitter));
    }

    // Real (non-forwarding) custom emitters register here as future
    // phases add them.  Example (when phase 05 lands):
    //     r.insert("OpWriteIntFile", Box::new(op_write_int_file::Emitter));

    // Phase 03 — parallel-for emitter family.  Replaces the 95-line
    // special case in dispatch.rs::output_call_inner.  Both Op names
    // share the same emitter (the special case treated them
    // identically; n_parallel_for_light is a thread-count hint that
    // doesn't change emission shape).
    r.insert("n_parallel_for", Box::new(parallel::ParallelForEmitter));
    r.insert(
        "n_parallel_for_light",
        Box::new(parallel::ParallelForEmitter),
    );

    // Phase 04 — key-keyed Op emitters.  Replaces ~70 lines of two
    // arms in dispatch.rs (`"OpGetRecord" =>` + `"OpIterate" =>`).
    r.insert("OpGetRecord", Box::new(key_ops::OpGetRecordEmitter));
    r.insert("OpIterate", Box::new(key_ops::OpIterateEmitter));

    // Phase 06 — parallel-queue emitter family (P202 close).  Mirrors
    // phase 03's for-par emitter for the queue protocol: `n_parallel_queue`
    // / `_text` / `_ref` build a worker closure and call the
    // `n_parallel_queue_*_native` runtime fns; `n_parallel_buf_get` /
    // `_text` / `_ref` and `n_parallel_buf_drop` / `_text` / `_ref` are
    // pass-through renames to their `_native` runtime counterparts.
    r.insert("n_parallel_queue", Box::new(parallel::ParallelQueueEmitter));
    r.insert(
        "n_parallel_queue_text",
        Box::new(parallel::ParallelQueueEmitter),
    );
    r.insert(
        "n_parallel_queue_ref",
        Box::new(parallel::ParallelQueueEmitter),
    );
    for name in [
        "n_parallel_buf_get",
        "n_parallel_buf_get_text",
        "n_parallel_buf_get_ref",
        "n_parallel_buf_drop",
        "n_parallel_buf_drop_text",
        "n_parallel_buf_drop_ref",
    ] {
        r.insert(name, Box::new(parallel::ParallelBufRenameEmitter));
    }

    // Phase 10 step 10.3 — integer comparison emitter family.
    // Closes P200 read-side: the default `@v1 == @v2` template
    // produces E0308 when the LHS is a narrowed block (`(...) as u8`)
    // and the RHS is an `_i64` literal.  This emitter wraps each
    // operand in `(... as i64)` so both sides match width.
    for name in ["OpEqInt", "OpNeInt", "OpLtInt", "OpLeInt"] {
        r.insert(name, Box::new(int_compare::IntCompareEmitter));
    }

    r
}

#[cfg(test)]
mod tests {
    //! Smoke-test the `OpEmitter` trait shape so phase 00's claim that
    //! "custom emitters can opt in per Op" is checked at compile time
    //! before phase 05 lands the first real one.
    //!
    //! These tests are compile-only — they prove that:
    //! - The trait can be impl'd by an external struct.
    //! - `EmitCtx` field access works from inside an impl (split-borrow
    //!   between `ctx.output` and `ctx.w` doesn't fight the borrow
    //!   checker).
    //! - The trait dispatch (`emit_op`) accepts a `&mut EmitCtx<'_, '_>`
    //!   without lifetime gymnastics at the call site.
    //!
    //! What they do NOT cover (deferred until a real Output fixture
    //! exists in the test surface):
    //! - Calling `ctx.output.<method>` from inside an emitter.
    //! - Verifying that the registry lookup actually finds a registered
    //!   emitter (the real registry is `OnceLock`-backed; tests can't
    //!   mutate it without a feature flag).
    //!
    //! Phase 05's first real custom emitter is the integration test for
    //! the deferred parts.

    use super::{EmitCtx, OpEmitter, has_custom_emitter, registry};
    use crate::data::Value;
    use std::io;

    /// Minimal emitter that doesn't touch any context state.  Compiles
    /// only if the trait's method signature is satisfiable by an
    /// external impl (no hidden lifetime constraints).
    struct StubEmitter;
    impl OpEmitter for StubEmitter {
        fn emit(&self, _ctx: &mut EmitCtx<'_, '_>, _args: &[Value]) -> io::Result<()> {
            Ok(())
        }
    }

    /// Emitter that writes to `ctx.w`.  Compiles only if the field
    /// access pattern works without re-borrow conflicts.
    struct WriterEmitter;
    impl OpEmitter for WriterEmitter {
        fn emit(&self, ctx: &mut EmitCtx<'_, '_>, _args: &[Value]) -> io::Result<()> {
            write!(ctx.w, "/* writer-emitter stub */")
        }
    }

    /// Emitter that reads `ctx.def_fn` while writing to `ctx.w`.
    /// Compiles only if the split-borrow rules let us hold both an
    /// immutable ref to `def_fn` and a mutable ref to `w` from the
    /// same `&mut EmitCtx`.
    struct DefAccessEmitter;
    impl OpEmitter for DefAccessEmitter {
        fn emit(&self, ctx: &mut EmitCtx<'_, '_>, _args: &[Value]) -> io::Result<()> {
            let name = ctx.def_fn.name.as_str();
            write!(ctx.w, "/* {name} */")
        }
    }

    #[test]
    fn op_emitter_trait_is_impl_able() {
        // If any of the three structs above don't compile, this test
        // doesn't compile.  The body just instantiates them so the
        // structs aren't dead-code-eliminated.
        let _: &dyn OpEmitter = &StubEmitter;
        let _: &dyn OpEmitter = &WriterEmitter;
        let _: &dyn OpEmitter = &DefAccessEmitter;
    }

    /// Verify the registry is queryable and (currently) empty.  When a
    /// future phase populates it, this test's count goes up — that's
    /// expected behaviour, not a regression, but the diff makes it
    /// visible.
    #[test]
    fn registry_count_matches_sanctioned_set() {
        let count = registry().len();
        // Phase 00 ships an empty registry.  Future phases bump this.
        assert!(
            count <= 16,
            "registry has {count} custom emitters — bump the cap if \
             this is intentional and document in plan 09 status table"
        );
    }

    /// Verify `has_custom_emitter` returns false for unregistered
    /// names (today: every name).  When phase 05+ register emitters,
    /// this test's expectations narrow.
    #[test]
    fn has_custom_emitter_returns_false_for_unregistered() {
        assert!(!has_custom_emitter("ThisOpDoesNotExist"));
        assert!(!has_custom_emitter("OpAddInt")); // common Op, not custom-overridden
    }
}
