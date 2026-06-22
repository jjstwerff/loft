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

use crate::data::{Block, Context, Data, DefType, Deps, Type, Value, v_if, v_set};
use crate::variables::{Function, compute_intervals, size};
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
    /// Plan-57 cluster-I two-phase scan: confined `__vdb`/local var → the block
    /// scope it should register at instead of function scope, so the block-exit
    /// `free_vars` sweep frees the store there.  Empty on phase 1; populated from
    /// `store_confinement` for the gated phase-2 re-scan (`put_scope` consults it).
    confined: HashMap<u16, u16>,
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
    /// @P378(a) — INVERSE of `paired_witness` for the case where the
    /// witness `v` is INNER-scoped relative to the `__ref_N` buffer
    /// `av` (e.g. `bs = alloc_bag(ci, __ref_1)` inside a `for` loop,
    /// where `bs` lives in the loop body and `__ref_1` is the
    /// function-scoped return buffer).  Here the buffer must stay
    /// reserved across iterations; freeing the witness each iteration
    /// (it adopts the buffer's store) would recycle that store to a
    /// callee temp next iteration and collide (SIGSEGV at the keyed
    /// insert).  Maps witness `v` → buffer `av`; consulted by
    /// `get_free_vars` to emit `OpFreeRefIfDistinct(v, av)` for the
    /// witness's free — a no-op in the adoption case (store stays
    /// reserved, freed once via the buffer's function-exit OpFreeRef),
    /// a real free in the fresh-store case.  Scope-safe for native:
    /// `av` (outer/function) outlives `v` (inner), so `av`'s Rust
    /// `let` is still live where `v`'s free fires.
    witness_buffer: HashMap<u16, u16>,
    /// #316 — Reference vars whose LATEST scanned assignment gave them an
    /// OWNED store (a call whose filtered return deps are empty, a deep-copied
    /// var, …), mapped to the loop depth (`loops.len()`) at that assignment.
    /// When such a var is reassigned with a BORROW, its merged static type
    /// already carries deps, so codegen's dep-empty pre-Set free never fires —
    /// `scan_set` emits an explicit `OpFreeRef(v)` for the orphaned store
    /// instead (only at the same loop depth: emitting inside a deeper loop
    /// would re-free a viewed store on iterations 2+).
    owned_refs: HashMap<u16, usize>,
}

/// Perform scope analysis on all currently known functions.
/// One scan pass for [`check`]: scan `orig_code`/`orig_vars`, prepend the
/// lift-var null-inits, apply the result to `def`, run the debug ref/leak checks,
/// and set each variable's scope.  Runs once normally; plan-57 cluster I re-runs
/// it with a non-empty `confined` map (`__vdb`/local → block scope) so a confined
/// store registers — and therefore frees — at its block exit (`put_scope`).
fn run_scan_phase(
    data: &mut Data,
    d_nr: u32,
    orig_code: &Value,
    orig_vars: &Function,
    confined: &HashMap<u16, u16>,
) {
    let mut scopes = Scopes {
        d_nr,
        max_scope: 1,
        scope: 0,
        stack: Vec::new(),
        var_scope: BTreeMap::new(),
        var_order: Vec::new(),
        var_mapping: HashMap::new(),
        confined: confined.clone(),
        loops: vec![],
        scan_depth: 0,
        lift_counter: 0,
        lift_vars: Vec::new(),
        ret_temp_counter: 0,
        paired_witness: HashMap::new(),
        witness_buffer: HashMap::new(),
        owned_refs: HashMap::new(),
    };
    let mut function = Function::copy(orig_vars);
    for a in function.arguments() {
        scopes.var_scope.insert(a, 0);
    }
    let mut code = scopes.scan(orig_code, &mut function, data);
    // lift vars from `scan_args` are assigned inside conditional branches but
    // their `OpFreeRef` lives at function exit; prepend the null-inits so codegen
    // reserves their slot along every path (see the original comment in check).
    if !scopes.lift_vars.is_empty()
        && let Value::Block(bl) = &mut code
    {
        for &v in scopes.lift_vars.iter().rev() {
            bl.operators.insert(0, v_set(v, Value::Null));
        }
    }
    data.definitions[d_nr as usize].code = code;
    data.definitions[d_nr as usize].variables = function;
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
}

/// True if `op` is the null-init `Set(vdb, Null)` — a work-ref's `first_def`.
fn is_var_null_init(op: &Value, vdb: u16) -> bool {
    matches!(op.unspan(), Value::Set(v, val) if *v == vdb && matches!(val.unspan(), Value::Null))
}

/// Prepend `ni` to the operators of the Block whose `scope == target`.  Returns
/// `None` once inserted, or `Some(ni)` (un-consumed) if no such block was found.
fn prepend_to_scope(node: &mut Value, target: u16, ni: Value) -> Option<Value> {
    match node {
        Value::Block(bl) if bl.scope == target => {
            bl.operators.insert(0, ni);
            None
        }
        Value::Block(bl) | Value::Loop(bl) => {
            let mut carry = Some(ni);
            for op in &mut bl.operators {
                carry = prepend_to_scope(op, target, carry.take().unwrap());
                carry.as_ref()?;
            }
            carry
        }
        Value::Insert(ls) => {
            let mut carry = Some(ni);
            for op in ls {
                carry = prepend_to_scope(op, target, carry.take().unwrap());
                carry.as_ref()?;
            }
            carry
        }
        Value::If(c, t, e) => {
            let ni = prepend_to_scope(c, target, ni)?;
            let ni = prepend_to_scope(t, target, ni)?;
            prepend_to_scope(e, target, ni)
        }
        Value::Span(b) => prepend_to_scope(&mut b.1, target, ni),
        Value::Return(b) | Value::Drop(b) | Value::Yield(b) => prepend_to_scope(b, target, ni),
        _ => Some(ni),
    }
}

/// Plan-57 cluster-I experiment: move a confined `__vdb`'s null-init
/// `Set(vdb, Null)` from body position 0 into its confined block, so the slot's
/// `first_def` (and therefore the SLOT, and codegen's free) live in the block —
/// not just the IR `OpFreeRef`.  The body-0 hoist (`parse_code`) was a
/// correctness over-reach made *without* lifetime info; the confinement analysis
/// now supplies that info, so the slot can live in its real scope.
fn relocate_null_init(code: &mut Value, vdb: u16, block_scope: u16) -> bool {
    let ni = {
        let Value::Block(body) = code else {
            return false;
        };
        let Some(pos) = body
            .operators
            .iter()
            .position(|op| is_var_null_init(op, vdb))
        else {
            return false;
        };
        body.operators.remove(pos)
    };
    if let Some(ni) = prepend_to_scope(code, block_scope, ni) {
        // Block not found — restore the null-init so the first_def is never lost.
        if let Value::Block(body) = code {
            body.operators.insert(0, ni);
        }
        debug_assert!(
            false,
            "relocate_null_init: block scope {block_scope} not found"
        );
        false
    } else {
        true
    }
}

