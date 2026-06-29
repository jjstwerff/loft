//! USE-analysis (first version) — derive a per-binding copy-vs-borrow VERDICT from
//! how a variable is *used*, not from the shape of its right-hand side.
//!
//! This is the dependable layer the copy-vs-borrow elision builds on; see
//! `doc/claude/plans/25-nullable-sequences/use-analysis-prework-design.md` and
//! `materialization-algorithm-design.md`. It is deliberately **behaviour-neutral**:
//! it computes a verdict and (under `LOFT_MATERIALIZE_DUMP`) prints it, and wires
//! into no codegen yet — so it can be iterated against tests before anything depends
//! on it for emission.
//!
//! By the time this runs (post-parse, in `scopes::check`), a `v = src.f` vector copy
//! has already been lowered to the **copy idiom**: a fresh `OpDatabase` buffer `vdb`,
//! `v = OpGetField(vdb, …)`, and one `OpAppendVector(v, src.f)` filling it. So the
//! analysis recognises that idiom (exactly what the future elision rewrite consumes)
//! and decides whether the copy could instead be a borrow.
//!
//! Soundness is by **conservative default**: a binding is `Borrow` ONLY when proven
//! safe at the Tier-0 envelope — single def, the source `src` is a parameter, and
//! neither `v` nor `src` ever appears outside a known-reader argument position (so `v`
//! is read-only and non-escaping — ¬D1/¬D3 — and `src` is unmutated — ¬D2; the param
//! lifetime gives the rest of ¬D3). Anything not proven is `Copy`. An unrecognised use
//! can only *lose* an elision, never produce a wrong borrow.

use crate::data::{Data, DefType, Value};
use crate::variables::Function;
use std::collections::{HashMap, HashSet};

/// The materialization decision for one binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Proven safe to alias the source store instead of deep-copying (Tier 0).
    Borrow,
    /// The conservative default — deep-copy into an independent store.
    Copy,
}

/// One row of the analysis result: the verdict for a single vector-copy binding.
#[derive(Clone, Debug)]
pub struct VerdictRow {
    pub var_nr: u16,
    pub var_name: String,
    /// The source variable of the copied `src` / `src.f` (`u16::MAX` if not a var).
    pub source: u16,
    pub verdict: Verdict,
    /// Human-readable justification (for the dump and test diagnostics).
    pub reason: &'static str,
}

fn def_nrs(data: &Data, names: &[&str]) -> HashSet<u32> {
    let mut s = HashSet::new();
    for name in names {
        let nr = data.def_nr(name);
        if nr != u32::MAX {
            s.insert(nr);
        }
    }
    s
}

/// PROJECTION ops return a *reference into* their base container (arg 0) — so the
/// base is accessed in whatever context the projection's RESULT is: a projection
/// inside a write (`OpSetInt(OpGetVector(t,…), …)`) writes `t`; inside a read
/// (`OpGetInt(OpGetVectorNullable(t,…), …)`) reads `t`. They therefore PROPAGATE the
/// incoming context to arg 0 (the remaining args — an index — are pure reads).
fn projection_ops(data: &Data) -> HashSet<u32> {
    def_nrs(
        data,
        &[
            "OpGetVector",
            "OpGetVectorNullable",
            "OpGetField",
            "OpGetDbRef",
        ],
    )
}

