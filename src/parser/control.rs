// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use crate::data::Deps;
use crate::data::IntegerSpec;
use std::collections::HashSet;

/// Which kind of return site `ref_return` is processing: a BODY-TAIL
/// site may NRVO-rename a local into the buffer attr (the local IS the
/// buffer for the whole fn), while a MID-BODY `return` is one site
/// among several — its named locals must never become arguments (the
/// 01b breakage), and its bare-call value needs the explicit
/// `Set + Var` shape or native loses it to the `Return(Null)`
/// fall-through (#356).
#[derive(PartialEq, Clone, Copy)]
pub(crate) enum RetSite {
    BlockTail,
    MidReturn,
}

/// @PLN85 / D-own-1 — how an implicit-tail `t == Vector` return delivers its
/// value into the fn's one `__retbuf` buffer. The SELECTOR
/// (`classify_vector_delivery`) reads the deps fact + tail shape once and picks a
/// variant; the dispatch (`dispatch_vector_delivery`) emits the matching
/// mechanism. This collapses the per-branch shape re-handling the vector arm of
/// `block_result` used to inline (OWNERSHIP_MODEL.md: ownership read once, not
/// re-derived per tail-shape at the delivery site).
enum Delivery {
    /// Promote the tail's work-ref(s) to BE `__retbuf` (no copy):
    /// `ref_return(ws) + nrvo_collapse_tail_set(ws)`. The owned-fresh / hidden-ref
    /// recovery (#120) / multi-arm (#437, cluster-V) case.
    Rename(Vec<u16>),
    /// The tail BORROWS a visible argument (the whole vector arg A.2, or a struct
    /// vector FIELD #415) — copy it into `__retbuf` for value semantics. `ls` is
    /// carried for the fallback rename if the copy's work-var allocation fails.
    CopyBorrow(Vec<u16>),
    /// A `#native`/`#rust` callee delivers its OWN store and never writes the
    /// `__retbuf` it was handed (#409) — mint a local, run the call into it, copy
    /// in. A no-op when there is no buffer or no work-var.
    ForwardCopy,
    /// Per-arm / fresh-local element COPY into `__retbuf` via
    /// `materialize_vector_arms_into`: a `match`/`if` branch tail (#416,
    /// cluster-II) OR a fresh-local tail whose buffer is already TAKEN by a sibling
    /// (#448). Finalises `returned` to `{__retbuf}` (idempotent when already set).
    Materialize,
    /// The tail already writes `__retbuf` (or there is no buffer / nothing to
    /// recover) — emit nothing here.
    AsIs,
}

/// @PLN85 D-own-1 — the delivery mechanism for a `Type::Reference` (struct) return,
/// the Reference counterpart of [`Delivery`]. Two mechanisms keyed on the deps fact:
/// rename the tail's work-ref(s) onto `__retbuf`, or materialise-copy a tail that
/// borrows a LOCAL (#306) before it escapes. (The nullable-unwrap tail is handled by
/// its own earlier `block_result` arm and is NOT routed here.)
enum RefDelivery {
    /// Promote the tail's work-ref(s) to BE `__retbuf` — `ref_return(ws) +
    /// nrvo_collapse_tail_set(ws)`. Covers #120 hidden-ref recovery (`ls` empty,
    /// `ws` = recovered) AND the plain arg-borrow/owned rename (`ws` = `ls`).
    Rename(Vec<u16>),
    /// The tail borrows a LOCAL's store (#306) — copy it into an owned work-ref
    /// via `materialize_view_return` before it escapes, then rename that.
    MaterializeView,
    /// `ls` empty and no work-ref to recover — the tail already delivers; emit nothing.
    AsIs,
}

use super::{
    DefType, I32, Level, LexItem, Parser, Position, Type, Value, diagnostic_format,
    merge_dependencies, v_block, v_if, v_loop, v_set,
};

/// Why an enclosing-scope capture inside a `parallel {}` arm is rejected.
/// See `parse_parallel` — each arm runs in an isolated worker (read-only heap
/// clone + private stack), so only *reading* an enclosing local is sound.
#[derive(Clone, Copy, PartialEq)]
enum ParViolation {
    /// Capturing a function parameter (read or write) — SIGSEGVs at teardown.
    Param,
    /// Writing or mutating an enclosing local — write is silently dropped
    /// (scalar/text) or crashes on the read-only store clone (heap).
    Mutation,
}

/// True for the IR ops that MUTATE their host — the var reachable from the
/// op's `args[0]` spine.  `Set(v, _)` is handled separately (the target var is
/// the node's first field); these are the in-place / element / field forms that
/// hide the host inside a read-projection chain.  (See the plan-57 capture
/// investigation for the exhaustive shape list.)
fn is_mutating_op(name: &str) -> bool {
    matches!(
        name,
        "OpSetInt"
            | "OpSetByte"
            | "OpSetShort"
            | "OpSetInt4"
            | "OpSetFloat"
            | "OpSetSingle"
            | "OpSetEnum"
            | "OpSetCharacter"
            | "OpSetText"
            | "OpSetKeyed"
            | "OpReplaceKeyed"
            | "OpClearKeyed"
            | "OpAppendVector"
            | "OpClearVector"
            | "OpNewRecord"
            | "OpFinishRecord"
            | "OpCopyRecord"
    )
}

/// A7.1: walk a body-tail expression and report whether it ends in
/// a literal `Value::Tuple(...)` at any reachable tail position.  Used
/// by `block_result` to decide whether the synthetic-struct rewrite
/// should fire.  Mirrors the recursion shape of
/// `rewrite_tail_tuple_with_work_ref` so the gate and the rewrite stay
/// in sync.
fn tail_has_tuple_leaf(value: &Value) -> bool {
    match value.tail() {
        Value::Tuple(_) => true,
        Value::If(_, then_branch, else_branch) => {
            tail_has_tuple_leaf(then_branch) || tail_has_tuple_leaf(else_branch)
        }
        _ => false,
    }
}

/// Check if the last meaningful expression in a block is divergent.
fn is_block_divergent(ops: &[Value]) -> bool {
    ops.iter().rev().any(|v| {
        matches!(
            v,
            Value::Return(_) | Value::Break(_) | Value::BreakWith(_, _) | Value::Continue(_)
        )
    })
}

/// Collected match arm data for enum/struct-enum match expressions.
struct EnumArm {
    /// Discriminants for this arm — Vec allows or-patterns (multiple variants per arm).
    discs: Vec<i32>,
    code: Value,
    tp: Type,
    guard: Option<Value>,
    bindings: Vec<Value>,
}

/// Returns true if the given AST value definitely returns on all code paths.
/// A block definitely-returns if its last statement is a `return`, or if it is
/// an `if` with an `else` where both branches definitely-return (recursive).
pub(crate) fn definitely_returns(val: &Value) -> bool {
    match val.tail() {
        Value::Return(_) => true,
        Value::If(_, t_branch, f_branch) => {
            // Both branches must definitely-return, and the else must not be null.
            !matches!(**f_branch, Value::Null)
                && definitely_returns(t_branch)
                && definitely_returns(f_branch)
        }
        _ => false,
    }
}

/// Match-arm type unification — strip `Type::RefVar(…)` wrappers before
/// delegating to `Type::is_same`.  Struct-enum pattern bindings yield a
/// `&T` borrow (e.g. `JString { value } => value` has type `&text`), while
/// sibling arms commonly return an owned `T` (`_ => ""`).  Requiring the
/// owned/borrow distinction to match exactly makes the straightforward
/// null-on-mismatch extractor pattern a compile error for no semantic
/// gain — the caller reads the value regardless of ownership.
fn match_arm_types_unify(a: &Type, b: &Type) -> bool {
    let strip = |t: &Type| -> Type {
        match t {
            Type::RefVar(inner) => (**inner).clone(),
            _ => t.clone(),
        }
    };
    strip(a).is_same(&strip(b))
}

