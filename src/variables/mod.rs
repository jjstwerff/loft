// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(clippy::cast_possible_truncation)]

mod intervals;
mod slots;
mod validate;

pub use intervals::compute_intervals;
pub use slots::assign_slots;
pub use validate::dump_variables;
#[cfg(any(debug_assertions, test))]
pub use validate::validate_slots;

use crate::data::{Context, Data, Type, Value};
use crate::diagnostics::{Level, diagnostic_format};
use crate::keys::DbRef;
use crate::lexer::Lexer;
/**
This administrates variables and scopes for a specific function.
- The first scope (0) is for function arguments.
- Variables might exist in multiple scopes but not with different types.
- We allow for variables to move to a higher scope.
*/
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{Display, Formatter};

// Iterator details on each for loop inside the current function
#[derive(Debug, Clone)]
struct Iterator {
    inside: u16,       // iterator number or MAX when top level loop
    variable: u16,     // variable number
    on: u8,            // structure type and direction
    db_tp: u16,        // database type of this structure
    value: Box<Value>, // code to gain the structure or Value::Null for a range
    /// The original user-written collection variable number being iterated.
    /// For vector loops the iterator works on a unique temp copy; this field
    /// stores the original var so mutation of the original can be detected.
    /// `u16::MAX` when the iterated expression is not a simple variable
    /// (e.g. a struct-field access like `db.map`).
    coll_var: u16,
    counter: u16, // variable number or MAX when it is not used
}

// This is created for every variable instance, even if those are of the same name.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct Variable {
    name: String,
    type_def: Type,
    source: (u32, u32),
    scope: u16,
    stack_pos: u16,
    uses: u16,
    uses_at_write: u16,
    write_source: (u32, u32),
    argument: bool,
    defined: bool,
    const_param: bool,
    /// Whether this variable's stack storage has been initialised by codegen.
    /// Set to `true` when the first-allocation init opcodes are emitted (A6.3).
    /// Arguments are pre-allocated by the caller, so they start as `true`.
    pub stack_allocated: bool,
    /// When true, `get_free_vars` must not emit `OpFreeRef` for this variable.
    /// Set by `clean_work_refs` for work-ref temporaries that have been re-purposed
    /// and must not be freed at scope exit (A14 replacement for type-mutation hack).
    pub skip_free: bool,
    /// Sequence number of the first `Value::Set` node for this variable; `u32::MAX` = never defined.
    pub first_def: u32,
    /// Sequence number of the last `Value::Var` (or implicit `OpFreeText`/`OpFreeRef`) for this variable.
    pub last_use: u32,
    /// Slot assigned by `assign_slots` before codegen may override it via `set_stack_pos`.
    /// `u16::MAX` means `assign_slots` has not run yet.  Shown as `pre:` in `validate_slots`
    /// diagnostics when it differs from the final `stack_pos`.
    pub pre_assigned_pos: u16,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub file: String,
    unique: u16,
    current_loop: u16,
    loops: Vec<Iterator>,
    variables: Vec<Variable>,
    work_text: u16,
    work_ref: u16,
    // Separate counter for work-refs allocated by `add_defaults` for recursive
    // self-calls on second pass.  Uses `__rref_N` names so it does not consume
    // `__ref_N` slots that the outer function's return-value work-ref needs.
    work_rref: u16,
    // Separate counter for vector-db work-refs created by `vector_db()`.
    // `vector_db` only runs on the second pass (first_pass guard), so it cannot
    // use the shared `work_ref` counter: that would shift the counter relative to
    // the first pass and break `ref_return`'s name-based attr matching.
    work_vdb: u16,
    // Work variables for texts
    work_texts: BTreeSet<u16>,
    // Work variables for stores
    work_refs: BTreeSet<u16>,
    // Subset of work_refs: inline-ref temporaries created by parse_part to capture
    // the result of a ref-returning method call that is immediately chained (e.g.
    // `p.shifted(1.0, 0.0).x`).  These need their preamble null-init inserted
    // AFTER the first user statement so they appear after user-scope vars in var_order
    // and are therefore freed BEFORE them — satisfying the database LIFO invariant.
    inline_ref_vars: BTreeSet<u16>,
    // The names store only the last known instance of this variable in the function.
    names: HashMap<String, u16>,
    // Scope numbers that correspond to loop bodies (Value::Loop), i.e. scopes whose
    // variables are freed by OpFreeStack when the loop exits.  If-block scopes
    // (Value::Block) are NOT in this set; their variables live until function return.
    // Used by assign_slots to compute the physical TOS accurately.
    loop_scopes: HashSet<u16>,
    // Maps each loop-body scope number → (seq_start, seq_end) where seq_start / seq_end
    // are the `compute_intervals` sequence counters immediately before / after the loop
    // body is traversed.  assign_slots uses this to decide whether a dead loop-scope
    // variable j is still physically present at i.first_def:
    //   - If i.first_def < seq_end(j.scope): the loop's FreeStack fires AFTER i.first_def
    //     → j's bytes are still on the physical stack at i.first_def (include in tos_estimate).
    //   - If i.first_def >= seq_end(j.scope): the loop exited before i.first_def
    //     → j's bytes were freed by FreeStack (exclude from tos_estimate).
    loop_seq_ranges: HashMap<u16, (u32, u32)>,
    // Maps each scope number to the source construct that introduced it: "block", "for", "if", etc.
    scope_origins: HashMap<u16, &'static str>,
    pub done: bool,
    pub logging: bool,
}