/// VALUE-READER ops return a fresh value, not a reference into a container — their
/// arguments are pure reads regardless of the surrounding context.
fn value_reader_ops(data: &Data) -> HashSet<u32> {
    def_nrs(
        data,
        &[
            "OpGetInt",
            "OpGetByte",
            "OpGetEnum",
            "OpGetSingle",
            "OpGetCharacter",
            "OpLengthVector",
            "t_6vector_len",
        ],
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    /// The node sits in a known-reader op's argument position (a pure read).
    ReaderArg,
    /// Any other position — conservatively a mutation / escape / unknown use.
    Other,
}

/// The base variable of a `src` / `src.f` expression, if any.
fn base_var(node: &Value, get_field: u32) -> Option<u16> {
    match node.unspan() {
        Value::Var(s) => Some(*s),
        Value::Call(d, args) if *d == get_field => match args.first().map(Value::unspan) {
            Some(Value::Var(s)) => Some(*s),
            _ => None,
        },
        _ => None,
    }
}

struct Uses {
    get_field: u32,
    op_append: u32,
    op_database: u32,
    op_free: u32,
    projections: HashSet<u32>,
    value_readers: HashSet<u32>,
    /// Pre-order position counter — a total order on nodes that, OUTSIDE loops,
    /// matches execution order (Tier 1 uses it to prove a source is unmutated
    /// after the copy-fill). Bumped once per visited node.
    pos: usize,
    /// Loop nesting at the current node (`Value::Loop`, which for-loops desugar
    /// to). Back-edges break the position↔execution correspondence, so Tier 1
    /// refuses any copy whose fill sits at depth > 0.
    loop_depth: u32,
    /// Max position at which a var appeared in a NON-reader (write / escape /
    /// pass-to-callee) position — EXCLUDING the benign scope ops (`OpFreeRef` /
    /// `Drop`) and the copy-fill itself. For a source `x`, "all such positions <
    /// copy-fill position" means `x` is only constructed (never mutated) before
    /// the snapshot — the Tier-1 ¬D2 fact the set-based `ineligible` can't give.
    other_max_pos: HashMap<u16, usize>,
    /// Position of each copy var's `OpAppendVector` fill (the snapshot moment).
    copyfill_pos: HashMap<u16, usize>,
    /// Copy vars whose fill sits inside a loop (Tier-1 ineligible).
    copyfill_in_loop: HashSet<u16>,
    /// Vars that appeared in a non-reader position (⇒ not borrow-eligible). The
    /// copy-fill `OpAppendVector(v, src.f)` is *excluded* — it is the copy machinery,
    /// not a user mutation of `v`.
    ineligible: HashSet<u16>,
    /// Number of `Set(v, _)` for each var (a reassignment is > 1).
    def_count: HashMap<u16, u32>,
    /// Vars allocated a fresh store by `OpDatabase(vdb, …)`.
    database_vars: HashSet<u16>,
    /// For `Set(v, OpGetField(Var(vdb), …))`: the backing buffer `vdb`.
    def_vdb: HashMap<u16, u16>,
    /// Source bases of `OpAppendVector(Var(v), <src>)` (one entry per append).
    append_src: HashMap<u16, Vec<Option<u16>>>,
    /// Source EXPRESSIONS of `OpAppendVector(Var(v), <src>)` — kept for the
    /// elision plan (the field-read to inline in place of `v`'s reads).
    append_expr: HashMap<u16, Vec<Value>>,
}

/// An elidable copy: replace `v`'s reads with `source` and drop the copy idiom
/// (the `vdb` buffer + its alloc/append/free).
#[derive(Clone, Debug)]
pub struct ElidePlan {
    pub var: u16,
    pub vdb: u16,
    pub source: Value,
    /// The base var of `source` (`s` in `v = s.f`) — the owner the borrowers below
    /// must re-point to once `v` is gone.
    pub source_base: u16,
    /// Vars that BORROW `v` (`e = v[i]` / `e = v.fld`, `deps` ∋ `v`). After inlining,
    /// each borrows the live source element, so its dep must be re-pointed
    /// `v → source_base`. Only present (the plan only emitted) when every borrower is
    /// read-only and non-escaping — else the copy is kept (value semantics).
    pub borrowers: Vec<u16>,
}

impl Uses {
    fn visit(&mut self, node: &Value, ctx: Ctx) {
        let pos = self.pos;
        self.pos += 1;
        match node.unspan() {
            Value::Var(v) => {
                if ctx != Ctx::ReaderArg {
                    self.ineligible.insert(*v);
                    let e = self.other_max_pos.entry(*v).or_insert(0);
                    *e = (*e).max(pos);
                }
            }
            // A loop body's back-edge can re-execute a write after a read, so the
            // pre-order position no longer tracks execution order inside it — bump
            // the depth so Tier 1 can refuse copies filled here.
            Value::Loop(b) => {
                self.loop_depth += 1;
                for op in &b.operators {
                    self.visit(op, Ctx::Other);
                }
                self.loop_depth -= 1;
            }
            // Scope machinery, not a user mutation: `OpFreeRef(x)` / `Drop(x)` must
            // not count as a non-reader use of `x` (a free placed AFTER the last
            // read is exactly what we want; the scope pass repositions it post-
            // elision anyway). Visit nothing — recursing would mark the arg `Other`.
            Value::Call(d, _) if *d == self.op_free => {}
            Value::Drop(_) => {}
            Value::Set(v, rhs) => {
                *self.def_count.entry(*v).or_insert(0) += 1;
                // Fresh-buffer def: `v = OpGetField(vdb, 0, _)` where vdb is OpDatabase'd.
                if let Value::Call(d, args) = rhs.unspan()
                    && *d == self.get_field
                    && let Some(Value::Var(vdb)) = args.first().map(Value::unspan)
                {
                    self.def_vdb.insert(*v, *vdb);
                }
                // The def-read is benign for its source; visit it as a read.
                self.visit(rhs, Ctx::ReaderArg);
            }
            Value::Call(d, args) if *d == self.op_append => {
                // `OpAppendVector(target, src, rec_tp)`.
                if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                    // Copy-fill into a plain local — record the source base; do NOT
                    // mark `v` ineligible (this append IS the copy, not a user write).
                    self.copyfill_pos.insert(*v, pos);
                    if self.loop_depth > 0 {
                        self.copyfill_in_loop.insert(*v);
                    }
                    let src = args.get(1).and_then(|s| base_var(s, self.get_field));
                    self.append_src.entry(*v).or_default().push(src);
                    if let Some(s) = args.get(1) {
                        self.append_expr
                            .entry(*v)
                            .or_default()
                            .push(s.unspan().clone());
                    }
                    for a in &args[1..] {
                        self.visit(a, Ctx::ReaderArg);
                    }
                } else {
                    // Append into a field / non-var target — a real mutation.
                    for a in args {
                        self.visit(a, Ctx::Other);
                    }
                }
            }
            Value::Call(d, args) if self.projections.contains(d) => {
                // Projection: arg 0 (the base container) is accessed in the INCOMING
                // context — a projection inside a write writes its base; inside a read
                // reads it. The remaining args (an index) are pure reads.
                let mut it = args.iter();
                if let Some(base) = it.next() {
                    self.visit(base, ctx);
                }
                for a in it {
                    self.visit(a, Ctx::ReaderArg);
                }
            }
            Value::Call(d, args) => {
                if *d == self.op_database
                    && let Some(Value::Var(vdb)) = args.first().map(Value::unspan)
                {
                    self.database_vars.insert(*vdb);
                }
                let c = if self.value_readers.contains(d) {
                    Ctx::ReaderArg
                } else {
                    Ctx::Other
                };
                for a in args {
                    self.visit(a, c);
                }
            }
            other => {
                // Structural nodes (Block, If, Loop, For, Return, Drop, Insert, …):
                // recurse with the conservative default — a `Var` directly under any
                // of these (a return/block tail, a stored value) is a non-reader use,
                // which correctly marks it ineligible.
                other.for_each_child(&mut |c| self.visit(c, Ctx::Other));
            }
        }
    }
}

