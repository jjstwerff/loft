// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Stack slot assignment: two-zone block pre-claim design.
//!
//! ## Frame layout
//!
//! ```text
//! ┌──────────────────┐  ← frame base (args_base)
//! │  arguments       │  parameters in declaration order
//! │  return address   │  4 bytes
//! ├──────────────────┤  ← local_start
//! │  zone 1: small   │  ≤16-byte types (int, float, bool, char, DbRef)
//! │                  │  pre-claimed per block; greedy interval colouring
//! ├──────────────────┤
//! │  zone 2: large   │  >16-byte types (text=24B String, tuples, etc.)
//! │                  │  sequential allocation order
//! └──────────────────┘  ← stack_pos (TOS)
//! ```
//!
//! Zone 1 variables are packed densely — dead variables' slots are reused
//! within the same scope.  Zone 2 variables are never reused because their
//! large size makes overlap-checking expensive and the savings minimal.

#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

use crate::data::{Context, Type, Value};

use super::{Function, size};

pub fn assign_slots(function: &mut Function, code: &mut Value, local_start: u16) {
    // Enable slot-assignment logging when LOFT_ASSIGN_LOG=<name> matches function name.
    #[cfg(debug_assertions)]
    if let Ok(filter) = std::env::var("LOFT_ASSIGN_LOG")
        && (filter == "*" || function.name.contains(&*filter))
    {
        function.logging = true;
        eprintln!(
            "[assign_slots] === {} ===  local_start={local_start}",
            function.name
        );
    }
    // Reset all non-argument variable slots.
    for v in &mut function.variables {
        if !v.argument {
            v.stack_pos = u16::MAX;
            v.pre_assigned_pos = u16::MAX;
        }
    }
    // Walk the IR tree, assigning slots scope-by-scope.
    process_scope(function, code, local_start, 0);
    // Place any variables that the IR walk missed (scope has no Block/Loop in IR).
    place_orphaned_vars(function);
    #[cfg(debug_assertions)]
    {
        function.logging = false;
    }
}

