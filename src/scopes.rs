// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

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
    /// Text temps minted mid-scan (the block-VALUE `__blk_N` hoists) whose
    /// `String` must be DECLARED at function scope on native — a block-local
    /// `String` behind the block's `Str` value is E0597 ("dropped while
    /// still borrowed").  Each gets a `Set(tmp, Text(""))` prepended at the
    /// function root (the `lift_vars` mechanism, text-typed).
    lift_texts: Vec<u16>,
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
    /// Reference vars whose LATEST scanned assignment gave them an OWNED store (a call
    /// whose filtered return deps are empty, a deep-copied var, …), mapped to the loop
    /// depth (`loops.len()`) at that assignment.
    ///
    /// Enforces @FR-O-Latest — a memo of @FR-O-Oracle's answer plus the loop depth at
    /// which the assignment was taken.
    ///
    /// ⚠ Not redundant with `deps` or with @FR-O-Override: it carries a TEMPORAL fact (the
    /// *latest* assignment) and a LOOP-DEPTH fact, neither of which a type-level dep list
    /// can express.  `scan_set`'s ownership-TRANSITION free is gated on this and not on
    /// `deps` for exactly that reason — freeing at the wrong loop depth would release the
    /// previous iteration's viewed store.
    /// When such a var is reassigned with a BORROW, its merged static type
    /// already carries deps, so codegen's dep-empty pre-Set free never fires —
    /// `scan_set` emits an explicit `OpFreeRef(v)` for the orphaned store
    /// instead (only at the same loop depth: emitting inside a deeper loop
    /// would re-free a viewed store on iterations 2+).
    owned_refs: HashMap<u16, usize>,
    /// @PLN85 `local_source` over-free fix (gated by `LOFT_JOIN_OWN`): heap slots
    /// that hold an OWNED store displaced by a later `Borrowed`/`Join` reassignment
    /// (`use_analysis::displaced_owned_slots`). For these, `scan_set` strips the
    /// declared deps so the OWNED path deep-copies + frees the slot — otherwise the
    /// displaced owned store is orphaned and leaks. Empty when the flag is off.
    displaced_owned: HashSet<u16>,
    /// @PLN130 F2/F8 — view bindings live across a disturbance of their container, and which
    /// disturbance it was.  See [`collect_views_to_materialise`].
    views_to_materialise: HashMap<u16, ViewCause>,
    /// loft#721 — fn-ref variable -> the definition it was assigned, or
    /// `u32::MAX` when more than one definition reaches it.  A `CallRef`'s callee
    /// is a runtime value, so this local fact is what lets the lift ask the
    /// callee's own `returns_borrowed_view()` instead of guessing from the type.
    fnref_target: HashMap<u16, u32>,
    /// loft#849 / @PLN139 — vars that no longer OWN what they hold, so their scope end must
    /// not drop it.  See [`collect_drop_transferred`].
    drop_transferred: HashSet<u16>,
    /// loft#890 — the lifted temps whose STORE a consuming op already freed, so
    /// `get_free_vars` must not free it again.  Scope-local on purpose: `skip_free` is a
    /// VARIABLE flag both backends read at ALLOCATION time too, so stamping it here made
    /// the lift borrow instead of own and the append wrote into its own source.
    free_transferred: HashSet<u16>,
    /// loft#854 — the whole-function half of the ownership oracle, computed once
    /// for `d_nr` instead of once per question.
    ///
    /// `ownership_of` walks the entire function body (and clones each defining
    /// right-hand side) to answer about ONE value. `scan_set` asks about every
    /// assignment, so a function with n of them paid n whole-function walks: a
    /// vector literal is one `Set` per element, and 86 400 elements took over 13
    /// minutes at 99 % CPU.
    ///
    /// Safe to memo for exactly as long as this `Scopes` lives: the body it
    /// summarises is `data.def(d_nr).code`, `data` is borrowed `&Data` for the
    /// whole traversal, and `run_scan_phase` installs the rewritten body only
    /// after the scan returns — so the borrow checker, not a convention, is what
    /// keeps this from going stale. A `Scopes` is built per scan phase, so a
    /// second phase re-derives it.
    fn_defs: Option<crate::use_analysis::Defs>,
}

/// Perform scope analysis on all currently known functions.
/// One scan pass for [`check`]: scan `orig_code`/`orig_vars`, prepend the
/// lift-var null-inits, apply the result to `def`, run the debug ref/leak checks,
/// and set each variable's scope.  Runs once normally; plan-57 cluster I re-runs
/// it with a non-empty `confined` map (`__vdb`/local → block scope) so a confined
/// store registers — and therefore frees — at its block exit (`put_scope`).
/// loft#721 — map each fn-ref VARIABLE to the definition assigned to it.
///
/// A non-capturing lambda is stored as a bare definition number, a capturing one
/// as `FnRef(d_nr, closure)`; both forms are searched.  A variable that receives
/// two different definitions maps to `u32::MAX` (ambiguous), and a caller that
/// cannot name one definition must not lift — see `callref_owned_return`.
/// @PLN130 F2 — the container variable an element/field read ultimately reads OUT of.
///
/// Peels the whole accessor chain, because a view can sit more than one level down:
/// `outs[2].inner` is `OpGetField(OpGetVector(outs, …), …)`, and looking only at the
/// outermost call's first argument finds another call rather than `outs`. Stopping there
/// left the nested case unrecognised — no materialise and, worse, no warning, which is the
/// one outcome the model does not allow.
///
/// Returns `None` when the chain does not bottom out in a plain variable; the caller then
/// leaves the binding exactly as it is today rather than guessing.
fn base_container_var(value: &Value, data: &Data) -> Option<u16> {
    let mut cur = value;
    loop {
        let Value::Call(d, args) = cur.unspan() else {
            return None;
        };
        if !is_view_op(data, *d) {
            return None;
        }
        match args.first().map(Value::unspan) {
            Some(Value::Var(c)) => return Some(*c),
            Some(inner) => cur = inner,
            None => return None,
        }
    }
}

/// Does this op answer a `DbRef` that lives INSIDE its first argument's store?
///
/// The one list for the accessor chain a view is read through: a field offset within a
/// record, an element within a vector, and a keyed lookup, which answers a record of the
/// collection's own store.  Every walk that asks *"what does this value ultimately read
/// out of?"* reads it, so a chain recognised by one walk cannot be unrecognised by the
/// next.
///
/// [`crate::use_analysis::is_projection_op`] is the narrower question the @P290 bracket
/// asks — which ops it can name a PROTECTABLE slot through — and stays separate.
fn is_view_op(data: &Data, d_nr: u32) -> bool {
    matches!(
        data.def(d_nr).name(),
        "OpGetVector" | "OpVectorRef" | "OpGetField" | "OpGetRecord"
    )
}

/// What an argument LIFTED into a `__lift_N` temp borrows — the deps its type must carry.
///
/// A lift temp holds a value the caller reads out of something it does not own: an element
/// of a container, a field of a record, the surviving arm of a `??`.  Typed without deps it
/// reads as the OWNER of that store and `get_free_vars` emits a scope-exit `OpFreeRef` for
/// it — releasing a container the caller still names.  Where the container is a local of the
/// same frame the bogus free lands on a store that was dying anyway and nothing reports it;
/// where it OUTLIVES the frame (a parameter, a global) the next allocation recycles the
/// record and the container reads back as another type's bytes.
///
/// So the temp borrows what the value borrows.  The walk bottoms out at:
/// * a plain `Var` — the chain reads out of that local's store;
/// * a `TupleGet` — out of the tuple's;
/// * a BLOCK, whose `result` type already carries the deps the parser derived for it (a
///   `??` lowers to one, and its type names the container the surviving arm reads);
/// * an `If`, where either arm can be the value, so the deps are the union.
///
/// `None` where none of those is reached — a value whose source cannot be named must not be
/// bound at all.  The caller then leaves the argument as it was, which costs the leak that
/// is already there; a temp typed as an owner of somebody else's store costs a
/// use-after-free, and a leak is the better of those two.
fn lift_view_deps(arg: &Value, data: &Data) -> Option<Vec<u16>> {
    match arg.unspan() {
        Value::Var(v) => Some(vec![*v]),
        Value::TupleGet(base, _) => Some(vec![*base]),
        Value::Block(bl) => {
            let d = bl.result.depend();
            if d.is_empty() { None } else { Some(d) }
        }
        Value::If(_, then_v, else_v) => {
            let mut d = lift_view_deps(then_v, data)?;
            for x in lift_view_deps(else_v, data)? {
                if !d.contains(&x) {
                    d.push(x);
                }
            }
            Some(d)
        }
        Value::Call(d_nr, cargs) if is_view_op(data, *d_nr) => lift_view_deps(cargs.first()?, data),
        _ => None,
    }
}

/// Every container variable `code` REMOVES from.
///
/// Detects one of @FR-B-Disturb's three place-ending events — REMOVING from a container.
/// (The other two are RE-KEYING an element and REASSIGNING the container itself; note that
/// OVERWRITING a place does not disturb it, since the write lands in the place the view
/// already points at.)
///
/// `v.remove(i)` lowers to `OpRemoveVector(v, size, index)` (container = arg 0) and the
/// in-loop `e#remove` to `OpRemove(index, container, …)` (container = arg 1). Both renumber
/// the positions inside the container's store, which is what invalidates an element view.
///
/// Only a container named by a plain `Var` is collected; a reshape reached through some
/// other expression is not recognised, so the answer is a lower bound and a missed case
/// keeps today's behaviour rather than inventing a new one.
/// **Deliberately NOT extended across a call.** A callee that removes from a `&vector`
/// parameter reshapes the CALLER's container, and a view the caller holds then goes stale with
/// no diagnostic — measured, the write is silently lost (@PLN130 probe 38 cell A1, both
/// backends, and it reproduces on mainline). Propagating the reshape to the call site was tried
/// and reverted: it did not fix A1 (the view's dep does not name a `&vector` PARAMETER, so the
/// materialise arm never fires) and it broke cell C1, where the viewed element does not move
/// and the write legitimately lands. It would also make a `&` argument silently become a copy
/// whenever the callee removes from the container, which changes what `&` means (@PLN87) rather
/// than fixing a bug. Filed as [loft#779](https://github.com/loft-lang/loft/issues/779); the
/// decided answer is to REFUSE that program, not to copy behind the author's back.
fn reshaped_containers(code: &Value, data: &Data) -> HashSet<u16> {
    let mut out: HashSet<u16> = HashSet::new();
    code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let arg = match data.def(*d).name() {
            "OpRemoveVector" => args.first(),
            "OpRemove" => args.get(1),
            _ => return,
        };
        if let Some(Value::Var(c)) = arg.map(Value::unspan) {
            out.insert(*c);
        }
    });
    out
}

/// @PLN130 F8 — which `&` parameters of `d_nr` are REASSIGNED wholesale by its body.
///
/// A `&` param's slot is a double indirection into the caller's variable, and @PLN87 P2.2
/// lowers `p = T{…}` on one to *"build a fresh store, write it through, free the store the
/// caller was holding"*. So a callee doing that destroys a store the CALLER may have views
/// into — measured on `--native` as a read of `703`, a `Wide` field belonging to an unrelated
/// later allocation. Nothing at the call site says so, which is why the caller needs this
/// fact about the callee rather than a guess from the argument's shape.
///
/// Answered from the callee's IR: a `Set` targeting an argument slot whose declared type is
/// `RefVar`. Argument slots lead the variable numbering, so the slot number indexes the
/// attribute list. A shape this does not recognise yields no fact and keeps today's
/// behaviour — the same lower-bound stance as [`collect_reshaped_containers`].
fn reassigned_ref_params(data: &Data, d_nr: u32) -> HashSet<u16> {
    let def = data.def(d_nr);
    let mut out: HashSet<u16> = HashSet::new();
    def.code.walk(&mut |v| {
        let Value::Set(slot, rhs) = v else { return };
        if matches!(rhs.unspan(), Value::Null) {
            return;
        }
        if let Some(a) = def.attributes.get(usize::from(*slot))
            && matches!(a.typedef, Type::RefVar(_))
        {
            out.insert(*slot);
        }
    });
    out
}

/// See through the `OpCreateStack` wrapper an argument passed to a `&` parameter carries.
///
/// A bare `Var` is returned unchanged, so a lowering change that stops wrapping cannot
/// silently lose the fact this is asked for.
fn peel_stack_ref<'a>(arg: &'a Value, data: &Data) -> &'a Value {
    let inner = arg.unspan();
    if let Value::Call(cs, cargs) = inner
        && data.def(*cs).name() == "OpCreateStack"
        && let Some(first) = cargs.first()
    {
        return first.unspan();
    }
    inner
}

/// @PLN130 F9 — which `&` parameters of `d_nr` its body REMOVES from.
///
/// The mirror of [`reassigned_ref_params`], and needed for the same reason: a `&` parameter
/// is a double indirection into the CALLER's variable, so `all.remove(0)` in the callee
/// renumbers the caller's container. Nothing at the call site says so, which is why the
/// caller needs this fact about the callee rather than a guess from the argument's shape
/// ([loft#779](https://github.com/loft-lang/loft/issues/779)).
///
/// Answered from the callee's IR: an `OpRemoveVector` / `OpRemove` whose container argument
/// is an argument slot declared `RefVar`. Argument slots lead the variable numbering, so the
/// slot number indexes the attribute list. This is the DIRECT answer only; a removal further
/// down reaches the caller through [`removed_params_map`], which closes it over the call graph.
fn removed_ref_params(data: &Data, d_nr: u32) -> HashSet<u16> {
    let def = data.def(d_nr);
    let mut out: HashSet<u16> = HashSet::new();
    if def.attributes.is_empty() {
        return out;
    }
    def.code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let arg = match data.def(*d).name() {
            "OpRemoveVector" => args.first(),
            "OpRemove" => args.get(1),
            _ => return,
        };
        if let Some(Value::Var(slot)) = arg.map(Value::unspan)
            && let Some(a) = def.attributes.get(usize::from(*slot))
            && matches!(a.typedef, Type::RefVar(_))
        {
            out.insert(*slot);
        }
    });
    out
}

/// @PLN130 F9 — every container variable `stmt` reshapes THROUGH A CALL: passed as a `&`
/// argument to a callee that removes from that parameter.
///
/// Deliberately separate from [`reshaped_containers`] rather than folded into it, because the
/// two answers feed different decisions and folding them was measured to break a cell. F2's
/// materialise is conservative — *any* reshape copies the view — so counting a callee's removal
/// there would silently COPY a plain view whose element never moves and whose write
/// legitimately lands, which is the regression the earlier attempt at this fix measured
/// (probe 38 cell C1). The REFUSAL wants the wider answer, because a rejected program is not
/// silently anything; the materialise wants the narrower one.
fn reshaped_via_call(stmt: &Value, data: &Data, removed: &RemovedParams) -> HashMap<u16, u32> {
    let mut out: HashMap<u16, u32> = HashMap::new();
    stmt.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let Some(params) = removed.get(d) else { return };
        for k in params {
            if let Some(Value::Var(c)) = args.get(usize::from(*k)).map(|a| peel_stack_ref(a, data))
            {
                out.insert(*c, *d);
            }
        }
    });
    out
}

/// Every user definition that removes from at least one of its `&` parameters, and which.
///
/// Built once per program rather than re-derived at each call site: the question is asked once
/// per CALL, and a callee body would otherwise be re-walked once per call to it.
type RemovedParams = HashMap<u32, HashSet<u16>>;

/// [`RemovedParams`], CLOSED OVER THE CALL GRAPH: a function that forwards its own `&`
/// parameter to something that removes from it removes from it too.
///
/// Without the closure the answer is one frame deep, and the hole is one an author would trip
/// over by refactoring: extracting `all.remove(0)` into a helper makes the refusal disappear and
/// the silent lost write come back (probe 40 cell X7). Closed with a worklist over
/// *"caller `c` passes its own `&` parameter `s` as callee `e`'s parameter `k`"* edges, built in
/// the same pass as the direct removals, so the cost stays one walk of each body plus the
/// propagation.
///
/// A callee reached only through a runtime fn-ref has no edge here and keeps today's behaviour —
/// a lower bound in the safe direction, since the refusal simply does not fire.
fn removed_params_map(data: &Data) -> RemovedParams {
    let mut out = RemovedParams::new();
    let mut work: Vec<(u32, u16)> = Vec::new();
    // (callee, its param) -> every (caller, caller's own `&` param) that feeds it.
    let mut forwards: HashMap<(u32, u16), Vec<(u32, u16)>> = HashMap::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !def.name.starts_with("n_") {
            continue;
        }
        for k in removed_ref_params(data, d_nr) {
            if out.entry(d_nr).or_default().insert(k) {
                work.push((d_nr, k));
            }
        }
        if def.attributes.is_empty() {
            continue;
        }
        def.code.walk(&mut |v| {
            let Value::Call(callee, args) = v else { return };
            if !data.def(*callee).name.starts_with("n_") {
                return;
            }
            for (i, arg) in args.iter().enumerate() {
                let Ok(i) = u16::try_from(i) else { continue };
                // Only the caller's OWN `&` parameter forwards a reshape upwards; a local
                // container passed down is reshaped inside this frame, not the caller's.
                if let Value::Var(slot) = peel_stack_ref(arg, data)
                    && def
                        .attributes
                        .get(usize::from(*slot))
                        .is_some_and(|a| matches!(a.typedef, Type::RefVar(_)))
                {
                    forwards
                        .entry((*callee, i))
                        .or_default()
                        .push((d_nr, *slot));
                }
            }
        });
    }
    while let Some(key) = work.pop() {
        let Some(ups) = forwards.get(&key) else {
            continue;
        };
        for (caller, slot) in ups {
            if out.entry(*caller).or_default().insert(*slot) {
                work.push((*caller, *slot));
            }
        }
    }
    out
}

/// @PLN130 F2/F8 — why a view had to give up its alias.
///
/// The two causes read differently to an author and have different remedies, so the walk
/// carries which one fired. A view reached by both reports the reshape: that is the cause
/// with something to act on at the container.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ViewCause {
    /// The container is RESHAPED — `v.remove(i)` / `e#remove` renumbers its positions (F2).
    Reshaped,
    /// The container VARIABLE is re-established, so the name stops meaning the store (F8).
    Reassigned,
}

/// One disturbance of a container, as the walk saw it.
///
/// The cause decides the advice line; `line` and `via` exist only so the F9 refusal can point
/// its caret at the statement responsible instead of at the whole function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Disturbance {
    cause: ViewCause,
    /// Source line of the statement that disturbed the container.
    line: u32,
    /// The callee that did the removal, when it was not this frame's own statement.
    via: Option<u32>,
    /// The container the view names — carried so a diagnostic can name it without re-deriving
    /// the view→container mapping from a frame that has since closed.
    container: u16,
}

/// Record `d` for `view`, keeping [`ViewCause::Reshaped`] when both apply.
fn record_cause(map: &mut HashMap<u16, Disturbance>, view: u16, d: Disturbance) {
    let slot = map.entry(view).or_insert(d);
    if d.cause == ViewCause::Reshaped {
        *slot = d;
    }
}

/// @PLN130 F2 + F8 — every VIEW binding that is live across a disturbance of its container.
///
/// Two disturbances, one question. A RESHAPE (`v.remove(i)`, `e#remove`) renumbers the
/// positions inside the container's store, and a view is a `DbRef` pinned to one — so a view
/// live across it silently starts naming a different element (probes 03-07, 29: a pure READ
/// answered `44/444` where its element held `33/333`, and a write tore a live record). A
/// REASSIGNMENT of the container variable is the other:
///
/// A dep names a VARIABLE, not a store instance, so a view bound from `bx.v[0]` keeps
/// reading "wherever `bx` points" — and `bx = <other value>` re-points it. The view then
/// answers the REPLACEMENT's value (measured 22 where its element held 11, on both
/// backends), and on `--native` the displaced store is freed, so the read is a genuine
/// use-after-free into whatever now occupies the space (measured 703, a `Wide` field).
///
/// A vector local is immune and that is what hid this: a vector literal allocates through a
/// hidden `__vdb_N` owner, so `a = [...]` twice mints TWO stores and the first keeps its
/// identity. A struct local has no such indirection — `bx` IS the owner.
///
/// The question this answers is deliberately about the VIEW, not the container. *"Is the
/// container re-established anywhere in this function"* is a per-FUNCTION proxy for a
/// per-BINDING fact, and it is wrong in the direction that costs correctness elsewhere: an
/// unrolled `for pf in fields(p)` re-establishes `pf` once per field, and the `is`-pattern
/// subject bound from it dies inside its own arm long before the next one — yet the proxy
/// stripped that subject's deps, which put an `OpFreeRef` in a scope where the declaration
/// was not visible and stopped `tests/scripts/45-field-iter.loft` compiling under `--native`.
///
/// So a view is at risk only where a disturbance of its container can be reached WHILE THE
/// VIEW IS STILL LIVE. A disturbance sitting in a block the view's own block has already
/// closed cannot reach it, which is exactly the unrolled-iteration case above.
///
/// Four forms establish a store, matching the four emitters measured to break:
///
/// - `OpDatabase(v, …)` — a struct literal built into `v`'s own store;
/// - `Set(v, Var(src))` — the C86 whole-value bind, which codegen lowers to
///   `OpDatabase` + `OpCopyRecord` into a fresh store (the `bx = other` swap);
/// - `Set(v, Call(f, …))` — a bind from a user function's return (`bx = mk(22)`);
/// - passing `v` as a `&` argument to a callee that reassigns that parameter wholesale
///   ([`reassigned_ref_params`]) — the callee frees the caller's store, so from the view's
///   point of view this is a reassignment that happens to be spelled as a call.
///
/// A container's FIRST establishment is never a risk: it necessarily runs before any view of
/// it can exist, so no separate "is this the definition" test is needed.
///
/// A vector-typed container can never be REASSIGNED into danger — it reaches its store
/// through a hidden `__vdb_N` owner, so re-pointing the local leaves the old store's identity
/// intact — and [`established_stores`] never reports one. It very much can be RESHAPED, which
/// is the F2 half.
///
/// **A disturbance alone is not enough: the view must still be USED afterwards.** Keying on
/// order alone is what F2 shipped (a flat per-function `reshaped_containers` set), and it
/// costs both halves of this plan's closure bar — a write that mainline lands is LOST, and the
/// advice says *"`v` is modified while `c` is in use"* of a `c` that is not in use (probe 39
/// cells L1, L5, L8, L10). So a disturbance only SHAKES the open views of that container;
/// a later read or write of a shaken view is what condemns it. A view whose last use precedes
/// the disturbance keeps its alias and writes through, which is the rustc rule.
///
/// Known lower bound: a view bound inside a nested block that is used on a LATER iteration of
/// an enclosing loop is not tracked, because the frame closes with the block. A disturbance
/// anywhere inside a loop shakes every view held from outside it before the body is walked, so
/// the ordinary loop shapes are covered; this one is not, and it keeps today's behaviour.
fn collect_views_to_materialise(
    code: &Value,
    function: &Function,
    data: &Data,
) -> HashMap<u16, ViewCause> {
    let out = ViewWalk::run(code, function, data, None, 0);
    if !out.is_empty() && std::env::var_os("LOFT_DEBUG_F8").is_some() {
        let mut names: Vec<String> = out
            .iter()
            .map(|(v, d)| format!("{}({:?})", function.name(*v), d.cause))
            .collect();
        names.sort();
        eprintln!(
            "[f8] views live across a container disturbance: {}",
            names.join(" ")
        );
    }
    out.into_iter().map(|(v, d)| (v, d.cause)).collect()
}

/// The state of [`collect_views_to_materialise`]'s in-order walk.
struct ViewWalk<'a> {
    function: &'a Function,
    data: &'a Data,
    /// One frame per open block: the views bound in it, and the container each one views.
    /// A view bound inside a block dies when that block closes.
    open: Vec<Vec<(u16, u16)>>,
    /// Views whose container has been disturbed since the bind, and by what. Being shaken is
    /// not yet a verdict — it becomes one at the next use.
    shaken: HashMap<u16, Disturbance>,
    /// The answer: views USED after their container was disturbed.
    out: HashMap<u16, Disturbance>,
    /// `Some` also counts a CALLEE's removal from a `&` parameter as a reshape (F9's refusal);
    /// `None` stays inside this frame (F2's materialise). See [`reshaped_via_call`] for why the
    /// two questions do not share an answer.
    cross_frame: Option<&'a RemovedParams>,
    /// The source line of the statement being walked, tracked from the `Value::Line` markers
    /// a block interleaves with its operators — the only line information the IR carries.
    line: u32,
}

impl ViewWalk<'_> {
    /// Walk `code` in source order and answer which views were used after their container was
    /// disturbed.
    ///
    /// `start_line` seeds the line tracking with the definition's own: a block emits a
    /// `Value::Line` marker only where the line CHANGES, so a body whose first statement is on
    /// the signature's line carries no marker at all and would otherwise report line 0.
    fn run<'a>(
        code: &Value,
        function: &'a Function,
        data: &'a Data,
        cross_frame: Option<&'a RemovedParams>,
        start_line: u32,
    ) -> HashMap<u16, Disturbance> {
        let mut walk = ViewWalk {
            function,
            data,
            open: vec![Vec::new()],
            shaken: HashMap::new(),
            out: HashMap::new(),
            cross_frame,
            line: start_line,
        };
        walk.walk_block(std::slice::from_ref(code));
        walk.out
    }

    fn walk_block(&mut self, stmts: &[Value]) {
        for stmt in stmts {
            if let Value::Line(n) = stmt.unspan() {
                self.line = *n;
            }
            self.walk_stmt(stmt);
        }
    }

    /// Descend in SOURCE ORDER, because the whole point is which came first.
    ///
    /// Only the block forms listed here are descended into; anything else — a `match`, say —
    /// is handled whole by [`Self::leaf`], which both shakes and reads uses over the entire
    /// statement. That is deliberately coarse in both directions for a form we cannot order
    /// internally, and it is the safety net that keeps an unrecognised construct from hiding
    /// a removal.
    fn walk_stmt(&mut self, stmt: &Value) {
        match stmt.unspan() {
            // `Insert` is spliced into the enclosing block rather than forming its own, so
            // its statements are siblings and a view bound there stays live here.
            Value::Insert(ops) => self.walk_block(ops),
            Value::Block(b) => self.scoped(&b.operators),
            Value::Loop(b) => {
                // A loop body runs again, so a disturbance ANYWHERE inside it precedes every
                // use inside it on the next iteration. Shaking before the body is walked is
                // what makes a view held from OUTSIDE the loop and used at the top of the
                // body come out live across a removal at the bottom of it.
                self.disturb(stmt);
                self.scoped(&b.operators);
            }
            Value::If(cond, t, e) => {
                // The condition is evaluated before either branch and is not part of one.
                self.leaf(cond);
                for branch in [t.unspan(), e.unspan()] {
                    match branch {
                        Value::Block(b) => self.scoped(&b.operators),
                        Value::Insert(ops) => self.scoped(ops),
                        other => self.leaf(other),
                    }
                }
            }
            other => self.leaf(other),
        }
    }

    /// Walk a nested block in its own frame: a view bound inside one dies with it.
    fn scoped(&mut self, stmts: &[Value]) {
        self.open.push(Vec::new());
        self.walk_block(stmts);
        self.open.pop();
    }

    /// Shake for everything `stmt` disturbs, at any depth inside it.
    fn disturb(&mut self, stmt: &Value) {
        self.shake(
            &reshaped_containers(stmt, self.data),
            ViewCause::Reshaped,
            None,
        );
        if let Some(removed) = self.cross_frame {
            for (container, callee) in reshaped_via_call(stmt, self.data, removed) {
                self.shake(
                    &HashSet::from([container]),
                    ViewCause::Reshaped,
                    Some(callee),
                );
            }
        }
        let established = established_stores(stmt, self.function, self.data);
        self.shake(&established, ViewCause::Reassigned, None);
    }

    /// One statement, in the order its parts take effect: what it disturbs, then what it
    /// uses, then what it (re)binds.
    fn leaf(&mut self, stmt: &Value) {
        self.disturb(stmt);
        // Reading or writing a shaken view is what makes the disturbance matter.
        self.note_uses(stmt);
        // A `Set` REPLACES whatever the slot held, so the old binding's troubles end here and
        // a view bound by this statement is live from here on. Both recorded LAST, so a
        // statement that re-establishes a container and binds a view of the NEW value does
        // not mark the fresh view against its own establishment.
        if let Value::Set(v, rhs) = stmt.unspan() {
            self.shaken.remove(v);
            for frame in &mut self.open {
                frame.retain(|(view, _)| view != v);
            }
            if matches!(
                self.function.tp(*v),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            ) && let Some(container) = base_container_var(rhs.unspan(), self.data)
                && let Some(frame) = self.open.last_mut()
            {
                frame.push((*v, container));
            }
        }
    }

    /// Mark every open view of one of `containers` as disturbed. Not a verdict yet — only a
    /// use after this point makes the view wrong, which is what separates one that is live
    /// across the disturbance from one that is already dead.
    fn shake(&mut self, containers: &HashSet<u16>, cause: ViewCause, via: Option<u32>) {
        if containers.is_empty() {
            return;
        }
        let hit: Vec<(u16, u16)> = self
            .open
            .iter()
            .flatten()
            .filter(|(_, container)| containers.contains(container))
            .copied()
            .collect();
        for (view, container) in hit {
            let d = Disturbance {
                cause,
                line: self.line,
                via,
                container,
            };
            record_cause(&mut self.shaken, view, d);
        }
    }

    /// Condemn every shaken view this statement reads or writes.
    ///
    /// A `Set`'s target is a `u16` slot rather than a `Value::Var`, so a walk for `Var` nodes
    /// already counts only genuine reads — a rebind does not read the binding it replaces.
    fn note_uses(&mut self, stmt: &Value) {
        if self.shaken.is_empty() {
            return;
        }
        let mut used: Vec<u16> = Vec::new();
        stmt.walk(&mut |v| {
            if let Value::Var(x) = v {
                used.push(*x);
            }
        });
        for v in used {
            if let Some(cause) = self.shaken.get(&v).copied() {
                record_cause(&mut self.out, v, cause);
            }
        }
    }
}

/// Visit every node of `code`, telling `f` which source LINE is in effect for it.
///
/// A statement's line is not on the statement: a block interleaves `Value::Line(n)` markers
/// with its operators, so the line has to be carried down the walk. `Block`, `Loop` and
/// `Insert` therefore re-read it per operator; every other form inherits its parent's.
fn walk_lined(code: &Value, line: u32, f: &mut impl FnMut(&Value, u32)) {
    let node = code.unspan();
    let stmts: Option<&[Value]> = match node {
        Value::Block(b) | Value::Loop(b) => Some(&b.operators),
        Value::Insert(ops) => Some(ops),
        _ => None,
    };
    f(node, line);
    if let Some(stmts) = stmts {
        let mut cur = line;
        for s in stmts {
            if let Value::Line(n) = s.unspan() {
                cur = *n;
            }
            walk_lined(s, cur, f);
        }
    } else {
        node.for_each_child(&mut |c| walk_lined(c, line, f));
    }
}

/// @PLN130 F9 — does argument `arg` name an ELEMENT of container `c`?
///
/// Both ways an author can write it, because the two reach the check in different shapes:
///
/// - **bound earlier** (`t = v[2]; f(t, v)`) — `t` arrives as a plain `Var` and carries `v` in
///   its type deps, which is the borrow relation itself;
/// - **written into the call** (`f(v[2], v)`) — the parser does not leave that inline. It lifts
///   the projection into a temp first, so the argument is
///   `Insert([Set(t, OpGetVector(v, …)), OpCreateStack(t)])` and the alias is a `Set` INSIDE the
///   argument expression. Reading only the argument's value misses it, which is how the issue's
///   own repro (`shift(v[2], v)`) went unreported while `t = v[2]; shift(t, v)` did not.
///
/// The lifted `Set` is looked up by the temp the argument actually passes rather than by
/// searching the expression for any projection of `c`: `f(w[v[0].n], v)` mentions `v[0]` but
/// passes an element of `w`, and a search would refuse it.
fn arg_references_element_of(arg: &Value, c: u16, function: &Function, data: &Data) -> bool {
    let Value::Var(t) = arg_target(arg, data) else {
        return base_container_var(arg_target(arg, data), data) == Some(c);
    };
    if *t == c {
        return false;
    }
    if function.tp(*t).depend().contains(&c) {
        return true;
    }
    let mut lifted = false;
    arg.walk(&mut |n| {
        if let Value::Set(s, rhs) = n
            && *s == *t
            && base_container_var(rhs.unspan(), data) == Some(c)
        {
            lifted = true;
        }
    });
    lifted
}

/// The value an argument ultimately passes: the tail of any lifting preamble, with the
/// `OpCreateStack` wrapper a `&` parameter adds peeled off.
///
/// Deliberately NOT folded into [`peel_stack_ref`]: that one answers which variable a `&`
/// CONTAINER argument names, where no lift is involved, and widening it would quietly change
/// what @PLN130 F8's `established_stores` treats as a reassignment.
fn arg_target<'a>(arg: &'a Value, data: &Data) -> &'a Value {
    let inner = arg.tail().unspan();
    if let Value::Call(cs, cargs) = inner
        && data.def(*cs).name() == "OpCreateStack"
        && let Some(first) = cargs.first()
    {
        return first.tail().unspan();
    }
    inner
}

/// @PLN130 F9 — one program shape the compiler REFUSES, ready to be reported.
///
/// Carries a position rather than being emitted here, because the analysis runs over `Data`
/// (where every callee's body is available) while the diagnostics collector lives on the
/// parser's lexer.
pub struct ReshapeRefusal {
    pub file: String,
    pub line: u32,
    pub message: String,
}

/// @PLN130 F9 / [loft#779](https://github.com/loft-lang/loft/issues/779) — the shapes where a
/// container is reshaped while a reference into it is still live, which loft REFUSES.
///
/// **B-Ref-Alias is unconditional** — a `&` binding is a live link to the source, so every
/// write through it reaches the source. There is exactly one program shape where it cannot:
/// `remove` renumbers the positions inside a container's store, and a reference is pinned to
/// one, so a write through it lands on the wrong element or on a vacated slot and is lost.
/// Rather than carry runtime machinery to re-point the link, that shape is rejected before it
/// runs (maker, 2026-08-05). It is the rustc bargain in loft's spelling: where rustc refuses
/// the mutation while a borrow is live, loft refuses the removal.
///
/// Two producers, because a reference into a container reaches a removal two ways:
///
/// 1. a **`&` LINK in this frame** (`c = &v[0]`) that is live across a removal from `v` — the
///    removal being either this frame's own or one a callee does through a `&` parameter;
/// 2. a **CALL that is handed both a container and a reference into it** (`shift(v[2], v)`),
///    where the callee removes from the container parameter. Checked at the CALL SITE, which is
///    the only place the two arguments are known to name the same store: inside the callee they
///    are two unrelated parameters, and refusing there would reject sound programs.
///
/// **Liveness is the condition, not existence** — the rustc rule, and the same walk F2 uses.
/// `c = &v[0]; c.n = 1; v.remove(0);` keeps compiling: the link is dead before the removal, so
/// there is no conflict and the write lands.
///
/// Producer 2 does **not** ask whether the argument was spelled `&`, and that is measured, not
/// an oversight: a plain struct parameter aliases the caller's element exactly as a `&` one
/// does (`fn w(t: Box) { t.n = 99 }` called as `w(v[2])` writes 99 into `v` — and loft's own
/// `warn_redundant_amp` advice tells authors so). Refusing only the `&` spelling would mean an
/// author who takes that advice and drops the `&` trades a compile error for a silent lost
/// write. Producer 1 is `&`-only for the opposite and equally measured reason: a PLAIN local
/// bind does not alias across a reshape, because @PLN130 F2 materialises it and says so.
///
/// Known lower bound, in the safe direction (the refusal simply does not fire): a callee
/// reached only through a runtime fn-ref has no static call edge, so [`removed_params_map`]'s
/// closure cannot follow it. Declaration order does NOT matter — the check runs once the whole
/// world is parsed, so a callee written below its caller is answered the same way.
///
/// Every definition is checked, the stdlib's included. Filtering by `source` was tried and is
/// wrong: `Parser::parse_str` — the whole Rust test harness — never leaves `STD_SOURCE`, so the
/// filter silently made the check a no-op there while it still fired on a file. A pass over
/// definitions that cannot possibly trip it is the cheaper mistake.
#[must_use]
pub fn reshape_refusals(data: &Data) -> Vec<ReshapeRefusal> {
    let removed = removed_params_map(data);
    let mut out: Vec<ReshapeRefusal> = Vec::new();
    for d_nr in 0..data.definitions() {
        out.extend(def_reshape_refusals(data, d_nr, &removed));
    }
    out
}

