//! @PLN94 Phase 1 — a control-flow graph over loft's structured Value-IR, and (Phase 2+)
//! a monotone dataflow fixpoint over it, run BESIDE the shipped analysis as an independent
//! completeness oracle. This module is an OBSERVER: it never rewrites IR and nothing in the
//! normal compile path consumes it — it is reached only via `LOFT_OWN_ORACLE` (SI-1: shipped
//! codegen stays byte-identical). Plan: `doc/claude/plans/94-cfg-ownership-dataflow/`.
//!
//! loft's control flow is STRUCTURED (`If` / `Loop` / `Break(n)` / `Continue(n)` / `Return`,
//! no gotos; `for` lowers to a `Loop` whose exit test is an `If(cond, Break(0), Null)` buried
//! in the range `Set`). So the CFG is built by a syntax-directed walk, not a MIR lowering:
//! statement-level `If` and control-carrying `If` split into then/else/join blocks; a `Loop`
//! gets a header (with the back-edge) and an exit block (reached via `Break`); `Break(n)` /
//! `Continue(n)` / `Return` add the corresponding edge and end the current straight line.

use crate::data::{Data, DefType, Value};
use crate::use_analysis::{Own, classifies_structurally, ownership_of, return_ownership};
use std::collections::{BTreeMap, BTreeSet};

type BlockId = usize;

/// One basic block: a maximal straight-line run plus its control successors.
#[derive(Default)]
struct Bb {
    label: &'static str,
    /// variables defined (assigned) in this block — the raw material for reaching-defs (1.2).
    defs: Vec<u16>,
    /// (var, RHS) per assignment in program order — the ownership pass (Phase 2) classifies each
    /// RHS structurally (`ownership_of`) or, for a `Var` RHS, resolves it flow-sensitively.
    owns: Vec<(u16, Value)>,
    /// short op labels, for the `cfg` dump / hand-verification only.
    ops: Vec<String>,
    succ: Vec<BlockId>,
}

/// A function's control-flow graph. Blocks are index-addressed (no references), so the graph
/// builds without borrow-checker friction.
pub struct Cfg {
    blocks: Vec<Bb>,
    entry: BlockId,
    exit: BlockId,
}

struct LoopCtx {
    header: BlockId,
    exit: BlockId,
}

/// Whether an `If` is being walked as a statement (a direct element of a block's operator
/// sequence — always a branch, since arms may define vars differently) or as a value (only a
/// branch when it carries a control transfer; a pure scalar `if a 0 else 1` stays atomic).
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Stmt,
    Val,
}

struct Builder {
    cfg: Cfg,
    loops: Vec<LoopCtx>,
}

impl Builder {
    fn new_block(&mut self, label: &'static str) -> BlockId {
        let id = self.cfg.blocks.len();
        self.cfg.blocks.push(Bb {
            label,
            ..Default::default()
        });
        id
    }

    fn edge(&mut self, from: BlockId, to: BlockId) {
        if !self.cfg.blocks[from].succ.contains(&to) {
            self.cfg.blocks[from].succ.push(to);
        }
    }

    fn op(&mut self, b: BlockId, label: String) {
        self.cfg.blocks[b].ops.push(label);
    }

    fn def(&mut self, b: BlockId, var: u16) {
        if !self.cfg.blocks[b].defs.contains(&var) {
            self.cfg.blocks[b].defs.push(var);
        }
    }

    /// Record an assignment's (var, RHS) in program order for the ownership pass.
    fn record_own(&mut self, b: BlockId, var: u16, rhs: &Value) {
        self.cfg.blocks[b].owns.push((var, rhs.clone()));
    }

    /// Does `v` contain a control transfer (`Break`/`Continue`/`Return`) targeting THIS or an
    /// enclosing loop/function — i.e. not one consumed by a nested `Loop`? Used to decide
    /// whether a `Set` RHS (or a value-context `If`) must split the CFG. A nested `Loop`'s own
    /// `Break(0)` targets itself, so we do not descend into nested loops (labeled `Break(n>0)`
    /// escaping an inner loop is a known-deferred edge case — see IMPL.md 1.3).
    fn has_transfer(v: &Value) -> bool {
        match v.unspan() {
            Value::Return(_) | Value::Break(_) | Value::BreakWith(..) | Value::Continue(_) => true,
            Value::Loop(_) => false,
            other => {
                let mut found = false;
                other.for_each_child(&mut |c| found |= Self::has_transfer(c));
                found
            }
        }
    }

    /// Record every `Set` target inside `v` as a def of block `b` (for an atomic value-`If`,
    /// whose arms may still assign vars even though it does not split control).
    fn collect_defs(&mut self, v: &Value, b: BlockId) {
        let mut vars: Vec<u16> = Vec::new();
        v.walk(&mut |x| {
            if let Value::Set(n, _) = x {
                vars.push(*n);
            }
        });
        for n in vars {
            self.def(b, n);
        }
    }

    /// Walk a statement sequence starting in `cur`; return the block control falls through to,
    /// or `None` if the sequence diverges (ends in return/break/continue).
    fn walk_seq(&mut self, ops: &[Value], cur: BlockId) -> Option<BlockId> {
        let mut c = Some(cur);
        for op in ops {
            let Some(cc) = c else { break };
            c = self.walk_stmt(op, cc, Ctx::Stmt);
        }
        c
    }