/// Compute the borrow-vs-copy verdict for every elidable vector-copy binding in
/// `code`, up to `max_tier` (0 = the shipped param-source rule; 1 = also
/// read-only-local sources, ordering-proven). Higher tiers are additive: a tier-1
/// run still emits every tier-0 Borrow.
fn analyze_fn(
    code: &Value,
    function: &Function,
    data: &Data,
    max_tier: u8,
) -> (Vec<VerdictRow>, Vec<ElidePlan>) {
    let mut u = Uses {
        get_field: data.def_nr("OpGetField"),
        op_append: data.def_nr("OpAppendVector"),
        op_database: data.def_nr("OpDatabase"),
        op_free: data.def_nr("OpFreeRef"),
        projections: projection_ops(data),
        value_readers: value_reader_ops(data),
        ineligible: HashSet::new(),
        def_count: HashMap::new(),
        database_vars: HashSet::new(),
        def_vdb: HashMap::new(),
        append_src: HashMap::new(),
        append_expr: HashMap::new(),
        pos: 0,
        loop_depth: 0,
        other_max_pos: HashMap::new(),
        copyfill_pos: HashMap::new(),
        copyfill_in_loop: HashSet::new(),
    };
    u.visit(code, Ctx::Other);

    // The SOURCE-mutation fact (¬D2) is the parser's mature, interprocedural
    // mutation analysis — `find_written_vars` also catches a source handed to a
    // mutating callee, which our intraprocedural walk would miss. (We cannot use it
    // for the LOCAL `v`: `find_written_vars` marks a var written by its own
    // defining `Set`, so every bound local is "written"; `v`'s read-only-ness needs
    // the copy-idiom-aware walk above, which excludes `v`'s def and copy-fill.)
    let written = {
        let mut w = HashSet::new();
        crate::parser::find_written_vars(code, data, &mut w, &mut HashMap::new());
        w
    };

    let mut vars: Vec<u16> = u.append_src.keys().copied().collect();
    vars.sort_unstable();

    let mut rows = Vec::new();
    let mut plans = Vec::new();
    for v in vars {
        let appends = &u.append_src[&v];
        // The copy idiom: v is a fresh OpDatabase buffer, filled by exactly one append.
        let fresh_buffer = u
            .def_vdb
            .get(&v)
            .is_some_and(|vdb| u.database_vars.contains(vdb));
        if !fresh_buffer || appends.len() != 1 {
            continue; // not a single-source vector copy — not ours to elide
        }
        let src = appends[0];
        let single_def = u.def_count.get(&v).copied().unwrap_or(0) == 1;
        let v_readonly = !u.ineligible.contains(&v);
        let src_is_param = src.is_some_and(|s| function.is_argument(s));
        let src_unmutated = src.is_some_and(|s| !written.contains(&s));

        // TIER 1 (max_tier >= 1): a read-only LOCAL source. The set-based facts
        // can't prove a local unmutated (its construction looks like a write), so
        // use the ordering fact: the copy-fill is on a straight-line path (not in a
        // loop) AND every non-reader appearance of the source precedes the fill —
        // i.e. the source is only constructed, never mutated/freed, after the
        // snapshot (¬D2), and the inlining itself extends the source's lifetime over
        // v's reads (¬D3). `v` read-only/non-escaping is the same ¬D1 as tier 0.
        let src_local_stable = max_tier >= 1
            && src.is_some_and(|s| !function.is_argument(s))
            && !u.copyfill_in_loop.contains(&v)
            && match (src, u.copyfill_pos.get(&v)) {
                (Some(s), Some(&fill)) => u.other_max_pos.get(&s).copied().unwrap_or(0) < fill,
                _ => false,
            };

        let (verdict, reason) = if single_def && v_readonly && src_is_param && src_unmutated {
            (
                Verdict::Borrow,
                "tier0: read-only local, unmutated param source",
            )
        } else if single_def && v_readonly && src_local_stable {
            (
                Verdict::Borrow,
                "tier1: read-only local, ordering-proven read-only local source",
            )
        } else if src.is_none() {
            (
                Verdict::Copy,
                "source is not a plain var/field (e.g. a literal)",
            )
        } else if !single_def {
            (Verdict::Copy, "reassigned (multiple defs)")
        } else if !v_readonly {
            (Verdict::Copy, "local mutated or escapes")
        } else if !src_is_param && !src_local_stable {
            (
                Verdict::Copy,
                "source not a parameter / not provably read-only local",
            )
        } else {
            (Verdict::Copy, "source mutated")
        };

        if verdict == Verdict::Borrow
            && let Some(vdb) = u.def_vdb.get(&v)
            && let Some(exprs) = u.append_expr.get(&v)
            && exprs.len() == 1
            && let Some(source_base) = base_var(&exprs[0], u.get_field)
        {
            // Vars that borrow `v` (their `deps` reference it). After `v` is inlined
            // they borrow the live source element, so each must be re-pointed
            // `v → source_base`. We may only do that — and therefore only elide a
            // borrowed `v` — when every borrower is itself read-only and
            // non-escaping (∉ ineligible); otherwise eliding would route the
            // borrower's write/escape onto the live source, so keep the copy.
            let borrowers: Vec<u16> = (0..function.next_var())
                .filter(|&e| e != v && function.tp(e).depend().contains(&v))
                .collect();
            if borrowers.iter().all(|e| !u.ineligible.contains(e)) {
                plans.push(ElidePlan {
                    var: v,
                    vdb: *vdb,
                    source: exprs[0].clone(),
                    source_base,
                    borrowers,
                });
            }
        }

        rows.push(VerdictRow {
            var_nr: v,
            var_name: function.name(v).to_string(),
            source: src.unwrap_or(u16::MAX),
            verdict,
            reason,
        });
    }
    (rows, plans)
}

