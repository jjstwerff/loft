// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Scope analysis and dependency-based freeing.
//!
//! After parsing, every function is walked by [`check`] which:
//! 1. Assigns each variable to a scope (block nesting level).
//! 2. Inserts `OpFreeText` / `OpFreeRef` at scope exits to free owned values.
//! 3. Handles variable shadowing across sibling scopes via `var_mapping`.
//! 4. Calls [`assign_slots`] and [`compute_intervals`] for stack layout.
//!
//! ## Dependency-based freeing
//!
//! Whether a heap value is freed at scope exit depends on the `dep` field
//! on its [`Type`]:
//!
//! - **`dep` empty** → the variable *owns* the value → emit `OpFreeRef`.
//! - **`dep` non-empty** → the variable *borrows* from a parameter → skip free
//!   (the caller owns the store; freeing here would corrupt it).
//!
//! **Text exception:** `OpFreeText` is always emitted for `Type::Text` regardless
//! of deps, because text lives as a `String` on the stack frame — it must be
//! dropped when the frame exits, even if borrowed.  The `Str` slice that was
//! passed as an argument is a view, not an allocation.
//!
//! **Return-value exemption:** the variable holding the function's return value
//! (`ret_var`) is never freed — its value is consumed by the caller.

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
    /// Counter for `__lift_N` temporary variables created to own inline struct
    /// arguments (P135 fix).
    lift_counter: u16,
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
            lift_counter: 0,
        };
        let mut function = Function::copy(&data.def(d_nr).variables);
        for a in function.arguments() {
            scopes.var_scope.insert(a, 0);
        }
        let code = scopes.scan(&data.definitions[d_nr as usize].code, &mut function, data);
        data.definitions[d_nr as usize].code = code;
        data.definitions[d_nr as usize].variables = function;
        // A5.6: in debug builds, assert that every owned Reference variable emitted an
        // OpFreeRef.  Catches scope-registration bugs before they reach the runtime
        // "Database N not correctly freed" assertion (which fires with no context).
        #[cfg(debug_assertions)]
        check_ref_leaks(
            &data.definitions[d_nr as usize].code,
            &data.definitions[d_nr as usize].variables,
            data,
            &data.definitions[d_nr as usize].name.clone(),
            &data.definitions[d_nr as usize].returned.clone(),
            &scopes.var_scope,
        );
        #[cfg(debug_assertions)]
        check_arg_ref_allocs(
            &data.definitions[d_nr as usize].code,
            &data.definitions[d_nr as usize].variables,
            &data.definitions[d_nr as usize].name.clone(),
        );
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