fn def_reshape_refusals(data: &Data, d_nr: u32, removed: &RemovedParams) -> Vec<ReshapeRefusal> {
    let def = data.def(d_nr);
    if !matches!(def.def_type, DefType::Function) || matches!(def.code, Value::Null) {
        return Vec::new();
    }
    let function = &def.variables;
    let file = def.position.file.clone();
    let mut out: Vec<ReshapeRefusal> = Vec::new();
    // (1) — a `&` link this frame holds, still live where its container is disturbed. Every
    // cause the walk reports is refused: each one ends the place the reference names, and a
    // reference that cannot reach its source is not what `&` asked for.
    for (view, d) in ViewWalk::run(&def.code, function, data, Some(removed), def.position.line) {
        if !function.is_amp_link(view) {
            continue;
        }
        let view_name = function.name(view);
        let container = function.name(d.container);
        // The two causes destroy the place differently, so they read differently and have
        // different ways out — but the verdict is the same.
        let (what, why) = match d.cause {
            ViewCause::Reshaped => (
                format!("remove from `{container}`"),
                format!(
                    "a removal renumbers the remaining elements, so a write through \
                     `{view_name}` would no longer reach the element it names"
                ),
            ),
            ViewCause::Reassigned => (
                format!("give `{container}` a new value"),
                format!(
                    "`{view_name}` names a place inside `{container}`, and replacing \
                     `{container}` leaves that place with nothing to point at"
                ),
            ),
        };
        let message = match d.via {
            Some(callee) => format!(
                "cannot call `{callee_name}` while `{view_name}` references a place inside \
                 `{container}` — `{callee_name}` would {what}, and {why}. Move the call after \
                 the last use of `{view_name}`, or bind without `&` to work on a copy",
                callee_name = data.def(callee).original_name()
            ),
            None => format!(
                "cannot {what} while `{view_name}` references a place inside it — {why}. Move \
                 it after the last use of `{view_name}`, or bind without `&` to work on a copy"
            ),
        };
        out.push(ReshapeRefusal {
            file: file.clone(),
            line: d.line,
            message,
        });
    }
    // (2) — a call handed both a container and a reference into it.
    walk_lined(&def.code, def.position.line, &mut |node, line| {
        let Value::Call(callee, args) = node else {
            return;
        };
        let cdef = data.def(*callee);
        let Some(params) = removed.get(callee) else {
            return;
        };
        for k in params {
            let k = usize::from(*k);
            let Some(Value::Var(c)) = args.get(k).map(|a| peel_stack_ref(a, data)) else {
                continue;
            };
            for (j, arg) in args.iter().enumerate() {
                if j == k {
                    continue;
                }
                // Only a parameter that can NAME an element is a hazard; a scalar or a text
                // copies, so there is nothing pinned to a position.
                let Some(attr) = cdef.attributes.get(j) else {
                    continue;
                };
                let ptp = match &attr.typedef {
                    Type::RefVar(inner) => inner.as_ref(),
                    other => other,
                };
                if !matches!(ptp, Type::Reference(_, _) | Type::Enum(_, true, _)) {
                    continue;
                }
                if !arg_references_element_of(arg, *c, function, data) {
                    continue;
                }
                out.push(ReshapeRefusal {
                    file: file.clone(),
                    line,
                    message: format!(
                        "cannot pass both `{cname}` and a reference into it to `{fname}` — \
                         `{fname}` removes from `{cparam}`, which renumbers the remaining \
                         elements while `{vparam}` still references one, so a write through \
                         `{vparam}` would be lost. Pass the INDEX instead and read the element \
                         again after the removal",
                        cname = function.name(*c),
                        fname = cdef.original_name(),
                        cparam = cdef.attributes[k].name,
                        vparam = attr.name,
                    ),
                });
            }
        }
    });
    out.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.message.cmp(&b.message)));
    out.dedup_by(|a, b| a.line == b.line && a.message == b.message);
    out
}

/// Every variable whose store `stmt` establishes, at any depth.
///
/// Nested blocks are included deliberately: `if flag { bx = T{…} }` establishes `bx` as far
/// as a view bound outside the `if` is concerned.
fn established_stores(stmt: &Value, function: &Function, data: &Data) -> HashSet<u16> {
    let mut out: HashSet<u16> = HashSet::new();
    let note = |v: u16, out: &mut HashSet<u16>| {
        if matches!(
            function.tp(v),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && !function.is_compiler_generated(v)
        {
            out.insert(v);
        }
    };
    stmt.walk(&mut |v| match v {
        Value::Call(d, args) if data.def(*d).name() == "OpDatabase" => {
            if let Some(Value::Var(t)) = args.first().map(Value::unspan) {
                note(*t, &mut out);
            }
        }
        // A call that reassigns one of its `&` parameters displaces the caller's store.
        Value::Call(d, args) if data.def(*d).name.starts_with("n_") => {
            let reassigned = reassigned_ref_params(data, *d);
            for (i, arg) in args.iter().enumerate() {
                if !u16::try_from(i).is_ok_and(|i| reassigned.contains(&i)) {
                    continue;
                }
                if let Value::Var(t) = peel_stack_ref(arg, data) {
                    note(*t, &mut out);
                }
            }
        }
        Value::Set(t, rhs) => {
            // `Set(v, Null)` is the in-place re-init prelude that PRECEDES an `OpDatabase`
            // for the same var, not an establishment of its own.
            let establishes = match rhs.unspan() {
                Value::Var(src) => matches!(
                    function.tp(*src),
                    Type::Reference(_, _) | Type::Enum(_, true, _)
                ),
                Value::Call(f, _) => data.def(*f).name.starts_with("n_"),
                _ => false,
            };
            if establishes {
                note(*t, &mut out);
            }
        }
        _ => {}
    });
    out
}

/// @PLN139 stage C — the vars that HANDED OFF what they hold, so their scope end must not
/// drop it.  Two ways a value stops being its variable's to release, both an `OpCopyRecord`:
///
/// - **the source-free bit (`0x8000`)** — "deep-copy me, then FREE my source store", the
///   move a collection element-append does for a value it knows is dead after the copy.
///   The store is gone but the variable still names it, so the scope-exit drop ran the
///   author's release on a freed record — and once the slot was recycled by the next
///   allocation, on somebody else's LIVE one. Two elements closed the second's resource
///   twice and the first's never (loft#849); `LOFT_STRICT_STORES` called it a use-after-free.
/// - **a FIELD destination (`OpGetField`)** — construction copies a droppable into a
///   container, and @PLN139 makes that a MOVE: the container's copy is the owner now, and
///   the container's death releases it through the cascade. Without this the resource is
///   released twice — once by the source at its own scope end (early, while the container
///   still holds it, which is the use-after-free @PLN138 met) and once by the cascade.
///
/// Only the DROP is suppressed. The two cases differ in what happens to the store — the
/// first has already been freed, the second keeps its own copy — so the free is left to the
/// ordinary sweep, which is null-tolerant either way.
///
/// Only a plain `Var` source can be marked: any other expression names no slot that could
/// carry a scope-exit drop.
fn collect_drop_transferred(code: &Value, function: &Function, data: &Data) -> HashSet<u16> {
    let copy_d = data.def_nr("OpCopyRecord");
    let mut out: HashSet<u16> = HashSet::new();
    if copy_d == u32::MAX {
        return out;
    }
    code.walk(&mut |n| {
        match n {
            Value::Call(d, args) if *d == copy_d && args.len() >= 3 => {
                let moved = matches!(args[2].unspan(), Value::Int(tp) if tp & 0x8000 != 0);
                if !moved
                    && !copy_hands_off(&args[1], function, data)
                    && !appends_to_element(&args[1], function, data)
                {
                    return;
                }
                if let Some(src) = drop_bearing_source(&args[0]) {
                    out.insert(src);
                }
            }
            // A CONSTRUCTION block delivers its work-ref's record to the binding rather than
            // copying it, so the two then name ONE record and only the binding owns it.
            // Without this both released it: a struct-enum literal always takes the work-ref
            // path (its declared type is the enum, the constructed one the variant, so the
            // record cannot be built in place) and `w: W = WH { h: c }` cascaded twice.
            Value::Set(v, rhs) => {
                if let Some(w) = construction_work_ref(rhs, function)
                    && w != *v
                {
                    out.insert(w);
                }
            }
            _ => {}
        }
    });
    out
}

/// The work-ref a CONSTRUCTION block hands to its target, if that is what `rhs` is.
///
/// Deliberately not a bare `Var`: `x = y` between two locals deep-copies, so both keep their
/// own store and both must release. Only a block/insert whose tail is a work-ref delivers
/// the record itself.
fn construction_work_ref(rhs: &Value, function: &Function) -> Option<u16> {
    match rhs.unspan() {
        Value::Block(_) | Value::Insert(_) => {
            let v = drop_bearing_source(rhs)?;
            let n = function.name(v);
            (n.starts_with("__ref_") || n.starts_with("__rref_")).then_some(v)
        }
        _ => None,
    }
}

/// loft#890 — the argument index whose STORE `outer_call` frees WHOLE for itself, via
/// the `0x8000` source-free bit on its `const u16` type parameter.
///
/// Only `OpReplaceKeyed` answers.  Every op carrying that bit frees SOMETHING, but only
/// the keyed whole-collection replace frees a store that a `__lift_N` also owns: its
/// source is a keyed collection minted by a call, which is a store of its own.
/// `OpCopyRecord`'s move releases a RECORD inside a store whose life the append site
/// already governs (@PLN85's Join-return machinery reads that site), so answering for it
/// here would take the free away from the analysis that owns it.
fn moved_source_arg(outer_call: u32, args: &[Value], data: &Data) -> Option<usize> {
    if outer_call == u32::MAX || outer_call != data.def_nr("OpReplaceKeyed") {
        return None;
    }
    matches!(args.get(2).map(Value::unspan), Some(Value::Int(tp)) if tp & 0x8000 != 0).then_some(0)
}

/// Does a copy into `dest` hand the source's OWNERSHIP over — i.e. will something else
/// release it?  True for a field of a container whose type has a synthesized cascade.
///
/// The cascade coverage is the whole point of asking (see `Data::has_drop_cascade`): while
/// enum payloads and collection elements are not cascaded yet, a copy into one of those is
/// NOT a hand-off, so its source keeps dropping exactly as it does today. Suppressing there
/// would turn today's early release into a silent leak — the failure mode that makes half a
/// cascade worse than none.
pub(crate) fn copy_hands_off(dest: &Value, function: &Function, data: &Data) -> bool {
    let get_field_d = data.def_nr("OpGetField");
    if get_field_d == u32::MAX {
        return false;
    }
    let Value::Call(d, fargs) = dest.unspan() else {
        return false;
    };
    if *d != get_field_d {
        return false;
    }
    // The container holding the field: the only form that names a type here is a var.
    let Some(Value::Var(cv)) = fargs.first().map(Value::unspan) else {
        return false;
    };
    match function.tp(*cv).base() {
        Type::Reference(cd, _) | Type::Enum(cd, true, _) => data.has_drop_cascade(*cd),
        _ => false,
    }
}

/// Does a copy into `dest` hand ownership to a COLLECTION element?
///
/// `_elm_N` is the element `OpNewRecord` hands back, so a copy into it is the element-append.
/// The releaser is not the element's own type but the COLLECTION's cascade, which walks every
/// element — and that loop is emitted exactly when the element type owns a droppable, so that
/// is the condition to test.
///
/// Needed beside the `0x8000` case, which only fires when the source is dead after the copy.
/// A NAMED local appended to a collection (`v: vector<H> = [h1, h2]`) stays live, so no move
/// bit is set — and once the collection releases its elements, leaving the local dropping too
/// means one resource released twice.
pub(crate) fn appends_to_element(dest: &Value, function: &Function, data: &Data) -> bool {
    let Value::Var(dv) = dest.unspan() else {
        return false;
    };
    if !function.name(*dv).starts_with("_elm_") {
        return false;
    }
    match function.tp(*dv).base() {
        Type::Reference(ed, _) | Type::Enum(ed, true, _) => data.owns_droppable(*ed),
        _ => false,
    }
}

/// The variable a copy SOURCE ultimately names, or `None` when it names no slot.
///
/// A plain `Var` is the named-local case. An `Object` construction reaches here as the block
/// that BUILDS it, whose tail is the work-ref holding the finished record — `Nest { s: S { … } }`
/// copies such a block into `Nest`'s field, and without peeling it the inner `S` temp kept a
/// scope-exit drop and released the payload a second time.
pub(crate) fn drop_bearing_source(src: &Value) -> Option<u16> {
    match src.unspan() {
        Value::Var(v) => Some(*v),
        Value::Block(bl) => bl.operators.last().and_then(drop_bearing_source),
        Value::Insert(ops) => ops.last().and_then(drop_bearing_source),
        _ => None,
    }
}

fn collect_fnref_targets(code: &Value, function: &Function) -> HashMap<u16, u32> {
    let mut out: HashMap<u16, u32> = HashMap::new();
    code.walk(&mut |v| {
        let Value::Set(var, rhs) = v else { return };
        if !matches!(function.tp(*var).base(), Type::Function(_, _, _)) {
            return;
        }
        // An explicit `FnRef`/`FnRefDnr` names the target wherever it sits — a
        // capturing lambda's assignment is a BLOCK (build the closure record,
        // then the ref), so the marker is nested.
        let mut found: Option<u32> = None;
        let mut ambiguous = false;
        rhs.walk(&mut |inner| {
            let d = match inner {
                Value::FnRef(d, _, _) => u32::try_from(*d).ok(),
                Value::FnRefDnr(d) => Some(u32::from(*d)),
                _ => None,
            };
            if let Some(d) = d {
                match found {
                    Some(prev) if prev != d => ambiguous = true,
                    _ => found = Some(d),
                }
            }
        });
        // A NON-capturing lambda is stored as the bare definition number.  Only a
        // DIRECT integer counts: that same block above is full of unrelated ints
        // (a type id, a field offset), and treating those as candidates made every
        // capturing case read as ambiguous.
        if found.is_none()
            && let Value::Int(d) = rhs.unspan()
        {
            found = u32::try_from(*d).ok();
        }
        if let Some(d) = found {
            let slot = out.entry(*var).or_insert(d);
            if ambiguous || *slot != d {
                *slot = u32::MAX;
            }
        }
    });
    out
}

/// The scope-exit / pre-reassignment frees for a TUPLE local's OWNED elements, in
/// reverse index order.
///
/// `Value::TupleGet(v, idx)` reads one element as a plain `DbRef` / `Str`, so each
/// free is the ordinary op on that read and needs no per-element stack-offset
/// machinery of its own.
///
/// Only an element the tuple OWNS. `owned_elements` answers on the type KIND — is
/// it heap-shaped — which a BORROWED element passes exactly as an owned one does,
/// and freeing that releases the source's store.  The empty-dep test is the
/// ownership half, the same one the scalar branch of `get_free_vars` uses.
///
/// STORE-backed elements only.  A tuple's `text` element is an owned `String` in a
/// stack slot, not a store, and `OpFreeText` takes the VARIABLE whose slot it
/// resets — handed a `TupleGet` value read instead, its stack-distance arithmetic
/// underflows (loft#1004's corpus script panics with "attempt to subtract with
/// overflow").  Text elements are released with the frame and are not the leak
/// this exists for.
fn tuple_owned_elem_frees(elems: &[Type], v: u16, data: &Data) -> Vec<Value> {
    let mut out = Vec::new();
    for &(_offset, idx) in crate::data::owned_elements(elems).iter().rev() {
        if !elems[idx].depend().is_empty() || matches!(elems[idx].base(), Type::Text(_)) {
            continue;
        }
        out.push(Value::Call(
            data.def_nr("OpFreeRef"),
            vec![Value::TupleGet(v, idx as u16)],
        ));
    }
    out
}

fn run_scan_phase(
    data: &mut Data,
    d_nr: u32,
    orig_code: &Value,
    orig_vars: &Function,
    confined: &HashMap<u16, u16>,
) {
    // @PLN85 `local_source` over-free fix (gated): the heap slots whose OWNED store
    // is displaced by a later borrow/join reassignment. Computed on the pre-scope
    // code so the dep classification is read before any dep-strip below mutates it.
    let displaced_owned = if crate::keys::join_own_enabled() {
        crate::use_analysis::displaced_owned_slots(orig_code, orig_vars, data)
    } else {
        HashSet::new()
    };
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
        lift_texts: Vec::new(),
        ret_temp_counter: 0,
        paired_witness: HashMap::new(),
        witness_buffer: HashMap::new(),
        owned_refs: HashMap::new(),
        displaced_owned,
        views_to_materialise: collect_views_to_materialise(orig_code, orig_vars, data),
        fnref_target: collect_fnref_targets(orig_code, orig_vars),
        drop_transferred: collect_drop_transferred(orig_code, orig_vars, data),
        free_transferred: HashSet::new(),
        fn_defs: None,
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
    if !scopes.lift_texts.is_empty()
        && let Value::Block(bl) = &mut code
    {
        for &v in scopes.lift_texts.iter().rev() {
            bl.operators.insert(0, v_set(v, Value::Text(String::new())));
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

/// Prepend `ni` to the operators of the Block whose `scope == target`, descending
/// only the control-flow spine (`Block`/`Insert`/`If`/`Span`/`Return`).  Returns
/// `None` once inserted, or `Some(ni)` (un-consumed) if no such block was reached.
/// A confined block nested inside a `Set`/`Call`/`Iter` value (a `map`/`filter`
/// body, a short-lambda capture) is deliberately NOT entered: the Plan-57 null-init
/// relocation is a best-effort watermark optimization, and such a block keeps its
/// body-0 null-init (the caller's fallback), which is leak/poison-clean.  Widening
/// the descent to every child would relocate into far more functions for marginal
/// benefit — out of scope here; the concern is only to stop the false-positive
/// debug assert on the (correct) un-reached case.
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
        // This relocation is BEST-EFFORT: where no arm reaches, the fallback — leaving the
        // null-init at body position 0 — is correct, so an unreached shape costs placement
        // quality and never correctness.  A confined block off the control-flow spine (a
        // `map`/`filter`/lambda body) is the standing example.
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
        // Not reached by the control-flow descent — restore the null-init so the
        // `first_def` is never lost, and skip the (best-effort) relocation: the
        // store keeps its body-0 `first_def` and is still freed by the confined
        // block's scope-exit sweep (verified leak/poison-clean, both backends).
        // The confined block can legitimately live inside a `map`/`filter` body or
        // a short-lambda capture (a `Call`/`Iter`/`Set` value `prepend_to_scope`
        // does not enter — 501, 85-short-lambda-capture).  Only a `block_scope`
        // that is ABSENT FROM THE IR ENTIRELY is a `store_confinement` bug worth
        // asserting; a present-but-unreached scope is the expected miss.
        if let Value::Block(body) = code {
            body.operators.insert(0, ni);
        }
        debug_assert!(
            block_scope_present(code, block_scope),
            "relocate_null_init: block scope {block_scope} is not in the IR at all \
             (a store_confinement bug, not merely an unreached inline block)"
        );
        false
    } else {
        true
    }
}

/// True if any `Block`/`Loop` anywhere in `node`'s subtree has `scope == target`.
/// Unlike [`prepend_to_scope`], this descends into EVERY child (`Value::walk`), so
/// it distinguishes a scope that is genuinely absent from one that merely lives off
/// the control-flow spine (inside a `map`/`filter` body, a lambda capture).  Only
/// consulted from the `relocate_null_init` `debug_assert!`, but must compile in
/// release too (the assert's argument is still type-checked there).
fn block_scope_present(node: &Value, target: u16) -> bool {
    let mut found = false;
    node.walk(&mut |n| {
        if let Value::Block(bl) | Value::Loop(bl) = n
            && bl.scope == target
        {
            found = true;
        }
    });
    found
}

/// EXPERIMENTAL (LOFT_BORROW_ELIDE) — inline the Tier-0 Borrow-verdict vector
/// copies: for each elidable `v = copy(s.f)`, replace every read of `v` with the
/// source field-access `s.f` and drop the copy idiom (the `vdb` buffer's alloc /
/// length-set / append / free, and `v`'s defining `Set`). The result is the IR a
/// direct `s.f[i]` program compiles — which already aliases with no copy — so it
/// needs no borrow-dep/skip_free surgery. Runs before the scope/free passes.
fn elide_borrows(data: &mut Data) {
    let op_database = data.def_nr("OpDatabase");
    let op_append = data.def_nr("OpAppendVector");
    let op_set_int4 = data.def_nr("OpSetInt4");
    let op_set_int = data.def_nr("OpSetInt");
    let op_free = data.def_nr("OpFreeRef");

    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let plans = crate::use_analysis::elision_plans(
            &data.def(d_nr).code,
            &data.def(d_nr).variables,
            data,
        );
        if plans.is_empty() {
            continue;
        }
        // A var that BORROWS `v` (`e = v[i]`, `deps` ∋ `v`) is left with a stale dep
        // once `v` is deleted, so the borrowed-view codegen (`stack(dep[0])`) would
        // dereference a dead slot. `use_analysis` only emits a plan when every such
        // borrower is read-only/non-escaping, and lists them; RE-POINT each borrower
        // `v → source_base` so it borrows the live source element instead (codegen
        // then reads the source param's valid slot). This is what lets the
        // borrowed-element accessors elide rather than fall back to copy.
        let mut elide_v: HashMap<u16, Value> = HashMap::new();
        let mut elide_vdb: HashSet<u16> = HashSet::new();
        for p in plans {
            for &e in &p.borrowers {
                data.definitions[d_nr as usize]
                    .variables
                    .make_independent(e, p.var);
                data.definitions[d_nr as usize]
                    .variables
                    .depend(e, p.source_base);
            }
            elide_v.insert(p.var, p.source);
            elide_vdb.insert(p.vdb);
        }
        let ops = ElideOps {
            op_database,
            op_append,
            op_set_int4,
            op_set_int,
            op_free,
        };
        let mut code = data.def(d_nr).code.clone();
        elide_rewrite(&mut code, &elide_v, &elide_vdb, &ops);
        data.definitions[d_nr as usize].code = code;
    }
}

// The `op_` prefix is meaningful (these are operator def-numbers), so the
// same-prefix style lint does not apply.
#[allow(clippy::struct_field_names)]
struct ElideOps {
    op_database: u32,
    op_append: u32,
    op_set_int4: u32,
    op_set_int: u32,
    op_free: u32,
}

/// Is `stmt` a copy-idiom statement for an elided binding (to be dropped)?
fn idiom_drop(
    stmt: &Value,
    elide_v: &HashMap<u16, Value>,
    elide_vdb: &HashSet<u16>,
    o: &ElideOps,
) -> bool {
    match stmt.unspan() {
        // `v = OpGetField(vdb,…)` (the def) and `vdb = null` (the init).
        Value::Set(v, _) => elide_v.contains_key(v) || elide_vdb.contains(v),
        Value::Call(d, args) => {
            let target = match args.first().map(Value::unspan) {
                Some(Value::Var(t)) => Some(*t),
                _ => None,
            };
            if *d == o.op_database || *d == o.op_set_int4 || *d == o.op_set_int {
                target.is_some_and(|t| elide_vdb.contains(&t)) // buffer alloc / length-set
            } else if *d == o.op_append {
                target.is_some_and(|t| elide_v.contains_key(&t)) // copy-fill
            } else if *d == o.op_free {
                target.is_some_and(|t| elide_vdb.contains(&t) || elide_v.contains_key(&t))
            } else {
                false
            }
        }
        _ => false,
    }
}

fn elide_rewrite(
    node: &mut Value,
    elide_v: &HashMap<u16, Value>,
    elide_vdb: &HashSet<u16>,
    o: &ElideOps,
) {
    // Inline a read of an elided var with its source field-access.
    let replacement = match node.unspan() {
        Value::Var(v) => elide_v.get(v).cloned(),
        _ => None,
    };
    if let Some(src) = replacement {
        *node = src;
        return;
    }
    match node {
        Value::Block(b) => {
            b.operators
                .retain(|s| !idiom_drop(s, elide_v, elide_vdb, o));
            for op in &mut b.operators {
                elide_rewrite(op, elide_v, elide_vdb, o);
            }
        }
        Value::Insert(ops) => {
            ops.retain(|s| !idiom_drop(s, elide_v, elide_vdb, o));
            for op in &mut *ops {
                elide_rewrite(op, elide_v, elide_vdb, o);
            }
        }
        _ => node.for_each_child_mut(&mut |c| elide_rewrite(c, elide_v, elide_vdb, o)),
    }
}

// The `op_` prefix is meaningful (operator def-numbers), so the same-prefix lint does not apply.
#[allow(clippy::struct_field_names)]
struct MoveOps {
    op_database: u32,
    op_copy_record: u32,
    op_free: u32,
    op_new_record: u32,
    op_get_field: u32,
    op_get_vector: u32,
}

/// @PLN90 phase B (B1.3) — the last-use MOVE-elision rewrite for the RECORD copy shape
/// (`v[i] = e` / `o.f = src`, lowered as `OpCopyRecord`). When a source `s`'s store is dead
/// after the copy (a [`crate::use_analysis::MovePlan`] with `kind == Record`), build `s`'s
/// fields DIRECTLY into the copy's destination slot instead of constructing a throwaway record
/// and deep-copying it:
///
/// ```text
///   OpDatabase(s)                           (dropped)
///   OpSetInt (s, off, v)  ── retarget ──▶  OpSetInt (dest, off, v)
///   OpSetText(s, off, v)  ── retarget ──▶  OpSetText(dest, off, v)
///   OpCopyRecord(s, dest)                   (dropped — the deep copy is gone)
///   OpFreeRef(s)                            (dropped — no throwaway store to free)
/// ```
///
/// where `dest` is the `OpCopyRecord` destination expression (`OpGetVector(v,…)` / `OpGetField`).
/// Emits NO new op — both backends already lower a retargeted `OpSet*`, so the rewrite is
/// backend-agnostic (one IR pass, like `elide_borrows`). DEFAULT ON (B1.5); `LOFT_NO_MOVE_ELIDE`
/// restores the copy. The CONSTRUCT shape is handled by [`construct_move_rewrite`] (B1.3b) for
/// the reorder-free field-append case; fresh construction (`a = Bag { items: base }`, container
/// built after the source) still needs a build-order reorder and stays a copy. Design:
/// `doc/claude/plans/90-copy-diagnostics/phase-b-design.md`.
fn move_elide(data: &mut Data) {
    if !crate::keys::move_elide_enabled() {
        return;
    }
    let mo = MoveOps {
        op_database: data.def_nr("OpDatabase"),
        op_copy_record: data.def_nr("OpCopyRecord"),
        op_free: data.def_nr("OpFreeRef"),
        op_new_record: data.def_nr("OpNewRecord"),
        op_get_field: data.def_nr("OpGetField"),
        op_get_vector: data.def_nr("OpGetVector"),
    };
    let co = ConstructOps {
        op_database: data.def_nr("OpDatabase"),
        op_free: data.def_nr("OpFreeRef"),
        op_append: data.def_nr("OpAppendVector"),
        op_prealloc: data.def_nr("OpPreAllocVector"),
        op_set_int4: data.def_nr("OpSetInt4"),
        op_get_field: data.def_nr("OpGetField"),
        op_clear: data.def_nr("OpClearVector"),
        op_new_record: data.def_nr("OpNewRecord"),
        op_finish_record: data.def_nr("OpFinishRecord"),
    };
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        // Plan-based rewrites (Record / Construct) key off these; the structural `a.field = base`
        // rewrite (B1.3d) does not — so we do NOT early-continue on an empty plan set.
        let plans = crate::use_analysis::move_plans(data, d_nr);
        let mut code = data.def(d_nr).code.clone();
        // Vars to suppress the (later `variables()`-emitted) scope-exit free for — the moved-out
        // owned store now lives in the destination, so its null slot must not be freed.
        let mut skip: HashSet<u16> = HashSet::new();
        // Containers a rewrite must NOT retarget a source's build into — NOT a stable, pre-existing,
        // single-def owned slot. Two producers, unioned; every rewrite consults the result:
        //  - transient element slots (`_elm_N = OpNewRecord(…)`, reused across a vector literal /
        //    nested construction) — a fresh record defined LATER (use-before-def);
        //  - vars allocated MORE THAN ONCE — REASSIGNED (`b = Bag{…}; … b = Bag{…}`) — the container
        //    has a prior store the reorder's hoist does not retire.
        let mut bad_containers = collect_element_vars(&code, &mo);
        bad_containers.extend(collect_multi_database(&code, &mo));
        // First-def order per var — a Record destination's container must be defined BEFORE its
        // source (else the retargeted build writes into an un-allocated container).
        let def_order = collect_def_order(&code, &mo);

        // ── RECORD shape (`v[i]=e` / `o.f=src`, OpCopyRecord) ──
        let rec_sources: HashSet<u16> = plans
            .iter()
            .filter(|p| p.kind == crate::use_analysis::MoveKind::Record)
            .map(|p| p.source)
            .collect();
        if !rec_sources.is_empty() {
            // Pass 1 — capture each source's UNIQUE copy destination. A source seen copying into
            // two different places is not the clean dead-after shape the plan assumes: skip it.
            let mut dest: HashMap<u16, Value> = HashMap::new();
            let mut ambiguous: HashSet<u16> = HashSet::new();
            collect_move_dest(
                &code,
                &mo,
                &data.def(d_nr).variables,
                &rec_sources,
                &bad_containers,
                &def_order,
                &mut dest,
                &mut ambiguous,
            );
            // A destination TOUCHED between the source's build and the copy cannot take the
            // retarget: the write would move ahead of that access.  See
            // `collect_move_disturbed`.
            let mut disturbed: HashSet<u16> = HashSet::new();
            collect_move_disturbed(&code, &mo, mo.op_copy_record, 0, &dest, &mut disturbed);
            let ready: HashSet<u16> = dest
                .keys()
                .copied()
                .filter(|s| !ambiguous.contains(s) && !disturbed.contains(s))
                .collect();
            if !ready.is_empty() {
                move_rewrite(&mut code, &ready, &dest, &mo);
                skip.extend(&ready);
            }
        }

        // ── CONSTRUCT shape (`x.field += src` field-append, OpAppendVector) — REORDER-FREE only ──
        let con_sources: HashSet<u16> = plans
            .iter()
            .filter(|p| p.kind == crate::use_analysis::MoveKind::Construct)
            .map(|p| p.source)
            .collect();
        if !con_sources.is_empty() {
            // Sources that are USED outside their own construction + the single copy — read between
            // being built and being moved (`out=[]; for{ out+=[…] }; assert("{out:j}"); w={items:out}`
            // — `out` is read by the assert BEFORE the move). Building such a source directly into the
            // destination would leave that intermediate read seeing the un-built source, so leave it
            // a copy. (Also subsumes append-grown sources: `v += w` is `v` at arg0 of a non-write op.)
            let escaping: HashSet<u16> = con_sources
                .iter()
                .copied()
                .filter(|&s| source_escapes(&code, s, &co))
                .collect();
            // B1.3b — reorder-free field-appends (`x.field += src`, container already exists).
            construct_move_rewrite(
                &mut code,
                &con_sources,
                &co,
                &mo,
                &bad_containers,
                &escaping,
                &mut skip,
            );
            // B1.3c — fresh construction (`a = Bag { items: base }`, container built after the
            // source): hoist `a`'s alloc, then retarget. Runs on the copies B1.3b left standing.
            // B1.4 — the interprocedural mutation set (`find_written_vars` knows which callees
            // mutate a `&`-param in ANY arg position), so a param used as a hoisted field value is
            // allowed only if genuinely never mutated.
            let mut written: HashSet<u16> = HashSet::new();
            crate::parser::find_written_vars(
                &data.def(d_nr).code,
                data,
                &mut written,
                &mut HashMap::new(),
            );
            construct_fresh_rewrite(
                &mut code,
                &con_sources,
                &co,
                &data.def(d_nr).variables,
                &written,
                &bad_containers,
                &escaping,
                &mut skip,
            );
        }

        // B1.3d — the `a.field = base` whole-vector replacement (the `__p154_rhs` idiom): a DOUBLE
        // copy (`base → __p154_rhs → a.field`, with an `OpClearVector`). Its source is a temp, so it
        // is NOT a MovePlan — this rewrite is STRUCTURAL and must run regardless of `con_sources`.
        construct_replace_rewrite(&mut code, &co, &mut skip);

        data.definitions[d_nr as usize].code = code;
        for &s in &skip {
            data.definitions[d_nr as usize].variables.set_skip_free(s);
        }
    }
}

/// Pass 1 of [`move_elide`]: map each move source to the `OpCopyRecord` destination it copies
/// into; a source seen with a second destination is marked ambiguous (and skipped).
#[allow(clippy::too_many_arguments)]
fn collect_move_dest(
    node: &Value,
    mo: &MoveOps,
    function: &Function,
    sources: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    def_order: &HashMap<u16, usize>,
    dest: &mut HashMap<u16, Value>,
    ambiguous: &mut HashSet<u16>,
) {
    if let Value::Call(d, args) = node.unspan()
        && *d == mo.op_copy_record
        && let Some(Value::Var(s)) = args.first().map(Value::unspan)
        && sources.contains(s)
        && args.len() >= 2
    {
        // A stable in-place target is a slot EXPRESSION over a PRE-EXISTING container
        // (`OpGetVector(v,…)` / `OpGetField(o,…)`). NOT stable, and skipped:
        //  - a bare `Var` dest (`x = e`) — no proof it pre-exists the source;
        //  - a dest based on a FRESH `OpNewRecord` element (`OpGetField(_elm_N,…)` nested
        //    construction) — defined AFTER the source (native `var__elm_N` not in scope);
        //  - a dest based on ANY compiler TEMP (`_`-prefixed: `__ref_N` field-iteration refs,
        //    `_slice_*`, …) — these are populated by machinery whose validity point the structural
        //    rewrite can't prove. Only a USER-named container is a proven-stable target.
        let unstable = match args[1].unspan() {
            Value::Var(_) => true,
            _ => base_var_of(&args[1], mo).is_none_or(|base| {
                bad_containers.contains(&base)
                    || function.name(base).starts_with('_')
                    // the container must be DEFINED before the source is built (else the retargeted
                    // build writes into an un-allocated slot).
                    || def_order
                        .get(&base)
                        .zip(def_order.get(s))
                        .is_none_or(|(&bd, &sd)| bd >= sd)
            }),
        };
        if unstable || dest.contains_key(s) {
            ambiguous.insert(*s);
        } else {
            dest.insert(*s, args[1].clone());
        }
    }
    node.for_each_child(&mut |c| {
        collect_move_dest(
            c,
            mo,
            function,
            sources,
            bad_containers,
            def_order,
            dest,
            ambiguous,
        );
    });
}

/// Does `node` mention variable `v` anywhere inside it?
fn mentions_var(node: &Value, v: u16) -> bool {
    if matches!(node.unspan(), Value::Var(x) if *x == v) {
        return true;
    }
    let mut found = false;
    node.for_each_child(&mut |c| {
        if !found && mentions_var(c, v) {
            found = true;
        }
    });
    found
}

/// Sources whose retarget would move the destination's WRITE across a statement that touches
/// that destination.
///
/// [`move_rewrite`] retargets the source's construction ops onto the destination and drops the
/// copy, so the destination is written at the CONSTRUCTION's position instead of at the copy's.
/// That is sound only while nothing in between touches the destination.  The guards in
/// [`collect_move_dest`] all ask whether the destination is a STABLE container; none of them
/// asks whether it is READ, and that is the hole:
///
/// ```text
///   for p in v { held = Tg { name: p.0.name }; p.0 = p.1; p.1 = held; }
/// ```
///
/// `held`'s build moves into `p.1`, so `p.0 = p.1` then copies the NEW value back and the swap
/// answers `x|x` where it wants `y|x` — silently, on both backends.
///
/// Read by BOTH move shapes — the Record copy (`OpCopyRecord(src, dst)`, source at arg 0) and
/// the Construct append (`OpAppendVector(dst, src)`, source at arg 1) — because they had the
/// same hole and two copies of one predicate is the shape loft#1006 was.  B1.3d already carried
/// this guard for its own rewrite (`try_replace_one`: *"`base`'s BUILD must not read the
/// destination container"*), which is what the other two were missing.
///
/// Conservative by BASE variable: any mention of the destination's base container between the
/// two points disqualifies the move.  The source's own construction ops are excluded (they are
/// what gets retargeted), so `o.f = T { x: o.g }` — building FROM the container into it — is
/// still admitted.  A slot-exact test would admit a few more and cannot be spelled reliably:
/// two spellings of one slot is the shape loft#1006 was.
fn collect_move_disturbed(
    node: &Value,
    mo: &MoveOps,
    copy_op: u32,
    src_arg: usize,
    dest: &HashMap<u16, Value>,
    out: &mut HashSet<u16>,
) {
    let scan = |ops: &[Value], out: &mut HashSet<u16>| {
        for (copy_idx, op) in ops.iter().enumerate() {
            let Value::Call(d, args) = op.unspan() else {
                continue;
            };
            if *d != copy_op {
                continue;
            }
            let Some(Value::Var(s)) = args.get(src_arg).map(Value::unspan) else {
                continue;
            };
            let Some(slot) = dest.get(s) else { continue };
            let Some(base) = base_var_of(slot, mo) else {
                continue;
            };
            // Where `s` is first defined in THIS list; absent (built in another block) is
            // treated as "from the top", which is the conservative reading.
            let def_idx = ops
                .iter()
                .position(|o| match o.unspan() {
                    Value::Set(v, _) => *v == *s,
                    Value::Call(d2, a2) => {
                        *d2 == mo.op_database
                            && matches!(a2.first().map(Value::unspan), Some(Value::Var(v)) if *v == *s)
                    }
                    _ => false,
                })
                .unwrap_or(0);
            if def_idx >= copy_idx {
                continue;
            }
            for between in &ops[def_idx + 1..copy_idx] {
                // The source's OWN construction ops are what the rewrite retargets, so they
                // are not a disturbance — that is what keeps `o.f = T { x: o.g }` elidable.
                let writes_source = matches!(between.unspan(), Value::Call(_, a)
                    if matches!(a.first().map(Value::unspan), Some(Value::Var(v)) if *v == *s));
                if !writes_source && mentions_var(between, base) {
                    out.insert(*s);
                    break;
                }
            }
        }
    };
    match node.unspan() {
        Value::Block(b) => scan(&b.operators, out),
        Value::Insert(ops) => scan(ops, out),
        _ => {}
    }
    node.for_each_child(&mut |c| collect_move_disturbed(c, mo, copy_op, src_arg, dest, out));
}

