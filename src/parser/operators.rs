// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    Data, Level, OPERATORS, Parser, Position, Type, Value, diagnostic_format, rename, v_block,
    v_if, v_set,
};

// Operator parsing and type dispatch.

impl Parser {
    pub(crate) fn assign_text(
        &mut self,
        code: &mut Value,
        tp: &Type,
        to: &Value,
        op: &str,
        var_nr: u16,
    ) {
        if !self.first_pass && var_nr != u16::MAX && self.vars.is_const_param(var_nr) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot modify {} '{}'; remove 'const' or use a local copy",
                self.vars.const_kind(var_nr),
                self.vars.name(var_nr)
            );
        }
        if let Value::Call(_, parms) = to.unspan().clone() {
            if op == "=" {
                let mut p = parms.clone();
                p.push(code.clone());
                *code = self.cl("OpSetText", &p);
            } else {
                let mut ls = Vec::new();
                ls.push(v_set(var_nr, to.clone()));
                if let Value::Insert(cd) = code {
                    for c in cd {
                        ls.push(c.clone());
                    }
                } else if *tp == Type::Character {
                    ls.push(self.cl("OpAppendCharacter", &[Value::Var(var_nr), code.clone()]));
                } else {
                    ls.push(self.cl("OpAppendText", &[Value::Var(var_nr), code.clone()]));
                }
                let mut p = parms.clone();
                p.push(Value::Var(var_nr));
                ls.push(self.cl("OpSetText", &p));
                *code = Value::Insert(ls);
            }
        } else if let Value::Insert(ls) = code {
            if op == "=" {
                // P217: detect `h = h + expr`: the first Insert entry is a
                // self-append `OpAppendText(var, Var(var))`.  Convert to
                // `+=` semantics by removing the self-append and skipping
                // the up-front clear.  Without this, the clear destroys
                // h's content before the appends read it.
                //
                // The `args[1]` operand is whatever `parse_append_text`
                // received as `code` — a `Var(var)` wrapped by the parser
                // in a `Value::Span` for source-position tracking.  We
                // unspan to compare structural identity, not literal
                // equality (the original `args[1] == Value::Var(var_nr)`
                // check missed every Span-wrapped self-reference and let
                // the clear-then-append path corrupt `h`).
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
                } else {
                    ls.insert(0, v_set(var_nr, Value::Text(String::new())));
                }
            }
        } else if op == "=" && var_nr != u16::MAX {
            // detect self-reference (t = t[N..], t = fn(t), etc.)
            // If the RHS reads from the same variable being assigned, use a
            // work text to avoid the clear-before-read problem.
            if code.reads_var(var_nr) {
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
        } else if *tp == Type::Character {
            *code = self.cl("OpAppendCharacter", &[Value::Var(var_nr), code.clone()]);
        } else {
            *code = self.cl("OpAppendText", &[Value::Var(var_nr), code.clone()]);
        }
    }

    pub(crate) fn create_vector(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        if let (Value::Insert(ls), Type::Vector(tp, _)) = (code, f_type) {
            if !self.first_pass && self.vars.is_const_param(var_nr) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot modify {} '{}'; remove 'const' or use a local copy",
                    self.vars.const_kind(var_nr),
                    self.vars.name(var_nr)
                );
            }
            if op == "=" {
                // Self-concat reassign `v = v + [...]` (sibling of @P390's
                // self-slice `v = v[a..b]`): `parse_append_vector` emitted a
                // leading identity `Set(v, Var(v))` because the concat's
                // accumulator IS the reassignment target.  The `vector_db` splice
                // below allocates a FRESH store for v and repoints v to it (empty)
                // BEFORE that Set / the append read v's OLD contents → v's original
                // elements are lost (only the appended tail survives; `v=[1,2];
                // v=v+[9]` gave `[9]`).  When the body is self-referential, skip
                // the new-store allocation and drop the now-useless identity-set
                // so the parts append IN PLACE to v's existing store — exactly what
                // `v += [...]` does.  The discriminator is the IR shape
                // `Set(var_nr, Var(var_nr))` (absent for `u = v + [9]` →
                // `Set(u, Var(v))` and `v = a + b` → `Set(v, Var(a))`), identical
                // on both parser passes — pass-stable, no @P384 alloc divergence.
                let self_ref = ls.first().is_some_and(|first| {
                    matches!(first.unspan(), Value::Set(s, rhs)
                        if *s == var_nr
                            && matches!(rhs.unspan(), Value::Var(r) if *r == var_nr))
                });
                if self_ref {
                    ls.remove(0);
                } else {
                    // Trailing self-reference `v = a + v` (the accumulator `v`
                    // appears as a NON-first operand): the parts loop emitted
                    // `OpAppendVector(v, <operand mentioning v>)` which would read
                    // v AFTER the `vector_db` clear below empties it → v's OLD
                    // contents are lost (the trailing read sees the freshly-copied
                    // prefix instead).  Sibling of @P390's self-slice.  Materialise
                    // each such operand's OLD value into a fresh temp BEFORE the
                    // clear (deep copy while v still holds its contents), then
                    // rewrite the operand to read the temp.  The first-operand
                    // deep-copy (`OpAppendVector(v, Var(a))`, a != v) never matches;
                    // the self-concat first-operand case is the `self_ref` branch
                    // above.  Pass-2 work (vector_db / create_unique are pass-2),
                    // matching the @P390 / @P287 temp precedent.
                    let mut prefix: Vec<Value> = Vec::new();
                    if !self.first_pass {
                        let append_nr = self.data.def_nr("OpAppendVector");
                        for stmt in ls.iter_mut() {
                            if let Value::Call(d, args) = stmt
                                && *d == append_nr
                                && args.len() == 3
                                && matches!(args[0].unspan(), Value::Var(t) if *t == var_nr)
                                && args[1].reads_var(var_nr)
                            {
                                let rec = args[2].clone();
                                let tmp = self.create_unique("__trail_tmp", f_type);
                                self.vars.defined(tmp);
                                let mut mat = self.vector_db(tp, tmp);
                                mat.push(self.cl(
                                    "OpAppendVector",
                                    &[Value::Var(tmp), Value::Var(var_nr), rec],
                                ));
                                prefix.extend(mat);
                                args[1] = Value::Var(tmp);
                            }
                        }
                    }
                    // plan-57 cluster II — a literal / comprehension / struct-vector
                    // init already allocated v's store in the RHS body (the literal's
                    // own `vector_db` emitted a head `Set(v, OpGetField(__vdb))`).  The
                    // `=` `vector_db` below would allocate a SECOND, immediately-
                    // orphaned store for v (the 2× store high-watermark).  Skip it when
                    // the body already allocated.  Concat (`OpAppendVector` / `Set(v,
                    // <temp>)`) and reassignment (`a=[1,2,3]; a=[4,5]` — no head alloc)
                    // have no such repoint, so they KEEP the `vector_db`: concat needs
                    // it for v's store, reassignment relies on the fresh store as its
                    // clear (cluster III — unchanged by this fix).  Pure watermark
                    // optimisation; results + lifetimes are identical.
                    let get_field_nr = self.data.def_nr("OpGetField");
                    let body_allocates = ls.iter().any(|stmt| {
                        matches!(stmt.unspan(),
                            Value::Set(s, rhs) if *s == var_nr
                                && matches!(rhs.unspan(), Value::Call(d, _) if *d == get_field_nr))
                    });
                    // The materialise prefix reads v while it is still intact, so
                    // it MUST precede the `vector_db` clear: insert prefix ++
                    // vector_db ops together, ahead of the body (prefix first).
                    let mut front = prefix;
                    if !body_allocates {
                        front.extend(self.vector_db(tp, var_nr));
                    }
                    for (i, p) in front.into_iter().enumerate() {
                        ls.insert(i, p);
                    }
                    if ls.is_empty()
                        && !self.first_pass
                        && var_nr != u16::MAX
                        && matches!(f_type, Type::Vector(_, _))
                    {
                        ls.push(self.cl("OpClearVector", &[Value::Var(var_nr)]));
                    }
                }
            }
            true
        } else {
            false
        }
    }

    /// P193: eager init for `local: keyed_collection<T[key]> = []`.
    ///
    /// Without this, the empty `[]` literal parses to `Value::Insert(empty)`
    /// which doesn't match either the `Set(v, Value::Null)` arm of
    /// codegen (which would call `gen_set_first_keyed_null`) nor the
    /// vector-style `create_vector` rewrite.  The variable then has no
    /// init bytecode emitted: subsequent reads hit u16::MAX slot, and
    /// when the FIRST WRITE is inside a loop body the lazy init runs
    /// once per iteration — every iteration zeros the collection's
    /// root pointer.  Symptom: `for i in 0..N { ix += ... }` over a
    /// local-var keyed collection leaves `len(ix) == 1`.
    ///
    /// Fix: rewrite `Set(v, Insert(empty))` to `Set(v, Null)` for
    /// keyed-collection types and op == "=", so the standard codegen
    /// path emits `gen_set_first_keyed_null` at the declaration site
    /// (outside any loop body).
    ///
    /// Returns true when the rewrite happened so the caller short-
    /// circuits the rest of the assign pipeline.
    pub(crate) fn create_keyed(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        if op != "="
            || var_nr == u16::MAX
            || !matches!(
                f_type,
                Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Spacial(_, _, _)
            )
        {
            return false;
        }
        let is_empty_insert = matches!(code, Value::Insert(ls) if ls.is_empty());
        if !is_empty_insert {
            return false;
        }
        if !self.first_pass && self.vars.is_const_param(var_nr) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot modify {} '{}'; remove 'const' or use a local copy",
                self.vars.const_kind(var_nr),
                self.vars.name(var_nr)
            );
        }
        // Codegen's Set(v, Null) arm matches keyed types and dispatches
        // to gen_set_first_keyed_null — emits OpInitRef + OpDatabase
        // for the slot, anchored at the declaration's statement
        // position (NOT inside any enclosing loop body).
        *code = Value::Null;
        true
    }

    /// Check whether `val` is a call to a user-defined function that returns a struct
    /// via a temporary store.  Used by `copy_ref` and the vector-append
    /// emit path (`vectors.rs`) to decide whether to free the source
    /// store after the deep copy.  The free bit's behaviour differs
    /// under WASM but the query is the same on every target — call
    /// sites in expressions.rs / objects.rs / vectors.rs / collections.rs
    /// are not feature-gated, so this helper must not be either.
    pub(crate) fn is_struct_returning_call(&self, val: &Value) -> bool {
        if self.first_pass {
            return false;
        }
        match val.unspan() {
            Value::Call(fn_nr, _) => {
                let def = &self.data.def(*fn_nr);
                // User function with code (not a built-in op)
                def.name().starts_with("n_") && *def.code() != Value::Null
            }
            // Struct constructor blocks allocate a store too — when assigned
            // to a field, the source store is a temporary that should be freed.
            Value::Block(bl) => bl.name == "Object",
            _ => false,
        }
    }

    pub(crate) fn copy_ref(&mut self, to: &Value, code: &Value, f_type: &Type) -> Value {
        let d_nr = self.data.type_def_nr(f_type);
        let tp = self.data.def(d_nr).known_type();
        // When the source is a struct-returning function CALL, set the high
        // bit (0x8000) on the type parameter to signal copy_record to free the
        // callee's temporary store after the deep copy.  Without this, the
        // __ref_N work-ref store the callee allocated to build its return value
        // leaks on every call in a loop.
        //
        // @P313: do NOT set 0x8000 for an inline struct-literal `Block "Object"`
        // source.  Unlike a call's unowned return temporary, the literal's
        // work-ref ALREADY carries its own scope `OpFreeRef`, so the free-source
        // bit double-frees it: the store is released while still owned, then
        // recycled by the next iteration's OpDatabase, corrupting the
        // nested-vector backings of every element written before it (silent
        // data loss on `vec[i] = Struct{…}`; use-after-free SIGSEGV under
        // churn).  Same shape as the @P311 OpSetKeyed fix, here on the
        // whole-value element-set path.
        #[cfg(not(feature = "wasm"))]
        let tp_val =
            if matches!(code.unspan(), Value::Call(_, _)) && self.is_struct_returning_call(code) {
                i32::from(tp) | 0x8000
            } else {
                i32::from(tp)
            };
        #[cfg(feature = "wasm")]
        let tp_val = i32::from(tp);
        self.cl(
            "OpCopyRecord",
            &[code.clone(), to.clone(), Value::Int(tp_val)],
        )
    }

    /** Mutate current code when it reads a value into writing it. This is needed for assignments.
     */
    pub(crate) fn compute_op_code(
        &mut self,
        op: &str,
        to: &Value,
        val: &Value,
        f_type: &Type,
    ) -> Value {
        if op == "=" {
            val.clone()
        } else if op == ">" {
            self.op("Lt", val.clone(), to.clone(), f_type.clone())
        } else if op == ">=" {
            self.op("Le", val.clone(), to.clone(), f_type.clone())
        } else {
            self.op(rename(op), to.clone(), val.clone(), f_type.clone())
        }
    }

    /// Dispatch an `OpGetX` getter name to the corresponding `OpSetX` setter call.
    pub(crate) fn call_to_set_op(
        &mut self,
        name: &str,
        args: &[Value],
        code: Value,
        _op: &str,
    ) -> Value {
        match name {
            "OpGetInt" => {
                // f#next = pos: seek the file AND update the stored field.
                if args[1] == Value::Int(16)
                    && let Value::Var(v_nr) = &args[0]
                    && self.is_file_var(*v_nr)
                {
                    let seek = self.cl("OpSeekFile", &[args[0].clone(), code.clone()]);
                    let set = self.cl(
                        "OpSetInt",
                        &[args[0].clone(), args[1].clone(), code.clone()],
                    );
                    return Value::Insert(vec![seek, set]);
                }
                self.cl("OpSetInt", &[args[0].clone(), args[1].clone(), code])
            }
            "OpGetByte" => self.cl(
                "OpSetByte",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            // #334: the nullable byte pair (sentinel-translating twin).
            "OpGetByteNullable" => self.cl(
                "OpSetByteNullable",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            "OpGetEnum" => self.cl("OpSetEnum", &[args[0].clone(), args[1].clone(), code]),
            // @PLN17: byte-stored boolean write, storing the u8 form 0/1/255 directly.
            "OpGetBoolean" => self.cl("OpSetBoolean", &[args[0].clone(), args[1].clone(), code]),
            "OpGetShort" => self.cl(
                "OpSetShort",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            // not-null 2-byte field write — reuses the raw `(val - min)` store
            // (the read twin `OpGetShortFull` decodes without a sentinel).
            "OpGetShortFull" => self.cl(
                "OpSetShortRaw",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            "OpGetInt4" => self.cl("OpSetInt4", &[args[0].clone(), args[1].clone(), code]),
            "OpGetFloat" => self.cl("OpSetFloat", &[args[0].clone(), args[1].clone(), code]),
            "OpGetSingle" => self.cl("OpSetSingle", &[args[0].clone(), args[1].clone(), code]),
            // Plan-22 phase 02d-iv — character / text cell writes
            // route through OpSetCharacter / OpSetText (3-arg
            // shape: ref, fld, val).  Mirrors the existing
            // OpGetInt → OpSetInt mapping above.  Required by
            // 02d-iii's auto-deref'd LHS pattern for boxed-
            // scalar locals + closure-body writes.
            "OpGetCharacter" => {
                self.cl("OpSetCharacter", &[args[0].clone(), args[1].clone(), code])
            }
            "OpGetText" => self.cl("OpSetText", &[args[0].clone(), args[1].clone(), code]),
            "OpGetField" => code,
            "n_get_store_lock" => {
                // d#lock = val — validation enforced in parse_assign before this call.
                self.cl("n_set_store_lock", &[args[0].clone(), code])
            }
            "OpSizeFile" => {
                // f#size = n: delegate to set_file_size which validates format and sign.
                let fn_nr = self.data.def_nr("t_4File_set_file_size");
                if fn_nr == u32::MAX {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "set_file_size is not defined");
                    }
                    Value::Null
                } else {
                    Value::Call(fn_nr, vec![args[0].clone(), code])
                }
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot assign to attribute on type '{name}'"
                    );
                }
                Value::Null
            }
        }
    }

    pub(crate) fn parse_operators(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
    ) -> Type {
        let mut ls = Vec::new();
        if precedence >= OPERATORS.len() {
            let t = self.parse_part(var_tp, code, parent_tp);
            return t;
        }
        let orig_var = if let Value::Var(nr) = code {
            *nr
        } else {
            u16::MAX
        };
        // Start of the left operand — `known_var_or_type` below must point an
        // "Unknown variable" caret here, not at the cursor that has drifted to
        // the operator / statement terminator while the operand was parsed.
        let operand_pos = self.lexer.peek_pos().clone();
        let mut current_type = self.parse_operators(var_tp, code, parent_tp, precedence + 1);
        loop {
            // a void left operand cannot have any binary operator
            // applied to it. Returning early prevents the pratt loop from
            // consuming a token that's actually the start of the *next*
            // statement — e.g. `if cond { return 0; }\n -1` where `-1` is
            // the function's tail expression, not `void - 1`.
            if matches!(current_type, Type::Void) {
                return current_type;
            }
            // Plan-07 phase 1, step 1.B.1 — capture the operator's source
            // position *before* `has_token` consumes it.  `op_pos` is then
            // threaded into `handle_operator` so the resulting Value::Span
            // points at the operator token (e.g. the `/`), not at whatever
            // the lexer drifted to while parsing the RHS.
            let op_pos = self.lexer.pos().clone();
            let mut operator = "";
            for op in OPERATORS[precedence] {
                if self.lexer.has_token(op) {
                    operator = op;
                    break;
                }
            }
            if operator.is_empty() {
                // `expr is VariantName` — variant check at comparison precedence.
                // Returns boolean: true if the enum value matches the named variant.
                if precedence == 3 && self.lexer.has_token("is") {
                    if let Some(variant_name) = self.lexer.has_identifier() {
                        current_type = self.parse_is_variant(code, &current_type, &variant_name);
                    } else if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "expect variant name after 'is'");
                    }
                    continue;
                }
                if !ls.is_empty() {
                    // Unwrap RefVar(Text) for text_return work buffer loop variables
                    let effective_type = if let Type::RefVar(inner) = &current_type {
                        if matches!(**inner, Type::Text(_)) {
                            *inner.clone()
                        } else {
                            current_type.clone()
                        }
                    } else {
                        current_type.clone()
                    };
                    if matches!(effective_type, Type::Text(_) | Type::Character) {
                        if current_type == Type::Character {
                            // a Character variable cannot serve as an OpAppendText
                            // destination.  Prepend it to the parts list and use an empty
                            // text literal as the first operand so parse_append_text
                            // creates a fresh work text.
                            ls.insert(0, (code.clone(), Type::Character));
                            *code = Value::Text(String::new());
                            return self.parse_append_text(
                                code,
                                &Type::Text(crate::data::Deps::none()),
                                &ls,
                                u16::MAX,
                            );
                        }
                        // P223: `orig_var` was captured BEFORE the recursive
                        // `parse_operators` filled `code`.  When the LHS of an
                        // assignment is `s` (so `code` started as `Var(s)`) and
                        // the RHS is a literal-first concat like `"hello " + s`,
                        // `code` ends up as `Text("hello ")` after recursion —
                        // but `orig_var` still points at `s`.  Passing this
                        // stale `orig_var` makes `parse_append_text` use `s` as
                        // the accumulator and emit `OpAppendText(s, "hello ")`
                        // as the first op, which (combined with the
                        // `assign_text` self-reference clear) destroys `s`'s
                        // original content before the second append reads it.
                        // Fall back to a fresh work-text whenever `code`
                        // (unspanned) no longer is `Var(orig_var)`.
                        let effective_orig = if orig_var != u16::MAX
                            && !matches!(code.unspan(), Value::Var(v) if *v == orig_var)
                        {
                            u16::MAX
                        } else {
                            orig_var
                        };
                        return self.parse_append_text(code, &current_type, &ls, effective_orig);
                    } else if matches!(current_type, Type::Vector(_, _)) {
                        return self.parse_append_vector(code, &current_type, &ls, orig_var);
                    } else if let Type::RefVar(inner) = &current_type
                        && matches!(**inner, Type::Vector(_, _))
                    {
                        return self.parse_append_vector(code, inner, &ls, orig_var);
                    }
                }
                return current_type;
            }
            self.known_var_or_type(code, &operand_pos);
            // Detect '++': not a valid operator in loft. Consume the extra '+',
            // emit an error, and continue as if a single '+' was written.
            if operator == "+" && self.lexer.has_token("+") && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'++' is not a valid operator — use '+' for concatenation or addition"
                );
            }
            // Unwrap RefVar(Text) for the + text concatenation check
            let eff_type_for_plus = if let Type::RefVar(inner) = &current_type {
                if matches!(**inner, Type::Text(_)) {
                    *inner.clone()
                } else {
                    current_type.clone()
                }
            } else {
                current_type.clone()
            };
            if operator == "+"
                && matches!(
                    eff_type_for_plus,
                    Type::Text(_) | Type::Character | Type::Vector(_, _)
                )
            {
                let mut second_code = Value::Null;
                let tp = self.parse_operators(var_tp, &mut second_code, parent_tp, precedence + 1);
                ls.push((second_code, tp));
            } else if let Some(value) = self.handle_operator(
                var_tp,
                code,
                parent_tp,
                precedence,
                &mut current_type,
                operator,
                &op_pos,
            ) {
                return value;
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_part(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
    ) -> Type {
        let mut t = self.parse_single(var_tp, code, parent_tp);
        // --show-types --trace: log the type after the initial
        // `parse_single` (variable, literal, parenthesised expr).
        self.record_type_trace(&t);
        while self.lexer.peek_token(".")
            || self.lexer.peek_token("[")
            || (self.lexer.peek_token("(") && matches!(t, Type::Function(_, _, _)))
        {
            // Plan-07 phase 1, steps 1.11 + 1.12 — capture the chaining
            // token's source position before `has_token` consumes it.
            // Wrapped when the iteration consumes a fault-prone access
            // (`.` field/method or `[` index — both can deref null or
            // out-of-bounds at runtime).  The `(` chained-call branch
            // is wrapped under step 1.13.
            let chain_pos = self.lexer.pos().clone();
            let mut wrap_chain = false;
            if !self.first_pass && t.is_unknown() && matches!(code, Value::Var(_)) {
                diagnostic!(self.lexer, Level::Error, "Unknown variable");
            }
            if self.lexer.has_token(".") {
                wrap_chain = true;
                *parent_tp = t.clone();
                // T1.2: tuple element access — t.0, t.1, etc.
                if let Type::Tuple(ref elems) = t {
                    let elems = elems.clone();
                    if let Some(idx) = self.lexer.has_integer() {
                        let idx = idx as usize;
                        if idx >= elems.len() {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Tuple index {idx} out of range — tuple has {} elements",
                                elems.len()
                            );
                            t = Type::Unknown(0);
                        } else {
                            // P197: propagate parent tuple's deps into the
                            // element type so a returned text/reference
                            // carries the host's lifetime through.  Without
                            // this, `fn f() -> text { ...; a.v.0 }` returns
                            // a `Str` whose ptr points into a freed host.
                            let parent_deps = t.depend();
                            t = elems[idx].clone();
                            for on in parent_deps {
                                t = t.depending(on);
                            }
                            // T1.4: emit TupleGet IR for codegen.
                            // Plan-07 phase 1: unspan() so wraps on `.`
                            // (step 1.12) don't hide the underlying Var
                            // or Tuple shape of the parent expression.
                            let unspanned = code.unspan().clone();
                            if let Value::Var(var_nr) = unspanned {
                                *code = Value::TupleGet(var_nr, idx as u16);
                            } else if let Value::Tuple(elems_v) = unspanned {
                                // P197: code is already a literal tuple of
                                // per-element reads (e.g. produced by
                                // `get_val::Type::Tuple` for a tuple struct
                                // field).  Materialising the whole tuple
                                // into a `(String, String)` work var causes
                                // a native-codegen borrow lifetime error
                                // because the owned-`String` tuple temp
                                // dies before the returned `&str` is
                                // consumed.  Short-circuit: take the
                                // already-built element read directly.
                                if idx < elems_v.len() {
                                    *code = elems_v[idx].clone();
                                } else {
                                    *code = Value::Null;
                                }
                            } else {
                                // Temporary tuple — store in work var first.
                                let tmp_tp = Type::Tuple(elems.clone());
                                let w = self.vars.work_refs(&tmp_tp, &mut self.lexer);
                                if !self.first_pass {
                                    self.change_var_type(w, &tmp_tp);
                                }
                                let orig = code.clone();
                                *code = Value::TupleGet(w, idx as u16);
                                // Prepend Set(w, orig) in a block.
                                *code = crate::data::v_block(
                                    vec![
                                        crate::data::v_set(w, orig),
                                        Value::TupleGet(w, idx as u16),
                                    ],
                                    t.clone(),
                                    "tuple_tmp",
                                );
                            }
                        }
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Tuple element access requires a numeric index (e.g. .0, .1)"
                        );
                    }
                } else if let Type::RefVar(ref inner) = t
                    && let Type::Tuple(ref elems) = **inner
                {
                    // T1.5: element access through a reference-tuple parameter — pair.0, pair.1.
                    let elems = elems.clone();
                    self.parse_ref_tuple_elem(&mut t, code, &elems);
                } else if let Type::Reference(d_nr, _) = t
                    && self.data.def(d_nr).name().starts_with("__tuple<")
                    && matches!(self.lexer.peek().has, crate::lexer::LexItem::Integer(_, _))
                {
                    // P189b: vector-of-tuple loop var / index result —
                    // the loop variable is typed as `Reference(__tuple<…>)`
                    // pointing at inline tuple bytes inside the vector
                    // record.  `.0` / `.1` route through `get_val` so
                    // both interpreter (`OpGetInt(off)` / `OpGetText(off)`)
                    // and native codegen (per-type `stores.store(&db)`
                    // pattern) read at the right field offset.  Element
                    // types come from the synthetic struct's attributes.
                    let elems: Vec<Type> = self
                        .data
                        .def(d_nr)
                        .attributes
                        .iter()
                        .map(|a| a.typedef.clone())
                        .collect();
                    if let Some(idx) = self.lexer.has_integer() {
                        let idx = idx as usize;
                        if idx >= elems.len() {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Tuple index {idx} out of range — tuple has {} elements",
                                elems.len()
                            );
                            t = Type::Unknown(0);
                        } else {
                            // Stored-tuple field offset goes through the
                            // synthetic struct's post-finish layout — same
                            // offsets `OpGetInt` uses for an ordinary
                            // struct field.
                            let elem_offset = if let Some(v) =
                                crate::data::stored_tuple_offsets_for_def(
                                    &self.data,
                                    &self.database,
                                    d_nr,
                                    elems.len(),
                                ) {
                                u32::from(v[idx])
                            } else {
                                crate::data::element_offsets(&elems)[idx] as u32
                            };
                            let elem_tp = elems[idx].clone();
                            *code =
                                self.get_val(&elem_tp, false, elem_offset, code.clone(), u32::MAX);
                            t = elem_tp;
                        }
                    }
                } else {
                    t = self.field(code, t);
                }
                // If the method returned an owned ref and more chaining follows, capture
                // it in a work-ref so scopes.rs emits OpFreeRef at end-of-scope.
                // Without this, the store allocated by the callee leaks and the LIFO
                // invariant in database::free() is violated.
                if !self.first_pass
                    && !matches!(code, Value::Var(_))
                    && (self.lexer.peek_token(".") || self.lexer.peek_token("["))
                    && let Type::Reference(d_nr, dep) = &t
                    && dep.is_empty()
                {
                    let d_nr = *d_nr;
                    let w = self.vars.work_refs(&t.clone(), &mut self.lexer);
                    // Mark as inline-ref temp so parse_code inserts its
                    // null-init after the first user statement, ensuring
                    // it appears after user-scope vars in var_order and is
                    // therefore freed before them (LIFO).
                    self.vars.mark_inline_ref(w);
                    let orig = code.clone();
                    *code = v_block(
                        vec![v_set(w, orig), Value::Var(w)],
                        Type::Reference(d_nr, crate::data::Deps::frame1(w)),
                        "inline ref",
                    );
                    t = Type::Reference(d_nr, crate::data::Deps::frame1(w));
                }
            } else if self.lexer.has_token("[") {
                wrap_chain = true;
                // #246: record the indexed container's type as the parent,
                // mirroring the `.` field branch above.  Without this, a
                // trailing index (`vv[0]`, or `h.vv[0]` after a `.field`) leaves
                // `parent_tp` either null or stale-from-the-`.`, so a compound
                // append to a vector element can't tell it is appending to a
                // VECTOR (not a struct field).
                *parent_tp = t.clone();
                t = self.parse_index(code, &t);
                self.lexer.token("]");
            } else if self.lexer.has_token("(") {
                // chained call on a Type::Function expression — expr(args).
                if let Type::Function(param_types, ret_type, _) = t.clone() {
                    let fn_type = Type::Function(
                        param_types.clone(),
                        ret_type.clone(),
                        crate::data::Deps::none(),
                    );
                    // Allocate temp variable on BOTH passes (consistent unique counter).
                    let fn_work = self.create_unique("__fn_ref_tmp", &fn_type);
                    self.vars.defined(fn_work);
                    // The fn_work temp is a borrowed copy of an existing
                    // fn-ref: its closure DbRef aliases the source's
                    // closure store, so emitting OpFreeRef on it would
                    // double-free.  Mark `skip_free` so scope-exit cleanup
                    // leaves the closure alone.  Also blocks the
                    // insert_free Return-wrap path that would otherwise
                    // wrap the trailing OpFreeRef in `return`, returning
                    // `()` from a value-returning block (P249-mirror).
                    self.vars.set_skip_free(fn_work);
                    // P227: one work-buffer per text-returning fn-ref
                    // call (the return-value buffer the lambda fills via
                    // its hidden RefVar(Text) attr).  Previously
                    // `deps.len()` — but fn-ref types carry `deps = []`,
                    // so the count was zero, causing SIGSEGV.
                    let work_vars: Vec<u16> = if matches!(ret_type.as_ref(), Type::Text(_)) {
                        vec![self.vars.work_text(&mut self.lexer)]
                    } else {
                        vec![]
                    };
                    // Parse arguments (both passes).
                    let mut list: Vec<Value> = Vec::new();
                    let mut types: Vec<Type> = Vec::new();
                    let mut first = true;
                    while !self.lexer.peek_token(")") && !self.lexer.peek_token("") {
                        if !first {
                            self.lexer.token(",");
                        }
                        first = false;
                        let mut arg_val = Value::Null;
                        let arg_tp = self.expression(&mut arg_val);
                        list.push(arg_val);
                        types.push(arg_tp);
                    }
                    self.lexer.token(")");
                    if !self.first_pass {
                        let mut converted = list;
                        for (i, expected) in param_types.iter().enumerate() {
                            if i < converted.len() {
                                self.convert(&mut converted[i], &types[i], expected);
                            }
                        }
                        let ref_def = self.data.def_nr("reference");
                        for &wv in &work_vars {
                            converted.push(v_block(
                                vec![
                                    v_set(wv, Value::Text(String::new())),
                                    self.cl("OpCreateStack", &[Value::Var(wv)]),
                                ],
                                Type::Reference(ref_def, crate::data::Deps::frame1(wv)),
                                "cref_work_buf",
                            ));
                        }
                        let orig = std::mem::replace(code, Value::Null);
                        *code = v_block(
                            vec![v_set(fn_work, orig), Value::CallRef(fn_work, converted)],
                            *ret_type.clone(),
                            "fn_call_tmp",
                        );
                    }
                    t = *ret_type;
                }
            }
            // Plan-07 phase 1, steps 1.11 + 1.12 — wrap the chained
            // expression in a Span at the access-token position so
            // runtime null-deref / out-of-bounds errors can be
            // reported with the source location of the offending `.`
            // or `[` token.  Skip when the result is an `Iter` (range
            // subscript like `v[0..5]` produces an iterator that
            // parse_for / iterator() must pattern-match without
            // a Span wrapper) — the access token doesn't fault for
            // a range-subscript shape, only the eventual element
            // reads do, and those go through the normal `[idx]`
            // path inside the rewritten loop.
            if wrap_chain && !self.first_pass && !matches!(code, Value::Iter(_, _, _, _)) {
                let inner = std::mem::replace(code, Value::Null);
                *code = Value::with_span(chain_pos, inner);
            }
            // --show-types --trace: log the resulting type after
            // each chaining step (`.field`, `.tuple_idx`, `[idx]`,
            // `(args)`).  Combined with the post-`parse_single`
            // log at the top, this produces a per-step "tape" of
            // how the type evolves through a chained expression
            // like `a.v.0` — the shape that hid the P197 dep loss.
            self.record_type_trace(&t);
        }
        t
    }

    /// T1.5: parse a `.N` element index on a `&(T1, T2, ...)` reference-tuple.
    /// Updates `t` to the element type and rewrites `code` to `TupleGet(var, idx)`
    /// when `code` is a plain variable reference.
    fn parse_ref_tuple_elem(&mut self, t: &mut Type, code: &mut Value, elems: &[Type]) {
        if let Some(idx) = self.lexer.has_integer() {
            let idx = idx as usize;
            if idx >= elems.len() {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Tuple index {idx} out of range — tuple has {} elements",
                    elems.len()
                );
                *t = Type::Unknown(0);
            } else {
                *t = elems[idx].clone();
                if let Value::Var(var_nr) = code {
                    *code = Value::TupleGet(*var_nr, idx as u16);
                }
            }
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Tuple element access requires a numeric index (e.g. .0, .1)"
            );
        }
    }

    /// C54.G-hybrid helper: if `code` is a direct call to an arithmetic
    /// opcode (`OpAddInt` / `OpMulInt` / etc. or their `*Long` siblings),
    /// swap its def number to the Nullable variant (`OpAddIntNullable` /
    /// `OpMulIntNullable` / …).  Only the outermost call is rewritten;
    /// nested sub-expressions keep trap semantics.  No-op if the Nullable
    /// variant isn't registered (defensive — should always be, via
    /// `default/01_code.loft`).
    fn rewrite_outer_arith_to_nullable(code: &mut Value, data: &crate::data::Data) {
        // Helper: try to swap the call's def_nr to its Nullable peer.
        // Returns `true` if it found and applied a swap.
        fn try_swap(def_nr: &mut u32, data: &crate::data::Data) -> bool {
            let name = data.def(*def_nr).original_name();
            let nullable_name = match name.as_str() {
                "AddInt" => "OpAddIntNullable",
                "MinInt" => "OpMinIntNullable",
                "MulInt" => "OpMulIntNullable",
                "DivInt" => "OpDivIntNullable",
                "RemInt" => "OpRemIntNullable",
                // Plan-07 phase 4d — vector / text indexing followed by `??`.
                // Mirrors the C54.G-hybrid pattern: when codegen detects
                // `??` immediately after a fault-prone op, swap to the
                // Nullable peer so the op returns its sentinel silently
                // (no log, no halt) and `??` discharges it.
                "GetVector" => "OpGetVectorNullable",
                "VectorRef" => "OpVectorRefNullable",
                "TextCharacter" => "OpTextCharacterNullable",
                // Plan-07 phase 4f.5 — float / single div / mod by zero
                // peers.  Same defense-dispatch contract as integer.
                "DivFloat" => "OpDivFloatNullable",
                "RemFloat" => "OpRemFloatNullable",
                "DivSingle" => "OpDivSingleNullable",
                "RemSingle" => "OpRemSingleNullable",
                _ => return false,
            };
            let new_nr = data.def_nr(nullable_name);
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
        // First try the outer call.  Direct hits cover OpDivInt /
        // OpRemInt / OpVectorRef / OpTextCharacter / arithmetic.
        if try_swap(def_nr, data) {
            return;
        }
        // Wrapped case: typed-vector indexing emits a type-specific
        // element getter wrapping the inner `OpGetVector` —
        // `OpGetInt(OpGetVector(v, size, idx), 0)` for `vector<integer>`
        // and analogues for every other base element type.  The outer
        // getter is null-tolerant (returns the type's null sentinel on a
        // `rec == 0` DbRef per the Store-accessor guards); the inner
        // `OpGetVector` is the one that raises.  Recurse into the first arg
        // to swap the inner op.  @P356: this list previously covered only
        // the integer wrappers, so `tv[i] ?? fb` over a non-integer vector
        // (text / float / single / enum / char / nested collection / ref)
        // kept the RAISING `OpGetVector` and still halted on OOB.
        let outer_name = data.def(*def_nr).original_name();
        if matches!(
            outer_name.as_str(),
            "GetInt"
                | "GetInt4"
                | "GetByte"
                | "GetShortRaw"
                | "GetShort"
                | "GetText"
                | "GetSingle"
                | "GetFloat"
                | "GetEnum"
                // @PLN17: byte-stored boolean field/element read (single-level
                // wrap of OpGetVector), so the inner OpGetVector swaps to its
                // mutable form for `v[i] = bool` — same as GetEnum.
                | "GetBoolean"
                | "GetCharacter"
                | "GetField"
                | "GetDbRef"
        ) && let Some(first_arg) = args.first_mut()
            && let Value::Call(inner_nr, _) = first_arg.unspan_mut()
        {
            try_swap(inner_nr, data);
        } else if outer_name == "EqInt"
            // `vector<boolean>` is `OpEqInt(OpGetByte(OpGetVector(...)), 1)`
            // — a TWO-level wrap, so descend through the `OpGetByte` to reach
            // the inner `OpGetVector`.
            && let Some(first_arg) = args.first_mut()
            && let Value::Call(byte_nr, byte_args) = first_arg.unspan_mut()
            && data.def(*byte_nr).original_name() == "GetByte"
            && let Some(gv) = byte_args.first_mut()
            && let Value::Call(inner_nr, _) = gv.unspan_mut()
        {
            try_swap(inner_nr, data);
        }
    }

    /// Plan-07 phase 4e.1 — recursive variant of
    /// [`Self::rewrite_outer_arith_to_nullable`].  Walks the whole
    /// `Value` tree and swaps every fault-prone call to its Nullable
    /// peer.  Used in format-string contexts (`"{expr}"` interpolation)
    /// where the interpolated expression may contain arbitrarily nested
    /// fault-prone ops (`"{a + v[i] / b}"`).  Per C66 +
    /// `DESIGN_DECISIONS.md` 2026-05-11: format strings are the user's
    /// observability surface and must NEVER halt, log, or warn — so
    /// every interpolated fault site routes through its silent peer.
    ///
    /// Phase 4e.3 — also returns the OUTERMOST fault kind id (as
    /// recognised by the swap table) so the caller (the format-
    /// string emitter in `parser/objects.rs::parse_format`) can
    /// prepend an `OpTagFault(kind_id)` sibling statement.  When
    /// the next format-conversion op (`OpFormatInt` /
    /// `OpAppendCharacter`) sees the type's null sentinel AND the
    /// tag is set, it renders `null(<reason>)` instead of bare
    /// `null`.  Returns `None` for non-fault outer calls (the
    /// expression may still contain inner faults that get swapped,
    /// but only the outermost one tags — inner faults have no
    /// renderer to feed the tag to).
    pub(crate) fn rewrite_subtree_to_nullable_kind(
        code: &mut Value,
        data: &crate::data::Data,
    ) -> Option<u8> {
        // Determine the kind BEFORE the swap.  For integer-vector
        // indexing the IR shape is `OpGetInt(OpGetVector(v, 4, i), 0)`
        // — the outer is `GetInt`, the fault-prone op is the inner
        // `GetVector`.  Mirror the recurse-one-level case from
        // `rewrite_outer_arith_to_nullable` so the kind id matches
        // the inner op when wrapped.
        fn classify(name: &str) -> Option<u8> {
            match name {
                "DivInt" | "DivFloat" | "DivSingle" => Some(1),
                "RemInt" | "RemFloat" | "RemSingle" => Some(2),
                "GetVector" | "VectorRef" | "TextCharacter" => Some(3),
                _ => None,
            }
        }
        let outer_kind = match code.unspan() {
            Value::Call(def_nr, args) => {
                let outer_name = data.def(*def_nr).original_name();
                let direct = classify(&outer_name);
                if direct.is_some() {
                    direct
                } else if matches!(
                    outer_name.as_str(),
                    "GetInt" | "GetInt4" | "GetByte" | "GetShortRaw"
                ) && let Some(first) = args.first()
                    && let Value::Call(inner_nr, _) = first.unspan()
                {
                    classify(&data.def(*inner_nr).original_name())
                } else {
                    None
                }
            }
            _ => None,
        };
        Self::rewrite_subtree_to_nullable(code, data);
        outer_kind
    }

    pub(crate) fn rewrite_subtree_to_nullable(code: &mut Value, data: &crate::data::Data) {
        // Helper mirrors the swap table in
        // `rewrite_outer_arith_to_nullable`; kept inline (no shared
        // helper) so the dispatch table stays grep-discoverable from
        // both swap sites.
        fn try_swap(def_nr: &mut u32, data: &crate::data::Data) -> bool {
            let name = data.def(*def_nr).original_name();
            let nullable_name = match name.as_str() {
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
            let new_nr = data.def_nr(nullable_name);
            if new_nr == u32::MAX {
                false
            } else {
                *def_nr = new_nr;
                true
            }
        }
        match code.unspan_mut() {
            Value::Call(def_nr, args) => {
                try_swap(def_nr, data);
                for arg in args {
                    Self::rewrite_subtree_to_nullable(arg, data);
                }
                // Phase 4e.3 (slice 2 — deferred): the design adds a
                // sibling `OpTagFault(kind)` immediately before each
                // swapped Nullable peer in format-string scope so the
                // format-conversion op renders `null(<reason>)`
                // instead of bare `null`.  The runtime infrastructure
                // (`Stores::set_format_fault` /
                // `Stores::take_format_fault` / `format_fault_tag`
                // field / `OpTagFault` opcode) is in place; the
                // Block-wrapping insertion proved fragile when run
                // through the format-string emitter (the Block's
                // result type isn't filled in time for `append_data`
                // to pick the right `OpAppend*` op, producing wrong-
                // type bytecode and SIGSEGV).  Wiring slice 2 needs
                // a different emit shape — likely new dedicated
                // `Op*Fmt` peers (one per fault kind) that fold the
                // tag into the Nullable peer's body instead of
                // sequencing.  Tracked in plan-07 phase 4e.3 row.
            }
            Value::CallRef(_, args) => {
                for arg in args {
                    Self::rewrite_subtree_to_nullable(arg, data);
                }
            }
            Value::Block(b) => {
                for child in &mut b.operators {
                    Self::rewrite_subtree_to_nullable(child, data);
                }
            }
            Value::Loop(b) => {
                for child in &mut b.operators {
                    Self::rewrite_subtree_to_nullable(child, data);
                }
            }
            Value::If(cond, then_b, else_b) => {
                Self::rewrite_subtree_to_nullable(cond, data);
                Self::rewrite_subtree_to_nullable(then_b, data);
                Self::rewrite_subtree_to_nullable(else_b, data);
            }
            Value::Set(_, src)
            | Value::Return(src)
            | Value::Drop(src)
            | Value::BreakWith(_, src)
            | Value::Yield(src)
            | Value::TuplePut(_, _, src) => {
                Self::rewrite_subtree_to_nullable(src, data);
            }
            Value::Iter(_, init, step, body) => {
                Self::rewrite_subtree_to_nullable(init, data);
                Self::rewrite_subtree_to_nullable(step, data);
                Self::rewrite_subtree_to_nullable(body, data);
            }
            Value::Tuple(items) | Value::Insert(items) | Value::Parallel(items) => {
                for child in items {
                    Self::rewrite_subtree_to_nullable(child, data);
                }
            }
            // Other Value variants (Int / Text / Var / FnRef / Keys / etc.)
            // carry no nested fault-prone calls to rewrite.  Spans are
            // stripped by `unspan_mut` above.
            _ => {}
        }
    }

    /// Desugar `lhs ?? ...` — both the plain-default form and the
    /// `?? return ret_expr` early-return form.  Lifted out of
    /// [`Self::handle_operator`] so each shape has its own focused helper.
    ///
    /// `?? default`         → `if lhs_nonnull { lhs } else { default }`,
    ///                        with a temp for non-trivial lhs.
    ///
    /// `?? return ret_expr` → `{ tmp = lhs; if is_null(tmp) { return ret_expr }; tmp }`.
    fn handle_null_coalesce(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
        ctp: &mut Type,
    ) {
        // Redundant-coalesce warning for `not null` operands.
        if self.expr_not_null && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Warning,
                "Redundant null coalescing — '{}' is 'not null', default is never used",
                self.expr_not_null_name,
            );
        }
        self.expr_not_null = false;
        // Plan-07 phase 4h — if the `??` LHS is the just-emitted
        // field read site (set by `Parser::field()`), mark the
        // (struct, field) as defended so the not-null hint won't
        // fire on it.  Conservative: covers `p.field ?? default`;
        // complex expressions like `(p.field + 1) ?? 0` and
        // `if p.field != null` are slice-2 work.
        if let Some(key) = self.last_field_read_site.take() {
            self.defended_field_reads.insert(key);
        }

        // C54.G-hybrid: if the LHS is an immediate arithmetic call
        // (`a + b` / `a - b` / etc.), swap it to the Nullable variant so
        // overflow / div-zero produce `i32::MIN` instead of trapping — the
        // `??` below then discharges the null to the RHS.  Only the
        // outermost op gets swapped; nested sub-expressions still trap.
        if !self.first_pass {
            Self::rewrite_outer_arith_to_nullable(code, &self.data);
        }

        let lhs_type = ctp.clone();
        // @PLN17: boolean now has a real null sentinel (255), so `??` works — the
        // null-check for a boolean LHS is `lhs == null` (raw `== 255`), NOT the
        // value's truthiness (see the Boolean arm in `null_check_builder` below).
        // `false ?? x` stays `false` (false is not null); `null ?? x` → x.
        if self.lexer.has_token("return") {
            self.build_null_coalesce_return(code, ctp, &lhs_type);
        } else {
            self.build_null_coalesce_default(var_tp, code, parent_tp, precedence, ctp, &lhs_type);
        }
    }

    /// `lhs ?? return ret_expr` — emit the block that returns early when `lhs`
    /// is null and otherwise evaluates to `lhs`.
    fn build_null_coalesce_return(&mut self, code: &mut Value, ctp: &mut Type, lhs_type: &Type) {
        // Parse the optional return expression, coercing to the function's
        // declared return type.  Empty `return` in a non-void function
        // produces the typed null sentinel.
        let mut ret_val = Value::Null;
        let r_type = self.data.def(self.context).returned().clone();
        if !self.lexer.peek_token(";") && !self.lexer.peek_token("}") {
            let ret_pos = self.lexer.peek_pos().clone();
            let t = self.expression(&mut ret_val);
            if t != Type::Null && !self.convert(&mut ret_val, &t, &r_type) && !self.first_pass {
                self.validate_convert("return", &t, &r_type, &ret_pos);
            }
        } else if r_type != Type::Void && !self.first_pass {
            ret_val = self.null(&r_type);
        }
        let ret_stmt = Value::Return(Box::new(ret_val));

        // { tmp = lhs; if (tmp == null) { return ret_expr; }; tmp }
        let tmp = self.create_unique("ncr", lhs_type);
        let set_tmp = v_set(tmp, code.clone());
        let is_null = if matches!(lhs_type, Type::Boolean) {
            // @PLN17: a boolean is null iff its byte is the 255 sentinel — a raw
            // `tmp == null` compare, NOT truthiness (which would treat `false` as
            // missing).  `false ?? return` keeps `false`; `null ?? return` returns.
            let null_b = self.cl("OpConvBoolFromNull", &[]);
            self.cl("OpEqBool", &[Value::Var(tmp), null_b])
        } else {
            let mut null_check = Value::Var(tmp);
            self.convert(&mut null_check, lhs_type, &Type::Boolean);
            // Negate: true when null (i.e. when the boolean conversion is false).
            self.cl("OpNot", &[null_check])
        };
        let if_ret = v_if(is_null, ret_stmt, Value::Null);
        *code = v_block(
            vec![set_tmp, if_ret, Value::Var(tmp)],
            lhs_type.clone(),
            "ncr",
        );
        *ctp = lhs_type.clone();
    }

    /// `lhs ?? default` — emit the `if` (or temp + `if` for non-trivial lhs)
    /// that selects the default when `lhs` is null.
    fn build_null_coalesce_default(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
        ctp: &mut Type,
        lhs_type: &Type,
    ) {
        let mut rhs = Value::Null;
        let rhs_pos = self.lexer.peek_pos().clone();
        let rhs_type = self.parse_operators(var_tp, &mut rhs, parent_tp, precedence + 1);
        self.known_var_or_type(&rhs, &rhs_pos);

        if matches!(lhs_type, Type::Null) {
            // LHS is an untyped null literal: always use the RHS.
            *code = rhs;
            *ctp = rhs_type;
            return;
        }

        // @P316 — pick the coalesce result type.  When the value and default are
        // integers of DIFFERENT specs (e.g. a narrow `u8` element + an
        // `integer`/`i64` default), native can't unify the `if` branches — it emits
        // `(if … {u8} else {0_i64}) as u8` → E0308 (`convert` leniently accepts any
        // Integer→Integer without actually re-typing, so the branches keep their
        // own native widths).  Widen BOTH branches to `i64` so they share a native
        // type; the result is `i64` (matching the surrounding integer arithmetic).
        // Matching-width integers and non-integer types keep the original
        // behaviour (bring the default to the value's type; result = value's type).
        let widen_ints = matches!(lhs_type, Type::Integer(_))
            && matches!(rhs_type, Type::Integer(_))
            && *lhs_type != rhs_type
            // A CONSTANT integer default that provably fits the value's narrow type
            // coerces to that type instead of widening: a literal emits at the
            // target width, so the `if` branches still share a native type and the
            // E0308 @P316 guards against (a wider-width *variable* default) cannot
            // arise.  Keeping the result narrow lets `vec_of_u8[i] ?? 0` stay `u8`
            // — no `as u8` cast and no spurious narrowing error at a later
            // `out += [x]`.  A non-const or non-fitting default still widens.
            && !self.int_value_fits(&rhs, lhs_type);
        // NB (@PLN25 E2 gap 2): do NOT coalesce a `__nullable<S> ?? dense_S` to dense
        // here.  It is tempting (it would make `chosen = v[i] ?? mk()` infer dense
        // `S` and dodge the return-boundary unwrap), but it BREAKS the common
        // `out += [chosen]` shape — a dense `chosen` then needs a dense→Some WRAP on
        // every append, which fails in loop/if contexts.  Keep the conservative
        // `__nullable<S>` result and let each USE site coerce (dense-assign routing,
        // the ref_return return-boundary unwrap) — that keeps appends nullable→nullable.
        let result_type = if widen_ints {
            crate::data::I64.clone()
        } else {
            lhs_type.clone()
        };
        // Bring the default to the result type (widen narrow→i64, or the original
        // default→value-type convert); report a genuine mismatch (e.g. `text ?? 0`).
        if !self.convert(&mut rhs, &rhs_type, &result_type) && !self.first_pass {
            self.can_convert(&rhs_type, lhs_type);
        }
        // `convert(value → result_type)` widens the value branch when widen_ints,
        // and is a no-op otherwise (result_type == lhs_type).
        // @PLAN52 cluster IV-Tuple (2026-05-30): Tuple values are not heap-DbRef
        // and have no `.rec != 0` discriminant; testing the WHOLE tuple as a
        // DbRef (the default `convert(Tuple, Boolean)` path) produces wrong
        // codegen (native E0308: `expected bool, found tuple`) and silent
        // corruption on interpret (treating bytes as a DbRef tag).  Convention:
        // a tuple is null when its FIRST FIELD is its type's null sentinel —
        // which matches what `OpGetVectorNullable` produces for OOB tuple reads
        // (each field gets its own null sentinel).
        let null_check_builder = |this: &mut Self, src: &Value| -> Value {
            if let Type::Tuple(elems) = lhs_type
                && !elems.is_empty()
                && let Value::Var(v) = src
            {
                let first_tp = elems[0].clone();
                let mut nc = Value::TupleGet(*v, 0);
                this.convert(&mut nc, &first_tp, &Type::Boolean);
                nc
            } else if let Type::Enum(syn, true, _) = lhs_type
                && this.data.def(*syn).name.starts_with("__nullable<")
            {
                // @PLN25 E2 — a synthetic `__nullable<S>` enum backs an INLINE
                // field / vector element (no DbRef slot), so null is discriminant 0
                // — NOT the `.rec`/store_nr sentinel that `OpConvBoolFromRef` below
                // tests (it would misread the inline value).  The builder produces
                // "is NOT null" (v_if true → keep lhs), so test discriminant != 0.
                // Mirrors the E2a.4 `== null` lowering (`enum_null`).
                let get_enum = this.cl("OpGetEnum", &[src.clone(), Value::Int(0)]);
                let disc = this.cl("OpConvIntFromEnum", &[get_enum]);
                let is_null = this.cl("OpEqInt", &[disc, Value::Int(0)]);
                this.cl("OpNot", &[is_null])
            } else if matches!(
                lhs_type,
                Type::Reference(_, _)
                    | Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Enum(_, true, _),
            ) {
                // @PLAN52 cluster IV interpret (2026-05-30): heap-DbRef types
                // have no registered `OpConv*FromX → Boolean` so the generic
                // `convert(Hash/Vector/..., Boolean)` returns the bare Var.
                // The interpreter's `if <bare Var(DbRef)>` then tests raw
                // bytes (not `.rec != 0`) and produces the wrong result.
                // Force an explicit `OpConvBoolFromRef` call — Reference,
                // Vector, Hash, Sorted, Index, struct-Enum all share the
                // 12-byte DbRef representation under the hood.
                let conv_nr = this.data.def_nr("OpConvBoolFromRef");
                Value::Call(conv_nr, vec![src.clone()])
            } else if matches!(lhs_type, Type::Boolean) {
                // @PLN17: the null-check is "is NOT null" (v_if true → keep lhs).
                // For a boolean that is `src != null` (raw `!= 255`), NOT the
                // value's truthiness — so `false ?? x` keeps `false`.
                let null_b = this.cl("OpConvBoolFromNull", &[]);
                this.cl("OpNeBool", &[src.clone(), null_b])
            } else {
                let mut nc = src.clone();
                this.convert(&mut nc, lhs_type, &Type::Boolean);
                nc
            }
        };
        if let Value::Var(_) = code {
            // Simple variable: reading twice is side-effect-free.
            let mut lhs = code.clone();
            let null_check = null_check_builder(self, code);
            self.convert(&mut lhs, lhs_type, &result_type);
            *code = v_if(null_check, lhs, rhs);
        } else {
            // Non-trivial expression: materialise into a temp to avoid double
            // evaluation (L6 fix).
            //
            // @PLAN52 cluster I iteration 2 (2026-05-30): name `__ncc_N`
            // (double-underscore, matching loft's hoisted-temp convention)
            // and mark `skip_free` for text.  The skip_free flag suppresses
            // `OpFreeText(_ncc_N)` at block-scope exit (interpret side, see
            // `src/scopes.rs::get_free_vars`), so the present-path Str's
            // backing String outlives the block.  Native emit recognises
            // the `__ncc_*` prefix at `src/generation/emit.rs::output_block`
            // and wraps the tail with `.to_string()` INSIDE the block,
            // producing an owned String that the outer consumer can copy
            // safely.
            let tmp = self.create_unique("_ncc", lhs_type);
            // @PLAN52 cluster IV interpret (2026-05-30, iteration 3): extend
            // skip_free from Text to ALL heap-DbRef LHS types.  Same
            // mechanism: the scope-exit free op (`OpFreeText` for text,
            // `OpFreeRef` for heap-DbRef per `src/scopes.rs::get_free_vars`
            // heap-Free branch at line ~1274 which ALREADY honors
            // `is_skip_free`) is suppressed for `__ncc_N` temps so the
            // present-path value's backing storage outlives the block.
            // Closes Set E interpret (probes 21, 22, 23, 36, 41, 50).
            if matches!(
                lhs_type,
                Type::Text(_)
                    | Type::Reference(_, _)
                    | Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Enum(_, true, _),
            ) {
                self.vars.set_skip_free(tmp);
            }
            // #319 — heap-DbRef ncc temps need a function-entry `Set(tmp,
            // Null)` (the work-ref preamble) to reserve their stack slot:
            // their only Set lives inside the ncc block, which the Zone-2
            // slot scan does not walk.  Shapes whose assigned-var deps reach
            // the temp got a slot via scan_set's dep-prefix; a subject whose
            // dep chain is broken (e.g. a comprehension-built vector) did
            // not — "Incorrect var __ncc_N[65535]" at codegen.
            if matches!(
                lhs_type,
                Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
            ) {
                self.vars.register_work_ref(tmp);
            }
            let set_tmp = v_set(tmp, code.clone());
            let null_check = null_check_builder(self, &Value::Var(tmp));
            let mut true_branch = Value::Var(tmp);
            self.convert(&mut true_branch, lhs_type, &result_type);
            let if_expr = v_if(null_check, true_branch, rhs);
            *code = v_block(vec![set_tmp, if_expr], result_type.clone(), "ncc");
        }
        *ctp = result_type;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_operator(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
        ctp: &mut Type,
        operator: &str,
        op_pos: &Position,
    ) -> Option<Type> {
        if operator == "??" {
            self.handle_null_coalesce(var_tp, code, parent_tp, precedence, ctp);
        } else if operator == "as" {
            self.expr_not_null = false;
            if let Some(tps) = self.lexer.has_identifier() {
                let Some(tp) = self.parse_type(u32::MAX, &tps, false) else {
                    diagnostic!(self.lexer, Level::Error, "Expect type");
                    return Some(Type::Null);
                };
                // @PLAN48 P2: an explicit `as <narrow-int>` is the sanctioned way to
                // narrow `integer` → `i32`/`u8`/… — accept it here so the
                // implicit-narrowing diagnostic in `convert` does NOT fire on an
                // explicit cast.  The value stays in the 8-byte slot (a width-tag);
                // the narrow target type is returned below (`rt = tp`).
                if !Self::is_narrowing_int(ctp, &tp)
                    && !self.convert(code, ctp, &tp)
                    && !self.cast(code, ctp, &tp)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown cast from {} to {tps}",
                        &ctp.name(&self.data),
                    );
                }
                // Post-2c: remember the cast target alias so `f += x as i32`
                // can narrow the file-serialisation width.  Only stored when
                // the alias carries a `size(N)` annotation; otherwise
                // `u32::MAX` = "no alias info".
                let alias_nr = self.data.def_nr(&tps);
                if alias_nr != u32::MAX && self.data.forced_size(alias_nr).is_some() {
                    self.last_cast_alias = alias_nr;
                }
                let mut rt = tp;
                for d in ctp.depend() {
                    rt = rt.depending(d);
                }
                // #254: set the current type and fall through to `None` rather
                // than returning — `as` sits at the top precedence level, so
                // letting the Pratt loop continue lets a chained cast
                // (`x as integer as float`) parse left-associatively instead of
                // stranding the second `as` against an expected `;`.
                *ctp = rt;
            } else {
                diagnostic!(self.lexer, Level::Error, "Expect type after as");
            }
        } else if operator == "or" || operator == "||" {
            self.expr_not_null = false;
            self.boolean_operator(code, ctp, precedence, true);
            *ctp = Type::Boolean;
        } else if operator == "and" || operator == "&&" {
            self.expr_not_null = false;
            self.boolean_operator(code, ctp, precedence, false);
            *ctp = Type::Boolean;
        } else if operator == "=="
            || operator == "!="
            || operator == "<"
            || operator == "<="
            || operator == ">"
            || operator == ">="
        {
            let lhs_not_null = self.expr_not_null;
            let lhs_not_null_name = self.expr_not_null_name.clone();
            self.expr_not_null = false;
            let mut second_code = Value::Null;
            let tp = parent_tp.clone();
            *parent_tp = ctp.clone();
            let second_pos = self.lexer.peek_pos().clone();
            let second_type =
                self.parse_operators(var_tp, &mut second_code, parent_tp, precedence + 1);
            self.known_var_or_type(&second_code, &second_pos);
            if !self.first_pass && (operator == "==" || operator == "!=") {
                if second_type == Type::Null && lhs_not_null {
                    let always = if operator == "==" { "false" } else { "true" };
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        "Redundant null check — '{lhs_not_null_name}' is 'not null', comparison is always {always}",
                    );
                } else if *ctp == Type::Null && self.expr_not_null {
                    let always = if operator == "==" { "false" } else { "true" };
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        "Redundant null check — '{}' is 'not null', comparison is always {always}",
                        self.expr_not_null_name,
                    );
                }
            }
            self.expr_not_null = false;
            let vec_null = (operator == "==" || operator == "!=")
                && ((matches!(*ctp, Type::Vector(_, _)) && second_type == Type::Null)
                    || (*ctp == Type::Null && matches!(second_type, Type::Vector(_, _))));
            // A float/single null is the NaN sentinel, and NaN compares unequal to
            // everything (including itself), so `f == null` can't go through OpEq —
            // it would always be false.  Test validity instead: convert(float, bool)
            // is `!is_nan` (= non-null), so `== null` is its negation.
            let float_null = (operator == "==" || operator == "!=")
                && ((matches!(*ctp, Type::Float | Type::Single) && second_type == Type::Null)
                    || (*ctp == Type::Null && matches!(second_type, Type::Float | Type::Single)));
            // A nullable enum variable holds the reference null sentinel
            // (`store_nr==u16::MAX`), exactly like a struct reference.  Its `== null`
            // must test that sentinel via OpEqRef — the default path reads the
            // discriminant (`OpGetEnum`), which derefs the absent record and OOB-crashes.
            let enum_null = (operator == "==" || operator == "!=")
                && ((matches!(*ctp, Type::Enum(_, _, _)) && second_type == Type::Null)
                    || (*ctp == Type::Null && matches!(second_type, Type::Enum(_, _, _))));
            if vec_null {
                // @PLN25: `vector == null` / `vector != null` tests the null
                // sentinel (store_nr == u16::MAX) via OpVectorIsNull — NOT eq_ref,
                // whose rec==0 null test would also match an empty `[]`.
                if !self.first_pass {
                    let vec_code = if matches!(*ctp, Type::Vector(_, _)) {
                        code.clone()
                    } else {
                        second_code
                    };
                    let is_null = self.cl("OpVectorIsNull", &[vec_code]);
                    *code = if operator == "==" {
                        is_null
                    } else {
                        self.cl("OpNot", &[is_null])
                    };
                }
                *ctp = Type::Boolean;
            } else if float_null {
                if !self.first_pass {
                    let (f_code, f_tp) = if *ctp == Type::Null {
                        (second_code, second_type.clone())
                    } else {
                        (code.clone(), ctp.clone())
                    };
                    // convert(float, boolean) = !is_nan = "is non-null".
                    let mut valid = f_code;
                    self.convert(&mut valid, &f_tp, &Type::Boolean);
                    *code = if operator == "==" {
                        self.cl("OpNot", &[valid])
                    } else {
                        valid
                    };
                }
                *ctp = Type::Boolean;
            } else if enum_null {
                if !self.first_pass {
                    let (e_code, e_def) = if *ctp == Type::Null {
                        let d = match &second_type {
                            Type::Enum(d, _, _) => *d,
                            _ => u32::MAX,
                        };
                        (second_code, d)
                    } else {
                        let d = match &*ctp {
                            Type::Enum(d, _, _) => *d,
                            _ => u32::MAX,
                        };
                        (code.clone(), d)
                    };
                    // @PLN25 E2a.4 — a synthetic `__nullable<S>` enum backs an INLINE
                    // struct field / vector element (no DbRef slot), so null is
                    // discriminant 0 — read it directly (OpGetEnum @ offset 0), NEVER
                    // OpRefIsNull, whose store_nr sentinel test would deref the absent
                    // record and OOB-crash.  A user enum VARIABLE is a DbRef whose null
                    // IS the store_nr sentinel (E1) — keep OpRefIsNull for it.
                    let inline =
                        e_def != u32::MAX && self.data.def(e_def).name.starts_with("__nullable<");
                    let is_null = if inline {
                        let get_enum = self.cl("OpGetEnum", &[e_code, Value::Int(0)]);
                        let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
                        self.cl("OpEqInt", &[disc, Value::Int(0)])
                    } else {
                        // Test the null sentinel via store_nr (OpRefIsNull), NOT OpEqRef's
                        // rec==0: a present enum is inline-represented on native and carries
                        // rec==0, which rec==0 would misread as null.
                        self.cl("OpRefIsNull", &[e_code])
                    };
                    *code = if operator == "==" {
                        is_null
                    } else {
                        self.cl("OpNot", &[is_null])
                    };
                }
                *ctp = Type::Boolean;
            } else if operator == ">" {
                *ctp = self.call_op(
                    code,
                    "<",
                    &[second_code, code.clone()],
                    &[second_type, ctp.clone()],
                );
            } else if operator == ">=" {
                *ctp = self.call_op(
                    code,
                    "<=",
                    &[second_code, code.clone()],
                    &[second_type, ctp.clone()],
                );
            } else {
                *ctp = self.call_op(
                    code,
                    operator,
                    &[code.clone(), second_code],
                    &[ctp.clone(), second_type],
                );
            }
            *parent_tp = tp;
        } else {
            self.expr_not_null = false;
            let mut second_code = Value::Null;
            let second_pos = self.lexer.peek_pos().clone();
            let second_type =
                self.parse_operators(var_tp, &mut second_code, parent_tp, precedence + 1);
            self.known_var_or_type(&second_code, &second_pos);
            if !self.first_pass
                && (operator == "/" || operator == "%")
                && (matches!(second_code, Value::Int(0)) || matches!(second_code, Value::Long(0)))
            {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "{} by constant zero — result is always null",
                    if operator == "/" {
                        "Division"
                    } else {
                        "Modulo"
                    }
                );
            }
            *ctp = self.call_op(
                code,
                operator,
                &[code.clone(), second_code],
                &[ctp.clone(), second_type],
            );
            // Plan-07 phase 1, step 1.B.1 — wrap binary fault-prone
            // arithmetic ops in `Value::Span` so runtime errors
            // (div-by-zero, narrow overflow, signed-overflow panic
            // from `checked_long!`) can be reported with the
            // operator's source position.  Covers `+ - * / %` plus
            // shifts; comparisons and boolean ops never panic, so
            // they stay unwrapped (saves IR size).
            //
            // Walker discipline: every IR walker that pattern-matches
            // `Value::Call(...)` either calls `unspan()` first or
            // carries a `Value::Span(b)` arm (scopes.rs, intervals.rs,
            // slots.rs, slots_v2.rs, validate.rs, codegen.rs,
            // parser/mod.rs::substitute_type_in_value, generation/*).
            if !self.first_pass && matches!(operator, "+" | "-" | "*" | "/" | "%" | "<<" | ">>") {
                let inner = std::mem::replace(code, Value::Null);
                *code = Value::with_span(op_pos.clone(), inner);
            }
            // The result of a binary arithmetic op is a *computed value*, not
            // a `not null` field read — `/` and `%` can yield null on
            // divide-by-zero, and parsing the RHS just above re-set
            // `expr_not_null` to reflect the RHS operand's field-nullness
            // (e.g. `len(v) / set.stride` where `stride` is `not null`).
            // Clear it so `a / b ?? default` is NOT flagged "Redundant null
            // coalescing" — companion to the vector-index clear in
            // `fields.rs` (a `??` after a fault-prone op is a real defense).
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Plan-07 phase 4e.2 — undefended fault-site compile-time warning
// ---------------------------------------------------------------------------
//
// Walks a parsed function body and emits `Level::Warning` at each
// fault-prone op call (OpDivInt / OpRemInt / OpGetVector / OpVectorRef /
// OpTextCharacter) that survived the 4d.1 / 4d.2 / 4e.1 swap passes —
// i.e., sites with no recognised defense.  The four canonical safe
// patterns from `04-runtime-error-kinds.md § Easy-proof skip list` are
// recognised here and quietly suppressed; without the skip list the
// warning would fire on most well-written loft code and developers
// would silence it within a session, defeating its purpose.
//
// Skip patterns implemented (per the design):
//   1. Constant non-zero literal divisor for OpDivInt / OpRemInt.
//   2. Constant non-negative literal index for OpGetVector / OpVectorRef
//      / OpTextCharacter (the developer typed a literal — trust it; if
//      it overruns at runtime the runtime fault still fires).
//   3. Index is the iteration variable of an enclosing for-loop
//      (`for i in <range> { … v[i] … }` — the loop's bound is the
//      developer's contract).
//   4. (Implicitly) sites inside `if x != null` / `if x` / format-string
//      interpolations — those were already swapped to Nullable peers by
//      4d.2 / 4e.1 BEFORE this walker runs, so the raising form is
//      gone from the IR there.