/// Walk `ir` and panic if any `Call` or `CallRef` argument directly contains a
/// `Set(ref_var, Null)` for an owned Reference (dep empty).
///
/// Such a nested allocation would place `ConvRefFromNull` on the eval stack
/// *between* call arguments, corrupting the arg layout and producing a garbage
/// `y` value inside the callee — the root cause of the A5.6 "Incorrect store"
/// bug.  `scan_args` in scopes.rs is responsible for bubbling these out; if it
/// misses a case this check catches it at compile time.
#[cfg(debug_assertions)]
fn check_arg_ref_allocs(ir: &Value, function: &Function, fn_name: &str) {
    fn check_args(args: &[Value], function: &Function, fn_name: &str) {
        for a in args {
            if let Value::Insert(ops) = a
                && ops.len() >= 2
                && let Value::Set(v, val) = &ops[0]
                && matches!(val.as_ref(), Value::Null)
                && matches!(function.tp(*v), Type::Reference(_, dep) if dep.is_empty())
            {
                panic!(
                    "[check_arg_ref_allocs] Set('{name}', Null) for owned Reference \
                     is nested inside a Call/CallRef argument in '{fn_name}'. \
                     This corrupts the CallRef arg layout (A5.6). \
                     scan_args in scopes.rs must bubble it out.",
                    name = function.name(*v),
                );
            }
            walk_check(a, function, fn_name);
        }
    }
    fn walk_check(ir: &Value, function: &Function, fn_name: &str) {
        match ir {
            Value::Call(_, args) | Value::CallRef(_, args) => {
                check_args(args, function, fn_name);
            }
            Value::Set(_, inner) => walk_check(inner, function, fn_name),
            Value::If(cond, t, f) => {
                walk_check(cond, function, fn_name);
                walk_check(t, function, fn_name);
                walk_check(f, function, fn_name);
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    walk_check(op, function, fn_name);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    walk_check(op, function, fn_name);
                }
            }
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                walk_check(inner, function, fn_name);
            }
            _ => {}
        }
    }
    walk_check(ir, function, fn_name);
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

    #[allow(clippy::too_many_lines)]
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
                // A5.6: pre-register a block-result Reference variable at the OUTER scope
                // before entering the block's inner scope.
                //
                // Without this, `scan_set(w, Null)` registers w at the inner scope.
                // At block exit, `free_vars` skips w (it is `ret_var`). At function exit,
                // `variables(outer_scope)` omits w (inner scope is not in the chain) →
                // OpFreeRef is never emitted → Database N not freed.
                //
                // Pre-registering at the outer scope causes `scan_set(w, Null)` inside the
                // block to see `var_scope.contains_key(&w) && *value == Null` → return
                // Insert([]) (the Set is suppressed from inside the block). We then hoist
                // Set(w, Null) to the outer level by returning Insert([Set(w,Null), Block]).
                //
                // This is necessary (not optional) because DbRef is 12 bytes (> 8) → Zone 2
                // of slot assignment handles it. Zone 2 of the outer scope's `process_scope`
                // walks its direct operators and finds Set(w, Null) in the Insert; Zone 2 of
                // the inner scope skips w (scope mismatch). If Set(w, Null) were left inside
                // the block, the outer Zone 2 would never see it and the slot would remain
                // u16::MAX → "variable never assigned a slot" panic at codegen.
                let mut hoisted_ref: Option<u16> = None;
                if let Some(Value::Var(ret_v)) = bl.operators.last() {
                    let ret_v = *self.var_mapping.get(ret_v).unwrap_or(ret_v);
                    if !self.var_scope.contains_key(&ret_v)
                        && let Type::Reference(_, dep) | Type::Vector(_, dep) = function.tp(ret_v)
                        && dep.is_empty()
                    {
                        self.var_scope.insert(ret_v, self.scope);
                        self.var_order.push(ret_v);
                        hoisted_ref = Some(ret_v);
                    }
                }
                let scope = self.enter_scope();
                let ls = self.convert(bl, function, data);
                self.exit_scope();
                let block = Value::Block(Box::new(Block {
                    operators: ls,
                    result: bl.result.clone(),
                    name: bl.name,
                    scope,
                    var_size: 0,
                }));
                if let Some(w) = hoisted_ref {
                    // Return Insert([Set(w, Null), Block]) so that:
                    // 1. Zone-2 slot assignment sees Set(w, Null) at the outer scope level.
                    // 2. get_free_vars at the outer scope emits OpFreeRef(w) on block exit.
                    Value::Insert(vec![v_set(w, Value::Null), block])
                } else {
                    block
                }
            }
            Value::Call(d_nr, args) => {
                let (preamble, ls) = self.scan_args(args, function, data, *d_nr);
                let call = Value::Call(*d_nr, ls);
                if preamble.is_empty() {
                    call
                } else {
                    let mut ops = preamble;
                    ops.push(call);
                    Value::Insert(ops)
                }
            }
            Value::CallRef(v_nr, args) => {
                let (preamble, ls) = self.scan_args(args, function, data, u32::MAX);
                let call = Value::CallRef(*v_nr, ls);
                if preamble.is_empty() {
                    call
                } else {
                    let mut ops = preamble;
                    ops.push(call);
                    Value::Insert(ops)
                }
            }
            Value::Insert(ops) => {
                Value::Insert(ops.iter().map(|v| self.scan(v, function, data)).collect())
            }
            Value::Drop(inner) => Value::Drop(Box::new(self.scan(inner, function, data))),
            Value::Iter(idx, create, next, extra) => Value::Iter(
                *idx,
                Box::new(self.scan(create, function, data)),
                Box::new(self.scan(next, function, data)),
                Box::new(self.scan(extra, function, data)),
            ),
            Value::Tuple(elems) => {
                Value::Tuple(elems.iter().map(|v| self.scan(v, function, data)).collect())
            }
            Value::TupleGet(var, idx) => {
                Value::TupleGet(*self.var_mapping.get(var).unwrap_or(var), *idx)
            }
            Value::TuplePut(var, idx, inner) => Value::TuplePut(
                *self.var_mapping.get(var).unwrap_or(var),
                *idx,
                Box::new(self.scan(inner, function, data)),
            ),
            Value::Yield(inner) => Value::Yield(Box::new(self.scan(inner, function, data))),
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
            if std::env::var("LOFT_LOG").as_deref() == Ok("scope_debug") {
                eprintln!(
                    "[scope_debug] copy trigger: var={ov} name='{}' \
                     registered_scope={s} current_scope={} stack={:?} value={value:?}",
                    function.name(ov),
                    self.scope,
                    self.stack,
                );
            }
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
            // Skip deps that reference variables from another function's scope
            // (e.g., closure work vars embedded in a fn-ref return type).
            if d >= function.count() {
                continue;
            }
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
        // When a Reference variable is assigned from a user-function call with no
        // visible Reference params (O-B2 adoption), the callee's __ref_N work-ref
        // store IS the returned struct's store.  Suppress FreeRef on those __ref_N
        // vars so native codegen doesn't free the store before the return value
        // reaches the caller.  Mirrors the interpreter codegen skip_free at
        // state/codegen.rs:1043-1050.
        if matches!(
            function.tp(v),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && let Value::Call(fn_nr, args) = value
            && data.def(*fn_nr).name.starts_with("n_")
            && data.def(*fn_nr).code != Value::Null
        {
            let has_ref_params = data.def(*fn_nr).attributes.iter().any(|a| {
                !a.hidden && matches!(a.typedef, Type::Reference(_, _) | Type::Enum(_, true, _))
            });
            if !has_ref_params {
                for arg in args {
                    if let Value::Var(wv) = arg {
                        let wv = *self.var_mapping.get(wv).unwrap_or(wv);
                        if function.name(wv).starts_with("__ref_") {
                            function.set_skip_free(wv);
                        }
                    }
                }
            }
        }
        let scanned = self.scan(value, function, data);
        // Flatten: if the scanned value is Insert([preamble..., final_call]),
        // hoist the preamble out so the IR becomes
        // Insert([preamble..., Set(v, final_call)]) instead of
        // Set(v, Insert([preamble..., final_call])).
        // This keeps Set(v, Call(...)) as a bare Call, which codegen's
        // gen_set_first_at_tos can handle correctly.
        let (mut ls, set_value) = if let Value::Insert(mut ops) = scanned {
            if ops.len() >= 2 {
                let final_val = ops.pop().unwrap();
                (ops, final_val)
            } else {
                (Vec::new(), Value::Insert(ops))
            }
        } else {
            (Vec::new(), scanned)
        };
        // Prepend dependency initializations.
        let mut prefix = Vec::new();
        for d in depend {
            if d == v {
                continue;
            }
            if matches!(function.tp(d), Type::Text(_)) {
                prefix.push(v_set(d, Value::Text(String::new())));
            } else {
                prefix.push(v_set(d, Value::Null));
            }
            self.var_scope.insert(d, self.scope);
        }
        if prefix.is_empty() && ls.is_empty() {
            Value::Set(v, Box::new(set_value))
        } else {
            let mut all = prefix;
            all.append(&mut ls);
            all.push(Value::Set(v, Box::new(set_value)));
            Value::Insert(all)
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

        // Also find small variables assigned in BOTH branches (or an else-if chain).
        let mut small_both: Vec<u16> = Vec::new();
        let mut t_vars: Vec<u16> = Vec::new();
        let mut f_vars: Vec<u16> = Vec::new();
        Self::find_assigned_vars(t_val, &self.var_mapping, &mut t_vars);
        Self::find_assigned_vars(f_val, &self.var_mapping, &mut f_vars);
        for &v in &t_vars {
            if f_vars.contains(&v)
                && !self.var_scope.contains_key(&v)
                && !pre_inits.contains(&v)
                && !needs_pre_init(function.tp(v))
            {
                small_both.push(v);
            }
        }

        // Register pre-inited vars in var_scope BEFORE scanning branches so that
        // the branch scans see them as already assigned and use the set_var/OpPutRef
        // re-assignment path instead of claim().
        for &v in &pre_inits {
            self.var_scope.insert(v, self.scope);
            self.var_order.push(v);
        }
        // Register small variables assigned in both branches at the parent scope too.
        for &v in &small_both {
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

    fn find_assigned_vars(val: &Value, mapping: &HashMap<u16, u16>, result: &mut Vec<u16>) {
        match val {
            Value::Set(v, inner) => {
                let resolved = *mapping.get(v).unwrap_or(v);
                if !result.contains(&resolved) {
                    result.push(resolved);
                }
                Self::find_assigned_vars(inner, mapping, result);
            }
            Value::Block(bl) => {
                for op in &bl.operators {
                    Self::find_assigned_vars(op, mapping, result);
                }
            }
            Value::If(c, t, f) => {
                Self::find_assigned_vars(c, mapping, result);
                Self::find_assigned_vars(t, mapping, result);
                Self::find_assigned_vars(f, mapping, result);
            }
            Value::Insert(ops) => {
                for op in ops {
                    Self::find_assigned_vars(op, mapping, result);
                }
            }
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::find_assigned_vars(a, mapping, result);
                }
            }
            Value::Drop(inner) | Value::Return(inner) => {
                Self::find_assigned_vars(inner, mapping, result);
            }
            _ => {}
        }
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

    #[allow(clippy::too_many_lines)]
    fn get_free_vars(
        &mut self,
        function: &mut Function,
        data: &Data,
        to_scope: u16,
        tp: &Type,
        ret_var: u16,
    ) -> Vec<Value> {
        let scope_debug = std::env::var("LOFT_LOG").as_deref() == Ok("scope_debug");
        let mut ls = Vec::new();
        for v in self.variables(to_scope) {
            if v == ret_var {
                continue;
            }
            // T1.3: tuple scope exit — free owned elements in reverse index order.
            if let Type::Tuple(elems) = function.tp(v) {
                let owned = crate::data::owned_elements(elems);
                for &(_offset, _idx) in owned.iter().rev() {
                    // T1.4 will emit per-element OpFreeText/OpFreeRef at the correct
                    // stack offset.  For now, record that cleanup is needed.
                    // The actual free ops require knowing the variable's stack slot +
                    // element offset, which is codegen's responsibility.
                }
                continue;
            }
            if matches!(function.tp(v), Type::Text(_)) {
                ls.push(call("OpFreeText", v, data));
            }
            if let Type::Reference(_, dep) | Type::Vector(_, dep) | Type::Enum(_, true, dep) =
                function.tp(v)
            {
                // A5.6-text: check both the block result type (tp) and the function's
                // declared return type.  When a closure escapes via implicit return,
                // the block result type may lack the dep that was propagated to the
                // function's declared return type (vectors.rs:704-711).
                // P117-fix: tp.depend() and returned.depend() contain attribute
                // indices (parameter positions), not variable numbers.  Resolve
                // each attribute index to its actual variable number before
                // comparing, to avoid false matches when a local var_nr happens
                // to equal an unrelated attribute index.
                let def = data.def(self.d_nr);
                let dep_has_var = |deps: &Vec<u16>| -> bool {
                    deps.iter().any(|&a| {
                        let a_idx = a as usize;
                        if a_idx < def.attributes.len() {
                            function.var(&def.attributes[a_idx].name) == v
                        } else {
                            a == v // fallback for non-attribute deps
                        }
                    })
                };
                let tp_deps = tp.depend().clone();
                let ret_deps = data.def(self.d_nr).returned.depend().clone();
                let in_ret = dep_has_var(&tp_deps)
                    || dep_has_var(&ret_deps)
                    || ret_var != u16::MAX && function.tp(ret_var).depend().contains(&v);
                let emit = dep.is_empty() && !in_ret && !function.is_skip_free(v);
                if scope_debug && !emit {
                    eprintln!(
                        "[scope_debug] NOT freeing '{}' (var={v}, scope={}, to_scope={to_scope}): \
                         dep_empty={} in_ret={in_ret} skip_free={}",
                        function.name(v),
                        self.var_scope.get(&v).copied().unwrap_or(u16::MAX),
                        dep.is_empty(),
                        function.is_skip_free(v),
                    );
                }
                if emit {
                    if scope_debug {
                        eprintln!(
                            "[scope_debug] freeing '{}' (var={v}, scope={})",
                            function.name(v),
                            self.var_scope.get(&v).copied().unwrap_or(u16::MAX),
                        );
                    }
                    ls.push(call("OpFreeRef", v, data));
                }
            }
            // A5.6-text: free the closure DbRef embedded at offset+4 in a fn-ref slot.
            // The 16-byte fn-ref stack slot is reclaimed by FreeStack, but the closure
            // store record at offset+4 must be explicitly freed via OpFreeRef.
            if let Type::Function(_, _, _) = function.tp(v) {
                // fn-ref variables OWN their closure store. The dep list
                // tracks captured variables, not store borrowing. Always
                // emit OpFreeRef unless the fn-ref is the return value.
                let in_ret =
                    tp.depend().contains(&v) || data.def(self.d_nr).returned.depend().contains(&v);
                let emit = !in_ret && !function.is_skip_free(v);
                if emit {
                    if scope_debug {
                        eprintln!(
                            "[scope_debug] freeing closure of fn-ref '{}' (var={v}, scope={})",
                            function.name(v),
                            self.var_scope.get(&v).copied().unwrap_or(u16::MAX),
                        );
                    }
                    ls.push(call("OpFreeRef", v, data));
                }
            }
        }
        // P120-fix: unlock const reference/vector parameters at function exit.
        // The lock was set in parse_code (expressions.rs:163-178) at function entry.
        // Arguments live at scope 0 which `variables()` intentionally skips, so
        // we handle them here as a separate pass.  Only emit when exiting to
        // the function's top scope (to_scope <= 1).
        if to_scope <= 1 {
            let lock_fn = data.def_nr("n_set_store_lock");
            if lock_fn != u32::MAX {
                let n_vars = function.next_var();
                for v_nr in 0..n_vars {
                    if function.is_argument(v_nr)
                        && function.is_const_param(v_nr)
                        && matches!(
                            function.tp(v_nr),
                            Type::Reference(_, _) | Type::Vector(_, _)
                        )
                    {
                        ls.push(Value::Call(
                            lock_fn,
                            vec![Value::Var(v_nr), Value::Boolean(false)],
                        ));
                    }
                }
            }
        }
        // scope_debug: also report Reference vars in var_order whose scope is NOT in
        // the current chain — these are "orphaned" vars that should never happen after
        // the A5.6 block-pre-registration fix.
        if scope_debug {
            let chain: HashSet<u16> = {
                let mut s = HashSet::new();
                let mut sc = self.scope;
                let mut pos = self.stack.len();
                loop {
                    if sc == 0 {
                        break;
                    }
                    s.insert(sc);
                    if sc == to_scope {
                        break;
                    }
                    if pos == 0 {
                        break;
                    }
                    pos -= 1;
                    sc = self.stack[pos];
                }
                s
            };
            for &v in &self.var_order {
                if v == ret_var {
                    continue;
                }
                let v_scope = *self.var_scope.get(&v).unwrap_or(&0);
                if v_scope == 0 {
                    continue;
                }
                if !chain.contains(&v_scope)
                    && let Type::Reference(_, dep) = function.tp(v)
                    && dep.is_empty()
                    && !function.is_skip_free(v)
                {
                    eprintln!(
                        "[scope_debug] ORPHANED Reference '{}' (var={v}): \
                         its scope={v_scope} is not in the chain to to_scope={to_scope}",
                        function.name(v),
                    );
                }
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
            // Do NOT recurse into Value::Loop — loop-interior Reference
            // variables are handled by the Loop handler in scan() which
            // pre-inits them at the pre-loop scope (issue #120).
            _ => {}
        }
    }

    /// Scan a list of call arguments.
    ///
    /// If any scanned arg comes back as `Insert([Set(w, Null), body])` where `w` is an
    /// owned Reference (dep empty) — a hoisted closure-record allocation — the `Set(w,
    /// Null)` is lifted out into `preamble` and the arg is replaced with `body` alone.
    ///
    /// This prevents `ConvRefFromNull` (12 B) from landing on the eval stack *between*
    /// other call arguments, which would corrupt the `CallRef` argument layout and cause
    /// the lambda to receive garbage for `y` (A5.6 "Incorrect store" bug).
    ///
    /// Returns `(preamble, scanned_args)`.  The caller wraps the result as
    /// `Insert([preamble..., Call/CallRef(...)])` when the preamble is non-empty;
    /// `convert` flattens this so the preamble executes before any args are pushed.
    fn scan_args(
        &mut self,
        args: &[Value],
        function: &mut Function,
        data: &Data,
        outer_call: u32,
    ) -> (Vec<Value>, Vec<Value>) {
        let mut preamble: Vec<Value> = Vec::new();
        let mut ls: Vec<Value> = Vec::new();
        for a in args {
            let scanned = self.scan(a, function, data);
            if let Value::Insert(ops) = scanned {
                // Existing A5.6 hoisting: lift Set(w, Null) for owned Reference.
                let is_a56_hoisted = ops.len() == 2
                    && if let Value::Set(v, val) = &ops[0] {
                        matches!(val.as_ref(), Value::Null)
                            && matches!(function.tp(*v), Type::Reference(_, dep) if dep.is_empty())
                    } else {
                        false
                    };
                // P135: hoist Set(__lift_N, ...) preamble from nested scan_args.
                // These are produced when an inner call's arguments contained
                // inline struct-returning calls that were already lifted.
                let n = ops.len();
                let is_p135_hoisted = n >= 2
                    && ops[..n - 1].iter().all(|v| {
                        matches!(v, Value::Set(v_nr, _) if function.name(*v_nr).starts_with("__lift_"))
                    });
                if is_a56_hoisted || is_p135_hoisted {
                    let mut it = ops.into_iter();
                    for _ in 0..n - 1 {
                        preamble.push(it.next().unwrap());
                    }
                    ls.push(it.next().unwrap()); // the actual call / value
                } else {
                    ls.push(Value::Insert(ops));
                }
            } else if let Some(struct_d_nr) = Self::inline_struct_return(&scanned, data, outer_call) {
                // P135-fix: inline struct-returning call as argument — lift to
                // a temporary variable so get_free_vars emits OpFreeRef at scope
                // exit.  Without this, the callee's store leaks every call.
                //
                // The argument becomes Set(tmp, call(...)) which the codegen
                // handles via gen_set_first_at_tos on first encounter and
                // generate_set (reassignment) on subsequent loop iterations.
                // get_free_vars emits OpFreeRef(tmp) at scope exit because
                // the dep is empty (owned).
                self.lift_counter += 1;
                let name = format!("__lift_{}", self.lift_counter);
                let tp = Type::Reference(struct_d_nr, vec![]); // owned
                let tmp = function.add_temp_var(&name, &tp);
                self.var_scope.insert(tmp, self.scope);
                self.var_order.push(tmp);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
            } else {
                ls.push(scanned);
            }
        }
        (preamble, ls)
    }

    /// Check whether a scanned argument at position `arg_idx` is an inline
    /// struct-returning call that needs lifting to a temporary variable (P135
    /// fix).  Returns the struct definition number if lifting is needed, None
    /// otherwise.
    ///
    /// Skips lifting when the outer call's return type depends on this argument
    /// (i.e. the result borrows from the argument's store).  Freeing the lifted
    /// temp at scope exit would be use-after-free in that case.
    fn inline_struct_return(
        val: &Value,
        data: &Data,
        outer_call: u32,
    ) -> Option<u32> {
        if let Value::Call(fn_nr, _) = val {
            let def = data.def(*fn_nr);
            if def.name.starts_with("n_")
                && def.code != Value::Null
                && let Type::Reference(d_nr, _) = &def.returned
            {
                return Some(*d_nr);
            }
        }
        None
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

/// Recursively collect every variable freed by `OpFreeRef` in `ir`.
/// Used by `check_ref_leaks` to verify no Reference variable is leaked.
#[cfg(debug_assertions)]
fn collect_freed_vars(ir: &Value, free_ref_nr: u32, result: &mut HashSet<u16>) {
    match ir {
        Value::Call(d_nr, args) if *d_nr == free_ref_nr => {
            if let Some(Value::Var(v)) = args.first() {
                result.insert(*v);
            }
        }
        Value::Call(_, args) => {
            for a in args {
                collect_freed_vars(a, free_ref_nr, result);
            }
        }
        Value::Set(_, inner) => collect_freed_vars(inner, free_ref_nr, result),
        Value::If(cond, t, f) => {
            collect_freed_vars(cond, free_ref_nr, result);
            collect_freed_vars(t, free_ref_nr, result);
            collect_freed_vars(f, free_ref_nr, result);
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                collect_freed_vars(op, free_ref_nr, result);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                collect_freed_vars(op, free_ref_nr, result);
            }
        }
        Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
            collect_freed_vars(inner, free_ref_nr, result);
        }
        _ => {}
    }
}

/// After scope analysis, assert that every Reference variable that should be
/// freed has a corresponding `OpFreeRef` somewhere in `ir`.
///
/// A variable "should be freed" when:
/// - Its type is `Reference(_, dep)` with `dep.is_empty()`
/// - It is not a function parameter (scope > 0)
/// - It is not marked `skip_free`
/// - It is not in the function's return-type dependencies
///
/// Only compiled in debug builds; the check panics rather than emitting a
/// diagnostic so that the failure is visible immediately during development.
#[cfg(debug_assertions)]
fn check_ref_leaks(
    ir: &Value,
    function: &Function,
    data: &Data,
    fn_name: &str,
    ret_type: &Type,
    var_scope: &BTreeMap<u16, u16>,
) {
    let free_ref_nr = data.def_nr("OpFreeRef");
    let mut freed: HashSet<u16> = HashSet::new();
    collect_freed_vars(ir, free_ref_nr, &mut freed);

    let mut ret_deps: HashSet<u16> = ret_type.depend().into_iter().collect();
    // The directly-returned variable (e.g. the owned struct constructed by a function
    // whose return type is Reference) passes ownership to the caller — no FreeRef is
    // emitted for it and that is correct.  Exclude it so check_ref_leaks does not
    // false-positive on `fn foo() -> S { S { ... } }`.
    let direct_ret_var = returned_var(ir);
    // Transitive: if the returned variable depends on another variable, that
    // variable's store must also survive — include it in ret_deps.
    if direct_ret_var != u16::MAX {
        for d in function.tp(direct_ret_var).depend() {
            ret_deps.insert(d);
        }
    }

    for (&v, &scope) in var_scope {
        if scope == 0 {
            continue; // function parameter — caller frees
        }
        if (v as usize) >= function.count() as usize {
            continue; // variable belongs to outer scope — not our problem
        }
        if function.is_skip_free(v) {
            continue;
        }
        if v == direct_ret_var {
            continue; // ownership transferred to caller
        }
        if let Type::Reference(_, dep) = function.tp(v) {
            assert!(
                !dep.is_empty() || ret_deps.contains(&v) || freed.contains(&v),
                "[check_ref_leaks] Reference variable '{}' (var_nr={v}) in function \
                 '{}' has no OpFreeRef — it is in scope {scope} but was never freed. \
                 This is likely a scope-registration bug: the variable was registered \
                 in an inner block scope that is not reachable from function-exit cleanup.",
                function.name(v),
                fn_name
            );
            // P117: warn about variables with deps that are only text-return work refs.
            // These deps are spurious (struct copies the text), but OpFreeRef is still
            // skipped, causing a store leak at runtime.
            if !dep.is_empty()
                && !ret_deps.contains(&v)
                && !freed.contains(&v)
                && dep.iter().all(|d| {
                    function.name(*d).starts_with("__ref_")
                        || function.name(*d).starts_with("__rref_")
                })
            {
                eprintln!(
                    "[check_ref_leaks] Warning: Reference variable '{}' (var_nr={v}) in \
                     function '{}' has only text-work deps {:?} — likely spurious. \
                     Store will leak at runtime (P117).",
                    function.name(v),
                    fn_name,
                    dep.iter()
                        .map(|d| function.name(*d).to_string())
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
}
