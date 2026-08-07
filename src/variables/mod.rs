// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I61 — Stack slot allocator

//! Per-function variable table and stack slot assignment.
//!
//! Each function being compiled gets a [`Function`] that tracks every
//! variable (name, type, scope, liveness interval, stack slot) and every
//! iterator/loop in the function body.
//!
//! ## Dependency tracking
//!
//! The `dep` field on [`Type`](crate::data::Type) controls ownership:
//! - **empty** → the variable *owns* its heap value (freed at scope exit).
//! - **non-empty** → the variable *borrows* from a parameter listed by
//!   attribute index (not freed — the caller owns the store).
//!
//! [`Function::depend`] adds a dependency when the parser discovers that a
//! local variable borrows from a parameter (e.g. field access on a reference
//! argument).
//!
//! ## Slot assignment
//!
//! After scope analysis, [`assign_slots`] assigns each variable a byte
//! offset on the stack using a two-zone layout:
//! - **Zone 1** (pre-claimed): small types (≤ 16 bytes) packed per-block.
//! - **Zone 2** (sequential): large types (text, references) allocated in
//!   the order they first appear.
//!
//! See `slots.rs` for the algorithm.

mod intervals;
mod slots_v2;
mod validate;

pub use intervals::compute_intervals;
// @PLAN53 — the aligned V2 allocator is the only allocator; scopes.rs drives
// it directly via `assign_slots_v2` + `apply_v2_result`.
#[allow(unused_imports)]
pub use slots_v2::{AllocatorResult, SlotAssignment, SlotKind, apply_v2_result, assign_slots_v2};
pub use validate::{dump_var_tables, dump_variables};
// Plan-04 Phase 2e: ungate validate_slots so LOFT_SLOT_V2=validate
// shadow mode can invoke it from any build profile (integration
// tests compile against loft without debug_assertions).  The
// call site in state/codegen.rs remains gated on
// `#[cfg(any(debug_assertions, test))]` — validate_slots is
// unconditionally *compiled*, but only *called* automatically
// during unit tests; shadow mode opts in at runtime via env var.
// The profile.dev.package.loft override disables debug_assertions
// in the hot interpreter path; clippy sees no in-crate caller and
// would otherwise flag this re-export.
#[allow(unused_imports)]
pub use validate::{validate_alignment, validate_slots};

use crate::data::{Context, Data, Deps, Type, Value};
use crate::diagnostics::{Level, diagnostic_format};
use crate::keys::DbRef;
use crate::lexer::Lexer;

/// True iff `name` contains at least one uppercase letter and no
/// lowercase letters.  Used by `warn_upper_case_locals` (P246
/// follow-up): `_`, `_foo`, and `123` should NOT trip the warning,
/// but `FOO`, `MAX_SIZE`, `X` should.  Distinct from the parser's
/// `is_upper`, which accepts pure-numeric names like `42` (no
/// lowercase chars at all but also no uppercase).
fn is_upper_case_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut has_upper = false;
    for c in name.chars() {
        if c.is_lowercase() {
            return false;
        }
        if c.is_uppercase() {
            has_upper = true;
        }
    }
    has_upper
}
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
    /// @PLN102 strict-index lint — for a `for i in 0..len(X)` range, the `VecKey` of `X`
    /// (the vector whose length bounds this loop). `Some` only when the range's upper bound
    /// is a bare `len(<addressable vector>)`; `None` for `0..n`, slices, or collection loops.
    /// Read by the gated `LOFT_LINT_STRICT_INDEX` warning to flag `w[i]` where `w != X`.
    len_bound: Option<crate::parser::operators::VecKey>,
}

/// @PLAN28 C4 — borrowed view of the codegen-read fields of one `Variable`,
/// produced by [`Function::snapshot_var`] for the snapshot encoder.
#[allow(clippy::struct_excessive_bools)] // mirrors `Variable`'s codegen-read flags
pub(crate) struct VarSnapshot<'a> {
    pub name: &'a str,
    pub type_def: &'a Type,
    pub stack_pos: u16,
    pub uses: u16,
    pub argument: bool,
    pub stack_allocated: bool,
    pub skip_free: bool,
    pub captured: bool,
    pub caller_hidden_buf: bool,
}

/// @PLAN28 C4 — owned codegen-read fields of one `Variable`, consumed by
/// [`Function::from_snapshot`] on the decode path.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct RestoredVar {
    pub name: String,
    pub type_def: Type,
    pub stack_pos: u16,
    pub uses: u16,
    pub argument: bool,
    pub stack_allocated: bool,
    pub skip_free: bool,
    pub captured: bool,
    pub caller_hidden_buf: bool,
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
    /// Binding-const (`const` PREFIX): `const x` local, `const p: T` param, and the
    /// field twin `Attribute.const_field`.  The slot is write-once — a rebind (`=`)
    /// is rejected — but the value it holds stays mutable (`+=` / element write are
    /// allowed).  @PLN40 const-model phase 1; see doc/claude/plans/40-const-fields/.
    const_binding: bool,
    /// Value-const (`const` before the TYPE): `p: const T` param, `x: const T` local.
    /// A read-only borrow of the value — every mutation THROUGH this name (`+=`,
    /// element, field, nested) is rejected, but a rebind (`=`) that re-points the
    /// slot is allowed.  The sibling of the `&T` mutable borrow.  @PLN40 phase 1.
    value_const: bool,
    /// @PLN130 F9 — this binding was spelled with `&` at a STRUCT-typed projection
    /// (`c = &v[0]`, `c = &o.inner`).  Such a projection is already a VIEW under B-View,
    /// so both spellings lower to byte-identical IR and the `&` used to be dropped as
    /// redundant.  It stopped being redundant when F2 made a view MATERIALISE on a
    /// reshape: from then on `&` also means *"and do not silently copy it"*, which is
    /// information the IR no longer carries.  A marker rather than `Type::RefVar` on
    /// purpose — RefVar would re-route every read and write through the double
    /// indirection parameters use, slowing every access to carry a compile-time fact.
    /// See loft#779 / `formal/binding.md` D-bind-8.
    amp_link: bool,
    /// Whether this variable's stack storage has been initialised by codegen.
    /// Set to `true` when the first-allocation init opcodes are emitted (A6.3).
    /// Arguments are pre-allocated by the caller, so they start as `true`.
    pub stack_allocated: bool,
    /// When true, `get_free_vars` must not emit `OpFreeRef` for this variable.
    /// Set by `clean_work_refs` for work-ref temporaries that have been re-purposed
    /// and must not be freed at scope exit (A14 replacement for type-mutation hack).
    pub skip_free: bool,
    /// Variable is captured by a closure.  Suppresses the "never read"
    /// warning in `test_used` without affecting the dead-assignment uses counter.
    pub captured: bool,
    /// Sequence number of the first `Value::Set` node for this variable; `u32::MAX` = never defined.
    pub first_def: u32,
    /// Sequence number of the last `Value::Var` (or implicit `OpFreeText`/`OpFreeRef`) for this variable.
    pub last_use: u32,
    /// Slot assigned by `assign_slots` before codegen may override it via `set_stack_pos`.
    /// `u16::MAX` means `assign_slots` has not run yet.  Shown as `pre:` in `validate_slots`
    /// diagnostics when it differs from the final `stack_pos`.
    pub pre_assigned_pos: u16,
    /// If this variable is a shadow local promoted from a text argument,
    /// `promoted_from` holds the var_nr of the original argument. `u16::MAX` = not promoted.
    pub promoted_from: u16,
    /// C61.local: set by `loop_var()` when this variable has served as
    /// the body variable of a `for <id> in …` loop.  Survives slot reuse
    /// across sequential loops in the same function, letting
    /// `parse_for_iter_setup` distinguish a safe sequential reuse from
    /// an outer-local shadow.
    pub was_loop_var: bool,
    /// @PLAN51 Cluster IV: set by `add_defaults` when it synthesises a
    /// caller-side work-ref for a callee's hidden return-buffer arg.
    /// These work-refs are allocated by the parser as call-site placeholders
    /// (the caller pre-allocates the buffer, the callee writes into it),
    /// so they need a leading `Set(r, Null)` IR so the slot allocator sees
    /// a `first_def` and assigns a stack slot.  Without the null-init, vars
    /// whose typedef has a non-empty dep list (e.g. `Reference(td, [arg_idx])`
    /// for if-tail / recursion shapes) skip the dep-empty guard in
    /// `parse_code` and end up SKIP'd by `assign_slots` ("no first_def") —
    /// codegen then panics with "Incorrect var __ref_N[65535]" when it tries
    /// to emit the call's arg.
    pub caller_hidden_buf: bool,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub file: String,
    /// Per-prefix counters for `unique()` temp names (`_<prefix>_<n>`).
    /// Per-PREFIX (not one shared counter) so a temp family created on only
    /// one parser pass (e.g. a second-pass-only lowering temp) cannot shift
    /// the numbering of every OTHER family between passes — a shifted name
    /// re-resolves to a pass-1 var of a different TYPE in `add_variable`
    /// (#320's frame-drift: an integer-typed `__ncc_N` holding a DbRef).
    unique: HashMap<String, u16>,
    pub(crate) current_loop: u16,
    loops: Vec<Iterator>,
    variables: Vec<Variable>,
    /// Variables whose type came from an EXPLICIT `: Type` annotation (not inferred
    /// from an assignment).  An annotated narrow integer (`x: u8`) stays constrained;
    /// an INFERRED local widens to the join of its assignments — the `(I-Join)` rule
    /// that closes the #433-residual (see `parse_assign_op`).
    annotated: HashSet<u16>,
    work_text: u16,
    /// Separate counter for the caller-allocated `&text` out-param buffers
    /// `caller_text_buf()` mints — the text twin of `work_vdb` below, and for
    /// the same reason (loft#662).  A call only needs one once the callee's
    /// `&text` ABI exists, which for a self-/forward-recursive callee is not
    /// until pass 1 has promoted it; sharing `work_text` would shift the
    /// `__work_N` names relative to pass 1 and break `text_return`'s name-based
    /// attr matching.
    work_ctext: u16,
    /// loft#665 piece 2 — the PASS-2-ONLY `__work` sequence.  A mint site that can
    /// only fire on pass 2 must not draw from `work_text`: doing so shifts every
    /// later `__work_N` relative to pass 1, and because the variable tables persist
    /// BY NAME, pass 2's buffers then re-find pass 1's variables under the wrong
    /// roles (loft#662).  Pass-availability is a STATIC property of each mint site
    /// (measured: 19 both-pass sites, 15 pass-2-only, none mixed), so the split is
    /// decidable at the call site.
    work_text_p2: u16,
    work_ref: u16,
    // Separate counter for vector-db work-refs created by `vector_db()`.
    // `vector_db` only runs on the second pass (first_pass guard), so it cannot
    // use the shared `work_ref` counter: that would shift the counter relative to
    // the first pass and break `ref_return`'s name-based attr matching.
    work_vdb: u16,
    /// loft#703 — separate counter for the work-ref a KEYED collection literal in value
    /// position builds into.  Its own namespace for two reasons: `__ref_N` is reserved
    /// for return buffers, whose name-based `ref_return` match a shared counter would
    /// shift; and `__vdb_N` means "a WRAPPER record holding a vector field", which
    /// `store_confinement` reads as "some other local backs this store, confine both to
    /// that local's block".  A keyed collection has no wrapper — the accumulator IS the
    /// value — so the only local depending on it is an ELEMENT inside it, and confining
    /// the store to the element's block left the block's own result unfreed.
    work_kvb: u16,
    /// @PLN124 — separate counter for the accumulator a format string BUILDS when
    /// its target type implements the interpolation contract (`"…{x}…"` into a
    /// `SqlText` rather than into text).
    ///
    /// Its own namespace for the same reason every counter above has one: the
    /// variable tables persist across passes BY NAME, so a mint that fires on
    /// only some strings must not shift anyone else's numbering (loft#662).  It
    /// is not a `__work_N` because it is not a text buffer, and not a `__ref_N`
    /// because that stem is reserved for return buffers whose `ref_return` match
    /// is name-based.  Like `__kvb_N`, the accumulator IS the value — no wrapper
    /// record backs its store — so it is function-scoped and the exit sweep frees
    /// it.
    work_fmt: u16,
    // Work variables for texts
    work_texts: BTreeSet<u16>,
    // Work variables for stores
    work_refs: BTreeSet<u16>,
    /// Vars the return-delivery materializer CONSUMED inside a branch arm
    /// (free-after-append on every path, incl. cross-arm frees).  Gates
    /// `insert_free`'s reads-filter: a pre-return free is dropped ONLY when
    /// the returned tail still reads the var AND an in-arm free covers it —
    /// dropping on reads alone leaks any read-but-unconsumed buffer (the
    /// `?? call()` hidden __ref, gate-ON elem_accumulate).
    arm_consumed: BTreeSet<u16>,
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
    // maps fn_ref_var_nr → closure_var_nr for native codegen.
    closure_var_map: HashMap<u16, u16>,
    /// @PLN87 P2.1 — reassignment-locality.  Maps a user-visible heap PARAMETER
    /// (whole-binding-reassigned in the body) → its `__orig` witness var, a
    /// skip-free work-ref holding the param's caller-supplied DbRef captured at
    /// function entry.  A rebind frees the param's CURRENT store only when it
    /// differs from this witness (`OpFreeRefIfDistinct`), so the caller's
    /// original store is never freed by the callee and a fresh rebind store is.
    /// Parse-time only: on a snapshot load `scopes::check` is skipped (the frees
    /// are already in `code`), so this map is not part of the snapshot.
    rebind_orig: HashMap<u16, u16>,
}