#[derive(Default)]
struct WarnCtx {
    /// Variable slots currently active as for-loop iteration variables.
    /// When a fault op uses `Var(v)` as its index AND `v` ∈ this set, we
    /// skip the warning (skip pattern 3).
    iter_vars: std::collections::HashSet<u16>,
    /// `(idx_var, vec_var)` pairs proven safe by an enclosing
    /// `if idx_var < len(vec_var) { ... }` guard.  Pushed on entry to
    /// the `then` block, popped on exit.  Skip pattern 5.
    guarded_pairs: Vec<(u16, u16)>,
    /// Map from local-var slot → vec-var slot when the local was bound
    /// via `n = len(vec)`.  Skip pattern 5 with `if i < n { v[i] }` then
    /// becomes equivalent to `if i < len(v) { v[i] }` via the lookup.
    /// Populated when walking `Value::Set(local, Call(len, [Var(vec)]))`.
    len_captures: std::collections::HashMap<u16, u16>,
    /// Position of the innermost enclosing `Value::Span` — used as the
    /// fault site's source location when we emit a warning.
    last_pos: Option<Position>,
}

#[derive(Copy, Clone)]
enum FaultKind {
    Div,
    Rem,
    VectorIndex,
    TextIndex,
}

impl Parser {
    /// Plan-07 phase 4e.2 — entry point.  Called from `parse_function`
    /// AFTER `parse_code` (the body parse) AND after `vars.test_used` /
    /// `warn_upper_case_locals` (so the diagnostic ordering matches
    /// existing per-function passes).  Second-pass only — the first
    /// pass doesn't have the swap-pass results in place.
    ///
    /// Silenceable via `LOFT_NO_WARN_RUNTIME=1` env var.  Stdlib
    /// (`default/*.loft`) is exempt — its functions are
    /// language-internal trusted code (they implement the very
    /// fault-handling primitives the warning is meant to nudge users
    /// toward) and warning on them is noise.
    pub(crate) fn warn_undefended_fault_sites(&mut self, body: &Value) {
        if self.default {
            return;
        }
        if std::env::var("LOFT_NO_WARN_RUNTIME").is_ok_and(|v| v == "1" || v == "true") {
            return;
        }
        let mut ctx = WarnCtx::default();
        // Initialise `last_pos` to the function's own source position so
        // any fault inside the body that lacks a finer-grained
        // `Value::Span` wrapper attributes to the function (the
        // legitimate fallback) instead of leaking to the lexer's
        // current cursor — which is *past* the just-parsed body and
        // would otherwise point at the next function's start.
        let fn_pos = self.data.definitions[self.context as usize]
            .position
            .clone();
        ctx.last_pos = Some(fn_pos);
        self.walk_for_warnings(body, &mut ctx);
    }

