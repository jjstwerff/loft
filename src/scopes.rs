// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use crate::data::{Block, Context, Data, DefType, Type, Value, v_set};
use crate::variables::{Function, assign_slots, compute_intervals, size};
use std::collections::{BTreeMap, HashMap, HashSet};

struct Scopes {
    /// The definition number of the current analyzed function.
    d_nr: u32,
    /// The next scope number that will be created.
    max_scope: u16,
    /// The current scope during traversal of the code. 0 is the scope of the function arguments.
    scope: u16,
    /// The currently open scopes.
    stack: Vec<u16>,
    /// Per encountered variable the scope where it was created. Later copied into the definition.
    var_scope: BTreeMap<u16, u16>,
    /// Insertion order of variables into `var_scope` (excluding scope-0 arguments).
    /// Used by `variables()` to emit `OpFreeRef` in reverse-allocation order so that
    /// `database::free()` LIFO invariant is satisfied.
    var_order: Vec<u16>,
    /// Variables that are redefined after running out-of-scope get copied with this mapping.
    var_mapping: HashMap<u16, u16>,
    /// The scopes of the currently traversed loops.
    loops: Vec<u16>,
    /// Recursion depth counter for `scan`; reset to 0 when scope analysis starts.
    scan_depth: usize,
}

/// Perform scope analysis on all currently known functions.
pub fn check(data: &mut Data) {
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) || data.def(d_nr).variables.done {
            continue;
        }
        let mut scopes = Scopes {
            d_nr,
            max_scope: 1,
            scope: 0,
            stack: Vec::new(),
            var_scope: BTreeMap::new(),
            var_order: Vec::new(),
            var_mapping: HashMap::new(),
            loops: vec![],
            scan_depth: 0,
        };
        let mut function = Function::copy(&data.def(d_nr).variables);
        for a in function.arguments() {
            scopes.var_scope.insert(a, 0);
        }
        let code = scopes.scan(&data.definitions[d_nr as usize].code, &mut function, data);
        data.definitions[d_nr as usize].code = code;
        data.definitions[d_nr as usize].variables = function;
        for (v_nr, scope) in scopes.var_scope {
            data.definitions[d_nr as usize]
                .variables
                .set_scope(v_nr, scope);
        }
        // Compute live intervals so validate_slots can check for slot conflicts after codegen.
        let free_text_nr = data.def_nr("OpFreeText");
        let free_ref_nr = data.def_nr("OpFreeRef");
        let code_ref = data.definitions[d_nr as usize].code.clone();
        let mut seq = 0u32;
        compute_intervals(
            &code_ref,
            &mut data.definitions[d_nr as usize].variables,
            free_text_nr,
            free_ref_nr,
            &mut seq,
            0,
        );
        // Run assign_slots in shadow mode: pre-compute slots, save them, then reset so
        // claim() continues to drive codegen as before (A6.2).  The saved layout is
        // validated by check_shadow_slots after byte_code completes.
        let local_start: u16 = {
            let vars = &data.definitions[d_nr as usize].variables;
            let arg_size: u16 = vars
                .arguments()
                .iter()
                .map(|&a| size(vars.var_type(a), &Context::Argument))
                .sum();
            arg_size + 4 // 4 bytes for the return-address slot
        };
        // Pre-assign stack slots using the two-zone block pre-claim approach.
        {
            let d = &mut data.definitions[d_nr as usize];
            assign_slots(&mut d.variables, &mut d.code, local_start);
        }
    }
}

impl Scopes {
    fn enter_scope(&mut self) -> u16 {
        self.stack.push(self.scope);
        self.scope = self.max_scope;
        self.max_scope += 1;
        self.scope
    }

    fn exit_scope(&mut self) {
        if let Some(scope) = self.stack.pop() {
            self.scope = scope;
        }
    }

    fn scan(&mut self, val: &Value, function: &mut Function, data: &Data) -> Value {
        self.scan_depth += 1;
        assert!(
            self.scan_depth <= 1000,
            "expression nesting limit exceeded at depth {}",
            self.scan_depth
        );
        let result = self.scan_inner(val, function, data);
        self.scan_depth -= 1;
        result
    }

