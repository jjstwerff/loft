// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-04 Phase 2 — single-pool, scope-blind slot allocator.
//!
//! Replaces the two-zone design in `slots.rs`.  V2 sorts live
//! intervals by `(live_start, var_nr)` and greedy-places each at
//! the lowest slot that does not conflict with any still-live
//! interval of compatible `SlotKind` and size (see `SPEC.md § 2`
//! at `doc/claude/plans/04-slot-assignment-redesign/SPEC.md`).
//!
//! The module is currently a **scaffolding stub**.  Phase 2c
//! implements the algorithm; Phase 2d transitions fixtures.
//!
//! When `LOFT_SLOT_V2=validate` is set in the environment, the
//! test harness runs V2 alongside V1 and asserts V2's output
//! satisfies invariants I1–I6 from SPEC § 5a.  Without the env
//! var, V2 is inert — V1 continues to drive codegen untouched.

// Stub scaffolding; types and functions are hooked into the
// `LOFT_SLOT_V2=validate` plumbing in Phase 2c.
#![allow(dead_code)]

use super::{Function, size};
use crate::data::{Context, Type};
use std::collections::HashMap;

/// Classification of a slot's runtime drop-opcode semantics.  Two
/// RefSlots with matching size share a drop opcode (`OpFreeRef`
/// for 12-B DbRef slots, `OpFreeText` for 24-B Text slots); Inline
/// slots have no drop opcode.  V2's placement refuses reuse
/// between incompatible kinds / sizes, matching invariant I5 in
/// `validate.rs::check_i5_kind_consistency`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Inline,
    RefSlot,
}

/// One interval fed into the V2 allocator.  One per placed local
/// or work-ref; arguments are excluded and handled by
/// `local_start`.
#[derive(Clone, Debug)]
pub struct LocalInterval {
    pub var_nr: u16,
    pub live_start: u32,
    pub live_end: u32,
    pub size: u16,
    pub kind: SlotKind,
}