/// Vars defined by `OpNewRecord` (`_elm_N = OpNewRecord(…)`) — the transient element slots the
/// vector-literal / construction lowering reuses. A copy destination based on one of these is a
/// FRESH element defined after the source, so it is not a stable in-place retarget target.
fn collect_element_vars(node: &Value, mo: &MoveOps) -> HashSet<u16> {
    fn walk(node: &Value, mo: &MoveOps, out: &mut HashSet<u16>) {
        if let Value::Set(v, rhs) = node.unspan()
            && matches!(rhs.unspan(), Value::Call(d, _) if *d == mo.op_new_record)
        {
            out.insert(*v);
        }
        node.for_each_child(&mut |c| walk(c, mo, out));
    }
    let mut out = HashSet::new();
    walk(node, mo, &mut out);
    out
}

/// Vars allocated (`OpDatabase(v, …)`) MORE THAN ONCE — a var REASSIGNED to a fresh record/vector
/// (`b = Bag{…}; … b = Bag{…}`). Such a container has a prior store the reorder rewrites' hoist does
/// not retire, so they must leave it a copy.
fn collect_multi_database(node: &Value, mo: &MoveOps) -> HashSet<u16> {
    fn walk(node: &Value, mo: &MoveOps, seen: &mut HashSet<u16>, multi: &mut HashSet<u16>) {
        if let Value::Call(d, args) = node.unspan()
            && *d == mo.op_database
            && let Some(Value::Var(v)) = args.first().map(Value::unspan)
            && !seen.insert(*v)
        {
            multi.insert(*v);
        }
        node.for_each_child(&mut |c| walk(c, mo, seen, multi));
    }
    let mut seen = HashSet::new();
    let mut multi = HashSet::new();
    walk(node, mo, &mut seen, &mut multi);
    multi
}

/// Does `src` ESCAPE its own construction — is it referenced anywhere OTHER than (a) as arg0 of a
/// WRITE op that builds it (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord`/`OpSetInt4`), or (b) as
/// arg1 of the append copy that moves it? A source that is built, then READ (`"{out:j}"`, `out[i]`,
/// passed to a fn), then moved is NOT dead-between-build-and-copy: building it directly into the
/// destination would leave the intermediate read seeing the un-built source (`var_out` not in
/// scope / wrong value). The `Set(src, …)` view-def / null-init targets `src` (not an arg) and is
/// fine; the `vdb` backing / `_elm` element temps aren't `src`. Any other appearance → escapes.
fn source_escapes(node: &Value, src: u16, co: &ConstructOps) -> bool {
    fn walk(node: &Value, src: u16, co: &ConstructOps, bad: &mut bool) {
        if let Value::Call(d, args) = node.unspan() {
            for (i, a) in args.iter().enumerate() {
                if matches!(a.unspan(), Value::Var(v) if *v == src) {
                    let write_arg0 = i == 0
                        && (*d == co.op_prealloc
                            || *d == co.op_new_record
                            || *d == co.op_finish_record
                            || *d == co.op_set_int4);
                    let copy_arg1 = i == 1 && *d == co.op_append;
                    if !(write_arg0 || copy_arg1) {
                        *bad = true;
                    }
                }
            }
        }
        node.for_each_child(&mut |c| walk(c, src, co, bad));
    }
    let mut bad = false;
    walk(node, src, co, &mut bad);
    bad
}

/// First-definition encounter order per var (`Set(v,…)` value-bind or `OpDatabase(v)` alloc). Used
/// to prove a Record destination's container is defined BEFORE the source is built — otherwise
/// retargeting the source's construction into `container.field` writes into an un-allocated slot
/// (`b = SABag { extra: extra }`: `b` allocated AFTER `extra` → `var_b` not in scope).
fn collect_def_order(node: &Value, mo: &MoveOps) -> HashMap<u16, usize> {
    // ⚠ Peels the SCRUTINEE while the recursion below walks the ORIGINAL `node`, so a spanned
    // node is visited TWICE: once here through the peel, once when `for_each_child` descends
    // to the same payload (it sees through a `Span` itself).  Safe here only because both
    // accumulations are IDEMPOTENT — `or_insert` ignores the second write, and `idx` feeds a
    // relative ORDER, so an extra tick shifts every later index equally.  Keep it that way, or
    // bind (`let node = node.unspan();`) so the match and the walk see the same node.  The
    // same shape counted a sandbox callee twice and inflated a heap bound by half — see
    // `sandbox::intrinsic_space`.
    fn walk(node: &Value, mo: &MoveOps, idx: &mut usize, out: &mut HashMap<u16, usize>) {
        match node.unspan() {
            Value::Set(v, _) => {
                out.entry(*v).or_insert(*idx);
            }
            Value::Call(d, args) if *d == mo.op_database => {
                if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                    out.entry(*v).or_insert(*idx);
                }
            }
            _ => {}
        }
        *idx += 1;
        node.for_each_child(&mut |c| walk(c, mo, idx, out));
    }
    let mut idx = 0;
    let mut out = HashMap::new();
    walk(node, mo, &mut idx, &mut out);
    out
}

/// The base container var of a slot expression: peel `OpGetField` / `OpGetVector` down to the
/// leading `Var`. `Var(v) → v`; a non-projection / non-Var expression → `None`.
fn base_var_of(expr: &Value, mo: &MoveOps) -> Option<u16> {
    match expr.unspan() {
        Value::Var(v) => Some(*v),
        Value::Call(d, args) if *d == mo.op_get_field || *d == mo.op_get_vector => {
            args.first().and_then(|a| base_var_of(a, mo))
        }
        _ => None,
    }
}

/// Pass 2 of [`move_elide`]: over each op list, DROP a ready source's `OpDatabase` /
/// `OpCopyRecord` / `OpFreeRef`, and RETARGET every remaining construction op that writes into
/// the source (`OpSet*(s, …)`) onto that source's captured destination expression.
fn move_rewrite(node: &mut Value, ready: &HashSet<u16>, dest: &HashMap<u16, Value>, mo: &MoveOps) {
    // `Value::unspan`'s obligation, in its mutable form.  Measured over the 858-program
    // corpus: 567 values arrive here, 41 of them spanned around an arm (33 `Call`, 8 `Block`).
    //
    // ⚠ Those 41 were NOT misses, and an earlier version of this comment claimed they were.
    // The `_` arm ends in `node.for_each_child_mut(…)`, and that walk DESCENDS THROUGH a
    // `Span`, so a spanned node was already reached one level down — measured
    // behaviour-identical across 120 corpus programs with and without this peel.  It is kept
    // because it obeys the rule and reaches the right arm directly; it fixes nothing.
    //
    // The `if let Value::Call(…) = node` below reads the ORIGINAL binding ON PURPOSE: peeling
    // it as well would retarget the same call twice, once here and once when the trailing walk
    // reaches it.  That is the double-count that peeling only the scrutinee produced in
    // `sandbox::intrinsic_space` — a bound inflated from `24 · n²` to `36 · n²`.
    match node.unspan_mut() {
        Value::Block(b) => {
            b.operators.retain(|s| !move_drop(s, ready, mo));
            for op in &mut b.operators {
                move_rewrite(op, ready, dest, mo);
            }
        }
        Value::Insert(ops) => {
            ops.retain(|s| !move_drop(s, ready, mo));
            for op in &mut *ops {
                move_rewrite(op, ready, dest, mo);
            }
        }
        _ => {
            if let Value::Call(d, args) = node {
                // Only a construction op (not one of the three dropped ops) whose target is a
                // ready source gets its target rewritten to the destination slot.
                let retarget = *d != mo.op_copy_record
                    && *d != mo.op_database
                    && *d != mo.op_free
                    && matches!(args.first().map(Value::unspan),
                        Some(Value::Var(s)) if ready.contains(s));
                if retarget
                    && let Some(Value::Var(s)) = args.first().map(Value::unspan)
                    && let Some(dst) = dest.get(s).cloned()
                {
                    args[0] = dst;
                }
            }
            node.for_each_child_mut(&mut |c| move_rewrite(c, ready, dest, mo));
        }
    }
}

/// Retain-predicate for [`move_rewrite`]: is `stmt` a dropped op (the source's alloc, the
/// `OpCopyRecord`, or the source's free) for a ready move source?
fn move_drop(stmt: &Value, ready: &HashSet<u16>, mo: &MoveOps) -> bool {
    if let Value::Call(d, args) = stmt.unspan()
        && (*d == mo.op_database || *d == mo.op_copy_record || *d == mo.op_free)
        && let Some(Value::Var(s)) = args.first().map(Value::unspan)
    {
        return ready.contains(s);
    }
    false
}

// The `op_` prefix is meaningful (operator def-numbers), so the same-prefix lint does not apply.
#[allow(clippy::struct_field_names)]
struct ConstructOps {
    op_database: u32,
    op_free: u32,
    op_append: u32,
    op_prealloc: u32,
    op_set_int4: u32,
    op_get_field: u32,
    op_clear: u32,
    op_new_record: u32,
    op_finish_record: u32,
}

/// @PLN90 phase B (B1.3b) — the CONSTRUCT copy shape (`x.field += src`, lowered as a copying
/// `OpAppendVector(x.field, src)`), restricted to the **reorder-free** case: the destination
/// container already exists when `src` is built. `src` is a vector view over its own backing
/// wrapper `vdb` (`src = OpGetField(vdb, …)`); instead of building `src`'s elements into `vdb`
/// then copying the whole vector into `x.field`, retarget `src`'s element-build ops
/// (`OpNewRecord`/`OpFinishRecord`) DIRECTLY onto `x.field`, and drop `vdb`'s alloc/init/free,
/// `src`'s view-def + capacity `OpPreAllocVector`, and the `OpAppendVector` copy. No new op — the
/// retargeted appends grow `x.field` exactly as the copy did.
///
/// **Reorder-free guard (the safety line):** fire only when `x`'s allocation precedes `vdb`'s
/// (or `x` is a parameter, i.e. never `OpDatabase`'d here). The fresh-construction case
/// (`a = Bag { items: base }` — `a` built AFTER `base`) needs a build-order reorder and is NOT
/// handled here; it is left as a copy. `skip` receives ONLY the backings of sources actually
/// rewritten, so a skipped source keeps its free (no live-store suppression).
fn construct_move_rewrite(
    code: &mut Value,
    con_sources: &HashSet<u16>,
    co: &ConstructOps,
    mo: &MoveOps,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) {
    // Pass 1 — pre-scan: per source capture the append destination, its backing wrapper, the
    // destination's container var, and the `OpDatabase` encounter order (for the reorder guard).
    let mut idx = 0usize;
    let mut db_order: HashMap<u16, usize> = HashMap::new();
    let mut dest: HashMap<u16, Value> = HashMap::new();
    let mut ambiguous: HashSet<u16> = HashSet::new();
    let mut vdb: HashMap<u16, u16> = HashMap::new();
    let mut container: HashMap<u16, Option<u16>> = HashMap::new();
    construct_prescan(
        code,
        co,
        con_sources,
        &mut idx,
        &mut db_order,
        &mut dest,
        &mut ambiguous,
        &mut vdb,
        &mut container,
    );

    // A destination TOUCHED between the source's build and the append cannot take the retarget:
    // the append would move ahead of that access.  `escaping` above guards the SOURCE being read
    // in between and this is its missing twin — measured, `seen = len(b.v); b.v += tmp` reported
    // the POST-append length, and `for x in c.v { t2 += [...] } c.v += t2` retargeted the loop's
    // appends onto the vector the loop was ITERATING and grew it without bound.
    let mut disturbed: HashSet<u16> = HashSet::new();
    collect_move_disturbed(code, mo, co.op_append, 1, &dest, &mut disturbed);

    // Ready = found a unique append destination + a backing wrapper, AND provably reorder-free.
    let ready: HashSet<u16> = con_sources
        .iter()
        .copied()
        .filter(|s| {
            !ambiguous.contains(s)
                && dest.contains_key(s)
                && vdb.contains_key(s)
                // A source READ between its build and the move (or grown by appends) can't be built
                // directly into the destination — leave it a copy.
                && !escaping.contains(s)
                && !disturbed.contains(s)
                // B1.3b handles a PURE append (`x.field += src`). If the field is CLEARED anywhere
                // it is a whole-vector REPLACE (`x.field = src`, an `OpClearVector` + append) — a
                // simple retarget would leave the clear stranded after the retargeted build (empties
                // the result), and the source may even READ the field (`s.v = s.v[1..]`). Leave the
                // replace to B1.3d (which moves the clear) or to a copy.
                && !field_is_cleared(code, &dest[s], co)
                && container.get(s).and_then(|c| *c).is_some_and(|cvar| {
                    // The container must PRE-EXIST when the source is built. A FRESH `OpNewRecord`
                    // element (`[Chunk { … }]` → `_elm_N.field += src`) has no `OpDatabase` but is
                    // NOT pre-existing — it is defined later, so retargeting there is use-before-def.
                    if bad_containers.contains(&cvar) {
                        return false;
                    }
                    // container built before the source's backing (both locals), or a real param.
                    match (db_order.get(&cvar), db_order.get(&vdb[s])) {
                        (Some(&c), Some(&v)) => c < v,
                        (None, Some(_)) => true, // container is a parameter (never allocated here)
                        _ => false,
                    }
                })
        })
        .collect();
    if ready.is_empty() {
        return;
    }
    let vdbs: HashSet<u16> = ready.iter().map(|s| vdb[s]).collect();

    // Pass 2 — retarget the element builds + drop the wrapper / view / prealloc / copy.
    construct_rewrite_ops(code, &ready, &vdbs, &dest, co);
    // The moved-out owned store is the backing wrapper (`src` is a borrow of it) — suppress ITS
    // free. Only ready sources' backings are added, so a skipped source keeps its free.
    skip.extend(vdbs);
}

/// The base var of a `OpGetField(Var(x), …)` destination expression (`x`), else `None`.
fn get_field_base(expr: &Value, co: &ConstructOps) -> Option<u16> {
    if let Value::Call(d, args) = expr.unspan()
        && *d == co.op_get_field
        && let Some(Value::Var(x)) = args.first().map(Value::unspan)
    {
        return Some(*x);
    }
    None
}

/// Pass 1 of [`construct_move_rewrite`]: gather the append destination / backing / container /
/// `OpDatabase` order for every construct source.
#[allow(clippy::too_many_arguments)]
fn construct_prescan(
    node: &Value,
    co: &ConstructOps,
    con: &HashSet<u16>,
    idx: &mut usize,
    db_order: &mut HashMap<u16, usize>,
    dest: &mut HashMap<u16, Value>,
    ambiguous: &mut HashSet<u16>,
    vdb: &mut HashMap<u16, u16>,
    container: &mut HashMap<u16, Option<u16>>,
) {
    match node.unspan() {
        Value::Call(d, args) => {
            if *d == co.op_database {
                if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                    db_order.entry(*v).or_insert(*idx);
                }
                *idx += 1;
            } else if *d == co.op_append
                && let Some(dst) = args.first()
                && let Some(Value::Var(s)) = args.get(1).map(Value::unspan)
                && con.contains(s)
            {
                if dest.contains_key(s) {
                    // ⚠ NOT idempotent, and this function double-visits a spanned node: the
                    // scrutinee above is peeled while `for_each_child` below walks the ORIGINAL,
                    // and that walk sees through a `Span` itself.  A second visit of the SAME
                    // append lands here and reads as two appends, which silently disqualifies the
                    // var from the construct rewrite.  Measured over the 858-program corpus: 77
                    // first-appends in 45 files and exactly ONE mark, which is genuine — it
                    // survives binding the peel.  So the hazard does not fire today; it is one
                    // edit away from firing.  See `sandbox::intrinsic_space` for the shape biting.
                    ambiguous.insert(*s); // appended into two places — not the clean shape.
                } else {
                    container.insert(*s, get_field_base(dst, co));
                    dest.insert(*s, dst.clone());
                }
            }
        }
        // `src = OpGetField(vdb, …)` — the source's view over its backing wrapper.
        Value::Set(s, rhs) if con.contains(s) => {
            if let Value::Call(gd, gargs) = rhs.unspan()
                && *gd == co.op_get_field
                && let Some(Value::Var(vd)) = gargs.first().map(Value::unspan)
            {
                vdb.insert(*s, *vd);
            }
        }
        _ => {}
    }
    node.for_each_child(&mut |c| {
        construct_prescan(c, co, con, idx, db_order, dest, ambiguous, vdb, container);
    });
}

/// Pass 2 of [`construct_move_rewrite`]: DROP the wrapper/view/prealloc/copy statements and
/// RETARGET each ready source's element-build ops (`OpNewRecord`/`OpFinishRecord`) onto its
/// append destination.
fn construct_rewrite_ops(
    node: &mut Value,
    ready: &HashSet<u16>,
    vdbs: &HashSet<u16>,
    dest: &HashMap<u16, Value>,
    co: &ConstructOps,
) {
    match node {
        Value::Block(b) => {
            b.operators.retain(|s| !construct_drop(s, ready, vdbs, co));
            for op in &mut b.operators {
                construct_rewrite_ops(op, ready, vdbs, dest, co);
            }
        }
        Value::Insert(ops) => {
            ops.retain(|s| !construct_drop(s, ready, vdbs, co));
            for op in &mut *ops {
                construct_rewrite_ops(op, ready, vdbs, dest, co);
            }
        }
        _ => {
            if let Value::Call(d, args) = node {
                // An element-build op (NOT one of the dropped/excluded ops) whose target is a
                // ready source → retarget it onto the append destination.
                let retarget = *d != co.op_database
                    && *d != co.op_prealloc
                    && *d != co.op_append
                    && *d != co.op_free
                    && *d != co.op_set_int4
                    && matches!(args.first().map(Value::unspan),
                        Some(Value::Var(s)) if ready.contains(s));
                if retarget
                    && let Some(Value::Var(s)) = args.first().map(Value::unspan)
                    && let Some(dst) = dest.get(s).cloned()
                {
                    args[0] = dst;
                }
            }
            node.for_each_child_mut(&mut |c| construct_rewrite_ops(c, ready, vdbs, dest, co));
        }
    }
}

/// Retain-predicate for [`construct_rewrite_ops`]: the wrapper's alloc/init/free, the source's
/// view-def + capacity pre-alloc, and the `OpAppendVector` copy are all dropped.
fn construct_drop(
    stmt: &Value,
    ready: &HashSet<u16>,
    vdbs: &HashSet<u16>,
    co: &ConstructOps,
) -> bool {
    match stmt.unspan() {
        // `src = OpGetField(vdb, …)` — the view-def is dead once the builds retarget.
        Value::Set(s, _) => ready.contains(s),
        Value::Call(d, args) => {
            let a0 = args.first().map(Value::unspan);
            if *d == co.op_database || *d == co.op_set_int4 || *d == co.op_free {
                matches!(a0, Some(Value::Var(v)) if vdbs.contains(v)) // wrapper alloc/len-init/free
            } else if *d == co.op_prealloc {
                matches!(a0, Some(Value::Var(s)) if ready.contains(s)) // src capacity hint — omit
            } else if *d == co.op_append {
                // the copy: OpAppendVector(dest, Var(src), …) — src in arg1.
                matches!(args.get(1).map(Value::unspan), Some(Value::Var(s)) if ready.contains(s))
            } else {
                false
            }
        }
        _ => false,
    }
}

/// @PLN90 phase B (B1.3c) — the FRESH-construction move-elision (`a = Bag { items: base }`, the
/// container built AFTER the source). Unlike B1.3b's field-append it needs a build-order REORDER:
/// hoist `a`'s allocation ahead of `base`'s build, retarget `base`'s build ops
/// (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord`) onto `a.field`, drop the backing wrapper +
/// the `OpAppendVector` copy. Runs AFTER [`construct_move_rewrite`], on the copies it left standing.
///
/// Conservative — operates on a FLAT top-level block only, and rewrites each construct that passes
/// EVERY guard: `a`'s construction is a contiguous run of statements immediately before the copy
/// that references only `a` or a **never-written parameter** (B1.4 — a param's value is constant,
/// so hoisting reads the same value; any other var could be a local built between `base` and `a`
/// → SKIP); the run contains `a`'s `OpDatabase`; and `a` is genuinely allocated AFTER the source's
/// backing (a real reorder). B1.4 also lifts the one-construct-per-fn cap — each safe construct is
/// rewritten independently (a cross-construct dependency is a non-`a`, non-param run var → SKIPs
/// that one). B1.4 (nested) walks EVERY block, so a construct inside an `if`/loop body is handled
/// too — its `a`-alloc + `base`-build + copy are flat within that block. Anything else stays a
/// copy. `skip` receives only the moved-out backings.
#[allow(clippy::too_many_arguments)]
fn construct_fresh_rewrite(
    code: &mut Value,
    con_sources: &HashSet<u16>,
    co: &ConstructOps,
    function: &Function,
    written: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) {
    // Apply the per-block reorder to the top block AND every nested block (if/loop bodies etc.).
    if let Value::Block(b) = code {
        fresh_rewrite_block(
            b,
            con_sources,
            co,
            function,
            written,
            bad_containers,
            escaping,
            skip,
        );
    }
    code.for_each_child_mut(&mut |c| {
        construct_fresh_rewrite(
            c,
            con_sources,
            co,
            function,
            written,
            bad_containers,
            escaping,
            skip,
        );
    });
}

/// Run the fresh-construction reorder over a SINGLE block's operators: rewrite each safe construct,
/// re-scanning after every rewrite (the reorder shifts indices), by earliest remaining copy (a
/// deterministic order); a guard-failing source is recorded so it is not retried (termination).
#[allow(clippy::too_many_arguments)]
fn fresh_rewrite_block(
    b: &mut Block,
    con_sources: &HashSet<u16>,
    co: &ConstructOps,
    function: &Function,
    written: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) {
    let mut failed: HashSet<u16> = HashSet::new();
    loop {
        let next = con_sources
            .iter()
            .copied()
            .filter(|s| !failed.contains(s))
            .filter_map(|s| {
                b.operators
                    .iter()
                    .position(|op| append_copy_of(op, s, co).is_some())
                    .map(|ci| (ci, s))
            })
            .min_by_key(|&(ci, _)| ci);
        let Some((_, src)) = next else {
            break;
        };
        if !try_fresh_one(
            b,
            src,
            co,
            function,
            written,
            bad_containers,
            escaping,
            skip,
        ) {
            failed.insert(src);
        }
    }
}

/// Attempt the fresh-construction reorder for ONE source `src` on the flat block `b`. Returns
/// `true` (and rewrites `b.operators` + records the moved-out backing in `skip`) iff every guard
/// holds; `false` otherwise, leaving `b` unchanged.
#[allow(clippy::too_many_arguments)]
fn try_fresh_one(
    b: &mut Block,
    src: u16,
    co: &ConstructOps,
    function: &Function,
    written: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) -> bool {
    if escaping.contains(&src) {
        return false; // read between build and copy (or append-grown) → not safe to retarget.
    }
    // Copy index + destination expression (`a.field`) — the first copy of `src`.
    let mut ci = None;
    let mut dest = None;
    for (i, op) in b.operators.iter().enumerate() {
        if let Some(d) = append_copy_of(op, src, co) {
            ci = Some(i);
            dest = Some(d);
            break;
        }
    }
    let (Some(ci), Some(dest)) = (ci, dest) else {
        return false;
    };
    let Some(a) = get_field_base(&dest, co) else {
        return false;
    };
    if bad_containers.contains(&a) {
        return false; // a REASSIGNED container has a prior store the hoist does not retire.
    }

    // The source's backing wrapper (`src = OpGetField(vdb, …)`).
    let mut vdb = None;
    for op in &b.operators {
        if let Value::Set(s, rhs) = op.unspan()
            && *s == src
            && let Value::Call(gd, gargs) = rhs.unspan()
            && *gd == co.op_get_field
            && let Some(Value::Var(v)) = gargs.first().map(Value::unspan)
        {
            vdb = Some(*v);
        }
    }
    let Some(vdb) = vdb else {
        return false;
    };

    // `a`'s construction run: the contiguous block [ps..ci) whose statements all target `a`.
    let mut ps = ci;
    while ps > 0 && stmt_targets_var(&b.operators[ps - 1], a) {
        ps -= 1;
    }
    let run = &b.operators[ps..ci];
    // Guards: the run allocates `a`, references only `a` or a never-written param (dependency-safe
    // to hoist past `base`), and `a` is allocated AFTER the backing (else this isn't a reorder).
    if !run.iter().any(|s| call_is(s, co.op_database, a)) {
        return false;
    }
    if !run.iter().all(|s| run_var_ok(s, a, function, written)) {
        return false;
    }
    match b
        .operators
        .iter()
        .position(|s| call_is(s, co.op_database, vdb))
    {
        Some(vi) if vi < ps => {}
        _ => return false,
    }

    // Rebuild: [a-construction run] ++ [base's build, wrapper dropped + retargeted] ++ [rest].
    let ops = std::mem::take(&mut b.operators);
    let mut new_ops = Vec::with_capacity(ops.len());
    for op in &ops[ps..ci] {
        new_ops.push(op.clone());
    }
    for (i, op) in ops.into_iter().enumerate() {
        if (ps..=ci).contains(&i) {
            continue; // run hoisted above; the copy at `ci` is dropped.
        }
        if fresh_drop(&op, src, vdb, co) {
            continue;
        }
        let mut op = op;
        fresh_retarget(&mut op, src, &dest, co);
        new_ops.push(op);
    }
    b.operators = new_ops;
    skip.insert(vdb); // the backing wrapper is the moved-out owned store.
    true
}

/// If `op` is `OpAppendVector(dest, Var(src), …)` (the copy of `src` into a field), return `dest`.
fn append_copy_of(op: &Value, src: u16, co: &ConstructOps) -> Option<Value> {
    if let Value::Call(d, args) = op.unspan()
        && *d == co.op_append
        && let Some(Value::Var(s)) = args.get(1).map(Value::unspan)
        && *s == src
    {
        return args.first().cloned();
    }
    None
}

/// Does `stmt` write into variable `v` (its `Set` target, or a `Call`'s first arg)?
fn stmt_targets_var(stmt: &Value, v: u16) -> bool {
    match stmt.unspan() {
        Value::Set(s, _) => *s == v,
        Value::Call(_, args) => {
            matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == v)
        }
        _ => false,
    }
}

/// Is `stmt` the call `op` with first arg `Var(v)` (e.g. `OpDatabase(v, …)`)?
fn call_is(stmt: &Value, op: u32, v: u16) -> bool {
    matches!(stmt.unspan(), Value::Call(d, args) if *d == op
        && matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == v))
}

/// Does every `Var` referenced anywhere in `stmt`'s value positions equal `only`? (A `Set`/`Call`
/// TARGET is `only` by construction; this checks the RHS/args carry no dependency on another var.)
/// Every `Var` in `stmt`'s subtree is either `a` or a never-written PARAMETER (whose value is
/// therefore constant, so hoisting `a`'s construction past `base`'s build reads the same value).
/// Any other var — a local that might be built between `base` and `a` — fails, so the construct
/// stays a copy. (`written` is the fn-wide over-approximation from [`collect_written`].)
fn run_var_ok(stmt: &Value, a: u16, function: &Function, written: &HashSet<u16>) -> bool {
    fn walk(node: &Value, a: u16, function: &Function, written: &HashSet<u16>, ok: &mut bool) {
        if let Value::Var(v) = node.unspan() {
            let v = *v;
            let allowed = v == a || (function.is_argument(v) && !written.contains(&v));
            if !allowed {
                *ok = false;
            }
        }
        node.for_each_child(&mut |c| walk(c, a, function, written, ok));
    }
    let mut ok = true;
    walk(stmt, a, function, written, &mut ok);
    ok
}

/// Retain-drop for [`construct_fresh_rewrite`]: the backing wrapper's alloc/len-init/free and the
/// source's view-def (`src = OpGetField(vdb, …)`).
fn fresh_drop(stmt: &Value, src: u16, vdb: u16, co: &ConstructOps) -> bool {
    match stmt.unspan() {
        Value::Set(s, _) => *s == src,
        Value::Call(d, args) => {
            (*d == co.op_database || *d == co.op_set_int4 || *d == co.op_free)
                && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == vdb)
        }
        _ => false,
    }
}

/// Retarget every source-targeting build op (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord` —
/// NOT the dropped alloc/append/free/set-int4/get-field ops) onto the `dest` field. Unlike the
/// field-append path, the capacity `OpPreAllocVector` IS retargeted here: the fresh field is empty,
/// so pre-claiming its capacity is correct (it mirrors the source's own initial claim).
fn fresh_retarget(node: &mut Value, src: u16, dest: &Value, co: &ConstructOps) {
    if let Value::Call(d, args) = node {
        let hit = *d != co.op_database
            && *d != co.op_append
            && *d != co.op_free
            && *d != co.op_set_int4
            && *d != co.op_get_field
            && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == src);
        if hit {
            args[0] = dest.clone();
        }
    }
    node.for_each_child_mut(&mut |c| fresh_retarget(c, src, dest, co));
}

/// @PLN90 phase B (B1.3d) — the `a.field = base` whole-vector REPLACEMENT, a DOUBLE copy the
/// compiler lowers as `base → __p154_rhs → a.field` with an `OpClearVector` between:
///
/// ```text
///   <build base into __vdb>
///   __p154_rhs = null; OpAppendVector(__p154_rhs, base);   (copy 1 — base → temp)
///   OpClearVector(a.field);                                (clear a.field's old contents)
///   OpAppendVector(a.field, __p154_rhs);                   (copy 2 — temp → a.field)
/// ```
///
/// `base`'s copy target is a temp (`__p154_rhs`), so it is not a `MovePlan`; this rewrite detects
/// the idiom STRUCTURALLY. When `base` is a dead-after local (its own `__vdb` backing), build it
/// DIRECTLY into the cleared `a.field`: move the `OpClearVector` ahead of `base`'s build, retarget
/// `base`'s build ops onto `a.field`, and drop the temp + both copies + the wrapper. Both temp and
/// wrapper join `skip` (their frees are suppressed). Walks every block (nested `if`/loop bodies too);
/// conservative.
fn construct_replace_rewrite(code: &mut Value, co: &ConstructOps, skip: &mut HashSet<u16>) {
    if let Value::Block(b) = code {
        replace_rewrite_block(b, co, skip);
    }
    code.for_each_child_mut(&mut |c| construct_replace_rewrite(c, co, skip));
}

/// Run the whole-vector-replacement rewrite over a SINGLE block's operators.
fn replace_rewrite_block(b: &mut Block, co: &ConstructOps, skip: &mut HashSet<u16>) {
    let mut failed: HashSet<usize> = HashSet::new();
    loop {
        // Find the earliest `copy 2` (`OpAppendVector(a.field, Var(rhs))`) preceded by the clear +
        // `copy 1`, not yet tried.
        let mut hit = None;
        for c2 in 2..b.operators.len() {
            if failed.contains(&c2) {
                continue;
            }
            let Some((field, rhs)) = append_field_temp(&b.operators[c2], co) else {
                continue;
            };
            // Preceding statement must be `OpClearVector(field)` for the SAME field.
            if !is_clear_of(&b.operators[c2 - 1], &field, co) {
                continue;
            }
            // And the one before that `OpAppendVector(Var(rhs), Var(base))`.
            let Some(base) = append_temp_src(&b.operators[c2 - 2], rhs, co) else {
                continue;
            };
            hit = Some((c2, field, rhs, base));
            break;
        }
        let Some((c2, field, rhs, base)) = hit else {
            break;
        };
        if !try_replace_one(b, c2, &field, rhs, base, co, skip) {
            failed.insert(c2);
        }
    }
}

/// One `a.field = base` replacement at `copy 2` index `c2`. Rewrites `b.operators` + records the
/// moved-out temp/wrapper in `skip` iff every guard holds; else returns `false` unchanged.
fn try_replace_one(
    b: &mut Block,
    c2: usize,
    field: &Value,
    rhs: u16,
    base: u16,
    co: &ConstructOps,
    skip: &mut HashSet<u16>,
) -> bool {
    let Some(a) = get_field_base(field, co) else {
        return false;
    };
    // `base` must be a dead-after LOCAL: it owns a backing wrapper `base = OpGetField(vdb, …)`, and
    // is not referenced after `copy 2` (the store transfers, so a later read would dangle).
    let mut vdb = None;
    let mut rhs_set = None;
    for (i, op) in b.operators.iter().enumerate() {
        match op.unspan() {
            Value::Set(s, r) if *s == base => {
                if let Value::Call(gd, ga) = r.unspan()
                    && *gd == co.op_get_field
                    && let Some(Value::Var(v)) = ga.first().map(Value::unspan)
                {
                    vdb = Some(*v);
                }
            }
            Value::Set(s, _) if *s == rhs => rhs_set = Some(i),
            _ => {}
        }
    }
    let (Some(vdb), Some(_)) = (vdb, rhs_set) else {
        return false;
    };
    if a == base || a == vdb {
        return false; // paranoia: the destination container must be independent of the source
    }
    if references_var_after(b, base, c2) {
        return false;
    }
    // `base`'s build start: the first statement that sets up its wrapper or references it. Everything
    // before it (`a`'s already-built value + null-inits) is kept; the clear is inserted there.
    let Some(bs) = b
        .operators
        .iter()
        .position(|op| fresh_drop(op, base, vdb, co) || refs_var(op, base))
    else {
        return false;
    };
    if bs >= c2 - 2 {
        return false; // base built after the field already exists but not as a real preceding build
    }
    // The container `a` must ALREADY EXIST at `base`'s build (we move the clear + build to `bs`, so
    // `a.field` must be valid there). If `a` is allocated in THIS block AFTER `base` (`fresh = …;
    // s = S{…}; s.field = fresh`), building into `a.field` at `bs` would hit an un-allocated `a` —
    // SKIP. A param / outer-scope `a` has no `OpDatabase` here → it pre-exists → fine.
    if let Some(a_db) = b
        .operators
        .iter()
        .position(|op| call_is(op, co.op_database, a))
        && a_db >= bs
    {
        return false;
    }
    let chain: HashSet<usize> = [rhs_set.unwrap(), c2 - 2, c2 - 1, c2].into_iter().collect();
    // `base`'s BUILD must not read the destination container `a` (a SELF-ASSIGN like
    // `s.v = s.v[1..]` builds the source by slicing `s.v` — moving the `OpClearVector` ahead of that
    // read would empty `s.v` before the slice copies it). Guard the build region [bs..c2) (excluding
    // the chain: copy1 / clear / copy2 legitimately reference `a`).
    if (bs..c2).any(|i| !chain.contains(&i) && refs_var(&b.operators[i], a)) {
        return false;
    }

    let clear = b.operators[c2 - 1].clone();
    let ops = std::mem::take(&mut b.operators);
    let mut new_ops = Vec::with_capacity(ops.len());
    for (i, op) in ops.into_iter().enumerate() {
        if i == bs {
            new_ops.push(clear.clone()); // clear a.field BEFORE building base into it
        }
        if i < bs {
            new_ops.push(op);
            continue;
        }
        if chain.contains(&i) || fresh_drop(&op, base, vdb, co) {
            continue;
        }
        let mut op = op;
        fresh_retarget(&mut op, base, field, co); // base's build → a.field
        new_ops.push(op);
    }
    b.operators = new_ops;
    skip.insert(vdb);
    skip.insert(rhs);
    true
}

/// If `op` is `OpAppendVector(OpGetField(…), Var(rhs))` (copy INTO a field from a temp), return
/// `(field_expr, rhs)`.
fn append_field_temp(op: &Value, co: &ConstructOps) -> Option<(Value, u16)> {
    if let Value::Call(d, args) = op.unspan()
        && *d == co.op_append
        && let Some(field) = args.first()
        && get_field_base(field, co).is_some()
        && let Some(Value::Var(rhs)) = args.get(1).map(Value::unspan)
    {
        return Some((field.clone(), *rhs));
    }
    None
}

/// Is `op` `OpClearVector(field)` for the given `field` expression?
fn is_clear_of(op: &Value, field: &Value, co: &ConstructOps) -> bool {
    matches!(op.unspan(), Value::Call(d, args) if *d == co.op_clear
        && args.first().map(Value::unspan) == Some(field.unspan()))
}

/// Is `field` cleared (`OpClearVector(field)`) ANYWHERE in `node`'s subtree? A cleared destination
/// means the append is really a whole-vector REPLACE, not a pure `+=` append.
fn field_is_cleared(node: &Value, field: &Value, co: &ConstructOps) -> bool {
    fn walk(node: &Value, field: &Value, co: &ConstructOps, found: &mut bool) {
        if is_clear_of(node, field, co) {
            *found = true;
        }
        node.for_each_child(&mut |c| walk(c, field, co, found));
    }
    let mut found = false;
    walk(node, field, co, &mut found);
    found
}