/// The elision tier selected by the environment: 0 (shipped param-source rule,
/// the default) unless `LOFT_ELIDE_T1` is set, which adds the Tier-1 read-only-
/// local-source verdicts. Higher tiers can attach more flags here as they land.
/// Kept here so every consumer (the rewrite, the dump, the tests' default) reads
/// one source of truth.
#[must_use]
pub fn env_tier() -> u8 {
    // Additive flags — each enabled tier raises the ceiling. Later tiers attach
    // their own flag here (e.g. `LOFT_ELIDE_T2` -> `tier = tier.max(2)`).
    let mut tier = 0;
    if std::env::var_os("LOFT_ELIDE_T1").is_some() {
        tier = tier.max(1);
    }
    tier
}

/// Public, test-facing entry: the verdicts for one function by its def number, at
/// the env-selected tier (Tier 0 unless `LOFT_ELIDE_T1` is set).
#[must_use]
pub fn verdicts_for(data: &Data, d_nr: u32) -> Vec<VerdictRow> {
    verdicts_for_tier(data, d_nr, env_tier())
}

/// Public, test-facing entry: the verdicts for one function at an EXPLICIT tier —
/// lets a test exercise a tier's logic regardless of the environment, so the
/// not-yet-wired tiers stay evaluable in isolation.
#[must_use]
pub fn verdicts_for_tier(data: &Data, d_nr: u32, max_tier: u8) -> Vec<VerdictRow> {
    let def = data.def(d_nr);
    analyze_fn(&def.code, &def.variables, data, max_tier).0
}

/// The elision plans (Borrow verdicts) for one function — what the borrow rewrite
/// consumes — at the env-selected tier.
#[must_use]
pub fn elision_plans(code: &Value, function: &Function, data: &Data) -> Vec<ElidePlan> {
    analyze_fn(code, function, data, env_tier()).1
}