    /// Walk one value as a statement; returns the fall-through block (`None` on divergence).
    fn walk_stmt(&mut self, v: &Value, cur: BlockId, ctx: Ctx) -> Option<BlockId> {
        match v.unspan() {
            Value::Block(b) => self.walk_seq(&b.operators, cur),
            Value::Insert(ops) => self.walk_seq(ops, cur),
            Value::Return(_) => {
                self.op(cur, "Return".into());
                let e = self.cfg.exit;
                self.edge(cur, e);
                None
            }
            Value::Break(n) | Value::BreakWith(n, _) => {
                self.op(cur, "Break".into());
                let t = self.loop_exit(*n);
                self.edge(cur, t);
                None
            }
            Value::Continue(n) => {
                self.op(cur, "Continue".into());
                let t = self.loop_header(*n);
                self.edge(cur, t);
                None
            }
            Value::Loop(body) => {
                let header = self.new_block("loop_hdr");
                self.edge(cur, header);
                let exit = self.new_block("loop_exit");
                self.loops.push(LoopCtx { header, exit });
                let body_end = self.walk_seq(&body.operators, header);
                if let Some(be) = body_end {
                    self.edge(be, header); // back-edge
                }
                self.loops.pop();
                Some(exit)
            }
            Value::If(_, t, e) => {
                let branch = ctx == Ctx::Stmt || Self::has_transfer(t) || Self::has_transfer(e);
                if !branch {
                    // pure scalar expression-`if`: no control split, but its arms may assign.
                    self.op(cur, "if(expr)".into());
                    self.collect_defs(t, cur);
                    self.collect_defs(e, cur);
                    return Some(cur);
                }
                self.op(cur, "if-cond".into());
                let tb = self.new_block("then");
                let eb = self.new_block("else");
                self.edge(cur, tb);
                self.edge(cur, eb);
                let te = self.walk_stmt(t, tb, Ctx::Stmt);
                let ee = self.walk_stmt(e, eb, Ctx::Stmt);
                if te.is_none() && ee.is_none() {
                    None // both arms diverge — nothing falls through
                } else {
                    let join = self.new_block("join");
                    if let Some(x) = te {
                        self.edge(x, join);
                    }
                    if let Some(y) = ee {
                        self.edge(y, join);
                    }
                    Some(join)
                }
            }
            Value::Set(var, val) => {
                if Self::has_transfer(val) {
                    // RHS carries control flow (the `for` range `Set` holds the exit `Break`).
                    let after = self.walk_stmt(val, cur, Ctx::Val)?;
                    self.def(after, *var);
                    self.record_own(after, *var, val);
                    self.op(after, format!("Set v{var}"));
                    Some(after)
                } else {
                    self.def(cur, *var);
                    self.record_own(cur, *var, val);
                    self.op(cur, format!("Set v{var}"));
                    Some(cur)
                }
            }
            other => {
                if Self::has_transfer(other) {
                    // A straight-line op whose child carries a transfer (degenerate in structured
                    // loft IR — breaks/returns live in statement/`If`/`Set`-RHS positions). Walk
                    // the children in order, in place, so no child ref escapes the closure.
                    let mut c = Some(cur);
                    other.for_each_child(&mut |k| {
                        if let Some(cc) = c {
                            c = self.walk_stmt(k, cc, Ctx::Val);
                        }
                    });
                    c
                } else {
                    self.op(cur, Self::op_label(other));
                    Some(cur)
                }
            }
        }
    }

    /// `Break(n)` / `Continue(n)`: `n == 0` is the innermost loop (top of the stack).
    fn loop_exit(&self, n: u16) -> BlockId {
        self.loops[self.loops.len() - 1 - n as usize].exit
    }
    fn loop_header(&self, n: u16) -> BlockId {
        self.loops[self.loops.len() - 1 - n as usize].header
    }

    fn op_label(v: &Value) -> String {
        match v {
            Value::Var(n) => format!("Var v{n}"),
            Value::Call(d, _) => format!("Call#{d}"),
            Value::CallRef(n, _) => format!("CallRef v{n}"),
            Value::Drop(_) => "Drop".into(),
            Value::Line(_) => "Line".into(),
            Value::Null => "Null".into(),
            _ => "op".into(),
        }
    }
}

/// Build the CFG for a function body (an owned clone, so it is not borrowed from `Data`).
fn build(body: &Value) -> Cfg {
    let cfg = Cfg {
        blocks: Vec::new(),
        entry: 0,
        exit: 0,
    };
    let mut b = Builder {
        cfg,
        loops: Vec::new(),
    };
    let entry = b.new_block("entry");
    let exit = b.new_block("exit");
    b.cfg.entry = entry;
    b.cfg.exit = exit;
    // The function body is a `Block` (possibly `Span`-wrapped).
    let end = match body.unspan() {
        Value::Block(bl) => b.walk_seq(&bl.operators, entry),
        other => b.walk_stmt(other, entry, Ctx::Stmt),
    };
    if let Some(e) = end {
        b.edge(e, exit); // implicit fall-through return
    }
    b.cfg
}