/// @PLN104 — swap membership of two indices in a set (for `Function::swap_variables`).
/// An index in the set moves to the other; if both or neither are present, no change.
fn swap_in_bset(s: &mut BTreeSet<u16>, a: u16, b: u16) {
    let (ha, hb) = (s.contains(&a), s.contains(&b));
    if ha != hb {
        if ha {
            s.remove(&a);
            s.insert(b);
        } else {
            s.remove(&b);
            s.insert(a);
        }
    }
}

fn swap_in_hset(s: &mut HashSet<u16>, a: u16, b: u16) {
    let (ha, hb) = (s.contains(&a), s.contains(&b));
    if ha != hb {
        if ha {
            s.remove(&a);
            s.insert(b);
        } else {
            s.remove(&b);
            s.insert(a);
        }
    }
}

/// Swap `a`/`b` in BOTH the keys and the values of a var→var map.
fn swap_map_indices(m: &mut HashMap<u16, u16>, a: u16, b: u16) {
    let swap1 = |x: u16| {
        if x == a {
            b
        } else if x == b {
            a
        } else {
            x
        }
    };
    *m = m.iter().map(|(&k, &v)| (swap1(k), swap1(v))).collect();
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
            unique: HashMap::new(),
            current_loop: u16::MAX,
            loops: Vec::new(),
            work_text: 0,
            work_ctext: 0,
            work_text_p2: 0,
            work_ref: 0,
            work_vdb: 0,
            work_kvb: 0,
            work_fmt: 0,
            variables: Vec::new(),
            annotated: HashSet::new(),
            work_texts: BTreeSet::new(),
            work_refs: BTreeSet::new(),
            arm_consumed: BTreeSet::new(),
            inline_ref_vars: BTreeSet::new(),
            names: HashMap::new(),
            loop_scopes: HashSet::new(),
            loop_seq_ranges: HashMap::new(),
            scope_origins: HashMap::new(),
            logging: false,
            done: false,
            closure_var_map: HashMap::new(),
            rebind_orig: HashMap::new(),
        }
    }

    // ─── @PLAN28 startup-cache snapshot seam (C4) ────────────────────────
    //
    // The variable table is stored in the snapshot as DEBUG SYMBOLS plus the
    // FINAL `stack_pos`.  The codec lives in `src/ir_schema.rs`; these
    // `pub(crate)` accessors give it field access without exposing the private
    // struct internals project-wide.  Only the fields codegen READS are
    // stored (`name`/`type_def`/`stack_pos`/`uses`/`argument`/
    // `stack_allocated`/`skip_free`/`captured`/`caller_hidden_buf`, plus the
    // `names` map and `inline_ref_vars` set).  Fields codegen never reads
    // (`scope`, the slot scratch `pre_assigned_pos`/`first_def`/`last_use`,
    // the parse-time `work_*` counters) are NOT stored — `scopes::check` is
    // skipped on load (it ran before the snapshot, and re-running would
    // double-insert the free-ops already in `code`).

    /// Number of variables (snapshot encode helper).
    #[must_use]
    pub(crate) fn snapshot_len(&self) -> usize {
        self.variables.len()
    }

    /// The nine codegen-read fields of variable `i`, for the snapshot encoder.
    #[must_use]
    pub(crate) fn snapshot_var(&self, i: usize) -> VarSnapshot<'_> {
        let v = &self.variables[i];
        VarSnapshot {
            name: &v.name,
            type_def: &v.type_def,
            stack_pos: v.stack_pos,
            uses: v.uses,
            argument: v.argument,
            stack_allocated: v.stack_allocated,
            skip_free: v.skip_free,
            captured: v.captured,
            caller_hidden_buf: v.caller_hidden_buf,
        }
    }

    /// The `names` map entries (name → var_nr), for the snapshot encoder, in a
    /// STABLE total order (by `var_nr`, then name).
    ///
    /// `self.names` is a `HashMap`, so iterating it raw yields a run-to-run
    /// varying order — which made the serialized variable-name list, and thus the
    /// cached `Data` (the store codec) AND the JSON codec, **non-reproducible**:
    /// two names sharing a `var_nr` (a text param and its `__tp_` first-mutation
    /// promotion shadow) flipped order between runs, so a fresh parse and a
    /// store/JSON round-trip of the same program disagreed.  Sorting here is the
    /// chokepoint — both codecs encode from this, so both become deterministic.
    /// The order is non-load-bearing (debug symbols, looked up by name), so the
    /// sort is free of behaviour change.
    #[must_use]
    pub(crate) fn snapshot_names(&self) -> Vec<(&str, u16)> {
        let mut out: Vec<(&str, u16)> = self.names.iter().map(|(k, &v)| (k.as_str(), v)).collect();
        out.sort_unstable_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        out
    }

    /// The `inline_ref_vars` set, for the snapshot encoder.
    #[must_use]
    pub(crate) fn snapshot_inline_refs(&self) -> Vec<u16> {
        self.inline_ref_vars.iter().copied().collect()
    }

    /// Reconstruct a `Function` from a snapshot (C4 decode).  Every field
    /// codegen does not read is filled with its post-parse default — harmless
    /// because `scopes::check` is not re-run on the load path.
    #[must_use]
    pub(crate) fn from_snapshot(
        name: &str,
        file: &str,
        vars: Vec<RestoredVar>,
        names: Vec<(String, u16)>,
        inline_refs: Vec<u16>,
    ) -> Function {
        let mut f = Function::new(name, file);
        f.variables = vars
            .into_iter()
            .map(|r| Variable {
                name: r.name,
                type_def: r.type_def,
                stack_pos: r.stack_pos,
                uses: r.uses,
                argument: r.argument,
                stack_allocated: r.stack_allocated,
                skip_free: r.skip_free,
                captured: r.captured,
                caller_hidden_buf: r.caller_hidden_buf,
                // codegen-irrelevant post-parse defaults (not stored):
                source: (0, 0),
                scope: u16::MAX,
                uses_at_write: 0,
                write_source: (0, 0),
                defined: false,
                const_binding: false,
                value_const: false,
                amp_link: false,
                first_def: u32::MAX,
                last_use: 0,
                pre_assigned_pos: u16::MAX,
                promoted_from: u16::MAX,
                was_loop_var: false,
            })
            .collect();
        f.names = names.into_iter().collect();
        f.inline_ref_vars = inline_refs.into_iter().collect();
        // A snapshot is taken AFTER `scopes::check` ran (the stored `code`
        // already carries its free-ops); mark the reconstructed function
        // `done` so a load-path `scopes::check` skips it and does not
        // double-insert those frees (@PLN11 arc D / @PLAN28 C4 insight).
        f.done = true;
        f
    }

    pub fn append(&mut self, other: &mut Function) {
        self.current_loop = u16::MAX;
        self.logging = other.logging;
        self.unique.clear();
        other.unique.clear();
        self.loops.clear();
        self.loops.append(&mut other.loops);
        self.variables.clear();
        self.variables.append(&mut other.variables);
        for v in &mut self.variables {
            v.uses = 0;
        }
        self.work_text = 0;
        self.work_ctext = 0;
        self.work_text_p2 = 0;
        self.work_ref = 0;
        self.work_vdb = 0;
        self.work_kvb = 0;
        self.work_fmt = 0;
        self.work_texts.clear();
        self.work_refs.clear();
        self.arm_consumed.clear();
        self.arm_consumed.clone_from(&other.arm_consumed);
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
        self.closure_var_map.clear();
        self.closure_var_map.clone_from(&other.closure_var_map);
        other.closure_var_map.clear();
        // @PLN87 P2.1 — carry the parsed pass's rebind-witness map into the
        // stored function so `scopes::check` can emit the function-exit
        // `OpFreeRefIfDistinct`.  Cleared first so a re-parse can't leave a
        // stale param→witness entry.
        self.rebind_orig.clear();
        self.rebind_orig.clone_from(&other.rebind_orig);
    }

    pub fn copy(other: &Function) -> Self {
        Function {
            name: other.name.clone(),
            file: other.file.clone(),
            current_loop: u16::MAX,
            unique: HashMap::new(),
            loops: other.loops.clone(),
            variables: other.variables.clone(),
            annotated: other.annotated.clone(),
            arm_consumed: other.arm_consumed.clone(),
            work_text: 0,
            work_ctext: 0,
            work_text_p2: 0,
            work_ref: 0,
            work_vdb: 0,
            work_kvb: 0,
            work_fmt: 0,
            work_texts: BTreeSet::new(),
            work_refs: BTreeSet::new(),
            inline_ref_vars: other.inline_ref_vars.clone(),
            names: other.names.clone(),
            loop_scopes: other.loop_scopes.clone(),
            loop_seq_ranges: other.loop_seq_ranges.clone(),
            scope_origins: other.scope_origins.clone(),
            logging: other.logging,
            done: other.done,
            closure_var_map: other.closure_var_map.clone(),
            rebind_orig: other.rebind_orig.clone(),
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
            len_bound: None,
        });
        self.current_loop = self.loops.len() as u16 - 1;
        self.current_loop
    }

    pub fn loop_var(&mut self, variable: u16) {
        self.loops[self.current_loop as usize].variable = variable;
        // C61.local: track that this variable has served as a for-loop
        // variable at some point in the function.  Used to distinguish
        // a sequential for-loop reuse (safe) from an outer-local shadow
        // (silent clobber) at parse-for time.
        if (variable as usize) < self.variables.len() {
            self.variables[variable as usize].was_loop_var = true;
        }
    }

    /// C61.local (planned): reserved for the future liveness-aware
    /// diagnostic that rejects `x = 5; for x in …` when the outer `x`
    /// has a live read after the loop.  Unused today — kept so the
    /// `was_loop_var` flag on Variable has a read path and is not
    /// dead-code-linted away before the diagnostic ships.
    #[allow(dead_code)]
    pub fn was_loop_var(&self, var_nr: u16) -> bool {
        if var_nr == u16::MAX || (var_nr as usize) >= self.variables.len() {
            return false;
        }
        self.variables[var_nr as usize].was_loop_var
    }

    pub fn set_loop(&mut self, on: u8, db_tp: u16, value: &Value) {
        // D-key-1: a keyed range / partial-key subscript used in a VALUE position (not a
        // `for`/comprehension iterable) reaches `fill_iter` with no active loop.  Skip the
        // loop-slot write instead of indexing `loops[u16::MAX]` and panicking — `parse_key`
        // emits a clean "keyed slice is a for-only iterator" diagnostic that aborts the
        // compile before this never-consumed loop state could matter.
        if self.current_loop == u16::MAX || self.current_loop as usize >= self.loops.len() {
            return;
        }
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

    /// @PLN102 strict-index lint — record the vector whose `len(...)` bounds the current
    /// loop's range (`for i in 0..len(X)` → `X`'s `VecKey`). No-op when there is no active
    /// loop. Set from `parse_in_range_body` right after the range's upper bound is parsed.
    pub(crate) fn set_loop_len_bound(&mut self, vk: crate::parser::operators::VecKey) {
        if self.current_loop != u16::MAX && (self.current_loop as usize) < self.loops.len() {
            self.loops[self.current_loop as usize].len_bound = Some(vk);
        }
    }

    /// @PLN102 strict-index lint — the `len(...)` bound recorded for the active for-loop
    /// whose variable is `var_nr` (walks the active-loop chain like `is_active_loop_var`).
    /// `None` when `var_nr` is not an active loop var or the loop wasn't a `0..len(X)` range.
    pub(crate) fn loop_len_bound(&self, var_nr: u16) -> Option<crate::parser::operators::VecKey> {
        if var_nr == u16::MAX {
            return None;
        }
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return self.loops[c as usize].len_bound;
            }
            c = self.loops[c as usize].inside;
        }
        None
    }

    /// C61: returns true when `var_nr` is the loop *variable* (the `<id>`
    /// bound by `for <id> in …`) of any currently active for-loop,
    /// including outer loops.  Used to detect nested same-name loops.
    pub fn is_active_loop_var(&self, var_nr: u16) -> bool {
        if var_nr == u16::MAX {
            return false;
        }
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return true;
            }
            c = self.loops[c as usize].inside;
        }
        false
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
        // Plan-07 phase 1: compare via unspan() so the iterated-collection
        // expression `db.items` (parsed once for the for-loop) and the
        // mutating expression `db.items` (parsed again at the `+=` site)
        // — each potentially wrapped at a different source position —
        // still compare equal.
        let unspanned = val.unspan();
        let mut c = self.current_loop;
        while c != u16::MAX {
            if *self.loops[c as usize].value.unspan() == *unspanned {
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

    /// Register `count_var` as the `#count` of the loop whose iteration variable
    /// is `loop_var` — searched OUTWARD from the current loop, not assumed to be
    /// it.
    ///
    /// loft#794 — `#count` vars are minted on first READ, and a read of an OUTER
    /// loop's `#count` happens while the parser sits in an INNER loop. Stamping
    /// the current loop then re-pointed the inner loop's counter at the outer
    /// loop's variable: with only the outer one read the inner loop silently
    /// incremented the WRONG counter, and with both read in the same body the
    /// inner loop's own count var was left with no init and no stack slot, which
    /// aborted the compiler ("Incorrect var q#count[65535]") on both backends.
    ///
    /// A `loop_var` that names no enclosing loop keeps the current-loop
    /// behaviour — a `#count` on something that is not an enclosing iteration
    /// variable is already diagnosed elsewhere, and this is not the place to
    /// change what it compiles to.
    pub fn loop_count_of(&mut self, loop_var: u16, count_var: u16) {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == loop_var {
                self.loops[c as usize].counter = count_var;
                return;
            }
            c = self.loops[c as usize].inside;
        }
        if self.current_loop != u16::MAX {
            self.loops[self.current_loop as usize].counter = count_var;
        }
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

    /// Return the iterated-collection variable for the loop whose index
    /// variable is `var_nr`, or `u16::MAX` when the loop iterates an
    /// expression that isn't a plain variable.  Used by `#remove` to
    /// detect the C60 hash-iteration scratch variable.
    #[must_use]
    pub fn loop_coll_var(&self, var_nr: u16) -> u16 {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return self.loops[c as usize].coll_var;
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

    /// Plan-04 Phase B.3: frame high-water mark — max slot end across
    /// all non-argument placed locals.  Used by `def_code` (B.3.i) to
    /// emit a single `OpReserveFrame(frame_hwm)` at function entry,
    /// which replaces the N per-block `OpReserveFrame(block.var_size)`
    /// paired with `OpFreeStack`.  Returns 0 if no local is placed.
    ///
    /// Dead code until B.3.i wires the caller — kept `pub` because
    /// `Function` is exported.
    #[must_use]
    #[allow(dead_code)]
    pub fn frame_hwm(&self, context: &Context) -> u16 {
        let mut hwm: u16 = 0;
        for v in &self.variables {
            if v.argument || v.stack_pos == u16::MAX {
                continue;
            }
            let end = v.stack_pos.saturating_add(size(&v.type_def, context));
            if end > hwm {
                hwm = end;
            }
        }
        hwm
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

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
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

    /// Replace all occurrences of `Type::Reference(tv_nr, _)` with `concrete`
    /// in every variable's type definition.  Used when instantiating a generic template.
    pub fn substitute_type(&mut self, tv_nr: u32, concrete: &Type) {
        let trace_target = crate::log_config::type_timeline_target();
        for (i, v) in self.variables.iter_mut().enumerate() {
            let new_tp = Self::subst_type(v.type_def.clone(), tv_nr, concrete);
            if let Some(target) = &trace_target
                && v.name == *target
                && new_tp != v.type_def
            {
                eprintln!(
                    "[type_timeline] {name} (v_nr={v_nr}) {old:?} -> {new:?}  origin=substitute_type(tv_nr={tv_nr})",
                    name = v.name,
                    v_nr = i,
                    old = v.type_def,
                    new = new_tp,
                );
            }
            v.type_def = new_tp;
        }
    }

    fn subst_type(tp: Type, tv_nr: u32, concrete: &Type) -> Type {
        match tp {
            Type::Reference(d, deps) if d == tv_nr => {
                // preserve the original deps when substituting T → concrete.
                // The deps carry vector-element borrowing info needed by get_free_vars
                // to suppress FreeRef on loop element variables.
                let mut result = concrete.clone();
                if !deps.is_empty() {
                    for dep in deps {
                        result = result.depending(dep);
                    }
                }
                result
            }
            Type::Vector(inner, deps) => {
                Type::Vector(Box::new(Self::subst_type(*inner, tv_nr, concrete)), deps)
            }
            // #493 — substitute through `Optional`/`Tuple` wrappers too, so a
            // generic body with a `T?` / `(T, …)` local monomorphises like the
            // signature does (mirrors `Parser::substitute_type`).  Missing this,
            // such a local kept the parametric `Reference(tv)` form and read the
            // wrong slot width at runtime.
            Type::Optional(inner) => Type::optional(Self::subst_type(*inner, tv_nr, concrete)),
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .into_iter()
                    .map(|e| Self::subst_type(e, tv_nr, concrete))
                    .collect(),
            ),
            _ => tp,
        }
    }

    pub fn is_independent(&self, var_nr: u16) -> bool {
        // No such variable — e.g. the `u16::MAX` sentinel a file-scope construction
        // carries when there is no destination slot.  Such a var is not an in-place
        // target; report `false` so the caller allocates fresh (mirrors `tp`, which
        // returns `Type::Null` for the same out-of-range index).  Guarding here keeps
        // `P p = P{}` at module scope from indexing an empty var table and panicking.
        if var_nr as usize >= self.variables.len() {
            return false;
        }
        let d = self.variables[var_nr as usize].type_def.depend();
        d.is_empty() || (d.len() == 1 && d[0] == var_nr)
    }

    /// Remove a lifetime dependency for this variable.
    /// Remove a lifetime dependency for this variable.
    ///
    /// Traced under `LOFT_LOG=type_timeline:<var>` like its `depend` sibling, and naming the
    /// caller. Without that, the timeline recorded deps being ADDED and never REMOVED — so a
    /// borrow that gets promoted to an owner looked like a variable that was born owned, and
    /// the promotion site could only be found by reading every `make_independent` call by
    /// hand. That is exactly how @PLN130's F1 had to be tracked down: the container-destroying
    /// free came from a dep strip the instrument could not show.
    #[track_caller]
    pub fn make_independent(&mut self, var_nr: u16, remove: u16) {
        if crate::log_config::type_timeline_target().is_some() {
            let mut after = self.variables[var_nr as usize].type_def.clone();
            if let Type::Reference(_, to)
            | Type::Enum(_, _, to)
            | Type::Vector(_, to)
            | Type::Sorted(_, _, to)
            | Type::Hash(_, _, to)
            | Type::Index(_, _, to)
            | Type::Radix(_, _, to)
            | Type::Trie(_, _, to) = &mut after
                && let Some(pos) = to.iter().position(|x| x == &remove)
            {
                to.remove(pos);
            }
            self.trace_type_change(var_nr, &after, "make_independent");
        }
        match &mut self.variables[var_nr as usize].type_def {
            Type::Reference(_, to)
            | Type::Enum(_, _, to)
            | Type::Vector(_, to)
            // @P295 — keyed collections carry a lifetime dep list too
            // (`Sorted`/`Hash`/`Index`/`Radix`'s last field).  Without
            // these arms `s = ns` left `s` depending on `ns`, so scope
            // analysis suppressed `s`'s own `OpFreeRef` (treating it as a
            // borrow) and deferred `ns`'s free — leaking the deep-copied
            // store and, in a loop, never re-clearing `ns` (accumulation).
            | Type::Sorted(_, _, to)
            | Type::Hash(_, _, to)
            | Type::Index(_, _, to)
            | Type::Radix(_, _, to) | Type::Trie(_, _, to) => {
                if let Some(pos) = to.iter().position(|x| x == &remove) {
                    to.remove(pos);
                }
            }
            _ => (),
        }
    }

    #[track_caller]
    pub fn depend(&mut self, var_nr: u16, on: u16) {
        if on != u16::MAX {
            let new_tp = self.variables[var_nr as usize].type_def.depending(on);
            self.trace_type_change(var_nr, &new_tp, "depend");
            self.variables[var_nr as usize].type_def = new_tp;
        }
    }

    pub fn uses(&self, var_nr: u16) -> u16 {
        self.variables[var_nr as usize].uses
    }

    /// Number of variables in this function's table (@PLN107 — for the dead-store walk).
    #[must_use]
    pub fn var_count(&self) -> usize {
        self.variables.len()
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
        if var.name.starts_with('_')
            || var.name.contains('#')
            || var.const_binding
            || var.value_const
        {
            return;
        }
        // #625 — the warning below seeks the lexer BACK to the previous write so it
        // reports there, but `to()` moves only the reporting line/pos and never
        // rewinds the read cursor: the tokenizer keeps incrementing that line for
        // every physical line it goes on to pull.  Left unrestored, the seek shifts
        // EVERY later diagnostic in the file back by its own distance — and, because
        // `write_source` below is then captured from the seeked position, each further
        // reassignment stacks another shift (`c = 1; c = 2; c = f();` misreported by
        // two lines).  Hold the true cursor and put it back; the seek is for REPORTING
        // only.  `definitions.rs` does the same around the end-of-function warning
        // passes — this one runs DURING the body parse, which is why it reaches user
        // code that has not been parsed yet.
        let here = lexer.at();
        if var.write_source != (0, 0) && var.uses == var.uses_at_write {
            // Variable was written before but not read since — dead assignment
            let name = var.name.clone();
            let prev_source = var.write_source;
            lexer.to(prev_source);
            diagnostic!(
                lexer,
                Level::Warning,
                code = "dead-assignment",
                "Dead assignment — '{}' is overwritten before being read",
                name,
            );
            lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "delete the assignment, or read it before the next one".to_string(),
                condition: Some("nothing between the two writes needs the first value".to_string()),
                edit: None,
                concept: "dead-code lint",
                concept_ref: "@F100",
            });

            lexer.to(here);
        }
        let var = &mut self.variables[var_nr as usize];
        var.uses_at_write = var.uses;
        var.write_source = here;
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

    /// Return all variable names and their types for capture analysis.
    pub fn all_names_and_types(&self) -> Vec<(String, Type)> {
        self.variables
            .iter()
            .map(|v| (v.name.clone(), v.type_def.clone()))
            .collect()
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
        let ctr = self.unique.entry(name.to_string()).or_insert(0);
        *ctr += 1;
        let nr = *ctr;
        self.add_variable(&format!("_{name}_{nr}"), type_def, lexer)
    }

    /// Mark a variable as carrying an EXPLICIT `: Type` annotation (vs an inferred type).
    /// An annotated narrow integer stays constrained; an inferred one widens (`widen_int`).
    pub fn set_annotated(&mut self, var_nr: u16) {
        self.annotated.insert(var_nr);
    }

    /// Whether the variable's type came from an explicit annotation.
    #[must_use]
    pub fn is_annotated(&self, var_nr: u16) -> bool {
        self.annotated.contains(&var_nr)
    }

    /// Widen an INFERRED integer variable's type directly to `type_def` (the `(I-Join)`
    /// join target).  Bypasses `change_var_type`, which no-ops on integers because
    /// `is_equal` collapses all integer widths to one type.
    pub fn widen_int(&mut self, var_nr: u16, type_def: &Type) {
        if let Some(v) = self.variables.get_mut(var_nr as usize) {
            v.type_def = type_def.clone();
        }
    }

    pub fn add_variable(&mut self, name: &str, type_def: &Type, lexer: &mut Lexer) -> u16 {
        // Due to 2 passes through the code, we will add the same variable a second time.
        if let Some(nr) = self.names.get(name) {
            let nr = *nr;
            let existing = &self.variables[nr as usize].type_def;
            // Refine an unknown; and for GENERATED temps (`__`-prefixed) let PASS 2 WIN
            // on a type CONFLICT: the `__ncc_N`/`__work_N` counters can diverge across
            // the two passes (a `??` that stays trivial in pass 1 materialises a temp in
            // pass 2), so the same NAME can denote a DIFFERENT site per pass. Keeping
            // pass 1's type then hands pass-2 code a contradicting temp — the routing
            // `add_tile` corruption: `txs as integer ?? -1`'s temp kept a pass-1
            // `ref(Img)` type, mis-emitting native (`E0605 as DbRef`) AND mis-reading
            // interp (a silent wrong value). A user variable keeps the old behaviour
            // (its name IS its cross-pass identity; type evolution has its own checks).
            if existing.is_unknown()
                || (name.starts_with("__") && !type_def.is_unknown() && existing != type_def)
            {
                self.trace_type_change(nr, type_def, "add_variable(reuse)");
                self.variables[nr as usize].type_def = type_def.clone();
            }
            return nr;
        }
        self.new_var(name, type_def, lexer)
    }

    /// Create a temporary variable during scope analysis (no Lexer needed).
    /// Reuses an existing variable if the name already exists (two-pass stability).
    /// Used to lift inline struct-returning call arguments.
    pub fn add_temp_var(&mut self, name: &str, type_def: &Type) -> u16 {
        if let Some(nr) = self.names.get(name) {
            let nr = *nr;
            let existing = &self.variables[nr as usize].type_def;
            // Same pass-2-wins rule as `add_variable` (see there): a generated
            // temp's cross-pass identity is name+type, so a conflicting re-add
            // re-types instead of handing back a contradicting temp.
            if existing.is_unknown()
                || (name.starts_with("__") && !type_def.is_unknown() && existing != type_def)
            {
                self.trace_type_change(nr, type_def, "add_temp_var(reuse)");
                self.variables[nr as usize].type_def = type_def.clone();
            }
            return nr;
        }
        let v = self.variables.len() as u16;
        self.names.insert(name.to_string(), v);
        self.variables.push(Variable {
            name: name.to_string(),
            type_def: type_def.clone(),
            source: (0, 0),
            scope: u16::MAX,
            stack_pos: u16::MAX,
            uses: 1,
            uses_at_write: 0,
            write_source: (0, 0),
            argument: false,
            defined: false,
            const_binding: false,
            value_const: false,
            amp_link: false,
            stack_allocated: false,
            skip_free: false,
            captured: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
            promoted_from: u16::MAX,
            was_loop_var: false,
            caller_hidden_buf: false,
        });
        v
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
            const_binding: self.variables[var as usize].const_binding,
            value_const: self.variables[var as usize].value_const,
            amp_link: self.variables[var as usize].amp_link,
            stack_allocated: false,
            skip_free: false,
            captured: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
            promoted_from: u16::MAX,
            was_loop_var: false,
            caller_hidden_buf: false,
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
            const_binding: false,
            value_const: false,
            amp_link: false,
            stack_allocated: false,
            skip_free: false,
            captured: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
            promoted_from: u16::MAX,
            was_loop_var: false,
            caller_hidden_buf: false,
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
            const_binding: false,
            value_const: false,
            amp_link: false,
            stack_allocated: false,
            skip_free: false,
            captured: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
            promoted_from: u16::MAX,
            was_loop_var: false,
            caller_hidden_buf: false,
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
        // A `u16::MAX` / out-of-range `var_nr` is the "no variable" sentinel — nothing to
        // retype (matches the guards in `tp`/`name`/`set_type`). Without it, malformed input
        // whose assignment LHS never resolved to a real variable (`Foo x = 5` with an unknown
        // type `Foo`) panics here instead of being diagnosed.
        if var_nr == u16::MAX || (var_nr as usize) >= self.variables.len() {
            return false;
        }
        let var_tp = &self.variables[var_nr as usize].type_def;
        // `_` is the universal unused variable — allow type changes silently.
        if self.variables[var_nr as usize].name == "_" && !type_def.is_unknown() {
            self.trace_type_change(var_nr, type_def, "change_var_type(_)");
            self.variables[var_nr as usize].type_def = type_def.clone();
            return self.is_new(var_nr);
        }
        // loft#663 — an integer-element vector's element WIDTH is layout-bearing: it
        // IS the store's stride.  `is_equal` deliberately collapses every integer
        // width to one type, so the equality early-return below keeps whichever
        // element type the variable was given FIRST.  When pass 1 could not resolve
        // the callee — a FORWARD-declared or recursive function — that first type
        // carries no declared width; pass 2 resolves the real one, and the collapse
        // then discards it.  The append writes 8-byte elements into a 1-byte-strided
        // store and they read back as 0.  Adopt a declared width over none; the
        // reverse (declared → undeclared) is left to the collapse, so an annotated
        // variable is never widened out from under its declaration.
        let adopt_elem_width = matches!(
            (var_tp, type_def),
            (Type::Vector(cur, _), Type::Vector(new, _))
                if matches!(
                    (&**cur, &**new),
                    (Type::Integer(c), Type::Integer(n))
                        if c.forced_size.is_none() && n.forced_size.is_some()
                )
        );
        if adopt_elem_width {
            self.trace_type_change(var_nr, type_def, "change_var_type(#663 element width)");
            self.variables[var_nr as usize].type_def = type_def.clone();
            for on in type_def.depend() {
                self.depend(var_nr, on);
            }
            return self.is_new(var_nr);
        }
        // @P376 — assigning the `Never` poison (an errored struct construction,
        // pass 2) to an as-yet-`Unknown` variable must OVERWRITE it to `Never`,
        // NOT take the early-return below.  `is_equal(Unknown, Never)` is true,
        // so without this guard the poison is dropped, the variable stays
        // `Unknown`, and the typo cascades (`p.name` → "Field of unknown
        // variable" → format-string fatal).  Falling through re-types it to
        // `Never`, which field access / format interpolation / the unknown-type
        // sweep all skip — leaving the single `unknown type '…'` diagnostic.
        let var_tp = &self.variables[var_nr as usize].type_def;
        let never_into_unknown = matches!(type_def, Type::Never) && var_tp.is_unknown();
        if !never_into_unknown && (type_def.is_unknown() || var_tp.is_equal(type_def)) {
            for on in type_def.depend() {
                self.depend(var_nr, on);
            }
            return self.is_new(var_nr);
        }
        // @PLN25 (N-Decl): `Optional(τ)` and `τ` share sentinel storage, so storing a
        // non-null `τ` into a nullable `τ?` slot is NOT a type change — accept and KEEP the
        // nullable slot type (do not narrow it to non-null). This is what makes nullable
        // LOCALS usable (`x: integer? = 5`); without it `change_var` rejected the assignment
        // as "cannot change type from integer? to integer". The reverse (an explicit non-null
        // target ← `τ?`) is the `(N-Store)` violation, caught at the store site before here.
        // Gate-OFF inert: the postfix `?` is a no-op so `var_tp` is never `Optional`.
        if let Type::Optional(inner) = var_tp
            // @PLN25 (N-Idem): peel the SOURCE too — `Optional(τ) ← Optional(τ)` (e.g. a
            // `text?` local reassigned from a `text?`-typed if-join whose frame-deps differ)
            // is not a type change. Without `.base()` the source's `Optional` wrapper made
            // `inner.is_equal` fail and change_var wrongly rejected `text? ← text?`.
            && (inner.is_equal(type_def.base()) || matches!(type_def, Type::Null))
        {
            for on in type_def.depend() {
                self.depend(var_nr, on);
            }
            return self.is_new(var_nr);
        }
        // @PLN25 DN6 (N-Join): an INFERRED local first assigned a bare `null`, then a
        // non-null INLINE scalar `τ`, widens to `Null ⊔ τ = τ?` instead of erroring — the
        // ergonomic escape valve for `a = null; a = 5` (a now `integer?`, so a later
        // `b: integer = a` still requires a discharge).  `var_tp == Null` is INHERENTLY the
        // inferred-from-null case: a variable cannot be ANNOTATED `null`, so this never
        // overrides an explicit non-null contract — `a: integer = null` carries
        // `var_tp == integer` and is the case-1 nullable-mix reject below.  Scoped to this
        // ONE direction (the reverse `a = 5; a = null` cannot be told apart from an
        // annotated `a: integer = null` here, so it keeps rejecting).  DN1-gated.
        //
        // SOUNDNESS BOUNDARY — INLINE scalars ONLY (Integer/Boolean/Float/Single/Character).
        // The retroactive widen keeps the slot allocated by the FIRST `= null`; that slot is
        // sound for a τ? only when Null and τ? share it.  Inline scalars carry the null as an
        // in-slot sentinel, so `null`→`τ?` reuses the same inline slot.  `Text` (the only
        // heap-backed scalar here) needs a heap-ref slot with text-position tracking that the
        // Null slot is NOT — widening it corrupts `fn_return`'s discard accounting (interp
        // underflow / native E0308).  A text null-start must annotate `s: text? = null` so the
        // slot is heap from the start; `s = null; s = "hi"` falls through to the case-1
        // nullable-mix error, which already says "declare it `text?`".
        if crate::keys::pln25_dn1_enabled()
            && matches!(var_tp, Type::Null)
            && matches!(
                type_def,
                Type::Integer(_) | Type::Boolean | Type::Float | Type::Single | Type::Character
            )
        {
            let widened = Type::optional(type_def.clone());
            self.trace_type_change(var_nr, &widened, "change_var_type(N-Join)");
            self.variables[var_nr as usize].type_def = widened;
            for on in type_def.depend() {
                self.depend(var_nr, on);
            }
            return self.is_new(var_nr);
        }
        // Allow assigning an iterator (vector slice) to a vector variable
        // when element types are compatible — the iterator is materialised.
        if let (Type::Vector(_, _), Type::Iterator(_, _)) = (var_tp, type_def) {
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
        } else if !var_tp.is_unknown()
            // `&unknown` → `&T` (#375): a `&` parameter whose pointee was an
            // unresolved forward / cross-package reference on pass 1 carries the
            // type `RefVar(Unknown)`, which the outer `is_unknown()` does not see
            // through.  Treat it as unknown here so pass 2's resolved `&T`
            // refines it (falling through to the type update below) instead of
            // erroring "cannot change type from &unknown to &T".
            && !matches!(var_tp, Type::RefVar(in_tp) if in_tp.is_unknown())
            // `Never` → `T` (#376): an errored-construction poison (or dead
            // post-divergence code) is the BOTTOM type — re-typeable to anything.
            // Lets pass 2 re-resolve a forward `c = Cell{…}` (poisoned `Never`
            // in pass 1) to its real type instead of erroring "cannot change
            // type from never to Cell".
            && !matches!(var_tp, Type::Never)
        {
            // @PLN25 DN1: peel an `Optional` source — `&text ← text?` is the hoisted
            // work-buffer local (control.rs return-deps hoist) re-assigned from a
            // nullable call result; `Optional(τ)` shares `τ`'s sentinel storage, so the
            // buffer carries the null (`STRING_NULL`) without a type change.
            if let Type::RefVar(in_tp) = var_tp
                && in_tp.is_equal(type_def.base())
            {
                return self.is_new(var_nr);
            }
            // annotated LHS struct-enum accepts a variant of
            // that enum as RHS.  `let k: Kind = Alpha { x: 1 };` is
            // idiomatic — the struct-literal constructor types the
            // variant as `Reference(variant_d, _)`, but the parent
            // relationship (`def(variant_d).parent == enum_d`)
            // proves subtype compatibility with `Enum(enum_d, true, _)`.
            if let (Type::Enum(parent_d, true, _), Type::Reference(rhs_d, _)) = (var_tp, type_def)
                && data.def(*rhs_d).parent == *parent_d
            {
                return self.is_new(var_nr);
            }
            // @PLN25 (N-Decl / DN6) — a `null` ↔ non-null-scalar transition is the
            // NULLABILITY case, not a generic type mismatch: `a: integer = null` (the
            // slot is committed non-null) or the inferred `a = null; a = 5` (the slot
            // was `null`). Name the real fix (`τ?`) and NEVER suggest `as` — `x as
            // integer` would LAUNDER the null into the non-null slot (the DN5 hole).
            // (Once `(N-Join)`/DN6 lands, the inferred direction widens silently instead
            // of erroring.) DN1-gated so gate-OFF stays byte-identical: gate-OFF the bare
            // `null` is coerced to the scalar sentinel before here, so `Type::Null` never
            // reaches `change_var` and this branch is unreachable.
            let is_null_scalar = |t: &Type| {
                matches!(
                    t,
                    Type::Integer(_)
                        | Type::Text(_)
                        | Type::Boolean
                        | Type::Float
                        | Type::Single
                        | Type::Character
                )
            };
            let nullable_mix = crate::keys::pln25_dn1_enabled()
                && ((is_null_scalar(var_tp) && matches!(type_def, Type::Null))
                    || (matches!(var_tp, Type::Null) && is_null_scalar(type_def)));
            if nullable_mix {
                let scalar = if is_null_scalar(var_tp) {
                    var_tp
                } else {
                    type_def
                };
                let scalar_name = scalar.name(data);
                diagnostic!(
                    lexer,
                    Level::Error,
                    "Variable '{}' cannot hold both `null` and the non-null scalar type `{}` — declare it `{}?` to allow null (do NOT cast with `as`: `null as {}` would store null into a non-null slot)",
                    self.name(var_nr),
                    scalar_name,
                    scalar_name,
                    scalar_name
                );
            } else {
                diagnostic!(
                    lexer,
                    Level::Error,
                    "Variable '{}' cannot change type from {} to {}; use a new variable name or cast with 'as'",
                    self.name(var_nr),
                    self.variables[var_nr as usize].type_def.name(data),
                    type_def.name(data)
                );
            }
        }
        self.trace_type_change(var_nr, type_def, "change_var_type");
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

    /// @PLN104 — renumber a FRAME variable `from` → `to` through every variable's
    /// TYPEDEF deps (frame space).  The variable-table companion to the IR walker
    /// (`Parser::renumber_frame_var`) and `swap_variables`: a var swap must move the
    /// deps a typedef holds on OTHER vars too, or the type table desyncs.
    pub fn renumber_frame_in_types(&mut self, from: u16, to: u16) {
        for v in &mut self.variables {
            v.type_def.renumber_frame_deps(from, to);
        }
    }

    /// @PLN104 — swap variable slots `a` and `b`, updating EVERY index-keyed table
    /// that references them (the variable-numbering namespace is a shared medium).
    /// Relocates a late-promoted text retbuf (minted after an inherited body local,
    /// so its variable index exceeds its attribute index) into the slot matching its
    /// attribute index — the `a == v` the returned-type dep needs (loft-lang/loft#568).
    /// SCOPE-keyed tables (`loop_scopes`, `loop_seq_ranges`, `scope_origins`) are left
    /// alone — they key on scope numbers, not variable numbers.  The CALLER must
    /// renumber the IR body and the typedef deps in tandem (`renumber_frame_var` +
    /// `renumber_frame_in_types`), or the code and the table desync.
    pub fn swap_variables(&mut self, a: u16, b: u16) {
        self.variables.swap(a as usize, b as usize);
        // name → index: the two names now resolve to the swapped slots.
        let na = self.variables[a as usize].name.clone();
        let nb = self.variables[b as usize].name.clone();
        self.names.insert(na, a);
        self.names.insert(nb, b);
        swap_in_bset(&mut self.work_texts, a, b);
        swap_in_bset(&mut self.work_refs, a, b);
        swap_in_bset(&mut self.arm_consumed, a, b);
        swap_in_bset(&mut self.inline_ref_vars, a, b);
        swap_in_hset(&mut self.annotated, a, b);
        swap_map_indices(&mut self.closure_var_map, a, b);
        swap_map_indices(&mut self.rebind_orig, a, b);
    }

    /// @PLAN59 (H1): drop a var from the argument set — used to retire the
    /// signature-time `__retbuf` placeholder when `ref_return` promotes a
    /// real local into the buffer role (the promoted local takes the
    /// placeholder's attribute; `arguments()` then yields the promoted var
    /// in the same last position by number order).
    pub fn retire_argument(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].argument = false;
    }

    pub fn is_argument(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].argument
    }

    /// Mark `var_nr` binding-const (`const` PREFIX): its slot is write-once.
    pub fn set_const_binding(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].const_binding = true;
    }

    /// Whether `var_nr` is binding-const — a rebind (`=`) is rejected, but the
    /// value it holds stays mutable.
    pub fn is_const_binding(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].const_binding
    }

    /// Mark `var_nr` value-const (`const` before the TYPE): the value is a
    /// read-only borrow — mutation through this name is rejected.
    pub fn set_value_const(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].value_const = true;
    }

    /// Whether `var_nr` is value-const — every mutation THROUGH it (`+=`, element,
    /// field, nested) is rejected; a rebind (`=`) that re-points the slot is allowed.
    pub fn is_value_const(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].value_const
    }

    /// Mark `var_nr` as bound with an explicit `&` at a struct-typed projection —
    /// the author asked for a live link, not a view loft may quietly copy (@PLN130 F9).
    pub fn set_amp_link(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].amp_link = true;
    }

    /// Whether `var_nr` was spelled `&` at a struct-typed projection.  The `&` is
    /// otherwise invisible after parsing: `c = &v[0]` and `c = v[0]` emit the same IR.
    pub fn is_amp_link(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].amp_link
    }

    /// Whether `var_nr` carries EITHER const axis — used by the guards that apply to
    /// any const binding (`d#lock` unlock, text-arg auto-promotion, dead-store and
    /// UPPER_CASE lints) regardless of which immutability it is.
    pub fn is_const_any(&self, var_nr: u16) -> bool {
        self.is_const_binding(var_nr) || self.is_value_const(var_nr)
    }

    pub fn is_captured(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].captured
    }

    /// @PLAN51 Cluster IV: mark this variable as a caller-side work-ref
    /// synthesised by `add_defaults` for a callee's hidden return-buffer.
    /// Used by `parse_code`'s preamble null-init loop to ensure these
    /// work-refs receive a `Set(r, Null)` IR regardless of their typedef's
    /// dep list — without it, the slot allocator skips them ("no first_def")
    /// and codegen panics with "Incorrect var __ref_N[65535]".
    /// #319 — add an existing var to the work-ref set so `parse_code`'s
    /// preamble null-init reserves its stack slot.  Used for heap-DbRef
    /// `__ncc_N` temps: their only `Set` lives inside the ncc block (an
    /// operand position the Zone-2 slot scan does not walk), so without a
    /// hoisted `Set(v, Null)` the slot allocator skips them ("no first_def")
    /// and codegen panics with "Incorrect var __ncc_N[65535]".
    pub fn register_work_ref(&mut self, var_nr: u16) {
        self.work_refs.insert(var_nr);
    }

    /// Whether `var_nr` is a registered work-ref temporary (a generated
    /// preamble-allocated buffer such as a vector-literal `_vec_N`) — the
    /// discriminator the return-delivery materializer uses to CONSUME an
    /// owned-fresh arm local: a plain param/user var is also deps-empty but
    /// is caller-owned and must never be freed by the arm.
    #[must_use]
    pub fn is_work_ref(&self, var_nr: u16) -> bool {
        self.work_refs.contains(&var_nr)
    }

    /// Mark a var as consumed in-arm by the return-delivery materializer.
    pub fn set_arm_consumed(&mut self, var_nr: u16) {
        self.arm_consumed.insert(var_nr);
    }

    /// Whether the return-delivery materializer consumed this var in-arm.
    #[must_use]
    pub fn is_arm_consumed(&self, var_nr: u16) -> bool {
        self.arm_consumed.contains(&var_nr)
    }

    /// Remove a work-ref from the preamble registry after the one-buffer
    /// binding substituted it out of the IR (`ref_return`'s chain leg).
    /// Without this the orphan still gets a `Set(v, Null)` preamble and a
    /// scope-exit free; the presence of FREES then flips the tail-`If`
    /// emission into the discarded-statement + `Return(Null)` shape that
    /// returns the null sentinel on native (the @P378 trap).
    pub fn unregister_work_ref(&mut self, var_nr: u16) {
        self.work_refs.remove(&var_nr);
    }

    pub fn mark_caller_hidden_buf(&mut self, var_nr: u16) {
        if (var_nr as usize) < self.variables.len() {
            self.variables[var_nr as usize].caller_hidden_buf = true;
        }
    }

    /// @PLN87 P2.1 — record that visible heap parameter `param` is
    /// whole-binding-reassigned in the body and `orig` is its caller-store
    /// witness (see [`Function::rebind_orig`]).  Idempotent — keyed on `param`.
    pub fn set_rebind_orig(&mut self, param: u16, orig: u16) {
        self.rebind_orig.insert(param, orig);
    }

    /// @PLN87 P2.1 — the witness var for a rebindable heap param, or `None` if
    /// `param` is never wholesale-reassigned (the common case).
    #[must_use]
    pub fn rebind_orig(&self, param: u16) -> Option<u16> {
        self.rebind_orig.get(&param).copied()
    }

    /// @PLN87 P2.1 — every (param, witness) pair, for the entry stash and the
    /// function-exit `OpFreeRefIfDistinct`.
    #[must_use]
    pub fn rebind_params(&self) -> Vec<(u16, u16)> {
        self.rebind_orig.iter().map(|(&p, &o)| (p, o)).collect()
    }

    pub fn is_caller_hidden_buf(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len()
            && self.variables[var_nr as usize].caller_hidden_buf
    }

    /// The variable a const-modification diagnostic should name.  A mutated text
    /// argument is promoted to a `__tp_` local (so a rebind has a slot to write); that
    /// synthetic local carries the const axis but not a user-facing name, so report
    /// against the ORIGINAL parameter it was promoted from.  Non-promoted vars map to
    /// themselves.
    pub fn const_report_var(&self, var_nr: u16) -> u16 {
        let origin = self.variables[var_nr as usize].promoted_from;
        if origin == u16::MAX { var_nr } else { origin }
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

    // text argument auto-promotion helpers

    pub fn set_promoted_from(&mut self, shadow: u16, original: u16) {
        self.variables[shadow as usize].promoted_from = original;
    }

    #[allow(dead_code)]
    pub fn promoted_from(&self, v: u16) -> u16 {
        self.variables[v as usize].promoted_from
    }

    /// Returns (shadow_var_nr, original_arg_var_nr) pairs for promoted text arguments.
    pub fn promoted_text_args(&self) -> Vec<(u16, u16)> {
        self.variables
            .iter()
            .enumerate()
            .filter(|(_, v)| v.promoted_from != u16::MAX)
            .map(|(i, v)| (i as u16, v.promoted_from))
            .collect()
    }

    pub fn remap_name(&mut self, name: &str, new_var: u16) {
        self.names.insert(name.to_string(), new_var);
    }

    #[allow(dead_code)]
    pub fn rename(&mut self, v: u16, new_name: &str) {
        self.variables[v as usize].name = new_name.to_string();
    }

    pub fn mark_used(&mut self, v: u16) {
        self.variables[v as usize].uses += 1;
    }

    pub fn var_source(&self, var_nr: u16) -> (u32, u32) {
        self.variables[var_nr as usize].source
    }

    pub fn test_used(&self, lexer: &mut Lexer, data: &Data, body: &Value) {
        for (nr, var) in self.variables.iter().enumerate() {
            if var.name.starts_with('_') || var.name.contains('#') {
                continue;
            }
            // A variable the emitted body never even NAMES is a pass-1 leftover, not an
            // unread local (loft#661's class, second half).  Pass 1 could not resolve the
            // match subject — a forward-declared callee is enough — so it bound the arm to
            // a plain variable; pass 2, with the callee resolved, bound the arm to its
            // `_mv_<field>` variable and read THAT.  Variables persist across passes, so
            // the abandoned pass-1 binding survives with `uses == 0` and warns about a
            // name the program does read.  The `Unknown`-type test below catches the
            // leftovers that never got a type; this catches the ones that did.
            //
            // It cannot silence a genuine unused local: `reads_var` counts `Set` TARGETS,
            // so `x = 5` with no read still names `x` and still warns.  Only a LOCAL
            // absent from the body entirely is skipped — a local that is not in the code
            // cannot be one the user failed to read.
            //
            // ARGUMENTS are exempt from the exemption, and that is not a detail: a
            // parameter is declared in the SIGNATURE, so "never appears in the body" is
            // exactly what an unread parameter looks like.  Skipping those silenced
            // `Parameter b is never read` outright — caught by keeping an unused-parameter
            // case in the matrix rather than only unused-local ones.
            if !var.argument && u16::try_from(nr).is_ok_and(|n| !body.reads_var(n)) {
                continue;
            }
            // A variable still typed `Unknown` after pass 2 is a pass-1 LEFTOVER, not
            // something the user wrote and failed to read (loft#661).  Pass 1 parses a
            // `match` whose subject type is not resolvable yet — e.g. a field whose type
            // is declared later in the file — and binds each arm pattern to a plain
            // variable; pass 2, with the enum resolved, binds the arm to its `_mv_<field>`
            // variable instead and reads THAT.  Variables persist across passes, so the
            // abandoned pass-1 binding survives with `uses == 0` and warned about a name
            // the program does read.  Every variable pass 2 actually lowered has a
            // resolved type, so this cannot silence a genuine unused local.
            if matches!(var.type_def, Type::Unknown(_)) {
                continue;
            }
            if var.uses == 0 && !var.captured && data.def_nr(&var.name) == u32::MAX {
                lexer.to(var.source);
                diagnostic!(
                    lexer,
                    Level::Warning,
                    code = "never-read",
                    "{} {} is never read",
                    if var.argument {
                        "Parameter"
                    } else {
                        "Variable"
                    },
                    var.name,
                );
                // A parameter and a local are the same lint but not the same fix: deleting
                // a parameter changes the signature every caller wrote, so that one is the
                // author's call in a way deleting a local is not.
                lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: if var.argument {
                        format!(
                            "drop the parameter `{}` — and its callers' argument",
                            var.name
                        )
                    } else {
                        format!("delete `{}`", var.name)
                    },
                    condition: Some(if var.argument {
                        "the parameter is not part of a signature you must keep".to_string()
                    } else {
                        "computing it has no effect you are relying on".to_string()
                    }),
                    edit: None,
                    concept: "dead-code lint",
                    concept_ref: "@F100",
                });
            }
        }
    }

    /// @PLN107 S1 — observable dump of the dead-store access classification (value-observing
    /// READS vs `OpSet*` WRITE-TARGET bases), gated on `LOFT_DUMP_READS`. Purely diagnostic:
    /// no warning is emitted, so the classifier can be verified against the shape corpus
    /// before S2 wires it into a lint. `uses` is printed alongside to make the read /
    /// write-target split visible against the codegen counter it deliberately does NOT change
    /// (`reads == 0 && write_targets > 0` is the future S2 dead-store signal — see
    /// `doc/claude/plans/107-dead-code-lint/`).
    pub fn debug_dead_store_dump(&self, fn_name: &str, body: &Value, data: &Data) {
        if std::env::var_os("LOFT_DUMP_READS").is_none() {
            return;
        }
        let acc = crate::use_analysis::dead_store_accesses(body, self.variables.len(), data);
        for (i, var) in self.variables.iter().enumerate() {
            if var.name.starts_with('_') || var.name.contains('#') || var.argument {
                continue;
            }
            let (reads, write_targets) = acc.get(i).copied().unwrap_or((0, 0));
            eprintln!(
                "dead-store-dbg: fn={fn_name} var={} uses={} reads={reads} write_targets={write_targets}",
                var.name, var.uses,
            );
        }
    }

    /// Warn on UPPER_CASE non-const locals (P246 follow-up).  The
    /// UPPER_CASE convention is reserved for constants — file-scope
    /// `const NAME = expr;` / `NAME = expr;` and in-fn `const FOO =
    /// expr;`.  A LOCAL written `FOO = …` without the `const` keyword
    /// violates the convention and confuses readers — they expect
    /// UPPER_CASE to mean "compiler-checked immutable" but the
    /// variable can be reassigned.  Emits a Warning telling the user
    /// to either add `const` or rename to lower_case.  Skips
    /// arguments (the `const T` parameter modifier already handles
    /// const-ness on parameters), `_`-prefixed names, and synthetic
    /// names containing `#`.
    pub fn warn_upper_case_locals(&self, lexer: &mut Lexer) {
        for var in &self.variables {
            if var.argument
                || var.const_binding
                || var.value_const
                || var.name.starts_with('_')
                || var.name.contains('#')
            {
                continue;
            }
            if !is_upper_case_name(&var.name) {
                continue;
            }
            lexer.to(var.source);
            diagnostic!(
                lexer,
                Level::Advice,
                code = "upper-case-local",
                "Variable '{}' is UPPER_CASE — that style is reserved for constants",
                var.name,
            );
            lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "declare it `const` to make it immutable".to_string(),
                condition: Some("the value never changes after this point".to_string()),
                edit: None,
                concept: "const",
                concept_ref: "@F18",
            });
            lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Mechanical,
                title: "rename it to lower_case".to_string(),
                condition: None,
                edit: None,
                concept: "const",
                concept_ref: "@F18",
            });
        }
    }

    /// Advance EVERY pooled work-name counter past the names already in `names`, so the
    /// next mint yields a genuinely-fresh variable instead of aliasing a live one.
    ///
    /// A mint helper reuses the name it finds in `names`.  That pooling is deliberate
    /// during a parse — it is what lets pass 2 re-find pass 1's buffer in the same role.
    /// Re-entering an ALREADY-PARSED function is where the reuse turns into aliasing: the
    /// counters were reset to 0 when the function was stored (`append`), so the first mint
    /// hands back the buffer the parse already gave to something still live.
    ///
    /// Call this at every such re-entry.  @PLN104 Phase B (`patch_tret_callers`) is the
    /// one today, and the buffer it aliased went to a callee AS ITS OWN ARGUMENT:
    /// `wrap(s())` gave `s`'s result buffer to `wrap` to build its return in, so `wrap`
    /// overwrote the bytes it was reading — `[hi]` came out `[[i]`, and `--native` dropped
    /// the backing `String` while a `&str` still pointed into it, a null-pointer copy
    /// (loft#671).
    ///
    /// **Every sequence belongs in this list.**  Syncing only `__work_N` is what left that
    /// hole: the caller retbuf moved to its own `__work_cN` counter (loft#662) and silently
    /// stopped being covered.  The shared `__work` stem self-disambiguates — `strip_prefix`
    /// leaves `c1` / `p2_1`, which do not parse as a number.
    pub fn sync_work_counters(&mut self) {
        fn index(name: &str, prefix: &str) -> Option<u16> {
            name.strip_prefix(prefix)?.parse::<u16>().ok()
        }
        let (mut text, mut ctext, mut p2, mut refs, mut vdb, mut kvb, mut fmt) = (
            self.work_text,
            self.work_ctext,
            self.work_text_p2,
            self.work_ref,
            self.work_vdb,
            self.work_kvb,
            self.work_fmt,
        );
        for name in self.names.keys() {
            if let Some(k) = index(name, "__work_") {
                text = text.max(k);
            }
            if let Some(k) = index(name, "__work_c") {
                ctext = ctext.max(k);
            }
            if let Some(k) = index(name, "__work_p2_") {
                p2 = p2.max(k);
            }
            if let Some(k) = index(name, "__ref_") {
                refs = refs.max(k);
            }
            if let Some(k) = index(name, "__vdb_") {
                vdb = vdb.max(k);
            }
            if let Some(k) = index(name, "__kvb_") {
                kvb = kvb.max(k);
            }
            if let Some(k) = index(name, "__fmt_") {
                fmt = fmt.max(k);
            }
        }
        self.work_text = text;
        self.work_ctext = ctext;
        self.work_text_p2 = p2;
        self.work_ref = refs;
        self.work_vdb = vdb;
        self.work_kvb = kvb;
        self.work_fmt = fmt;
    }
    #[track_caller]
    pub fn work_text(&mut self, lexer: &mut Lexer) -> u16 {
        let n = format!("__work_{}", self.work_text + 1);
        self.work_text += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            *nr
        } else {
            self.add_variable(&n, &Type::Text(Deps::none()), lexer)
        };
        self.work_texts.insert(v);
        v
    }

    /// The pass-2-only twin of [`Function::work_text`] — same buffer, own sequence.
    ///
    /// Use it at any mint site that cannot fire on pass 1 (typically one gated
    /// `!first_pass`, or one that needs a callee signature pass 1 has not promoted
    /// yet).  Such a site drawing from `work_text` shifts every later `__work_N`
    /// relative to pass 1; since the variable tables persist BY NAME, pass 2 then
    /// re-finds pass 1's variables under the wrong roles — loft#662.  Drawing from
    /// its own sequence, a pass-2-only mint cannot perturb anyone else's numbering,
    /// and its own names simply have no pass-1 counterpart to collide with.
    ///
    /// The name keeps the `__work` prefix so the free/scope/coroutine/introspect
    /// passes that key on it are unaffected; only `sync_work_text_counter`'s
    /// `__work_<N>` parse skips it, as it does for `__work_c<N>`.
    #[track_caller]
    pub fn work_text_p2(&mut self, lexer: &mut Lexer) -> u16 {
        let n = format!("__work_p2_{}", self.work_text_p2 + 1);
        self.work_text_p2 += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            *nr
        } else {
            self.add_variable(&n, &Type::Text(Deps::none()), lexer)
        };
        self.work_texts.insert(v);
        v
    }

    /// A buffer the CALLER allocates for a callee's hidden `&text` out-param —
    /// the text twin of `work_refs`' `__ref_N` retbuf for a vector/enum callee.
    ///
    /// It gets its own `__work_c<N>` counter rather than sharing `work_text`'s
    /// `__work_<N>` because the two are minted on different schedules, and the
    /// variable tables persist across passes BY NAME (loft#662).  A call to a
    /// text-returning function only needs this buffer once the callee's `&text`
    /// ABI exists — which for a SELF- or forward-recursive callee is not until
    /// pass 1 has promoted it.  Sharing one counter therefore let a pass-2-only
    /// mint shift every later `__work_N` by one, so pass 2's format buffers
    /// re-found pass 1's variables under the wrong roles: the return buffer
    /// landed on a fresh name (growing the signature — "Too few parameters") and
    /// a plain local landed on the variable pass 1 had promoted to a `&text`
    /// parameter.  Separate counters keep `__work_N` driven only by the body's
    /// format sites, which ARE pass-stable.
    ///
    /// The name keeps the `__work` prefix: the free/scope/coroutine/introspect
    /// passes that key on it treat this buffer identically, and only
    /// `sync_work_text_counter`'s `__work_<N>` parse is deliberately skipped.
    #[track_caller]
    pub fn caller_text_buf(&mut self, lexer: &mut Lexer) -> u16 {
        let n = format!("__work_c{}", self.work_ctext + 1);
        self.work_ctext += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            *nr
        } else {
            self.add_variable(&n, &Type::Text(Deps::none()), lexer)
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

    #[track_caller]
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

    /// Work-ref for `vector_db()` — uses a separate `__vdb_N` counter/namespace.
    /// `vector_db` only runs on the second pass (it is guarded by `!first_pass`),
    /// so it must NOT share the `work_ref` / `__ref_N` counter with `add_defaults`.
    /// Using a distinct counter prevents the name-shift that would cause
    /// `ref_return` to fail its name-based attr match and add a spurious attr.
    /// These variables are inserted into `work_refs` so they receive null-inits.
    /// loft#703 — a function-scoped work-ref that OWNS a keyed collection's store, for a
    /// keyed literal standing in VALUE position (a return, a call argument).  See the
    /// `work_kvb` field for why it is neither a `__ref_N` nor a `__vdb_N`.
    #[track_caller]
    pub fn work_keyed(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        let n = format!("__kvb_{}", self.work_kvb + 1);
        self.work_kvb += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            let nr = *nr;
            self.set_type(nr, tp.clone());
            nr
        } else {
            self.add_variable(&n, tp, lexer)
        };
        self.work_refs.insert(v);
        v
    }

    /// @PLN124 — the accumulator a format string builds when its target type
    /// implements the interpolation contract, in its own `__fmt_N` namespace.
    ///
    /// Function-scoped, like [`Function::work_keyed`] and for the same reason:
    /// the accumulator IS the value the expression produces, so no wrapper
    /// record backs its store, and a block-local temp would leave one in an
    /// argument position unfreed.
    #[track_caller]
    pub fn work_format(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        let n = format!("__fmt_{}", self.work_fmt + 1);
        self.work_fmt += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            let nr = *nr;
            self.set_type(nr, tp.clone());
            nr
        } else {
            self.add_variable(&n, tp, lexer)
        };
        self.work_refs.insert(v);
        v
    }

    #[track_caller]
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
    #[track_caller]
    pub fn set_skip_free(&mut self, v: u16) {
        if let Ok(want) = std::env::var("LOFT_SKIPFREE_TRACE")
            && self.variables[v as usize].name == want
        {
            eprintln!(
                "[skip_free] {} (var={v}) in {} @ {}",
                want,
                self.name,
                std::panic::Location::caller()
            );
        }
        self.variables[v as usize].skip_free = true;
    }

    /// Is `v` marked `skip_free`? Match-arm field bindings (`_mv_<field> =
    /// OpGetField(subject,…)`) are, being borrowed views of the match subject.
    #[must_use]
    pub fn skip_free(&self, v: u16) -> bool {
        self.variables[v as usize].skip_free
    }

    /// Mark a variable as captured by a closure.
    /// Suppresses the "never read" warning without affecting dead-assignment tracking.
    pub fn set_captured(&mut self, v: u16) {
        self.variables[v as usize].captured = true;
    }

    /// Register an existing variable as a work-reference so that `parse_code`
    /// inserts `Set(v, Null)` at the function body start.  This pre-reserves v
    /// in the outer scope, ensuring its frame slot survives inner-block FreeStack.
    pub fn add_to_work_refs(&mut self, v: u16) {
        self.work_refs.insert(v);
    }

    /// Return true if `v` is a compiler-generated temporary — its
    /// name starts with `_`, which `Function::unique` reserves for
    /// the `_<kind>_<counter>` prefix (e.g. `_elm_N`, `_for_result_N`,
    /// `_vector_N`, `__ref_N`, `__vdb_N`).  User-declared loft
    /// variables cannot start with `_` (parser rejects such names),
    /// so this reliably distinguishes owned user locals from aliases
    /// and internal scratch slots that borrow storage from an
    /// enclosing container.
    #[must_use]
    pub fn is_compiler_generated(&self, v: u16) -> bool {
        self.variables[v as usize].name.starts_with('_')
    }

    /// Does `v` own the store it points at — i.e. may a site allocate into its
    /// slot and free it independently?
    ///
    /// ONE home for a fact three different derivations used to answer separately
    /// (loft#664): an empty dep list, the `_elm` NAME prefix loft#660 had to match
    /// on, and the structural "defined by `OpNewRecord`" scan.  Deps alone cannot
    /// carry it — a dep names the borrow SOURCE, so a borrow with no source
    /// VARIABLE (a vector inside an enum payload is addressed by a field DbRef)
    /// came back with an empty list and read as OWNING, which is a wrong answer
    /// rather than an unknown one.  So the two markers are read first and the deps
    /// only decide what they do not cover:
    ///
    /// - `inline_ref` — "borrow, don't allocate", set at every producer of a
    ///   non-owning slot (a vector-literal element, a lift temp, a rebind backing);
    /// - `skip_free` — the free-time half of the same fact;
    /// - otherwise a non-empty dep list means the value is a view of something
    ///   else, and a lone SELF dep is the @P302 owned-keyed-local marker.
    #[must_use]
    pub fn owns_store(&self, v: u16) -> bool {
        // `u16::MAX` reaches here from a file-scope construction with no destination
        // slot; report "does not own" so the caller allocates fresh, as `is_independent`
        // does for the same sentinel.
        if v as usize >= self.variables.len() {
            return false;
        }
        !self.is_inline_ref(v) && !self.is_skip_free(v) && self.is_independent(v)
    }

    /// Record that fn_ref variable `fn_ref` has its closure stored in `clos`.
    pub fn set_closure_var_of(&mut self, fn_ref: u16, clos: u16) {
        self.closure_var_map.insert(fn_ref, clos);
    }

    /// Return the closure variable number for a fn_ref variable, if any.
    pub fn closure_var_of(&self, fn_ref: u16) -> Option<u16> {
        self.closure_var_map.get(&fn_ref).copied()
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
        // After assign_slots has run (pre_assigned_pos != u16::MAX),
        // interpreter codegen should not move variables to a different slot.
        // Native codegen has its own slot management and may legitimately adjust.
        // This assertion is a diagnostic — it logs but does not block.
        #[cfg(debug_assertions)]
        {
            let v = &self.variables[var as usize];
            if v.pre_assigned_pos != u16::MAX
                && v.pre_assigned_pos != pos
                && !v.argument
                && std::env::var("LOFT_SLOT_LOG").is_ok()
            {
                eprintln!(
                    "[set_stack_pos] '{}' scope={}: assign_slots placed at {} but \
                     codegen is moving to {}",
                    v.name, v.scope, v.pre_assigned_pos, pos,
                );
            }
        }
        self.variables[var as usize].stack_pos = pos;
    }

    /// Plan-22 02d-vii follow-up — `LOFT_LOG=type_timeline:<varname>`
    /// trace.  Called from every site that mutates a variable's
    /// `type_def` field; logs to stderr when the env var matches the
    /// variable's name.  No-op fast path when the env var is unset
    /// or has a different value (one `env::var` read per call;
    /// type-mutations are rare).
    /// `#[track_caller]` so the timeline names the SOURCE LINE that rewrote the type.
    /// A dep list is overwritten, not merged (`Type::depending`), so "who wrote this
    /// dep last" is the whole question when a borrow points at the wrong variable
    /// (loft#666) — and the origin word alone ("depend") cannot answer it.
    #[track_caller]
    fn trace_type_change(&self, var_nr: u16, new_tp: &Type, origin: &str) {
        let Some(target) = crate::log_config::type_timeline_target() else {
            return;
        };
        if (var_nr as usize) >= self.variables.len() {
            return;
        }
        let v = &self.variables[var_nr as usize];
        if v.name != target {
            return;
        }
        eprintln!(
            "[type_timeline] {name} (v_nr={v_nr}) {old:?} -> {new:?}  origin={origin} at {site}",
            name = v.name,
            v_nr = var_nr,
            old = v.type_def,
            new = new_tp,
            site = std::panic::Location::caller(),
        );
        // `LOFT_TIMELINE_BT=1` adds the stack behind that line.  The immediate caller is
        // often a shared helper (`change_var_type`'s adopt-deps branch rewrites a dep list
        // on behalf of whoever assigned), and the question is always which PARSE site is
        // behind it.
        if std::env::var_os("LOFT_TIMELINE_BT").is_some() {
            eprintln!("{}", std::backtrace::Backtrace::force_capture());
        }
    }

    #[track_caller]
    pub fn set_type(&mut self, var_nr: u16, tp: Type) {
        self.trace_type_change(var_nr, &tp, "set_type");
        self.variables[var_nr as usize].type_def = tp;
    }

    /// Reset every non-argument variable's `stack_pos` and
    /// `pre_assigned_pos` to `u16::MAX`.  Called by V1's
    /// `assign_slots` inline; V2's caller calls this explicitly
    /// before invoking `assign_slots_v2` so a stale state from
    /// an earlier pass does not leak.
    #[allow(dead_code)]
    pub fn reset_local_slots(&mut self) {
        for v in &mut self.variables {
            if !v.argument {
                v.stack_pos = u16::MAX;
                v.pre_assigned_pos = u16::MAX;
            }
        }
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
        // @PLN25 slice (b): `Optional(τ)` shares its base's sentinel storage — same size.
        Type::Optional(inner) => size(inner, context),
        // A declared `size(N)` on an integer alias wins over the range heuristic
        // below, which only knows a 1 / 2 / 8 ladder and so has no way to express
        // 4.  For every alias that existed before this arm the two agree (`u8` /
        // `i8` force 1 and range to 1; `u16` / `i16` force 2 and range to 2; plain
        // `integer` forces nothing), so reading the declaration changes no
        // constant that was already being emitted — verified by a byte-identical
        // `loft introspect` over a corpus of all of them.  It is what makes a
        // 4-byte constant expressible at all, which the jump displacement needs.
        Type::Integer(s) if context == &Context::Constant && s.forced_size.is_some() => {
            u16::from(s.forced_size.expect("checked by the guard").get())
        }
        Type::Integer(s) if context == &Context::Constant && s.range() - 1 <= 256 => 1,
        Type::Integer(s) if context == &Context::Constant && s.range() - 1 <= 65536 => 2,
        Type::Boolean | Type::Enum(_, false, _) => 1,
        Type::Single | Type::Character => 4,
        Type::Integer(_) | Type::Float => 8,
        Type::Function(_, _, _) => 20, // Phase 2c: 8B d_nr (i64) + 12B closure DbRef
        Type::Text(_) if context == &Context::Variable => size_of::<String>() as u16,
        Type::Text(_) => size_of::<&str>() as u16,
        Type::RefVar(_)
        | Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Sorted(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _)
        | Type::Iterator(_, _) => size_of::<DbRef>() as u16,
        Type::Tuple(elems) => crate::data::element_stack_size(&Type::Tuple(elems.clone())) as u16,
        _ => 0,
    }
}

