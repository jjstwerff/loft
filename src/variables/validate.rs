// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Slot validation and variable table dump: assert no overlapping live intervals,
//! scope-parent analysis, and debug output.

#![allow(clippy::cast_possible_truncation)]

#[cfg(any(debug_assertions, test))]
use crate::data::Value;
use crate::data::{Context, Data, Type};
#[cfg(any(debug_assertions, test))]
use std::collections::HashMap;
use std::io::{Error, Write};

#[cfg(any(debug_assertions, test))]
use super::Variable;
use super::{Function, size};

fn short_type(tp: &Type) -> String {
    match tp {
        Type::Unknown(_) => "?".to_string(),
        Type::Null => "null".to_string(),
        Type::Void | Type::Never => "void".to_string(),
        Type::Integer(_, _, _) => "int".to_string(),
        Type::Boolean => "bool".to_string(),
        Type::Long => "long".to_string(),
        Type::Float => "float".to_string(),
        Type::Single => "single".to_string(),
        Type::Character => "char".to_string(),
        Type::Text(_) => "text".to_string(),
        Type::Keys => "keys".to_string(),
        Type::Enum(t, _, _) => format!("enum({t})"),
        Type::Reference(t, _) => format!("ref({t})"),
        Type::RefVar(inner) => format!("&{}", short_type(inner)),
        Type::Vector(inner, _) => format!("vec<{}>", short_type(inner)),
        Type::Routine(t) => format!("routine({t})"),
        Type::Iterator(inner, _) => format!("iter<{}>", short_type(inner)),
        Type::Sorted(t, _, _) => format!("sorted({t})"),
        Type::Index(t, _, _) => format!("index({t})"),
        Type::Spacial(t, _, _) => format!("spacial({t})"),
        Type::Hash(t, _, _) => format!("hash({t})"),
        Type::Function(_, _, _) => "fn".to_string(),
        Type::Rewritten(inner) => format!("~{}", short_type(inner)),
        Type::Tuple(elems) => {
            let es: Vec<String> = elems.iter().map(short_type).collect();
            format!("({})", es.join(","))
        }
    }
}

/// Build a map from each scope number → its parent scope number, by walking the IR tree.
/// Scopes with no parent (e.g. the root block) are not in the map.
///
/// If a scope number appears more than once in the tree (e.g. a synthetic block sharing
/// a scope number with an outer block), keep the first-seen parent — it is the
/// structurally outermost one.  Never insert a self-loop (`scope == parent`).
#[cfg(any(debug_assertions, test))]
fn build_scope_parents(val: &Value, parent: u16, parents: &mut HashMap<u16, u16>) {
    match val {
        Value::Block(bl) | Value::Loop(bl) => {
            // Guard: never insert a self-loop; keep the first-seen parent.
            if bl.scope != parent {
                parents.entry(bl.scope).or_insert(parent);
            }
            for op in &bl.operators {
                build_scope_parents(op, bl.scope, parents);
            }
        }
        Value::If(cond, t, f) => {
            build_scope_parents(cond, parent, parents);
            build_scope_parents(t, parent, parents);
            build_scope_parents(f, parent, parents);
        }
        Value::Set(_, inner) | Value::Drop(inner) | Value::Return(inner) => {
            build_scope_parents(inner, parent, parents);
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                build_scope_parents(a, parent, parents);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                build_scope_parents(op, parent, parents);
            }
        }
        Value::Iter(_, create, next, extra) => {
            build_scope_parents(create, parent, parents);
            build_scope_parents(next, parent, parents);
            build_scope_parents(extra, parent, parents);
        }
        _ => {}
    }
}

/// Returns true if `ancestor` is a strict ancestor of `child` in the scope tree.
#[cfg(any(debug_assertions, test))]
fn is_scope_ancestor(ancestor: u16, child: u16, parents: &HashMap<u16, u16>) -> bool {
    let mut cur = child;
    let mut steps = 0u32;
    loop {
        assert!(
            steps <= 10_000,
            "is_scope_ancestor: cycle in scope parent map after {steps} steps \
             (ancestor={ancestor}, child={child}, cur={cur}). \
             This indicates build_scope_parents inserted a scope with itself as parent."
        );
        steps += 1;
        match parents.get(&cur) {
            Some(&p) if p == ancestor => return true,
            Some(&p) if p == cur => return false, // self-loop → not an ancestor
            Some(&p) => cur = p,
            None => return false,
        }
    }
}