/// Assign slots for all variables in the scope owned by `block_val` (a Block or Loop node),
/// then recurse into child scopes.
#[allow(clippy::too_many_lines)]
fn process_scope(function: &mut Function, block_val: &mut Value, frame_base: u16, depth: u32) {
    assert!(
        depth <= 1000,
        "assign_slots scope nesting limit exceeded at depth {depth}"
    );
    let bl_scope = match block_val {
        Value::Block(bl) | Value::Loop(bl) => bl.scope,
        _ => return,
    };

    // ── Zone 1: colour small variables (size ≤ 8) ─────────────────────────────
    let mut small_vars: Vec<usize> = function
        .variables
        .iter()
        .enumerate()
        .filter(|(_, v)| {
            !v.argument && v.scope == bl_scope && v.first_def != u32::MAX && {
                let s = size(&v.type_def, &Context::Variable);
                s > 0 && s <= 8
            }
        })
        .map(|(i, _)| i)
        .collect();
    small_vars.sort_by_key(|&i| function.variables[i].first_def);

    if function.logging {
        eprintln!(
            "[assign_slots] process_scope  scope={bl_scope}  frame_base={frame_base}  \
             zone1_vars=[{}]",
            small_vars
                .iter()
                .map(|&i| format!(
                    "{}({}B,fd={},lu={})",
                    function.variables[i].name,
                    size(&function.variables[i].type_def, &Context::Variable),
                    function.variables[i].first_def,
                    function.variables[i].last_use
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let mut zone1_hwm: u16 = frame_base;
    for &i in &small_vars {
        let v_size = size(&function.variables[i].type_def, &Context::Variable);
        let first_def = function.variables[i].first_def;
        let last_use = function.variables[i].last_use;

        let mut candidate = frame_base;
        let mut retry_count = 0u32;
        'retry: loop {
            assert!(
                retry_count <= 10_000,
                "assign_slots: greedy coloring loop exceeded 10000 iterations for variable '{}' \
                 (size={v_size}, scope={bl_scope}, candidate={candidate}). \
                 Infinite loop in slot search — check for conflicting variables that prevent placement.",
                function.variables[i].name
            );
            retry_count += 1;
            let end = candidate + v_size;
            for &j in &small_vars {
                if j == i {
                    continue;
                }
                let js = function.variables[j].stack_pos;
                if js == u16::MAX {
                    continue;
                }
                let j_size = size(&function.variables[j].type_def, &Context::Variable);
                if candidate < js + j_size && end > js {
                    let jf = function.variables[j].first_def;
                    let jl = function.variables[j].last_use;
                    if first_def <= jl && last_use >= jf {
                        // Live-interval overlap: try next slot.
                        candidate = js + j_size;
                        continue 'retry;
                    }
                    // Dead slot: only reuse if sizes match (avoids displacement errors).
                    if v_size != j_size {
                        candidate = js + j_size;
                        continue 'retry;
                    }
                }
            }
            break;
        }
        function.variables[i].stack_pos = candidate;
        function.variables[i].pre_assigned_pos = candidate;
        zone1_hwm = zone1_hwm.max(candidate + v_size);
        if function.logging {
            eprintln!(
                "[assign_slots]   zone1  '{}' scope={bl_scope} size={v_size}B → slot={candidate}  \
                 live=[{first_def},{last_use}]",
                function.variables[i].name
            );
        }
    }
    let zone1_size = zone1_hwm - frame_base;

    // Store var_size (zone1 bytes) in the Block node so generate_block can emit OpReserveFrame.
    if let Value::Block(bl) | Value::Loop(bl) = block_val {
        bl.var_size = zone1_size;
    }

    // ── Zone 2: place large variables and recurse into child scopes ────────────
    // tos tracks the physical TOS after zone1 is pre-claimed.
    let mut tos = frame_base + zone1_size;
    if function.logging {
        eprintln!("[assign_slots]   zone1_size={zone1_size}  zone2_tos_start={tos}");
    }

    let operators = match block_val {
        Value::Block(bl) | Value::Loop(bl) => &mut bl.operators,
        _ => return,
    };

    for op in operators.iter_mut() {
        place_large_and_recurse(function, op, bl_scope, &mut tos, depth);
    }
}

/// Place large (> 8B) variables at TOS in IR-walk order, recurse into child scopes.
fn place_large_and_recurse(
    function: &mut Function,
    val: &mut Value,
    scope: u16,
    tos: &mut u16,
    depth: u32,
) {
    assert!(
        depth <= 1000,
        "assign_slots nesting limit exceeded at depth {depth}"
    );
    match val {
        Value::Set(v_nr, inner) => {
            let v = *v_nr as usize;
            if function.variables[v].scope == scope && function.variables[v].stack_pos == u16::MAX {
                let v_size = size(&function.variables[v].type_def, &Context::Variable);
                if v_size > 8 {
                    // Block-return pattern: Set(v, Block([..., Var(inner_result)])).
                    // For non-Text types, generate_block is called with `to = v.stack_pos`,
                    // so at runtime the block's frame starts at v's slot (v is not yet live).
                    // Text is excluded: gen_set_first_text emits OpText BEFORE the block runs.
                    if matches!(inner.as_ref(), Value::Block(_))
                        && !matches!(function.variables[v].type_def, Type::Text(_))
                    {
                        let v_slot = place_zone2(function, v, scope, tos);
                        process_scope(function, inner, v_slot, depth + 1);
                        return;
                    }
                    // Non-Block inner with child scopes (e.g. Call(fn, [Block(...)])):
                    // codegen evaluates inner (including Block args) BEFORE placing v.
                    // Process inner first so child scopes see the correct tos.
                    // Text excluded: OpText emitted before inner evaluation.
                    if !matches!(function.variables[v].type_def, Type::Text(_))
                        && inner_has_pre_assignments(inner)
                    {
                        place_large_and_recurse(function, inner, scope, tos, depth + 1);
                        place_zone2(function, v, scope, tos);
                        return;
                    }
                    // Simple inner: place v first, then recurse.
                    place_zone2(function, v, scope, tos);
                }
            } else if function.logging && function.variables[v].scope != scope {
                eprintln!(
                    "[assign_slots]   zone2  skip '{}' (scope={}, not {scope})",
                    function.variables[v].name, function.variables[v].scope
                );
            }
            place_large_and_recurse(function, inner, scope, tos, depth + 1);
        }
        Value::Block(_) => {
            let child_base = *tos;
            process_scope(function, val, child_base, depth + 1);
            // Child cleans up with its own OpFreeStack; tos unchanged after child exits.
        }
        Value::Loop(_) => {
            let child_base = *tos;
            process_scope(function, val, child_base, depth + 1);
        }
        Value::If(cond, then_val, else_val) => {
            place_large_and_recurse(function, cond, scope, tos, depth + 1);
            let branch_tos = *tos;
            if matches!(then_val.as_ref(), Value::Block(_)) {
                process_scope(function, then_val, branch_tos, depth + 1);
            } else {
                place_large_and_recurse(function, then_val, scope, tos, depth + 1);
                *tos = branch_tos;
            }
            if matches!(else_val.as_ref(), Value::Block(_)) {
                process_scope(function, else_val, branch_tos, depth + 1);
            } else {
                place_large_and_recurse(function, else_val, scope, tos, depth + 1);
            }
            *tos = branch_tos;
        }
        Value::Insert(ops) => {
            for op in ops {
                place_large_and_recurse(function, op, scope, tos, depth + 1);
            }
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                place_large_and_recurse(function, a, scope, tos, depth + 1);
            }
        }
        Value::Drop(inner) | Value::Return(inner) => {
            place_large_and_recurse(function, inner, scope, tos, depth + 1);
        }
        _ => {}
    }
}

/// C43.1: find a dead zone-2 variable whose slot can be reused by variable `v`.
/// Returns `Some(slot)` if a conflict-free candidate exists, `None` otherwise.
/// Guards: same size, same type discriminant, dead (`last_use` < `first_def`),
/// no spatial+temporal overlap with any other assigned variable.
/// Place a zone-2 variable at tos, trying dead-slot reuse first.
fn place_zone2(function: &mut Function, v: usize, scope: u16, tos: &mut u16) -> u16 {
    let v_size = size(&function.variables[v].type_def, &Context::Variable);
    let reuse_slot = find_reusable_zone2_slot(function, v, scope);
    let v_slot = if let Some(slot) = reuse_slot {
        slot
    } else {
        let s = *tos;
        *tos += v_size;
        s
    };
    if function.logging {
        eprintln!(
            "[assign_slots]   zone2  '{}' scope={scope} size={v_size}B → slot={v_slot}",
            function.variables[v].name,
        );
    }
    function.variables[v].stack_pos = v_slot;
    function.variables[v].pre_assigned_pos = v_slot;
    v_slot
}

/// Check whether a Value tree contains nodes that codegen will evaluate
/// before the enclosing Set's target variable — child scopes (Block/Loop)
/// or Set nodes for other variables (e.g. __lift_N in Insert preambles).
fn inner_has_pre_assignments(val: &Value) -> bool {
    match val {
        Value::Block(_) | Value::Loop(_) => true,
        Value::Set(_, _) => true, // inner Set is evaluated before the outer Set's target
        Value::Call(_, args) | Value::CallRef(_, args) => args.iter().any(inner_has_pre_assignments),
        Value::If(c, t, e) => inner_has_pre_assignments(c) || inner_has_pre_assignments(t) || inner_has_pre_assignments(e),
        Value::Insert(ops) => ops.iter().any(inner_has_pre_assignments),
        Value::Drop(inner) | Value::Return(inner) => inner_has_pre_assignments(inner),
        _ => false,
    }
}

/// Place variables that the IR walk missed (scope has no Block/Loop in IR).
fn place_orphaned_vars(function: &mut Function) {
    let orphans: Vec<usize> = function
        .variables
        .iter()
        .enumerate()
        .filter(|(_, v)| {
            !v.argument && v.first_def != u32::MAX && v.stack_pos == u16::MAX
                && size(&v.type_def, &Context::Variable) > 0
        })
        .map(|(i, _)| i)
        .collect();
    for &i in &orphans {
        let v_size = size(&function.variables[i].type_def, &Context::Variable);
        let v_first = function.variables[i].first_def;
        let v_last = function.variables[i].last_use;
        let mut candidate = 0u16;
        loop {
            let end = candidate + v_size;
            let conflict = function.variables.iter().enumerate().any(|(j, jv)| {
                if j == i || jv.stack_pos == u16::MAX { return false; }
                let js = jv.stack_pos;
                let je = js + size(&jv.type_def, &Context::Variable);
                candidate < je && end > js && v_first <= jv.last_use && v_last >= jv.first_def
            });
            if !conflict { break; }
            candidate += 1;
        }
        if function.logging {
            eprintln!(
                "[assign_slots]   orphan '{}' scope={} size={v_size}B → slot={candidate}",
                function.variables[i].name, function.variables[i].scope,
            );
        }
        function.variables[i].stack_pos = candidate;
        function.variables[i].pre_assigned_pos = candidate;
    }
}

fn find_reusable_zone2_slot(function: &Function, v: usize, scope: u16) -> Option<u16> {
    // C43: only reuse Text-to-Text slots.  Other zone-2 types (Reference, Vector)
    // have complex interactions with IR-walk-order placement that cause partial
    // overlaps.  Text is safe because gen_set_first_text emits OpText before the
    // block runs, so there's no block-return frame-sharing to worry about.
    if !matches!(function.variables[v].type_def, Type::Text(_)) {
        return None;
    }
    let v_size = size(&function.variables[v].type_def, &Context::Variable);
    let v_first = function.variables[v].first_def;
    let v_last = function.variables[v].last_use;
    let v_disc = std::mem::discriminant(&function.variables[v].type_def);
    for (j, jv) in function.variables.iter().enumerate() {
        if j == v || jv.stack_pos == u16::MAX || jv.scope != scope {
            continue;
        }
        let j_size = size(&jv.type_def, &Context::Variable);
        if j_size != v_size || std::mem::discriminant(&jv.type_def) != v_disc {
            continue;
        }
        if jv.last_use >= v_first {
            continue;
        }
        let slot = jv.stack_pos;
        let conflict = function.variables.iter().enumerate().any(|(k, kv)| {
            if k == v || k == j || kv.stack_pos == u16::MAX {
                return false;
            }
            let ks = kv.stack_pos;
            let ke = ks + size(&kv.type_def, &Context::Variable);
            let spatial = slot < ke && ks < slot + v_size;
            let temporal = v_first <= kv.last_use && v_last >= kv.first_def;
            spatial && temporal
        });
        if !conflict {
            return Some(slot);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Block;
    use crate::keys::DbRef;
    use crate::variables::intervals::compute_intervals;
    use crate::variables::validate::find_conflict;
    use std::collections::HashMap;
    use std::mem::size_of;

    // ── helpers ──────────────────────────────────────────────────────────────

    const INT: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32, false);

    /// Wrap `assign_slots` for unit tests: builds a minimal flat Block (scope 0) with
    /// `Value::Set` nodes for every non-argument large (>8 B) variable so Zone 2 can
    /// place them.  Small variables (≤ 8 B) are handled by Zone 1 without needing IR nodes.
    fn run_assign_slots(f: &mut Function, local_start: u16) {
        let large_sets: Vec<Value> = f
            .variables
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                !v.argument && v.first_def != u32::MAX && size(&v.type_def, &Context::Variable) > 8
            })
            .map(|(i, _)| Value::Set(i as u16, Box::new(Value::Null)))
            .collect();
        let mut code = Value::Block(Box::new(Block {
            name: "",
            operators: large_sets,
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));
        assign_slots(f, &mut code, local_start);
    }

    /// Variant of `run_assign_slots` for the multi-scope sequential-for-loops test.
    /// Builds a nested Block tree matching the scope hierarchy supplied by the caller.
    /// `scope_tree`: list of `(scope, parent_scope, is_loop)` entries.
    /// Large vars in each scope are placed as Set nodes in that scope's block.
    fn run_assign_slots_scoped(
        f: &mut Function,
        local_start: u16,
        root_scope: u16,
        // (child_scope, parent_scope, is_loop)
        child_scopes: &[(u16, u16, bool)],
    ) {
        // Build the nested Value tree bottom-up.
        // Maps scope → Vec<Value> of operators for that scope's block.
        let mut operators: HashMap<u16, Vec<Value>> = HashMap::new();

        // Seed with large-var Set nodes per scope.
        for (i, v) in f.variables.iter().enumerate() {
            if v.argument || v.first_def == u32::MAX {
                continue;
            }
            if size(&v.type_def, &Context::Variable) > 8 {
                operators
                    .entry(v.scope)
                    .or_default()
                    .push(Value::Set(i as u16, Box::new(Value::Null)));
            }
        }

        // Insert child blocks into their parent's operator list, innermost first.
        // Process in reverse order so deeper scopes are nested before shallower ones.
        for &(child, parent, is_loop) in child_scopes.iter().rev() {
            let ops = operators.remove(&child).unwrap_or_default();
            let child_block = if is_loop {
                Value::Loop(Box::new(Block {
                    name: "",
                    operators: ops,
                    result: Type::Void,
                    scope: child,
                    var_size: 0,
                }))
            } else {
                Value::Block(Box::new(Block {
                    name: "",
                    operators: ops,
                    result: Type::Void,
                    scope: child,
                    var_size: 0,
                }))
            };
            operators.entry(parent).or_default().push(child_block);
        }

        let root_ops = operators.remove(&root_scope).unwrap_or_default();
        let mut code = Value::Block(Box::new(Block {
            name: "",
            operators: root_ops,
            result: Type::Void,
            scope: root_scope,
            var_size: 0,
        }));
        assign_slots(f, &mut code, local_start);
    }

    /// Add a variable with an already-known slot and live interval.
    fn add_var(f: &mut Function, tp: &Type, slot: u16, first_def: u32, last_use: u32) -> u16 {
        let v = f.add_unique("v", tp, 0);
        f.variables[v as usize].stack_pos = slot;
        f.variables[v as usize].first_def = first_def;
        f.variables[v as usize].last_use = last_use;
        v
    }

    /// Add a variable for `assign_slots` tests: named, scoped, with a live interval
    /// but no pre-assigned slot.  The scope is recorded on the variable; call
    /// `declare_loop` separately if the scope is a loop scope.
    fn add_scoped_var(
        f: &mut Function,
        name: &str,
        tp: &Type,
        scope: u16,
        first_def: u32,
        last_use: u32,
    ) -> u16 {
        let v = f.add_unique(name, tp, scope);
        f.variables[v as usize].scope = scope;
        f.variables[v as usize].first_def = first_def;
        f.variables[v as usize].last_use = last_use;
        v
    }

    /// Mark `scope` as a loop scope and record its seq-number range [`seq_start`, `seq_end`).
    /// Must be called before `assign_slots` runs for the loop scope to influence
    /// `tos_estimate`.
    fn declare_loop(f: &mut Function, scope: u16, seq_start: u32, seq_end: u32) {
        f.mark_loop_scope(scope);
        f.record_loop_range(scope, seq_start, seq_end);
    }

    // ── find_conflict unit tests ──────────────────────────────────────────────

    /// Slot reuse is fine when the two live intervals are strictly sequential.
    #[test]
    fn no_conflict_sequential_slot_reuse() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 10); // dies at seq 10
        add_var(&mut f, &INT, 0, 11, 20); // born at seq 11 — no overlap
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    /// Variables that are simultaneously alive but occupy adjacent, non-overlapping slots are fine.
    #[test]
    fn no_conflict_adjacent_slots() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 20); // slot [0, 4)
        add_var(&mut f, &INT, 4, 0, 20); // slot [4, 8)
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    /// Two variables at the exact same slot that are alive at the same time must be flagged.
    #[test]
    fn conflict_identical_slot_and_overlapping_interval() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 10);
        add_var(&mut f, &INT, 0, 5, 15); // overlaps both in slot and time
        assert!(find_conflict(&f.variables, &HashMap::new()).is_some());
    }

    /// Reproduces the `res`/`_elm_1` pattern from the real bug:
    /// a 4-byte variable at slot 4 stays alive while a 12-byte `DbRef` is later placed at
    /// slot 0 — its range [0, 12) swallows the 4-byte var's slot [4, 8).
    #[test]
    fn conflict_small_var_inside_wider_db_ref_slot() {
        let mut f = Function::new("f", "test");
        let ref_tp = Type::Reference(0, vec![]); // size_of::<DbRef>() bytes
        add_var(&mut f, &INT, 4, 0, 100); // long-lived int at slot [4, 8)
        add_var(&mut f, &ref_tp, 0, 50, 80); // DbRef at slot [0, 12), alive [50, 80]
        // Both are alive at e.g., seq 50..80, and [0,12) overlaps [4,8).
        assert!(find_conflict(&f.variables, &HashMap::new()).is_some());
    }

    /// A variable with no assigned slot (`stack_pos == u16::MAX`) must never trigger a conflict.
    #[test]
    fn no_conflict_unassigned_slot() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 20);
        let v = f.add_unique("y", &INT, 0); // stack_pos stays u16::MAX
        f.variables[v as usize].first_def = 5;
        f.variables[v as usize].last_use = 15;
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    /// A variable that was declared but never assigned (`first_def == u32::MAX`) must be ignored,
    /// even if its slot otherwise collides.
    #[test]
    fn no_conflict_never_defined_variable() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 20);
        let v = f.add_unique("y", &INT, 0);
        f.variables[v as usize].stack_pos = 0; // same slot, but first_def stays u32::MAX
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── compute_intervals unit tests ──────────────────────────────────────────

    /// A single Set followed by a Var read: `first_def` and `last_use` must be populated
    /// and `last_use` must be >= `first_def`.
    #[test]
    fn compute_intervals_set_then_read() {
        let mut f = Function::new("f", "test");
        let v = f.add_unique("x", &INT, 0);
        let code = Value::Block(Box::new(Block {
            name: "",
            operators: vec![Value::Set(v, Box::new(Value::Int(42))), Value::Var(v)],
            result: INT,
            scope: 0,
            var_size: 0,
        }));
        let mut seq = 0u32;
        compute_intervals(&code, &mut f, u32::MAX, u32::MAX, &mut seq, 0);
        assert_ne!(
            f.variables[v as usize].first_def,
            u32::MAX,
            "first_def not set"
        );
        assert!(
            f.variables[v as usize].last_use >= f.variables[v as usize].first_def,
            "last_use must be >= first_def"
        );
    }

    /// A variable that is Set before a loop and read inside it: `last_use` must exceed `first_def`,
    /// proving that the in-loop read was recorded at a higher sequence number.
    #[test]
    fn compute_intervals_loop_extends_last_use() {
        let mut f = Function::new("f", "test");
        let v = f.add_unique("x", &INT, 0);
        let code = Value::Block(Box::new(Block {
            name: "",
            operators: vec![
                Value::Set(v, Box::new(Value::Int(0))),
                Value::Loop(Box::new(Block {
                    name: "",
                    operators: vec![Value::Var(v)],
                    result: Type::Void,
                    scope: 0,
                    var_size: 0,
                })),
            ],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));
        let mut seq = 0u32;
        compute_intervals(&code, &mut f, u32::MAX, u32::MAX, &mut seq, 0);
        let fd = f.variables[v as usize].first_def;
        let lu = f.variables[v as usize].last_use;
        assert_ne!(fd, u32::MAX, "first_def not set");
        assert!(
            lu > fd,
            "last_use {lu} should exceed first_def {fd} after an in-loop read"
        );
    }

    /// Two variables in a sequential if/else: the one used only in the true branch and the one
    /// used only in the false branch can share the same slot without conflict because their
    /// live intervals do not overlap.
    #[test]
    fn compute_intervals_if_branches_can_reuse_slot() {
        let mut f = Function::new("f", "test");
        let a = f.add_unique("a", &INT, 0);
        let b = f.add_unique("b", &INT, 0);
        // code: if true { a = 1; a } else { b = 2; b }
        let code = Value::If(
            Box::new(Value::Boolean(true)),
            Box::new(Value::Block(Box::new(Block {
                name: "",
                operators: vec![Value::Set(a, Box::new(Value::Int(1))), Value::Var(a)],
                result: INT,
                scope: 0,
                var_size: 0,
            }))),
            Box::new(Value::Block(Box::new(Block {
                name: "",
                operators: vec![Value::Set(b, Box::new(Value::Int(2))), Value::Var(b)],
                result: INT,
                scope: 0,
                var_size: 0,
            }))),
        );
        let mut seq = 0u32;
        compute_intervals(&code, &mut f, u32::MAX, u32::MAX, &mut seq, 0);
        // a's live interval is entirely before b's — they could share a slot.
        let a_last = f.variables[a as usize].last_use;
        let b_first = f.variables[b as usize].first_def;
        assert!(
            a_last < b_first,
            "if-branch var a (last_use={a_last}) should finish before else-branch var b starts (first_def={b_first})"
        );
        // Manually assign them the same slot and confirm no conflict is reported.
        f.variables[a as usize].stack_pos = 0;
        f.variables[b as usize].stack_pos = 0;
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── regression tests for specific known bugs ──────────────────────────────

    /// Documents the exact slot geometry of the `t_4Code_define` bug (discovered 2026-03-11).
    ///
    /// In `lib/code.loft::define`, the `res` variable (integer, 4 bytes) was allocated at
    /// slot 66.  In the else-branch, `_elm_1` (`DbRef`, 12 bytes) was later allocated at
    /// slot 62 — after `CopyRecord` dropped `stack.position` from 86 to 62.  The range
    /// [62, 74) for `_elm_1` swallows `res` at [66, 70).  Both are alive at the same time.
    ///
    /// The correct fix is to assign `_elm_1` at slot ≥ 70, not at 62.  This requires
    /// live-interval information to know that `res` is still alive at that point.
    #[test]
    fn t_4code_define_res_elm1_geometry() {
        let mut f = Function::new("define", "code.loft");
        let ref_tp = Type::Reference(0, vec![]);
        // res: integer, slot [66, 70), alive from the start to the end of the function.
        add_var(&mut f, &INT, 66, 0, 200);
        // _elm_1: DbRef, slot [62, 74), alive only in the else-branch.
        // This is the buggy assignment — placing it at 62 conflicts with res at [66, 70).
        add_var(&mut f, &ref_tp, 62, 100, 150);
        assert!(
            find_conflict(&f.variables, &HashMap::new()).is_some(),
            "_elm_1 at [62,74) must be detected as conflicting with res at [66,70)"
        );
    }

    /// Demonstrates that a post-loop variable CAN share the slot of a loop-body variable
    /// when their live intervals are strictly non-overlapping.  This is the pattern in the
    /// `polymorph` test: after the loop, `stack.position` drops back to `loop_pos`, and the
    /// next variable (`t`) is correctly allowed to reuse the slot of the dead loop element `v`.
    ///
    /// A naive fix that advances `stack.position` to `max_assigned_slot` before every claim
    /// would incorrectly BLOCK this safe reuse and must not be used.
    #[test]
    fn post_loop_slot_reuse_is_allowed() {
        let mut f = Function::new("test_expr", "test");
        let ref_tp = Type::Reference(0, vec![]);
        // v: loop element (DbRef), slot [144, 156), alive only inside the loop (seq 50..80).
        add_var(&mut f, &ref_tp, 144, 50, 80);
        // a: loop-body accumulator (integer), slot [156, 160), alive only inside the loop.
        add_var(&mut f, &INT, 156, 55, 80);
        // t: post-loop variable (DbRef), slot [144, 156), alive after the loop (seq 90..120).
        // t reuses v's slot — safe because their intervals [50..80] and [90..120] don't overlap.
        add_var(&mut f, &ref_tp, 144, 90, 120);
        assert!(
            find_conflict(&f.variables, &HashMap::new()).is_none(),
            "t should be allowed to reuse v's slot after the loop ends"
        );
    }

    /// Slot reuse between a loop variable and a post-loop variable is ONLY safe when the
    /// intervals don't overlap.  If they DO overlap (impossible in practice for a well-formed
    /// loop, but detectable), it must be flagged.
    #[test]
    fn overlapping_loop_and_post_loop_is_conflict() {
        let mut f = Function::new("f", "test");
        let ref_tp = Type::Reference(0, vec![]);
        // v: loop element alive in [50, 100]
        add_var(&mut f, &ref_tp, 144, 50, 100);
        // t: "post-loop" variable placed at the same slot but (mistakenly) started at seq 80
        // while v is still alive — live intervals overlap → conflict.
        add_var(&mut f, &ref_tp, 144, 80, 120);
        assert!(
            find_conflict(&f.variables, &HashMap::new()).is_some(),
            "overlapping intervals at the same slot must be a conflict"
        );
    }

    // ── assign_slots unit tests ───────────────────────────────────────────────

    /// Two sequential variables: `assign_slots` should place the second at the same slot
    /// as the first because their intervals don't overlap.
    #[test]

    fn assign_slots_sequential_reuse() {
        let mut f = Function::new("f", "test");
        let v1 = f.add_unique("v1", &INT, 0);
        f.variables[v1 as usize].first_def = 0;
        f.variables[v1 as usize].last_use = 10;
        let v2 = f.add_unique("v2", &INT, 0);
        f.variables[v2 as usize].first_def = 11;
        f.variables[v2 as usize].last_use = 20;
        run_assign_slots(&mut f, 0);
        assert_eq!(
            f.variables[v1 as usize].stack_pos, f.variables[v2 as usize].stack_pos,
            "non-overlapping variables should share a slot"
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    /// Two concurrent variables must get distinct slots.
    #[test]

    fn assign_slots_concurrent_get_separate_slots() {
        let mut f = Function::new("f", "test");
        let v1 = f.add_unique("v1", &INT, 0);
        f.variables[v1 as usize].first_def = 0;
        f.variables[v1 as usize].last_use = 20;
        let v2 = f.add_unique("v2", &INT, 0);
        f.variables[v2 as usize].first_def = 0;
        f.variables[v2 as usize].last_use = 20;
        run_assign_slots(&mut f, 0);
        assert_ne!(
            f.variables[v1 as usize].stack_pos, f.variables[v2 as usize].stack_pos,
            "simultaneously-live variables must not share a slot"
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    /// A `DbRef` variable is 12 bytes; the slot after it must start at offset 12,
    /// not at offset 4 (the size of an integer).
    #[test]

    fn assign_slots_respects_variable_size() {
        let ref_tp = Type::Reference(0, vec![]);
        let mut f = Function::new("f", "test");
        let v1 = f.add_unique("v1", &ref_tp, 0);
        f.variables[v1 as usize].first_def = 0;
        f.variables[v1 as usize].last_use = 5;
        let v2 = f.add_unique("v2", &INT, 0);
        f.variables[v2 as usize].first_def = 0;
        f.variables[v2 as usize].last_use = 5;
        run_assign_slots(&mut f, 0);
        let s1 = f.variables[v1 as usize].stack_pos;
        let s2 = f.variables[v2 as usize].stack_pos;
        let dbref_size = size_of::<DbRef>() as u16;
        let no_overlap = s2 >= s1 + dbref_size || s1 >= s2 + 4;
        assert!(no_overlap, "DbRef slot must not overlap integer slot");
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    /// Variables that were never defined (`first_def` == `u32::MAX`) must be skipped.
    #[test]

    fn assign_slots_skips_never_defined() {
        let mut f = Function::new("f", "test");
        let v1 = f.add_unique("v1", &INT, 0);
        f.variables[v1 as usize].first_def = 0;
        f.variables[v1 as usize].last_use = 10;
        let v2 = f.add_unique("v2", &INT, 0);
        // v2 is never defined — first_def stays u32::MAX
        run_assign_slots(&mut f, 0);
        assert_eq!(
            f.variables[v2 as usize].stack_pos,
            u16::MAX,
            "never-defined variable must keep stack_pos == u16::MAX"
        );
    }

    // ── A6.3b: Bug B — narrow → wide slot reuse ──────────────────────────────

    /// A dead 1-byte variable's slot must not be reused by a wider variable via
    /// displacement.  `flag` (scope 0, argument/outermost scope — permanent, never freed)
    /// remains physically on the stack even after its live interval ends, so `tos_estimate`=1.
    /// `fnref` (4B) cannot displace into the 1B flag slot (size mismatch) and is
    /// placed at slot 1 (fresh TOS), which is also correct for direct placement.
    #[test]
    fn assign_slots_no_narrow_to_wide_reuse() {
        const BOOL: Type = Type::Boolean;
        // flag: boolean (1 byte), dead early; fnref: integer (4 bytes), born after flag dies.
        let mut f = Function::new("f", "test");
        let flag = f.add_unique("flag", &BOOL, 0);
        f.variables[flag as usize].first_def = 0;
        f.variables[flag as usize].last_use = 2;
        f.variables[flag as usize].scope = 0; // function scope — not a loop scope
        let fnref = f.add_unique("fnref", &INT, 0);
        f.variables[fnref as usize].first_def = 5;
        f.variables[fnref as usize].last_use = 10;
        run_assign_slots(&mut f, 0);
        // flag (scope 0) is dead but physically present → tos_estimate=1.
        // fnref cannot displace into the mismatched 1B slot; placed at fresh TOS slot 1.
        assert_eq!(
            f.variables[fnref as usize].stack_pos, 1,
            "4-byte fnref must not reuse 1-byte flag slot; it gets a fresh slot at TOS"
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── A6.3b: Bug C — Value::Iter not traversed by compute_intervals ─────────

    /// The index variable of a `Value::Iter` node is read inside the iterator's
    /// `create` / `next` sub-expressions.  `compute_intervals` must recurse into those
    /// sub-expressions so that `last_use` is set beyond the loop body.  Without this,
    /// `last_use` stays 0 and `assign_slots` treats the index as dead at birth,
    /// allowing a later variable to steal its slot and corrupting the loop counter.
    #[test]
    fn compute_intervals_iter_index_var_gets_last_use() {
        let mut f = Function::new("f", "test");
        let idx = f.add_unique("idx", &INT, 0);
        // Simulate: create = Set(idx, 0), next = Var(idx), extra_init = Null
        let create = Value::Set(idx, Box::new(Value::Int(0)));
        let next = Value::Var(idx);
        let extra_init = Value::Null;
        let iter = Value::Iter(idx, Box::new(create), Box::new(next), Box::new(extra_init));
        let mut seq = 0u32;
        compute_intervals(&iter, &mut f, u32::MAX, u32::MAX, &mut seq, 0);
        assert_ne!(
            f.variables[idx as usize].last_use, 0,
            "index variable's last_use must be set by traversing Iter sub-expressions"
        );
        assert_ne!(
            f.variables[idx as usize].first_def,
            u32::MAX,
            "index variable's first_def must be set"
        );
    }

    // ── A13: Float/Long dead-slot reuse ──────────────────────────────────────

    /// Two sequential Long (8-byte) variables must share a slot after A13.
    /// Before A13 `can_reuse = var_size <= 4` prevented Long/Float from reusing dead slots.
    #[test]
    fn assign_slots_sequential_long_reuse() {
        let mut f = Function::new("f", "test");
        let v1 = f.add_unique("v1", &Type::Long, 0);
        f.variables[v1 as usize].first_def = 0;
        f.variables[v1 as usize].last_use = 10;
        let v2 = f.add_unique("v2", &Type::Long, 0);
        f.variables[v2 as usize].first_def = 11;
        f.variables[v2 as usize].last_use = 20;
        run_assign_slots(&mut f, 0);
        assert_eq!(
            f.variables[v1 as usize].stack_pos, f.variables[v2 as usize].stack_pos,
            "sequential Long variables must share a slot (A13)"
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    /// Two concurrent Long variables must still get distinct slots — the reuse
    /// guard must not fire when intervals overlap.
    #[test]
    fn assign_slots_concurrent_long_separate_slots() {
        let mut f = Function::new("f", "test");
        let v1 = f.add_unique("v1", &Type::Long, 0);
        f.variables[v1 as usize].first_def = 0;
        f.variables[v1 as usize].last_use = 20;
        let v2 = f.add_unique("v2", &Type::Long, 0);
        f.variables[v2 as usize].first_def = 5;
        f.variables[v2 as usize].last_use = 15;
        run_assign_slots(&mut f, 0);
        assert_ne!(
            f.variables[v1 as usize].stack_pos, f.variables[v2 as usize].stack_pos,
            "concurrent Long variables must not share a slot"
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── A14: skip_free flag ───────────────────────────────────────────────────

    /// `clean_work_refs` must set `skip_free = true` on the work-ref variables it marks,
    /// and must NOT mutate their `type_def`.  Before A14 it set the type to
    /// `Type::Reference(0, vec![0])` to suppress the `OpFreeRef` emit — a type-mutation
    /// hack that confused downstream code.
    #[test]
    fn clean_work_refs_sets_flag_not_type() {
        use crate::lexer::Lexer;
        let ref_tp = Type::Reference(1, vec![]);
        let mut f = Function::new("f", "test");
        let mut lexer = Lexer::from_str("", "test");
        // Allocate a real work-ref variable via work_refs() so the naming matches.
        let baseline = f.work_ref();
        let v_nr = f.work_refs(&ref_tp, &mut lexer);
        assert_eq!(
            f.work_ref(),
            baseline + 1,
            "work_ref counter should have incremented"
        );
        // Mark the range [baseline, work_ref) as skip_free.
        f.clean_work_refs(baseline);
        // The variable's type must be unchanged — not mutated to Reference(0, [0]).
        assert!(
            !matches!(f.tp(v_nr), Type::Reference(0, dep) if dep == &[0u16]),
            "clean_work_refs must not mutate the type to Reference(0, [0])"
        );
        // The variable must have skip_free set.
        assert!(
            f.is_skip_free(v_nr),
            "clean_work_refs must set skip_free = true on the marked variable"
        );
    }

    // ── A6.3b: Bug C part 2 — write-only variable last_use ───────────────────

    /// A variable that is only ever WRITTEN (never read via `Value::Var`) must still
    /// have its `last_use` updated so that `assign_slots` does not treat it as dead.
    /// Without this, the slot is reused by later variables while the write is still
    /// live, corrupting adjacent stack data at runtime.
    #[test]
    fn compute_intervals_write_only_var_gets_last_use() {
        let mut f = Function::new("f", "test");
        // acc: written at seq 0, then written again at seq 4 (inside a block simulating a loop
        // body); never read via Var.  Its last_use must be >= 4 so assign_slots sees it as live.
        let acc = f.add_unique("acc", &INT, 0);
        let other = f.add_unique("other", &INT, 0);
        // Simulate: Set(acc, 0), Set(other, 1), Set(acc, other+1)
        let block = Value::Block(Box::new(crate::data::Block {
            name: "",
            operators: vec![
                Value::Set(acc, Box::new(Value::Int(0))),
                Value::Set(other, Box::new(Value::Int(1))),
                Value::Set(
                    acc,
                    Box::new(Value::Call(0, vec![Value::Var(other), Value::Int(1)])),
                ),
            ],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));
        let mut seq = 0u32;
        compute_intervals(&block, &mut f, u32::MAX, u32::MAX, &mut seq, 0);
        // acc is written twice; last_use must reflect the second write.
        assert!(
            f.variables[acc as usize].last_use > f.variables[other as usize].first_def,
            "write-only acc must outlive other to prevent slot aliasing"
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── A12: Lazy work-variable initialization — Text slot sharing ────────────

    /// Two sequential Text (24 B) variables with non-overlapping intervals must
    /// share a slot after A12 extends `can_reuse` to the `Text` type.
    /// Before A12, `can_reuse = var_size <= 8` prevented Text (24 B) from
    /// reusing dead same-type slots.
    #[test]
    fn assign_slots_sequential_text_reuse() {
        let text_tp = Type::Text(Vec::new());
        let mut f = Function::new("f", "test");
        let v1 = f.add_unique("v1", &text_tp, 0);
        f.variables[v1 as usize].first_def = 0;
        f.variables[v1 as usize].last_use = 10;
        let v2 = f.add_unique("v2", &text_tp, 0);
        f.variables[v2 as usize].first_def = 11;
        f.variables[v2 as usize].last_use = 20;
        run_assign_slots(&mut f, 0);
        assert_eq!(
            f.variables[v1 as usize].stack_pos, f.variables[v2 as usize].stack_pos,
            "sequential Text variables must share a slot (A12)"
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── A15: sequential for-loops must not let iter_state alias total ─────────

    /// Regression for the `sorted_remove` slot-conflict: two sequential for-loops where
    /// the first loop's variables (non-loop-scope block vars `e#iter_state`, `e#index`) are
    /// dead when the second loop starts, and a non-loop variable `total` is born between
    /// the two loops and lives through the second.
    ///
    /// Before the `loop_seq_ranges` fix, `assign_slots` computed `tos_estimate` for the second
    /// `e#iter_state` by including the dead first-loop block vars (non-loop scope → physically
    /// present until return).  This raised `tos_estimate` to 64, which caused the second
    /// `e#iter_state` to be placed at slot 56 (past `total` at 52).  Codegen then remapped it
    /// to 52 (actual TOS) → conflict with `total`.
    ///
    /// The correct behavior: `assign_slots` must see the dead non-loop-scope vars and place
    /// the second `e#iter_state` at `tos_estimate`, which codegen's actual TOS also matches.
    #[test]
    fn assign_slots_sequential_for_loops_no_conflict() {
        let ref_tp = Type::Reference(0, vec![]);
        let mut f = Function::new("n_test", "test");
        // scope 3: first for-loop body (loop scope, seq range [95, 129])
        declare_loop(&mut f, 3, 95, 129);
        // scope 8: second for-loop body (loop scope, seq range [142, 167])
        declare_loop(&mut f, 8, 142, 167);

        // Always-live variables (scope 1, non-loop)
        add_scoped_var(&mut f, "work", &Type::Text(vec![]), 1, 0, 187);
        add_scoped_var(&mut f, "db", &ref_tp, 1, 3, 186);
        // Dead at seq 131 (non-loop scope → physically present until return)
        add_scoped_var(&mut f, "_elm_1", &ref_tp, 1, 12, 81);
        // First for-loop vars: scope 2 = non-loop block wrapper, scope 3 = loop body
        add_scoped_var(&mut f, "e#iter_state_1", &Type::Long, 2, 95, 129);
        add_scoped_var(&mut f, "e#index_1", &INT, 2, 97, 129);
        add_scoped_var(&mut f, "e_1", &ref_tp, 3, 98, 115);
        // total: born after first loop, lives through second (non-loop scope)
        add_scoped_var(&mut f, "total", &INT, 1, 131, 174);
        // Second for-loop vars: scope 7 = non-loop block wrapper, scope 8 = loop body
        add_scoped_var(&mut f, "e#iter_state_2", &Type::Long, 7, 142, 167);
        add_scoped_var(&mut f, "e#index_2", &INT, 7, 144, 167);
        add_scoped_var(&mut f, "e_2", &ref_tp, 8, 145, 163);

        // Scope hierarchy: root=1, children: 2→1 (non-loop), 3→2 (loop), 7→1 (non-loop), 8→7 (loop)
        run_assign_slots_scoped(
            &mut f,
            4,
            1,
            &[(2, 1, false), (3, 2, true), (7, 1, false), (8, 7, true)],
        ); // local_start=4: no-arg function, 4-byte return address
        assert!(
            find_conflict(&f.variables, &HashMap::new()).is_none(),
            "second e#iter_state must not alias total; variable table:\n{f}",
        );
    }

    /// `compute_intervals` must panic with a depth-limit message when nesting exceeds 1000.
    #[test]
    #[should_panic(expected = "expression nesting limit")]
    fn compute_intervals_depth_limit() {
        let mut v: Value = Value::Null;
        for _ in 0..1100 {
            v = Value::Block(Box::new(Block {
                name: "",
                operators: vec![v],
                result: Type::Void,
                scope: 0,
                var_size: 0,
            }));
        }
        let mut f = Function::new("f", "test");
        let mut seq = 0u32;
        compute_intervals(&v, &mut f, u32::MAX, u32::MAX, &mut seq, 0);
    }

    // ── C43.1: zone-2 dead-slot reuse with full conflict scan ───────────────

    /// Three 24-byte text variables: v1 (live 0–10), v2 (live 5–15, overlaps v1),
    /// v3 (live 11–20, does not overlap v1).  v3 should reuse v1's slot.
    /// v2 must NOT reuse v1 (temporal overlap).
    #[test]
    fn zone2_reuse_conflict_free() {
        let text_tp = Type::Text(Vec::new());
        let mut f = Function::new("f", "test");
        let v1 = f.add_unique("v1", &text_tp, 0);
        f.variables[v1 as usize].first_def = 0;
        f.variables[v1 as usize].last_use = 10;
        let v2 = f.add_unique("v2", &text_tp, 0);
        f.variables[v2 as usize].first_def = 5;
        f.variables[v2 as usize].last_use = 15;
        let v3 = f.add_unique("v3", &text_tp, 0);
        f.variables[v3 as usize].first_def = 11;
        f.variables[v3 as usize].last_use = 20;
        // Simulate zone-2 placement: v1 at 0, v2 at 24.
        f.variables[v1 as usize].stack_pos = 0;
        f.variables[v2 as usize].stack_pos = 24;
        // v3 should reuse v1's slot (v1 dead at 10, v3 starts at 11).
        let slot = find_reusable_zone2_slot(&f, v3 as usize, 0);
        assert_eq!(slot, Some(0), "v3 should reuse v1's slot");
        // v2 should NOT find a reusable slot (overlaps v1 temporally).
        f.variables[v3 as usize].stack_pos = u16::MAX; // reset
        f.variables[v2 as usize].stack_pos = u16::MAX; // reset
        let slot2 = find_reusable_zone2_slot(&f, v2 as usize, 0);
        assert_eq!(slot2, None, "v2 must not reuse v1 (temporal overlap)");
    }

    /// C46: text reuse works even when a non-text variable is placed between
    /// the dead text and the new one (no top-of-stack restriction).
    #[test]
    fn zone2_text_reuse_non_consecutive() {
        let text_tp = Type::Text(Vec::new());
        let ref_tp = Type::Reference(0, Vec::new());
        let mut f = Function::new("f", "test");
        // t1: text, live 0–10
        let t1 = f.add_unique("t1", &text_tp, 0);
        f.variables[t1 as usize].first_def = 0;
        f.variables[t1 as usize].last_use = 10;
        // r: reference (12 bytes), live 5–25 — sits between the two texts
        let r = f.add_unique("r", &ref_tp, 0);
        f.variables[r as usize].first_def = 5;
        f.variables[r as usize].last_use = 25;
        // t2: text, live 11–20 — should reuse t1's slot (not blocked by r)
        let t2 = f.add_unique("t2", &text_tp, 0);
        f.variables[t2 as usize].first_def = 11;
        f.variables[t2 as usize].last_use = 20;
        run_assign_slots(&mut f, 0);
        assert_eq!(
            f.variables[t1 as usize].stack_pos, f.variables[t2 as usize].stack_pos,
            "t2 should reuse t1's slot despite r being placed in between"
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── P122: parent-scope zone2 variables must not overlap child-scope zone1 ──

    /// When a parent scope has many zone2 reference variables (like __lift_N temps
    /// from P135 inline struct arg lifting), a late-placed reference variable in
    /// the parent (like `r`) occupies slots right before the child scope's frame_base.
    /// The child scope's zone1 variables must not overlap the parent's zone2 range.
    ///
    /// Reproduces the GL crash: 13 refs in scope 1 (slots 68–224), a for-loop in
    /// scope 2 with a zone1 integer `idx`.  idx must start at 224 or higher, never
    /// inside `r`'s [212, 224) range.
    #[test]
    fn assign_slots_parent_zone2_does_not_overlap_child_zone1() {
        let ref_tp = Type::Reference(0, vec![]);
        let mut f = Function::new("n_render_native", "test");
        // scope 2: for-loop body
        declare_loop(&mut f, 2, 50, 90);

        // Parent scope (1): 13 reference variables, all live across the loop
        for i in 0..13u32 {
            add_scoped_var(
                &mut f,
                &format!("lift{}", i + 1),
                &ref_tp,
                1,
                i * 10,
                100,
            );
        }
        // r: the last parent zone2 ref, live across the loop
        let r_var = add_scoped_var(&mut f, "r", &ref_tp, 1, 45, 95);

        // Child scope (2): zone1 integer variable (loop index)
        let idx_var = add_scoped_var(&mut f, "idx", &INT, 2, 50, 90);

        run_assign_slots_scoped(
            &mut f,
            4, // local_start: 4-byte return address
            1,
            &[(2, 1, true)],
        );

        let r_slot = f.stack(r_var);
        let r_end = r_slot + size(&ref_tp, &Context::Variable);
        let idx_slot = f.stack(idx_var);
        let idx_end = idx_slot + size(&INT, &Context::Variable);

        // idx must not overlap r
        assert!(
            idx_slot >= r_end || idx_end <= r_slot,
            "child zone1 variable 'idx' at [{idx_slot}, {idx_end}) overlaps parent zone2 \
             variable 'r' at [{r_slot}, {r_end}). \
             assign_slots must place child zone1 after all live parent zone2 variables."
        );
        assert!(
            find_conflict(&f.variables, &HashMap::new()).is_none(),
            "slot conflict detected in parent-zone2 / child-zone1 test"
        );
    }

    // ── P122: coroutine Set(gen, Call(fn, [Block])) ──
    //
    // Codegen evaluates Call arguments before placing the result.
    // Set(gen, Call(fn, [Block(scope=5, [Set(vec, ...)])])):
    //   - Block(scope=5) evaluated first → vec placed at tos
    //   - gen placed after → must be above vec

    #[test]
    fn call_with_block_arg_places_block_vars_first() {
        let ref_tp = Type::Reference(0, vec![]);
        let text_tp = Type::Text(vec![]);
        let mut f = Function::new("f", "test");
        declare_loop(&mut f, 3, 5, 70);

        let work = add_scoped_var(&mut f, "work", &text_tp, 1, 1, 80);
        add_scoped_var(&mut f, "total", &INT, 3, 6, 66);
        let gen_var = add_scoped_var(&mut f, "gen", &ref_tp, 4, 20, 65);
        let vec_var = add_scoped_var(&mut f, "vec", &ref_tp, 5, 22, 30);
        let elm_var = add_scoped_var(&mut f, "elm", &ref_tp, 5, 23, 29);

        let scope5 = Value::Block(Box::new(Block {
            name: "", scope: 5, var_size: 0, result: Type::Void,
            operators: vec![
                Value::Set(vec_var, Box::new(Value::Null)),
                Value::Set(elm_var, Box::new(Value::Null)),
            ],
        }));
        let gen_set = Value::Set(gen_var, Box::new(Value::Call(999, vec![scope5])));
        let loop3 = Value::Loop(Box::new(Block {
            name: "", scope: 3, var_size: 0, result: Type::Void,
            operators: vec![gen_set],
        }));
        let mut code = Value::Block(Box::new(Block {
            name: "", scope: 1, var_size: 0, result: Type::Void,
            operators: vec![Value::Set(work, Box::new(Value::Null)), loop3],
        }));
        assign_slots(&mut f, &mut code, 4);

        assert_ne!(f.stack(gen_var), u16::MAX, "gen must be placed");
        assert_ne!(f.stack(vec_var), u16::MAX, "vec must be placed");
        assert!(
            f.stack(vec_var) < f.stack(gen_var),
            "vec at {} must be below gen at {} — Call args evaluated first",
            f.stack(vec_var), f.stack(gen_var)
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── if_typing pattern: Set(tv, Block(scope=2, [Set(a, ...), If(...)]))
    //
    // tv is Text (scope=1). Inner is Block(scope=2) containing child scopes 3,4.
    // Text is excluded from block-return frame sharing because codegen emits
    // OpText before the Block. So tv must be at tos BEFORE the Block, and a
    // (scope=2, also Text) must be placed inside scope=2's frame.
    // tv and a must not overlap.

    #[test]
    fn text_block_return_no_overlap_with_child_text() {
        let text_tp = Type::Text(vec![]);
        let mut f = Function::new("f", "test");

        let work = add_scoped_var(&mut f, "work", &text_tp, 1, 1, 50);
        let tv = add_scoped_var(&mut f, "tv", &text_tp, 1, 5, 45);
        let a = add_scoped_var(&mut f, "a", &text_tp, 2, 10, 30);

        // IR: Set(work, Null), Set(tv, Block(scope=2, [Set(a, "12"), ...]))
        let scope2 = Value::Block(Box::new(Block {
            name: "", scope: 2, var_size: 0, result: Type::Void,
            operators: vec![Value::Set(a, Box::new(Value::Null))],
        }));
        let mut code = Value::Block(Box::new(Block {
            name: "", scope: 1, var_size: 0, result: Type::Void,
            operators: vec![
                Value::Set(work, Box::new(Value::Null)),
                Value::Set(tv, Box::new(scope2)),
            ],
        }));
        assign_slots(&mut f, &mut code, 4);

        let tv_slot = f.stack(tv);
        let tv_end = tv_slot + size(&text_tp, &Context::Variable);
        let a_slot = f.stack(a);
        let a_end = a_slot + size(&text_tp, &Context::Variable);

        // tv and a must not overlap (both live at seq 10-30)
        assert!(
            a_slot >= tv_end || a_end <= tv_slot,
            "tv [{tv_slot},{tv_end}) and a [{a_slot},{a_end}) must not overlap"
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── closure_capture pattern: parent var placed after child scope exits
    //
    // scope=1: work(24B), clos(12B), greeting(24B)
    // scope=2: f(16B) via Set(f, Block(scope=3))  — block-return
    // then back in scope=1's IR: Set(tv, Call(...)) where Call args contain
    // Block(scope=4) with child scopes.
    // tv must not overlap f even though f's child scope exits before tv.

    #[test]
    fn parent_var_after_child_block_return_no_overlap() {
        let ref_tp = Type::Reference(0, vec![]);
        let text_tp = Type::Text(vec![]);
        let mut f = Function::new("f_test", "test");

        let work = add_scoped_var(&mut f, "work", &text_tp, 1, 1, 50);
        let greeting = add_scoped_var(&mut f, "greeting", &text_tp, 1, 3, 48);
        // f is block-return: Set(f, Block(scope=2, [...]))
        let f_var = add_scoped_var(&mut f, "fv", &ref_tp, 2, 10, 20);
        // tv comes after f dies; Set(tv, Call(fn, [Block(scope=4)]))
        let tv = add_scoped_var(&mut f, "tv", &text_tp, 1, 25, 45);

        // IR: Set(work, Null), Set(greeting, Null),
        //     Block(scope=2, [Set(f, Block(scope=3))]),
        //     Set(tv, Call(fn, [Block(scope=4)]))
        let scope3 = Value::Block(Box::new(Block {
            name: "", scope: 3, var_size: 0, result: Type::Void,
            operators: vec![],
        }));
        let scope2 = Value::Block(Box::new(Block {
            name: "", scope: 2, var_size: 0, result: Type::Void,
            operators: vec![Value::Set(f_var, Box::new(scope3))],
        }));
        let scope4 = Value::Block(Box::new(Block {
            name: "", scope: 4, var_size: 0, result: Type::Void,
            operators: vec![],
        }));
        let mut code = Value::Block(Box::new(Block {
            name: "", scope: 1, var_size: 0, result: Type::Void,
            operators: vec![
                Value::Set(work, Box::new(Value::Null)),
                Value::Set(greeting, Box::new(Value::Null)),
                scope2,
                Value::Set(tv, Box::new(Value::Call(999, vec![scope4]))),
            ],
        }));
        assign_slots(&mut f, &mut code, 4);

        let f_slot = f.stack(f_var);
        let f_end = f_slot + size(&ref_tp, &Context::Variable);
        let tv_slot = f.stack(tv);
        let tv_end = tv_slot + size(&text_tp, &Context::Variable);

        // f dies at 20, tv born at 25 → no temporal overlap, spatial reuse is OK
        // But if they DO overlap temporally, spatial must not overlap.
        if tv.min(f_var) <= tv.max(f_var) {
            // Check find_conflict which handles all cases
        }
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }

    // ── Parent-scope var Set node inside child scope's operator list ──
    //
    // After scope analysis, Set(tv, Insert([...])) can end up inside
    // scope=4's operators even though tv is scope=1.  place_large_and_recurse
    // skips it (scope mismatch), but still recurses into the Insert's inner.
    // The inner may contain child Blocks. tv must still get a valid slot
    // that doesn't overlap any child scope variable.

    #[test]
    fn parent_var_set_inside_child_scope_operators() {
        let text_tp = Type::Text(vec![]);
        let ref_tp = Type::Reference(0, vec![]);
        let mut f = Function::new("f", "test");

        // scope 1 vars
        let work = add_scoped_var(&mut f, "work", &text_tp, 1, 1, 50);
        let tv = add_scoped_var(&mut f, "tv", &text_tp, 1, 15, 45);
        // scope 2: block with Set(f, Block(scope=3))
        let fv = add_scoped_var(&mut f, "fv", &ref_tp, 2, 10, 20);

        // IR: scope 1 root contains:
        //   Set(work, Null)
        //   Block(scope=2, [
        //     Set(fv, Block(scope=3, []))
        //     Set(tv, Null)           ← scope=1 var inside scope=2's operators!
        //   ])
        let scope3 = Value::Block(Box::new(Block {
            name: "", scope: 3, var_size: 0, result: Type::Void,
            operators: vec![],
        }));
        let scope2 = Value::Block(Box::new(Block {
            name: "", scope: 2, var_size: 0, result: Type::Void,
            operators: vec![
                Value::Set(fv, Box::new(scope3)),
                Value::Set(tv, Box::new(Value::Null)),  // parent var in child scope!
            ],
        }));
        let mut code = Value::Block(Box::new(Block {
            name: "", scope: 1, var_size: 0, result: Type::Void,
            operators: vec![
                Value::Set(work, Box::new(Value::Null)),
                scope2,
            ],
        }));
        assign_slots(&mut f, &mut code, 4);

        let fv_slot = f.stack(fv);
        let fv_end = fv_slot + size(&ref_tp, &Context::Variable);
        let tv_slot = f.stack(tv);
        let tv_end = tv_slot + size(&text_tp, &Context::Variable);

        // tv must not overlap fv if their lives overlap
        if f.variables[tv as usize].first_def <= f.variables[fv as usize].last_use
            && f.variables[tv as usize].last_use >= f.variables[fv as usize].first_def
        {
            assert!(
                tv_slot >= fv_end || tv_end <= fv_slot,
                "tv [{tv_slot},{tv_end}) overlaps fv [{fv_slot},{fv_end}) — both live"
            );
        }
        assert!(
            find_conflict(&f.variables, &HashMap::new()).is_none(),
            "slot conflict: parent var Set inside child scope"
        );
    }

    // ── P122/P135: Set(v, Insert([Set(__lift_1, ...), ..., Call(...)]))
    //
    // The P135 lift produces Insert nodes containing preamble Sets for
    // __lift_N temporaries followed by the actual Call.  Codegen evaluates
    // the Insert sequentially — the __lift Sets advance TOS before the
    // Call result is assigned to v.  assign_slots must place __lift vars
    // BEFORE v.

    #[test]
    fn insert_preamble_sets_placed_before_target() {
        let ref_tp = Type::Reference(0, vec![]);
        let mut f = Function::new("f", "test");
        declare_loop(&mut f, 2, 5, 60);

        add_scoped_var(&mut f, "total", &Type::Float, 1, 1, 60);
        // scope 3: loop body block
        // view = mat4_look_at(__lift_1, __lift_2, __lift_3)
        let view = add_scoped_var(&mut f, "view", &ref_tp, 3, 20, 50);
        let lift1 = add_scoped_var(&mut f, "__lift_1", &ref_tp, 3, 15, 50);
        let lift2 = add_scoped_var(&mut f, "__lift_2", &ref_tp, 3, 16, 50);
        let lift3 = add_scoped_var(&mut f, "__lift_3", &ref_tp, 3, 17, 50);

        // IR: Loop(scope=2, [Block(scope=3, [Set(view, Insert([Set(lift1, ...), ..., Call(...)]))])])
        let insert = Value::Insert(vec![
            Value::Set(lift1, Box::new(Value::Null)),
            Value::Set(lift2, Box::new(Value::Null)),
            Value::Set(lift3, Box::new(Value::Null)),
            Value::Call(999, vec![Value::Var(lift1), Value::Var(lift2), Value::Var(lift3)]),
        ]);
        let view_set = Value::Set(view, Box::new(insert));
        let block3 = Value::Block(Box::new(Block {
            name: "", scope: 3, var_size: 0, result: Type::Void,
            operators: vec![view_set],
        }));
        let loop2 = Value::Loop(Box::new(Block {
            name: "", scope: 2, var_size: 0, result: Type::Void,
            operators: vec![block3],
        }));
        let mut code = Value::Block(Box::new(Block {
            name: "", scope: 1, var_size: 0, result: Type::Void,
            operators: vec![loop2],
        }));
        assign_slots(&mut f, &mut code, 4);

        // All lifts must be placed before view
        assert_ne!(f.stack(lift1), u16::MAX, "lift1 must be placed");
        assert_ne!(f.stack(view), u16::MAX, "view must be placed");
        assert!(
            f.stack(lift1) < f.stack(view),
            "lift1 at {} must be below view at {} — Insert preamble evaluated first",
            f.stack(lift1), f.stack(view)
        );
        assert!(
            f.stack(lift2) < f.stack(view),
            "lift2 at {} must be below view at {}",
            f.stack(lift2), f.stack(view)
        );
        assert!(
            f.stack(lift3) < f.stack(view),
            "lift3 at {} must be below view at {}",
            f.stack(lift3), f.stack(view)
        );
        assert!(find_conflict(&f.variables, &HashMap::new()).is_none());
    }
}