    fn scan_inner(&mut self, val: &Value, function: &mut Function, data: &Data) -> Value {
        match val {
            Value::Var(ov) => Value::Var(*self.var_mapping.get(ov).unwrap_or(ov)),
            Value::Set(ov, value) => self.scan_set(*ov, value, function, data),
            Value::Loop(lp) => {
                let scope = self.enter_scope();
                self.loops.push(scope);
                function.mark_loop_scope(scope);
                let ls = self.convert(lp, function, data);
                self.loops.pop();
                self.exit_scope();
                Value::Loop(Box::new(Block {
                    operators: ls,
                    result: Type::Void,
                    name: lp.name,
                    scope,
                    var_size: 0,
                }))
            }
            Value::If(test, t_val, f_val) => self.scan_if(test, t_val, f_val, function, data),
            Value::Break(lv) => {
                let mut ls = self.get_free_vars(
                    function,
                    data,
                    self.loops[self.loops.len() - *lv as usize - 1],
                    &Type::Void,
                    u16::MAX,
                );
                if ls.is_empty() {
                    Value::Break(*lv)
                } else {
                    ls.push(Value::Break(*lv));
                    Value::Insert(ls)
                }
            }
            Value::Continue(lv) => {
                let mut ls = self.get_free_vars(
                    function,
                    data,
                    self.loops[self.loops.len() - *lv as usize - 1],
                    &Type::Void,
                    u16::MAX,
                );
                if ls.is_empty() {
                    Value::Continue(*lv)
                } else {
                    ls.push(Value::Continue(*lv));
                    Value::Insert(ls)
                }
            }
            Value::Return(v) => {
                let expr = self.scan(v, function, data);
                Value::Insert(self.free_vars(
                    true,
                    &expr,
                    function,
                    data,
                    &data.def(self.d_nr).returned,
                    1,
                ))
            }
            Value::Block(bl) => {
                let scope = self.enter_scope();
                let ls = self.convert(bl, function, data);
                self.exit_scope();
                Value::Block(Box::new(Block {
                    operators: ls,
                    result: bl.result.clone(),
                    name: bl.name,
                    scope,
                    var_size: 0,
                }))
            }
            Value::Call(d_nr, args) => {
                let mut ls = Vec::new();
                for v in args {
                    ls.push(self.scan(v, function, data));
                }
                Value::Call(*d_nr, ls)
            }
            Value::CallRef(v_nr, args) => {
                let mut ls = Vec::new();
                for a in args {
                    ls.push(self.scan(a, function, data));
                }
                Value::CallRef(*v_nr, ls)
            }
            Value::Insert(ops) => {
                Value::Insert(ops.iter().map(|v| self.scan(v, function, data)).collect())
            }
            Value::Drop(inner) => Value::Drop(Box::new(self.scan(inner, function, data))),
            // COVERAGE GAP: Value::Iter(index_var, create, next, extra_init) is NOT recursed
            // into here.  Iter nodes ARE present in the IR at this point (compute_intervals
            // handles them after scan_inner runs).  Any Value::Set inside create/next/extra_init
            // is never seen by scan_set, so those variables keep scope = u16::MAX.
            // Currently safe because Iter sub-expressions are synthesised by the parser and
            // contain only index-variable reads (no user Set nodes in named variables).
            // If that invariant ever changes — e.g. a Set(v, ...) appears inside an Iter
            // sub-expression — v will keep scope = u16::MAX, making scopes_can_conflict always
            // return true for v, and validate_slots will panic with a false-positive conflict.
            // Fix: add a Value::Iter arm that recurses into all three sub-expressions, mirroring
            // the compute_intervals arm in variables.rs.
            _ => val.clone(),
        }
    }

    fn scan_set(&mut self, ov: u16, value: &Value, function: &mut Function, data: &Data) -> Value {
        assert_ne!(
            ov,
            u16::MAX,
            "Incorrect variable in {} fn {}",
            function.file,
            function.name
        );
        if let Some(s) = self.var_scope.get(&ov)
            && self.scope != *s
            && !self.stack.contains(s)
        {
            if let Some(&existing_copy) = self.var_mapping.get(&ov) {
                // Replace the mapping only if the existing copy's scope has exited.
                if let Some(&copy_scope) = self.var_scope.get(&existing_copy)
                    && copy_scope != self.scope
                    && !self.stack.contains(&copy_scope)
                {
                    self.var_mapping.insert(ov, function.copy_variable(ov));
                }
            } else {
                self.var_mapping.insert(ov, function.copy_variable(ov));
            }
        }
        let v = *self.var_mapping.get(&ov).unwrap_or(&ov);
        if self.var_scope.contains_key(&v) && *value == Value::Null {
            return Value::Insert(Vec::new());
        }
        // remember the scope of the variable
        let mut depend = Vec::new();
        for d in function.tp(v).depend() {
            if !self.var_scope.contains_key(&d) {
                depend.push(d);
                self.var_scope.insert(d, self.scope);
                self.var_order.push(d);
            }
        }
        if !self.var_scope.contains_key(&v) {
            self.var_scope.insert(v, self.scope);
            self.var_order.push(v);
        }
        if depend.is_empty() {
            Value::Set(v, Box::new(self.scan(value, function, data)))
        } else {
            let mut ls = Vec::new();
            for d in depend {
                if d == v {
                    continue;
                }
                if matches!(function.tp(d), Type::Text(_)) {
                    ls.push(v_set(d, Value::Text(String::new())));
                } else {
                    ls.push(v_set(d, Value::Null));
                }
                self.var_scope.insert(d, self.scope);
            }
            ls.push(Value::Set(v, Box::new(self.scan(value, function, data))));
            Value::Insert(ls)
        }
    }

