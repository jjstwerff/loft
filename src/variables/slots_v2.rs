// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-04 Phase 2 — single-pool, scope-blind slot allocator.
//!
//! Replaces the two-zone design in `slots.rs`.  V2 sorts live
//! intervals by `(live_start, var_nr)` and greedy-places each at
//! the lowest slot that does not conflict with any still-live
//! interval of compatible `SlotKind` and size (see `SPEC.md § 2`
//! at `doc/claude/plans/finished/04-slot-assignment-redesign/SPEC.md`).
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
    _code: &crate::data::Value,
    local_start: u16,
) -> AllocatorResult {
    // @PLAN53 cluster 2 — scope-blind, kind-aware, ALIGNED interval-graph
    // greedy allocator (the SPEC § 2 design).  Replaces the earlier
    // TOS-reset IR-walk, which reused slots across sibling scopes WITHOUT
    // a kind check and so violated I5 (an int and a ref sharing a slot have
    // different drop opcodes).  Each local is placed at the lowest
    // `align(tp)`-aligned offset ≥ `local_start` that does NOT conflict with
    // any already-placed local, where conflict = overlapping slot range AND
    // (overlapping live interval OR incompatible `SlotKind`/size).  Tight
    // sizes; alignment via padding (the lowest-slot search backfills holes).
    struct Iv {
        var_nr: u16,
        ls: u32,
        le: u32,
        size: u16,
        align: u16,
        kind: SlotKind,
        slot: u16,
    }
    let mut ivs: Vec<Iv> = Vec::new();
    for v in 0..function.next_var() {
        if function.is_argument(v) {
            continue;
        }
        let fd = function.first_def(v);
        if fd == u32::MAX {
            continue;
        }
        let tp = function.tp(v);
        let sz = size(tp, &Context::Variable);
        if sz == 0 {
            continue;
        }
        ivs.push(Iv {
            var_nr: v,
            ls: fd,
            le: function.variables[v as usize].last_use,
            size: sz,
            align: u16::from(super::align(tp)).max(1),
            kind: slot_kind(tp),
            slot: 0,
        });
    }
    // PINNED: argument-typed vars (incl. hidden SRet return buffers) already
    // own a fixed slot set by the caller / V1 — V2 must NOT place a local on
    // top of them.  Treat each as an always-live ([0, MAX]) fixed interval the
    // greedy search avoids.  (@PLAN53 cluster 2: without this, V2 placed
    // locals over SRet args' slots → spurious I5/I6.)
    let pinned: Vec<Iv> = (0..function.next_var())
        .filter(|&v| function.is_argument(v) && function.variables[v as usize].stack_pos != u16::MAX)
        .map(|v| {
            let tp = function.tp(v);
            Iv {
                var_nr: v,
                ls: 0,
                le: u32::MAX,
                size: size(tp, &Context::Variable),
                align: u16::from(super::align(tp)).max(1),
                kind: slot_kind(tp),
                slot: function.variables[v as usize].stack_pos,
            }
        })
        .collect();
    // Greedy by live-start, then var_nr for determinism.
    ivs.sort_by_key(|i| (i.ls, i.var_nr));
    // Loop scopes (I6): two vars may share a slot only if, for every loop,
    // they are BOTH inside `[s,e)` or BOTH outside — never straddling (a
    // loop-local recurs each iteration, so it conflicts with anything live
    // across the loop boundary).
    let loop_ranges: Vec<(u32, u32)> = {
        let mut seen = std::collections::BTreeSet::new();
        let mut r = Vec::new();
        for v in 0..function.next_var() {
            let sc = function.variables[v as usize].scope;
            if sc != u16::MAX
                && function.is_loop_scope(sc)
                && seen.insert(sc)
                && let Some(range) = function.loop_seq_range(sc)
            {
                r.push(range);
            }
        }
        r
    };
    let mut hwm = local_start;
    for idx in 0..ivs.len() {
        let (sz, al, ls, le, kind) = {
            let i = &ivs[idx];
            (i.size, i.align, i.ls, i.le, i.kind)
        };
        let mut pos = local_start.next_multiple_of(al);
        'search: loop {
            let end = pos + sz;
            for p in pinned.iter().chain(ivs[..idx].iter()) {
                let overlap_range = pos < p.slot + p.size && p.slot < end;
                if !overlap_range {
                    continue;
                }
                // Reuse is only valid as an EXACT slot match (same start+size);
                // any PARTIAL overlap is always a conflict (it would alias only
                // part of a live value).
                let exact = pos == p.slot && sz == p.size;
                let life_overlap = ls <= p.le && p.ls <= le;
                let incompatible = p.kind != kind || p.size != sz;
                // I6: forbid sharing that straddles any loop scope.
                let straddle = loop_ranges.iter().any(|&(s, e)| {
                    let iv_in = ls >= s && le < e;
                    let iv_out = le < s || ls >= e;
                    let p_in = p.ls >= s && p.le < e;
                    let p_out = p.le < s || p.ls >= e;
                    !((iv_in && p_in) || (iv_out && p_out))
                });
                if !exact || life_overlap || incompatible || straddle {
                    // Jump past the blocker, re-align, retry.
                    pos = (p.slot + p.size).next_multiple_of(al);
                    continue 'search;
                }
            }
            break;
        }
        ivs[idx].slot = pos;
        hwm = hwm.max(pos + sz);
    }
    let mut slots: Vec<SlotAssignment> = ivs
        .iter()
        .map(|i| SlotAssignment {
            var_nr: i.var_nr,
            slot: i.slot,
        })
        .collect();
    slots.sort_by_key(|s| s.var_nr);
    // Scope-blind: one function-entry reserve (frame_hwm) covers every slot;
    // no per-block reserves.
    AllocatorResult {
        slots,
        hwm,
        per_block_var_size: HashMap::new(),
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
            if v >= function.next_var() as usize || function.is_argument(*v_nr) || w.assigned[v] {
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
            // @PLAN53 cluster 2: align the slot to its type's natural alignment
            // so an `addr`/`addr_mut::<T>` into it is sound.  Sizes stay tight;
            // alignment is padding — the bumped TOS leaves a hole that V2's
            // lowest-slot search backfills with a smaller var.
            let al = u16::from(super::align(function.tp(*v_nr))).max(1);
            w.tos = w.tos.next_multiple_of(al);
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
        Value::Span(b) => walk_node(&b.1, function, w),
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
//     (is-capture aliasing, loop-carry, block-return, sibling-scope
//     reuse, late-local-in-nested-for, …).
//   - `tests/issues.rs::p178_is_capture_slot_alias`
//   - `tests/issues.rs::p185_slot_alias_on_late_local_in_nested_for`
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
        Ok(val) => {
            let mode = val.split(':').next().unwrap_or("");
            mode == "validate" || mode == "drive" || mode == "report"
        }
        Err(_) => false,
    }
}