    fn walk_for_warnings(&mut self, code: &Value, ctx: &mut WarnCtx) {
        match code {
            Value::Span(boxed) => {
                let saved = ctx.last_pos.clone();
                ctx.last_pos = Some(boxed.0.clone());
                self.walk_for_warnings(&boxed.1, ctx);
                ctx.last_pos = saved;
            }
            Value::Call(def_nr, args) => {
                let name = self.data.def(*def_nr).original_name();
                let kind: Option<FaultKind> = match name.as_str() {
                    "DivInt" | "DivFloat" | "DivSingle" => Some(FaultKind::Div),
                    "RemInt" | "RemFloat" | "RemSingle" => Some(FaultKind::Rem),
                    "GetVector" | "VectorRef" => Some(FaultKind::VectorIndex),
                    "TextCharacter" => Some(FaultKind::TextIndex),
                    _ => None,
                };
                if let Some(kind) = kind
                    && !is_easy_proof(kind, args, ctx, &self.data)
                {
                    self.emit_undefended_warning(kind, ctx);
                }
                for arg in args {
                    self.walk_for_warnings(arg, ctx);
                }
            }
            Value::CallRef(_, args) => {
                for arg in args {
                    self.walk_for_warnings(arg, ctx);
                }
            }
            Value::Iter(_, init, step, body) => {
                // The Iter's `u16` is the iterator's INTERNAL var
                // (e.g., `i#index` / `range`), NOT the user-visible
                // loop var.  The user-visible loop var is set OUTSIDE
                // the Iter via `Loop { Set(loop_var, Iter(…)); body }`
                // — handled by the `Loop` arm's lookahead below.
                self.walk_for_warnings(init, ctx);
                self.walk_for_warnings(step, ctx);
                self.walk_for_warnings(body, ctx);
            }
            Value::Block(b) => {
                for child in &b.operators {
                    self.walk_for_warnings(child, ctx);
                }
            }
            Value::Loop(b) => {
                // Recognise the canonical for-loop shape that the
                // parser emits:
                //   Loop {
                //     Set(loop_var, Block { name: "Iter range" / "Iter …",
                //                           operators: [increment, break-check, yield] });
                //     Block { user body using loop_var };
                //   }
                // Marking `loop_var` as iter-bound for the whole
                // Loop's operators makes skip-pattern 3 fire on
                // `v[loop_var]` reads inside the body.
                //
                // We accept either:
                //  (a) the RHS Block is named with an "Iter " prefix
                //      (today: "Iter range" / "Iter " variants), or
                //  (b) the RHS subtree contains a `Value::Break` (the
                //      iter step signals end-of-iteration via break).
                // Both are robust to parser-name changes — (b) is the
                // semantic check, (a) is the fast-path.
                fn contains_break(v: &Value) -> bool {
                    v.any_node(&mut |n| matches!(n, Value::Break(_) | Value::BreakWith(_, _)))
                }
                let mut loop_vars_added: Vec<u16> = Vec::new();
                for child in &b.operators {
                    if let Value::Set(loop_var, src) = child.unspan() {
                        let is_iter_step = match src.unspan() {
                            Value::Iter(..) => true,
                            Value::Block(blk) => {
                                blk.name.starts_with("Iter ")
                                    || blk.operators.iter().any(contains_break)
                            }
                            _ => false,
                        };
                        if is_iter_step && ctx.iter_vars.insert(*loop_var) {
                            loop_vars_added.push(*loop_var);
                        }
                    }
                }
                for child in &b.operators {
                    self.walk_for_warnings(child, ctx);
                }
                for v in loop_vars_added {
                    ctx.iter_vars.remove(&v);
                }
            }
            Value::If(cond, then_b, else_b) => {
                self.walk_for_warnings(cond, ctx);
                // Skip pattern 5 — recognise `if idx < len(vec) { ... }` and
                // `if idx < n { ... }` (where `n` is a captured `len(vec)`),
                // and push (idx_var, vec_var) onto the guarded-pairs stack so
                // indexing inside `then_b` is treated as safe.  Also walks
                // AND-conjuncted conditions (`if a<len(u) and b<len(v) { ... }`)
                // and pushes each qualifying conjunct.
                let pushed = collect_guard_pairs(cond.unspan(), &self.data, &ctx.len_captures);
                for pair in &pushed {
                    ctx.guarded_pairs.push(*pair);
                }
                self.walk_for_warnings(then_b, ctx);
                for _ in &pushed {
                    ctx.guarded_pairs.pop();
                }
                self.walk_for_warnings(else_b, ctx);
            }
            Value::Set(local_var, src) => {
                // Skip-pattern 5 capture — `n = len(vec)` registers `n` as
                // "the length of vec" for later `if i < n { v[i] }` proofs.
                if let Some(vec_var) = len_capture_target(src.unspan(), &self.data) {
                    ctx.len_captures.insert(*local_var, vec_var);
                }
                self.walk_for_warnings(src, ctx);
            }
            Value::Return(src)
            | Value::Drop(src)
            | Value::BreakWith(_, src)
            | Value::Yield(src)
            | Value::TuplePut(_, _, src) => {
                self.walk_for_warnings(src, ctx);
            }
            Value::Tuple(items) | Value::Insert(items) | Value::Parallel(items) => {
                for child in items {
                    self.walk_for_warnings(child, ctx);
                }
            }
            // Other Value variants (Int / Long / Text / Var / FnRef /
            // Keys / Boolean / etc.) carry no nested fault-prone calls.
            _ => {}
        }
    }