/// If `op` is `OpAppendVector(Var(rhs), Var(base))` (copy INTO the temp from `base`), return `base`.
fn append_temp_src(op: &Value, rhs: u16, co: &ConstructOps) -> Option<u16> {
    if let Value::Call(d, args) = op.unspan()
        && *d == co.op_append
        && matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == rhs)
        && let Some(Value::Var(base)) = args.get(1).map(Value::unspan)
    {
        return Some(*base);
    }
    None
}

/// Does `op` reference `Var(v)` anywhere in its subtree?
fn refs_var(op: &Value, v: u16) -> bool {
    fn walk(node: &Value, v: u16, found: &mut bool) {
        if matches!(node.unspan(), Value::Var(x) if *x == v) {
            *found = true;
        }
        node.for_each_child(&mut |c| walk(c, v, found));
    }
    let mut found = false;
    walk(op, v, &mut found);
    found
}

/// Is `Var(v)` referenced in any operator strictly after index `idx`?
fn references_var_after(b: &Block, v: u16, idx: usize) -> bool {
    b.operators[idx + 1..].iter().any(|op| refs_var(op, v))
}

/// @PLN94 TEST-ONLY: the var name whose scope-exit free `get_free_vars` drops (injecting a genuine
/// leak, the `check-leak` true-positive gate), or `None`. Cached — ONE env read per process, so the
/// production (unset) path pays nothing per-free. Never set outside tests.
fn inject_drop_free() -> Option<&'static str> {
    use std::sync::OnceLock;
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| std::env::var("LOFT_OWN_INJECT_DROP_FREE").ok())
        .as_deref()
}

/// @PLN94 TEST-ONLY: the BORROWED var name whose scope-exit free `get_free_vars` is forced to emit
/// (injecting a genuine OVER-free — an unconditional `OpFreeRef` of a dep-carrying view), the
/// over-free-check (`run_over_free_check`) true-positive gate, or `None`. Cached like
/// [`inject_drop_free`]; the production (unset) path pays nothing. Never set outside tests.
fn inject_free_borrowed() -> Option<&'static str> {
    use std::sync::OnceLock;
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| std::env::var("LOFT_OWN_INJECT_FREE_BORROWED").ok())
        .as_deref()
}

/// @PLN101 — ISOLATED value-struct copy pass. A `value struct` is an ordinary
/// `Type::Reference` record (marked only by `Data.value_structs`); the ONLY behavioural
/// difference is value (copy) semantics. When such a struct is BOUND to a local from a VIEW
/// (a field/element read — `e = record.f` / `e = vec[i]`), rewrite the bind to mint a fresh
/// store and deep-copy into it, so the local owns its own record and mutating it cannot write
/// back through the view. Not wired into the type system: no `Type` variant, no assignment-path
/// surgery, no ownership-oracle change. Runs after `move_elide` and BEFORE the ownership scan,
/// so the emitted `OpDatabase` makes the local classify `Owned` on its own (sound: it earns
/// ownership, it is not asserted). Only fires on a `Set(local, borrowed-view)`; a fresh
/// construction (`Owned` rhs) or a method-call arg is not rewritten (copy-elision / zero-copy
/// hot path fall out).
fn value_struct_copy(data: &mut Data) {
    // No value structs in the program → the pass is a no-op (and the read-only-elision oracles
    // below need not run for any function).
    if data.value_structs.is_empty() {
        return;
    }
    let op_database = data.def_nr("OpDatabase");
    let op_copy_record = data.def_nr("OpCopyRecord");
    if op_database == u32::MAX || op_copy_record == u32::MAX {
        return;
    }
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let mut code = data.definitions[d_nr as usize].code.clone();
        // Read-only-elision oracle (function scope; rescoped per loop body inside the walk): the
        // TAINTED set — variables whose backing may be mutated, or which escape, during a view's
        // lifetime. Seeded from field/element writes (`find_field_written_vars`, catches nested
        // `v.a.b = …`) plus escapes (return / passed to a user fn), then closed over pure-`Var`
        // alias edges so mutating one co-alias taints the shared backing. A value-struct view-bind
        // is left as a zero-cost view iff neither the local nor any base variable is tainted.
        let tainted = vs_scope_taint(std::slice::from_ref(&code), data);
        let mut cleared: Vec<u16> = Vec::new();
        vs_copy_walk(
            &mut code,
            data,
            d_nr,
            op_database,
            op_copy_record,
            &tainted,
            &mut cleared,
        );
        if cleared.is_empty() {
            continue;
        }
        data.definitions[d_nr as usize].code = code;
        // Clear each copied local's VIEW dep so it frees as an owned store (no leak, no
        // double-free): the local now owns a fresh copy, not a borrow into the source.
        for v in cleared {
            if let Type::Reference(p, _) = *data.def(d_nr).variables.tp(v) {
                data.definitions[d_nr as usize]
                    .variables
                    .set_type(v, Type::Reference(p, Deps::none()));
            }
        }
    }
}

/// @PLN101 zero-cost — root variable a VIEW projects from: follow the arg-0 spine of read ops
/// (`OpGet*`) down to a `Var`. Index/offset args are ignored (a mutated loop counter is not a base
/// mutation). `None` if the spine doesn't bottom out in a plain variable.
fn vs_view_root(node: &Value, data: &Data) -> Option<u16> {
    match node.unspan() {
        Value::Var(v) => Some(*v),
        Value::Call(fn_nr, args)
            if !args.is_empty() && data.def(*fn_nr).name().starts_with("OpGet") =>
        {
            vs_view_root(&args[0], data)
        }
        // A view can be wrapped in a block that yields its tail value — notably a for-loop's
        // `{#iter next}` (advance the index, then project the element). Follow the tail.
        Value::Block(b) => b.operators.last().and_then(|t| vs_view_root(t, data)),
        Value::Insert(ops) => ops.last().and_then(|t| vs_view_root(t, data)),
        _ => None,
    }
}

/// First rhs of `Set(var, rhs)` for `var` (its binding), for one-hop base-alias tracing.
fn vs_find_binding(code: &Value, var: u16) -> Option<&Value> {
    fn walk<'a>(node: &'a Value, var: u16, found: &mut Option<&'a Value>) {
        if found.is_some() {
            return;
        }
        match node {
            Value::Set(v, body) => {
                if *v == var {
                    *found = Some(body);
                } else {
                    walk(body, var, found);
                }
            }
            Value::Block(b) | Value::Loop(b) => {
                for op in &b.operators {
                    walk(op, var, found);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    walk(op, var, found);
                }
            }
            Value::If(c, t, e) => {
                walk(c, var, found);
                walk(t, var, found);
                walk(e, var, found);
            }
            Value::Return(x) | Value::Drop(x) => walk(x, var, found),
            Value::Call(_, args) => {
                for a in args {
                    walk(a, var, found);
                }
            }
            Value::Iter(_, a, b, c) => {
                walk(a, var, found);
                walk(b, var, found);
                walk(c, var, found);
            }
            Value::Span(b) => walk(&b.1, var, found),
            _ => {}
        }
    }
    let mut found = None;
    walk(code, var, &mut found);
    found
}

/// The variables whose backing a value-struct view reads from: the spine root plus, one hop at a
/// time, the root of whatever each is bound from (`_vector_1 = b.items` → also `b`), so an alias of
/// a mutated source is caught. Bounded to a few hops (straight-line binds, no cycles).
fn vs_base_vars(rhs: &Value, data: &Data, code: &Value, out: &mut HashSet<u16>) {
    let mut work: Vec<u16> = Vec::new();
    if let Some(r) = vs_view_root(rhs, data) {
        work.push(r);
    }
    let mut hops = 0;
    while let Some(v) = work.pop() {
        if !out.insert(v) {
            continue;
        }
        hops += 1;
        if hops > 8 {
            break;
        }
        if let Some(bind) = vs_find_binding(code, v)
            && let Some(r) = vs_view_root(bind, data)
        {
            work.push(r);
        }
    }
}

/// Walk the function collecting the TAINT seed + pure-`Var` alias edges for read-only elision.
/// A variable is a taint SOURCE if it ESCAPES — returned, or handed as a bare `Var` argument to a
/// user function (non-`Op`), which could store or mutate it. (Field/element mutations are folded in
/// separately from `find_field_written_vars`; a store like `coll.push(p)` copies `p` and so does not
/// taint it.) An `w = x` bind adds a symmetric alias EDGE so that mutating one co-alias taints the
/// backing both share. The caller closes `seed` over `edges` to get the full tainted set.
fn vs_collect_taint(
    node: &Value,
    data: &Data,
    seed: &mut HashSet<u16>,
    edges: &mut Vec<(u16, u16)>,
) {
    match node {
        Value::Set(w, body) => {
            if let Value::Var(x) = body.unspan() {
                edges.push((*w, *x)); // pure alias `w = x` — taint flows both ways
            }
            vs_collect_taint(body, data, seed, edges);
        }
        Value::Return(x) | Value::Drop(x) => {
            if let Value::Var(v) = x.unspan() {
                seed.insert(*v); // escape via return
            }
            vs_collect_taint(x, data, seed, edges);
        }
        Value::Call(fn_nr, args) => {
            let is_op = data.def(*fn_nr).name().starts_with("Op");
            for a in args {
                // A bare `Var` passed to a USER function may be stored or mutated by the callee.
                if !is_op && let Value::Var(v) = a.unspan() {
                    seed.insert(*v);
                }
                vs_collect_taint(a, data, seed, edges);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for op in &b.operators {
                vs_collect_taint(op, data, seed, edges);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                vs_collect_taint(op, data, seed, edges);
            }
        }
        Value::If(c, t, e) => {
            vs_collect_taint(c, data, seed, edges);
            vs_collect_taint(t, data, seed, edges);
            vs_collect_taint(e, data, seed, edges);
        }
        Value::Iter(_, a, b, c) => {
            vs_collect_taint(a, data, seed, edges);
            vs_collect_taint(b, data, seed, edges);
            vs_collect_taint(c, data, seed, edges);
        }
        Value::Span(b) => vs_collect_taint(&b.1, data, seed, edges),
        _ => {}
    }
}

/// Close `seed` under the symmetric alias `edges` — the set of variables whose backing may change
/// or escape during a view's lifetime. A value-struct view-bind is elidable iff neither the local
/// nor any base variable is in this set.
fn vs_tainted(seed: HashSet<u16>, edges: &[(u16, u16)]) -> HashSet<u16> {
    let mut adj: HashMap<u16, Vec<u16>> = HashMap::new();
    for &(a, b) in edges {
        adj.entry(a).or_default().push(b);
        adj.entry(b).or_default().push(a);
    }
    let mut tainted = seed;
    let mut work: Vec<u16> = tainted.iter().copied().collect();
    while let Some(v) = work.pop() {
        if let Some(ns) = adj.get(&v) {
            for &n in ns {
                if tainted.insert(n) {
                    work.push(n);
                }
            }
        }
    }
    tainted
}

/// The tainted set over a straight-line SCOPE (a function body, or a loop body). Scoping to the
/// enclosing loop is what makes a `for p in ps { …read p… }` bind elidable: BUILDING `ps` field-
/// writes it, but that construction runs BEFORE the loop, so it cannot diverge a view read INSIDE
/// the loop from a copy. Only a mutation/escape WITHIN the loop (which repeats) can — S1's
/// `b.items[0].x = 99` in the body still taints `b`.
fn vs_scope_taint(ops: &[Value], data: &Data) -> HashSet<u16> {
    let mut seed: HashSet<u16> = HashSet::new();
    let mut edges: Vec<(u16, u16)> = Vec::new();
    for op in ops {
        crate::parser::find_field_written_vars(op, data, &mut seed);
        vs_collect_taint(op, data, &mut seed, &mut edges);
    }
    vs_tainted(seed, &edges)
}

/// Recursive rewriter for [`value_struct_copy`] — mirrors the IR walk in
/// `variables/validate.rs::build_scope_parents`. `tainted` is the function-wide read-only-elision
/// oracle (computed once by the caller): a value-struct view-bind is left as a zero-cost view (like
/// a reference struct) when neither the local nor any base variable is tainted; otherwise the copy
/// is emitted for value semantics.
fn vs_copy_walk(
    node: &mut Value,
    data: &Data,
    d_nr: u32,
    op_database: u32,
    op_copy_record: u32,
    tainted: &HashSet<u16>,
    cleared: &mut Vec<u16>,
) {
    match node {
        Value::Set(v, rhs) => {
            let vv = *v;
            vs_copy_walk(
                rhs,
                data,
                d_nr,
                op_database,
                op_copy_record,
                tainted,
                cleared,
            );
            if let Type::Reference(p, _) = *data.def(d_nr).variables.tp(vv)
                && data.is_value_struct(p)
                && matches!(
                    crate::use_analysis::ownership_of(data, d_nr, rhs),
                    crate::use_analysis::Own::Borrowed { .. }
                )
            {
                // Zero-cost read-only elision: if the local and its whole projection base are only
                // ever read (never field-written, never escaping), a plain view is observably
                // identical to a copy — so skip the copy and keep the reference-struct-cheap view.
                let mut affected: HashSet<u16> = HashSet::new();
                affected.insert(vv);
                vs_base_vars(rhs, data, &data.def(d_nr).code, &mut affected);
                let needs_copy = affected.iter().any(|x| tainted.contains(x));
                if !needs_copy {
                    return;
                }
                let kt = i32::from(data.def(p).known_type());
                let source = (**rhs).clone();
                *node = Value::Insert(vec![
                    Value::Set(vv, Box::new(Value::Null)),
                    Value::Call(op_database, vec![Value::Var(vv), Value::Int(kt)]),
                    Value::Call(op_copy_record, vec![source, Value::Var(vv), Value::Int(kt)]),
                ]);
                cleared.push(vv);
            }
        }
        Value::Block(b) => {
            for op in &mut b.operators {
                vs_copy_walk(
                    op,
                    data,
                    d_nr,
                    op_database,
                    op_copy_record,
                    tainted,
                    cleared,
                );
            }
        }
        Value::Loop(b) => {
            // Rescope taint to this loop body — pre-loop construction of a base does not diverge a
            // view read inside the loop, only an in-body mutation/escape does.
            let loop_taint = vs_scope_taint(&b.operators, data);
            for op in &mut b.operators {
                vs_copy_walk(
                    op,
                    data,
                    d_nr,
                    op_database,
                    op_copy_record,
                    &loop_taint,
                    cleared,
                );
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                vs_copy_walk(
                    op,
                    data,
                    d_nr,
                    op_database,
                    op_copy_record,
                    tainted,
                    cleared,
                );
            }
        }
        Value::If(c, t, e) => {
            vs_copy_walk(c, data, d_nr, op_database, op_copy_record, tainted, cleared);
            vs_copy_walk(t, data, d_nr, op_database, op_copy_record, tainted, cleared);
            vs_copy_walk(e, data, d_nr, op_database, op_copy_record, tainted, cleared);
        }
        Value::Return(x) | Value::Drop(x) => {
            vs_copy_walk(x, data, d_nr, op_database, op_copy_record, tainted, cleared);
        }
        Value::Call(_, args) => {
            for a in args {
                vs_copy_walk(a, data, d_nr, op_database, op_copy_record, tainted, cleared);
            }
        }
        Value::Iter(_, a, b, c) => {
            vs_copy_walk(a, data, d_nr, op_database, op_copy_record, tainted, cleared);
            vs_copy_walk(b, data, d_nr, op_database, op_copy_record, tainted, cleared);
            vs_copy_walk(c, data, d_nr, op_database, op_copy_record, tainted, cleared);
        }
        Value::Span(b) => {
            vs_copy_walk(
                &mut b.1,
                data,
                d_nr,
                op_database,
                op_copy_record,
                tainted,
                cleared,
            );
        }
        _ => {}
    }
}

/// Scope / lifetime analysis pass over every function definition.
///
/// # Panics
/// Under the `LASTUSE_RECLAIM` gate only (a Plan-57 testing build), panics if the
/// reclaim pass left a store the model says is dead un-freed past a later
/// allocation (the Phase-4 Goal-E watermark guard).  Never panics in normal builds.
pub fn check(data: &mut Data) {
    // @PLN94 — the CFG/dataflow completeness oracle, an OBSERVER reached only via
    // LOFT_OWN_ORACLE (SI-1: shipped codegen byte-identical; a no-op when unset).
    crate::ownership_cfg::oracle(data);
    // Behaviour-neutral USE-analysis dump (LOFT_MATERIALIZE_DUMP) — the
    // copy-vs-borrow verdict per binding, before any codegen consumes it.
    crate::use_analysis::dump_all(data);
    // @PLN90 Step 5 — the user-facing copy report (`--report-copies`) is emitted ONCE from
    // main after the whole program is loaded (not here — `check` runs per file-load).
    // @PLN90 phase B B1.2 — dump the last-use MOVE-elision plans (LOFT_MOVE_ELIDE). Detection
    // only; no lowering consumes them yet, so this is behaviour-neutral.
    crate::use_analysis::dump_move_plans(data);
    // Tier-0 borrow elision (DEFAULT ON; opt-out LOFT_NO_BORROW_ELIDE). Inlines
    // Borrow-verdict vector copies before the scope/free passes. `elide_borrows`
    // refuses to elide a `v` that another var borrows (its `deps` point at `v`),
    // which is the dogfood-found dangling-dep hazard. The copy mechanism stays the
    // substrate; the opt-out forces the always-correct copy (the A-B lever).
    if std::env::var_os("LOFT_NO_BORROW_ELIDE").is_none() {
        elide_borrows(data);
    }
    // @PLN90 phase B (B1.3) — last-use MOVE-elision: build a dead-after owned source directly
    // into its destination slot instead of copy-then-free. Gated on `LOFT_MOVE_ELIDE`; a no-op
    // off (byte-identical). Runs AFTER borrow elision — the two cover disjoint verdicts (borrow
    // vs owned copy), so ordering is safe, but move-elision assumes the copy still stands.
    move_elide(data);
    // @PLN101 — insert value-struct copies (BEFORE the ownership scan, so the emitted
    // OpDatabase makes each copied local classify Owned on its own).
    value_struct_copy(data);
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
        // @PLN35 — the `..rest` store-lifetime OBSERVER (reporting only; no IR change).
        if std::env::var("LOFT_REST_ORACLE").is_ok() {
            rest_store_oracle(
                &data.definitions[d_nr as usize].code,
                &data.definitions[d_nr as usize].variables,
                free_ref_nr,
                data.def_nr("OpDatabase"),
                data.def(d_nr).name(),
            );
        }
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
        let create_stack_nr = data.def_nr("OpCreateStack");
        let mut seq = 0u32;
        compute_intervals(
            &code_ref,
            &mut data.definitions[d_nr as usize].variables,
            free_text_nr,
            free_ref_nr,
            create_stack_nr,
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
    // @PLN94 C.0 (DEV tier) — the POST-codegen free-based checks (over-free / under-free), now that
    // `get_free_vars` has inserted the frees into `def.code` above. Self-gates on
    // `LOFT_OWN_ORACLE=check-dev`; observer only (SI-1), a no-op on the default `check` path.
    crate::ownership_cfg::oracle_free_checks(data);
    // #682 — record which closure captures the record ADOPTS, now that every dep
    // rewrite above has settled.  Must run after the loop, not inside it: the
    // verdict is the same `owns` fact `get_free_vars` uses, and that fact is only
    // final once the call-result rewrites (`make_independent`) have run.
    mark_borrowed_captures(data);
    // `LOFT_VAR_TABLE=<fn substring>` — the variable table beside the IR dump, with
    // each type dep resolved to `name(index)`.  Observer only; a no-op when unset.
    crate::variables::dump_var_tables(data, 0);
}

/// #682 — decide, per closure-record capture, whether the record ADOPTS the
/// captured store (`free_named`'s cascade reclaims it) or merely BORROWS it, and
/// mark the borrowed ones on the record's attribute.
///
/// Adoption exists for exactly one reason: a store the DEFINING frame owned and
/// would otherwise free at scope exit.  `get_free_vars` suppresses that free for
/// a captured reference and hands the store to the record, which is what keeps an
/// escaping factory closure's capture alive past the frame that minted it (#323).
/// A capture with no such free to hand over is a BORROW — a PARAMETER (whose
/// caller owns the store and outlives this frame) or a projection local viewing
/// into someone else's store — and cascading there is a second free of a live
/// store: the caller's value went dangling and the crash surfaced thousands of
/// ops later in whatever function next touched it.
///
/// Why here and not at record synthesis: a capture's ownership is not knowable
/// while parsing.  `ch = pick(w, 1)` parses as "borrows `w`" from the callee's
/// declared return, and only the scan above rewrites it to OWNED once it knows
/// the return ABI deep-copies into a fresh store (`make_independent`, the
/// `!adopts_fresh_store` arm).  Reading the parse-time dep leaked that copy.
///
/// The record is reached through the enclosing frame's `___clos_N` variable
/// rather than by walking the IR: `emit_lambda_code` always mints one, and it
/// lives in exactly the frame whose variables decide the verdict.
fn mark_borrowed_captures(data: &mut Data) {
    let mut borrowed: Vec<(u32, usize)> = Vec::new();
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let function = &data.def(d_nr).variables;
        for v in 0..function.next_var() {
            // The DEFINING frame holds the record in the `___clos_N` local
            // `emit_lambda_code` mints for it.  A frame that receives the record as
            // an ARGUMENT is the closure BODY (its hidden `__closure` parameter),
            // whose own variable table knows nothing about who owns the captures —
            // reading it flipped the verdict depending on definition order.
            if function.is_argument(v) {
                continue;
            }
            let Type::Reference(record, _) = function.tp(v) else {
                continue;
            };
            let record = *record;
            if !data.def(record).name.starts_with("__closure_") {
                continue;
            }
            for a in 0..data.attributes(record) {
                // Only the two share markers are cascade-relevant; an
                // inline-bytes capture (empty deps, e.g. a `text` copy) holds no
                // DbRef for the cascade to follow.
                if !matches!(data.attr_type(record, a), Type::Reference(_, ref deps) if !deps.is_empty())
                {
                    continue;
                }
                if !record_adopts_capture(data, function, record, a) {
                    borrowed.push((record, a));
                }
            }
        }
    }
    for (record, a) in borrowed {
        data.mark_capture_borrowed(record, a);
    }
}

/// Does the closure record own the store behind capture `a` of `record`, as seen
/// from the defining frame `function`?  See [`mark_borrowed_captures`].
fn record_adopts_capture(data: &Data, function: &Function, record: u32, a: usize) -> bool {
    // A `__cell_<T>` is minted FOR this closure (plan-22 boxes a mutated scalar /
    // text capture into one), so the record is its only possible owner however the
    // original binding was reached — including from a parameter.
    if let Type::Reference(cell, _) = data.attr_type(record, a)
        && data.def(cell).name.starts_with("__cell_")
    {
        return true;
    }
    let v = function.var(&data.attr_name(record, a));
    // An unresolvable name defaults to BORROW: an unfreed store is a leak the
    // store checker reports, while an extra free silently corrupts a caller.
    // A parameter never enters the scope-exit sweep at all (`variables()`:
    // "never return function arguments"), so it has no free to hand over.
    if v == u16::MAX || function.is_argument(v) {
        return false;
    }
    // The same test as `get_free_vars`' `owns`, so the two cannot drift: empty
    // deps means owned, and a keyed collection's self-dep is an ownership marker
    // rather than a borrow (@P302).
    let tp = function.tp(v);
    let dep = tp.depend();
    dep.is_empty() || (dep.len() == 1 && dep[0] == v && crate::parser::vectors::is_keyed(tp))
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
            // Every value-WRAPPING node, not a chosen subset: the shape this walker
            // reports — a `Set(v, Null)` nested inside a call argument — can sit under any
            // of them, so one omitted wrapper is a silently missed diagnostic.
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                walk_check(inner, function, fn_name);
            }
            _ => {}
        }
    }
    walk_check(ir, function, fn_name);
}

impl Scopes {
    /// The type's scope-end hook for `v`, unless a MOVE-copy already released `v`'s
    /// store — see [`collect_drop_transferred`].  One home for the rule, because
    /// both emission sites (the buffer-adoption leg and the ordinary one) must agree:
    /// a drop that runs on a released store is a use-after-free either way.
    fn scope_end_drop(&self, function: &Function, v: u16, data: &Data) -> Option<Value> {
        if self.drop_transferred.contains(&v) {
            return None;
        }
        drop_hook(function, v, data)
    }