/// Returns true if scope SA and scope SB can be physically concurrent, i.e., one is an
/// ancestor of the other (or they are equal).  Variables in sibling branches of the IR tree
/// cannot be simultaneously on the stack, so byte-range overlap between them is allowed.
#[cfg(any(debug_assertions, test))]
fn scopes_can_conflict(sa: u16, sb: u16, parents: &HashMap<u16, u16>) -> bool {
    // u16::MAX = "no scope" (global or argument) — always treat as possible conflict.
    if sa == u16::MAX || sb == u16::MAX {
        return true;
    }
    sa == sb || is_scope_ancestor(sa, sb, parents) || is_scope_ancestor(sb, sa, parents)
}

/// Scan `vars` for the first pair of variables whose stack slots AND live intervals both
/// overlap AND whose scopes are in the same execution branch (i.e. one scope is an ancestor
/// of the other).  Variables in sibling branches cannot be simultaneously on the stack.
#[cfg(any(debug_assertions, test))]
pub(super) fn find_conflict(
    vars: &[Variable],
    scope_parents: &HashMap<u16, u16>,
) -> Option<(usize, u16, usize, u16)> {
    for left_idx in 0..vars.len() {
        let left = &vars[left_idx];
        if left.stack_pos == u16::MAX || left.first_def == u32::MAX {
            continue;
        }
        let left_size = size(&left.type_def, &Context::Variable);
        if left_size == 0 {
            continue;
        }
        let left_slot_end = left.stack_pos + left_size;
        for (right_idx, right) in vars.iter().enumerate().skip(left_idx + 1) {
            if right.stack_pos == u16::MAX || right.first_def == u32::MAX {
                continue;
            }
            let right_size = size(&right.type_def, &Context::Variable);
            if right_size == 0 {
                continue;
            }
            let right_slot_end = right.stack_pos + right_size;
            let slots_overlap = left.stack_pos < right_slot_end && right.stack_pos < left_slot_end;
            let intervals_overlap =
                left.first_def <= right.last_use && right.first_def <= left.last_use;
            if slots_overlap && intervals_overlap {
                // Same name + same slot = sequential reuse of one logical variable across
                // block scopes.  The compiler creates a fresh Variable entry per block but
                // assigns it the same slot; the overlap in live ranges is a conservative
                // artefact of compute_intervals, not a real runtime conflict.
                if left.name == right.name && left.stack_pos == right.stack_pos {
                    continue;
                }
                // Variables in sibling (or cousin) scope branches cannot physically overlap:
                // one block exits before the other starts.  The live-interval overlap is an
                // artefact of OpFreeRef/OpFreeText tracking across scope boundaries.
                if !scopes_can_conflict(left.scope, right.scope, scope_parents) {
                    continue;
                }
                // S34 Option A: a work variable moved down to the same slot as an outer
                // variable is marked skip_free so that only the outer variable emits
                // OpFreeRef.  The slot is intentionally aliased — not a real conflict.
                if left.skip_free || right.skip_free {
                    continue;
                }
                return Some((left_idx, left_slot_end, right_idx, right_slot_end));
            }
        }
    }
    None
}