    fn emit_undefended_warning(&mut self, kind: FaultKind, ctx: &WarnCtx) {
        let msg = match kind {
            // @P368 — wording: not "integer" (the same `/` warning covers float
            // and single division too).
            FaultKind::Div => {
                "division may produce null on divide-by-zero with no defensive check; \
                 consider `a / b ?? 0` or wrap in `if b != 0 { ... }`"
            }
            FaultKind::Rem => {
                "modulus may produce null on divide-by-zero with no defensive check; \
                 consider `a % b ?? 0` or wrap in `if b != 0 { ... }`"
            }
            FaultKind::VectorIndex => {
                "`v[i]` may produce null on out-of-bounds with no defensive check; \
                 consider `v[i] ?? <fallback>`, `if i < len(v) { v[i] }`, \
                 or `x = v[i]; if x != null { ... }`"
            }
            FaultKind::TextIndex => {
                "`s[i]` may produce null on out-of-bounds with no defensive check; \
                 consider `s[i] ?? <fallback>`, `if i < len(s) { s[i] }`, \
                 or `x = s[i]; if x != null { ... }`"
            }
        };
        if let Some(pos) = &ctx.last_pos {
            self.lexer.pos_diagnostic(Level::Warning, pos, msg);
        } else {
            self.lexer.diagnostic(Level::Warning, msg);
        }
    }
}

