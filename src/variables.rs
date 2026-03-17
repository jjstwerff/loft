// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(clippy::cast_possible_truncation)]
#![allow(dead_code)]
#![allow(clippy::large_types_passed_by_value)]
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
    /// Sequence number of the first `Value::Set` node for this variable; `u32::MAX` = never defined.
    pub first_def: u32,
    /// Sequence number of the last `Value::Var` (or implicit `OpFreeText`/`OpFreeRef`) for this variable.
    pub last_use: u32,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub file: String,
    steps: Vec<u8>,
    unique: u16,
    current_loop: u16,
    loops: Vec<Iterator>,
    variables: Vec<Variable>,
    work_text: u16,
    work_ref: u16,
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
            steps: Vec::new(),
            unique: 0,
            current_loop: u16::MAX,
            loops: Vec::new(),
            work_text: 0,
            work_ref: 0,
            variables: Vec::new(),
            work_texts: BTreeSet::new(),
            work_refs: BTreeSet::new(),
            inline_ref_vars: BTreeSet::new(),
            names: HashMap::new(),
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
        self.work_texts.clear();
        self.work_refs.clear();
        self.inline_ref_vars.clear();
        self.inline_ref_vars.clone_from(&other.inline_ref_vars);
        self.names.clear();
        self.names.clone_from(&other.names);
        other.names.clear();
    }

    pub fn copy(other: &Function) -> Self {
        Function {
            name: other.name.clone(),
            file: other.file.clone(),
            current_loop: u16::MAX,
            steps: Vec::new(),
            unique: 0,
            loops: other.loops.clone(),
            variables: other.variables.clone(),
            work_text: 0,
            work_ref: 0,
            work_texts: BTreeSet::new(),
            work_refs: BTreeSet::new(),
            inline_ref_vars: other.inline_ref_vars.clone(),
            names: other.names.clone(),
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

    pub fn counter(&mut self) -> u16 {
        self.loops[self.current_loop as usize].counter
    }

    pub fn needs_counter(&mut self, counter: u16) {
        self.loops[self.current_loop as usize].counter = counter;
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

    pub fn scope(&self, var_nr: u16) -> u16 {
        if var_nr as usize >= self.variables.len() {
            return u16::MAX;
        }
        self.variables[var_nr as usize].scope
    }

    pub fn on_scope(&self, scopes: &HashSet<u16>) -> Vec<u16> {
        let mut res = Vec::new();
        for (v_nr, v) in self.variables.iter().enumerate() {
            if scopes.contains(&v.scope) {
                res.push(v_nr as u16);
            }
        }
        res
    }

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
    pub fn min_var_position(&self) -> u16 {
        let mut max_end = 0u16;
        for var in &self.variables {
            if var.stack_pos == u16::MAX {
                continue;
            }
            let end = var
                .stack_pos
                .saturating_add(size(&var.type_def, &Context::Variable));
            if end > max_end {
                max_end = end;
            }
        }
        max_end
    }

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

    pub fn get_variable(&self, name: &str) -> Option<&Variable> {
        if let Some(nr) = self.names.get(name) {
            return Some(&self.variables[*nr as usize]);
        }
        None
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
            first_def: u32::MAX,
            last_use: 0,
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
            first_def: u32::MAX,
            last_use: 0,
        });
        v
    }

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
            first_def: u32::MAX,
            last_use: 0,
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
            // prevent free for this variable
            self.variables[v_nr as usize].type_def = Type::Reference(0, vec![0]);
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

    pub fn claim(&mut self, var: u16, pos: u16, context: &Context) -> u16 {
        self.variables[var as usize].stack_pos = pos;
        pos + size(&self.variables[var as usize].type_def, context)
    }

    pub fn set_type(&mut self, var_nr: u16, tp: Type) {
        self.variables[var_nr as usize].type_def = tp;
    }

    pub fn var_type(&self, var_nr: u16) -> &Type {
        &self.variables[var_nr as usize].type_def
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
pub fn compute_intervals(
    val: &Value,
    function: &mut Function,
    free_text_nr: u32,
    free_ref_nr: u32,
    seq: &mut u32,
) {
    match val {
        Value::Var(v) => {
            let v = *v as usize;
            if v < function.variables.len() {
                function.variables[v].last_use = function.variables[v].last_use.max(*seq);
            }
            *seq += 1;
        }
        Value::Set(v, value) => {
            // Process the value expression first so that variables defined inside it
            // (e.g., block-return temporaries) get sequence numbers before the target.
            compute_intervals(value, function, free_text_nr, free_ref_nr, seq);
            let v = *v as usize;
            if v < function.variables.len() && function.variables[v].first_def == u32::MAX {
                function.variables[v].first_def = *seq;
            }
            *seq += 1;
        }
        Value::Block(bl) => {
            for op in &bl.operators {
                compute_intervals(op, function, free_text_nr, free_ref_nr, seq);
            }
        }
        Value::Loop(lp) => {
            for op in &lp.operators {
                compute_intervals(op, function, free_text_nr, free_ref_nr, seq);
            }
        }
        Value::If(test, t_val, f_val) => {
            compute_intervals(test, function, free_text_nr, free_ref_nr, seq);
            compute_intervals(t_val, function, free_text_nr, free_ref_nr, seq);
            compute_intervals(f_val, function, free_text_nr, free_ref_nr, seq);
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
                compute_intervals(arg, function, free_text_nr, free_ref_nr, seq);
            }
            *seq += 1;
        }
        Value::CallRef(v_nr, args) => {
            for a in args {
                compute_intervals(a, function, free_text_nr, free_ref_nr, seq);
            }
            // Mark the fn-ref variable as used at this point
            function.variables[*v_nr as usize].last_use =
                function.variables[*v_nr as usize].last_use.max(*seq);
            *seq += 1;
        }
        Value::Return(v) | Value::Drop(v) => {
            compute_intervals(v, function, free_text_nr, free_ref_nr, seq);
        }
        Value::Insert(ops) => {
            for op in ops {
                compute_intervals(op, function, free_text_nr, free_ref_nr, seq);
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

/// Scan `vars` for the first pair of variables whose stack slots AND live intervals both
/// overlap.  Returns `(i, u_slot_end, j, v_slot_end)` for the first conflicting pair found,
/// where `i < j` are indices into `vars`.
fn find_conflict(vars: &[Variable]) -> Option<(usize, u16, usize, u16)> {
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
                return Some((left_idx, left_slot_end, right_idx, right_slot_end));
            }
        }
    }
    None
}

/// Assert that no two variables with overlapping live intervals occupy the same stack slot.
/// Gated on `debug_assertions`; a no-op in release builds.
/// On failure, logs the full variable table and IR code before panicking.
pub fn validate_slots(function: &Function, data: &Data, def_nr: u32) {
    if !cfg!(debug_assertions) {
        return;
    }
    let vars = &function.variables;
    let Some((left_idx, left_slot_end, right_idx, right_slot_end)) = find_conflict(vars) else {
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
        "  {:<4} {:<2} {:<20} {:<14} {:<12} {:<14}",
        "#", "", "name", "type", "slot", "live"
    );
    eprintln!("  {}", "-".repeat(70));
    for (idx, var) in vars.iter().enumerate() {
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
        let mark = if idx == left_idx || idx == right_idx {
            "*"
        } else {
            " "
        };
        eprintln!(
            "  {idx:<4} {mark:<2} {:<20} {:<14} {slot_str:<12} {live_str:<14}",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Block;

    // ── helpers ──────────────────────────────────────────────────────────────

    const INT: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32);

    /// Add a variable with an already-known slot and live interval.
    fn add_var(f: &mut Function, tp: &Type, slot: u16, first_def: u32, last_use: u32) -> u16 {
        let v = f.add_unique("v", tp, 0);
        f.variables[v as usize].stack_pos = slot;
        f.variables[v as usize].first_def = first_def;
        f.variables[v as usize].last_use = last_use;
        v
    }

    // ── find_conflict unit tests ──────────────────────────────────────────────

    /// Slot reuse is fine when the two live intervals are strictly sequential.
    #[test]
    fn no_conflict_sequential_slot_reuse() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 10); // dies at seq 10
        add_var(&mut f, &INT, 0, 11, 20); // born at seq 11 — no overlap
        assert!(find_conflict(&f.variables).is_none());
    }

    /// Variables that are simultaneously alive but occupy adjacent, non-overlapping slots are fine.
    #[test]
    fn no_conflict_adjacent_slots() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 20); // slot [0, 4)
        add_var(&mut f, &INT, 4, 0, 20); // slot [4, 8)
        assert!(find_conflict(&f.variables).is_none());
    }

    /// Two variables at the exact same slot that are alive at the same time must be flagged.
    #[test]
    fn conflict_identical_slot_and_overlapping_interval() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 10);
        add_var(&mut f, &INT, 0, 5, 15); // overlaps both in slot and time
        assert!(find_conflict(&f.variables).is_some());
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
        assert!(find_conflict(&f.variables).is_some());
    }

    /// A variable with no assigned slot (`stack_pos == u16::MAX`) must never trigger a conflict.
    #[test]
    fn no_conflict_unassigned_slot() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 20);
        let v = f.add_unique("y", &INT, 0); // stack_pos stays u16::MAX
        f.variables[v as usize].first_def = 5;
        f.variables[v as usize].last_use = 15;
        assert!(find_conflict(&f.variables).is_none());
    }

    /// A variable that was declared but never assigned (`first_def == u32::MAX`) must be ignored,
    /// even if its slot otherwise collides.
    #[test]
    fn no_conflict_never_defined_variable() {
        let mut f = Function::new("f", "test");
        add_var(&mut f, &INT, 0, 0, 20);
        let v = f.add_unique("y", &INT, 0);
        f.variables[v as usize].stack_pos = 0; // same slot, but first_def stays u32::MAX
        assert!(find_conflict(&f.variables).is_none());
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
        }));
        let mut seq = 0u32;
        compute_intervals(&code, &mut f, u32::MAX, u32::MAX, &mut seq);
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
                })),
            ],
            result: Type::Void,
            scope: 0,
        }));
        let mut seq = 0u32;
        compute_intervals(&code, &mut f, u32::MAX, u32::MAX, &mut seq);
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
            }))),
            Box::new(Value::Block(Box::new(Block {
                name: "",
                operators: vec![Value::Set(b, Box::new(Value::Int(2))), Value::Var(b)],
                result: INT,
                scope: 0,
            }))),
        );
        let mut seq = 0u32;
        compute_intervals(&code, &mut f, u32::MAX, u32::MAX, &mut seq);
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
        assert!(find_conflict(&f.variables).is_none());
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
            find_conflict(&f.variables).is_some(),
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
            find_conflict(&f.variables).is_none(),
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
            find_conflict(&f.variables).is_some(),
            "overlapping intervals at the same slot must be a conflict"
        );
    }
}