/// Stack alignment (in bytes) a value of type `tp` requires so that an
/// `addr`/`addr_mut::<T>` into the eval stack / frame slot forms a sound,
/// aligned reference (@PLAN53 cluster 2).
///
/// Unlike [`size`], this is **not** context-dependent: stack `text` is
/// either an align-8 `String` (Variable context) or an align-8 `Str`
/// (otherwise) — both contain a raw pointer, so 8 either way.  This is
/// deliberately STRONGER than `data::element_align` (which aligns the
/// record-stored `Str` to 4); records keep their own weaker layout on
/// the `element_align` path and are unaffected.  Not meaningful for
/// `Context::Constant` (byte-packed bytecode operands) — callers only
/// use it for stack/frame layout.
// S3: called by the aligned V2 allocator (`slots_v2::assign_slots_v2`).
pub fn align(tp: &Type) -> u8 {
    match tp {
        // @PLN25: `Optional(τ)` shares its base's sentinel storage — same align as the
        // base (mirrors `size`, which already peels). Missing this aligned an `integer?`
        // stack slot to 1 instead of 8 → misaligned i64 reference → UB/SIGSEGV.
        Type::Optional(inner) => align(inner),
        Type::Boolean | Type::Enum(_, false, _) => 1,
        Type::Single | Type::Character => 4,
        Type::Integer(_) | Type::Float | Type::Function(_, _, _) => 8,
        // String (Variable) and Str (otherwise) both hold a raw pointer → align 8.
        Type::Text(_) => 8,
        Type::RefVar(_)
        | Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Sorted(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _)
        | Type::Iterator(_, _) => 4, // DbRef = u16 + u32 + u32 → align 4
        // @PLN114 — a stack tuple's alignment is the strongest alignment ITS OWN
        // elements need on the stack, so recurse through THIS function rather than
        // the record table.  `data::element_stack_align` gives `Text` 4 (the record's
        // weaker `Str` rule); on the stack a `Str` holds a raw pointer and needs 8, as
        // the doc above says.  Taking the record answer left `(P, text)` locals
        // 4-aligned and landed the `Str` on a 4-mod-8 address — real UB, caught by
        // `stack_align_guard` once the matrices joined its sweep.
        Type::Tuple(elems) => elems.iter().map(align).max().unwrap_or(1),
        _ => 1,
    }
}