impl Display for Function {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for v in &self.variables {
            f.write_fmt(format_args!("{v:?}\n"))?;
        }
        Ok(())
    }
}

impl Function {
    pub fn new(name: &str, file: &str) -> Self {
        Function {
            name: name.to_string(),
            file: file.to_string(),
            unique: 0,
            current_loop: u16::MAX,
            loops: Vec::new(),
            work_text: 0,
            work_ref: 0,
            work_rref: 0,
            work_vdb: 0,
            variables: Vec::new(),
            work_texts: BTreeSet::new(),
            work_refs: BTreeSet::new(),
            inline_ref_vars: BTreeSet::new(),
            names: HashMap::new(),
            loop_scopes: HashSet::new(),
            loop_seq_ranges: HashMap::new(),
            scope_origins: HashMap::new(),
            logging: false,
            done: false,
        }
    }

    pub fn append(&mut self, other: &mut Function) {
        self.current_loop = u16::MAX;
        self.logging = other.logging;
        self.unique = 0;
        other.unique = 0;
        self.loops.clear();
        self.loops.append(&mut other.loops);
        self.variables.clear();
        self.variables.append(&mut other.variables);
        for v in &mut self.variables {
            v.uses = 0;
        }
        self.work_text = 0;
        self.work_ref = 0;
        self.work_rref = 0;
        self.work_vdb = 0;
        self.work_texts.clear();
        self.work_refs.clear();
        self.inline_ref_vars.clear();
        self.inline_ref_vars.clone_from(&other.inline_ref_vars);
        self.names.clear();
        self.names.clone_from(&other.names);
        other.names.clear();
        self.loop_scopes.clear();
        self.loop_scopes.clone_from(&other.loop_scopes);
        self.loop_seq_ranges.clear();
        self.loop_seq_ranges.clone_from(&other.loop_seq_ranges);
        self.scope_origins.clear();
        self.scope_origins.clone_from(&other.scope_origins);
    }

    pub fn copy(other: &Function) -> Self {
        Function {
            name: other.name.clone(),
            file: other.file.clone(),
            current_loop: u16::MAX,
            unique: 0,
            loops: other.loops.clone(),
            variables: other.variables.clone(),
            work_text: 0,
            work_ref: 0,
            work_rref: 0,
            work_vdb: 0,
            work_texts: BTreeSet::new(),
            work_refs: BTreeSet::new(),
            inline_ref_vars: other.inline_ref_vars.clone(),
            names: other.names.clone(),
            loop_scopes: other.loop_scopes.clone(),
            loop_seq_ranges: other.loop_seq_ranges.clone(),
            scope_origins: other.scope_origins.clone(),
            logging: other.logging,
            done: other.done,
        }
    }