// ============================================================================
// Ownership classification (Owned | Borrowed | Join) — the @PLN85 over-free fact.
//
// The over-free class (a borrowed view escapes into an owned position — return,
// reassign, append — and a free site frees its store while the view is still
// live) is NOT a per-site bug; it is one missing fact RE-DERIVED at ~16 sites.
// The fact: for any value that escapes into an owned position, is its store
//   * Owned     — a freshly minted store the producer owns (free is correct), or
//   * Borrowed  — a view into a store owned elsewhere (must NOT free here), or
//   * Join      — owned on one runtime branch, borrowed on the other (the
//                 `v[i] ?? default` shape) — the decision is runtime-dependent,
//                 so the escape must MATERIALISE the borrow branch to owned.
//
// Crucially the JOIN is the part the flattened return-dep facts
// (`return_adopts_fresh_store` / `returns_borrowed_view`) LOSE: a `fn pick(t,i)
// -> M { t[i] ?? m_none() }` flattens to "borrowed view of t", hiding that the
// `m_none()` arm is owned. So this classifier walks the return EXPRESSION (which
// recovers the `??`/`if-else` join) rather than reading the collapsed dep.
//
// STAGE 1 (this code) is deliberately INERT: it computes the classification and
// (under `LOFT_MATERIALIZE_DUMP`) prints it, and wires into no codegen. It is
// tested separately (see the `ownership_*` tests) before any free site reads it.
// See `doc/claude/plans/85-store-lifetime-retirement/over-free-class-study.md`
// (§ Three chokepoints) and `NEXT-SESSION-join-ownership-analysis.md`.
//
// APPROXIMATIONS (sound for an inert fact; revisited when a free site reads it):
//   * Var resolution is flow-INSENSITIVE — a var classifies as the join of ALL
//     its real (non-`= null`-init) defs across the body, not the def that reaches
//     the point of use. For the over-free shapes (single-def views, owned-then-
//     reassigned slots) this gives the right answer; a genuinely path-split var
//     can only over-report Join, never wrongly report Owned.
//   * A bare PARAMETER used as a value classifies as `Borrowed` (the caller owns
//     it) — including a retbuf param a callee fills in place, which is really
//     owned. Conservative: it can lose an Owned, never invent one.
// ============================================================================

/// The store-ownership of a value that escapes into an owned position. The one
/// carried fact the over-free free sites should READ instead of re-deriving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Own {
    /// A freshly minted store the producer owns (an `OpDatabase`/`OpNewRecord`
    /// buffer, a struct literal, a call whose return adopts a fresh store).
    Owned,
    /// A view into a store owned elsewhere (a vector-element / field projection,
    /// a returned parameter, or a value that resolves to one).
    Borrowed,
    /// Runtime-dependent: owned on one branch, borrowed on the other (a `??` /
    /// `if-else` whose arms split). No static free decision — the escape must
    /// materialise the borrow branch to owned, after which free is safe.
    Join,
}

impl Own {
    /// The lattice join: equal classes collapse; any disagreement is a `Join`.
    #[must_use]
    fn join(self, other: Own) -> Own {
        if self == other { self } else { Own::Join }
    }
}

/// One owned-slot reassignment (`v = X` where `v` already held a value): the class
/// of the value `v` held BEFORE this assignment and of the new RHS. A `prior =
/// Owned`, `rhs = Join`/`Borrowed` row is the over-free leak shape — the displaced
/// owned store must be freed before `v` takes the borrow (the `local_source` root).
#[derive(Clone, Debug)]
pub struct ReassignSite {
    pub var: u16,
    pub var_name: String,
    pub prior: Own,
    pub rhs: Own,
}

/// The kind of free site the over-free fix acts at — the Gap-A sites the value
/// classification alone does not surface (see `ownership-analysis-gaps.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeKind {
    /// `OpCopyRecord(src, _, tp)` with the `0x8000` source-free bit — frees `src`
    /// after copying it into a vector element (the `out += [src]` append idiom,
    /// the `elem_accumulate` chokepoint). A `Borrowed`/`Join` `src` is over-freed.
    AppendSource,
    /// A return-delivery buffer (a heap PARAMETER the fn delivers its result
    /// through) reassigned to a `Borrowed`/`Join` value — the buffer then aliases a
    /// store it does not own, so freeing it over-frees that store (the
    /// `match_return` chokepoint: `_mv_items_1 = OpGetField(e, …)`).
    ParamDeliver,
}

/// A site where the over-free fix must read the carried ownership instead of
/// blindly freeing. Carries the value's class AND (Gap B) the borrow base to
/// materialise from when the borrow is a direct projection in this function.
#[derive(Clone, Debug)]
pub struct FreeSite {
    pub kind: FreeKind,
    /// The freed source var (`AppendSource`) or the buffer param (`ParamDeliver`);
    /// `u16::MAX` when the source is not a plain var.
    pub slot: u16,
    pub slot_name: String,
    /// Ownership of the value the site frees/delivers.
    pub class: Own,
    /// The base var to materialise from when the borrow arm is a DIRECT projection
    /// in this fn (`None` when the borrow comes from a call — materialise = deep
    /// copy the whole returned value — or the value is `Owned`).
    pub base: Option<u16>,
    pub base_name: Option<String>,
}

/// The recursive ownership classifier over the post-lowering `Value` IR. Holds the
/// op-def numbers it keys on and a memoised, recursion-guarded per-function return
/// classification (so an interprocedural `pick(t,i)` call resolves to `pick`'s
/// return class, recovering the `??` join the flattened return dep loses).
struct Ownership<'a> {
    data: &'a Data,
    op_database: u32,
    op_new_record: u32,
    op_copy_record: u32,
    projections: HashSet<u32>,
    ret_memo: HashMap<u32, Own>,
    visiting: HashSet<u32>,
}

