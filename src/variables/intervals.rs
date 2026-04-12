// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Live-interval analysis: walk the IR tree and record first-def / last-use
//! sequence numbers for every variable.

use crate::data::{Context, Type, Value};

use super::{Function, size};

/// Walk the IR tree in execution order, recording sequence numbers for each `Set` and `Var` node.
/// After this pass every variable has `first_def` and `last_use` populated so that
/// overlapping live intervals can be detected by `validate_slots`.
///
/// `free_text_nr` / `free_ref_nr` are the definition numbers of `OpFreeText` / `OpFreeRef`
/// (pass `u32::MAX` if the definition is not yet registered).
#[allow(clippy::too_many_lines)]
pub fn compute_intervals(
    val: &Value,
    function: &mut Function,
    free_text_nr: u32,
    free_ref_nr: u32,
    seq: &mut u32,
    depth: usize,
) {
    assert!(
        depth <= 1000,
        "expression nesting limit exceeded at depth {depth}"
    );
    match val {
        Value::Var(v) => {
            let v = *v as usize;
            if v < function.variables.len() {
                function.variables[v].last_use = function.variables[v].last_use.max(*seq);
            }
            *seq += 1;
        }
        Value::Set(v, value) => {
            let v = *v as usize;
            // For Text and Reference (size > 4 bytes), a pre-init opcode (OpText,
            // OpConvRefFromNull, etc.) fires at TOS BEFORE the value expression runs during
            // codegen.  Set first_def here — before traversing value — so assign_slots gives
            // this variable a lower slot than any inner variable.  Without this, inner
            // variables grab the lower slots and force the outer variable above TOS,
            // triggering the claim() fallback with a slot conflict.
            //
            // Only types whose first assignment emits a pre-init opcode BEFORE the value
            // expression runs qualify: Text (OpText), owned Reference (OpConvRefFromNull),
            // struct-enum ref (OpConvRefFromNull).  Float (8 B), Long (8 B), and Vector do
            // NOT have pre-init opcodes; setting first_def early for them causes spurious
            // interval overlaps with variables defined inside the value expression.
            let needs_early_first_def = v < function.variables.len()
                && matches!(
                    function.variables[v].type_def,
                    Type::Text(_) | Type::Reference(_, _) | Type::Enum(_, true, _)
                );
            if needs_early_first_def && function.variables[v].first_def == u32::MAX {
                function.variables[v].first_def = *seq;
                *seq += 1;
            }
            // Process the value expression (inner variables get seq numbers after the target).
            compute_intervals(value, function, free_text_nr, free_ref_nr, seq, depth + 1);
            // Small/primitive types and Vector types: record first_def after traversing value
            // so that inner temporaries (which finish before this assignment takes effect) can
            // potentially share the same stack slot as this variable.
            if !needs_early_first_def
                && v < function.variables.len()
                && function.variables[v].first_def == u32::MAX
            {
                function.variables[v].first_def = *seq;
            }
            // A write to a variable occupies its stack slot just as much as a read does.
            // Without this update, variables that are only ever WRITTEN (never read after
            // their last write) keep last_use = 0, making them appear dead at birth.
            // assign_slots then lets later variables reuse their slot while they are still
            // being written — corrupting the written values at runtime.
            // Classic case: c#index in a text for-loop is written every iteration
            // (Set(c#index, Var(c#next))) but never read by the user; without this
            // update its last_use stays 0 and the loop counter slot gets aliased.
            if v < function.variables.len() {
                function.variables[v].last_use = function.variables[v].last_use.max(*seq);
            }
            *seq += 1;
        }
        Value::Block(bl) => {
            function.record_scope_origin(bl.scope, bl.name);
            for op in &bl.operators {
                compute_intervals(op, function, free_text_nr, free_ref_nr, seq, depth + 1);
            }
        }
        Value::Loop(lp) => {
            function.record_scope_origin(lp.scope, lp.name);
            let seq_start = *seq;
            for op in &lp.operators {
                compute_intervals(op, function, free_text_nr, free_ref_nr, seq, depth + 1);
            }
            let seq_end = *seq;
            function.record_loop_range(lp.scope, seq_start, seq_end);
            // Extend last_use of loop-carried variables.
            // A variable that is (a) defined BEFORE the loop and (b) used inside
            // the loop may be read again at the top of the next iteration.  Extend
            // such variables' last_use to loop_last (= seq_end - 1, the last seq
            // inside the loop) so assign_slots does not let any loop-internal
            // variable reuse their stack slot.
            //
            // Variables first defined INSIDE the loop (first_def >= seq_start) are
            // intentionally excluded: they are written before each use within the
            // same iteration and are not loop-carried (e.g. block-scope temporaries
            // like `_for_result_1` that share a slot with the outer Set target).
            if seq_end > seq_start {
                let loop_last = seq_end - 1;
                for v in &mut function.variables {
                    // Extend loop-carried variables: any variable defined BEFORE the loop
                    // and read INSIDE the loop.  Such variables may be read again at the
                    // top of the next iteration; without extension, assign_slots would
                    // consider them dead and let loop-internal variables reuse their slot,
                    // causing corruption when iteration N+1 reads the stale slot.
                    //
                    // Variables first defined INSIDE the loop (first_def >= seq_start) are
                    // intentionally excluded: they are written before each use within the
                    // same iteration and are not loop-carried (e.g. block-scope temporaries
                    // like `_for_result_1` that share a slot with the outer Set target).
                    let var_size = size(&v.type_def, &Context::Variable);
                    if var_size > 0
                        && v.first_def != u32::MAX
                        && v.first_def < seq_start   // defined before the loop
                        && v.last_use >= seq_start   // used inside the loop
                        && v.last_use < seq_end
                    {
                        v.last_use = loop_last;
                    }
                }
            }
        }
        Value::Iter(index_var, create, next, extra_init) => {
            // Record the index variable as used at this point, then recurse into all
            // three sub-expressions so variables read inside create/next/extra_init
            // get correct last_use values.  Without this, index variables that are only
            // read inside the Iter sub-expressions keep last_use = 0 and appear dead at
            // birth, allowing assign_slots to place a later variable at the same slot
            // and corrupting the loop counter at runtime.
            let v = *index_var as usize;
            if v < function.variables.len() {
                function.variables[v].last_use = function.variables[v].last_use.max(*seq);
            }
            *seq += 1;
            compute_intervals(create, function, free_text_nr, free_ref_nr, seq, depth + 1);
            compute_intervals(next, function, free_text_nr, free_ref_nr, seq, depth + 1);
            compute_intervals(
                extra_init,
                function,
                free_text_nr,
                free_ref_nr,
                seq,
                depth + 1,
            );
        }
        Value::If(test, t_val, f_val) => {
            compute_intervals(test, function, free_text_nr, free_ref_nr, seq, depth + 1);
            compute_intervals(t_val, function, free_text_nr, free_ref_nr, seq, depth + 1);
            compute_intervals(f_val, function, free_text_nr, free_ref_nr, seq, depth + 1);
        }
        Value::Call(op_nr, args) => {
            // OpFreeText / OpFreeRef are implicit last uses of the variable they free.
            if (*op_nr == free_text_nr || *op_nr == free_ref_nr)
                && args.len() == 1
                && let Value::Var(v) = &args[0]
            {
                let v = *v as usize;
                if v < function.variables.len() {
                    function.variables[v].last_use = function.variables[v].last_use.max(*seq);
                }
                *seq += 1;
                return;
            }
            for arg in args {
                compute_intervals(arg, function, free_text_nr, free_ref_nr, seq, depth + 1);
            }
            *seq += 1;
        }
        Value::CallRef(v_nr, args) => {
            for a in args {
                compute_intervals(a, function, free_text_nr, free_ref_nr, seq, depth + 1);
            }
            // Mark the fn-ref variable as used at this point
            function.variables[*v_nr as usize].last_use =
                function.variables[*v_nr as usize].last_use.max(*seq);
            *seq += 1;
        }
        Value::Return(v) | Value::Drop(v) => {
            compute_intervals(v, function, free_text_nr, free_ref_nr, seq, depth + 1);
        }
        Value::Insert(ops) => {
            for op in ops {
                compute_intervals(op, function, free_text_nr, free_ref_nr, seq, depth + 1);
            }
        }
        Value::Break(_) | Value::Continue(_) | Value::Null | Value::Line(_) => {}
        _ => {
            *seq += 1;
        }
    }
}