    pub fn start_loop(&mut self) -> u16 {
        self.loops.push(Iterator {
            inside: self.current_loop,
            variable: u16::MAX,
            on: 0,
            db_tp: u16::MAX,
            value: Box::new(Value::Null),
            coll_var: u16::MAX,
            counter: u16::MAX,
        });
        self.current_loop = self.loops.len() as u16 - 1;
        self.current_loop
    }

    pub fn loop_var(&mut self, variable: u16) {
        self.loops[self.current_loop as usize].variable = variable;
    }

    pub fn set_loop(&mut self, on: u8, db_tp: u16, value: &Value) {
        let l = &mut self.loops[self.current_loop as usize];
        l.on = on;
        l.db_tp = db_tp;
        *l.value = value.clone();
        // Auto-extract coll_var when the iterated expression is a plain variable.
        // For vector loops this will be overridden by set_coll_var() because the
        // iterator works on a unique temp copy, not the original user variable.
        l.coll_var = if let Value::Var(v) = value {
            *v
        } else {
            u16::MAX
        };
    }

    /// Override the iterated collection variable after `set_loop`.
    /// Called from `parse_for` for vector loops where a unique temp copy is created:
    /// the iterator runs over the copy, but the user-visible variable is `orig_var`.
    pub fn set_coll_var(&mut self, orig_var: u16) {
        self.loops[self.current_loop as usize].coll_var = orig_var;
    }

    /// Override the iterated collection `value` expression after `set_loop`.
    /// Called from `parse_for` for vector loops so that `is_iterated_value` can compare
    /// the original user-written expression (e.g. `db.items`) instead of the internal
    /// temp-copy variable that `set_loop` records.
    pub fn set_coll_value(&mut self, orig_value: Value) {
        *self.loops[self.current_loop as usize].value = orig_value;
    }

