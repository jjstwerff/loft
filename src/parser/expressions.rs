// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{Level, Parser, Parts, Type, Value, diagnostic_format, v_block, v_if, v_loop, v_set};
use crate::data::Deps;

/// @PLN86 step 0.1 — maximum expression-nesting depth allowed inside a sandboxed
/// def's body.  Hostile deep nesting (`((((…))))`) drives the recursive-descent
/// parser into a native stack overflow (rc=139); past this bound the parser
/// rejects with a clean diagnostic at LOAD time instead.
///
/// Each nesting level costs roughly 10 KB of native stack (the
/// expression→operators→part→single chain), so the bound must be REACHABLE
/// without overflowing the smallest stack the parser runs on: 128 levels ≈ 1.3 MB,
/// safe on a standard ≥2 MB thread with margin, and still far deeper than any
/// hand-written script nests.  (Host-configurable later, per the plan.)
pub(crate) const SANDBOX_MAX_PARSE_DEPTH: u32 = 128;

/// P194 helper — extract the host reference and base position from
/// the leftmost `OpGet*` leaf of a tuple-typed field read.  Returns
/// `(host_ref, first_element_position)` where `first_element_position`
/// equals `host_field_pos + element_offset[0]`.  For nested-tuple
/// reads, recurses into the inner `Value::Tuple`.
fn leaf_tuple_lhs(v: &Value) -> Option<(Value, i32)> {
    // Plan-07 phase 1: unspan() so wraps on `.` (step 1.12) don't
    // hide the field-read shape of the tuple-LHS.
    match v.unspan() {
        Value::Call(_, args) if args.len() >= 2 => {
            if let Value::Int(p) = &args[1] {
                Some((args[0].clone(), *p))
            } else {
                None
            }
        }
        Value::Tuple(inner) if !inner.is_empty() => leaf_tuple_lhs(&inner[0]),
        _ => None,
    }
}

/// Returns true if `val` contains a `Set(r, _)` node at any depth.
/// Used to find which block statement first assigns an inline-ref temporary.
/// Descent comes from `Value::for_each_child`, so a new compound variant
/// cannot be silently missed (A15).  The hand-rolled predecessor treated
/// `BreakWith` as a leaf and missed a `Set` inside its value — the unified
/// walker descends it (pass-2 wave 2 widening; the wider answer is the
/// correct null-init insertion point).
fn inline_ref_set_in(val: &Value, r: u16) -> bool {
    val.any_node(&mut |n| matches!(n, Value::Set(v, _) if *v == r))
}

/// P248 — extracted nested-tuple LHS shape for assignment.
///
/// `t.0.1.2 = …` parses to either a single `Value::TupleGet` (depth 1)
/// or a chain of `Block[Set(w_k, …), TupleGet(w_k, idx)]` nodes (depth
/// ≥ 2 — operators.rs case 3 materialises the intermediate reads
/// through work vars).  This struct flattens both shapes so the
/// assignment dispatcher can rewrite them uniformly.
///
/// For `t.0.1 = 99`:
///   - root = t_var_nr
///   - chain = [(w0, 0)]   // w0 = t.0
///   - leaf_idx = 1        // …w0.1 = rhs
///
/// For `t.0 = 99`:
///   - root = t_var_nr
///   - chain = []
///   - leaf_idx = 0
struct NestedTupleLhs {
    root: u16,
    /// Pairs of `(work_var, index_into_parent)`, ordered root → leaf.
    chain: Vec<(u16, u16)>,
    leaf_idx: u16,
}