impl Parser {
    /// Consume the `=>` separator that follows a match-arm pattern.
    ///
    /// If the user wrote `->` instead (a common slip — `->` is the lambda
    /// return-arrow and the historical TUPLES.md design draft used `->`
    /// for arms), emit a precise diagnostic and consume the wrong arrow
    /// so the arm-loop can continue and parse the body.  Without this
    /// recovery the surrounding loop spins on the unconsumed token —
    /// see PROBLEMS.md P206.
    fn expect_match_arm_arrow(&mut self) {
        // Trace point: match arm-arrow consumption.  Captures whether
        // the parser is looking at `->` (wrong), `=>` (right), or
        // something else (recover via `recover_to`).  Recurring
        // vantage during match-pattern debugging (P206, plan-18).
        // Enable with `LOFT_TRACE=match`.
        crate::loft_trace!(
            match_arm,
            "expect arrow: peek_arrow={} peek_eq={} first_pass={}",
            self.lexer.peek_token("->"),
            self.lexer.peek_token("=>"),
            self.first_pass,
        );
        if self.lexer.peek_token("->") {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "match arm separator is `=>`, not `->`"
                );
            }
            self.lexer.has_token("->");
        } else if !self.lexer.has_token("=>") {
            // P206 + plan-18: emit the missing-arrow diagnostic, then
            // recover to the next arm boundary.  Without recovery, a
            // malformed pattern like `x @ 1 | x @ 2 => …` (where the
            // or-pattern loop's `parse_match_pattern` doesn't consume
            // `x @ N`) leaves the lexer parked on an unexpected token;
            // the surrounding scalar/tuple/enum match loop then
            // re-enters pattern parsing on the same unconsumed token
            // and spins (PROBLEMS.md plan-18 phase 01 finding).
            //
            // `token("=>")` already emitted "Expect token =>"; here we
            // skip ahead until a `,`, `}`, or `;` so the outer loop
            // can pick up the next arm or exit cleanly instead of
            // looping forever.
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Expect token =>");
            }
            self.lexer.recover_to(&[",", "}", ";"]);
        }
    }

    // <block> ::= '}' | <expression> {';' <expression} '}'
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_block(&mut self, context: &str, val: &mut Value, result: &Type) -> Type {
        if let Value::Var(v) = val
            && let Type::Reference(r, _) = self.vars.tp(*v).clone()
            && context == "block"
        {
            // We actually scan a record here instead of a block of statements
            // — the LHS is a pre-typed struct variable and `{ ... }` is its
            // body.  Disambiguate between struct-body `{ field: val, ... }`
            // and block-expression `{ expr }` (e.g. `{ S { a: 3 } }`) by peeking
            // at the first two tokens after `{`:
            //   - `ident :`  — struct body (canonical form).
            //   - `ident ,`  — likely struct-body typo (missing colons); keep
            //                  the struct-body path so parse_object's failure
            //                  produces the historical "Expect token ;"
            //                  diagnostic at the first bare identifier (test
            //                  INC#30 locks this wording).
            //   - anything else (e.g. `ident =`, `ident {`, `[`, a literal) —
            //                  block expression; fall through.
            let link = self.lexer.link();
            self.lexer.token("{");
            let looks_like_struct_body = self.lexer.has_identifier().is_some()
                && ((self.lexer.peek_token(":") && !self.lexer.peek_token(":="))
                    || self.lexer.peek_token(","));
            self.lexer.revert(link);
            if looks_like_struct_body {
                self.parse_object(r, val);
                return Type::Reference(r, Deps::none());
            }
        }
        self.lexer.token("{");
        if self.lexer.has_token("}") {
            *val = v_block(Vec::new(), Type::Void, "empty block");
            return Type::Void;
        }
        let mut t = Type::Void;
        let mut l = Vec::new();
        let mut terminated: Option<&str> = None;
        // T1.7: track the start-position of the last expression for not-null diagnostics.
        let mut last_expr_peek = self.lexer.peek();
        loop {
            let line = self.lexer.pos().line;
            if line > self.line {
                if matches!(l.last(), Some(Value::Line(_))) {
                    l.pop();
                }
                l.push(Value::Line(line));
                self.line = line;
            }
            if self.lexer.has_token(";") {
                continue;
            }
            if self.lexer.peek_token("}") {
                break;
            }
            // detect file-scope-only declarations inside a block and
            // emit a single clean diagnostic instead of cascading parse
            // errors like "Expect token =" + "Expect constants to be in
            // upper case".  `fn` is special-cased because `fn(args) {...}`
            // is also a lambda expression — only reject `fn <name>(...)`.
            let bad_kw: Option<&'static str> = if self.lexer.peek_token("struct") {
                Some("struct")
            } else if self.lexer.peek_token("enum") {
                Some("enum")
            } else if self.lexer.peek_token("type") {
                Some("type")
            } else if self.lexer.peek_token("interface") {
                Some("interface")
            } else if self.lexer.peek_token("use") {
                Some("use")
            } else if self.lexer.peek_token("pub") {
                Some("pub")
            } else if self.lexer.peek_token("fn") {
                // distinguish `fn(args)` (lambda) from `fn name(args)`.
                let lexer_link = self.lexer.link();
                self.lexer.token("fn");
                let is_named_fn =
                    self.lexer.peek().has != crate::lexer::LexItem::Token("(".to_string());
                self.lexer.revert(lexer_link);
                if is_named_fn { Some("fn") } else { None }
            } else {
                None
            };
            if let Some(kw) = bad_kw {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{kw}' definitions must be at file scope, not inside a function or block"
                    );
                }
                // Consume the offending declaration: skip until the matching
                // top-level `}` or the outer block's `}`/`;`.
                self.lexer.token(kw);
                let mut depth: i32 = 0;
                while depth >= 0 {
                    if self.lexer.has_token("{") {
                        depth += 1;
                    } else if self.lexer.peek_token("}") {
                        if depth == 0 {
                            break;
                        }
                        self.lexer.token("}");
                        depth -= 1;
                    } else if self.lexer.peek().has == crate::lexer::LexItem::None {
                        break;
                    } else {
                        self.lexer.cont();
                    }
                }
                self.lexer.has_token(";");
                continue;
            }
            // Warn about unreachable code after an unconditional terminator.
            if let Some(kind) = terminated {
                if !self.first_pass {
                    diagnostic!(self.lexer, Level::Warning, "Unreachable code after {kind}");
                }
                // Only warn once per terminator
                terminated = None;
            }
            let mut n = Value::Null;
            last_expr_peek = self.lexer.peek();
            // @PLN22 Phase 1 — hint the block's expected enum so a bare
            // value-position variant tail (`fn f() -> Light { Red }`, or an
            // `if c { Red } else { Green }` block) resolves against it.  SAVE and
            // RESTORE the prior hint rather than clearing to Unknown, so sibling
            // statements / if-branches under the same expected type each still see
            // it (clearing made only the FIRST branch of an `if`-return resolve).
            let saved_expected = self.expected.clone();
            if self.enum_context(result) {
                self.expected = result.clone();
            }
            t = self.expression(&mut n);
            self.expected = saved_expected;
            // Track unconditional terminators at block scope.
            // if/else/loop/match contain terminators inside branches — not unconditional.
            match &n {
                Value::Return(_) => terminated = Some("return"),
                Value::Break(_) | Value::BreakWith(_, _) => terminated = Some("break"),
                Value::Continue(_) => terminated = Some("continue"),
                _ => {}
            }
            if let Value::Insert(ls) = n {
                Self::move_insert_elements(&mut l, ls);
                // preserve `Type::Rewritten(_)` when flattening an
                // Insert.  A first-pass `parse_object` struct literal
                // returns `Type::Rewritten(Type::Reference(_))` together
                // with a Value::Insert body that has no terminating
                // Var — the Rewritten tag is the only signal that a
                // value of that type is produced.  Blindly resetting
                // `t = Void` here caused `x = { S { a: 3 } }` to infer
                // `x: void` in first_pass, which then tripped the
                // "cannot change type from void to S" diagnostic in
                // second_pass when the real Reference(S) type arrived.
                if !matches!(t, Type::Rewritten(_)) {
                    t = Type::Void;
                }
            } else if !matches!(t, Type::Void | Type::Never)
                && (self.lexer.peek_token(";") || *result == Type::Void)
            {
                l.push(Value::Drop(Box::new(n)));
            } else {
                l.push(n);
            }
            if self.lexer.peek_token("}") {
                break;
            }
            // Preserve Never for blocks that end with return/break/continue.
            if !matches!(t, Type::Never) {
                t = Type::Void;
            }
            match l.last() {
                Some(
                    Value::If(_, _, _) | Value::Loop(_) | Value::Block(_) | Value::Parallel(_),
                ) => (),
                _ => {
                    if !self.lexer.token(";") {
                        // L1: recover to the next statement boundary or the
                        // block end so a missing `;` doesn't cascade into
                        // "Expect token }", "Expect constants to be in upper
                        // case style", etc. on the following lines.
                        if self.lexer.recover_to(&[";", "}"]) {
                            self.lexer.has_token(";");
                            continue;
                        }
                        break;
                    }
                }
            }
        }
        self.lexer.token("}");
        if matches!(l.last(), Some(Value::Line(_))) {
            l.pop();
        }
        if matches!(t, Type::RefVar(_)) {
            let mut code = l.pop().unwrap().clone();
            self.un_ref(&mut t, &mut code);
            l.push(code);
        }
        // T1.7: check for null assigned to `integer not null` tuple elements in the
        // last expression of the block (the implicit return value).
        // After emitting the error, update the type to remove Null elements so that
        // type-conversion validation does not produce a redundant type-mismatch error.
        if !self.first_pass
            && !l.is_empty()
            && let Type::Tuple(expected) = result
            && let Type::Tuple(t_elems) = &t
        {
            let expected = expected.clone();
            let t_elems = t_elems.clone();
            let mut fixed = false;
            let new_elems: Vec<Type> = t_elems
                .iter()
                .zip(expected.iter())
                .map(|(te, ex)| {
                    if matches!(te, Type::Null)
                        && matches!(ex, Type::Integer(IntegerSpec { not_null: true, .. }))
                    {
                        fixed = true;
                        ex.clone()
                    } else {
                        te.clone()
                    }
                })
                .collect();
            if fixed && let Some(Value::Tuple(elems)) = l.last_mut() {
                let expected = expected.clone();
                for (elem_val, elem_tp) in elems.iter_mut().zip(expected.iter()) {
                    if matches!(elem_val, Value::Null)
                        && matches!(elem_tp, Type::Integer(IntegerSpec { not_null: true, .. }))
                    {
                        specific!(
                            &mut self.lexer,
                            &last_expr_peek,
                            Level::Error,
                            "cannot assign null to 'integer not null' element"
                        );
                        *elem_val = Value::Call(self.data.def_nr("OpConvIntFromNull"), vec![]);
                    }
                }
                t = Type::Tuple(new_elems);
            }
        }
        // Plan-07 phase 4d.2 — defensive-check flow-analysis.  When
        // a fault-prone op's result is assigned to a variable AND
        // the immediately-following sibling is an `if` whose
        // condition mentions that variable, the user has written
        // defensive code (`if x != null { … }`, `if x { … }`,
        // `if x > 10 { … }`, etc.) — swap the source op to its
        // Nullable peer at COMPILE TIME so neither runtime path
        // (production log + continue, OR development halt + render)
        // fires.  Both modes get the same silent-sentinel behaviour
        // because the Nullable peer never calls `s.raise`.
        //
        // Both `if x != null` and bare `if x` (truthy check) are
        // accepted; loft's `if x` lowers to a Reference→Boolean
        // conversion that's `false` for null DbRef / 0 / null int —
        // exactly the defensive shape we want to honor.  An over-
        // broad test like `if x > 10` also counts: any mention of
        // `Var(x)` in the if condition signals defensive intent.
        //
        // Single-block, single-step lookahead: covers the canonical
        // pattern.  Cross-function defenses or many-statement gaps
        // fall through to the raising peer + log path; phase 4e's
        // compile-time warning will nudge those toward the
        // recognised defenses.
        if !self.first_pass {
            self.rewrite_defended_fault_sites(&mut l);
        }
        t = self.block_result(context, result, &t, &mut l, &last_expr_peek.position);
        *val = v_block(l, t.clone(), "block");
        t
    }

    /// Plan-07 phase 4d.2 — walks `ops` looking for adjacent
    /// `Set(x, fault_op); If(test_using_x, …)` pairs and swaps the
    /// fault op's def_nr to its Nullable peer.  See the call site
    /// in `parse_block` for the rationale.  The swap suppresses
    /// BOTH the production-mode log entry AND the development-mode
    /// halt, because the Nullable peers never call `s.raise`.
    pub(crate) fn rewrite_defended_fault_sites(&self, ops: &mut [Value]) {
        // Two-pass to avoid borrow conflicts.  First pass: collect
        // the indices of statements that need rewriting.  Second
        // pass: apply rewrites.
        let mut to_rewrite: Vec<usize> = Vec::new();
        for i in 0..ops.len() {
            let Value::Set(var, _) = ops[i].unspan() else {
                continue;
            };
            let var = *var;
            // Look ahead for the next non-Line sibling.
            let mut j = i + 1;
            while j < ops.len() && matches!(ops[j].unspan(), Value::Line(_)) {
                j += 1;
            }
            if j >= ops.len() {
                continue;
            }
            // Sibling must be an `if` whose test mentions Var(x).
            let Value::If(test, _, _) = ops[j].unspan() else {
                continue;
            };
            if !test.reads_var(var) {
                continue;
            }
            to_rewrite.push(i);
        }
        for i in to_rewrite {
            if let Value::Set(_, source) = ops[i].unspan_mut() {
                Self::rewrite_outer_to_nullable(source, &self.data);
            }
        }
    }

    /// Helper for `rewrite_defended_fault_sites` — same shape as
    /// `parser/operators.rs::rewrite_outer_arith_to_nullable` but
    /// recurses one level into wrapping getters
    /// (`OpGetInt` / `OpGetByte` / `OpGetShortRaw` / `OpGetInt4`)
    /// so integer-vector indexing's
    /// `OpGetInt(OpGetVector(…), 0)` shape is also handled.
    fn rewrite_outer_to_nullable(code: &mut Value, data: &crate::data::Data) {
        fn try_swap(def_nr: &mut u32, data: &crate::data::Data) -> bool {
            let name = data.def(*def_nr).original_name();
            let nullable = match name.as_str() {
                "AddInt" => "OpAddIntNullable",
                "MinInt" => "OpMinIntNullable",
                "MulInt" => "OpMulIntNullable",
                "DivInt" => "OpDivIntNullable",
                "RemInt" => "OpRemIntNullable",
                "GetVector" => "OpGetVectorNullable",
                "VectorRef" => "OpVectorRefNullable",
                "TextCharacter" => "OpTextCharacterNullable",
                // Plan-07 phase 4f.5 — float / single div / mod peers.
                "DivFloat" => "OpDivFloatNullable",
                "RemFloat" => "OpRemFloatNullable",
                "DivSingle" => "OpDivSingleNullable",
                "RemSingle" => "OpRemSingleNullable",
                _ => return false,
            };
            let new_nr = data.def_nr(nullable);
            if new_nr == u32::MAX {
                false
            } else {
                *def_nr = new_nr;
                true
            }
        }
        let Value::Call(def_nr, args) = code.unspan_mut() else {
            return;
        };
        if try_swap(def_nr, data) {
            return;
        }
        let outer_name = data.def(*def_nr).original_name();
        if matches!(
            outer_name.as_str(),
            "GetInt" | "GetInt4" | "GetByte" | "GetShortRaw"
        ) && let Some(first_arg) = args.first_mut()
            && let Value::Call(inner_nr, _) = first_arg.unspan_mut()
        {
            try_swap(inner_nr, data);
        }
    }

    pub(crate) fn un_ref(&mut self, t: &mut Type, code: &mut Value) {
        if let Type::RefVar(tp) = t.clone() {
            self.convert(code, t, &tp);
            *t = *tp;
            for on in t.depend() {
                *t = t.depending(on);
            }
        }
    }

    pub(crate) fn move_insert_elements(l: &mut Vec<Value>, elms: Vec<Value>) {
        for el in elms {
            if let Value::Insert(ls) = el {
                Self::move_insert_elements(l, ls);
            } else {
                l.push(el);
            }
        }
    }

    pub(crate) fn block_result(
        &mut self,
        context: &str,
        result: &Type,
        t: &Type,
        l: &mut [Value],
        tail_pos: &Position,
    ) -> Type {
        let mut tp = t.clone();
        // #416 — set when the vector match/if tail below was materialised into the
        // return buffer; gates the type-keyed vector arm (which is reached only in
        // the IMPLICIT-tail `t = Vector` case) so it doesn't re-process / re-promote
        // an arm buffer the materialise already delivered.
        let mut vec_arm_handled = false;
        if *result != Type::Void && !matches!(*result, Type::Unknown(_)) {
            // An empty block (e.g. an empty comprehension body `[for i in r {}]`) has no
            // tail to convert/deliver; without this guard `l.len() - 1` underflows to
            // usize::MAX and the index below panics.  Leave the empty block to downstream
            // type-checking (which reports the real "expected <T>, produced nothing").
            if l.is_empty() {
                return tp;
            }
            let last = l.len() - 1;
            // CO1.3c: generator bodies return void (values come from yield),
            // so suppress the void-vs-iterator mismatch.
            let is_generator = matches!(result, Type::Iterator(_, _));
            let ignore = is_generator
                || (matches!(*t, Type::Void | Type::Never)
                    && (matches!(l[last], Value::Return(_)) || definitely_returns(&l[last])));
            // Plan-14 phase 07 (P234 runtime): when the function's expected
            // return type is `Reference(__tuple<…>)` (rewritten in
            // `parse_function` for any tuple whose elements have lifetime
            // concerns) AND the body's tail expression is a literal
            // `Value::Tuple(elements)`, transform the tail into synthetic-
            // struct construction so the existing struct-return machinery
            // applies.  Without this rewrite, `convert` would fail
            // (Tuple is not assignable to Reference(__tuple<…>)) and the
            // user would see a confusing "expected __tuple<…>, got tuple([…])"
            // diagnostic.
            // A7.1: gate broadened to also fire for `If` / `Block` /
            // `Insert` tails — the recursive helper descends through
            // these wrappers and rewrites every leaf `Value::Tuple` that
            // lives at a tail position with a synthetic-struct
            // construction sharing one work-ref.  Without this, function
            // bodies whose final expression is `if cond { (a, b) } else
            // { (c, d) }` left two tuple leaves and convert would then
            // fail with Tuple → Reference(__tuple<…>).
            let tuple_rewritten = !self.first_pass
                && context == "return from block"
                && matches!(t, Type::Tuple(_))
                && tail_has_tuple_leaf(l[last].unspan())
                && matches!(result, Type::Reference(d, _) if self.data.def(*d).name().starts_with("__tuple<"))
                && {
                    let synthetic_d_nr = if let Type::Reference(d, _) = result {
                        *d
                    } else {
                        unreachable!()
                    };
                    self.rewrite_tail_tuple_to_synthetic_struct(synthetic_d_nr, &mut l[last]);
                    true
                };
            // P236: when the body's tail is a `Value::If(...)` (or
            // `match`, which lowers to nested `If`) and the function
            // returns a heap-owned reference, unify the branches'
            // work-refs so all paths share one return slot.  Without
            // this, native codegen drops the if/else's value and
            // returns the typed null sentinel.  See `unify_if_branches_work_refs`
            // for the full rationale.  Wrap in `Value::Return(...)` so
            // the existing `Return(If(...))` native codegen at
            // `src/generation/emit.rs:166-182` emits
            // `return if cond { ... } else { ... }` correctly.
            let if_unified = !self.first_pass
                && context == "return from block"
                && matches!(
                    result,
                    Type::Reference(_, _)
                        | Type::Vector(_, _)
                        | Type::Enum(_, true, _)
                        | Type::Sorted(_, _, _)
                        | Type::Hash(_, _, _)
                        | Type::Index(_, _, _)
                        | Type::Spacial(_, _, _)
                )
                && matches!(l[last].unspan(), Value::If(_, _, _))
                && self.unify_if_branches_work_refs(&mut l[last]).is_some();
            if if_unified {
                let inner = std::mem::replace(&mut l[last], Value::Null);
                l[last] = Value::Return(Box::new(inner));
            }
            // @PLN85 cluster II — a VECTOR-returning fn whose tail is a `match`/`if`
            // with per-arm LOCAL buffers (arms are `_vec_N`, not the `__ref_N`
            // work-refs `if_unified` shares; the match types as `Never`, so the
            // type-keyed vector arm below — keyed on `t` — is skipped). Without
            // NRVO the result is delivered via a fresh local while the caller's
            // eagerly-allocated `__retbuf` work-ref store is orphaned and LEAKS on
            // the interpreter (Edge B / `init_ref`). Deliver per-arm into `__retbuf`.
            // Fires for an explicit `return match` (t = Never) AND an implicit
            // `{ match }` block tail (t = Vector — #416). `tail_terminal_is_branch`
            // keeps it to match/if tails. `tail_if_has_null_arm` EXCLUDES a nullable
            // return (`{ if b { [..] } else { null } }`): materialising it would set
            // `returned = Vector[__retbuf]` while a reachable arm yields null, which
            // native cannot represent. enc's exhaustive-match default-null is nested
            // and unreachable, so it is not a direct arm-null and still materialises.
            let vec_match_candidate = !tuple_rewritten
                && !if_unified
                && !self.first_pass
                && context == "return from block"
                && matches!(result, Type::Vector(_, _))
                && matches!(t, Type::Never | Type::Void | Type::Vector(_, _))
                && Self::tail_terminal_is_branch(&l[last])
                && !self.tail_if_has_null_arm(&l[last]);
            if vec_match_candidate && let Type::Vector(elm, _) = result {
                // #416 — a match/if branch tail materialises each arm into __retbuf.
                // Routed through the ONE vector dispatch (Delivery::Materialize); it
                // gates convert via vec_arm_handled on whether a rewritable arm was
                // found (no buffer / no terminal → false, convert runs as before).
                let elm_ty = (**elm).clone();
                vec_arm_handled = self.dispatch_vector_delivery(Delivery::Materialize, &elm_ty, l);
            }
            // (#448, the early-`return <call>` + tail-`return [literal]` shape, was a
            // SECOND upper materialise block here. It is now a CELL of the tail-return
            // handling below — `Delivery::Materialize` when the buffer is already
            // TAKEN by a sibling return — so it shares one fresh-owned-vector classifier
            // (`fresh_owned_vector_deps`) and one dispatch with the buffer-free #437/c5
            // rename. See the `tail_ret_owned` block.)
            if !tuple_rewritten
                && !if_unified
                && !vec_match_candidate
                && !vec_arm_handled
                && !self.convert(&mut l[last], t, result)
                && !ignore
            {
                // for function bodies with `not null` return, downgrade to a warning.
                if context == "return from block"
                    && self.context != u32::MAX
                    && self.data.definitions[self.context as usize].returned_not_null
                {
                    if !self.first_pass {
                        let fn_name = self.data.definitions[self.context as usize].original_name();
                        diagnostic!(
                            self.lexer,
                            Level::Warning,
                            "Not all code paths return a value — function '{fn_name}' may return null",
                        );
                    }
                } else {
                    self.validate_convert(context, t, result, tail_pos);
                }
            }
            tp = result.clone();
        }
        // I9-var: skip ref_return/text_return for generic templates.
        // The return type T = Reference(tv_nr) triggers ref_return which promotes local
        // variables to hidden parameters.  After specialization to a value type (Integer,
        // Float), those hidden params are wrong.  Specialized copies inherit the template's
        // body and variable table; struct-returning specializations work correctly because
        // they return arguments (not locals), so ref_return would be a no-op anyway.
        //
        // a7: this block PROMOTES a body-tail local to the function's hidden return
        // buffer (`ref_return` renames `__retbuf` to the local; `text_return` likewise).
        // That is only sound at the GENUINE function tail — `parse_code` parses it with
        // context "return from block". An `if`/`match` ARM (context "if"/"else"/
        // "match_arm") is NOT the function tail: promoting an arm's own `__vdb_N` makes
        // that arm both build into AND free the shared return buffer, so its value is lost
        // (interp reads the sibling arm, native reads empty — the two backends diverge).
        // The fn-body tail then delivers every arm into `__retbuf` (the `match` path,
        // `materialize_vector_arms_into`), so gating the promotion to the real return
        // context lets the `if` arms behave exactly like `match` arms already do.
        if self.data.def_type(self.context) != DefType::Generic && context == "return from block" {
            // @PLN25 single-payload: the tail was just coerced `__nullable<S>` → dense `S`
            // via a payload sub-ref (`OpGetField`), so `t` is still the Enum tail type and
            // the type-keyed branches below (which match `t`) all miss it — the default
            // epilogue then demotes the unwrap to a discarded statement + `return null`
            // (native returns the null sentinel).  Key off the dense return type `result`
            // instead: materialise the unwrap tail into an owned work-ref (copy the viewed
            // `S`) and promote that — the #306 view-return shape.  Gate-off-inert.
            // #437 + c5/#448 residual — a TAIL explicit `return <fresh-owned vector>`
            // is semantically identical to the implicit tail `<expr>`, but
            // `parse_return` left it as a Never-typed `Value::Return(<expr>)`, so the
            // implicit-tail vector arm below (gated on `t == Vector`) never delivered
            // it: the signature stayed a BARE vector. A direct caller copes (it owns
            // the result), but an NRVO caller that CHAINS this return into its buffer
            // (`return wrap()` → `__retbuf = wrap(__retbuf)`) orphans the fresh store
            // wrap never wrote into __retbuf (#448 c5). `<expr>` is either a named
            // non-arg local (#437) OR a fresh literal / comprehension whose block owns
            // a `__vdb` store (the c5 residual). Strip the `return` → implicit tail and
            // route through the SAME ref_return + NRVO (renames its store onto
            // __retbuf, no copy); ref_return then delivers any sibling mid-body returns
            // via deliver_mid_vector_returns. A mid-body `if { return e }` is in
            // "if"/"match_arm" context, never "return from block", so it is untouched.
            // `!vec_arm_handled` — when the upper match/if (#416) or #448 path
            // already materialised this tail into __retbuf, its delivered block's
            // RESULT TYPE still reads the original `["__vdb"]` (the inner build),
            // so without this gate `fresh_owned_vector_deps` is fooled and delivers
            // it a SECOND time (appending __retbuf into itself → doubled length).
            let tail_ret_owned: Option<Vec<u16>> = if !self.first_pass
                && !vec_arm_handled
                && matches!(result, Type::Vector(_, _))
                && let Some(Value::Return(inner)) = l.last().map(Value::unspan)
            {
                self.fresh_owned_vector_deps(inner)
            } else {
                None
            };
            if let Some(ls) = tail_ret_owned {
                let last = l.len() - 1;
                // #448 (now a CELL, not a separate upper block) — when the buffer is
                // already TAKEN by a sibling return that delivers __retbuf (an early
                // `return <call>` NRVO-adopted it), RENAMING this fresh-owned tail onto
                // __retbuf would double-own the buffer, so COPY it in via the ONE vector
                // dispatch (Delivery::Materialize: clear + append + free the local; the
                // `returned` re-set to {__retbuf} is idempotent — returned_uses_buffer
                // checked it is already there). The buffer-FREE case RENAMES (#437/c5).
                // One fresh-owned-vector classifier (`fresh_owned_vector_deps`), the deps
                // fact deciding rename-vs-copy.
                let mut delivered = false;
                if !Self::tail_terminal_is_branch(&l[last])
                    && let Type::Vector(elm, _) = result
                    && let Some((buf_attr, buf_var)) = self.return_buffer()
                    && self.returned_uses_buffer(buf_attr)
                    && Self::body_has_buffer_return(&l[..last], buf_var)
                {
                    let elm_ty = (**elm).clone();
                    delivered = self.dispatch_vector_delivery(Delivery::Materialize, &elm_ty, l);
                }
                if !delivered {
                    // buffer FREE (or the copy did not fire) → strip the `return` →
                    // implicit tail (peel any Span, then the Return, keeping the owned
                    // expr — a bare Var #437 or the literal block) and RENAME its store
                    // onto __retbuf via ref_return + NRVO.
                    let mut taken = std::mem::replace(&mut l[last], Value::Null);
                    loop {
                        taken = match taken {
                            Value::Span(b) => b.1,
                            Value::Return(inner) => {
                                l[last] = *inner;
                                break;
                            }
                            other => {
                                l[last] = other;
                                break;
                            }
                        };
                    }
                    self.ref_return(&ls, l, RetSite::BlockTail);
                    self.nrvo_collapse_tail_set(l, &ls);
                }
            } else if let Type::Reference(td, _) = result
                && !l.is_empty()
                && self.tail_is_nullable_unwrap(&l[l.len() - 1])
            {
                let last = l.len() - 1;
                let w = self.materialize_view_return(*td, &mut l[last]);
                self.ref_return(&[w], l, RetSite::BlockTail);
                self.nrvo_collapse_tail_set(l, &[w]);
            } else if let Type::Text(ls) = t {
                self.text_return(ls);
            } else if !vec_arm_handled && let Type::Vector(elm, ls) = t {
                // @PLN85 / D-own-1 — classify ONCE from the deps fact + tail shape,
                // then emit. The three old inline branches (recover-hidden-refs /
                // arg-borrow-copy / multi-arm-rename) are now cells of one selector.
                let delivery = self.classify_vector_delivery(ls, l, context);
                let elm_ty = (**elm).clone();
                self.dispatch_vector_delivery(delivery, &elm_ty, l);
            } else if let Type::Reference(td, ls) = t {
                // @PLN85 D-own-1 — Reference return sub-thicket: classify ONCE from
                // the deps fact + tail shape, then dispatch to the ONE mechanism
                // (rename via ref_return, or materialise-copy a borrowed-local view).
                // Mirrors the vector `classify_vector_delivery` collapse.
                let td = *td;
                let delivery = self.classify_reference_delivery(ls, l);
                self.dispatch_reference_delivery(delivery, td, l);
            } else if let Type::Vector(elm, _) = result
                && let Some((buf_attr, buf_var)) = self.return_buffer()
                && self.returned_uses_buffer(buf_attr)
            {
                // #448 mirror — the fn is buffer-bound but NONE of the cells above
                // handled this tail: it became buffer-bound via a tail `return <call>`
                // chain (parse_return sets that up as a MidReturn, which — unlike a tail
                // rename — never triggers deliver_mid_vector_returns). A mid-body
                // fresh-owned return (an early `return [literal]`) was deferred by
                // parse_return and would orphan its store on that path. Deliver every
                // mid-body return into __retbuf now that the binding is final. Cells
                // that DID handle the tail short-circuit this arm (their ref_return
                // already delivered the mid-body), so this never double-delivers.
                let elm_ty = (**elm).clone();
                self.deliver_mid_vector_returns(&elm_ty, l, buf_var);
            }
        }
        tp
    }

    /// @PLN85 / D-own-1 — the SELECTOR for an implicit-tail `t == Vector` return:
    /// read the deps fact `ls` and the tail shape ONCE and pick a [`Delivery`].
    /// Pure (`&self`) so classification and emission stay separable. Replaces the
    /// three inline branches the vector arm of `block_result` used to carry.
    fn classify_vector_delivery(&self, ls: &[u16], l: &[Value], context: &str) -> Delivery {
        if ls.is_empty() && !l.is_empty() {
            // Issue #120 mirror (see the Reference arm): when filter_hidden
            // stripped the deps, recover the tail call's work refs so the site
            // still binds to the one buffer.
            let last = &l[l.len() - 1];
            let extra = Self::collect_hidden_ref_args(last, &self.data);
            // Chain the wrapper into its callee's buffer — UNLESS the callee
            // forwards a foreign store and never writes that buffer
            // (`fn f() -> vector { stack_trace() }`): chaining a grand-caller's
            // buffer through such a forwarder orphans it (#355 follow-up leak,
            // 55-stack-trace). The forwarder test reads the callee's BODY shape,
            // which is pass-stable (unlike its `returned` deps).
            let callee_forwards = matches!(last.unspan(), Value::Call(d, _)
                if self.callee_forwards_foreign_store(*d));
            // #409: a NATIVE / `#rust` decl with a heap return delivers its OWN
            // store and never writes the `__retbuf` it was handed — route the
            // result through a fresh local and COPY into `__retbuf` (ForwardCopy).
            // Such a callee is PASS-STABLE (`code==Null` + a symbol set, identical
            // in both parse passes), so minting a local here is safe. Copying (not
            // chaining) keeps the #355 orphan impossible.
            let native_forwarder = matches!(last.unspan(), Value::Call(d, _) if {
                let cd = self.data.def(*d);
                *cd.code() == Value::Null
                    && (!cd.native().is_empty() || !cd.rust().is_empty())
                    && matches!(
                        cd.returned(),
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
            });
            if !extra.is_empty() && !callee_forwards {
                Delivery::Rename(extra)
            } else if native_forwarder {
                Delivery::ForwardCopy
            } else {
                Delivery::AsIs
            }
        } else if !self.first_pass
            && ls.iter().any(|&d| self.vars.is_argument(d))
            && (self.tail_is_struct_field_read(l)
                // The whole-arg copy (a2) fires ONLY at the function-body tail: an
                // `if`/`match` ARM block also reaches block_result (context "if"/
                // "else"/"match_arm") with a `{ v }` tail, but the arm is already
                // delivered into the buffer by the outer if-unify / arm-materialise
                // path — copying it again here orphans the buffer (a11 leak).
                // `return from block` is the one funnelled return path (row 104).
                || (context == "return from block"
                    && self.tail_whole_arg_vector(l).is_some())
                // @PLN85 P14 — a borrowed match-arm binding (or local) returned
                // directly borrows a visible arg; copy it into __retbuf rather
                // than rename it onto (and alias) the caller's buffer.
                || (context == "return from block" && self.tail_borrows_arg(l)))
            && self
                .return_buffer()
                .is_some_and(|(_, buf_var)| !ls.contains(&buf_var))
        {
            // Row-104 — an implicit-tail return whose value BORROWS a visible
            // argument: a STRUCT vector FIELD of an arg (`fn getv(b: Box) ->
            // vector { b.v }`, #415) OR the whole vector arg itself
            // (`fn idv(v) -> vector { v }`, A.2/a2). Returning the tail as-is
            // ALIASES the caller's store, so copy it into `__retbuf` (value
            // semantics). The EXPLICIT `return v` / `return b.v` already does this
            // (parse_return), suite-proven. Narrowed by the two tail predicates to
            // whole-arg / struct-field tails: index / call tails stay on the rename
            // path (the over-broad cut regressed the suite). `ls` is carried for
            // the alloc-failure fallback rename.
            Delivery::CopyBorrow(ls.to_vec())
        } else {
            // #437/@PLN85 cluster V (O-Move): a multi-arm `match`/`if` vector tail
            // must deliver EVERY arm's buffer into the one return buffer. The
            // Vector type dep `ls` can be INCOMPLETE — it carries only the first
            // arm's `__ref_1`, while a later arm's `__ref_N` is allocated but
            // unregistered, so scope analysis frees it and the function returns a
            // dangling ref into a freed store. Union `ls` with every hidden
            // buffer-arg ref in the tail so ref_return renames each arm's buffer
            // onto the retbuf (the pre-#437 [__ref_1, __ref_2] shape).
            let mut full: Vec<u16> = ls.to_vec();
            if let Some(last) = l.last() {
                for w in Self::collect_hidden_ref_args(last, &self.data) {
                    if !full.contains(&w) {
                        full.push(w);
                    }
                }
            }
            Delivery::Rename(full)
        }
    }

    /// @PLN85 / D-own-1 — emit the mechanism the selector chose for a vector
    /// return. `elm` is the vector element type (for the element-copy ops); the
    /// tail of `l` is rewritten in place. Returns whether a `Materialize` actually
    /// delivered (the upper #416 / #448 callers gate `vec_arm_handled` / a fallback
    /// rename on it); the other variants always handle their tail.
    fn dispatch_vector_delivery(
        &mut self,
        delivery: Delivery,
        elm: &Type,
        l: &mut [Value],
    ) -> bool {
        match delivery {
            Delivery::Rename(ws) => {
                self.ref_return(&ws, l, RetSite::BlockTail);
                // @P377 / S1: collapse `cv = inner_call(...); cv` so the inner
                // call's hidden buffer arg points at cv directly.
                self.nrvo_collapse_tail_set(l, &ws);
                true
            }
            Delivery::CopyBorrow(ls) => {
                // The buffer's existence was verified by the selector; re-fetch it
                // for the copy (idempotent — nothing mutated in between). Fall back
                // to the rename path if the copy's work-var allocation fails.
                if let Some((buf_attr, buf_var)) = self.return_buffer()
                    && !self.copy_borrow_tail_into_retbuf(elm, l, buf_attr, buf_var)
                {
                    self.ref_return(&ls, l, RetSite::BlockTail);
                    self.nrvo_collapse_tail_set(l, &ls);
                }
                true
            }
            Delivery::ForwardCopy => {
                self.emit_forward_copy_409(elm, l);
                true
            }
            Delivery::Materialize => {
                // The per-arm / fresh-local element copy (#416 branch tails, #448
                // buffer-taken tails). Materialise each arm/tail into __retbuf and
                // finalise the return-type dep to {__retbuf} — the step #416 always
                // did and #448 relied on (returned already `["__retbuf"]`, so the
                // set is idempotent). Returns false when there is no buffer or the
                // materialiser found no rewritable terminal, so the caller can fall
                // back (#448) or leave convert to run (#416).
                let last = l.len() - 1;
                if let Some((buf_attr, buf_var)) = self.return_buffer()
                    && self.materialize_vector_arms_into(elm, &mut l[last], buf_var)
                {
                    self.data.definitions[self.context as usize].returned =
                        Type::Vector(Box::new(elm.clone()), Deps::attrs(vec![buf_attr]));
                    true
                } else {
                    false
                }
            }
            Delivery::AsIs => false,
        }
    }

    /// @PLN85 D-own-1 — the SELECTOR for a `Type::Reference` (struct) return: read
    /// the deps fact `ls` + the tail shape ONCE and pick a [`RefDelivery`]. Pure
    /// (`&self`). Replaces the three inline branches the Reference arm of
    /// `block_result` carried; mirrors `classify_vector_delivery`.
    fn classify_reference_delivery(&self, ls: &[u16], l: &[Value]) -> RefDelivery {
        if ls.is_empty() {
            // Issue #120: deps stripped — recover the tail's hidden work-refs so the
            // site still binds to the one buffer. No work-ref to recover → AsIs.
            if let Some(last) = l.last() {
                let extra = Self::collect_hidden_ref_args(last, &self.data);
                if !extra.is_empty() {
                    return RefDelivery::Rename(extra);
                }
            }
            RefDelivery::AsIs
        } else if self.return_views_local(ls) {
            // #306: the tail borrows a LOCAL's store — copy it before it escapes.
            RefDelivery::MaterializeView
        } else {
            // Owned / arg-borrow: rename the tail's work-ref(s) onto `__retbuf`.
            RefDelivery::Rename(ls.to_vec())
        }
    }

    /// @PLN85 D-own-1 — emit the mechanism the Reference selector chose. The tail of
    /// `l` is rewritten in place; mirrors `dispatch_vector_delivery`.
    fn dispatch_reference_delivery(&mut self, delivery: RefDelivery, td: u32, l: &mut [Value]) {
        match delivery {
            RefDelivery::Rename(ws) => {
                self.ref_return(&ws, l, RetSite::BlockTail);
                self.nrvo_collapse_tail_set(l, &ws);
            }
            RefDelivery::MaterializeView => {
                let last = l.len() - 1;
                let w = self.materialize_view_return(td, &mut l[last]);
                self.ref_return(&[w], l, RetSite::BlockTail);
                self.nrvo_collapse_tail_set(l, &[w]);
            }
            RefDelivery::AsIs => {}
        }
    }

    /// #409 — a `#native`/`#rust` callee delivers its OWN store and never writes
    /// the `__retbuf` it was handed; leaving the forward returns that foreign
    /// value with `__retbuf` empty, so the caller's later in-place `+=` rebuilds
    /// the empty buffer and drops the data. Mint a fresh `__fwd` local, run the
    /// call into it, then COPY into `__retbuf` (clear + element-append) — the
    /// shape a hand-written `r = native(); r` produces. Finalize the return-type
    /// dep to `{__retbuf}` so a caller binds its result var to the buffer it
    /// passed (else the signature stays bare-vector and `+=` drops data). A no-op
    /// when there is no buffer, no work-var, or no tail.
    fn emit_forward_copy_409(&mut self, elm: &Type, l: &mut [Value]) {
        let elm_ty = elm.clone();
        let Some((buf_attr, buf_var)) = self.return_buffer() else {
            return;
        };
        let fwd = self.create_var(
            "__fwd",
            &Type::Vector(Box::new(elm_ty.clone()), Deps::none()),
        );
        if fwd == u16::MAX {
            return;
        }
        let rec_tp = self.append_elem_tp(&elm_ty);
        let clear = self.cl("OpClearVector", &[Value::Var(buf_var)]);
        let append = self.cl(
            "OpAppendVector",
            &[Value::Var(buf_var), Value::Var(fwd), Value::Int(rec_tp)],
        );
        let Some(last) = l.last_mut() else {
            return;
        };
        let orig = std::mem::replace(last, Value::Null);
        let set_fwd = crate::data::v_set(fwd, orig);
        *last = crate::data::v_block(
            vec![set_fwd, clear, append, Value::Var(buf_var)],
            Type::Vector(Box::new(elm_ty.clone()), Deps::frame1(buf_var)),
            "fwd_copy_409",
        );
        let dep = Deps::attrs(vec![buf_attr]);
        self.data.definitions[self.context as usize].returned = Type::Vector(Box::new(elm_ty), dep);
    }

    /// Plan-14 phase 07 (P234 runtime): rewrite a body-tail
    /// `Value::Tuple([elem_0, elem_1, …])` into the synthetic-struct
    /// construction sequence that an inline struct literal would
    /// produce — `(p, 5)` becomes
    ///
    /// ```text
    /// {
    ///     w = null;
    ///     OpDatabase(w, __tuple<…>_known_type);
    ///     w._0 = elem_0;     // OpSet* at field offset 0
    ///     w._1 = elem_1;     // OpSet* at field offset 16 (alignment-padded)
    ///     w
    /// }
    /// ```
    ///
    /// Mirrors `parse_object`'s allocation + per-field-init pattern.
    /// The work-ref `w` is created via `vars.work_refs(...)`; the
    /// resulting block carries `Reference(synthetic_d_nr, vec![w])`
    /// so scope analysis tracks `w`'s store as the source of the
    /// returned DbRef's lifetime — same machinery struct returns
    /// use today.
    /// P236: when a function body's tail is `Value::If(...)` (or `match`,
    /// which lowers to nested `If`) and each branch terminates with a
    /// fresh work-ref via Object/struct construction, the branches end
    /// up with DIFFERENT work-refs (`__ref_1`, `__ref_2`, …).  Native
    /// codegen then loses the if/else's value: each branch's local DbRef
    /// is dropped, both work-refs get freed, and the function returns the
    /// typed null sentinel.  Interp accidentally works because OpReturn
    /// reads from eval-stack top.
    ///
    /// Fix: pick the FIRST branch's terminal work-ref as the shared one
    /// and rewrite every other branch in place — substitute their
    /// work-ref `Var` references with the shared one, and rewrite Set
    /// LHS slots and Block.result deps so scope analysis tracks the
    /// shared work-ref as the unique source of the returned DbRef's
    /// lifetime.  After unification, `returned_var(If)` (extended in
    /// `scopes.rs::returned_var`) recognises the shared var and skips
    /// `OpFreeRef` on it; `ref_return` promotes it to a hidden caller
    /// arg as it would for a single-branch reference return.
    ///
    /// Returns `Some(shared_work_ref)` if the rewrite fired (so the
    /// caller can wrap the if/else in `Value::Return`), `None`
    /// otherwise (mixed shapes, no work-refs, or branches already
    /// share a var).
    pub(crate) fn unify_if_branches_work_refs(&mut self, tail: &mut Value) -> Option<u16> {
        let if_value = tail.unspan_mut();
        if !matches!(if_value, Value::If(_, _, _)) {
            return None;
        }
        // Collect EVERY arm's terminal var across the whole if-tree — an `else if`
        // chain nests the alternatives as `If(_, arm, If(_, arm, …))`, so a 3-arm
        // (or deeper) tail has three+ distinct terminals (`__ref_1`, `__ref_2`,
        // `__ref_3`). The 2-arm case is just N=2 of this. Without collecting the
        // whole chain the nested `If`'s terminals differ and the old pair-only
        // lookup bailed, leaving native to drop the value and return the typed
        // null sentinel for every arm (struct/ref 3-arm if returned the LAST arm
        // on native — the struct sibling of the vector a7 bug).
        let mut terms = Vec::new();
        Self::collect_branch_terminal_vars(if_value, &mut terms);
        // Need at least one terminal, and ALL must be parser-internal work-refs
        // (`__ref_N` / `__rref_N`). Renaming a user-named parameter (e.g.
        // `if c { gen_x } else { gen_y }`) would corrupt the result, so bail and
        // let the existing scope analysis handle the tail.
        let first = *terms.first()?;
        let all_work_refs = terms.iter().all(|&v| {
            let n = self.vars.name(v);
            n.starts_with("__ref_") || n.starts_with("__rref_")
        });
        if !all_work_refs {
            return None;
        }
        // Pick the FIRST arm's work-ref as the shared one; rewrite every OTHER arm
        // (Var references, Set LHS slots, Block.result deps) to it across the whole
        // tail, so all arms deliver through one return slot. Idempotent when the
        // arms already share `first`.
        for &other in terms.iter().skip(1) {
            if other != first {
                Self::substitute_work_ref(if_value, other, first);
            }
        }
        Some(first)
    }

    /// Collect the terminal `Value::Var` of EVERY arm reachable through an
    /// `if`/`else-if` chain — descends both branches of each nested `If` so an
    /// N-arm chain yields all N terminals. A non-`Var`-terminating arm
    /// contributes nothing, so a mixed tail (one arm not ending in a work-ref)
    /// is detected by the caller's `all_work_refs` check and left un-unified.
    fn collect_branch_terminal_vars(branch: &Value, out: &mut Vec<u16>) {
        match branch.unspan() {
            Value::Var(v) => {
                if !out.contains(v) {
                    out.push(*v);
                }
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last() {
                    Self::collect_branch_terminal_vars(last, out);
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last() {
                    Self::collect_branch_terminal_vars(last, out);
                }
            }
            Value::If(_, t, f) => {
                Self::collect_branch_terminal_vars(t, out);
                Self::collect_branch_terminal_vars(f, out);
            }
            _ => {}
        }
    }

    /// Replace every reference to work-ref `from` with `to` in `val` —
    /// extends `replace_var_in_ir` semantics to also rewrite `Set`
    /// LHS slots and `Block.result` dep entries.  Used by
    /// `unify_branch_to` so the parser-level dep tracking (which feeds
    /// scope analysis and `ref_return`) sees only the shared work-ref
    /// after unification.
    fn substitute_work_ref(val: &mut Value, from: u16, to: u16) {
        match val {
            Value::Var(v) if *v == from => {
                *v = to;
            }
            Value::Set(slot, body) => {
                if *slot == from {
                    *slot = to;
                }
                Self::substitute_work_ref(body, from, to);
            }
            Value::TuplePut(slot, _, body) => {
                if *slot == from {
                    *slot = to;
                }
                Self::substitute_work_ref(body, from, to);
            }
            Value::BreakWith(_, body) => {
                Self::substitute_work_ref(body, from, to);
            }
            Value::Return(body) | Value::Drop(body) | Value::Yield(body) => {
                Self::substitute_work_ref(body, from, to);
            }
            Value::Call(_, args)
            | Value::CallRef(_, args)
            | Value::Insert(args)
            | Value::Tuple(args)
            | Value::Parallel(args) => {
                for a in args.iter_mut() {
                    Self::substitute_work_ref(a, from, to);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &mut bl.operators {
                    Self::substitute_work_ref(op, from, to);
                }
                Self::rewrite_dep_in_type(&mut bl.result, from, to);
            }
            Value::If(cond, t, f) => {
                Self::substitute_work_ref(cond, from, to);
                Self::substitute_work_ref(t, from, to);
                Self::substitute_work_ref(f, from, to);
            }
            Value::Iter(_, a, b, c) => {
                Self::substitute_work_ref(a, from, to);
                Self::substitute_work_ref(b, from, to);
                Self::substitute_work_ref(c, from, to);
            }
            Value::Span(b) => Self::substitute_work_ref(&mut b.1, from, to),
            Value::ParFor(b) => {
                Self::substitute_work_ref(&mut b.input, from, to);
                Self::substitute_work_ref(&mut b.worker, from, to);
            }
            Value::Var(_)
            | Value::Int(_)
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
            | Value::Null => {}
        }
    }

    /// Replace `from` with `to` in any dep list inside `tp`'s
    /// reference-bearing variants.  Mirrors how
    /// `Type::Reference(_, deps)` carries the source-of-lifetime var
    /// list that scope analysis reads.
    fn rewrite_dep_in_type(tp: &mut Type, from: u16, to: u16) {
        let deps_mut: Option<&mut Vec<u16>> = match tp {
            Type::Reference(_, d)
            | Type::Vector(_, d)
            | Type::Enum(_, true, d)
            | Type::Sorted(_, _, d)
            | Type::Hash(_, _, d)
            | Type::Index(_, _, d)
            | Type::Spacial(_, _, d)
            | Type::Text(d) => Some(d),
            _ => None,
        };
        if let Some(d) = deps_mut {
            for v in d.iter_mut() {
                if *v == from {
                    *v = to;
                }
            }
        }
    }

    /// @P377 / S1 — parse-time NRVO for the intermediate-local return shape.
    ///
    /// After `ref_return` has promoted `cv` to the function's hidden return
    /// buffer attribute, an inner heap-returning call inside the body still
    /// targets its own parser-synthesised `__ref_N` work-ref, and the
    /// `Set(cv, …)` then copies that into `cv`.  `__ref_N`'s store has no
    /// remaining owner — that's the @P377 leak.
    ///
    /// S1 substitutes `__ref_N → cv` in the inner Call's hidden-buffer arg
    /// (and anywhere else `__ref_N` is referenced inside the inner Call)
    /// so the inner Call writes directly into the outer fn's hidden buffer.
    /// `Set(cv, …)` becomes a same-store self-copy — already a no-op in
    /// `OpCopyRecord` and exercised today by every direct-return shape.
    ///
    /// Preconditions — fires only when ALL hold:
    ///   1. `cv` is in `ls` (just promoted by `ref_return` immediately above).
    ///   2. Block tail is `Var(cv)` or `Return(Var(cv))` (modulo `Span`).
    ///   3. Penultimate statement is `Set(cv, Call(fn_nr, args))`.
    ///   4. `fn_nr` has a hidden Reference / Vector / struct-Enum attribute
    ///      at some index `i`.
    ///   5. `args[i]` is `Value::Var(work_ref)` and `vars.name(work_ref)`
    ///      starts with `__ref_` / `__rref_` (parser-internal, not a
    ///      user-named alias).
    ///   6. `work_ref != cv` (idempotency).
    ///
    /// Bails silently on any mismatch.  No warnings, no errors.  The
    /// Set/Var pair is left in place — the codegen treats a same-store
    /// `OpCopyRecord` as a no-op, so the IR shape stays uniform with the
    /// direct-return path.
    pub(crate) fn nrvo_collapse_tail_set(&mut self, l: &mut [Value], ls: &[u16]) {
        if self.first_pass || l.is_empty() || ls.is_empty() {
            return;
        }
        let last = l.len() - 1;

        // (1) Tail must be `Var(cv)` or `Return(Var(cv))`, modulo Span.
        let Some(cv) = Self::tail_var(&l[last]) else {
            return;
        };
        if !ls.contains(&cv) {
            return;
        }

        if last == 0 {
            // No prior op to substitute — only the tail Var(cv).
            return;
        }

        // (2) FAST PATH — the penultimate op is `Set(cv, Call(...))`: collapse
        //     it (and any earlier CONSECUTIVE `Set(cv, Call)` chain) below.
        //     When the defining call is EARLIER, with in-place mutation between
        //     it and the tail (`t = base(); t += …; t` — the @PLN85 cluster-462
        //     merge / `game_items()` shape), the penultimate is the mutation,
        //     not the call: fall back to redirecting cv's single top-level
        //     defining call instead of returning (else its `__ref_N` buffer is
        //     allocated, orphaned, and leaks one store per call).
        let penultimate_is_set_cv_call = matches!(
            l[last - 1].unspan(),
            Value::Set(slot, rhs) if *slot == cv && matches!(rhs.unspan(), Value::Call(_, _))
        );
        if !penultimate_is_set_cv_call {
            let collapsed = self.nrvo_collapse_defining_call(l, cv);
            self.suppress_collapsed_workrefs(l, collapsed);
            return;
        }
        let prev = l[last - 1].unspan_mut();
        let Value::Set(slot, rhs) = prev else { return };
        if *slot != cv {
            return;
        }
        let rhs_inner = rhs.unspan_mut();
        let Value::Call(fn_nr, args) = rhs_inner else {
            return;
        };
        let fn_nr_val = *fn_nr;

        // (3) Find the first hidden buffer attribute index on the callee
        //     whose typedef is heap-allocated (Reference / Vector / struct-Enum).
        let hidden_idx = {
            let def = self.data.def(fn_nr_val);
            def.attributes().iter().enumerate().find_map(|(i, a)| {
                if !a.hidden {
                    return None;
                }
                if !matches!(
                    &a.typedef,
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                ) {
                    return None;
                }
                Some(i)
            })
        };
        let Some(i) = hidden_idx else { return };

        // (4) args[i] must be a parser-internal __ref_N / __rref_N work-ref,
        //     distinct from cv.
        if args.len() <= i {
            return;
        }
        let work_ref = match args[i].unspan() {
            Value::Var(v) => *v,
            _ => return,
        };
        if work_ref == cv {
            return;
        }
        let nm = self.vars.name(work_ref);
        if !nm.starts_with("__ref_") && !nm.starts_with("__rref_") {
            return;
        }

        // (5) Substitute work_ref → cv inside the call's args.
        for a in args.iter_mut() {
            Self::substitute_work_ref(a, work_ref, cv);
        }
        // Each work-ref collapsed onto `cv` is now redirected: the inner call
        // delivers into `cv` directly, so the work-ref's own buffer is orphaned.
        // Collect them and (below, once `l` is final) suppress their eager
        // allocation — without this they leak one store per call (the
        // adopt-and-re-return shape, @PLN85 cluster-462 / #462).
        let mut collapsed_refs = vec![work_ref];

        // (6) @PLAN51 Cluster II — extend the substitution backwards to
        //     EARLIER consecutive `Set(cv, Call(_))` ops (probes 02, 21).
        //     Stops at any non-Set/non-Line op (intervening stmt, If,
        //     etc.) — those are unsafe to swap through (the discard's
        //     RHS may read cv; conditional Sets need branch-aware
        //     reasoning).  Probes 03, 04, 07, 11, 25, 26, 28 remain
        //     leaky; their substitution requires extending into IR
        //     wrappers which is parser-invasive (an earlier attempt
        //     broke tests/scripts/87-store-leaks.loft because
        //     conditional Sets to cv interact with paired_witness in
        //     ways that a blanket "substitute every Set(cv, Call)"
        //     doesn't handle correctly).
        let mut idx = last - 1;
        while idx > 0 {
            idx -= 1;
            if matches!(l[idx], Value::Line(_)) {
                continue;
            }
            let earlier = l[idx].unspan_mut();
            let Value::Set(eslot, erhs) = earlier else {
                break;
            };
            if *eslot != cv {
                break;
            }
            let erhs_inner = erhs.unspan_mut();
            let Value::Call(efn, eargs) = erhs_inner else {
                break;
            };
            let efn = *efn;
            let ehidden_idx = {
                let def = self.data.def(efn);
                def.attributes().iter().enumerate().find_map(|(i, a)| {
                    if !a.hidden {
                        return None;
                    }
                    if !matches!(
                        &a.typedef,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    ) {
                        return None;
                    }
                    Some(i)
                })
            };
            let Some(ei) = ehidden_idx else { break };
            if eargs.len() <= ei {
                break;
            }
            let ework_ref = match eargs[ei].unspan() {
                Value::Var(v) => *v,
                _ => break,
            };
            if ework_ref == cv {
                continue;
            }
            let enm = self.vars.name(ework_ref);
            if !enm.starts_with("__ref_") && !enm.starts_with("__rref_") {
                break;
            }
            for a in eargs.iter_mut() {
                Self::substitute_work_ref(a, ework_ref, cv);
            }
            collapsed_refs.push(ework_ref);
        }

        self.suppress_collapsed_workrefs(l, collapsed_refs);
    }

    /// A work-ref collapsed onto `cv` (its inner call now delivers into `cv`
    /// directly) no longer owns a store, so suppress its eager buffer
    /// allocation.  For a VECTOR work-ref `skip_free` flips
    /// `gen_set_first_vector_null` to the no-alloc `OpInitRefSentinel` path and
    /// tells scope analysis there is nothing to free — closing the
    /// adopt-and-re-return leak (#462) at its source instead of minting then
    /// orphaning a store.
    ///
    /// VECTOR-only on purpose: a Reference/struct-Enum work-ref is initialised
    /// through `gen_set_first_ref_*` (a deep-COPY path, not the sentinel
    /// branch), where its store is genuinely live and already freed balanced by
    /// scope analysis (the @P377 `render_struct(p) -> Canvas` shape).  Marking
    /// THAT skip_free both skips a real alloc and suppresses a real free →
    /// leak.  Skip a work-ref that still has any live use (a defensive guard; a
    /// freshly-minted call buffer never does).
    fn suppress_collapsed_workrefs(&mut self, l: &[Value], refs: Vec<u16>) {
        for w in refs {
            if matches!(self.vars.tp(w), Type::Vector(_, _)) && !Self::ir_var_has_live_use(l, w) {
                self.vars.set_skip_free(w);
            }
        }
    }

    /// General NRVO collapse for the "adopt → mutate in place → return" shape
    /// (`t = base(); t += …; t` — the @PLN85 cluster-462 merge / `game_items()`
    /// case, #462).  The penultimate-op fast path in `nrvo_collapse_tail_set`
    /// misses it because the call defining `cv` is followed by in-place
    /// mutation before the tail.  When `cv` has EXACTLY ONE defining
    /// `Set(cv, Call(fn, [..__ref_N..]))` and it sits at the TOP LEVEL of the
    /// body, redirect that call's hidden buffer arg onto `cv` (the promoted
    /// retbuf) so the inner call delivers there directly — no orphaned buffer.
    /// Returns the collapsed work-ref(s) for the caller's skip_free pass.
    ///
    /// Guarded against the conditional-reassign hazard the penultimate chain
    /// documents (a blanket substitution broke `87-store-leaks.loft`): a SECOND
    /// defining `Set(cv, Call)` ANYWHERE — including nested in an `if`/loop arm
    /// — means `cv` may be conditionally re-defined, so leave it untouched.  A
    /// sole defining call that is itself nested (assigned only inside a branch)
    /// is also skipped: redirecting a conditionally-run delivery into the
    /// retbuf is unsound.
    fn nrvo_collapse_defining_call(&self, l: &mut [Value], cv: u16) -> Vec<u16> {
        // VECTOR returns only.  The adopt-then-mutate-then-return shape is
        // proven safe for a vector tail (`t = base(); t += …; t`): the in-place
        // mutations append, never re-own.  A STRUCT (Reference/Enum) tail of the
        // same syntactic shape (`rs = new(); rs.f = …; rs`) instead OVERWRITES
        // owned fields after the defining call, and redirecting that call into
        // the retbuf perturbs the field-overwrite free ordering (a small Sim
        // field leak in the crawler).  That case is a separate, harder shape —
        // left on its existing (correct) delivery path until proven.
        if !matches!(self.vars.tp(cv), Type::Vector(_, _)) {
            return Vec::new();
        }
        // Count EVERY assignment to `cv` (`Set(cv, _)` with ANY rhs, incl.
        // nested in `if`/loop arms), and note the FIRST top-level buffer-call
        // assignment.  `cv` is eligible ONLY when it is assigned exactly once —
        // its defining buffer call — and that lone assignment is at the top
        // level.  A second assignment anywhere means `cv` is conditionally
        // re-defined (`best = mon_none(); … if … { best = cand } … best`, where
        // `cand` is a borrowed view): redirecting the first call into the retbuf
        // and freeing nothing would orphan the buffer the later value never
        // wrote.  Counting ALL assignments (not just call-assignments) is what
        // separates the safe merge shape from this hazard — a `cv = view`
        // re-define is a `Set(cv, Var)`, invisible to a call-only count.
        let mut assigns = 0usize;
        let mut top_level_idx: Option<usize> = None;
        for (idx, op) in l.iter().enumerate() {
            assigns += Self::count_cv_assignments(op, cv);
            if top_level_idx.is_none() && self.buffer_call_workref(op, cv).is_some() {
                top_level_idx = Some(idx);
            }
        }
        if assigns != 1 {
            return Vec::new();
        }
        let Some(idx) = top_level_idx else {
            // The sole assignment is not a top-level buffer call (a bare view
            // bind, or nested in a branch) — unsafe / nothing to redirect.
            return Vec::new();
        };
        // Re-resolve the work-ref against the (mutable) node, then substitute.
        let Some(work_ref) = self.buffer_call_workref(&l[idx], cv) else {
            return Vec::new();
        };
        if let Value::Set(_, rhs) = l[idx].unspan_mut()
            && let Value::Call(_, args) = rhs.unspan_mut()
        {
            for a in args.iter_mut() {
                Self::substitute_work_ref(a, work_ref, cv);
            }
            vec![work_ref]
        } else {
            Vec::new()
        }
    }

    /// Recursively count assignments to slot `cv` — every `Set(cv, _)` /
    /// `TuplePut(cv, …)`, with ANY right-hand side, anywhere in `node`
    /// (including nested `if`/loop arms).  In-place mutation of `cv` (vector
    /// append, struct field write) is NOT an assignment to `cv`'s slot, so it
    /// is correctly not counted.  `nrvo_collapse_defining_call` uses this to
    /// fire only when `cv` is assigned exactly once.
    fn count_cv_assignments(node: &Value, cv: u16) -> usize {
        let here = matches!(node.unspan(), Value::Set(s, _) | Value::TuplePut(s, _, _) if *s == cv);
        let children = match node {
            Value::Set(_, b)
            | Value::TuplePut(_, _, b)
            | Value::Return(b)
            | Value::Drop(b)
            | Value::Yield(b)
            | Value::BreakWith(_, b) => Self::count_cv_assignments(b, cv),
            Value::Span(b) => Self::count_cv_assignments(&b.1, cv),
            Value::Call(_, a)
            | Value::CallRef(_, a)
            | Value::Insert(a)
            | Value::Tuple(a)
            | Value::Parallel(a) => a.iter().map(|x| Self::count_cv_assignments(x, cv)).sum(),
            Value::Block(bl) | Value::Loop(bl) => bl
                .operators
                .iter()
                .map(|o| Self::count_cv_assignments(o, cv))
                .sum(),
            Value::If(c, t, f) => {
                Self::count_cv_assignments(c, cv)
                    + Self::count_cv_assignments(t, cv)
                    + Self::count_cv_assignments(f, cv)
            }
            Value::Iter(_, a, b, c) => {
                Self::count_cv_assignments(a, cv)
                    + Self::count_cv_assignments(b, cv)
                    + Self::count_cv_assignments(c, cv)
            }
            Value::ParFor(b) => {
                Self::count_cv_assignments(&b.input, cv) + Self::count_cv_assignments(&b.worker, cv)
            }
            _ => 0,
        };
        usize::from(here) + children
    }

    /// Read-only probe: if `node` is `Set(cv, Call(fn, args))` whose callee has
    /// a hidden heap buffer attribute filled by a parser-internal
    /// `__ref_N`/`__rref_N` work-ref distinct from `cv`, return that work-ref.
    /// Mirrors steps (3)–(4) of `nrvo_collapse_tail_set`'s fast path without
    /// mutating, so it can drive both detection and the redirect.
    fn buffer_call_workref(&self, node: &Value, cv: u16) -> Option<u16> {
        let Value::Set(slot, rhs) = node.unspan() else {
            return None;
        };
        if *slot != cv {
            return None;
        }
        let Value::Call(fn_nr, args) = rhs.unspan() else {
            return None;
        };
        let def = self.data.def(*fn_nr);
        let i = def.attributes().iter().position(|a| {
            a.hidden
                && matches!(
                    &a.typedef,
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                )
        })?;
        let work_ref = match args.get(i)?.unspan() {
            Value::Var(v) => *v,
            _ => return None,
        };
        if work_ref == cv {
            return None;
        }
        let nm = self.vars.name(work_ref);
        (nm.starts_with("__ref_") || nm.starts_with("__rref_")).then_some(work_ref)
    }

    /// True when `v` appears in `l` as anything other than its own
    /// null-init `Set(v, Null)` — i.e. a live read (`Var(v)`) or a real
    /// store-producing reassignment (`Set(v, non-null)`).  Used by
    /// `nrvo_collapse_tail_set` to confirm a collapsed work-ref is fully
    /// dead before suppressing its allocation/free (skipping the free of a
    /// still-live owned ref would itself leak).
    fn ir_var_has_live_use(l: &[Value], v: u16) -> bool {
        fn walk(val: &Value, v: u16) -> bool {
            match val {
                Value::Var(x) => *x == v,
                // `Set(v, Null)` is the work-ref's own init — not a live use;
                // any other `Set(v, …)` allocates/writes into it (live).
                Value::Set(slot, body) | Value::TuplePut(slot, _, body) => {
                    (*slot == v && !matches!(body.unspan(), Value::Null)) || walk(body, v)
                }
                Value::Return(b) | Value::Drop(b) | Value::Yield(b) | Value::BreakWith(_, b) => {
                    walk(b, v)
                }
                Value::Span(b) => walk(&b.1, v),
                Value::Call(_, args)
                | Value::CallRef(_, args)
                | Value::Insert(args)
                | Value::Tuple(args)
                | Value::Parallel(args) => args.iter().any(|a| walk(a, v)),
                Value::Block(bl) | Value::Loop(bl) => bl.operators.iter().any(|op| walk(op, v)),
                Value::If(c, t, f) => walk(c, v) || walk(t, v) || walk(f, v),
                Value::Iter(_, a, b, c) => walk(a, v) || walk(b, v) || walk(c, v),
                Value::ParFor(b) => walk(&b.input, v) || walk(&b.worker, v),
                _ => false,
            }
        }
        l.iter().any(|op| walk(op, v))
    }

    /// Walk past `Span` / `Return` wrappers to find a tail `Var(v)`.
    /// Used by `nrvo_collapse_tail_set` to recognise the two shapes the
    /// parser produces for "the body returns variable `v`".
    fn tail_var(v: &Value) -> Option<u16> {
        match v.unspan() {
            Value::Var(v) => Some(*v),
            Value::Return(inner) => Self::tail_var(inner),
            _ => None,
        }
    }

    pub(crate) fn rewrite_tail_tuple_to_synthetic_struct(
        &mut self,
        synthetic_d_nr: u32,
        tail: &mut Value,
    ) {
        // A7.1: allocate ONE shared work-ref up front, then descend
        // recursively through `If` / `Block` / `Insert` / `Span`
        // wrappers so every leaf `Value::Tuple` writes into the same
        // record.  Sharing avoids ref_return promoting two separate
        // hidden args (one per branch); the function then returns a
        // single work-ref whose value is well-defined at the join
        // point.  Mirrors the unification done by P236's
        // `unify_if_branches_work_refs` for struct returns.
        let synth_ref_type = Type::Reference(synthetic_d_nr, Deps::none());
        let w = self.vars.work_refs(&synth_ref_type, &mut self.lexer);
        let known_type = self.data.def(synthetic_d_nr).known_type();
        self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, tail);
    }

    fn rewrite_tail_tuple_with_work_ref(
        &mut self,
        synthetic_d_nr: u32,
        known_type: u16,
        w: u16,
        tail: &mut Value,
    ) {
        match tail {
            Value::Span(b) => {
                self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, &mut b.1);
                return;
            }
            Value::If(_, then_branch, else_branch) => {
                self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, then_branch);
                self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, else_branch);
                return;
            }
            Value::Block(b) => {
                if let Some(last) = b.operators.last_mut() {
                    self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, last);
                }
                b.result = Type::Reference(synthetic_d_nr, Deps::frame1(w));
                return;
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, last);
                }
                return;
            }
            _ => {}
        }
        let elements = match std::mem::replace(tail, Value::Null) {
            Value::Tuple(elems) => elems,
            other => {
                *tail = other;
                return;
            }
        };
        let mut ops: Vec<Value> = Vec::with_capacity(elements.len() + 3);
        ops.push(crate::data::v_set(w, Value::Null));
        ops.push(self.cl(
            "OpDatabase",
            &[Value::Var(w), Value::Int(i32::from(known_type))],
        ));
        for (i, elem) in elements.into_iter().enumerate() {
            ops.push(self.set_field_no_check(synthetic_d_nr, i, 0, Value::Var(w), elem));
        }
        ops.push(Value::Var(w));
        *tail = crate::data::v_block(
            ops,
            Type::Reference(synthetic_d_nr, Deps::frame1(w)),
            "synthetic_tuple_return",
        );
    }

    // <operator> ::= '..' ['='] |
    //                '||' | 'or' |
    //                '&&' | 'and' |
    //                '==' | '!=' | '<' | '<=' | '>' | '>=' |
    //                '|' |
    //                '^' |
    //                '&' |
    //                '<<' | '>>' |
    //                '-' | '+' |
    //                '*' | '/' | '%'
    // <operators> ::= <single>  { '.' <field> | '[' <index> ']' } | <operators> <operator> <operators>
    pub(crate) fn parse_if(&mut self, code: &mut Value) -> Type {
        let mut test = Value::Null;
        let tp = self.expression(&mut test);
        self.convert(&mut test, &tp, &Type::Boolean);
        let is_aliases: Vec<(String, Option<u16>)> = self.is_capture_aliases.drain(..).collect();
        let is_bindings: Vec<Value> = self.is_capture_bindings.drain(..).collect();
        let mut true_code = Value::Null;
        let write_state = self.vars.save_and_clear_write_state();
        self.vars.clear_write_state();
        let mut true_type = self.parse_block("if", &mut true_code, &Type::Unknown(0));
        if !is_bindings.is_empty()
            && let Value::Block(bl) = &mut true_code
        {
            let mut new_ops = is_bindings;
            new_ops.append(&mut bl.operators);
            bl.operators = new_ops;
        }
        for (name, old) in &is_aliases {
            if let Some(old_nr) = old {
                self.vars.set_name(name, *old_nr);
            } else {
                self.vars.remove_name(name);
            }
        }
        let mut false_type = Type::Void;
        let mut false_code = Value::Null;
        if self.lexer.has_token("else") {
            self.vars.restore_write_state(&write_state);
            self.vars.clear_write_state();
            if self.lexer.has_token("if") {
                self.parse_if(&mut false_code);
            } else {
                if matches!(true_type, Type::Null | Type::Never) {
                    true_type = Type::Unknown(0);
                }
                false_type = self.parse_block("else", &mut false_code, &true_type);
                if true_type == Type::Unknown(0) {
                    if let Value::Block(bl) = &mut true_code {
                        let p = bl.operators.len() - 1;
                        if !is_block_divergent(&bl.operators) {
                            bl.operators[p] = self.null(&false_type);
                        }
                        bl.result = false_type.clone();
                    }
                    true_type = false_type.clone();
                }
            }
        } else {
            self.vars.restore_write_state(&write_state);
            if !matches!(true_type, Type::Void | Type::Never) {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "If-expression produces a value but has no else clause; add an else branch or make the body a statement"
                    );
                }
                false_code = v_block(vec![self.null(&true_type)], true_type.clone(), "else");
            }
        }
        self.vars.restore_write_state(&write_state);
        *code = v_if(test, true_code, false_code);
        merge_dependencies(&true_type, &false_type)
    }

    // <match> ::= 'match' <expression> '{' { <pattern> '=>' <expression> } '}'
    // <pattern> ::= '_' | <variant> [ '{' <field> { ',' <field> } '}' ]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_match(&mut self, code: &mut Value) -> Type {
        // Save position of the match keyword for exhaustiveness diagnostics.
        let match_pos = self.lexer.pos().clone();
        // 1. Parse the subject expression.
        let mut subject = Value::Null;
        let subject_type = self.expression(&mut subject);

        // Resolve type info from the subject.
        // Accepts: plain enums, struct-enums, struct-enum variants, and plain structs (T1-18).
        let (e_nr, is_struct, valid_enum, is_plain_struct) = match &subject_type {
            Type::Enum(nr, s, _) => (*nr, *s, true, false),
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::EnumValue => {
                let parent = self.data.def(*d_nr).parent();
                (parent, true, true, false)
            }
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::Enum => {
                // iterating a `vector<StructEnum>` yields loop variables
                // typed `Type::Reference(enum_def, _)` (via `for_type` in this
                // file, line 1952 — struct-enums degrade to a reference type
                // when carried through generic collections).  Without this
                // arm, matching a for-loop variable over a struct-enum vector
                // dropped into the error branch and every arm produced
                // 'Expect token }' cascades.
                (*d_nr, true, true, false)
            }
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::Struct => {
                (*d_nr, true, true, true)
            }
            // scalar types — dispatch to scalar match handler.
            Type::Integer(_)
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Text(_) => {
                return self.parse_scalar_match(subject, &subject_type, code);
            }
            // vector types — dispatch to vector match handler.
            Type::Vector(_, _) => {
                return self.parse_vector_match(subject, &subject_type, code);
            }
            // T1.9: tuple types — dispatch to tuple match handler.
            Type::Tuple(_) => {
                return self.parse_tuple_match(subject, &subject_type, code);
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "match requires an enum, struct, or scalar type"
                    );
                }
                (u32::MAX, false, false, false)
            }
        };

        // For plain enums (stack bytes), use a temp var to avoid re-evaluating the subject.
        // For struct enums (database references / DbRef), do NOT create a temp var — the
        // allocation system requires DbRefs to be freed in strict LIFO order and copying them
        // to a new variable breaks that invariant.  Instead, use the subject Value directly.
        let (subject_val, preamble): (Value, Option<(u16, Value)>) = if is_struct || !valid_enum {
            (subject, None)
        } else {
            let v = self.create_unique("match_subj", &subject_type);
            self.vars.defined(v);
            (Value::Var(v), Some((v, subject)))
        };

        // Build discriminant expression: integer representation of the active variant.
        let disc_expr = if is_struct {
            let get_enum = self.cl("OpGetEnum", &[subject_val.clone(), Value::Int(0)]);
            self.cl("OpConvIntFromEnum", &[get_enum])
        } else {
            self.cl("OpConvIntFromEnum", std::slice::from_ref(&subject_val))
        };

        self.lexer.token("{");

        let mut arms: Vec<EnumArm> = Vec::new();
        let mut covered: HashSet<u32> = HashSet::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;
        // L2: field bindings in conditional arms are hoisted before the if-chain
        // to avoid codegen stack-layout issues with text operations inside branches.
        let mut hoisted_bindings: Vec<Value> = Vec::new();

        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            // @PLN25 — a `null` pattern arm on a nullable inline enum element
            // (`match vr[i] { null => …, Some{…} => …/_ => … }`) matches the ABSENT
            // state: discriminant 0.  The synthetic `__nullable<S>` enum represents
            // null as disc 0 (not a produced variant), and `disc_expr` already reads
            // the discriminant, so this arm is just `discs == [0]`.  Scoped to the
            // synth enum (a regular enum's null is the variable store_nr sentinel,
            // not an inline disc — E1).  `null` is a keyword, not an identifier, so
            // it must be matched before the `has_identifier()` variant path below.
            if valid_enum
                && e_nr != u32::MAX
                && self.data.def(e_nr).name.starts_with("__nullable<")
                && self.lexer.has_token("null")
            {
                self.expect_match_arm_arrow();
                let arm_write_state = self.vars.save_and_clear_write_state();
                self.vars.clear_write_state();
                let mut arm_body = Value::Null;
                let arm_type = if self.lexer.peek_token("{") {
                    self.parse_block("match_arm", &mut arm_body, &Type::Unknown(0))
                } else {
                    self.expression(&mut arm_body)
                };
                self.vars.restore_write_state(&arm_write_state);
                if result_type == Type::Void || result_type == Type::Null {
                    result_type = arm_type.clone();
                } else if !self.first_pass
                    && arm_type != Type::Void
                    && arm_type != Type::Null
                    && !match_arm_types_unify(&result_type, &arm_type)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot unify: {} and {}",
                        result_type.name(&self.data),
                        arm_type.name(&self.data)
                    );
                }
                // A `null` arm (disc 0) covers the synth enum's `Null` variant for
                // exhaustiveness — disc 1 (the vestigial `Null` variant) is never
                // produced (null is disc 0), so `null` + `Some` IS exhaustive.
                let null_variant = self.data.variant_of(e_nr, "Null");
                if null_variant != u32::MAX {
                    covered.insert(null_variant);
                }
                arms.push(EnumArm {
                    discs: vec![0],
                    code: arm_body,
                    tp: arm_type,
                    guard: None,
                    bindings: Vec::new(),
                });
                self.lexer.has_token(","); // optional trailing comma
                continue;
            }
            let Some(first_ident) = self.lexer.has_identifier() else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expect variant name or '_' in match arm"
                    );
                }
                break;
            };

            // accept `Library::Variant` or `EnumName::Variant` qualified patterns.
            // The `::` resolves the right-hand identifier in the named scope.
            let pattern_name = if self.lexer.has_token("::") {
                let Some(vname) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "expect variant name after '::'");
                    }
                    break;
                };
                vname
            } else {
                first_ident.clone()
            };

            if pattern_name == "_" {
                let (arm, is_exhaustive) = self.parse_match_wildcard_arm(&mut result_type);
                has_wildcard = is_exhaustive;
                arms.push(arm);
                self.lexer.has_token(","); // optional trailing comma
                if !has_wildcard {
                    continue;
                }
                break;
            }

            // @PLN22 Phase 1 — resolve the variant against the subject enum via
            // the variant_of chokepoint (the (enum, variant) scope key), not the
            // bare global def_nr.  This also subsumes the C53 fix (a library
            // variant not wildcard-imported is still a child of its enum).  A
            // plain-struct match's "pattern" is the struct TYPE itself (still
            // globally keyed, never an EnumValue), so fall back to def_nr when
            // variant_of finds nothing.
            let mut variant_def_nr = self.data.variant_of(e_nr, &pattern_name);
            if variant_def_nr == u32::MAX {
                variant_def_nr = self.data.def_nr(&pattern_name);
            }

            // for plain struct match, the pattern name must match the struct type.
            // There is no discriminant — the arm always matches.
            if is_plain_struct {
                if variant_def_nr != e_nr && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' does not match struct type {}",
                        pattern_name,
                        self.data.def(e_nr).name()
                    );
                }
                let (arm, exhaustive) = self.parse_match_struct_arm(
                    e_nr,
                    &subject_val,
                    &mut result_type,
                    &mut hoisted_bindings,
                );
                has_wildcard = exhaustive;
                arms.push(arm);
                if has_wildcard {
                    break;
                }
                self.lexer.has_token(",");
                continue;
            }

            let bad_variant = e_nr == u32::MAX
                || variant_def_nr == u32::MAX
                || self.data.def_type(variant_def_nr) != DefType::EnumValue
                || self.data.def(variant_def_nr).parent() != e_nr;
            if bad_variant {
                if !self.first_pass && valid_enum && variant_def_nr != u32::MAX {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        pattern_name,
                        self.data.def(e_nr).name()
                    );
                }
                // Skip this arm gracefully.
                if self.lexer.peek_token("{") {
                    self.lexer.token("{");
                    while !self.lexer.peek_token("}") && !self.lexer.peek_token(";") {
                        self.lexer.has_identifier();
                        self.lexer.has_token(",");
                    }
                    self.lexer.token("}");
                }
                self.expect_match_arm_arrow();
                let mut arm_code = Value::Null;
                self.expression(&mut arm_code);
                // Consume the optional trailing comma, mirroring the wildcard /
                // struct arm paths.  Without it, the next loop iteration sees the
                // leading `,` instead of a variant name and breaks early, leaving
                // the lexer mid-arm-list — which desyncs into "Expect token }".
                // This skip path fires on pass 1 whenever `e_nr == u32::MAX`
                // (the subject enum is an unresolved cross-package forward
                // reference whose dependency parses later — #375); a clean skip
                // keeps pass 1 from aborting before the dependency registers, so
                // pass 2 (with the enum resolved) parses the arms normally.
                self.lexer.has_token(",");
                continue;
            }

            // Get the discriminant integer for this variant.
            let disc: i32 = if is_struct {
                // Struct enum: field-carrying variants store the discriminant
                // in attributes[0] (the synthetic "enum" attr added by
                // parse_enum_variants).  Unit variants (`pub enum E { Null,
                // Some { … } }`) carry no attributes of their own — fall
                // back to the parent enum's attribute for this variant name.
                let variant_attrs = self.data.def(variant_def_nr).attributes();
                if let Some(first) = variant_attrs.first()
                    && let Value::Enum(nr, _) = first.value
                {
                    i32::from(nr)
                } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&pattern_name) {
                    if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                        i32::from(nr)
                    } else {
                        0
                    }
                } else {
                    0
                }
            } else {
                // Plain enum: discriminant is stored in the parent enum's attributes.
                if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&pattern_name) {
                    if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                        i32::from(nr)
                    } else {
                        0
                    }
                } else {
                    0
                }
            };

            // or-patterns — collect additional variants separated by `|`.
            // Only for plain enum arms without field bindings.
            let mut all_discs = vec![disc];
            while self.lexer.has_token("|") {
                let Some(first_or) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "expect variant name after '|'");
                    }
                    break;
                };
                // accept Lib::Variant in or-patterns as well.
                let next_name = if self.lexer.has_token("::") {
                    let Some(vname) = self.lexer.has_identifier() else {
                        if !self.first_pass {
                            diagnostic!(self.lexer, Level::Error, "expect variant name after '::'");
                        }
                        break;
                    };
                    vname
                } else {
                    first_or.clone()
                };
                // @PLN22 Phase 1 — or-pattern variant resolves against the
                // subject enum via the variant_of chokepoint (or-patterns are
                // plain-enum only, so no struct fallback is needed).
                let next_def_nr = self.data.variant_of(e_nr, &next_name);
                if !self.first_pass
                    && (next_def_nr == u32::MAX
                        || self.data.def_type(next_def_nr) != DefType::EnumValue
                        || self.data.def(next_def_nr).parent() != e_nr)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        next_name,
                        self.data.def(e_nr).name()
                    );
                } else {
                    let next_disc = if is_struct {
                        // B1-style guard (same shape as line 603): unit
                        // variants carry no attributes of their own; fall
                        // back to the parent enum's attr list.
                        let next_variant_attrs = self.data.def(next_def_nr).attributes();
                        if let Some(first) = next_variant_attrs.first()
                            && let Value::Enum(nr, _) = first.value
                        {
                            i32::from(nr)
                        } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&next_name) {
                            if let Value::Enum(nr, _) =
                                self.data.def(e_nr).attributes()[*a_nr].value
                            {
                                i32::from(nr)
                            } else {
                                0
                            }
                        } else {
                            0
                        }
                    } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&next_name) {
                        if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                            i32::from(nr)
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    all_discs.push(next_disc);
                    // Each or-pattern variant counts for exhaustiveness.
                    if !self.first_pass {
                        covered.insert(next_def_nr);
                    }
                }
            }

            // Parse optional field bindings for struct-enum arms.
            let mut arm_stmts: Vec<Value> = Vec::new();
            let mut field_conditions: Vec<Value> = Vec::new();
            let mut name_aliases: Vec<(String, Option<u16>)> = Vec::new();
            if is_struct && self.lexer.peek_token("{") {
                self.parse_match_enum_field_bindings(
                    variant_def_nr,
                    &pattern_name,
                    &subject_val,
                    &mut arm_stmts,
                    &mut field_conditions,
                    &mut name_aliases,
                );
            }

            // parse optional guard clause after pattern + field bindings.
            // Field-bound variables are in scope for the guard expression.
            let guard_opt = self.parse_optional_guard();
            // L2: combine field sub-pattern conditions with the explicit guard (if any).
            let guard_opt = if field_conditions.is_empty() {
                guard_opt
            } else {
                let mut combined = field_conditions.remove(0);
                for c in field_conditions {
                    combined = v_if(combined, c, Value::Boolean(false));
                }
                // If there's also an explicit `if` guard, AND them.
                if let Some(g) = guard_opt {
                    combined = v_if(combined, g, Value::Boolean(false));
                }
                Some(combined)
            };

            // Duplicate arm detection.
            // Guarded arms don't count as covering the variant for exhaustiveness.
            if guard_opt.is_none() {
                if covered.contains(&variant_def_nr) {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Warning,
                            "unreachable arm: {} already matched",
                            pattern_name
                        );
                    }
                } else {
                    covered.insert(variant_def_nr);
                }
            }

            self.expect_match_arm_arrow();

            // Parse the arm body expression.
            // If the body starts with `{`, parse it as a scoped block so
            // the closing `}` is not confused with the match's `}`.
            // Save/restore write tracking so writes in one arm don't cause
            // false dead-assignment warnings in sibling arms.
            let arm_write_state = self.vars.save_and_clear_write_state();
            self.vars.clear_write_state();
            let mut arm_body = Value::Null;
            let arm_type = if self.lexer.peek_token("{") {
                self.parse_block("match_arm", &mut arm_body, &Type::Unknown(0))
            } else {
                self.expression(&mut arm_body)
            };
            self.vars.restore_write_state(&arm_write_state);

            // S15: restore name mappings after arm body so the next arm can
            // create its own alias for the same field name.
            for (name, old) in name_aliases.drain(..) {
                if let Some(old_nr) = old {
                    self.vars.set_name(&name, old_nr);
                } else {
                    self.vars.remove_name(&name);
                }
            }

            // Type unification across arms.  A `null` arm (Type::Null) lowers to
            // the result type's null sentinel — it unifies with any sibling type
            // and never pins the result, so the first CONCRETE arm wins even when
            // a `null` arm comes first (`Jade => null, Crimson => S{…}`).  Treat a
            // current `Null` result like `Void` for promotion, and skip the unify
            // check when this arm is itself `null`.  Without this, struct-or-null
            // enum matches were rejected ("cannot unify: S and null", #365).
            if result_type == Type::Void || result_type == Type::Null {
                result_type = arm_type.clone();
            } else if !self.first_pass
                && arm_type != Type::Void
                && arm_type != Type::Null
                && !match_arm_types_unify(&result_type, &arm_type)
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot unify: {} and {}",
                    result_type.name(&self.data),
                    arm_type.name(&self.data)
                );
            }

            // When there is a guard, keep field bindings separate — they must
            // be emitted before the guard check so bound variables are available.
            // When there is no guard, wrap them into a block as before.
            let (arm_code, binding_stmts) = if guard_opt.is_some() && !arm_stmts.is_empty() {
                (arm_body, arm_stmts)
            } else if arm_stmts.is_empty() {
                (arm_body, Vec::new())
            } else {
                arm_stmts.push(arm_body);
                (
                    v_block(arm_stmts, arm_type.clone(), "match_arm"),
                    Vec::new(),
                )
            };

            arms.push(EnumArm {
                discs: all_discs,
                code: arm_code,
                tp: arm_type,
                guard: guard_opt,
                bindings: binding_stmts,
            });
            if self.lexer.peek_token("}") {
                self.lexer.has_token(","); // optional trailing comma
            } else {
                self.lexer.token(","); // comma required between arms
            }
        }

        self.lexer.token("}");

        // Exhaustiveness check (second pass only, when no wildcard, when subject is a known enum).
        if !self.first_pass && !has_wildcard && valid_enum {
            let missing: Vec<String> = self
                .data
                .definitions
                .iter()
                .enumerate()
                .filter(|(_, d)| d.def_type == DefType::EnumValue && d.parent == e_nr)
                .filter(|(v_nr, _)| !covered.contains(&(*v_nr as u32)))
                .map(|(_, d)| d.name.clone())
                .collect();
            if !missing.is_empty() {
                let msg = format!(
                    "match on {} is not exhaustive — missing: {}; add the missing variants or a '_ =>' wildcard",
                    self.data.def(e_nr).name(),
                    missing.join(", ")
                );
                self.lexer.pos_diagnostic(Level::Error, &match_pos, &msg);
            }
        }

        // A `null` arm lowers to the result type's null sentinel — `parse_if`
        // (~line 1250) and `build_scalar_chain` do the same.  Now that
        // result_type is final, convert bare-null (and block-trailing-null) arm
        // bodies, and keep `arm.tp` in step so the guarded-binding block wrapper
        // (below) declares the right result type.  Without this a `null` arm
        // pushes nothing and the if-chain join reads an unwritten, value-sized
        // slot (interp stack underflow / native lost value) — the #365 family.
        let base = if matches!(result_type, Type::Void | Type::Null) {
            Value::Null
        } else {
            let typed_null = self.null(&result_type);
            for arm in &mut arms {
                let null_body = match &arm.code {
                    Value::Null => true,
                    Value::Block(bl) => bl
                        .operators
                        .last()
                        .is_some_and(|o| matches!(o, Value::Null)),
                    _ => false,
                };
                if !null_body {
                    continue;
                }
                match &mut arm.code {
                    Value::Block(bl) => {
                        let last = bl.operators.len() - 1;
                        bl.operators[last] = typed_null.clone();
                        bl.result = result_type.clone();
                    }
                    _ => arm.code = typed_null.clone(),
                }
                arm.tp = result_type.clone();
            }
            // Seed the chain base with the typed null too: an exhaustive enum
            // match's innermost else is unreachable, but codegen still emits it
            // and it must balance the value-sized stack slot the arms push.
            typed_null
        };

        // Build the if-chain from the collected arms (last to first).
        // `base` is reached only when no arm matches (only possible if
        // exhaustiveness fails, which is a compile error) — but it still has to
        // typecheck and balance the stack, so it carries the typed null.
        let mut chain = base;
        for arm in arms.iter().rev() {
            if arm.discs.is_empty() {
                // Wildcard — always taken; becomes the else branch of the chain.
                // guarded wildcard wraps body in If(guard, body, chain_rest).
                chain = match &arm.guard {
                    Some(guard) => v_if(guard.clone(), arm.code.clone(), chain),
                    None => arm.code.clone(),
                };
            } else {
                // build OR'd comparison for all discriminants in this arm.
                let mut cmp = self.cl("OpEqInt", &[disc_expr.clone(), Value::Int(arm.discs[0])]);
                for &d in &arm.discs[1..] {
                    let next = self.cl("OpEqInt", &[disc_expr.clone(), Value::Int(d)]);
                    cmp = v_if(cmp, Value::Boolean(true), next);
                }
                // guarded arms nest the guard inside the pattern branch.
                chain = match &arm.guard {
                    Some(guard) => {
                        let guarded = v_if(guard.clone(), arm.code.clone(), chain.clone());
                        let inner = if arm.bindings.is_empty() {
                            guarded
                        } else {
                            let mut stmts = arm.bindings.clone();
                            stmts.push(guarded);
                            v_block(stmts, arm.tp.clone(), "match_arm")
                        };
                        v_if(cmp, inner, chain)
                    }
                    None => v_if(cmp, arm.code.clone(), chain),
                };
            }
        }

        // When not a valid enum, just emit Null (errors were already reported).
        if !valid_enum {
            *code = Value::Null;
            return Type::Void;
        }

        // Emit the match:
        // - Plain enum: { match_subj = subject; chain }  (temp var to eval subject once)
        // - Struct enum: chain only  (subject_val is already the original expression/var)
        // L2: hoisted bindings are prepended so field reads happen before the if-chain.
        *code = if !hoisted_bindings.is_empty() || preamble.is_some() {
            let mut stmts = Vec::new();
            if let Some((v, init)) = preamble {
                stmts.push(v_set(v, init));
            }
            stmts.append(&mut hoisted_bindings);
            stmts.push(chain);
            v_block(stmts, result_type.clone(), "match")
        } else {
            chain
        };
        result_type
    }

    /// Parse a wildcard (`_`) arm in a match expression.
    /// Returns the arm and whether it is exhaustive (no guard).
    fn parse_match_wildcard_arm(&mut self, result_type: &mut Type) -> (EnumArm, bool) {
        let guard_opt = self.parse_optional_guard();
        let is_exhaustive = guard_opt.is_none();
        self.expect_match_arm_arrow();
        let mut arm_code = Value::Null;
        let arm_type = if self.lexer.peek_token("{") {
            self.parse_block("match_arm", &mut arm_code, &Type::Unknown(0))
        } else {
            self.expression(&mut arm_code)
        };
        if *result_type == Type::Void {
            *result_type = arm_type.clone();
        } else if !self.first_pass
            && arm_type != Type::Void
            && !match_arm_types_unify(result_type, &arm_type)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "cannot unify: {} and {}",
                result_type.name(&self.data),
                arm_type.name(&self.data)
            );
        }
        let arm = EnumArm {
            discs: vec![],
            code: arm_code,
            tp: arm_type,
            guard: guard_opt,
            bindings: Vec::new(),
        };
        (arm, is_exhaustive)
    }

    /// Parse a plain-struct match arm (field bindings + body).
    /// Returns the arm and whether it is exhaustive.
    fn parse_match_struct_arm(
        &mut self,
        e_nr: u32,
        subject_val: &Value,
        result_type: &mut Type,
        hoisted_bindings: &mut Vec<Value>,
    ) -> (EnumArm, bool) {
        let mut field_conditions: Vec<Value> = Vec::new();
        if self.lexer.peek_token("{") {
            self.lexer.token("{");
            while !self.lexer.peek_token("}") {
                if let Some(field_name) = self.lexer.has_identifier() {
                    let attr_idx = self.data.attr(e_nr, &field_name);
                    if attr_idx != usize::MAX {
                        let field_val = self.get_field(e_nr, attr_idx, subject_val.clone());
                        let field_type = self.data.attr_type(e_nr, attr_idx);
                        if self.lexer.has_token(":") {
                            if let Some(cond) = self.parse_field_sub_pattern(field_val, &field_type)
                            {
                                field_conditions.push(cond);
                            }
                        } else {
                            let v = self.create_var(&field_name, &field_type);
                            self.vars.defined(v);
                            hoisted_bindings.push(v_set(v, field_val));
                        }
                    } else if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "unknown field '{}' on struct {}",
                            field_name,
                            self.data.def(e_nr).name()
                        );
                    }
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("}");
        }
        self.expect_match_arm_arrow();
        let mut arm_code = Value::Null;
        let arm_type = if self.lexer.peek_token("{") {
            self.parse_block("match_arm", &mut arm_code, &Type::Unknown(0))
        } else {
            self.expression(&mut arm_code)
        };
        let block = v_block(vec![arm_code], arm_type.clone(), "struct_match");
        if *result_type == Type::Void {
            *result_type = arm_type;
        }
        let (guard, exhaustive) = if field_conditions.is_empty() {
            (None, true)
        } else {
            let mut combined = field_conditions.remove(0);
            for c in field_conditions {
                combined = v_if(combined, c, Value::Boolean(false));
            }
            (Some(combined), false)
        };
        let arm = EnumArm {
            discs: vec![],
            code: block,
            tp: result_type.clone(),
            guard,
            bindings: Vec::new(),
        };
        (arm, exhaustive)
    }

    /// #429: the frame var a match-arm field binding BORROWS from — the
    /// subject's backing variable.  A heap match binding (`CMap { entries }`)
    /// is a DbRef into the subject's record, so its type must carry a borrow
    /// dep on this var (see the call site).  Returns `Some(var)` only when the
    /// subject reduces to a plain `Var` (the common `match m { … }` /
    /// `match self.f { … }`-into-a-temp case, where `subject_val` is the
    /// match temp or the parameter itself); a non-`Var` subject (a raw inline
    /// field/index/call expression with no single backing var) yields `None`,
    /// leaving the binding dep-free exactly as before.
    fn match_borrow_source(subject_val: &Value) -> Option<u16> {
        match subject_val.unspan() {
            Value::Var(v) => Some(*v),
            _ => None,
        }
    }

    /// Parse field bindings for a struct-enum match arm.
    fn parse_match_enum_field_bindings(
        &mut self,
        variant_def_nr: u32,
        pattern_name: &str,
        subject_val: &Value,
        arm_stmts: &mut Vec<Value>,
        field_conditions: &mut Vec<Value>,
        name_aliases: &mut Vec<(String, Option<u16>)>,
    ) {
        self.lexer.token("{");
        let mut seen_fields: HashSet<String> = HashSet::new();
        while let Some(field_name) = self.lexer.has_identifier() {
            if !self.first_pass && seen_fields.contains(&field_name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "duplicate field binding '{}' in match arm",
                    field_name
                );
            }
            seen_fields.insert(field_name.clone());

            let attr_idx_and_type = {
                let variant_def = self.data.def(variant_def_nr);
                variant_def.attributes[1..]
                    .iter()
                    .enumerate()
                    .find(|(_, a)| a.name == field_name)
                    .map(|(i, a)| (i + 1, a.typedef.clone()))
            };

            match attr_idx_and_type {
                Some((attr_idx, field_type)) => {
                    let field_read = self.get_field(variant_def_nr, attr_idx, subject_val.clone());
                    if self.lexer.has_token(":") {
                        if let Some(cond) = self.parse_field_sub_pattern(field_read, &field_type) {
                            field_conditions.push(cond);
                        }
                    } else {
                        let v_nr = self.create_unique(&format!("mv_{field_name}"), &field_type);
                        if v_nr != u16::MAX {
                            self.vars.defined(v_nr);
                            arm_stmts.push(v_set(v_nr, field_read));
                            let old = self.vars.set_name(&field_name, v_nr);
                            name_aliases.push((field_name.clone(), old));
                            // B5 remaining half (2026-04-14): match-arm
                            // bindings are field extractions from the
                            // subject — the subject owns the store and
                            // the binding is a borrowed view (a DbRef
                            // pointing into the subject's record).
                            // Emitting OpFreeRef for the binding at
                            // function exit would decrement a store the
                            // binding doesn't own; worse, if the arm
                            // wasn't taken the slot is never assigned
                            // and the free reads garbage bytes as a
                            // DbRef (observed as out-of-bounds store_nr
                            // ≈ 4621 in `p54_b5_recursive_struct_enum`).
                            // Mark the binding `skip_free` so scope
                            // cleanup leaves it alone in both the
                            // taken and not-taken arms.
                            self.vars.set_skip_free(v_nr);
                            // #429: the binding is a BORROWED VIEW of the
                            // subject, so its TYPE must record that borrow —
                            // otherwise a value derived from it and returned
                            // (`CMap { entries } => { r = entries[..]; return r }`)
                            // breaks the borrow chain at the binding: `ref_return`
                            // walks `r` → `entries` → <this binding> and stops (the
                            // binding has empty deps), never reaching the subject
                            // parameter, so the fn is mis-classified OWNED and the
                            // caller whole-store-frees the subject's record (#429
                            // interp-vs-native divergence).  Give a HEAP
                            // (DbRef-carrying) binding a frame dep on the subject's
                            // source var so the chain reaches the parameter — exactly
                            // the `["src"]` dep a `b = subj.field` bind already
                            // carries.  Scalars hold no DbRef, so they need no borrow
                            // dep (the `_mv_value` integer binding stays dep-free).
                            if matches!(
                                &field_type,
                                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                            ) && let Some(src) = Self::match_borrow_source(subject_val)
                            {
                                let bound_tp = match self.vars.tp(v_nr).clone() {
                                    Type::Reference(td, _) => {
                                        Type::Reference(td, crate::data::Deps::frame1(src))
                                    }
                                    Type::Vector(it, _) => {
                                        Type::Vector(it, crate::data::Deps::frame1(src))
                                    }
                                    Type::Enum(td, su, _) => {
                                        Type::Enum(td, su, crate::data::Deps::frame1(src))
                                    }
                                    other => other,
                                };
                                self.vars.set_type(v_nr, bound_tp);
                            }
                        }
                    }
                }
                None => {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "variant {} has no field '{}'",
                            pattern_name,
                            field_name
                        );
                    }
                }
            }

            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("}");
    }

    /// Parse an optional `if <expr>` guard clause.
    fn parse_optional_guard(&mut self) -> Option<Value> {
        if self.lexer.has_token("if") {
            let mut guard_code = Value::Null;
            let guard_type = self.expression(&mut guard_code);
            if !self.first_pass && guard_type != Type::Boolean {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "guard must be boolean, got {}",
                    guard_type.name(&self.data)
                );
            }
            Some(guard_code)
        } else {
            None
        }
    }

    /// Parse a sub-pattern in a match field position (L2).
    /// Given a field value expression and its type, returns a boolean condition.
    /// Handles: enum variant names, scalar literals, ranges, `_` (wildcard).
    fn parse_field_sub_pattern(&mut self, field_val: Value, field_type: &Type) -> Option<Value> {
        // Enum field: the sub-pattern is a variant name (or `_`).
        if let Type::Enum(e_nr, false, _) = field_type
            && let Some(name) = self.lexer.has_identifier()
        {
            // Wildcard — no condition.
            if name == "_" {
                return None;
            }
            // Look up variant discriminant.
            let disc = if let Some(a_nr) = self.data.def(*e_nr).attr_names.get(&name) {
                if let Value::Enum(nr, _) = self.data.def(*e_nr).attributes()[*a_nr].value {
                    i32::from(nr)
                } else {
                    0
                }
            } else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        name,
                        self.data.def(*e_nr).name()
                    );
                }
                return None;
            };
            // Build equality: field_val == Enum(disc)
            let variant_val = Value::Enum(disc as u8, *e_nr as u16);
            let mut cond = Value::Null;
            self.call_op(
                &mut cond,
                "==",
                &[field_val.clone(), variant_val],
                &[field_type.clone(), field_type.clone()],
            );
            // or-pattern: Paid | Refunded
            while self.lexer.has_token("|") {
                if let Some(next_name) = self.lexer.has_identifier() {
                    let next_disc = if let Some(a_nr) =
                        self.data.def(*e_nr).attr_names.get(&next_name)
                    {
                        if let Value::Enum(nr, _) = self.data.def(*e_nr).attributes()[*a_nr].value {
                            i32::from(nr)
                        } else {
                            0
                        }
                    } else {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "'{}' is not a variant of {}",
                                next_name,
                                self.data.def(*e_nr).name()
                            );
                        }
                        0
                    };
                    let next_variant = Value::Enum(next_disc as u8, *e_nr as u16);
                    let mut next_cond = Value::Null;
                    self.call_op(
                        &mut next_cond,
                        "==",
                        &[field_val.clone(), next_variant],
                        &[field_type.clone(), field_type.clone()],
                    );
                    // OR: if first matches → true, else check next.
                    cond = v_if(cond, Value::Boolean(true), next_cond);
                }
            }
            return Some(cond);
        }
        // Wildcard for non-enum fields.
        if matches!(&self.lexer.peek().has, LexItem::Identifier(id) if id == "_") {
            self.lexer.has_identifier(); // consume the `_`
            return None;
        }
        // Scalar field: store in a temp and use parse_match_pattern.
        let tmp = self.create_unique("fp_subj", field_type);
        self.vars.defined(tmp);
        let (pat, pat_type) = self.parse_match_pattern(field_type, tmp);
        // If parse_match_pattern returned a Block (range pattern or null pattern),
        // use it directly as a condition.
        if matches!(pat_type, Type::Boolean) || matches!(pat, Value::Block(_)) {
            return Some(v_block(
                vec![v_set(tmp, field_val), pat],
                Type::Boolean,
                "field_sub",
            ));
        }
        // Otherwise it's a literal — generate an equality comparison.
        let mut eq = Value::Null;
        self.call_op(
            &mut eq,
            "==",
            &[Value::Var(tmp), pat],
            &[field_type.clone(), field_type.clone()],
        );
        Some(v_block(
            vec![v_set(tmp, field_val), eq],
            Type::Boolean,
            "field_sub",
        ))
    }

    /// Parse a match pattern literal (integer, float, text, boolean) and optionally
    /// a range suffix `..` or `..=`. Returns the pattern Value and its type.
    fn parse_match_pattern(&mut self, subject_type: &Type, subject_var: u16) -> (Value, Type) {
        // INC#31: reject open-start ranges (`..hi =>`) in match arms with a
        // useful diagnostic.  The range-pattern codegen further down assumes
        // both `lo` and `hi` are real values — an absent `lo` would be
        // silently encoded as Value::Null and either never match
        // (interpreter) or crash native codegen (E0308: `()` vs i32).
        if self.lexer.peek_token("..") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "open-ended range pattern `..hi` is not supported in match arms — \
                 write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
                 or use a guard like `n if n < hi`"
            );
            // Consume the `..` so the rest of the arm parses cleanly.
            self.lexer.token("..");
            self.lexer.has_token("=");
            let mut hi = Value::Null;
            self.expression(&mut hi);
            return (Value::Boolean(false), Type::Boolean);
        }
        let mut lit = Value::Null;
        let negate = self.lexer.has_token("-");
        let lit_type = if let Some(n) = self.lexer.has_integer() {
            let v = n as i32;
            lit = Value::Int(if negate { -v } else { v });
            Type::Integer(IntegerSpec::signed32())
        } else if let Some(n) = self.lexer.has_long() {
            let v = n as i64;
            lit = Value::Long(if negate { -v } else { v });
            crate::data::I64.clone()
        } else if let Some(n) = self.lexer.has_float() {
            lit = Value::Float(if negate { -n } else { n });
            Type::Float
        } else if let Some(s) = self.lexer.has_cstring() {
            lit = Value::Text(s);
            Type::Text(Deps::none())
        } else if let Some(c) = self.lexer.has_char() {
            lit = self.cl("OpConvCharacterFromInt", &[Value::Int(c as i32)]);
            Type::Character
        } else if self.lexer.has_token("true") {
            lit = Value::Boolean(true);
            Type::Boolean
        } else if self.lexer.has_token("false") {
            lit = Value::Boolean(false);
            Type::Boolean
        } else {
            self.expression(&mut lit)
        };
        if !self.first_pass && lit_type != Type::Null && !lit_type.is_same(subject_type) {
            self.can_convert(&lit_type, subject_type);
        }
        // check for range pattern `lo..hi` or `lo..=hi`.
        if self.lexer.has_token("..") {
            let inclusive = self.lexer.has_token("=");
            // INC#31: reject open-end range `lo..` in match arms — same
            // silent-never-matches / native-codegen-crash trap as open-start.
            if self.lexer.peek_token("=>")
                || self.lexer.peek_token("|")
                || self.lexer.peek_token("if")
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "open-ended range pattern `lo..` is not supported in match arms — \
                     write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
                     or use a guard like `n if n >= lo`"
                );
                return (Value::Boolean(false), Type::Boolean);
            }
            let mut hi = Value::Null;
            self.expression(&mut hi);
            let mut lo_cond = Value::Null;
            self.call_op(
                &mut lo_cond,
                "<=",
                &[lit, Value::Var(subject_var)],
                &[subject_type.clone(), subject_type.clone()],
            );
            let mut hi_cond = Value::Null;
            self.call_op(
                &mut hi_cond,
                if inclusive { "<=" } else { "<" },
                &[Value::Var(subject_var), hi],
                &[subject_type.clone(), subject_type.clone()],
            );
            let range_cond = v_if(lo_cond, hi_cond, Value::Boolean(false));
            (
                v_block(vec![range_cond], Type::Boolean, "range_pattern"),
                Type::Boolean,
            )
        } else {
            (lit, lit_type)
        }
    }

    /// Parse a match expression over a scalar (integer, text, boolean, etc.).
    /// Builds an if/else chain: `if subject == lit1 { arm1 } else if subject == lit2 { arm2 } else { wildcard }`
    #[allow(clippy::too_many_lines)] // match-arm dispatch with pattern/guard/binding logic
    fn parse_scalar_match(
        &mut self,
        subject: Value,
        subject_type: &Type,
        code: &mut Value,
    ) -> Type {
        // Store subject in a temp var to avoid re-evaluation.
        let v = self.create_unique("match_subj", subject_type);
        self.vars.defined(v);

        self.lexer.token("{");

        // Collect arms: (literal_value, arm_code, arm_type, optional guard)
        let mut arms: Vec<(Option<Value>, Value, Type, Option<Value>)> = Vec::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;

        loop {
            if self.lexer.peek_token("}") {
                break;
            }

            // Parse pattern: literal, `true`, `false`, `_`, `name @ pattern`, or string.
            let mut pattern_val: Option<Value> = None;
            let mut is_wildcard = false;
            let mut arm_bindings: Vec<Value> = Vec::new();

            // null pattern — matches when subject is null.
            if self.lexer.has_token("null") {
                let mut null_cond = Value::Null;
                self.call_op(
                    &mut null_cond,
                    "!",
                    &[Value::Var(v)],
                    std::slice::from_ref(subject_type),
                );
                // Wrap as a Block so build_scalar_chain recognizes it as a pre-built condition.
                pattern_val = Some(v_block(vec![null_cond], Type::Boolean, "null_pattern"));
            // Check for wildcard `_` or binding `name @ pattern`.
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    is_wildcard = true;
                } else if self.lexer.has_token("@") {
                    // binding pattern `name @ pattern` — bind the subject to
                    // a variable and continue parsing the sub-pattern.
                    let bind_nr = self.vars.add_variable(&id, subject_type, &mut self.lexer);
                    self.vars.defined(bind_nr);
                    arm_bindings.push(v_set(bind_nr, Value::Var(v)));
                    // Parse the sub-pattern after `@`.
                    let (pat, _) = self.parse_match_pattern(subject_type, v);
                    pattern_val = Some(pat);
                } else {
                    // Bare identifier without `@` — wildcard binding (binds subject to name).
                    let bind_nr = self.vars.add_variable(&id, subject_type, &mut self.lexer);
                    self.vars.defined(bind_nr);
                    arm_bindings.push(v_set(bind_nr, Value::Var(v)));
                    is_wildcard = true;
                }
            } else {
                let (pat, _) = self.parse_match_pattern(subject_type, v);
                pattern_val = Some(pat);
            }

            // or-patterns in scalar match — `1 | 2 | 3 => ...`
            while self.lexer.has_token("|") && !is_wildcard {
                let (next_pat, _) = self.parse_match_pattern(subject_type, v);
                if let Some(prev) = pattern_val.take() {
                    // Combine: build equality condition for prev, equality for next,
                    // then OR them: If(prev_eq, true, next_eq).
                    let mut prev_cond = Value::Null;
                    self.build_scalar_cond(&mut prev_cond, v, subject_type, prev);
                    let mut next_cond = Value::Null;
                    self.build_scalar_cond(&mut next_cond, v, subject_type, next_pat);
                    let or_cond = v_if(prev_cond, Value::Boolean(true), next_cond);
                    pattern_val = Some(v_block(vec![or_cond], Type::Boolean, "or_pattern"));
                }
            }

            // parse optional guard clause.
            let mut guard_opt = if self.lexer.has_token("if") {
                let mut guard_code = Value::Null;
                let guard_type = self.expression(&mut guard_code);
                if !self.first_pass && guard_type != Type::Boolean {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "guard must be boolean, got {}",
                        guard_type.name(&self.data)
                    );
                }
                Some(guard_code)
            } else {
                None
            };

            // Only mark exhaustive if wildcard has no guard.
            if is_wildcard && guard_opt.is_none() {
                has_wildcard = true;
            }

            self.expect_match_arm_arrow();
            let mut arm_code = Value::Null;
            let arm_type = if self.lexer.peek_token("{") {
                self.parse_block("match_arm", &mut arm_code, &Type::Unknown(0))
            } else {
                self.expression(&mut arm_code)
            };
            // A `null`-first arm must NOT pin the result to `Null` — promote to
            // the first CONCRETE arm's type (else `match c { false => null, true
            // => S{…} }` resolves to `Null`, `build_scalar_chain` can't type the
            // null sentinel, and the value arm returns null — silently wrong).
            if result_type == Type::Void || result_type == Type::Null {
                result_type = arm_type.clone();
            }
            // P209 — when the arm has both a guard and pattern bindings
            // (e.g. `x if x < 0 => …`), the guard must see the bound
            // variable.  Prepend the binding assignments to the guard
            // expression so the bound name is initialised before the
            // guard reads it.  Without this the guard saw the
            // uninitialised slot (typically 0), causing `x if x < 0`
            // to mis-fire and either skip the arm (interp) or fall
            // through to a sibling guard (`x == 0`) silently.  The
            // enum-variant struct-field path at the call site of
            // `build_scalar_chain` already wraps guards this way.
            if !arm_bindings.is_empty()
                && let Some(guard) = guard_opt.take()
            {
                let mut stmts = arm_bindings.clone();
                stmts.push(guard);
                guard_opt = Some(v_block(stmts, Type::Boolean, "binding_guard"));
            }
            // prepend any binding assignments (from `name @ pattern` or bare `name`)
            // to the arm body so the variable is assigned before the body executes.
            if !arm_bindings.is_empty() {
                arm_bindings.push(arm_code);
                arm_code = v_block(arm_bindings, arm_type.clone(), "binding_arm");
            }
            arms.push((pattern_val, arm_code, arm_type, guard_opt));
            if has_wildcard {
                self.lexer.has_token(","); // optional trailing comma
                break;
            }
            if self.lexer.peek_token("}") {
                self.lexer.has_token(","); // optional trailing comma
            } else {
                self.lexer.token(","); // comma required between arms
            }
        }
        self.lexer.token("}");

        let chain = self.build_scalar_chain(v, subject_type, has_wildcard, &result_type, arms);
        *code = v_block(
            vec![v_set(v, subject), chain],
            result_type.clone(),
            "scalar_match",
        );
        result_type
    }

    /// Parse a match expression over a vector subject.
    /// Slice patterns: `[a, b] =>`, `[first, ..] =>`, `[.., last] =>`, `_ =>`.
    /// Each arm generates a length check and element bindings.
    #[allow(clippy::too_many_lines)] // slice pattern parsing with head/tail/rest dispatch
    fn parse_vector_match(
        &mut self,
        subject: Value,
        subject_type: &Type,
        code: &mut Value,
    ) -> Type {
        let elm_tp = subject_type.content();
        let v = self.create_unique("match_subj", subject_type);
        self.vars.defined(v);
        let elm_size = Value::Int(self.element_store_size(&elm_tp));

        self.lexer.token("{");
        let mut result_type = Type::Void;
        let mut arms: Vec<(Option<Value>, Value, Type)> = Vec::new();
        let mut has_wildcard = false;
        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            let mut bindings: Vec<Value> = Vec::new();
            let mut cond: Option<Value> = None;
            if self.lexer.has_token("[") {
                // Parse slice pattern elements
                let mut head: Vec<String> = Vec::new();
                let mut tail: Vec<String> = Vec::new();
                let mut has_rest = false;
                loop {
                    if self.lexer.has_token("]") {
                        break;
                    }
                    if self.lexer.has_token("..") {
                        has_rest = true;
                    } else if let Some(id) = self.lexer.has_identifier() {
                        if has_rest {
                            tail.push(id);
                        } else {
                            head.push(id);
                        }
                    } else if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "expected identifier or '..' in slice pattern"
                        );
                        break;
                    }
                    self.lexer.has_token(",");
                }
                let fixed = (head.len() + tail.len()) as i32;
                // Generate length condition
                let len_call = self.cl("OpLengthVector", &[Value::Var(v)]);
                if has_rest {
                    // length >= fixed  →  fixed <= length
                    self.call_op(
                        cond.get_or_insert(Value::Null),
                        "<=",
                        &[Value::Int(fixed), len_call],
                        &[
                            Type::Integer(IntegerSpec {
                                min: 0,
                                max: 0,
                                not_null: false,
                                forced_size: None,
                            }),
                            Type::Integer(IntegerSpec {
                                min: 0,
                                max: 0,
                                not_null: false,
                                forced_size: None,
                            }),
                        ],
                    );
                } else {
                    // length == fixed
                    self.call_op(
                        cond.get_or_insert(Value::Null),
                        "==",
                        &[len_call, Value::Int(fixed)],
                        &[
                            Type::Integer(IntegerSpec {
                                min: 0,
                                max: 0,
                                not_null: false,
                                forced_size: None,
                            }),
                            Type::Integer(IntegerSpec {
                                min: 0,
                                max: 0,
                                not_null: false,
                                forced_size: None,
                            }),
                        ],
                    );
                }
                // Bind head elements: head[i] = v[i]
                for (i, name) in head.iter().enumerate() {
                    if name == "_" {
                        continue;
                    }
                    let bind_nr = self.vars.add_variable(name, &elm_tp, &mut self.lexer);
                    self.vars.defined(bind_nr);
                    let get = self.cl(
                        "OpGetVector",
                        &[Value::Var(v), elm_size.clone(), Value::Int(i as i32)],
                    );
                    let val = self.get_field(self.data.type_def_nr(&elm_tp), usize::MAX, get);
                    bindings.push(v_set(bind_nr, val));
                }
                // Bind tail elements: tail[j] = v[len - tail.len() + j]
                for (j, name) in tail.iter().enumerate() {
                    if name == "_" {
                        continue;
                    }
                    let bind_nr = self.vars.add_variable(name, &elm_tp, &mut self.lexer);
                    self.vars.defined(bind_nr);
                    let idx = Value::Int(-((tail.len() - j) as i32));
                    let get = self.cl("OpGetVector", &[Value::Var(v), elm_size.clone(), idx]);
                    let val = self.get_field(self.data.type_def_nr(&elm_tp), usize::MAX, get);
                    bindings.push(v_set(bind_nr, val));
                }
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    has_wildcard = true;
                } else {
                    // bare name — wildcard binding
                    let bind_nr = self.vars.add_variable(&id, subject_type, &mut self.lexer);
                    self.vars.defined(bind_nr);
                    bindings.push(v_set(bind_nr, Value::Var(v)));
                    has_wildcard = true;
                }
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "expected slice pattern '[...]' or '_' in vector match arm"
                );
                break;
            }
            // Parse guard
            let guard_opt = if self.lexer.has_keyword("if") {
                let mut guard = Value::Null;
                let gt = self.expression(&mut guard);
                if !self.first_pass && gt != Type::Boolean {
                    self.convert(&mut guard, &gt, &Type::Boolean);
                }
                Some(guard)
            } else {
                None
            };
            self.expect_match_arm_arrow();
            let mut arm_code = Value::Null;
            let arm_type = self.expression(&mut arm_code);
            if result_type == Type::Void {
                result_type = arm_type.clone();
            }
            // Prepend bindings
            if !bindings.is_empty() {
                bindings.push(arm_code);
                arm_code = v_block(bindings, arm_type.clone(), "slice_binding");
            }
            // Combine condition with guard
            let full_cond = match (cond, guard_opt) {
                (Some(c), Some(g)) => Some(self.op("&&", c, g, Type::Boolean)),
                (Some(c), None) => Some(c),
                (None, Some(g)) => Some(g),
                (None, None) => None,
            };
            arms.push((full_cond, arm_code, arm_type));
            if has_wildcard {
                self.lexer.has_token(",");
                break;
            }
            if self.lexer.peek_token("}") {
                self.lexer.has_token(",");
            } else {
                self.lexer.token(",");
            }
        }
        self.lexer.token("}");

        // Build if-else chain from arms
        let fallback = if has_wildcard {
            let (_, arm_code, _) = arms.pop().unwrap();
            arm_code
        } else {
            self.null(&result_type)
        };
        let mut chain = fallback;
        for (cond_opt, arm_code, _) in arms.into_iter().rev() {
            if let Some(cond) = cond_opt {
                chain = v_if(cond, arm_code, chain);
            } else {
                chain = arm_code;
            }
        }
        *code = v_block(
            vec![v_set(v, subject), chain],
            result_type.clone(),
            "vector_match",
        );
        result_type
    }

    /// Parse a `match` expression whose subject is a `Type::Tuple`.
    ///
    /// Arm syntax: `_ => expr` (wildcard) or `(pat0, pat1, ...) => expr` (element patterns).
    /// Element patterns: `_` (wildcard), `identifier` (binding), or a literal value.
    /// Arms are separated by `,` or `;` (optional after the last arm).
    #[allow(clippy::too_many_lines)]
    fn parse_tuple_match(&mut self, subject: Value, subject_type: &Type, code: &mut Value) -> Type {
        let Type::Tuple(elem_types) = subject_type else {
            unreachable!("parse_tuple_match called with non-tuple subject")
        };
        let elem_types = elem_types.clone();
        let arity = elem_types.len();

        // Store the tuple in a temp var so elements can be read multiple times.
        let tmp = self.create_unique("match_tuple", subject_type);
        self.vars.defined(tmp);

        self.lexer.token("{");

        // arms: (Option<cond>, arm_body, arm_type, Option<guard>)
        let mut arms: Vec<(Option<Value>, Value, Type, Option<Value>)> = Vec::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;

        loop {
            if self.lexer.peek_token("}") {
                break;
            }

            let mut is_wildcard = false;
            let mut bindings: Vec<Value> = Vec::new();
            let mut elem_conds: Vec<Value> = Vec::new();

            if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    is_wildcard = true;
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expected '_' or a tuple pattern '(...)' in tuple match"
                    );
                }
            } else if self.lexer.has_token("(") {
                // Element-by-element pattern
                for (i, elem_type) in elem_types.iter().enumerate().take(arity) {
                    if i > 0 && !self.lexer.has_token(",") && !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "expected ',' between tuple pattern elements"
                        );
                        break;
                    }
                    let elem_type = elem_type.clone();
                    let elem_get = Value::TupleGet(tmp, i as u16);
                    if let Some(id) = self.lexer.has_identifier() {
                        if id == "_" {
                            // element wildcard — no condition, no binding
                        } else {
                            // binding variable — always matches, captures element value
                            let bind_nr = self.vars.add_variable(&id, &elem_type, &mut self.lexer);
                            self.vars.defined(bind_nr);
                            bindings.push(v_set(bind_nr, elem_get));
                        }
                    } else {
                        // literal: build elem_get == literal condition
                        let negate = self.lexer.has_token("-");
                        let lit: Value = if let Some(n) = self.lexer.has_integer() {
                            let v = n as i32;
                            Value::Int(if negate { -v } else { v })
                        } else if let Some(n) = self.lexer.has_long() {
                            let v = n as i64;
                            Value::Long(if negate { -v } else { v })
                        } else if let Some(n) = self.lexer.has_float() {
                            Value::Float(if negate { -n } else { n })
                        } else if let Some(s) = self.lexer.has_cstring() {
                            Value::Text(s)
                        } else if self.lexer.has_token("true") {
                            Value::Boolean(true)
                        } else if self.lexer.has_token("false") {
                            Value::Boolean(false)
                        } else {
                            let mut e = Value::Null;
                            self.expression(&mut e);
                            e
                        };
                        let mut elem_cond = Value::Null;
                        self.call_op(
                            &mut elem_cond,
                            "==",
                            &[elem_get, lit],
                            &[elem_type.clone(), elem_type],
                        );
                        elem_conds.push(elem_cond);
                    }
                }
                if !self.lexer.has_token(")") && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expected ')' to close tuple pattern"
                    );
                }
                // All element positions were wildcards/bindings with no literal conditions.
                // The arm is effectively unconditional (wildcard) when there are no bindings
                // either; if there are bindings it acts like a wildcard-with-capture.
                if elem_conds.is_empty() && bindings.is_empty() {
                    is_wildcard = true;
                }
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "expected '_' or a tuple pattern '(...)' in tuple match"
                );
            }

            // Optional guard clause
            let guard_opt = if self.lexer.has_keyword("if") {
                let mut g = Value::Null;
                let gt = self.expression(&mut g);
                if !self.first_pass && gt != Type::Boolean {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "guard must be boolean, got {}",
                        gt.name(&self.data)
                    );
                }
                Some(g)
            } else {
                None
            };

            if is_wildcard && guard_opt.is_none() {
                has_wildcard = true;
            }

            self.expect_match_arm_arrow();

            let arm_write_state = self.vars.save_and_clear_write_state();
            self.vars.clear_write_state();
            let mut arm_body = Value::Null;
            let arm_type = if self.lexer.peek_token("{") {
                self.parse_block("match_arm", &mut arm_body, &Type::Unknown(0))
            } else {
                self.expression(&mut arm_body)
            };
            self.vars.restore_write_state(&arm_write_state);

            // Combine element conditions with AND (short-circuit: if a { b } else { false })
            let cond: Option<Value> = if elem_conds.is_empty() {
                None
            } else {
                let mut combined = elem_conds.remove(0);
                for c in elem_conds {
                    combined = v_if(combined, c, Value::Boolean(false));
                }
                Some(combined)
            };

            // Combine condition with guard
            let full_cond = match (cond, guard_opt) {
                (Some(c), Some(g)) => Some(v_if(c, g, Value::Boolean(false))),
                (Some(c), None) => Some(c),
                (None, Some(g)) => Some(g),
                (None, None) => None,
            };

            // Prepend bindings to arm body
            let arm_body = if bindings.is_empty() {
                arm_body
            } else {
                bindings.push(arm_body);
                v_block(bindings, arm_type.clone(), "tuple_binding")
            };

            if result_type == Type::Void {
                result_type = arm_type.clone();
            }
            arms.push((full_cond, arm_body, arm_type, None));

            if has_wildcard {
                self.lexer.has_token(",");
                self.lexer.has_token(";");
                break;
            }
            if self.lexer.peek_token("}") {
                self.lexer.has_token(",");
                self.lexer.has_token(";");
            } else {
                // optional arm separator
                self.lexer.has_token(",");
                self.lexer.has_token(";");
            }
        }
        self.lexer.token("}");

        // Build if-else chain (last arm is fallback / wildcard)
        let fallback = if has_wildcard {
            let (_, arm_code, _, _) = arms.pop().unwrap();
            arm_code
        } else {
            self.null(&result_type)
        };
        let mut chain = fallback;
        for (cond_opt, arm_code, _, _) in arms.into_iter().rev() {
            chain = if let Some(cond) = cond_opt {
                v_if(cond, arm_code, chain)
            } else {
                arm_code
            };
        }

        *code = v_block(
            vec![v_set(tmp, subject), chain],
            result_type.clone(),
            "tuple_match",
        );
        result_type
    }

    /// Build a boolean condition for a single scalar pattern value.
    fn build_scalar_cond(&mut self, cond: &mut Value, v: u16, subject_type: &Type, pat: Value) {
        // Reuse the same logic as build_scalar_chain for special block patterns.
        if let Value::Block(ref bl) = pat
            && bl.result == Type::Boolean
            && (bl.name == "range_pattern" || bl.name == "null_pattern" || bl.name == "or_pattern")
        {
            *cond = bl.operators[0].clone();
            return;
        }
        self.call_op(
            cond,
            "==",
            &[Value::Var(v), pat],
            &[subject_type.clone(), subject_type.clone()],
        );
    }

    /// Build the if-chain for a scalar match from collected arms.
    fn build_scalar_chain(
        &mut self,
        v: u16,
        subject_type: &Type,
        has_wildcard: bool,
        result_type: &Type,
        mut arms: Vec<(Option<Value>, Value, Type, Option<Value>)>,
    ) -> Value {
        // A bare `null` arm value (`false => null`) parses to `Value::Null`, which
        // lowers to NO push (Type::Void).  In a value-producing match the if-chain
        // join then reads an unwritten, value-sized slot — interp stack underflow
        // ("No elements left on the stack"), native a lost value.  Convert each
        // bare-null arm to the result type's typed null sentinel, the same
        // transform `parse_if` applies to a null branch (~line 1250) and the
        // fallback gets just below.  `self.null` is a no-op (returns
        // `Value::Null`) for Void/Unknown result types, so a statement-style
        // match is untouched.  Compute the typed null once (it's the same for
        // every arm — `result_type` is fixed), releasing the `&mut self` borrow
        // before mutating `arms`.
        if arms.iter().any(|a| matches!(a.1, Value::Null)) {
            let typed_null = self.null(result_type);
            for arm in &mut arms {
                if matches!(arm.1, Value::Null) {
                    arm.1 = typed_null.clone();
                }
            }
        }
        let fallback = if has_wildcard {
            let (_, arm_code, _, _) = arms.pop().unwrap();
            arm_code
        } else {
            self.null(result_type)
        };

        let mut chain = fallback;
        for (pattern_val, arm_code, _, guard_opt) in arms.into_iter().rev() {
            if let Some(lit) = pattern_val {
                // range/null/or patterns stored as Block with Boolean result.
                if let Value::Block(ref bl) = lit
                    && bl.result == Type::Boolean
                    && (bl.name == "range_pattern"
                        || bl.name == "null_pattern"
                        || bl.name == "or_pattern")
                {
                    let range_cond = bl.operators[0].clone();
                    chain = match guard_opt {
                        Some(guard) => {
                            let guarded = v_if(guard, arm_code, chain.clone());
                            v_if(range_cond, guarded, chain)
                        }
                        None => v_if(range_cond, arm_code, chain),
                    };
                    continue;
                }
                let mut cond = Value::Null;
                let cond_tp = self.call_op(
                    &mut cond,
                    "==",
                    &[Value::Var(v), lit],
                    &[subject_type.clone(), subject_type.clone()],
                );
                if cond_tp == Type::Null {
                    chain = arm_code;
                } else {
                    chain = match guard_opt {
                        Some(guard) => {
                            let guarded = v_if(guard, arm_code, chain.clone());
                            v_if(cond, guarded, chain)
                        }
                        None => v_if(cond, arm_code, chain),
                    };
                }
            } else {
                // Wildcard or guarded wildcard (no pattern).
                chain = match guard_opt {
                    Some(guard) => v_if(guard, arm_code, chain),
                    None => arm_code,
                };
            }
        }
        chain
    }

    // <for> ::= <identifier> 'in' <expression> [ 'par' '(' <id> '=' <worker> ',' <threads> ')' ] '{' <block>
    //
    // The optional parallel clause `par(b=worker(a), N)` desugars to a parallel map
    // followed by an index-based loop over the results.  Three worker call forms
    // are supported — see `parse_parallel_for_loop` for details.
    /// Set up iterator variables for a for-loop header and return
    /// `(iter_var, pre_var, for_var, if_step, create_iter, iter_next)`.
    /// `expr is VariantName` — generates a boolean discriminant check.
    /// For plain enums: `OpConvIntFromEnum(expr) == disc`.
    /// For struct-enums: `OpConvIntFromEnum(OpGetEnum(expr, 0)) == disc`.
    pub(crate) fn parse_is_variant(
        &mut self,
        code: &mut Value,
        subject_type: &Type,
        variant_name: &str,
    ) -> Type {
        let (e_nr, is_struct) = match subject_type {
            Type::Enum(nr, true, _) => (*nr, true),
            Type::Enum(nr, false, _) => (*nr, false),
            // EnumValue variant type (e.g. `s = Circle { ... }` has type
            // Reference(Circle_def_nr) where Circle's parent is Shape).
            Type::Reference(d_nr, _)
                if self.data.def_type(*d_nr) == DefType::EnumValue
                    && matches!(
                        self.data.def(self.data.def(*d_nr).parent).returned(),
                        Type::Enum(_, true, _)
                    ) =>
            {
                (self.data.def(*d_nr).parent(), true)
            }
            // Reference to an Enum itself (e.g. loop variable from
            // vector<Shape> iteration gets Type::Reference(Shape_nr, _)).
            Type::Reference(d_nr, _)
                if self.data.def_type(*d_nr) == DefType::Enum
                    && matches!(self.data.def(*d_nr).returned(), Type::Enum(_, true, _)) =>
            {
                (*d_nr, true)
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'is' requires an enum type, got {}",
                        subject_type.name(&self.data)
                    );
                }
                return Type::Boolean;
            }
        };
        // @PLN22 Phase 1 — resolve the variant against the subject enum via the
        // variant_of chokepoint (the (enum, variant) scope key), not the bare
        // global def_nr.  `is` is always enum-typed here (see the match above).
        let variant_def_nr = self.data.variant_of(e_nr, variant_name);
        if variant_def_nr == u32::MAX || self.data.def_type(variant_def_nr) != DefType::EnumValue {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}' is not a variant of {}",
                    variant_name,
                    self.data.def(e_nr).name()
                );
            }
            return Type::Boolean;
        }
        let disc: i32 = if is_struct {
            let variant_attrs = self.data.def(variant_def_nr).attributes();
            if let Some(first) = variant_attrs.first()
                && let Value::Enum(nr, _) = first.value
            {
                i32::from(nr)
            } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(variant_name) {
                if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                    i32::from(nr)
                } else {
                    0
                }
            } else {
                0
            }
        } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(variant_name) {
            if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                i32::from(nr)
            } else {
                0
            }
        } else {
            0
        };
        let subject_clone = code.clone();
        let disc_expr = if is_struct {
            let get_enum = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
            self.cl("OpConvIntFromEnum", &[get_enum])
        } else {
            self.cl("OpConvIntFromEnum", std::slice::from_ref(code))
        };
        let disc_check = self.cl("OpEqInt", &[disc_expr, Value::Int(disc)]);
        let is_field_capture = is_struct && self.lexer.peek_token("{") && {
            let link = self.lexer.link();
            self.lexer.token("{");
            let is_capture = self.lexer.has_identifier().is_some()
                && (self.lexer.peek_token(",") || self.lexer.peek_token("}"));
            self.lexer.revert(link);
            is_capture
        };
        if is_field_capture {
            let mut condition: Vec<Value> = Vec::new();
            let stable_subject = if matches!(subject_clone, Value::Var(_)) {
                subject_clone
            } else {
                let tmp = self.create_unique("is_subj", subject_type);
                if tmp != u16::MAX {
                    self.vars.defined(tmp);
                    condition.push(v_set(tmp, subject_clone));
                }
                Value::Var(tmp)
            };
            self.lexer.token("{");
            let mut seen_fields: HashSet<String> = HashSet::new();
            while let Some(field_name) = self.lexer.has_identifier() {
                if !self.first_pass && seen_fields.contains(&field_name) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "duplicate field binding '{}' in is-capture",
                        field_name
                    );
                }
                seen_fields.insert(field_name.clone());
                let attr_idx_and_type = {
                    let variant_def = self.data.def(variant_def_nr);
                    variant_def.attributes[1..]
                        .iter()
                        .enumerate()
                        .find(|(_, a)| a.name == field_name)
                        .map(|(i, a)| (i + 1, a.typedef.clone()))
                };
                match attr_idx_and_type {
                    Some((attr_idx, field_type)) => {
                        let field_read =
                            self.get_field(variant_def_nr, attr_idx, stable_subject.clone());
                        let v_nr = self.create_unique(&format!("mv_{field_name}"), &field_type);
                        if v_nr != u16::MAX {
                            self.vars.defined(v_nr);
                            // The capture binds a borrowed view into the
                            // subject's record — scope cleanup must not
                            // emit OpFreeRef for it (see the same
                            // note at parse_match_enum_field_bindings in
                            // this file for the match-arm path).
                            self.vars.set_skip_free(v_nr);
                            self.is_capture_bindings.push(v_set(v_nr, field_read));
                            let old = self.vars.set_name(&field_name, v_nr);
                            self.is_capture_aliases.push((field_name.clone(), old));
                        }
                    }
                    None => {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "variant {} has no field '{}'",
                                self.data.def(variant_def_nr).name(),
                                field_name
                            );
                        }
                    }
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("}");
            if condition.is_empty() {
                *code = disc_check;
            } else {
                condition.push(disc_check);
                *code = Value::Insert(condition);
            }
        } else {
            *code = disc_check;
        }
        Type::Boolean
    }

    pub(crate) fn for_type(&mut self, in_type: &Type) -> Type {
        // unwrap &vector<T> so the element type resolves correctly.
        if let Type::RefVar(inner) = in_type {
            return self.for_type(inner);
        }
        if let Type::Vector(t_nr, dep) = &in_type {
            let mut t = *t_nr.clone();
            if let Type::Enum(nr, true, _) = t
                && !self.data.def(nr).name.starts_with("__nullable<")
            {
                // @PLN25 E2 — keep a synthetic `__nullable<S>` element in `Enum`
                // form for the loop variable: field access on `Type::Enum(.., true)`
                // unwraps to the `Some` variant via `find_poly_enum_field`
                // (fields.rs), whereas `Reference(enum_def)` does not (the enum
                // itself has no payload field) → "Unknown field __nullable<S>.f".
                // Hand-written struct-enums keep the Reference conversion (variant
                // field-access resolves against the variant def, not the parent).
                t = Type::Reference(nr, Deps::none());
            }
            // P189b: vector elements that are tuples live as inline bytes
            // in the vector record.  Iteration yields a 12-byte DbRef
            // pointing at those bytes; treat the loop var as a reference
            // to the synthetic `__tuple<...>` struct so per-element loads
            // happen through `OpVarRef` + `OpGet*(offset)` rather than the
            // stack-tuple `OpTupleGet` which would read DbRef bytes as
            // garbage integers.  parse_part recognises the def-name prefix
            // `__tuple<` and routes `.0` / `.1` to TupleGet IR.
            if let Type::Tuple(ref elems) = t {
                let elems_clone = elems.clone();
                let tuple_d = self.data.tuple_def(&mut self.lexer, &elems_clone);
                t = Type::Reference(tuple_d, Deps::none());
            }
            for d in dep {
                t = t.depending(*d);
            }
            t
        } else if let Type::Sorted(dnr, _, dep)
        | Type::Index(dnr, _, dep)
        | Type::Hash(dnr, _, dep) = &in_type
        {
            // C60 path 2c piece 2: hash iteration yields `reference<T>`,
            // same shape as Sorted/Index.  This is the parser-side
            // prerequisite before fill_iter (src/parser/fields.rs:599)
            // can flip the hash arm to `on = 4`.  Without this, for-loop
            // body parsing sees `e` as Type::Null and field access on
            // `e.name` fails with "Unknown type null".
            //
            // @PLN25 E2 — a synth `__nullable<S>` element keeps `Enum(.., true)`
            // so the loop body's field access unwraps through `Some` (mirrors
            // the Vector arm above and the `index_type` lookup path); without
            // it `e.field` errors "Unknown field __nullable<S>.field" because
            // the enum itself carries no payload field.  Inert gate-off (no
            // keyed element type is ever a `__nullable<` enum).
            if self.data.def(*dnr).name.starts_with("__nullable<") {
                Type::Enum(*dnr, true, dep.clone())
            } else {
                Type::Reference(*dnr, dep.clone())
            }
        } else if let Type::Iterator(i_tp, _) = &in_type {
            if **i_tp == Type::Null {
                I32.clone()
            } else {
                *i_tp.clone()
            }
        } else if let Type::Text(_) = in_type {
            Type::Character
        } else if let Type::Reference(_, _) | Type::Integer(_) = in_type {
            // I13: check for custom iterator protocol before falling back.
            let next_d_nr = self.data.find_fn(u16::MAX, "next", in_type);
            if next_d_nr != u32::MAX {
                return self.data.def(next_d_nr).returned().clone();
            }
            in_type.clone()
        } else if !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown in expression type {}",
                in_type.name(&self.data)
            );
            Type::Null
        } else {
            // First pass, iterable type not yet resolved — a forward or
            // cross-package reference whose definition registers later in the
            // recursion (#375: a dependency imported at a high source number is
            // parsed AFTER the importing package on pass 1).  Return Unknown,
            // not Null, so downstream field/method access on the loop variable
            // routes through the existing `Type::Unknown` defer-guard in
            // `field()` and DEFERS to pass 2, instead of hard-erroring
            // "Unknown type null" — which would abort pass 1 before the
            // dependency's definitions are registered, leaving pass 2 (which
            // would resolve cleanly) unreachable.
            Type::Unknown(0)
        }
    }

    pub(crate) fn text_return(&mut self, ls: &[u16]) {
        if let Type::Text(cur) = &self.data.definitions[self.context as usize].returned {
            let mut dep = cur.clone();
            for v in ls {
                let n = self.vars.name(*v);
                let tp = self.vars.tp(*v);
                // skip related variables that are already attributes
                if let Some(a) = self.data.def(self.context).attr_names.get(n) {
                    if !dep.contains(&(*a as u16)) {
                        dep.push(*a as u16);
                    }
                    continue;
                }
                // captured text variables are read from the closure record at
                // runtime — they must NOT be registered as hidden RefVar(Text) work-buffer
                // arguments.  Adding them would shift __closure to a wrong stack position.
                if self.captured_names.iter().any(|(name, _)| name == n) {
                    continue;
                }
                if matches!(tp, Type::Text(_)) {
                    // create a new attribute with this name
                    let a = self.data.add_attribute(
                        &mut self.lexer,
                        self.context,
                        n,
                        Type::RefVar(Box::new(Type::Text(Deps::none()))),
                    );
                    // @P387 zero-cost: mark the work-buffer HIDDEN so it rides the
                    // same adaptive hidden-return-buffer dispatch struct/vector use
                    // (`fn_call_ref` pushes one per hidden buf — 0 for a fn with no
                    // promotable local).  This replaces the static `cref_work_buf`
                    // injection and keeps the buffer out of the fn-ref TYPE without
                    // the deps-based exclusion that wrongly dropped returned params.
                    self.data.definitions[self.context as usize].attributes[a].hidden = true;
                    self.vars.become_argument(*v);
                    dep.push(a as u16);
                    self.vars
                        .set_type(*v, Type::RefVar(Box::new(Type::Text(Deps::none()))));
                } else if matches!(tp, Type::Tuple(_)) {
                    // @P330: a tuple local hoisted to a tuple parameter
                    // doesn't have a well-defined caller-side null-init —
                    // call sites would push a 12-byte null DbRef placeholder
                    // where the parameter slot is 16+ bytes per text element,
                    // corrupting the callee's frame layout.  Skip the hoist
                    // entirely: do NOT add an attribute, do NOT promote the
                    // local to an argument, and do NOT propagate the dep to
                    // the return type.  The function's return type loses
                    // the dep on this local, which lets `scopes::free_vars`
                    // (B5-L3 single-text branch, src/scopes.rs:961-988) save
                    // the body's tail expression to a `__ret_N: text` temp
                    // via `Set` (lowers to `OpAppendText`, deep-copying the
                    // text-element bytes into an owned String) before the
                    // local is freed.  Same logical fix family as @P329,
                    // applied one layer up: @P329 fixed tuple-of-text
                    // RETURN values via deep-copy temps; @P330 fixes
                    // single-text returns derived from tuple-element access
                    // on a local tuple variable via the same B5-L3 pattern,
                    // just by NOT hoisting the tuple local to a parameter
                    // (which is the wrong escape hatch).  No-op body —
                    // the local stays a local, the dep is dropped.
                } else {
                    let a = self
                        .data
                        .add_attribute(&mut self.lexer, self.context, n, tp.clone());
                    self.vars.become_argument(*v);
                    dep.push(a as u16);
                }
            }
            // P227: ensure every text-returning LAMBDA has at least one
            // `RefVar(Text)` hidden work-buffer attribute so the fn-ref
            // dispatch ABI is uniform — callers always allocate exactly
            // one buffer per text-returning fn-ref call, regardless of
            // whether the assigned lambda's body uses formatting.
            // Limited to lambdas (`n___lambda_*` prefix); the fix matches
            // the trio used by the existing text_return arm above:
            // (1) add_attribute, (2) create_var, (3) become_argument.
            // Gated on first_pass to avoid duplicate-add on the second
            // pass; the second-pass `__closure` injection (if any)
            // happens later in parse_lambda so the trailing position is
            // preserved.
            // Only LAMBDAS carry a `RefVar(Text)` work-buffer: their fn-ref
            // dispatch (control.rs) is the ONE text path that hands the callee a
            // caller-owned buffer.  Named/literal text fns return owned text (no
            // buffer) — giving them one (the reverted @P387 option A) broke par
            // workers (#273) and the markdown viewer, because not every call site
            // injects the buffer.  Zero-cost @P387: the fn-ref dispatch no longer
            // injects a text buffer (see `text_fn_ref_owned` below), so even a
            // named text fn works as a fn-value without one.
            let is_lambda = self
                .data
                .def(self.context)
                .name()
                .starts_with("n___lambda_");
            let has_work_buf =
                self.data.def(self.context).attributes().iter().any(
                    |a| matches!(a.typedef, Type::RefVar(ref t) if matches!(**t, Type::Text(_))),
                );
            if self.first_pass && is_lambda && !has_work_buf {
                let work_tp = Type::RefVar(Box::new(Type::Text(Deps::none())));
                let a = self.data.add_attribute(
                    &mut self.lexer,
                    self.context,
                    "__work_ret",
                    work_tp.clone(),
                );
                // @P387 zero-cost: hidden like the text_return buffer above, so the
                // runtime fn-ref dispatch pushes it adaptively (no static injection).
                self.data.definitions[self.context as usize].attributes[a].hidden = true;
                let v = self.create_var("__work_ret", &work_tp);
                if v != u16::MAX {
                    self.vars.become_argument(v);
                }
                dep.push(a as u16);
            }
            self.data.definitions[self.context as usize].returned = Type::Text(dep);
        }
    }

    /// Walk a return expression to find work-ref variables passed as hidden
    /// Reference arguments to struct-returning calls.  Used by `block_result`
    /// to recover deps that `filter_hidden` stripped from the return type.
    /// Issue #120: without this, the work-ref stays a local and gets freed
    /// before the caller reads the return value.
    /// True iff the callee returns a FOREIGN store it never writes into the
    /// hidden return buffer it was handed — `fn f() -> vector { g() }` where
    /// `g`'s result is delivered by `g` itself (a native builtin, or another
    /// forwarder), not built into `f`'s buffer.  Read off the callee's BODY
    /// (the tail is a `Call` whose own callee exposes no hidden heap buffer
    /// arg for `f`'s value), which is pass-stable.  A callee whose body is
    /// not parsed yet (forward ref, `code == Null`) is assumed to CONSUME —
    /// the common multi-site wrapper case #355 needs.
    fn callee_forwards_foreign_store(&self, d_nr: u32) -> bool {
        let def = self.data.def(d_nr);
        if *def.code() == Value::Null {
            return false; // unparsed / native stub — assume it consumes.
        }
        fn tail_forwards(node: &Value, data: &crate::data::Data) -> bool {
            match node.unspan() {
                Value::Call(d, _) => {
                    Parser::collect_hidden_ref_args(node, data).is_empty()
                        && matches!(
                            data.def(*d).returned(),
                            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                        )
                }
                Value::Block(bl) => bl.operators.last().is_some_and(|o| tail_forwards(o, data)),
                Value::Insert(ops) => ops.last().is_some_and(|o| tail_forwards(o, data)),
                Value::Return(inner) => tail_forwards(inner, data),
                _ => false,
            }
        }
        tail_forwards(def.code(), &self.data)
    }

    pub(crate) fn collect_hidden_ref_args(val: &Value, data: &crate::data::Data) -> Vec<u16> {
        match val {
            Value::Call(d_nr, args) => {
                let mut result = Vec::new();
                let attrs = data.def(*d_nr).attributes();
                for (i, attr) in attrs.iter().enumerate() {
                    if attr.hidden
                        && matches!(
                            attr.typedef,
                            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                        )
                        && let Some(Value::Var(v)) = args.get(i)
                    {
                        result.push(*v);
                    }
                }
                result
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last() {
                    Self::collect_hidden_ref_args(last, data)
                } else {
                    vec![]
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last() {
                    Self::collect_hidden_ref_args(last, data)
                } else {
                    vec![]
                }
            }
            Value::Set(_, inner) => Self::collect_hidden_ref_args(inner, data),
            // P198 convention: the parser wraps most expressions — including a
            // body-tail call — in `Value::Span` for diagnostics.  Unwrap so a
            // `Span(Call(...))` tail is recognised, matching the Block/Insert/
            // Set/If arms.  Without this, a thin wrapper `fn f() -> S { g() }`
            // whose tail is `Span(Call(g, [..., __ref_N]))` recovers no hidden
            // work-ref, `ref_return` never promotes the placeholder to f's
            // hidden return dest, and the local `__ref_N` is freed at f's exit
            // — corrupting the returned struct's store once it is reused (#299).
            Value::Span(b) => Self::collect_hidden_ref_args(&b.1, data),
            Value::If(_, t, f) => {
                let mut r = Self::collect_hidden_ref_args(t, data);
                r.extend(Self::collect_hidden_ref_args(f, data));
                r
            }
            _ => vec![],
        }
    }

    /// #306: true when a ref-typed return value may alias a LOCAL's store —
    /// a transitive dep of `ls` resolves to a variable that is not a function
    /// attribute.  The direct `ls` entries themselves are the NRVO-promotion
    /// candidates (handled by `ref_return`); it is their *deps* that reveal a
    /// borrow.  Such a view dangles the moment the local owner's store is
    /// freed at function exit, so the return value must be materialised.
    fn return_views_local(&self, ls: &[u16]) -> bool {
        let attr_names = &self.data.def(self.context).attr_names;
        let mut work: Vec<u16> = ls.to_vec();
        let mut seen: std::collections::HashSet<u16> = work.iter().copied().collect();
        let mut i = 0;
        while i < work.len() {
            let v = work[i];
            i += 1;
            if v >= self.vars.count() {
                continue;
            }
            for d in self.vars.tp(v).depend() {
                if d < self.vars.count() && seen.insert(d) {
                    if !attr_names.contains_key(self.vars.name(d)) {
                        return true; // borrows from a non-parameter local
                    }
                    work.push(d);
                }
            }
        }
        false
    }

    /// #306: rewrite a return value that views a local's store into an owned
    /// copy — `{ __ref_N = null; OpDatabase(__ref_N, kt);
    /// OpCopyRecord(<orig>, __ref_N, kt); __ref_N }`.  The returned work-ref
    /// is then NRVO-promoted by `ref_return`, so the copy lands directly in
    /// the caller-provided buffer (no extra allocation in the adopt case).
    /// Returns the work-ref var so the caller passes `[w]` to `ref_return`.
    fn materialize_view_return(&mut self, td: u32, tail: &mut Value) -> u16 {
        let ref_tp = Type::Reference(td, Deps::none());
        let w = self.vars.work_refs(&ref_tp, &mut self.lexer);
        self.materialize_return_into(td, tail, w);
        w
    }

    /// Rewrite a return-site value so it lands in `w` (an existing DbRef
    /// var) as an owned copy: `{ w = null; OpDatabase(w, kt);
    /// OpCopyRecord(<orig>, w, kt); w }`.  Used with a fresh work-ref by
    /// `materialize_view_return` (#306 views) and with the fn's ONE return
    /// buffer by `ref_return`'s copy leg (a named local another return
    /// site can read must not alias the buffer, so it is copied at the
    /// return instead).
    fn materialize_return_into(&mut self, td: u32, tail: &mut Value, w: u16) {
        if let Value::Return(inner) = tail {
            return self.materialize_return_into(td, inner, w);
        }
        let kt = self.data.def(td).known_type();
        let copy_d = self.data.def_nr("OpCopyRecord");
        let orig = std::mem::replace(tail, Value::Null);
        *tail = crate::data::v_block(
            vec![
                crate::data::v_set(w, Value::Null),
                self.cl("OpDatabase", &[Value::Var(w), Value::Int(i32::from(kt))]),
                Value::Call(copy_d, vec![orig, Value::Var(w), Value::Int(i32::from(kt))]),
                Value::Var(w),
            ],
            Type::Reference(td, Deps::frame1(w)),
            "materialized_view_return",
        );
    }

    /// The work-ref that carries a return site's VALUE: for a tail call,
    /// the `Var` in the callee's hidden heap-buffer argument slot; for a
    /// plain `Var` tail, the var itself.  Only this ref may bind to the
    /// fn's ONE return buffer — an INNER call's ref (`return wrap(mk(x))`
    /// has two) must stay a plain local, or the outer call's destination
    /// would alias its own argument (the callee's buffer clear then frees
    /// the record the argument still views).
    /// @PLN25 single-payload — is the return body-tail the `__nullable<S>` → dense `S`
    /// unwrap (now a payload sub-ref `OpGetField`, see `unwrap_source_is_nullable`)?  Such
    /// a tail's dense type doesn't match the still-`Enum` tail type `t`, so the type-keyed
    /// `ref_return` branches miss it and the default epilogue demotes it to `return null`.
    /// When this holds, `materialize_view_return` copies the viewed `S` into an owned buffer
    /// and promotes that — the #306 view-return shape.  Gate-off-inert.
    fn tail_is_nullable_unwrap(&self, tail: &Value) -> bool {
        match tail.unspan() {
            Value::Return(inner) => self.tail_is_nullable_unwrap(inner),
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|t| self.tail_is_nullable_unwrap(t)),
            // Single-payload: the `__nullable<S>` → dense `S` unwrap is now a payload
            // SUB-REF `OpGetField(<__nullable<S> value>, payload_offset, S_kt)` (the convert
            // emits it via `get_val`).  Materialise it — copy the viewed `S` into the return
            // buffer so the result is OWNED, not a dangling view into the caller's container —
            // ONLY when the unwrap source is a LOCAL (`Var`, e.g. `return chosen`) or a
            // materialised sub-expression (`Block`/`If`, e.g. `return v[i] ?? d`'s ncc block).
            // A direct `v[i]` index source is the sole returnable that the default epilogue
            // returns correctly; materialising it would NRVO-rename the work-ref onto the
            // caller's buffer and re-`OpDatabase` it → free-list corruption.  The
            // source-is-`__nullable<S>` check distinguishes the unwrap from an ordinary
            // struct-field read (whose source is a dense struct, not the synth enum).
            Value::Call(d, args) => {
                self.data.def(*d).name() == "OpGetField"
                    && args
                        .first()
                        .is_some_and(|s| self.unwrap_source_is_nullable(s))
            }
            _ => false,
        }
    }

    /// Is `src` a `__nullable<S>` value (the source of a payload-unwrap `OpGetField`)?
    /// Only a LOCAL (`Var`), a materialised `Block`/`If` tail qualifies — a direct
    /// index/call source returns `false` so its unwrap is NOT materialised (see
    /// `tail_is_nullable_unwrap`).  Gate-off-inert (no `__nullable<>` type exists).
    fn unwrap_source_is_nullable(&self, src: &Value) -> bool {
        let tp = match src.unspan() {
            Value::Var(v) => self.vars.tp(*v).clone(),
            Value::Block(bl) => bl.result.clone(),
            Value::If(_, t, _) => return self.unwrap_source_is_nullable(t),
            _ => return false,
        };
        matches!(&tp, Type::Enum(d, _, _) | Type::Reference(d, _)
            if self.data.def(*d).name().starts_with("__nullable<"))
    }

    /// #416 — does the tail's OUTERMOST `if`/`match` have a DIRECT `null` arm
    /// (`{ if b { [..] } else { null } }`)? Such a return is nullable, and the
    /// per-arm `__retbuf` materialise must not fire for it (it would force an
    /// owned-buffer return type onto a path that yields null — the native
    /// nullable-vector miscompile). An exhaustive `match`'s default-null is NESTED
    /// (the inner-most else after the variant tests), so it is not a direct arm and
    /// such a match still materialises. Only the outermost branch's arms are
    /// inspected — descending would also catch the unreachable match default.
    fn tail_if_has_null_arm(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Return(i) | Value::Drop(i) => self.tail_if_has_null_arm(i),
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|x| self.tail_if_has_null_arm(x)),
            Value::Insert(ops) => ops.last().is_some_and(|x| self.tail_if_has_null_arm(x)),
            Value::If(_, t, f) => self.arm_is_null(t) || self.arm_is_null(f),
            _ => false,
        }
    }

    /// Does this branch arm reduce to a `null` value (descending through the arm's
    /// block/insert tail)? A `null` vector arm lowers to `{ OpNullRefSentinel() }`,
    /// not a bare `Value::Null`, so both forms count. A nested `if` arm is NOT null
    /// — that's how enc's nested match-default is distinguished from maybe's direct
    /// `else null`.
    fn arm_is_null(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Null => true,
            Value::Call(d, _) => *d == self.data.def_nr("OpNullRefSentinel"),
            Value::Block(bl) => bl.operators.last().is_some_and(|x| self.arm_is_null(x)),
            Value::Insert(ops) => ops.last().is_some_and(|x| self.arm_is_null(x)),
            _ => false,
        }
    }

    /// #425 — if the return tail is a struct/enum FIELD projection
    /// (`OpGetField(Var(base), …)`, possibly wrapped in `Return`/`Block`),
    /// return the base var being projected. Used to decide whether the
    /// projected field's record is locally owned (and freed at scope exit) or
    /// caller-owned (a parameter).
    fn return_field_base_var(&self, tail: &Value) -> Option<u16> {
        match tail.unspan() {
            Value::Return(inner) | Value::Drop(inner) => self.return_field_base_var(inner),
            Value::Block(bl) => bl
                .operators
                .last()
                .and_then(|t| self.return_field_base_var(t)),
            Value::Insert(ops) => ops.last().and_then(|t| self.return_field_base_var(t)),
            Value::Call(d, args) if *d == self.data.def_nr("OpGetField") => {
                match args.first().map(Value::unspan) {
                    Some(Value::Var(b)) => Some(*b),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// #425 sibling — is the return tail a heap-field projection of an INLINE
    /// CALL result (`return mk().value`), as opposed to a named local
    /// (`return d.value`)?  The base `mk()` is an owned temporary that scope
    /// analysis lifts to a `__lift_N` local and frees at scope exit; the
    /// projected sub-ref is a VIEW into it, so returning the projection as-is
    /// makes the buffer dangle (native re-reads the freed store → null
    /// sentinel; the `bound_field` workaround `t = mk(); return t.value`
    /// dodges it because the named local reaches `ref_return`'s copy leg).
    /// Returns `true` only for a DIRECT `OpGetField(Call(fn,…), …)` whose base
    /// is a plain function call — NOT a chained projection (`mk().a.b`, whose
    /// intermediate `Reference` is already materialised into a work-ref) and
    /// NOT a `Var`/parameter base (handled by `return_field_base_var`).  The
    /// caller copies the projected field into `__retbuf` via
    /// `materialize_view_return` so the field's record survives the lift's
    /// free — the same owned-copy `return d.value` already performs.
    fn return_field_base_is_call(&self, tail: &Value) -> bool {
        match tail.unspan() {
            Value::Return(inner) | Value::Drop(inner) => self.return_field_base_is_call(inner),
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|t| self.return_field_base_is_call(t)),
            Value::Insert(ops) => ops
                .last()
                .is_some_and(|t| self.return_field_base_is_call(t)),
            Value::Call(d, args) if *d == self.data.def_nr("OpGetField") => {
                // The base is an owned temporary iff it is a plain function
                // CALL (not an `OpGetField` chain, not a `Var`).  An inner
                // `OpGetField` base is the chained case (E), already delivered
                // through a materialised work-ref.
                matches!(
                    args.first().map(Value::unspan),
                    Some(Value::Call(bd, _)) if *bd != self.data.def_nr("OpGetField")
                )
            }
            _ => false,
        }
    }

    fn site_value_ref(&self, tail: &Value) -> Option<u16> {
        match tail.unspan() {
            Value::Var(v) => Some(*v),
            Value::Return(inner) => self.site_value_ref(inner),
            Value::Block(bl) => bl.operators.last().and_then(|t| self.site_value_ref(t)),
            Value::Call(d, args) => {
                let def = self.data.def(*d);
                let i = def.attributes().iter().position(|a| {
                    a.hidden
                        && matches!(
                            &a.typedef,
                            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                        )
                })?;
                match args.get(i).map(Value::unspan) {
                    Some(Value::Var(v)) => Some(*v),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Rewrite a chained return site whose tail is a bare `call(..., w)`
    /// into the canonical NRVO pair `{ w = call(..., w); w }`.  A bare
    /// call tail only delivers its value via eval-stack-top — interp reads
    /// it, but scopes' `returned_var` sees no `Var`, so the fn epilogue
    /// emits `Return(Null)` and native returns the null sentinel.  The
    /// `Set` is a same-store self-assign at runtime (the call already
    /// wrote into `w`'s buffer — the @P377 no-op shape every consumer
    /// understands); a plain `{ call; w }` block would instead make the
    /// call a DISCARD whose result the witness machinery frees.
    /// @PLN85 — true iff a return tail's terminal value is a `match`/`if`,
    /// descending through `Insert`/`Block`/`Span`/`Return` wrappers.
    fn tail_terminal_is_branch(v: &Value) -> bool {
        match v.unspan() {
            Value::If(_, _, _) => true,
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(Self::tail_terminal_is_branch),
            Value::Insert(ops) => ops.last().is_some_and(Self::tail_terminal_is_branch),
            Value::Return(inner) | Value::Drop(inner) => Self::tail_terminal_is_branch(inner),
            _ => false,
        }
    }

    /// #437 + c5/#448 residual — the fresh-local vector deps of a tail expression
    /// that OWNS a fresh store: a named non-argument local vector (#437), OR a
    /// literal / comprehension whose block result owns a `__vdb` store (every dep
    /// a non-argument local — the c5 residual). `None` if it borrows an argument,
    /// already delivers into `__retbuf` (its dep is the hidden buffer arg), or
    /// isn't a fresh-owned vector. The precondition for renaming its store onto
    /// `__retbuf` so the fn delivers via NRVO instead of returning a bare store an
    /// NRVO caller's chain would orphan.
    fn fresh_owned_vector_deps(&self, v: &Value) -> Option<Vec<u16>> {
        match v.unspan() {
            // #437 — a named non-argument local vector with a backing store.
            Value::Var(o)
                if self.vars.exists(*o)
                    && !self.vars.is_argument(*o)
                    && matches!(self.vars.tp(*o), Type::Vector(_, d) if !d.is_empty()) =>
            {
                let Type::Vector(_, d) = self.vars.tp(*o) else {
                    unreachable!()
                };
                Some(d.iter().copied().collect())
            }
            // c5 residual — a fresh literal / comprehension block that owns its
            // store. Every dep must be a non-argument local; this excludes a block
            // already delivering into `__retbuf` (whose dep is the hidden buffer
            // arg) and an arg / struct-field borrow (copied, not renamed).
            Value::Block(bl) => match &bl.result {
                Type::Vector(_, d)
                    if !d.is_empty()
                        && d.iter()
                            .all(|&x| self.vars.exists(x) && !self.vars.is_argument(x)) =>
                {
                    Some(d.iter().copied().collect())
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// #448 — is the fn's `returned` type already classified to deliver into the
    /// hidden return-buffer attribute `buf_attr`? When so, EVERY return path must
    /// deliver into `__retbuf`, or the caller's buffer free orphans a path that
    /// builds its own store. The precondition for the copy-into-buffer rewrite of
    /// a fresh-local tail (so it does not re-derive / overwrite the classification).
    fn returned_uses_buffer(&self, buf_attr: u16) -> bool {
        matches!(
            self.data.def(self.context).returned(),
            Type::Vector(_, d) if d.contains(&buf_attr)
        )
    }

    /// #448 — does any statement BEFORE the tail contain a `return` that already
    /// DELIVERS into the return buffer `buf` (its value's terminal is the `__retbuf`
    /// var — the lowered shape of an early `return <call>` that NRVO-adopted into
    /// the buffer)? This is the precise precondition for COPYING a fresh-local tail
    /// into `__retbuf`: the buffer is already TAKEN, so `tail_ret_local` cannot
    /// RENAME the tail onto it. When NO early return delivers the buffer (e.g. the
    /// stdlib `split_text`'s early `return [self]` literal, or a plain single-return
    /// fn), the tail must still be renamed — copying it instead orphans the original
    /// store (the 104-split-text regression). Stays within this fn's own control
    /// flow (does not descend into nested fn / closure bodies).
    fn body_has_buffer_return(stmts: &[Value], buf: u16) -> bool {
        fn terminal_is_buf(v: &Value, buf: u16) -> bool {
            match v.unspan() {
                Value::Var(o) => *o == buf,
                Value::Block(bl) => bl.operators.last().is_some_and(|x| terminal_is_buf(x, buf)),
                Value::Insert(ops) => ops.last().is_some_and(|x| terminal_is_buf(x, buf)),
                _ => false,
            }
        }
        fn walk(v: &Value, buf: u16) -> bool {
            match v.unspan() {
                Value::Return(inner) => terminal_is_buf(inner, buf),
                Value::Drop(inner) => walk(inner, buf),
                Value::If(_, t, f) => walk(t, buf) || walk(f, buf),
                Value::Block(bl) => bl.operators.iter().any(|x| walk(x, buf)),
                Value::Insert(ops) => ops.iter().any(|x| walk(x, buf)),
                _ => false,
            }
        }
        stmts.iter().any(|s| walk(s, buf))
    }

    /// @PLN85 cluster II — PER-ARM, native-safe vector NRVO delivery. Descends a
    /// `match`/`if` to each arm's terminal local-vector `Var` and rewrites it to
    /// `Insert([OpClearVector(w), OpAppendVector(w, <local>, rec_tp),
    /// OpFreeRef(<local's __vdb dep>)…, w])`: the arm's element copy is delivered
    /// into the caller's return buffer `w`, the now-dead local backing store is
    /// freed, and the arm yields `w`. So the `If` yields `w` (a hidden param) from
    /// every arm — a return-`if` the native generator already handles — and the
    /// append source is a bare `Var` (not an `if`, which native can't scope in
    /// expression position). This delivers into the eager `__retbuf` work-ref store
    /// (fixing the interp orphan leak) while staying native-compilable. `null` arms
    /// (an exhaustive match's unreachable fall-through) are left untouched.
    fn materialize_vector_arms_into(&mut self, elm: &Type, op: &mut Value, w: u16) -> bool {
        match op {
            Value::Span(b) => self.materialize_vector_arms_into(elm, &mut b.1, w),
            Value::Return(inner) | Value::Drop(inner) => {
                self.materialize_vector_arms_into(elm, inner, w)
            }
            Value::If(_, t, f) => {
                let a = self.materialize_vector_arms_into(elm, t, w);
                let b2 = self.materialize_vector_arms_into(elm, f, w);
                a || b2
            }
            Value::Block(bl) => bl
                .operators
                .last_mut()
                .is_some_and(|last| self.materialize_vector_arms_into(elm, last, w)),
            Value::Insert(ops) => ops
                .last_mut()
                .is_some_and(|last| self.materialize_vector_arms_into(elm, last, w)),
            Value::Var(v) if *v != w && matches!(self.vars.tp(*v), Type::Vector(_, _)) => {
                let local = *v;
                let deps = self.vars.tp(local).depend();
                let rec_tp = self.append_elem_tp(elm);
                let clear = self.cl("OpClearVector", &[Value::Var(w)]);
                let append = self.cl(
                    "OpAppendVector",
                    &[Value::Var(w), Value::Var(local), Value::Int(rec_tp)],
                );
                let mut seq = vec![clear, append];
                // Free the now-dead local backing store(s) the arm built (the
                // append copied their elements into `w`); without this the
                // interpreter orphans them. Idempotent with any scope-exit free.
                for d in deps {
                    seq.push(self.cl("OpFreeRef", &[Value::Var(d)]));
                }
                seq.push(Value::Var(w));
                *op = Value::Insert(seq);
                true
            }
            Value::Call(_, _) => {
                // #437/@PLN85 cluster V cluster I-b (O-Move): a Call-terminal arm
                // (`head(0,value)`) writes its OWN hidden `__ref_N` buffer, which
                // this materialiser left untouched (only `Var` terminals above were
                // rewritten).  The epilogue then freed that `__ref_N` while it was
                // the arm's returned value — a dangling ref / silent clobber.
                // Substitute the arm's hidden buffer ref onto the shared return
                // buffer `w` and unregister it (no null-init, no scope-exit free),
                // exactly as `ref_return` does for a bare-call return — so EVERY
                // arm of a materialised single-tail vector match delivers into the
                // one buffer.  `buf == w` (an arm already writing the buffer) is a
                // no-op via the guard, so this is idempotent.
                let mut changed = false;
                for buf in Self::collect_hidden_ref_args(op, &self.data) {
                    if buf != w {
                        Self::substitute_work_ref(op, buf, w);
                        self.vars.unregister_work_ref(buf);
                        changed = true;
                    }
                }
                changed
            }
            _ => false,
        }
    }

    /// #415 — does the block's tail expression read a STRUCT vector field
    /// (`b.v` where the base is a `Reference`), as opposed to a whole var, a
    /// call, or a vector INDEX read (`vv[i]`, base is a `Vector`)? Gates the
    /// implicit-tail copy below: only a struct-field borrow of an argument needs
    /// copying into the return buffer.
    ///
    /// A vector INDEX / nested-element read (`OpGetField(OpGetVector …)`) is
    /// DELIBERATELY excluded.  #426 (A.1) probed generalizing this funnel to the
    /// index-read tail (`fn idx0(w) -> vector { w[0] }`): forcing it through this
    /// `__retbuf` copy path collides the forward temp's inner-element view
    /// store-nr with a freed sibling store once the caller frame has released a
    /// vector store (the `borrow_tail_copy_104` return-buffer model is proven only
    /// for whole-arg / struct-field tails).  The index-read RETURN (#426B) stays
    /// ALIASED until that store-reuse / return-buffer substrate is fixed (routed
    /// forward, the a7 class — see `STABILITY_REDFLAG_REMEDIATION.md` A.1).
    fn tail_is_struct_field_read(&self, l: &[Value]) -> bool {
        let mut v = match l.last() {
            Some(v) => v,
            None => return false,
        };
        loop {
            match v.unspan() {
                Value::Return(inner) | Value::Drop(inner) => v = inner,
                Value::Block(bl) => match bl.operators.last() {
                    Some(x) => v = x,
                    None => return false,
                },
                Value::Call(d, args) => {
                    return *d == self.data.def_nr("OpGetField")
                        && matches!(
                            args.first().map(Value::unspan),
                            Some(Value::Var(bv)) if matches!(self.vars.tp(*bv), Type::Reference(_, _))
                        );
                }
                _ => return false,
            }
        }
    }

    /// Row-104 funnel: does the body tail return a WHOLE vector PARAMETER
    /// directly (`fn idv(v) -> vector { v }` — the implicit-tail sibling of an
    /// explicit `return v`)?  Such a tail borrows the caller's store, so
    /// returning the param as-is ALIASES the argument.  Returns the param var
    /// so the caller can copy it into `__retbuf` — the same value-semantics
    /// COPY the explicit `return v` path (`parse_return`) and the struct-field
    /// tail (`tail_is_struct_field_read`, #415) both perform.  Narrowed to a
    /// bare `Var` that is a vector argument: index / call / field tails keep
    /// their existing handling (the over-broad cut regressed the suite — A.2).
    fn tail_whole_arg_vector(&self, l: &[Value]) -> Option<u16> {
        let mut v = l.last()?;
        loop {
            match v.unspan() {
                Value::Return(inner) | Value::Drop(inner) => v = inner,
                Value::Block(bl) => v = bl.operators.last()?,
                Value::Insert(ops) => v = ops.last()?,
                Value::Var(bv) => {
                    return (self.vars.is_argument(*bv)
                        && matches!(self.vars.tp(*bv), Type::Vector(_, _)))
                    .then_some(*bv);
                }
                _ => return None,
            }
        }
    }

    /// @PLN85 over-free class — does the body tail return a vector LOCAL that
    /// BORROWS a visible argument (its type deps name an arg)?  The canonical
    /// case is a match-arm field binding returned directly
    /// (`Filled { items } => items`, where `items` is a borrowed view of the
    /// subject's `items` field, deps `["c"]`).  Such a tail is neither an
    /// `OpGetField` struct-field read (`tail_is_struct_field_read`) nor a whole
    /// vector ARG (`tail_whole_arg_vector`), so without this it falls through to
    /// the `Rename` path — which promotes the borrowed binding onto the CALLER's
    /// return buffer, aliasing the buffer to the arg's store; the caller's later
    /// buffer free then corrupts the arg (P14 enum-field-vector crash).  Routing
    /// it through `CopyBorrow` copies the view into `__retbuf` (value semantics).
    fn tail_borrows_arg(&self, l: &[Value]) -> bool {
        let mut v = match l.last() {
            Some(v) => v,
            None => return false,
        };
        loop {
            match v.unspan() {
                Value::Return(inner) | Value::Drop(inner) => v = inner,
                Value::Block(bl) => match bl.operators.last() {
                    Some(x) => v = x,
                    None => return false,
                },
                Value::Insert(ops) => match ops.last() {
                    Some(x) => v = x,
                    None => return false,
                },
                Value::Var(bv) => {
                    return matches!(self.vars.tp(*bv), Type::Vector(_, _))
                        && self
                            .vars
                            .tp(*bv)
                            .depend()
                            .iter()
                            .any(|&d| self.vars.is_argument(d));
                }
                _ => return false,
            }
        }
    }

    /// Row-104 funnel: copy a BORROWED implicit-tail vector return into the
    /// function's one `__retbuf` buffer and finalize the return-type dep to
    /// `{buf_attr}`, so the caller adopts an independent copy (value
    /// semantics).  The single home for the "the tail borrows a visible arg →
    /// COPY" decision in `block_result`; the struct-field tail (#415) and the
    /// whole-arg param tail (a2) both route here instead of re-deriving the
    /// copy shape inline.  Mirrors the explicit `return <borrow>` copy in
    /// `parse_return` (~4651): capture the tail value into `__fwd`, then
    /// `OpClearVector(buf); OpAppendVector(buf, __fwd); buf`.  Returns true on
    /// success; false (var allocation failed / no tail) tells the caller to
    /// fall back to the `ref_return` path.
    fn copy_borrow_tail_into_retbuf(
        &mut self,
        elm: &Type,
        l: &mut [Value],
        buf_attr: u16,
        buf_var: u16,
    ) -> bool {
        let elm_ty = elm.clone();
        let Some(last) = l.last_mut() else {
            return false;
        };
        let rec_tp = self.append_elem_tp(&elm_ty);
        let clear = self.cl("OpClearVector", &[Value::Var(buf_var)]);
        // Append the borrowed tail value DIRECTLY into the buffer — no `__fwd`
        // local.  This function is only the BORROWED-arg case (the tail views a
        // visible param: a whole-arg vector, a struct-field of an arg), so `orig`
        // never owns its store and never aliases the hidden buffer.  A captured
        // `__fwd` local carried empty deps, so its scope-exit `OpFreeRef` freed
        // the borrowed source — i.e. the caller's vector (P462 over-free, recycled
        // under allocation pressure -> corruption).  Inlining matches the proven
        // explicit `return <borrow>` path in `parse_return`, which appends inline
        // and frees nothing.
        let orig = std::mem::replace(last, Value::Null);
        let append = self.cl(
            "OpAppendVector",
            &[Value::Var(buf_var), orig, Value::Int(rec_tp)],
        );
        *last = crate::data::v_block(
            vec![clear, append, Value::Var(buf_var)],
            Type::Vector(Box::new(elm_ty.clone()), Deps::frame1(buf_var)),
            "borrow_tail_copy_104",
        );
        self.data.definitions[self.context as usize].returned =
            Type::Vector(Box::new(elm_ty), Deps::attrs(vec![buf_attr]));
        true
    }

    fn chain_site_set_shape(ret: &Type, tail: &mut Value, w: u16) {
        match tail {
            Value::Span(b) => Self::chain_site_set_shape(ret, &mut b.1, w),
            Value::Return(inner) => Self::chain_site_set_shape(ret, inner, w),
            // Argument lifting wraps the site call in `Insert([lifts…,
            // call])`; descend to the call so its value still surfaces.
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    Self::chain_site_set_shape(ret, last, w);
                }
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last_mut() {
                    Self::chain_site_set_shape(ret, last, w);
                }
            }
            Value::Call(_, _) => {
                let block_tp = match ret {
                    Type::Reference(td, _) => Type::Reference(*td, Deps::frame1(w)),
                    Type::Vector(it, _) => Type::Vector(it.clone(), Deps::frame1(w)),
                    other => other.clone(),
                };
                let call = std::mem::replace(tail, Value::Null);
                *tail = crate::data::v_block(
                    vec![crate::data::v_set(w, call), Value::Var(w)],
                    block_tp,
                    "one_buffer_chain",
                );
            }
            _ => {}
        }
    }

    /// Vector counterpart of [`materialize_return_into`]: copy a return
    /// site's vector value into the fn's one buffer by element append —
    /// `{ OpClearVector(w); OpAppendVector(w, <orig>, rec_tp); w }` — the
    /// same element copy the explicit-return vector path has always used.
    /// The clear makes delivery REPLACE the buffer's content: a caller
    /// that re-passes the same buffer (a call inside a loop reuses the
    /// fn-scoped `__ref_N`) must see exactly this invocation's result,
    /// not an accumulation of every iteration's appends.
    fn materialize_vector_return_into(&mut self, elm: &Type, tail: &mut Value, w: u16) {
        if let Value::Return(inner) = tail {
            return self.materialize_vector_return_into(elm, inner, w);
        }
        let rec_tp = self.append_elem_tp(elm);
        let orig = std::mem::replace(tail, Value::Null);
        let clear = self.cl("OpClearVector", &[Value::Var(w)]);
        let append = self.cl("OpAppendVector", &[Value::Var(w), orig, Value::Int(rec_tp)]);
        *tail = crate::data::v_block(
            vec![clear, append, Value::Var(w)],
            Type::Vector(Box::new(elm.clone()), Deps::frame1(w)),
            "one_buffer_vec_copy",
        );
    }

    /// Rewrite every mid-body `return <named local vector>` of a
    /// buffer-bound fn into the delivering shape
    /// `Insert([OpClearVector(buf), OpAppendVector(buf, <local>, rec_tp),
    /// Return(buf)])` — the same element copy + replace semantics as
    /// [`materialize_vector_return_into`].  Sites already delivering
    /// (their innermost return value is the buffer var, whether from the
    /// chain shape or the legacy `__ref_1` injection) are left alone, so
    /// the walk is idempotent across parse passes.  Only bare `Var`
    /// values are rewritten: call-chain sites deliver through the callee,
    /// and every other shape keeps its existing behaviour.
    fn deliver_mid_vector_returns(&mut self, elm: &Type, body: &mut [Value], buf_var: u16) {
        for op in body.iter_mut() {
            self.deliver_mid_vector_walk(elm, op, buf_var);
        }
    }

    /// #457 — does `cv` get reassigned to a CALL result anywhere in `body`?
    /// `cv = some_fn(.., __ref_N)` ADOPTS the callee's delivery store, so at the
    /// tail `cv` holds a store DISTINCT from its NRVO buffer.  In-place
    /// `cv += [..]` does NOT count (its `Set` target is the element temp), nor
    /// does the initial `cv: vector = []`.
    fn body_reassigns_var_to_call(body: &[Value], cv: u16) -> bool {
        fn walk(node: &Value, cv: u16) -> bool {
            if let Value::Set(w, val) = node
                && *w == cv
                && matches!(val.unspan(), Value::Call(_, _))
            {
                return true;
            }
            match node {
                Value::Set(_, val) => walk(val, cv),
                Value::Call(_, args)
                | Value::Insert(args)
                | Value::Tuple(args)
                | Value::Parallel(args) => args.iter().any(|a| walk(a, cv)),
                Value::Block(bl) | Value::Loop(bl) => bl.operators.iter().any(|o| walk(o, cv)),
                Value::If(c, t, e) => walk(c, cv) || walk(t, cv) || walk(e, cv),
                Value::Iter(_, c, n, e) => walk(c, cv) || walk(n, cv) || walk(e, cv),
                Value::Return(x) | Value::Drop(x) | Value::Yield(x) | Value::BreakWith(_, x) => {
                    walk(x, cv)
                }
                Value::Span(b) => walk(&b.1, cv),
                _ => false,
            }
        }
        body.iter().any(|o| walk(o, cv))
    }

    fn deliver_mid_vector_walk(&mut self, elm: &Type, op: &mut Value, buf_var: u16) {
        match op {
            Value::Return(inner) => {
                if let Value::Var(v) = inner.unspan()
                    && *v != buf_var
                    && matches!(self.vars.tp(*v), Type::Vector(_, _))
                {
                    let local = *v;
                    let rec_tp = self.append_elem_tp(elm);
                    // Aliasing-safe deliver: `local` may ALIAS `buf_var` (an
                    // un-reassigned `return out` where `out` borrows the buffer),
                    // and the old `clear(buf); append(buf, out)` then emptied it
                    // (the mid-body-return self-copy).  `OpReplaceVector` no-ops
                    // when the two name the same backing vector.
                    let replace = self.cl(
                        "OpReplaceVector",
                        &[Value::Var(buf_var), Value::Var(local), Value::Int(rec_tp)],
                    );
                    *op =
                        Value::Insert(vec![replace, Value::Return(Box::new(Value::Var(buf_var)))]);
                } else if self.fresh_owned_vector_deps(inner.unspan()).is_some() {
                    // c5/#448 residual sibling — a mid-body `return <fresh literal>`
                    // in an NRVO-promoted vector fn must ALSO deliver into __retbuf,
                    // or the buffer-classified caller frees __retbuf and orphans this
                    // path's store (the `dual` early-path leak). The literal block's
                    // terminal Var is a fresh `_vec`, so the cluster-I per-arm
                    // materialiser delivers it (clear+append+free the __vdb), leaving
                    // the block yielding __retbuf; wrap it back in the `return`.
                    self.materialize_vector_arms_into(elm, inner.unspan_mut(), buf_var);
                }
            }
            Value::Span(b) => self.deliver_mid_vector_walk(elm, &mut b.1, buf_var),
            Value::Insert(ops) | Value::Parallel(ops) => {
                for o in ops {
                    self.deliver_mid_vector_walk(elm, o, buf_var);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for o in &mut bl.operators {
                    self.deliver_mid_vector_walk(elm, o, buf_var);
                }
            }
            Value::If(c, t, e) => {
                self.deliver_mid_vector_walk(elm, c, buf_var);
                self.deliver_mid_vector_walk(elm, t, buf_var);
                self.deliver_mid_vector_walk(elm, e, buf_var);
            }
            Value::Iter(_, c, n, e) => {
                self.deliver_mid_vector_walk(elm, c, buf_var);
                self.deliver_mid_vector_walk(elm, n, buf_var);
                self.deliver_mid_vector_walk(elm, e, buf_var);
            }
            _ => {}
        }
    }

    /// The fn's ONE hidden return buffer: the first hidden heap-typed
    /// attribute of the current context, as `(attr index, bound var)`.
    /// After the first promotion the attr carries the promoted local's
    /// name (the attr↔var coupling is by name), so the var is looked up
    /// through the attr's CURRENT name.  Returns None when the context
    /// has no hidden heap attr or its var is not in this fn's table.
    fn return_buffer(&self) -> Option<(u16, u16)> {
        let def = self.data.def(self.context);
        let (a_idx, a) = def.attributes().iter().enumerate().find(|(_, a)| {
            a.hidden
                && matches!(
                    &a.typedef,
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                )
        })?;
        let v = self.vars.var(&a.name);
        if v == u16::MAX {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        Some((a_idx as u16, v))
    }

    pub(crate) fn ref_return(&mut self, ls: &[u16], body: &mut [Value], site: RetSite) {
        // Plan-57: a returned local that gets a fresh vector literal more than once
        // cannot be NRVO-promoted to the caller's buffer — each `z=[lit]` builds INTO
        // the buffer (`OpNewRecord(z, …)`), so the second literal appends rather than
        // replaces, leaving the FIRST value (`z=[a]; z=[b]; z` returned [a]).  Each
        // literal uses a DISTINCT element-temp (`Set(_elm_k, OpNewRecord(z, …))`), so
        // the count of distinct element-temps building into `z` is the number of
        // literal assignments; ≥2 ⇒ reassigned ⇒ leave it a normal local (the `__vdb`
        // + return-copy path explicit return uses, which handles reassignment).  This
        // is visible on the FIRST pass (where the promotion happens), unlike the
        // later `OpPreAllocVector` form.
        let newrecord_nr = self.data.def_nr("OpNewRecord");
        fn reassign_count(body: &[Value], v: u16, nr: u32) -> usize {
            fn collect(node: &Value, v: u16, nr: u32, temps: &mut std::collections::HashSet<u16>) {
                if let Value::Set(w, val) = node
                    && let Value::Call(op, args) = val.unspan()
                    && *op == nr
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == v)
                {
                    temps.insert(*w);
                }
                match node {
                    Value::Set(_, val) => collect(val, v, nr, temps),
                    Value::Call(_, args)
                    | Value::Insert(args)
                    | Value::Tuple(args)
                    | Value::Parallel(args) => {
                        for a in args {
                            collect(a, v, nr, temps);
                        }
                    }
                    Value::Block(bl) | Value::Loop(bl) => {
                        for o in &bl.operators {
                            collect(o, v, nr, temps);
                        }
                    }
                    Value::If(c, t, e) => {
                        collect(c, v, nr, temps);
                        collect(t, v, nr, temps);
                        collect(e, v, nr, temps);
                    }
                    Value::Iter(_, c, n, e) => {
                        collect(c, v, nr, temps);
                        collect(n, v, nr, temps);
                        collect(e, v, nr, temps);
                    }
                    Value::Return(x)
                    | Value::Drop(x)
                    | Value::Yield(x)
                    | Value::BreakWith(_, x) => {
                        collect(x, v, nr, temps);
                    }
                    Value::Span(b) => collect(&b.1, v, nr, temps),
                    _ => {}
                }
            }
            let mut temps = std::collections::HashSet::new();
            for o in body {
                collect(o, v, nr, &mut temps);
            }
            temps.len()
        }
        let ret = self.data.definitions[self.context as usize]
            .returned
            .clone();
        if std::env::var("LOFT_TRACE_RR").is_ok() {
            let fn_name = self.data.def(self.context).name();
            let ls_named: Vec<String> = ls
                .iter()
                .map(|v| format!("{}={:?}", self.vars.name(*v), self.vars.tp(*v)))
                .collect();
            eprintln!(
                "[rr] fn={fn_name} pass1={} ls={ls:?} ls_tps={ls_named:?} ret={ret:?}",
                self.first_pass
            );
        }
        // B2-runtime / B3 / B7 unification (2026-04-13): struct-enums
        // (Type::Enum with struct-enum discriminator `true`) live as
        // heap-allocated records just like Reference and Vector do, so
        // their return-slot must also be promoted to a hidden caller
        // argument.  Without this arm the callee allocates its own
        // DbRef locally; the caller never reserves matching stack space;
        // OpReturn's value-width mismatches the reserved slot and the
        // interpreter loops on Return(ret=0, value=16) at PC=0.
        if let Type::Vector(_, cur) | Type::Reference(_, cur) | Type::Enum(_, true, cur) = &ret {
            let mut dep = cur.clone();
            // #306: a returned local can itself hold a view — its TYPE deps name
            // the vars it borrows from (`chosen = table[idx]; chosen` gives
            // `chosen: Reference(M, [table])`).  Walk deps transitively and merge
            // every PARAMETER the returned value may alias into the declared
            // return deps; otherwise the call site treats the value as owned and
            // frees the caller's store at scope exit.  Transitively-reached vars
            // are merge-only: promoting them to hidden ref args (as direct `ls`
            // entries are) would change the call ABI for locals the NRVO
            // machinery cannot host (e.g. a call-result vector), breaking callers.
            let mut expanded: Vec<u16> = ls.to_vec();
            let direct_count = expanded.len();
            let mut seen: std::collections::HashSet<u16> = expanded.iter().copied().collect();
            let mut i = 0;
            while i < expanded.len() {
                let v = expanded[i];
                i += 1;
                if v >= self.vars.count() {
                    continue; // foreign dep (e.g. closure work var) — not ours
                }
                for d in self.vars.tp(v).depend() {
                    if d < self.vars.count() && seen.insert(d) {
                        expanded.push(d);
                    }
                }
            }
            // The ref carrying THE SITE'S VALUE (a tail call's buffer arg /
            // a plain Var tail).  Only this ref may bind to the fn's one
            // return buffer; an INNER call's work ref (`return wrap(mk(x))`
            // carries two) must stay a plain local — binding both would
            // alias the outer call's destination with its own argument.
            let site_value = body.last().and_then(|t| self.site_value_ref(t));
            let is_plain_fn = !self.data.def(self.context).name().contains("__lambda")
                && self.data.def_type(self.context) == crate::data::DefType::Function;
            for (e_idx, v) in expanded.iter().enumerate() {
                let transitive = e_idx >= direct_count;
                let n = self.vars.name(*v);
                let is_work_ref = n.starts_with("__ref_") || n.starts_with("__rref_");
                // A reassigned returned local must NOT be NRVO-promoted (see
                // above) — but a NAMED local at a vector fn's body tail still
                // DELIVERS: it falls through to the one-buffer branch below,
                // whose named-local leg copies the final value into the
                // buffer at the return (reassignment is irrelevant to a
                // single copy-at-exit).  Skipping it entirely leaves the fn
                // value-returning while callers — who can only consult the
                // signature (a forward caller parses before this body) —
                // assume buffer delivery and free the buffer alone: the
                // returned store leaks (#355 fallout, the 93-vsort suite
                // leak).
                let reassigned = reassign_count(body, *v, newrecord_nr) >= 2;
                if reassigned
                    && !(!is_work_ref
                        && is_plain_fn
                        && site == RetSite::BlockTail
                        && matches!(&ret, Type::Vector(_, _)))
                {
                    continue;
                }
                // skip related variables that are already attributes
                if let Some(a) = self.data.def(self.context).attr_names.get(n) {
                    let a = *a as u16;
                    if !dep.contains(&a) {
                        dep.push(a);
                    }
                    // #356: pass 2 re-finds a pass-1-promoted site work ref
                    // by name here — the site STILL needs its value made
                    // explicit each pass (a mid-body bare-call tail loses
                    // its value to the Return(Null) fall-through on native
                    // once argument lifting decomposes it).
                    if site == RetSite::MidReturn
                        && is_work_ref
                        && !transitive
                        && self.data.def(self.context).attributes()[a as usize].hidden
                        && let Some(tail) = body.last_mut()
                    {
                        Self::chain_site_set_shape(&ret, tail, *v);
                    }
                    continue;
                }
                if transitive {
                    continue; // merge-only for transitively-reached vars (see above)
                }
                // An inner work ref that is not the site's value stays a
                // plain local: the outer call deep-copies its record into
                // the destination before scope exit frees it.
                //
                // Cluster I-d (@PLN85 cluster V) EXCEPTION — the site value ADOPTS this
                // work ref: `buf = head(.., __ref_1); …; return buf`, where
                // `buf`'s dep is `__ref_1` (buf aliases head's returned store).
                // Here `buf == __ref_1` at runtime, so promoting `__ref_1` to
                // `__retbuf` makes `buf == __retbuf` (true NRVO) — the same
                // end-state the `buf = []` literal path reaches directly.  Left
                // un-promoted the fn returns a FRESH adopt store while a `["??"]`
                // caller (e.g. a `match` wrapper) frees the unused buffer and
                // the adopted store LEAKS (the I-c face-flip).  Only skip when
                // the site value does NOT adopt `v`.
                let site_adopts_v =
                    site_value.is_some_and(|sv| self.vars.tp(sv).depend().contains(v));
                if is_work_ref && site_value.is_some() && site_value != Some(*v) && !site_adopts_v {
                    continue;
                }
                // @PLAN59 / H1: bind the promoted local to the
                // signature-time `__retbuf` buffer instead of GROWING the
                // signature — rename the ATTR to the local's name (the
                // attr↔var coupling is by name, probe C3; pass 2's
                // `attr_names` lookup above then hits directly) and retire
                // the placeholder argument var (the promoted local takes
                // the same last frame slot by var-number order, probe C6).
                // A MID-BODY vector return never renames: the rename makes
                // the site's local the fn-wide buffer, which is only sound
                // at the body tail (the 01b breakage) — vector mid-returns
                // bind through the one-buffer branch below instead.  And
                // once ANY earlier site chained into the placeholder
                // (`dep` already names the buffer attr), renaming would
                // retire the placeholder var those sites reference — the
                // later candidate must copy instead.
                let bound_already = self.return_buffer().is_some_and(|(a, _)| dep.contains(&a));
                // #425 — the return value is a struct/enum FIELD projection of THIS
                // candidate (`return d.value`, where `d` is the container local).
                // Renaming `d` to the return buffer is wrong: `d` holds the WHOLE
                // record (`Decoded`) while the fn returns its inner field
                // (`CborValue`), so the promoted buffer would be the container, the
                // field sub-ref would be dropped, and `d` freed at scope exit — the
                // returned value dangles (native re-encodes to 0 bytes). Suppress the
                // rename so `d` stays an ordinary local and the candidate falls
                // through to the copy-into-buffer leg below (`materialize_return_into`
                // deep-copies `d.value` into the separate `__retbuf`). A field-of-
                // ARGUMENT never reaches here (a true parameter hits the earlier
                // `attr_names` continue), and a local-bind (`v = d.value; return v`)
                // returns `v` itself (not a projection), so this is field-projection-
                // of-a-local only.
                let returns_own_field =
                    self.return_field_base_var(body.last().unwrap_or(&Value::Null)) == Some(*v);
                let allow_rename = !(bound_already
                    || reassigned
                    || returns_own_field
                    || (site == RetSite::MidReturn && matches!(&ret, Type::Vector(_, _))));
                if allow_rename
                    && let Some(&buf_attr) = self.data.def(self.context).attr_names.get("__retbuf")
                {
                    let def = &mut self.data.definitions[self.context as usize];
                    def.attributes[buf_attr].name = n.to_string();
                    def.attr_names.remove("__retbuf");
                    def.attr_names.insert(n.to_string(), buf_attr);
                    let placeholder = self.vars.var("__retbuf");
                    if placeholder != u16::MAX {
                        self.vars.retire_argument(placeholder);
                    }
                    self.vars.become_argument(*v);
                    dep.push(buf_attr as u16);
                    // #356: a mid-body `return f(g(x))` site loses its value
                    // on native once argument lifting decomposes the bare
                    // call — give the freshly bound site the explicit
                    // `Set + Var` shape.  Body-tail sites keep their NRVO /
                    // unify wiring untouched (wrapping there broke if-arm
                    // emission).
                    if site == RetSite::MidReturn
                        && is_work_ref
                        && let Some(tail) = body.last_mut()
                    {
                        Self::chain_site_set_shape(&ret, tail, *v);
                    }
                    continue;
                }
                // ONE-BUFFER invariant (stability roadmap #1): a plain fn's
                // arity is FIXED at signature parse — never grow it here.
                // Growth crashes forward callers (a caller parsed earlier
                // holds a short arg list), diverges on recursion
                // (buffers(f) = k + buffers(f) has no finite fixpoint), and
                // leaks the buffer count into the user-facing fn TYPE (two
                // fns with the same declared signature could not share a
                // fn-ref variable).  Instead the site BINDS to the one
                // existing buffer:
                //   - a parser-minted work ref (`__ref_N` — referenced only
                //     at its own return site) is SUBSTITUTED by the buffer
                //     var, so the site's call writes directly into the
                //     caller's buffer (return paths are mutually exclusive,
                //     so sharing one buffer is sound);
                //   - a named local (readable by sibling return sites —
                //     substitution could alias the buffer into another
                //     site's argument list) keeps its own store and is
                //     deep-copied into the buffer at the return.
                // Lambdas keep in-place growth: they are defined at their
                // literal site and invoked via CallRef, so no earlier
                // caller can hold a short arg list.
                if is_plain_fn
                    && matches!(
                        &ret,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
                    && let Some((buf_attr, buf_var)) = self.return_buffer()
                    && buf_var != *v
                {
                    if is_work_ref {
                        for op in body.iter_mut() {
                            Self::substitute_work_ref(op, *v, buf_var);
                        }
                        // The substituted-out ref must not get a null-init
                        // preamble or a scope-exit free (see
                        // `unregister_work_ref`).
                        self.vars.unregister_work_ref(*v);
                        // A bare-call site tail needs its value made
                        // explicit (see `chain_site_set_shape`).
                        if let Some(tail) = body.last_mut() {
                            Self::chain_site_set_shape(&ret, tail, buf_var);
                        }
                    } else if let Some(tail) = body.last_mut() {
                        // Named local: keep its own store; deliver a COPY in
                        // the buffer at the return.  #425 — a struct-enum
                        // (heap `Type::Enum`) field-of-local return copies the
                        // same way as a Reference: `materialize_return_into`
                        // emits `OpCopyRecord(d.field → buf)` (the record copy
                        // works for any heap record, enum or struct).
                        match ret.clone() {
                            Type::Reference(td, _) | Type::Enum(td, true, _) => {
                                self.materialize_return_into(td, tail, buf_var);
                            }
                            Type::Vector(elm, _) => {
                                self.materialize_vector_return_into(&elm, tail, buf_var);
                            }
                            _ => {}
                        }
                    } else {
                        // No body tail to rewrite (defensive) — keep the
                        // local unpromoted; the return-copy path handles it.
                    }
                    if !dep.contains(&buf_attr) {
                        dep.push(buf_attr);
                    }
                    continue;
                }
                // Vector / struct-Enum returns and lambdas still grow.
                // PASS-1 growth is sound (pass 2 re-parses every caller
                // against the final arity and re-finds the grown attr by
                // name); PASS-2 growth on a plain fn must never happen —
                // callers compiled in pass 2 before the growth would hold
                // a short arg list.
                debug_assert!(
                    self.first_pass
                        || self.data.def(self.context).name().contains("__lambda")
                        || self.data.def_type(self.context) != crate::data::DefType::Function,
                    "@PLAN59: arity grew in PASS 2 on plain fn '{}'",
                    self.data.def(self.context).name()
                );
                let a = self
                    .data
                    .add_attribute(&mut self.lexer, self.context, n, ret.clone());
                // mark as hidden return-mechanism parameter
                self.data.definitions[self.context as usize].attributes[a].hidden = true;
                self.vars.become_argument(*v);
                dep.push(a as u16);
                // Growth here is lambda-only (asserted above): a lambda is
                // defined at its literal site and invoked via CallRef
                // (fn-ref dispatch, never an arity-filled Call), so no
                // earlier caller can hold a short arg list — the #339
                // retro-patch this branch once needed is deleted
                // (@PLAN59 phase 2).
            }
            // A buffer-bound vector fn must deliver at EVERY return site —
            // callers (a forward caller in particular) can only consult the
            // signature, so they free the buffer alone and read the value
            // from it.  Mid-body `return <named local>` sites parsed BEFORE
            // the tail's promotion ran could not know the fn would bind
            // (vsort's base case: the legacy `__ref_1` injection missed its
            // `__ref_3`-named buffer and the leaf vectors leaked, #355
            // fallout) — rewrite them here, where the binding decision is
            // final and the full body is in hand.
            if site == RetSite::BlockTail
                && let Type::Vector(elm, _) = &ret
                && let Some((buf_attr, buf_var)) = self.return_buffer()
                && dep.contains(&buf_attr)
            {
                let elm = (**elm).clone();
                self.deliver_mid_vector_returns(&elm, body, buf_var);
                // #457 — deliver the IMPLICIT tail too. `deliver_mid_vector_returns`
                // rewrites `Return(Var(cv))` sites, but a body ending in an implicit
                // `cv` (no `return` keyword) leaves a bare `Var(cv)` tail it does not
                // touch. When `cv` was reassigned to a call-ADOPT in an arm
                // (`cv = recurse(.., __ref_N)`), `cv` holds a store distinct from
                // `buf_var`; returning it as-is was the #457 adopt (the callee then
                // freed the buffer it returned, fixed previously by a per-site
                // free thicket). Deliver `cv` into `buf_var` via the aliasing-safe
                // `OpReplaceVector` (a NO-OP when `cv` still aliases the buffer, so a
                // single-arm / non-reassigned tail is untouched — this is why it no
                // longer self-copies), so the fn ALWAYS returns its buffer and the
                // dep is accurate: no adopt, no per-site free derivation.
                let tail_cv = body.last().and_then(Self::tail_var);
                if let Some(cv) = tail_cv
                    && cv != buf_var
                    && matches!(self.vars.tp(cv), Type::Vector(_, _))
                    && Self::body_reassigns_var_to_call(body, cv)
                {
                    let rec_tp = self.append_elem_tp(&elm);
                    let replace = self.cl(
                        "OpReplaceVector",
                        &[Value::Var(buf_var), Value::Var(cv), Value::Int(rec_tp)],
                    );
                    if let Some(last) = body.last_mut() {
                        *last = Value::Insert(vec![replace, Value::Var(buf_var)]);
                    }
                }
                // Clear the buffer ON ENTRY: a caller's loop re-passes the
                // same fn-scoped buffer every iteration, and the NRVO
                // literal build (unlike the copy/injection sites) appends
                // without resetting — without this, each iteration's
                // result piles on top of the previous one (silent wrong
                // results, not just leaks).
                if let Some(first) = body.first_mut() {
                    let clear = self.cl("OpClearVector", &[Value::Var(buf_var)]);
                    let old = std::mem::replace(first, Value::Null);
                    *first = Value::Insert(vec![clear, old]);
                }
            }
            // H2: the rebuilt return-type deps are ATTRIBUTE indices —
            // tag them so `as_attr_indices` readers verify in debug builds.
            let dep = Deps::attrs(dep.to_vec());
            self.data.definitions[self.context as usize].returned = match ret {
                Type::Vector(it, _) => Type::Vector(it, dep),
                Type::Reference(td, _) => Type::Reference(td, dep),
                Type::Enum(td, true, _) => Type::Enum(td, true, dep),
                _ => {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unexpected return type in ref_return: {}",
                        ret.name(&self.data)
                    );
                    return;
                }
            };
        }
    }

    // <return> ::= [ <expression> ]
    pub(crate) fn parse_return(&mut self, val: &mut Value) {
        // validate if there is a defined return value
        let mut v = Value::Null;
        let r_type = self.data.def(self.context).returned().clone();
        if !self.lexer.peek_token(";") && !self.lexer.peek_token("}") {
            // T1.7: save the position of the first token in the return expression,
            // used to report `not null` violations at the tuple literal site.
            let expr_start = self.lexer.peek();
            // @P365: a `return [ … ]` vector literal needs the function's return
            // type threaded in as the element-type hint — exactly as an assignment
            // threads its declared LHS type (parse_assign_op → parse_operators).
            // Without it an EMPTY `return []` types as Unknown, skips the
            // Vector-construction lowering below, and emits `return ()` (native,
            // E0308) / a garbage DbRef (interpret).  Gated to a `[`-led literal
            // returned from a vector-typed fn so every other return keeps the
            // existing `expression` path verbatim (for a literal, `expression`
            // already reduces to `parse_operators(Unknown)` — only the hint differs).
            let t = if let Type::Vector(elm, _) = &r_type
                && self.lexer.peek_token("[")
            {
                // Thread the element type but NOT the return type's dep: a
                // vector-returning fn carries `[__ref_1]` as its dep, and
                // inheriting that on the literal would fool the `Type::Vector`
                // arm below (`!dep.contains(ref1_var)`) into skipping the
                // OpAppendVector copy into __ref_1.  Element type only.
                let hint = Type::Vector(elm.clone(), Deps::none());
                let mut parent_tp = Type::Null;
                self.parse_operators(&hint, &mut v, &mut parent_tp, 0)
            } else {
                self.expression(&mut v)
            };
            if r_type == Type::Void {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect no expression after return"
                );
                *val = Value::Return(Box::new(Value::Null));
                return;
            }
            // T1.7: check for null assigned to `integer not null` tuple elements.
            if !self.first_pass
                && let (Value::Tuple(elems), Type::Tuple(expected)) = (&v, &r_type)
            {
                for (elem_val, elem_tp) in elems.iter().zip(expected.iter()) {
                    if matches!(elem_val, Value::Null)
                        && matches!(elem_tp, Type::Integer(IntegerSpec { not_null: true, .. }))
                    {
                        specific!(
                            &mut self.lexer,
                            &expr_start,
                            Level::Error,
                            "cannot assign null to 'integer not null' element"
                        );
                    }
                }
            }
            // @P374: mirror block_result's tuple→synthetic-struct rewrite for an
            // explicit `return (a, b);` whose declared return type is a
            // `Reference(__tuple<…>)` (a tuple of types with lifetime concerns —
            // e.g. structs — which `parse_function` rewrites that way).  Without
            // it, `convert(Tuple, Reference(__tuple<…>))` fails and the user sees
            // "expected __tuple<…>, got (…)" even though the SAME tuple as a
            // function's final expression compiles.  parse_return is the statement
            // path; block_result is the tail path — they must agree.
            let tuple_rewritten = !self.first_pass
                && matches!(t, Type::Tuple(_))
                && tail_has_tuple_leaf(v.unspan())
                && matches!(&r_type, Type::Reference(d, _) if self.data.def(*d).name().starts_with("__tuple<"))
                && {
                    let synthetic_d_nr = if let Type::Reference(d, _) = &r_type {
                        *d
                    } else {
                        unreachable!()
                    };
                    self.rewrite_tail_tuple_to_synthetic_struct(synthetic_d_nr, &mut v);
                    true
                };
            if t == Type::Null {
                v = self.null(&r_type);
            } else if !tuple_rewritten && !self.convert(&mut v, &t, &r_type) {
                self.validate_convert("return", &t, &r_type, &expr_start.position);
            }
            // Phase 1b (inline-lift-safety): mirror block_result's ref/enum
            // merge for mid-body `return` statements.  Without this, a function
            // like `fn f(c) -> Inner { if ... return c.items[i]; Inner{} }` loses
            // the `[c]` dep from the mid-body return path (only the owned-fresh
            // tail reaches block_result), and codegen's 0x8000 gate misses at
            // the call site → caller store corruption.  Skip for generic
            // templates (same I9-var rationale as block_result line 340).
            // Vector arm deliberately not mirrored: mid-body Vector returns can
            // reference globals/locals which ref_return would promote to hidden
            // ref args, breaking callers (see 01b for full analysis).
            // #355: set when the new one-buffer vector arm below handled
            // this site — the legacy `__ref_1` OpAppendVector injection
            // further down must then NOT fire a second copy.
            let mut vector_bound = false;
            if self.data.def_type(self.context) != DefType::Generic {
                // Explicit `return <expr>;`: the full body is not available
                // here, so pass the return expression itself as a one-element
                // body — `ref_return`'s one-buffer binding substitutes /
                // copy-rewrites inside it (the reassignment guard does not
                // apply: explicit return already copies the value).
                if let Type::Reference(td, ls) = &t {
                    if self.return_field_base_is_call(&v) {
                        // #425 sibling — `return mk().field` projects a struct
                        // field of an inline-call temporary.  The call result is
                        // freed at scope exit (lifted to `__lift_N`), so the
                        // sub-ref dangles; copy the field's record into an owned
                        // buffer first (the same owned-copy `return d.field` gets
                        // from `ref_return`'s named-local leg).
                        let w = self.materialize_view_return(*td, &mut v);
                        self.ref_return(&[w], std::slice::from_mut(&mut v), RetSite::MidReturn);
                    } else if ls.is_empty() {
                        let extra = Self::collect_hidden_ref_args(&v, &self.data);
                        if !extra.is_empty() {
                            let ls_own = extra.clone();
                            self.ref_return(
                                &ls_own,
                                std::slice::from_mut(&mut v),
                                RetSite::MidReturn,
                            );
                        }
                    } else if self.return_views_local(ls) {
                        // #306: mid-body `return <view of a local>` — copy it
                        // into an owned work-ref before it escapes (mirrors
                        // block_result's tail handling).
                        let w = self.materialize_view_return(*td, &mut v);
                        self.ref_return(&[w], std::slice::from_mut(&mut v), RetSite::MidReturn);
                    } else {
                        let ls_own: Vec<u16> = ls.to_vec();
                        self.ref_return(&ls_own, std::slice::from_mut(&mut v), RetSite::MidReturn);
                    }
                } else if let Type::Enum(e_d, true, ls) = &t {
                    // @PLN25 single-payload: a mid-body `return <nullable-element>` whose value is
                    // coerced to a dense `S` keeps `t` as the synth `__nullable<S>` Enum tail type,
                    // while the fn's DECLARED return is a dense `Reference(S)`.  The value is a VIEW
                    // into a local (the unwrapped payload) — left as a view it makes the fn
                    // VIEW-classified, so a SIBLING OWNED return on another path (a fallback `S{}`)
                    // is never freed by the caller (149: `map_get_hex` fallback `make_hex(0,0)` × N
                    // leaks).  Detect it from the TYPES (a `__nullable<S>` Enum tail + dense
                    // `Reference(S)` declared) — NOT the IR source, so a DIRECT `v[i]` unwrap
                    // qualifies too — and copy the view into an OWNED buffer so the fn is
                    // owned-classified.  No `nrvo_collapse_tail_set` (its work-ref→caller-buffer
                    // rename + re-OpDatabase was the documented direct-`v[i]` free-list-corruption
                    // hazard, now also defused by zero-on-claim; the plain copy here does not
                    // rename).  Gated on `__nullable<>` so a real user struct-enum return is
                    // untouched.
                    let declared_ret = self.data.def(self.context).returned().clone();
                    if let Type::Reference(rtd, _) = declared_ret
                        && self.data.def(*e_d).name().starts_with("__nullable<")
                    {
                        let w = self.materialize_view_return(rtd, &mut v);
                        self.ref_return(&[w], std::slice::from_mut(&mut v), RetSite::MidReturn);
                    } else if self.return_field_base_is_call(&v) {
                        // #425 sibling — `return mk().field` where `field` is a
                        // struct-enum (heap record): the inline-call base is freed
                        // at scope exit, so copy the field's record into an owned
                        // buffer first.  The Reference arm above does the same for
                        // a struct field; this is the struct-enum twin.
                        let ed = *e_d;
                        let w = self.materialize_view_return(ed, &mut v);
                        self.ref_return(&[w], std::slice::from_mut(&mut v), RetSite::MidReturn);
                    } else {
                        let ls_own: Vec<u16> = ls.to_vec();
                        self.ref_return(&ls_own, std::slice::from_mut(&mut v), RetSite::MidReturn);
                    }
                } else if let Type::Vector(_, ls) = &t
                    && self.return_buffer().is_some()
                {
                    // #355: a mid-body VECTOR return whose value comes from
                    // a CALL (a site work ref backs it) binds to the one
                    // buffer; `RetSite::MidReturn` keeps `ref_return` from
                    // renaming a site local into the fn-wide buffer (the
                    // 01b hazard that kept this arm un-mirrored).  Literal /
                    // named-local returns keep the legacy `__ref_1` append
                    // path below — its element-copy handles nested rows,
                    // which a plain buffer append would shallow-copy.
                    let ls_own: Vec<u16> = if ls.is_empty() {
                        Self::collect_hidden_ref_args(&v, &self.data)
                    } else {
                        ls.to_vec()
                    };
                    let site_refs: Vec<u16> = ls_own
                        .iter()
                        .copied()
                        .filter(|w| {
                            let nm = self.vars.name(*w);
                            nm.starts_with("__ref_") || nm.starts_with("__rref_")
                        })
                        .collect();
                    if !site_refs.is_empty() {
                        vector_bound = true;
                        self.ref_return(
                            &site_refs,
                            std::slice::from_mut(&mut v),
                            RetSite::MidReturn,
                        );
                    }
                }
            }
            if let Type::Text(ls) = &t {
                self.text_return(ls);
            } else if !self.first_pass {
                // When a function returns a vector and the caller provides an output
                // buffer (__ref_1 as a function argument), an explicit `return expr`
                // where `expr` is backed by a local __vdb_N store would return a
                // dangling DbRef: __vdb_N is freed before the return.
                //
                // Fix: if __ref_1 is a function argument and the returned expression
                // is NOT already backed by __ref_1 (dep does not contain ref1_var),
                // inject OpAppendVector to copy the elements into __ref_1 and return
                // __ref_1 instead.
                if let Type::Vector(elm_tp, dep) = &t {
                    let ref1_var = self.vars.var("__ref_1");
                    // `__ref_1` is the promoted-local name after ref_return renames
                    // the signature-time `__retbuf` placeholder.  When a function
                    // returns a PARAMETER directly (`return v`) without going through
                    // ref_return (because the parameter is not a work-ref), the
                    // buffer stays named `__retbuf` and vars.var("__ref_1") returns
                    // MAX.  Fall back to return_buffer() only when the returned value
                    // is backed by a PARAMETER variable — a fresh LOCAL vector
                    // (`return o`) is NOT delivered here: copying it into __retbuf
                    // would orphan the local on a MID-BODY return (it never reaches
                    // its scope-free).  A fresh-local TAIL return is instead promoted
                    // by `block_result`'s #437 tail-intercept (strip the `return`,
                    // route through the implicit-tail ref_return + NRVO — no copy).
                    // (a, _) keeps the buffer-attr index for the #437 dep finalize.
                    let (buf_attr, buf_var) =
                        if ref1_var != u16::MAX && self.vars.is_argument(ref1_var) {
                            (self.return_buffer().map_or(u16::MAX, |(a, _)| a), ref1_var)
                        } else if let Some((a, bv)) = self.return_buffer()
                            && dep.iter().any(|&d| d != bv && self.vars.is_argument(d))
                        {
                            (a, bv)
                        } else {
                            (u16::MAX, u16::MAX)
                        };
                    if !vector_bound && buf_var != u16::MAX && !dep.contains(&buf_var) {
                        // @P314 — narrow-aware element type (see `append_elem_tp`).
                        let elm = (**elm_tp).clone();
                        let rec_tp = self.append_elem_tp(&elm);
                        // Clear first: delivery REPLACES the buffer content
                        // (a caller's loop reuses the same fn-scoped buffer;
                        // without the clear each iteration's elements pile
                        // on top of the previous ones).
                        let clear = self.cl("OpClearVector", &[Value::Var(buf_var)]);
                        let append = self.cl(
                            "OpAppendVector",
                            &[Value::Var(buf_var), v, Value::Int(rec_tp)],
                        );
                        *val = Value::Insert(vec![
                            clear,
                            append,
                            Value::Return(Box::new(Value::Var(buf_var))),
                        ]);
                        // #437 — finalize the return-type dep to {__retbuf}, the step
                        // the implicit-tail path does (fwd_copy_409, ~825) and this
                        // explicit path omitted.  An arg / struct-field return
                        // (`return v` / `return b.v`) already element-copied its value
                        // INTO __retbuf above, but left the SIGNATURE a bare vector —
                        // so a caller (which consults only the signature) rebound its
                        // result var to a fresh empty store and the first in-place
                        // `+=` DROPPED the returned elements (#437).  Finalizing the
                        // dep makes the caller bind to the buffer it passed, so the
                        // result owns an appendable store and `+=` grows it in place.
                        if buf_attr != u16::MAX {
                            self.data.definitions[self.context as usize].returned =
                                Type::Vector(Box::new(elm), Deps::attrs(vec![buf_attr]));
                        }
                        return;
                    }
                }
                // @PLAN51 probe 39 — Reference parallel of the Vector
                // arm above.  A function returning a heap struct that
                // has been ref_return-promoted to a caller-side hidden
                // buffer (`__ref_1`) leaks ONE store per mid-body
                // `return borrowed_slice` when the returned DbRef is
                // NOT backed by `__ref_1`.  `OpReturn` then writes the
                // borrowed 12-byte DbRef into the caller's buffer slot
                // — orphaning the buffer's pre-allocated store.
                //
                // Pattern: `for x in vec { ... return x.field[i]; } default`.
                // probe 39's `map_get_hex` is the canonical case
                // (lib/moros_map's deep-slice borrow).
                //
                // Fix: deep-copy the borrowed slice into `__ref_1` via
                // OpCopyRecord, then return `__ref_1`.  Mirrors the
                // Vector arm's OpAppendVector treatment.
            }
        } else if !self.first_pass && r_type != Type::Void {
            diagnostic!(self.lexer, Level::Error, "Expect expression after return");
        }
        *val = Value::Return(Box::new(v));
    }

    /// Parse an assert or panic keyword call: `assert(expr, msg)` / `panic(msg)`.
    /// The opening `(` is consumed by the caller; this function parses args and `)`.
    pub(crate) fn parse_intrinsic_call(&mut self, val: &mut Value, name: &str) -> Type {
        let call_pos = self.lexer.pos().clone();
        let mut list = Vec::new();
        let mut types = Vec::new();
        if !self.lexer.has_token(")") {
            loop {
                let mut p = Value::Null;
                let t = self.expression(&mut p);
                types.push(t);
                list.push(p);
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token(")");
        }
        let ret = self.parse_call_diagnostic(val, name, &list, &types, &call_pos);
        // Plan-07 phase 1, step 1.13 — wrap intrinsic-keyword calls
        // (`assert(...)`, `panic(...)`) at the `(` token so runtime
        // failure inside `n_panic` / `n_assert` carries the call site
        // position into `state.source_spans`.  Mirrors the wrap in
        // `parse_call` for the regular fn-call dispatch path.
        if !self.first_pass && matches!(val, Value::Call(_, _) | Value::CallRef(_, _)) {
            let inner = std::mem::replace(val, Value::Null);
            *val = Value::with_span(call_pos, inner);
        }
        ret
    }

    /// Extract the assert condition expression from the source line.
    /// Reads the line at `pos.file:pos.line`, finds `assert(`, and extracts
    /// the text up to the matching `)`.
    fn extract_assert_expr(pos: &crate::lexer::Position) -> String {
        let line = Self::read_source_line(&pos.file, pos.line);
        // Find "assert(" and extract the condition
        if let Some(start) = line.find("assert(") {
            let after = start + 7; // skip "assert("
            let bytes = line.as_bytes();
            let mut depth = 1;
            let mut end = after;
            while end < bytes.len() && depth > 0 {
                match bytes[end] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b'"' => {
                        // Skip string literals
                        end += 1;
                        while end < bytes.len() && bytes[end] != b'"' {
                            if bytes[end] == b'\\' {
                                end += 1;
                            }
                            end += 1;
                        }
                    }
                    _ => {}
                }
                if depth > 0 {
                    end += 1;
                }
            }
            let expr = line[after..end].trim();
            // If it contains a comma, only take up to the first top-level comma
            // (the rest is the user message argument).
            let mut comma_depth = 0;
            for (i, b) in expr.bytes().enumerate() {
                match b {
                    b'(' | b'[' | b'{' => comma_depth += 1,
                    b')' | b']' | b'}' => comma_depth -= 1,
                    b',' if comma_depth == 0 => return expr[..i].trim().to_string(),
                    b'"' => {
                        // skip — don't count commas inside strings
                        // (simplified: the expression without message has no commas at top level)
                    }
                    _ => {}
                }
            }
            expr.to_string()
        } else {
            "assert failure".to_string()
        }
    }

    /// Read a single source line from a file (or VirtFS under WASM).
    fn read_source_line(file: &str, line: u32) -> String {
        #[cfg(feature = "wasm")]
        {
            if let Some(content) = crate::wasm::virt_fs_get(file) {
                return content
                    .lines()
                    .nth(line as usize - 1)
                    .unwrap_or("")
                    .to_string();
            }
        }
        if let Ok(content) = std::fs::read_to_string(file) {
            content
                .lines()
                .nth(line as usize - 1)
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        }
    }

    // <call> ::= [ <expression> { ',' <expression> } ] ')'
    pub(crate) fn parse_call_diagnostic(
        &mut self,
        val: &mut Value,
        name: &str,
        list: &[Value],
        types: &[Type],
        call_pos: &Position,
    ) -> Type {
        if name == "assert" {
            let mut test = list[0].clone();
            self.convert(&mut test, &types[0], &Type::Boolean);
            let message = if list.len() > 1 {
                list[1].clone()
            } else {
                // Extract the assert expression from the source line.
                let expr = Self::extract_assert_expr(call_pos);
                Value::str(&expr)
            };
            if self.first_pass {
                *val = Value::Null;
                return Type::Void;
            }
            let d_nr = self.data.def_nr("n_assert");
            *val = Value::Call(
                d_nr,
                vec![
                    test,
                    message,
                    Value::str(&call_pos.file),
                    Value::Int(call_pos.line as i32),
                ],
            );
            Type::Void
        } else if name == "panic" {
            let message = if list.is_empty() {
                Value::str("panic")
            } else {
                list[0].clone()
            };
            if self.first_pass {
                *val = Value::Null;
                return Type::Void;
            }
            let d_nr = self.data.def_nr("n_panic");
            *val = Value::Call(
                d_nr,
                vec![
                    message,
                    Value::str(&call_pos.file),
                    Value::Int(call_pos.line as i32),
                ],
            );
            Type::Void
        } else {
            // log_info / log_warn / log_error / log_fatal
            let message = if list.is_empty() {
                Value::str("")
            } else {
                list[0].clone()
            };
            if self.first_pass {
                *val = Value::Null;
                return Type::Void;
            }
            let fn_name = format!("n_{name}");
            let d_nr = self.data.def_nr(&fn_name);
            *val = Value::Call(
                d_nr,
                vec![
                    message,
                    Value::str(&call_pos.file),
                    Value::Int(call_pos.line as i32),
                ],
            );
            Type::Void
        }
    }

    #[allow(clippy::too_many_lines)] // pre-existing length; A5.6b.2 added ~9 lines
    pub(crate) fn parse_call(&mut self, val: &mut Value, source: u16, name: &str) -> Type {
        let call_pos = self.lexer.pos().clone();
        let mut list = Vec::new();
        let mut types = Vec::new();
        let mut arg_pos: Vec<Position> = Vec::new();
        if self.lexer.has_token(")") {
            // Check for zero-argument fn-ref call
            if self.vars.name_exists(name) {
                let v_nr = self.vars.var(name);
                if let Type::Function(param_types, ret_type, _) = self.vars.tp(v_nr).clone()
                    && param_types.is_empty()
                {
                    // P227: text-returning fn-ref calls need exactly ONE
                    // work-buffer at caller-function scope (the return-value
                    // buffer that the lambda fills via its hidden RefVar(Text)
                    // attr).  The fn-ref TYPE's `Type::Text(deps)` is always
                    // `deps = []`, so the previous `(0..deps.len())` count
                    // was always zero — leaving the lambda's stack slot for
                    // its work-buffer empty and causing a SIGSEGV when the
                    // lambda body read it.  Allocating one work_text var here
                    // matches the canonical "one return buffer per text fn"
                    // ABI; lambdas with multiple `RefVar(Text)` hidden attrs
                    // (the rare case) are diagnosed separately.
                    let work_vars: Vec<u16> = if matches!(ret_type.as_ref(), Type::Text(_)) {
                        vec![self.vars.work_text(&mut self.lexer)]
                    } else {
                        vec![]
                    };
                    if !self.first_pass {
                        self.var_usages(v_nr, true);
                        let mut args = vec![];
                        // inject work-buffer DbRef blocks before __closure (zero-param case).
                        // clear the work buffer before each call so loop iterations start fresh.
                        let ref_def = self.data.def_nr("reference");
                        for &wv in &work_vars {
                            args.push(v_block(
                                vec![
                                    crate::data::v_set(wv, Value::Text(String::new())),
                                    self.cl("OpCreateStack", &[Value::Var(wv)]),
                                ],
                                Type::Reference(ref_def, Deps::frame1(wv)),
                                "cref_work_buf",
                            ));
                        }
                        // closure is embedded in the 16-byte fn-ref slot; fn_call_ref
                        // pushes it automatically — no explicit injection needed here.
                        // mark captured vars as read at the call site
                        for &cv in &std::mem::take(&mut self.last_closure_captured_vars) {
                            self.var_usages(cv, true);
                        }
                        *val = Value::CallRef(v_nr, args);
                    }
                    return *ret_type;
                }
            }
            return self.call(val, source, name, &list, &Vec::new(), &[], &[]);
        }
        let fn_def_nr = if self.first_pass {
            None
        } else {
            let d_nr = self.data.def_nr(&format!("n_{name}"));
            (d_nr != u32::MAX).then_some(d_nr)
        };
        let mut arg_idx = 0usize;
        let mut named_args: Vec<(String, Value, Type)> = Vec::new();
        let mut in_named = false;
        loop {
            // Check for named argument: `name: expr`
            if let Some(arg_name) = self.lexer.peek_named_arg() {
                in_named = true;
                self.lexer.has_identifier(); // consume name
                self.lexer.has_token(":"); // consume :
                // #432 — a named vector-literal argument (`f(v: [10, 255, 20])`)
                // builds at the parameter's element width too.  Map the name to its
                // parameter to seed the hint, then clear it after parsing.
                let hint_d_nr = self.data.def_nr(&format!("n_{name}"));
                if hint_d_nr != u32::MAX {
                    for a in 0..self.data.attributes(hint_d_nr) {
                        if self.data.attr_name(hint_d_nr, a) == arg_name {
                            let expected = self.data.attr_type(hint_d_nr, a);
                            if Self::seeds_vector_hint(&expected) {
                                self.expected = expected;
                            }
                            break;
                        }
                    }
                }
                let mut p = Value::Null;
                let t = self.expression(&mut p);
                self.expected = Type::Unknown(0);
                named_args.push((arg_name, p, t));
                // accept trailing comma on the last named arg.
                if !self.lexer.has_token(",") || self.lexer.peek_token(")") {
                    break;
                }
                continue;
            }
            if in_named && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Positional argument after named argument"
                );
            }
            if let Some(d_nr) = fn_def_nr
                && arg_idx < self.data.attributes(d_nr)
            {
                let expected = self.data.attr_type(d_nr, arg_idx);
                if matches!(expected, Type::Function(_, _, _)) {
                    self.expected = expected;
                }
            }
            // @PLN22 Phase 1 — hint the expected enum so a bare value-position
            // variant argument (`f(Red)`) resolves against the parameter's enum.
            // Resolved on BOTH passes (unlike the pass-2-only `fn_def_nr` above):
            // on pass 1 the callee is already registered, and skipping the hint
            // there would let a bare variant become a stray placeholder var that
            // shadows the real variant on pass 2.
            if !in_named {
                let hint_d_nr = self.data.def_nr(&format!("n_{name}"));
                if hint_d_nr != u32::MAX && arg_idx < self.data.attributes(hint_d_nr) {
                    let expected = self.data.attr_type(hint_d_nr, arg_idx);
                    if self.enum_context(&expected) {
                        self.expected = expected;
                    } else if Self::seeds_vector_hint(&expected) {
                        // #432 — seed a bare vector-literal argument's element width
                        // from the parameter type, so it builds at the callee's
                        // stride instead of `vector<integer>`.  Both passes (like the
                        // enum hint): the literal's element type must agree across
                        // passes, and the callee is already registered on pass 1.
                        self.expected = expected;
                    }
                }
            }
            // for map/filter/reduce, infer lambda hint from the vector
            // element type so that short-form |x| lambdas can infer types.
            if fn_def_nr.is_none()
                && !types.is_empty()
                && let Type::Vector(elm, _) = &types[0]
            {
                let elem = *elm.clone();
                let hint = match (name, arg_idx) {
                    ("map", 1) => Some(Type::Function(
                        vec![elem.clone()],
                        Box::new(elem),
                        Deps::none(),
                    )),
                    ("filter" | "any" | "all" | "count_if", 1) => Some(Type::Function(
                        vec![elem],
                        Box::new(Type::Boolean),
                        Deps::none(),
                    )),
                    ("reduce", 2) => {
                        let init_tp = types.get(1).cloned().unwrap_or(elem.clone());
                        Some(Type::Function(
                            vec![init_tp.clone(), elem],
                            Box::new(init_tp),
                            Deps::none(),
                        ))
                    }
                    _ => None,
                };
                if let Some(h) = hint {
                    self.expected = h;
                }
            }
            let mut p = Value::Null;
            // Capture each argument's start so a later type-mismatch diagnostic
            // (in `process_call_args`) points the caret at the argument, not at
            // the cursor drifted to `)` / `,`.
            arg_pos.push(self.lexer.peek_pos().clone());
            let t = self.expression(&mut p);
            self.expected = Type::Unknown(0);
            types.push(t);
            list.push(p);
            arg_idx += 1;
            // accept trailing comma on the last positional arg —
            // matching the struct-enum field list and enum variant list.
            if !self.lexer.has_token(",") || self.lexer.peek_token(")") {
                break;
            }
        }
        self.lexer.token(")");
        let ret = self.dispatch_call(
            val,
            source,
            name,
            &list,
            &types,
            &named_args,
            &call_pos,
            &arg_pos,
        );
        // Plan-07 phase 1, step 1.13 — wrap user-typed Call / CallRef
        // at the `(` token position so runtime errors inside the call
        // (panic, divide-by-zero in callee, etc.) can be reported with
        // the call site's source location.  Skip on first pass and skip
        // when dispatch left val unchanged (e.g. early-return paths).
        if !self.first_pass && matches!(val, Value::Call(_, _) | Value::CallRef(_, _)) {
            let inner = std::mem::replace(val, Value::Null);
            *val = Value::with_span(call_pos, inner);
        }
        ret
    }

    /// Dispatch a parsed call to the appropriate handler: diagnostics, special
    /// forms (`map/filter/reduce/sort/parallel_for`), fn-ref calls, or normal calls.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_call(
        &mut self,
        val: &mut Value,
        source: u16,
        name: &str,
        list: &[Value],
        types: &[Type],
        named_args: &[(String, Value, Type)],
        call_pos: &Position,
        arg_pos: &[Position],
    ) -> Type {
        if matches!(
            name,
            "assert" | "panic" | "log_info" | "log_warn" | "log_error" | "log_fatal"
        ) {
            return self.parse_call_diagnostic(val, name, list, types, call_pos);
        }
        match name {
            "parallel_for" => return self.parse_parallel_for(val, list, types),
            "par_fold" => return self.parse_par_fold(val, list, types),
            "map" => return self.parse_map(val, list, types),
            "filter" => return self.parse_filter(val, list, types),
            "reduce" => return self.parse_reduce(val, list, types),
            "sort" => return self.parse_sort(val, list, types),
            "insert" => return self.parse_insert(val, list, types),
            "reverse" => return self.parse_reverse(val, list, types),
            "any" => return self.parse_any(val, list, types),
            "all" => return self.parse_all(val, list, types),
            "count_if" => return self.parse_count_if(val, list, types),
            "next" if types.len() == 1 => {
                // CO1.6a: next(gen) — advance a coroutine iterator.
                // Encode value_size as second parameter so codegen can emit it.
                // @P327 — same encoding as the for-loop's `iterator()` path
                // in `parser/collections.rs`: high byte = channel tag
                // (1 = unified `next_into` for tuple yields).  Without this,
                // manual `next()` on `iterator<(integer, integer)>` routes
                // through the legacy text channel (size 16 ≡ `&str`) and
                // returns a `String` where Rust expected a tuple.
                if let Type::Iterator(inner, _) = &types[0] {
                    let yield_tp = (**inner).clone();
                    let byte_size = i32::from(crate::variables::size(
                        &yield_tp,
                        &crate::data::Context::Argument,
                    ));
                    // @PLAN16 phase 02 — same packed encoding as the
                    // for-loop's `iterator()` path; tag 1 = layout-driven
                    // tuple walk (kind codes appended as extra args), tag 2 =
                    // fn-ref.  `tuple_kinds` is the shared gate so the consumer
                    // and the generator's producer never diverge.
                    let tkinds = crate::coroutine_layout::tuple_kinds(&yield_tp);
                    // #401 — shared channel decision (float/single/enum get their
                    // own tags); the for-loop path uses the same helper, so manual
                    // `next()` no longer diverges (it dropped float/single/enum →
                    // native E0308 on `let var: f64 = coroutine_next_i64(..)`).
                    let channel_tag = crate::coroutine_layout::channel_tag(&yield_tp);
                    let value_size: i32 = (channel_tag << 8) | byte_size;
                    let op = self.data.def_nr("OpCoroutineNext");
                    let mut args = list.to_vec();
                    args.push(Value::Int(value_size));
                    if let Some(kinds) = &tkinds {
                        args.extend(kinds.iter().map(|k| Value::Int(k.code())));
                    }
                    *val = Value::Call(op, args);
                    return yield_tp;
                }
                if self.first_pass {
                    return Type::Unknown(0);
                }
            }
            "exhausted" if types.len() == 1 && matches!(&types[0], Type::Iterator(_, _)) => {
                // CO1.3c: exhausted(gen) on a coroutine iterator.
                let op = self.data.def_nr("OpCoroutineExhausted");
                *val = Value::Call(op, list.to_vec());
                return Type::Boolean;
            }
            _ => {}
        }
        if let Some(tp) = self.try_fn_ref_call(val, name, list, types) {
            return tp;
        }
        self.call(val, source, name, list, types, named_args, arg_pos)
    }

    /// Try to dispatch as a call through a function-reference variable.
    /// Returns `Some(return_type)` if `name` is a fn-ref variable, `None` otherwise.
    fn try_fn_ref_call(
        &mut self,
        val: &mut Value,
        name: &str,
        list: &[Value],
        types: &[Type],
    ) -> Option<Type> {
        // P215: name lookup for outer-scope fn-ref captures.
        //
        // Bare-name reads route through `parser/objects.rs:162-200`
        // which scans `capture_context`; call syntax `name(args)`
        // bypasses that path and lands here.  When `name` matches a
        // `Type::Function` capturable from the outer scope, we need
        // to (a) push it to `captured_names` (drives
        // `synthesize_closure_record`'s attribute set), (b) create a
        // placeholder local var on the first pass so subsequent
        // lookups find it.  At call-emit time below, an
        // `is_outer_fnref` test on `capture_context` decides whether
        // to wrap the CallRef in a closure-record load.
        let outer_fnref_type = self
            .capture_context
            .iter()
            .find(|(n, t)| n == name && matches!(t, Type::Function(_, _, _)))
            .cloned()
            .map(|(_, t)| t);
        if !self.vars.name_exists(name) {
            let ctype = outer_fnref_type.clone()?;
            if !self.captured_names.iter().any(|(n, _)| n == name) {
                self.captured_names.push((name.to_string(), ctype.clone()));
            }
            let v_nr = self.create_var(name, &ctype);
            self.var_usages(v_nr, true);
        } else if outer_fnref_type.is_some() && !self.captured_names.iter().any(|(n, _)| n == name)
        {
            // Second-pass: var exists from first pass but
            // captured_names is fresh (reset per-lambda).  Re-record.
            if let Some(ctype) = outer_fnref_type.clone() {
                self.captured_names.push((name.to_string(), ctype));
            }
        }
        let v_nr = self.vars.var(name);
        let Type::Function(param_types, ret_type, _) = self.vars.tp(v_nr).clone() else {
            return None;
        };
        // P227: one work-buffer per text-returning fn-ref call.
        // The fn-ref TYPE's `Type::Text(deps)` is always `deps = []`,
        // so the previous deps-derived count was zero — leaving the
        // lambda's stack slot for its work-buffer empty and causing
        // SIGSEGV.  Allocating one work_text matches the canonical
        // "one return buffer per text fn" ABI; lambdas with multiple
        // `RefVar(Text)` hidden attrs are diagnosed separately.
        let work_vars: Vec<u16> = if matches!(ret_type.as_ref(), Type::Text(_)) {
            vec![self.vars.work_text(&mut self.lexer)]
        } else {
            vec![]
        };
        if !self.first_pass {
            if list.len() != param_types.len() {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Function reference '{name}' expects {} argument(s), got {}",
                    param_types.len(),
                    list.len()
                );
                return Some(*ret_type);
            }
            let mut converted = list.to_vec();
            for (i, expected) in param_types.iter().enumerate() {
                self.convert(&mut converted[i], &types[i], expected);
            }
            // inject hidden work-buffer DbRef args for text-returning lambdas.
            // Each block emits OpCreateStack → 12-byte DbRef, matching callee's &text param.
            // Order: visible params → work bufs → __closure (must match callee slot layout).
            // prepend v_set(wv, "") to clear the buffer so loop iterations start fresh.
            let ref_def = self.data.def_nr("reference");
            for &wv in &work_vars {
                converted.push(v_block(
                    vec![
                        crate::data::v_set(wv, Value::Text(String::new())),
                        self.cl("OpCreateStack", &[Value::Var(wv)]),
                    ],
                    Type::Reference(ref_def, Deps::frame1(wv)),
                    "cref_work_buf",
                ));
            }
            // inject hidden __closure argument — the closure allocation
            // expression is generated inline so it runs at the call site, avoiding
            // closure is embedded in the 16-byte fn-ref slot; fn_call_ref
            // pushes it automatically — no explicit injection needed at call sites.
            // mark captured vars as read at the call site
            for &cv in &std::mem::take(&mut self.last_closure_captured_vars) {
                self.var_usages(cv, true);
            }
            self.var_usages(v_nr, true);
            // P215: if we just captured this name from outer scope,
            // populate the placeholder var from the closure record's
            // field BEFORE the CallRef.  Without this the CallRef
            // reads garbage from the uninitialised local slot.  The
            // closure-record attribute was registered in
            // `synthesize_closure_record` (parser/vectors.rs:762);
            // `closure_param` (parser/vectors.rs:419) holds the DbRef
            // at runtime.  `get_field` produces the (d_nr,
            // closure_DbRef) tuple via the new fn_ref_field_read
            // gate added in P215 (parser/mod.rs::get_field).
            let call_ir = Value::CallRef(v_nr, converted);
            // P215: detect "this name was captured from outer scope" by
            // checking `captured_names` (populated either in this turn
            // through Step 1 above, or in a prior pass).  `name_exists`
            // returns true on the second pass even for captured names
            // (placeholder var was created in first pass), so we can't
            // gate just on `captured_via_closure` — that flag only
            // fires on the first pass when the var is created fresh.
            // P215: detect captured-from-outer status via
            // `capture_context` rather than `captured_names`, since
            // `captured_names` only tracks captures ADDED during the
            // current pass — second-pass `inner(y)` lookups don't
            // re-add and the flag would miss them.  `capture_context`
            // is populated at `parse_lambda` entry from the outer
            // scope's all-names (parser/vectors.rs:364) and is stable
            // across both passes.
            let was_captured = self
                .capture_context
                .iter()
                .any(|(n, t)| n == name && matches!(t, Type::Function(_, _, _)));
            if was_captured
                && self.closure_param != u16::MAX
                && let closure_rec_d = self.data.def(self.context).closure_record()
                && closure_rec_d != u32::MAX
            {
                let f_nr = self.data.attr(closure_rec_d, name);
                if f_nr != usize::MAX {
                    let load = self.get_field(closure_rec_d, f_nr, Value::Var(self.closure_param));
                    *val = v_block(
                        vec![crate::data::v_set(v_nr, load), call_ir],
                        *ret_type.clone(),
                        "captured_fn_ref_call",
                    );
                    return Some(*ret_type);
                }
            }
            *val = call_ir;
            // for void-return capturing lambdas, write updated closure
            // record fields back to the corresponding outer variables so the caller
            // observes mutations made inside the lambda body (e.g. `count += x`).
            // Non-void returns are not handled here — they require a temp to hold
            // the return value while writing back, which is left for A5.6 (1.1+).
            if matches!(*ret_type, Type::Void)
                && let Some(&closure_w) = self.closure_vars.get(&v_nr)
                && let Type::Reference(closure_rec_d, _) = self.vars.tp(closure_w).clone()
            {
                let n_attrs = self.data.attributes(closure_rec_d);
                let mut block: Vec<Value> = vec![val.clone()];
                for aid in 0..n_attrs {
                    let cap_name = self.data.attr_name(closure_rec_d, aid).clone();
                    let outer_v = self.vars.var(&cap_name);
                    if outer_v == u16::MAX {
                        continue;
                    }
                    // Plan-22 phase 02d-iii.e + @P319 — skip the
                    // write-back for ALL shared-reference captures,
                    // i.e. those stored in the closure record via the
                    // auto-Reference 12-byte DbRef encoding (the
                    // closure attribute is `Reference(d, deps)` with
                    // NON-EMPTY deps).  This covers boxed `__cell_<T>`
                    // scalars (02d-iii.e, the original case) AND struct
                    // / reference captures such as a captured `Mesh`
                    // whose `.vertices` vector is appended to inside the
                    // lambda (@P319).
                    //
                    // For these the closure holds a DbRef into the LIVE
                    // outer value, so body mutations already propagate
                    // through the shared store.  A bare
                    // `v_set(outer, OpGetDbRef(rec, off))` copies that
                    // 12-byte DbRef back over itself — a value no-op —
                    // but the reassignment's free-old-ref step releases
                    // the store the closure record still references.
                    // That premature free lets the next call reuse the
                    // store, clobbering the captured value: silent data
                    // loss when the trampled field is at offset 0 (a
                    // `len` reads back 0), or a SIGSEGV in `new_record`
                    // when it is at a non-zero offset.  Native compiles
                    // the same IR without the free, so this corrupted
                    // only the interpreter.  Only genuine by-VALUE
                    // captures (inline-bytes encoding, empty deps) need
                    // the write-back to observe their mutations.
                    if matches!(
                        self.data.attr_type(closure_rec_d, aid),
                        Type::Reference(_, ref deps) if !deps.is_empty()
                    ) {
                        continue;
                    }
                    let field_val = self.get_field(closure_rec_d, aid, Value::Var(closure_w));
                    block.push(v_set(outer_v, field_val));
                }
                if block.len() > 1 {
                    // Use Insert rather than Block: we must NOT create a new scope
                    // here because ___clos_1 (closure_w) is owned by the outer scope.
                    // A Block would cause scopes.rs to emit OpFreeRef at the inner
                    // scope exit, leaving a dangling ref for the next call.
                    *val = Value::Insert(block);
                }
            }
        }
        Some(*ret_type)
    }

    // Validate and rewrite a user-friendly `parallel_for(fn f, vec, threads)` call
    // into a `Value::Call(n_parallel_for_d_nr, [input, elem_size, return_size, threads, func])`.
    //
    // The parser intercepts calls by name "parallel_for" before normal overload
    // resolution.  Compile-time checks performed here:
    // - First arg must be `Type::Function(args, ret)` (produced by `fn <name>` expression).
    // - Second arg must be `Type::Vector(T, _)`.
    // - Worker's first parameter must be a reference to T (type checked by name).
    // - Return type must be a primitive: integer, long, float, or boolean.
    // - Extra arg count must match the worker's extra parameters (args[1..]).
    /// Compiler special-case for `reduce(v: vector<T>, init: U, f: fn(U, T) -> U) -> U`.
    /// Generates inline bytecode equivalent to a left-fold over the vector.
    pub(crate) fn parse_reduce(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            // On first pass, return the accumulator type (second arg) if available.
            if types.len() >= 2 {
                return types[1].clone();
            }
            return Type::Unknown(0);
        }
        if list.len() != 3 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce requires 3 arguments: reduce(vector, init, fn f)"
            );
            return Type::Unknown(0);
        }
        let _in_elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: first argument must be a vector"
            );
            return Type::Unknown(0);
        };
        let acc_type = types[1].clone();
        let (fn_param_types, _fn_ret_type) = if let Type::Function(params, ret, _) = &types[2] {
            (params.clone(), *ret.clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: third argument must be a function reference (use fn <name>)"
            );
            return Type::Unknown(0);
        };
        if fn_param_types.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: function must take exactly two arguments (accumulator, element)"
            );
            return Type::Unknown(0);
        }
        // Extract the compile-time d_nr from the fn-ref value (always Value::Int(d_nr)).
        let fn_d_nr = if let Value::Int(d) = &list[2] {
            *d as u32
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: function must be a compile-time constant (use fn <name>)"
            );
            return Type::Unknown(0);
        };

        let acc_var = self.create_unique("reduce_acc", &acc_type);
        self.vars.defined(acc_var);

        let mut in_type = types[0].clone();
        let vec_copy_var = self.create_unique("reduce_vec", &in_type);
        in_type = in_type.depending(vec_copy_var);

        let iter_var = self.create_unique("reduce_idx", &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique("reduce_elm", &var_tp);
        self.vars.defined(for_var);

        let mut create_iter_code = Value::Var(vec_copy_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter_code, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut test_for = Value::Var(for_var);
        self.convert(&mut test_for, &var_tp, &Type::Boolean);
        let not_test = self.cl("OpNot", &[test_for]);
        let break_if_null = v_if(
            not_test,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );

        // Use Value::Call(d_nr, ...) directly — no fn_ref_var local needed.
        let fold_step = v_set(
            acc_var,
            Value::Call(fn_d_nr, vec![Value::Var(acc_var), Value::Var(for_var)]),
        );

        let loop_body = vec![for_next, break_if_null, fold_step];

        *val = v_block(
            vec![
                v_set(acc_var, list[1].clone()),
                v_set(vec_copy_var, list[0].clone()),
                create_iter_code,
                v_loop(loop_body, "reduce loop"),
                Value::Var(acc_var),
            ],
            acc_type.clone(),
            "reduce",
        );
        acc_type
    }

    // <size> ::= ( <type> | <var> ) ')'
    pub(crate) fn parse_size(&mut self, val: &mut Value) -> Type {
        let mut found = false;
        let lnk = self.lexer.link();
        if let Some(id) = self.lexer.has_identifier() {
            let d_nr = self.data.def_nr(&id);
            if d_nr != u32::MAX && self.data.def_type(d_nr) != DefType::EnumValue {
                if !self.first_pass && self.data.def_type(d_nr) == DefType::Unknown {
                    found = true;
                } else if let Some(tp) = self.parse_type(u32::MAX, &id, false) {
                    found = true;
                    if !self.first_pass {
                        // Post-2c: prefer the alias's forced size(N) annotation.
                        // `d_nr` (local above) is the def_nr of the alias the user
                        // typed — e.g. i32 — not the base integer it collapses to
                        // via type_elm.  Only forced_size on the alias applies.
                        let forced = self.data.forced_size(d_nr);
                        let packed = tp.size(false);
                        *val = if let Some(n) = forced {
                            Value::Int(i32::from(n))
                        } else if packed > 0 {
                            // Range-constrained integer: use packed field size
                            Value::Int(i32::from(packed))
                        } else {
                            Value::Int(i32::from(
                                self.database
                                    .size(self.data.def(self.data.type_elm(&tp)).known_type()),
                            ))
                        };
                    }
                }
            }
        }
        if !found {
            let mut drop = Value::Null;
            self.lexer.revert(lnk);
            let tp = self.expression(&mut drop);
            let e_tp = self.data.type_elm(&tp);
            if e_tp != u32::MAX {
                found = true;
                if matches!(tp, Type::Enum(_, true, _) | Type::Reference(_, _)) && !self.first_pass
                {
                    // Polymorphic enum or reference: size depends on runtime variant.
                    *val = self.cl("OpSizeofRef", &[drop]);
                } else {
                    *val = Value::Int(i32::from(
                        self.database.size(self.data.def(e_tp).known_type()),
                    ));
                }
            }
        }
        if !self.first_pass && !found {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect a variable or type after sizeof"
            );
        }
        self.lexer.token(")");
        I32.clone()
    }

    /// `type_name(expr)` — compile-time intrinsic that returns the static type
    /// of `expr` as a text constant.  Works on both type names and expressions:
    /// `type_name(integer)`, `type_name(my_var)`, `type_name(1 + 2)`.
    pub(crate) fn parse_type_name(&mut self, val: &mut Value) -> Type {
        // Try parsing as a type name first (like sizeof does).
        let mut found = false;
        let lnk = self.lexer.link();
        if let Some(id) = self.lexer.has_identifier() {
            let d_nr = self.data.def_nr(&id);
            if d_nr != u32::MAX && self.data.def_type(d_nr) != DefType::EnumValue {
                if !self.first_pass && self.data.def_type(d_nr) == DefType::Unknown {
                    found = true;
                } else if let Some(tp) = self.parse_type(u32::MAX, &id, false) {
                    found = true;
                    if !self.first_pass {
                        *val = Value::Text(self.data.type_name_str(&tp));
                    }
                }
            }
        }
        if !found {
            let mut drop = Value::Null;
            self.lexer.revert(lnk);
            let tp = self.expression(&mut drop);
            if !self.first_pass {
                *val = Value::Text(self.data.type_name_str(&tp));
            }
        }
        self.lexer.token(")");
        Type::Text(Deps::none())
    }

    /// #432 — should a bare vector-literal argument be seeded with this parameter
    /// type's element width (`vector_hint`)?  Only for a CONCRETE narrow-integer
    /// element (`vector<u8>` … `vector<i32>`): an untyped integer literal infers
    /// `vector<integer>` (8-byte stride) and the callee would reinterpret it at the
    /// narrow stride.  Each branch below is deliberately NOT covered:
    /// - A generic `vector<T>` (element is a `Reference` to a type-var) must NOT
    ///   seed — the literal cannot be built at an abstract element type, and seeding
    ///   it wrongly fails `min_of([3, 1, 2])` with "would lose precision".
    /// - `vector<single>` is excluded on purpose: a float literal infers
    ///   `vector<float>` and f64→f32 is rejected as precision-loss regardless of the
    ///   constant, so seeding would turn the (separate, pre-existing) stride bug
    ///   into a fresh compile error — out of #432's "integer-vector literal" scope.
    /// - Struct/enum element vectors already build from their own literal.
    ///
    /// Recurses through nested vector layers so `vector<vector<u8>>` seeds too (the
    /// outer literal is seeded; inner literals thread their element type through
    /// `var_tp`).  The leaf must be a narrow integer.
    pub(crate) fn seeds_vector_hint(expected: &Type) -> bool {
        match expected {
            Type::Vector(elem, _) => {
                matches!(**elem, Type::Integer(_)) || Self::seeds_vector_hint(elem)
            }
            _ => false,
        }
    }

    // <call> ::= [ <expression> { ',' <expression> } ] ')'
    pub(crate) fn parse_method(&mut self, val: &mut Value, md_nr: u32, on: Type) -> Type {
        let mut list = vec![val.clone()];
        let mut types = vec![on];
        // arg_pos aligns with `list` by index; slot 0 is the receiver (its
        // position is the method-name token, the best available caret).
        let mut arg_pos: Vec<Position> = vec![self.lexer.peek_pos().clone()];
        if self.lexer.has_token(")") {
            return self.call_nr(val, md_nr, &list, &types, true, &arg_pos);
        }
        loop {
            // #432 — `list[0]` is the receiver (attribute 0), so `list.len()` is the
            // attribute index of the explicit argument about to be parsed.  Seed a
            // bare vector-literal argument's element width from that parameter type,
            // matching the free-function path in `parse_call`.
            if md_nr != u32::MAX && list.len() < self.data.attributes(md_nr) {
                let expected = self.data.attr_type(md_nr, list.len());
                if Self::seeds_vector_hint(&expected) {
                    self.expected = expected;
                }
            }
            let mut p = Value::Null;
            arg_pos.push(self.lexer.peek_pos().clone());
            let t = self.expression(&mut p);
            self.expected = Type::Unknown(0);
            types.push(t);
            list.push(p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        self.call_nr(val, md_nr, &list, &types, true, &arg_pos)
    }

    pub(crate) fn parse_parameters(&mut self) -> (Vec<Type>, Vec<Value>) {
        let mut list = vec![];
        let mut types = vec![];
        if self.lexer.has_token(")") {
            return (types, list);
        }
        loop {
            let mut p = Value::Null;
            types.push(self.expression(&mut p));
            list.push(p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        (types, list)
    }

    /// Parse `parallel { arm1; arm2; ... }`.
    /// Each semicolon-separated expression in the block becomes one concurrent arm.
    pub(crate) fn parse_parallel(&mut self, code: &mut Value) {
        self.lexer.token("{");
        // INVARIANT (load-bearing — the capture check below depends on it).
        // Snapshot which vars are already DEFINED at the block's opening brace.
        // The two-pass parser pre-populates the whole function's var table in
        // pass 1, so var-nr ordering CANNOT separate an enclosing var from an
        // arm-local one.  The signal that does is: **in this (non-first) pass,
        // `is_defined` is set in source order**, so a var defined *before* the
        // block reads defined here (enclosing) and one first defined *inside* an
        // arm reads undefined (arm-local).  Params read defined (`become_argument`)
        // ⇒ enclosing.  Two known exceptions read defined-at-entry despite being
        // arm-local — for-loop vars (handled by the `was_loop_var` exclusion) and
        // compiler temps (the `_`/`#` name exclusion); both live in `is_user_var`.
        // If a future change sets `is_defined` out of this source order, the
        // monotonicity `debug_assert!` in `note_mutation`/`note_param` and the
        // `tests/scripts/171-parallel-armlocal-ok.loft` arm-local-compiles guard
        // are the alarms.
        let enclosing: Vec<bool> = (0..self.vars.count())
            .map(|v| self.vars.is_defined(v))
            .collect();
        let mut arms = Vec::new();
        while !self.lexer.peek_token("}") {
            let mut arm = Value::Null;
            self.expression(&mut arm);
            if arm != Value::Null {
                arms.push(arm);
            }
            self.lexer.has_token(";");
        }
        self.lexer.token("}");
        if !self.first_pass {
            if arms.is_empty() {
                diagnostic!(self.lexer, Level::Warning, "Empty parallel block");
            }
            self.reject_unsound_parallel_captures(&arms, &enclosing);
        }
        *code = Value::Parallel(arms);
    }

    /// Soundness floor for `parallel {}` (plan-57 Bug 2).  An arm runs in an
    /// isolated worker — a read-only clone of the heap plus a private stack — so
    /// only *reading* an enclosing local is sound (the value is copied in).
    /// Everything else is the unbuilt/broken surface and must be a clean compile
    /// error, not a silent no-op or a crash:
    /// - **writing or mutating** an enclosing local — the write is dropped
    ///   (scalar/text) or crashes on the read-only store clone (heap);
    /// - **capturing a parameter** (read or write) — SIGSEGVs at teardown.
    ///
    /// Reads of enclosing locals (any position/type) stay legal — that is the
    /// proven-sound P245 surface that test-81 guards.  Known residual: passing a
    /// captured heap value to a function that mutates it is transitive and is not
    /// caught here (it still faults at runtime); catching it needs callee
    /// analysis.  The full capture model is deferred to its driving consumer (the
    /// server/client library) — see the plan-57 deferred-follow-ups.
    fn reject_unsound_parallel_captures(&mut self, arms: &[Value], enclosing: &[bool]) {
        let mut viol: Vec<(u16, ParViolation)> = Vec::new();
        for arm in arms {
            self.collect_parallel_violations(arm, enclosing, &mut viol);
        }
        let mut reported: Vec<u16> = Vec::new();
        for (v, _) in &viol {
            if reported.contains(v) {
                continue;
            }
            reported.push(*v);
            // A var flagged as both Param and Mutation reads clearest as Param.
            let is_param = viol
                .iter()
                .any(|(v2, k)| v2 == v && *k == ParViolation::Param);
            let name = self.vars.name(*v).to_string();
            if is_param {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot capture function parameter '{name}' inside a parallel arm — \
                     a parallel arm runs in an isolated worker with no safe access to the \
                     parent frame; copy '{name}' into a local before the block, or pass it \
                     to a function-call arm"
                );
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot write or mutate enclosing-scope variable '{name}' inside a \
                     parallel arm — an arm runs in an isolated worker, so the write does not \
                     propagate (and heap mutation crashes); a parallel arm may only READ \
                     enclosing state, not write it"
                );
            }
        }
    }

    /// Recursive walk collecting capture violations in one arm (see
    /// `reject_unsound_parallel_captures`).  Reads of enclosing non-parameter
    /// locals are sound and are never flagged.
    fn collect_parallel_violations(
        &self,
        node: &Value,
        encl: &[bool],
        out: &mut Vec<(u16, ParViolation)>,
    ) {
        match node.unspan() {
            // Direct assignment target (scalar `=`/`+=`, vector concat-reassign).
            Value::Set(v, rhs) => {
                self.note_mutation(*v, encl, out);
                self.collect_parallel_violations(rhs, encl, out);
            }
            Value::TuplePut(v, _, rhs) => {
                self.note_mutation(*v, encl, out);
                self.collect_parallel_violations(rhs, encl, out);
            }
            // In-place / element / field mutation hides the host in args[0].
            Value::Call(d, args) => {
                if is_mutating_op(self.data.def(*d).name())
                    && let Some(host) = args.first().and_then(Value::base_var)
                {
                    self.note_mutation(host, encl, out);
                }
                for a in args {
                    self.collect_parallel_violations(a, encl, out);
                }
            }
            // CallRef's first field is the var holding the fn-ref — a capture if
            // it is an enclosing parameter.
            Value::CallRef(v, args) => {
                self.note_param(*v, encl, out);
                for a in args {
                    self.collect_parallel_violations(a, encl, out);
                }
            }
            // Any reference to a var: flagged only if it is a captured parameter.
            Value::Var(v) | Value::TupleGet(v, _) | Value::FnRefDnr(v) => {
                self.note_param(*v, encl, out);
            }
            Value::FnRef(_, v, _) => self.note_param(*v, encl, out),
            // Container recursion.
            Value::Insert(ls) | Value::Tuple(ls) | Value::Parallel(ls) => {
                for x in ls {
                    self.collect_parallel_violations(x, encl, out);
                }
            }
            Value::Block(b) | Value::Loop(b) => {
                for x in &b.operators {
                    self.collect_parallel_violations(x, encl, out);
                }
            }
            Value::Return(b) | Value::Drop(b) | Value::Yield(b) | Value::BreakWith(_, b) => {
                self.collect_parallel_violations(b, encl, out);
            }
            Value::If(c, t, e) => {
                self.collect_parallel_violations(c, encl, out);
                self.collect_parallel_violations(t, encl, out);
                self.collect_parallel_violations(e, encl, out);
            }
            Value::Iter(_, a, b, c) => {
                self.collect_parallel_violations(a, encl, out);
                self.collect_parallel_violations(b, encl, out);
                self.collect_parallel_violations(c, encl, out);
            }
            _ => {}
        }
    }

    /// True if `v` was already defined when the parallel block opened — i.e. it
    /// is an enclosing-scope variable, not one declared inside an arm.
    fn is_enclosing(v: u16, encl: &[bool]) -> bool {
        (v as usize) < encl.len() && encl[v as usize]
    }

    /// Flag a write/mutation of an enclosing **user** local.  Compiler temps
    /// (`__work`/`__vdb`) carry the codegen for reads/format-strings and are not
    /// user captures.
    fn note_mutation(&self, v: u16, encl: &[bool], out: &mut Vec<(u16, ParViolation)>) {
        if Self::is_enclosing(v, encl) && self.is_user_var(v) {
            self.assert_enclosing_invariant(v);
            out.push((v, ParViolation::Mutation));
        }
    }

    /// Flag a capture of an enclosing **parameter** (read or write both fault).
    fn note_param(&self, v: u16, encl: &[bool], out: &mut Vec<(u16, ParViolation)>) {
        if Self::is_enclosing(v, encl) && self.is_user_var(v) && self.vars.is_argument(v) {
            self.assert_enclosing_invariant(v);
            out.push((v, ParViolation::Param));
        }
    }

    /// Guard the `parse_parallel` enclosing-snapshot invariant.  A var the
    /// block-entry snapshot marked enclosing (`is_defined` was true at entry) must
    /// still read defined now — `is_defined` is monotonic across the block parse.
    /// If it does not, the snapshot has desynced from the var table and the
    /// enclosing/arm-local split is unsound.  (This catches `is_defined` being
    /// *cleared* mid-parse; it cannot catch it being *set* out of source order —
    /// that failure is undetectable from `is_defined` alone, which is what the
    /// `171-parallel-armlocal-ok.loft` compile guard exists for.)
    fn assert_enclosing_invariant(&self, v: u16) {
        debug_assert!(
            self.vars.is_defined(v),
            "parallel-capture invariant broken: enclosing var '{}' lost is_defined \
             mid-parse — the block-entry snapshot no longer matches the var table \
             (see parse_parallel)",
            self.vars.name(v)
        );
    }

    /// Whether `v` is a user variable that an arm could genuinely capture —
    /// excludes the codegen artefacts that `is_defined` would otherwise misread
    /// as enclosing:
    ///   * compiler temps, named with a leading `_` (`__work`, `__vdb`,
    ///     `_match_subj`, `_elm`, `_vector`) or a `#` (`i#index`, `i#next`);
    ///   * for-loop iteration variables (`was_loop_var`) — the loop desugar
    ///     marks them defined in pass 1, but the loop's own advance of its var
    ///     must not read as an enclosing write.
    fn is_user_var(&self, v: u16) -> bool {
        let n = self.vars.name(v);
        !n.starts_with('_') && !n.contains('#') && !self.vars.was_loop_var(v)
    }
}