    /// Returns true when `var_nr` is the collection variable of any currently active
    /// for-loop (including outer loops).  Used to detect unsafe mutation during iteration.
    pub fn is_iterated_var(&self, var_nr: u16) -> bool {
        if var_nr == u16::MAX {
            return false;
        }
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].coll_var == var_nr {
                return true;
            }
            c = self.loops[c as usize].inside;
        }
        false
    }

    /// Returns true when `val` structurally matches the iterated-collection expression of
    /// any currently active for-loop.  Catches field-access cases like `db.items` where
    /// `coll_var` is `u16::MAX` (no single variable covers the expression).
    pub fn is_iterated_value(&self, val: &Value) -> bool {
        if matches!(val, Value::Null) {
            return false;
        }
        let mut c = self.current_loop;
        while c != u16::MAX {
            if *self.loops[c as usize].value == *val {
                return true;
            }
            c = self.loops[c as usize].inside;
        }
        false
    }

    /**
    Stop the current loop.
    # Panics
    When this loop is not started.
    */
    pub fn finish_loop(&mut self, loop_nr: u16) {
        assert_eq!(self.current_loop, loop_nr, "Incorrect loop finish");
        self.current_loop = self.loops[self.current_loop as usize].inside;
    }

    pub fn loop_count(&mut self, count_var: u16) {
        self.loops[self.current_loop as usize].counter = count_var;
    }

    pub fn loop_counter(&mut self) -> u16 {
        self.loops[self.current_loop as usize].counter
    }

    pub fn loop_nr(&self, variable: &str) -> u16 {
        let mut c = self.current_loop;
        let mut nr = 0;
        while c != u16::MAX
            && self.variables[self.loops[c as usize].variable as usize].name != variable
        {
            c = self.loops[c as usize].inside;
            nr += 1;
        }
        nr
    }

    pub fn loop_on(&self, var_nr: u16) -> u8 {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return self.loops[c as usize].on;
            }
            c = self.loops[c as usize].inside;
        }
        0
    }

    pub fn loop_value(&self, var_nr: u16) -> &Value {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return &self.loops[c as usize].value;
            }
            c = self.loops[c as usize].inside;
        }
        &Value::Null
    }

    pub fn loop_db_tp(&self, var_nr: u16) -> u16 {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return self.loops[c as usize].db_tp;
            }
            c = self.loops[c as usize].inside;
        }
        u16::MAX
    }

    /// Number of variables declared in this function (arguments + locals).
    #[must_use]
    pub fn count(&self) -> u16 {
        self.variables.len() as u16
    }

    pub fn name(&self, var_nr: u16) -> &str {
        if var_nr as usize >= self.variables.len() {
            return "??";
        }
        &self.variables[var_nr as usize].name
    }

    pub fn set_scope(&mut self, var_nr: u16, scope: u16) {
        assert!((var_nr as usize) < self.variables.len(), "Unknown variable");
        assert_eq!(
            self.variables[var_nr as usize].scope,
            u16::MAX,
            "Variable has a scope"
        );
        self.variables[var_nr as usize].scope = scope;
        self.done = true;
    }

    /// Mark a scope number as corresponding to a loop body (`Value::Loop`).
    /// Variables in loop scopes are freed by `OpFreeStack` when the loop exits;
    /// if-block scopes (`Value::Block`) are NOT marked and live until function return.
    pub fn mark_loop_scope(&mut self, scope: u16) {
        self.loop_scopes.insert(scope);
    }

    /// Returns true if `scope` is a loop-body scope (variables freed by `OpFreeStack`).
    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn is_loop_scope(&self, scope: u16) -> bool {
        self.loop_scopes.contains(&scope)
    }

    /// Record the seq-number range [`seq_start`, `seq_end`) for a loop-body scope.
    /// Called by `compute_intervals` when it finishes traversing a `Value::Loop`.
    pub fn record_loop_range(&mut self, scope: u16, seq_start: u32, seq_end: u32) {
        self.loop_seq_ranges.insert(scope, (seq_start, seq_end));
    }

    #[cfg(any(debug_assertions, test))]
    pub fn loop_seq_range(&self, scope: u16) -> Option<(u32, u32)> {
        self.loop_seq_ranges.get(&scope).copied()
    }

    pub fn record_scope_origin(&mut self, scope: u16, name: &'static str) {
        let short = match name {
            "For block" => "for",
            "For loop" | "Slice materialise" | "For comprehension" => "loop",
            "Formatted string" => "fmt",
            "" => "if",
            o => o,
        };
        self.scope_origins.entry(scope).or_insert(short);
    }

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn scope_origin(&self, scope: u16) -> &'static str {
        self.scope_origins.get(&scope).copied().unwrap_or("block")
    }

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn first_def(&self, var_nr: u16) -> u32 {
        self.variables[var_nr as usize].first_def
    }

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn last_use(&self, var_nr: u16) -> u32 {
        self.variables[var_nr as usize].last_use
    }

    pub fn scope(&self, var_nr: u16) -> u16 {
        if var_nr as usize >= self.variables.len() {
            return u16::MAX;
        }
        self.variables[var_nr as usize].scope
    }

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn size(&self, var_nr: u16, context: &Context) -> u16 {
        size(&self.variables[var_nr as usize].type_def, context)
    }

    pub fn tp(&self, var_nr: u16) -> &Type {
        if var_nr as usize >= self.variables.len() {
            &Type::Null
        } else {
            &self.variables[var_nr as usize].type_def
        }
    }

    pub fn is_independent(&self, var_nr: u16) -> bool {
        let d = self.variables[var_nr as usize].type_def.depend();
        d.is_empty() || (d.len() == 1 && d[0] == var_nr)
    }

    /// Remove a lifetime dependency for this variable.
    pub fn make_independent(&mut self, var_nr: u16, remove: u16) {
        match &mut self.variables[var_nr as usize].type_def {
            Type::Reference(_, to) | Type::Enum(_, _, to) | Type::Vector(_, to) => {
                if let Some(pos) = to.iter().position(|x| x == &remove) {
                    to.remove(pos);
                }
            }
            _ => (),
        }
    }

    pub fn depend(&mut self, var_nr: u16, on: u16) {
        if on != u16::MAX {
            self.variables[var_nr as usize].type_def =
                self.variables[var_nr as usize].type_def.depending(on);
        }
    }

    pub fn uses(&self, var_nr: u16) -> u16 {
        self.variables[var_nr as usize].uses
    }

    pub fn is_defined(&self, var_nr: u16) -> bool {
        self.variables[var_nr as usize].defined
    }

    pub fn stack(&self, var_nr: u16) -> u16 {
        self.variables[var_nr as usize].stack_pos
    }

    /// Return the lowest byte offset at which a new variable slot can safely be placed —
    /// i.e. the maximum end-byte of all variables that already have an assigned slot.
    ///
    /// Currently unused in production code.  Retained for Step 3 of the stack-slot
    /// assignment plan (`assign_slots` in ASSIGNMENT.md): the linear-scan pass will use
    /// this to find the next free position when no expired slot is available for reuse.
    ///
    /// Note: a naive guard that advances `stack.position` to this value inside
    /// `generate_set` was attempted and reverted — it broke the bridging invariant
    /// (compile-time `stack.position` diverged from the runtime stack pointer).  This
    /// function is correct; the problem was the call site, not the computation.
    pub fn set_stack(&mut self, var_nr: u16, pos: u16) {
        self.variables[var_nr as usize].stack_pos = pos;
    }

    pub fn in_use(&mut self, var_nr: u16, plus: bool) {
        if plus {
            self.variables[var_nr as usize].uses += 1;
        } else {
            self.variables[var_nr as usize].uses -= 1;
        }
    }

    pub fn defined(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].defined = true;
    }

    /// Check for dead assignment (overwritten before read) and update write tracking.
    /// Call this on every `=` assignment to a user variable during the second pass.
    pub fn track_write(&mut self, var_nr: u16, lexer: &mut Lexer) {
        let var = &self.variables[var_nr as usize];
        if var.name.starts_with('_') || var.name.contains('#') || var.const_param {
            return;
        }
        if var.write_source != (0, 0) && var.uses == var.uses_at_write {
            // Variable was written before but not read since — dead assignment
            let name = var.name.clone();
            let prev_source = var.write_source;
            lexer.to(prev_source);
            diagnostic!(
                lexer,
                Level::Warning,
                "Dead assignment — '{}' is overwritten before being read",
                name,
            );
        }
        let var = &mut self.variables[var_nr as usize];
        var.uses_at_write = var.uses;
        var.write_source = lexer.at();
    }

    /// Save write-tracking state for all variables, then clear pending writes.
    /// Call before entering a branch — the branch should not see pre-branch writes
    /// as "unread" because the branch might not execute.
    pub fn save_and_clear_write_state(&self) -> Vec<(u16, (u32, u32))> {
        self.variables
            .iter()
            .map(|v| (v.uses_at_write, v.write_source))
            .collect()
    }

    /// Restore write-tracking state for all variables (call after leaving a branch).
    pub fn restore_write_state(&mut self, state: &[(u16, (u32, u32))]) {
        for (i, (uses_at_write, write_source)) in state.iter().enumerate() {
            if i < self.variables.len() {
                self.variables[i].uses_at_write = *uses_at_write;
                self.variables[i].write_source = *write_source;
            }
        }
    }

    /// Clear all pending write tracking (no variable has an "unread write").
    pub fn clear_write_state(&mut self) {
        for v in &mut self.variables {
            v.write_source = (0, 0);
        }
    }

    pub fn exists(&self, var_nr: u16) -> bool {
        var_nr < self.variables.len() as u16
    }

    pub fn name_exists(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    pub fn arguments(&self) -> Vec<u16> {
        let mut arg = Vec::new();
        for (v_nr, v) in self.variables.iter().enumerate() {
            if v.argument {
                arg.push(v_nr as u16);
            }
        }
        arg
    }

    pub fn var(&self, name: &str) -> u16 {
        if let Some(nr) = self.names.get(name) {
            return *nr;
        }
        u16::MAX
    }

    pub fn next_var(&self) -> u16 {
        self.variables.len() as u16
    }

    /// Set a name→variable mapping, returning the previous mapping (if any).
    /// Used by match arm bindings (S15) to alias a user-visible field name
    /// to a per-arm unique variable.
    pub fn set_name(&mut self, name: &str, var_nr: u16) -> Option<u16> {
        self.names.insert(name.to_string(), var_nr)
    }

    /// Remove a name→variable mapping.
    pub fn remove_name(&mut self, name: &str) {
        self.names.remove(name);
    }

    pub fn unique(&mut self, name: &str, type_def: &Type, lexer: &mut Lexer) -> u16 {
        self.unique += 1;
        self.add_variable(&format!("_{name}_{}", self.unique), type_def, lexer)
    }

    pub fn add_variable(&mut self, name: &str, type_def: &Type, lexer: &mut Lexer) -> u16 {
        // Due to 2 passes through the code, we will add the same variable a second time.
        if let Some(nr) = self.names.get(name) {
            if self.variables[*nr as usize].type_def.is_unknown() {
                self.variables[*nr as usize].type_def = type_def.clone();
            }
            return *nr;
        }
        self.new_var(name, type_def, lexer)
    }

    /// Create an exact copy of a variable, used to duplicate them when reused in later scopes.
    pub fn copy_variable(&mut self, var: u16) -> u16 {
        let v = self.variables.len() as u16;
        self.variables.push(Variable {
            name: self.variables[var as usize].name.clone(),
            type_def: self.variables[var as usize].type_def.clone(),
            source: self.variables[var as usize].source,
            scope: u16::MAX,
            stack_pos: u16::MAX,
            uses: 1,
            uses_at_write: 0,
            write_source: (0, 0),
            argument: false,
            defined: self.variables[var as usize].defined,
            const_param: self.variables[var as usize].const_param,
            stack_allocated: false,
            skip_free: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
        });
        v
    }

    fn new_var(&mut self, name: &str, type_def: &Type, lexer: &mut Lexer) -> u16 {
        let v = self.variables.len() as u16;
        if !self.names.contains_key(name) {
            self.names.insert(name.to_string(), v);
        }
        self.variables.push(Variable {
            name: name.to_string(),
            type_def: type_def.clone(),
            source: lexer.at(),
            scope: u16::MAX,
            stack_pos: u16::MAX,
            uses: 1,
            uses_at_write: 0,
            write_source: (0, 0),
            argument: false,
            defined: false,
            const_param: false,
            stack_allocated: false,
            skip_free: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
        });
        v
    }

    #[cfg(test)]
    pub fn add_unique(&mut self, prefix: &str, type_def: &Type, scope: u16) -> u16 {
        let v = self.variables.len() as u16;
        self.variables.push(Variable {
            name: format!("_{prefix}_{v}"),
            type_def: type_def.clone(),
            source: (0, 0),
            scope,
            stack_pos: u16::MAX,
            uses: 1,
            uses_at_write: 0,
            write_source: (0, 0),
            argument: false,
            defined: true,
            const_param: false,
            stack_allocated: false,
            skip_free: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
        });
        v
    }

    pub fn change_var_type(
        &mut self,
        var_nr: u16,
        type_def: &Type,
        data: &Data,
        lexer: &mut Lexer,
    ) -> bool {
        let var_tp = &self.variables[var_nr as usize].type_def;
        if type_def.is_unknown() || var_tp.is_equal(type_def) {
            for on in type_def.depend() {
                self.depend(var_nr, on);
            }
            return self.is_new(var_nr);
        }
        if let (Type::Vector(tp, _), Type::Vector(to, _)) = (var_tp, type_def) {
            if to.is_unknown() {
                return self.is_new(var_nr);
            }
            if !tp.is_unknown() {
                diagnostic!(
                    lexer,
                    Level::Error,
                    "Variable '{}' cannot change type from {} to {}; use a new variable name or cast with 'as'",
                    self.variables[var_nr as usize].name,
                    self.variables[var_nr as usize].type_def.name(data),
                    type_def.name(data)
                );
            }
        } else if !var_tp.is_unknown() {
            if let Type::RefVar(in_tp) = var_tp
                && in_tp.is_equal(type_def)
            {
                return self.is_new(var_nr);
            }
            diagnostic!(
                lexer,
                Level::Error,
                "Variable '{}' cannot change type from {} to {}; use a new variable name or cast with 'as'",
                self.name(var_nr),
                self.variables[var_nr as usize].type_def.name(data),
                type_def.name(data)
            );
        }
        self.variables[var_nr as usize].type_def = type_def.clone();
        true
    }

    fn is_new(&self, var_nr: u16) -> bool {
        self.variables[var_nr as usize].uses == 0
    }

    pub fn become_argument(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].argument = true;
        self.variables[var_nr as usize].defined = true;
        self.variables[var_nr as usize].stack_allocated = true;
    }

    pub fn is_argument(&self, var_nr: u16) -> bool {
        self.variables[var_nr as usize].argument
    }

    pub fn set_const_param(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].const_param = true;
    }

    pub fn is_const_param(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].const_param
    }

    /// Returns the appropriate error noun for a const-modification diagnostic.
    /// Parameters say "const parameter"; local variables say "const variable".
    pub fn const_kind(&self, var_nr: u16) -> &'static str {
        if (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].argument {
            "const parameter"
        } else {
            "const variable"
        }
    }

    pub fn var_source(&self, var_nr: u16) -> (u32, u32) {
        self.variables[var_nr as usize].source
    }

    pub fn test_used(&self, lexer: &mut Lexer, data: &Data) {
        for var in &self.variables {
            if var.name.starts_with('_') || var.name.contains('#') {
                continue;
            }
            if var.uses == 0 && data.def_nr(&var.name) == u32::MAX {
                lexer.to(var.source);
                diagnostic!(
                    lexer,
                    Level::Warning,
                    "{} {} is never read",
                    if var.argument {
                        "Parameter"
                    } else {
                        "Variable"
                    },
                    var.name,
                );
            }
        }
    }

    pub fn work_text(&mut self, lexer: &mut Lexer) -> u16 {
        let n = format!("__work_{}", self.work_text + 1);
        self.work_text += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            *nr
        } else {
            self.add_variable(&n, &Type::Text(Vec::new()), lexer)
        };
        self.work_texts.insert(v);
        v
    }

    pub fn work_ref(&self) -> u16 {
        self.work_ref
    }

    pub fn clean_work_refs(&mut self, work_ref: u16) {
        for w in work_ref..self.work_ref {
            let n = format!("__ref_{}", w + 1);
            let v_nr = self.var(&n);
            // Mark skip_free so get_free_vars does not emit OpFreeRef for this variable.
            // with this explicit flag, keeping the type_def intact for downstream passes.
            self.variables[v_nr as usize].skip_free = true;
        }
    }

    pub fn work_refs(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        let n = format!("__ref_{}", self.work_ref + 1);
        self.work_ref += 1;
        let mut v = if let Some(nr) = self.names.get(&n) {
            *nr
        } else {
            u16::MAX
        };
        if v == u16::MAX {
            v = self.add_variable(&n, tp, lexer);
        } else {
            self.set_type(v, tp.clone());
            self.variables[v as usize].source = lexer.at();
        }
        self.work_refs.insert(v);
        v
    }

    /// Like `work_refs` but uses a separate `__rref_N` counter/namespace.
    /// Used by `add_defaults` for work-refs allocated for recursive self-calls
    /// on the second pass.  This prevents the `__ref_N` counter from being
    /// consumed by those recursive-call temporaries, so the outer function's
    /// return-value work-ref continues to receive the same `__ref_N` name it
    /// got on the first pass — allowing `ref_return` to find the name match
    /// and reuse the existing attribute instead of adding a new one.
    pub fn work_refs_recursive(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        let n = format!("__rref_{}", self.work_rref + 1);
        self.work_rref += 1;
        let v = self.add_variable(&n, tp, lexer);
        self.work_refs.insert(v);
        v
    }

    /// Work-ref for `vector_db()` — uses a separate `__vdb_N` counter/namespace.
    /// `vector_db` only runs on the second pass (it is guarded by `!first_pass`),
    /// so it must NOT share the `work_ref` / `__ref_N` counter with `add_defaults`.
    /// Using a distinct counter prevents the name-shift that would cause
    /// `ref_return` to fail its name-based attr match and add a spurious attr.
    /// These variables are inserted into `work_refs` so they receive null-inits.
    pub fn work_vec_db(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        let n = format!("__vdb_{}", self.work_vdb + 1);
        self.work_vdb += 1;
        let v = self.add_variable(&n, tp, lexer);
        self.work_refs.insert(v);
        v
    }

    /// Mark `v` as an inline-ref temporary (created by `parse_part` for chained
    /// ref-returning calls).  These get their null-init inserted AFTER the first
    /// user statement in `parse_code` so they appear in `var_order` after user-scope
    /// reference variables, giving the correct LIFO-reversed free order.
    pub fn mark_inline_ref(&mut self, v: u16) {
        self.inline_ref_vars.insert(v);
    }

    pub fn is_inline_ref(&self, v: u16) -> bool {
        self.inline_ref_vars.contains(&v)
    }

    /// Returns true if this work-ref variable should be skipped when emitting `OpFreeRef`.
    /// Set by `clean_work_refs` for ref variables that were re-assigned to a different type
    /// and must not be freed at scope exit.
    /// Returns true if `get_free_vars` must not emit `OpFreeRef` for this variable.
    /// Set by `clean_work_refs` for work-ref temporaries that are re-purposed after use.
    pub fn is_skip_free(&self, v: u16) -> bool {
        self.variables[v as usize].skip_free
    }

    /// Mark a variable so that `get_free_vars` will not emit `OpFreeRef` for it.
    /// Used for borrowed references (e.g. par-loop result variables that point
    /// into the result vector store).
    pub fn set_skip_free(&mut self, v: u16) {
        self.variables[v as usize].skip_free = true;
    }

    pub fn inline_ref_references(&self) -> Vec<u16> {
        self.inline_ref_vars.iter().copied().collect()
    }

    pub fn work_texts(&self) -> Vec<u16> {
        let mut res = Vec::new();
        for v in &self.work_texts {
            res.push(*v);
        }
        res
    }

    pub fn work_references(&self) -> Vec<u16> {
        let mut res = Vec::new();
        for v in &self.work_refs {
            res.push(*v);
        }
        res
    }

    /// Set the pre-assigned stack position for `var`.  Called once per argument during
    /// argument layout in `def_code`; the caller advances `stack.position` separately.
    pub fn set_stack_pos(&mut self, var: u16, pos: u16) {
        self.variables[var as usize].stack_pos = pos;
    }

    pub fn set_type(&mut self, var_nr: u16, tp: Type) {
        self.variables[var_nr as usize].type_def = tp;
    }

    pub fn var_type(&self, var_nr: u16) -> &Type {
        &self.variables[var_nr as usize].type_def
    }

    /// Returns `true` when codegen has already emitted the first-allocation init opcodes
    /// for this variable (e.g. `OpText`, `OpConvRefFromNull`).  Used by A6.3 to replace
    /// the `stack_pos == u16::MAX` first-assignment guard in `generate_set`.
    pub fn is_stack_allocated(&self, var_nr: u16) -> bool {
        self.variables[var_nr as usize].stack_allocated
    }

    /// Mark `var_nr` as having been allocated on the stack (call once per variable,
    /// when the first-allocation init opcodes are emitted in `generate_set`).
    pub fn set_stack_allocated(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].stack_allocated = true;
    }
}

pub fn size(tp: &Type, context: &Context) -> u16 {
    match tp {
        Type::Integer(min, max)
            if context == &Context::Constant && i64::from(*max) - i64::from(*min) <= 256 =>
        {
            1
        }
        Type::Integer(min, max)
            if context == &Context::Constant && i64::from(*max) - i64::from(*min) <= 65536 =>
        {
            2
        }
        Type::Boolean | Type::Enum(_, false, _) => 1,
        Type::Integer(_, _) | Type::Single | Type::Function(_, _) | Type::Character => 4,
        Type::Long | Type::Float => 8,
        Type::Text(_) if context == &Context::Variable => size_of::<String>() as u16,
        Type::Text(_) => size_of::<&str>() as u16,
        Type::RefVar(_)
        | Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Sorted(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Spacial(_, _, _) => size_of::<DbRef>() as u16,
        _ => 0,
    }
}