/// @PLAN53 cluster 2 / S4 — one eval-TOS / frame-reserve advance step.
///
/// When `aligned`, round `size` up to 8 (the max alignment on the
/// stack) so successive pushes stay 8-aligned and every typed write
/// lands on its required boundary; the LIFO pop reverses the same
/// rounded step (the design's § 3 "uniform-8 step" choice).  When not
/// `aligned`, returns `size` unchanged — V1's tight, real-size step.
///
/// This is the single seam the S4 work toggles: route every
/// `stack_pos += size` / `-= size` and `stack.position += size` site
/// through it so codegen and runtime advance in lockstep (S1).
#[must_use]
#[inline]
pub fn aligned_stack_step(size: u32) -> u32 {
    size.next_multiple_of(8)
}

#[cfg(test)]
mod align_tests {
    use super::*;

    // S2 (@PLAN53 cluster 2): the stack-alignment table.  Text is align-8
    // (both `String` and `Str` hold a raw pointer); small types stay tight
    // (align 1/4) so they pack into the holes alignment leaves.
    #[test]
    fn align_values_for_stack_layout() {
        assert_eq!(align(&Type::Text(Deps::none())), 8);
        assert_eq!(align(&Type::Boolean), 1);
        assert_eq!(align(&Type::Character), 4);
        assert_eq!(align(&Type::Single), 4);
        assert_eq!(align(&Type::Float), 8);
        assert_eq!(align(&crate::data::I64), 8);
    }

    // S4 (@PLAN53 cluster 2): the eval-TOS step — every advance rounds up to
    // the 8-byte max-alignment so typed writes always land on their boundary.
    #[test]
    fn aligned_stack_step_contract() {
        assert_eq!(aligned_stack_step(1), 8);
        assert_eq!(aligned_stack_step(4), 8);
        assert_eq!(aligned_stack_step(8), 8);
        assert_eq!(aligned_stack_step(12), 16);
        assert_eq!(aligned_stack_step(16), 16);
        assert_eq!(aligned_stack_step(0), 0);
    }
}