/// Evaluate the easy-proof skip list against a fault-prone call's args.
/// Returns `true` when a skip pattern matches and the warning should
/// NOT fire.
fn is_easy_proof(kind: FaultKind, args: &[Value], ctx: &WarnCtx, data: &Data) -> bool {
    fn lit_int(v: &Value) -> Option<i64> {
        match v.unspan() {
            Value::Int(n) => Some(i64::from(*n)),
            Value::Long(n) => Some(*n),
            _ => None,
        }
    }
    // @P368 — a literal divisor is statically known; only a *non-zero* literal
    // makes divide-by-zero impossible.  Covers float / single literals too
    // (`x / 2.0`, `x / 0.75`), which `lit_int` missed — so the divide-by-zero
    // warning no longer fires on a statically-safe float division.
    //
    // @P368 follow-up — when the dividend is float / single and the divisor
    // is an integer literal (`x / 3` with `x: float`), the parser wraps the
    // literal in an `OpConvFloatFromInt` / `OpConvSingleFromInt` cast so the
    // types match.  Without seeing through that cast, `lit_nonzero` on the
    // outer Call returns None and the warning fires spuriously.  Add a
    // recursive look-through for the two widening casts that wrap integer
    // literals on the divisor path; OpConvFloatFromSingle (single→float)
    // doesn't apply because no single literal can wrap an integer literal.
    fn lit_nonzero(v: &Value, data: &Data) -> Option<bool> {
        match v.unspan() {
            Value::Int(n) => Some(*n != 0),
            Value::Long(n) => Some(*n != 0),
            Value::Float(f) => Some(*f != 0.0),
            Value::Single(f) => Some(*f != 0.0),
            Value::Call(def_nr, call_args) => {
                // `original_name()` strips the "Op" prefix, so the
                // names here are without it.
                let name = data.def(*def_nr).original_name();
                if (name == "ConvFloatFromInt" || name == "ConvSingleFromInt")
                    && call_args.len() == 1
                {
                    lit_nonzero(&call_args[0], data)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    match kind {
        FaultKind::Div | FaultKind::Rem => {
            // Skip pattern 1 — divisor is a non-zero literal (int or float).
            args.get(1)
                .and_then(|v| lit_nonzero(v, data))
                .unwrap_or(false)
        }
        FaultKind::VectorIndex | FaultKind::TextIndex => {
            // Index is the LAST arg in both `OpGetVector(coll, size, idx)`
            // and `OpVectorRef(coll, idx)` and `OpTextCharacter(text, idx)`.
            let Some(idx) = args.last() else {
                return false;
            };
            // Skip pattern 2 — non-negative literal index.
            if lit_int(idx).is_some_and(|n| n >= 0) {
                return true;
            }
            // Skip pattern 3 — index is an active for-loop iteration var.
            if let Value::Var(v) = idx.unspan()
                && ctx.iter_vars.contains(v)
            {
                return true;
            }
            // Skip pattern 4 — index is *loop-bounded arithmetic*: an
            // expression built only from active loop-iteration variables
            // and integer literals (e.g. `mul_k * 4 + mul_row`, `i / 4`,
            // `row * width + col` where every variable is a loop counter).
            // This generalises pattern 3 (a bare loop var) to the integer
            // arithmetic the counters participate in — the developer is
            // iterating within the loop's bounds, so the computed index is
            // as trustworthy as the bare counter.  An index that mixes in a
            // struct field, a parameter, or a call result is NOT bounded
            // here and still warns (the genuine "could be OOB from data"
            // case).
            if index_loop_bounded(idx, ctx, data) {
                return true;
            }
            // Skip pattern 5 — `if idx_var < len(vec_var) { ... v[idx_var] ... }`.
            // When an enclosing If's condition proves `idx < len(vec)`, the
            // walker has pushed `(idx_var, vec_var)` onto `guarded_pairs`.
            // Match the indexing's vec arg (first arg) + idx arg (last arg)
            // against any pushed pair; if both are bare `Var(_)` references
            // to a guarded pair, the index is safe.
            // Skip pattern 5 — `if idx_var < len(vec_var)` proves any
            // indexing `vec_var[expr]` where `expr` is loop/literal/guarded-var
            // arithmetic over the guard's idx_var.  Strip casts on the idx.
            if let Some(first) = args.first()
                && let Value::Var(vec_var) = first.unspan()
            {
                let unwrapped = unwrap_cond(idx, data);
                if let Value::Var(idx_var) = unwrapped
                    && ctx
                        .guarded_pairs
                        .iter()
                        .any(|(i, v)| i == idx_var && v == vec_var)
                {
                    return true;
                }
            }
            false
        }
    }
}

/// Recognise `idx_var < len(vec_var)` (canonical bounds check).
/// Returns `Some((idx_var, vec_var))` on a match, `None` otherwise.
/// Skip pattern 5 in `is_easy_proof` consults the resulting pair via
/// `WarnCtx::guarded_pairs`.
/// Recognise `if idx_var < len(vec_var) { ... }` (or
/// `if idx_var < n { ... }` where `n` was captured by `n = len(vec_var)`),
/// returning `(idx_var, vec_var)` on a match.  Skip pattern 5's entry
/// point for the If walker — pushed onto `WarnCtx::guarded_pairs` for
/// the duration of the then-block.
fn guard_pair_with_ctx(
    v: &Value,
    data: &Data,
    captures: Option<&std::collections::HashMap<u16, u16>>,
) -> Option<(u16, u16)> {
    let inner = unwrap_cond(v, data);
    let Value::Call(def_nr, args) = inner else {
        return None;
    };
    let raw = data.def(*def_nr).original_name();
    let name = raw.strip_suffix("Nullable").unwrap_or(raw.as_str());
    if name != "LtInt" || args.len() != 2 {
        return None;
    }
    let Value::Var(idx_var) = args[0].unspan() else {
        return None;
    };
    // RHS can be either `len(<vec>)` inline OR a bare Var that was
    // captured earlier as `<local> = len(<vec>)`.
    let rhs = args[1].unspan();
    let vec_var = match rhs {
        Value::Call(len_def, len_args) => {
            let len_raw = data.def(*len_def).original_name();
            let len_name = len_raw.strip_suffix("Nullable").unwrap_or(len_raw.as_str());
            if !matches!(len_name, "len" | "LengthVector") || len_args.len() != 1 {
                return None;
            }
            let Value::Var(v) = len_args[0].unspan() else {
                return None;
            };
            *v
        }
        Value::Var(n) => {
            let caps = captures?;
            *caps.get(n)?
        }
        _ => return None,
    };
    Some((*idx_var, vec_var))
}

/// Collect every `(idx_var, vec_var)` pair that `cond` proves safe.
/// Handles both a single comparison and AND-conjuncted comparisons
/// like `if a < len(u) and b < len(v) { ... }`.  Caller pushes each
/// returned pair onto `ctx.guarded_pairs` for the duration of the
/// then-block, then pops them.
fn collect_guard_pairs(
    cond: &Value,
    data: &Data,
    captures: &std::collections::HashMap<u16, u16>,
) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    collect_guard_pairs_into(cond, data, captures, &mut out);
    out
}

fn collect_guard_pairs_into(
    cond: &Value,
    data: &Data,
    captures: &std::collections::HashMap<u16, u16>,
    out: &mut Vec<(u16, u16)>,
) {
    // Strip Conv* casts (handled by unwrap_cond).
    let inner = unwrap_cond(cond, data);
    // Loft's short-circuit `a and b` lowers to `if a then b else false`
    // — recurse into both arms so each conjunct contributes.  Match on
    // shape `If(left, right, Boolean(false))`.
    if let Value::If(left, right, else_v) = inner
        && matches!(else_v.unspan(), Value::Boolean(false))
    {
        collect_guard_pairs_into(left, data, captures, out);
        collect_guard_pairs_into(right, data, captures, out);
        return;
    }
    // Also recurse into explicit AND-call forms in case the lowering
    // changes / for `int & int`-style bool checks built on LandInt.
    if let Value::Call(def_nr, args) = inner {
        let raw = data.def(*def_nr).original_name();
        let name = raw.strip_suffix("Nullable").unwrap_or(raw.as_str());
        if matches!(name, "AndBool" | "And" | "LandInt") && args.len() == 2 {
            collect_guard_pairs_into(&args[0], data, captures, out);
            collect_guard_pairs_into(&args[1], data, captures, out);
            return;
        }
    }
    if let Some(pair) = guard_pair_with_ctx(inner, data, Some(captures)) {
        out.push(pair);
    }
}

/// True when `src` is a `len(<Var>)` call — used to recognise the
/// `<local> = len(<vec>)` capture pattern.  Returns the vec var-id.
fn len_capture_target(src: &Value, data: &Data) -> Option<u16> {
    let Value::Call(def_nr, args) = src.unspan() else {
        return None;
    };
    let raw = data.def(*def_nr).original_name();
    let name = raw.strip_suffix("Nullable").unwrap_or(raw.as_str());
    if !matches!(name, "len" | "LengthVector") || args.len() != 1 {
        return None;
    }
    let Value::Var(v) = args[0].unspan() else {
        return None;
    };
    Some(*v)
}

/// Strip `ConvBoolFromInt` / `ConvIntFromInt` casts from a condition
/// expression so the underlying comparison call is reachable.
fn unwrap_cond<'a>(v: &'a Value, data: &Data) -> &'a Value {
    let mut cur = v.unspan();
    loop {
        let Value::Call(def_nr, args) = cur else {
            return cur;
        };
        if args.len() != 1 {
            return cur;
        }
        let raw = data.def(*def_nr).original_name();
        if !raw.starts_with("Conv") {
            return cur;
        }
        cur = args[0].unspan();
    }
}

/// True when `v` is an index expression composed solely of active
/// loop-iteration variables (`ctx.iter_vars`), integer literals, and
/// integer arithmetic over them — i.e. the index is bounded by the same
/// loops that produce it.  See `is_easy_proof` skip-pattern 4.
fn index_loop_bounded(v: &Value, ctx: &WarnCtx, data: &Data) -> bool {
    match v.unspan() {
        Value::Int(_) | Value::Long(_) => true,
        Value::Var(n) => ctx.iter_vars.contains(n),
        Value::Call(def_nr, call_args) => {
            // `original_name()` strips the "Op" prefix; arithmetic also
            // appears in `*Nullable` form when an operand is nullable.
            let raw = data.def(*def_nr).original_name();
            let name = raw.strip_suffix("Nullable").unwrap_or(raw.as_str());
            // Integer arithmetic / bitwise ops ("MinInt" is subtraction —
            // loft spells minus "Min").  A result built from bounded
            // operands stays bounded.
            const ARITH: &[&str] = &[
                "AddInt",
                "MinInt",
                "MulInt",
                "DivInt",
                "RemInt",
                "AbsInt",
                "EorInt",
                "LandInt",
                "LorInt",
                "SLeftInt",
                "SRightInt",
            ];
            ARITH.contains(&name) && call_args.iter().all(|a| index_loop_bounded(a, ctx, data))
        }
        _ => false,
    }
}