/// V2's allocator output.
#[derive(Clone, Debug, Default)]
pub struct AllocatorResult {
    /// Per-variable slot assignments.  Sorted by `var_nr` for
    /// stable comparison against V1.
    pub slots: Vec<SlotAssignment>,
    /// Function-level high-water mark.  Used by codegen's
    /// function-entry `OpReserveFrame` (§ 3.1 follow-up) and as
    /// the O1 optimality metric.
    pub hwm: u16,
    /// Per-scope zone-1-style reserve, computed as
    /// `max(slot + size) - frame_base(scope)` per § 3.1.  Keeps
    /// bytecode codegen's per-block `OpReserveFrame(var_size)`
    /// emission unchanged.
    pub per_block_var_size: std::collections::HashMap<u16, u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotAssignment {
    pub var_nr: u16,
    pub slot: u16,
}

/// Map a Type to its SlotKind.  Any owned `DbRef`-handle type —
/// Reference, Vector, Hash, Sorted, Index, Spacial, Iterator, Enum
/// (struct variant), or RefVar — is `RefSlot`.  Text is also
/// `RefSlot` (size 24 B, uses `OpFreeText`); every RefSlot of size
/// 12 B shares the `OpFreeRef` drop opcode.  Everything else is
/// `Inline`.
pub fn slot_kind(tp: &Type) -> SlotKind {
    match tp {
        Type::Text(_)
        | Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Sorted(_, _, _)
        | Type::Spacial(_, _, _)
        | Type::Iterator(_, _)
        | Type::Enum(_, true, _)
        | Type::RefVar(_) => SlotKind::RefSlot,
        _ => SlotKind::Inline,
    }
}

/// V2 entry point — IR-walk-based monotonic slot assignment.
///
/// V2 walks the IR tree in the same order codegen emits bytecode.
/// At every `Set(v, value)` first-assignment, `v.slot = current_TOS`
/// and `TOS += v.size`.  At scope exit (Block/Loop close),
/// `TOS` is reset to the scope's `frame_base` — this is the point
/// where cross-scope slot reuse happens naturally, matching
/// codegen's `OpFreeStack` semantics.
///
/// **Design consequences:**
/// - No dead-slot reuse *within* a scope.  Two non-overlapping
///   Inline vars in the same block occupy distinct slots.  This
///   is a trade-off for codegen simplicity (the fixup-free
///   invariant `v.slot == codegen.TOS` at first-assignment holds
///   by construction).
/// - Cross-scope reuse is automatic via scope exit.
/// - No sort, no interval-graph colouring, no `placed` table.
/// - One pass through the IR, O(n) in the number of IR nodes.
///
/// Does not mutate `function`.  The caller applies
/// `AllocatorResult` via `apply_v2_result`.
pub fn assign_slots_v2(
    function: &Function,
    code: &crate::data::Value,
    local_start: u16,
) -> AllocatorResult {
    let mut walk = WalkState {
        slots: Vec::new(),
        assigned: vec![false; function.next_var() as usize],
        per_block_var_size: HashMap::new(),
        tos: local_start,
        hwm: local_start,
    };
    walk_node(code, function, &mut walk);
    walk.slots.sort_by_key(|s| s.var_nr);
    AllocatorResult {
        slots: walk.slots,
        hwm: walk.hwm,
        per_block_var_size: walk.per_block_var_size,
    }
}

struct WalkState {
    slots: Vec<SlotAssignment>,
    assigned: Vec<bool>,
    per_block_var_size: HashMap<u16, u16>,
    tos: u16,
    hwm: u16,
}

fn walk_node(val: &crate::data::Value, function: &Function, w: &mut WalkState) {
    use crate::data::Value;
    match val {
        Value::Block(bl) | Value::Loop(bl) => {
            let frame_base = w.tos;
            for op in &bl.operators {
                walk_node(op, function, w);
            }
            let scope_top = w.tos;
            let reserve = scope_top.saturating_sub(frame_base);
            w.per_block_var_size.insert(bl.scope, reserve);
            // Exit scope: reset TOS.  Matches codegen's OpFreeStack.
            w.tos = frame_base;
        }
        Value::Set(v_nr, inner) => {
            // Evaluate inner first (codegen does this: RHS is
            // built on the eval stack before the Set commits).
            walk_node(inner, function, w);
            let v = *v_nr as usize;
            // Skip arguments, zero-sized, already-placed, no-first-def vars.
            if v >= function.next_var() as usize
                || function.is_argument(*v_nr)
                || w.assigned[v]
            {
                return;
            }
            let fd = function.first_def(*v_nr);
            if fd == u32::MAX {
                return;
            }
            let sz = size(function.tp(*v_nr), &Context::Variable);
            if sz == 0 {
                return;
            }
            w.slots.push(SlotAssignment {
                var_nr: *v_nr,
                slot: w.tos,
            });
            w.assigned[v] = true;
            w.tos += sz;
            w.hwm = w.hwm.max(w.tos);
        }
        Value::If(cond, t, f) => {
            walk_node(cond, function, w);
            // Both branches start at the same TOS; treat each as
            // a temporary scope.  After the If, TOS returns to
            // pre-branch position (codegen reconciles via
            // OpFreeStack / join-point handling).
            let branch_base = w.tos;
            walk_node(t, function, w);
            let t_tos = w.tos;
            w.tos = branch_base;
            walk_node(f, function, w);
            w.hwm = w.hwm.max(t_tos);
        }
        Value::Insert(ops) => {
            for op in ops {
                walk_node(op, function, w);
            }
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                walk_node(a, function, w);
            }
        }
        Value::Drop(inner) | Value::Return(inner) => {
            walk_node(inner, function, w);
        }
        Value::Iter(_, c, n, e) => {
            walk_node(c, function, w);
            walk_node(n, function, w);
            walk_node(e, function, w);
        }
        _ => {}
    }
}