    fn scan_if(
        &mut self,
        test: &Value,
        t_val: &Value,
        f_val: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Value {
        // Find Reference/Vector/Text variables first assigned inside either branch
        // (including nested ifs, but not inside loops).
        let mut pre_inits: Vec<u16> = Vec::new();
        self.find_first_ref_vars(t_val, function, &mut pre_inits);
        self.find_first_ref_vars(f_val, function, &mut pre_inits);

        // Register pre-inited vars in var_scope BEFORE scanning branches so that
        // the branch scans see them as already assigned and use the set_var/OpPutRef
        // re-assignment path instead of claim().
        for &v in &pre_inits {
            self.var_scope.insert(v, self.scope);
            self.var_order.push(v);
        }

        let scanned_if = Value::If(
            Box::new(self.scan(test, function, data)),
            Box::new(self.scan(t_val, function, data)),
            Box::new(self.scan(f_val, function, data)),
        );

        if pre_inits.is_empty() {
            return scanned_if;
        }

        // Emit Set(v, Null/empty) for each variable at the current scope, before the
        // If node.  These are NOT passed through scan() again — the var_scope check
        // in the Set arm would strip them (contains_key + Null → Insert([])).
        let mut stmts: Vec<Value> = Vec::new();
        for &v in &pre_inits {
            if matches!(function.tp(v), Type::Text(_)) {
                stmts.push(v_set(v, Value::Text(String::new())));
            } else {
                stmts.push(v_set(v, Value::Null));
            }
        }
        stmts.push(scanned_if);
        Value::Insert(stmts)
    }

    /// Convert the content of loops and blocks
    fn convert(&mut self, bl: &Block, function: &mut Function, data: &Data) -> Vec<Value> {
        let mut ls = Vec::new();
        for v in &bl.operators {
            let sv = self.scan(v, function, data);
            if let Value::Insert(to_insert) = sv {
                for i in to_insert {
                    ls.push(i.clone());
                }
            } else {
                ls.push(sv);
            }
        }
        let expr = if ls.is_empty() || bl.result == Type::Void {
            Value::Null
        } else {
            ls.pop().unwrap()
        };
        let scope_vars = self.variables(self.scope);
        for &v in &scope_vars {
            self.var_mapping.remove(&v);
        }
        for v in self.free_vars(false, &expr, function, data, &bl.result, self.scope) {
            ls.push(v);
        }
        ls
    }

    #[must_use]
    fn variables(&self, to_scope: u16) -> Vec<u16> {
        let mut scopes = HashSet::new();
        let mut sc = self.scope;
        let mut scope_pos = self.stack.len();
        loop {
            if sc == 0 {
                // never return function arguments
                break;
            }
            scopes.insert(sc);
            if sc == to_scope {
                break;
            }
            if scope_pos == 0 {
                break;
            }
            scope_pos -= 1;
            sc = self.stack[scope_pos];
        }
        // Iterate var_order in reverse (most-recently-inserted first) so that
        // OpFreeRef/OpFreeText are emitted in reverse-allocation order, satisfying
        // the LIFO invariant enforced by database::free().
        let mut res = Vec::new();
        for &v_nr in self.var_order.iter().rev() {
            if let Some(sc) = self.var_scope.get(&v_nr)
                && scopes.contains(sc)
            {
                res.push(v_nr);
            }
        }
        res
    }

    fn free_vars(
        &mut self,
        is_return: bool,
        expr: &Value,
        function: &mut Function,
        data: &Data,
        tp: &Type,
        to_scope: u16,
    ) -> Vec<Value> {
        let ret_var = returned_var(expr);
        let mut ls = self.get_free_vars(function, data, to_scope, tp, ret_var);
        if ls.is_empty() || matches!(expr, Value::Null | Value::Var(_)) {
            if is_return {
                ls.push(Value::Return(Box::new(expr.clone())));
            } else if !matches!(expr, Value::Null) {
                ls.push(expr.clone());
            }
        } else if let Value::Block(bl) = expr {
            return insert_free(bl, &ls, is_return);
        } else {
            ls.insert(0, expr.clone());
            if is_return {
                ls.push(Value::Return(Box::new(Value::Null)));
            }
        }
        ls
    }

    fn get_free_vars(
        &mut self,
        function: &mut Function,
        data: &Data,
        to_scope: u16,
        tp: &Type,
        ret_var: u16,
    ) -> Vec<Value> {
        let mut ls = Vec::new();
        for v in self.variables(to_scope) {
            if v == ret_var {
                continue;
            }
            if matches!(function.tp(v), Type::Text(_)) {
                ls.push(call("OpFreeText", v, data));
            }
            if let Type::Reference(_, dep) | Type::Vector(_, dep) | Type::Enum(_, true, dep) =
                function.tp(v)
                && dep.is_empty()
                && !tp.depend().contains(&v)
                && !function.is_skip_free(v)
            {
                ls.push(call("OpFreeRef", v, data));
            }
        }
        ls
    }

    /// Recursively collect variables that need a pre-init `Set(v, Null)` before an if/else.
    ///
    /// A variable is collected when it:
    /// - appears as the target of `Value::Set(v, ...)`,
    /// - has not yet been assigned (`var_scope` does not contain it), and
    /// - owns its allocation (`needs_pre_init` returns true).
    ///
    /// Recurses into nested `If` and `Block` but NOT into `Loop` — loop variables have
    /// per-iteration scope management and must not be pre-inited at the enclosing scope.
    fn find_first_ref_vars(&self, val: &Value, function: &Function, result: &mut Vec<u16>) {
        match val {
            Value::Set(v, _) => {
                let resolved = *self.var_mapping.get(v).unwrap_or(v);
                // For borrowed types (non-empty dep), only pre-init if every dep is already
                // in var_scope — otherwise the OpCreateStack emitted at pre-init time would
                // reference an uninitialised slot.
                let deps_ready = function
                    .tp(resolved)
                    .depend()
                    .iter()
                    .all(|d| self.var_scope.contains_key(d));
                if !self.var_scope.contains_key(&resolved)
                    && needs_pre_init(function.tp(resolved))
                    && deps_ready
                    && !result.contains(&resolved)
                {
                    result.push(resolved);
                }
            }
            Value::Block(bl) => {
                for op in &bl.operators {
                    self.find_first_ref_vars(op, function, result);
                }
            }
            Value::If(_, t, f) => {
                self.find_first_ref_vars(t, function, result);
                self.find_first_ref_vars(f, function, result);
            }
            Value::Insert(ops) => {
                for op in ops {
                    self.find_first_ref_vars(op, function, result);
                }
            }
            // Do NOT recurse into Value::Loop.
            _ => {}
        }
    }
}

fn needs_pre_init(tp: &Type) -> bool {
    matches!(
        tp,
        Type::Text(_) | Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
    )
}

fn call(to: &'static str, v: u16, data: &Data) -> Value {
    Value::Call(data.def_nr(to), vec![Value::Var(v)])
}

fn insert_free(block: &Block, free: &[Value], is_return: bool) -> Vec<Value> {
    let mut res = Vec::new();
    let mut ls = Vec::new();
    for (o_nr, o) in block.operators.iter().enumerate() {
        if o_nr + 1 == block.operators.len() {
            if let Value::Block(bl) = &block.operators[o_nr] {
                for v in insert_free(bl, free, is_return) {
                    ls.push(v);
                }
            } else if block.result == Type::Void {
                ls.push(o.clone());
                ls.push(Value::Return(Box::new(Value::Null)));
            } else {
                for v in free {
                    ls.push(v.clone());
                }
                if is_return {
                    ls.push(Value::Return(Box::new(o.clone())));
                } else {
                    ls.push(o.clone());
                }
            }
        } else {
            ls.push(o.clone());
        }
    }
    res.push(Value::Block(Box::new(Block {
        name: block.name,
        operators: ls,
        result: block.result.clone(),
        scope: block.scope,
        var_size: 0,
    })));
    res
}

fn returned_var(expr: &Value) -> u16 {
    match expr {
        Value::Var(v) => *v,
        Value::Block(bl) => {
            let mut v = u16::MAX;
            for o in &bl.operators {
                v = returned_var(o);
            }
            v
        }
        _ => u16::MAX,
    }
}
