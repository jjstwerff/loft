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

type BlockId = usize;

/// One basic block: a maximal straight-line run plus its control successors.
#[derive(Default)]
struct Bb {
    label: &'static str,
    /// variables defined (assigned) in this block — the raw material for reaching-defs (1.2).
    defs: Vec<u16>,
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
                    self.op(after, format!("Set v{var}"));
                    Some(after)
                } else {
                    self.def(cur, *var);
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

/// The oracle entry point, reached only under `LOFT_OWN_ORACLE` (SI-1). `cfg` mode builds and
/// dumps each user function's CFG for hand-verification (Phase 1.1). Other modes come with the
/// dataflow fixpoint (Phase 2+).
pub fn oracle(data: &Data) {
    let Ok(mode) = std::env::var("LOFT_OWN_ORACLE") else {
        return;
    };
    if mode != "cfg" {
        return;
    }
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
        dump(data.def(d_nr).name(), &cfg);
    }
}
