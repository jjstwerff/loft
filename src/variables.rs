// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(clippy::cast_possible_truncation)]
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
use std::io::{Error, Write};

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
            // A14: replaced the previous type-mutation hack (setting type to Reference(0,[0]))
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

fn short_type(tp: &Type) -> String {
    match tp {
        Type::Unknown(_) => "?".to_string(),
        Type::Null => "null".to_string(),
        Type::Void => "void".to_string(),
        Type::Integer(_, _) => "int".to_string(),
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
        Type::Function(_, _) => "fn".to_string(),
        Type::Rewritten(inner) => format!("~{}", short_type(inner)),
    }
}

/// Build a map from each scope number → its parent scope number, by walking the IR tree.
/// Scopes with no parent (e.g. the root block) are not in the map.
#[cfg(any(debug_assertions, test))]
fn build_scope_parents(val: &Value, parent: u16, parents: &mut HashMap<u16, u16>) {
    match val {
        Value::Block(bl) | Value::Loop(bl) => {
            parents.insert(bl.scope, parent);
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
fn find_conflict(
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
                return Some((left_idx, left_slot_end, right_idx, right_slot_end));
            }
        }
    }
    None
}

/// Assert that no two variables with overlapping live intervals occupy the same stack slot.
/// Only compiled in debug/test builds; the call site in `codegen.rs` is gated on
/// `#[cfg(debug_assertions)]`.
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

/// `sum_of_argument_sizes + 4`.
///
/// Variables with `argument == true` or `first_def == u32::MAX` are skipped.
/// Assign stack slots to all local variables using the two-zone block pre-claim approach.
///
/// Zone 1 (small variables, ≤ 8 B): greedy interval colouring within each scope's frame.
/// Zone 2 (large variables, > 8 B): placed sequentially at TOS in IR-walk order.
///
/// `code` is the function's top-level IR value (the outermost `Value::Block`).
/// `local_start` is the stack offset immediately after the function arguments.
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

/// Walk a single IR node to place large variables and recurse into child scopes.
///
/// - `Value::Set(v, _)` where `v` belongs to `scope` and `v` is large (> 8 B):
///   assign `v.stack_pos = *tos` and advance `*tos`.
/// - `Value::Block` / `Value::Loop`: recurse via `process_scope` (child has own frame).
/// - `Value::If`: process then/else each starting from the same `*tos`; after both, `*tos`
///   is unchanged (`gen_if` resets `stack.position` between arms and restores on exit).
/// - Other compound nodes: recurse into sub-expressions.
///
/// # Zone-2 ordering invariant
///
/// This function finds a large variable `v` only when `Value::Set(v, ...)` appears as a
/// **direct top-level operator** of the enclosing scope's Block, or as the direct RHS of
/// such a Set (e.g. `Set(outer, Block([Set(inner, ...), ...]))`).  If a parser change
/// places a first-assignment `Set(v, ...)` inside a non-recursed position — for example
/// as an argument to a `Call` node — `v` would never be visited here and would keep
/// `stack_pos = u16::MAX`, causing a panic in `generate_set` at codegen time.
///
/// The parser currently guarantees that every variable's first assignment is a block-level
/// statement, never nested inside an expression.  Document any future exception here.
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
                    let v_slot = *tos;
                    if function.logging {
                        eprintln!(
                            "[assign_slots]   zone2  '{}' scope={scope} size={v_size}B → slot={v_slot}  \
                             inner={}",
                            function.variables[v].name,
                            match inner.as_ref() {
                                Value::Block(bl) => format!("Block(scope={})", bl.scope),
                                Value::Loop(bl) => format!("Loop(scope={})", bl.scope),
                                other => format!("{:?}", std::mem::discriminant(other)),
                            }
                        );
                    }
                    function.variables[v].stack_pos = v_slot;
                    function.variables[v].pre_assigned_pos = v_slot;
                    *tos += v_size;
                    // Block-return pattern: Set(v, Block([..., Var(inner_result)])).
                    // For non-Text types, generate_block is called with `to = v.stack_pos`,
                    // so at runtime the block's frame starts at v's slot (v is not yet live).
                    // Zone-1 vars of the child scope share v's slot area safely.
                    // Using frame_base = v_slot (not *tos after advancing) prevents the
                    // pos > TOS override in generate_set for Zone-2 vars of the child scope.
                    //
                    // Text is excluded: gen_set_first_text emits OpText BEFORE the block runs,
                    // advancing stack.position by v_size, so the block's frame_base at codegen
                    // time is v_slot + v_size — matching the old *tos value.
                    if matches!(inner.as_ref(), Value::Block(_))
                        && !matches!(function.variables[v].type_def, Type::Text(_))
                    {
                        process_scope(function, inner, v_slot, depth + 1);
                        return;
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Block;
    use std::collections::HashMap;
    use std::mem::size_of;

    // ── helpers ──────────────────────────────────────────────────────────────

    const INT: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32);

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
    #[ignore = "A12: can_reuse not yet extended to Text (assign_slots)"]
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

    /// S2: `compute_intervals` must panic with a depth-limit message when nesting exceeds 1000.
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
}
