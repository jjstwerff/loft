//! USE-analysis (first version) — derive a per-binding copy-vs-borrow VERDICT from
//\! @I60 — Scope & dependency/lifetime tracker (deps)
//! how a variable is *used*, not from the shape of its right-hand side.
//!
//! This is the dependable layer the copy-vs-borrow elision builds on; the original
//! design is `doc/claude/plans/25-nullable-sequences/use-analysis-prework-design.md`
//! and `materialization-algorithm-design.md`. Its outputs are consumed by
//! **default-on codegen**: the `Verdict` drives an `ElidePlan` (borrow-inline, wired
//! into `scopes::elide_borrows`) and a `MovePlan` (last-use move, wired into
//! `scopes::move_elide`); the dumps (`LOFT_MATERIALIZE_DUMP`, `--report-copies`,
//! `LOFT_WARN_COPIES`) are opt-in views on the same verdicts.
//!
//! By the time this runs (post-parse, in `scopes::check`), a `v = src.f` vector copy
//! has already been lowered to the **copy idiom**: a fresh `OpDatabase` buffer `vdb`,
//! `v = OpGetField(vdb, …)`, and one `OpAppendVector(v, src.f)` filling it. So the
//! analysis recognises that idiom (exactly what the elision rewrite consumes) and
//! decides whether the copy could instead be a borrow.
//!
//! Soundness is by **conservative default**: a binding is `Borrow` ONLY when proven
//! safe at the Tier-0 envelope — single def, the source `src` is a parameter, and
//! neither `v` nor `src` ever appears outside a known-reader argument position (so `v`
//! is read-only and non-escaping — ¬D1/¬D3 — and `src` is unmutated — ¬D2; the param
//! lifetime gives the rest of ¬D3). Anything not proven is `Copy`. An unrecognised use
//! can only *lose* an elision, never produce a wrong borrow.

use crate::data::{Data, DefType, Value};
use crate::lexer::Position;
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

/// @PLN90 — the WARNING bucket for a copy (which `COPY_DIAGNOSTICS.md` drives). The verdict
/// says *whether* a copy happens; this says *what to do about it*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyClass {
    /// A `Borrow` — already eliminated, not a copy. No warning.
    Eliminated,
    /// Bucket 2 — a borrow WOULD be sound; the copy is only analysis/codegen weakness. The
    /// north-star elimination worklist. WARN (the actionable "you didn't have to copy").
    Avoidable,
    /// Inherent to the ownership model — constructing an owning structure (`S { f: src }`)
    /// or assigning into an owning slot (`v[i] = e`) *owns* its data. This is exactly what
    /// the programmer asked for, not a surprise. SILENT (no warning).
    Implicit,
    /// Forced by circumstance (a short-lived source, a later mutation) — required as
    /// written, but the user could restructure. Indicated (informational), never silent.
    Forced,
    /// @PLN90 (item 1) — an UNBOUND copy whose SOURCE is a compiler-generated temporary
    /// (`_`-prefixed: `__ref_N`, `___par_mat_e_N`, `_comp_N`, …; `is_compiler_generated`).
    /// A real copy and a candidate for US to eliminate (the developer worklist), but NOT
    /// user-actionable — the user never wrote the source, so the user-facing report must
    /// exclude it. Distinct from `Implicit` (genuinely no unbound copy) and from
    /// `Avoidable`/`Forced` (which name a source the user can act on).
    Internal,
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
    /// @PLN90 — the warning bucket: `Eliminated` (Borrow) · `Avoidable` (warn — the
    /// worklist) · `Implicit` (model-inherent ownership — silent) · `Forced` (informational).
    /// See [`CopyClass`] and COPY_DIAGNOSTICS.md.
    pub class: CopyClass,
    /// @PLN90 item 2 — the copy site's source location (`file:line:pos`), when the emitting
    /// op carries a span. Makes a `<record>` element-set copy (no named target var)
    /// actionable in the report. `None` when the op is unspanned.
    pub loc: Option<Position>,
    /// @PLN90 Step 5 — true for a SURVIVAL-SPLIT row (a construction / record copy classified by
    /// its source's fate — "you duplicated a live value"). The user-facing `report_copies` shows
    /// only these; the var-buffer / return-buffer copies (a separate elision/`__retbuf` class,
    /// which is where the stdlib's copies land) stay in the developer dump.
    pub survival: bool,
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

/// @PLN90 item 3 — ops that WRITE THROUGH THEIR FIRST ARGUMENT (mirrors the
/// `first_arg_write` set in `parser::find_written_vars`). Used to build `mut_max_pos`,
/// the position-aware "the source is mutated after the copy" fact. `OpCopyRecord` writes
/// its SECOND arg (the dest), handled separately.
fn first_arg_write_ops(data: &Data) -> HashSet<u32> {
    let mut s = HashSet::new();
    for d in 0..data.definitions() {
        let n = data.def(d).name();
        if n.starts_with("OpSet")
            || n.starts_with("OpAppendStack")
            || n.starts_with("OpClearStack")
            || matches!(
                n,
                "OpNewRecord"
                    | "OpAppendCopy"
                    | "OpAppendVector"
                    | "OpClearVector"
                    | "OpClearKeyed"
                    | "OpSetKeyed"
                    | "OpHashRemove"
                    | "OpInsertVector"
                    | "OpRemoveVector"
            )
        {
            s.insert(d);
        }
    }
    s
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

/// @PLN107 S1 — per-variable ACCESS classification for the dead-store lint. Returns, indexed
/// by `var_nr`, `(reads, write_targets)`: how many times each local is READ (its value
/// observed) versus used only as the WRITE-TARGET base of an element/field/keyed setter
/// (`d[i]=x`, `d.f=x`).
///
/// Deliberately DECOUPLED from `Function::uses` (which drives codegen last-use elision and
/// MUST NOT change): a `Var(d)` at arg 0 of an `OpSet*` bumps `uses` but is a write-target,
/// not a read. WRITES are restricted to the `OpSet*` family on purpose — the copy-fill
/// `OpAppendVector` that lowers `d = s.f` is a DEFINITION, not a user mutation, and counting
/// it would misread an unused copy as a dead store. Projection handling mirrors
/// [`projection_ops`]: `d.f[i]=x` → `OpSetInt(OpGetField(Var(d),f), i, v)` descends to the
/// root var `d`, and the projection's INDEX args are ordinary reads.
pub(crate) fn dead_store_accesses(body: &Value, n_vars: usize, data: &Data) -> Vec<(u16, u16)> {
    let projs = projection_ops(data);
    let writes = first_arg_write_ops(data);
    let mut acc = vec![(0u16, 0u16); n_vars];
    let cx = AccessCx {
        data,
        projs: &projs,
        writes: &writes,
    };
    classify_access(body, &cx, &mut acc);
    acc
}

/// Shared read-only context for the access walk (op-name sets computed once).
struct AccessCx<'a> {
    data: &'a Data,
    /// Projection ops (`OpGetField`/`OpGetVector`/…) — a write through one of these
    /// propagates the write context to arg 0.
    projs: &'a HashSet<u32>,
    /// Ops that write through arg 0 (`first_arg_write_ops`): their arg-0 base is a
    /// write-DESTINATION, never a read (this is what makes the `d = s.f` copy-fill append
    /// stop counting `d` as read).
    writes: &'a HashSet<u32>,
}

fn is_setter(op: u32, data: &Data) -> bool {
    data.def(op).name().starts_with("OpSet")
}

fn bump_read(acc: &mut [(u16, u16)], v: u16) {
    if let Some(slot) = acc.get_mut(v as usize) {
        slot.0 = slot.0.saturating_add(1);
    }
}

fn bump_write(acc: &mut [(u16, u16)], v: u16) {
    if let Some(slot) = acc.get_mut(v as usize) {
        slot.1 = slot.1.saturating_add(1);
    }
}

/// Classify every `Var` occurrence reachable from `node`. Only the write-introducing /
/// var-carrying variants are special-cased; everything else delegates to `for_each_child`
/// (whose `Var` children are all value-observing reads). `Set`/`Iter` TARGET vars are NOT
/// visited by `for_each_child`, so a plain definition/reassignment correctly counts as
/// neither a read nor a copy-mutate write.
fn classify_access(node: &Value, cx: &AccessCx, acc: &mut [(u16, u16)]) {
    match node.unspan() {
        Value::Var(v) | Value::TupleGet(v, _) | Value::FnRefDnr(v) => bump_read(acc, *v),
        Value::FnRef(_, clos, _) => bump_read(acc, *clos),
        Value::TuplePut(v, _, inner) => {
            bump_write(acc, *v);
            classify_access(inner, cx, acc);
        }
        Value::CallRef(v, args) => {
            bump_read(acc, *v);
            for a in args {
                classify_access(a, cx, acc);
            }
        }
        // Any op that writes through arg 0: arg-0 base is a write-DESTINATION (not a read).
        // Count it as a copy-mutate WRITE-TARGET only for the `OpSet*` family — append/insert/
        // clear are definitional/bulk fills (the `d = s.f` copy-fill lands here) and are neither
        // a read nor the dead-store mutation signal. The remaining args are ordinary reads.
        Value::Call(op, args) if cx.writes.contains(op) && !args.is_empty() => {
            classify_write_base(&args[0], cx, acc, is_setter(*op, cx.data));
            for a in &args[1..] {
                classify_access(a, cx, acc);
            }
        }
        other => other.for_each_child(&mut |c| classify_access(c, cx, acc)),
    }
}

/// Descend the write-destination base (`args[0]` of a write-through op) through its
/// projection chain (`d.f[i]=x` → `OpSet…(OpGetField(Var(d),f), …)`) to the root var,
/// counting projection INDEX args as ordinary reads. The root var is recorded as a
/// write-target only when `count_target` (an `OpSet*` mutation); otherwise it is a
/// definitional/bulk destination — neither read nor target.
fn classify_write_base(node: &Value, cx: &AccessCx, acc: &mut [(u16, u16)], count_target: bool) {
    match node.unspan() {
        Value::Var(v) => {
            if count_target {
                bump_write(acc, *v);
            }
        }
        Value::Call(op, args) if cx.projs.contains(op) && !args.is_empty() => {
            classify_write_base(&args[0], cx, acc, count_target);
            for a in &args[1..] {
                classify_access(a, cx, acc);
            }
        }
        other => classify_access(other, cx, acc),
    }
}

struct Uses {
    get_field: u32,
    op_append: u32,
    op_database: u32,
    op_free: u32,
    /// @PLN90 — `OpCopyRecord` def_nr: a record deep-copy (`v[i] = e`, a `?? E{…}` default
    /// element, a struct copy). Not append-based, so the var-buffer / construction /
    /// return-buffer branches never see it; recorded here so the decision covers it.
    op_copy_record: u32,
    projections: HashSet<u32>,
    value_readers: HashSet<u32>,
    /// @PLN90 item 3 — ops that write through their first argument (see `first_arg_write_ops`).
    write_first_arg: HashSet<u32>,
    /// Pre-order position counter — a total order on nodes that, OUTSIDE loops,
    /// matches execution order (Tier 1 uses it to prove a source is unmutated
    /// after the copy-fill). Bumped once per visited node.
    pos: usize,
    /// Loop nesting at the current node (`Value::Loop`, which for-loops desugar
    /// to). Back-edges break the position↔execution correspondence, so Tier 1
    /// refuses any copy whose fill sits at depth > 0.
    loop_depth: u32,
    /// @PLN90 item 2 — track source locations for the report. Only ON when
    /// `LOFT_COPY_SURVIVAL` is set (positions are needed only for the survival report), so the
    /// default hot path pays no `Position` clone and stays byte-identical.
    track_pos: bool,
    /// @PLN90 item 2 — the nearest ENCLOSING span position (breadcrumb). The copy ops
    /// (`OpAppendVector` / `OpCopyRecord`) carry no span themselves, so a copy site borrows the
    /// most recent spanned node's position. Updated (when `track_pos`) at every spanned node.
    cur_pos: Option<Position>,
    /// @PLN90 item 3 — the max position at which a var is actually WRITTEN (a `Set` target, an
    /// append/insert target, a record-copy target). Unlike `other_max_pos` (any non-reader use,
    /// which counts a read-only pass-to-callee or being another copy's source), this counts only
    /// real mutations — so "the source is mutated AFTER the copy" (⇒ a borrow is unsound ⇒
    /// Forced) is precise, and a read-only survivor stays Avoidable. Only tracked when
    /// `track_pos` (read solely by `survival_class`); `other_max_pos` is left untouched so the
    /// shipped var-buffer elision stays byte-identical.
    mut_max_pos: HashMap<u16, usize>,
    /// @PLN90 item 4 — the position of each var's FIRST definition (`Set`). Compared against a
    /// copy's enclosing-loop entry to tell a source defined OUTSIDE the loop (duplicated every
    /// iteration → unbound) from a per-iteration LOCAL (consumed each pass → a move). Tracked
    /// only when `track_pos`.
    first_def_pos: HashMap<u16, usize>,
    /// @PLN90 item 4 — a stack of enclosing-loop entry positions (the `Value::Loop` node's pos).
    /// A source whose first def is `< loop_entry` existed before the loop (outside); `>=` means
    /// it is defined inside the loop body (per-iteration). Tracked only when `track_pos`.
    loop_entry: Vec<usize>,
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
    /// @PLN90 (bound-vs-unbound survival split) — max position at which a var appeared in
    /// ANY position (reader OR non-reader), i.e. its last use. Unlike `other_max_pos` (which
    /// tracks only non-reader uses) this includes plain reads, so "does the source survive
    /// (a use strictly after the copy site)?" — the move-vs-copy discriminator — can be
    /// answered. See `survival_class`.
    last_use_pos: HashMap<u16, usize>,
    /// @PLN90 — field-target appends `OpAppendVector(OpGetField(rec, fld), src)`:
    /// `(base record var, source base var, copy-site-end pos, loop-survives, source location)`.
    /// This is the struct/enum-construction copy (`S { f: src }`) and `x.field += src` — the
    /// source is deep-copied into the field, which the var-buffer copy idiom above never sees.
    /// The position (taken AFTER the args are walked) + `loop_survives` (item 4: the source
    /// outlives the enclosing loop) drive the survival split; the location (item 2) is for the
    /// report.
    construct_copy: Vec<(Option<u16>, Option<u16>, usize, bool, Option<Position>)>,
    /// @PLN90 — `OpCopyRecord` deep-copies: `(dest base var, SOURCE base var, copy-site-end pos,
    /// loop-survives, source location)`. A record copy (`v[i] = e`, a `?? E{…}` default element,
    /// a struct copy) — not append-based, so the branches above miss it. The same-var no-op
    /// alias is excluded when recorded.
    record_copy: Vec<(Option<u16>, Option<u16>, usize, bool, Option<Position>)>,
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

/// @PLN90 phase B — a construction / record copy that can be lowered as a MOVE (a store transfer)
/// instead of a deep copy: the source is a **dead-after owned local**, so its store transfers into
/// the field/element (C86's last-use elision — the same rule `ElidePlan` runs for the var-buffer
/// idiom). Produced only under `LOFT_MOVE_ELIDE`. B1.2 computes + dumps it; the lowering that
/// consumes it lands in B1.3+ (no codegen change yet — byte-identical off). Design + captured IR:
/// `doc/claude/plans/90-copy-diagnostics/phase-b-design.md`.
#[derive(Clone, Debug)]
pub struct MovePlan {
    /// The container being built / assigned into (struct rec or vector target); `u16::MAX` if it
    /// has no named var (an anonymous element slot).
    pub container: u16,
    /// The dead-after owned-local source var whose store transfers into the field/element.
    pub source: u16,
    /// Construction (`S { f: src }` / `x.field += src`) vs record (`v[i] = e` / `o.f = src`).
    pub kind: MoveKind,
    /// The copy-site-end position — the lowering (B1.3) keys the transfer + free-suppression here.
    pub copy_end: usize,
    /// The source location, for the B1.2 detection dump.
    pub loc: Option<Position>,
}

/// The copy shape a [`MovePlan`] covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    /// `S { f: src }` / `x.field += src` — a field-target append (`construct_copy`).
    Construct,
    /// `v[i] = e` / `o.f = src` — an `OpCopyRecord` (`record_copy`).
    Record,
}

