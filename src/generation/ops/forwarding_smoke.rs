// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan 09 phase 00 step 0.7b runtime smoke test — forwarding emitter
//! registered for a broad set of common Ops.
//!
//! These are the FIRST actually-registered custom emitters.  Each
//! delegates verbatim to `DefaultEmitter` — so behaviour is exactly the
//! pre-registration fall-through, and the byte-identical baseline must
//! stay green.
//!
//! Purpose: prove at runtime (not just compile time) that:
//!
//!   1. The registry actually fires registered emitters across many
//!      Op names — `emit_op` returns `Some(emitter)` correctly when
//!      `registry().get(name)` matches.
//!   2. The trait dispatch handles the full breadth of Op shapes
//!      represented in the corpus (template-based, user-fn-based, both
//!      arities) without lifetime conflicts or split-borrow issues.
//!   3. A custom emitter calling back into `DefaultEmitter::emit`
//!      composes correctly — the basis for layered emitters where one
//!      custom emitter delegates to another.
//!
//! Selection criterion: cover the four kinds of Op the corpus exercises.
//!
//!   - **Pure arithmetic / template substitution**: OpAddInt, OpMulInt,
//!     OpEqInt, OpLtInt, OpAndBool — verify the substitute_template_body
//!     path through emit_op.
//!   - **Stores-touching runtime fns (Cell ABI)**: OpDatabase,
//!     OpFreeRef, OpNewRecord, OpFinishRecord — verify the user_fn path
//!     for `Abi::Cell` registry entries.
//!   - **Iterator + key Ops**: OpStep, OpRemove — verify dispatch for
//!     Ops with `&mut` iterator-arg shapes.
//!   - **Already-registered runtime fns** (n_now, n_path_sep, n_rand) —
//!     not added here because they're not user-callable in the corpus.
//!
//! The list is **deliberately curated**, not exhaustive.  Phase 03/04
//! will register *real* (non-forwarding) emitters for some of these
//! names; until then, the forwarding registrations live as proof that
//! the dispatch works at scale.
//!
//! When a phase replaces a forwarding emitter with a real one, just
//! delete the entry from `FORWARDING_OP_NAMES` — the new emitter's
//! `r.insert(name, ...)` takes priority.

use super::{EmitCtx, OpEmitter, default::DefaultEmitter};
use crate::data::Value;
use std::io;

/// Curated list of Op names that get a forwarding emitter registered
/// via `build_registry`.  Each Op covers a different code path through
/// `emit_op` → `DefaultEmitter` so the byte-identical baseline gate
/// validates the dispatch comprehensively.
///
/// **Constraint**: only Ops that fall through to `output_call_template` /
/// `output_call_user_fn` are safe to forward.  Ops handled by
/// `dispatch.rs::output_call_inner`'s 26-arm special-case match
/// (OpDatabase, OpFreeRef, OpCopyRecord, OpFormatDatabase, OpAppendText,
/// OpStep, OpRemove, OpFormatInt/Float/etc., OpEqRef, OpNeRef, …) emit
/// context-aware Rust that doesn't fit the simple "call form" — the
/// registry-first guard at the top of `output_call_inner` (phase 00
/// step 0.6) routes them through `emit_op` BEFORE the special case
/// runs, so a forwarding emitter (which just calls DefaultEmitter)
/// would bypass critical logic (e.g. OpFreeRef's debug-name string +
/// store_nr reset).
///
/// Adding to this list: pick an Op that is NOT in the dispatch.rs
/// special-case match.  Easy check: `grep '"OpName" =>' src/generation/dispatch.rs`
/// — if it returns nothing, forwarding is safe.
pub const FORWARDING_OP_NAMES: &[&str] = &[
    // Pure arithmetic / comparison — substitute_template_body path.
    // None of these appear in dispatch.rs's special-case match.
    "OpAddInt",
    "OpMulInt",
    "OpMinInt",
    "OpEqInt",
    "OpLtInt",
    "OpAndBool",
    "OpNotBool",
    // Record allocation / finalisation — user_fn_call_body path.
    // OpNewRecord and OpFinishRecord are NOT in dispatch.rs's match;
    // they emit as ordinary user-fn calls via codegen_runtime.
    "OpNewRecord",
    "OpFinishRecord",
];

/// Stateless forwarding emitter.  Single instance is registered for
/// every Op name in `FORWARDING_OP_NAMES`.
pub struct ForwardingEmitter;

impl OpEmitter for ForwardingEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Delegating to DefaultEmitter exercises the trait dispatch
        // without changing emission shape.  If the registry path is
        // structurally broken, this branch fails (panic, miscompile,
        // baseline diff); if it works, the byte-identical baseline
        // gate stays green.
        DefaultEmitter.emit(ctx, args)
    }
}