/// Per-function V2 switch (@PLAN53 cluster 2).  Parses `LOFT_SLOT_V2` as
/// `mode` or `mode:filter` and returns the mode (`"validate"` / `"report"`
/// / `"drive"`) **iff it applies to `fn_name`** — no filter means all
/// functions; a `:filter` suffix restricts to functions whose name
/// contains `filter`.  This is the per-function command-line switch: e.g.
/// `LOFT_SLOT_V2=drive:build_chunk` drives only that function onto V2 while
/// every other function keeps V1.
pub fn v2_mode_for(fn_name: &str) -> Option<String> {
    let raw = std::env::var("LOFT_SLOT_V2").ok()?;
    let (mode, filter) = match raw.split_once(':') {
        Some((m, f)) => (m, Some(f)),
        None => (raw.as_str(), None),
    };
    if !matches!(mode, "validate" | "report" | "drive") {
        return None;
    }
    match filter {
        Some(f) if !fn_name.contains(f) => None,
        _ => Some(mode.to_string()),
    }
}

/// @PLAN53 cluster 2 — side-by-side dump of V1 (original, authoritative) vs
/// the aligned V2 slot assignment, for debugging the aligned allocator.
/// Triggered by `report` mode.  Shows per-local size, alignment, and both
/// slot offsets so alignment shifts + gap-fill are visible at a glance.
pub fn dump_v1_v2_slots(v1: &Function, v2: &AllocatorResult, d_nr: u32) {
    eprintln!(
        "[slot-cmp] d_nr={d_nr} fn='{}'  (V2 hwm={})",
        v1.name, v2.hwm
    );
    eprintln!(
        "  {:<24} {:>4} {:>4} {:>8} {:>8}",
        "name", "sz", "al", "V1", "V2"
    );
    for (idx, var) in v1.variables.iter().enumerate() {
        if var.argument || var.first_def == u32::MAX {
            continue;
        }
        let sz = size(&var.type_def, &Context::Variable);
        let al = super::align(&var.type_def);
        let v1s = if var.stack_pos == u16::MAX {
            "-".to_string()
        } else {
            var.stack_pos.to_string()
        };
        let v2s = v2
            .slots
            .iter()
            .find(|s| s.var_nr == idx as u16)
            .map_or_else(|| "-".to_string(), |s| s.slot.to_string());
        eprintln!("  {:<24} {sz:>4} {al:>4} {v1s:>8} {v2s:>8}", var.name);
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
    // Plan-22 02d-vii follow-up — `LOFT_LOG=slots:<fn>` trace.
    // Logs each slot decision + WHY any var was skipped, BEFORE
    // applying the result.  Helps diagnose "Incorrect var X[65535]"
    // codegen panics where the allocator silently skipped X.
    if let Some(target) = crate::log_config::slots_trace_target()
        && function.name.contains(&target)
    {
        eprintln!("[slots] fn={} hwm={}", function.name, result.hwm);
        let assigned: std::collections::HashSet<u16> =
            result.slots.iter().map(|s| s.var_nr).collect();
        for v_nr in 0..function.next_var() {
            let name = function.name(v_nr);
            let tp = function.tp(v_nr);
            if let Some(s) = result.slots.iter().find(|s| s.var_nr == v_nr) {
                eprintln!(
                    "[slots]   ASSIGN  v_nr={v_nr:<3} name={name:<20} slot={slot:<4} type={tp:?}",
                    slot = s.slot,
                );
            } else if !assigned.contains(&v_nr) {
                let reason = slot_skip_reason(function, v_nr);
                eprintln!(
                    "[slots]   SKIP    v_nr={v_nr:<3} name={name:<20} type={tp:?}  reason={reason}",
                );
            }
        }
    }
    for s in &result.slots {
        function.set_stack_pos(s.var_nr, s.slot);
    }
    apply_var_size(code, &result.per_block_var_size);
}

/// Plan-22 02d-vii — explain why `assign_slots_v2` skipped a var.
/// Mirrors the bail-out conditions in `walk_node`'s `Set` arm.
fn slot_skip_reason(function: &Function, v_nr: u16) -> &'static str {
    if (v_nr as usize) >= function.next_var() as usize {
        return "v_nr out of range";
    }
    if function.is_argument(v_nr) {
        return "is_argument (allocated externally by caller)";
    }
    let tp = function.tp(v_nr);
    let sz = crate::variables::size(tp, &crate::variables::Context::Variable);
    if sz == 0 {
        return "size(type) == 0 (zero-byte var)";
    }
    if function.first_def(v_nr) == u32::MAX {
        return "no first_def (no Set IR for this var — read-only or unused)";
    }
    "no Set IR walked (var referenced but never assigned via Value::Set)"
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
        Value::Span(b) => apply_var_size(&mut b.1, sizes),
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