/// The tail (value) expression of a function body, or `None` for a native/`#rust`
/// definition (no loft `Block` body — its flattened return dep is then exact).
fn fn_body_tail(code: &Value) -> Option<&Value> {
    match code.unspan() {
        Value::Block(b) => b.operators.last(),
        _ => None,
    }
}

/// A function's def facts: every real definition `v = rhs` (in source order,
/// skipping `v = null` declaration sentinels) and the vars `OpDatabase` mints a
/// fresh store into (which are Owned even with no `Set`-def — e.g. a retbuf param
/// a `materialized_view_return` fills in place).
#[derive(Default)]
struct Defs {
    rhs: HashMap<u16, Vec<Value>>,
    db_vars: HashSet<u16>,
}

fn collect_defs(node: &Value, op_database: u32, out: &mut Defs) {
    match node.unspan() {
        Value::Set(v, rhs) => {
            if !matches!(rhs.unspan(), Value::Null) {
                out.rhs.entry(*v).or_default().push(rhs.unspan().clone());
            }
            collect_defs(rhs, op_database, out);
        }
        Value::Call(d, args) if *d == op_database => {
            if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                out.db_vars.insert(*v);
            }
            for a in args {
                collect_defs(a, op_database, out);
            }
        }
        other => other.for_each_child(&mut |c| collect_defs(c, op_database, out)),
    }
}

/// Collect the over-free candidate sites in `node` (recursively):
/// - `appends`: the `src` of each `OpCopyRecord(src, _, tp)` whose `tp` carries the
///   `0x8000` source-free bit (the `out += [src]` element-append free).
/// - `delivers`: each `(param, rhs)` of a `Set` to a HEAP parameter (a
///   return-delivery buffer reassigned — the `match_return` aliasing site).
fn collect_free_candidates(
    node: &Value,
    op_copy_record: u32,
    func: &Function,
    appends: &mut Vec<Value>,
    delivers: &mut Vec<(u16, Value)>,
) {
    match node.unspan() {
        Value::Call(d, args) if *d == op_copy_record => {
            if args.len() >= 3
                && let Value::Int(tp) = args[2].unspan()
                && tp & 0x8000 != 0
            {
                appends.push(args[0].unspan().clone());
            }
            for a in args {
                collect_free_candidates(a, op_copy_record, func, appends, delivers);
            }
        }
        Value::Set(v, rhs) => {
            if func.is_argument(*v)
                && func.tp(*v).heap_dep().is_some()
                && !matches!(rhs.unspan(), Value::Null)
            {
                delivers.push((*v, rhs.unspan().clone()));
            }
            collect_free_candidates(rhs, op_copy_record, func, appends, delivers);
        }
        other => other.for_each_child(&mut |c| {
            collect_free_candidates(c, op_copy_record, func, appends, delivers)
        }),
    }
}

impl<'a> Ownership<'a> {
    fn new(data: &'a Data) -> Self {
        Ownership {
            data,
            op_database: data.def_nr("OpDatabase"),
            op_new_record: data.def_nr("OpNewRecord"),
            op_copy_record: data.def_nr("OpCopyRecord"),
            projections: projection_ops(data),
            ret_memo: HashMap::new(),
            visiting: HashSet::new(),
        }
    }

    /// The ownership class of the value `d_nr` returns. For a loft-body function it
    /// classifies the return EXPRESSION (recovering a `??` join); for a native
    /// definition it falls back to the flattened canonical fact.
    fn return_ownership(&mut self, d_nr: u32) -> Own {
        if let Some(&c) = self.ret_memo.get(&d_nr) {
            return c;
        }
        let def = self.data.def(d_nr);
        let tail = fn_body_tail(&def.code);
        // No loft body (native op / `#rust` stdlib): no intraprocedural join to
        // recover — the flattened canonical fact is exact.
        if tail.is_none() || !matches!(def.def_type, DefType::Function) {
            let c = if def.returns_borrowed_view() {
                Own::Borrowed
            } else {
                Own::Owned
            };
            self.ret_memo.insert(d_nr, c);
            return c;
        }
        if !self.visiting.insert(d_nr) {
            // Recursion back-edge: conservatively Borrowed (never assume a self-
            // referential return is freshly owned). Not memoised — the enclosing
            // frame computes and caches the real class.
            return Own::Borrowed;
        }
        let mut defs = Defs::default();
        collect_defs(&def.code, self.op_database, &mut defs);
        let class = self.classify(tail.unwrap(), &def.variables, &defs);
        self.visiting.remove(&d_nr);
        self.ret_memo.insert(d_nr, class);
        class
    }

