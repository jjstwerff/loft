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
    /// arguments.
    lift_counter: u16,
    /// Variables added by `scan_args` for inline struct-returning call arguments.
    /// These are conditionally assigned inside if-chains / match arms, so the
    /// outer block needs a `Set(v, Null)` at function entry to reserve their
    /// slot in codegen's stack.position — otherwise the function-level
    /// `OpFreeRef(__lift_N)` at function exit reads a slot that was never
    /// allocated along every execution path.
    lift_vars: Vec<u16>,
    /// Counter for `__ret_N` temporaries used by `free_vars` to hold a
    /// non-trivial tail expression's value while free ops run (B5-L3 fix).
    ret_temp_counter: u16,
    /// `__ref_N` work_ref → witness variable whose call-return
    /// value might alias `__ref_N`'s store at runtime.  Populated by
    /// `scan_set` when the work_ref is passed as an arg to a user-fn
    /// call whose Reference result is assigned to the witness.
    /// Consulted by `get_free_vars` to emit `OpFreeRefIfDistinct` (a
    /// runtime store-nr check) instead of the unconditional `OpFreeRef`
    /// — see the comment block around `scan_set`'s witness-pairing branch.
    paired_witness: HashMap<u16, u16>,
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
            lift_vars: Vec::new(),
            ret_temp_counter: 0,
            paired_witness: HashMap::new(),
        };
        let mut function = Function::copy(&data.def(d_nr).variables);
        for a in function.arguments() {
            scopes.var_scope.insert(a, 0);
        }
        let mut code = scopes.scan(&data.definitions[d_nr as usize].code, &mut function, data);
        // lift vars from `scan_args` are assigned inside conditional
        // branches (if-chains, match arms) but their `OpFreeRef` lives at
        // function exit.  Without a Set(v, Null) at function entry, codegen's
        // stack.position never reserves their slot along every execution path,
        // and `generate_var` asserts when the function-exit free reads a slot
        // that some branch never pushed.  Prepend the null-inits now.
        if !scopes.lift_vars.is_empty()
            && let Value::Block(bl) = &mut code
        {
            for &v in scopes.lift_vars.iter().rev() {
                bl.operators.insert(0, v_set(v, Value::Null));
            }
        }
        data.definitions[d_nr as usize].code = code;
        data.definitions[d_nr as usize].variables = function;
        // in debug builds, assert that every owned Reference variable emitted an
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
        #[cfg(debug_assertions)]
        check_text_return(
            &data.definitions[d_nr as usize].code,
            &data.definitions[d_nr as usize].variables,
            &data.definitions[d_nr as usize].name.clone(),
            &data.definitions[d_nr as usize].returned.clone(),
            data,
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
        // Plan-04 close-out (2026-04-22): V1 remains the slot
        // allocator.  The Phase 2h "codegen is the allocator" pivot
        // and the V2-drive alternative both failed on variables
        // declared at an outer scope but first-Set in an inner scope
        // (e.g. match-arm pattern bindings lifted to body scope by
        // `scan_if`'s `small_both` pre-registration).  V1's zone-1
        // pre-pass is load-bearing — see
        // `doc/claude/plans/finished/04-slot-assignment-redesign/README.md`
        // § Status.  Invariants I1–I7 in `validate.rs` check V1's
        // output at every codegen completion (debug / test builds).
        let local_start: u16 = {
            let vars = &data.definitions[d_nr as usize].variables;
            let arg_size: u16 = vars
                .arguments()
                .iter()
                .map(|&a| size(vars.var_type(a), &Context::Argument))
                .sum();
            arg_size + 4 // 4 bytes for the return-address slot
        };
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
                && function.tp(*v).is_heap_owned()
            {
                panic!(
                    "[check_arg_ref_allocs] Set('{name}', Null) for owned heap type \
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
                let ls = self.convert(lp, function, data, false);
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
            Value::BreakWith(lv, val) => {
                let scanned_val = self.scan(val, function, data);
                let mut ls = self.get_free_vars(
                    function,
                    data,
                    self.loops[self.loops.len() - *lv as usize - 1],
                    &Type::Void,
                    u16::MAX,
                );
                if ls.is_empty() {
                    Value::BreakWith(*lv, Box::new(scanned_val))
                } else {
                    ls.push(Value::BreakWith(*lv, Box::new(scanned_val)));
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
                // pre-register a block-result Reference variable at the OUTER scope
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
                        && let Type::Reference(_, dep)
                        | Type::Vector(_, dep)
                        | Type::Enum(_, true, dep) = function.tp(ret_v)
                        && dep.is_empty()
                    {
                        self.var_scope.insert(ret_v, self.scope);
                        self.var_order.push(ret_v);
                        hoisted_ref = Some(ret_v);
                    }
                }
                // The function body block (scope 0 → 1) with a non-void
                // result needs is_return=true so frees land between the
                // tail expression and the Return, not after it.
                let is_body_return = self.scope == 0
                    && bl.result != Type::Void
                    && data.def(self.d_nr).returned != Type::Void;
                let scope = self.enter_scope();
                // Move hoisted var from outer scope (0) to body scope so
                // get_free_vars at body exit can find and free it.
                if let Some(w) = hoisted_ref {
                    self.var_scope.insert(w, scope);
                }
                let ls = self.convert(bl, function, data, is_body_return);
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
        // When a Reference variable is assigned from a user-function call,
        // codegen has two sub-paths (state/codegen.rs:1039-1066):
        // - has_ref_params == true → gen_set_first_ref_call_copy deep-copy
        // - has_ref_params == false → adoption (callee's __ref_N store IS
        //   the returned struct's store), OR the callee returned a
        //   different fresh store and the caller's __ref_N pre-alloc is
        //   orphaned.
        if matches!(
            function.tp(v),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && let Value::Call(fn_nr, _) = value
            && data.def(*fn_nr).name.starts_with("n_")
            && data.def(*fn_nr).code != Value::Null
        {
            let has_ref_params = data.def(*fn_nr).attributes.iter().any(|a| {
                !a.hidden && matches!(a.typedef, Type::Reference(_, _) | Type::Enum(_, true, _))
            });
            if has_ref_params {
                // codegen will take gen_set_first_ref_call_copy
                // (state/codegen.rs:1186-1238) — OpConvRefFromNull +
                // OpDatabase + lock-args + OpCopyRecord deep-copy into a
                // FRESH store owned by `v`.  Strip v's declared deps so
                // get_free_vars emits OpFreeRef at scope exit; otherwise
                // the parser's "borrows from arg N" inference suppresses
                // emission and the deep-copied store leaks (the
                // `dep_empty=false` path in scopes.rs:906).
                let deps: Vec<u16> = function.tp(v).depend().clone();
                for d in deps {
                    function.make_independent(v, d);
                }
            }
            // `has_ref_params == false` call whose result is assigned
            // to a Reference variable `v`.  At runtime the callee either:
            //   - **adopts** the placeholder (writes into the passed
            //     `__ref_N` and returns the same DbRef) — then `v`
            //     and `__ref_N` share a store;
            //   - **allocates fresh** (e.g. `return map_empty()` or
            //     `T.parse(text)` with an internal fresh alloc) —
            //     then `v`'s store and `__ref_N`'s placeholder store
            //     are distinct, and the placeholder is orphaned.
            //
            // The compiler cannot resolve the choice statically: a
            // single callee (`map_from_json`) branches both ways on
            // `json == ""`.  Both patterns must work.
            //
            // Plain `OpFreeRef(__ref_N)` at scope exit is wrong in
            // the adoption case when `v` flows into the enclosing
            // function's return — the placeholder free happens
            // BEFORE the caller reads `v`, corrupting `v`'s shared
            // store.  Unconditionally skipping the free is wrong in
            // the fresh-store case — placeholder orphaned.
            //
            // Record `__ref_N → v` in `paired_witness`.  At scope
            // exit, `get_free_vars` emits `OpFreeRefIfDistinct(__ref_N,
            // v)` instead of `OpFreeRef(__ref_N)`: the runtime
            // store-nr comparison settles the two cases per execution
            // path (match → skip; differ → free).
            if !has_ref_params && let Value::Call(_, args) = value {
                for arg in args {
                    let arg_var = match arg {
                        Value::Var(av) => Some(*av),
                        Value::Set(av, _) => Some(*av),
                        _ => None,
                    };
                    if let Some(av) = arg_var {
                        let n = function.name(av);
                        if n.starts_with("__ref_") || n.starts_with("__rref_") {
                            // `av`'s scope is inherited from the enclosing
                            // assignment: `self.scope`.  `v`'s scope was
                            // just written above.  Only pair when the
                            // witness `v` lives AT LEAST as long as
                            // `av` — i.e. `var_scope[v] <= var_scope[av]`.
                            // Otherwise, when codegen lowers the function
                            // to Rust, the witness's `let` falls out of
                            // its block scope before `av`'s OpFreeRef
                            // fires, and the emitted `var_f.store_nr`
                            // references a dead name (e.g. `f = file(…,
                            // __ref_1)` inside a nested `{}` block).
                            let av_scope = self.var_scope.get(&av).copied().unwrap_or(u16::MAX);
                            let v_scope = self.var_scope.get(&v).copied().unwrap_or(u16::MAX);
                            if v_scope <= av_scope && v_scope != u16::MAX {
                                self.paired_witness.entry(av).or_insert(v);
                            }
                        }
                    }
                }
            }
        }
        // Companion to the has_ref_params == true branch above for the
        // var-to-var deep-copy path.  When `Set(v, Var(src))` and
        // both are References to the same struct, codegen takes
        // `gen_set_first_ref_var_copy` (state/codegen.rs:1025-1033)
        // which OpConvRefFromNull + OpDatabase + OpCopyRecord
        // deep-copies src into a FRESH store owned by `v`.  This
        // path is hit by the I13 iterator protocol's hidden
        // `__iter_obj_N = c` setup (parser/collections.rs:209).
        // Strip v's declared deps so get_free_vars emits OpFreeRef.
        if let Value::Var(src) = value
            && let Type::Reference(d_nr, _) | Type::Enum(d_nr, true, _) = function.tp(v).clone()
            && let Type::Reference(src_d, _) | Type::Enum(src_d, true, _) = function.tp(*src)
            && d_nr == *src_d
        {
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
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

    /// Convert the content of loops and blocks.
    /// `is_return` should be true for the function body block of a non-void
    /// function — frees must happen before the tail expression returns.
    fn convert(
        &mut self,
        bl: &Block,
        function: &mut Function,
        data: &Data,
        is_return: bool,
    ) -> Vec<Value> {
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
        let frees = self.free_vars(is_return, &expr, function, data, &bl.result, self.scope);
        for v in frees {
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
        // The B5-L3 wrap (Set(__ret_N, expr); free ops; Return(Var(__ret_N)))
        // must not fire when `expr` is already a `Return` or contains one
        // at its tail — otherwise we'd emit `let _ret = return …` (E0308 in
        // native).  Recurse through `Insert` (which scopes wraps Return in
        // for free-vars cleanup) and `Block`.
        let expr_is_terminal = expr_ends_in_return(expr);
        if ls.is_empty() || matches!(expr, Value::Null | Value::Var(_)) {
            if is_return && !expr_is_terminal {
                ls.push(Value::Return(Box::new(expr.clone())));
            } else if matches!(expr, Value::Null) {
                // skip
            } else {
                ls.push(expr.clone());
            }
        } else if let Value::Block(bl) = expr {
            return insert_free(bl, &ls, is_return);
        } else if expr_is_terminal {
            // expr is already a `Return(...)` (or `Insert(...)` ending in
            // one) — the cleanup was emitted alongside it by the inner
            // Return arm's free_vars call.  Re-emitting `ls` here would
            // duplicate every OpFreeText/OpFreeRef and tack on a dead
            // `Return(Null)`.  Just propagate the terminal as-is.
            return vec![expr.clone()];
        } else if is_return && is_value_return_type(tp) && !expr_is_terminal {
            // B5-L3: when a value-returning function's tail expression is a
            // non-Block, non-Var, non-Null value (If/Match/Call etc.) and
            // there are free ops to run before return, save the expression's
            // value to a temp, run the free ops, then return the temp.  The
            // old path inserted the expression as a discarded statement and
            // emitted Return(Null) — interpreter bytecode got away with it by
            // reading the expression's result from top-of-stack via Return's
            // `value` bytes, but native codegen produced `let _ = expr; ...;
            // return 0` and dropped the function's actual return value.
            // Skip when expr is already a `Value::Return(...)` — wrapping
            // would generate `let _ret = return …` (E0308 in native).
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return && matches!(tp, Type::Text(_)) && !expr_is_terminal {
            // B5-L3 extension for text returns: save the expression's text
            // to a `__ret_N` temp, run free ops, then return the temp.  The
            // temp's String holds an OWN copy (OpAppendText copies bytes),
            // so subsequent OpFreeText on the original work-text doesn't
            // dangle the returned Str.  Mark the temp `skip_free` so its
            // OpFreeText isn't emitted at scope exit — the String leaks
            // for the duration of the caller's read, which is fine because
            // the caller copies bytes via AppendText immediately on return.
            //
            // Native codegen also needs the wrap (otherwise the call result
            // is dropped + `return null` returns the typed null sentinel).
            // The native emit converts `Set(__ret, call)` into
            // `let __ret: String = call(...).to_string()` — fine for the
            // interpreter but for native, `Str::new(&__ret)` after Return
            // would dangle.  Detect this in `output_block` and emit
            // `return Str::new(call(...))` directly, dropping the temp.
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            function.set_skip_free(tmp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
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
        let vars = self.variables(to_scope);
        if scope_debug {
            eprintln!(
                "[get_free_vars] fn={} to_scope={to_scope} scope={} vars={vars:?} ret_var={ret_var}",
                data.def(self.d_nr).name,
                self.scope
            );
        }
        for v in vars {
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
                // check both the block result type (tp) and the function's
                // declared return type.  When a closure escapes via implicit return,
                // the block result type may lack the dep that was propagated to the
                // function's declared return type (vectors.rs:704-711).
                // tp.depend() and returned.depend() contain attribute
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
                // Work-refs (`__ref_N` / `__rref_N`) carry their own var
                // in the dep list (`src/parser/mod.rs:1924-1928`) so the
                // standard `dep.is_empty()` gate skips them.  But work-
                // refs allocated to back ref-returning calls accumulate
                // unfreed stores when:
                //   - `gen_set_first_ref_call_copy`'s `0x8000` doesn't
                //     fire (e.g. when the callee MIGHT return a DbRef
                //     aliasing one of its args), or
                //   - the call-site reuses the same work-ref slot across
                //     loop iterations and `OpDatabase`'s `clear+claim`
                //     leaves the store marked `free` from the previous
                //     iteration even while live data lives in it.
                // Free them explicitly at function exit so the leak-check
                // at `src/state/debug.rs:1045` doesn't trip.  Skip when
                // the work-ref participates in the return chain.
                let is_work_ref = {
                    let n = function.name(v);
                    n.starts_with("__ref_") || n.starts_with("__rref_")
                };
                let emit = (dep.is_empty() || is_work_ref) && !in_ret && !function.is_skip_free(v);
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
                    // when `v` is a `__ref_*` / `__rref_*` work-ref
                    // that was passed to a user-fn call whose Reference
                    // result lives on as `witness`, emit the runtime-
                    // conditional `OpFreeRefIfDistinct(v, witness)` — it
                    // is a no-op in the adoption case (v and witness
                    // share a store) and a real free in the fresh-store
                    // case (distinct stores, placeholder orphaned).
                    // Falls through to plain `OpFreeRef` when no pairing
                    // was recorded.
                    if is_work_ref && let Some(&witness) = self.paired_witness.get(&v) {
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(witness)],
                        ));
                    } else {
                        ls.push(call("OpFreeRef", v, data));
                    }
                }
            }
            // free the closure DbRef embedded at offset+4 in a fn-ref slot.
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
        // unlock const reference/vector parameters at function exit.
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
                        && function.tp(v_nr).heap_dep().is_some()
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
                    && function.tp(v).is_heap_owned()
                    && !function.is_skip_free(v)
                {
                    eprintln!(
                        "[scope_debug] ORPHANED heap var '{}' (var={v}): \
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
            // pre-inits them at the pre-loop scope.
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
                        matches!(val.as_ref(), Value::Null) && function.tp(*v).is_heap_owned()
                    } else {
                        false
                    };
                // hoist Set(__lift_N, ...) preamble from nested scan_args.
                // These are produced when an inner call's arguments contained
                // inline struct-returning calls that were already lifted.
                let n = ops.len();
                let is_p135_hoisted = n >= 2
                    && ops[..n - 1].iter().all(|v| {
                        matches!(v, Value::Set(v_nr, _) if function.name(*v_nr).starts_with("__lift_"))
                    });
                // hoist Set(__ref_N, expr) preamble produced by the
                // parser's `&T`-conversion path for non-Var sources.  The
                // final op is always OpCreateStack(Var(__ref_N)); after
                // hoisting it stays as the arg value, while the Set moves
                // into the enclosing statement list so the work-ref lives
                // at function scope (its slot must survive the call).
                let is_p179_hoisted = n >= 2
                    && ops[..n - 1].iter().all(|v| {
                        matches!(v, Value::Set(v_nr, _) if function.name(*v_nr).starts_with("__ref_"))
                    })
                    && matches!(&ops[n - 1], Value::Call(d_nr, _)
                        if data.def(*d_nr).name == "OpCreateStack");
                if is_a56_hoisted || is_p135_hoisted || is_p179_hoisted {
                    let mut it = ops.into_iter();
                    for _ in 0..n - 1 {
                        preamble.push(it.next().unwrap());
                    }
                    let final_val = it.next().unwrap();
                    // the remaining Call may also be struct-returning
                    // (e.g. normalize3(__lift_1) inside add_dir).  Lift it too.
                    if let Some(tp) = Self::inline_struct_return(&final_val, data, outer_call) {
                        self.lift_counter += 1;
                        let name = format!("__lift_{}", self.lift_counter);
                        let tmp = function.add_temp_var(&name, &tp);
                        function.mark_inline_ref(tmp);
                        self.var_scope.insert(tmp, self.scope);
                        self.var_order.push(tmp);
                        self.lift_vars.push(tmp);
                        preamble.push(v_set(tmp, final_val));
                        ls.push(Value::Var(tmp));
                    } else {
                        ls.push(final_val);
                    }
                } else {
                    ls.push(Value::Insert(ops));
                }
            } else if let Some(tp) = Self::inline_struct_return(&scanned, data, outer_call) {
                // inline struct-returning or vector-returning call as argument
                // — lift to a temporary variable so get_free_vars emits
                // OpFreeRef at scope exit.  Without this, the callee's store
                // leaks every call.
                //
                // The argument becomes Set(tmp, call(...)) which the codegen
                // handles via gen_set_first_at_tos on first encounter and
                // generate_set (reassignment) on subsequent loop iterations.
                // get_free_vars emits OpFreeRef(tmp) at scope exit because
                // the dep is empty (owned).
                self.lift_counter += 1;
                let name = format!("__lift_{}", self.lift_counter);
                let tmp = function.add_temp_var(&name, &tp);
                function.mark_inline_ref(tmp);
                self.var_scope.insert(tmp, self.scope);
                self.var_order.push(tmp);
                self.lift_vars.push(tmp);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
            } else {
                ls.push(scanned);
            }
        }
        (preamble, ls)
    }

    /// Check whether a scanned argument at position `arg_idx` is an inline
    /// struct-returning call that needs lifting to a temporary variable.
    /// Returns the struct definition number if lifting is needed, None
    /// otherwise.
    ///
    /// Skips lifting when the outer call's return type depends on this argument
    /// (i.e. the result borrows from the argument's store).  Freeing the lifted
    /// temp at scope exit would be use-after-free in that case.
    fn inline_struct_return(val: &Value, data: &Data, _outer_call: u32) -> Option<Type> {
        if let Value::Call(fn_nr, _) = val {
            let def = data.def(*fn_nr);
            if def.name.starts_with("n_")
                && def.code != Value::Null
                && let Type::Reference(d_nr, _) = &def.returned
            {
                return Some(Type::Reference(*d_nr, Vec::new()));
            }
            // Native struct-enum constructors: no body (code == Null), return type
            // is a struct-enum with empty dep (allocates a new store, doesn't borrow).
            // Accessors carry dep=[0] after parser dep-inference and are skipped here.
            if def.code == Value::Null
                && let Type::Enum(d_nr, true, dep) = &def.returned
                && dep.is_empty()
            {
                return Some(Type::Enum(*d_nr, true, Vec::new()));
            }
            // Native vector-returning fns (e.g. `keys()`, `fields()` on
            // JsonValue) allocate a fresh vector store that the caller owns.
            // Without lifting, the chained call `v.keys().len()` leaks the
            // intermediate vector — same mechanism as the struct-return case.
            if def.code == Value::Null
                && let Type::Vector(elem, dep) = &def.returned
                && dep.is_empty()
            {
                return Some(Type::Vector(elem.clone(), Vec::new()));
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

/// True when `expr` is a `Return` (or recursively ends with one through
/// `Insert`/`Block` wrappers).  Used by `free_vars` to decide whether the
/// B5-L3 `__ret_N` wrap is safe — wrapping a terminal expression would
/// produce `let _ret = return …` in native and double-emit the inner
/// Return inside the Set's expression generator.
fn expr_ends_in_return(expr: &Value) -> bool {
    match expr {
        Value::Return(_) => true,
        Value::Insert(ops) => ops.last().is_some_and(expr_ends_in_return),
        Value::Block(bl) => bl.operators.last().is_some_and(expr_ends_in_return),
        _ => false,
    }
}

/// Whether a function's return type holds a plain value (no heap ownership).
/// Used by the B5-L3 fix in `free_vars` to decide whether saving the tail
/// expression into a `__ret_N` temp is safe.  Heap-owned types are excluded
/// for now — their ownership transfer interacts with `OpFreeRef` emission
/// and needs a separate design pass.
fn is_value_return_type(tp: &Type) -> bool {
    matches!(
        tp,
        Type::Integer(_)
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Enum(_, false, _)
    )
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
        Value::Return(inner) | Value::Drop(inner) => returned_var(inner),
        Value::Insert(ops) => ops.last().map_or(u16::MAX, returned_var),
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
/// Debug-only check: when a text-returning function's `Return` source Str
/// is backed by a local text variable `v`, refuse to compile if any
/// `OpFreeText(v)` appears before that `Return`.  The returned Str would
/// dangle into freed `String` memory — the interpreter occasionally gets
/// away with it (if the underlying allocator hasn't reused the slot), but
/// native codegen materialises this as `let _v = String::new(); … free(_v);
/// return &_v;` and trips Rust's UB check.
///
/// Companion to `check_ref_leaks` above — that check catches owned-ref leaks
/// at compile time; this one catches use-after-free on return.
#[cfg(debug_assertions)]
fn check_text_return(ir: &Value, function: &Function, fn_name: &str, ret_type: &Type, data: &Data) {
    if !matches!(ret_type, Type::Text(_)) {
        return;
    }
    let free_text_nr = data.def_nr("OpFreeText");
    if free_text_nr == u32::MAX {
        return;
    }

    // Collect every text var freed anywhere in the body (order-agnostic —
    // we only care whether the var *is* freed, not when).  If the var is
    // both the Return source and freed locally, codegen emits the free
    // before the return value lands at the caller, leaving a dangling
    // Str.  False negatives are fine (later walker will be stricter);
    // false positives would misfire on valid patterns, so keep the
    // criteria narrow.
    let mut freed: HashSet<u16> = HashSet::new();
    collect_freed_vars(ir, free_text_nr, &mut freed);
    if freed.is_empty() {
        return;
    }

    let ret_var = returned_var(ir);
    if ret_var == u16::MAX {
        return;
    }
    if !matches!(function.tp(ret_var), Type::Text(_)) {
        return;
    }
    assert!(
        !freed.contains(&ret_var),
        "[check_text_return] fn '{}' frees local text '{}' (var_nr={ret_var}) \
         before its Return — the returned Str would dangle into freed \
         String memory.  scopes.rs must leave '{}' for the caller to free.",
        fn_name,
        function.name(ret_var),
        function.name(ret_var),
    );
}

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
            // warn about variables with deps that are only text-return work refs.
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
                     Store will leak at runtime.",
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

// ── Plan-06 phase 5b — par-safety analyser (DESIGN.md D8) ────────────────────

use crate::data::{ImpureCategory, Purity};

/// Plan-06 phase 5b minimal — purity-driven `is_par_safe` classifier.
///
/// Returns `true` iff `d_nr`'s body contains only par-safe calls per
/// the Purity classification (DESIGN.md D8.1).  Recursive into user
/// fn callees; cycles short-circuit to `true` (5b's placeholder
/// trick — phase 5e replaces with proper monotonic fixed-point).
///
/// **Minimum implementation — covers Purity-driven rejection only.**
/// Full D8 rules require additional analysis not in this commit:
/// - R1 (writes to non-local) — partially captured via stdlib's
///   `Impure(ParentWrite)` annotations on `vector_add`/`hash_set`/etc.
///   The per-call "is first arg local?" check is missing; today
///   any call to a `ParentWrite` fn is rejected outright.
/// - R2 (nested par) — `Impure(ParCall)` returns true here; full
///   5b proper recurses into the inner worker fn.
/// - R4 (mutation through captured Reference) — not yet detected.
///
/// Non-`Function` def_nrs return `false` (only fns can be par
/// workers).  `CallRef` (runtime fn-ref) callsites pessimise to
/// `false` — the actual callee is not statically known.
///
/// Currently no production caller — phase 5b proper hooks the
/// analyser into codegen so par worker fns that return false here
/// produce a compile error per D8 diagnostics.  The accessor +
/// helpers carry `#[allow(dead_code)]` until then.
#[allow(dead_code)]
#[must_use]
pub fn is_par_safe(data: &Data, d_nr: u32) -> bool {
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return false;
    }
    let mut visited = HashSet::new();
    walk_par_safe(data, d_nr, &mut visited)
}

#[allow(dead_code)]
fn walk_par_safe(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> bool {
    if !visited.insert(d_nr) {
        // Cycle detected — break recursion optimistically (placeholder
        // trick).  Phase 5e replaces this with monotonic fixed-point
        // iteration so mutually-recursive pure pairs classify correctly.
        return true;
    }
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[d_nr as usize];
    if !matches!(def.def_type, DefType::Function) {
        return false;
    }
    walk_par_safe_value(&def.code, data, visited)
}

#[allow(dead_code)]
fn walk_par_safe_value(value: &Value, data: &Data, visited: &mut HashSet<u32>) -> bool {
    match value {
        Value::Call(callee, args) => {
            let safe = call_is_par_safe(*callee, data, visited);
            safe && args.iter().all(|a| walk_par_safe_value(a, data, visited))
        }
        Value::CallRef(_, _args) => {
            // Runtime fn-ref — actual callee is unknown at compile
            // time.  Conservative: reject.
            false
        }
        Value::Block(b) => b
            .operators
            .iter()
            .all(|v| walk_par_safe_value(v, data, visited)),
        Value::Insert(vs) => vs.iter().all(|v| walk_par_safe_value(v, data, visited)),
        Value::If(c, t, e) => {
            walk_par_safe_value(c, data, visited)
                && walk_par_safe_value(t, data, visited)
                && walk_par_safe_value(e, data, visited)
        }
        Value::Loop(body) => body
            .operators
            .iter()
            .all(|v| walk_par_safe_value(v, data, visited)),
        Value::Set(_, rhs) => walk_par_safe_value(rhs, data, visited),
        // Leaves — primitive literals, var reads, etc.  Safe.
        _ => true,
    }
}

#[allow(dead_code)]
fn call_is_par_safe(callee: u32, data: &Data, visited: &mut HashSet<u32>) -> bool {
    if callee == u32::MAX || (callee as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[callee as usize];
    match def.purity {
        Purity::Pure => true,
        Purity::Impure(ImpureCategory::HostIo)
        | Purity::Impure(ImpureCategory::Prng)
        | Purity::Impure(ImpureCategory::Io) => true,
        Purity::Impure(ImpureCategory::ParCall) => {
            // Nested par: D8 R2 says inner worker fn must itself be
            // par-safe.  Minimum impl returns true; full 5b looks
            // up the worker fn arg and recurses into it.
            true
        }
        Purity::Impure(ImpureCategory::ParentWrite) => false,
        Purity::Unknown => {
            if matches!(def.code, Value::Null) {
                // Native stdlib fn with no annotation — conservative.
                false
            } else {
                // User fn: recurse into its body.
                walk_par_safe(data, callee, visited)
            }
        }
    }
}

#[cfg(test)]
mod par_safety_tests {
    use super::is_par_safe;
    use crate::data::{Block, Data, DefType, ImpureCategory, Purity, Type, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: String::new(),
            line: 0,
            pos: 0,
        }
    }

    #[test]
    fn pure_fn_with_no_calls_is_par_safe() {
        let mut d = Data::new();
        let id = d.add_def("pure_leaf", &pos(), DefType::Function);
        d.definitions[id as usize].code = Value::Int(42);
        assert!(is_par_safe(&d, id));
    }

    #[test]
    fn fn_calling_pure_stdlib_is_par_safe() {
        let mut d = Data::new();
        let stdlib = d.add_def("min", &pos(), DefType::Function);
        d.definitions[stdlib as usize].purity = Purity::Pure;
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        assert!(is_par_safe(&d, user));
    }

    #[test]
    fn fn_calling_parent_write_stdlib_is_not_par_safe() {
        let mut d = Data::new();
        let stdlib = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[stdlib as usize].purity =
            Purity::Impure(ImpureCategory::ParentWrite);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        assert!(!is_par_safe(&d, user));
    }

    #[test]
    fn fn_calling_host_io_stdlib_is_par_safe() {
        let mut d = Data::new();
        let stdlib = d.add_def("log_warn", &pos(), DefType::Function);
        d.definitions[stdlib as usize].purity = Purity::Impure(ImpureCategory::HostIo);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        assert!(is_par_safe(&d, user));
    }

    #[test]
    fn fn_calling_unannotated_native_is_not_par_safe() {
        let mut d = Data::new();
        let stdlib = d.add_def("mystery_native", &pos(), DefType::Function);
        // purity defaults to Unknown; code defaults to Value::Null
        // (native fn with no body).
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        assert!(!is_par_safe(&d, user));
    }

    #[test]
    fn fn_calling_callref_is_not_par_safe() {
        let mut d = Data::new();
        let user = d.add_def("user", &pos(), DefType::Function);
        // Var slot 5, no args — runtime fn-ref of unknown target.
        d.definitions[user as usize].code = Value::CallRef(5, vec![]);
        assert!(!is_par_safe(&d, user));
    }

    #[test]
    fn user_fn_recursion_into_par_safe_callee() {
        let mut d = Data::new();
        let pure_stdlib = d.add_def("min", &pos(), DefType::Function);
        d.definitions[pure_stdlib as usize].purity = Purity::Pure;
        let inner = d.add_def("inner", &pos(), DefType::Function);
        d.definitions[inner as usize].code = Value::Call(pure_stdlib, vec![]);
        let outer = d.add_def("outer", &pos(), DefType::Function);
        d.definitions[outer as usize].code = Value::Call(inner, vec![]);
        assert!(is_par_safe(&d, outer));
    }

    #[test]
    fn cycle_breaks_optimistically() {
        // Mutually recursive a→b→a — placeholder trick returns true.
        // Phase 5e's fixed-point iteration handles this properly.
        let mut d = Data::new();
        let a = d.add_def("a", &pos(), DefType::Function);
        let b = d.add_def("b", &pos(), DefType::Function);
        d.definitions[a as usize].code = Value::Call(b, vec![]);
        d.definitions[b as usize].code = Value::Call(a, vec![]);
        assert!(is_par_safe(&d, a));
    }

    #[test]
    fn block_walks_every_operator() {
        let mut d = Data::new();
        let pure_fn = d.add_def("min", &pos(), DefType::Function);
        d.definitions[pure_fn as usize].purity = Purity::Pure;
        let bad_fn = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad_fn as usize].purity =
            Purity::Impure(ImpureCategory::ParentWrite);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Block(Box::new(Block {
            name: "test",
            operators: vec![Value::Call(pure_fn, vec![]), Value::Call(bad_fn, vec![])],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));
        assert!(
            !is_par_safe(&d, user),
            "block walk must reject when any operator calls a parent-write fn"
        );
    }
}

// ── Plan-06 phase 5d — par-safety diagnostic helpers (DESIGN.md D8) ──────────

/// Plan-06 phase 5d (DESIGN.md D8 diagnostic shapes) — explains
/// **why** a fn is par-unsafe by walking its body once and
/// returning the first violating call's information.
///
/// Returns `None` if the fn is par-safe (no violations to report).
/// Returns `Some(reason)` describing the first encountered violation
/// — currently one of:
///   - `"call to parent-write stdlib fn '<name>'"`
///   - `"call to unannotated native fn '<name>'"`
///   - `"runtime fn-ref call (callee unknown at compile time)"`
///   - `"recursive descent into par-unsafe user fn '<name>'"`
///
/// Used by phase 5b proper's codegen integration: when
/// `is_par_safe(d_nr) == false`, the parser calls
/// `par_unsafe_reason(d_nr)` to embed the specific cause in the
/// compile-error diagnostic body, matching D8's example error
/// shape with `--> file:line` + offending construct + fix-it.
///
/// Currently no production caller — phase 5b proper hooks it.
#[allow(dead_code)]
#[must_use]
pub fn par_unsafe_reason(data: &Data, d_nr: u32) -> Option<String> {
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return Some(format!("invalid def_nr {d_nr}"));
    }
    let mut visited = HashSet::new();
    walk_par_unsafe_reason(data, d_nr, &mut visited)
}

#[allow(dead_code)]
fn walk_par_unsafe_reason(
    data: &Data,
    d_nr: u32,
    visited: &mut HashSet<u32>,
) -> Option<String> {
    if !visited.insert(d_nr) {
        // Cycle — same optimistic short-circuit as is_par_safe.
        return None;
    }
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return Some(format!("invalid def_nr {d_nr}"));
    }
    let def = &data.definitions[d_nr as usize];
    if !matches!(def.def_type, DefType::Function) {
        return Some(format!("def {} is not a function", def.name));
    }
    walk_par_unsafe_reason_value(&def.code, data, visited)
}

#[allow(dead_code)]
fn walk_par_unsafe_reason_value(
    value: &Value,
    data: &Data,
    visited: &mut HashSet<u32>,
) -> Option<String> {
    match value {
        Value::Call(callee, args) => {
            if let Some(r) = call_reason(*callee, data, visited) {
                return Some(r);
            }
            for a in args {
                if let Some(r) = walk_par_unsafe_reason_value(a, data, visited) {
                    return Some(r);
                }
            }
            None
        }
        Value::CallRef(_, _args) => Some(
            "runtime fn-ref call (callee unknown at compile time)".to_string(),
        ),
        Value::Block(b) => {
            for v in &b.operators {
                if let Some(r) = walk_par_unsafe_reason_value(v, data, visited) {
                    return Some(r);
                }
            }
            None
        }
        Value::Insert(vs) => {
            for v in vs {
                if let Some(r) = walk_par_unsafe_reason_value(v, data, visited) {
                    return Some(r);
                }
            }
            None
        }
        Value::If(c, t, e) => walk_par_unsafe_reason_value(c, data, visited)
            .or_else(|| walk_par_unsafe_reason_value(t, data, visited))
            .or_else(|| walk_par_unsafe_reason_value(e, data, visited)),
        Value::Loop(body) => {
            for v in &body.operators {
                if let Some(r) = walk_par_unsafe_reason_value(v, data, visited) {
                    return Some(r);
                }
            }
            None
        }
        Value::Set(_, rhs) => walk_par_unsafe_reason_value(rhs, data, visited),
        _ => None,
    }
}

#[allow(dead_code)]
fn call_reason(callee: u32, data: &Data, visited: &mut HashSet<u32>) -> Option<String> {
    if callee == u32::MAX || (callee as usize) >= data.definitions.len() {
        return Some(format!("invalid callee def_nr {callee}"));
    }
    let def = &data.definitions[callee as usize];
    match def.purity {
        Purity::Pure
        | Purity::Impure(ImpureCategory::HostIo)
        | Purity::Impure(ImpureCategory::Prng)
        | Purity::Impure(ImpureCategory::Io)
        | Purity::Impure(ImpureCategory::ParCall) => None,
        Purity::Impure(ImpureCategory::ParentWrite) => Some(format!(
            "call to parent-write stdlib fn '{}'",
            def.name
        )),
        Purity::Unknown => {
            if matches!(def.code, Value::Null) {
                Some(format!("call to unannotated native fn '{}'", def.name))
            } else {
                walk_par_unsafe_reason(data, callee, visited).map(|inner| {
                    format!(
                        "recursive descent into par-unsafe user fn '{}': {}",
                        def.name, inner
                    )
                })
            }
        }
    }
}

#[cfg(test)]
mod par_diag_tests {
    use super::par_unsafe_reason;
    use crate::data::{Block, Data, DefType, ImpureCategory, Purity, Type, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: String::new(),
            line: 0,
            pos: 0,
        }
    }

    #[test]
    fn par_safe_fn_has_no_reason() {
        let mut d = Data::new();
        let id = d.add_def("safe", &pos(), DefType::Function);
        d.definitions[id as usize].code = Value::Int(0);
        assert!(par_unsafe_reason(&d, id).is_none());
    }

    #[test]
    fn parent_write_call_reports_offending_fn_name() {
        let mut d = Data::new();
        let stdlib = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[stdlib as usize].purity =
            Purity::Impure(ImpureCategory::ParentWrite);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        let r = par_unsafe_reason(&d, user).unwrap();
        assert!(
            r.contains("vector_add") && r.contains("parent-write"),
            "expected parent-write reason mentioning vector_add; got: {r}"
        );
    }

    #[test]
    fn unannotated_native_reports_specifically() {
        let mut d = Data::new();
        let stdlib = d.add_def("mystery", &pos(), DefType::Function);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        let r = par_unsafe_reason(&d, user).unwrap();
        assert!(
            r.contains("unannotated") && r.contains("mystery"),
            "got: {r}"
        );
    }

    #[test]
    fn callref_reports_runtime_unknown() {
        let mut d = Data::new();
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::CallRef(3, vec![]);
        let r = par_unsafe_reason(&d, user).unwrap();
        assert!(r.contains("runtime fn-ref"), "got: {r}");
    }

    #[test]
    fn nested_user_fn_reports_the_chain() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let inner = d.add_def("inner", &pos(), DefType::Function);
        d.definitions[inner as usize].code = Value::Call(bad, vec![]);
        let outer = d.add_def("outer", &pos(), DefType::Function);
        d.definitions[outer as usize].code = Value::Call(inner, vec![]);
        let r = par_unsafe_reason(&d, outer).unwrap();
        assert!(
            r.contains("recursive descent")
                && r.contains("inner")
                && r.contains("vector_add"),
            "expected chain explanation through inner→vector_add; got: {r}"
        );
    }

    #[test]
    fn first_violating_call_in_block_wins() {
        let mut d = Data::new();
        let pure_fn = d.add_def("min", &pos(), DefType::Function);
        d.definitions[pure_fn as usize].purity = Purity::Pure;
        let bad_first = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad_first as usize].purity =
            Purity::Impure(ImpureCategory::ParentWrite);
        let bad_second = d.add_def("hash_set", &pos(), DefType::Function);
        d.definitions[bad_second as usize].purity =
            Purity::Impure(ImpureCategory::ParentWrite);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Block(Box::new(Block {
            name: "test",
            operators: vec![
                Value::Call(pure_fn, vec![]),
                Value::Call(bad_first, vec![]),
                Value::Call(bad_second, vec![]),
            ],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));
        let r = par_unsafe_reason(&d, user).unwrap();
        // First violator wins: should mention vector_add, not hash_set.
        assert!(r.contains("vector_add"), "got: {r}");
        assert!(!r.contains("hash_set"), "second violator leaked: {r}");
    }
}

// ── Plan-06 phase 5e — fixed-point par-safety (DESIGN.md D8 phase 5e) ────────

/// Plan-06 phase 5e — monotonic fixed-point over the call graph.
///
/// Replaces 5b's placeholder "cycle returns true optimistically"
/// trick with a proper fixed-point iteration: every user fn starts
/// classified true; the worklist demotes fns whose bodies invoke
/// par-unsafe callees; demotions propagate to callers via the
/// caller graph (Data::callers_of / D12).
///
/// Result: mutually-recursive pure fns (`is_even` / `is_odd` shape)
/// classify true, where 5b's placeholder would have returned false
/// pessimistically.  Mutually-recursive fns where ANY participant
/// is impure correctly demote the whole cycle.
///
/// Termination: classifications are monotonic (true → false, never
/// reverse); worklist re-enqueues only when a demotion actually
/// happens.  Worst case: every user fn walked twice = O(N + E)
/// where E = call-graph edge count.
///
/// Currently no production caller — phase 5b' wires this in place
/// of the per-fn `is_par_safe` for the parser's diagnostic.
#[allow(dead_code)]
#[must_use]
pub fn analyse_par_safety_fixpoint(data: &Data) -> HashMap<u32, bool> {
    use std::collections::VecDeque;

    // Step 1: initial classification.  Every user fn starts true;
    // stdlib annotations are taken at face value.
    let user_fns: Vec<u32> = data.user_fn_d_nrs();
    let mut classification: HashMap<u32, bool> = HashMap::new();
    for &d_nr in &user_fns {
        classification.insert(d_nr, true);
    }

    // Step 2: worklist iteration.
    let mut worklist: VecDeque<u32> = user_fns.iter().copied().collect();
    while let Some(d_nr) = worklist.pop_front() {
        // Skip if already demoted — monotonic.
        if !classification.get(&d_nr).copied().unwrap_or(false) {
            continue;
        }
        let def = &data.definitions[d_nr as usize];
        let still_safe = walk_classified(&def.code, data, &classification);
        if !still_safe {
            classification.insert(d_nr, false);
            // Propagate demotion: every caller may need to re-check
            // because their body now calls a newly-demoted callee.
            for caller in data.callers_of(d_nr) {
                if classification.get(&caller).copied().unwrap_or(false) {
                    worklist.push_back(caller);
                }
            }
        }
    }
    classification
}

/// Walk a Value tree using the current classification map (not
/// recursive descent like 5b's walk_par_safe_value).  For user-fn
/// callees, looks up classification[callee]; for stdlib callees,
/// uses the Purity annotation.  No cache placeholder needed —
/// the fixed-point loop owns convergence.
#[allow(dead_code)]
fn walk_classified(
    value: &Value,
    data: &Data,
    classification: &HashMap<u32, bool>,
) -> bool {
    match value {
        Value::Call(callee, args) => {
            let safe = call_classified(*callee, data, classification);
            safe && args
                .iter()
                .all(|a| walk_classified(a, data, classification))
        }
        Value::CallRef(_, _) => false,
        Value::Block(b) => b
            .operators
            .iter()
            .all(|v| walk_classified(v, data, classification)),
        Value::Insert(vs) => vs.iter().all(|v| walk_classified(v, data, classification)),
        Value::If(c, t, e) => {
            walk_classified(c, data, classification)
                && walk_classified(t, data, classification)
                && walk_classified(e, data, classification)
        }
        Value::Loop(body) => body
            .operators
            .iter()
            .all(|v| walk_classified(v, data, classification)),
        Value::Set(_, rhs) => walk_classified(rhs, data, classification),
        _ => true,
    }
}

#[allow(dead_code)]
fn call_classified(
    callee: u32,
    data: &Data,
    classification: &HashMap<u32, bool>,
) -> bool {
    if callee == u32::MAX || (callee as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[callee as usize];
    match def.purity {
        Purity::Pure
        | Purity::Impure(ImpureCategory::HostIo)
        | Purity::Impure(ImpureCategory::Prng)
        | Purity::Impure(ImpureCategory::Io)
        | Purity::Impure(ImpureCategory::ParCall) => true,
        Purity::Impure(ImpureCategory::ParentWrite) => false,
        Purity::Unknown => {
            if matches!(def.code, Value::Null) {
                false
            } else {
                // User fn — look up the classification.  If absent
                // (user_fn_d_nrs missed it), conservative false.
                classification.get(&callee).copied().unwrap_or(false)
            }
        }
    }
}

#[cfg(test)]
mod par_fixpoint_tests {
    use super::analyse_par_safety_fixpoint;
    use crate::data::{Data, DefType, ImpureCategory, Purity, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: String::new(),
            line: 0,
            pos: 0,
        }
    }

    #[test]
    fn mutually_recursive_pure_fns_both_classify_safe() {
        // is_even / is_odd shape — the canonical case 5b's
        // placeholder trick gets WRONG (returns false for both)
        // and that 5e gets RIGHT (returns true for both).
        let mut d = Data::new();
        let pure_fn = d.add_def("min", &pos(), DefType::Function);
        d.definitions[pure_fn as usize].purity = Purity::Pure;
        let is_even = d.add_def("is_even", &pos(), DefType::Function);
        let is_odd = d.add_def("is_odd", &pos(), DefType::Function);
        // is_even calls is_odd + min (pure)
        d.definitions[is_even as usize].code = Value::Call(is_odd, vec![Value::Call(pure_fn, vec![])]);
        // is_odd calls is_even
        d.definitions[is_odd as usize].code = Value::Call(is_even, vec![]);
        let result = analyse_par_safety_fixpoint(&d);
        assert_eq!(result.get(&is_even), Some(&true), "is_even should be safe");
        assert_eq!(result.get(&is_odd), Some(&true), "is_odd should be safe");
    }

    #[test]
    fn impure_in_cycle_demotes_all_participants() {
        // a→b→c→a, but b also calls vector_add (parent_write).
        // All three should classify false.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let a = d.add_def("a", &pos(), DefType::Function);
        let b = d.add_def("b", &pos(), DefType::Function);
        let c = d.add_def("c", &pos(), DefType::Function);
        d.definitions[a as usize].code = Value::Call(b, vec![]);
        // b calls bad + c
        d.definitions[b as usize].code = Value::Call(c, vec![Value::Call(bad, vec![])]);
        d.definitions[c as usize].code = Value::Call(a, vec![]);
        let result = analyse_par_safety_fixpoint(&d);
        assert_eq!(result.get(&a), Some(&false), "a → b → bad");
        assert_eq!(result.get(&b), Some(&false), "b → bad");
        assert_eq!(result.get(&c), Some(&false), "c → a → b → bad");
    }

    #[test]
    fn pure_fn_unaffected_by_unrelated_impure_fn() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let pure_user = d.add_def("pure_user", &pos(), DefType::Function);
        let bad_user = d.add_def("bad_user", &pos(), DefType::Function);
        d.definitions[pure_user as usize].code = Value::Int(0);
        d.definitions[bad_user as usize].code = Value::Call(bad, vec![]);
        let result = analyse_par_safety_fixpoint(&d);
        assert_eq!(result.get(&pure_user), Some(&true));
        assert_eq!(result.get(&bad_user), Some(&false));
    }

    #[test]
    fn empty_data_returns_empty_map() {
        let d = Data::new();
        let result = analyse_par_safety_fixpoint(&d);
        assert!(result.is_empty());
    }
}

// ── Plan-06 phase 5b' shallow check (precise, no false positives) ────────────

/// Plan-06 phase 5b' — precise shallow par-safety check.
///
/// Walks `worker_d_nr`'s body looking for **direct** calls to fns
/// classified `Impure(ParentWrite)`.  Does NOT recurse into callee
/// bodies — so it only fires when the worker code itself contains
/// the offending call, not when a transitive callee does.  This
/// produces ZERO false positives because every `parent_write`
/// classification is explicit (came from a `#impure(parent_write)`
/// annotation in the stdlib or user code).
///
/// Trade-off vs the full `is_par_safe`: misses transitive
/// violations.  A worker that calls a user fn that calls
/// vector_add slips through.  But unlike the full check, it
/// never warns on a worker that's actually safe — making it
/// usable as a parser warning today, before the 5a annotation
/// sweep is comprehensive.
///
/// Returns `Some(callee_name)` if a direct ParentWrite call was
/// found; `None` otherwise.
#[allow(dead_code)]
#[must_use]
pub fn worker_calls_parent_write(data: &Data, worker_d_nr: u32) -> Option<String> {
    if worker_d_nr == u32::MAX || (worker_d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let def = &data.definitions[worker_d_nr as usize];
    walk_shallow_parent_write(&def.code, data)
}

#[allow(dead_code)]
fn walk_shallow_parent_write(value: &Value, data: &Data) -> Option<String> {
    match value {
        Value::Call(callee, args) => {
            // Check this call's purity.
            if (*callee as usize) < data.definitions.len() {
                let cdef = &data.definitions[*callee as usize];
                if matches!(
                    cdef.purity,
                    Purity::Impure(ImpureCategory::ParentWrite)
                ) {
                    return Some(cdef.name.clone());
                }
            }
            // Walk arg expressions (could contain nested Call).
            for a in args {
                if let Some(name) = walk_shallow_parent_write(a, data) {
                    return Some(name);
                }
            }
            None
        }
        // Don't recurse into CallRef target (runtime fn-ref); shallow.
        Value::CallRef(_, args) => {
            for a in args {
                if let Some(name) = walk_shallow_parent_write(a, data) {
                    return Some(name);
                }
            }
            None
        }
        Value::Block(b) => {
            for v in &b.operators {
                if let Some(name) = walk_shallow_parent_write(v, data) {
                    return Some(name);
                }
            }
            None
        }
        Value::Insert(vs) => {
            for v in vs {
                if let Some(name) = walk_shallow_parent_write(v, data) {
                    return Some(name);
                }
            }
            None
        }
        Value::If(c, t, e) => walk_shallow_parent_write(c, data)
            .or_else(|| walk_shallow_parent_write(t, data))
            .or_else(|| walk_shallow_parent_write(e, data)),
        Value::Loop(body) => {
            for v in &body.operators {
                if let Some(name) = walk_shallow_parent_write(v, data) {
                    return Some(name);
                }
            }
            None
        }
        Value::Set(_, rhs) => walk_shallow_parent_write(rhs, data),
        _ => None,
    }
}

#[cfg(test)]
mod par_shallow_tests {
    use super::worker_calls_parent_write;
    use crate::data::{Data, DefType, ImpureCategory, Purity, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: String::new(),
            line: 0,
            pos: 0,
        }
    }

    #[test]
    fn direct_parent_write_call_detected() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity =
            Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(bad, vec![]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }

    #[test]
    fn pure_call_not_detected() {
        let mut d = Data::new();
        let safe = d.add_def("min", &pos(), DefType::Function);
        d.definitions[safe as usize].purity = Purity::Pure;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(safe, vec![]);
        assert!(worker_calls_parent_write(&d, worker).is_none());
    }

    #[test]
    fn unannotated_call_not_detected() {
        // Shallow check is precise: only fires for explicit
        // ParentWrite annotations.  Unknown stays None (the full
        // is_par_safe rejects this; shallow doesn't).
        let mut d = Data::new();
        let unknown = d.add_def("mystery", &pos(), DefType::Function);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(unknown, vec![]);
        assert!(worker_calls_parent_write(&d, worker).is_none());
    }

    #[test]
    fn transitive_parent_write_not_detected() {
        // Worker calls inner; inner calls vector_add.  Shallow
        // does NOT recurse into inner — only the worker fn's
        // direct calls are checked.  Plan-06 phase 5b' (eventual)
        // adds transitive detection once 5a annotation coverage
        // is comprehensive enough not to false-positive.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity =
            Purity::Impure(ImpureCategory::ParentWrite);
        let inner = d.add_def("inner", &pos(), DefType::Function);
        d.definitions[inner as usize].code = Value::Call(bad, vec![]);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(inner, vec![]);
        assert!(worker_calls_parent_write(&d, worker).is_none());
    }

    #[test]
    fn parent_write_inside_arg_detected() {
        // bad_call(vector_add(...)) — the arg evaluation is also
        // a parent-write site.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity =
            Purity::Impure(ImpureCategory::ParentWrite);
        let safe = d.add_def("min", &pos(), DefType::Function);
        d.definitions[safe as usize].purity = Purity::Pure;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code =
            Value::Call(safe, vec![Value::Call(bad, vec![])]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }
}