/// Scope / lifetime analysis pass over every function definition.
///
/// # Panics
/// Under the `LASTUSE_RECLAIM` gate only (a Plan-57 testing build), panics if the
/// reclaim pass left a store the model says is dead un-freed past a later
/// allocation (the Phase-4 Goal-E watermark guard).  Never panics in normal builds.
pub fn check(data: &mut Data) {
    // Plan-57 store-identity gate (Phase 2.5): emit the verifying store ops only
    // when LOFT_STORE_TAG is set.  Counter is global so ids are unique across
    // functions (a cross-function wrong-store free mismatches).
    let tag_mode = std::env::var("LOFT_STORE_TAG").is_ok();
    let mut tag_counter = 1u16;
    // Plan-57 Phase 5: last-use freeing (reclaim) is ON by default.  `LASTUSE_RECLAIM_OFF`
    // disables it for A/B watermark measurement.  The Goal-E enforcement assert runs in
    // debug builds always, and in release on demand via `LOFT_STORE_GUARD`.
    let reclaim_off = std::env::var("LASTUSE_RECLAIM_OFF").is_ok();
    let reclaim_guard = cfg!(debug_assertions) || std::env::var("LOFT_STORE_GUARD").is_ok();
    // Positive-control fault injection (test-only, never set in production): skip the
    // early-free insertion below while STILL running the Phase-4 guard, so a program
    // with reclaim-eligible stores trips the assertion.  This makes the Goal-E guard
    // *falsifiable* — proving it fires on a real reclaim regression, so its silence on
    // the corpus is evidence.  Differs from `LASTUSE_RECLAIM_OFF` (which also disables
    // the guard); correctness is preserved either way by the scope-exit `OpFreeRef`.
    let inject_unfreed = reclaim_guard && std::env::var("LOFT_STORE_GUARD_INJECT").is_ok();
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) || data.def(d_nr).variables.done {
            continue;
        }
        let free_ref_nr = data.def_nr("OpFreeRef");
        let orig_code = data.definitions[d_nr as usize].code.clone();
        let orig_vars = Function::copy(&data.def(d_nr).variables);
        // Phase 1: the normal scan → apply → set-scope pass.
        run_scan_phase(data, d_nr, &orig_code, &orig_vars, &HashMap::new());
        // Plan-57 cluster I-a — two-phase scan.  If a vector store is block-confined,
        // re-scan registering its `__vdb` (+ backed local) at the confined block scope
        // so the block-exit `free_vars` sweep frees the store there, then relocate the
        // null-init into that block (so its `first_def` / codegen free live there too).
        let confined = store_confinement(
            &data.definitions[d_nr as usize].code,
            &data.definitions[d_nr as usize].variables,
            free_ref_nr,
            data.def_nr("OpGetField"),
        );
        if !confined.is_empty() {
            let mut cmap: HashMap<u16, u16> = HashMap::new();
            for (&vdb, &(local, b)) in &confined {
                cmap.insert(vdb, b);
                // Register the backed local at the block only when it is
                // single-store (its lifetime == the store's).  A multi-store
                // local (shared `z`) spans several sibling blocks, so it stays
                // function-scoped; only its per-block stores move.
                if data.def(d_nr).variables.tp(local).depend().len() == 1 {
                    cmap.insert(local, b);
                }
            }
            run_scan_phase(data, d_nr, &orig_code, &orig_vars, &cmap);
            for (&vdb, &(_local, b)) in &confined {
                relocate_null_init(&mut data.definitions[d_nr as usize].code, vdb, b);
            }
        }
        // Plan-57 last-use freeing, Phase 3 (DEFAULT since Phase 5): null-init
        // relocation + early free — the combination that lowers the body-0-locked
        // watermark.  Runs before compute_intervals so the moved first_def is
        // reflected.  `LASTUSE_RECLAIM_OFF` disables it for A/B measurement.
        if !reclaim_off {
            let db_nr = data.def_nr("OpDatabase");
            let gf_nr = data.def_nr("OpGetField");
            if !inject_unfreed {
                let d = &mut data.definitions[d_nr as usize];
                lastuse_reclaim(&mut d.code, &d.variables, db_nr, gf_nr, free_ref_nr);
            }
            // Plan-57 Phase 4 — Goal-E enforcement (THE watermark guard, supersedes
            // the scope-exit `store_lifetime_guard`).  Every store the model says is
            // dead and reclaim claimed (its `intent`) must now be freed before its
            // sibling allocates; a non-zero count is a reclaim regression — the rule
            // silently re-acquired an exception.  On in debug; release on demand via
            // LOFT_STORE_GUARD (`reclaim_guard`); zero-cost otherwise.
            if reclaim_guard {
                let d = &data.definitions[d_nr as usize];
                let unfreed =
                    reclaim_unfreed_eligible(&d.code, &d.variables, db_nr, gf_nr, free_ref_nr);
                assert_eq!(
                    unfreed, 0,
                    "plan-57 Phase 4: {} left {unfreed} reclaim-eligible store(s) live-but-dead past a later alloc",
                    d.name,
                );
            }
        }
        // Plan-57 store-identity gate (Phase 2.5): rewrite store ops to verifying
        // variants (gated; no-op in normal builds).
        if tag_mode {
            let db_nr = data.def_nr("OpDatabase");
            let gf_nr = data.def_nr("OpGetField");
            let store_tag_nr = data.def_nr("OpStoreTag");
            let free_ref_tag_nr = data.def_nr("OpFreeRefTag");
            // Scope the gate to the reclaim-eligible stores (the same `owning` set
            // `lastuse_reclaim` acts on).  Adopted / shared / file-reused stores are
            // NOT eligible, so they stay untagged with plain OpFreeRef — the gate
            // verifies exactly the frees reclaim is responsible for and cannot
            // false-positive on legitimate store-sharing.
            let d = &data.definitions[d_nr as usize];
            let (owning, _intent) =
                reclaim_free_intent(&d.code, &d.variables, db_nr, gf_nr, free_ref_nr);
            let tagset: HashSet<u16> = owning.into_iter().collect();
            let mut ids: HashMap<u16, u16> = HashMap::new();
            tag_stores(
                &mut data.definitions[d_nr as usize].code,
                db_nr,
                free_ref_nr,
                store_tag_nr,
                free_ref_tag_nr,
                &tagset,
                &mut ids,
                &mut tag_counter,
            );
        }
        // Compute live intervals so validate_slots can check for slot conflicts after codegen.
        let free_text_nr = data.def_nr("OpFreeText");
        // Plan-57 cluster I: store-lifetime guard (diagnostic, gated).
        if std::env::var("LOFT_STORE_GUARD").is_ok() {
            let gf_nr = data.def_nr("OpGetField");
            let d = &data.definitions[d_nr as usize];
            store_lifetime_guard(&d.code, &d.variables, free_ref_nr, gf_nr, &d.name);
        }
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
        // Plan-57 last-use freeing, Phase 1: definition-point liveness diagnostic
        // (read-only).  Reports each function-scoped owning store held past its
        // last use while later allocations run — the I-b / III-straight-line
        // watermark divergence.  See fix-design-last-use-freeing.md.
        if std::env::var("LOFT_LASTUSE_GUARD").is_ok() {
            let db_nr = data.def_nr("OpDatabase");
            let gf_nr = data.def_nr("OpGetField");
            let d = &data.definitions[d_nr as usize];
            last_use_guard(&d.code, &d.variables, db_nr, gf_nr, free_ref_nr, &d.name);
        }
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
        // @PLAN53 cluster 2 / S4: in aligned mode each arg + the return-address
        // slot occupies a STEPPED span, so the locals start at Σ step(arg) +
        // step(4) — matching codegen's stepped args loop + return slot, which
        // keeps the frame base (args_base) 8-aligned.  Identity when off.
        let local_start: u16 = {
            let vars = &data.definitions[d_nr as usize].variables;
            let step = |s: u16| crate::variables::aligned_stack_step(u32::from(s)) as u16;
            let arg_size: u16 = vars
                .arguments()
                .iter()
                .map(|&a| step(size(vars.var_type(a), &Context::Argument)))
                .sum();
            arg_size + step(4) // return-address slot
        };
        // @PLAN53 — the aligned V2 allocator is the ONLY allocator.  Compute the
        // V2 layout from the (immutable) function intervals, reset stale local
        // slots, then apply it.  `apply_v2_result` also zeroes every block's
        // var_size: V2 is scope-blind, a single function-entry reserve (frame
        // hwm) covers all slots, so there are no per-block reserves.
        let result = {
            let d = &data.definitions[d_nr as usize];
            crate::variables::assign_slots_v2(&d.variables, local_start)
        };
        {
            let d = &mut data.definitions[d_nr as usize];
            d.variables.reset_local_slots();
            crate::variables::apply_v2_result(&mut d.variables, &mut d.code, &result);
        }
        #[cfg(debug_assertions)]
        {
            crate::variables::validate_slots(
                &data.definitions[d_nr as usize].variables,
                data,
                d_nr,
                true, // V2 is scope-blind — skip I7 (zone-frame invariant).
            );
            crate::variables::validate_alignment(&data.definitions[d_nr as usize].variables);
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
                // #316 — a loop body executes repeatedly: any ownership entry
                // the body touches is unreliable afterwards.  Keep only the
                // entries the body left unchanged.
                let owned_before = self.owned_refs.clone();
                let ls = self.convert(lp, function, data, false);
                self.owned_refs
                    .retain(|k, depth| owned_before.get(k) == Some(depth));
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
                    &HashSet::new(),
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
                    &HashSet::new(),
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
                    &HashSet::new(),
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
            Value::Iter(idx, create, next, extra) => {
                let scanned_create = self.scan(create, function, data);
                // #316 — `next`/`extra` execute once per iteration: drop any
                // ownership entry they touch (same rationale as Value::Loop).
                let owned_before = self.owned_refs.clone();
                let scanned_next = self.scan(next, function, data);
                let scanned_extra = self.scan(extra, function, data);
                self.owned_refs
                    .retain(|k, depth| owned_before.get(k) == Some(depth));
                Value::Iter(
                    *idx,
                    Box::new(scanned_create),
                    Box::new(scanned_next),
                    Box::new(scanned_extra),
                )
            }
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
            // @PLAN53 cluster 2: remap the var-numbers these IR nodes carry through
            // `var_mapping` — exactly like `Var`/`TupleGet`/`TuplePut` above.  They
            // were missing, so when a sibling scope reuses a name (`copy_variable`),
            // a fn-ref loop break-test (`FnRefDnr`) or a copied closure capture
            // (`FnRef.clos_var`) kept pointing at the ORIGINAL var.  V1 masked it (the
            // original + copy share a slot); V2 gives them distinct slots, so the
            // stale read hit the wrong slot (repro_p352: 2nd reused-name fn-ref loop
            // read loop-1's exhausted sentinel → 0).  `clos_var == u16::MAX`
            // (non-capturing) is left untouched — `var_mapping` never holds u16::MAX.
            Value::FnRefDnr(var) => Value::FnRefDnr(*self.var_mapping.get(var).unwrap_or(var)),
            Value::FnRef(d_nr, clos_var, fn_type) => Value::FnRef(
                *d_nr,
                *self.var_mapping.get(clos_var).unwrap_or(clos_var),
                fn_type.clone(),
            ),
            Value::Yield(inner) => Value::Yield(Box::new(self.scan(inner, function, data))),
            Value::Span(b) => {
                let scanned = self.scan(&b.1, function, data);
                // When scanning lifted an inline struct-returning-call argument
                // (@P297), the result is `Insert([Set(__lift_N, …), final])` — a
                // statement sequence, not a positioned expression.  Re-wrapping
                // it in a Span hides the lift preamble from the consumers that
                // hoist it to statement level (`scan_set`'s flatten and
                // `scan_args`'s `is_p135_hoisted` bubbling, both `if let
                // Value::Insert`).  The interpreter tolerates the hidden Insert;
                // the native backend would emit `Set(__lift_N, …)` inside an
                // enclosing expression and fail to compile.  Inner ops keep
                // their own positions, so dropping the outer span is safe.
                //
                // SURGICAL: only unwrap when the Insert's leading op is a lift
                // `Set(__lift_N, …)`.  Other span-wrapped Inserts (closure-record
                // construction, etc.) MUST keep their span — unwrapping them
                // broadly regressed the closure-in-struct-field cases (`invalid
                // fn-ref` in native codegen, @P258/@P259 territory).
                let is_lift_preamble = matches!(&scanned, Value::Insert(ops)
                    if ops.first().is_some_and(|op| matches!(op,
                        Value::Set(v, _) if function.name(*v).starts_with("__lift_"))));
                if is_lift_preamble {
                    scanned
                } else {
                    Value::with_span(b.0.clone(), scanned)
                }
            }
            Value::ParFor(b) => {
                // Plan-06 spine step 3 — recurse into each child Value.
                // No new scope is opened by ParFor itself: the worker fn
                // runs in a worker State (separate scope), and `body`
                // runs in the enclosing scope on the main thread.
                Value::ParFor(Box::new(crate::data::ParForBody {
                    input: self.scan(&b.input, function, data),
                    x_var: b.x_var,
                    r_var: b.r_var,
                    worker: self.scan(&b.worker, function, data),
                    threads: self.scan(&b.threads, function, data),
                    body: self.scan(&b.body, function, data),
                    stitch_id: b.stitch_id,
                }))
            }
            _ => val.clone(),
        }
    }

    /// Register `v`'s scope.  Normally the current scope, but on the gated
    /// phase-2 re-scan a confined `__vdb`/local registers at its block scope
    /// (plan-57 cluster I) so the block-exit `free_vars` sweep frees its store
    /// there instead of at function exit.  `confined` is empty on phase 1, so
    /// this is identical to `var_scope.insert(v, self.scope)` in the common case.
    fn put_scope(&mut self, v: u16) {
        let scope = self.confined.get(&v).copied().unwrap_or(self.scope);
        self.var_scope.insert(v, scope);
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
        // #316 — capture BEFORE put_scope below: an ownership-transition free
        // only applies to a REassignment.
        let was_in_scope = self.var_scope.contains_key(&v);
        // A redundant re-init `Set(v, Null)` for an already-in-scope var is
        // elided (Reference/Vector/Enum/Text locals don't need re-null-ing).
        // EXCEPTION (@P302): keyed collections — `s = []` lowers to
        // `Set(s, Null)`, which on a reassignment is a genuine CLEAR (codegen
        // emits an in-place `OpDatabase`).  Eliding it left the old contents
        // intact (silent no-op) and leaked `s`'s store.  Let keyed Set-Null
        // through so codegen's keyed reassign arm clears in place.
        if self.var_scope.contains_key(&v)
            && *value == Value::Null
            && !matches!(
                function.tp(v),
                Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Spacial(_, _, _)
            )
        {
            return Value::Insert(Vec::new());
        }
        // #316 — ownership-transition free.  When this var's latest scanned
        // assignment gave it an OWNED store and this reassignment installs a
        // BORROW, the merged static type already carries deps, so codegen's
        // dep-empty pre-Set free never fires and the owned store is orphaned
        // (`chosen = m_none(); chosen = pool[i] ?? m_none()` leaked one store
        // per call).  Emit the free here, in the IR, before the new value
        // lands.  Depth guard: only at the loop depth that owned the store —
        // inside a deeper loop the free would re-run on iterations 2+ and
        // release the previous iteration's VIEWED store.
        let mut transition_free: Option<Value> = None;
        if was_in_scope
            && matches!(function.tp(v), Type::Reference(_, d) if !d.is_empty())
            && self.owned_refs.get(&v) == Some(&self.loops.len())
            && matches!(
                Self::ref_rhs_ownership(value, function, data, v),
                RefRhs::View
            )
            // `value` is pre-scan IR: reads may name the original id (`ov`)
            // or the remapped one (`v`) — guard against both.  A self-
            // reading borrow (`x = x.next`, #328) keeps its owned store
            // until scope exit — a bounded, documented residual of this
            // conservatism (LIFETIME.md § Ownership-transition free).
            && !value.reads_var(v)
            && !value.reads_var(ov)
        {
            transition_free = Some(call("OpFreeRef", v, data));
        }
        // Track the LATEST assignment's ownership for this var.
        if matches!(function.tp(v), Type::Reference(_, _)) {
            match Self::ref_rhs_ownership(value, function, data, v) {
                RefRhs::Owned => {
                    self.owned_refs.insert(v, self.loops.len());
                }
                RefRhs::View | RefRhs::Unknown => {
                    self.owned_refs.remove(&v);
                }
            }
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
                self.put_scope(d);
                self.var_order.push(d);
            }
        }
        if !self.var_scope.contains_key(&v) {
            self.put_scope(v);
            self.var_order.push(v);
        }
        // When a Reference variable is assigned from a user-function call,
        // codegen has two sub-paths (state/codegen.rs gen_set_first_at_tos /
        // gen_set_first_ref_call_copy), keyed on the SAME carried adopt-vs-copy
        // fact `Definition::return_adopts_fresh_store()` (Cluster A.3,
        // OWNERSHIP_MODEL row 102):
        // - adopts_fresh_store == false → the return is tied to a passed
        //   buffer/param (a visible arg it aliases, or a hidden ref_return
        //   work-ref the caller reuses); gen_set_first_ref_call_copy deep-copies
        //   into a FRESH store `v` owns.
        // - adopts_fresh_store == true → the return is genuinely fresh (empty
        //   dep or the `["??"]` one-buffer marker); `v` adopts the callee's
        //   store (the callee's `__ref_N` store IS the returned struct's store),
        //   OR the callee minted a different fresh store and the caller's
        //   `__ref_N` pre-alloc is orphaned.
        // This reads the precise carried adopt-vs-copy fact rather than the
        // coarse "callee has any visible ref param" proxy `has_ref_params` the
        // 11 sites used to re-derive (A.3): a callee with a ref param that
        // returns a *fresh* store (`fn mk_from(seed) -> Box { Box { v: [...] } }`)
        // now adopts instead of wastefully deep-copying, while a hidden
        // work-ref return (`fn render(p) -> Canvas { cv = …; cv }`, dep
        // `["cv"]`) still copies — the coarse proxy lumped both as "copy".
        // P198 — most operators are wrapped in Value::Span by the parser
        // for diagnostics.  Unwrap before pattern-matching so the
        // deep-copy / make_independent logic fires for Span(Call(...))
        // assignments — without this, OpFreeRef is never emitted for the
        // freshly-allocated store and Database N leaks at scope exit
        // (e.g. tests/scripts/95-alias-copy.loft Database 3 leak).
        let unspanned_value = value.unspan();
        if matches!(
            function.tp(v),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && let Value::Call(fn_nr, _) = unspanned_value
            && data.def(*fn_nr).name.starts_with("n_")
            && data.def(*fn_nr).code != Value::Null
        {
            let adopts_fresh_store = data.def(*fn_nr).return_adopts_fresh_store();
            if !adopts_fresh_store {
                // codegen will take gen_set_first_ref_call_copy —
                // OpConvRefFromNull +
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
            // `adopts_fresh_store == true` call whose result is assigned
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
            if adopts_fresh_store && let Value::Call(_, args) = unspanned_value {
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
                            } else if v_scope != u16::MAX
                                && av_scope != u16::MAX
                                && v_scope > av_scope
                            {
                                // @P378(a) — witness `v` is INNER-scoped (e.g.
                                // a loop body) while the `__ref_N` buffer `av`
                                // is OUTER (function).  The buffer is reserved
                                // once but `v` (which adopts the buffer's
                                // store) is freed every iteration; that frees
                                // the buffer's store, which `find_free_slot`
                                // then recycles to a callee temp next
                                // iteration — two OpDatabase targets collide on
                                // one record (self-referential keyed insert →
                                // SIGSEGV).  Make `v`'s per-iteration free
                                // conditional on NOT aliasing the buffer:
                                // adoption → skip (store stays reserved, freed
                                // once by the buffer's function-exit OpFreeRef);
                                // fresh-store → real free.  Scope-safe for
                                // native because `av` (outer) outlives `v`.
                                self.witness_buffer.entry(v).or_insert(av);
                            }
                        }
                    }
                }
            }
        }
        // Companion to the !adopts_fresh_store (deep-copy) branch above for the
        // var-to-var deep-copy path.  When `Set(v, Var(src))` and
        // both are References to the same struct, codegen takes
        // `gen_set_first_ref_var_copy` (state/codegen.rs:1025-1033)
        // which OpConvRefFromNull + OpDatabase + OpCopyRecord
        // deep-copies src into a FRESH store owned by `v`.  This
        // path is hit by the I13 iterator protocol's hidden
        // `__iter_obj_N = c` setup (parser/collections.rs:209).
        // Strip v's declared deps so get_free_vars emits OpFreeRef.
        if let Value::Var(src) = unspanned_value
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
        // #316 — the ownership-transition free runs FIRST: before the dep
        // inits, the hoisted RHS preamble, and the Set itself, so the owned
        // store is released before any part of the new value is computed.
        if let Some(free) = transition_free {
            prefix.push(free);
        }
        for d in depend {
            if d == v {
                continue;
            }
            if matches!(function.tp(d), Type::Text(_)) {
                prefix.push(v_set(d, Value::Text(String::new())));
            } else {
                prefix.push(v_set(d, Value::Null));
            }
            self.put_scope(d);
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

    /// #316 — classify the (pre-scan) RHS of a `Set` into Reference var `v`.
    /// Only two shapes are provably OWNED: a user-fn call whose declared
    /// return carries no visible-attribute dep (the callee materialises /
    /// owns its result), and a same-struct `Var` copy (codegen deep-copies
    /// both first assignment and reassignment).  A `Block` whose result type
    /// carries deps is a view — unless a dep names `v` itself (the new value
    /// might point into the store about to be freed).
    fn ref_rhs_ownership(value: &Value, function: &Function, data: &Data, v: u16) -> RefRhs {
        match value.unspan() {
            Value::Call(d, _)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name().starts_with("n_") =>
            {
                if let Type::Reference(_, deps) = data.def(*d).returned() {
                    let attrs = data.def(*d).attributes();
                    let visible_dep = deps
                        .iter()
                        .any(|&i| (i as usize) >= attrs.len() || !attrs[i as usize].hidden);
                    if visible_dep {
                        RefRhs::View
                    } else {
                        RefRhs::Owned
                    }
                } else {
                    RefRhs::Unknown
                }
            }
            Value::Var(src)
                if *src < function.count()
                    && matches!(
                        (function.tp(v), function.tp(*src)),
                        (Type::Reference(a, _), Type::Reference(b, _)) if a == b
                    ) =>
            {
                RefRhs::Owned
            }
            Value::Block(bl) => match &bl.result {
                Type::Reference(_, deps) if !deps.is_empty() => {
                    if deps.contains(&v) {
                        RefRhs::Unknown
                    } else {
                        RefRhs::View
                    }
                }
                _ => RefRhs::Unknown,
            },
            _ => RefRhs::Unknown,
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
            self.put_scope(v);
            self.var_order.push(v);
        }
        // Register small variables assigned in both branches at the parent scope too.
        for &v in &small_both {
            self.put_scope(v);
            self.var_order.push(v);
        }

        let scanned_test = self.scan(test, function, data);
        // #316 — ownership state is path-sensitive: scan each branch from the
        // same pre-If state, then keep only entries BOTH branches agree on.
        let owned_before = self.owned_refs.clone();
        let scanned_true = self.scan(t_val, function, data);
        let owned_after_true = std::mem::replace(&mut self.owned_refs, owned_before);
        let scanned_false = self.scan(f_val, function, data);
        self.owned_refs
            .retain(|k, depth| owned_after_true.get(k) == Some(depth));
        let scanned_if = Value::If(
            Box::new(scanned_test),
            Box::new(scanned_true),
            Box::new(scanned_false),
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
        // @PLN85 cluster II / A.1 part i (OWNERSHIP_MODEL row 100, invariant #5
        // "per binding, per path, complete") — the return-source SET, not the
        // single `returned_var`, drives free-suppression.  `returned_var`
        // collapses an arms-differ `match`/`if` to `u16::MAX`, which would free
        // every arm's transferred buffer at scope exit (the freed-return bug,
        // #405 / probe 05).  `collect_return_sources` is the union of every arm's
        // terminal var — the values this return transfers to the caller.
        //
        // The suppression is PATH-LOCAL: it is computed per `free_vars`
        // invocation and consumed only by THIS scope-exit's `get_free_vars`, so a
        // variable transferred on this return path is suppressed here while the
        // SAME variable, dead on a sibling path (an early `return e` vs a tail
        // `return [..]`, or the `null` arm of a nullable return), is still freed
        // by its own path's sweep.  A single global `skip_free` bit cannot encode
        // that path-dependence — marking the source do-not-free everywhere
        // over-suppresses and LEAKS the dead-path allocation (repro_p365's
        // `via_local`/`nested`, 25-nullable's `maybe_row`).  So the set is passed
        // down, not stamped onto the variable.
        let return_sources: HashSet<u16> = if is_return {
            let mut sources = Vec::new();
            collect_return_sources(expr, &mut sources);
            // A nullable return (`if b { Struct{} } else { null }`) leaves the
            // present arm's work-ref placeholder orphaned on the null path.  When
            // a null arm is reachable, do NOT SET-suppress a Reference/Enum work-
            // ref source — hand it to the standard work-ref free path so the
            // orphan is freed.  Vector sources are unaffected (their backing is
            // NRVO-delivered, not orphaned).  See `return_has_null_arm`.
            let null_sentinel_nr = data.def_nr("OpNullRefSentinel");
            if return_has_null_arm(expr, null_sentinel_nr) {
                sources.retain(|&v| {
                    !matches!(
                        function.tp(v),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                });
            }
            sources.into_iter().collect()
        } else {
            HashSet::new()
        };
        let mut ls = self.get_free_vars(function, data, to_scope, tp, ret_var, &return_sources);
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
        } else if is_return
            && let Type::Tuple(elems) = tp
            && elems.iter().any(|e| matches!(e, Type::Text(_)))
            && !expr_is_terminal
            && let Value::Tuple(orig_elems) = expr.unspan()
            && orig_elems.len() == elems.len()
        {
            // @P329: tuple-of-text return — when an element is a non-literal
            // expression (typically a Call returning text that borrows from
            // a local), hoist it to a `__ret_text_N` temp before running
            // scope frees.  `Set(__ret_text_N, elem)` lowers to OpAppendText
            // (line 526 / src/state/codegen.rs), which deep-copies bytes
            // into the temp's owned String.  The frees then run safely; the
            // returned tuple's text elements point to temp Strings that
            // outlive the function's scope (skip_free marks them so the
            // function epilogue leaves the allocation for the caller's
            // AppendText to consume on return — same pattern as the
            // single-text B5-L3 branch above, generalised across tuple
            // elements).
            //
            // Without this, a function shape like
            //   fn f<T: Printable>(x: T) -> (text, text) { (x.to_text(), "x") }
            // returns a tuple whose element 0 Str points into the caller's
            // (function-local) __work_1 buffer; the scope's OpFreeText runs
            // BEFORE the Return, invalidating the Str — the caller reads
            // empty / garbage bytes.  See PROBLEMS.md @P329.
            let mut new_elems = Vec::with_capacity(orig_elems.len());
            let mut pre_ops = Vec::new();
            for (elem_expr, elem_type) in orig_elems.iter().zip(elems.iter()) {
                let unspanned = elem_expr.unspan();
                if matches!(elem_type, Type::Text(_))
                    && !matches!(unspanned, Value::Text(_) | Value::Var(_) | Value::Null)
                {
                    self.ret_temp_counter += 1;
                    let name = format!("__ret_text_{}", self.ret_temp_counter);
                    let tmp = function.add_temp_var(&name, &Type::Text(Deps::none()));
                    function.set_skip_free(tmp);
                    self.var_scope.insert(tmp, self.scope);
                    self.var_order.push(tmp);
                    pre_ops.push(v_set(tmp, elem_expr.clone()));
                    new_elems.push(Value::Var(tmp));
                } else {
                    new_elems.push(elem_expr.clone());
                }
            }
            let mut result = Vec::with_capacity(pre_ops.len() + ls.len() + 1);
            result.extend(pre_ops);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Tuple(new_elems))));
            return result;
        } else if is_return
            && !expr_is_terminal
            && matches!(
                tp,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
            && matches!(expr.unspan(), Value::If(c, _, _)
                if matches!(c.unspan(), Value::Insert(ops)
                    if ops.first().is_some_and(|o| matches!(o,
                        Value::Set(v, _) if function.name(*v).starts_with("__lift_")))))
        {
            // @P378(b) — a heap-returning tail `If` whose CONDITION lifted a
            // ref-temp (`__lift_N = call()`; the branches were not unified to a
            // shared work-ref, so `ret_var == u16::MAX`) and which has frees to
            // run before returning.  The fall-through (below) inserts the If as
            // a discarded statement + `Return(Null)`: interpret reads the value
            // off eval-stack-top, but native returns the null DbRef sentinel
            // (keys.rs:251 OOB in the caller).  A `Set(var, If)` save-to-temp
            // does NOT work (native voids the if/else branches → E0308); only
            // `Return(If(...))` value-emits them.  So PRESERVE the Return(If):
            // pull the condition's lift-preamble out, evaluate the boolean to a
            // value-typed temp, run the frees (the lift's OpFreeRef), then
            // `Return(If(Var(cond_tmp), t, f))` — the if/else stays the Return's
            // value-expression and the lift is freed before (not after) it.
            let Value::If(cond, t, f) = expr.unspan().clone() else {
                unreachable!()
            };
            let mut result = Vec::new();
            // split `Insert([__lift = …, …, bool_expr])` into preamble + bool.
            let bool_expr = if let Value::Insert(ops) = cond.unspan() {
                let mut ops = ops.clone();
                let last = ops.pop().expect("non-empty condition Insert");
                result.extend(ops);
                last
            } else {
                (*cond).clone()
            };
            self.ret_temp_counter += 1;
            let cname = format!("__cond_{}", self.ret_temp_counter);
            let cond_tmp = function.add_temp_var(&cname, &Type::Boolean);
            self.var_scope.insert(cond_tmp, self.scope);
            self.var_order.push(cond_tmp);
            result.push(v_set(cond_tmp, bool_expr));
            result.extend(ls);
            result.push(Value::Return(Box::new(v_if(Value::Var(cond_tmp), *t, *f))));
            return result;
        } else {
            ls.insert(0, expr.clone());
            if is_return {
                // P236: when `expr` is an `If/Match` whose unified
                // tail var is known (returned_var(expr) recurses
                // through If — see line 1454), emit
                // `Return(Var(ret_var))` instead of the legacy
                // `Return(Null)` pattern.  The legacy pattern relied on
                // OpReturn(value=N) reading from eval-stack top —
                // bytecode-only, native discards the if/else's value
                // and returns the typed null sentinel.  After Step 2
                // unification (parser/control.rs::unify_if_branches_work_refs)
                // every branch of the if/else writes to the SAME work-
                // ref via OpDatabase + per-field SetInt; the if/else
                // statement leaves all writes in place; native
                // `return var___ref_N` then returns the active
                // branch's value correctly.
                if ret_var == u16::MAX {
                    ls.push(Value::Return(Box::new(Value::Null)));
                } else {
                    ls.push(Value::Return(Box::new(Value::Var(ret_var))));
                }
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
        return_sources: &HashSet<u16>,
    ) -> Vec<Value> {
        let scope_debug = std::env::var("LOFT_LOG").as_deref() == Ok("scope_debug");
        let mut ls = Vec::new();
        let vars = self.variables(to_scope);
        if scope_debug {
            eprintln!(
                "[get_free_vars] fn={} to_scope={to_scope} scope={} vars={vars:?} ret_var={ret_var} \
                 return_sources={return_sources:?}",
                data.def(self.d_nr).name,
                self.scope
            );
        }
        // @PLN85 A.1 part i — the directly-owned heap BUFFER of EACH transferred
        // return arm (the union-of-arms SET, not just `returned_var`'s single
        // var) is freed by the caller, so suppress its scope-exit free here.
        // PATH-LOCAL: `return_sources` is this return's set, so a source dead on
        // a sibling path is absent and still freed by that path's sweep (no
        // global `skip_free` stamp — that would over-suppress and leak the
        // dead-path allocation).  The dep buffer of a BORROWING terminal
        // (`_vec_N["__vdb_N"]`) is handled in the `in_ret` computation below.
        //
        // The SET drives suppression ONLY for the heap-buffer block (Vector /
        // Reference / Enum / keyed).  TEXT and TUPLE returns keep their own,
        // mature free paths (`OpFreeText` + the B5-L3 `__ret_N` text hoist;
        // per-element tuple frees) — a `match`-returning-text whose owned
        // `__work_N` buffer is alive but unused on a sibling arm MUST still be
        // freed, and short-circuiting the whole iteration on `return_sources`
        // would leak it (the enum-vector param-return `show()` regression).
        let suppress_source = |function: &Function, v: u16| {
            return_sources.contains(&v)
                && matches!(
                    function.tp(v),
                    Type::Reference(_, _)
                        | Type::Vector(_, _)
                        | Type::Enum(_, true, _)
                        | Type::Sorted(_, _, _)
                        | Type::Hash(_, _, _)
                        | Type::Index(_, _, _)
                        | Type::Spacial(_, _, _)
                )
        };
        for v in vars {
            if v == ret_var || suppress_source(function, v) {
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
                // @PLAN52 cluster I iteration 2 (2026-05-30): honor skip_free
                // for text vars too.  The file-level "Text exception"
                // doc-comment ("OpFreeText is always emitted ... regardless
                // of deps") remains true for borrowed-from-parameter text
                // (`dep` non-empty, not skip_free).  The new rule only
                // suppresses OpFreeText for an EXPLICITLY-set skip_free text
                // temp — used by the `__ncc_N` null-coalesce temp at
                // `src/parser/operators.rs::build_null_coalesce_default` so
                // the present-path Str outlives the block scope.  Native
                // emit's `needs_ncc_materialise` (in `output_block`)
                // materialises an owned String inside the block tail so the
                // outer consumer takes ownership cleanly on both backends.
                if !function.is_skip_free(v) {
                    ls.push(call("OpFreeText", v, data));
                }
            }
            // P193: include keyed collections (Sorted/Hash/Index/Spacial)
            // — `gen_set_first_keyed_null` allocates a fresh store via
            // `OpDatabase` for each local-var keyed collection, so each
            // needs scope-exit `OpFreeRef`.  Without this they leak as
            // "Stores not freed at program exit".
            if let Type::Reference(_, dep)
            | Type::Vector(_, dep)
            | Type::Enum(_, true, dep)
            | Type::Sorted(_, _, dep)
            | Type::Hash(_, _, dep)
            | Type::Index(_, _, dep)
            | Type::Spacial(_, _, dep) = function.tp(v)
            {
                // H2 step 5 (DEPS_INVENTORY): the declared return type's
                // dep list is DEF-space — attr indices from `ref_return`,
                // plus tagged callee-frame notes (a returned fn-ref's
                // closure work var, `Deps::callee_frame1`).  Decode per
                // entry; the historical positional guess (in-range = attr
                // index, out-of-range = frame var) is retired.
                let def = data.def(self.d_nr);
                let ret_borrows_v = def.returned.deps_ref().is_some_and(|deps| {
                    deps.entries().any(|e| match e {
                        crate::data::DepEntry::Attr(a) => {
                            (a as usize) < def.attributes.len()
                                && function.var(&def.attributes[a as usize].name) == v
                        }
                        crate::data::DepEntry::CalleeFrame(w) => w == v,
                    })
                });
                // @PLN85 A.1 part i — `v` is the dep BUFFER of a borrowing
                // return-source terminal (`_vec_N["__vdb_N"]`, where `_vec_N` is
                // in `return_sources` and its dep names `v`): the buffer's store
                // is what the caller adopts, so suppress it here.  Path-local for
                // the same reason as the directly-owned case above.
                let backs_return_source = return_sources
                    .iter()
                    .any(|&src| src != v && function.tp(src).depend().contains(&v));
                let in_ret = ret_borrows_v
                    || backs_return_source
                    || ret_var != u16::MAX && function.tp(ret_var).depend().contains(&v);
                // H2 step-5 sentinel: the BLOCK-RESULT type's deps were
                // read here for years under the positional guess; the
                // 2026-06-12 corpus probes (scripts, docs, examples,
                // tools, libs) show that read never decides alone — the
                // declared-return and returned-var checks subsume it.
                // Scream if a live case ever appears; re-add the read
                // WITH a typed decode then (DEPS_INVENTORY § step 5).
                #[cfg(debug_assertions)]
                {
                    let tp_alone = !in_ret
                        && tp.depend().iter().any(|&a| {
                            if (a as usize) < def.attributes.len() {
                                function.var(&def.attributes[a as usize].name) == v
                            } else {
                                a == v
                            }
                        });
                    debug_assert!(
                        !tp_alone,
                        "H2 step-5 sentinel: the block-result dep read would have \
                         decided alone for var {v} in '{}'",
                        def.name()
                    );
                }
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
                // @P302 — a keyed-collection local backed by its OWN store
                // carries a self-dep `[v]` (added by the `s = []` clear path
                // so a later `s += …` re-inits in place).  That self-dep is an
                // ownership marker, not a borrow — treat it like `dep.is_empty()`
                // so the store is freed at scope exit.  Mirrors the fn-ref
                // ownership rule below.  Keyed-only + exact self-dep; `in_ret`
                // still suppresses returned keyed locals.
                let owns = dep.is_empty()
                    || (dep.len() == 1
                        && dep[0] == v
                        && matches!(
                            function.tp(v),
                            Type::Sorted(_, _, _)
                                | Type::Hash(_, _, _)
                                | Type::Index(_, _, _)
                                | Type::Spacial(_, _, _)
                        ));
                // Plan-57 Phase B (Mechanism B), widened by #323: a
                // Reference-typed capture — a boxed `__cell_<T>` AND, per
                // P260's storage rule, any plain struct capture — is OWNED
                // by the closure record, not the defining frame.  The
                // record stores the capture's 12-byte DbRef and
                // `free_named`'s cascade (allocation.rs) walks every DbRef
                // field when the record dies, so the defining-frame
                // `OpFreeRef` here is redundant — and actively harmful: for
                // an ESCAPED closure (factory return) it frees a slot the
                // live closure still references, the next allocation reuses
                // it, and the closure silently corrupts the new occupant
                // (#323; interp only *appeared* sound through slot-reuse
                // luck).  Suppress it; the cascade is the sole owner for
                // escaping AND in-frame captures (in-frame: the fn-ref's
                // own scope-exit free triggers the cascade).  Mirrored by
                // the captured-Reference exemption in `check_ref_leaks`.
                let captured_ref =
                    function.is_captured(v) && matches!(function.tp(v), Type::Reference(_, _));
                let emit =
                    (owns || is_work_ref) && !in_ret && !function.is_skip_free(v) && !captured_ref;
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
                    } else if let Some(&buffer) = self.witness_buffer.get(&v) {
                        // @P378(a) — `v` is an inner-scoped witness whose store
                        // is the outer `__ref_N` buffer (adoption).  Skip the
                        // per-iteration free when they still alias so the
                        // buffer stays reserved across iterations; the buffer's
                        // own function-exit OpFreeRef releases it once.
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(buffer)],
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
                // H2 step 5: the declared return's closure-work-var note is
                // a TAGGED callee-frame entry — decode it (a raw `contains`
                // never matches the tagged value, and an explicit
                // `return adder;` reaches here with an empty block-result
                // dep list, so the closure record would be freed under the
                // escaping fn-ref).
                let ret_carries = data.def(self.d_nr).returned.deps_ref().is_some_and(|d| {
                    d.entries()
                        .any(|e| matches!(e, crate::data::DepEntry::CalleeFrame(w) if w == v))
                });
                let in_ret = tp.depend().contains(&v) || ret_carries;
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
        // @P376 follow-up — no const-param unlock emitted anymore (matching
        // the dropped function-entry lock in `parser/expressions.rs`).  See
        // there for the rationale: compile-time const checks already cover
        // every mutation path, and the function-entry lock was a
        // false-positive trigger on iteration over a const-param's hash
        // field.  Par-worker `read_only` clones still enforce immutability
        // independently.
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
        // #248 (interpreter arg-layout) — when the call's first argument is a
        // borrowed receiver pushed via `OpCreateStack(Var(_))` (a `&self` / `&T`
        // method or free-function call), a LATER argument that is an inline
        // heap-returning call which grows the eval frame (e.g. `tick(s, mk(), …)`
        // where `mk()` returns a vector via a hidden `__ref_N` work buffer)
        // shifts the receiver's stack slot relative to where the callee reads
        // `self`.  The interpreter then derefs the receiver at the wrong offset
        // and lands on a CONST_STORE record → "Write to read-only store".  The
        // native backend passes args by the Rust ABI and is immune.  Force such a
        // trailing call argument into a `__lift_N` temp (preamble), so it is
        // evaluated and its store materialised BEFORE the receiver `OpCreateStack`
        // is pushed — exactly the shape `x = mk(); tick(s, x, …)` that already
        // works on both backends.  Gated on a CreateStack-receiver first arg so
        // ordinary calls keep their existing argument lowering untouched.
        let create_stack_nr = data.def_nr("OpCreateStack");
        let has_create_stack_receiver = args.first().is_some_and(|a| {
            matches!(a.unspan(), Value::Call(d, cargs)
                if *d == create_stack_nr
                    && matches!(cargs.first().map(Value::unspan), Some(Value::Var(_))))
        });
        for (arg_idx, a) in args.iter().enumerate() {
            let scanned = self.scan(a, function, data);
            // #248 — force-lift a trailing inline heap-returning call argument
            // (one NOT already lifted by the `inline_struct_return` arms below
            // because it returns via a hidden work-ref / non-empty dep) when the
            // receiver is a borrowed CreateStack ref.  Must run before the
            // Insert/`inline_struct_return` handling so it is not skipped for the
            // exact shape that triggers the bug.
            if has_create_stack_receiver
                && arg_idx > 0
                && Self::inline_struct_return(&scanned, data, outer_call).is_none()
                && let Some(tp) = Self::heap_call_return(&scanned, data)
            {
                self.lift_counter += 1;
                let name = format!("__lift_{}", self.lift_counter);
                let tmp = function.add_temp_var(&name, &tp);
                function.mark_inline_ref(tmp);
                self.var_scope.insert(tmp, self.scope);
                self.var_order.push(tmp);
                self.lift_vars.push(tmp);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
                continue;
            }
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

    /// #248 — does this scanned argument lower to an inline call (or an
    /// `Insert`/`Span` whose final op is one) that PRODUCES a heap value
    /// (vector / reference / struct-enum)?  Used by `scan_args` to force-lift a
    /// trailing heap-returning call argument when the call's receiver is a
    /// borrowed `OpCreateStack` ref — the interpreter arg-layout hazard #248.
    ///
    /// Unlike [`inline_struct_return`], this DOES match calls that return via a
    /// hidden caller work-ref (non-empty dep like `["??"]`): those are exactly
    /// the frame-growing inline calls that `inline_struct_return` skips but that
    /// still shift the receiver's stack slot.  Returns the owned element/struct
    /// type (empty dep) for the `__lift_N` temp, or `None`.
    fn heap_call_return(val: &Value, data: &Data) -> Option<Type> {
        // Peel `Span` / trailing-op-of-`Insert` to reach the producing call.
        let inner = match val.unspan() {
            Value::Insert(ops) => ops.last()?.unspan(),
            other => other,
        };
        let Value::Call(fn_nr, _) = inner else {
            return None;
        };
        let def = data.def(*fn_nr);
        // Only user/method bodies (n_* / t_*) — native helpers and the
        // OpCreateStack/OpVar* lowering ops never own a fresh return store here.
        if (!def.name.starts_with("n_") && !def.name.starts_with("t_")) || def.code == Value::Null {
            return None;
        }
        match &def.returned {
            Type::Vector(elem, _) => Some(Type::Vector(elem.clone(), Deps::none())),
            Type::Reference(d_nr, _) => Some(Type::Reference(*d_nr, Deps::none())),
            Type::Enum(d_nr, true, _) => Some(Type::Enum(*d_nr, true, Deps::none())),
            _ => None,
        }
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
        // @P297 — a USER struct-returning call (`n_*` with a body) passed
        // directly as a call argument is wrapped in `Value::Span` by
        // `parse_call` (and re-wrapped by `scan`), so the argument reaching
        // here is `Span(Call(...))`.  Unspan before matching this branch or the
        // lift never fires and the call-result temporary leaks — the same
        // pitfall `scan_set` was patched for under @P198 (`value.unspan()`).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            if def.name.starts_with("n_") && def.code != Value::Null {
                if let Type::Reference(d_nr, _) = &def.returned {
                    return Some(Type::Reference(*d_nr, Deps::none()));
                }
                // @P303 — a user fn returning a struct-enum by FRESH owned
                // store (empty dep) leaks its result temp when used directly
                // as a call argument; lift it like the Reference case above so
                // `get_free_vars` emits its `OpFreeRef`.  A NON-empty dep means
                // a hidden-param return (@P301 via-local: ownership handled by
                // `add_defaults`'s `__ref_N` work-ref) or a borrowed view — must
                // NOT be lifted here.  Matches the native-constructor Enum
                // branch's `dep.is_empty()` guard below.
                if let Type::Enum(d_nr, true, dep) = &def.returned
                    && dep.is_empty()
                {
                    return Some(Type::Enum(*d_nr, true, Deps::none()));
                }
                // Plan-57: a user fn returning a CAPTURING closure (`fn(...) -> T`
                // whose fn-ref carries a fresh closure record) leaks its result temp
                // when used directly as a call argument — `apply(make())` left the
                // `__closure_*` record at rc 1 and the cell uncollected.  Lift it like
                // the Reference / struct-enum cases so `get_free_vars`' fn-ref arm
                // emits the `OpFreeRef`; codegen frees the closure DbRef at offset+8
                // and `free_named` cascades to the captured `__cell_*`.  NOT guarded on
                // `dep.is_empty()` — a capturing closure's dep IS the cell (e.g.
                // `function([], integer, [1])`), so guarding would skip the very case.
                // A non-capturing return carries the null closure sentinel → the free
                // is a safe no-op; a borrowed fn-ref copy is marked `skip_free`
                // elsewhere, so only a freshly produced closure is lifted here.
                if let Type::Function(params, ret, _) = &def.returned {
                    return Some(Type::Function(params.clone(), ret.clone(), Deps::none()));
                }
            }
        }
        // @P393 (t9) — a loft-source fn OR `t_` method returning an OWNED vector
        // BY VALUE (empty dep) is de-NRVO'd when its body builds the result with
        // >=2 distinct element-temps (`01a3f24f` in `ref_return`): the signature
        // is `n_f() -> vector<T>` / `t_..split() -> vector<text>`, with no hidden
        // `__vdb`/`__ref` buffer param.  Used inline-unbound (`len(split(x))`,
        // `split(x).join(y)`) the by-value temp gets no scope-exit free on the
        // interpreter → its store leaks (native codegen already frees it).  Lift
        // it like the Reference / struct-Enum / Function cases above so
        // `get_free_vars` emits the `OpFreeRef`.  A SEPARATE block from the
        // `n_`-gated one above because `split` is a `t_` method — broadening that
        // block's guard would also change the Reference/Enum/Function arms' scope.
        // Gated on `dep.is_empty()`: the NRVO'd hidden-param return carries dep
        // `["??"]` (caller already frees `__ref`) and a borrowed view carries
        // `[self]`, so both are excluded — no double-free, no UAF.  `val.unspan()`
        // because `parse_call`/`parse_method` Span-wrap the call (arg AND
        // receiver = arg0).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            if (def.name.starts_with("n_") || def.name.starts_with("t_"))
                && def.code != Value::Null
                && let Type::Vector(elem, dep) = &def.returned
                && dep.is_empty()
            {
                return Some(Type::Vector(elem.clone(), Deps::none()));
            }
        }
        // The native-constructor branches below are intentionally matched on the
        // BARE call only (no unspan).  Broadening them to span-wrapped calls
        // lifts a native constructor used as a method receiver (e.g.
        // `file(...).sync()`), which exposes native-codegen gaps for the lifted
        // method-receiver shape (wrong ABI arg / type mismatch).  They keep
        // their original reach: the chained-builtin case (`v.keys().len()`)
        // where the receiver is already a bare `Value::Call`.
        if let Value::Call(fn_nr, _) = val {
            let def = data.def(*fn_nr);
            // Native struct-enum constructors: no body (code == Null), return type
            // is a struct-enum with empty dep (allocates a new store, doesn't borrow).
            // Accessors carry dep=[0] after parser dep-inference and are skipped here.
            if def.code == Value::Null
                && let Type::Enum(d_nr, true, dep) = &def.returned
                && dep.is_empty()
            {
                return Some(Type::Enum(*d_nr, true, Deps::none()));
            }
            // Native vector-returning fns (e.g. `keys()`, `fields()` on
            // JsonValue) allocate a fresh vector store that the caller owns.
            // Without lifting, the chained call `v.keys().len()` leaks the
            // intermediate vector — same mechanism as the struct-return case.
            if def.code == Value::Null
                && let Type::Vector(elem, dep) = &def.returned
                && dep.is_empty()
            {
                return Some(Type::Vector(elem.clone(), Deps::none()));
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

/// #316 — what kind of store does the RHS of a `Set` into a Reference var
/// yield?  Conservative: anything not provably one of the two certain shapes
/// is `Unknown` (no ownership-transition free is emitted for it).
enum RefRhs {
    /// A store the variable will own (safe to free on a later transition).
    Owned,
    /// A borrowed view into someone else's store (must never be freed).
    View,
    /// Not provable either way.
    Unknown,
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
                // @P322 — when the function body ends with a nested
                // Void-result block whose last op is `Return(...)` (the
                // iterator-generator shape: `for n in […] { yield n; }
                // return null;`), the OUTER-scope frees passed in via
                // `free` must run BEFORE the return so function-scope
                // owned locals (`__vdb_*` vector backings, etc.) get
                // cleaned up.  Prior to this fix the void branch
                // dropped `free` entirely and only emitted the inner
                // `Return` + a redundant trailing `Return(Null)`, so
                // function-scope vectors leaked at program exit.
                let o_is_terminal = expr_ends_in_return(o);
                if o_is_terminal {
                    for v in free {
                        ls.push(v.clone());
                    }
                    ls.push(o.clone());
                } else {
                    ls.push(o.clone());
                    for v in free {
                        ls.push(v.clone());
                    }
                    ls.push(Value::Return(Box::new(Value::Null)));
                }
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
    matches!(expr.tail(), Value::Return(_))
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
        // P236: when both branches of a tail If terminate with the SAME
        // var (via the work-ref unification done in `block_result`'s
        // `unify_if_branches_work_refs`), report it so `get_free_vars`
        // skips OpFreeRef on it — same handling Block-with-tail-Var
        // returns get for single-branch reference returns.
        Value::If(_, t, f) => {
            let t_var = returned_var(t);
            let f_var = returned_var(f);
            if t_var == f_var && t_var != u16::MAX {
                t_var
            } else {
                u16::MAX
            }
        }
        Value::Span(b) => returned_var(&b.1),
        _ => u16::MAX,
    }
}

/// @PLN85 cluster II — the SET version of `returned_var`: every terminal var a
/// return expression can yield, INCLUDING all arms of an `If`/`match` (which
/// `returned_var` collapses to `u16::MAX` when the arms differ). These are the
/// function's "return-source" locals — their heap store is transferred to the
/// caller, so the callee must not free them at scope exit.
fn collect_return_sources(expr: &Value, out: &mut Vec<u16>) {
    match expr {
        Value::Var(v) => {
            if !out.contains(v) {
                out.push(*v);
            }
        }
        Value::Block(bl) => {
            if let Some(last) = bl.operators.last() {
                collect_return_sources(last, out);
            }
        }
        Value::Return(inner) | Value::Drop(inner) => collect_return_sources(inner, out),
        Value::Insert(ops) => {
            if let Some(last) = ops.last() {
                collect_return_sources(last, out);
            }
        }
        Value::If(_, t, f) => {
            collect_return_sources(t, out);
            collect_return_sources(f, out);
        }
        Value::Span(b) => collect_return_sources(&b.1, out),
        _ => {}
    }
}

/// @PLN85 A.1 part i — does this return expression have a reachable arm whose
/// terminal is the typed-null sentinel (`OpNullRefSentinel` / `Value::Null`)?
///
/// A nullable `if b { Struct{} } else { null }` allocates the present arm's
/// work-ref placeholder (`__ref_N`) at function entry, but the `null` arm
/// returns the sentinel and leaves that placeholder ORPHANED.  The SET-driven
/// free-suppression must therefore NOT suppress that work-ref: whether it is
/// transferred (present path) or orphaned-and-must-free (null path) is a RUNTIME
/// decision, not a static one — so the SET hands it back to the standard
/// work-ref free path (which frees the orphan correctly).  Resolving the runtime
/// split (NRVO-delivering each arm into `__retbuf`, or a conditional free) is the
/// callee-side control.rs / native work, out of scope-analysis's reach.
///
/// `null_sentinel_nr` is `OpNullRefSentinel`'s def number (resolved by the
/// caller, which holds `data`).
fn return_has_null_arm(expr: &Value, null_sentinel_nr: u32) -> bool {
    match expr.unspan() {
        Value::Null => true,
        Value::Call(d, _) => *d == null_sentinel_nr,
        Value::If(_, t, f) => {
            return_has_null_arm(t, null_sentinel_nr) || return_has_null_arm(f, null_sentinel_nr)
        }
        Value::Block(bl) => bl
            .operators
            .last()
            .is_some_and(|o| return_has_null_arm(o, null_sentinel_nr)),
        Value::Insert(ops) => ops
            .last()
            .is_some_and(|o| return_has_null_arm(o, null_sentinel_nr)),
        Value::Return(inner) | Value::Drop(inner) => return_has_null_arm(inner, null_sentinel_nr),
        _ => false,
    }
}

/// Recursively collect every variable freed by `OpFreeRef` in `ir`.
/// Used by `check_ref_leaks` to verify no Reference variable is leaked.
#[cfg(debug_assertions)]
fn collect_freed_vars(ir: &Value, free_ops: &[u32], result: &mut HashSet<u16>) {
    ir.walk(&mut |n| {
        if let Value::Call(d_nr, args) = n
            && free_ops.contains(d_nr)
            && let Some(Value::Var(v)) = args.first().map(Value::unspan)
        {
            result.insert(*v);
        }
    });
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
    collect_freed_vars(ir, &[free_text_nr], &mut freed);
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
    // Every op that FREES its first argument counts as that var's free
    // site: the plain scope-exit free, the @P317 tag-checked free, and
    // the witness-pair conditional free (`OpFreeRefIfDistinct` — how a
    // ref-returning call's work ref is released when it doesn't alias
    // the assigned var; the armed-corpus sweep's ~130 "no OpFreeRef"
    // false positives were all this shape).
    let free_ops = [
        data.def_nr("OpFreeRef"),
        data.def_nr("OpFreeRefTag"),
        data.def_nr("OpFreeRefIfDistinct"),
    ];
    let mut freed: HashSet<u16> = HashSet::new();
    collect_freed_vars(ir, &free_ops, &mut freed);

    // H2: `ret_type` deps are ATTRIBUTE indices — translate each to its
    // frame var through the attribute name before pooling with the
    // frame-space deps below (the old code inserted them raw, so an attr
    // index colliding with an unrelated var number silently suppressed a
    // leak report).
    let fn_def_nr = data.def_nr(fn_name);
    let mut ret_deps: HashSet<u16> = HashSet::new();
    for raw in ret_type.depend() {
        match crate::data::DepEntry::decode(raw) {
            crate::data::DepEntry::Attr(a) => {
                let a_idx = a as usize;
                if fn_def_nr != u32::MAX && a_idx < data.def(fn_def_nr).attributes().len() {
                    let av = function.var(&data.def(fn_def_nr).attributes()[a_idx].name);
                    if av != u16::MAX {
                        ret_deps.insert(av);
                    }
                }
            }
            // H2 step 5: a tagged callee-frame note IS a frame var — pool
            // it directly (the untagged value was silently dropped by the
            // attr-range guard before, so a returned closure's work var
            // could surface as a false leak report).
            crate::data::DepEntry::CalleeFrame(w) => {
                ret_deps.insert(w);
            }
        }
    }
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
        // #323: a Reference-typed capture is owned by the closure record
        // (the record stores its 12-byte DbRef; `free_named`'s cascade
        // frees it when the record dies), so `get_free_vars` emits no
        // frame-exit OpFreeRef for it — mirror that exemption here or
        // every capturing closure false-positives this assert.
        if function.is_captured(v) && matches!(function.tp(v), Type::Reference(_, _)) {
            continue;
        }
        if v == direct_ret_var {
            continue; // ownership transferred to caller
        }
        if let Type::Reference(_, dep) = function.tp(v) {
            // LOFT_REF_LEAK_WARN=1 downgrades the assert to a warning so a
            // debug build can still RUN a program with a known leak shape
            // (e.g. to chase a separate runtime corruption past compile).
            let warn_only = std::env::var("LOFT_REF_LEAK_WARN").is_ok();
            if warn_only && !(!dep.is_empty() || ret_deps.contains(&v) || freed.contains(&v)) {
                eprintln!(
                    "[check_ref_leaks] WARNING: Reference variable '{}' (var_nr={v}) in \
                     function '{fn_name}' has no OpFreeRef (scope {scope}) — store leak.",
                    function.name(v),
                );
            } else {
                assert!(
                    !dep.is_empty() || ret_deps.contains(&v) || freed.contains(&v),
                    "[check_ref_leaks] Reference variable '{}' (var_nr={v}) in function \
                     '{}' has no OpFreeRef — it is in scope {scope} but was never freed. \
                     This is likely a scope-registration bug: the variable was registered \
                     in an inner block scope that is not reachable from function-exit cleanup.",
                    function.name(v),
                    fn_name
                );
            }
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
    body_calls_par_safe(value, data, &mut |callee, data| {
        walk_par_safe(data, callee, visited)
    })
}

/// Shared body walk behind the two par-safety analyses (pass-3 dedupe):
/// every `Call` must pass the purity gate; a `CallRef` (runtime fn-ref,
/// callee unknown at compile time) rejects conservatively.  The two
/// analyses differ ONLY in how an UNKNOWN user fn is decided — `user_fn`
/// supplies that policy (the 5b walk recurses with a visited set; the
/// fixed-point pass looks up its classification table).
fn body_calls_par_safe(
    value: &Value,
    data: &Data,
    user_fn: &mut dyn FnMut(u32, &Data) -> bool,
) -> bool {
    !value.any_node(&mut |n| match n {
        Value::Call(callee, _) => !call_purity_safe(*callee, data, user_fn),
        Value::CallRef(_, _) => true,
        _ => false,
    })
}

/// The shared purity gate: Purity decides directly except for UNKNOWN
/// user fns, which `user_fn` decides.
fn call_purity_safe(callee: u32, data: &Data, user_fn: &mut dyn FnMut(u32, &Data) -> bool) -> bool {
    if callee == u32::MAX || (callee as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[callee as usize];
    match def.purity {
        Purity::Pure
        | Purity::Impure(
            ImpureCategory::HostIo
            | ImpureCategory::Prng
            | ImpureCategory::Io
            // Nested par: D8 R2 says the inner worker fn must itself be
            // par-safe.  Minimum impl accepts; full 5b looks up the worker
            // fn arg and recurses into it.
            | ImpureCategory::ParCall,
        ) => true,
        Purity::Impure(ImpureCategory::ParentWrite) => false,
        Purity::Unknown => {
            if matches!(def.code, Value::Null) {
                // Native stdlib fn with no annotation — conservative.
                false
            } else {
                user_fn(callee, data)
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
        d.definitions[stdlib as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
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
        d.definitions[bad_fn as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
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
fn walk_par_unsafe_reason(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> Option<String> {
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
        Value::CallRef(_, _args) => {
            Some("runtime fn-ref call (callee unknown at compile time)".to_string())
        }
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
        Value::Span(b) => walk_par_unsafe_reason_value(&b.1, data, visited),
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
        | Purity::Impure(
            ImpureCategory::HostIo
            | ImpureCategory::Prng
            | ImpureCategory::Io
            | ImpureCategory::ParCall,
        ) => None,
        Purity::Impure(ImpureCategory::ParentWrite) => {
            Some(format!("call to parent-write stdlib fn '{}'", def.name))
        }
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
        d.definitions[stdlib as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
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
            r.contains("recursive descent") && r.contains("inner") && r.contains("vector_add"),
            "expected chain explanation through inner→vector_add; got: {r}"
        );
    }

    #[test]
    fn first_violating_call_in_block_wins() {
        let mut d = Data::new();
        let pure_fn = d.add_def("min", &pos(), DefType::Function);
        d.definitions[pure_fn as usize].purity = Purity::Pure;
        let bad_first = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad_first as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let bad_second = d.add_def("hash_set", &pos(), DefType::Function);
        d.definitions[bad_second as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
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
fn walk_classified(value: &Value, data: &Data, classification: &HashMap<u32, bool>) -> bool {
    body_calls_par_safe(value, data, &mut |callee, _| {
        // User fn — look up the classification.  If absent
        // (user_fn_d_nrs missed it), conservative false.
        classification.get(&callee).copied().unwrap_or(false)
    })
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
        d.definitions[is_even as usize].code =
            Value::Call(is_odd, vec![Value::Call(pure_fn, vec![])]);
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

/// Plan-06 phase 5b' — DEEP parent-write check for par() workers.
/// Recurses through user-fn callees (those with a body) until it
/// finds a direct call to a `Purity::Impure(ParentWrite)` stdlib fn.
/// Returns the chain "worker → helper → bad_callee" or None.
///
/// Crucially, unannotated declared-only natives (Op*, n_*, t_* with
/// `code == Value::Null` and `purity == Unknown`) are treated as
/// safe — they're C primitives that don't write to parent state
/// unless explicitly tagged.  This is what avoids the 16 false
/// positives the strict `par_unsafe_reason` walk would produce.
///
/// `Purity::Impure(ParCall)` stdlib fns (parallel_for / _light)
/// are also safe — D8 R2 and D2.1.1 cover recursive Arc promotion.
pub fn worker_calls_parent_write_deep(data: &Data, worker_d_nr: u32) -> Option<String> {
    if worker_d_nr == u32::MAX || (worker_d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let mut visited = std::collections::HashSet::new();
    let def = &data.definitions[worker_d_nr as usize];
    let worker_name = def.name.clone();
    walk_deep_parent_write(&def.code, data, worker_d_nr, &mut visited).map(|chain| {
        if chain == worker_name {
            chain
        } else {
            format!("{worker_name}{chain}")
        }
    })
}

#[allow(dead_code)]
fn walk_deep_parent_write(
    value: &Value,
    data: &Data,
    current_fn: u32,
    visited: &mut std::collections::HashSet<u32>,
) -> Option<String> {
    match value {
        Value::Call(callee, args) => {
            if let Some(chain) = call_deep_parent_write(*callee, args, data, current_fn, visited) {
                return Some(chain);
            }
            for a in args {
                if let Some(chain) = walk_deep_parent_write(a, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        // Don't recurse into CallRef target (callee unknown until runtime).
        Value::CallRef(_, args) => {
            for a in args {
                if let Some(chain) = walk_deep_parent_write(a, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::Block(b) => {
            for v in &b.operators {
                if let Some(chain) = walk_deep_parent_write(v, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::Insert(vs) => {
            for v in vs {
                if let Some(chain) = walk_deep_parent_write(v, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::If(c, t, e) => walk_deep_parent_write(c, data, current_fn, visited)
            .or_else(|| walk_deep_parent_write(t, data, current_fn, visited))
            .or_else(|| walk_deep_parent_write(e, data, current_fn, visited)),
        Value::Loop(body) => {
            for v in &body.operators {
                if let Some(chain) = walk_deep_parent_write(v, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::Set(_, rhs) => walk_deep_parent_write(rhs, data, current_fn, visited),
        Value::Span(b) => walk_deep_parent_write(&b.1, data, current_fn, visited),
        _ => None,
    }
}

/// Plan-06 phase 5b' G5 — for a `ParentWrite` callee, treat the
/// call as safe when its first argument is a LOCAL variable
/// (defined within the calling function, not a parameter).  Avoids
/// the false positive on `out: vector<integer> = []; out += [i]`
/// where `OpAppendVector(out, ...)` mutates a worker-local
/// vector — `out` was just allocated locally and isn't shared
/// with the parent.
///
/// This is a heuristic: a local variable could still hold a
/// reference to parent data (e.g. via `let v = some_param_field`),
/// so the rule is conservative for the common pattern but may
/// miss adversarial aliases.  Future precision work could track
/// per-var initialiser provenance.
fn first_arg_is_local_var(args: &[Value], current_fn: u32, data: &Data) -> bool {
    let Some(Value::Var(v)) = args.first() else {
        return false;
    };
    if current_fn == u32::MAX || (current_fn as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[current_fn as usize];
    if !def.variables.is_argument(*v) {
        return true;
    }
    // Plan-06 phase 5b' G5 — heap-typed return values are passed
    // via a hidden destination argument promoted by ref_return().
    // The promotion sets `argument: true` on the variable AND
    // marks the corresponding `def.attributes[…].hidden = true`.
    // Workers writing to these hidden destinations are populating
    // their own per-worker output buffer, NOT parent state.
    let name = def.variables.name(*v);
    def.attributes.iter().any(|a| a.hidden && a.name == name)
}

#[allow(dead_code)]
fn call_deep_parent_write(
    callee: u32,
    args: &[Value],
    data: &Data,
    current_fn: u32,
    visited: &mut std::collections::HashSet<u32>,
) -> Option<String> {
    if callee == u32::MAX || (callee as usize) >= data.definitions.len() {
        return None;
    }
    let def = &data.definitions[callee as usize];
    match def.purity {
        Purity::Pure
        | Purity::Impure(
            ImpureCategory::HostIo
            | ImpureCategory::Prng
            | ImpureCategory::Io
            | ImpureCategory::ParCall,
        ) => None,
        Purity::Impure(ImpureCategory::ParentWrite) => {
            if first_arg_is_local_var(args, current_fn, data) {
                None
            } else {
                Some(format!(" → {}", def.name))
            }
        }
        Purity::Unknown => {
            // Declared-only native (no body) → trust as safe.
            if matches!(def.code, Value::Null) {
                None
            } else if !visited.insert(callee) {
                // Cycle — optimistic short-circuit (consistent with
                // is_par_safe / fixpoint convergence).
                None
            } else {
                walk_deep_parent_write(&def.code, data, callee, visited)
                    .map(|chain| format!(" → {}{}", def.name, chain))
            }
        }
    }
}

#[allow(dead_code)]
fn walk_shallow_parent_write(value: &Value, data: &Data) -> Option<String> {
    // "Shallow" = the runtime callee behind a `CallRef` is never followed
    // (it is not a child node); arg expressions ARE scanned.
    let mut found = None;
    value.any_node(&mut |n| {
        if let Value::Call(callee, _) = n
            && (*callee as usize) < data.definitions.len()
            && matches!(
                data.definitions[*callee as usize].purity,
                Purity::Impure(ImpureCategory::ParentWrite)
            )
        {
            found = Some(data.definitions[*callee as usize].name.clone());
            true
        } else {
            false
        }
    });
    found
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
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(bad, vec![]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }

    /// Pass-2 wave 4 regression: the hand-rolled walker had no `Return`
    /// arm (`_ => None`), so a parent-write call appearing only in
    /// `return f(...)` escaped the scan — the worker classified safe.
    /// The keystone descent sees every position.
    #[test]
    fn parent_write_in_return_value_detected() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Return(Box::new(Value::Call(bad, vec![])));
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }

    /// Same hole for a tuple element (`(f(...), 1)` result shapes).
    #[test]
    fn parent_write_in_tuple_element_detected() {
        let mut d = Data::new();
        let bad = d.add_def("hash_set", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code =
            Value::Tuple(vec![Value::Call(bad, vec![]), Value::Int(1)]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("hash_set".to_string())
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
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
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
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let safe = d.add_def("min", &pos(), DefType::Function);
        d.definitions[safe as usize].purity = Purity::Pure;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(safe, vec![Value::Call(bad, vec![])]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }
}

#[cfg(test)]
mod par_deep_tests {
    use super::worker_calls_parent_write_deep;
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
    fn deep_walks_through_user_helper() {
        // Worker calls helper; helper calls vector_add.  Deep walk
        // returns the chain.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let helper = d.add_def("helper", &pos(), DefType::Function);
        d.definitions[helper as usize].code = Value::Call(bad, vec![]);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(helper, vec![]);
        let result = worker_calls_parent_write_deep(&d, worker);
        assert!(result.is_some());
        let chain = result.unwrap();
        assert!(chain.contains("worker"));
        assert!(chain.contains("helper"));
        assert!(chain.contains("vector_add"));
    }

    #[test]
    fn deep_skips_unannotated_native() {
        // Worker calls OpAddInt (Unknown + Value::Null) → safe.
        let mut d = Data::new();
        let op = d.add_def("OpAddInt", &pos(), DefType::Function);
        // purity defaults to Unknown, code defaults to Null
        let _ = op;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(op, vec![]);
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    #[test]
    fn deep_handles_recursive_user_fn() {
        // Worker calls itself — visited set prevents infinite loop.
        let mut d = Data::new();
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(worker, vec![]);
        // No parent-write reachable, even with the cycle.
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    #[test]
    fn deep_par_call_callee_is_safe() {
        // ARC.md A4 (closed 2026-05-07) — `parallel_for_light` was
        // retired but any Impure(ParCall) callee still demonstrates
        // the same D8 R2 invariant: nested par() under recursive Arc
        // promotion is safe.  Use `parallel_queue` (a current
        // ParCall-purity callee) as the stand-in.
        let mut d = Data::new();
        let pf = d.add_def("parallel_queue", &pos(), DefType::Function);
        d.definitions[pf as usize].purity = Purity::Impure(ImpureCategory::ParCall);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(pf, vec![]);
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    #[test]
    fn deep_local_arg_to_parent_write_is_safe() {
        // Plan-06 phase 5b' G5 — calling a ParentWrite stdlib fn
        // on a worker-LOCAL variable is safe.  worker calls
        // OpAppendVector(local_v, ...); local_v isn't an arg.
        // Var(0) defaults to argument=false (local).
        let mut d = Data::new();
        let bad = d.add_def("OpAppendVector", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(bad, vec![Value::Var(0)]);
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    // Note: the hidden-return-arg exception (`def.attributes[…].hidden`
    // case) is exercised end-to-end by par_struct_to_vector_t4 in
    // tests/threading_chars.rs.  A unit test would need to construct a
    // full Function with a promoted hidden attribute, which the
    // parser does multi-step; the integration test is the cleaner
    // verification.
}

// ── Plan-57 cluster I: store-lifetime guard (diagnostic) ─────────────────────
//
// Fires (under LOFT_STORE_GUARD) when a vector local's references are confined
// to one non-loop nested block, yet its backing `__vdb_N` store is scoped to an
// ancestor (function) and so frees late — the lifetime model under-freeing a
// block-confined store.  A detector for the watermark; once the model scopes
// such stores to their block it goes silent.  Read-only, gated, no behaviour
// change.
//
// Confinement = the least-common-ancestor of the block/loop scope-paths of every
// reference.  Tracking the full path (not just the innermost block) is required:
// a vector created in block B and read in a nested sub-block (nested `if`, a
// for-loop's `#For` block, or inside a loop body) is still confined to B — the
// LCA of `[B]` and `[B, sub]` is `B`.  Exact-scope-match misses these (probes
// 20/25/26).  The LCA's last element being a LOOP scope means the local lives
// only inside that loop (per-iteration reuse) → not relocatable.

/// Record a reference at the current scope-path: fold it into the running LCA
/// (longest common prefix of all reference paths).
fn guard_note(stack: &[(u16, bool)], lca: &mut Option<Vec<(u16, bool)>>) {
    *lca = Some(match lca.take() {
        None => stack.to_vec(),
        Some(prev) => prev
            .iter()
            .zip(stack)
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| *a)
            .collect(),
    });
}

fn guard_refs(
    node: &Value,
    target: u16,
    free_ref_nr: u32,
    stack: &mut Vec<(u16, bool)>,
    lca: &mut Option<Vec<(u16, bool)>>,
) {
    match node {
        Value::Var(v) if *v == target => guard_note(stack, lca),
        Value::Set(v, val) => {
            if *v == target && !matches!(val.unspan(), Value::Null) {
                guard_note(stack, lca);
            }
            guard_refs(val, target, free_ref_nr, stack, lca);
        }
        Value::Call(op, args) => {
            if *op == free_ref_nr
                && args.len() == 1
                && matches!(args[0].unspan(), Value::Var(v) if *v == target)
            {
                return;
            }
            for a in args {
                guard_refs(a, target, free_ref_nr, stack, lca);
            }
        }
        Value::Block(bl) => {
            stack.push((bl.scope, false));
            for op in &bl.operators {
                guard_refs(op, target, free_ref_nr, stack, lca);
            }
            stack.pop();
        }
        Value::Loop(lp) => {
            stack.push((lp.scope, true));
            for op in &lp.operators {
                guard_refs(op, target, free_ref_nr, stack, lca);
            }
            stack.pop();
        }
        Value::If(t, a, b) => {
            guard_refs(t, target, free_ref_nr, stack, lca);
            guard_refs(a, target, free_ref_nr, stack, lca);
            guard_refs(b, target, free_ref_nr, stack, lca);
        }
        Value::Iter(idx, c, n, e) => {
            if *idx == target {
                guard_note(stack, lca);
            }
            guard_refs(c, target, free_ref_nr, stack, lca);
            guard_refs(n, target, free_ref_nr, stack, lca);
            guard_refs(e, target, free_ref_nr, stack, lca);
        }
        Value::CallRef(v, args) => {
            if *v == target {
                guard_note(stack, lca);
            }
            for a in args {
                guard_refs(a, target, free_ref_nr, stack, lca);
            }
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) | Value::BreakWith(_, v) => {
            guard_refs(v, target, free_ref_nr, stack, lca)
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for op in ops {
                guard_refs(op, target, free_ref_nr, stack, lca);
            }
        }
        Value::TupleGet(v, _) if *v == target => guard_note(stack, lca),
        Value::TuplePut(v, _, inner) => {
            if *v == target {
                guard_note(stack, lca);
            }
            guard_refs(inner, target, free_ref_nr, stack, lca);
        }
        Value::Span(b) => guard_refs(&b.1, target, free_ref_nr, stack, lca),
        Value::ParFor(b) => {
            guard_refs(&b.input, target, free_ref_nr, stack, lca);
            guard_refs(&b.worker, target, free_ref_nr, stack, lca);
            guard_refs(&b.threads, target, free_ref_nr, stack, lca);
            guard_refs(&b.body, target, free_ref_nr, stack, lca);
        }
        _ => {}
    }
}

/// True if `target` is handed out of the function as-is — directly returned,
/// yielded, or broken out of a loop (`Return(Var(target))` etc.).  Such a local
/// escapes; its store must NOT be freed at block exit (probe 30).  (Catches the
/// direct form; an escape buried in a sub-expression like `return if c { a }`
/// is left for the fix's full escape analysis.)
/// True if `v` hands `target` out as-is: directly (`a`), or as a direct element
/// of a tuple / vector-literal value (`(a, n)`, `[a, b]`).  A *derived* value
/// (`a[0]`, `a.len()`) does NOT count — it produces a fresh value, not a's store.
fn escapes_value(v: &Value, target: u16) -> bool {
    match v.unspan() {
        Value::Var(t) => *t == target,
        Value::Tuple(elems) | Value::Insert(elems) => {
            elems.iter().any(|e| escapes_value(e, target))
        }
        _ => false,
    }
}

fn guard_escapes(node: &Value, target: u16) -> bool {
    node.any_node(&mut |n| match n {
        Value::Return(v) | Value::Yield(v) | Value::BreakWith(_, v) => escapes_value(v, target),
        // The block's VALUE is its last operator; if that hands out the local
        // (directly or in a tuple/literal), it escapes (block-result `x = {
        // …; a }` U3; `return (a, n)` t2).
        Value::Block(bl) | Value::Loop(bl) => bl
            .operators
            .last()
            .is_some_and(|o| escapes_value(o, target)),
        _ => false,
    })
}

/// Soundness gate for confining a *multi-store* local — one reassigned a fresh
/// store per block (the shared-`z`-across-`else`-blocks / shared-`x`-across-
/// match-arms shape).  Returns true iff every READ of `local` is dominated by a
/// non-null assignment of `local` earlier in the same straight-line block, so
/// the local never carries a store across a block boundary unreassigned.  When
/// that holds, freeing each store at *its* block exit cannot be a use-after-free
/// (the local's next read sees a freshly-assigned store, never the freed one).
/// Conditional assignments (inside `if`/`loop`) do NOT establish dominance for
/// code after the construct — the walk under-claims, so it stays sound.
fn confine_reassign_safe(code: &Value, local: u16) -> bool {
    let mut ok = true;
    dominance_walk(code, local, false, &mut ok, false);
    ok
}

/// Shared dominance walk behind the two Plan-57 soundness gates
/// (pass-3 dedupe: the two walkers were 80 identical lines apart, and the
/// stronger one had silently lost the `ParFor` arm).  `dom` = "every read
/// of `local` here is preceded by a non-null assignment that definitely
/// executed".  The two gates differ ONLY in:
/// - the START value (`confine_reassign_safe` starts false — a definite
///   reassignment must precede any read; `store_dead_after_block` starts
///   true — the fn-level init counts);
/// - `invalidate_conditional` (`store_dead_after_block` only): a
///   reassignment inside an `If`/`Loop`/`Iter`/`ParFor` body INVALIDATES
///   dominance — afterwards `local` may hold that block's confined store.
///
/// A read of `local` while `!dom` clears `ok`.
fn dominance_walk(node: &Value, local: u16, dom: bool, ok: &mut bool, inv: bool) -> bool {
    match node {
        Value::Set(v, val) => {
            // RHS evaluated before the write lands; a read of `local` in it
            // is gated by the *current* dominance.
            dominance_walk(val, local, dom, ok, inv);
            if *v == local {
                return !matches!(val.unspan(), Value::Null);
            }
            dom
        }
        Value::Var(v) | Value::TupleGet(v, _) => {
            if *v == local && !dom {
                *ok = false;
            }
            dom
        }
        Value::CallRef(v, args) => {
            if *v == local && !dom {
                *ok = false;
            }
            let mut d = dom;
            for a in args {
                d = dominance_walk(a, local, d, ok, inv);
            }
            dom
        }
        Value::Block(bl) => {
            let mut d = dom;
            for op in &bl.operators {
                d = dominance_walk(op, local, d, ok, inv);
            }
            d
        }
        Value::Loop(lp) => {
            // Body runs 0+ times → conditional; dominance does not leak out.
            let mut d = dom;
            for op in &lp.operators {
                d = dominance_walk(op, local, d, ok, inv);
            }
            if inv && assigns_local(node, local) {
                false
            } else {
                dom
            }
        }
        Value::If(t, a, b) => {
            let dc = dominance_walk(t, local, dom, ok, inv);
            dominance_walk(a, local, dc, ok, inv);
            dominance_walk(b, local, dc, ok, inv);
            // Branch assignments are conditional — they establish no
            // post-`if` dominance; under `inv` they additionally invalidate.
            if inv && (assigns_local(a, local) || assigns_local(b, local)) {
                false
            } else {
                dc
            }
        }
        Value::Iter(idx, c, n, e) => {
            if *idx == local && !dom {
                *ok = false;
            }
            let mut d = dominance_walk(c, local, dom, ok, inv); // the iteration SOURCE reads `local`
            d = dominance_walk(n, local, d, ok, inv);
            dominance_walk(e, local, d, ok, inv); // body conditional
            if inv && assigns_local(node, local) {
                false
            } else {
                d
            }
        }
        Value::Call(_, args) | Value::Insert(args) | Value::Tuple(args) | Value::Parallel(args) => {
            let mut d = dom;
            for a in args {
                d = dominance_walk(a, local, d, ok, inv);
            }
            d
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) | Value::BreakWith(_, v) => {
            dominance_walk(v, local, dom, ok, inv);
            dom
        }
        Value::TuplePut(v, _, inner) => {
            if *v == local && !dom {
                *ok = false;
            }
            dominance_walk(inner, local, dom, ok, inv);
            dom
        }
        Value::Span(b) => dominance_walk(&b.1, local, dom, ok, inv),
        Value::ParFor(b) => {
            let mut d = dominance_walk(&b.input, local, dom, ok, inv);
            d = dominance_walk(&b.worker, local, d, ok, inv);
            d = dominance_walk(&b.threads, local, d, ok, inv);
            dominance_walk(&b.body, local, d, ok, inv);
            // The parallel body is conditional from the caller's view.
            if inv && assigns_local(node, local) {
                false
            } else {
                d
            }
        }
        _ => dom,
    }
}

/// Plan-57 cluster-III Route 2: recover the backer of an *orphaned* store — one
/// the single-valued `dep` no longer records because its holding local was
/// reassigned.  Returns the local `L` the store flows into via its repoint
/// `Set(L, OpGetField(Var(vdb), …))` (the canonical `z = [..]` lowering).
fn recover_backer(code: &Value, vdb: u16, gf_nr: u32) -> Option<u16> {
    let mut backer = None;
    code.any_node(&mut |n| {
        if let Value::Set(l, val) = n
            && let Value::Call(op, args) = val.unspan()
            && *op == gf_nr
            && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == vdb)
        {
            backer = Some(*l);
            true
        } else {
            false
        }
    });
    backer
}

/// Does `node` contain a non-null reassignment of `local` anywhere?
fn assigns_local(node: &Value, local: u16) -> bool {
    node.any_node(
        &mut |n| matches!(n, Value::Set(v, val) if *v == local && !matches!(val.unspan(), Value::Null)),
    )
}

/// Plan-57 cluster-III Route 2 soundness gate (STRONGER than `confine_reassign_safe`,
/// which only proves the backer is *defined* at every read — the fn-level init
/// satisfies that even when a confined block store is still live, an empirically
/// confirmed UAF via `for x in v` after the block).
///
/// Dominance walk over the body: `dom` = "`local` is known NOT to hold a confined
/// block store here" (it holds the fn-level init or an unconditional reassignment).
/// `dom` starts true and is **invalidated** by any CONDITIONAL reassignment
/// (inside an `If`/`Loop`/`Iter`) — afterwards `local` might hold that block's store,
/// which the fix would free at block exit.  A read of `local` while `!dom` is an
/// over-free hazard → unsound to confine.  Mirrors `confine_reassign_safe` but with
/// the conditional-reassignment invalidation (the missing soundness property).
fn store_dead_after_block(code: &Value, local: u16) -> bool {
    let mut ok = true;
    dominance_walk(code, local, true, &mut ok, true);
    ok
}

/// Per `__vdb` store, the LCA non-loop block scope it is provably confined to —
/// i.e. the scope at which it could be freed instead of at function exit.
/// Returns `vdb -> (backed local, block scope)` for every store-backed local
/// confined to a non-loop block deeper than where its store is currently
/// registered.  Two consumers share this one analysis:
/// - the cluster-I fix — re-register the confined `__vdb` (+ its local) at the
///   block scope so the standard block-exit `free_vars` sweep frees it there;
/// - the `LOFT_STORE_GUARD` detector ([`store_lifetime_guard`]) — a thin
///   wrapper that reports each entry.
///
/// Soundness (adversarially hardened across the probe rounds): excludes escapes
/// (return/yield/break, block-result, tuple/vector element via `guard_escapes`),
/// loop-internal confinement (per-iteration reuse, not a watermark), and any
/// store aliased by a variable that outlives block `b`.
fn store_confinement(
    code: &Value,
    vars: &Function,
    free_ref_nr: u32,
    gf_nr: u32,
) -> HashMap<u16, (u16, u16)> {
    // Plan-57 cluster-III Route 2 (gated, experimental): recover the backer of an
    // orphaned (overwritten) store so its per-block store can confine.
    let recover = std::env::var("LOFT_CONF_RECOVER").is_ok();
    let mut out: HashMap<u16, (u16, u16)> = HashMap::new();
    for vdb in 0..vars.count() {
        if !vars.name(vdb).starts_with("__vdb") {
            continue;
        }
        // The single local that holds `vdb` (vdb in its dep), non-arg,
        // non-captured.  A *single-store* local (dep == [vdb]) shares its store's
        // span; a *multi-store* local (dep ⊇ vdb, reassigned a fresh store per
        // block — shared `z` across `else`-blocks, shared `x` across match arms)
        // spans the whole function even though each store lives in one block.
        let mut backed: Option<u16> = None;
        let mut ambiguous = false;
        for v in 0..vars.count() {
            if vars.tp(v).depend().contains(&vdb) {
                if vars.is_argument(v) || vars.is_captured(v) || backed.is_some() {
                    ambiguous = true;
                    break;
                }
                backed = Some(v);
            }
        }
        if ambiguous {
            continue;
        }
        // Route 2: an orphaned store has no dep-backer (single-valued dep dropped
        // the link when the local was reassigned).  Recover the local it flows into
        // via its `OpGetField` repoint; the recovered backer is necessarily
        // multi-store.  Gated off by default until soundness is locked in.
        let recovered = backed.is_none();
        let local = if let Some(l) = backed {
            l
        } else if recover {
            match recover_backer(code, vdb, gf_nr) {
                Some(l) if !vars.is_argument(l) && !vars.is_captured(l) => l,
                _ => continue,
            }
        } else {
            continue;
        };
        // An escaping local hands its store to the caller — freeing it at block
        // exit is a use-after-free.  Exclude direct returns/yields/breaks
        // (probe 30) and anything scope-analysis already marked skip-free.
        if vars.is_skip_free(local) || vars.is_skip_free(vdb) || guard_escapes(code, local) {
            continue;
        }
        let multi_store = recovered || vars.tp(local).depend().len() != 1;
        // A multi-store local must never carry `vdb`'s store across a block
        // boundary unreassigned, else block-exit freeing is a UAF.  Gate on the
        // write-dominates-read walk before trusting the per-store span.
        if multi_store && !confine_reassign_safe(code, local) {
            continue;
        }
        // Confinement block = the LCA non-loop block of the *store's own* refs
        // (multi-store) or the local's (single-store — equal to the store's, kept
        // for the U3 alias path below).
        let span_target = if multi_store { vdb } else { local };
        let mut stack: Vec<(u16, bool)> = Vec::new();
        let mut lca: Option<Vec<(u16, bool)>> = None;
        guard_refs(code, span_target, free_ref_nr, &mut stack, &mut lca);
        // Confined iff the LCA path is a non-empty chain of NON-LOOP blocks
        // (NO loop anywhere in it — a confinement *inside* a loop is per-
        // iteration reuse, not a watermark; probes 33/34), and the innermost
        // block is deeper than where the store is currently scoped.
        if let Some(path) = lca
            && let Some(&(b, _)) = path.last()
            && path.iter().all(|&(_, is_loop)| !is_loop)
            && vars.scope(vdb) != b
            // Route 2 soundness: the recovered backer's block store must be dead
            // after its block on EVERY path — `confine_reassign_safe` only proves
            // the backer is *defined* at reads (the fn-level init satisfies that),
            // so an extra "no body-scope read of the backer" gate is required.
            && (!recovered || store_dead_after_block(code, local))
            // dep-escape: the store must not be aliased by a USER variable that
            // OUTLIVES block `b`.  A block-result `x = { …; a }` gives x the dep
            // `["a"]` and x is read at function level (U3) — freeing a here would
            // corrupt x.  Compiler temps (`_elm`, `__vdb`) are confined to their
            // own block and, for a multi-store local, may legitimately alias the
            // local in a *sibling* block (holding a different store there) — so
            // skip them rather than false-positive.
            && !(0..vars.count()).any(|w| {
                w != local
                    && !vars.name(w).starts_with('_')
                    && vars.tp(w).depend().contains(&local)
                    && {
                        let mut wst: Vec<(u16, bool)> = Vec::new();
                        let mut wlca: Option<Vec<(u16, bool)>> = None;
                        guard_refs(code, w, free_ref_nr, &mut wst, &mut wlca);
                        // w aliases the store AND is referenced outside `b`
                        // (b absent from w's confinement path) ⇒ outlives it.
                        wlca.is_some_and(|p| !p.iter().any(|&(s, _)| s == b))
                    }
            })
        {
            out.insert(vdb, (local, b));
        }
    }
    out
}

/// `LOFT_STORE_GUARD` detector — reports each store-backed local that frees at
/// function exit despite being confined to an inner block.  Thin wrapper over
/// [`store_confinement`] (the same analysis that drives the cluster-I fix).
/// Returns the number of late-freed stores.
fn store_lifetime_guard(
    code: &Value,
    vars: &Function,
    free_ref_nr: u32,
    gf_nr: u32,
    fn_name: &str,
) -> usize {
    let confined = store_confinement(code, vars, free_ref_nr, gf_nr);
    for (&vdb, &(local, b)) in &confined {
        eprintln!(
            "[store-guard] {fn_name}: store {} (local '{}') confined to block scope {b} but stored at scope {} — frees late",
            vars.name(vdb),
            vars.name(local),
            vars.scope(vdb),
        );
    }
    confined.len()
}

/// Which store a `Set`'s RHS binds the assigned local to (so the local now "holds"
/// that store's data): `OpGetField(Var(s), …)` → `s` (the repoint idiom); a copy
/// `Var(other)` → whatever `other` currently holds; anything else → unbound.
fn binding_source(val: &Value, gf_nr: u32, holds: &HashMap<u16, u16>) -> Option<u16> {
    match val.unspan() {
        Value::Call(op, args) if *op == gf_nr => match args.first().map(Value::unspan) {
            Some(Value::Var(s)) => Some(*s),
            _ => None,
        },
        Value::Var(other) => holds.get(other).copied(),
        _ => None,
    }
}

/// Flow walk: trace which store each local **holds** so each store's *data*
/// liveness is recovered — `alloc` (its `OpDatabase`), `last_read` (last read of
/// its data via a holding local or a direct build op, EXCLUDING the scope-exit
/// `OpFreeRef`), and `dead` (the point a holding local is rebound away, the
/// reassignment case).  This is the real liveness `compute_intervals` cannot give
/// (its `last_use` is pinned to the teardown `OpFreeRef`).  Sequential approximation
/// across branches — fine for the straight-line / sequential shapes this targets.
#[allow(clippy::too_many_arguments)]
fn store_liveness_walk(
    node: &Value,
    seq: &mut u32,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
    holds: &mut HashMap<u16, u16>,
    alloc: &mut HashMap<u16, u32>,
    dead: &mut HashMap<u16, u32>,
    last_read: &mut HashMap<u16, u32>,
) {
    *seq += 1;
    let s = *seq;
    match node {
        Value::Var(v) => {
            if let Some(&store) = holds.get(v) {
                last_read.insert(store, s);
            }
        }
        Value::Set(local, val) => {
            if matches!(val.unspan(), Value::Null) {
                return; // null-init — not a real def/rebind
            }
            store_liveness_walk(val, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            let new_store = binding_source(val, gf_nr, holds);
            if let Some(&old) = holds.get(local)
                && Some(old) != new_store
            {
                dead.entry(old).or_insert(s); // local rebound away → old store dead here
            }
            match new_store {
                Some(st) => {
                    holds.insert(*local, st);
                }
                None => {
                    holds.remove(local);
                }
            }
        }
        Value::Call(op, args) => {
            if *op == fr_nr && args.len() == 1 && matches!(args[0].unspan(), Value::Var(_)) {
                return; // OpFreeRef — not a data read
            }
            if *op == db_nr {
                if let Some(Value::Var(st)) = args.first().map(Value::unspan) {
                    alloc.insert(*st, s);
                }
                return; // OpDatabase(store, size) — alloc point, args are not data reads
            }
            for a in args {
                store_liveness_walk(a, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                store_liveness_walk(op, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            }
        }
        Value::If(c, t, e) => {
            store_liveness_walk(c, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            store_liveness_walk(t, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            store_liveness_walk(e, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for op in ops {
                store_liveness_walk(op, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            }
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) | Value::BreakWith(_, v) => {
            store_liveness_walk(v, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
        }
        Value::Span(b) => {
            store_liveness_walk(
                &b.1, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read,
            );
        }
        _ => {}
    }
}

/// True if `node` contains the `OpDatabase(Var(store), …)` allocation of `store`.
fn contains_alloc(node: &Value, store: u16, db_nr: u32) -> bool {
    node.any_node(&mut |n| {
        matches!(n, Value::Call(op, args) if *op == db_nr
        && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == store))
    })
}

/// Plan-57 Phase-3 soundness gate (dominance): true only if `store`'s `OpDatabase`
/// allocation is reached **unconditionally** from the body — never gated by an
/// `If`/`Loop`/`Parallel` branch.  A reclaim that early-frees / drops the scope-exit
/// free of a store allocated in an untaken branch leaks or double-frees
/// (`20-binary`).  Plain nested blocks always run, so they stay unconditional.
fn contains_alloc_unconditional(node: &Value, store: u16, db_nr: u32) -> bool {
    match node.unspan() {
        Value::Call(op, args) => {
            (*op == db_nr
                && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == store))
                || args
                    .iter()
                    .any(|a| contains_alloc_unconditional(a, store, db_nr))
        }
        Value::Set(_, val) => contains_alloc_unconditional(val, store, db_nr),
        Value::Block(bl) => bl
            .operators
            .iter()
            .any(|o| contains_alloc_unconditional(o, store, db_nr)),
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            contains_alloc_unconditional(v, store, db_nr)
        }
        Value::Insert(ops) | Value::Tuple(ops) => ops
            .iter()
            .any(|o| contains_alloc_unconditional(o, store, db_nr)),
        Value::Span(b) => contains_alloc_unconditional(&b.1, store, db_nr),
        // If / Loop / Parallel / Iter / ParFor — conditional, so an alloc inside is
        // NOT unconditional.
        _ => false,
    }
}

/// Plan-57 Phase-3 soundness gate (retention): true if a `holder` var (the store or
/// any local that holds it) appears in a value position that could keep the store's
/// data reachable **past the early-free point** — embedded in a tuple/vector literal,
/// stored as a struct-field / keyed value, returned/yielded, copied to an alias, or
/// passed as a non-receiver argument.  A holder as the FIRST argument of a call (the
/// receiver of an index/len/field read, or an in-place append target) does not
/// retain — that is exactly the straight-line shape this pass targets.  Everything
/// else is treated as retention (conservative — errs toward leaving the store alone).
fn holder_retained(node: &Value, holders: &HashSet<u16>) -> bool {
    match node {
        Value::Var(h) => holders.contains(h),
        Value::Call(_, args) | Value::CallRef(_, args) => args.iter().enumerate().any(|(i, a)| {
            // The receiver (first arg) being a bare holder Var is a read, not a
            // retention; any nested expression there is still scanned.
            if i == 0 && matches!(a.unspan(), Value::Var(h) if holders.contains(h)) {
                false
            } else {
                holder_retained(a, holders)
            }
        }),
        Value::Set(_, val) => holder_retained(val, holders),
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.iter().any(|o| holder_retained(o, holders))
        }
        Value::If(c, t, e) => {
            holder_retained(c, holders)
                || holder_retained(t, holders)
                || holder_retained(e, holders)
        }
        Value::Insert(xs) | Value::Tuple(xs) | Value::Parallel(xs) => {
            xs.iter().any(|x| holder_retained(x, holders))
        }
        Value::Return(v) | Value::Yield(v) | Value::Drop(v) | Value::BreakWith(_, v) => {
            holder_retained(v, holders)
        }
        Value::TuplePut(_, _, v) => holder_retained(v, holders),
        Value::Iter(_, c, n, e) => {
            holder_retained(c, holders)
                || holder_retained(n, holders)
                || holder_retained(e, holders)
        }
        Value::Span(b) => holder_retained(&b.1, holders),
        Value::ParFor(b) => {
            holder_retained(&b.input, holders)
                || holder_retained(&b.worker, holders)
                || holder_retained(&b.threads, holders)
                || holder_retained(&b.body, holders)
        }
        _ => false,
    }
}

/// #426B — every var that transitively reaches store `st` through the dep graph.
///
/// `reclaim_safe`'s holder model needs the full closure, not the one-hop depers:
/// a binding can borrow a store INDIRECTLY through an intermediate local — a
/// fn-return-of-index `b = idx0(ww){ w[0] }` binds `b` with dep `["ww"]`, and
/// `ww` deps `["__vdb_1"]`, so `b` holds `__vdb_1` via `ww`.  Missing `b` here
/// lets reclaim free `__vdb_1` before `b`'s last read (store-reuse-after-free).
///
/// Walks to a fixpoint: a var is a deper of `st` if its dep list contains `st`
/// or contains any var already known to be a deper.  Marker deps (`u16::MAX`
/// one-buffer sentinel, the `0x8000` callee-frame tag) name no frame var and are
/// skipped.  Bounded by `vars.count()` iterations (each pass adds at least one
/// var or stops), so it always terminates.
fn transitive_depers(vars: &Function, st: u16) -> Vec<u16> {
    let n = vars.count();
    let mut reaches: HashSet<u16> = HashSet::new();
    loop {
        let mut added = false;
        for v in 0..n {
            if reaches.contains(&v) {
                continue;
            }
            let deps_st = vars
                .tp(v)
                .depend()
                .into_iter()
                .any(|d| d != u16::MAX && d & 0x8000 == 0 && (d == st || reaches.contains(&d)));
            if deps_st {
                reaches.insert(v);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let mut out: Vec<u16> = reaches.into_iter().collect();
    out.sort_unstable();
    out
}

/// Plan-57 Phase-3 soundness gate: is store `st` (a `__vdb` work-ref) safe to
/// early-free + relocate + strip-its-scope-exit-free?  Mirrors `store_confinement`'s
/// per-store predicates (the I-a soundness model) for the function-scoped case:
///
/// - not `skip_free` / `captured` / a `RefVar` alias, and not directly escaping
///   (`guard_escapes`);
/// - every var that *holds* it (`st ∈ depend`) is a non-arg, non-captured,
///   non-`skip_free`, non-`RefVar`, non-escaping local; at most one is a *user*
///   local; a multi-store user local must pass `confine_reassign_safe`;
/// - none of the holders is *retained* anywhere (`holder_retained`).
///
/// Orphaned stores (no holder — the reassignment case, `__vdb_1..10` in probe 14)
/// are safe-because-dead: nothing reaches them after the rebind.  Conservative by
/// design — an escape/alias/capture/struct-field case falls back to the (sound)
/// scope-exit free.
fn reclaim_safe(code: &Value, vars: &Function, st: u16) -> bool {
    if !vars.name(st).starts_with("__vdb") {
        return false;
    }
    if vars.is_skip_free(st) || vars.is_captured(st) || matches!(vars.tp(st), Type::RefVar(_)) {
        return false;
    }
    if guard_escapes(code, st) {
        return false;
    }
    // #426B — the holder set must be the TRANSITIVE dep closure, not just the
    // one-hop depers.  A view-of-a-view keeps `st` live through an intermediate
    // local: `b = idx0(ww)` where `idx0` returns `w[0]` binds `b` with dep
    // `["ww"]`, and `ww` deps `["__vdb_1"]` — so `b` transitively holds
    // `__vdb_1` and extends its lifetime to `b`'s last use.  A single-hop scan
    // sees only `ww` (the receiver arg of the call, treated as a read, not a
    // retention), misses `b`, and reclaim frees `__vdb_1` right after the call —
    // before `b` reads it.  The freed slot is then recycled into the next
    // allocation, corrupting `b` (store-reuse-after-free).  Walking the dep
    // graph to fixpoint makes `b` a holder, so the retention scan / multi-user-
    // local gate below leave `st`'s sound scope-exit free in place.
    let depers: Vec<u16> = transitive_depers(vars, st);
    let mut user_locals = 0;
    for &v in &depers {
        if vars.is_argument(v)
            || vars.is_captured(v)
            || vars.is_skip_free(v)
            || matches!(vars.tp(v), Type::RefVar(_))
            || guard_escapes(code, v)
        {
            return false;
        }
        if !vars.name(v).starts_with('_') {
            user_locals += 1;
            if vars.tp(v).depend().len() != 1 && !confine_reassign_safe(code, v) {
                return false;
            }
        }
    }
    if user_locals > 1 {
        return false; // multiple live aliases — leave the store alone
    }
    // Holders for the retention scan: the raw store plus its NON-`_` user-var
    // holders.  `_`-prefixed compiler temps (`_elm`, nested `__vdb`) are build-
    // internal — confined to the store they construct, not external aliases — so
    // they are skipped, mirroring `store_confinement`'s dep-escape treatment.
    // Without this, a comprehension's per-iteration element record (`_elm`, which
    // deps the result `__vdb`) false-positives as retention.
    let mut holders: HashSet<u16> = depers
        .into_iter()
        .filter(|&v| !vars.name(v).starts_with('_'))
        .collect();
    holders.insert(st);
    !holder_retained(code, &holders)
}

/// Plan-57 — the reclaim PLAN, the single source of truth shared by
/// [`lastuse_reclaim`] (which acts on it) and [`reclaim_unfreed_eligible`] (the
/// Phase-4 guard, which verifies the frees landed), so the two cannot drift.
/// Returns `(owning, intent)`:
/// - `owning` — function-scoped owning stores passing the dominance + soundness
///   gates (the ones whose null-init may relocate);
/// - `intent` — `(store, trigger)` pairs: `store`'s data dies before the eligible
///   sibling `trigger` allocates, so `store` must be freed before `trigger`'s build.
fn reclaim_free_intent(
    code: &Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
) -> (Vec<u16>, Vec<(u16, u16)>) {
    let body_scope = match code.unspan() {
        Value::Block(bl) => bl.scope,
        _ => return (Vec::new(), Vec::new()),
    };
    let mut holds: HashMap<u16, u16> = HashMap::new();
    let mut alloc: HashMap<u16, u32> = HashMap::new();
    let mut dead: HashMap<u16, u32> = HashMap::new();
    let mut last_read: HashMap<u16, u32> = HashMap::new();
    let mut seq = 0u32;
    store_liveness_walk(
        code,
        &mut seq,
        db_nr,
        gf_nr,
        fr_nr,
        &mut holds,
        &mut alloc,
        &mut dead,
        &mut last_read,
    );
    let dead_at = |st: u16| {
        dead.get(&st)
            .copied()
            .or_else(|| last_read.get(&st).copied())
    };
    let mut owning: Vec<u16> = alloc
        .keys()
        .copied()
        .filter(|&st| {
            vars.scope(st) == body_scope
                && contains_alloc_unconditional(code, st, db_nr)
                && reclaim_safe(code, vars, st)
        })
        .collect();
    owning.sort_unstable();
    let mut intent: Vec<(u16, u16)> = Vec::new();
    for &st in &owning {
        let Some(d) = dead_at(st) else { continue };
        if let Some(&later) = owning
            .iter()
            .filter(|&&w| w != st && alloc.get(&w).is_some_and(|&a| a > d))
            .min_by_key(|&&w| alloc[&w])
        {
            intent.push((st, later));
        }
    }
    (owning, intent)
}

/// True if `op` is a top-level `OpFreeRef(Var(st))`.
fn is_top_free(op: &Value, st: u16, fr_nr: u32) -> bool {
    matches!(op.unspan(), Value::Call(o, args) if *o == fr_nr
        && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == st))
}

/// Plan-57 Phase-4 guard (Goal-E enforcement): after [`lastuse_reclaim`] has run,
/// every store in the reclaim plan's `intent` must have its `OpFreeRef` placed at
/// body top-level BEFORE the op that allocates its `trigger`.  Returns the count of
/// reclaim-eligible stores left live-but-dead past a later alloc — must be 0.  A
/// non-zero result means reclaim silently failed to stop a store the model says is
/// dead (a regression the watermark rule must not re-acquire).  Escape/alias cases
/// are not in `intent` (the soundness gate excluded them) — they legitimately keep
/// their scope-exit free and are not asserted on.
fn reclaim_unfreed_eligible(
    code: &Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
) -> usize {
    let (_owning, intent) = reclaim_free_intent(code, vars, db_nr, gf_nr, fr_nr);
    let Value::Block(bl) = code.unspan() else {
        return 0;
    };
    let mut count = 0;
    for &(st, later) in &intent {
        let free_idx = bl.operators.iter().position(|o| is_top_free(o, st, fr_nr));
        let alloc_idx = bl
            .operators
            .iter()
            .position(|o| contains_alloc(o, later, db_nr));
        match (free_idx, alloc_idx) {
            (Some(f), Some(a)) if f < a => {} // freed before the trigger allocates — good
            _ => count += 1,
        }
    }
    count
}

/// Plan-57 last-use freeing, Phase 3 — reclaim via null-init RELOCATION + early
/// free (gated `LASTUSE_RECLAIM`).
///
/// Phase 2 proved free-alone is inert: every `__vdb`'s null-init (`Set(vdb, Null)`,
/// hoisted to body-0 by `parse_code`) ALLOCATES its store up front, so the runtime
/// watermark is locked before any inserted free can run (probe 14: peak 11 = the 11
/// null-inits stacking at body-0).  This pass closes that with two coordinated edits
/// per dead store:
///
/// 1. **Relocate the null-init** out of body-0 to immediately before its own
///    `OpDatabase` build — so the stores stop batching at body-0 and allocate
///    interleaved.  (The I-a `relocate_null_init` lever, applied to a body *index*
///    instead of a sub-block.)
/// 2. **Early free** before the next store allocates — so a freed slot is reused by
///    the following null-init (`+alloc, -free, +alloc, -free`) instead of stacking.
///
/// The scope-exit `OpFreeRef` is left in place as an idempotent double-free
/// (`free_named` no-ops an already-free store — measured safe in Phase 2).  Both
/// edits run **before** `compute_intervals`, so the moved `first_def` is reflected
/// in the slot intervals.  Flat-body straight-line / sequential shapes only — the
/// I-b / III-straight-line cases block-confinement (I-a) cannot reach.  Returns the
/// count of relocations + frees applied.
fn lastuse_reclaim(code: &mut Value, vars: &Function, db_nr: u32, gf_nr: u32, fr_nr: u32) -> usize {
    // Eligibility + free-intent come from the shared plan, so the Phase-4 guard
    // (`reclaim_unfreed_eligible`) verifies exactly what this pass acts on.
    let (owning, intent) = reclaim_free_intent(code, vars, db_nr, gf_nr, fr_nr);
    if owning.is_empty() {
        return 0;
    }
    let Value::Block(bl) = code else { return 0 };
    // #260 Fix B replaced Fix A here: native codegen now declares every
    // `__vdb` local up front (sentinel-bound prologue, `generation/mod.rs::
    // output_function`), so relocating a null-init below an early-return
    // scope-exit free can no longer strand the free's `var_…` reference out
    // of scope (rustc E0425) — the `has_free_before_alloc` exclusion guard
    // is gone and those stores get their watermark reclaim back (46/46
    // owning stores were forfeited in brick-buster's generator pre-B).
    // Early-free groups: before[trigger] = dead stores to free right before
    // `trigger` allocates.  Their scope-exit `OpFreeRef` is REMOVED, not kept as an
    // "idempotent double-free": under reclaim the freed slot is reused by a later
    // store, so a stale scope-exit free of `st` would target a *different live
    // owner's* store (the tag gate catches exactly this).  The early free becomes
    // the store's sole free.
    let mut before: HashMap<u16, Vec<u16>> = HashMap::new();
    for &(st, later) in &intent {
        before.entry(later).or_default().push(st);
    }
    let freed_set: HashSet<u16> = intent.iter().map(|&(st, _)| st).collect();
    // Reloc set: owning stores whose null-init sits at body top-level AND whose
    // OpDatabase build is a top-level body op (so the null-init can be placed right
    // before it).  Excludes any store I-a already relocated into a sub-block.
    let reloc: Vec<u16> = owning
        .iter()
        .copied()
        .filter(|&st| {
            bl.operators.iter().any(|o| is_var_null_init(o, st))
                && bl.operators.iter().any(|o| contains_alloc(o, st, db_nr))
        })
        .collect();
    // Pull the relocatable null-inits out of the body (keyed by store), and DROP the
    // existing scope-exit `OpFreeRef(Var(st))` for every store we early-free.
    let mut saved: HashMap<u16, Value> = HashMap::new();
    let kept: Vec<Value> = std::mem::take(&mut bl.operators)
        .into_iter()
        .filter_map(|op| {
            if let Some(&st) = reloc.iter().find(|&&st| is_var_null_init(&op, st)) {
                saved.insert(st, op);
                return None;
            }
            if let Value::Call(o, args) = op.unspan()
                && *o == fr_nr
                && let Some(Value::Var(v)) = args.first().map(Value::unspan)
                && freed_set.contains(v)
            {
                return None; // scope-exit free of an early-freed store — drop it
            }
            Some(op)
        })
        .collect();
    // Rebuild: before each op that allocates store `st`, emit the early frees of the
    // stores that died before it, then `st`'s relocated null-init, then the op.
    let mut count = 0;
    for op in kept {
        let allocs_here: Vec<u16> = owning
            .iter()
            .copied()
            .filter(|&st| contains_alloc(&op, st, db_nr))
            .collect();
        for &st in &allocs_here {
            if let Some(frees) = before.get(&st) {
                for &f in frees {
                    bl.operators.push(Value::Call(fr_nr, vec![Value::Var(f)]));
                    count += 1;
                }
            }
        }
        for &st in &allocs_here {
            if let Some(ni) = saved.remove(&st) {
                bl.operators.push(ni);
                count += 1;
            }
        }
        bl.operators.push(op);
    }
    // Safety: restore any null-init whose alloc was not matched, so first_def is
    // never lost (should not happen given the reloc filter, but keep it sound).
    for (_st, ni) in saved {
        bl.operators.insert(0, ni);
    }
    count
}

/// Per-function allocation-site id for a store-owning var (1-based; 0 is the
/// "untagged" sentinel). Stable within a function so a var's `OpDatabase` and its
/// `OpFreeRef` share the same id; the global `counter` keeps ids unique across
/// functions so a cross-function wrong-store free mismatches.
fn store_site_id(v: u16, ids: &mut HashMap<u16, u16>, counter: &mut u16) -> u16 {
    *ids.entry(v).or_insert_with(|| {
        let id = *counter;
        *counter = counter.wrapping_add(1);
        if *counter == 0 {
            *counter = 1;
        }
        id
    })
}

/// Plan-57 store-identity gate (Phase 2.5) — gated IR post-pass (`LOFT_STORE_TAG`).
///
/// Rewrites store ops to their verifying variants so a free can be checked against
/// the allocation that owns the store: insert `OpStoreTag(vdb, id)` right after each
/// `OpDatabase(vdb, …)`, and replace `OpFreeRef(vdb)` with `OpFreeRefTag(vdb, id)`.
/// Normal builds (no env) never run this, so the bytecode stays byte-identical.
#[allow(clippy::too_many_arguments)]
fn tag_stores(
    code: &mut Value,
    db_nr: u32,
    fr_nr: u32,
    store_tag_nr: u32,
    free_ref_tag_nr: u32,
    tagset: &HashSet<u16>,
    ids: &mut HashMap<u16, u16>,
    counter: &mut u16,
) {
    match code {
        Value::Block(bl) | Value::Loop(bl) => {
            let mut i = 0;
            while i < bl.operators.len() {
                tag_stores(
                    &mut bl.operators[i],
                    db_nr,
                    fr_nr,
                    store_tag_nr,
                    free_ref_tag_nr,
                    tagset,
                    ids,
                    counter,
                );
                // Identify a top-level OpDatabase / OpFreeRef on a tracked Var.  Only
                // reclaim-eligible stores (`tagset`) are tagged/verified — adopted /
                // shared / file stores carry no tag (no OpStoreTag) and keep their
                // plain OpFreeRef, so the gate cannot false-positive on them.
                let hit = match bl.operators[i].unspan() {
                    Value::Call(op, args) if *op == db_nr || (*op == fr_nr && args.len() == 1) => {
                        match args.first().map(Value::unspan) {
                            Some(Value::Var(v)) if tagset.contains(v) => Some((*op == db_nr, *v)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some((is_alloc, vdb)) = hit {
                    let id = i32::from(store_site_id(vdb, ids, counter));
                    if is_alloc {
                        bl.operators.insert(
                            i + 1,
                            Value::Call(store_tag_nr, vec![Value::Var(vdb), Value::Int(id)]),
                        );
                        i += 1; // skip the inserted tag op
                    } else {
                        bl.operators[i] =
                            Value::Call(free_ref_tag_nr, vec![Value::Var(vdb), Value::Int(id)]);
                    }
                }
                i += 1;
            }
        }
        Value::If(c, t, e) => {
            tag_stores(
                c,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
            tag_stores(
                t,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
            tag_stores(
                e,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for o in ops {
                tag_stores(
                    o,
                    db_nr,
                    fr_nr,
                    store_tag_nr,
                    free_ref_tag_nr,
                    tagset,
                    ids,
                    counter,
                );
            }
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) | Value::BreakWith(_, v) => {
            tag_stores(
                v,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
        }
        Value::Span(b) => {
            tag_stores(
                &mut b.1,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
        }
        _ => {}
    }
}

/// Plan-57 last-use freeing, Phase 1 — definition-point liveness diagnostic.
///
/// Reports each **function-scoped store** whose *data* dies (last read or a
/// rebind-away) **before another store allocates** — so it is held dead to scope
/// exit while the watermark grows.  This is the I-b (sequential distinct) and
/// III-straight-line (sequential reassign) divergence block-confinement cannot
/// reach.  Returns the count.  Read-only; gated by `LOFT_LASTUSE_GUARD`.
///
/// Block-confined stores (already freed at block exit by I-a) are excluded by the
/// body-scope filter.  Genuinely live-to-the-end stores self-exclude — nothing
/// allocates after their last read.
fn last_use_guard(
    code: &Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
    fn_name: &str,
) -> usize {
    let body_scope = match code.unspan() {
        Value::Block(bl) => bl.scope,
        _ => return 0,
    };
    let mut holds: HashMap<u16, u16> = HashMap::new();
    let mut alloc: HashMap<u16, u32> = HashMap::new();
    let mut dead: HashMap<u16, u32> = HashMap::new();
    let mut last_read: HashMap<u16, u32> = HashMap::new();
    let mut seq = 0u32;
    store_liveness_walk(
        code,
        &mut seq,
        db_nr,
        gf_nr,
        fr_nr,
        &mut holds,
        &mut alloc,
        &mut dead,
        &mut last_read,
    );
    // data-death point: rebind-away if any, else last read of the data.
    let dead_at = |st: u16| {
        dead.get(&st)
            .copied()
            .or_else(|| last_read.get(&st).copied())
    };
    let mut count = 0;
    let mut stores: Vec<u16> = alloc.keys().copied().collect();
    stores.sort_unstable();
    for &st in &stores {
        if vars.scope(st) != body_scope {
            continue; // block-confined — freed at block exit by I-a
        }
        let Some(d) = dead_at(st) else { continue };
        if let Some(&later) = stores
            .iter()
            .filter(|&&w| w != st && alloc.get(&w).is_some_and(|&a| a > d))
            .min_by_key(|&&w| alloc[&w])
        {
            eprintln!(
                "[lastuse-guard] {fn_name}: store '{}' data dead @{d} but held to scope exit \
                 while '{}' allocates @{} — should have been stopped",
                vars.name(st),
                vars.name(later),
                alloc[&later],
            );
            count += 1;
        }
    }
    count
}