    /// Classify a value expression within `func` (using `defs` to resolve local
    /// vars to their defining RHS). The recursive core of the analysis.
    fn classify(&mut self, node: &Value, func: &Function, defs: &Defs) -> Own {
        match node.unspan() {
            // A var `OpDatabase` minted a fresh store into is Owned regardless of
            // any other def (the retbuf a `materialized_view_return` fills).
            Value::Var(v) if defs.db_vars.contains(v) => Own::Owned,
            Value::Var(v) => match defs.rhs.get(v) {
                Some(rhss) if !rhss.is_empty() => rhss
                    .iter()
                    .map(|r| self.classify(r, func, defs))
                    .reduce(Own::join)
                    .unwrap_or(Own::Owned),
                // No local def: a parameter (caller owns it ⇒ Borrowed) or an
                // uninitialised local (treat as Owned — nothing to mis-free).
                _ => {
                    if func.is_argument(*v) {
                        Own::Borrowed
                    } else {
                        Own::Owned
                    }
                }
            },
            Value::Call(d, _) => {
                if *d == self.op_database || *d == self.op_new_record {
                    Own::Owned
                } else if self.projections.contains(d) {
                    // A projection (`OpGetField`/`OpGetVector*`/`OpGetDbRef`) is a
                    // view into its base container (arg 0).
                    Own::Borrowed
                } else {
                    self.return_ownership(*d)
                }
            }
            // `??` / `if-else` lowers to `If`: the join of its two arms.
            Value::If(_, then, els) => self
                .classify(then, func, defs)
                .join(self.classify(els, func, defs)),
            // A block's value is its tail; passthrough wrappers forward.
            Value::Block(b) => b
                .operators
                .last()
                .map_or(Own::Owned, |t| self.classify(t, func, defs)),
            Value::Insert(ops) => ops
                .last()
                .map_or(Own::Owned, |t| self.classify(t, func, defs)),
            Value::Return(v) | Value::BreakWith(_, v) => self.classify(v, func, defs),
            // Everything else (literals, scalar/void ops, control with no value
            // payload) is a fresh value or irrelevant to the heap over-free class.
            _ => Own::Owned,
        }
    }

    /// The owned-slot reassignments in function `d_nr`: for each var with more than
    /// one real def, the class it held before its LAST def and the class of that
    /// def. The `prior = Owned` rows are the displaced-owned-store leak candidates.
    fn reassign_sites(&mut self, d_nr: u32) -> Vec<ReassignSite> {
        let def = self.data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            return Vec::new();
        }
        let func = &def.variables;
        let mut defs = Defs::default();
        collect_defs(&def.code, self.op_database, &mut defs);
        // Only HEAP-typed vars can carry the over-free leak: a reassigned scalar
        // loop counter has no store to displace (the class is record-specific —
        // "scalar never fires" per the boundary map). Filter them out.
        let mut vars: Vec<u16> = defs
            .rhs
            .keys()
            .copied()
            .filter(|v| defs.rhs[v].len() > 1 && func.tp(*v).heap_dep().is_some())
            .collect();
        vars.sort_unstable();
        vars.into_iter()
            .map(|v| {
                let rhss = defs.rhs[&v].clone();
                let n = rhss.len();
                let prior = rhss[..n - 1]
                    .iter()
                    .map(|r| self.classify(r, func, &defs))
                    .reduce(Own::join)
                    .unwrap_or(Own::Owned);
                let rhs = self.classify(&rhss[n - 1], func, &defs);
                ReassignSite {
                    var: v,
                    var_name: func.name(v).to_string(),
                    prior,
                    rhs,
                }
            })
            .collect()
    }

    /// The base var a `Borrowed`/`Join` value's borrow arm views, when that arm is
    /// a DIRECT projection in this function — the store the Stage-3 materialise
    /// copies FROM (Gap B). `None` when the borrow is produced by a call (the
    /// materialise deep-copies the whole returned value) or the value is `Owned`.
    fn borrow_base(&self, node: &Value, func: &Function, defs: &Defs) -> Option<u16> {
        match node.unspan() {
            Value::Call(d, args) if self.projections.contains(d) => {
                match args.first().map(Value::unspan) {
                    Some(Value::Var(b)) => Some(*b),
                    Some(inner) => self.borrow_base(inner, func, defs), // nested `o.inner.rows`
                    None => None,
                }
            }
            Value::Var(v) => {
                if let Some(rhss) = defs.rhs.get(v) {
                    rhss.iter()
                        .rev()
                        .find_map(|r| self.borrow_base(r, func, defs))
                } else if func.is_argument(*v) && func.tp(*v).heap_dep().is_some() {
                    Some(*v) // a borrowed heap param IS its own base (the caller's arg)
                } else {
                    None
                }
            }
            Value::If(_, then, els) => self
                .borrow_base(then, func, defs)
                .or_else(|| self.borrow_base(els, func, defs)),
            Value::Block(b) => b
                .operators
                .last()
                .and_then(|t| self.borrow_base(t, func, defs)),
            Value::Return(v) | Value::BreakWith(_, v) => self.borrow_base(v, func, defs),
            _ => None,
        }
    }

    /// The over-free free SITES in function `d_nr` (Gap A): each append element
    /// source-free (`AppendSource`) with its source's class + base, and each
    /// return-buffer reassign to a `Borrowed`/`Join` value (`ParamDeliver`). These
    /// are the sites the value classification alone does not surface — the Stage-3
    /// fix reads the class here and frees / materialises accordingly.
    fn free_sites(&mut self, d_nr: u32) -> Vec<FreeSite> {
        let def = self.data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            return Vec::new();
        }
        let func = &def.variables;
        let mut defs = Defs::default();
        collect_defs(&def.code, self.op_database, &mut defs);
        let mut appends = Vec::new();
        let mut delivers = Vec::new();
        collect_free_candidates(
            &def.code,
            self.op_copy_record,
            func,
            &mut appends,
            &mut delivers,
        );

        let name = |v: u16| (v != u16::MAX).then(|| func.name(v).to_string());
        let mut sites = Vec::new();
        for src in appends {
            let class = self.classify(&src, func, &defs);
            // Only a Borrowed/Join source is over-freed; an Owned source-free is
            // correct (and load-bearing for the owned branch of a Join elsewhere).
            if !matches!(class, Own::Borrowed | Own::Join) {
                continue;
            }
            let base = self.borrow_base(&src, func, &defs);
            let slot = match src.unspan() {
                Value::Var(v) => *v,
                _ => u16::MAX,
            };
            sites.push(FreeSite {
                kind: FreeKind::AppendSource,
                slot,
                slot_name: name(slot).unwrap_or_default(),
                class,
                base,
                base_name: base.and_then(name),
            });
        }
        for (p, rhs) in delivers {
            let class = self.classify(&rhs, func, &defs);
            let base = self.borrow_base(&rhs, func, &defs);
            // A retbuf aliases a store it does not own ONLY when set to a DIRECT
            // borrow — a raw projection with a known base (`_mv_items_1 =
            // OpGetField(e,…)`). A call delivering into the retbuf MATERIALISES a
            // copy (the retbuf is then Owned — the clean `best = rows(b)` shape),
            // so `base.is_none()` cases are excluded; this also guarantees a usable
            // materialise base for every reported ParamDeliver.
            if matches!(class, Own::Borrowed | Own::Join) && base.is_some() {
                sites.push(FreeSite {
                    kind: FreeKind::ParamDeliver,
                    slot: p,
                    slot_name: func.name(p).to_string(),
                    class,
                    base,
                    base_name: base.and_then(name),
                });
            }
        }
        sites
    }
}