// No synthetic Rust-level unit tests here.  V2 correctness is
// verified against real `.loft` programs:
//
//   - `tests/scripts/96-slot-assign.loft` — patterns historically
//     prone to slot-assignment bugs; executed by `loft_suite` in
//     `tests/wrap.rs`.
//   - `tests/slot_v2_baseline.rs` — 29 Phase-0 fixtures, each a
//     `code!("fn test() { … }")` block exercising one shape
//     (P178, loop-carry, block-return, sibling-scope reuse, …).
//   - `tests/issues.rs::p178_is_capture_slot_alias` — P178.
//   - `tests/issues.rs::p185_slot_alias_on_late_local_in_nested_for`
//     — P185 (un-ignored in Phase 2h.5).
//
// A slot-inspection suite can live in a separate integration test
// that loads specific `.loft` files from `tests/scripts/` and
// asserts structural slot properties — it never hand-constructs
// `Function` objects, it points to the `.loft` file + function
// name that holds the problem.

/// Returns true when the runtime environment has opted into V2
/// shadow validation via `LOFT_SLOT_V2=validate` (or `report`,
/// `drive`).  The test harness and `assign_slots`' callers check
/// this to decide whether to run V2 alongside V1.
#[allow(dead_code)]
pub fn v2_validate_enabled() -> bool {
    match std::env::var("LOFT_SLOT_V2") {
        Ok(val) => val == "validate" || val == "drive" || val == "report",
        Err(_) => false,
    }
}

/// Apply an `AllocatorResult` to the function: set each variable's
/// `stack_pos`, and populate `var_size` on every Block node in the
/// IR tree so codegen's existing `OpReserveFrame(var_size)` call
/// per block (`src/state/codegen.rs:1896`) reserves the right
/// amount of space.  Loop nodes keep `var_size = 0` — their
/// internal vars are placed at TOS one-by-one during codegen's
/// walk and the loop wrapper does not pre-reserve.
pub fn apply_v2_result(
    function: &mut Function,
    code: &mut crate::data::Value,
    result: &AllocatorResult,
) {
    for s in &result.slots {
        function.set_stack_pos(s.var_nr, s.slot);
    }
    apply_var_size(code, &result.per_block_var_size);
}

fn apply_var_size(val: &mut crate::data::Value, sizes: &HashMap<u16, u16>) {
    use crate::data::Value;
    match val {
        Value::Block(bl) => {
            bl.var_size = sizes.get(&bl.scope).copied().unwrap_or(0);
            for op in &mut bl.operators {
                apply_var_size(op, sizes);
            }
        }
        Value::Loop(lp) => {
            lp.var_size = 0;
            for op in &mut lp.operators {
                apply_var_size(op, sizes);
            }
        }
        Value::If(c, t, f) => {
            apply_var_size(c, sizes);
            apply_var_size(t, sizes);
            apply_var_size(f, sizes);
        }
        Value::Set(_, inner) | Value::Drop(inner) | Value::Return(inner) => {
            apply_var_size(inner, sizes);
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                apply_var_size(a, sizes);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                apply_var_size(op, sizes);
            }
        }
        Value::Iter(_, c, n, e) => {
            apply_var_size(c, sizes);
            apply_var_size(n, sizes);
            apply_var_size(e, sizes);
        }
        _ => {}
    }
}

/// Returns true when the runtime environment requests per-function
/// hwm reporting via `LOFT_SLOT_V2=report`.  Reports V1's and V2's
/// hwm for every compiled function so the corpus-wide optimality
/// delta (SPEC § 5a invariant O1) can be aggregated by the caller.
#[allow(dead_code)]
pub fn v2_report_enabled() -> bool {
    matches!(std::env::var("LOFT_SLOT_V2"), Ok(val) if val == "report")
}