impl Uses {
    /// @PLN90 item 3 — record a real WRITE of `base` at `pos` (max). Only tracked when
    /// `track_pos` (read solely by `survival_class`), so the default path is unaffected.
    fn mark_write(&mut self, base: Option<u16>, pos: usize) {
        if self.track_pos
            && let Some(v) = base
        {
            let e = self.mut_max_pos.entry(v).or_insert(0);
            *e = (*e).max(pos);
        }
    }

    /// @PLN90 item 4 — is a copy of `src` at the current site a repeated duplicate of a value
    /// that lives OUTSIDE the enclosing loop (⇒ unbound, one copy per iteration), rather than a
    /// per-iteration local consumed each pass (⇒ a move)? False outside any loop. A source with
    /// no `Set` def (a parameter) is outside by definition.
    fn loop_survives(&self, src: Option<u16>) -> bool {
        if self.loop_depth == 0 {
            return false;
        }
        let Some(&entry) = self.loop_entry.last() else {
            return false;
        };
        match src {
            None => false,
            Some(s) => self.first_def_pos.get(&s).is_none_or(|&d| d < entry),
        }
    }

    fn visit(&mut self, node: &Value, ctx: Ctx) {
        let pos = self.pos;
        self.pos += 1;
        // @PLN90 item 2 — breadcrumb the nearest enclosing span position (only when tracking,
        // so the default path pays nothing). A copy op borrows this for its report location.
        if self.track_pos {
            if let Some(p) = node.span_pos() {
                self.cur_pos = Some(p.clone());
            } else if let Value::Line(n) = node {
                // @PLN90 S5.2 — a bare line marker is a coarse fallback for copies that
                // sit under no span (an inline construct's `OpAppendVector`, an `[]` fold).
                // Empty `file` signals "borrow the caller's source file" (report/warn fill
                // it); a real span later in the same statement overrides this.
                self.cur_pos = Some(Position {
                    file: String::new(),
                    line: *n,
                    pos: 0,
                });
            }
        }
        // @PLN90 item 3 — record a REAL write of a var at this position, for `mut_max_pos` (the
        // Forced test). Covers every write op (first-arg family + `OpCopyRecord`'s second-arg
        // dest); the `Set` arm below adds reassign targets. Gated inside `mark_write`.
        if let Value::Call(d, args) = node.unspan() {
            if self.write_first_arg.contains(d) {
                self.mark_write(args.first().and_then(|a| base_var(a, self.get_field)), pos);
            } else if *d == self.op_copy_record {
                self.mark_write(args.get(1).and_then(|a| base_var(a, self.get_field)), pos);
            }
        }
        match node.unspan() {
            Value::Var(v) => {
                // @PLN90 — last use in ANY position (the survival discriminator).
                let lu = self.last_use_pos.entry(*v).or_insert(0);
                *lu = (*lu).max(pos);
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
                // @PLN90 item 4 — the loop's entry position: a source whose first def is < this
                // lives outside the loop (copied every iteration); >= means a per-iteration local.
                if self.track_pos {
                    self.loop_entry.push(pos);
                }
                for op in &b.operators {
                    self.visit(op, Ctx::Other);
                }
                if self.track_pos {
                    self.loop_entry.pop();
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
                // @PLN90 item 3 — a `Set` writes `v` (its def is before any copy of it; a
                // reassign after a copy is the mutation that forces the copy to be independent).
                self.mark_write(Some(*v), pos);
                // @PLN90 item 4 — remember where `v` was FIRST defined (loop-inside vs -outside).
                if self.track_pos {
                    self.first_def_pos.entry(*v).or_insert(pos);
                }
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
                // `OpAppendVector(target, src, rec_tp)`. (The append target's write is recorded
                // by the general write-mark at the top of `visit`.)
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
                    // @PLN90 — when the target is a struct/enum FIELD
                    // (`OpAppendVector(OpGetField(rec, fld), src)`) the source is
                    // deep-copied into the field: a structure copy the var-buffer idiom
                    // above never records (struct/enum construction `S { f: src }`, or
                    // `x.field += src`). Capture (base record, source base) so the
                    // copy-vs-borrow decision + survival split cover it.
                    let cc = if let Some(Value::Call(d0, _)) = args.first().map(Value::unspan)
                        && *d0 == self.get_field
                    {
                        let rec = args.first().and_then(|t| base_var(t, self.get_field));
                        let src = args.get(1).and_then(|s| base_var(s, self.get_field));
                        Some((rec, src))
                    } else {
                        None
                    };
                    for a in args {
                        self.visit(a, Ctx::Other);
                    }
                    // Position taken AFTER the args are walked: a use strictly greater is a
                    // use of the source AFTER this copy (⇒ the source survives). The copy op
                    // carries no span, so borrow the nearest enclosing one (item 2). `loop_surv`
                    // (item 4) = the source outlives the enclosing loop (copied every iteration).
                    if let Some((rec, src)) = cc {
                        let loop_surv = self.loop_survives(src);
                        self.construct_copy.push((
                            rec,
                            src,
                            self.pos,
                            loop_surv,
                            self.cur_pos.clone(),
                        ));
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
            Value::Call(d, args) if *d == self.op_copy_record => {
                // @PLN90 — `OpCopyRecord(source, dest, tp)` deep-copies one record: arg 0 is the
                // SOURCE (`data`), arg 1 is the DEST (`to`) — verified against `State::copy_record`
                // (io.rs) and `parser::find_written_vars` (dest = second arg). The survival split
                // classifies the SOURCE's fate, and the row's target/name is the DEST. Skip the
                // same-var no-op alias (`OpCopyRecord(x, x)` — the runtime short-circuits it).
                // The dest's write is recorded by the general write-mark at the top of `visit`.
                let src = args.first().and_then(|a| base_var(a, self.get_field));
                let dest = args.get(1).and_then(|a| base_var(a, self.get_field));
                let record = !(dest.is_some() && dest == src);
                let c = if self.value_readers.contains(d) {
                    Ctx::ReaderArg
                } else {
                    Ctx::Other
                };
                for a in args {
                    self.visit(a, c);
                }
                // Position AFTER the args: a later use of the source ⇒ it survives. The
                // `OpCopyRecord` carries no span, so borrow the nearest enclosing one. The row's
                // target/name is the DEST; the classified var is the SOURCE (`src`). `loop_surv`
                // (item 4) = the source outlives the enclosing loop (copied every iteration).
                if record {
                    let loop_surv = self.loop_survives(src);
                    self.record_copy
                        .push((dest, src, self.pos, loop_surv, self.cur_pos.clone()));
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
/// Build + walk the copy/borrow use-facts for one function body. Extracted from `analyze_fn` so the
/// report-only link-safety oracle (`link_safety_of`) reads the SAME facts the shipped elision does,
/// with no second, drifting analysis.
fn collect_uses(code: &Value, data: &Data, survival_on: bool) -> Uses {
    let mut u = Uses {
        get_field: data.def_nr("OpGetField"),
        op_append: data.def_nr("OpAppendVector"),
        op_database: data.def_nr("OpDatabase"),
        op_free: data.def_nr("OpFreeRef"),
        op_copy_record: data.def_nr("OpCopyRecord"),
        projections: projection_ops(data),
        value_readers: value_reader_ops(data),
        write_first_arg: first_arg_write_ops(data),
        ineligible: HashSet::new(),
        def_count: HashMap::new(),
        database_vars: HashSet::new(),
        def_vdb: HashMap::new(),
        append_src: HashMap::new(),
        append_expr: HashMap::new(),
        pos: 0,
        loop_depth: 0,
        track_pos: survival_on,
        cur_pos: None,
        mut_max_pos: HashMap::new(),
        first_def_pos: HashMap::new(),
        loop_entry: Vec::new(),
        other_max_pos: HashMap::new(),
        copyfill_pos: HashMap::new(),
        copyfill_in_loop: HashSet::new(),
        last_use_pos: HashMap::new(),
        construct_copy: Vec::new(),
        record_copy: Vec::new(),
    };
    u.visit(code, Ctx::Other);
    u
}

/// @PLN102 transparent-link widening — the per-bind SAFETY predicate (step 2). The single source of
/// truth read by BOTH the report-only oracle (`link_safety_of`) and the codegen widening
/// (`analyze_fn` under `LOFT_LINK_WIDEN`), so the gate-on suite validates exactly what codegen uses.
/// See `link_safety_of` for the invariant + the soundness argument.
fn bind_link_safe(u: &Uses, function: &Function, v: u16, src: Option<u16>) -> bool {
    // A SELF-source (`v += v`, so `append_src[v] = [Some(v)]`) is a self-append, not a copy of a
    // distinct store — a "link" would be `v` borrowing itself, which drops its buffer and leaves it
    // slotless. It is never a link candidate. (The shipped tiers dodge it via `src_is_param` /
    // `src_local_stable` never firing here by default; the widening must exclude it explicitly.)
    if src == Some(v) {
        return false;
    }
    let single_def = u.def_count.get(&v).copied().unwrap_or(0) == 1;
    let v_non_escaping = !u.ineligible.contains(&v);
    let source_outlives = src.is_some_and(|s| {
        function.is_argument(s)
            || (u.def_count.get(&s).copied().unwrap_or(0) <= 1
                && u.last_use_pos.get(&s).copied().unwrap_or(0)
                    >= u.last_use_pos.get(&v).copied().unwrap_or(0))
    });
    single_def && src.is_some() && v_non_escaping && source_outlives
}

/// @PLN102 transparent-link widening — the per-bind OBSERVABILITY predicate (step 3), ALIAS-AWARE.
/// The single source of truth for both `link_observability_of` and the codegen widening. See
/// `link_observability_of` for the invariant. `false` if the bind has no recorded copy-fill.
fn bind_link_unobservable(u: &Uses, function: &Function, v: u16, src: Option<u16>) -> bool {
    let Some(fill) = u.copyfill_pos.get(&v).copied() else {
        return false;
    };
    let local_readonly = !u.ineligible.contains(&v);
    let n = function.next_var();
    let source_stable = src.is_some_and(|base| {
        let base_stable = u.other_max_pos.get(&base).copied().unwrap_or(0) < fill;
        let aliases_stable = (0..n).filter(|&b| b != v && b != base).all(|b| {
            if function.tp(b).base().depend().contains(&base) {
                u.other_max_pos.get(&b).copied().unwrap_or(0) < fill
            } else {
                true
            }
        });
        base_stable && aliases_stable
    });
    local_readonly && source_stable
}

/// @PLN102 transparent-link widening — build step 2: the REPORT-ONLY safety oracle. For each
/// single-source copy-fill bind `v = <src>.f` it returns `(v, base, safe)`, where `safe` is the
/// conservative "a shared-store LINK would be UAF-safe here" verdict:
///
///   * the source `base`'s store OUTLIVES `v` — a parameter (caller-owned, alive across the frame),
///     or a non-reassigned local whose last use is at/after `v`'s (its store is not reclaimed while
///     `v` is live), AND
///   * `v` does NOT escape (`v ∉ ineligible` — no return / store / pass-to-callee that could outlive
///     `base`).
///
/// SOUND BY CONSERVATISM: last-use ordering under-approximates store lifetime (a store lives to scope
/// exit), and `ineligible` also excludes a mutated local, so this can only MISS a safe link, never
/// invent an unsafe one — it cannot green-light a #415 dangle. Drives no codegen. Pinned by
/// `tests/link_safe_oracle.rs` against the safety matrix.
fn link_safety_of(code: &Value, function: &Function, data: &Data) -> Vec<(u16, Option<u16>, bool)> {
    let u = collect_uses(code, data, false);
    let mut vars: Vec<u16> = u.append_src.keys().copied().collect();
    vars.sort_unstable();
    let mut out = Vec::new();
    for v in vars {
        // Only the single-source copy-fill idiom (a fresh `OpDatabase` buffer filled by one append).
        let fresh_buffer = u
            .def_vdb
            .get(&v)
            .is_some_and(|vdb| u.database_vars.contains(vdb));
        let appends = &u.append_src[&v];
        if !fresh_buffer || appends.len() != 1 {
            continue;
        }
        out.push((v, appends[0], bind_link_safe(&u, function, v, appends[0])));
    }
    out
}

/// `LOFT_DUMP_LINK_SAFE` — emit one `link-safe-dbg:` line per copy-fill bind of every USER function
/// (`STD_SOURCE` skipped — its facts are stable and would flood the dump). Report-only.
pub fn dump_link_safety(data: &Data) {
    if !crate::keys::dump_link_safe() {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) || def.source == crate::data::STD_SOURCE {
            continue;
        }
        for (v, base, safe) in link_safety_of(&def.code, &def.variables, data) {
            let base_name = base.map_or("-".to_string(), |b| def.variables.name(b).to_string());
            eprintln!(
                "link-safe-dbg: fn={} var={} base={} safe={}",
                def.name,
                def.variables.name(v),
                base_name,
                u8::from(safe)
            );
        }
    }
}

/// @PLN102 transparent-link widening — build step 3: the REPORT-ONLY observability oracle. For each
/// single-source copy-fill bind `a = s.v` it returns `(a, base, unobservable)`, where a shared-store
/// LINK is UNOBSERVABLE (copy ≡ link) iff **nothing mutates either side's store after the bind**:
///
///   * the local `a` is not mutated after the bind (`a ∉ ineligible` — the copy-idiom-aware
///     read-only fact), AND
///   * the SOURCE store is not mutated after the bind — `other_max_pos[base] < fill` (no non-reader
///     use of the base after the copy-fill), ALIAS-AWARE: every var whose store aliases the base (its
///     `deps` reference `base` — a sibling `&`-reference, an element view) is ALSO stable after the
///     fill. Without the alias clause, `a = s.v; b = &s.v; b[i]=x; read a` would falsely read
///     unobservable (the write via `b` reaches `s.v`'s store, so a link would reflect it).
///
/// SOUND BY CONSERVATISM: `other_max_pos` counts a pre-bind def too, but the bind's own fill is later,
/// so a stable source reads `< fill`; a set-based over-count only MISSES a link, never invents an
/// observable one. Drives no codegen. Pinned by `tests/link_obs_oracle.rs` against Matrix O.
fn link_observability_of(
    code: &Value,
    function: &Function,
    data: &Data,
) -> Vec<(u16, Option<u16>, bool)> {
    let u = collect_uses(code, data, false);
    let mut vars: Vec<u16> = u.append_src.keys().copied().collect();
    vars.sort_unstable();
    let mut out = Vec::new();
    for a in vars {
        let fresh_buffer = u
            .def_vdb
            .get(&a)
            .is_some_and(|vdb| u.database_vars.contains(vdb));
        let appends = &u.append_src[&a];
        if !fresh_buffer || appends.len() != 1 {
            continue;
        }
        out.push((
            a,
            appends[0],
            bind_link_unobservable(&u, function, a, appends[0]),
        ));
    }
    out
}

/// `LOFT_DUMP_LINK_OBS` — emit one `link-obs-dbg:` line per copy-fill bind of every USER function.
pub fn dump_link_observability(data: &Data) {
    if !crate::keys::dump_link_obs() {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) || def.source == crate::data::STD_SOURCE {
            continue;
        }
        for (v, base, unobs) in link_observability_of(&def.code, &def.variables, data) {
            let base_name = base.map_or("-".to_string(), |b| def.variables.name(b).to_string());
            eprintln!(
                "link-obs-dbg: fn={} var={} base={} unobs={}",
                def.name,
                def.variables.name(v),
                base_name,
                u8::from(unobs)
            );
        }
    }
}

fn analyze_fn(
    code: &Value,
    function: &Function,
    data: &Data,
    max_tier: u8,
) -> (Vec<VerdictRow>, Vec<ElidePlan>, Vec<MovePlan>) {
    // @PLN90 — the survival split + its report locations are produced under LOFT_COPY_SURVIVAL
    // (the raw dev dump) OR the user-facing report (`report_copies`); read once so both the walk
    // (`track_pos`) and the classification below agree.
    let survival_on = std::env::var_os("LOFT_COPY_SURVIVAL").is_some()
        || crate::keys::report_copies_enabled()
        || crate::keys::warn_copies_enabled();
    let u = collect_uses(code, data, survival_on);

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
            // @PLN90 phase 1 — the return-buffer copy. When the single-append target is
            // not a fresh local buffer but IS an argument vector, it is the return buffer
            // (a passed-in buffer the function fills and returns): `fn f(b: Box) -> vector
            // { b.rows }` materialises `b.rows` into `__retbuf`. The var-buffer idiom skips
            // it (the buffer is a param, not an `OpDatabase` local), so emit a Copy row for
            // coverage. Diagnostic only — `continue` below means no `ElidePlan`, so no
            // codegen change (and eliding it would be the P4 borrowed-return).
            if appends.len() == 1 && function.is_argument(v) {
                rows.push(VerdictRow {
                    var_nr: v,
                    var_name: function.name(v).to_string(),
                    source: appends[0].unwrap_or(u16::MAX),
                    verdict: Verdict::Copy,
                    reason: "materialised into the return buffer (field / whole-vector return copy)",
                    // AVOIDABLE: returning a borrowed view of the field is sound (the caller
                    // keeps the subject alive); the copy is here only because the borrowed
                    // return path is not yet correct (@PLN85 P4). Eliminating it = that fix.
                    class: CopyClass::Avoidable,
                    loc: None,
                    survival: false,
                });
            }
            continue; // not a single-source local copy — not ours to elide
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

        // @PLN90 — the warning bucket. Avoidable = a borrow would be sound, blocked only by
        // analysis conservatism (an unproven read-only local source) — the worklist.
        // Implicit = a literal source is construction, not a copy of existing data (owned by
        // the model — silent). Forced = the result escapes / is reassigned / mutated, or the
        // source is mutated (the value must own its store). `Borrow` is already eliminated.
        let (verdict, reason, class) = if single_def && v_readonly && src_is_param && src_unmutated
        {
            (
                Verdict::Borrow,
                "tier0: read-only local, unmutated param source",
                CopyClass::Eliminated,
            )
        } else if single_def && v_readonly && src_local_stable {
            (
                Verdict::Borrow,
                "tier1: read-only local, ordering-proven read-only local source",
                CopyClass::Eliminated,
            )
        } else if crate::keys::link_widen_enabled()
            && bind_link_safe(&u, function, v, src)
            && bind_link_unobservable(&u, function, v, src)
        {
            // @PLN102 build step 4 — the transparent-link WIDENING (gated `LOFT_LINK_WIDEN`). A bind
            // the tiers above leave as a copy is realized as a shared-store link when it is provably
            // SAFE (source outlives the local, no escape) AND UNOBSERVABLE (neither side's store is
            // mutated after the bind, ALIAS-AWARE) — the two oracles proven report-only in steps 2/3.
            // The observable result is unchanged (copy ≡ link here); it just realizes more links. The
            // `ElidePlan` production below runs unchanged, incl. its own borrower-safety gate. Dead
            // when the flag is off ⇒ byte-identical.
            (
                Verdict::Borrow,
                "widen: safe + unobservable link (LOFT_LINK_WIDEN)",
                CopyClass::Eliminated,
            )
        } else if src.is_none() {
            (
                Verdict::Copy,
                "source is not a plain var/field (e.g. a literal)",
                CopyClass::Implicit,
            )
        } else if !single_def {
            (
                Verdict::Copy,
                "reassigned (multiple defs)",
                CopyClass::Forced,
            )
        } else if !v_readonly {
            (Verdict::Copy, "local mutated or escapes", CopyClass::Forced)
        } else if !src_is_param && !src_local_stable {
            (
                Verdict::Copy,
                "source not a parameter / not provably read-only local",
                CopyClass::Avoidable,
            )
        } else {
            (Verdict::Copy, "source mutated", CopyClass::Forced)
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
            // @PLN25 — deps are a lifetime property, agnostic to the `Optional`
            // nullability marker; peel it (`.base()`) so a nullable element
            // borrower (`e = v[i]` typing `Item?` under the DN1 index flip) is
            // still recognised as borrowing `v`. Without the peel its dep is
            // invisible here, so a mutated/escaping borrower is missed and the
            // copy is wrongly elided (the mutation leaks to the source).
            let borrowers: Vec<u16> = (0..function.next_var())
                .filter(|&e| e != v && function.tp(e).base().depend().contains(&v))
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
            class,
            loc: None,
            survival: false,
        });
    }

    // @PLN90 phase 1 — construction / field-append copies. A field-target append
    // (`S { f: src }` construction, or `x.field += src`) deep-copies the source into the
    // field, which the var-buffer idiom above does not classify. Emit a Copy row so the
    // copy-vs-borrow decision COVERS the copy. Diagnostic only: always `Copy`, so it never
    // produces an `ElidePlan` (no codegen change). Phase 2 (the bound-vs-unbound survival
    // split, `survival_class`) sorts Implicit/Avoidable/Forced — gated on `LOFT_COPY_SURVIVAL`
    // (`survival_on`, read once at the top of `analyze_fn`).
    for entry in &u.construct_copy {
        let (rec, src, copy_end, loop_surv) = (entry.0, entry.1, entry.2, entry.3);
        // Flag OFF → the original phase-1 classification verbatim (byte-identical). Flag ON →
        // the bound-vs-unbound survival split.
        let (class, reason) = if survival_on {
            survival_class(src, copy_end, loop_surv, &u, function)
        } else {
            (
                CopyClass::Implicit,
                "struct/enum field owns its data (construction/field-append copy)",
            )
        };
        rows.push(VerdictRow {
            var_nr: rec.unwrap_or(u16::MAX),
            var_name: rec.map_or_else(|| "<field>".to_string(), |r| function.name(r).to_string()),
            source: src.unwrap_or(u16::MAX),
            verdict: Verdict::Copy,
            reason,
            class,
            loc: entry.4.clone(),
            survival: true,
        });
    }

    // @PLN90 phase 1 — record deep-copies (`OpCopyRecord`): a `v[i] = e` element-slot set,
    // a `?? E{…}` default element, a struct copy. Not append-based, so the var-buffer /
    // construction / return-buffer paths above never see them. Emit a Copy row so the
    // decision covers the copy. Diagnostic only — never an `ElidePlan`, no codegen change.
    for entry in &u.record_copy {
        let (tgt, src, copy_end, loop_surv) = (entry.0, entry.1, entry.2, entry.3);
        // Flag OFF → the original phase-1 classification verbatim (byte-identical). Flag ON →
        // the bound-vs-unbound survival split.
        let (class, reason) = if survival_on {
            survival_class(src, copy_end, loop_surv, &u, function)
        } else {
            (CopyClass::Implicit, "record deep-copy (OpCopyRecord)")
        };
        rows.push(VerdictRow {
            var_nr: tgt.unwrap_or(u16::MAX),
            var_name: tgt.map_or_else(|| "<record>".to_string(), |t| function.name(t).to_string()),
            source: src.unwrap_or(u16::MAX),
            verdict: Verdict::Copy,
            reason,
            class,
            loc: entry.4.clone(),
            survival: true,
        });
    }

    // @PLN90 phase B (B1.2) — the MOVE-elision plans: a construction/record copy whose source is a
    // dead-after owned local can transfer its store into the field/element instead of copying.
    // Gated on `LOFT_MOVE_ELIDE`; empty otherwise (default zero-cost + byte-identical). No lowering
    // consumes these yet — B1.2 only computes + dumps them to prove detection.
    let mut move_plans: Vec<MovePlan> = Vec::new();
    if crate::keys::move_elide_enabled() {
        for entry in &u.construct_copy {
            if let Some(s) = move_elidable_source(entry.1, entry.2, entry.3, &u, function) {
                move_plans.push(MovePlan {
                    container: entry.0.unwrap_or(u16::MAX),
                    source: s,
                    kind: MoveKind::Construct,
                    copy_end: entry.2,
                    loc: entry.4.clone(),
                });
            }
        }
        for entry in &u.record_copy {
            if let Some(s) = move_elidable_source(entry.1, entry.2, entry.3, &u, function) {
                move_plans.push(MovePlan {
                    container: entry.0.unwrap_or(u16::MAX),
                    source: s,
                    kind: MoveKind::Record,
                    copy_end: entry.2,
                    loc: entry.4.clone(),
                });
            }
        }
    }
    (rows, plans, move_plans)
}

/// @PLN90 — the bound-vs-unbound survival split for a construction / record copy. The
/// silent/indicate line is keyed on the copy's SOURCE FATE, never on the emitting op
/// (COPY_DIAGNOSTICS.md § bound vs unbound):
///
/// - **bound → `Implicit` (silent):** a literal / freshly-built source (nothing pre-existing
///   is duplicated), or a **move** (the source is consumed at the copy — no use strictly
///   after the copy site — so its single backing transfers).
/// - **unbound → indicated:** a still-live source is duplicated into an independent structure.
///   `Avoidable` when the survivor is read-only (a borrow/move would have avoided the copy —
///   the worklist); `Forced` when the source is mutated after the copy (an independent copy is
///   genuinely required). `loop_surv` (item 4): a copy inside a loop whose source is defined
///   OUTSIDE the loop is a duplicate made every iteration → also a survivor (indicated); a
///   per-iteration local source is a move (silent).
///
/// Called only when `LOFT_COPY_SURVIVAL` is set — the flag-OFF path keeps the original
/// phase-1 classification verbatim at the call site, so the default dump stays byte-identical.
fn survival_class(
    src: Option<u16>,
    copy_end: usize,
    loop_surv: bool,
    u: &Uses,
    function: &Function,
) -> (CopyClass, &'static str) {
    let Some(s) = src else {
        return (
            CopyClass::Implicit,
            "born-owned: literal / freshly-built source — no live structure duplicated",
        );
    };
    // The copy's OWN read of the source is at a position <= copy_end, so a use strictly after
    // is a genuine later use — the source survives, an independent duplicate now coexists.
    let survives_straight = u.last_use_pos.get(&s).is_some_and(|&p| p > copy_end);
    if !survives_straight && !loop_surv {
        // A move — the source is consumed here (a per-iteration local, in the loop case). Bound,
        // silent for everyone; not a copy to eliminate.
        return (
            CopyClass::Implicit,
            "move: source consumed at the copy — its single backing transfers",
        );
    }
    let (class, reason) = if u.mut_max_pos.get(&s).is_some_and(|&p| p > copy_end) {
        // @PLN90 item 3 — the source is genuinely WRITTEN after the copy (reassigned, appended
        // to, record-copied into): a borrow would let that mutation leak into the copy's owner,
        // so the independent copy is forced. This is precise — unlike the old `other_max_pos`
        // test it does NOT count a read-only pass-to-callee or being another copy's source.
        // (Residual, conservative-toward-Avoidable: a mutating-callee on a single-def LOCAL is
        // not caught → shows Avoidable; the phase-B elision analysis is the real borrow checker
        // and rejects an unsound candidate, so a false-Avoidable is safe for the worklist.)
        (
            CopyClass::Forced,
            "unbound: source survives AND is written after — an independent copy is required",
        )
    } else if loop_surv && !survives_straight {
        // @PLN90 item 4 — a value defined outside the loop, copied every iteration.
        (
            CopyClass::Avoidable,
            "in-loop copy of a value defined outside the loop — duplicated every iteration; a borrow would avoid it",
        )
    } else {
        // A read-only survivor (read, or passed read-only) — the avoidable worklist.
        (
            CopyClass::Avoidable,
            "unbound: source survives read-only — a borrow/move would avoid this copy",
        )
    };
    // @PLN90 item 1 — an INDICATED copy whose SOURCE is a compiler-generated temporary
    // (`_`-prefixed) is not user-actionable: the user never wrote `__ref_N` / `_comp_N`. Route
    // it to `Internal` so the user-facing report excludes it while the developer worklist still
    // counts it (it may be a copy WE can eliminate).
    if function.is_compiler_generated(s) {
        return (
            CopyClass::Internal,
            "compiler-internal source (_-prefixed) — a copy we may eliminate; excluded from the user report",
        );
    }
    (class, reason)
}

/// @PLN90 phase B (B1.2) — is this construction/record copy site a MOVE-elidable one, and if so,
/// which source var transfers? A site is move-elidable iff the source is the phase-A survival
/// **move** (consumed at the copy: not straight-line-surviving, not loop-repeated) AND it meets the
/// elision preconditions: it is a **local** (not a parameter — the caller owns a param's store) that
/// **owns a transferable store** (a vdb-buffered vector `def_vdb`, or a fresh `OpDatabase`'d record
/// `database_vars` — never a view/projection, which owns no store to move). Conservative by design:
/// a false negative just keeps the copy; a false positive would be an unsound move, so the checks
/// only widen with proof. Returns `Some(source)` when elidable.
fn move_elidable_source(
    src: Option<u16>,
    copy_end: usize,
    loop_surv: bool,
    u: &Uses,
    function: &Function,
) -> Option<u16> {
    let s = src?; // a literal source has nothing pre-existing to move
    if function.is_argument(s) {
        return None; // a parameter's store belongs to the caller — not ours to transfer
    }
    // Survival MOVE only: a source that survives (read after) or is copied every loop iteration
    // must keep its COPY (C86) — never move.
    let survives = u.last_use_pos.get(&s).is_some_and(|&p| p > copy_end);
    if survives || loop_surv {
        return None;
    }
    // Owns a transferable store.
    if u.def_vdb.contains_key(&s) || u.database_vars.contains(&s) {
        Some(s)
    } else {
        None
    }
}

/// @PLN90 phase B (B1.2) — the move-elidable construction/record copies of function `d_nr` (a
/// dead-after owned-local source whose store can transfer into the field/element). Empty unless
/// `LOFT_MOVE_ELIDE` is set. No lowering consumes these yet; this is the detection the B1.3+
/// lowering will read. See [`MovePlan`] and `phase-b-design.md`.
#[must_use]
pub fn move_plans(data: &Data, d_nr: u32) -> Vec<MovePlan> {
    let def = data.def(d_nr);
    analyze_fn(&def.code, &def.variables, data, env_tier()).2
}

/// @PLN90 phase B — dump every move-elidable site when `LOFT_MOVE_ELIDE` is set (the opt-in
/// diagnostic; the rewrite itself is default-on). A no-op otherwise, so the default-on elision does
/// not spew plan lines. The dump is the positive control that detection fires on the dead-source
/// shapes and stays silent on survivors.
pub fn dump_move_plans(data: &Data) {
    if !crate::keys::move_elide_dump_enabled() {
        return;
    }
    let mut total = 0u32;
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        for p in analyze_fn(&def.code, &def.variables, data, env_tier()).2 {
            total += 1;
            let at = p
                .loc
                .as_ref()
                .map_or_else(String::new, |l| format!(" at {l}"));
            let container = if p.container == u16::MAX {
                "<slot>".to_string()
            } else {
                def.variables.name(p.container).to_string()
            };
            eprintln!(
                "MOVE-PLAN fn={} kind={:?} container={} source={}{}",
                def.name,
                p.kind,
                container,
                def.variables.name(p.source),
                at
            );
        }
    }
    eprintln!("MOVE-PLAN total={total}");
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
// This classification is consumed by DEFAULT-ON codegen: `ownership_of` (below) is
// the one fact every own-vs-borrow site reads instead of re-deriving — interp
// `state/codegen.rs`, native `generation/dispatch.rs`, gated `join_own_enabled`. The
// `ownership_*` tests + the @PLN89 differential oracle validate it. Design study:
// `doc/claude/plans/85-store-lifetime-retirement/over-free-class-study.md`
// (§ Three chokepoints).
//
// APPROXIMATIONS (sound by conservatism — a value can only OVER-report Join/Borrowed):
//   * Var resolution is flow-INSENSITIVE — a var classifies as the join of ALL
//     its real (non-`= null`-init) defs across the body, not the def that reaches
//     the point of use. For the over-free shapes (single-def views, owned-then-
//     reassigned slots) this gives the right answer; a genuinely path-split var
//     can only over-report Join, never wrongly report Owned.
//   * A bare PARAMETER used as a value classifies as `Borrowed` (the caller owns
//     it) — including a retbuf param a callee fills in place, which is really
//     owned. Conservative: it can lose an Owned, never invent one.
// ============================================================================

/// The store-ownership of a value at an own-vs-borrow decision site — THE one fact
/// every such site READS instead of re-deriving (the OWNERSHIP_MODEL north star).
/// `Borrowed`/`Join` carry the `base`: the caller-visible var whose store the value
/// aliases — the witness the `Join` runtime guard needs and what distinguishes a
/// borrowed-of-arg value from an owned one. `base` is `u16::MAX` when unresolvable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Own {
    /// A freshly minted store the producer owns (an `OpDatabase`/`OpNewRecord`
    /// buffer, a struct literal, a call whose return adopts a fresh store). Free /
    /// adopt / set the source-free bit is correct.
    Owned,
    /// A view into `base`'s store (a vector-element / field projection, a returned
    /// parameter, or a value that resolves to one). Never free it; to land in an
    /// owned slot, deep-copy.
    Borrowed { base: u16 },
    /// Runtime-dependent: owned on one branch, borrowed-of-`base` on the other (a
    /// `??` / `if-else` whose arms split). The decision is per-execution — adopt iff
    /// the value's store ≠ `base`'s store (the owned branch ran), else materialise.
    Join { base: u16 },
}

impl Own {
    /// The base var a `Borrowed`/`Join` value aliases, or `None` for `Owned`.
    #[must_use]
    fn base(self) -> Option<u16> {
        match self {
            Own::Owned => None,
            Own::Borrowed { base } | Own::Join { base } => Some(base),
        }
    }

    /// The lattice join of two `??`/`if` arms. Two equal borrows of the SAME base
    /// stay `Borrowed`; any owned-vs-borrowed split (or differing bases) becomes a
    /// `Join` witnessed by whichever arm carries a base.
    #[must_use]
    fn join(self, other: Own) -> Own {
        match (self, other) {
            (Own::Owned, Own::Owned) => Own::Owned,
            (Own::Borrowed { base: a }, Own::Borrowed { base: b }) if a == b => {
                Own::Borrowed { base: a }
            }
            _ => Own::Join {
                base: self.base().or_else(|| other.base()).unwrap_or(u16::MAX),
            },
        }
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
    /// Vars currently being classified — the var-level twin of [`Self::visiting`].
    /// A self-referential default (`c = t[k] ?? c`) makes a var's RHS mention the var
    /// itself, so `classify` recursed forever and overflowed the stack: a SIGSEGV
    /// during COMPILATION, reproducible with `--check` on two lines of valid-looking
    /// source (crawler LOFT-HANDOFF H1).
    visiting_vars: HashSet<u16>,
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
            visiting_vars: HashSet::new(),
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
        // recover — the flattened canonical fact is exact. Its base is the first
        // VISIBLE param the return dep names (in this callee's own var space).
        if tail.is_none() || !matches!(def.def_type, DefType::Function) {
            let attrs = def.attributes();
            let c = if def.returns_borrowed_view() {
                let base = def
                    .returned()
                    .depend()
                    .iter()
                    .find(|&&a| (a as usize) < attrs.len() && !attrs[a as usize].hidden)
                    .map_or(u16::MAX, |&a| a);
                Own::Borrowed { base }
            } else {
                Own::Owned
            };
            self.ret_memo.insert(d_nr, c);
            return c;
        }
        if !self.visiting.insert(d_nr) {
            // Recursion back-edge: conservatively Borrowed (never assume a self-
            // referential return is freshly owned), base unresolved. Not memoised —
            // the enclosing frame computes and caches the real class.
            return Own::Borrowed { base: u16::MAX };
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
            Value::Var(v) if !self.visiting_vars.insert(*v) => {
                // Recursion back-edge: this var appears in its OWN definition, as in
                // `c = t[k] ?? c`.  Mirrors the function-level guard below — return
                // conservatively rather than recursing.  `Borrowed { base: MAX }`
                // (never "freshly owned") joins with the real arm to `Join`, the
                // owned-vs-borrowed split the reassign check already treats as the
                // risky shape; the alternative was an unbounded recursion that took
                // the compiler down with it.
                Own::Borrowed { base: u16::MAX }
            }
            Value::Var(v) => {
                let class = match defs.rhs.get(v) {
                    Some(rhss) if !rhss.is_empty() => rhss
                        .iter()
                        .map(|r| self.classify(r, func, defs))
                        .reduce(Own::join)
                        .unwrap_or(Own::Owned),
                    // No local def: a parameter (the caller owns it ⇒ Borrowed of
                    // itself) or an uninitialised local (Owned — nothing to mis-free).
                    _ => {
                        if func.is_argument(*v) {
                            Own::Borrowed { base: *v }
                        } else {
                            Own::Owned
                        }
                    }
                };
                self.visiting_vars.remove(v);
                class
            }
            Value::Call(d, args) => {
                if *d == self.op_database || *d == self.op_new_record {
                    Own::Owned
                } else if self.projections.contains(d) {
                    // A projection (`OpGetField`/`OpGetVector*`/`OpGetDbRef`) is a
                    // view into its base container (arg 0), rooted at a var.
                    match self.borrow_base(node, func, defs) {
                        Some(base) => Own::Borrowed { base },
                        None => Own::Owned,
                    }
                } else {
                    self.call_ownership(*d, args)
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

    /// The ownership of a `call(args)` result, with the borrow base translated from
    /// the callee's parameter space into the CALLER's argument (the interprocedural
    /// piece): the callee's return borrows one of its visible params; map that param
    /// position to the caller's argument so the `base` is a var the caller can witness.
    fn call_ownership(&mut self, callee_d: u32, caller_args: &[Value]) -> Own {
        let callee_own = self.return_ownership(callee_d);
        let callee_base = match callee_own {
            Own::Owned => return Own::Owned,
            Own::Borrowed { base } | Own::Join { base } => base,
        };
        let base = self.caller_arg_base(callee_d, callee_base, caller_args);
        match callee_own {
            Own::Join { .. } => Own::Join { base },
            _ => Own::Borrowed { base },
        }
    }

    /// Map the callee's borrowed parameter `callee_base` (a var in the callee's
    /// space) to the CALLER's argument var at the same VISIBLE-parameter position.
    /// `u16::MAX` when it is not a visible param or the matching arg is not a var.
    fn caller_arg_base(&self, callee_d: u32, callee_base: u16, caller_args: &[Value]) -> u16 {
        let attrs = self.data.def(callee_d).attributes();
        if callee_base == u16::MAX
            || (callee_base as usize) >= attrs.len()
            || attrs[callee_base as usize].hidden
        {
            return u16::MAX;
        }
        // The caller's args align with the callee's VISIBLE params, in order.
        let arg_index = attrs[..callee_base as usize]
            .iter()
            .filter(|a| !a.hidden)
            .count();
        match caller_args.get(arg_index).map(Value::unspan) {
            Some(Value::Var(cv)) => *cv,
            _ => u16::MAX,
        }
    }

    /// The owned-slot reassignments in function `d_nr`: for each var with more than
    /// one real def, the class it held before its LAST def and the class of that
    /// def. The `prior = Owned` rows are the displaced-owned-store leak candidates.
    fn reassign_sites(&mut self, d_nr: u32) -> Vec<ReassignSite> {
        let data = self.data;
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            return Vec::new();
        }
        self.reassign_sites_of(&def.code, &def.variables)
    }

    /// As [`Self::reassign_sites`] but on a `(code, function)` pair directly — for
    /// the scope pass, which holds the function being analysed by reference (it is
    /// not yet written back into `data`).
    fn reassign_sites_of(&mut self, code: &Value, func: &Function) -> Vec<ReassignSite> {
        let mut defs = Defs::default();
        collect_defs(code, self.op_database, &mut defs);
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
        let mut visited = Vec::new();
        self.borrow_base_guarded(node, func, defs, &mut visited)
    }

    /// The recursive worker with CYCLE protection: a var whose def-chain reaches
    /// itself (`bs += […]` reads `bs`; two-var swaps) otherwise recursed forever
    /// — a stack-overflow SIGSEGV once the oracle ran by default (the D-own-1
    /// flip; loft_suite's 85-store-lifetime-return-field-of-local under the wrap
    /// harness).  A revisited var yields `None`: a cyclic chain has no single
    /// borrow base, and every caller handles `None` conservatively.
    fn borrow_base_guarded(
        &self,
        node: &Value,
        func: &Function,
        defs: &Defs,
        visited: &mut Vec<u16>,
    ) -> Option<u16> {
        match node.unspan() {
            Value::Call(d, args) if self.projections.contains(d) => {
                match args.first().map(Value::unspan) {
                    Some(Value::Var(b)) => Some(*b),
                    // nested `o.inner.rows`
                    Some(inner) => self.borrow_base_guarded(inner, func, defs, visited),
                    None => None,
                }
            }
            Value::Var(v) => {
                if visited.contains(v) {
                    return None;
                }
                visited.push(*v);
                if let Some(rhss) = defs.rhs.get(v) {
                    rhss.iter()
                        .rev()
                        .find_map(|r| self.borrow_base_guarded(r, func, defs, visited))
                } else if func.is_argument(*v) && func.tp(*v).heap_dep().is_some() {
                    Some(*v) // a borrowed heap param IS its own base (the caller's arg)
                } else {
                    None
                }
            }
            Value::If(_, then, els) => self
                .borrow_base_guarded(then, func, defs, visited)
                .or_else(|| self.borrow_base_guarded(els, func, defs, visited)),
            Value::Block(b) => b
                .operators
                .last()
                .and_then(|t| self.borrow_base_guarded(t, func, defs, visited)),
            Value::Return(v) | Value::BreakWith(_, v) => {
                self.borrow_base_guarded(v, func, defs, visited)
            }
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
            if !matches!(class, Own::Borrowed { .. } | Own::Join { .. }) {
                continue;
            }
            let base = class.base().filter(|&b| b != u16::MAX);
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
            let base = class.base().filter(|&b| b != u16::MAX);
            // A retbuf aliases a store it does not own ONLY when set to a DIRECT
            // borrow — a raw projection with a known base (`_mv_items_1 =
            // OpGetField(e,…)`). A call delivering into the retbuf MATERIALISES a
            // copy (the retbuf is then Owned — the clean `best = rows(b)` shape),
            // so `base.is_none()` cases are excluded; this also guarantees a usable
            // materialise base for every reported ParamDeliver.
            if matches!(class, Own::Borrowed { .. } | Own::Join { .. }) && base.is_some() {
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

/// @PLN104 — the loft#568 interpreter-orphan predicate, in ONE place so the promotion
/// oracle (`report_tret_promotions`) and the `--show-ownership` overlay name the same
/// class.  Returns the risk kind when a text-returning fn hands owned text back BY VALUE:
/// its return is backed FRAME-LOCALLY (`Owned`, or a `Borrowed`/`Join` of a LOCAL — not an
/// argument the caller owns and outlives the frame) AND is not already delivered through a
/// hidden `&text` retbuf.  The interpreter orphans such a return (the `String` dies with
/// the frame); native RAII drops it.  `None` = safe: already buffered, borrows an argument,
/// or not a text return.
#[must_use]
pub fn text_return_orphan_risk(data: &Data, d_nr: u32) -> Option<&'static str> {
    use crate::data::Type;
    let def = data.def(d_nr);
    if !matches!(def.returned().base(), Type::Text(_)) {
        return None;
    }
    let has_buf = def
        .attributes()
        .iter()
        .any(|a| matches!(a.typedef, Type::RefVar(ref t) if matches!(**t, Type::Text(_))));
    if has_buf {
        return None;
    }
    let borrows_arg = |base: u16| base != u16::MAX && def.variables.is_argument(base);
    match return_ownership(data, d_nr) {
        Own::Owned => Some("owned-by-value"),
        Own::Borrowed { base } if !borrows_arg(base) => Some("view-of-local"),
        Own::Join { base } if !borrows_arg(base) => Some("join-of-local"),
        _ => None,
    }
}

/// Public, test-facing entry: the owned-slot reassignment sites of function `d_nr`.
#[must_use]
pub fn reassign_sites(data: &Data, d_nr: u32) -> Vec<ReassignSite> {
    Ownership::new(data).reassign_sites(d_nr)
}

/// The `local_source` over-free fix's input: the heap slots that hold an OWNED
/// store displaced by a later `Borrowed`/`Join` reassignment (`prior=Owned`). The
/// scope pass strips these slots' deps so the owned path deep-copies + frees them
/// (the displaced store would otherwise be orphaned — the `chosen = dflt(); … chosen
/// = pool[wj]` leak). Operates on the pre-scope `(code, function)` directly.
#[must_use]
pub fn displaced_owned_slots(code: &Value, function: &Function, data: &Data) -> HashSet<u16> {
    Ownership::new(data)
        .reassign_sites_of(code, function)
        .into_iter()
        .filter(|s| {
            matches!(s.prior, Own::Owned)
                && matches!(s.rhs, Own::Borrowed { .. } | Own::Join { .. })
                // A retbuf-promoted PARAM slot is NOT this fix's territory: the
                // dep-strip would disable the dep-carrying explicit reassign-free
                // (the p462 `_rb_w_` witness partner) while codegen's dep-empty
                // pre-Set free excludes arguments — the displaced first store
                // then leaks one per call (p462_cond_reassign_retbuf under
                // gate-ON, M×N).  Param-slot displaced frees stay with the
                // witness mechanism.
                && !function.is_argument(s.var)
        })
        .map(|s| s.var)
        .collect()
}

/// Public, test-facing entry: the over-free free SITES of function `d_nr` (the
/// append element source-frees + return-buffer-aliasing deliveries, with class +
/// borrow base) — the Gap-A/B context the Stage-3 fix reads.
#[must_use]
pub fn free_sites(data: &Data, d_nr: u32) -> Vec<FreeSite> {
    Ownership::new(data).free_sites(d_nr)
}

/// THE own-vs-borrow oracle: the ownership of `value` as produced in function
/// `d_nr`, with the borrow `base` resolved (interprocedurally for a call — the
/// callee's borrowed param mapped to the caller's argument). This is the ONE fact
/// every own-vs-borrow chokepoint READS instead of re-deriving — the unification
/// entry point (the OWNERSHIP_MODEL north star).
#[must_use]
pub fn ownership_of(data: &Data, d_nr: u32, value: &Value) -> Own {
    let def = data.def(d_nr);
    let mut own = Ownership::new(data);
    let mut defs = Defs::default();
    collect_defs(&def.code, own.op_database, &mut defs);
    own.classify(value, &def.variables, &defs)
}

/// True when `classify` resolves a `Call(d, …)` STRUCTURALLY — a store mint
/// (`OpDatabase`/`OpNewRecord` → `Owned`) or a projection (`OpGetField` &c → the
/// `Borrowed(base)` view into arg 0) — rather than through the callee's
/// interprocedural return summary (`call_ownership`). The @PLN94 oracle's transfer
/// routes only NON-structural (genuine user/native) calls through its independent
/// `call_own`; these primitives carry fixed ownership semantics both analyses share,
/// so it must delegate them to `ownership_of` instead of the summary path (else a
/// projection local is mis-classed `Owned` — the over-free/unsound direction).
#[must_use]
pub fn classifies_structurally(data: &Data, d: u32) -> bool {
    d == data.def_nr("OpDatabase")
        || d == data.def_nr("OpNewRecord")
        || projection_ops(data).contains(&d)
}

/// The bare verdict name (no base) — for the free-site dump's `class=` field.
fn own_kind(own: Own) -> &'static str {
    match own {
        Own::Owned => "Owned",
        Own::Borrowed { .. } => "Borrowed",
        Own::Join { .. } => "Join",
    }
}

/// A readable `Owned` / `Borrowed(base=<name>)` / `Join(base=<name>)`, resolving the
/// base var to its name in `func`'s space (`?` when unresolved).
pub fn fmt_own(own: Own, func: &Function) -> String {
    let base = |b: u16| {
        if b == u16::MAX {
            "?".to_string()
        } else {
            func.name(b).to_string()
        }
    };
    match own {
        Own::Owned => "Owned".to_string(),
        Own::Borrowed { base: b } => format!("Borrowed(base={})", base(b)),
        Own::Join { base: b } => format!("Join(base={})", base(b)),
    }
}

/// @PLN103 P1.4 — is `name` a SYNTHESIZED owned buffer (a var whose store the var
/// itself owns: the vector-delivery / NRVO / materialise-copy / return buffers)?
/// A `Borrowed` verdict whose base is one of these (or the var itself) is really an
/// OWNED store held via that buffer, not an alias of a live sibling (P0.2 finding).
#[must_use]
pub fn is_synth_buffer(name: &str) -> bool {
    name.starts_with("__vdb")
        || name.starts_with("__ref")
        || name.starts_with("_mvcopy")
        || name == "__retbuf"
}

/// @PLN107 value-struct completeness — is `ty` a `value struct` local (a `Type::Reference`
/// whose record is marked in `Data.value_structs`)? Such a local always OWNS its store: a
/// value struct read out of a field/element is COPIED (§ `scopes::value_struct_copy`), never
/// aliased, so a never-read mutation of it is a lost write. A plain reference struct is also a
/// `Type::Reference` but NOT value-marked → aliases → not owned here. `&value struct` is a
/// `RefVar` (excluded upstream), so only the bare value-struct binding reaches this.
fn is_value_struct_local(ty: &crate::data::Type, data: &Data) -> bool {
    matches!(ty, crate::data::Type::Reference(d, _) if data.is_value_struct(*d))
}

/// @PLN103 P1.4 — render `own` for the ownership overlay (the P0.2 rendering rule,
/// corrected in P1): `ownership_of` reports a bare argument as `Borrowed{base=self}`
/// (self-base) and an owned delivery buffer as `Borrowed{base=<buffer>}`, neither of
/// which is a dangerous alias — so translate them:
/// - **self-base** (`base == v`): the var is an argument with no local def → it borrows
///   the CALLER's value → `Borrowed(caller-arg)`.
/// - **synthesized-buffer base** (`base != v`, [`is_synth_buffer`]): the var OWNS its
///   store, held via that delivery buffer → `Owned (backing=<name>)`.
/// - any other `Borrowed{base=X}` → a genuine ALIAS of a live sibling `X` (the dangerous
///   case) → `Borrowed(base=<name>)`.
///
/// `Join` is NEVER translated (a real runtime split). RENDERER-ONLY — `ownership_of`
/// stays faithful (Out-of-scope §1); `v` is the var being classified (for self-base).
#[must_use]
pub fn render_own(own: Own, func: &Function, v: u16) -> String {
    let name = |b: u16| {
        if b == u16::MAX {
            "?".to_string()
        } else {
            func.name(b).to_string()
        }
    };
    match own {
        Own::Owned => "Owned".to_string(),
        Own::Borrowed { base: b } if b == v => "Borrowed(caller-arg)".to_string(),
        Own::Borrowed { base: b } if b != u16::MAX && is_synth_buffer(func.name(b)) => {
            format!("Owned (backing={})", name(b))
        }
        Own::Borrowed { base: b } => format!("Borrowed(base={})", name(b)),
        Own::Join { base: b } => format!("Join(base={})", name(b)),
    }
}

/// Print every function's verdicts when `LOFT_MATERIALIZE_DUMP` is set. Called from
/// `scopes::check`; a no-op otherwise. Behaviour-neutral — diagnostics only.
pub fn dump_all(data: &Data) {
    if std::env::var_os("LOFT_MATERIALIZE_DUMP").is_none() {
        return;
    }
    let mut own = Ownership::new(data);
    // @PLN90 — the tally: `avoidable` is the north-star elimination worklist; `implicit`
    // is model-inherent ownership (silent, not a copy-to-fix); `forced` is informational;
    // `internal` is a copy of a compiler-generated source — a developer-worklist copy we may
    // eliminate, but excluded from the user-facing report (item 1).
    let (mut avoidable_copies, mut implicit_copies, mut forced_copies, mut internal_copies) =
        (0u32, 0u32, 0u32, 0u32);
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        for r in analyze_fn(&def.code, &def.variables, data, env_tier()).0 {
            let bucket = match r.class {
                CopyClass::Eliminated => "eliminated",
                CopyClass::Avoidable => {
                    avoidable_copies += 1;
                    "AVOIDABLE"
                }
                CopyClass::Implicit => {
                    implicit_copies += 1;
                    "implicit"
                }
                CopyClass::Forced => {
                    forced_copies += 1;
                    "forced"
                }
                CopyClass::Internal => {
                    internal_copies += 1;
                    "internal"
                }
            };
            // @PLN90 item 2 — the copy-site location, when the emitting op carried a span.
            // Makes a `<record>` element-set copy (no named target) actionable.
            let at = r
                .loc
                .as_ref()
                .map_or_else(String::new, |p| format!(" at {p}"));
            eprintln!(
                "MAT fn={} v={}({}) src={} verdict={:?} bucket={} [{}]{}",
                def.name, r.var_nr, r.var_name, r.source, r.verdict, bucket, r.reason, at
            );
        }
        // The ownership fact (inert): the return class + the owned-slot
        // reassignments, each with its borrow base. The over-free leak shape is a
        // `prior=Owned rhs=Join(...)` row.
        let fvars = &def.variables;
        eprintln!(
            "OWN fn={} return={}",
            def.name,
            fmt_own(own.return_ownership(d_nr), fvars)
        );
        for s in own.reassign_sites(d_nr) {
            eprintln!(
                "OWN fn={} reassign v={}({}) prior={} rhs={}",
                def.name,
                s.var,
                s.var_name,
                fmt_own(s.prior, fvars),
                fmt_own(s.rhs, fvars)
            );
        }
        // The free SITES (Gap A) + borrow base (Gap B), now interprocedurally
        // resolved (a call source's base is the CALLER's argument), still inert.
        for s in own.free_sites(d_nr) {
            eprintln!(
                "OWN fn={} free kind={:?} slot={}({}) class={} base={}",
                def.name,
                s.kind,
                s.slot,
                s.slot_name,
                own_kind(s.class),
                s.base_name.as_deref().unwrap_or("-")
            );
        }
    }
    // @PLN90 — the worklist headline: avoidable = the north-star target (teach the analysis
    // to borrow these so they auto-eliminate); implicit = model-inherent ownership (silent);
    // forced = must own by circumstance (informational); internal = compiler-generated source
    // (developer worklist, excluded from the user-facing report).
    eprintln!(
        "MAT-WORKLIST avoidable_copies={avoidable_copies} implicit_copies={implicit_copies} forced_copies={forced_copies} internal_copies={internal_copies}"
    );
}

/// @PLN107 S4a — the ENFORCED dead-store lint (`LOFT_DEAD_STORES`). Warns when a non-escaping
/// local is mutated via an `OpSet*` (`d[i]=x`, `d.f=x`) yet its value is never read AND it OWNS
/// its store — the copy-mutate footgun (`d = self.data; d[i]=x` COPIES, so the write is lost).
///
/// Runs POST-`scopes::check` (called from `main` after the program loads) so the copy-idiom /
/// value-struct-copy rewrites are in place and [`ownership_of`] is reliable: a `Borrowed`/`Join`
/// var aliases a source it borrows (a `&`-reference, a reference-struct field alias, a
/// vector-element view), so its write PROPAGATES and is NOT a dead store — only `Owned` copies
/// warn. The read/write split is [`dead_store_accesses`]; exclusions mirror `test_used`
/// (`_`/`#` temporaries, arguments, closure-captured, global shadows). The `reads==0 &&
/// write_targets>0` signal implies `uses>0`, so this lint and `test_used` (`uses==0`) are
/// disjoint. Populates `diags`; the caller renders them. Sibling of [`warn_copies`].
pub fn warn_dead_stores(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    if !crate::keys::dead_stores_enabled() {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        let func = &def.variables;
        let n = func.var_count();
        let acc = dead_store_accesses(&def.code, n, data);
        for i in 0..n {
            let v = i as u16;
            let name = func.name(v);
            if name.starts_with('_')
                || name.contains('#')
                || func.is_argument(v)
                || func.is_captured(v)
                || data.def_nr(name) != u32::MAX
            {
                continue;
            }
            // A `&`-reference (`RefVar`) explicitly aliases its target; a write through it always
            // propagates, so never a dead store — exclude regardless of the ownership verdict.
            if matches!(func.tp(v), crate::data::Type::RefVar(_)) {
                continue;
            }
            let (reads, write_targets) = acc.get(i).copied().unwrap_or((0, 0));
            // The dead-store signal: mutated via an `OpSet*` (write_targets>0) but never read
            // (reads==0). A var mutated this way was read as the write BASE at parse, so `uses`
            // was >0 there and `test_used` (uses==0) stayed silent — the two lints are disjoint
            // structurally. (`uses` is not maintained in the post-scopes `def.variables`.)
            if !(reads == 0 && write_targets > 0) {
                continue;
            }
            // A dead store requires the var to OWN its store (a copy). A Borrowed/Join var
            // aliases a source it borrows, so the write PROPAGATES — not dead. Exception: the
            // copy idiom builds a vector copy as `OpGetField(__vdb, …)`, so `ownership_of` reports
            // `Borrowed{base=__vdb}` for a genuinely-owned copy — a base that is a SYNTH buffer
            // (or the var itself) means owned-via-buffer, not an alias of a live sibling
            // (`is_synth_buffer`, the P0.2 finding shared with `report_copies`).
            //
            // @PLN107 value-struct completeness: a `value struct` local ALWAYS owns its store —
            // reading a value struct out of a field/element COPIES it (the `value_struct_copy`
            // pass materialises a fresh `OpCopyRecord`; DESIGN @PLN101), so `m = w.field; m.x = 9`
            // (m unread) is a lost write, the same footgun as the vector copy. `ownership_of`
            // conservatively reports `Borrowed{source-view}` there, so without this explicit
            // value-struct test it is a false NEGATIVE. A plain reference `struct` ALIASES (the
            // write propagates) and correctly stays `Borrowed` → silent; `&value struct` is a
            // `RefVar`, already excluded above.
            let owns = is_value_struct_local(func.tp(v), data)
                || match ownership_of(data, d_nr, &Value::Var(v)) {
                    Own::Owned => true,
                    Own::Borrowed { base } | Own::Join { base } => {
                        base == v || (base != u16::MAX && is_synth_buffer(func.name(base)))
                    }
                };
            if !owns {
                continue;
            }
            let (line, col) = func.var_source(v);
            // @PLN102 arc-C steer (alias-where-correct.md step 6): the write-through-intent case.
            // A copy mutated then discarded has no correct copy meaning, so point straight at the
            // explicit fix — `&` for write-through — and offer the read-back alternative if a copy
            // really was intended. (The write-through is ALWAYS the programmer's explicit `&`, never
            // inferred — see alias-where-correct.md: copy is the semantics, links are explicit.)
            let msg = format!(
                "'{name}' is mutated but its value is never read — the write is LOST. A whole-value \
                 bind (`{name} = …`) COPIES the heap value (C86), so the mutation lands in the copy, \
                 not the source. For write-through, bind a live reference with `&` (`{name} = &…`). \
                 If a copy was intended, read `{name}` after the mutation."
            );
            diags.add_at(
                crate::diagnostics::Level::Warning,
                &msg,
                fallback_file,
                line,
                col,
            );
        }
    }
}

/// @PLN102 arc C step 4 — the FOLD lint (C5.2).  For every `#superseded "Y"` symbol X in loft's
/// OWN code — the stdlib (`STD_SOURCE`, so loft's `make ci` enforces its stdlib folds) or the entry
/// project (`MAIN_SOURCE`, a library author's own lib / a user's program); a third-party dependency
/// (source `2..`) is excluded, since a consumer cannot fix its fold: (a) the successor `Y` must
/// RESOLVE — an unresolvable successor is a hard
/// `Level::Error`, so a *dangling* steer never ships; (b) X's body must CALL `Y` — X is a shim over
/// the successor, not independent code — else an advisory `Level::Warning` (promote to a hard
/// `make ci` check once the surface is clean).  Every steer thus ships with its fold, or the lint
/// fires.  INERT until a symbol is actually marked `#superseded`, so the suite is byte-identical.
/// Populates `diags`; the caller renders them with the other diagnostics.
pub fn superseded_fold_diagnostics(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        let succ = def.superseded();
        // Scope to loft's OWN code — the stdlib (`STD_SOURCE`, checked by loft's `make ci`) or the
        // entry project being built (`MAIN_SOURCE`, a library author's own lib / a user's program).
        // A THIRD-PARTY dependency (source `2..`) is excluded: a consumer cannot fix a dependency's
        // fold, so it must not error their compile (the dependency's author catches it in their own
        // build, where their lib is the entry).
        let own = def.source == crate::data::STD_SOURCE || data.source_is_owned(def.source);
        if succ.is_empty() || !own {
            continue;
        }
        let pos = def.position();
        let file = if pos.file.is_empty() {
            fallback_file
        } else {
            pos.file.as_str()
        };
        let shown = def.display_name();
        // (a) the successor must resolve — as a free fn `n_<succ>`, or (if X is a
        // method) the same-receiver method `t_<LEN><Type>_<succ>`.  Each tried in
        // X's own source, then the stdlib (`STD_SOURCE`, visible everywhere).
        let mut candidates = vec![format!("n_{succ}")];
        if let Some(prefix) = def.method_type_prefix() {
            candidates.push(format!("{prefix}{succ}"));
        }
        let y_nr = candidates
            .iter()
            .map(|name| {
                let n = data.source_nr(def.source, name);
                if n == u32::MAX {
                    data.source_nr(crate::data::STD_SOURCE, name)
                } else {
                    n
                }
            })
            .find(|&n| n != u32::MAX)
            .unwrap_or(u32::MAX);
        if y_nr == u32::MAX {
            diags.add_at(
                crate::diagnostics::Level::Error,
                &format!(
                    "`#superseded \"{succ}\"` on `{shown}`: no such successor `{succ}` — a \
                     superseded symbol must name a real replacement, or a dangling steer would ship"
                ),
                file,
                pos.line,
                pos.pos,
            );
            continue;
        }
        // (b) the shim check — X's body must CALL Y (fold the old form onto the new).
        // For a GENERIC successor the body's call targets the monomorphised
        // instantiation (`t_<Type>_sum`), not the `n_sum` template `y_nr` resolved to,
        // so also match by the user-facing name (both render as `sum`). Gate that loose
        // match on the successor ACTUALLY being generic, so an unrelated same-named
        // symbol can't masquerade as the fold — a non-generic successor must be the
        // direct call to `y_nr`.
        let succ_generic = matches!(data.def(y_nr).def_type, crate::data::DefType::Generic);
        let folds = def.code.any_node(&mut |n| {
            matches!(n, Value::Call(d, _)
                if *d == y_nr || (succ_generic && data.def(*d).display_name() == succ))
        });
        if !folds {
            diags.add_at(
                crate::diagnostics::Level::Warning,
                &format!(
                    "`{shown}` is `#superseded` by `{succ}` but its body never calls `{succ}` — \
                     fold it onto the successor (reimplement `{shown}` as a shim over `{succ}`) so \
                     the steer ships with its fold"
                ),
                file,
                pos.line,
                pos.pos,
            );
        }
    }
}

/// @PLN90 W5 — the ENFORCED copy lint (`LOFT_WARN_COPIES`). Routes every **Avoidable** unbound
/// structure copy (a still-live value duplicated where a borrow/move would remove it — the
/// worklist) through the normal `Level::Warning` diagnostics channel, so it surfaces during a
/// normal compile with a source location, the copied type, and the `&`/restructure hint. `Forced`
/// (required as written) and `Implicit`/`Eliminated`/`Internal` stay silent here — only the
/// actionable set warns. Shares the survival-split verdict with `report_copies`; a no-op unless
/// `warn_copies_enabled()`. Populates `diags`; the caller renders them with the other diagnostics.
pub fn warn_copies(data: &Data, diags: &mut crate::diagnostics::Diagnostics, fallback_file: &str) {
    if !crate::keys::warn_copies_enabled() {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        for r in analyze_fn(&def.code, &def.variables, data, env_tier()).0 {
            // Only survival-split (source-duplicating) copies are user-facing, and only the
            // Avoidable class is the actionable worklist — mirror `report_copies`'s filter.
            if !r.survival || !matches!(r.class, CopyClass::Avoidable) {
                continue;
            }
            let ty = if r.source == u16::MAX {
                "a structure".to_string()
            } else {
                data.type_name_str(def.variables.tp(r.source))
            };
            // The copy op carries no span; it borrows the nearest span or (S5.2) the enclosing
            // line marker. A line-only fallback has an empty `file` — substitute the caller's
            // source file. When even the line is unknown, fall back to line 0 + the fn name.
            let (file, line, col) = r.loc.as_ref().map_or((fallback_file, 0, 0), |p| {
                let f = if p.file.is_empty() {
                    fallback_file
                } else {
                    p.file.as_str()
                };
                (f, p.line, p.pos)
            });
            let where_ = if r.loc.is_some() {
                String::new()
            } else {
                format!(" in `{}`", def.name)
            };
            let msg = format!(
                "avoidable copy of {ty}{where_} ({}) — a `&` borrow or a small restructure would remove it",
                r.reason
            );
            diags.add_at(crate::diagnostics::Level::Warning, &msg, file, line, col);
        }
    }
}

/// @PLN90 Step 5 — the USER-FACING copy report (`LOFT_REPORT_COPIES` / `--report-copies`).
///
/// Unlike `dump_all` (the raw developer trace), this surfaces ONLY the *unbound* structure
/// copies the user can act on — `Avoidable` (a borrow/move would remove it — the worklist) and
/// `Forced` (required as written, informational) — each with a source location, the copied
/// type, and a fix hint, followed by a rollup and the ranked Avoidable worklist. `Implicit`
/// (moves / literals) and `Internal` (compiler-generated sources) are excluded — they are not
/// the user's to fix. Diagnostic only; called from `scopes::check`, a no-op unless enabled.
pub fn report_copies(data: &Data) {
    if !crate::keys::report_copies_enabled() {
        return;
    }
    struct Row {
        fname: String,
        loc: String,
        ty: String,
        avoidable: bool,
        reason: &'static str,
    }
    let mut rows: Vec<Row> = Vec::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        for r in analyze_fn(&def.code, &def.variables, data, env_tier()).0 {
            // Only survival-split copies (source duplications) are user-facing; the var-buffer /
            // return-buffer copies are a separate elision class (and where the stdlib's copies
            // land — the survival baseline is 0), kept to the developer dump.
            if !r.survival {
                continue;
            }
            let avoidable = match r.class {
                CopyClass::Avoidable => true,
                CopyClass::Forced => false,
                // Eliminated / Implicit (move, literal) / Internal (compiler-gen) — not the
                // user's to act on.
                _ => continue,
            };
            // S5.2 — a line-only fallback carries an empty `file` (the copy sat under no span);
            // render it as `line N` rather than the raw `:N:0`.
            let loc = r.loc.as_ref().map_or_else(
                || "<location unknown>".to_string(),
                |p| {
                    if p.file.is_empty() {
                        format!("line {}", p.line)
                    } else {
                        p.to_string()
                    }
                },
            );
            let ty = if r.source == u16::MAX {
                "a structure".to_string()
            } else {
                data.type_name_str(def.variables.tp(r.source))
            };
            rows.push(Row {
                fname: def.name.clone(),
                loc,
                ty,
                avoidable,
                reason: r.reason,
            });
        }
    }

    eprintln!(
        "loft copy report — unbound structure copies (a copy the alias-default did not make silently)"
    );
    if rows.is_empty() {
        eprintln!("  none — every structure copy is a move, a literal, or already borrowed.");
        return;
    }
    for row in &rows {
        let tag = if row.avoidable {
            "avoidable"
        } else {
            "forced "
        };
        eprintln!(
            "  {}  fn {}  copies {}  [{tag}]  {}",
            row.loc, row.fname, row.ty, row.reason
        );
    }
    let avoidable = rows.iter().filter(|r| r.avoidable).count();
    let forced = rows.len() - avoidable;
    eprintln!(
        "  ── {} unbound {}: {avoidable} avoidable, {forced} forced (moves & literals are silent)",
        rows.len(),
        if rows.len() == 1 { "copy" } else { "copies" }
    );
    if avoidable > 0 {
        eprintln!("  Avoidable — a `&` borrow or a small restructure would remove these:");
        for row in rows.iter().filter(|r| r.avoidable) {
            eprintln!("    {}  {}", row.loc, row.ty);
        }
    }
}

/// @PLN103 (temporal extension) — static free-before-dependent-read detector.
///
/// The `--show-ownership` overlay classifies WHO owns each store, but the verdict is
/// temporal-agnostic: it renders identically whether a store's `OpFreeRef` lands
/// before or after the last read of a view into it. The captured-group element-access
/// UAF (`plans/captured-group-elem-uaf.md`) was invisible to it for exactly that
/// reason — the correct and the use-after-free lowering share every ownership verdict.
/// This walk adds the missing temporal check.
///
/// Invariant enforced: along each straight-line path of the committed IR, an
/// `OpFreeRef(S)` must not be followed by a genuine READ of any binding `B` whose
/// static deps include `S` (`B` is a live view into the freed store), unless `S` is
/// re-allocated (`OpDatabase(S)`) between the free and the read.
///
/// Returns `(store, via)` pairs — the freed store var and the view read after it —
/// sorted + deduped. Empty = clean. Conservative by construction (branch state is
/// intersected at joins; only an UNCONDITIONAL `OpFreeRef` frees), so it under-reports
/// rather than false-positives.
pub fn free_before_dependent_read(data: &Data, d_nr: u32) -> Vec<(u16, u16)> {
    let def = data.def(d_nr);
    let vars = &def.variables;
    let free_ref = data.def_nr("OpFreeRef");
    let op_db = data.def_nr("OpDatabase");
    // Reverse map: store var `S` -> bindings `B` that VIEW `S`, TRANSITIVELY. A view
    // of a view keeps `S` live through an intermediate local — a nested `match arg {…}`
    // binds `_match_subj_2 = arg`, and `arg` deps `__vdb_1`, so a deref of
    // `_match_subj_2` reads `__vdb_1`. A single-hop map misses it (`arg` is only
    // bare-MOVED into `_match_subj_2`, never deref'd), so walk the dep graph to a
    // fixpoint — the same closure `scopes::transitive_depers` uses for reclaim.
    // Skip marker deps (the u16::MAX one-buffer sentinel, the 0x8000 callee-frame tag)
    // and self-deps (work-refs carry their own var in the dep list).
    let n = vars.count();
    let direct = |b: u16| -> Vec<u16> {
        vars.tp(b)
            .depend()
            .into_iter()
            .filter(|&d| d != u16::MAX && d & 0x8000 == 0 && d != b)
            .collect::<Vec<u16>>()
    };
    let mut reaches: Vec<HashSet<u16>> = (0..n).map(|b| direct(b).into_iter().collect()).collect();
    loop {
        let mut changed = false;
        for b in 0..n as usize {
            for m in reaches[b].iter().copied().collect::<Vec<u16>>() {
                if (m as usize) < n as usize {
                    for a in reaches[m as usize].iter().copied().collect::<Vec<u16>>() {
                        if reaches[b].insert(a) {
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut dependents: HashMap<u16, Vec<u16>> = HashMap::new();
    for b in 0..n {
        for &s in &reaches[b as usize] {
            dependents.entry(s).or_default().push(b);
        }
    }
    if dependents.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(u16, u16)> = Vec::new();
    let mut freed: HashSet<u16> = HashSet::new();
    scan_uaf(
        &def.code,
        &mut freed,
        &dependents,
        free_ref,
        op_db,
        &mut out,
    );
    out.sort_unstable();
    out.dedup();
    out
}

fn scan_uaf(
    node: &Value,
    freed: &mut HashSet<u16>,
    dependents: &HashMap<u16, Vec<u16>>,
    free_ref: u32,
    op_db: u32,
    out: &mut Vec<(u16, u16)>,
) {
    match node.unspan() {
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                scan_uaf(op, freed, dependents, free_ref, op_db, out);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                scan_uaf(op, freed, dependents, free_ref, op_db, out);
            }
        }
        Value::If(c, t, e) => {
            scan_uaf(c, freed, dependents, free_ref, op_db, out);
            let mut ft = freed.clone();
            scan_uaf(t, &mut ft, dependents, free_ref, op_db, out);
            let mut fe = freed.clone();
            scan_uaf(e, &mut fe, dependents, free_ref, op_db, out);
            // Definitely-freed after the join = freed on BOTH branches. A store freed
            // on only one path is live on the other, so it is NOT carried as freed.
            *freed = ft.intersection(&fe).copied().collect();
        }
        Value::Return(inner)
        | Value::Drop(inner)
        | Value::Yield(inner)
        | Value::BreakWith(_, inner) => {
            scan_uaf(inner, freed, dependents, free_ref, op_db, out);
        }
        Value::Call(op, args) if *op == free_ref => {
            if let Some(Value::Var(s)) = args.first().map(Value::unspan) {
                freed.insert(*s);
            }
        }
        Value::Call(op, args) if *op == op_db => {
            if let Some(Value::Var(s)) = args.first().map(Value::unspan) {
                freed.remove(s); // re-allocated — the store is live again
            }
        }
        // Any other op is a read-leaf for this walk: it neither frees/allocs a store
        // nor branches, so check whether it DEREFERENCES a view of a freed store.
        other => check_reads(other, freed, dependents, out),
    }
}

fn check_reads(
    node: &Value,
    freed: &HashSet<u16>,
    dependents: &HashMap<u16, Vec<u16>>,
    out: &mut Vec<(u16, u16)>,
) {
    if freed.is_empty() {
        return;
    }
    for &s in freed {
        if let Some(views) = dependents.get(&s) {
            for &b in views {
                if deref_read(node, b, false) {
                    out.push((s, b));
                }
            }
        }
    }
}

/// True when `node` DEREFERENCES var `target` — reads the CONTENTS of its store,
/// as opposed to merely delivering the reference. The distinction is what separates
/// a real use-after-free from a benign stale dep:
///
///   * A bare `Var(target)` in a DELIVERY position — `return v`, `x = v` (a move),
///     a block tail — hands the *reference* on. The ownership / retbuf machinery
///     moves it safely, so a stale dep on an already-freed store there is NOT a UAF
///     (the return-hoist `__ret_N` false positives, `85-...` / `562-...`).
///   * `target` consumed inside a `Call` (`OpGetVector(v, …)`, `OpGetText(…v…)`, a
///     fn arg) reads the store's contents — a genuine deref that IS a UAF once the
///     store is freed (the captured-group `arg[0]` access).
///
/// So a bare `Var` only counts when it is already inside a call (`in_call`); the
/// inherently-dereferencing slots (`TupleGet` / `CallRef` / `Iter` / `FnRef` /
/// `ParFor`) always count. Write targets (`Set`/`TuplePut` LHS) never do —
/// `for_each_child` restricts those to their RHS.
fn deref_read(node: &Value, target: u16, in_call: bool) -> bool {
    let n = node.unspan();
    let hit = match n {
        Value::Var(x) => in_call && *x == target,
        Value::TupleGet(x, _) | Value::FnRefDnr(x) => *x == target,
        Value::CallRef(x, _) | Value::Iter(x, _, _, _) => *x == target,
        Value::FnRef(_, w, _) => *w == target,
        Value::ParFor(b) => b.x_var == target || b.r_var == target,
        _ => false,
    };
    if hit {
        return true;
    }
    // A Call / CallRef consumes its arguments, so a bare `Var` beneath one is a read.
    let child_in_call = in_call || matches!(n, Value::Call(_, _) | Value::CallRef(_, _));
    let mut found = false;
    n.for_each_child(&mut |c| {
        if !found && deref_read(c, target, child_in_call) {
            found = true;
        }
    });
    found
}

#[cfg(test)]
mod uaf_overlay_tests {
    //! Injected-fault controls for the free-before-dependent-read walk (`scan_uaf`),
    //! parser-free so the positive control does not depend on the compiler still
    //! EMITTING the bug (the materialisation fix removed the only real-code trigger).
    use super::{check_reads, scan_uaf};
    use crate::data::{Type, Value, v_block};
    use std::collections::{HashMap, HashSet};

    // Arbitrary op-numbers for the walk; the walk only compares against these.
    const FREE_REF: u32 = 100;
    const OP_DB: u32 = 101;
    const GETTER: u32 = 200; // stands in for OpGetVector / OpGetField / OpGetText

    fn deps(store: u16, view: u16) -> HashMap<u16, Vec<u16>> {
        let mut d = HashMap::new();
        d.insert(store, vec![view]);
        d
    }
    fn run(code: &Value, dependents: &HashMap<u16, Vec<u16>>) -> Vec<(u16, u16)> {
        let mut freed = HashSet::new();
        let mut out = Vec::new();
        scan_uaf(code, &mut freed, dependents, FREE_REF, OP_DB, &mut out);
        out.sort_unstable();
        out.dedup();
        out
    }
    fn free(s: u16) -> Value {
        Value::Call(FREE_REF, vec![Value::Var(s)])
    }
    fn deref(v: u16) -> Value {
        Value::Call(GETTER, vec![Value::Var(v)])
    }

    /// POSITIVE control — a deref of view `B` AFTER `OpFreeRef(S)` (S ∈ deps(B)) is flagged.
    #[test]
    fn flags_deref_after_free() {
        let code = v_block(vec![free(10), deref(3)], Type::Void, "body");
        assert_eq!(run(&code, &deps(10, 3)), vec![(10, 3)]);
    }

    /// NEGATIVE — a bare `Var` DELIVERY (`return b`) after the free is a safe move, not a deref.
    #[test]
    fn silent_on_bare_delivery_after_free() {
        let code = v_block(
            vec![free(10), Value::Return(Box::new(Value::Var(3)))],
            Type::Void,
            "body",
        );
        assert!(run(&code, &deps(10, 3)).is_empty());
    }

    /// NEGATIVE — a deref BEFORE the free is correctly ordered.
    #[test]
    fn silent_when_deref_precedes_free() {
        let code = v_block(vec![deref(3), free(10)], Type::Void, "body");
        assert!(run(&code, &deps(10, 3)).is_empty());
    }

    /// NEGATIVE — re-allocating the store (`OpDatabase(S)`) between free and deref clears it.
    #[test]
    fn silent_after_realloc() {
        let code = v_block(
            vec![free(10), Value::Call(OP_DB, vec![Value::Var(10)]), deref(3)],
            Type::Void,
            "body",
        );
        assert!(run(&code, &deps(10, 3)).is_empty());
    }

    /// NEGATIVE — a store freed on only ONE branch is live on the other; a deref after the
    /// join is not a definite UAF (branch state is intersected).
    #[test]
    fn silent_when_free_on_one_branch_only() {
        let code = v_block(
            vec![
                Value::If(
                    Box::new(Value::Var(0)),
                    Box::new(v_block(vec![free(10)], Type::Void, "then")),
                    Box::new(v_block(vec![], Type::Void, "else")),
                ),
                deref(3),
            ],
            Type::Void,
            "body",
        );
        assert!(run(&code, &deps(10, 3)).is_empty());
    }

    /// The `check_reads` leaf helper agrees with the walk on a lone deref.
    #[test]
    fn check_reads_matches_walk() {
        let mut freed = HashSet::new();
        freed.insert(10);
        let mut out = Vec::new();
        check_reads(&deref(3), &freed, &deps(10, 3), &mut out);
        assert_eq!(out, vec![(10, 3)]);
    }
}

/// @PLN35 — static return-source-free detector (sibling of `free_before_dependent_read`).
///
/// A record work-ref that the return value ALIASES (is transferred as) must not be freed
/// with a PLAIN `OpFreeRef` before the return — that hands the caller a freed store (35c
/// sub-class A: `[Kw { word }, ..] => LetS{…}` froze the returned `LetS` record). The
/// P4-records safe form is `OpFreeRefIfDistinct(S, ret)`, which is never a plain free and
/// so is not flagged. This class is invisible to `free_before_dependent_read`: the return
/// does not DEREFERENCE the store in-frame, it delivers a reference the caller reads.
///
/// Returns the freed return-source vars, sorted + deduped. Empty = clean.
pub fn return_source_freed(data: &Data, d_nr: u32) -> Vec<u16> {
    let def = data.def(d_nr);
    let ops = FreeOps {
        fr: data.def_nr("OpFreeRef"),
        ft: data.def_nr("OpFreeText"),
        fif: data.def_nr("OpFreeRefIfDistinct"),
        db: data.def_nr("OpDatabase"),
    };
    let mut out: Vec<u16> = Vec::new();
    let mut freed: HashSet<u16> = HashSet::new();
    scan_rsf(&def.code, &def.code, &ops, &mut freed, &mut out);
    out.sort_unstable();
    out.dedup();
    out
}

/// The scope-free / alloc op numbers the return-source walk keys off.
struct FreeOps {
    fr: u32,  // OpFreeRef (plain)
    ft: u32,  // OpFreeText
    fif: u32, // OpFreeRefIfDistinct (the SAFE conditional free)
    db: u32,  // OpDatabase (re-alloc)
}

/// Path-sensitive walk: track the stores plain-`OpFreeRef`-freed on the current path
/// (an `OpFreeRefIfDistinct` or a re-`OpDatabase` clears one), and at each `Return R`
/// flag any record `R` aliases that is currently in the freed set. Path-sensitive so a
/// plain free on a `return null` path (where the freed record is NOT the return) is not
/// a false positive — only a free of the store that IS the returned value is a bug.
fn scan_rsf(
    node: &Value,
    code: &Value,
    ops: &FreeOps,
    freed: &mut HashSet<u16>,
    out: &mut Vec<u16>,
) {
    match node.unspan() {
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                scan_rsf(op, code, ops, freed, out);
            }
        }
        Value::Insert(items) => {
            for op in items {
                scan_rsf(op, code, ops, freed, out);
            }
        }
        Value::Set(_, rhs) => scan_rsf(rhs, code, ops, freed, out),
        Value::If(c, t, e) => {
            scan_rsf(c, code, ops, freed, out);
            let mut ft = freed.clone();
            scan_rsf(t, code, ops, &mut ft, out);
            let mut fe = freed.clone();
            scan_rsf(e, code, ops, &mut fe, out);
            *freed = ft.intersection(&fe).copied().collect();
        }
        Value::Call(d, args) if *d == ops.fr => {
            if let Some(Value::Var(s)) = args.first().map(Value::unspan) {
                freed.insert(*s);
            }
        }
        Value::Call(d, args) if *d == ops.fif || *d == ops.db => {
            // IfDistinct is the SAFE free; OpDatabase re-allocates — both clear the store.
            if let Some(Value::Var(s)) = args.first().map(Value::unspan) {
                freed.remove(s);
            }
        }
        Value::Return(r) | Value::Drop(r) => {
            let mut sources: HashSet<u16> = HashSet::new();
            let mut seen: HashSet<u16> = HashSet::new();
            ret_alias_sources(r, code, ops.fr, ops.ft, ops.fif, &mut seen, &mut sources);
            for &s in &sources {
                if freed.contains(&s) {
                    out.push(s);
                }
            }
        }
        Value::Span(b) => scan_rsf(&b.1, code, ops, freed, out),
        _ => {}
    }
}

fn is_free_op(node: &Value, fr: u32, ft: u32, fif: u32) -> bool {
    matches!(node.unspan(), Value::Call(d, _) if *d == fr || *d == ft || *d == fif)
}

/// The last op of a sequence that carries the block's VALUE — skipping trailing
/// scope-exit frees and `Line` markers (the same rule as `scopes::last_non_free_result`).
fn last_value_op(ops: &[Value], fr: u32, ft: u32, fif: u32) -> Option<&Value> {
    ops.iter()
        .rev()
        .find(|op| !is_free_op(op, fr, ft, fif) && !matches!(op.unspan(), Value::Line(_)))
}

/// Collect the record work-refs the return value aliases: a returned `Var` is traced
/// through its last `Set` to the RHS (the arm Objects), a returned `If` unions both arms,
/// a block yields its last value op. A var with no `Set` assignment is a leaf source
/// (a directly-built work-ref or a param — harmless if not actually freed).
fn ret_alias_sources(
    node: &Value,
    code: &Value,
    fr: u32,
    ft: u32,
    fif: u32,
    seen: &mut HashSet<u16>,
    out: &mut HashSet<u16>,
) {
    match node.unspan() {
        Value::Var(v) => {
            if seen.insert(*v) {
                if let Some(rhs) = last_set_rhs(*v, code) {
                    ret_alias_sources(rhs, code, fr, ft, fif, seen, out);
                } else {
                    out.insert(*v);
                }
            }
        }
        Value::If(_, t, e) => {
            ret_alias_sources(t, code, fr, ft, fif, seen, out);
            ret_alias_sources(e, code, fr, ft, fif, seen, out);
        }
        Value::Block(bl) | Value::Loop(bl) => {
            if let Some(l) = last_value_op(&bl.operators, fr, ft, fif) {
                ret_alias_sources(l, code, fr, ft, fif, seen, out);
            }
        }
        Value::Insert(ops) => {
            if let Some(l) = last_value_op(ops, fr, ft, fif) {
                ret_alias_sources(l, code, fr, ft, fif, seen, out);
            }
        }
        Value::Span(b) => ret_alias_sources(&b.1, code, fr, ft, fif, seen, out),
        _ => {}
    }
}

/// The RHS of the LAST `Set(v, rhs)` for var `v` anywhere in `code` (pre-order; later
/// wins). Explicit recursion rather than `for_each_child` — the callback form cannot
/// escape a `&'a` reference to the RHS.
fn last_set_rhs<'a>(v: u16, code: &'a Value) -> Option<&'a Value> {
    let mut found: Option<&'a Value> = None;
    collect_last_set(v, code, &mut found);
    found
}

fn collect_last_set<'a>(v: u16, node: &'a Value, found: &mut Option<&'a Value>) {
    match node {
        Value::Set(t, rhs) => {
            // Skip the hoisted null-init `Set(v, Null)`: a record work-ref is BUILT by
            // `OpDatabase` (a Call, not a Set), and its only `Set` is the declaration
            // `= null`. Tracing through that loses the build and drops the source (35c).
            // The last NON-null Set is the real value (`__ret_1 = if{…}`).
            if *t == v && !matches!(rhs.unspan(), Value::Null) {
                *found = Some(rhs);
            }
            collect_last_set(v, rhs, found);
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for o in &bl.operators {
                collect_last_set(v, o, found);
            }
        }
        Value::Insert(ops)
        | Value::Call(_, ops)
        | Value::CallRef(_, ops)
        | Value::Tuple(ops)
        | Value::Parallel(ops) => {
            for o in ops {
                collect_last_set(v, o, found);
            }
        }
        Value::If(c, t, e) => {
            collect_last_set(v, c, found);
            collect_last_set(v, t, found);
            collect_last_set(v, e, found);
        }
        Value::Return(inner)
        | Value::Drop(inner)
        | Value::Yield(inner)
        | Value::BreakWith(_, inner)
        | Value::TuplePut(_, _, inner) => collect_last_set(v, inner, found),
        Value::Iter(_, a, b, c) => {
            collect_last_set(v, a, found);
            collect_last_set(v, b, found);
            collect_last_set(v, c, found);
        }
        Value::ParFor(b) => {
            collect_last_set(v, &b.input, found);
            collect_last_set(v, &b.worker, found);
            collect_last_set(v, &b.threads, found);
            collect_last_set(v, &b.body, found);
        }
        Value::Span(b) => collect_last_set(v, &b.1, found),
        _ => {}
    }
}

#[cfg(test)]
mod return_source_tests {
    //! Injected-fault controls for the path-sensitive return-source-free walk (`scan_rsf`),
    //! parser-free — the compiler no longer emits the bug, so the positive control must be
    //! synthetic. Op-numbers are arbitrary; the walk only compares against them.
    use super::{FreeOps, scan_rsf};
    use crate::data::{Type, Value, v_block, v_if, v_set};
    use std::collections::HashSet;

    const FR: u32 = 100;
    const FT: u32 = 101;
    const FIF: u32 = 102;
    const DB: u32 = 103;
    fn ops() -> FreeOps {
        FreeOps {
            fr: FR,
            ft: FT,
            fif: FIF,
            db: DB,
        }
    }
    fn run(code: &Value) -> Vec<u16> {
        let mut out = Vec::new();
        let mut freed = HashSet::new();
        scan_rsf(code, code, &ops(), &mut freed, &mut out);
        out.sort_unstable();
        out.dedup();
        out
    }
    fn free(s: u16) -> Value {
        Value::Call(FR, vec![Value::Var(s)])
    }
    fn free_if(s: u16, r: u16) -> Value {
        Value::Call(FIF, vec![Value::Var(s), Value::Var(r)])
    }
    fn ret(v: u16) -> Value {
        Value::Return(Box::new(Value::Var(v)))
    }
    // A record work-ref built into a block that yields it (the arm `Object`).
    fn obj(v: u16) -> Value {
        v_block(vec![Value::Var(v)], Type::Void, "Object")
    }

    /// POSITIVE — `ret` aliases {1,2}; both plain-freed before `return ret` (the 35c bug).
    #[test]
    fn flags_plain_free_of_return_source() {
        let code = v_block(
            vec![
                v_set(10, v_if(Value::Var(0), obj(1), obj(2))),
                free(1),
                free(2),
                ret(10),
            ],
            Type::Void,
            "body",
        );
        assert_eq!(run(&code), vec![1, 2]);
    }

    /// NEGATIVE — `OpFreeRefIfDistinct` is the safe conditional free (post-fix shape).
    #[test]
    fn silent_on_free_if_distinct() {
        let code = v_block(
            vec![
                v_set(10, v_if(Value::Var(0), obj(1), obj(2))),
                free_if(1, 10),
                free_if(2, 10),
                ret(10),
            ],
            Type::Void,
            "body",
        );
        assert!(run(&code).is_empty());
    }

    /// NEGATIVE — path-sensitive: a plain free on a `return null` path (the freed record is
    /// NOT the return there) is safe — the 497/98 false-positive shape.
    #[test]
    fn silent_when_other_path_returns_it() {
        let code = v_block(
            vec![v_if(
                Value::Var(0),
                v_block(vec![free_if(2, 1), ret(1)], Type::Void, "then"),
                v_block(
                    vec![free(1), free(2), Value::Return(Box::new(Value::Null))],
                    Type::Void,
                    "else",
                ),
            )],
            Type::Void,
            "body",
        );
        assert!(run(&code).is_empty());
    }

    /// NEGATIVE — re-`OpDatabase` between the free and the return clears it.
    #[test]
    fn silent_after_realloc_before_return() {
        let code = v_block(
            vec![free(1), Value::Call(DB, vec![Value::Var(1)]), ret(1)],
            Type::Void,
            "body",
        );
        assert!(run(&code).is_empty());
    }
}