/// Public, test-facing entry: the ownership class of function `d_nr`'s return.
#[must_use]
pub fn return_ownership(data: &Data, d_nr: u32) -> Own {
    Ownership::new(data).return_ownership(d_nr)
}

/// Public, test-facing entry: the owned-slot reassignment sites of function `d_nr`.
#[must_use]
pub fn reassign_sites(data: &Data, d_nr: u32) -> Vec<ReassignSite> {
    Ownership::new(data).reassign_sites(d_nr)
}

/// Public, test-facing entry: the over-free free SITES of function `d_nr` (the
/// append element source-frees + return-buffer-aliasing deliveries, with class +
/// borrow base) — the Gap-A/B context the Stage-3 fix reads.
#[must_use]
pub fn free_sites(data: &Data, d_nr: u32) -> Vec<FreeSite> {
    Ownership::new(data).free_sites(d_nr)
}

/// Print every function's verdicts when `LOFT_MATERIALIZE_DUMP` is set. Called from
/// `scopes::check`; a no-op otherwise. Behaviour-neutral — diagnostics only.
pub fn dump_all(data: &Data) {
    if std::env::var_os("LOFT_MATERIALIZE_DUMP").is_none() {
        return;
    }
    let mut own = Ownership::new(data);
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        for r in analyze_fn(&def.code, &def.variables, data, env_tier()).0 {
            eprintln!(
                "MAT fn={} v={}({}) src={} verdict={:?} [{}]",
                def.name, r.var_nr, r.var_name, r.source, r.verdict, r.reason
            );
        }
        // The Stage-1 ownership fact (inert): the return class + the owned-slot
        // reassignments. The over-free leak shape is a `prior=Owned rhs=Join` row.
        eprintln!(
            "OWN fn={} return={:?}",
            def.name,
            own.return_ownership(d_nr)
        );
        for s in own.reassign_sites(d_nr) {
            eprintln!(
                "OWN fn={} reassign v={}({}) prior={:?} rhs={:?}",
                def.name, s.var, s.var_name, s.prior, s.rhs
            );
        }
        // The Stage-1.5 free SITES (Gap A) + borrow base (Gap B), still inert.
        for s in own.free_sites(d_nr) {
            eprintln!(
                "OWN fn={} free kind={:?} slot={}({}) class={:?} base={}",
                def.name,
                s.kind,
                s.slot,
                s.slot_name,
                s.class,
                s.base_name.as_deref().unwrap_or("-")
            );
        }
    }
}