fn dump(name: &str, cfg: &Cfg) {
    eprintln!(
        "CFG {name}  blocks={} entry=b{} exit=b{}",
        cfg.blocks.len(),
        cfg.entry,
        cfg.exit
    );
    for (i, bb) in cfg.blocks.iter().enumerate() {
        let succ = if bb.succ.is_empty() {
            "(none)".to_string()
        } else {
            bb.succ
                .iter()
                .map(|s| format!("b{s}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let defs = if bb.defs.is_empty() {
            String::new()
        } else {
            format!(
                "  defs=[{}]",
                bb.defs
                    .iter()
                    .map(|v| format!("v{v}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        eprintln!(
            "  b{i} [{}] -> {succ}{defs}  {}",
            bb.label,
            bb.ops.join(" · ")
        );
    }
}

// ============================================================================
// @PLN94 Phase 1.2 — the worklist dataflow engine, exercised on the classic trivial lattice
// (REACHING DEFINITIONS): a forward may-analysis over sets of definition sites, meet = union.
// `OUT[b] = gen[b] ∪ (IN[b] \ kill[b])`, `IN[b] = ⋃ OUT[preds]`. Monotone over a finite set of
// sites ⇒ the round-robin fixpoint converges (SI-3 asserts a bound so non-termination is loud,
// not a hang). This is the engine the ownership fact (Phase 2) rides on; reaching-defs is here
// only to validate the fixpoint mechanics against hand-computable expectations.
// ============================================================================

/// A definition site: variable `var` assigned in block `block` (block-granular).
struct Site {
    var: u16,
    block: BlockId,
}

struct ReachInfo {
    sites: Vec<Site>,
    inb: Vec<BTreeSet<usize>>,
    outb: Vec<BTreeSet<usize>>,
    passes: usize,
}

fn reaching_defs(cfg: &Cfg) -> ReachInfo {
    let n = cfg.blocks.len();
    // Enumerate def sites (var, block) — one per (var defined, block).
    let mut sites: Vec<Site> = Vec::new();
    for (b, bb) in cfg.blocks.iter().enumerate() {
        for &v in &bb.defs {
            sites.push(Site { var: v, block: b });
        }
    }
    // Predecessors, inverted from successors.
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for (b, bb) in cfg.blocks.iter().enumerate() {
        for &s in &bb.succ {
            preds[s].push(b);
        }
    }
    // gen[b] = sites in b; kill[b] = every OTHER site of a var b (re)defines.
    let mut gens: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut kill: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for (si, s) in sites.iter().enumerate() {
        gens[s.block].insert(si);
    }
    for (b, bb) in cfg.blocks.iter().enumerate() {
        for (si, s) in sites.iter().enumerate() {
            if s.block != b && bb.defs.contains(&s.var) {
                kill[b].insert(si);
            }
        }
    }
    // Round-robin fixpoint in program order (≈ RPO for a mostly-structured CFG → fast).
    let mut inb: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut outb: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut passes = 0usize;
    loop {
        passes += 1;
        // SI-3: a monotone analysis over `sites` converges; the bound turns a bug that would
        // hang into a loud panic instead. `n + 2` is generous (convergence is ~loop-depth+2).
        assert!(
            passes <= n + 2,
            "reaching-defs did not converge in {passes} passes (n={n}) — monotonicity bug"
        );
        let mut changed = false;
        for b in 0..n {
            let mut nin = BTreeSet::new();
            for &p in &preds[b] {
                nin.extend(outb[p].iter().copied());
            }
            inb[b] = nin;
            let mut nout = gens[b].clone();
            for &d in &inb[b] {
                if !kill[b].contains(&d) {
                    nout.insert(d);
                }
            }
            if nout != outb[b] {
                outb[b] = nout;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    ReachInfo {
        sites,
        inb,
        outb,
        passes,
    }
}

fn dump_reach(name: &str, cfg: &Cfg, rd: &ReachInfo) {
    let fmt = |set: &BTreeSet<usize>| {
        if set.is_empty() {
            "{}".to_string()
        } else {
            let items: Vec<String> = set
                .iter()
                .map(|&d| format!("v{}@b{}", rd.sites[d].var, rd.sites[d].block))
                .collect();
            format!("{{{}}}", items.join(","))
        }
    };
    eprintln!(
        "RD {name}  blocks={} sites={} passes={}",
        cfg.blocks.len(),
        rd.sites.len(),
        rd.passes
    );
    for b in 0..cfg.blocks.len() {
        eprintln!(
            "  b{b} [{}]  in={}  out={}",
            cfg.blocks[b].label,
            fmt(&rd.inb[b]),
            fmt(&rd.outb[b])
        );
    }
}

// ============================================================================
// @PLN94 Phase 2 — the forward OWNERSHIP fact, flow-sensitive, on the CFG fixpoint. The lattice
// mirrors the shipped `Own` (Owned | Borrowed(base) | Join(base)) plus a `Bottom` (unreached).
// Where the shipped classifier is flow-INSENSITIVE (a var = the join of ALL its defs), this is
// per-program-point: the meet happens only at the arm-joins that actually merge, so a var whose
// reaching def is single here classifies precisely instead of collapsing to `Join`. The
// per-def transfer REUSES the shipped `ownership_of` for a structural RHS (OpDatabase → Owned,
// projection → Borrowed(base), …); a bare `Var` RHS resolves flow-sensitively to the source's
// current state. Shadow-diffed against `ownership_of` — must AGREE where there is no flow.
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OFact {
    Bottom,
    Owned,
    Borrowed(u16),
    Join(u16),
}

impl OFact {
    fn from_own(o: Own) -> OFact {
        match o {
            Own::Owned => OFact::Owned,
            Own::Borrowed { base } => OFact::Borrowed(base),
            Own::Join { base } => OFact::Join(base),
        }
    }

    /// Lattice meet (⊔) at a control-flow merge: `Bottom` is the identity; `Owned` merged with a
    /// `Borrowed`/different-base becomes runtime-dependent `Join`; `Join` dominates.
    fn meet(a: OFact, b: OFact) -> OFact {
        use OFact::{Borrowed, Bottom, Join, Owned};
        match (a, b) {
            (Bottom, x) | (x, Bottom) => x,
            (Owned, Owned) => Owned,
            (Borrowed(b1), Borrowed(b2)) if b1 == b2 => Borrowed(b1),
            (Join(bb), _) | (_, Join(bb)) => Join(bb),
            // remaining: Owned×Borrowed or Borrowed×Borrowed(differing base) — runtime-dependent.
            (Borrowed(bb), _) | (_, Borrowed(bb)) => Join(bb),
        }
    }

    fn show(self) -> String {
        match self {
            OFact::Bottom => "⊥".into(),
            OFact::Owned => "Owned".into(),
            OFact::Borrowed(b) => format!("Borrowed(v{b})"),
            OFact::Join(b) => format!("Join(v{b})"),
        }
    }

    /// Does `self` REFINE `other` (`self ⊑ other`, i.e. `self` is at least as precise)? A definite
    /// `Owned`/`Borrowed` refines the runtime-dependent `Join`; `Bottom` refines everything. Used
    /// to split a shadow-diff into a PRECISION win (this fact ⊏ the shipped `Join`) vs an unsound
    /// or coarser DISAGREE-ment (which must be zero — the soundness direction, tightened in 3.5).
    fn refines(self, other: OFact) -> bool {
        self == other
            || matches!(
                (self, other),
                (OFact::Bottom, _) | (OFact::Owned | OFact::Borrowed(_), OFact::Join(_))
            )
    }
}

type OState = BTreeMap<u16, OFact>;

/// @PLN94 (3.3) — consume a callee's return-ownership SUMMARY at a call site, INDEPENDENTLY of
/// the shipped classifier (so the oracle can eventually disagree with it on calls): `Owned` →
/// the result is owned; a borrowed/join-of-param return maps back to the caller's argument var.
/// Mirrors `use_analysis::call_ownership` so it agrees where the shipped fact is right.
fn call_own(data: &Data, callee_d: u32, args: &[Value]) -> OFact {
    match return_ownership(data, callee_d) {
        Own::Owned => OFact::Owned,
        Own::Borrowed { base } => OFact::Borrowed(caller_arg_base(data, callee_d, base, args)),
        Own::Join { base } => OFact::Join(caller_arg_base(data, callee_d, base, args)),
    }
}

/// Map the callee's borrowed VISIBLE parameter (`callee_base`, an attribute index) to the
/// caller's argument var at the same visible-parameter position; `u16::MAX` when it is hidden,
/// out of range, or the matching arg is not a var. Mirrors `use_analysis::caller_arg_base`.
fn caller_arg_base(data: &Data, callee_d: u32, callee_base: u16, args: &[Value]) -> u16 {
    let attrs = data.def(callee_d).attributes();
    if callee_base == u16::MAX
        || (callee_base as usize) >= attrs.len()
        || attrs[callee_base as usize].hidden
    {
        return u16::MAX;
    }
    let arg_index = attrs[..callee_base as usize]
        .iter()
        .filter(|a| !a.hidden)
        .count();
    match args.get(arg_index).map(Value::unspan) {
        Some(Value::Var(cv)) => *cv,
        _ => u16::MAX,
    }
}

/// Forward ownership fixpoint: returns each block's OUT-state and the pass count (SI-3).
fn ownership_dataflow(data: &Data, d_nr: u32, cfg: &Cfg) -> (Vec<OState>, usize) {
    let n = cfg.blocks.len();
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for (b, bb) in cfg.blocks.iter().enumerate() {
        for &s in &bb.succ {
            preds[s].push(b);
        }
    }
    let mut outb: Vec<OState> = vec![OState::new(); n];
    let mut passes = 0usize;
    loop {
        passes += 1;
        assert!(
            passes <= n + 2,
            "ownership dataflow did not converge in {passes} passes (n={n}) — monotonicity bug"
        );
        let mut changed = false;
        for b in 0..n {
            // IN[b] = per-var meet of preds' OUT.
            let mut st: OState = OState::new();
            for &p in &preds[b] {
                for (&v, &f) in &outb[p] {
                    let cur = st.get(&v).copied().unwrap_or(OFact::Bottom);
                    st.insert(v, OFact::meet(cur, f));
                }
            }
            // Apply the block's assignments in program order (a later def sees earlier ones).
            for (var, rhs) in &cfg.blocks[b].owns {
                // A `= null` DECLARATION sentinel is not a real def — skip it (B's `collect_defs`
                // does the same). Recording it would default a var whose real def is nested/
                // conditional (a `??` `__ncc_N` temp) to `Owned`, the unsound over-free direction;
                // skipping lets a read fall through to `ownership_of` (B's fact) instead.
                if matches!(rhs.unspan(), Value::Null) {
                    continue;
                }
                let f = match rhs.unspan() {
                    // A bare source var resolves flow-sensitively to its current state.
                    Value::Var(u) => st
                        .get(u)
                        .copied()
                        .unwrap_or_else(|| OFact::from_own(ownership_of(data, d_nr, rhs))),
                    // A NON-NATIVE user-function call consumes the callee summary directly (3.3).
                    // Native-bodied functions and ops fall to the structural classifier: their
                    // return ownership is carried by codegen metadata (`returns_borrowed_view`),
                    // not the loft body `return_ownership` reads — replicating that is 3.4's op-tail.
                    // EXCLUDE the primitive STRUCTURAL ops (store mint / projection): they are
                    // `DefType::Function` with an empty native body too, but `ownership_of`'s
                    // classify handles them exactly (mint → Owned, projection → Borrowed(base));
                    // routing them through `call_own` mis-classes a projection local `Owned` — the
                    // unsound over-free direction (3.4a: `xs = OpGetField(vdb, …)` regression).
                    Value::Call(callee_d, args)
                        if matches!(data.def(*callee_d).def_type, DefType::Function)
                            && data.def(*callee_d).native().is_empty()
                            && !classifies_structurally(data, *callee_d) =>
                    {
                        call_own(data, *callee_d, args)
                    }
                    _ => OFact::from_own(ownership_of(data, d_nr, rhs)),
                };
                // A self-borrow `Borrowed(v)` for var `v` is NOT a real borrow — it is the @P302
                // self-dep `[v]` a keyed-collection local carries so a later `s += …` re-inits in
                // place: an OWNERSHIP marker, freed at scope exit. Normalise to `Owned` (matching
                // the shipped `get_free_vars` @P302 carve-out; you cannot borrow from yourself).
                // (A flow-INSENSITIVE db_var→Owned rule was tried and REVERTED: it forces the A1b
                // return work-ref — `OpDatabase`'d for its Cell then returned as a borrowing view —
                // to `Owned`, matching B's wrong-under-`LOFT_NO_A1B` fact and LOSING the catch.)
                let f = if f == OFact::Borrowed(*var) { OFact::Owned } else { f };
                st.insert(*var, f);
            }
            if st != outb[b] {
                outb[b] = st;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    (outb, passes)
}

/// The def_nrs of the UNCONDITIONAL reference free — the only free-op Check B inspects.
/// Deliberately NOT `OpFreeText` (text ownership is a separate model: a `text` sub is copied via
/// `to_string`, so it is `Owned` at runtime even where the fact reads `Borrowed`, and `OpFreeText`
/// is emitted regardless of deps — the "Text exception" in `scopes::get_free_vars`) and NOT
/// `OpFreeRefIfDistinct` (runtime-guarded: a no-op when the store aliases its witness — the guard
/// IS the correctness mechanism for freeing a maybe-borrowed store, so flagging it cries wolf). An
/// unconditional `OpFreeRef` of a `Borrowed` store, by contrast, is an unguarded over-free.
fn free_op_nrs(data: &Data) -> Vec<u32> {
    let d = data.def_nr("OpFreeRef");
    if d == u32::MAX { Vec::new() } else { vec![d] }
}

/// Collect the vars freed (arg 0 of an `OpFree*` call) anywhere in `body` — Check B's free-site set.
fn collect_free_targets(body: &Value, free_ops: &[u32]) -> Vec<u16> {
    let mut out = Vec::new();
    body.walk(&mut |x| {
        if let Value::Call(d, args) = x
            && free_ops.contains(d)
            && let Some(Value::Var(v)) = args.first().map(Value::unspan)
        {
            out.push(*v);
        }
    });
    out
}

/// The vars a function TRANSFERS OUT — so this frame does not free them, and their absence from the
/// free set is NOT a leak (Check C, dev tier). A var is transferred iff it is returned, or consumed
/// into a container (the element/source arg of `OpFinishRecord`/`OpAppendVector` (arg 1) or
/// `OpCopyRecord` (arg 0)). Deliberately OVER-approximated (bias to EXCLUDE): missing a leak is safe
/// (the runtime leak-check backstops); a missed transfer would cry wolf.
fn transferred_out(body: &Value, data: &Data) -> BTreeSet<u16> {
    let finish = data.def_nr("OpFinishRecord");
    let append = data.def_nr("OpAppendVector");
    let copy = data.def_nr("OpCopyRecord");
    let set_dbref = data.def_nr("OpSetDbRef"); // capture into a record (closure/struct field)
    let mut s = BTreeSet::new();
    body.walk(&mut |x| match x {
        Value::Call(d, args) => {
            // The consumed/captured arg: element/source for a container op, or the captured value
            // (arg 2) of `OpSetDbRef(record, offset, v)` — v is moved into the record, whose cascade
            // frees it (closure capture / field store), so this frame does not.
            let consumed_idx = if *d == finish || *d == append {
                Some(1)
            } else if *d == copy {
                Some(0)
            } else if *d == set_dbref {
                Some(2)
            } else {
                None
            };
            if let Some(i) = consumed_idx
                && let Some(Value::Var(v)) = args.get(i).map(Value::unspan)
            {
                s.insert(*v);
            }
        }
        Value::Return(inner) => {
            let mut rv = Vec::new();
            inner.walk(&mut |y| {
                if let Value::Var(v) = y {
                    rv.push(*v);
                }
            });
            s.extend(rv);
        }
        _ => {}
    });
    s
}

/// Check C (under-free / leak) — an `Owned`, HEAP-typed, LOCAL var (not a parameter) that appears in
/// no free op and is not transferred out leaks its store. Returns the offending vars. Still in the
/// DEV tier: its false-positive rate is bounded by the fact's precision (materialised copies read
/// `Borrowed` today), tracked by the ratchet test, not yet asserted 0.
fn under_free(
    exit_state: &OState,
    freed: &BTreeSet<u16>,
    transferred: &BTreeSet<u16>,
    func: &crate::variables::Function,
) -> Vec<u16> {
    // OWNS its store iff its post-codegen type dep is EMPTY (materialisation is baked in: a copied
    // borrowed-source has an empty dep here — the dep is the ground truth codegen freed against,
    // unlike the usage-blind flow-sensitive fact). Iterating the fixpoint's exit-state keeps it CLEAN
    // (0 FP) but MISSES `OpDatabase` db-var backing stores (only owns-entry is `= null`, absent from
    // the state). Checking ALL vars (`snapshot_names`) catches those — and a genuine dropped free —
    // but over-approximates leaks (backing stores of returns, `par` materialise, retbufs) and needs
    // comprehensive transfer tracking first. That is the promotion blocker (CHECK_C_UNDERFREE_DESIGN
    // § promotion); until then Check C stays exit-state-scoped in the dev tier.
    exit_state
        .iter()
        .filter(|&(&v, &_f)| {
            func.tp(v).heap_dep().is_some() // only a HEAP store can leak (not a scalar)
                && func.tp(v).depend().is_empty() // owns its store (empty dep)
                && !func.is_argument(v)
                && !freed.contains(&v)
                && !transferred.contains(&v)
        })
        .map(|(&v, _)| v)
        .collect()
}

/// Check B — free-legitimacy: a freed var whose fact is `Borrowed(base)` is an over-free of a store
/// owned elsewhere (RED). Returns `(freed_var, base)` per offending site. Pure — the free set and the
/// facts are supplied by the caller — so it is unit-testable without a `Data`.
fn free_of_borrowed(freed: &[u16], facts: &OState) -> Vec<(u16, u16)> {
    freed
        .iter()
        .filter_map(|&v| match facts.get(&v) {
            Some(&OFact::Borrowed(base)) => Some((v, base)),
            _ => None,
        })
        .collect()
}

/// Check A — the shadow-diff: vars whose flow-sensitive fact does NOT refine B's `ownership_of`
/// (neither identical nor a `⊏ Join` precision win). Each is a real defect in one implementation —
/// the coexistence finding. Returns `(var, mine, theirs)`; `precision_out` accumulates the wins.
fn disagreements(
    exit_state: &OState,
    data: &Data,
    d_nr: u32,
    precision_out: &mut Vec<(u16, OFact, OFact)>,
) -> Vec<(u16, OFact, OFact)> {
    let mut disagree = Vec::new();
    for (&v, &mine) in exit_state {
        let theirs = OFact::from_own(ownership_of(data, d_nr, &Value::Var(v)));
        // AGREE at the ownership-DECISION level: the same KIND (both `Owned` / both `Borrowed` /
        // both `Join`) is the same free decision (free / don't-free / conditional), even when the
        // tracked base var differs — the base is informational, and a conditional free
        // (`OpFreeRefIfDistinct`) keys off runtime distinctness, not the static base. Only a KIND
        // mismatch that is not a precision win (`⊏ Join`) is a real ownership disagreement.
        if std::mem::discriminant(&mine) == std::mem::discriminant(&theirs) {
            // agree (same free decision)
        } else if mine.refines(theirs) {
            precision_out.push((v, mine, theirs));
        } else {
            disagree.push((v, mine, theirs));
        }
    }
    disagree
}

/// Run the ownership fixpoint, dump per-block OUT-states, and SHADOW-DIFF each var's fact at the
/// function exit against the shipped flow-insensitive `ownership_of` — the 3.1 gate (agree on
/// straight-line) and the surface where the flow-sensitive `Join` precision shows up later.
fn dump_own(name: &str, cfg: &Cfg, data: &Data, d_nr: u32) {
    let (outb, passes) = ownership_dataflow(data, d_nr, cfg);
    let exit_state = &outb[cfg.exit];
    // Shadow-diff at the exit, split three ways: AGREE (identical), PRECISION (my flow-sensitive
    // fact ⊏ the shipped `Join` — a win), DISAGREE (my fact does NOT refine theirs — coarser or
    // unsound; must be zero). The shipped classifier is flow-insensitive (join of ALL defs), so a
    // var whose reaching def here is single classifies definite where B collapses to `Join`.
    let mut precision_pairs: Vec<(u16, OFact, OFact)> = Vec::new();
    let disagree_pairs = disagreements(exit_state, data, d_nr, &mut precision_pairs);
    let agree = exit_state.len() - precision_pairs.len() - disagree_pairs.len();
    let fmt = |(v, mine, theirs): &(u16, OFact, OFact)| {
        format!("v{v}: mine={} B={}", mine.show(), theirs.show())
    };
    let precision: Vec<String> = precision_pairs.iter().map(fmt).collect();
    let disagree: Vec<String> = disagree_pairs.iter().map(fmt).collect();
    eprintln!(
        "OWN {name}  blocks={} passes={passes}  agree={agree} precision={} disagree={}",
        cfg.blocks.len(),
        precision.len(),
        disagree.len()
    );
    for p in &precision {
        eprintln!("  PRECISION {p}");
    }
    for d in &disagree {
        eprintln!("  DISAGREE {d}");
    }
    for (b, st) in outb.iter().enumerate() {
        if st.is_empty() {
            continue;
        }
        let items: Vec<String> = st
            .iter()
            .map(|(&v, &f)| format!("v{v}={}", f.show()))
            .collect();
        eprintln!("  b{b} [{}]  {}", cfg.blocks[b].label, items.join(", "));
    }
}

/// Phase 4 `check` mode — run the two consistency checks over one function BESIDE the shipped
/// analysis and report each violation as a `RED` line (function, store, site). Returns the finding
/// count so the caller can gate on the total. Pure observer (SI-1 unchanged).
///
/// - **Check A (the A1b catch):** the shadow-diff — any var whose fact does not refine B's is a RED
///   fact-disagreement (the two independent implementations conflict).
/// - **Check B (over-free):** free-legitimacy — an unconditional `OpFreeRef` of a var whose fact is
///   `Borrowed` frees a store owned elsewhere.
fn run_check(name: &str, body: &Value, cfg: &Cfg, data: &Data, d_nr: u32, free_ops: &[u32]) -> usize {
    let (outb, _passes) = ownership_dataflow(data, d_nr, cfg);
    let exit_state = &outb[cfg.exit];
    let nm = |v: u16| data.def(d_nr).variables.name(v).to_string();
    let mut reds = 0;
    let mut precision = Vec::new();
    for (v, mine, theirs) in disagreements(exit_state, data, d_nr, &mut precision) {
        eprintln!(
            "RED {name}: fact-disagree v{v}({}) mine={} B={}",
            nm(v),
            mine.show(),
            theirs.show()
        );
        reds += 1;
    }
    for (v, base) in free_of_borrowed(&collect_free_targets(body, free_ops), exit_state) {
        eprintln!(
            "RED {name}: free-of-borrowed {} (v{v}) is Borrowed(v{base}={})",
            nm(v),
            nm(base)
        );
        reds += 1;
    }
    reds
}

/// The oracle entry point, reached only under `LOFT_OWN_ORACLE` (SI-1). Modes:
/// `cfg` dumps each user function's CFG (Phase 1.1); `rd` runs the reaching-defs fixpoint (1.2);
/// `own` runs the forward ownership fixpoint + shadow-diff vs `ownership_of` (Phase 2); `check`
/// runs the Phase-4 consistency checks BESIDE the shipped analysis and reports `RED` violations.
pub fn oracle(data: &Data) {
    let Ok(mode) = std::env::var("LOFT_OWN_ORACLE") else {
        return;
    };
    if !matches!(mode.as_str(), "cfg" | "rd" | "own" | "check" | "check-dev") {
        return;
    }
    let free_ops = free_op_nrs(data);
    let mut total_reds = 0usize;
    let mut checked = 0usize;
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let body = data.def(d_nr).code.clone();
        // skip empty / native-only bodies
        if matches!(body.unspan(), Value::Null) {
            continue;
        }
        let cfg = build(&body);
        let name = data.def(d_nr).name();
        match mode.as_str() {
            "cfg" => dump(name, &cfg),
            "rd" => {
                let rd = reaching_defs(&cfg);
                dump_reach(name, &cfg, &rd);
            }
            // Both `check` and `check-dev` run the STABLE fact-based Check A + B here (pre-codegen);
            // `check-dev` additionally runs the DEV free-based checks post-codegen (oracle_free_checks).
            "check" | "check-dev" => {
                total_reds += run_check(name, &body, &cfg, data, d_nr, &free_ops);
                checked += 1;
            }
            _ => dump_own(name, &cfg, data, d_nr),
        }
    }
    if matches!(mode.as_str(), "check" | "check-dev") {
        if total_reds == 0 {
            eprintln!("OWN-CHECK: clean — 0 RED over {checked} functions");
        } else {
            eprintln!("OWN-CHECK: {total_reds} RED finding(s) over {checked} functions");
        }
    }
}

/// The DEV-tier POST-codegen free-based checks — run at the END of `scopes::check` (after
/// `get_free_vars` has inserted the frees into `def.code`) ONLY under `LOFT_OWN_ORACLE=check-dev`.
/// The pre-codegen `oracle()` cannot see user-function frees; this pass can. IN DEVELOPMENT: its
/// false-positive rate is bounded by the fact's precision (materialised copies read `Borrowed`), so
/// it is NEVER on the default `check` path — the ratchet test (`tests/ownership_oracle.rs`) tracks
/// the count down to zero, at which point these promote into `check`.
pub fn oracle_free_checks(data: &Data) {
    if std::env::var("LOFT_OWN_ORACLE").as_deref() != Ok("check-dev") {
        return;
    }
    let free_ops = free_op_nrs(data);
    let mut total_reds = 0usize;
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let body = data.def(d_nr).code.clone();
        if matches!(body.unspan(), Value::Null) {
            continue;
        }
        let cfg = build(&body);
        total_reds += run_free_checks(data.def(d_nr).name(), &body, &cfg, data, d_nr, &free_ops);
    }
    eprintln!("OWN-CHECK-DEV-FREE: {total_reds} RED finding(s) (dev tier — ratcheting to 0)");
}

/// DEV tier: the POST-codegen free-based checks — Check B (over-free) on user functions (now that
/// their frees are visible) + Check C (under-free/leak). Both key off the post-codegen TYPE DEP (the
/// materialisation-aware ownership signal), so they need no fixpoint — they are a free-placement
/// CONSISTENCY layer (does the emitted free match the dep), beside Check A's independent cross-check.
fn run_free_checks(name: &str, body: &Value, cfg: &Cfg, data: &Data, d_nr: u32, free_ops: &[u32]) -> usize {
    let (outb, _passes) = ownership_dataflow(data, d_nr, cfg);
    let exit_state = &outb[cfg.exit];
    let func = &data.def(d_nr).variables;
    let nm = |v: u16| func.name(v).to_string();
    let mut reds = 0;
    // Check B (over-free) — a var freed by an unconditional `OpFreeRef` whose post-codegen type dep
    // is NON-empty borrows a store owned elsewhere. The dep is codegen's ground truth (empty = owns,
    // incl. a materialised copy); the flow-sensitive fact is usage-blind here. Exclude a self-dep
    // work-ref `[v]` (@P302 ownership marker, not a borrow).
    let mut seen = BTreeSet::new();
    for v in collect_free_targets(body, free_ops) {
        if !seen.insert(v) {
            continue;
        }
        let dep = func.tp(v).depend();
        let self_dep = dep.len() == 1 && dep[0] == v;
        // A `??` null-coalesce temp (`__ncc_*`) keeps its present-arm/JOIN dep even after the
        // default-arm materialisation makes it an owned copy the shipped code frees unconditionally
        // (@PLN25) — its dep is a stale borrow, not a live one. Exclude it (a documented narrow gap).
        let ncc = func.name(v).starts_with("__ncc_");
        // A freed PARAMETER is the retbuf-displacement reassignment (`get_free_vars` otherwise
        // suppresses freeing params — they are caller-owned), not a user over-free. Its declared dep
        // is a MAY-borrow; the displaced value freed here was owned.
        if func.tp(v).heap_dep().is_some()
            && !dep.is_empty()
            && !self_dep
            && !ncc
            && !func.is_argument(v)
        {
            eprintln!("RED {name}: free-of-borrowed {} (v{v}) dep={dep:?}", nm(v));
            reds += 1;
        }
    }
    // Check C skips CLOSURE bodies (`n___lambda_*`): their frees are inserted at closure-compile
    // time, not into `def.code` here, so their free set reads empty (a documented coverage gap, not
    // unsoundness — the runtime leak-check still covers closures).
    if !name.starts_with("n___lambda_") {
        let all_frees: Vec<u32> = ["OpFreeRef", "OpFreeText", "OpFreeRefIfDistinct"]
            .iter()
            .map(|n| data.def_nr(n))
            .filter(|&d| d != u32::MAX)
            .collect();
        let freed: BTreeSet<u16> = collect_free_targets(body, &all_frees).into_iter().collect();
        let transferred = transferred_out(body, data);
        for v in under_free(exit_state, &freed, &transferred, func) {
            eprintln!("RED {name}: under-free (leak) {} (v{v}) Owned heap, never freed/transferred", nm(v));
            reds += 1;
        }
    }
    reds
}

#[cfg(test)]
mod tests {
    //! @PLN94 Phase 1.3 — the CFG + fixpoint on the control-flow shapes the position-proxy cannot
    //! express, on hand-built IR (parser-free). Each asserts a reaching-defs / edge fact AND the
    //! SI-3 bound. A back-edge is detected as an edge to a lower-numbered block (loop header).
    use super::{BlockId, build, reaching_defs};
    use crate::data::{Type, Value, v_block, v_if, v_set};

    fn loop_block(ops: Vec<Value>) -> Value {
        match v_block(ops, Type::Void, "loop") {
            Value::Block(b) => Value::Loop(b),
            _ => unreachable!(),
        }
    }

    #[test]
    fn branch_join_unions_arms_and_kills_initial() {
        // r(v1) = 0; if c { r = 1 } else { r = 2 }; r
        let body = v_block(
            vec![
                v_set(1, Value::Int(0)),
                v_if(
                    Value::Var(0),
                    v_block(vec![v_set(1, Value::Int(1))], Type::Void, "then"),
                    v_block(vec![v_set(1, Value::Int(2))], Type::Void, "else"),
                ),
                Value::Var(1),
            ],
            Type::Void,
            "body",
        );
        let cfg = build(&body);
        let rd = reaching_defs(&cfg);
        let v1_at_exit: Vec<BlockId> = rd.inb[cfg.exit]
            .iter()
            .map(|&d| &rd.sites[d])
            .filter(|s| s.var == 1)
            .map(|s| s.block)
            .collect();
        assert_eq!(v1_at_exit.len(), 2, "join must union both arm defs of r");
        assert!(
            !v1_at_exit.contains(&cfg.entry),
            "the initial r=0 must not reach past the branch (killed on both arms)"
        );
    }

    #[test]
    fn loop_carried_def_reaches_header_and_converges() {
        // s(v1) = 0; loop { if c break; s = s + 1 }
        let body = v_block(
            vec![
                v_set(1, Value::Int(0)),
                loop_block(vec![
                    v_if(Value::Var(0), Value::Break(0), Value::Null),
                    v_set(1, Value::Int(9)),
                ]),
            ],
            Type::Void,
            "body",
        );
        let cfg = build(&body);
        let rd = reaching_defs(&cfg);
        assert!(
            rd.passes <= cfg.blocks.len() + 2,
            "SI-3: bounded convergence"
        );
        let header = cfg
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(b, bb)| bb.succ.iter().copied().filter(move |&s| s < b))
            .next()
            .expect("a loop must have a back-edge");
        let v1_at_header = rd.inb[header]
            .iter()
            .filter(|&&d| rd.sites[d].var == 1)
            .count();
        assert!(
            v1_at_header >= 2,
            "loop header must see s from BOTH the initial store and the body (loop-carried), got {v1_at_header}"
        );
    }

    #[test]
    fn early_return_edges_to_function_exit() {
        // loop { if c { return } ; s = 1 }
        let body = v_block(
            vec![loop_block(vec![
                v_if(
                    Value::Var(0),
                    v_block(
                        vec![Value::Return(Box::new(Value::Var(0)))],
                        Type::Void,
                        "ret",
                    ),
                    Value::Null,
                ),
                v_set(1, Value::Int(1)),
            ])],
            Type::Void,
            "body",
        );
        let cfg = build(&body);
        let returns_to_exit = cfg
            .blocks
            .iter()
            .any(|bb| bb.ops.iter().any(|o| o == "Return") && bb.succ.contains(&cfg.exit));
        assert!(
            returns_to_exit,
            "an early return must edge to the FUNCTION exit, not the loop exit"
        );
    }

    #[test]
    fn nested_loops_converge_with_two_headers() {
        // loop { loop { s = 1; if c break } if c break } — each break(0) targets its own loop.
        let body = v_block(
            vec![loop_block(vec![
                loop_block(vec![
                    v_set(1, Value::Int(1)),
                    v_if(Value::Var(0), Value::Break(0), Value::Null),
                ]),
                v_if(Value::Var(0), Value::Break(0), Value::Null),
            ])],
            Type::Void,
            "body",
        );
        let cfg = build(&body);
        let rd = reaching_defs(&cfg);
        assert!(
            rd.passes <= cfg.blocks.len() + 2,
            "SI-3: bounded convergence on nested loops"
        );
        let back_targets: std::collections::BTreeSet<BlockId> = cfg
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(b, bb)| bb.succ.iter().copied().filter(move |&s| s < b))
            .collect();
        assert!(
            back_targets.len() >= 2,
            "nested loops → at least two distinct loop headers, got {}",
            back_targets.len()
        );
    }

    #[test]
    fn ofact_meet_is_a_join_semilattice() {
        use super::OFact;
        use super::OFact::{Borrowed, Bottom, Join, Owned};
        // Bottom (unreached) is the identity.
        assert_eq!(OFact::meet(Bottom, Owned), Owned);
        assert_eq!(OFact::meet(Borrowed(3), Bottom), Borrowed(3));
        // Equal facts are idempotent.
        assert_eq!(OFact::meet(Owned, Owned), Owned);
        assert_eq!(OFact::meet(Borrowed(2), Borrowed(2)), Borrowed(2));
        // Owned ⊔ Borrowed → runtime-dependent Join (the `v[i] ?? d` / reassign-per-arm shape).
        assert_eq!(OFact::meet(Owned, Borrowed(5)), Join(5));
        assert_eq!(OFact::meet(Borrowed(5), Owned), Join(5)); // commutative
        // Borrowing from different bases is also runtime-dependent.
        assert_eq!(OFact::meet(Borrowed(1), Borrowed(2)), Join(1));
        // Join dominates.
        assert_eq!(OFact::meet(Join(7), Owned), Join(7));
        assert_eq!(OFact::meet(Owned, Join(7)), Join(7));
    }

    #[test]
    fn ofact_refines_marks_precision_and_flags_the_unsound_direction() {
        use super::OFact::{Borrowed, Bottom, Join, Owned};
        // A definite fact refines the shipped runtime-dependent `Join` — a PRECISION win (sound:
        // narrowing "maybe owned/borrowed" to a definite borrow/own never frees more than B would).
        assert!(Owned.refines(Join(3)));
        assert!(Borrowed(1).refines(Join(2)));
        assert!(Bottom.refines(Owned));
        assert!(Owned.refines(Owned)); // identity
        // The DANGEROUS direction must NOT refine (→ flagged DISAGREE): claiming `Owned` where B
        // says `Borrowed` would free a borrowed store (UAF/double-free). This is the soundness gate.
        assert!(!Owned.refines(Borrowed(1)));
        assert!(!Borrowed(1).refines(Owned));
        // Coarser-than-B (my `Join` where B is definite) is also not a refinement.
        assert!(!Join(1).refines(Owned));
    }

    // ---- Phase 4.1: the consistency-check primitives (Check B core), on hand-built inputs ----

    #[test]
    fn free_of_borrowed_flags_only_the_borrowed_frees() {
        use super::OFact::{Borrowed, Join, Owned};
        use super::{OState, free_of_borrowed};
        let mut facts = OState::new();
        facts.insert(1, Owned); //  freeing an owned store is legitimate
        facts.insert(2, Borrowed(5)); //  freeing a borrowed alias is the over-free — RED
        facts.insert(3, Join(6)); //  a runtime-dependent store: the shipped code frees it
        //  conditionally (OpFreeRefIfDistinct) — not a definite over-free, so NOT flagged in 4.1
        let freed = [1u16, 2, 3];
        let reds = free_of_borrowed(&freed, &facts);
        // Only the definite Borrowed(5) free is a violation; it names its base.
        assert_eq!(reds, vec![(2, 5)]);
    }

    #[test]
    fn collect_free_targets_finds_only_free_op_arg0() {
        use super::collect_free_targets;
        // Body: OpFreeRef(v3) ; OpSetInt(v4, v5) — only the free-op's arg0 (v3) is a free target.
        let free_ref = 99u32;
        let other = 88u32;
        let body = v_block(
            vec![
                Value::Call(free_ref, vec![Value::Var(3)]),
                Value::Call(other, vec![Value::Var(4), Value::Var(5)]),
            ],
            Type::Void,
            "body",
        );
        assert_eq!(collect_free_targets(&body, &[free_ref]), vec![3]);
    }
}