/// Assert that no two variables with overlapping live intervals occupy the same stack slot.
/// Only compiled in debug/test builds; the call site in `codegen.rs` is gated on
/// `#[cfg(any(debug_assertions, test))]`.
/// On failure, logs the full variable table and IR code before panicking.
#[cfg(any(debug_assertions, test))]
pub fn validate_slots(function: &Function, data: &Data, def_nr: u32) {
    // Build scope parent map from the IR tree so find_conflict can skip sibling-branch conflicts.
    let mut scope_parents: HashMap<u16, u16> = HashMap::new();
    build_scope_parents(&data.def(def_nr).code, u16::MAX, &mut scope_parents);

    let vars = &function.variables;
    let Some((left_idx, left_slot_end, right_idx, right_slot_end)) =
        find_conflict(vars, &scope_parents)
    else {
        return;
    };
    let left = &vars[left_idx];
    let right = &vars[right_idx];
    // Log full diagnostics before panicking so the cause is immediately clear.
    eprintln!("\n=== Slot conflict in function '{}' ===\n", function.name);
    eprintln!("  Conflicting pair:");
    eprintln!(
        "  * '{}'  slot [{}, {left_slot_end})  live [{}, {}]",
        left.name, left.stack_pos, left.first_def, left.last_use
    );
    eprintln!(
        "  * '{}'  slot [{}, {right_slot_end})  live [{}, {}]",
        right.name, right.stack_pos, right.first_def, right.last_use
    );
    eprintln!();
    eprintln!(
        "  {:<4} {:<2} {:<20} {:<14} {:<16} {:<12} {:<12} {:<14}",
        "#", "", "name", "type", "scope", "slot", "pre", "live"
    );
    eprintln!("  {}", "-".repeat(96));
    for (idx, var) in vars.iter().enumerate() {
        let vs = size(&var.type_def, &Context::Variable);
        let slot_str = if var.stack_pos == u16::MAX {
            "-".to_string()
        } else {
            format!("[{}, {})", var.stack_pos, var.stack_pos + vs)
        };
        let pre_str = if var.pre_assigned_pos == u16::MAX || var.pre_assigned_pos == var.stack_pos {
            String::new()
        } else {
            format!("[{}, {})", var.pre_assigned_pos, var.pre_assigned_pos + vs)
        };
        let live_str = if var.first_def == u32::MAX {
            "-".to_string()
        } else {
            format!("[{}, {}]", var.first_def, var.last_use)
        };
        let mark = if idx == left_idx || idx == right_idx {
            "*"
        } else {
            " "
        };
        // Show scope number; append "L seq:[s..e)" for loop scopes so physical-TOS
        // decisions are immediately visible without reading the full IR.
        let scope_str = if var.scope == u16::MAX {
            "-".to_string()
        } else if let Some((s, e)) = function.loop_seq_range(var.scope) {
            format!("{}L seq:[{}..{})", var.scope, s, e)
        } else {
            var.scope.to_string()
        };
        eprintln!(
            "  {idx:<4} {mark:<2} {:<20} {:<14} {scope_str:<16} {slot_str:<12} {pre_str:<12} {live_str:<14}",
            var.name,
            short_type(&var.type_def),
        );
    }
    eprintln!();
    eprintln!("=== IR code for '{}' ===", function.name);
    let mut buf: Vec<u8> = Vec::new();
    let mut vars_copy = Function::copy(function);
    if data
        .show_code(&mut buf, &mut vars_copy, &data.def(def_nr).code, 0, true)
        .is_ok()
    {
        eprintln!("{}", String::from_utf8_lossy(&buf));
    }
    panic!(
        "Variables '{}' (slot [{}, {left_slot_end}), live [{}, {}]) and '{}' (slot [{}, {right_slot_end}), live [{}, {}]) \
         share a stack slot while both live in function '{}'",
        left.name,
        left.stack_pos,
        left.first_def,
        left.last_use,
        right.name,
        right.stack_pos,
        right.first_def,
        right.last_use,
        function.name,
    );
}

/// Write the variable table for `function` to `f`.
///
/// Columns: index, argument flag, name, short type, scope, stack slot range, live interval.
/// Variables with no slot (`stack_pos == u16::MAX`) or no definition are still listed.
///
/// # Errors
/// Propagates any I/O error from the writer.
pub fn dump_variables(f: &mut dyn Write, function: &Function, data: &Data) -> Result<(), Error> {
    writeln!(
        f,
        "  {:<4} {:<4} {:<20} {:<14} {:<6} {:<12} live",
        "#", "arg", "name", "type", "scope", "slot"
    )?;
    writeln!(f, "  {}", "-".repeat(70))?;
    for (idx, var) in function.variables.iter().enumerate() {
        let vs = size(&var.type_def, &Context::Variable);
        let slot_str = if var.stack_pos == u16::MAX {
            "-".to_string()
        } else {
            format!("[{}, {})", var.stack_pos, var.stack_pos + vs)
        };
        let live_str = if var.first_def == u32::MAX {
            "-".to_string()
        } else {
            format!("[{}, {}]", var.first_def, var.last_use)
        };
        let scope_str = if var.scope == u16::MAX {
            "-".to_string()
        } else {
            var.scope.to_string()
        };
        let arg_flag = if var.argument { "arg" } else { "" };
        let type_str = short_type(&var.type_def);
        writeln!(
            f,
            "  {idx:<4} {arg_flag:<4} {:<20} {type_str:<14} {scope_str:<6} {slot_str:<12} {live_str}",
            var.name
        )?;
        let _ = data; // reserved for future type name resolution
    }
    writeln!(f)
}