/// Walk a Value that might be a chained tuple read (single `TupleGet`
/// or nested `Block[Set(w, source), TupleGet(w, idx)]`) and return a
/// flattened `NestedTupleLhs`.  Returns `None` for any other shape.
fn extract_nested_tuple_lhs(code: &Value) -> Option<NestedTupleLhs> {
    match code.unspan() {
        Value::TupleGet(var_nr, idx) => Some(NestedTupleLhs {
            root: *var_nr,
            chain: vec![],
            leaf_idx: *idx,
        }),
        Value::Block(b) if b.operators.len() == 2 => {
            let (set_op, tail) = (&b.operators[0], &b.operators[1]);
            if let (Value::Set(w_set, source), Value::TupleGet(w_get, leaf_idx)) = (set_op, tail)
                && w_set == w_get
            {
                let inner = extract_nested_tuple_lhs(source)?;
                let mut chain = inner.chain;
                chain.push((*w_set, inner.leaf_idx));
                Some(NestedTupleLhs {
                    root: inner.root,
                    chain,
                    leaf_idx: *leaf_idx,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Build the assignment IR for `<lhs> = rhs`.  For depth-1 it's a
/// plain `TuplePut(root, leaf_idx, rhs)`.  For depth ≥ 2 we keep the
/// existing Block's `Set` ops (they read intermediates into the same
/// work vars), strip the trailing `TupleGet`, write the leaf via
/// `TuplePut(deepest_w, leaf_idx, rhs)`, then write each intermediate
/// back to its parent in reverse so the modification propagates up
/// to `root`.
fn build_nested_tuple_assign(orig_code: &Value, lhs: &NestedTupleLhs, rhs: Value) -> Value {
    if lhs.chain.is_empty() {
        return Value::TuplePut(lhs.root, lhs.leaf_idx, Box::new(rhs));
    }
    // Reuse the Set ops the existing Block already emitted (read
    // chain into the same work-var slots).  Strip the trailing
    // TupleGet — we're replacing it with writes.
    let mut ops: Vec<Value> = if let Value::Block(b) = orig_code.unspan() {
        let mut clone = b.operators.clone();
        if matches!(clone.last(), Some(Value::TupleGet(_, _))) {
            clone.pop();
        }
        clone
    } else {
        // Defensive — shouldn't happen since chain.len() >= 1 implies
        // we matched the Block arm above.  Reconstruct the reads.
        let mut prev = lhs.root;
        let mut acc = Vec::with_capacity(lhs.chain.len());
        for (w, idx) in &lhs.chain {
            acc.push(crate::data::v_set(*w, Value::TupleGet(prev, *idx)));
            prev = *w;
        }
        acc
    };
    let last_w = lhs.chain.last().expect("chain non-empty").0;
    // Leaf: write rhs into the deepest work var at leaf_idx.
    ops.push(Value::TuplePut(last_w, lhs.leaf_idx, Box::new(rhs)));
    // Writebacks in reverse — propagate the modified intermediate
    // back up the chain so the change reaches `root`.
    for (k, (w, idx)) in lhs.chain.iter().enumerate().rev() {
        let parent = if k == 0 { lhs.root } else { lhs.chain[k - 1].0 };
        ops.push(Value::TuplePut(parent, *idx, Box::new(Value::Var(*w))));
    }
    crate::data::v_block(ops, Type::Void, "nested_tuple_assign")
}

/// Recursively replace every occurrence of `from` in `into` with a clone
/// of `to`.  Used by the `field += elem` keyed-collection branch to
/// retarget a struct-literal's field-init steps from the LHS field
/// expression onto a freshly-allocated element variable.
/// @P390 — does `val` reference `Value::Var(target)` anywhere (a READ of the
/// variable)?  Mirrors `replace_var_in_ir`'s traversal so it covers every `Value`
/// variant soundly.  Used to detect a self-aliasing slice-assign (`v = v[a..b]`),
/// where the materialise-into-iterator would read the same variable it is about
/// to `OpClearVector`.  `Set`/`TuplePut` slots are NOT counted (they are writes,
/// matching the walker) — only `Var(target)` reads.
fn ir_mentions_var(val: &Value, target: u16) -> bool {
    match val {
        Value::Var(v) => *v == target,
        Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Text(_)
        | Value::Enum(_, _)
        | Value::Line(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _)
        | Value::FnRef(_, _, _)
        | Value::FnRefDnr(_)
        | Value::RawExpr(_)
        | Value::Null => false,
        Value::Call(_, args)
        | Value::CallRef(_, args)
        | Value::Insert(args)
        | Value::Tuple(args)
        | Value::Parallel(args) => args.iter().any(|a| ir_mentions_var(a, target)),
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.iter().any(|op| ir_mentions_var(op, target))
        }
        Value::Set(_, body)
        | Value::Return(body)
        | Value::BreakWith(_, body)
        | Value::Drop(body)
        | Value::TuplePut(_, _, body)
        | Value::Yield(body) => ir_mentions_var(body, target),
        Value::If(cond, t, f) => {
            ir_mentions_var(cond, target)
                || ir_mentions_var(t, target)
                || ir_mentions_var(f, target)
        }
        Value::Iter(_, a, b, c) => {
            ir_mentions_var(a, target) || ir_mentions_var(b, target) || ir_mentions_var(c, target)
        }
        Value::Span(b) => ir_mentions_var(&b.1, target),
        Value::ParFor(b) => {
            ir_mentions_var(&b.input, target)
                || ir_mentions_var(&b.worker, target)
                || ir_mentions_var(&b.threads, target)
                || ir_mentions_var(&b.body, target)
        }
    }
}

pub(crate) fn substitute_value(into: &mut Value, from: &Value, to: &Value) {
    if into == from {
        *into = to.clone();
        return;
    }
    match into {
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                substitute_value(a, from, to);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for s in &mut bl.operators {
                substitute_value(s, from, to);
            }
        }
        Value::Insert(steps) => {
            for s in steps {
                substitute_value(s, from, to);
            }
        }
        Value::If(c, t, e) => {
            substitute_value(c, from, to);
            substitute_value(t, from, to);
            substitute_value(e, from, to);
        }
        Value::Set(_, v)
        | Value::Return(v)
        | Value::Drop(v)
        | Value::Yield(v)
        | Value::BreakWith(_, v)
        | Value::TuplePut(_, _, v) => substitute_value(v, from, to),
        Value::Iter(_, a, b, c) => {
            substitute_value(a, from, to);
            substitute_value(b, from, to);
            substitute_value(c, from, to);
        }
        Value::Tuple(vs) | Value::Parallel(vs) => {
            for v in vs {
                substitute_value(v, from, to);
            }
        }
        // Plan-07 phase 1 — Span is transparent; recurse into the
        // wrapped node.
        Value::Span(b) => substitute_value(&mut b.1, from, to),
        // Plan-06 spine step 3 — recurse into all child Values.
        Value::ParFor(b) => {
            substitute_value(&mut b.input, from, to);
            substitute_value(&mut b.worker, from, to);
            substitute_value(&mut b.threads, from, to);
            substitute_value(&mut b.body, from, to);
        }
        // Leaf variants.
        Value::Null
        | Value::Int(_)
        | Value::Enum(_, _)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Var(_)
        | Value::Line(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _)
        | Value::FnRef(_, _, _)
        | Value::FnRefDnr(_) => {}
        // Phase 09 phase 00 step 0.7 — codegen-internal.
        Value::RawExpr(_) => {}
    }
}

impl Parser {
    /// @PLN10 — wrap every text-dest native called in *value position* in a
    /// scope-bound work-text temp, so its result lives in a freed local instead
    /// of the never-cleared `stores.scratch` buffer.  Replaces
    /// `Call(native, args)` with `Block([Set(w, native()), Var(w)])` where `w`
    /// is a fresh `work_text` — reusing the proven `set_var` dest-pass for the
    /// inner `Set`.  The two codegen fast paths already dest-pass their shapes
    /// directly, so we skip wrapping a *bare* native that is the value of a
    /// `Set` (set_var) or the appended operand of `OpAppendText`
    /// (try_text_dest_pass) — but still recurse into their sub-arguments.
    fn wrap_value_text_dest(&mut self, v: &mut Value) {
        // A text-dest native reached here (not via a fast-path skip) is in
        // value position — recurse into its args (nested natives), then wrap.
        let wrap_here = matches!(
            &*v,
            Value::Call(op, _) if crate::state::codegen::is_text_dest_native(self.data.def(*op).name())
                || crate::state::codegen::is_cdylib_text_call(self.data.def(*op))
        );
        if wrap_here {
            if let Value::Call(_, args) = v {
                for a in args.iter_mut() {
                    self.wrap_value_text_dest(a);
                }
            }
            let w = self.vars.work_text(&mut self.lexer);
            let call = std::mem::replace(v, Value::Null);
            *v = v_block(
                vec![v_set(w, call), Value::Var(w)],
                Type::Text(Deps::frame1(w)),
                "synth text dest",
            );
            return;
        }
        match v {
            Value::Call(op, args)
                if self.data.def(*op).name() == "OpAppendText" && args.len() == 2 =>
            {
                self.wrap_value_text_dest(&mut args[0]);
                self.descend_skip_direct(&mut args[1]);
            }
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args.iter_mut() {
                    self.wrap_value_text_dest(a);
                }
            }
            Value::Set(_, rhs) => self.descend_skip_direct(rhs),
            Value::Block(bl) | Value::Loop(bl) => {
                for s in &mut bl.operators {
                    self.wrap_value_text_dest(s);
                }
            }
            Value::Insert(steps) => {
                for s in steps {
                    self.wrap_value_text_dest(s);
                }
            }
            Value::If(c, t, e) => {
                self.wrap_value_text_dest(c);
                self.wrap_value_text_dest(t);
                self.wrap_value_text_dest(e);
            }
            Value::Return(x)
            | Value::Drop(x)
            | Value::Yield(x)
            | Value::BreakWith(_, x)
            | Value::TuplePut(_, _, x) => self.wrap_value_text_dest(x),
            Value::Iter(_, a, b, c) => {
                self.wrap_value_text_dest(a);
                self.wrap_value_text_dest(b);
                self.wrap_value_text_dest(c);
            }
            Value::Tuple(vs) | Value::Parallel(vs) => {
                for x in vs {
                    self.wrap_value_text_dest(x);
                }
            }
            Value::Span(b) => self.wrap_value_text_dest(&mut b.1),
            Value::ParFor(b) => {
                self.wrap_value_text_dest(&mut b.input);
                self.wrap_value_text_dest(&mut b.worker);
                self.wrap_value_text_dest(&mut b.threads);
                self.wrap_value_text_dest(&mut b.body);
            }
            _ => {}
        }
    }

    /// Descend into a fast-path position (a `Set` value or `OpAppendText`'s
    /// appended operand): a *bare* text-dest native here is dest-passed
    /// directly by codegen, so don't wrap it — only recurse into its args
    /// (nested natives still need a temp).  Any other shape walks normally.
    fn descend_skip_direct(&mut self, v: &mut Value) {
        if let Value::Call(op, args) = v.unspan_mut() {
            let is_dest = crate::state::codegen::is_text_dest_native(self.data.def(*op).name())
                || crate::state::codegen::is_cdylib_text_call(self.data.def(*op));
            if is_dest {
                for a in args.iter_mut() {
                    self.wrap_value_text_dest(a);
                }
                return;
            }
        }
        self.wrap_value_text_dest(v);
    }

    // <code> = '{' <block> '}'
    /// Parse the code on the last inserted definition.
    /// This way we can use recursion with the definition itself.
    pub(crate) fn parse_code(&mut self) -> Type {
        let mut v = Value::Null;
        let result = if self.context == u32::MAX {
            Type::Void
        } else {
            self.data.def(self.context).returned().clone()
        };
        self.parse_block("return from block", &mut v, &result);
        // @PLN10 — synth a scope-bound work-text destination for every text-dest
        // native called in *value position*, so its result lives in a freed temp
        // instead of the never-cleared `stores.scratch` buffer.  Runs on the final
        // pass before the work_texts null-init loop below, so the synthesized
        // temps get null-inited + slot-allocated + freed like any work-text.
        if !self.first_pass {
            self.wrap_value_text_dest(&mut v);
        }
        if let Value::Block(bl) = &mut v {
            let ls = &mut bl.operators;
            for wt in self.vars.work_texts() {
                ls.insert(0, v_set(wt, Value::Text(String::new())));
            }
            // copy text arguments into promoted shadow locals at function entry.
            for (shadow, original) in self.vars.promoted_text_args() {
                ls.insert(0, v_set(shadow, Value::Var(original)));
            }
            for r in self.vars.work_references() {
                if std::env::var("LOFT_TRACE_PREAMBLE").is_ok() {
                    eprintln!(
                        "[preamble] pass1={} r={r} name={} arg={} inline={} deps={:?} chb={}",
                        self.first_pass,
                        self.vars.name(r),
                        self.vars.is_argument(r),
                        self.vars.is_inline_ref(r),
                        self.vars.tp(r).depend(),
                        self.vars.is_caller_hidden_buf(r),
                    );
                }
                if !self.vars.is_argument(r)
                    && !self.vars.is_inline_ref(r)
                    // @PLAN51 Cluster IV: also null-init caller-side hidden-
                    // buffer work-refs even when their typedef carries a
                    // non-empty dep list (e.g. Reference(td, [arg_idx]) for
                    // if-tail / recursion / explicit-return-in-if shapes).
                    // Without it, the slot allocator skips them ("no
                    // first_def") and codegen panics at codegen.rs:2529.
                    // Empty-dep refs still take this path (the original
                    // arm); caller_hidden_buf is the additional gate.
                    // #319: `__ncc_N` heap-DbRef temps likewise — their only
                    // Set is inside the ncc block, so they need the preamble
                    // init regardless of their dep list.
                    && (self.vars.tp(r).depend().is_empty()
                        || self.vars.is_caller_hidden_buf(r)
                        || self.vars.name(r).starts_with("__ncc_"))
                {
                    ls.insert(0, v_set(r, Value::Null));
                }
            }
            // Inline-ref temporaries (parse_part work-refs for chained ref calls):
            // Insert null-init for each temp immediately BEFORE the statement that
            // first assigns it (the statement containing {Set(r, call_result)}).
            // This ensures scan_set encounters them AFTER the body variables whose
            // stores precede theirs (e.g. `p`), so reversed var_order frees the
            // inline-ref temps BEFORE those body variables — satisfying LIFO.
            //
            // For temps used in the same statement we insert in descending var_nr order
            // so that lower var_nrs end up first in ls (allocated first = freed last).
            {
                let inline_refs = self.vars.inline_ref_references();
                // Build (first_use_position, var_nr) pairs.
                let mut insertions: Vec<(usize, u16)> = Vec::new();
                // Fallback position: after the first non-Line-marker stmt in ls.
                let mut fallback = 0usize;
                while fallback < ls.len() && matches!(ls[fallback], Value::Line(_)) {
                    fallback += 1;
                }
                if fallback < ls.len() {
                    fallback += 1;
                }
                for r in &inline_refs {
                    if !self.vars.is_argument(*r) && self.vars.tp(*r).depend().is_empty() {
                        let pos = ls
                            .iter()
                            .position(|stmt| inline_ref_set_in(stmt, *r))
                            .unwrap_or(fallback);
                        insertions.push((pos, *r));
                    }
                }
                // Insert from end to start to avoid index invalidation; within the
                // same position insert higher var_nr first so lower var_nr lands first.
                insertions.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
                for (pos, r) in insertions {
                    ls.insert(pos, v_set(r, Value::Null));
                }
            }
            // Auto-lock the stores for every const Reference/Vector argument at the very
            // start of the function body (after work-variable initialisations).
            // Applies in all build profiles so that writes to const parameters panic in
            // release builds too (S22 — previously guarded by #[cfg(debug_assertions)]).
            // detect variables that remain Unknown(0) after the second pass.
            // These are names from the first pass that were never resolved — likely typos.
            // Note: `known_var_or_type()` in objects.rs already emits "Unknown variable"
            // during expression parsing for variables with Unknown type or undefined status.
            // This post-parse check catches the complementary case: variables that were
            // assigned (is_defined) but whose type remained Unknown after both passes.
            if !self.first_pass {
                let n_vars = self.vars.next_var();
                for v_nr in 0..n_vars {
                    if self.vars.tp(v_nr).is_unknown()
                        && !self.vars.is_argument(v_nr)
                        && self.vars.is_defined(v_nr)
                        && !self.vars.name(v_nr).starts_with('_')
                    {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Variable '{}' has unknown type — possible typo or missing definition",
                            self.vars.name(v_nr)
                        );
                    }
                }
            }
            // @P376 follow-up — formerly emitted `n_set_store_lock(p, true)`
            // at function entry for every `const` Reference/Vector parameter.
            // The intent was a defense-in-depth tripwire for compile-time
            // const-check bugs.  In practice, the compile-time check catches
            // every mutation path (`p = X`, `p.f = X`, `p.h[k] = v`,
            // `p.h += ...`, non-`const`-method calls on `p`), and the
            // function-entry lock fires SPURIOUSLY on legitimate iteration
            // over a hash field of `p`: `for x in p.h` calls
            // `build_hash_sorted_vec` which (by C60 piece-3 edit-A design)
            // allocates sort scratch IN THE HASH'S STORE — i.e. `p`'s store —
            // and panics on the locked claim.  Par-worker safety is
            // INDEPENDENT and untouched: `clone_locked` /
            // `clone_locked_for_worker` / `borrow_locked_for_light_worker`
            // set `read_only = true` on the worker's cloned store, so a
            // worker that writes through a `const` arg still panics on
            // `addr_mut`.  See PROBLEMS.md @P376 follow-up + PLANNING.md S22
            // (the S22 motivation — par-worker silent-mutation in release —
            // remains addressed by the clone-side lock).
        }
        // Plan-22 phase 02a (2026-05-12): also save body in pass 1
        // so the closure mutation walker can run in pass 1 BEFORE
        // synthesize_closure_record sets attribute types.  The body
        // gets overwritten in pass 2 with the properly-typed
        // version; pass 1's body is only consulted by phase 01's
        // walker (`collect_mutated_captures`) and never by codegen.
        //
        // Risk: any other code path that assumes
        // `data.def(d_nr).code` is Null in pass 1 would break.
        // Verified clean against the regression net (633 issues +
        // 47 wrap + 22 closure_matrix + 6 mut_closure_matrix); if
        // a future regression surfaces, narrow this to only-save
        // when `self.in_lambda` is true.
        if self.context != u32::MAX {
            self.data.definitions[self.context as usize].code = v;
        }
        result
    }

    // <expression> ::= <for> | 'continue' | 'break' | 'return' | 'yield' | '{' <block> | <operators>
    #[allow(clippy::too_many_lines)]
    /// @PLN86 step 0.1 — depth-guarded entry to expression parsing.  For trusted
    /// code (`!in_sandbox`) this is a single bool check then a tail call — zero
    /// cost.  Inside a sandboxed def it bounds the nesting depth so hostile
    /// `((((…))))` is a clean LOAD-time parse error, never a native stack
    /// overflow.  All recursion into nested sub-expressions routes back through
    /// here (parens, arithmetic, indexing → `parse_single` → `expression`), so
    /// one chokepoint bounds every nesting form.
    pub(crate) fn expression(&mut self, val: &mut Value) -> Type {
        if !self.in_sandbox {
            return self.expression_inner(val);
        }
        // Once the limit has tripped for this def, every further expression parse
        // is a no-op: the def is already rejected, so we stop recursing entirely
        // — this prevents a re-entry from re-walking the unconsumed deep tail and
        // guarantees the parser unwinds in O(remaining tokens).  Reset per-def in
        // `parse_function`.
        if self.depth_overflowed {
            return Type::Unknown(0);
        }
        self.parse_depth += 1;
        if self.parse_depth > SANDBOX_MAX_PARSE_DEPTH {
            // Stop recursing — emit once (latched) and unwind cleanly.
            self.depth_overflowed = true;
            diagnostic!(
                self.lexer,
                Level::Error,
                "expression nesting too deep in sandboxed code (limit {})",
                SANDBOX_MAX_PARSE_DEPTH
            );
            self.parse_depth -= 1;
            return Type::Unknown(0);
        }
        let result = self.expression_inner(val);
        self.parse_depth -= 1;
        result
    }

    fn expression_inner(&mut self, val: &mut Value) -> Type {
        // Start of the expression — an "Unknown variable" caret on a bare-Var
        // expression (e.g. a single call argument) must point here, not at the
        // cursor that has drifted to the closing `)` / `;` by detection time.
        let expr_pos = self.lexer.peek_pos().clone();
        if self.lexer.has_token("for") {
            self.parse_for(val);
            Type::Void
        } else if self.lexer.has_token("while") {
            self.parse_while(val);
            Type::Void
        } else if self.lexer.has_token("continue") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot continue outside a loop");
            }
            *val = Value::Continue(0);
            Type::Never
        } else if self.lexer.has_token("break") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot break outside a loop");
            }
            // `break expr` — break with a value.  Desugars to `return expr`
            // since loft loops are currently void-typed.  Covers the common
            // find/search pattern where break-with-value exits the function.
            // TODO: implement for...else for the general case.
            if !self.lexer.peek_token("}")
                && !self.lexer.peek_token(";")
                && !matches!(self.lexer.peek().has, crate::lexer::LexItem::None)
            {
                let mut break_val = Value::Null;
                let break_tp = self.expression(&mut break_val);
                let ret_tp = self.data.def(self.context).returned().clone();
                if !self.first_pass && matches!(ret_tp, Type::Void) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "`break <value>` requires a non-void function — \
                         the value is returned from the enclosing function"
                    );
                } else if !self.first_pass
                    && !matches!(ret_tp, Type::Void)
                    && !break_tp.is_same(&ret_tp)
                    && !break_tp.is_unknown()
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "`break` value type {} does not match function return type {}",
                        break_tp.name(&self.data),
                        ret_tp.name(&self.data)
                    );
                }
                *val = Value::Return(Box::new(break_val));
            } else {
                *val = Value::Break(0);
            }
            Type::Never
        } else if self.lexer.has_token("return") {
            self.parse_return(val);
            Type::Never
        } else if self.lexer.has_keyword("parallel") {
            self.parse_parallel(val);
            Type::Void
        } else if self.lexer.has_token("yield") {
            // CO1.3c: yield expr — only valid inside generator functions.
            // M11-a: also forbidden inside a par() body (worker runs in a
            // separate thread; there is no safe coroutine resumption path).
            if self.in_par_body && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "yield is not allowed inside a par(...) parallel body"
                );
                // Consume the expression tokens to keep the lexer in sync, but
                // emit Value::Null so no coroutine IR is generated — generating
                // yield state-machine code outside a coroutine context confuses
                // scope analysis (ref variables without matching OpFreeRef).
                if !self.lexer.has_keyword("from") {
                    let mut discarded = Value::Null;
                    self.expression(&mut discarded);
                }
                return Type::Void;
            }
            let r_type = self.data.def(self.context).returned().clone();
            if !matches!(r_type, Type::Iterator(_, _)) && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "yield is only allowed inside generator functions (return type must be iterator<T>)"
                );
            }
            if self.lexer.has_keyword("from") {
                // CO1.4: yield from sub_gen — desugar to:
                //   __sub = sub; loop { __item = next(__sub); if !__item break; yield __item; }
                let mut sub = Value::Null;
                let sub_type = self.expression(&mut sub);
                if let Type::Iterator(inner, _) = &sub_type {
                    let elem_tp = (**inner).clone();
                    let sub_var = self.create_unique("__yf_sub", &sub_type);
                    self.vars.defined(sub_var);
                    let item_var = self.create_unique("__yf_item", &elem_tp);
                    self.vars.defined(item_var);
                    let op = self.data.def_nr("OpCoroutineNext");
                    let value_size =
                        crate::variables::size(&elem_tp, &crate::data::Context::Argument);
                    let next_call = Value::Call(
                        op,
                        vec![Value::Var(sub_var), Value::Int(i32::from(value_size))],
                    );
                    let mut test = Value::Var(item_var);
                    self.convert(&mut test, &elem_tp, &Type::Boolean);
                    test = self.cl("OpNot", &[test]);
                    let lp = vec![
                        crate::data::v_set(item_var, next_call),
                        crate::data::v_if(
                            test,
                            crate::data::v_block(vec![Value::Break(0)], Type::Void, "break"),
                            Value::Null,
                        ),
                        Value::Yield(Box::new(Value::Var(item_var))),
                    ];
                    let steps = vec![
                        crate::data::v_set(sub_var, sub),
                        crate::data::v_loop(lp, "yield from"),
                    ];
                    *val = crate::data::v_block(steps, Type::Void, "yield from block");
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "yield from requires an iterator expression"
                    );
                }
                Type::Void
            } else {
                let mut v = Value::Null;
                self.expression(&mut v);
                // @P328 — when yielding a NON-CAPTURING closure into an
                // `iterator<fn(...) -> ...>` generator, the expression
                // parser leaves the lambda as a bare `Value::Int(d_nr)`
                // (the closure DbRef would be the null sentinel anyway).
                // But the generator's yielded type is `Function(...)`
                // which the coroutine machinery treats as 20 bytes
                // (8B d_nr + 12B closure DbRef).  Pushing only 4 bytes
                // for the bare Int crashes the consumer's
                // `OpCoroutineNext(gen, 20)` (interp SIGBUS) and
                // mis-types the native channel.  Wrap the bare Int as
                // a proper `Value::FnRef(d_nr, u16::MAX, _)` so the
                // yielded value is a full 20-byte fn-ref.  Capturing
                // closures already arrive as a Block ending in
                // `FnRef(d_nr, closure_var, _)`, so no rewrite needed.
                if let Type::Iterator(elem_tp, _) = &r_type
                    && matches!(**elem_tp, Type::Function(_, _, _))
                {
                    let unspanned = v.unspan().clone();
                    if let Value::Int(d_nr) = unspanned {
                        v = Value::FnRef(d_nr, u16::MAX, Box::new((**elem_tp).clone()));
                    } else if let Value::Long(d_nr) = unspanned {
                        v = Value::FnRef(d_nr as i32, u16::MAX, Box::new((**elem_tp).clone()));
                    }
                }
                *val = Value::Yield(Box::new(v));
                Type::Void
            }
        } else if self.lexer.peek_token("{") {
            self.parse_block("block", val, &Type::Void)
        } else {
            // `const x = expr` — mark the resulting local variable as const after initialisation.
            let const_decl = self.lexer.has_keyword("const");
            let res = self.parse_assign(val);
            if const_decl && !self.first_pass {
                let v_nr = match val {
                    Value::Set(nr, _) => Some(*nr),
                    Value::Insert(ls) => ls.iter().find_map(|v| {
                        if let Value::Set(nr, _) = v {
                            Some(*nr)
                        } else {
                            None
                        }
                    }),
                    _ => None,
                };
                if let Some(v_nr) = v_nr {
                    self.vars.set_const_param(v_nr);
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "const keyword requires a variable assignment"
                    );
                }
            }
            self.known_var_or_type(val, &expr_pos);
            res
        }
    }

    /// L10: `while <cond> { <body> }` desugars to an infinite loop with a break guard.
    ///
    /// The emitted IR is equivalent to:
    ///   loop { if !cond { break }; body }
    pub(crate) fn parse_while(&mut self, code: &mut Value) {
        let mut cond = Value::Null;
        self.expression(&mut cond);
        if !self.first_pass && matches!(cond, Value::Null) {
            diagnostic!(self.lexer, Level::Error, "Expected condition after 'while'");
            return;
        }
        let not_cond = self.cl("OpNot", &[cond]);
        let break_if = v_if(
            not_cond,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );
        let loop_nr = self.vars.start_loop();
        let in_loop = self.in_loop;
        self.in_loop = true;
        let mut body = Value::Null;
        let loop_write_state = self.vars.save_and_clear_write_state();
        self.parse_block("while", &mut body, &Type::Void);
        self.vars.restore_write_state(&loop_write_state);
        self.in_loop = in_loop;
        self.vars.finish_loop(loop_nr);
        *code = v_loop(vec![break_if, body], "while");
    }

    pub(crate) fn change_var(&mut self, code: &Value, tp: &Type) -> bool {
        if let Value::Var(v_nr) = code {
            let mut is_text = matches!(self.vars.tp(*v_nr), Type::Text(_));
            if let Type::RefVar(i) = self.vars.tp(*v_nr)
                && matches!(**i, Type::Text(_))
            {
                is_text = true;
            }
            // #328: `x = x.next` would give x a SELF-dep ("x borrows from
            // x") — a degenerate borrow that flips the var into the
            // dependent-view codegen class (InitCreateStack) and corrupts
            // the frame.  Strip the self-entry; the remaining deps (if
            // any) still carry the real borrow sources.  The pre-Set free
            // stays safe: codegen's S1 guard skips it whenever the RHS
            // reads `v` itself.
            let stripped: Type;
            let tp = if let Type::Reference(d, deps) = tp
                && deps.contains(v_nr)
            {
                let kept: Vec<u16> = deps.iter().copied().filter(|d2| d2 != v_nr).collect();
                stripped = Type::Reference(*d, Deps::frame(kept));
                &stripped
            } else {
                tp
            };
            if !is_text || *tp != Type::Character {
                self.change_var_type(*v_nr, tp);
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn change_var_type(&mut self, v_nr: u16, tp: &Type) {
        // Plan-22 phase 02d-iii.c — preserve the boxed scalar's
        // `Reference(__cell_<T>, _)` type when an assignment's
        // RHS would otherwise revert it.  `parse_assign_op`
        // calls `change_var(to, &s_type)` after parsing the
        // RHS; without this guard, every `n = expr` line in
        // the body would undo the type flip the moment the
        // body walk begins.  Fires only when the current type
        // is a `__cell_*` Reference AND the new type is the
        // cell's value type (the safe "scalar-into-boxed-scalar"
        // overwrite), leaving every other re-typing untouched.
        //
        // Dormant in production today (02d-iii.a's flip is
        // dormant — no variable carries `Reference(__cell_*, _)`).
        // Activates in 02d-iii.e together with the flip.
        if !self.first_pass
            && self.vars.exists(v_nr)
            && let Type::Reference(d, _) = self.vars.tp(v_nr)
            && self.data.def(*d).name().starts_with("__cell_")
            && let Some(value_attr) = self.data.def(*d).attributes().first()
            && value_attr.name == "value"
            && (value_attr.typedef.is_equal(tp)
                || (matches!(value_attr.typedef, Type::Integer(_))
                    && matches!(tp, Type::Integer(_))))
        {
            return;
        }
        let chg = self
            .vars
            .change_var_type(v_nr, tp, &self.data, &mut self.lexer);
        if chg
            && !tp.is_unknown()
            && let Type::Vector(elm, _) = tp
        {
            self.data.vector_def(&mut self.lexer, elm);
        }
    }

    /// Check for iteration-safety violation on `+=` to collections; emit diagnostics.
    pub(crate) fn check_iter_safety(&mut self, to: &Value, f_type: &Type, op: &str) {
        if self.first_pass
            || op != "+="
            || !matches!(
                f_type,
                Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Spacial(_, _, _)
            )
        {
            return;
        }
        if let Value::Var(lhs_nr) = to
            && self.vars.is_iterated_var(*lhs_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot add elements to '{}' while it is being iterated — \
use a separate collection or add after the loop",
                self.vars.name(*lhs_nr)
            );
        } else if !matches!(to, Value::Var(_)) && self.vars.is_iterated_value(to) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot add elements to a collection while it is being iterated — \
use a separate collection or add after the loop"
            );
        }
    }

    /// Validate `d#lock = expr` assignment; returns true if handled (caller should return Void).
    pub(crate) fn validate_lock_assign(&mut self, code: &Value, to: &Value) -> bool {
        if self.first_pass {
            return false;
        }
        let Value::Call(lock_nr, lock_args) = to.unspan() else {
            return false;
        };
        if self.data.def(*lock_nr).name() != "n_get_store_lock" {
            return false;
        }
        if !matches!(code, Value::Boolean(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "d#lock can only be assigned a constant boolean (true or false)"
            );
            return true;
        }
        if matches!(code, Value::Boolean(false))
            && let Some(Value::Var(v_nr)) = lock_args.first()
            && self.vars.is_const_param(*v_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot unlock const variable '{}' via d#lock = false",
                self.vars.name(*v_nr)
            );
            return true;
        }
        false
    }

    /// Apply the operator `op` to an already-parsed LHS and parse the RHS,
    /// then rewrite `code` into the assignment IR. Returns `Type::Void`.
    // threads LHS context (to, f_type, parent_tp, var_nr) alongside op and &mut self
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn parse_assign_op(
        &mut self,
        code: &mut Value,
        op: &str,
        f_type: &Type,
        to: &Value,
        mut parent_tp: Type,
        var_nr: u16,
    ) -> Type {
        self.check_iter_safety(to, f_type, op);
        // Save parent struct type before the RHS parse overwrites parent_tp.
        let lhs_parent_tp = parent_tp.clone();
        // #330: `x = x` is the identity — emit nothing.  Letting it through
        // produced a deep-copy-onto-self whose pre-Set free released the
        // store the RHS was about to read (silent corruption on the
        // interpreter, null on native — and the two diverged).  Killing it
        // here covers BOTH backends and keeps the store identity stable
        // for any live borrows.
        if op == "="
            && let Value::Var(lhs) = to
            && self.lexer.peek().has
                == crate::lexer::LexItem::Identifier(self.vars.name(*lhs).to_string())
        {
            let link = self.lexer.link();
            self.lexer.cont();
            if self.lexer.peek_token(";") {
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            self.lexer.revert(link);
        }
        // @P277 — `local_keyed += [literal]`.  Without this branch, the
        // RHS parse below descends into `parse_vector` (vectors.rs:1372)
        // which at line ~1434 calls `change_var_type(vec, Vector<T>, …)`
        // and fires "Variable 'x' cannot change type from sorted<…> to
        // vector<…>" — the LHS's declared keyed-collection type is lost.
        // The P188 scalar branch below (line ~870) runs AFTER the RHS
        // parse, so by then the diagnostic has already fired.  Intercept
        // here, manually tokenise the literal, parse each item against
        // the element type, and dispatch via `new_record` (vectors.rs:1745)
        // which already routes per-kind (sorted_finish / hash::add /
        // tree::add / ordered_finish) via its P188-followup `lhs_known`
        // lookup at lines 1783-1822.  The struct-field twin at lines
        // ~1159-1194 handles the same shape for fields after-the-fact;
        // we cannot do that here because the LHS-local path errors out
        // before reaching it.
        if op == "+="
            && var_nr != u16::MAX
            && matches!(
                f_type,
                Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Spacial(_, _, _)
            )
            && self.lexer.peek_token("[")
        {
            let elm_tp = f_type.content();
            self.lexer.token("[");
            // Empty literal `+= []` — no-op append.
            if self.lexer.has_token("]") {
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            let mut all_steps: Vec<Value> = Vec::new();
            loop {
                let elm = self.unique_elm_var(&lhs_parent_tp, &elm_tp, var_nr);
                let mut item = Value::Var(elm);
                let mut item_parent = Type::Null;
                let _ = self.parse_operators(&elm_tp, &mut item, &mut item_parent, 0);
                if !self.first_pass {
                    let steps = self.new_record(
                        &mut Value::Var(var_nr),
                        f_type,
                        elm,
                        var_nr,
                        &[item],
                        &elm_tp,
                    );
                    all_steps.extend(steps);
                }
                if !self.lexer.has_token(",") {
                    break;
                }
                if self.lexer.peek_token("]") {
                    // trailing comma
                    break;
                }
            }
            self.lexer.token("]");
            *code = Value::Insert(all_steps);
            return Type::Void;
        }
        // Hint the RHS that the destination has this type — `f#read`
        // (no parens, no cast) picks it up so `s.field = f#read` matches
        // the symmetry of `f += s.field` (which already takes the field's
        // declared width).  Restored to Unknown after the RHS parse so
        // it doesn't leak into unrelated sub-expressions.
        let prev_read_target = std::mem::replace(&mut self.read_target_type, f_type.clone());
        let rhs_pos = self.lexer.peek_pos().clone();
        let mut s_type = self.parse_operators(f_type, code, &mut parent_tp, 0);
        self.read_target_type = prev_read_target;
        // @P376 — POISON an errored whole-RHS that resolves to `Unknown` (pass 2)
        // so the assigned variable doesn't cascade.  In the final pass an Unknown
        // whole-RHS is always an unresolved-name error — an undefined variable
        // (`p = qqq`), an undefined function (`p = nofn(1)`), or an unknown
        // struct (`p = Plyer {…}`, already `Never` via the construction).  The
        // root error is reported (by `known_var_or_type` / the call resolver /
        // the type lookup); leaving the variable `Unknown` makes every downstream
        // use (`p.name`, the `{p.name}` format string, the post-parse sweep)
        // re-report — an 8-9 error pileup ending in a format-string fatal.
        // `Never` makes it the silent poison.  Pass 1 defers, and a forward ref
        // resolves to a concrete type in pass 2, so its RHS is not `Unknown`
        // there; a partly-bad RHS that still types (`p = qqq + 1` → integer) is
        // not unknown, so it is untouched.
        let poison = !self.first_pass && s_type.is_unknown();
        // Check the RHS for unresolved variables — but NOT when `code` IS the
        // assignment target: a failed struct construction / call leaves its
        // discarded target `Var(var_nr)` as `code`, and re-validating it reports
        // the spurious "Unknown variable '<target>'" (the cascade tail).  A
        // genuine RHS variable (`p = qqq`) has `code != Var(var_nr)`, so its real
        // "Unknown variable 'qqq'" still fires.
        let code_is_target = matches!(code, Value::Var(c) if *c == var_nr);
        let skip_rhs_check = matches!(s_type, Type::Never) || (poison && code_is_target);
        if !skip_rhs_check {
            self.known_var_or_type(code, &rhs_pos);
        }
        if poison {
            s_type = Type::Never;
        }
        if let Type::Rewritten(tp) = s_type {
            s_type = *tp;
        }
        // Dead assignment check: after the RHS is parsed (so RHS reads of the
        // variable are already counted), check if the previous write was never read.
        if op == "=" && var_nr != u16::MAX && !self.first_pass && self.vars.exists(var_nr) {
            self.vars.track_write(var_nr, &mut self.lexer);
        }
        // Convert untyped null to typed null for scalar assignments (not collections).
        if s_type == Type::Null
            && op == "="
            && !matches!(
                f_type,
                Type::Reference(_, _)
                    | Type::Enum(_, true, _)
                    | Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
            )
        {
            self.convert(code, &Type::Null, f_type);
        }
        if var_nr == u16::MAX {
            self.validate_write(to, &parent_tp);
        }
        // materialise a collection iterator (e.g. v[a..b] slice) into a vector
        // variable.  CO1.3c: the original "second type Null = coroutine, skip"
        // heuristic was wrong — `parse_vector_index` also returns
        // `Iterator<T, Null>` for slices, and slice → vector is exactly the
        // case we want to materialise.  Instead detect coroutine iters
        // structurally: a coroutine `yield`-driven iter does NOT carry a
        // `Value::Iter(_, _, Block(_), _)` shape — its `next` is the
        // resume-call.  Materialise only when the `next` is a Block (the
        // slice / range / collection-iteration shape).  (@P287)
        // Materialisable iter detection — by IR shape, NOT by s_type.  The
        // `s_type` is the type the RHS *evaluates to*, but parse_operators
        // may silently convert `Iterator<T>` to `Vector<T>` when the LHS
        // expects a vector (`+=` case especially) — leaving `code` as the
        // raw iter IR even though s_type now claims Vector.  We trust the
        // IR shape: a `Value::Iter` with a `Block` next is the
        // slice / range / collection-iter shape we can materialise into a
        // vector via the per-element record-allocator loop.  Coroutine
        // iters have a different `next` shape (resume-call) and don't
        // match.  (@P287)
        let materialisable_iter_shape = matches!(code,
            Value::Iter(_, _, n, _) if matches!(n.as_ref(), Value::Block(_)));
        // Recover the element type from either s_type or the iter's annotation.
        let iter_elm_tp: Option<Type> = if let Type::Iterator(elm, _) = &s_type {
            Some((**elm).clone())
        } else if let Type::Vector(elm, _) = &s_type {
            // s_type was silently converted; pull element type from there.
            Some((**elm).clone())
        } else {
            None
        };
        // @P390 — `v = v[a..b]` self-slice-assign.  The plain materialise emits
        // `OpClearVector(var)` BEFORE the loop reads `v[i]` from the SAME record,
        // so every element reads back null (length preserved, values lost).  When
        // the slice source IS this variable, route through a temp local first (the
        // proven @P287 struct-field pattern), then `OpClearVector + OpAppendVector`
        // on the destination — the temp breaks the alias.  Scoped to TRUE aliasing
        // (`ir_mentions_var`) so the non-aliased `t = v[a..b]` keeps the direct
        // fast path; `+=` never clears, so it stays on the direct path below too.
        if materialisable_iter_shape
            && let Some(elm_tp) = iter_elm_tp.clone()
            && matches!(f_type, Type::Unknown(_) | Type::Vector(_, _))
            && var_nr != u16::MAX
            && op == "="
            && !self.first_pass
            && ir_mentions_var(code, var_nr)
        {
            let iter_tp = Type::Iterator(Box::new(elm_tp.clone()), Box::new(Type::Null));
            let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
            let tmp = self.create_unique("__p390_tmp", &vec_tp);
            self.vars.defined(tmp);
            // (1) materialise the slice iterator into the fresh temp (reads the
            //     source — still intact — and appends to tmp; tmp != source).
            self.materialize_iterator(code, &iter_tp, &Value::Var(tmp), &lhs_parent_tp, tmp, "=");
            // (2) clear the destination and append the temp's contents.
            let dn = self.data.type_def_nr(&elm_tp);
            let rec_tp = Value::Int(i32::from(self.data.def(dn).known_type()));
            let clear = self.cl("OpClearVector", &[Value::Var(var_nr)]);
            let append = self.cl(
                "OpAppendVector",
                &[Value::Var(var_nr), Value::Var(tmp), rec_tp],
            );
            *code = Value::Insert(vec![code.clone(), clear, append]);
            return Type::Void;
        }
        if materialisable_iter_shape
            && let Some(elm_tp) = iter_elm_tp.clone()
            && matches!(f_type, Type::Unknown(_) | Type::Vector(_, _))
            && var_nr != u16::MAX
            && matches!(op, "=" | "+=")
        {
            // Rebuild a real Iterator type so materialize_iterator's destructure works.
            let iter_tp = Type::Iterator(Box::new(elm_tp), Box::new(Type::Null));
            self.materialize_iterator(code, &iter_tp, to, &lhs_parent_tp, var_nr, op);
            return Type::Void;
        }
        // #410 — a DIRECT `#native`-decl call returning a vector delivers a
        // FOREIGN-store value: the FFI bridge wraps the return into the
        // return-only null store (`extensions.rs::bridge_push_ref`).  Bound
        // to a local with a plain `Set`, the local BORROWS that foreign
        // store and never gets its own `__vdb` buffer — so the first
        // in-place `+=` runs `vector_db` (build_vector_list /
        // operators.rs), which allocates a FRESH EMPTY buffer and repoints
        // the local, silently DROPPING the returned elements (len 4 → 1).
        // #409 fixed the loft-WRAPPER shape at the return site; a direct
        // call has no wrapper, so materialise HERE at the assignment: mint
        // the local's own `vector_db` buffer and COPY the foreign return
        // into it (the clear+append delivery the @P390 / @P287 paths use),
        // so by the `+=` the local is shaped like any owned vector and
        // appends in place.  A `#native` decl is `code()==Null &&
        // !native().is_empty()`; an empty `native()` is a builtin op (e.g.
        // `split`), whose vector return is already owned (data.rs::
        // native_symbol_collisions draws the same line).
        let native_vec_elm: Option<Type> = if !self.first_pass
            && op == "="
            && var_nr != u16::MAX
            && !self.vars.is_argument(var_nr)
            && matches!(f_type, Type::Unknown(_) | Type::Vector(_, _))
        {
            match code.unspan() {
                Value::Call(d, _) => {
                    let def = self.data.def(*d);
                    if *def.code() == Value::Null
                        && !def.native().is_empty()
                        && let Type::Vector(elm, _) = def.returned()
                    {
                        Some((**elm).clone())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(elm_tp) = native_vec_elm {
            let rec_tp = self.append_elem_tp(&elm_tp);
            // Route the foreign return through a named local `__fwd` (the
            // #409 shape, control.rs::fwd_copy_409) — NOT the call inline.
            // A named local is what scope analysis recognises as an owned
            // temporary and frees (`OpFreeRef(__fwd)`); appending the call
            // inline copies the elements but orphans the foreign source
            // store (a per-assignment leak, ×N in a loop).
            let fwd = self.create_unique(
                "__fwd",
                &Type::Vector(Box::new(elm_tp.clone()), Deps::none()),
            );
            self.vars.defined(fwd);
            let set_fwd = v_set(fwd, code.clone());
            // `vector_db` takes the ELEMENT type, mints a fresh owned store
            // for the local (OpDatabase + `Set(v, field)` + length 0) and
            // records the `__vdb` dep, so the later `+=` skips its own
            // (destructive) `vector_db`.
            let mut ls = vec![set_fwd];
            ls.extend(self.vector_db(&elm_tp, var_nr));
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(var_nr), Value::Var(fwd), Value::Int(rec_tp)],
            ));
            *code = Value::Insert(ls);
            return Type::Void;
        }
        // @P287 — same materialisation, but the LHS is a struct field
        // (`s.v = slice`) so var_nr is u16::MAX.  Three-step lower:
        //  (1) allocate a temp local vector,
        //  (2) materialise the iterator into the temp via the existing
        //      `materialize_iterator` helper,
        //  (3) emit `OpClearVector(field) + OpAppendVector(field, tmp)` —
        //      the same shape that lines ~1020-1033 below build for
        //      `s.v = fresh_vec` whole-vector field-replace.
        if materialisable_iter_shape
            && let Some(elm_tp) = iter_elm_tp
            && matches!(f_type, Type::Vector(_, _))
            && var_nr == u16::MAX
            && op == "="
            && !self.first_pass
            && self.is_field(to)
        {
            let iter_tp = Type::Iterator(Box::new(elm_tp.clone()), Box::new(Type::Null));
            let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
            let tmp = self.create_unique("__p287_tmp", &vec_tp);
            self.vars.defined(tmp);
            // (2) materialise iter → tmp (mutates *code into the materialise IR).
            //     Pass the rebuilt iter_tp so materialize_iterator's destructure
            //     succeeds even when s_type was silently converted upstream.
            self.materialize_iterator(code, &iter_tp, &Value::Var(tmp), &lhs_parent_tp, tmp, op);
            // (3) emit clear + append on the destination field.
            let dn = self.data.type_def_nr(&elm_tp);
            let rec_tp = Value::Int(i32::from(self.data.def(dn).known_type()));
            let clear = self.cl("OpClearVector", std::slice::from_ref(to));
            let append = self.cl("OpAppendVector", &[to.clone(), Value::Var(tmp), rec_tp]);
            *code = Value::Insert(vec![code.clone(), clear, append]);
            return Type::Void;
        }
        // P188 — local-var collection `+= elem` for keyed collections
        // (sorted/hash/index/spacial) ONLY.  Routes the singleton element
        // through OpNewRecord + OpFinishRecord (per-kind dispatch via
        // record_finish: hash::add / sorted_finish / tree::add /
        // ordered_finish).  Returns Type::Void before change_var fires
        // the "cannot change type from sorted<…> to T" diagnostic that
        // would otherwise reject this shape.
        //
        // @PLAN52 cluster IV-Vec-nested-field-push (2026-05-30): Vector
        // LHS REMOVED from this branch.  Vector `+= elem` is ambiguous
        // with concat for nested-vector cases (`vec<vec<T>> += vec<T>` —
        // bare element type matches both element_type AND a sub-shape of
        // the LHS type).  Strict rule: vector push MUST use `+= [elem]`
        // (explicit brackets).  Falls through to the diagnostic below
        // when the RHS doesn't match the concat shape.
        if op == "+="
            && var_nr != u16::MAX
            && matches!(
                f_type,
                Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Spacial(_, _, _)
            )
        {
            let elm_tp = f_type.content();
            if !elm_tp.is_unknown() && elm_tp.is_equal(&s_type) {
                if !self.first_pass {
                    let elm = self.unique_elm_var(f_type, &elm_tp, var_nr);
                    let mut scalar = code.clone();
                    // Mirror of the field-+= retarget at line ~755:
                    // a struct-literal RHS pre-parses with the LHS local
                    // (`Var(var_nr)`) as its target.  After allocating a
                    // fresh element via new_record, retarget the inits
                    // onto `Var(elm)` so the writes land in the new
                    // record instead of the local-var's storage.
                    substitute_value(&mut scalar, &Value::Var(var_nr), &Value::Var(elm));
                    let ls = self.new_record(
                        &mut Value::Var(var_nr),
                        f_type,
                        elm,
                        var_nr,
                        &[scalar],
                        &elm_tp,
                    );
                    *code = Value::Insert(ls);
                }
                return Type::Void;
            }
        }
        // @PLAN52 cluster IV-Vec-nested-field-push (2026-05-30): strict
        // rule — VECTOR `+= elem` (bare element) is rejected.  Use
        // `+= [elem]` to push a single element, or `+= other_vec`
        // (typeof must equal LHS) to concatenate.  This eliminates the
        // ambiguity class where `vec<vec<T>> += vec<T>` is BOTH
        // "push one element of element-type" AND syntactically resembles
        // "concat" (RHS is vector-typed) — the parser branch order used
        // to misroute the latter.
        if op == "+="
            && let Type::Vector(elm_tp, _) = f_type
            && !s_type.is_unknown()
            && (**elm_tp).is_equal(&s_type)
            && !s_type.is_equal(f_type)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "vector `+= elem` is ambiguous; use `+= [elem]` to push one \
                 element, or `+= other_vec` (typeof must match) to concatenate"
            );
            *code = Value::Insert(Vec::new());
            return Type::Void;
        }
        // C54.A incremental 2a — if the variable carries an annotated
        // target type `: Long` with a narrower `Integer` RHS
        // (e.g. `x: u32 = 100` where `u32` promoted to Long), run
        // `convert()` BEFORE `change_var` fires the "cannot change
        // type" diagnostic.  Narrowly scoped to Integer → Long so
        // other cross-category conversions (e.g. Enum → Integer via
        // OpConvIntFromEnum, which would silently pass but IS a real
        // type mismatch) still fall through to the existing error
        // path.  Similar widens for single→float etc. stay handled
        // by the later convert at op == "=".
        // Post-2c round 10c: Type::Long was folded into Type::Integer;
        // any Integer↔Integer assignment is a no-op and needs no early widen.
        let _ = op;
        let _ = f_type;
        // @PLN25 single-payload — a dense-`S` target assigned a `__nullable<S>` value
        // (`d: S = v[i]`): unwrap the RHS via `convert` (→ a payload sub-ref `OpGetField`)
        // BEFORE `change_var`, then retype it as dense `S` so the var-type-change
        // check accepts (d:S ← S) instead of erroring "cannot change type from S to
        // __nullable<S>".  Mirrors the C54.A early-convert pattern above; NOT
        // pass-2-gated because the type error fires on pass 1 too (`convert`
        // self-guards emission to pass 2 and returns true on both).  Gate-off-inert.
        let nullable_to_dense_assign = op == "="
            && matches!(f_type, Type::Reference(struct_d, _)
                if matches!(&s_type, Type::Enum(syn, true, _)
                    if self.data.def(*syn).name
                        == format!("__nullable<{}>", self.data.def(*struct_d).name())));
        if nullable_to_dense_assign && self.convert(code, &s_type, f_type) {
            s_type = f_type.clone();
        }
        self.change_var(to, &s_type);
        // Plan-22 phase 02d-vi — bypass the text-special branch
        // when the LHS is auto-deref'd boxed text.  The general
        // path (towards_set + call_to_set_op + maybe_prepend_cell_alloc)
        // already has the right OpGetText → OpSetText mapping
        // (added in 02d-iv) and alloc-preamble logic (02d-iii.d/v).
        // The text-special branch's argument-promotion + work-buffer
        // logic doesn't apply to boxed-text locals — they're
        // already a Reference(__cell_text, _), not an argument
        // and not a plain text Var.
        let is_boxed_text_lhs = matches!(f_type, Type::Text(_))
            && self
                .extract_boxed_var_from_lhs(to)
                .and_then(|v_nr| {
                    if !self.vars.exists(v_nr) {
                        return None;
                    }
                    match self.vars.tp(v_nr) {
                        Type::Reference(d, _)
                            if self.data.def(*d).name().starts_with("__cell_") =>
                        {
                            Some(v_nr)
                        }
                        _ => None,
                    }
                })
                .is_some();
        if matches!(f_type, Type::Text(_)) && !is_boxed_text_lhs {
            // auto-promote text argument to local String on first mutation.
            let effective_var = if self.first_pass
                && var_nr != u16::MAX
                && self.vars.is_argument(var_nr)
                && !self.vars.is_const_param(var_nr)
                && (op == "=" || op == "+=")
            {
                let name = self.vars.name(var_nr).to_string();
                let shadow = self.vars.add_variable(
                    &format!("__tp_{name}"),
                    &Type::Text(Deps::none()),
                    &mut self.lexer,
                );
                self.vars.set_promoted_from(shadow, var_nr);
                self.vars.remap_name(&name, shadow);
                shadow
            } else {
                var_nr
            };
            self.assign_text(code, &s_type, to, op, effective_var);
            return Type::Void;
        }
        if self.assign_refvar_text(code, f_type, &s_type, op, var_nr) {
            return Type::Void;
        }
        if self.assign_refvar_vector(code, f_type, op, var_nr) {
            return Type::Void;
        }
        if var_nr != u16::MAX && self.create_vector(code, f_type, op, var_nr) {
            return Type::Void;
        }
        // P193: rewrite `local: keyed_collection<T> = []` to
        // `Set(v, Null)` so codegen's gen_set_first_keyed_null fires
        // at the declaration site (not lazily on first write).
        // Falls through to the standard assign path which emits
        // Set(v, code) — codegen then takes the Null arm.
        if var_nr != u16::MAX && !self.first_pass && self.create_keyed(code, f_type, op, var_nr) {
            // Don't return here — let the standard pipeline emit
            // Set(v, Null) so codegen sees it.  No further special-
            // case handling needed: the rest of the pipeline tolerates
            // `code == Value::Null`.
        }
        // vector-typed field whole-replacement.
        // `s.v = fresh` (RHS is a vector variable/expr) used to silently drop
        // the assignment because towards_set's vector branch returned bare
        // `val`.  `s.v = []` likewise leaked through to the Insert bypass at
        // the end of this function with no SET operator emitted.  Both cases
        // are now rewritten to `OpClearVector(s.v)` (+ `OpAppendVector` when
        // there is RHS data to copy in).
        //
        // P261 (2026-05-13): the third case — `s.v = [literal items]` —
        // previously fell through to the Insert bypass without an
        // OpClearVector prefix, so the literal's element-construction
        // ops appended to the existing items instead of replacing them.
        // Fixed by prepending OpClearVector to the literal's
        // statement list — the existing element-construction ops then
        // run on a fresh-empty field.
        //
        // Skipped:
        // - RHS type is non-Vector (e.g. `b.data = f#read(...)` where f#read
        //   returns text) — preserve the historical silent no-op rather than
        //   emit a type-mismatched OpAppendVector.
        if !self.first_pass
            && op == "="
            && var_nr == u16::MAX
            && matches!(f_type, Type::Vector(_, _))
            && self.is_field(to)
        {
            let is_empty_literal = matches!(code, Value::Insert(ls) if ls.is_empty());
            let is_nonempty_literal = matches!(code, Value::Insert(ls) if !ls.is_empty());
            let rhs_is_vector = matches!(s_type, Type::Vector(_, _));
            if is_empty_literal {
                *code = Value::Insert(vec![self.cl("OpClearVector", std::slice::from_ref(to))]);
                return Type::Void;
            }
            if is_nonempty_literal {
                let clear = self.cl("OpClearVector", std::slice::from_ref(to));
                if let Value::Insert(ls) = code {
                    ls.insert(0, clear);
                }
                return Type::Void;
            }
            if !is_nonempty_literal
                && rhs_is_vector
                && let Type::Vector(elm_tp, _) = f_type
            {
                // `s.v = s.v` is a no-op; don't
                // emit a clear (which would wipe the field) + append (which
                // would then see an empty source).
                // Plan-07 phase 1: compare via unspan() so the same
                // expression wrapped at two different source positions
                // (LHS `s.v` at one column, RHS `s.v` at another) still
                // compares equal.
                if *code.unspan() == *to.unspan() {
                    *code = Value::Insert(Vec::new());
                    return Type::Void;
                }
                let elm_tp_clone = (**elm_tp).clone();
                // @P314 — narrow-aware element type (see `append_elem_tp`).
                let rec_tp = Value::Int(self.append_elem_tp(&elm_tp_clone));
                // When the RHS may alias the destination, the OpClearVector
                // below would wipe that data before OpAppendVector can copy
                // it — capture the RHS into a fresh local first so its
                // storage is independent of the clear.  Two aliasing shapes:
                // - a non-Var RHS (e.g. `s.v = pop_tail(s.v, 1)`), and
                // - #320: a Var that BORROWS the destination (`w = s.v;
                //   w += [43]; s.v = w` — w and the field share one store;
                //   clear+append emptied the field).  A borrow is visible in
                //   the var's type deps; only a dep-free (owned) var is
                //   provably alias-free and may take the direct fast path.
                let owned_var_rhs = matches!(
                    code.unspan(),
                    Value::Var(rv) if self.vars.tp(*rv).depend().is_empty()
                );
                if owned_var_rhs {
                    let clear = self.cl("OpClearVector", std::slice::from_ref(to));
                    let append = self.cl("OpAppendVector", &[to.clone(), code.clone(), rec_tp]);
                    *code = Value::Insert(vec![clear, append]);
                } else if matches!(code.unspan(), Value::Var(_)) {
                    // Borrowed Var: a synthetic `Set(tmp, Var(w))` would alias
                    // again (it bypasses the parse-time vector deep-copy
                    // lowering) — build the temp as a fresh empty vector and
                    // element-append the borrow into it BEFORE the clear.
                    let rhs_saved = code.clone();
                    let dep_free_tp = Type::Vector(Box::new(elm_tp_clone.clone()), Deps::none());
                    let tmp = self.vars.unique("_p154_rhs", &dep_free_tp, &mut self.lexer);
                    let init_tmp = v_set(tmp, Value::Null);
                    let fill_tmp = self.cl(
                        "OpAppendVector",
                        &[Value::Var(tmp), rhs_saved, rec_tp.clone()],
                    );
                    let clear = self.cl("OpClearVector", std::slice::from_ref(to));
                    let append = self.cl("OpAppendVector", &[to.clone(), Value::Var(tmp), rec_tp]);
                    *code = Value::Insert(vec![init_tmp, fill_tmp, clear, append]);
                } else {
                    let rhs_saved = code.clone();
                    let tmp = self.vars.unique("_p154_rhs", f_type, &mut self.lexer);
                    let set_tmp = v_set(tmp, rhs_saved);
                    let clear = self.cl("OpClearVector", std::slice::from_ref(to));
                    let append = self.cl("OpAppendVector", &[to.clone(), Value::Var(tmp), rec_tp]);
                    *code = Value::Insert(vec![set_tmp, clear, append]);
                }
                return Type::Void;
            }
        }
        // @P307 — keyed-collection STRUCT FIELD clear: `s.h = []` where
        // `s.h: sorted`/`hash`/`index<T[K]>`.  The vector-field branch above
        // handles `s.v = []`; the keyed analog used to fall through to the
        // Insert bypass with no op emitted (silent no-op + leak) AND the
        // keyed-field write was never recognised by `check_ref_mutations`
        // (rejecting a `&` param as unmodified — see find_field_written_vars).
        // Lower the empty-literal clear to `OpClearKeyed(field, kt)` which
        // `remove_claims`-frees the contents and zeroes the field's claim
        // pointer, leaving an empty collection a later `+= [..]` re-inits.
        // Mirrors the keyed-LOCAL clear (@P302, via OpDatabase) but for the
        // in-struct claim shape.  Non-empty / non-literal keyed-field
        // reassignment is a separate (harder) case left to its current path.
        if !self.first_pass
            && op == "="
            && var_nr == u16::MAX
            && self.is_field(to)
            && matches!(code, Value::Insert(ls) if ls.is_empty())
        {
            let kt = match &f_type {
                Type::Sorted(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    (c != u16::MAX).then(|| self.database.sorted(c, key))
                }
                Type::Hash(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    (c != u16::MAX).then(|| self.database.hash(c, key))
                }
                Type::Index(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    (c != u16::MAX).then(|| self.database.index(c, key))
                }
                _ => None,
            };
            if let Some(kt) = kt {
                *code = Value::Insert(vec![
                    self.cl("OpClearKeyed", &[to.clone(), Value::Int(i32::from(kt))]),
                ]);
                return Type::Void;
            }
        }
        // @P292 / @P394 — `local_v = other_var_v` where the RHS is a bare vector
        // Var read (not a fresh-storage Block / Call / slice).  The standard Set
        // path would emit `Set(v, Var(rhs))`, which makes v ALIAS rhs's storage:
        //  - @P292: when rhs later goes out of scope its storage is freed → v
        //    dangles → next `len(v)` returns 0 or SEGVs.
        //  - @P394: when this is v's FIRST assignment (its own store not yet
        //    allocated) the alias path never gives v a stack slot → v lands on
        //    u16::MAX → `generate_var` asserts (codegen.rs:2669), or with a
        //    trailing `+=` v silently stays empty and rhs corrupts.
        // Fix (mirrors the slice-materialise branch above): give v its OWN store
        // — `insert_new` when v doesn't own one yet (first assignment), else
        // clear v's existing store (reassignment) — then `OpAppendVector` deep-
        // copies rhs's elements in.  Vectors are copy-semantics, so v is fully
        // independent of rhs afterwards.  Fires on BOTH passes (change_var sets
        // v's type each pass — preserving an existing dep; the alloc/append emit
        // on the second), so the __vdb_N dep is created consistently, exactly as
        // the materialise path at lines ~995 does.  Element type is read from the
        // RHS (s_type) so an untyped `b = a` (f_type Unknown) is covered too.
        // #415 — a STRUCT vector-field read (`af = bx.v`) bound to a fresh local
        // must deep-copy like `a = x`, not alias the field's store. Narrow to a
        // struct field read (base is a `Reference`): a vector INDEX read (`vv[2]`,
        // base is a `Vector`) is also an `OpGetField` but reaches a nested element
        // whose stride the append-copy here would mishandle (plan-58 nested-bool),
        // and it already binds correctly elsewhere — so it is deliberately excluded.
        let struct_vec_field = if let Value::Call(d, args) = code.unspan() {
            *d == self.data.def_nr("OpGetField")
                && matches!(
                    args.first().map(Value::unspan),
                    Some(Value::Var(bv)) if matches!(self.vars.tp(*bv), Type::Reference(_, _))
                )
        } else {
            false
        };
        if op == "="
            && var_nr != u16::MAX
            && (matches!(code, Value::Var(_)) || struct_vec_field)
            && matches!(f_type, Type::Unknown(_) | Type::Vector(_, _))
            && let Type::Vector(elm_tp, _) = &s_type
        {
            // `v = v` self-assign — emit nothing rather than clear+reappend
            // off the same storage.
            if matches!(code.unspan(), Value::Var(rhs_var) if *rhs_var == var_nr) {
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            let elm_tp_clone = (**elm_tp).clone();
            let vec_tp = Type::Vector(Box::new(elm_tp_clone.clone()), Deps::none());
            self.change_var(to, &vec_tp);
            if !self.first_pass {
                // Break the alias.  The standard type-inference copied the RHS
                // var's store dep onto v (making v *borrow* rhs's storage — the
                // dangling-on-scope-exit @P292 hazard, and the no-own-store @P394
                // crash on first assignment).  Strip rhs's deps from v so v gets
                // its OWN store.  This is a no-op when v already owns an
                // independent store (`v = [9]; v = a` reassignment), so that case
                // still takes the clear+refill path below.  (Mirrors the @P295
                // Var-to-var deep-copy dep-strip.)
                if let Value::Var(rhs_var) = code.unspan() {
                    self.vars.make_independent(var_nr, *rhs_var);
                    for d in self.vars.tp(*rhs_var).depend() {
                        self.vars.make_independent(var_nr, d);
                    }
                } else {
                    // #415 — field-read RHS (`af = bx.v`): there is no rhs_var,
                    // but `af` inherited the base's dep ({bx}) during type
                    // resolution.  Strip af's own inherited deps so
                    // `vector_needs_db` below sees an empty dep and allocates af
                    // its OWN store; otherwise it takes the reassignment/clear arm,
                    // af never owns a store, and the alias to bx's field persists.
                    // The OpAppendVector then deep-copies the field's elements in.
                    let inherited = self.vars.tp(var_nr).depend();
                    for d in inherited {
                        self.vars.make_independent(var_nr, d);
                    }
                }
                // @P314 — narrow-aware element type (see `append_elem_tp`).
                let rec_tp = Value::Int(self.append_elem_tp(&elm_tp_clone));
                let mut stmts = Vec::new();
                if self.vector_needs_db(var_nr, &elm_tp_clone, true) {
                    // First assignment: v owns no store yet — allocate one.
                    let elm_var = self.unique_elm_var(&lhs_parent_tp, &elm_tp_clone, var_nr);
                    let db = self.insert_new(var_nr, elm_var, &elm_tp_clone, &mut stmts);
                    self.vars.depend(var_nr, db);
                } else {
                    // Reassignment: v already owns a store — clear, then refill.
                    stmts.push(self.cl("OpClearVector", std::slice::from_ref(to)));
                }
                stmts.push(self.cl("OpAppendVector", &[to.clone(), code.clone(), rec_tp]));
                *code = Value::Insert(stmts);
            }
            return Type::Void;
        }
        // @P295 — `local_s = keyed_expr` where the LHS is a KEYED-collection
        // LOCAL (`sorted`/`hash`/`index`).  The standard Set path emits
        // `Set(s, …)` which `gen_put_var` cannot lower (no `OpPut*` arm for
        // keyed kinds → `Unknown var … type sorted<…>` panic), and a naive
        // alias would dangle when a loop-local RHS is freed (the @P292 bug,
        // for keyed kinds).  Fix: deep-copy via `OpReplaceKeyed`, which does
        // `remove_claims(dest)` (frees s's prior collection + resets its
        // store header) then `copy_claims(src, dest)` — the per-kind
        // deep-copy that rebuilds the bucket/tree index from scratch
        // (`copy_claims_seq_vector` / `_hash_body` / `_index_body`).  This is
        // the same machinery that deep-copies a keyed FIELD when its owning
        // struct is copied; here we route the local-assignment shape through
        // it.  NOTE: unlike `OpCopyRecord` there is NO `copy_block` step —
        // a keyed local's slot is a `DbRef` to a dedicated store, so the
        // collection header lives at (store, 1, 8); copy_block'ing
        // `size(tp)` bytes there corrupts the store (the failure mode of the
        // first attempt).  `s = []` (empty literal) and first-declaration
        // go through `create_keyed` above and are unaffected.
        let keyed_kt = if !self.first_pass && op == "=" && var_nr != u16::MAX {
            match &f_type {
                Type::Sorted(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    (c != u16::MAX).then(|| self.database.sorted(c, key))
                }
                Type::Hash(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    (c != u16::MAX).then(|| self.database.hash(c, key))
                }
                Type::Index(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    (c != u16::MAX).then(|| self.database.index(c, key))
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(kt) = keyed_kt
            && matches!(
                s_type,
                Type::Sorted(_, _, _) | Type::Hash(_, _, _) | Type::Index(_, _, _)
            )
            && !matches!(code, Value::Insert(_) | Value::Null)
        {
            // `s = s` self-assign — emit nothing rather than clear+recopy
            // off the same storage.
            if matches!(code.unspan(), Value::Var(rhs) if *rhs == var_nr) {
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            // 0x8000 high bit frees the source store after the deep copy when
            // the RHS is a fresh-storage call (`s = build()`), matching
            // `copy_ref`'s leak guard.  Plain Var-RHS aliases a live local —
            // no source-free (its own scope frees it).
            #[cfg(not(feature = "wasm"))]
            let tp_val = if self.is_struct_returning_call(code) {
                i32::from(kt) | 0x8000
            } else {
                i32::from(kt)
            };
            #[cfg(feature = "wasm")]
            let tp_val = i32::from(kt);
            let replace = self.cl(
                "OpReplaceKeyed",
                &[code.clone(), to.clone(), Value::Int(tp_val)],
            );
            // The deep-copy gives `s` its OWN store, so it no longer borrows
            // the RHS.  Strip the `s["ns"]` lifetime dep the assignment set
            // up — otherwise scope analysis treats `s` as a borrow and
            // suppresses its own `OpFreeRef` (store leak) while deferring the
            // RHS's free (loop accumulation).  Mirrors the Reference var-to-
            // var deep-copy dep-strip in `scopes.rs::scan_set`.
            //
            // @PLAN52 cluster IV (Vec/Hash/Sorted/Index `??`): the original
            // form below only stripped a single Var-RHS dep, but `??` lowers
            // to a `Block` RHS, so the dep stays in place.  Native
            // `emit_null_dbref` then sees `owns_store=false` and emits the
            // null-DbRef sentinel, which crashes `OpReplaceKeyed`'s
            // `stores.allocations[u16::MAX]` lookup.  Strip ALL deps for
            // any non-Var RHS — after the deep copy `var_nr` owns its store
            // regardless of how the RHS was shaped.
            if let Value::Var(rhs) = code.unspan() {
                self.vars.make_independent(var_nr, *rhs);
            } else {
                let deps: Vec<u16> = match self.vars.tp(var_nr) {
                    Type::Sorted(_, _, d)
                    | Type::Hash(_, _, d)
                    | Type::Index(_, _, d)
                    | Type::Spacial(_, _, d) => d.to_vec(),
                    _ => Vec::new(),
                };
                for d in deps {
                    self.vars.make_independent(var_nr, d);
                }
            }
            // @P300 — prepend `Set(v, Null)` so the destination keeps a
            // recordable `Value::Set` node.  `compute_intervals` only
            // records `first_def` from a `Set`, so without it a FIRST
            // assignment (`x = mk()`) leaves `x` slot-less → the
            // `Incorrect var x[65535]` panic in `generate_var`.
            // `scan_set` (`scopes.rs`) makes the prepend do the right
            // thing for free: on a FIRST assignment `x` is not yet in
            // scope so the `Set(x, Null)` survives → codegen's keyed-Null
            // arm allocates an empty store (`gen_set_first_keyed_null`);
            // on a REASSIGNMENT `v` is already in scope so `scan_set`
            // elides the redundant `Set(v, Null)`, leaving just
            // `OpReplaceKeyed` (whose `remove_claims` clears `v`'s
            // existing store before `copy_claims`).  No parse-time
            // first-vs-reassign discriminator needed.
            *code = Value::Insert(vec![Value::Set(var_nr, Box::new(Value::Null)), replace]);
            return Type::Void;
        }
        // @P295 — `spacial` reassignment is not yet supported (copy_claims
        // and insert_record both `panic!("Not implemented")` for Spacial).
        // Reject with an actionable error instead of crashing in codegen.
        if !self.first_pass
            && op == "="
            && var_nr != u16::MAX
            && matches!(f_type, Type::Spacial(_, _, _))
            && matches!(s_type, Type::Spacial(_, _, _))
            && !matches!(code, Value::Insert(_) | Value::Null)
            && !matches!(code.unspan(), Value::Var(rhs) if *rhs == var_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reassigning a spacial collection local is not yet supported \
                 (@P295) — build into a fresh local you return/pass, or mutate \
                 in place"
            );
            *code = Value::Insert(Vec::new());
            return Type::Void;
        }
        // `lhs += other_vec` where both sides are vectors: append all elements
        // in-place via OpAppendVector.
        //
        // @PLAN52 cluster IV-Vec-nested-field-push: STRICT rule — concat is
        // legal only when `typeof(rhs) == typeof(lhs)` exactly.  Any mismatch
        // (element-type, narrow-type, or otherwise) is rejected.  Single-
        // element-push case (RHS type == LHS element type) is already caught
        // by the diagnostic just before this branch.
        if !self.first_pass
            && op == "+="
            && let Type::Vector(elm_tp, _) = &f_type.clone()
            && matches!(s_type, Type::Vector(_, _))
            && !matches!(code, Value::Insert(_))
        {
            if !s_type.is_equal(f_type) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "vector `+= other_vec` requires equal types ({} != {})",
                    f_type.name(&self.data),
                    s_type.name(&self.data)
                );
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            // @P314 — narrow-aware element type (see `append_elem_tp`).
            let elm = (**elm_tp).clone();
            let rec_tp = self.append_elem_tp(&elm);
            *code = Value::Insert(vec![self.cl(
                "OpAppendVector",
                &[to.clone(), code.clone(), Value::Int(rec_tp)],
            )]);
            return Type::Void;
        }
        // Scalar `field += elem` where field is a vector field (var_nr == u16::MAX)
        // and the RHS is a single expression (variable, function call) — NOT
        // a struct literal.  Struct-literal RHS is handled by the keyed-
        // collection branch below, which also covers vectors.
        if !self.first_pass
            && var_nr == u16::MAX
            && op == "+="
            && self.is_field(to)
            && let Type::Vector(elm_tp, _) = f_type
            && !matches!(code, Value::Insert(_))
        {
            let elm_tp = (**elm_tp).clone();
            let elm = self.unique_elm_var(&lhs_parent_tp, &elm_tp, u16::MAX);
            let scalar = code.clone();
            let ls = self.new_record(
                &mut to.clone(),
                &lhs_parent_tp,
                elm,
                u16::MAX,
                &[scalar],
                &elm_tp,
            );
            *code = Value::Insert(ls);
            return Type::Void;
        }
        // P192 follow-up: `field += elem` for keyed-collection fields
        // (hash/sorted/index/spacial<T[key]>) AND vector fields when
        // the RHS is a struct literal (Value::Insert).  Without this,
        // `db.h += Score{...}` emitted raw `OpSetText` / `OpSetInt`
        // writes targeting the field itself — which for hash/index
        // fields overwrote the root pointer (4-byte ref) and for
        // vector fields wrote into the length-prefix word.  The
        // pre-parsed struct-literal steps target `to` (the LHS field
        // expression); after allocating a new element via
        // `new_record_field_op`, walk the steps and substitute `to`
        // → `Var(elm)` so each field write lands in the new record.
        if !self.first_pass
            && var_nr == u16::MAX
            && op == "+="
            && self.is_field(to)
            && matches!(
                f_type,
                Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Spacial(_, _, _)
            )
            && matches!(code, Value::Insert(_))
        {
            let elm_tp = f_type.content();
            // Only fire for single-element append (RHS type matches the
            // collection's element type — e.g. `Score{}` for `hash<Score>`).
            // Multi-element append (RHS is a vector literal of element-typed
            // values like `[1, 2, 3]` for `vector<i32>`) keeps its
            // pre-existing handling further down (OpAppendVector / direct
            // bulk inits).
            if !elm_tp.is_unknown() && elm_tp.is_equal(&s_type) {
                let elm = self.unique_elm_var(&lhs_parent_tp, &elm_tp, u16::MAX);
                let mut scalar = code.clone();
                substitute_value(&mut scalar, to, &Value::Var(elm));
                let ls = self.new_record(
                    &mut to.clone(),
                    &lhs_parent_tp,
                    elm,
                    u16::MAX,
                    &[scalar],
                    &elm_tp,
                );
                *code = Value::Insert(ls);
                return Type::Void;
            }
        }
        // route the RHS of a simple assignment through `convert()`
        // so it picks up the same widening-with-explicit-narrowing policy
        // the constructor path (`handle_field`) and return-type path
        // (`parse_return`) already use.  `convert()` wraps `code` with an
        // `OpConv*FromY` call when a matching widening op is registered
        // (`integer → long`, `integer → float`, `single → float`, …) and
        // returns false on unrelated / narrowing mismatches — which we
        // surface as a clean diagnostic rather than silent runtime
        // corruption.  Guarded to `op == "="`: compound assignments
        // (`+=`, `-=`, …) flow through `compute_op_code` which type-checks
        // via the operator's own attribute list.  Skip when either side
        // is `Unknown` — generic / bounded-template bodies carry
        // placeholder types until monomorphisation; letting convert()
        // fire there would report spurious "cannot assign X to
        // unknown(0)" errors.
        // Restrict to scalar target types: collection targets
        // (`vector`, `hash`, `sorted`, `index`, `spacial`) and
        // `Reference` / `RefVar` compound types have their own
        // handling paths (`towards_set`, `copy_ref`, …).  Running
        // `convert()` on them would flag legitimate initialisations
        // (e.g. `h.field = [...]` for a hash field) as mismatches.
        let scalar_target = matches!(
            f_type,
            Type::Integer(_)
                | Type::Float
                | Type::Single
                | Type::Boolean
                | Type::Character
                | Type::Text(_)
        );
        if op == "="
            && scalar_target
            && !self.first_pass
            && !f_type.is_unknown()
            && !s_type.is_unknown()
            && !matches!(s_type, Type::Null)
            && !f_type.is_equal(&s_type)
            && !self.convert(code, &s_type, f_type)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot assign {} to a field of type {} — use 'as {}' to cast explicitly",
                s_type.name(&self.data),
                f_type.name(&self.data),
                f_type.name(&self.data),
            );
        }
        // @PLAN48 P2: `x: i32 = some_integer` narrows (loses data) but integer and
        // i32 are `is_equal`, so it bypasses the convert-based check above.  Require
        // an explicit `as` unless the RHS is a constant that provably fits.
        if op == "=" && !self.first_pass && Self::is_narrowing_int(&s_type, f_type) {
            let dst = self.int_type_name(f_type);
            if let Some(hint) = self.nullable_sentinel_hint(code, f_type, &dst) {
                // The literal fits the type but lands on the reserved null
                // sentinel of a nullable narrow FIELD — explain that, not "too big".
                diagnostic!(self.lexer, Level::Error, "{hint}");
            } else if !self.int_value_fits(code, f_type) {
                let src = self.int_type_name(&s_type);
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot implicitly narrow {src} to {dst} (may lose data) — cast explicitly with `as {dst}`"
                );
            }
        }
        if self.validate_lock_assign(code, to) {
            return Type::Void;
        }
        // For const variables the Insert path (e.g. struct constructor) bypasses
        // towards_set, so check const here before that path can be taken.
        if matches!(code, Value::Insert(_))
            && !self.first_pass
            && var_nr != u16::MAX
            && self.vars.is_const_param(var_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot modify {} '{}'; remove 'const' or use a local copy",
                self.vars.const_kind(var_nr),
                self.vars.name(var_nr)
            );
        }
        if !matches!(code, Value::Insert(_)) {
            *code = self.towards_set(to, code, f_type, &s_type, &op[0..1]);
            // Plan-22 phase 02d-iii.e — wrap a first-set boxed-
            // scalar write with the cell-allocation preamble.
            // No-op for any other LHS shape (subsequent sets,
            // closure-body writes via `get_field`, non-boxed
            // locals, struct field writes).
            *code = self.maybe_prepend_cell_alloc(code.clone(), to);
        }
        // emit field constraint check after assignment to a constrained field.
        if !self.first_pass
            && let Type::Reference(struct_dnr, _) = &parent_tp
            && let Value::Call(_, to_args) = to.unspan()
            && to_args.len() >= 2
            && let Value::Int(field_offset) = to_args[1].unspan()
        {
            let sd = *struct_dnr;
            let off = *field_offset;
            // Find the field by matching its database offset.
            for a_nr in 0..self.data.def(sd).attributes().len() {
                let nm = self.data.attr_name(sd, a_nr);
                let fpos = self.database.position(self.data.def(sd).known_type(), &nm);
                if i32::from(fpos) == off
                    && self.data.def(sd).attributes()[a_nr].check != Value::Null
                {
                    let check = self.data.def(sd).attributes()[a_nr].check.clone();
                    let ref_val = to_args[0].clone();
                    let bound = Self::replace_record_ref(check, &ref_val);
                    let msg = match &self.data.def(sd).attributes()[a_nr].check_message {
                        Value::Text(s) => Value::Text(s.clone()),
                        _ => Value::Text(format!(
                            "field constraint failed on {}.{nm}",
                            self.data.def(sd).name()
                        )),
                    };
                    let assert_dnr = self.data.def_nr("n_assert");
                    let pos = self.lexer.pos();
                    let assert_call = Value::Call(
                        assert_dnr,
                        vec![
                            bound,
                            msg,
                            Value::Text(pos.file.clone()),
                            Value::Int(pos.line as i32),
                        ],
                    );
                    *code = Value::Insert(vec![code.clone(), assert_call]);
                    break;
                }
            }
        }
        Type::Void
    }

    /// @PLN25 E2/E3 — rewrite a nullable struct ELEMENT type `Reference(S)` to the
    /// synthetic `__nullable<S>` enum (the inline nullable representation).  Called
    /// at the ONE chokepoint where every inline `vector<S>` element resolves
    /// (`sub_type`'s `vector` arm, definitions.rs), so locals / params / returns /
    /// fields / nested all rewrite consistently.  **Default = nullable;** the
    /// caller skips this for an `S not null` element (E3 dense opt-out).  Gated on
    /// `LOFT_E2_SYNTH` — the inline-element ACCESS glue (par workers, native
    /// element reads, ref-params, casts, element-into-local mutation) is
    /// incomplete, so default-on breaks ~107 tree-wide tests (plan25 § Default-on
    /// trigger).  Stdlib excluded so the gated state never rewrites stdlib types.
    /// Creates the def ONLY — `fill_all` does the `register_enum_db` + layout in the
    /// correct order (registering mid-parse corrupts the shared enum: the
    /// `known_type != MAX` guard in fill_database then suppresses the correct
    /// in-order registration → `id × 512` reads).
    /// @PLN25 — whether the E2 vector-element rewrite (`vector<S>` →
    /// `vector<__nullable<S>>`) is active for the CURRENT source.  The native
    /// stdlib (`STD_SOURCE`) always stays DENSE: its `#rust` bodies write the
    /// dense struct ABI (e.g. `fields()` → `vector<JsonField>`), which an E2 wrap
    /// would desync.  E2 applies ABOVE the native layer — user files + libraries.
    /// THE default-on switch lives here (one place): drop the source check / add
    /// an `LOFT_E2_SYNTH` env gate to re-gate.
    pub(crate) fn e2_rewrite_enabled(&self) -> bool {
        // @PLN25 — GATE FLIP LANDED (2026-06-20): nullable-by-default is ON for all user files +
        // libraries; only the native stdlib (`STD_SOURCE`) stays dense.  No `LOFT_E2_SYNTH` env gate
        // any more — the full suite is 2416/2417 (the lone fail is the pre-existing environmental
        // `kernel_port`, unrelated to E2).  The flip tail was cleared: keyed collections reverted to
        // dense + shared-nullable record-sharing (Scope A/B), 86 (bounded-generic dispatch on a
        // nullable element), 371 (forward-ref local-vector synth-enum layout), imaging (native-managed
        // pixel vector marked `not null`); moros_glb was a cache-gotcha artifact.  See
        // single-payload-refactor.md § "Gate-ON flip tail".
        self.data.source != crate::data::STD_SOURCE
    }

    pub(crate) fn e2_nullable_elem(&mut self, elem: Type) -> Type {
        if !self.e2_rewrite_enabled() {
            return elem;
        }
        let Type::Reference(struct_d, _) = &elem else {
            return elem;
        };
        let struct_d = *struct_d;
        // @PLN25 E2 — a generic `vector<T>` must stay dense: `T` is an opaque
        // type-variable stub (registered as a `DefType::Struct` so `parse_type`
        // resolves it), not a real struct, and nullability is decided at
        // instantiation by whatever concrete element type the caller's vector
        // carries (monomorphization substitutes T's bound type directly — see
        // `substitute_type`).  Rewriting here would bury T inside `__nullable<T>`
        // and break the "T appears in the first parameter" check + the return
        // type unification.
        if struct_d == self.cur_type_var {
            return elem;
        }
        // The eligibility (non-stdlib, non-synthetic struct) and the synth-enum
        // lookup live in ONE home — `typedef::nullable_vector_elem` — shared with the
        // deferred forward-ref resolver (`copy_unknown_fields`), so a `vector<S>`
        // element resolves identically whether `S` was known here or only later.
        // (A STDLIB struct stays DENSE even inside a consumer's collection — its
        // `#rust` bodies write the dense ABI — which the helper's struct-source check
        // enforces.)
        match crate::typedef::nullable_vector_elem(&mut self.data, &mut self.lexer, struct_d) {
            Some(syn) => Type::Enum(syn, true, Deps::none()),
            None => elem,
        }
    }

    // <assign> ::= <operators> [ '=' | '+=' | '-=' | '*=' | '%=' | '/=' <operators> ]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_assign(&mut self, code: &mut Value) -> Type {
        let mut parent_tp = Type::Null;
        let mut f_type = self.parse_operators(&Type::Unknown(0), code, &mut parent_tp, 0);
        if let (Type::RefVar(_), Value::Var(v_nr)) = (&f_type, &code) {
            self.vars.in_use(*v_nr, true);
        }
        // Type annotation: `v: type = expr`
        // Only attempt outside format-string expressions (where `:` is used for
        // format specifiers like `{c:#}`).  Consume `: type` only when `=`
        // follows, confirming this is an annotated declaration.
        if let Value::Var(v_nr) = code
            && self.vars.exists(*v_nr)
            && !self.in_format_expr
            && self.lexer.peek_token(":")
        {
            let lnk = self.lexer.link();
            self.lexer.cont(); // consume ":"
            let mut got_annotation = false;
            if let Some(tp) = self.parse_type_full(u32::MAX, false)
                && self.lexer.peek_token("=")
            {
                // @PLN25 E2/E3 — the nullable-element rewrite now happens at the
                // vector-type-resolution chokepoint (definitions.rs `sub_type`
                // `vector` arm), so a `vector<S>` annotation already arrives
                // rewritten; no per-site hook here.
                self.change_var_type(*v_nr, &tp);
                f_type = tp;
                got_annotation = true;
            }
            if !got_annotation {
                self.lexer.revert(lnk);
            }
        }
        // T1.2: LHS tuple destructuring — (a, b) = expr
        // P194: tuple-typed field reassignment — `p.v = (...)` where
        // v is a tuple-typed field.  `get_val::Type::Tuple` returns
        // `Value::Tuple([reads])` for a tuple field read; the reads
        // are `OpGet*(host_ref, base_pos + element_offset[i])`.  When
        // the LHS shape is "tuple of reads (not all Var)" AND f_type
        // is `Type::Tuple`, route to `emit_tuple_set_ops` instead of
        // the destructuring branch.
        if let Value::Tuple(vars) = code.unspan()
            && self.lexer.has_token("=")
        {
            if let Type::Tuple(elems) = &f_type
                && !vars.is_empty()
                && !vars.iter().all(|v| matches!(v, Value::Var(_)))
                && let Some((host_ref, first_pos)) = leaf_tuple_lhs(&vars[0])
            {
                let elems_vec = elems.clone();
                let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
                let offsets: Vec<u16> = crate::data::stored_tuple_offsets_for_def(
                    &self.data,
                    &self.database,
                    tuple_d_nr,
                    elems_vec.len(),
                )
                .unwrap_or_else(|| {
                    crate::data::element_offsets(&elems_vec)
                        .into_iter()
                        .map(|x| x as u16)
                        .collect()
                });
                let host_field_pos = (first_pos as u16).saturating_sub(offsets[0]);
                let mut rhs = Value::Null;
                let _rhs_type = self.expression(&mut rhs);
                let ops = self.emit_tuple_set_ops(&host_ref, host_field_pos, &elems_vec, rhs);
                *code = crate::data::v_block(ops, Type::Void, "tuple_field_set_via_assign");
                return Type::Void;
            }
            let var_nrs: Vec<u16> = vars
                .iter()
                .filter_map(|v| {
                    if let Value::Var(nr) = v {
                        Some(*nr)
                    } else {
                        None
                    }
                })
                .collect();
            if var_nrs.len() != vars.len() {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Tuple destructuring requires plain variable names"
                );
            }
            let mut rhs = Value::Null;
            let rhs_type = self.expression(&mut rhs);
            // A7.1: accept both `Type::Tuple([…])` and the synthetic
            // `Reference(__tuple<…>)` shape that A7.1's parse_function
            // gate widen produces for tuple returns wider than 8B.
            // For the synthetic-struct shape, element reads go through
            // `get_val` with the struct's per-attribute offset — same
            // path P189b's `.0` / `.1` element access takes (mirrors
            // the for-loop destructure in collections.rs:1289-1304).
            let (rhs_elems_opt, ref_def_nr): (Option<Vec<Type>>, u32) = match &rhs_type {
                Type::Tuple(elems) => (Some(elems.clone()), u32::MAX),
                Type::Reference(d_nr, _) if self.data.def(*d_nr).name().starts_with("__tuple<") => {
                    let elems: Vec<Type> = self
                        .data
                        .def(*d_nr)
                        .attributes
                        .iter()
                        .map(|a| a.typedef.clone())
                        .collect();
                    (Some(elems), *d_nr)
                }
                _ => (None, u32::MAX),
            };
            if let Some(rhs_elems) = rhs_elems_opt {
                if rhs_elems.len() != var_nrs.len() {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Tuple arity mismatch: left has {} names, right has {} elements",
                        var_nrs.len(),
                        rhs_elems.len()
                    );
                }
                // T1.4: create a temp variable for the RHS tuple, then read elements.
                let tmp_tp = rhs_type.clone();
                let tmp = self.vars.work_refs(&tmp_tp, &mut self.lexer);
                if !self.first_pass {
                    self.change_var_type(tmp, &tmp_tp);
                }
                let mut steps = vec![Value::Set(tmp, Box::new(rhs))];
                for (i, &v_nr) in var_nrs.iter().enumerate() {
                    if self.vars.exists(v_nr) {
                        self.vars.defined(v_nr);
                        if i < rhs_elems.len() {
                            self.change_var_type(v_nr, &rhs_elems[i]);
                        }
                    }
                    let read = if ref_def_nr == u32::MAX {
                        Value::TupleGet(tmp, i as u16)
                    } else {
                        let elem_offset = if let Some(offs) =
                            crate::data::stored_tuple_offsets_for_def(
                                &self.data,
                                &self.database,
                                ref_def_nr,
                                rhs_elems.len(),
                            ) {
                            u32::from(offs[i])
                        } else {
                            crate::data::element_offsets(&rhs_elems)[i] as u32
                        };
                        // P250 fix (2026-05-11): when destructuring a synthetic
                        // `__tuple<...>` struct (the wider-than-8B tuple-return
                        // shape), each LHS Reference element is a VIEW into the
                        // tmp's storage (`OpGetField(tmp, offset, ...)` returns
                        // a DbRef that shares store_nr/rec with tmp).  Without a
                        // dep, scope analysis emits an independent `OpFreeRef`
                        // for the LHS at scope exit; that free works on a
                        // store_nr basis and frees the entire tmp's underlying
                        // store.  In a loop body, the next iteration's `tmp =
                        // make_pair(...)` reassignment then runs `OpFreeRef(tmp)`
                        // on the now-stale DbRef whose store_nr has been recycled
                        // by an unrelated allocation (e.g. the new `pa`),
                        // silently destroying that allocation.  The first LHS
                        // arg's projection is most affected because the freshly-
                        // allocated `pa` lands in the same store slot the prior
                        // tuple occupied.  Marking the LHS dependent on tmp
                        // suppresses its independent free; tmp's `OpFreeRef`
                        // alone reclaims the storage at the right time.  Only
                        // applies to the synthetic-struct path (Reference
                        // elements); the inline `TupleGet` path (small tuples ≤
                        // 8B) reads value-typed elements that need no free.
                        if matches!(rhs_elems[i], Type::Reference(_, _)) {
                            self.vars.depend(v_nr, tmp);
                        }
                        self.get_val(&rhs_elems[i], false, elem_offset, Value::Var(tmp), u32::MAX)
                    };
                    steps.push(Value::Set(v_nr, Box::new(read)));
                }
                *code = Value::Insert(steps);
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot destructure a non-tuple value"
                );
            }
            return Type::Void;
        }
        // T1.4-fix-a + P248 — tuple element assignment `t.<chain> = expr`,
        // including nested `t.0.1 = expr`.
        //
        // Plan-07 phase 1: unspan() so the wrap on `.` (step 1.12) does
        // not hide the TupleGet shape.  For depth ≥ 2 the parser
        // materialised the read as `Block[Set(w0, TupleGet(t, 0)),
        // TupleGet(w0, 1)]` (operators.rs case 3 — temporary tuple
        // work var).  Without P248's recursive extractor that Block
        // landed at the assignment dispatcher, didn't match the
        // single-level TupleGet arm, and fell through to the
        // "Not implemented operation = for type integer" diagnostic.
        if let Some(lhs) = extract_nested_tuple_lhs(code)
            && self.lexer.has_token("=")
        {
            let mut rhs = Value::Null;
            self.expression(&mut rhs);
            *code = build_nested_tuple_assign(code, &lhs, rhs);
            return Type::Void;
        }
        // T1.11b: compound assignment on a tuple LHS is not supported.
        // (a, b) = expr is handled above; (a, b) += expr has no defined semantics.
        // Return early in both passes to prevent downstream "No matching operator" errors.
        // Consume the operator and RHS so the parser state stays clean after the early exit.
        if matches!(code, Value::Tuple(_))
            && ["+=", "-=", "*=", "%=", "/="]
                .iter()
                .any(|op| self.lexer.peek_token(op))
        {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "compound assignment is not supported for tuple destructuring — use (a, b) = expr instead"
                );
            }
            for op in ["+=", "-=", "*=", "%=", "/="] {
                if self.lexer.has_token(op) {
                    break;
                }
            }
            let mut discard = Value::Null;
            self.expression(&mut discard);
            return Type::Void;
        }
        let to = code.clone();
        for op in ["=", "+=", "-=", "*=", "%=", "/="] {
            if self.lexer.has_token(op) {
                // Mark the variable as defined only once we have confirmed the `=` token
                // is actually present. Doing this before the token check caused any bare
                // `Value::Var` (e.g. `{cd}` inside a format string) to be marked defined
                // prematurely, hiding the "use before assignment" diagnostic.
                if op == "="
                    && let Value::Var(v_nr) = code
                    && !self.first_pass
                    && self.vars.exists(*v_nr)
                {
                    self.vars.defined(*v_nr);
                }
                let var_nr = self.assign_var_nr(code, op, &f_type, &mut parent_tp);
                // Handle `f += X` for File variables before type-changing logic.
                if op == "+="
                    && self.is_file_var_type(&f_type)
                    && let Value::Var(file_v) = to
                {
                    self.append_to_file(code, file_v);
                    return Type::Void;
                }
                // record closure association if the RHS was a capturing lambda.
                // NOTE: must come AFTER parse_assign_op because that is where the RHS
                // lambda is parsed and last_closure_work_var gets set by emit_lambda_code.
                let result = self.parse_assign_op(code, op, &f_type, &to, parent_tp, var_nr);
                if op == "=" && self.last_closure_work_var != u16::MAX && var_nr != u16::MAX {
                    self.closure_vars.insert(var_nr, self.last_closure_work_var);
                    // store mapping in Function struct for native codegen.
                    self.vars
                        .set_closure_var_of(var_nr, self.last_closure_work_var);
                    self.last_closure_work_var = u16::MAX;
                }
                return result;
            }
        }
        *code = to;
        f_type
    }

    pub(crate) fn append_to_text(
        &mut self,
        code: &mut Value,
        op: &str,
        var_nr: u16,
        s_type: &Type,
    ) {
        if !self.first_pass && self.vars.is_const_param(var_nr) && !matches!(code, Value::Insert(_))
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot modify {} '{}'; remove 'const' or use a local copy",
                self.vars.const_kind(var_nr),
                self.vars.name(var_nr)
            );
        }
        if let Value::Insert(ls) = code {
            // P217: same self-append handling as `Parser::assign_text`
            // (operators.rs).  When the RHS expression was `var + parts`,
            // `parse_append_text` emits the first op as
            // `OpAppendText(var, Var(var))` (self-append).  Without
            // detection, the downstream emit clears the destination
            // before the appends and so destroys the existing value
            // (interp's `out = out + "y"` produced "xxy"; native rejected
            // with E0502 because `*var += &*(&*var)` is borrow-conflict).
            // The check unspans both args so a `Span(Var(...))` operand
            // (the parser's source-position wrapper) doesn't slip past.
            if op == "=" {
                let self_append = ls.first().is_some_and(|first| {
                    if let Value::Call(_, args) = first.unspan() {
                        args.len() >= 2
                            && matches!(args[0].unspan(), Value::Var(v) if *v == var_nr)
                            && matches!(args[1].unspan(), Value::Var(v) if *v == var_nr)
                    } else {
                        false
                    }
                });
                if self_append {
                    ls.remove(0);
                }
            }
        } else if op == "=" {
            // P223: when the RHS Block (or any expression) reads the
            // destination var, the interpreter's text-Set clears the
            // destination before evaluating the RHS, so the read sees
            // the cleared value.  Wrap in a work-text so the Block is
            // fully evaluated into the work, then assigned to the
            // destination after the read has happened.  Mirrors the
            // analogous wrap in `Parser::assign_text` (operators.rs)
            // — local-text path — but for the RefVar(Text) parameter
            // path that lands here.
            if !self.first_pass && var_nr != u16::MAX && code.reads_var(var_nr) {
                let work = self.vars.work_text(&mut self.lexer);
                let ls = vec![
                    self.cl("OpClearText", &[Value::Var(work)]),
                    self.cl("OpAppendText", &[Value::Var(work), code.clone()]),
                    v_set(var_nr, Value::Var(work)),
                ];
                *code = Value::Insert(ls);
            } else {
                *code = v_set(var_nr, code.clone());
            }
        } else if s_type == &Type::Character {
            *code = self.cl(
                "OpAppendStackCharacter",
                &[Value::Var(var_nr), code.clone()],
            );
        } else {
            *code = self.cl("OpAppendStackText", &[Value::Var(var_nr), code.clone()]);
        }
    }

    pub(crate) fn append_to_file(&mut self, code: &mut Value, file_v: u16) {
        let mut rhs_code = Value::Null;
        let mut unused = Type::Null; // parent_tp, this is normally used to unpack the vector fill
        // Clear any leftover cast marker so we only pick up a cast parsed as
        // part of THIS rhs expression.
        self.last_cast_alias = u32::MAX;
        let mut rhs_type = self.parse_operators(&Type::Unknown(0), &mut rhs_code, &mut unused, 0);
        if let Type::Rewritten(tp) = rhs_type {
            rhs_type = *tp;
        }
        let cast_alias = self.last_cast_alias;
        self.last_cast_alias = u32::MAX;
        *code = self.write_to_file(file_v, rhs_code, &rhs_type, cast_alias);
    }

    /// Plan-22 phase 02d-v — extract the boxed-scalar `v_nr`
    /// from an auto-deref'd LHS expression.  Recognises two
    /// shapes that phase 02d-iii.b's `auto_deref_boxed_scalar`
    /// produces:
    ///
    /// - **Direct** (Integer / Float / Single / Character /
    ///   Text / plain Enum):
    ///   `Call(OpGet<T>, [Var(v_nr), Int(0)])`
    /// - **Boolean** (byte-storage with bool conversion):
    ///   `Call(OpEqInt, [Call(OpGetByte, [Var(v_nr), Int(0),
    ///   Int(0)]), Int(1)])`
    ///
    /// Returns `None` for any other shape (struct field reads,
    /// non-zero offsets, plain `Var` LHS, closure-body `get_field`
    /// inner — the closure-body case doesn't need the alloc
    /// preamble because the cell was already allocated by the
    /// parent's first-set).
    fn extract_boxed_var_from_lhs(&self, lhs: &Value) -> Option<u16> {
        let Value::Call(op_d, args) = lhs.unspan() else {
            return None;
        };
        let op_name = self.data.def(*op_d).name();
        // Direct shape: Call(OpGet<T>, [Var(v_nr), Int(0)])
        if args.len() == 2
            && matches!(args[1].unspan(), Value::Int(0))
            && op_name.starts_with("OpGet")
            && let Value::Var(v_nr) = args[0].unspan()
        {
            return Some(*v_nr);
        }
        // Boolean shape: Call(OpEqInt, [Call(OpGetByte, [Var, 0, 0]), Int(1)])
        if op_name == "OpEqInt"
            && args.len() == 2
            && matches!(args[1].unspan(), Value::Int(1))
            && let Value::Call(inner_op, inner_args) = args[0].unspan()
            && self.data.def(*inner_op).name() == "OpGetByte"
            && inner_args.len() == 3
            && matches!(inner_args[1].unspan(), Value::Int(0))
            && matches!(inner_args[2].unspan(), Value::Int(0))
            && let Value::Var(v_nr) = inner_args[0].unspan()
        {
            return Some(*v_nr);
        }
        None
    }

    /// Plan-22 phase 02d-iii.d — prepend cell allocation when a
    /// first-set assignment targets an uninitialised boxed-scalar
    /// local in the parent body.
    ///
    /// Context: phase 02d-iii.b's auto-deref wraps every read of
    /// a boxed-scalar local as `Call(OpGet<T>, [Var(v_nr),
    /// Int(0)])`.  When that shape lands on the LHS of an
    /// assignment, the existing `towards_set` →
    /// `call_to_set_op` machinery (parser/operators.rs:283)
    /// correctly maps `OpGet<T>` → `OpSet<T>` and yields
    /// `Call(OpSet<T>, [Var(v_nr), Int(0), rhs])`.  But for the
    /// FIRST set, `v_nr`'s slot holds an uninitialised DbRef —
    /// writing through it would crash the runtime.
    ///
    /// This helper detects that pattern and wraps the result in
    /// `Insert([v_set(v_nr, Null), OpDatabase(v_nr, cell_kt),
    /// result])` so the cell is allocated before the first
    /// write.  Marks `v_nr` as defined; subsequent writes hit
    /// the same OpSet directly without the prepend.
    ///
    /// CLOSURE-BODY case: when the LHS auto-deref's `inner` is
    /// NOT a `Var` (e.g. `get_field(closure, n_field)` for a
    /// captured boxed scalar), the helper is a no-op — the
    /// cell was already allocated by the parent's first-set,
    /// the closure record holds the shared DbRef, and the
    /// closure's write goes through that DbRef directly via the
    /// existing `Call(OpSet<T>, [get_field, Int(0), rhs])` IR.
    /// This is the missing-write-rewrite that 02d-iii.d delivers
    /// FOR FREE via 02d-iii.b's auto-deref + the existing
    /// `call_to_set_op` machinery.
    ///
    /// Dormant in production (02d-iii.a's flip is dormant — no
    /// variable carries `Reference(__cell_*, _)`).  Activates in
    /// 02d-iii.e.
    #[allow(
        dead_code,
        reason = "Helper invoked from tests; parse_assign_op hook activates with 02d-iii.e."
    )]
    pub(crate) fn maybe_prepend_cell_alloc(&mut self, result: Value, lhs: &Value) -> Value {
        let Some(v_nr) = self.extract_boxed_var_from_lhs(lhs) else {
            return result;
        };
        if !self.vars.exists(v_nr) {
            return result;
        }
        let tp = self.vars.tp(v_nr).clone();
        let Type::Reference(cell_d_nr, _) = &tp else {
            return result;
        };
        if !self.data.def(*cell_d_nr).name().starts_with("__cell_") {
            return result;
        }
        if self.vars.is_defined(v_nr) {
            // Subsequent: cell already exists, no alloc needed.
            return result;
        }
        // First-set: prepend cell allocation.
        let op_db = self.data.def_nr("OpDatabase");
        if op_db == u32::MAX {
            return result;
        }
        let cell_kt = i32::from(self.data.def(*cell_d_nr).known_type());
        self.vars.defined(v_nr);
        Value::Insert(vec![
            v_set(v_nr, Value::Null),
            Value::Call(op_db, vec![Value::Var(v_nr), Value::Int(cell_kt)]),
            result,
        ])
    }

    /// Plan-22 phase 02d-iii.c — build the rewrite IR for an
    /// assignment to a boxed-scalar local.  Returns `Some(IR)`
    /// when `var_nr`'s type is `Reference(__cell_<T>, _)` and
    /// the cell's value-field type is one of the supported
    /// primitives; `None` otherwise.
    ///
    /// Shapes:
    /// - **First assignment** (`!is_defined(var_nr)`):
    ///   `Insert([v_set(n, Null), OpDatabase(n, cell_kt),
    ///   OpSet<T>(Var(n), 0, rhs)])` — allocate a fresh cell
    ///   record + initialise the `value` field.  Mirrors the
    ///   pattern `parse_object` uses for struct literals
    ///   (objects.rs:1306-1361).
    /// - **Subsequent assignment** (`is_defined(var_nr)`):
    ///   `Call(OpSet<T>, [Var(n), Int(0), rhs])` — write the
    ///   `value` field of the existing cell.
    ///
    /// Op handling: only `=` is rewritten by this helper.
    /// Compound `+=` / `-=` / etc. need read + compute + write,
    /// which the parser already builds via
    /// `n = n + 1` lowering (the read side uses 02d-iii.b's
    /// auto-deref; the write hits this helper as a plain `=`).
    ///
    /// Boolean cells fall through to None — `OpSetByte` takes
    /// 4 args (ref, fld, min, val) instead of 3, and the
    /// boolean-to-byte conversion needs different IR.  Phase
    /// 02d-iii.e (or later) extends to boolean if needed.
    ///
    /// Dormant in production today (02d-iii.a's flip is dormant).
    /// 02d-iii.e activates the flip + this helper fires for real
    /// on every assignment to a boxed scalar.
    #[allow(
        dead_code,
        reason = "Helper invoked from tests; parse_assign_op hook activates with 02d-iii.e."
    )]
    pub(crate) fn boxed_scalar_assign_rewrite(
        &self,
        var_nr: u16,
        op: &str,
        rhs: Value,
    ) -> Option<Value> {
        if op != "=" || var_nr == u16::MAX || !self.vars.exists(var_nr) {
            return None;
        }
        let tp = self.vars.tp(var_nr).clone();
        let Type::Reference(cell_d_nr, _) = &tp else {
            return None;
        };
        if !self.data.def(*cell_d_nr).name().starts_with("__cell_") {
            return None;
        }
        let value_attr = self.data.def(*cell_d_nr).attributes().first()?;
        if value_attr.name != "value" {
            return None;
        }
        let value_tp = value_attr.typedef.clone();
        let op_set_name = match &value_tp {
            Type::Integer(_) => "OpSetInt",
            Type::Float => "OpSetFloat",
            Type::Single => "OpSetSingle",
            Type::Text(_) => "OpSetText",
            Type::Character => "OpSetCharacter",
            Type::Enum(_, false, _) => "OpSetEnum",
            // Boolean / exotic types: phase 02d-iii.e or later.
            _ => return None,
        };
        let op_set_d_nr = self.data.def_nr(op_set_name);
        if op_set_d_nr == u32::MAX {
            return None;
        }
        let op_db_d_nr = self.data.def_nr("OpDatabase");
        if op_db_d_nr == u32::MAX {
            return None;
        }
        let cell_kt = i32::from(self.data.def(*cell_d_nr).known_type());
        let pos = Value::Int(0);
        if self.vars.is_defined(var_nr) {
            // Subsequent: write value field of existing cell.
            Some(Value::Call(op_set_d_nr, vec![Value::Var(var_nr), pos, rhs]))
        } else {
            // First-set: allocate cell + fill value field.
            Some(Value::Insert(vec![
                v_set(var_nr, Value::Null),
                Value::Call(op_db_d_nr, vec![Value::Var(var_nr), Value::Int(cell_kt)]),
                Value::Call(op_set_d_nr, vec![Value::Var(var_nr), pos, rhs]),
            ]))
        }
    }

    /// Determine the variable number for an assignment target.
    /// For text `+=`, creates a unique temporary variable.
    pub(crate) fn assign_var_nr(
        &mut self,
        code: &mut Value,
        op: &str,
        f_type: &Type,
        parent_tp: &mut Type,
    ) -> u16 {
        if let Value::Var(v_nr) = *code {
            v_nr
        } else if op == "+=" && matches!(f_type, Type::Text(_)) {
            let v = self
                .vars
                .unique("field", &Type::Text(Deps::none()), &mut self.lexer);
            *code = Value::Var(v);
            *parent_tp = Type::Null;
            v
        } else {
            u16::MAX
        }
    }

    /// Handle assignment into a `RefVar(Text)` target; returns true if handled.
    pub(crate) fn assign_refvar_text(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        s_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        let Type::RefVar(t) = f_type else {
            return false;
        };
        if !matches!(**t, Type::Text(_)) {
            return false;
        }
        self.append_to_text(code, op, var_nr, s_type);
        true
    }

    /// Handle `v += expr` where `v: &vector<T>`; returns true if handled.
    /// NOTE: does NOT intercept `Value::Insert` — bracket-form `[elem]` literals are already
    /// handled by the Insert-expansion in `parse_block` → `OpFinishRecord`.
    pub(crate) fn assign_refvar_vector(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        let Type::RefVar(inner) = f_type else {
            return false;
        };
        let Type::Vector(elm_tp, _) = inner.as_ref() else {
            return false;
        };
        if op != "+=" {
            return false;
        }
        // Bracket-form [elem] and vector comprehensions produce Insert/Block; leave those
        // to the existing parse_block expansion path which uses OpFinishRecord.
        if matches!(code, Value::Insert(_) | Value::Block(_)) {
            return false;
        }
        if self.first_pass {
            return true;
        }
        // @P314 — narrow-aware element type (see `append_elem_tp`).
        let elm = (**elm_tp).clone();
        let rec_tp = self.append_elem_tp(&elm);
        *code = self.cl(
            "OpAppendVector",
            &[Value::Var(var_nr), code.clone(), Value::Int(rec_tp)],
        );
        true
    }

    pub(crate) fn validate_write(&mut self, to: &Value, parent_tp: &Type) {
        if let Value::Call(_, vars) = to.unspan()
            && vars.len() > 1
            && let Value::Int(pos) = vars[1].unspan()
        {
            let pos = *pos;
            let d_nr = self.data.type_def_nr(parent_tp);
            if d_nr != u32::MAX {
                let known = self.data.def(d_nr).known_type();
                if known != u16::MAX
                    && let Parts::Struct(fields) = &self.database.types[known as usize].parts
                {
                    for (f_nr, f) in fields.iter().enumerate() {
                        if f.position == pos as u16
                            && !self.data.def(d_nr).attributes()[f_nr].mutable
                        {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Cannot write to key field {}.{} create a record instead",
                                self.data.def(d_nr).name(),
                                f.name
                            );
                        }
                    }
                }
            }
        }
    }

    /// Materialise an iterator (e.g. `v[a..b]` slice) into a vector variable.
    /// Promotes the LHS variable to `Vector<elm_tp>` and builds a loop that appends
    /// each element in-place.
    fn materialize_iterator(
        &mut self,
        code: &mut Value,
        s_type: &Type,
        to: &Value,
        lhs_parent_tp: &Type,
        var_nr: u16,
        op: &str,
    ) {
        let Type::Iterator(elm_tp, _) = s_type.clone() else {
            unreachable!()
        };
        let elm_tp = *elm_tp;
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        self.change_var(to, &vec_tp);
        if !self.first_pass
            && let Value::Iter(_, init, next, _) = code.clone()
            && matches!(*next, Value::Block(_))
        {
            let ed_nr = self.data.type_def_nr(&elm_tp);
            let known_db = if ed_nr == u32::MAX || self.data.def(ed_nr).known_type() == u16::MAX {
                0
            } else {
                self.database.vector(self.data.def(ed_nr).known_type())
            };
            let known = Value::Int(i32::from(known_db));
            let fld = Value::Int(i32::from(u16::MAX));
            let elm_var = self.unique_elm_var(lhs_parent_tp, &elm_tp, var_nr);
            let for_var = self.create_unique("slice_elm", &elm_tp);
            let comp_var = self.create_unique("comp", &elm_tp);
            let for_next = v_set(for_var, *next);
            let mut lp = vec![for_next];
            lp.push(v_set(comp_var, Value::Var(for_var)));
            lp.push(v_set(
                elm_var,
                self.cl(
                    "OpNewRecord",
                    &[Value::Var(var_nr), known.clone(), fld.clone()],
                ),
            ));
            lp.push(self.set_field(
                ed_nr,
                usize::MAX,
                0,
                Value::Var(elm_var),
                Value::Var(comp_var),
            ));
            lp.push(self.cl(
                "OpFinishRecord",
                &[Value::Var(var_nr), Value::Var(elm_var), known, fld],
            ));
            let needs_db = self.vector_needs_db(var_nr, &elm_tp, true);
            let mut stmts = Vec::new();
            if op == "=" && !needs_db {
                stmts.push(self.cl("OpClearVector", &[Value::Var(var_nr)]));
            }
            stmts.push(*init);
            stmts.push(v_loop(lp, "Slice materialise"));
            if needs_db {
                let db = self.insert_new(var_nr, elm_var, &elm_tp, &mut stmts);
                self.vars.depend(var_nr, db);
            }
            *code = Value::Insert(stmts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inline_ref_set_in;
    use crate::data::{Block, Type, Value};

    /// Deep nesting must neither overflow the stack nor change the answer.
    #[test]
    fn inline_ref_set_in_deep_nesting_is_safe() {
        let mut v: Value = Value::Null;
        for _ in 0..1100 {
            v = Value::Block(Box::new(Block {
                name: "",
                operators: vec![v],
                result: Type::Void,
                scope: 0,
                var_size: 0,
            }));
        }
        assert!(!inline_ref_set_in(&v, 0), "no Set node anywhere");
        let mut w: Value = Value::Set(7, Box::new(Value::Null));
        for _ in 0..1100 {
            w = Value::Block(Box::new(Block {
                name: "",
                operators: vec![w],
                result: Type::Void,
                scope: 0,
                var_size: 0,
            }));
        }
        assert!(inline_ref_set_in(&w, 7), "deeply nested Set must be found");
    }
}