    /// Record what the `__lift_N` temp `tmp` — holding argument `arg_idx` of the call
    /// `scan_args` is lowering — no longer owns, because the call takes it over.
    ///
    /// Two different hand-offs, and they cost different things to get wrong:
    ///
    /// - the **drop** (@PLN139 stage C): a copy into a container field or a collection
    ///   element makes the container the releaser, so running the source's own scope-end
    ///   hook releases one resource twice.
    /// - the **store** (loft#890): a `0x8000` move FREES the source store inside the op.
    ///   The lift still names it, so its scope-exit `OpFreeRef` is a second free — silent
    ///   while the slot stays free, and a stolen store the moment the allocator hands
    ///   that slot to somebody else.  `br = mk_hash(n); br[7, 0]` in a record-returning
    ///   function is exactly that: the return buffer is allocated between the two frees
    ///   and lands on the recycled slot, so the function returns freed bytes.
    fn mark_lift_handoff(
        &mut self,
        tmp: u16,
        arg_idx: usize,
        transfer_copy: bool,
        moved_arg: Option<usize>,
    ) {
        if transfer_copy && arg_idx == 0 || moved_arg == Some(arg_idx) {
            self.drop_transferred.insert(tmp);
        }
        if moved_arg == Some(arg_idx) {
            self.free_transferred.insert(tmp);
        }
    }

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
                let outer_scope = self.scope;
                let lift_watermark = self.lift_vars.len();
                let scope = self.enter_scope();
                // Move hoisted var from outer scope (0) to body scope so
                // get_free_vars at body exit can find and free it.
                if let Some(w) = hoisted_ref {
                    self.var_scope.insert(w, scope);
                }
                let mut ls = self.convert(bl, function, data, is_body_return);
                self.exit_scope();
                // loft#722 — a lift temp minted INSIDE a value-producing block must
                // outlive the block, because the block's result can be a borrow INTO
                // it.  `x = f().items[0] ?? Fallback {}` lowers the `??` to a block;
                // the temp holding `f()`'s result was registered at that block's
                // scope and freed on the way out, while `x` — a binding in the
                // ENCLOSING scope — still pointed into it.  It read correctly once
                // and returned zeroes after the store was reused.
                //
                // Without the `??` the same expression is lifted at statement level
                // and is correct, which is exactly the behaviour restored here:
                // re-register at the ENCLOSING scope and drop the block-exit free,
                // so the outer scope frees it instead.
                //
                // The enclosing scope, not the function: a lift inside a LOOP body
                // then still frees once per iteration, which is what keeps a loop
                // over such an expression flat instead of accumulating a store per
                // round.
                // Not for a BODY return.  The hoist hands the temp to the enclosing
                // scope and drops the block-exit free on the promise that the outer
                // scope frees it instead — and when the value block IS the function
                // body, the enclosing scope is that same function, so the free it was
                // handed to is the one just dropped and nothing frees it at all.
                // `fn txt(n: integer) -> text { mk(n).label }` leaked one record per
                // call for exactly that reason.  A body return also does not need the
                // hoist: the return delivers its value into the CALLER's buffer, so
                // freeing the temp at body exit — the behaviour before loft#722 — is
                // both correct and what the caller's copy relies on.
                if !matches!(bl.result, Type::Void)
                    && !is_body_return
                    && self.lift_vars.len() > lift_watermark
                {
                    let fresh: Vec<u16> = self.lift_vars[lift_watermark..]
                        .iter()
                        .copied()
                        .filter(|v| self.var_scope.get(v) == Some(&scope))
                        .collect();
                    // Only the temps the RESULT actually borrows from.  Hoisting
                    // every lift in a value block moves frees that nothing was
                    // waiting on, and those then went missing entirely (27 leaked
                    // `File` stores in the file suite) — the block, not the outer
                    // scope, is the right owner when the result does not point into
                    // the temp.
                    let borrowed = Self::result_borrow_roots(&ls, data);
                    let hoisted: Vec<u16> =
                        fresh.into_iter().filter(|v| borrowed.contains(v)).collect();
                    for v in &hoisted {
                        self.var_scope.insert(*v, outer_scope);
                    }
                    if !hoisted.is_empty() {
                        ls.retain(|op| {
                            scope_free_op_var(op, data).is_none_or(|v| !hoisted.contains(&v))
                        });
                    }
                }
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
                let (preamble, ls, postamble) = self.scan_args(args, function, data, *d_nr);
                let call = Value::Call(*d_nr, ls);
                if preamble.is_empty() && postamble.is_empty() {
                    call
                } else if postamble.is_empty() {
                    let mut ops = preamble;
                    ops.push(call);
                    Value::Insert(ops)
                } else {
                    // @PLN90 / loft#506 — run the store-back postamble AFTER the call.  For a
                    // non-void call, capture the result into a temp so the Insert still yields
                    // the CALL's value; a scalar must NOT be inline-ref (freeing a scalar-as-ref
                    // corrupts the store), so use a plain slotted temp.
                    let mut ops = preamble;
                    let ret = data.def(*d_nr).returned.clone();
                    if ret == Type::Void {
                        ops.push(call);
                        ops.extend(postamble);
                    } else {
                        let is_scalar = matches!(
                            ret,
                            Type::Integer(..)
                                | Type::Float
                                | Type::Single
                                | Type::Boolean
                                | Type::Character
                        );
                        let rtmp = if is_scalar {
                            self.lift_counter += 1;
                            let name = format!("__wbret_{}", self.lift_counter);
                            let t = function.add_temp_var(&name, &ret);
                            self.var_scope.insert(t, self.scope);
                            t
                        } else {
                            self.new_lift_var(function, &ret)
                        };
                        ops.push(v_set(rtmp, call));
                        ops.extend(postamble);
                        ops.push(Value::Var(rtmp));
                    }
                    Value::Insert(ops)
                }
            }
            Value::CallRef(v_nr, args) => {
                let (preamble, ls, _postamble) = self.scan_args(args, function, data, u32::MAX);
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
            Value::Drop(inner) => {
                let scanned = self.scan(inner, function, data);
                // #490 — a discarded statement result that owns a fresh store
                // (`json_parse(x);`, `mk();`) lowers to a plain stack-pop
                // (`FreeStack` on the interpreter, a dropped Rust return value
                // on native), which never frees the store — once per iteration
                // inside a loop.  Bind it to a `__lift_N` temp instead, so
                // `get_free_vars` emits the store's `OpFreeRef` at scope exit —
                // the same machinery `scan_args` uses for owned call-argument
                // temps.  A `Set` consumes the value, so the `Drop` wrapper is
                // dropped with it.
                if let Value::Insert(mut ops) = scanned {
                    // A call whose arguments were themselves lifted arrives as
                    // `Insert([Set(__lift_i, …)…, call])` — the owned result is
                    // the final op.
                    if let Some(last) = ops.last()
                        && let Some(tp) = self.inline_struct_return(last, data, u32::MAX, function)
                    {
                        let tmp = self.new_lift_var(function, &tp);
                        let last = ops.pop().unwrap();
                        ops.push(v_set(tmp, last));
                        Value::Insert(ops)
                    } else {
                        Value::Drop(Box::new(Value::Insert(ops)))
                    }
                } else if let Some(tp) =
                    self.inline_struct_return(&scanned, data, u32::MAX, function)
                {
                    let tmp = self.new_lift_var(function, &tp);
                    v_set(tmp, scanned)
                } else {
                    Value::Drop(Box::new(scanned))
                }
            }
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
                //
                // The A5.6 null-init preamble ([`Self::is_null_init_preamble`]) is the
                // second shape that must survive to `scan_args`, and it is just as
                // narrow: exactly two ops, led by an owned-heap `Set(v, Null)`.
                let is_lift_preamble = matches!(&scanned, Value::Insert(ops)
                    if ops.first().is_some_and(|op| matches!(op,
                        Value::Set(v, _) if function.name(*v).starts_with("__lift_")))
                        || Self::is_null_init_preamble(ops, function));
                if is_lift_preamble {
                    scanned
                } else {
                    Value::with_span(b.0.clone(), scanned)
                }
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
            && !crate::parser::vectors::is_keyed(function.tp(v))
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
        // The same transition for a TUPLE: reassigning a tuple local installs new
        // elements over the old ones, and without this the previous turn's owned
        // element stores are named by nobody.  Invisible until a call-site return
        // buffer stopped being pre-allocated by the caller (loft#1085): before that
        // the callee ADOPTED the caller's `__ref_N` store, so the caller's own
        // scope-exit free covered every iteration's element and one store served
        // the whole loop.  A first-iteration free is a no-op — the entry null-init
        // leaves the elements at the sentinel, which `free` ignores.
        if was_in_scope
            && let Type::Tuple(elems) = function.tp(v)
            && !value.reads_var(v)
            && !value.reads_var(ov)
        {
            let elems = elems.clone();
            let frees = tuple_owned_elem_frees(&elems, v, data);
            if !frees.is_empty() {
                transition_free = Some(Value::Insert(frees));
            }
        }
        if was_in_scope
            && matches!(function.tp(v), Type::Reference(_, d) if !d.is_empty())
            && self.owned_refs.get(&v) == Some(&self.loops.len())
            && matches!(
                self.ref_rhs_ownership(value, data),
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
            match self.ref_rhs_ownership(value, data) {
                RefRhs::Owned => {
                    self.owned_refs.insert(v, self.loops.len());
                }
                RefRhs::View => {
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
        // loft#759 — a `&` parameter is `RefVar(Reference|Enum)`, so the bare
        // `matches!` above read it as "not a record" and skipped this whole
        // block.  Peel the `&` for the record-KIND question (the loft#753 /
        // loft#740 shape: one question, one peel) and carry the `&`-ness
        // separately, because the two halves below want opposite answers:
        //
        // - the deep-copy dep-strips are LOCAL-only.  They exist because
        //   `gen_set_first_ref_call_copy` copies the callee's buffer into a
        //   fresh store `v` owns — but a set THROUGH a `&` never reaches that
        //   path (codegen writes the returned DbRef straight into the caller's
        //   slot with `SetStackRef`), and `v` is a parameter the caller owns,
        //   so stripping its deps would only make the callee free it.
        // - the witness pairing is exactly what the `&` case needs, and needs
        //   it whatever `adopts_fresh_store` says.  With no copy in between,
        //   the buffer the callee filled IS what the caller now holds, so the
        //   scope-exit `OpFreeRef(__ref_N)` freed the record the caller went
        //   on reading and writing through.
        // loft#1106 — a NULLABLE heap local first-bound from a call whose return may
        // borrow an argument.  `S?` is `Optional(Reference(S))`, and the shape question
        // below is asked against the bare type, so the nullable spelling of the same
        // storage never reached the deps strip: the local kept the argument's dep, read
        // as a permanent borrow, and nothing freed the store the callee minted on its
        // other arm.  Both backends now bind it through the runtime join guard, which
        // leaves the local owning a store either way — so the deps have to go, or the
        // free that guard exists to make correct is never emitted.
        //
        // Gated on the SAME question the two emitters read, so a strip here always has
        // a guard under it.  Without that pairing the strip would license a scope-exit
        // free on a plain alias, which is the caller's store.
        if crate::use_analysis::nullable_join_first_bind(data, self.d_nr, function.tp(v), value)
            .is_some()
        {
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
        }
        let publishes_through_ref = matches!(function.tp(v), Type::RefVar(_));
        let mut record_target = function.tp(v);
        while let Type::RefVar(inner) = record_target {
            record_target = inner.base();
        }
        if matches!(
            record_target,
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && let Value::Call(fn_nr, _) = unspanned_value
            // A loft-defined callee — an `n_` global OR a `t_` method / generic
            // monomorph (@PLN85 generic-tuple-return-fix.md — a generic tuple return
            // is a `t_<Type>_<fn>` monomorph; without `t_` the adopts-fresh /
            // OpFreeRefIfDistinct pairing was skipped and the caller freed the
            // aliased return with a plain OpFreeRef, orphaning its text fields).
            // This decision and codegen's copy-or-adopt one have to name the SAME set
            // of callees, which is why the predicate lives in one place (loft#810).
            && data.def(*fn_nr).is_loft_defined()
        {
            let adopts_fresh_store = data.def(*fn_nr).return_adopts_fresh_store();
            // @PLN85 `local_source` over-free fix (LOFT_JOIN_OWN): `v` holds an OWNED
            // store (this adopts-fresh call) that a later borrow/join reassignment
            // displaces. Strip `v`'s declared deps so it is OWNED everywhere — the
            // owned path then deep-copies the borrow into `v`'s store and frees it at
            // scope exit; without this the displaced owned store is orphaned (it was
            // bound to `v`, not to the source retbuf the cleanup guards) and leaks.
            if !publishes_through_ref
                && self.displaced_owned.contains(&ov)
                && !function.tp(v).depend().is_empty()
            {
                let deps: Vec<u16> = function.tp(v).depend().clone();
                for d in deps {
                    function.make_independent(v, d);
                }
            }
            if !adopts_fresh_store && !publishes_through_ref {
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
            //
            // loft#759 — a set THROUGH a `&` parameter needs the same pairing
            // whatever `adopts_fresh_store` says.  For a LOCAL target the flag
            // decides whether a deep copy stands between the buffer and `v`
            // (`!adopts_fresh_store` copies, so the two are always distinct and
            // the pairing would be a no-op).  No copy stands in the `&` case,
            // so the buffer and the caller's slot alias whenever the callee
            // returned the buffer it was handed — the majority shape, and the
            // one `file()` has (`result = File{..}; result`).
            if (adopts_fresh_store || publishes_through_ref)
                && let Value::Call(_, args) = unspanned_value
            {
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
                            //
                            // loft#759 — a PARAMETER has no such block to fall
                            // out of: its `let` is the function signature, so
                            // it outlives every local including `av`, on both
                            // backends.  Its `var_scope` entry is written when
                            // the body first assigns it, which for a set inside
                            // an `if` reads as INNER-scoped and would route a
                            // valid witness into the @P378(a) branch below.
                            let av_scope = self.var_scope.get(&av).copied().unwrap_or(u16::MAX);
                            let v_scope = self.var_scope.get(&v).copied().unwrap_or(u16::MAX);
                            // A buffer is never its own witness.  The pairing exists to
                            // skip the buffer's free when ANOTHER variable adopted its
                            // store; `__ref_N = f(__ref_N)` has no other variable, and
                            // the guard then compares the store with itself and never
                            // frees at all — the buffer's own store leaks (loft#1013,
                            // where capturing the call's answer into the buffer it was
                            // handed is what gives the value an owner).
                            if av == v {
                                continue;
                            }
                            if (publishes_through_ref && function.is_argument(v))
                                || (v_scope <= av_scope && v_scope != u16::MAX)
                            {
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
        // @PLN85 over-free class — a VECTOR return-buffer (the hidden SRet arg,
        // NRVO'd into a source local like `best`) bound from a call that
        // DETERMINISTICALLY returns its OWN buffer (`!return_adopts_fresh_store()`,
        // e.g. `best = rows(b, __ref_1)` where `rows` returns its `__retbuf`) is
        // PutRef-ALIASED to the work-ref arg `__ref_1` — no deep copy (unlike the
        // Reference path above, which `gen_set_first_ref_call_copy` copies). A plain
        // `OpFreeRef(__ref_1)` at scope exit then whole-store-frees the RETURNED
        // buffer. Pair `__ref_1 → ov` so the free becomes
        // `OpFreeRefIfDistinct(__ref_1, ov)`: a no-op since they alias (the caller
        // owns + frees the returned buffer). Restricted to the return-buffer ARG —
        // a dead vector LOCAL (e.g. `other`) is not an argument, so it keeps its
        // plain free (its borrowed source must still be released, else it leaks).
        // Extended to Reference (struct) + struct-Enum return-buffers too: a struct
        // retbuf `ns = sim_new_gen_s(…, __ref_1)` that ADOPTS the buffer-returning
        // callee's `__ref_1` has the SAME plain-`OpFreeRef(__ref_1)` over-free (#462's
        // sim_descend driver, tp=194). The witness-pair is conservative —
        // `OpFreeRefIfDistinct(__ref_1, ov)` frees `__ref_1` exactly as the plain free
        // did when they are DISTINCT (the deep-copy case), and only skips when they
        // alias (the adopt case, the bug) — so this can only fix, never regress.
        if matches!(
            function.tp(ov),
            Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && function.is_argument(ov)
            && data
                .def(self.d_nr)
                .attributes
                .iter()
                .any(|a| a.hidden && a.name == function.name(ov))
            && let Value::Call(fn_nr, args) = unspanned_value
            && data.def(*fn_nr).name.starts_with("n_")
            && data.def(*fn_nr).code != Value::Null
            && !data.def(*fn_nr).return_adopts_fresh_store()
        {
            for arg in args {
                let av = match arg {
                    Value::Var(a) => Some(*a),
                    Value::Set(a, _) => Some(*a),
                    _ => None,
                };
                if let Some(av) = av {
                    let n = function.name(av);
                    if n.starts_with("__ref_") || n.starts_with("__rref_") {
                        self.paired_witness.entry(av).or_insert(ov);
                    }
                }
            }
        }
        // @PLN130 F2 — an element view that is live across a RESHAPE of its container cannot
        // stay an alias.  `remove` renumbers the positions in the container's store and the
        // view is a `DbRef` pinned to one, so it silently starts naming a different element:
        // measured, a pure READ answered `44/444` where its element held `33/333`, and a
        // write tore a live record (`99/444` — n from the stray write, tag from the real
        // element).  No detector sees it: nothing is freed and the pointer is live.
        //
        // Strip the container dep so the binding materialises into a store it owns (the same
        // F1 arm in `state/codegen.rs` picks it up, and native's generator already
        // materialises off empty deps).  The alias is LOST for this binding, so say so —
        // constraint 2, where rustc errors on a use-after-move loft copies and warns.
        //
        // Only fires for a view that is still USED after the reshape. A view whose last use
        // precedes it is not at risk, and materialising it would lose a write that lands
        // today — see `collect_views_to_materialise`.
        if matches!(
            function.tp(v),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && let Some(cause) = self.views_to_materialise.get(&v).copied()
            && let Some(container) = base_container_var(unspanned_value, data)
            && function.tp(v).depend().contains(&container)
        {
            let vname = function.name(v).to_string();
            let cname = function.name(container).to_string();
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
            let fname = data.def(self.d_nr).original_name();
            match cause {
                ViewCause::Reshaped => {
                    crate::copy_manifest::note_materialised_view(&vname, &cname, &fname);
                }
                // @PLN130 F8 — the third invalidator: the container VARIABLE is reassigned,
                // so the dep still names `bx` while the store it named is gone. Different
                // cause, different way out, so a distinct advice line.
                ViewCause::Reassigned => {
                    crate::copy_manifest::note_reassigned_view(&vname, &cname, &fname);
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
            // @PLN130 F1 — this strip is LOAD-BEARING FOR NATIVE, which is why the obvious
            // narrowing does not work.  Skipping it when both sides are borrows fixes the
            // interpreter (probe 30 goes green) and BREAKS native (`len 0 want 3`, the same
            // destruction the other way round): the emptied deps are exactly what makes the
            // native generator materialise `_own_store_k` at the bind, so leaving them makes
            // native alias the container instead.  Measured, then reverted.
            //
            // The real fix therefore cannot be "stop promoting the borrow" — it has to
            // materialise at the BIND for both backends, which is what native already does
            // as a side effect of this strip. See @PLN130 § F1 + F2 design.
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
    /// The free-side owned-vs-view verdict for a Reference reassignment RHS,
    /// read from the CANONICAL `ownership_of` oracle (@PLN90 D-own-1 — the last
    /// per-site ownership re-derivation, folded onto the one fact the delivery
    /// side already reads).  Owned → track the var as owned.  Borrowed AND Join →
    /// View: a reassignment whose new value is a borrow OR a runtime join
    /// DISPLACES the var's prior owned store (freed by the #316 transition-free),
    /// and the var must NOT be tracked as owned afterward (a join might be a
    /// borrow — tracking it owned would over-free the NEXT transition).  So Join
    /// folds to View, not "don't-track" — the p462 conditional
    /// `chosen = t[i] ?? m_none()` reassign needs the prior-store free.
    /// loft#854 — the memoised form. `ownership_of` recomputes the whole-function
    /// summary per question; `scan_set` asks once per assignment, which made a
    /// function with n assignments cost n whole-function walks.
    fn ref_rhs_ownership(&mut self, value: &Value, data: &Data) -> RefRhs {
        let d_nr = self.d_nr;
        let defs = self
            .fn_defs
            .get_or_insert_with(|| crate::use_analysis::function_defs(data, d_nr));
        match crate::use_analysis::ownership_of_with(data, d_nr, value, defs) {
            crate::use_analysis::Own::Owned => RefRhs::Owned,
            crate::use_analysis::Own::Borrowed { .. } | crate::use_analysis::Own::Join { .. } => {
                RefRhs::View
            }
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
        // Enforces @FR-O-Complete — the fact is per BINDING and per PATH, a
        // set-and-reconcile rather than one structural walk.  Both arms are scanned from
        // the SAME pre-`If` state and only what they AGREE on survives, so a store owned
        // on one path only is not treated as owned after the join.
        //
        // ⚠ @FR-O-Complete is the load-bearing invariant of the whole model: loft has no
        // user-facing borrow checker, so an incomplete fact is not a compile error someone
        // fixes — it is a miscompile or a leak.  Erring toward "not owned" here is why the
        // reconcile intersects rather than unions.
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

    /// Collect the variables an `if` branch ASSIGNS, so the caller can emit their
    /// null-init before the `If` — a variable written in one arm and read after it must
    /// hold null rather than an uninitialised slot.
    ///
    /// ⚠ `unspan()` first — the requirement `Value::unspan`'s own doc states for every site
    /// that pattern-matches a specific variant.  Without it a `Span` wrapping a `Set` falls
    /// to the catch-all, the assignment is not collected, and the variable loses its init.
    ///
    /// That path is REACHED: instrumented over 200 corpus programs, the catch-all dropped
    /// 2 Span-wrapped `Set`s and 8 whole Span-wrapped `Block`s.  No program's IR changes
    /// when the peel is added, so nothing observable was riding on it — the vars in
    /// question were already covered another way.  The peel stays because the reachability
    /// is what makes it a trap: Span placement has moved before, and the failure mode is a
    /// missing initialisation with nothing to report it.
    fn find_assigned_vars(val: &Value, mapping: &HashMap<u16, u16>, result: &mut Vec<u16>) {
        match val.unspan() {
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
        // @PLN85 skip_free-orphan (case a) — free each `__ncc_N` text temp that a
        // NON-TAIL statement consumes IN PLACE, right after that statement.  A
        // `skip_free` text ncc temp (`v[i] ?? ""`) is suppressed from its own
        // scope-exit free because the ncc block's result ALIASES it (freeing at
        // the ncc block would dangle the value the consumer still reads).  But a
        // text consumer (SetText / assignment / append) COPIES the String, so once
        // the consuming statement completes the temp's backing String is dead —
        // never freed on the interpreter → orphan.  The tail expression is left
        // untouched (case b: it IS the returned value, copied by the caller after
        // return, so any in-function free UAFs).  Native drops the String via RAII
        // and treats `OpFreeText` as a no-op, so the added op is interp-only.
        {
            let mut with_frees = Vec::with_capacity(ls.len());
            for stmt in ls.drain(..) {
                let mut ncc = Vec::new();
                collect_consumed_ncc_text(&stmt, function, &mut ncc);
                with_frees.push(stmt);
                for v in ncc {
                    with_frees.push(call("OpFreeText", v, data));
                }
            }
            ls = with_frees;
        }
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

    /// Is `v` the hidden NRVO return buffer promoted onto an argument slot, rather
    /// than a user parameter?
    ///
    /// A true parameter belongs to the caller and the callee may free none of it; the
    /// promoted buffer is a LOCAL that `classify_ret_promotion` renamed onto the hidden
    /// `__retbuf` attribute and `become_argument`ed, and this function mints its store.
    /// The un-renamed `__retbuf` placeholder is left out — no local was promoted onto
    /// it, so it holds no store of ours (loft#688).
    fn is_promoted_ret_buffer(&self, function: &Function, data: &Data, v: u16) -> bool {
        let n = function.name(v);
        n != "__retbuf"
            && data
                .def(self.d_nr)
                .attr_names
                .get(n)
                .is_some_and(|&a| data.def(self.d_nr).attributes()[a].hidden)
    }

    /// Enforces @FR-O-Derived: free placement is DERIVED, not decided — a local is freed
    /// iff it owns its store and does not transfer it out, once, at scope exit.  A
    /// per-site heuristic anywhere else in codegen is the bug that rule names.
    fn free_vars(
        &mut self,
        is_return: bool,
        expr: &Value,
        function: &mut Function,
        data: &Data,
        tp: &Type,
        to_scope: u16,
    ) -> Vec<Value> {
        let ret_var = returned_var_null_unified(expr, data.def_nr("OpNullRefSentinel"));
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
        let mut null_arm_record_sources: Vec<u16> = Vec::new();
        let return_sources: HashSet<u16> = if is_return {
            let mut sources = Vec::new();
            collect_return_sources(expr, data, &mut sources);
            // A nullable return (`if b { Struct{} } else { null }`) leaves the
            // present arm's work-ref placeholder orphaned on the null path.  When
            // a null arm is reachable, do NOT SET-suppress a Reference/Enum work-
            // ref source — hand it to the standard work-ref free path so the
            // orphan is freed.  See `return_has_null_arm`.
            let null_sentinel_nr = data.def_nr("OpNullRefSentinel");
            if return_has_null_arm(expr, null_sentinel_nr) {
                // @PLN85 P4-records: a record source with a reachable NULL arm
                // is a runtime JOIN — transferred to the caller on the present
                // path, an orphan (its preamble null-init ALLOCATES on interp)
                // on the null path.  The old design freed it unconditionally
                // (no orphan, but the present path returned a FREED store off
                // the eval stack — the poison-visible UAF).  Keep it
                // SUPPRESSED here and record it; the return leg below hoists
                // the value to `__ret_N` and emits
                // `OpFreeRefIfDistinct(src, __ret_N)` — the runtime decides.
                for &v in &sources {
                    if matches!(
                        function.tp(v),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    ) {
                        null_arm_record_sources.push(v);
                    }
                }
                // loft#936 — a COLLECTION source is the same runtime join, one
                // level down.  `_vec_N` is a VIEW into a backing record
                // (`vector<T>["__vdb_N"]`), so it is the BACKING var that owns
                // the store and that `get_free_vars` suppresses (via
                // `backs_return_source`).  That suppression is only correct on
                // the arm that actually delivers the store: on the null arm the
                // caller gets the sentinel and the entry-allocated backing
                // record is owned by nobody — one orphan per CALL, so a loop
                // calling such a function leaked until the 65,535-slot store
                // table was exhausted.  `OpFreeRefIfDistinct` compares
                // `store_nr`, and the view shares the backing record's store, so
                // the delivering arm still transfers it untouched.
                for &v in &sources {
                    if !crate::parser::vectors::is_collection(function.tp(v)) {
                        continue;
                    }
                    for d in function.tp(v).depend() {
                        if d != v
                            && !function.is_argument(d)
                            && !null_arm_record_sources.contains(&d)
                        {
                            null_arm_record_sources.push(d);
                        }
                    }
                }
            }
            // @PLN85 F2 (the fuzzer's catch: `t[i] ?? N{..}`) — a
            // MULTI-source return mixing an OWNED record work-ref arm with
            // view arms is a runtime JOIN even WITHOUT a null arm: the owned
            // arm's store transfers only when ITS branch ran, else its
            // (interp-allocating) preamble store orphans.  The CALL-default
            // twin never hits this — a Call arm contributes no source var, so
            // the work-ref keeps its plain free.  Route the owned sources
            // through the same hoist + `OpFreeRefIfDistinct` leg the null-arm
            // join uses; view sources (non-empty deps) stay suppressed as
            // borrows.
            //
            // loft#1078 — the promoted NRVO buffer is one of those owned sources,
            // and `is_argument` alone excluded it.  `fn pick(c) -> S { w = S{a:7};
            // if c { S{a:9} } else { w } }` renames `w` onto the hidden buffer, so
            // both arms name an owned candidate and exactly one wins; on the arm
            // that does NOT deliver `w`, the store `w` minted is returned by nobody
            // and freed by nobody — one orphan per call, which is the same u16
            // store-table exhaustion loft#688 and loft#1022 each closed for their
            // own shape.  The carve-out is the one those two already established:
            // a user PARAMETER belongs to the caller and the callee frees none of
            // it, while the promoted buffer is the one argument that is really a
            // local this function minted (`is_promoted_ret_buffer` — hidden attr,
            // renamed off `__retbuf`).  It reaches here rather than loft#688's leg
            // because that leg excludes anything in `sources`, and the owning arm
            // puts it there — the identical reason loft#1022 restated the carve-out
            // in its own gate.
            if sources.len() > 1 {
                for &v in &sources {
                    if matches!(
                        function.tp(v),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    ) && function.tp(v).depend().is_empty()
                        && (!function.is_argument(v)
                            || self.is_promoted_ret_buffer(function, data, v))
                        && !null_arm_record_sources.contains(&v)
                    {
                        null_arm_record_sources.push(v);
                    }
                }
            }
            // loft#688 — the NRVO return buffer is an ARGUMENT, and `variables()`
            // stops at scope 0 ("never return function arguments"): a true
            // parameter belongs to the caller, so the callee must not free it.
            // An NRVO buffer breaks that premise.  It is a promoted LOCAL — the
            // rename in `classify_ret_promotion` gives the hidden `__retbuf` attr
            // the local's name and `become_argument`s it — and THIS function mints
            // its store with `OpDatabase`.  When a sibling return path delivers a
            // different store, the minted one is returned by nobody and freed by
            // nobody: one orphan per call, which exhausted the 65,535-entry store
            // table after 65,535 calls and was invisible below that.
            //
            // So treat it like any other candidate source that may or may not be
            // the returned store: route it through the same hoist +
            // `OpFreeRefIfDistinct` leg the null-arm join uses.  On the path that
            // DOES return it, `return_sources` holds it and it is skipped here; on
            // a path that returns something else the runtime comparison frees it,
            // and a buffer not yet minted on this path is the null sentinel, which
            // `free` ignores.
            //
            // A promoted buffer is exactly an argument whose attribute is HIDDEN
            // and no longer called `__retbuf`: `ref_return` renames the attr to the
            // local's name but leaves `hidden` set, and a user-declared parameter
            // is never hidden.  The un-renamed `__retbuf` placeholder is left alone
            // — no local was promoted onto it, so it holds no store of ours.
            //
            // loft#1096 — and a COLLECTION return's promoted buffer is left alone too,
            // because the premise above ("a buffer not yet minted on this path is the
            // null sentinel, which `free` ignores") is false for it.  The leg reads the
            // buffer as a local THIS function mints; that is true where the caller's
            // work-ref reaches the call as a bare `OpInitRef` sentinel, which is what a
            // record work-ref does.  A collection work-ref does not:
            // `codegen::gen_set_first_vector_null` gives an owned vector local
            // `OpInitRef` + `OpDatabase`, so the buffer arrives ALIVE, and the callee's
            // own `OpDatabase` then only clears it in place (`alloc_record_at` reuses a
            // live slot rather than minting beside it).  So there is never a distinct
            // callee-minted store to reclaim here, and the store this free reached was
            // always the CALLER's — which the caller still names and still frees at its
            // own scope exit.  A `-> vector<T>` function with a `null` arm therefore
            // handed the next call a freed record to `OpClearVector`, and a loop faulted
            // on its second iteration on both backends (`@FR-O-Owner`: a free is for a
            // store the value OWNS).
            let collection_return = crate::parser::vectors::is_collection(
                data.def(self.d_nr).returned.ret_promo_base(),
            );
            for v in 0..function.count() {
                if function.is_argument(v)
                    && !sources.contains(&v)
                    && !null_arm_record_sources.contains(&v)
                    && matches!(
                        function.tp(v),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                {
                    let n = function.name(v);
                    if n != "__retbuf"
                        && !collection_return
                        && let Some(&a) = data.def(self.d_nr).attr_names.get(n)
                        && data.def(self.d_nr).attributes()[a].hidden
                    {
                        null_arm_record_sources.push(v);
                    }
                }
            }
            // loft#1022 — the THIRD shape of the same runtime join, after the null arm
            // above and loft#688's promoted buffer: a return whose arms disagree about
            // OWNERSHIP.  `fn pick(bx, take) -> P { if take { bx.p } else { P { x: 9 } } }`
            // types as `P["bx"]` — a view — while the else arm mints its own store in a
            // work-ref.  `collect_return_sources` is the UNION of the arms, so the
            // work-ref lands in `return_sources` and its scope-exit free is suppressed on
            // EVERY path; on the borrowing path nothing returns it and nothing frees it.
            // One orphan per call, unbounded in a loop, and the store the entry preamble
            // allocated leaks even when the owning arm never runs.
            //
            // The suppression's own comment calls itself PATH-LOCAL, and it is — across
            // separate `return` statements.  A single `return` whose value is a JOIN puts
            // both arms in one set, which is the case the path-locality argument does not
            // cover.  So route it to the same hoist + `OpFreeRefIfDistinct` leg the other
            // two use and let the runtime decide: on the arm that delivers the store the
            // comparison matches and the free is a no-op, on the borrowing arm the stores
            // differ and the orphan is released.
            //
            // The gate needs BOTH conditions, and the loop a third.
            //
            // A RECORD return only.  A collection or text return carries its own mature
            // machinery — loft#936's backing-store comparison and the B5-L3 text hoist —
            // and its `__vdb_N` backing is a `Type::Reference` too, so a test on the
            // SOURCE's type alone claims it and re-routes a return that was already
            // correct (repro_p365's `nested` leaked its backing under exactly that).
            //
            // And a genuine BORROWING ARM, not merely a return type that carries deps.
            // A record LITERAL whose fields alias locals (`TableDef { columns: cols,
            // indexes: ixs }`) has deps too, and every one of its arms delivers the
            // source — nothing can be orphaned, so hoisting it to `__ret_N` and freeing
            // the original released the vectors the returned copy still names, and the
            // sqldb round trip read back zero indexes.
            //
            // And in the loop: only a source THIS function owns.  A source that is
            // itself a borrow carries deps naming what it views, and freeing that would
            // release the CALLER's store — an over-free where the defect is a leak.
            if matches!(tp.base(), Type::Reference(_, _) | Type::Enum(_, true, _))
                && return_has_non_source_arm(expr, &sources)
            {
                for &v in &sources {
                    if null_arm_record_sources.contains(&v)
                        || !matches!(
                            function.tp(v),
                            Type::Reference(_, _) | Type::Enum(_, true, _)
                        )
                        || !function.tp(v).depend().is_empty()
                    {
                        continue;
                    }
                    // A user PARAMETER belongs to the caller and the callee frees none of
                    // it.  The promoted NRVO buffer is the one argument that is really a
                    // local — loft#688's leg names it the same way, by its attribute
                    // being HIDDEN — and it reaches the borrowing arm with a store this
                    // function minted.  That leg cannot claim it here because it excludes
                    // anything in `sources`, and the owning arm puts it there.
                    if function.is_argument(v) && !self.is_promoted_ret_buffer(function, data, v) {
                        continue;
                    }
                    null_arm_record_sources.push(v);
                }
            }
            sources.into_iter().collect()
        } else {
            HashSet::new()
        };
        let mut ls = self.get_free_vars(function, data, to_scope, tp, ret_var, &return_sources);
        // @PLN85 P4-records — at a RETURN site, a record work-ref's store may
        // BE the returned store: a named local adopts the arm's fresh Object
        // (`v: E = Pass{..}; if c { v = Fail{..} }; v` — two candidate stores,
        // one winner at runtime), and an unconditional OpFreeRef frees the
        // winner too — the caller then reads a freed store (silently stale
        // without LOFT_POISON; the par t4 catch).  Make every record work-ref
        // free at a return CONDITIONAL on not being the returned store — for
        // an unrelated work-ref the stores are distinct and the free runs
        // exactly as before.
        if is_return
            && ret_var != u16::MAX
            && (ret_var as usize) < function.count() as usize
            && matches!(
                function.tp(ret_var),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
        {
            let free_nr = data.def_nr("OpFreeRef");
            let free_if = data.def_nr("OpFreeRefIfDistinct");
            for op in &mut ls {
                // ANY record-typed local's store may BE the returned store —
                // not only a parser-minted work ref: a NAMED local aliased
                // into the hidden return-buffer param (`best = cand` — the
                // NRVO buffer keeps raw-alias Sets by design) had its
                // unconditional free kill the returned store on the
                // reassigned path (the 150-i306 `choose` shape; poison read
                // the caller's field as 0xDEADBEEF).  Distinct stores free
                // exactly as before.
                if let Value::Call(d, args) = op
                    && *d == free_nr
                    && let Some(a0) = args.first()
                    && let Value::Var(w) = a0.unspan()
                    && matches!(
                        function.tp(*w),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                {
                    *op = Value::Call(free_if, vec![Value::Var(*w), Value::Var(ret_var)]);
                }
            }
        }
        // The B5-L3 wrap (Set(__ret_N, expr); free ops; Return(Var(__ret_N)))
        // must not fire when `expr` is already a `Return` or contains one
        // at its tail — otherwise we'd emit `let _ret = return …` (E0308 in
        // native).  Recurse through `Insert` (which scopes wraps Return in
        // for free-vars cleanup) and `Block`.
        let expr_is_terminal = expr_ends_in_return(expr);
        // @PLN85 P4-records — a null-arm record return: hoist the value to a
        // `__ret_N` temp, then free each suppressed JOIN source CONDITIONALLY
        // (`OpFreeRefIfDistinct(src, __ret_N)`): the present arm returns the
        // source's store (not distinct → kept, transferred to the caller); the
        // null arm returns the sentinel (distinct → the preamble-allocated
        // placeholder is freed — no orphan).  Runs FIRST: the fast path would
        // otherwise emit `Return(expr)` with the sources leaking on the null
        // path (frees suppressed above), and the legacy path would re-open the
        // eval-stack UAF.
        if is_return && !expr_is_terminal && !null_arm_record_sources.is_empty() {
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let free_if = data.def_nr("OpFreeRefIfDistinct");
            let mut result = Vec::with_capacity(ls.len() + null_arm_record_sources.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            for &src in &null_arm_record_sources {
                result.push(Value::Call(free_if, vec![Value::Var(src), Value::Var(tmp)]));
            }
            result.append(&mut ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        }
        // @PLN85 poison-green — a bare-Var tail is free-safe (its slot holds
        // the value) EXCEPT a `&τ` place ref (`RefVar`): returning it DEREFS
        // the place DbRef at the Return, after `ls`'s frees released the
        // source store (the @PLN87 L3/L4 live-read shapes under LOFT_POISON).
        // Exclude it from the fast path so it takes the B5-L3 hoist below —
        // `Set(__ret_N, Var(r))` performs the deref BEFORE the frees.
        // (Text-inner RefVars are EXCLUDED from the exclusion: a `&text` tail
        // is the promoted out-BUFFER returned per the text-return contract —
        // the buffer lives in the caller, so returning it raw is free-safe,
        // and hoisting it emitted a native `Str::new(&local_String)` dangle.)
        let var_is_place_ref = !ls.is_empty()
            && matches!(expr, Value::Var(v)
                if matches!(function.tp(*v), Type::RefVar(inner)
                    if !matches!(inner.base(), Type::Text(_))));
        // @PLN85 Class B2 — a plain text LITERAL tail reads none of the
        // to-be-freed locals and has no side effects, so `ls (frees); return
        // "lit"` is correct and copy-free.  The B5-L3 text hoist below would
        // otherwise mint an OWNED `__ret_N = "lit"` copy (skip_free) that the
        // caller consumes-and-leaks — the leak that appears whenever a
        // text-returning fn with ANY freeable local returns a literal (the
        // p54 match-of-literals / json-classify family).  Returning the literal
        // directly is exactly what a fn with NO frees already emits.
        //
        // @PLN85 corpus — the null-text sentinel `OpConvTextFromNull()` (an
        // early `return null` in a `-> text?` fn) is the SAME free-safe shape:
        // it lowers to `Str::new(STRING_NULL)`, a borrowed static that owns no
        // allocation and aliases none of the to-be-freed locals.  Treated as a
        // literal it returns directly (matching native's `return
        // Str::new(STRING_NULL)`); routed through the B5-L3 text hoist it minted
        // an OWNED skip_free `__ret_N` copy of the sentinel that a non-copying
        // caller (`f(-1) == null`) never freed → append_text orphan.
        let expr_is_null_text_sentinel = matches!(expr.unspan(),
            Value::Call(d, args) if args.is_empty() && *d == data.def_nr("OpConvTextFromNull"));
        if ls.is_empty()
            || ((matches!(expr, Value::Null | Value::Var(_) | Value::Text(_))
                || expr_is_null_text_sentinel)
                && !var_is_place_ref)
        {
            if is_return && !expr_is_terminal {
                ls.push(Value::Return(Box::new(expr.clone())));
            } else if matches!(expr, Value::Null) {
                // skip
            } else {
                ls.push(expr.clone());
            }
        } else if expr_is_terminal {
            // expr is already a `Return(...)` (or a `Block`/`Insert(...)` ending
            // in one) — the cleanup was emitted alongside it by the inner Return
            // arm's free_vars call.  Re-emitting `ls` here would duplicate every
            // OpFreeText/OpFreeRef (and tack on a dead `Return(Null)`).  Just
            // propagate the terminal as-is.  #549 bug 2: a terminal *Block* must
            // hit this dedup BEFORE the `Value::Block` insert_free arm below —
            // an explicit `return (owned_text, …)` at a body tail is processed by
            // both the `Value::Return` scan arm AND `convert`'s is_body_return
            // tail sweep; the first makes the synthetic tuple block terminal, and
            // without ordering this check first the second re-ran `insert_free`,
            // emitting a second `OpFreeText` on the owned element (double free
            // under `-C debug-assertions=on`; text.rs:334).
            return vec![expr.clone()];
        } else if let Value::Block(bl) = expr {
            return self.insert_free(bl, &ls, is_return, data, function);
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
            free_copied_work_texts(&mut result, expr, function, data);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return
            && !expr_is_terminal
            && ret_var == u16::MAX
            && is_heap_return_type(tp)
            && !matches!(expr, Value::Var(_))
            && expr.is_place_read(data)
        {
            // loft#754 — the B5-L3 rule for a HEAP return (`vector` / record /
            // struct-enum) whose tail is a PLACE read.  `is_value_return_type`
            // names only the scalars and the text branch below only text, so
            // such a tail with pending frees reached the fall-through and was
            // emitted as a DISCARDED statement plus a fabricated
            // `Return(Null)`.  The interpreter read the value off eval-stack
            // top and answered correctly; native emitted
            // `let _ = expr; …; return DbRef::NULL`, so
            // `fn f(w) -> vector<u8> { if … { return []; } w.items[0].bytes }`
            // handed back an EMPTY vector — silently, and on one backend only.
            // (rustc even flagged it as `unused_must_use` on the dropped
            // element read.)
            //
            // The hoist states the interpreter's own order in the IR: evaluate
            // the tail, run the frees, return the captured value.
            //
            // A PLACE read is the whole class, and the bound is load-bearing in
            // both directions.  Only a place leaves its value on the eval stack
            // alone — it allocates nothing and writes no return buffer — so
            // only a place can be dropped by a `Return(Null)`; and `Set(tmp,
            // place)` is a bare `DbRef` copy, which is why the hoist adds no
            // ownership.  A CALL tail already delivers through its hidden
            // buffer, and hoisting one instead engaged the store-transfer
            // machinery (`protect_store_frees` + `CopyRefOrNull`) around a
            // borrowed argument, which over-froze the caller's store
            // ("Delete on locked store", `return-borrow-of-mutated-arg`).  A
            // bare `Var` is excluded because the fast path above already
            // returns it directly.
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
        } else if is_return
            && !expr_is_terminal
            && ret_var == u16::MAX
            && is_heap_return_type(tp)
            && !has_return_buffer(self.d_nr, data)
            && tail_is_value_call(expr, data)
        {
            // loft#793 — the CALL sibling of loft#754, in the one regime that
            // fix's "a CALL tail already delivers through its hidden buffer"
            // does not cover: a NULLABLE heap return (`-> S?`,
            // `-> vector<S>?`, `-> StructEnum?`).  Only a DENSE heap return is
            // given a hidden `__retbuf` argument — both reservation sites
            // (`parser/mod.rs`, `parser/definitions.rs`) gate on
            // `Reference | Vector | Enum(_, true, _)`, which `Optional` is not
            // — so for a nullable one the callee's value comes back ONLY as
            // the call's own return value.
            //
            // Dropped, the fall-through emitted the call as a DISCARDED
            // statement plus a fabricated `Return(Null)`.  The interpreter
            // read the value off eval-stack top and answered correctly, so
            // the whole class was invisible there; native — and any
            // `StaticCall` into a library's compiled half, which is why it
            // surfaced across a library boundary first — returned the null
            // sentinel.  `fn f() -> S? { return mk(); }` answered null,
            // silently, with the record left leaked.
            //
            // Hoist the call into `__ret_N` and return that, and make each
            // pending record/vector free CONDITIONAL on not being the hoisted
            // store: a callee with a return buffer may deliver a fresh store
            // OR chain the work ref this frame passed in, and only the
            // runtime knows which.
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let free_nr = data.def_nr("OpFreeRef");
            let free_if = data.def_nr("OpFreeRefIfDistinct");
            for op in &mut ls {
                if let Value::Call(d, args) = op
                    && *d == free_nr
                    && let Some(Value::Var(w)) = args.first().map(Value::unspan)
                    && matches!(
                        function.tp(*w),
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
                {
                    *op = Value::Call(free_if, vec![Value::Var(*w), Value::Var(tmp)]);
                }
            }
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            result.append(&mut ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return && matches!(tp.base(), Type::Text(_)) && !expr_is_terminal {
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
            free_copied_work_texts(&mut result, expr, function, data);
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
            // @PLN85 poison-green — the block-VALUE variant of the B5-L3 rule:
            // a non-Void block's exit frees run AFTER the tail expression but
            // BEFORE the enclosing consumer copies the value out
            // (`test_value = { mk()[0] }` — the text tail borrows the
            // block-local vector's element bytes; the block-exit OpFreeRef
            // poisons them before the Set's byte copy).  Hoist the value to a
            // temp: `Set(__blk_N, expr)` deep-copies the bytes (OpAppendText)
            // while the source store is still live, the frees run, and the
            // temp is the block's value.  Text-typed only — the one shape
            // where the Set IS a deep copy; record/vector block values keep
            // their existing paths.
            // (A branch tail — if/match — is EXCLUDED: its arms are unified
            // to write the assignment target directly, so the hoist's Set
            // would append the branch's value a SECOND time onto what the arm
            // already wrote — `if c { null } else { "error" }` read back
            // "errorerror".  Branch tails keep their existing arm-delivery.)
            if !is_return
                && !ls.is_empty()
                && matches!(tp.base(), Type::Text(_))
                && !matches!(expr, Value::Null | Value::Var(_))
                && !Self::tail_is_branch(expr)
                && !expr_is_terminal
            {
                self.ret_temp_counter += 1;
                let name = format!("__blk_{}", self.ret_temp_counter);
                let tmp = function.add_temp_var(&name, tp);
                // @PLN85 n3 — register the hoist temp at the FUNCTION BODY scope
                // (1), not `self.scope` (this nested block's scope, where `__blk_N`
                // is the tail value and so is EXCLUDED from `get_free_vars` as the
                // block's `ret_var` → its owned String leaked, e.g.
                // `test_value = { a = Item{name:"x"}; b = a; a.name }`).  The block
                // value is delivered to the outer consumer by COPY (OpAppendText),
                // never moved, so the temp stays owned and must be freed.  Its
                // `InitText` is hoisted to the function root (the `lift_texts`
                // mechanism below), so its `OpFreeText` must fire exactly ONCE at
                // function exit — registering at scope 1 makes the function-exit
                // sweep (`get_free_vars(to_scope = 1)`) emit it, matching the
                // root-level init and avoiding a per-iteration double-free in loops.
                // The hoist only fires for `!is_return` blocks (always nested,
                // scope >= 2), so `__blk_N` is never a function return value — the
                // function-exit free can never free a value the caller adopts.
                self.var_scope.insert(tmp, 1);
                self.var_order.push(tmp);
                self.lift_texts.push(tmp);
                let mut result = Vec::with_capacity(ls.len() + 2);
                result.push(v_set(tmp, expr.clone()));
                result.append(&mut ls);
                result.push(Value::Var(tmp));
                return result;
            }
            // Whether anything runs BETWEEN the value and the return.  `ls` holds
            // this scope's frees; the `insert` below puts the value in front of
            // them, so a non-empty `ls` here is exactly "frees follow the value".
            let frees_follow = !ls.is_empty();
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
                // loft#1097 — `ret_var` cannot carry the sentinel when the tail has a
                // NULL arm and the var it unified onto is a COLLECTION.
                // `returned_var_null_unified` folds a null arm into its sibling's var
                // on the premise it states itself: *"the work-ref null-inits at
                // function entry and a null arm never allocates into it, so
                // `Return(Var(v))` yields the same null the sentinel did"*.  That holds
                // for a RECORD work-ref, which `gen_set_first_ref_null` sentinel-inits.
                // It is false for a collection: `gen_set_first_vector_null` gives an
                // owned vector local `OpInitRef` + `OpDatabase`, and a PROMOTED buffer
                // arrives alive from the caller — so on the null path `Var(v)` is a
                // live, populated vector.  `f(-1) == null` answered FALSE while
                // `len(f(-1))` answered 2, with no diagnostic: @FR-E-Null says the
                // sentinel is a real observable value, and calls.md's `(F-Return)` says
                // a body ending in an expression returns THAT expression — here the
                // expression was demoted to a statement and its value dropped.
                //
                // Hoist the tail's value to a temp instead — the shape the null-arm
                // RECORD join above already uses — so the frees still run between the
                // value and the return while the arm's own answer, sentinel included,
                // is what comes back.  Like loft#957's temp below it is deliberately
                // NOT registered in `var_scope`: it holds the value being transferred
                // to the caller, and a scope-exit free of it would free what the caller
                // adopts.
                let null_arm_needs_the_value = ret_var != u16::MAX
                    && frees_follow
                    && !expr_is_terminal
                    && (ret_var as usize) < function.count() as usize
                    && crate::parser::vectors::is_collection(function.tp(ret_var))
                    && return_has_null_arm(expr, data.def_nr("OpNullRefSentinel"));
                if null_arm_needs_the_value {
                    self.ret_temp_counter += 1;
                    let name = format!("__ret_tail_{}", self.ret_temp_counter);
                    let tmp = function.add_temp_var(&name, tp);
                    ls[0] = v_set(tmp, expr.clone());
                    ls.push(Value::Return(Box::new(Value::Var(tmp))));
                } else if ret_var != u16::MAX {
                    ls.push(Value::Return(Box::new(Value::Var(ret_var))));
                } else if frees_follow
                    && *tp != Type::Void
                    && !expr_is_terminal
                    // A CALL, and only a call.  That is the producer the diagnosis
                    // names — the one whose result no binding holds — and it is the
                    // same shape `chain_site_set_shape` promotes into `__retbuf`
                    // when promotion does run.  A wider test is not a safer one: an
                    // earlier cut allowed any non-null tail and so fired on a
                    // COROUTINE's body, whose tail is the `while` loop itself.  That
                    // wrapped the loop into `__ret_tail_1 = loop { … }`, and the
                    // state-machine lowering then could not see the captured
                    // parameters — four `coroutine_matrix` cells failed to compile
                    // with `cannot find value var_n`.  An `iterator` return is
                    // excluded outright for the same reason: a coroutine does not
                    // return its body's value.
                    && matches!(expr.unspan(), Value::Call(_, _) | Value::CallRef(_, _))
                    && !matches!(tp.base(), Type::Iterator(_, _))
                {
                    // loft#957 — the same eval-stack reliance P236 names above, for
                    // the case where the value lives in NO variable: a tail
                    // `return <call>` whose callee is a bodiless `#rust` native
                    // returning a collection (`read_bytes`, `list_dir`).  Return
                    // promotion never runs for it — there is no local candidate to
                    // promote — so nothing binds the result, and the legacy shape
                    // lowered to `read_bytes(p); free …; return null`.  The
                    // interpreter answered correctly by accident, its `OpReturn`
                    // taking the eval-stack top the call happened to leave there;
                    // native emitted the typed null sentinel and the bytes were
                    // gone, silently, on a backend `loft test --interpret` cannot
                    // see.  P236 could reuse a variable that already existed; here
                    // there is none, so give the value one and return THAT.
                    //
                    // Gated on frees actually following: with nothing between the
                    // value and the return, the existing shape already emits
                    // `return <expr>` directly and needs no temp.  The temp is
                    // deliberately NOT registered in `var_scope` — it holds the
                    // value being transferred to the caller, so a scope-exit free
                    // of it would free what the caller adopts.
                    self.ret_temp_counter += 1;
                    let name = format!("__ret_tail_{}", self.ret_temp_counter);
                    let tmp = function.add_temp_var(&name, tp);
                    ls[0] = v_set(tmp, expr.clone());
                    ls.push(Value::Return(Box::new(Value::Var(tmp))));
                } else {
                    ls.push(Value::Return(Box::new(Value::Null)));
                }
            }
        }
        ls
    }

    /// Which variables this scope must free on the way out, as the `OpFreeRef` /
    /// `OpFreeText` / per-element ops to emit before leaving it.
    ///
    /// Enforces @FR-O-Derived — free placement is DERIVED, not decided: a local is freed
    /// iff it OWNS its store and does not transfer it out, once, at scope exit — and
    /// @FR-O-Owner, the single-owner invariant that makes "once" correct.  There is no
    /// per-site heuristic here; every arm answers from a carried fact.
    ///
    /// `ret_var` and `return_sources` are what "does not transfer it out" means in
    /// practice: a store handed to the caller is the caller's to free (@FR-O-Move), so its
    /// scope-exit free is suppressed.  `return_sources` is PATH-LOCAL — a source dead on a
    /// sibling path is absent from it and is still freed by that path's own sweep, which is
    /// what keeps @FR-O-Complete true without a global "skip" stamp that would over-suppress
    /// and leak.
    ///
    /// ⚠ Ownership is read here from a carried fact, but "empty deps" is only a PROXY for
    /// it (loft#723) — see [`crate::variables::Function::is_skip_free`], the second fact
    /// that vetoes the proxy for a borrow whose dep list was never populated.
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
            return_sources.contains(&v) && crate::data::is_dbref(function.tp(v))
        };
        for v in vars {
            if v == ret_var || suppress_source(function, v) {
                continue;
            }
            // on=4 iteration scratch (`hash_scratch`): a `return` out of an exposed loop
            // bypasses the loop epilogue's OpFreeScratch, so free the dedicated scratch
            // store here at scope exit too.  OpFreeScratch is conditional (frees only a
            // read-only source's dedicated store) and rec==0-guarded; the epilogue nulls
            // the var on the complete/break paths, so the two never double-free.
            // (expose-iteration-scratch.md Open question A.)
            if function.name(v).contains("hash_scratch") {
                ls.push(call("OpFreeScratch", v, data));
                continue;
            }
            // T1.3: tuple scope exit — free owned elements in reverse index order.
            if let Type::Tuple(elems) = function.tp(v) {
                let elems = elems.clone();
                ls.extend(tuple_owned_elem_frees(&elems, v, data));
                continue;
            }
            if matches!(function.tp(v).base(), Type::Text(_)) {
                // @PLN25 slice (c): peel `Optional` — a `text?` local owns the same heap
                // text as `text` and must be freed identically (else its interval is not
                // extended and the slot allocator aliases it — the `text? = text?` copy
                // read back empty).
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
            // P193: include keyed collections (Sorted/Hash/Index/Radix)
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
            // @PLN25 slice (c): peel `Optional` so an owned `vector?`/`reference?` local is
            // still freed at scope exit (same reasoning as the `text?` case above).
            | Type::Radix(_, _, dep) | Type::Trie(_, _, dep) = function.tp(v).base()
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
                // H2 step 5 (DEPS_INVENTORY): the BLOCK-RESULT type's deps were
                // read here for years under the positional guess.  That read is
                // RETIRED: the declared-return (`ret_borrows_v`, a TYPED decode),
                // returned-var, and return-source-backing checks above decide the
                // suppression.  A debug sentinel used to scream when the old read
                // would have "decided alone" (`tp.depend()` names `v` while
                // `in_ret` is false), on the theory that such a case would need
                // the read re-added.  It does NOT: every firing is a FALSE positive
                // of the retired POSITIONAL decode — a field / enum-field / match-
                // arm return that COPIES its source into the caller's retbuf
                // (`return fv_c.pts`, `match e { Filled{items} => items }`), so the
                // local source `v` is correctly freed at scope exit AFTER the copy.
                // Re-adding the read would instead SUPPRESS that free and LEAK the
                // source.  Verified on the seven firing cases (450, 508, repro_p365,
                // four 85-store-lifetime-*) — value + leak + LOFT_POISON + the DA
                // store-free asserts all clean, both backends — so the read stays
                // retired and the sentinel is removed (the reliable checks subsume
                // every TRUE return source; the positional read only added noise).
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
                        && crate::parser::vectors::is_keyed(function.tp(v)));
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
                //
                // The handover is only sound where this free EXISTS to be
                // suppressed — `owns` — so the record's cascade must reach
                // no further.  #682 is what happens when it does: a captured
                // PARAMETER never enters this sweep (see `variables()`: "never
                // return function arguments") and a projection local is
                // `owns == false`, yet the cascade freed both, destroying the
                // caller's store.  `mark_borrowed_captures` recomputes this same
                // verdict once every function is scanned and marks those captures
                // borrowed on the record, which is what stops the cascade.
                let captured_ref =
                    function.is_captured(v) && matches!(function.tp(v), Type::Reference(_, _));
                // @PLN94 TEST-ONLY over-free injection (never set in production): force the scope-exit
                // free of a NAMED borrowed var (owns=false) so the over-free check has a firing
                // true-positive. Subject to the same !in_ret/!skip_free/!captured guards as a real free.
                let inject_free = inject_free_borrowed() == Some(function.name(v));
                let emit = (owns || is_work_ref || inject_free)
                    && !in_ret
                    && !function.is_skip_free(v)
                    && !self.free_transferred.contains(&v)
                    && !captured_ref;
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
                    // @PLN125 arc B — the scope-end hook fires where the BINDING's life
                    // ends, which is not always where the STORE is released.  A value
                    // delivered through a caller-side return buffer has two variables
                    // naming one record: the buffer (`__ref_N`, function-scoped and
                    // reused) and the witness the author actually bound.  The author's
                    // binding is the one that ends — per loop iteration, at its own
                    // scope — so the witness drops and the buffer never does.  Getting
                    // that backwards is a rollback that runs once for a loop that opened
                    // a transaction on every pass.
                    let is_buffer = is_work_ref && self.paired_witness.contains_key(&v)
                        || self.witness_buffer.values().any(|&b| b == v);
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
                        //
                        // The FREE is skipped in the adoption case; the DROP is not.
                        // The store surviving into the next iteration is a reuse
                        // optimisation, and the value it held is over either way.
                        if let Some(hook) = self.scope_end_drop(function, v, data) {
                            ls.push(hook);
                        }
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(buffer)],
                        ));
                    } else if inject_drop_free() == Some(function.name(v)) {
                        // @PLN94 TEST-ONLY positive control (never set in production; one cached env
                        // read/process): drop the scope-exit free for the NAMED owned var, injecting
                        // a genuine leak. The `check-leak` scan must go RED on it — the true-positive
                        // gate. Mirrors LOFT_NO_A1B / LOFT_STORE_GUARD_INJECT.
                    } else {
                        // The type's scope-end hook, immediately BEFORE the free that ends
                        // the value's life — unless `v` is a buffer whose witness already
                        // ran it.
                        if !is_buffer
                            && let Some(hook) = self.scope_end_drop(function, v, data)
                        {
                            ls.push(hook);
                        }
                        ls.push(call("OpFreeRef", v, data));
                    }
                }
            }
            // A generator HANDLE owns its coroutine frame, and through it every heap local
            // the generator body allocated.  `Type::Iterator` was absent from the heap block
            // above, so no scope carried a free for one: a generator whose consumer stopped
            // early never reached the tail where its own `OpFreeRef`s live, and the vector it
            // was walking stayed allocated for the rest of the program (loft#835).  Stopping
            // early is ordinary code — iterating until a match is found and breaking is the
            // main reason to reach for a generator at all.
            //
            // Ownership is unconditional here because a handle is never a view of somebody
            // else's store, so `Type::Iterator` carries no dep list to consult.  A handle
            // being RETURNED is already skipped by the `v == ret_var` test at the top of the
            // loop, and a parameter never enters this sweep, so the caller keeps its own.
            // Freeing an already-exhausted handle is safe: the frame carries a generation
            // stamp the free checks, so a stale handle cannot reach a recycled slot.
            if matches!(function.tp(v).base(), Type::Iterator(_, _)) && !function.is_skip_free(v) {
                ls.push(call("OpFreeRef", v, data));
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
                // The free above is what TRIGGERS the capture cascade (see the
                // `captured_ref` note earlier in this function: the frame's own free of a
                // captured cell is suppressed, so the record's cascade is its sole
                // owner).  That makes this fn-ref's scope the lifetime of everything it
                // captured — which is only right while the fn-ref is not scoped TIGHTER
                // than the record it points at.
                //
                // A closure written in a nested block is exactly that: the binding lives
                // in the block, the `___clos_N` record and the `__bx_<n>` cell are frame
                // vars one scope out.  Freeing through the binding at the block's end
                // then cascaded into a cell the frame still reads —
                // `fn f(n) { { b = fn(k) { n = n + k }; b(1); } n + 4 }` read
                // 0xDEADBEEF under LOFT_POISON, and plausible-looking stale bytes
                // without it.  Both backends, every scalar type, and a captured LOCAL as
                // readily as a parameter.
                //
                // The record carries its own scope-exit free (it is a plain OWNS frame
                // var, not a capture, so nothing suppresses it), and that one runs at the
                // right time.  So when the record sits in a different scope, leave the
                // cascade to it.  A fn-ref with no local record — a parameter, a call
                // result — keeps its free: nothing else would ever release it.
                let record_outlives = function.tp(v).depend().iter().any(|&r| {
                    r != v
                        && (r as usize) < function.count() as usize
                        && self.var_scope.get(&r) != self.var_scope.get(&v)
                });
                let emit = !in_ret && !function.is_skip_free(v) && !record_outlives;
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
        // @PLN87 P2.1 — function exit (`to_scope == 1` is the body scope; every
        // `return` and the tail both free up to it).  For each rebindable heap
        // param, free its CURRENT store iff it differs from the caller-supplied
        // original (witness) captured at entry: a wholesale-reassigned param
        // points at a callee-owned FRESH store (freed here), while an
        // un-reassigned or field-only-mutated param still equals its witness
        // (`OpFreeRefIfDistinct` no-ops, the caller owns + frees it).  The
        // runtime distinctness check makes this sound across conditional and
        // repeated rebinds.  Arguments are deliberately excluded from the normal
        // `variables()` sweep ("never return function arguments"), so this is the
        // sole site that frees a rebound param.  Loop scopes / nested blocks use
        // `to_scope >= 2`, so a `break`/`continue`/block-exit never fires this.
        if to_scope == 1 {
            let free_distinct = data.def_nr("OpFreeRefIfDistinct");
            for (param, orig) in function.rebind_params() {
                ls.push(Value::Call(
                    free_distinct,
                    vec![Value::Var(param), Value::Var(orig)],
                ));
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
        // Peeled for the same reason as its sibling [`Self::find_assigned_vars`], which
        // `scan_if` calls two lines below this one for the same job.  The arms discriminate
        // on `Set` / `Block` / `If` / `Insert`, so a spanned one took `_ => {}` — contributing
        // nothing for that whole subtree, where the shape being looked for is a branch's
        // FIRST assignment of a Reference/Vector/Text, i.e. the thing that decides pre-init.
        //
        // Reachable, and measured latent.  Over the 858-program corpus the peel changes the
        // decision at 46 sites in 16 programs, and at every one of them it newly
        // pre-initialises **0** variables: the same variables were already reaching `result`
        // by another path.  So no emitted code moves today.  The peel stays because it is one
        // word and it obeys `Value::unspan`'s documented rule, and because the reachability is
        // what makes it a trap — span placement has moved before, and the failure mode here is
        // a missing initialisation with nothing to report it.  Claiming a defect was fixed
        // would be the dressed-up version of this result.
        match val.unspan() {
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
    ) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
        let mut preamble: Vec<Value> = Vec::new();
        let mut ls: Vec<Value> = Vec::new();
        // @PLN90 / loft#506 — POST-call store-backs for computed-lvalue `&`-write-back args.
        let mut postamble: Vec<Value> = Vec::new();
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
        // @PLN139 stage C — the lift-site half of [`collect_drop_transferred`].  A copy that
        // hands its source off — the `0x8000` move into a collection element, or a copy
        // whose destination is a container FIELD — must not leave the source dropping what
        // it no longer owns.  When that source is an inline call it is lifted into a
        // `__lift_N` temp right here, so this is the only point that knows which temp the
        // copy took: before the lift the IR still names the CALL, and after it nothing
        // records the pairing.  The copy's source is arg 0.
        let moved_arg = moved_source_arg(outer_call, args, data);
        let transfer_copy = outer_call == data.def_nr("OpCopyRecord") && {
            let moved =
                matches!(args.get(2).map(Value::unspan), Some(Value::Int(tp)) if tp & 0x8000 != 0);
            moved
                || args
                    .get(1)
                    .is_some_and(|d| copy_hands_off(d, function, data))
        };
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
                && self
                    .inline_struct_return(&scanned, data, outer_call, function)
                    .is_none()
                && let Some(tp) = Self::heap_call_return(&scanned, data)
            {
                let tmp = self.new_lift_var(function, &tp);
                // loft#735 — this lift is an ORDERING device, never an ownership
                // transfer.  It fires exactly where `inline_struct_return` said "do
                // NOT lift into an owned temp": the callee delivers through the
                // caller's hidden work-ref (`__ref_N`), which the caller already
                // frees at function exit.  `heap_call_return` hands back the type
                // with `Deps::none()` (a caller temp cannot carry the callee's
                // DEF-space dep), so without this mark `get_free_vars` reads
                // `owns == true` and frees a store the caller still owns — the slot
                // is recycled under the live value and the next write lands in it.
                // `new_lift_var` already sets the allocate-time half of the fact
                // (`inline_ref` — borrow, don't allocate); this is the free-time
                // half.  The hand-correct source shape proves the target: binding
                // the same call to a named local yields `flat:vector<integer>
                // ["__ref_1"]` and NO `OpFreeRef`.
                function.set_skip_free(tmp);
                self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
                continue;
            }
            // A `Span` is source position, not structure. `parse_call` wraps a call
            // argument in one, so an argument this pass rewrote into a preamble-plus-value
            // sequence arrives as `Span(Insert(…))` and the bare-variant match below sees
            // nothing to split — which is how loft#1029's hoisted argument reached the
            // emitters still wrapped, with the lift that owns its result never firing.
            // Peel it for the Insert case only, so every other argument keeps its position.
            let scanned = match scanned {
                Value::Span(b) if matches!(b.1, Value::Insert(_)) => b.1,
                other => other,
            };
            if let Value::Insert(ops) = scanned {
                // Existing A5.6 hoisting: lift Set(w, Null) for owned Reference.
                let is_a56_hoisted = Self::is_null_init_preamble(&ops, function);
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
                //
                // loft#745 — a work-ref that ALREADY holds a value carries its
                // overwrite-free, so the parser's `Set` arrives wrapped as
                // `Insert([OpFreeRef(__ref_N), Set(__ref_N, …)])`.  Matching only the
                // bare `Set` left that shape unhoisted, so the materialisation stayed
                // INSIDE the argument: native then had no `OpCreateStack(Var(_))` to
                // recognise, hoisted the whole argument into a `let _pre_N = …`
                // binding whose value is the assignment's `()`, and rustc rejected the
                // call with E0308 (expected `&mut DbRef`, found `()`).
                let is_p179_hoisted = n >= 2
                    && ops[..n - 1]
                        .iter()
                        .all(|v| Self::is_ref_materialisation(v, function, data))
                    && matches!(&ops[n - 1], Value::Call(d_nr, _)
                        if data.def(*d_nr).name == "OpCreateStack");
                if is_a56_hoisted || is_p135_hoisted || is_p179_hoisted {
                    // @PLN90 / loft#506 — a computed-lvalue `&`-WRITE-BACK arg.  Capture
                    // `items[i]` into a FRESH OWNED temp (so the callee's write-back frees the
                    // COPY, never the caller's element), pass the temp, then copy the result
                    // back into the element after the call.  The element's record is never
                    // freed — it is the stable backing.  A field-mutation callee is untouched.
                    if is_p179_hoisted
                        && let Some((pre, arg, post)) =
                            self.amp_writeback_owned_copy(&ops, arg_idx, outer_call, function, data)
                    {
                        preamble.extend(pre);
                        ls.push(arg);
                        postamble.push(post);
                        continue;
                    }
                    // loft#899 — hoisting the null-init out of the value block moves
                    // the temp's OWNER to the enclosing scope: its declaration now
                    // stands in that statement list, and an argument is only READ, so
                    // no binding adopts the store the way `v = <block>` does.  Nothing
                    // freed it, so every unbound `f#read(n) as vector<T>` leaked one
                    // store.  Re-register it at the current scope for `get_free_vars`,
                    // and run the same hand-off marking a lifted call-result gets so an
                    // argument the callee MOVES from does not drop twice.
                    let a56_owned = if is_a56_hoisted {
                        match &ops[0] {
                            Value::Set(v, _) => Some(*v),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let mut it = ops.into_iter();
                    for _ in 0..n - 1 {
                        preamble.push(it.next().unwrap());
                    }
                    if let Some(v) = a56_owned {
                        self.var_scope.insert(v, self.scope);
                        self.mark_lift_handoff(v, arg_idx, transfer_copy, moved_arg);
                    }
                    let final_val = it.next().unwrap();
                    // the remaining Call may also be struct-returning
                    // (e.g. normalize3(__lift_1) inside add_dir).  Lift it too.
                    if let Some(tp) =
                        self.inline_struct_return(&final_val, data, outer_call, function)
                    {
                        let tmp = self.new_lift_var(function, &tp);
                        self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                        preamble.push(v_set(tmp, final_val));
                        ls.push(Value::Var(tmp));
                    } else {
                        ls.push(final_val);
                    }
                } else if let Some(tp) = ops
                    .last()
                    .and_then(|last| self.inline_struct_return(last, data, outer_call, function))
                {
                    // loft#1029 — an `Insert` whose TAIL is a heap-returning call, which is
                    // what the argument hoist below produces one level down: the ops that
                    // build the inner call's argument, then the call.  The lift that gives
                    // such a result an OWNER matches a `Call`, so the wrapper hid it and the
                    // callee's store was orphaned — `print("{pick(S { a: 7 }, false).a}")`
                    // leaked one record per evaluation on both backends while the same call
                    // BOUND to a local was clean.
                    //
                    // That is @P297's pitfall exactly one wrapper later ("the argument
                    // reaching here is `Span(Call(…))`; unspan before matching or the lift
                    // never fires"), so the cure is the same shape: read through to the
                    // value.  The preamble ops move into the enclosing statement list, where
                    // they already ran, and only the call is lifted — the three recognisers
                    // above split the same way for their own shapes.
                    let mut it = ops.into_iter();
                    let call = it.next_back().expect("checked non-empty by `last`");
                    preamble.extend(it);
                    let tmp = self.new_lift_var(function, &tp);
                    self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                    preamble.push(v_set(tmp, call));
                    ls.push(Value::Var(tmp));
                } else {
                    ls.push(Value::Insert(ops));
                }
            } else if let Some(tp) = self.inline_struct_return(&scanned, data, outer_call, function)
            {
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
                let tmp = self.new_lift_var(function, &tp);
                self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
            } else if let Some((w, absorb)) =
                self.inline_built_borrow_source(&scanned, outer_call, data)
            {
                // loft#1029 — an argument BUILT INLINE (`pick(S { a: 7 }, …)`) that the
                // callee's return may BORROW.  The @P290 bracket that decides borrow-vs-
                // owned at runtime can only name a bare `Var` slot, so an argument still
                // wrapped in its construction block leaves the witness set incomplete
                // (`use_analysis::protectable_ref_args`) and the caller keeps the
                // conservative answer: it COPIES the returned store and orphans the one
                // the callee minted — one leaked record per call, both backends.
                //
                // The slot already exists and this frame already frees it: the parser
                // builds the literal into a function-scope work-ref and the block's tail
                // IS that work-ref.  So nothing needs a new owner — only the CALL SITE
                // needs to be able to say its name.  Hoisting the construction into the
                // preamble and passing `Var(w)` makes the argument nameable, which is
                // exactly the hand-written spelling that was always clean
                // (`q = S { a: 7 }; pick(q, …)`), and the emitted code becomes identical
                // to it.
                //
                // Deliberately NOT done by widening `protectable_ref_args` to see through
                // the block: `protect_store_frees` reads the DbRef VALUE at call time and
                // the bracket is emitted BEFORE the arguments are evaluated, so a work-ref
                // still holding its null would be "protected" while empty — the witness
                // set would read complete while protecting nothing, and the source-free it
                // then licenses would release a store the caller still reaches.  That
                // trades this leak for a use-after-free.
                let Value::Block(bl) = scanned else {
                    unreachable!("inline_built_borrow_source matched a non-Block")
                };
                // The block's own scope disappears with the block, so every var it
                // declared is now declared in the statement list we hoisted into. Move
                // their `var_scope` entries with them: an entry left pointing at a scope
                // no emitted code opens would have slot assignment place it against a
                // sibling scope's zone. Only the VECTOR-literal shape reaches this — a
                // struct literal's tail is the function-scope work-ref itself.
                if let Some(block_scope) = absorb {
                    for sc in self.var_scope.values_mut() {
                        if *sc == block_scope {
                            *sc = self.scope;
                        }
                    }
                }
                let mut ops = bl.operators;
                ops.pop();
                preamble.extend(ops);
                ls.push(Value::Var(w));
            } else if let Some(tp) =
                Self::unnameable_borrow_source(&scanned, outer_call, arg_idx, data)
            {
                // loft#1105 — an argument the @P290 bracket cannot NAME, at a call whose return
                // may borrow it.  The bracket protects a store through a variable holding a
                // `DbRef`, and `view_root_slots` walks a bare `Var`, a projection chain and a
                // JOIN to find one.  A `??` in argument position lowers to an `ncc` BLOCK whose
                // tail is a join with a CALL arm, and neither the multi-statement block nor the
                // call is nameable — so the witness set read incomplete and the caller copied
                // the returned store, orphaning the one the callee minted.
                //
                // Binding it to a temp is the same cure the two cases above take, generalised to
                // the question itself: if the bracket cannot name the value, give it a name.  The
                // preamble runs BEFORE the bracket is emitted, so the temp holds the real `DbRef`
                // by then — which is why the hand-written `e = v[0] ?? mk(); pick(e, …)` was
                // always clean and this now emits the same thing.
                //
                // The temp takes the CALLEE'S PARAMETER type, because that is what the argument
                // is converted to anyway and it is the one type available for a value with no
                // variable behind it.
                let tmp = self.new_lift_var(function, &tp);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
            } else {
                ls.push(scanned);
            }
        }
        (preamble, ls, postamble)
    }

    /// loft#1105 — the TYPE to bind an argument to when the @P290 bracket cannot NAME the store
    /// its value will lie in, at a call whose return may borrow it.
    ///
    /// `Some(tp)` is the callee's PARAMETER shape — the type the argument is converted to
    /// regardless — carrying the DEPS the value itself borrows ([`lift_view_deps`]).
    ///
    /// The deps are the load-bearing half.  The parameter's declared type has none, and a
    /// temp typed that way reads as the OWNER of a store it only VIEWS: `get_free_vars`
    /// emits a scope-exit free that releases the caller's container.  It is silent while the
    /// container is a local of the same frame — the store was dying at that scope exit
    /// anyway — and a use-after-free the moment the container OUTLIVES the call, which is
    /// why `pick(h[k], …)` corrupted a `hash` passed in as a parameter.
    ///
    /// So a value whose source cannot be named is NOT bound (`lift_view_deps` answers
    /// `None`), and the argument stays exactly as it was — the leak that is already there,
    /// which is the better of the two.
    ///
    /// Gated as its two siblings are, plus one exclusion of its own: a bare `Var` is already
    /// nameable and must not be re-bound, and an argument the bracket CAN name needs nothing.
    /// The inline-construction and tuple-element cases are tried first and handle their shapes
    /// more precisely — a construction is HOISTED rather than bound, because binding a work-ref
    /// that still holds null at bracket-emit time would read as covered while protecting
    /// nothing (loft#981).
    fn unnameable_borrow_source(
        arg: &Value,
        outer_call: u32,
        arg_idx: usize,
        data: &Data,
    ) -> Option<Type> {
        if outer_call == u32::MAX {
            return None;
        }
        let callee = data.def(outer_call);
        if !callee.is_loft_defined() {
            return None;
        }
        if matches!(callee.returned().base(), Type::Function(_, _, _)) {
            return None;
        }
        if !callee.returns_borrowed_view() {
            return None;
        }
        if matches!(arg.unspan(), Value::Var(_)) {
            return None;
        }
        if crate::use_analysis::bracket_can_name(data, arg) {
            return None;
        }
        let tp = callee.attributes().get(arg_idx)?.typedef.clone();
        // Only an argument that CARRIES a store needs a witness at all.
        if !crate::data::is_dbref(&tp) {
            return None;
        }
        Some(tp.with_deps(&Deps::frame(lift_view_deps(arg, data)?)))
    }

    /// loft#1029 — the work-ref an INLINE-built argument yields, when the callee's return
    /// may borrow it.
    ///
    /// `Some(w)` for a value block that fills a work-ref and ends in it — the shape the
    /// parser gives `S { … }` / a collection literal in argument position — at a call whose
    /// return names a visible parameter (`returns_borrowed_view`). `w` is a function-scope
    /// slot this frame already allocates and frees, so hoisting the block moves nothing's
    /// ownership; it only lets the call site NAME the borrow source.
    ///
    /// Gated on `returns_borrowed_view` on purpose. Every other call is already correct as
    /// it stands, and hoisting an argument reorders it relative to its left-hand siblings —
    /// a cost worth paying only where the alternative is a leak.
    fn inline_built_borrow_source(
        &self,
        arg: &Value,
        outer_call: u32,
        data: &Data,
    ) -> Option<(u16, Option<u16>)> {
        // `scan_args` runs for argument lists with no enclosing DEF as well (the
        // `u32::MAX` no-call sentinel), and `Data::def` asserts on it. Nothing to decide
        // there: with no callee there is no return that could borrow the argument.
        if outer_call == u32::MAX {
            return None;
        }
        let callee = data.def(outer_call);
        // Only a call into a LOFT-DEFINED body, which is what the @P290 copy-or-adopt
        // bracket serves. A native accessor answers a borrowed view too (`OpGetText` is
        // `text[v1]`), but it never goes through that machinery, so hoisting its argument
        // buys nothing — and it is reached in EXPRESSION position, where the preamble is
        // not a statement list: the hoisted ops were rendered into the argument parens and
        // `--native` rejected the call with E0277 (`((), (), (), (), &str): AsRef<str>` in
        // 875-json-absent-text-field).
        if !callee.is_loft_defined() {
            return None;
        }
        // A callee returning a CLOSURE is not a heap return, and `returns_borrowed_view`
        // is documented as a heap-return ownership read: a `Type::Function` return carries
        // `CALLEE_FRAME`-tagged deps (a closure-internal frame var, never an attr index)
        // and its own debug assert says such a dep must not reach it.  This bracket serves
        // heap returns alone — the shape test below names them — so the ownership question
        // is asked only of a callee that has one.  Without the gate, `fn make_adder(b) ->
        // fn(integer) -> integer` tripped that assert before any of its own work ran.
        if matches!(callee.returned().base(), Type::Function(_, _, _)) {
            return None;
        }
        if !callee.returns_borrowed_view() {
            return None;
        }
        let Value::Block(bl) = arg else {
            return None;
        };
        // At least one construction op plus the trailing `Var` — a bare `{ v }` has
        // nothing to hoist and is already a nameable value.
        if bl.operators.len() < 2
            || !matches!(
                bl.result.base(),
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
        {
            return None;
        }
        let Value::Var(w) = bl.operators.last()?.unspan() else {
            return None;
        };
        // The block must MINT ITS STORE INTO A SLOT THAT OUTLIVES THE HOIST. Its result dep
        // names that slot — the owner — and the dep the block already carries is the fact
        // itself, so nothing here keeps a second list of it. (Not `is_work_ref`: that set
        // is the return-delivery materialiser's own register and does not contain the
        // parser's object work-ref — measured, it answers false for the `__ref_1` this very
        // shape builds into.)
        let [owner] = bl.result.depend()[..] else {
            return None;
        };
        // The owner's declaration must ENCLOSE the statement list we are hoisting into, so
        // the moved ops land inside its lifetime and nothing changes about who frees the
        // store. `self.stack` is the chain of scopes currently open, so this asks the real
        // question — a numeric `<=` would not, because scope numbers are allocated in
        // encounter order and an earlier SIBLING also compares less while enclosing nothing.
        if !self.scope_encloses(owner) {
            return None;
        }
        // The tail names either that same owner — `S { … }`, whose block fills `__ref_N`
        // and yields it — or a VIEW the block opened at its own scope: a vector literal
        // fills `__vdb_N` one level up and yields `_vec_N`, a view of it. The second is
        // still ownership-neutral, because the store's owner is `__vdb_N` and that is not
        // moving; what moves is the view's DECLARATION, out of a block that ceases to
        // exist. So the block's scope has to be absorbed into the one we hoist into, or
        // `var_scope` would keep pointing those vars at a scope no emitted code opens and
        // slot assignment would place them against a sibling's zone.
        if self.scope_encloses(*w) {
            Some((*w, None))
        } else if self.var_scope.get(w) == Some(&bl.scope) {
            Some((*w, Some(bl.scope)))
        } else {
            None
        }
    }

    /// Is `v` declared in a scope that is still OPEN — the one being scanned or one
    /// enclosing it?  That is the condition for moving code into the current statement
    /// list and still being inside `v`'s lifetime.
    fn scope_encloses(&self, v: u16) -> bool {
        self.var_scope
            .get(&v)
            .is_some_and(|sc| *sc == self.scope || self.stack.contains(sc))
    }

    /// The A5.6 hoistable preamble: `Insert([Set(v, Null), value])` whose `v` OWNS a
    /// heap store.  It is the shape the `Value::Block` arm returns for a value block
    /// that yields an owned temp — a `#reading file` read, a `??` join — and
    /// [`Self::scan_args`] lifts that `Set` into the enclosing statement list so the
    /// slot's `first_def` lives OUTSIDE the argument expression.
    ///
    /// One home for the question, because two places ask it: `scan_args`, which does
    /// the hoisting, and the `Value::Span` arm of [`Self::scan`], which must drop a
    /// span that would otherwise hide this Insert from `scan_args`' `if let
    /// Value::Insert`.  While only `scan_args` knew the shape, a span-wrapped one
    /// stayed inside the argument: `println("{len(f#read(8) as vector<single>)}")`
    /// left the temp's declaration in an expression slot, which native emitted
    /// literally as a `let` statement inside an argument list — rustc "expected
    /// expression, found `let` statement" — and which nothing then freed (loft#899).
    fn is_null_init_preamble(ops: &[Value], function: &Function) -> bool {
        ops.len() == 2
            && matches!(&ops[0], Value::Set(v, val)
                if matches!(val.as_ref(), Value::Null) && function.tp(*v).is_heap_owned())
    }

    /// True when `v` writes the work-ref that a `&`-argument's `OpCreateStack` then
    /// borrows — the `Set(__ref_N, …)` the parser emits for a non-`Var` `&`-source,
    /// either bare or wrapped with the overwrite-`OpFreeRef` a re-assigned work-ref
    /// carries (loft#745).  `scan_args` hoists these out of the argument so the
    /// work-ref lives at function scope and its slot survives the call.
    fn is_ref_materialisation(v: &Value, function: &Function, data: &Data) -> bool {
        let writes_work_ref = |op: &Value| matches!(op, Value::Set(v_nr, _) if function.name(*v_nr).starts_with("__ref_"));
        match v {
            Value::Set(_, _) => writes_work_ref(v),
            // The free targets the work-ref's PREVIOUS value and belongs with the write,
            // so the pair hoists as one unit.  A wrapper holding anything else is not a
            // materialisation and stays inside the argument.
            Value::Insert(inner) => {
                inner.iter().any(writes_work_ref)
                    && inner.iter().all(|op| {
                        writes_work_ref(op)
                            || matches!(op, Value::Call(d_nr, _)
                                if data.def(*d_nr).name == "OpFreeRef")
                    })
            }
            _ => false,
        }
    }

    /// @PLN90 / loft#506 — a computed-lvalue `&`-WRITE-BACK argument.  The arg is
    /// `Insert([Set(wv, orig), OpCreateStack(wv)])` where `orig` reads a heap lvalue
    /// (`OpGetVector`/`OpGetField`).  When the callee WHOLE-REASSIGNS this `&`-param (a
    /// `Set(param, non-Null)` through the ref — a write-back), the reassignment FREES the
    /// displaced record; if the arg aliased the caller's element that would free the
    /// element (corruption).  So capture `orig` into a FRESH OWNED temp: the callee frees
    /// the COPY, the element's record survives, and after the call the temp's new record is
    /// copied back into the element.  Returns `(preamble, arg, postamble)` — the owned-copy
    /// setup, the `OpCreateStack(tmp)` argument, and the copy-back — or `None` (field
    /// mutation / not a computed lvalue → keep the default aliasing lowering).
    fn amp_writeback_owned_copy(
        &mut self,
        ops: &[Value],
        arg_idx: usize,
        outer_call: u32,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Value, Value)> {
        if outer_call == u32::MAX || ops.len() != 2 {
            return None;
        }
        let Value::Set(wv, orig) = &ops[0] else {
            return None;
        };
        let wv = *wv;
        let orig = orig.unspan().clone();
        if !matches!(&orig, Value::Call(g, _)
            if matches!(data.def(*g).name(), "OpGetVector" | "OpVectorRef" | "OpGetField"))
        {
            return None;
        }
        // Does the callee whole-reassign the `&`-param at this position? (`Set(param, non-Null)`
        // through the ref — NOT `rebind_orig`, which is non-`&` reassignment-locality.)
        let attrs = data.def(outer_call).attributes();
        if arg_idx >= attrs.len() {
            return None;
        }
        let callee_vars = &data.definitions[outer_call as usize].variables;
        let param_var = callee_vars.var(&attrs[arg_idx].name);
        if param_var == u16::MAX || !matches!(callee_vars.tp(param_var), Type::RefVar(_)) {
            return None;
        }
        let mut writes_back = false;
        data.definitions[outer_call as usize]
            .code
            .walk(&mut |node| {
                if let Value::Set(v, rhs) = node
                    && *v == param_var
                    && !matches!(rhs.unspan(), Value::Null)
                {
                    writes_back = true;
                }
            });
        if !writes_back {
            return None;
        }
        let struct_d = match function.tp(wv) {
            Type::RefVar(inner) => match &**inner {
                Type::Reference(d, _) => *d,
                _ => return None,
            },
            Type::Reference(d, _) => *d,
            _ => return None,
        };
        let type_val = Value::Int(i32::from(data.def(struct_d).known_type()));
        let inner_tp = Type::Reference(struct_d, Deps::none());
        // A fresh OWNED temp via `new_lift_var` — registers the slot + scope-exit `OpFreeRef`
        // (var_scope / var_order / lift_vars) and an inline-ref entry init (no alloc); the
        // `OpDatabase` below allocates its store, freed at scope exit.
        let tmp = self.new_lift_var(function, &inner_tp);
        let db = data.def_nr("OpDatabase");
        let cp = data.def_nr("OpCopyRecord");
        let cs = data.def_nr("OpCreateStack");
        let preamble = vec![
            Value::Call(db, vec![Value::Var(tmp), type_val.clone()]),
            Value::Call(cp, vec![orig.clone(), Value::Var(tmp), type_val.clone()]),
        ];
        let arg = Value::Call(cs, vec![Value::Var(tmp)]);
        let postamble = Value::Call(cp, vec![Value::Var(tmp), orig, type_val]);
        Some((preamble, arg, postamble))
    }

    /// Create a `__lift_N` temporary that OWNS an inline call result, so
    /// `get_free_vars` emits its `OpFreeRef` at scope exit.  Registers the
    /// var in the current scope and in `lift_vars` (which drives the
    /// function-entry `Set(v, Null)` slot reservation).  The caller emits
    /// the `Set(tmp, call)` itself — as an arg preamble (`scan_args`) or as
    /// the statement replacing a `Drop` (#490).
    fn new_lift_var(&mut self, function: &mut Function, tp: &Type) -> u16 {
        self.lift_counter += 1;
        let name = format!("__lift_{}", self.lift_counter);
        let tmp = function.add_temp_var(&name, tp);
        function.mark_inline_ref(tmp);
        self.var_scope.insert(tmp, self.scope);
        self.var_order.push(tmp);
        self.lift_vars.push(tmp);
        tmp
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
        if !def.is_loft_defined() {
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
    /// loft#721 — may the aggregate a `CallRef` returns be LIFTED into an owned
    /// temp (and therefore freed)?
    ///
    /// The direct-call branch answers this from the callee's definition; a
    /// `CallRef`'s callee is a runtime value, so the fn-ref TYPE is all that is
    /// available at the call — and it is not enough. Measured: a closure that
    /// calls a struct-returning function and one that hands back a borrowed
    /// element present the SAME signature (`ret_deps=[1]`, one param), because
    /// the dep index lives in the callee's attribute space and says nothing on
    /// its own. Lifting on the type alone frees a borrowed element: a
    /// use-after-free that only `LOFT_POISON=1` makes visible.
    ///
    /// So resolve the target definition instead — `fnref_target` records which
    /// definition each fn-ref variable was assigned — and then ask the SAME
    /// canonical question the direct call asks, `returns_borrowed_view()`.
    /// An unknown or ambiguous target answers `None`: the result is not lifted,
    /// which is the pre-existing behaviour (it leaks, and a leak is recoverable
    /// where a premature free is not).
    fn callref_owned_return(&self, val: &Value, data: &Data, function: &Function) -> Option<Type> {
        let Value::CallRef(v_nr, _) = val.unspan() else {
            return None;
        };
        let d_nr = match self.fnref_target.get(v_nr).copied() {
            Some(d) if d != u32::MAX => d,
            _ => return None,
        };
        let def = data.def(d_nr);
        if def.code == Value::Null || def.returns_borrowed_view() {
            return None;
        }
        // The fn-ref variable's own type is the declared shape; the definition is
        // the authority on what it returns.
        let _ = function;
        let (returned, opt) = def.returned().peel_optional();
        match returned {
            Type::Reference(d, _) => Some(Self::reopt(opt, Type::Reference(*d, Deps::none()))),
            Type::Enum(d, true, _) => Some(Self::reopt(opt, Type::Enum(*d, true, Deps::none()))),
            _ => None,
        }
    }

    /// loft#879 — the shape question `inline_struct_return` asks ("does this call
    /// hand back a store the caller must own, and of what shape?") is about the
    /// BASE type: `Optional(τ)` is a compile-time wrapper over τ's own runtime
    /// layout (@PLN25), so `-> C?` allocates and delivers exactly what `-> C`
    /// does.  Matching the arms below on the unpeeled type therefore answered
    /// "not liftable" for every optional aggregate return, and the result got a
    /// bare stack-pop (`FreeStack`) that never freed the store — one leaked
    /// record per call, unbounded in a loop, on the interpreter.
    ///
    /// The arms peel for the match; this puts the wrapper back on the temp's
    /// type, so a lifted temp is typed exactly like the hand-correct bound form
    /// (`x = pick(1)` → `x: optional(reference(C))` + a scope-exit `OpFreeRef`)
    /// that has always been the clean spelling.  Keeping the `Optional` matters:
    /// the temp may legitimately hold the null sentinel, and it is the bound
    /// form — not the non-optional one — that proves this type flows correctly
    /// through slot assignment, `get_free_vars`, and both backends' codegen.
    fn reopt(was_optional: bool, tp: Type) -> Type {
        if was_optional { Type::optional(tp) } else { tp }
    }

    fn inline_struct_return(
        &self,
        val: &Value,
        data: &Data,
        _outer_call: u32,
        function: &Function,
    ) -> Option<Type> {
        // loft#721 — a closure call is lifted only when its target definition is
        // known AND that definition does not return a borrowed view.
        if let Some(tp) = self.callref_owned_return(val, data, function) {
            return Some(tp);
        }
        // loft#879 — a null-coalesce (`??`) lowers to an `ncc` value-block that
        // assigns the subject to a `__ncc_N` temp and yields either that temp or
        // the default arm's `__ref_N`.  The temp is `skip_free` (the block's
        // result ALIASES it, so freeing at the block would dangle the value the
        // consumer reads), which leaves the subject's store owned by nothing when
        // the block is used INLINE — one leaked record per evaluation,
        // unbounded in a loop.  Binding it first (`x = pick(1) ?? C{}`) has always
        // been clean because the `Set` gives the store an owner; lifting here
        // rewrites the inline spelling into exactly that bound form.
        //
        // REFERENCE results only.  A text ncc temp is already freed in place by
        // the @PLN85 skip_free-orphan pass ([`collect_consumed_ncc_text`]) and a
        // vector one by its own delivery path; both measured clean, and lifting
        // them too would free what those mechanisms free.
        if let Value::Block(bl) = val.unspan()
            && bl.name == "ncc"
            && let (Type::Reference(d_nr, dep), opt) = bl.result.peel_optional()
            && dep.is_empty()
        {
            return Some(Self::reopt(opt, Type::Reference(*d_nr, Deps::none())));
        }
        // @P297 — a USER struct-returning call (`n_*` with a body) passed
        // directly as a call argument is wrapped in `Value::Span` by
        // `parse_call` (and re-wrapped by `scan`), so the argument reaching
        // here is `Span(Call(...))`.  Unspan before matching this branch or the
        // lift never fires and the call-result temporary leaks — the same
        // pitfall `scan_set` was patched for under @P198 (`value.unspan()`).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            // loft#879 — peel `Optional` before asking the shape question; see
            // [`Self::reopt`], which puts it back on the temp.
            let (returned, opt) = def.returned.peel_optional();
            // #549 — a generic monomorph (`t_…`) whose return SHAPE is a concrete
            // aggregate (`f<T>(x) -> (integer,integer)` / `-> Struct` / `-> Enum`)
            // leaks its result store when used inline or discarded: the caller
            // lifts+frees an `n_` aggregate return (below) but historically not a
            // `t_` one, so the fresh store the monomorph allocated via `__retbuf`
            // was orphaned (both backends).  Extend the lift to `t_` — BUT a
            // monomorph LOSES its return dep during specialization, so the
            // dep-based ownership guards here cannot tell a fresh-owned return
            // from a borrowed-arg one (`id<T>(x) -> T { x }` reads as empty-dep =
            // owned and would DOUBLE-FREE if lifted).  The reliable "delivers a
            // fresh owned aggregate" signal a monomorph keeps is the `__retbuf`
            // NRVO parameter: a concrete-aggregate return gets it at signature
            // finalization; a borrowed-reference return never does.  So gate the
            // `t_` extension on `__retbuf`, leaving every `n_` case untouched.
            // loft#1066 — a monomorph with NO `__retbuf` still delivers a fresh store when
            // its body allocated one, and nobody freed it: one leaked record per inline
            // call, N calls leaking N stores, on both backends.  `monomorph_return_is_fresh`
            // answers from the BODY the question the deps cannot — substitution gives every
            // monomorph of `-> T` the same empty dep, so `{ x }` and `{ y: T = x; y }` are
            // indistinguishable to `returns_borrowed_view` and lifting on that would double
            // free the first.  The proof is positive and under-approximating: a shape it
            // cannot read stays unlifted, which costs the leak that was already there.
            let lift_owned_return = def.name.starts_with("n_")
                || (def.name.starts_with("t_")
                    && (def.attr_names.contains_key("__retbuf")
                        || def.monomorph_return_is_fresh()));
            if lift_owned_return && def.code != Value::Null {
                if let Type::Reference(d_nr, _) = returned {
                    return Some(Self::reopt(opt, Type::Reference(*d_nr, Deps::none())));
                }
                // @P303 / #490 — a user fn returning a heap struct-enum that the
                // caller OWNS leaks its result temp when used directly as a call
                // argument (or discarded); lift it like the Reference case above
                // so `get_free_vars` emits its `OpFreeRef`.  The owned-vs-borrowed
                // split is the canonical `returns_borrowed_view()` fact:
                //   - EMPTY dep (`fn mk() -> H { Bytes{…} }`) — fresh, owned.
                //   - HIDDEN work-ref dep (`fn f() -> H { mk().x }`, dep → the
                //     `__ref_N`/`__retbuf` the callee reallocated) — a fresh store
                //     delivered through the caller's hidden buffer; the caller owns
                //     it and the lift's copy-path free (`0x8000` source-free) claims
                //     it exactly like the `h = f()` bound case does.
                // Both are `returns_borrowed_view() == false` → lift.  A dep naming
                // a VISIBLE param (`fn field_of_arg(d) -> H { d.value }`) IS a
                // borrow → true → must NOT lift (freeing it dangles the caller's
                // arg).  Was `dep.is_empty()`, which missed the hidden-work-ref
                // case: on native the caller freed the by-value-stale `__ref_N`
                // (never delivered back) instead of the reallocated store, leaking
                // one enum store per inline use (#490 kt=65).
                if let Type::Enum(d_nr, true, _) = returned
                    && !def.returns_borrowed_view()
                {
                    return Some(Self::reopt(opt, Type::Enum(*d_nr, true, Deps::none())));
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
                if let Type::Function(params, ret, _) = returned {
                    return Some(Self::reopt(
                        opt,
                        Type::Function(params.clone(), ret.clone(), Deps::none()),
                    ));
                }
            }
            // loft#792 — the same lift for a body-less NATIVE global that MINTS a
            // record.  `type_of(x)` / `type_named(n)` lower to `n_reflect_type` /
            // `n_type_named`, which allocate a `TypeInfo` store; passed straight as a
            // call argument the value was bound to nothing, so nothing freed it.
            // Binding it to a local first leaked nothing, which is what made this read
            // as a reflection quirk rather than the missing lift it is.  Native frees
            // the record through its own drop path, so the leak was interpreter-only —
            // and it CASCADES: with a callee that returns a struct holding a freshly
            // built vector, that vector leaked once per call after the first, so a loop
            // calling `f(type_of(x))` grew the heap without bound.
            //
            // The bound is what keeps this sound.  A native has no body to read, so
            // lift only where the answer cannot be anything but a fresh record: the
            // return names a concrete STRUCT, carries no dep, and no parameter has a
            // type that could have supplied one.  That excludes every view-returning
            // native in the stdlib — `hash_sorted(h: reference, …) -> reference` and
            // `parallel_buf_get_ref(i) -> reference` both hand back a borrow, and both
            // return the untyped `reference` rather than a named struct.  The
            // `JsonValue` constructors are struct-ENUMs and keep their own arm above.
            if lift_owned_return
                && def.code == Value::Null
                && let Type::Reference(d_nr, dep) = returned
                && dep.is_empty()
                && data.def_type(*d_nr) == DefType::Struct
                && !def
                    .attributes()
                    .iter()
                    .any(|a| matches!(a.typedef.base(), Type::Reference(p, _) if p == d_nr))
            {
                return Some(Self::reopt(opt, Type::Reference(*d_nr, Deps::none())));
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
            if def.is_loft_defined() {
                // loft#879 — peel `Optional` for the match, restore it on the temp
                // ([`Self::reopt`]).  The dep guards keep reading the BASE's dep,
                // which is where a borrowed view records itself.
                let (returned, opt) = def.returned.peel_optional();
                match returned {
                    Type::Vector(elem, dep) if dep.is_empty() => {
                        return Some(Self::reopt(opt, Type::Vector(elem.clone(), Deps::none())));
                    }
                    // @PLN85 p188 — a discarded (or inline-unbound) owned KEYED
                    // collection return (`build() -> sorted<T[k]>`, and the
                    // index/hash/spatial siblings) leaks its by-value store exactly
                    // like the vector case above — the `Drop` lift binds it to a
                    // `__lift_N` temp so `get_free_vars` emits the store's
                    // `OpFreeRef` (both backends).  Empty dep = OWNED (fresh); a
                    // borrowed view / NRVO'd hidden-buffer return carries a
                    // non-empty dep and is excluded (no double-free / UAF).
                    Type::Sorted(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(
                            opt,
                            Type::Sorted(*d, keys.clone(), Deps::none()),
                        ));
                    }
                    Type::Index(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(
                            opt,
                            Type::Index(*d, keys.clone(), Deps::none()),
                        ));
                    }
                    Type::Hash(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(opt, Type::Hash(*d, keys.clone(), Deps::none())));
                    }
                    Type::Radix(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(
                            opt,
                            Type::Radix(*d, keys.clone(), Deps::none()),
                        ));
                    }
                    Type::Trie(d, key, dep) if dep.is_empty() => {
                        return Some(Self::reopt(opt, Type::Trie(*d, key.clone(), Deps::none())));
                    }
                    _ => {}
                }
            }
        }
        // Native-constructor calls arrive BARE when chained onto a builtin
        // (`v.keys().len()`) and Span-wrapped when passed as a call argument or
        // method receiver (`jt(json_parse(x), n)`, `json_parse(x).field(n)`), so
        // match through the span — an unlifted native-constructor temp owns a
        // fresh store nothing ever frees (#490).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            // loft#879 — peel `Optional` for the match, restore it on the temp
            // ([`Self::reopt`]).
            let (returned, opt) = def.returned.peel_optional();
            // Native struct-enum constructors: no body (code == Null), return type
            // is a struct-enum with empty dep (allocates a new store, doesn't borrow).
            // Accessors carry dep=[0] after parser dep-inference and are skipped here.
            if def.code == Value::Null
                && let Type::Enum(d_nr, true, dep) = returned
                && dep.is_empty()
            {
                return Some(Self::reopt(opt, Type::Enum(*d_nr, true, Deps::none())));
            }
            // Native vector-returning fns (e.g. `keys()`, `fields()` on
            // JsonValue) allocate a fresh vector store that the caller owns.
            // Without lifting, the chained call `v.keys().len()` leaks the
            // intermediate vector — same mechanism as the struct-return case.
            if def.code == Value::Null
                && let Type::Vector(elem, dep) = returned
                && dep.is_empty()
            {
                return Some(Self::reopt(opt, Type::Vector(elem.clone(), Deps::none())));
            }
        }
        None
    }
}

/// Does a variable of this type need its slot established at the PARENT scope
/// when an `if`/`else` assigns it in a branch?
///
/// The question is really "is this variable backed by a heap store".  One that
/// is cannot be treated like a scalar written in both arms: `scan_if` registers
/// such a scalar at the parent scope directly (`small_both`), and for a
/// store-backed variable that hoists the OWNERSHIP without ever creating the
/// store, so the scope-exit free meets a stack-record ref where it expects an
/// owned heap store.
///
/// The KEYED collections were missing, and every one of them crashed the
/// interpreter for it: two arms of one `if`/`else` chain declaring the same
/// `hash` / `sorted` / `index` name gave `BUG (#306): a stack-record ref was
/// treated as an owned heap store`, then a SIGSEGV once the branch appended.
/// `vector` was in the list and so was fine, which is why the fault looked
/// type-specific rather than like the omission it was.  `--native` computes the
/// answer separately and was always right, so nothing outside the interpreter
/// changed.
fn needs_pre_init(tp: &Type) -> bool {
    matches!(
        tp,
        Type::Text(_) | Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
    ) || crate::parser::vectors::is_keyed(tp)
}

/// @PLN85 text-tail-return-leak — after a B5-L3 `__ret_N` COPY hoist
/// (`Set(__ret_N, expr)` lowers to `OpAppendText`, a deep copy), any `__work_N`
/// text temp that `expr` reads is now dead: the caller consumes the `__ret_N`
/// copy, not `__work_N`.  `wrap_value_text_dest` synthesises that work-text
/// precisely so it CAN be freed, but as the return terminal its scope-exit free
/// is suppressed (it looked like the returned value — `ret_var`).  Emit the free
/// HERE, at the copy, so it fires ONLY when a copy actually happened; the
/// direct-transfer path (fast-path `Return(Var(__work_N))`, no `__ret_N`) reaches
/// neither this nor a free and correctly leaves `__work_N` for the caller.  Fixes
/// the tail native-text-CALL leak (and the `-> text?` freed-then-read UAF) without
/// touching the direct-transfer shapes attempt 1 broke.  See
/// plans/85-store-lifetime-retirement/text-tail-return-leak.md.
fn free_copied_work_texts(result: &mut Vec<Value>, expr: &Value, function: &Function, data: &Data) {
    let mut srcs = Vec::new();
    collect_return_sources(expr, data, &mut srcs);
    for w in srcs {
        if function.name(w).starts_with("__work_") && matches!(function.tp(w).base(), Type::Text(_))
        {
            result.push(call("OpFreeText", w, data));
        }
    }
}

fn call(to: &'static str, v: u16, data: &Data) -> Value {
    Value::Call(data.def_nr(to), vec![Value::Var(v)])
}

/// @PLN125 arc B — the scope-end hook `v`'s type declares, as a call on `v`.
///
/// > **A drop runs exactly where the value's own `OpFree*` runs — the same binding, the
/// > same scope exit, the same early-exit paths — and never anywhere else.**
///
/// That is the whole design, and phrasing it that way is what makes it small: loft already
/// COMPUTES the fact.  The ownership model decides per binding whether this scope owns the
/// value and whether it dies here, which is what puts an `OpFreeRef` in this list; a
/// returned or borrowed value is already excluded, and the early-`return`, `break` and
/// return-out-of-a-loop paths are already handled here (loft#731 exists because a hand-
/// rolled version of exactly those went wrong).  So the drop DERIVES from the borrow model
/// rather than sitting beside it: there is one answer to "when does this run", not two that
/// can drift.
///
/// Scope is honest and narrow: a **binding this scope owns**.  A droppable that is a FIELD
/// of another record is released by that record's cascade, which is not this list, so it
/// does not fire — a hook that ran from two different mechanisms would be the drift this
/// design exists to avoid.
///
/// **The free is null-tolerant and a drop is not**, which is the one place "where the free
/// runs" needed sharpening.  `OpFreeRef` on a slot that was never written is a no-op — it
/// checks `rec == 0` and returns — so the emitter has never had to know whether a binding
/// actually holds anything.  A drop is a USER call, and running it on an unwritten slot
/// runs the author's rollback against a record that does not exist:
///
/// ```loft
/// if n > 0 { t = Tx { … } }     // the else path never writes `t`
/// ```
///
/// printed `[drop null]` before the guard.  So the call is wrapped in the same liveness
/// test the free performs internally (`OpConvBoolFromRef` IS `rec != 0`), which makes the
/// rule *where the free runs, on a value that exists*.  The same guard settles the aliasing
/// case for free: a caller-side `__ref_N` return buffer that the callee did not adopt is
/// null here and correctly does not fire, while one that WAS adopted never reaches this
/// branch at all (it takes the `OpFreeRefIfDistinct` pairing above).
fn drop_hook(function: &Function, v: u16, data: &Data) -> Option<Value> {
    // A struct-enum binding is a heap record exactly as a `Reference` one is — it just
    // carries a discriminator at its head — so it drops the same way. Reading only
    // `Reference` here is why an enum's cascade was synthesized and then never called
    // (@PLN139 stage D).
    let (Type::Reference(d, _) | Type::Enum(d, true, _)) = function.tp(v).base() else {
        return None;
    };
    // @PLN139 — the CASCADE, not the bare hook: for a type that owns droppable members it
    // is the synthesized function that runs the type's own hook and then releases what it
    // owns, and for every other type it IS the bare hook (`Data::drop_cascade_nr`), so a
    // program with no containers is unchanged.
    let nr = data.drop_cascade_nr(*d);
    if nr == u32::MAX {
        return None;
    }
    let live = Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(v)]);
    Some(Value::If(
        Box::new(live),
        Box::new(Value::Call(nr, vec![Value::Var(v)])),
        Box::new(Value::Null),
    ))
}

/// @PLN85 skip_free-orphan (case a): collect the `skip_free` text `__ncc_N` temps
/// whose null-coalesce value-block is nested (as a sub-expression) inside `node` —
/// i.e. the temps this statement CONSUMES IN PLACE.  Descends through expression
/// constructs (`Call`/`If`/`Insert`/…) and INTO `ncc`-named value-blocks (to reach
/// a nested `??`), but STOPS at any other `Block`/`Loop`: those run their own
/// `convert` and free their own temps, so descending would double-free.  A bare
/// `Set(__ncc, …)` outside an `ncc` block (the temp's own declaration inside the
/// ncc block) is deliberately NOT matched — only the value-block's presence counts,
/// which is why the ncc block's own `convert` attributes no free (its statement is
/// the declaration, not a nested ncc consumer).
fn collect_consumed_ncc_text(node: &Value, function: &Function, out: &mut Vec<u16>) {
    match node {
        Value::Span(b) => collect_consumed_ncc_text(&b.1, function, out),
        Value::Block(bl) if bl.name == "ncc" => {
            for op in &bl.operators {
                if let Value::Set(v, val) = op.unspan()
                    && function.is_skip_free(*v)
                    && matches!(function.tp(*v).base(), Type::Text(_))
                    && function.name(*v).starts_with("__ncc_")
                    // Only the REAL coalesce-subject assignment (a Call / field
                    // access / nested block — a producer of an owned String)
                    // gets an in-place free.  A right-nested `??` (`a ?? (b ?? c)`)
                    // hoists a merge-var pre-declaration `__ncc_N = ""` (a literal
                    // Text init) into the OUTER ncc block while the real
                    // assignment lives in the inner block; collecting the literal
                    // init too freed the temp twice (156 sibling: right-nested
                    // `??` double-free).  A subject is never a bare literal.
                    && !matches!(val.unspan(), Value::Text(_) | Value::Null)
                {
                    out.push(*v);
                }
            }
            // Do NOT recurse INTO this ncc block: a nested `??` (`a ?? b ?? c`)
            // lowers to an ncc block whose Set value is ANOTHER ncc block, and
            // that inner block gets its OWN `convert` (and thus its own in-place
            // free pass) when it is scanned.  Recursing here would ALSO collect
            // the inner block's `__ncc_*` temp from the outer level, freeing it
            // twice (a `text.rs:334` double-free on the interpreter for a chained
            // `??` whose first operand is an owned/call-produced text; 156).
            // Non-ncc structures (call args, if-branches) are still descended
            // through by the `_` arm below, so sibling / nested-in-expression ncc
            // blocks are reached exactly once.
        }
        Value::Block(_) | Value::Loop(_) => {}
        _ => node.for_each_child(&mut |c| collect_consumed_ncc_text(c, function, out)),
    }
}

/// #316 — what kind of store does the RHS of a `Set` into a Reference var
/// yield?  Derived from the `ownership_of` oracle (@PLN90 fold): `Own::Owned`
/// maps to `Owned`, `Borrowed`/`Join` to `View` — the oracle's `_ => Owned`
/// fallback means there is no third "unprovable" case at this site.
enum RefRhs {
    /// A store the variable will own (safe to free on a later transition).
    Owned,
    /// A borrowed view into someone else's store (must never be freed).
    View,
}

/// If `op` is a scope-exit free (`OpFreeRef` / `OpFreeText` /
/// `OpFreeRefIfDistinct`), return the var it frees.  The scopes-side twin of
/// `pre_eval::free_op_var` (generation is not depended on from here).
/// @PLN35 sub-class B — insert `frees` into every arm of an `If`/nested tail, just BEFORE
/// the arm's RESULT value (keeping the result as the tail). Used when a non-hoistable
/// `&text` return's arm allocates a sibling store: the frees run after the allocation
/// inside the arm instead of before the whole `return`. A store null on an arm's path
/// makes its `OpFreeRef` a no-op, so pushing into every arm is safe.
fn push_frees_into_arms(tail: &mut Value, frees: &[Value]) {
    match tail {
        Value::Span(b) => push_frees_into_arms(&mut b.1, frees),
        Value::If(_, then, els) => {
            push_frees_into_arms(then, frees);
            push_frees_into_arms(els, frees);
        }
        Value::Block(bl) => {
            let at = bl.operators.len().saturating_sub(1);
            for (i, fv) in frees.iter().enumerate() {
                bl.operators.insert(at + i, fv.clone());
            }
        }
        Value::Insert(ops) => {
            let at = ops.len().saturating_sub(1);
            for (i, fv) in frees.iter().enumerate() {
                ops.insert(at + i, fv.clone());
            }
        }
        leaf => {
            // A bare result value — wrap `[frees…, value]` in an `Insert` (a flat statement
            // sequence whose value is its last element).
            let v = std::mem::replace(leaf, Value::Null);
            let mut ops: Vec<Value> = frees.to_vec();
            ops.push(v);
            *leaf = Value::Insert(ops);
        }
    }
}

impl Scopes {
    /// loft#722 — the variables a block's RESULT may point INTO.
    ///
    /// `OpGetField` / `OpGetVector` / `OpGetEnum` read into their first argument's
    /// record, so a chain of them still points at the variable the chain starts
    /// from — the same "walk the getters to the root" fact loft#666 needed for a
    /// `match` subject. A chain rooted in a CALL produces a value of its own and
    /// roots nothing.
    ///
    /// The result of a `??` block is a variable assigned EARLIER in the block
    /// (`__ncc_N = OpGetVectorNullable(OpGetField(tmp, …))`), so a var is resolved
    /// through its in-block assignment before being reported.
    fn result_borrow_roots(ops: &[Value], data: &Data) -> HashSet<u16> {
        // var -> what its assignment points into, for Sets seen in this block.
        let mut from: HashMap<u16, u16> = HashMap::new();
        for op in ops {
            if let Value::Set(v, rhs) = op.unspan()
                && let Some(root) = Self::borrow_root(rhs, data)
            {
                from.insert(*v, root);
            }
        }
        // The result is the last op that is not a scope-exit free; take every var
        // it could evaluate to (both arms of an `if`, etc.).
        let Some(result) = ops
            .iter()
            .rev()
            .find(|o| scope_free_op_var(o, data).is_none())
        else {
            return HashSet::new();
        };
        let mut roots = HashSet::new();
        result.walk(&mut |n| {
            if let Some(r) = Self::borrow_root(n, data) {
                let mut cur = r;
                // Follow the in-block assignment chain, bounded by its own size so
                // a cycle cannot spin.
                for _ in 0..=from.len() {
                    roots.insert(cur);
                    match from.get(&cur) {
                        Some(next) if *next != cur => cur = *next,
                        _ => break,
                    }
                }
            }
        });
        roots
    }

    /// The variable a value points INTO, following getter chains; `None` when it
    /// produces a value of its own.
    fn borrow_root(val: &Value, data: &Data) -> Option<u16> {
        match val.unspan() {
            Value::Var(v) => Some(*v),
            // loft#722 — a getter roots the chain only when it RETURNS A BORROW, and
            // the stdlib declaration already says which do: `OpGetField(v1, fld) ->
            // reference[v1]` and `OpGetVector(r, …) -> reference[r]` name the argument
            // they read into, while `OpGetInt(v1, fld) -> integer` names nothing
            // because it copies a scalar out.  Read that declared dep instead of the
            // `OpGet` name prefix: the prefix is a proxy, and it was too wide.
            //
            // `run() -> integer { make2().n }` lowers to `OpGetInt(__lift_1, 0)`, whose
            // result is an integer that cannot point into the temp. Treating it as a
            // borrow hoisted `__lift_1` to the enclosing scope and dropped its
            // block-exit free, so the call's store was never freed — one leaked record
            // per inline struct-returning call, on both backends.
            //
            // Every borrowing getter names its FIRST argument, so the walk itself is
            // unchanged; only which calls enter it.
            Value::Call(d, args)
                if data.def(*d).name().starts_with("OpGet")
                    && matches!(
                        data.def(*d).returned.base(),
                        Type::Reference(_, _)
                            | Type::Vector(_, _)
                            | Type::Text(_)
                            | Type::Enum(_, true, _)
                            | Type::Sorted(_, _, _)
                            | Type::Hash(_, _, _)
                            | Type::Index(_, _, _)
                    ) =>
            {
                args.first().and_then(|a| Self::borrow_root(a, data))
            }
            _ => None,
        }
    }
}

fn scope_free_op_var(op: &Value, data: &Data) -> Option<u16> {
    if let Value::Call(d, args) = op.unspan() {
        let name = data.def(*d).name();
        if matches!(name, "OpFreeRef" | "OpFreeText" | "OpFreeRefIfDistinct")
            && let Some(arg0) = args.first()
            && let Value::Var(v) = arg0.unspan()
        {
            return Some(*v);
        }
    }
    None
}

impl Scopes {
    fn insert_free(
        &mut self,
        block: &Block,
        free: &[Value],
        is_return: bool,
        data: &Data,
        function: &mut Function,
    ) -> Vec<Value> {
        let mut res = Vec::new();
        let mut ls = Vec::new();
        let n = block.operators.len();
        // @PLN35 — the block's RESULT op is the last op that is NOT a scope-exit free.
        // A value-returning block can end in a free of a block-scoped local that was
        // materialised inside a branch (e.g. a `..rest` text read-temp, freed at the
        // common-parent block after the value-producing `if`): that trailing free is not
        // the block's value.  Hoisting it as the result minted `<int> __ret_N =
        // OpFreeText(local)` — an empty result on interp and invalid native (`= ;`).  So
        // when a value-result return-block's LAST op is a scope-free and its real result
        // is a plain value op (not a nested Block), treat that value op as the result and
        // run the trailing free(s) AFTER the hoist.  All other blocks keep `n-1`.
        let result_idx = if is_return
            && block.result != Type::Void
            && n > 0
            && scope_free_op_var(&block.operators[n - 1], data).is_some()
        {
            (0..n)
                .rev()
                .find(|&i| scope_free_op_var(&block.operators[i], data).is_none())
                .filter(|&i| !matches!(&block.operators[i], Value::Block(_)))
                .unwrap_or(n.wrapping_sub(1))
        } else {
            n.wrapping_sub(1)
        };
        let trailing_frees: Vec<Value> = if n > 0 && result_idx + 1 < n {
            block.operators[result_idx + 1..].to_vec()
        } else {
            Vec::new()
        };
        for (o_nr, o) in block.operators.iter().enumerate() {
            if o_nr > result_idx {
                // A trailing scope-free op (collected into `trailing_frees`); it runs
                // after the result hoist below, never as the block's value.
                continue;
            }
            if o_nr == result_idx {
                if let Value::Block(bl) = &block.operators[o_nr] {
                    for v in self.insert_free(bl, free, is_return, data, function) {
                        ls.push(v);
                    }
                } else if block.result == Type::Void || matches!(block.result, Type::Never) {
                    // `Never` joins `Void` here because it is the same SHAPE for free
                    // placement: a block that never completes yields no value, so there
                    // is nothing to hoist into a `__ret_N` and nothing to return — and
                    // the value leg below, having nothing to hoist, emitted the frees
                    // BEFORE the tail.  When that tail is a branch (a `match` whose arm
                    // `return`s is what types the block `never`), the arm that does NOT
                    // return then reads a variable already released: `null(oob)` on
                    // native, and on a droppable a drop before the arm plus a second one
                    // at the `return` — a use-after-free (loft#992).  The two legs below
                    // put the tail where it belongs either way: a tail that
                    // unconditionally returns keeps the frees in front of it, a tail that
                    // may still complete runs first and the frees follow.
                    //
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
                    // @PLN85 poison-green — the B5-L3 invariant extended INTO
                    // block tails: return-site frees must not run before the tail
                    // expression EVALUATES.  The non-block leg (free_vars) has
                    // always hoisted a value tail into a `__ret_N` temp before the
                    // frees; this leg emitted `frees; Return(tail)` — the tail
                    // then evaluated AFTER the frees, and any store it still read
                    // (directly or through an alias no fact carries, e.g. a
                    // fn-ref read out of a struct field calling a closure record
                    // the host struct's free just cascaded) was a use-after-free:
                    // silent stale data without LOFT_POISON, deterministic
                    // garbage with it (the closure/field-capture family).
                    // A bare-Var tail is free-safe (its slot holds the value) —
                    // EXCEPT a `&τ` place ref (`RefVar`): returning it DEREFS the
                    // place DbRef at the Return, after the frees released the
                    // source store (the @PLN87 L3/L4 live-read shapes under
                    // poison).  Hoisting `Set(__ret_N, Var(r))` performs the
                    // deref NOW, before the frees.
                    let tail_needs_eval = match o.unspan() {
                        Value::Null => false,
                        // A `&τ` place ref derefs at the Return (hoist) —
                        // EXCEPT a `&text`: the promoted out-buffer returned
                        // per the text-return contract (alive in the caller;
                        // hoisting it broke native's buffer materialization).
                        Value::Var(v) => matches!(function.tp(*v), Type::RefVar(inner)
                            if !matches!(inner.base(), Type::Text(_))),
                        _ => true,
                    };
                    // Text results take the same hoist with the text-leg
                    // mechanics: the temp's String owns a byte copy, and
                    // `skip_free` keeps its OpFreeText out of the scope exit
                    // (the caller copies bytes immediately on return — the
                    // established `__ret_N` text contract pre_eval/native read).
                    let is_text_result = matches!(block.result.base(), Type::Text(_));
                    // @PLN35 sub-class A — a heap-record / vector return (a `Reference`
                    // struct, a struct-`Enum(_, true)`, or a `Vector`) ALSO takes the hoist:
                    // when the block's tail is an `if`/`match` whose taken arm ALLOCATES a
                    // sibling store (a `..rest` materialisation's `__vdb`), the un-hoisted
                    // `frees; return <if>` emits `OpFreeRef(__vdb)` BEFORE the allocation
                    // inside the return → the store leaks (`ANALYSIS.md`, oracle
                    // FREE-before-ALLOC). Hoisting to a `__ret` temp runs the allocation
                    // first. A DbRef `__ret` is native-safe (the `&text` out-buffer is NOT —
                    // it is excluded above via `tail_needs_eval`).
                    let is_heap_ref_result = matches!(
                        block.result.base(),
                        Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
                    );
                    let mut hoist_tmp: Option<u16> = None;
                    if is_return
                        && (!free.is_empty() || !trailing_frees.is_empty())
                        && (is_value_return_type(&block.result)
                            || is_text_result
                            || is_heap_ref_result)
                        && tail_needs_eval
                        && !expr_ends_in_return(o)
                    {
                        self.ret_temp_counter += 1;
                        let name = format!("__ret_{}", self.ret_temp_counter);
                        let tmp = function.add_temp_var(&name, &block.result);
                        // The hoisted value is the RETURN value (transferred to the caller):
                        // its scope-exit free must NOT fire, else the caller reads a freed
                        // record. Text already does this; a heap ref/vector needs it too.
                        if is_text_result || is_heap_ref_result {
                            function.set_skip_free(tmp);
                        }
                        self.var_scope.insert(tmp, self.scope);
                        self.var_order.push(tmp);
                        ls.push(v_set(tmp, o.clone()));
                        hoist_tmp = Some(tmp);
                    }
                    // The block's OWN trailing scope-frees (after the result op) run first,
                    // then the enclosing scope's `free`; both after the result is hoisted.
                    let mut ret_frees: Vec<Value> = Vec::new();
                    for v in trailing_frees.iter().chain(free.iter()) {
                        // A free of a var the RETURNED tail expression still READS
                        // cannot run before it — that is a use-after-free (the
                        // `?? [literal]` return-tail class: interp read the freed
                        // store silently — LOFT_POISON turns it into a SIGSEGV —
                        // and native crashed on the 65535 sentinel).  Its freeing
                        // is owned INSIDE the expression instead: the return-
                        // delivery materializer consumes an owned-fresh arm local
                        // after its append, on EVERY path (cross-arm frees).  So
                        // the pre-return free is DROPPED here, not moved (even
                        // under the hoist — the materializer already freed the
                        // consumed store inside the expression; re-emitting the
                        // free after the temp would double-free it).
                        if is_return
                            && let Some(fv) = scope_free_op_var(v, data)
                            && o.reads_var(fv)
                            && function.is_arm_consumed(fv)
                        {
                            continue;
                        }
                        ret_frees.push(v.clone());
                    }
                    // @PLN35 sub-class B — a NON-hoistable `&text` (RefVar-text) tail whose `If`
                    // arm ALLOCATES a sibling store: the frees can't run before the return (they
                    // would precede the arm's `OpDatabase` → the store leaks) and the `&text`
                    // out-buffer can't hoist (native `Str::new(&local)` dangle, excluded via
                    // `tail_needs_eval`). Push the frees INTO each arm, just before the arm's
                    // result, so they run AFTER the allocation and the buffer is yielded raw.
                    let is_refvar_text = matches!(block.result.base(),
                        Type::RefVar(inner) if matches!(inner.base(), Type::Text(_)));
                    if hoist_tmp.is_none()
                        && is_return
                        && is_refvar_text
                        && !ret_frees.is_empty()
                        && matches!(o.unspan(), Value::If(_, _, _))
                    {
                        let mut tail = o.clone();
                        push_frees_into_arms(&mut tail, &ret_frees);
                        ls.push(Value::Return(Box::new(tail)));
                    } else {
                        ls.extend(ret_frees);
                        if let Some(tmp) = hoist_tmp {
                            ls.push(Value::Return(Box::new(Value::Var(tmp))));
                        } else if is_return {
                            ls.push(Value::Return(Box::new(o.clone())));
                        } else {
                            ls.push(o.clone());
                        }
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
    // @PLN25: peel `Optional` — a `float?`/`integer?`/… return is the same
    // value-return shape (native reps it as the sentinel-carrying scalar).
    // Without the peel a `-> τ?` tail call with pending free-ops fell through
    // the B5-L3 wrap: the call was emitted as a DISCARDED statement + a
    // fabricated `return null`, which native materialised as `return 0.0`
    // — a silent NON-NULL corruption (the routing elevation-kernel bug).
    // loft#816 — a FLAT pure-value tuple is a value return too.  @PLN135 gave
    // `-> (float, integer)` Rust's own tuple ABI (no synthetic `__tuple`
    // record), and that shape reaches here as a bare `Type::Tuple`; unnamed, an
    // anonymous tail tuple with pending frees fell through to the discard +
    // fabricated `Return(Null)` path and native answered the ZERO initialiser
    // (`return (0.0_f64, 0)`) for the whole tuple.  Silent, and on one backend
    // only — the interpreter read the elements off eval-stack top.
    //
    // Recursing per element is what keeps the bound exact.  A NESTED pure-value
    // tuple qualifies too (loft#817): the wrap emits `Set(__ret_N, <tuple>)` and
    // then reads the temp back, and both halves are nested-aware —
    // `emit_tuple_var_pop_put` writes the leaves and `generate_var` delegates to
    // `emit_tuple_var_push_recursive` to read them.  An element with a lifetime
    // (text / vector / record) is either rewritten to the boxed `Reference`
    // shape upstream or belongs to the @P329 tuple-of-text branch, and neither
    // may be hoisted by a plain `Set` — so those stop the recursion here.
    if let Type::Tuple(elems) = tp.base() {
        return !elems.is_empty() && elems.iter().all(is_value_return_type);
    }
    is_scalar_value_type(tp)
}

/// The scalar half of [`is_value_return_type`]: a type returned in a register,
/// owning nothing on the heap.  Split out so the tuple case reads as "every
/// element is itself a value return", with this as the recursion's base.
fn is_scalar_value_type(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Integer(_)
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Enum(_, false, _)
    )
}

/// loft#754 — a return type delivered as a store POINTER (`DbRef`): a vector, a
/// record, or a struct-enum, including one reached through a `&`-parameter
/// place ref.  Names the half of the return space that neither
/// `is_value_return_type` nor the text branch covers, so a tail expression of
/// this shape is hoisted before the scope frees instead of being dropped.
fn is_heap_return_type(tp: &Type) -> bool {
    match tp.base() {
        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => true,
        Type::RefVar(inner) => is_heap_return_type(inner),
        _ => false,
    }
}

/// loft#793 — does this function receive a hidden return BUFFER argument?
///
/// Only a DENSE heap return is given one: both reservation sites
/// (`parser/mod.rs`, `parser/definitions.rs`) gate on
/// `Reference | Vector | Enum(_, true, _)`.  A NULLABLE heap return (`-> S?`)
/// is not one of those, so it has no buffer and its value can travel back only
/// as the call's own return value — which is what makes a dropped call tail a
/// silently-null answer there rather than a delivered one.
fn has_return_buffer(d_nr: u32, data: &Data) -> bool {
    data.def(d_nr).attributes().iter().any(|a| {
        a.hidden
            && matches!(
                a.typedef,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
    })
}

/// loft#793 — the tail of `expr` is a CALL that carries the block's VALUE, the
/// shape the legacy `Return(Null)` fall-through drops.
///
/// The typed-NULL producers are excluded — `OpNullRefSentinel` and the
/// `OpConv<T>FromNull` family.  For those `Return(Null)` is exactly right, and
/// hoisting one is actively wrong: a `-> StructEnum?` fall-through null is
/// `OpConvEnumFromNull`, whose native form is the `255u8` discriminator
/// sentinel, so binding it to a `DbRef`-typed temp emitted `255u8 as DbRef`
/// (E0605).
fn tail_is_value_call(expr: &Value, data: &Data) -> bool {
    match expr.tail().unspan() {
        Value::Call(d, _) => {
            let name = data.def(*d).name();
            name != "OpNullRefSentinel"
                && !(name.starts_with("OpConv") && name.ends_with("FromNull"))
        }
        Value::CallRef(_, _) => true,
        _ => false,
    }
}

/// A branch whose TAIL is literally null — `Value::Null` or the
/// `OpNullRefSentinel()` fall-through `parse_match` injects.  Deliberately does
/// NOT recurse into `If`: a branch that merely CONTAINS a null sub-arm is not a
/// null terminal (unifying through it would lose the other sub-arm's value).
impl Scopes {
    /// The tail of `expr` is an `If` (a lowered if/match) — its arms deliver
    /// into the assignment target via the arm-unification machinery.
    fn tail_is_branch(expr: &Value) -> bool {
        match expr {
            Value::If(_, _, _) => true,
            Value::Block(bl) => bl.operators.last().is_some_and(Self::tail_is_branch),
            Value::Insert(ops) => ops.last().is_some_and(Self::tail_is_branch),
            Value::Span(b) => Self::tail_is_branch(&b.1),
            _ => false,
        }
    }
}

fn is_null_terminal(expr: &Value, null_nr: u32) -> bool {
    match expr.unspan() {
        Value::Null => true,
        Value::Call(d, _) => *d == null_nr,
        Value::Block(bl) => bl
            .operators
            .last()
            .is_some_and(|o| is_null_terminal(o, null_nr)),
        Value::Insert(ops) => ops.last().is_some_and(|o| is_null_terminal(o, null_nr)),
        Value::Return(inner) | Value::Drop(inner) => is_null_terminal(inner, null_nr),
        _ => false,
    }
}

/// P236 extension (@PLN85 P4-records): `returned_var` with a NULL-arm terminal
/// unifying as a WILDCARD against the other arm's var.  The work-ref null-inits
/// at function entry and a null arm never allocates into it, so
/// `Return(Var(v))` yields the same null the sentinel did — while the PRESENT
/// arm's record now rides the var instead of the freed-TOS channel (the
/// record match/if-arm UAF the poison sweep exposed: the legacy pattern freed
/// the arm's store, then `Return(Null)` handed the caller the freed store's
/// bytes off the eval stack — silently stale without LOFT_POISON, null with).
fn returned_var_null_unified(expr: &Value, null_nr: u32) -> u16 {
    match expr {
        Value::Var(v) => *v,
        Value::Block(bl) => {
            let mut v = u16::MAX;
            for o in &bl.operators {
                v = returned_var_null_unified(o, null_nr);
            }
            v
        }
        Value::Return(inner) | Value::Drop(inner) => returned_var_null_unified(inner, null_nr),
        Value::Insert(ops) => ops
            .last()
            .map_or(u16::MAX, |o| returned_var_null_unified(o, null_nr)),
        Value::If(_, t, f) => {
            let t_var = returned_var_null_unified(t, null_nr);
            let f_var = returned_var_null_unified(f, null_nr);
            if t_var == f_var || (f_var == u16::MAX && is_null_terminal(f, null_nr)) {
                t_var
            } else if t_var == u16::MAX && is_null_terminal(t, null_nr) {
                f_var
            } else {
                u16::MAX
            }
        }
        Value::Span(b) => returned_var_null_unified(&b.1, null_nr),
        _ => u16::MAX,
    }
}

/// @PLN85 cluster II — the SET version of `returned_var`: every terminal var a
/// return expression can yield, INCLUDING all arms of an `If`/`match` (which
/// `returned_var` collapses to `u16::MAX` when the arms differ). These are the
/// function's "return-source" locals — their heap store is transferred to the
/// caller, so the callee must not free them at scope exit.
/// Every variable ANY `return` in this body can hand back — including an EARLY return
/// nested mid-block, which the two helpers below cannot see.
///
/// Both of them answer for the body's TAIL value: `returned_var_null_unified` keeps the
/// last operator's answer and `collect_return_sources` takes a block's last non-free
/// result.  So `if a { return a?; } x` reports only `x`, and the work var the guard arm
/// delivers looks like a local nobody frees — which is exactly what it is NOT, because
/// returning it transfers it to the caller.  `check_ref_leaks` asserted on that shape as a
/// leak (a generic instantiated at a STRUCT, where the return is a heap ref).
#[cfg(debug_assertions)]
fn collect_all_return_vars(expr: &Value, data: &Data, out: &mut Vec<u16>) {
    if let Value::Return(inner) = expr.unspan() {
        collect_return_sources(inner, data, out);
    }
    expr.walk(&mut |v| {
        if let Value::Return(inner) = v {
            collect_return_sources(inner, data, out);
        }
    });
}

fn collect_return_sources(expr: &Value, data: &Data, out: &mut Vec<u16>) {
    match expr {
        Value::Var(v) => {
            if !out.contains(v) {
                out.push(*v);
            }
        }
        // @PLN85 P4-records — a block's VALUE is its last op that is NOT a scope-exit
        // free. A captured struct-enum field binds an owned `_mv_<f>` text whose
        // `OpFreeText` is appended AFTER the arm chain (`[Kw { word }, ..] => LetS{…}`),
        // so `.last()` alone hits that free and hides the record sources — the returned
        // enum store is then freed unconditionally before the return (35c sub-class A,
        // plans/captured-group-elem-uaf.md). Skip trailing frees to reach the real value.
        Value::Block(bl) => {
            if let Some(last) = last_non_free_result(&bl.operators, data) {
                collect_return_sources(last, data, out);
            }
        }
        Value::Return(inner) | Value::Drop(inner) => collect_return_sources(inner, data, out),
        Value::Insert(ops) => {
            if let Some(last) = last_non_free_result(ops, data) {
                collect_return_sources(last, data, out);
            }
        }
        Value::If(_, t, f) => {
            collect_return_sources(t, data, out);
            collect_return_sources(f, data, out);
        }
        Value::Span(b) => collect_return_sources(&b.1, data, out),
        _ => {}
    }
}

/// The last operator of a sequence that carries the block's VALUE — i.e. skipping
/// trailing scope-exit frees (`OpFreeRef` / `OpFreeText` / `OpFreeRefIfDistinct`) and
/// `Line` position markers. A value-returning block can end in a free of a block-local
/// (a captured `_mv_<f>` text, a `..rest` `__vdb`) appended after the result, so the
/// naive `.last()` would mistake that free for the value.
fn last_non_free_result<'a>(ops: &'a [Value], data: &Data) -> Option<&'a Value> {
    ops.iter()
        .rev()
        .find(|op| scope_free_op_var(op, data).is_none() && !matches!(op.unspan(), Value::Line(_)))
}

/// Does this return have an arm that yields something OTHER than one of its
/// `return_sources` — a genuine BORROWING arm (loft#1022)?
///
/// `collect_return_sources` is the UNION of the arms' terminal VARS, so a work-ref that
/// only one arm delivers still lands in it and its scope-exit free is suppressed on
/// every path.  That is right when every arm delivers a source and wrong when one arm
/// hands back a borrow instead: `if take { bx.p } else { P { x: 9 } }` yields a field
/// access on the first arm, which is no variable at all, and the second arm's work-ref
/// is then owned by nobody.
///
/// Answers false for a return with no join in it — one path cannot orphan the value it
/// is itself delivering — and false when every arm's terminal is a source, which is the
/// shape a record literal aliasing locals has.
fn return_has_non_source_arm(expr: &Value, sources: &[u16]) -> bool {
    fn walk(e: &Value, sources: &[u16], in_join: bool) -> bool {
        match e.unspan() {
            Value::If(_, t, f) => walk(t, sources, true) || walk(f, sources, true),
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|o| walk(o, sources, in_join)),
            Value::Insert(ops) => ops.last().is_some_and(|o| walk(o, sources, in_join)),
            Value::Return(inner) | Value::Drop(inner) => walk(inner, sources, in_join),
            Value::Var(v) => in_join && !sources.contains(v),
            _ => in_join,
        }
    }
    walk(expr, sources, false)
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
///
/// One home, because the same fact decides two things at opposite ends of one return:
/// whether scope analysis may suppress the work-ref's free (here), and whether the
/// `Bind` leg may copy the tail into the return buffer WHOLE or has to deliver the arms
/// one at a time (`ref_return`) — the whole-tail copy answers the buffer on every path,
/// so with a null arm present it swallows the sentinel.
pub(crate) fn return_has_null_arm(expr: &Value, null_sentinel_nr: u32) -> bool {
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

/// Block-tail temps whose store is ADOPTED (moved) into a freed assignment LHS,
/// so freeing the LHS frees them — no `OpFreeRef` of the temp is emitted, and
/// none should be (it would double-free the shared record).
///
/// The shape is `Set(lhs, Block[…, Var(v)])`: the block allocates a fresh record
/// into `v` and yields it, and `lhs = <block>` PutRef-aliases that record into
/// `lhs` (the empty-dep adopt — e.g. the `#reading file` surface temp behind
/// `q = f#read as S`).  `v`'s free responsibility transfers to `lhs`, so when
/// `lhs` is freed, `v` is covered.
///
/// Narrow by construction: it matches only a BLOCK-valued RHS (a plain
/// `lhs = v` COPY has an RHS of `Var(v)`, not `Block`, and its `v` is freed
/// separately, so `v` is already in `freed` and never reaches the leak assert).
/// It credits `v` only when `lhs` is in `freed`, so it cannot mask a genuine
/// leak where the adopting LHS itself is never freed.
#[cfg(debug_assertions)]
fn collect_adopted_block_results(ir: &Value, freed: &HashSet<u16>, result: &mut HashSet<u16>) {
    ir.walk(&mut |n| {
        if let Value::Set(lhs, rhs) = n
            && freed.contains(lhs)
            && let Value::Block(bl) = rhs.unspan()
            && let Some(Value::Var(v)) = bl.operators.last().map(Value::unspan)
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

    let ret_var = returned_var_null_unified(ir, data.def_nr("OpNullRefSentinel"));
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

    // A block-tail temp adopted into a freed LHS (`q = f#read as S`, whose
    // `#reading file` surface temp `_read_N` moves its record into `q`) has no
    // OpFreeRef of its own and must not — `q`'s free covers it.  Credit it so
    // the leak assert below does not false-positive on the moved-from source.
    let mut adopted: HashSet<u16> = HashSet::new();
    collect_adopted_block_results(ir, &freed, &mut adopted);

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
    let direct_ret_var = returned_var_null_unified(ir, data.def_nr("OpNullRefSentinel"));
    // Transitive: if the returned variable depends on another variable, that
    // variable's store must also survive — include it in ret_deps.
    if direct_ret_var != u16::MAX {
        for d in function.tp(direct_ret_var).depend() {
            ret_deps.insert(d);
        }
    }
    // …and every variable an EARLY return hands back, which the tail-value helper above
    // cannot see: `if a { return a?; } x` reports only `x`, so the guard arm's work var
    // read as a local nobody freed.  Returning it IS the transfer, wherever the return sits.
    let mut early_ret: Vec<u16> = Vec::new();
    collect_all_return_vars(ir, data, &mut early_ret);
    for v in early_ret {
        ret_deps.insert(v);
        for d in function.tp(v).depend() {
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
        if adopted.contains(&v) {
            continue; // moved into a freed LHS — that free covers this store
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
/// ⚠ NOT WIRED TO ANYTHING.  Nothing calls this outside its own tests, so no program is
/// rejected for an impure par worker today.  That is not a missing gate against the rules:
/// @FR-C-Impure says an impure worker's result is UNDEFINED, not that the compiler must
/// refuse it — the contract is "make the worker pure", and conformance is differential
/// (concurrency.md § Deviations).  This classifier is the DIAGNOSTIC that DESIGN.md D8
/// wants on top of that, and its consumer has never been written.  Keep or delete it as a
/// deliberate choice; do not read the `#[allow(dead_code)]` as a temporary state.
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

/// @PLN35 PC4 — is a sub-rule PURE, hence safe to invoke speculatively and backtrack over?  This is
/// COMPLEMENTARY to par-safety (which bans only parent-store writes): a sub-rule's effect must be
/// UNOBSERVABLE, because a cursor `match` may invoke it even when the arm is not taken (the call is
/// hoisted unconditionally) and, once mixing lands, may backtrack over it.  So the OBSERVABLE /
/// non-deterministic / concurrent categories (host I/O, I/O, prng, par-call) disqualify it; a
/// `parent_write` does NOT (record construction + the cursor advance ARE parent writes — internal
/// and unavoidable), and an un-annotated native builtin is pure (every observable builtin is
/// annotated `#impure`).  Descends the call graph; a recursion cycle is optimistically pure (it must
/// consume through a base case — and PC3 already rejects left-recursive sub-rules).  An indirect
/// `CallRef` is conservatively impure (its target is not statically known).
pub fn sub_rule_is_pure(data: &Data, d_nr: u32) -> bool {
    let mut visited = HashSet::new();
    walk_sub_rule_pure(data, d_nr, &mut visited)
}

fn walk_sub_rule_pure(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> bool {
    if !visited.insert(d_nr) {
        return true; // cycle — break optimistically (PC3 rejects left recursion separately)
    }
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[d_nr as usize];
    if !matches!(def.def_type, DefType::Function) {
        return false;
    }
    match def.purity {
        Purity::Pure => true,
        // OBSERVABLE / non-deterministic / concurrent effects disqualify a sub-rule.
        Purity::Impure(
            ImpureCategory::HostIo
            | ImpureCategory::Io
            | ImpureCategory::Prng
            | ImpureCategory::ParCall,
        ) => false,
        // A parent-store WRITE is how a sub-rule builds its result record and advances the cursor
        // (OpNewRecord / OpSetInt / OpFinishRecord are `#impure(parent_write)`) — unavoidable and
        // internal, so allowed.  The narrow case of writing EXTERNAL shared state and then
        // backtracking over it is not distinguishable at this category granularity — deferred.
        Purity::Impure(ImpureCategory::ParentWrite) => true,
        Purity::Unknown => {
            if matches!(def.code, Value::Null) {
                // A native builtin with NO annotation is pure: every OBSERVABLE builtin (I/O, host)
                // IS annotated `#impure`, so the un-annotated natives are pure reads / arithmetic /
                // record ops (OpGetField, OpGetVector, OpNeRef, …).
                true
            } else {
                !def.code.any_node(&mut |n| match n {
                    Value::Call(callee, _) => !walk_sub_rule_pure(data, *callee, visited),
                    Value::CallRef(_, _) => true, // indirect call — cannot analyze
                    _ => false,
                })
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

    /// The two classifiers must give the same verdict — one returns a bool, the other the
    /// REASON for it, and a verdict with no reason to give is a contradiction.
    ///
    /// They did not. `par_unsafe_reason` was hand-rolled with a `_ => None` fallback and no
    /// `Return` arm, so a body of `return <call>` was reported par-SAFE without ever being
    /// looked at, while `is_par_safe` (walking with `any_node`, which visits every node)
    /// called the same function unsafe. Measured over the corpus par workers: 16 of 42
    /// disagreed, every one of them `is_par_safe=false` against `reason=none`.
    ///
    /// `Return` is the arm that exposed it; the fallback now descends via `for_each_child`,
    /// so a wrapper nobody thought of cannot reintroduce the gap.
    #[test]
    fn the_two_par_classifiers_agree_through_a_return() {
        let mut d = Data::new();
        let native = d.add_def("OpMulInt", &pos(), DefType::Function);
        d.definitions[native as usize].purity = Purity::Unknown;
        d.definitions[native as usize].code = Value::Null; // a native: no loft body
        let user = d.add_def("dbl", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Return(Box::new(Value::Call(native, vec![])));

        let flag = is_par_safe(&d, user);
        let reason = super::par_unsafe_reason(&d, user);
        assert_eq!(
            flag,
            reason.is_none(),
            "bool says {flag} while the reason is {reason:?} — a verdict with no reason \
             behind it, or a reason behind no verdict"
        );
        assert!(
            reason.is_some_and(|r| r.contains("OpMulInt")),
            "and the reason must NAME the call it found through the `Return`"
        );
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
/// ⚠ NOT WIRED TO ANYTHING — the reason-string twin of [`is_par_safe`], and unreached for
/// the same reason.  See there.
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
        // Every OTHER node descends via the keystone rather than stopping.
        //
        // This arm used to be `_ => None`, and the named arms above do not include `Return` —
        // so `fn dbl(x) { return x * 2; }` was reported par-SAFE without its body ever being
        // looked at, while `is_par_safe` (which walks with `any_node`, so it sees every node)
        // called the same function unsafe.  16 of 42 par workers in the corpus disagreed that
        // way, every one of them `is_par_safe=false` against `reason=none`: a verdict of
        // "unsafe" with no reason to give for it.
        other => {
            let mut found = None;
            other.for_each_child(&mut |c| {
                if found.is_none() {
                    found = walk_par_unsafe_reason_value(c, data, visited);
                }
            });
            found
        }
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
    let worker_name = def.name.strip_prefix("n_").unwrap_or(&def.name).to_string();
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
            // @PLN102 C93 — a RAW field/element write (`OpSet*`) whose accessor root is a
            // NON-LOCAL PARAMETER is a write to captured/parent state: in a `par` worker the
            // captured state is read-only, so this is the race the deep check must reject
            // (the tagged-stdlib ParentWrite path below only catches `+=`/`hash_set`/… — a
            // bare `s.n = v` / `cap[i] = v` lowers to `OpSet*`, a "safe" primitive, and used
            // to slip through into a codegen slot-panic).  A write to a worker-LOCAL (or the
            // hidden return-buffer destination) is fine — same locality test as
            // `first_arg_is_local_var`.
            if let Some(var) = raw_write_to_captured(*callee, args, current_fn, data) {
                return Some(format!(" writes captured `{var}`"));
            }
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
        // Descends via the keystone, for the reason its twin `walk_par_unsafe_reason_value`
        // does: the arms above are the same list, and they do not include `Return`, so a
        // worker whose body is `return helper(...)` was reported free of parent writes without
        // that call ever being examined.  The two walkers are near-copies; fixing one and not
        // the other is how they came to differ from `is_par_safe` in the first place.
        other => {
            let mut found = None;
            other.for_each_child(&mut |c| {
                if found.is_none() {
                    found = walk_deep_parent_write(c, data, current_fn, visited);
                }
            });
            found
        }
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

/// @PLN102 C93 — walk an accessor chain (`OpGet*(base, …)` / `Var`, span-transparent)
/// down to its root variable.  The base of an `OpSet*` write target is arg 0.
fn accessor_root_var(v: &Value, data: &Data) -> Option<u16> {
    match v {
        Value::Var(n) => Some(*n),
        Value::Span(b) => accessor_root_var(&b.1, data),
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.definitions[*d as usize].name.starts_with("OpGet") =>
        {
            args.first().and_then(|a| accessor_root_var(a, data))
        }
        _ => None,
    }
}

/// @PLN102 C93 — is this call a RAW field/element write (`OpSet*`) whose accessor root is
/// a NON-LOCAL PARAMETER of `current_fn` (captured/parent state)?  Returns the captured
/// var's name if so.  A write to a worker-LOCAL variable, or to the hidden return-buffer
/// destination (a promoted `ref_return` output), is safe.  Mirrors `first_arg_is_local_var`.
fn raw_write_to_captured(
    callee: u32,
    args: &[Value],
    current_fn: u32,
    data: &Data,
) -> Option<String> {
    if (callee as usize) >= data.definitions.len()
        || !data.definitions[callee as usize].name.starts_with("OpSet")
    {
        return None;
    }
    let root = args.first().and_then(|a| accessor_root_var(a, data))?;
    if current_fn == u32::MAX || (current_fn as usize) >= data.definitions.len() {
        return None;
    }
    let def = &data.definitions[current_fn as usize];
    if !def.variables.is_argument(root) {
        return None; // a worker-local write — fine
    }
    let name = def.variables.name(root).to_string();
    // hidden return-buffer destination = the worker's own output, not parent state.
    if def.attributes.iter().any(|a| a.hidden && a.name == name) {
        return None;
    }
    Some(name)
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
                Some(format!(
                    " → {}",
                    def.name.strip_prefix("n_").unwrap_or(&def.name)
                ))
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
                walk_deep_parent_write(&def.code, data, callee, visited).map(|chain| {
                    format!(
                        " → {}{}",
                        def.name.strip_prefix("n_").unwrap_or(&def.name),
                        chain
                    )
                })
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

    /// A parent write reached only through a `Return` must still be found.
    ///
    /// `walk_deep_parent_write`'s arms are the same list as its twin
    /// `walk_par_unsafe_reason_value`'s, and neither included `Return` — so a worker whose
    /// body is `return helper(...)` was reported free of parent writes without that call ever
    /// being examined. Both now descend via the keystone; this pins the half that would
    /// otherwise drift back the moment one is edited alone.
    #[test]
    fn deep_parent_write_is_found_through_a_return() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Return(Box::new(Value::Call(bad, vec![])));
        assert!(
            worker_calls_parent_write_deep(&d, worker).is_some(),
            "a parent write behind a `return` must still be reported"
        );
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
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
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
        Value::Return(v) | Value::Yield(v) => escapes_value(v, target),
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
/// stronger one had silently lost a child arm).  `dom` = "every read
/// of `local` here is preceded by a non-null assignment that definitely
/// executed".  The two gates differ ONLY in:
/// - the START value (`confine_reassign_safe` starts false — a definite
///   reassignment must precede any read; `store_dead_after_block` starts
///   true — the fn-level init counts);
/// - `invalidate_conditional` (`store_dead_after_block` only): a
///   reassignment inside an `If`/`Loop`/`Iter` body INVALIDATES
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
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
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
/// @PLN35 `..rest` store-lifetime OBSERVER (reached only via `LOFT_REST_ORACLE`; never
/// rewrites IR). For every `__vdb` store it re-runs the `store_confinement` gates in
/// REPORTING mode and prints the verdict — CONFINED to a block, or REJECTED with the
/// exact gate — so a leak can be attributed to a precise decision (e.g. the ambiguous
/// dep-backer gate that blocks the escaping-field `..rest` shape). Diagnostic only.
fn rest_store_oracle(code: &Value, vars: &Function, free_ref_nr: u32, db_nr: u32, fn_name: &str) {
    let mut any = false;
    for vdb in 0..vars.count() {
        if !vars.name(vdb).starts_with("__vdb") {
            continue;
        }
        // Backers: every var whose dep carries this store.
        let mut backers: Vec<u16> = Vec::new();
        for v in 0..vars.count() {
            if vars.tp(v).depend().contains(&vdb) {
                backers.push(v);
            }
        }
        let user_backers: Vec<u16> = backers
            .iter()
            .copied()
            .filter(|&v| !vars.name(v).starts_with('_'))
            .collect();
        let temp_backers: Vec<u16> = backers
            .iter()
            .copied()
            .filter(|&v| vars.name(v).starts_with('_'))
            .collect();
        let arg_cap = backers
            .iter()
            .any(|&v| vars.is_argument(v) || vars.is_captured(v));
        if !any {
            eprintln!("[rest-oracle] fn={fn_name}");
            any = true;
        }
        let bnames: Vec<String> = backers.iter().map(|&v| vars.name(v).to_string()).collect();
        // Walk the same gate ladder store_confinement uses, but report the stop.
        let verdict = if arg_cap {
            "REJECT(arg/captured backer)".to_string()
        } else if backers.len() > 1 {
            // The gate that blocks the escaping-field `..rest` shape: the store is
            // dep-backed by the user vector AND a `_`-temp (`_elm`/`_comp`).
            format!(
                "REJECT(ambiguous: {} backers — user={:?} temp={:?})",
                backers.len(),
                user_backers
                    .iter()
                    .map(|&v| vars.name(v))
                    .collect::<Vec<_>>(),
                temp_backers
                    .iter()
                    .map(|&v| vars.name(v))
                    .collect::<Vec<_>>(),
            )
        } else if let Some(&local) = backers.first() {
            if vars.is_skip_free(local) || vars.is_skip_free(vdb) {
                "REJECT(skip_free — treated as escaping/borrowed)".to_string()
            } else if guard_escapes(code, local) {
                "REJECT(escapes: return/yield/break/element)".to_string()
            } else {
                let multi_store = vars.tp(local).depend().len() != 1;
                if multi_store && !confine_reassign_safe(code, local) {
                    "REJECT(multi-store, reassign-unsafe)".to_string()
                } else {
                    let span_target = if multi_store { vdb } else { local };
                    let mut stack: Vec<(u16, bool)> = Vec::new();
                    let mut lca: Option<Vec<(u16, bool)>> = None;
                    guard_refs(code, span_target, free_ref_nr, &mut stack, &mut lca);
                    match lca {
                        None => "REJECT(no ref LCA)".to_string(),
                        Some(path) if path.iter().any(|&(_, is_loop)| is_loop) => {
                            "REJECT(loop in LCA path — per-iteration reuse)".to_string()
                        }
                        Some(path) => match path.last() {
                            Some(&(b, _)) if vars.scope(vdb) == b => {
                                format!("already fn/block scope {b} (frees there)")
                            }
                            Some(&(b, _)) => format!("CONFINE to block {b}"),
                            None => "REJECT(empty LCA path)".to_string(),
                        },
                    }
                }
            }
        } else {
            "REJECT(no dep-backer — orphaned store)".to_string()
        };
        // THE DIRECT LEAK PREDICTOR (independent of confinement): does OpFreeRef(vdb)
        // execute BEFORE OpDatabase(vdb)? A value-type return hoists the allocating arm
        // into a `__ret` temp so the free lands after; a Reference / promoted-&text return
        // is NOT hoisted, so the free precedes the allocation → the store leaks. Pre-order
        // index of each op (Return(expr)→[expr], If(c,t,e)→[c,t,e]) approximates exec order.
        let mut ctr = 0usize;
        let mut alloc_at: Option<usize> = None;
        let mut free_at: Option<usize> = None;
        preorder_op_index(
            code,
            vdb,
            db_nr,
            free_ref_nr,
            &mut ctr,
            &mut alloc_at,
            &mut free_at,
        );
        let order = match (free_at, alloc_at) {
            (Some(f), Some(a)) if f < a => "FREE-before-ALLOC → LEAKS".to_string(),
            (Some(_), Some(_)) => "alloc-before-free → clean".to_string(),
            (Some(_), None) => "free, no alloc (null-only) → n/a".to_string(),
            (None, Some(_)) => "alloc, no free → LEAKS".to_string(),
            (None, None) => "neither → n/a".to_string(),
        };
        let ret_ty = block_result_type(code);
        eprintln!(
            "  store {} scope={} backers={bnames:?} ret={ret_ty} conf={verdict}",
            vars.name(vdb),
            vars.scope(vdb),
        );
        eprintln!("      free/alloc order: {order}");
    }
}

/// Pre-order (execution-approximating) index of the FIRST `OpDatabase(vdb)` and the FIRST
/// `OpFreeRef(vdb)` in `node`. `Return(expr)` unfolds to its inner (expr evaluates first);
/// `If(c,t,e)` visits cond then arms. Reporting helper for [`rest_store_oracle`].
fn preorder_op_index(
    node: &Value,
    vdb: u16,
    db_nr: u32,
    free_nr: u32,
    ctr: &mut usize,
    alloc_at: &mut Option<usize>,
    free_at: &mut Option<usize>,
) {
    *ctr += 1;
    if let Value::Call(op, args) = node.unspan()
        && let Some(Value::Var(v)) = args.first().map(Value::unspan)
        && *v == vdb
    {
        if *op == db_nr && alloc_at.is_none() {
            *alloc_at = Some(*ctr);
        }
        if *op == free_nr && free_at.is_none() {
            *free_at = Some(*ctr);
        }
    }
    node.for_each_child(&mut |c| preorder_op_index(c, vdb, db_nr, free_nr, ctr, alloc_at, free_at));
}

/// The result `Type` of a function body: the top-level `Block`'s declared result.
/// Reporting helper (the return type is the free-analysis hoist discriminator).
fn block_result_type(code: &Value) -> String {
    match code.unspan() {
        Value::Block(b) => {
            // The exact hoist gate the free-analysis (`insert_free`) uses.
            let hoists =
                is_value_return_type(&b.result) || matches!(b.result.base(), Type::Text(_));
            format!(
                "{:?}{}",
                b.result,
                if hoists { " (hoists)" } else { " (NO-hoist)" }
            )
        }
        _ => "<non-block>".to_string(),
    }
}

/// loft#750 — the result is a `BTreeMap`, and the ORDER is load-bearing.  Its
/// caller relocates each confined `__vdb`'s null-init, and a relocation that
/// cannot reach its block puts the init back at body position 0; run over
/// several confined stores, the visit order therefore PERMUTES the null-inits
/// at the head of the body — which moves the stack slots under them.  A
/// `HashMap` gave that order Rust's per-process hash seed, so compiling one
/// file twice with one binary produced different bytecode and different slots
/// (same answers, but no reproducible `--native` build, and a byte-identical-IR
/// inertness gate that could not tell "my change did nothing" from "the seed
/// moved").  Keyed by variable number, the visit order is now the declaration
/// order.
fn store_confinement(
    code: &Value,
    vars: &Function,
    free_ref_nr: u32,
    gf_nr: u32,
) -> BTreeMap<u16, (u16, u16)> {
    // Plan-57 cluster-III Route 2: recover the backer of an orphaned (overwritten) store so
    // its per-block store can confine.  DEFAULT ON since 2026-08-21; `LOFT_NO_CONF_RECOVER=1`
    // emits the pre-Route-2 form and is the first bisect step for a wrong answer in a
    // function that reassigns a local across sibling blocks.
    //
    // A local reassigned across sibling `if`/`else if`/`match` arms otherwise keeps EVERY
    // arm's store to scope exit, so the store watermark grows with the number of
    // reassignment SITES and not with how many of them run: a 16-site function measured
    // peak 20 whichever single arm was taken, against a flat 5 with this on.
    //
    // What makes it safe is `store_dead_after_block`, not this flag: a local READ after the
    // blocks does not confine, because freeing a confined block store while the local still
    // holds it returns the wrong element on the branch that did NOT run.  That shape refuted
    // the first gate design and is pinned by
    // `tests/scripts/reassign-across-sibling-blocks.loft`, which asserts the same answers
    // with the recovery on and off.
    let recover = std::env::var("LOFT_NO_CONF_RECOVER").is_err();
    let mut out: BTreeMap<u16, (u16, u16)> = BTreeMap::new();
    for vdb in 0..vars.count() {
        if !vars.name(vdb).starts_with("__vdb") {
            continue;
        }
        // A caller-provided return buffer arrives as a PARAMETER, so it is not this
        // function's store to confine — and "free it at the inner block's exit" would
        // have the callee free the CALLER's buffer.  The backer check below rejects an
        // argument BACKER but never asked whether the store itself is one.
        //
        // `return <vector local>` lowers to exactly that shape: the incoming `__vdb`
        // becomes the buffer and the named local a field-view of it
        // (`buf = OpGetField(__vdb_1, …); return __vdb_1`), so the local is a non-arg
        // backer of an arg store and every such function was reported.  That is the
        // most ordinary vector idiom in the language — `cbor`'s `head` is the shape
        // that surfaced it — so the report was noise wherever anyone looked.
        if vars.is_argument(vdb) {
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
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
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
        // If / Loop / Parallel / Iter — conditional, so an alloc inside is
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
        Value::Return(v) | Value::Yield(v) | Value::Drop(v) => holder_retained(v, holders),
        Value::TuplePut(_, _, v) => holder_retained(v, holders),
        Value::Iter(_, c, n, e) => {
            holder_retained(c, holders)
                || holder_retained(n, holders)
                || holder_retained(e, holders)
        }
        Value::Span(b) => holder_retained(&b.1, holders),
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
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
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
