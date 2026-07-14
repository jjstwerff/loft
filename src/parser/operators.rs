// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    Data, IntegerSpec, Level, OPERATORS, Parser, Position, Type, Value, diagnostic_format, rename,
    v_block, v_if, v_set,
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
            // A branch RHS (`q = if …`, `q = match …`, `q = a ?? b`) delivers per ARM
            // into `q` instead of being lowered as a value — see `try_branch_text_bind`.
            if self.try_branch_text_bind(code, var_nr) {
                // `code` is now a void branch of `Set(q, …)`; nothing further to wrap.
            }
            // detect self-reference (t = t[N..], t = fn(t), etc.)
            // If the RHS reads from the same variable being assigned, use a
            // work text to avoid the clear-before-read problem.
            else if code.reads_var(var_nr) {
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
                    // Cluster I-d (@PLN85 cluster V) — `a = <call> + …`:
                    // parse_append_vector emits a leading `Set(a, call())` so `a`
                    // ADOPTS the call's fresh `["??"]` store (no copy).  Allocating a
                    // SECOND `__vdb` backing here orphans that empty store — it leaks
                    // when `a` escapes (the N shape: `a = head()+head()`, returned).
                    // The adopt IS the body providing a's store, exactly like the
                    // literal-init `body_allocates` case — so skip `vector_db` too.
                    let body_adopts_call = ls.first().is_some_and(|first| {
                        matches!(first.unspan(), Value::Set(s, rhs) if *s == var_nr
                            && matches!(rhs.unspan(), Value::Call(d, _)
                                if self.data.def(*d).name().starts_with("n_")
                                    && *self.data.def(*d).code() != Value::Null
                                    && !self.data.def(*d).returns_borrowed_view()))
                    });
                    // The materialise prefix reads v while it is still intact, so
                    // it MUST precede the `vector_db` clear: insert prefix ++
                    // vector_db ops together, ahead of the body (prefix first).
                    let mut front = prefix;
                    if !body_allocates && !body_adopts_call {
                        let db_ops = self.vector_db(tp, var_nr);
                        // @PLN85 #492 — `vector_db` no-ops on a non-rebind ARGUMENT (the
                        // NRVO return buffer IS the caller's store, so it cannot get a
                        // fresh backing).  Without a fresh store, a `=` NON-EMPTY literal
                        // reassignment on the buffer (`r = [9,9]` in a JOIN arm, after an
                        // earlier arm's `r = x` filled it) would APPEND to the stale
                        // content instead of REPLACING — the returned vector piles both
                        // arms (`[1,2,3,9,9]`).  Clear the buffer first: `=` is a replace,
                        // so this is correct on a fresh buffer (a no-op) and a filled one.
                        // The empty-literal `v = []` reassign is handled by the
                        // `ls.is_empty()` clear below; a rebind param gets a fresh backing
                        // (`db_ops` non-empty), so neither reaches here.
                        if db_ops.is_empty()
                            && !self.first_pass
                            && self.vars.is_argument(var_nr)
                            && !ls.is_empty()
                        {
                            front.push(self.cl("OpClearVector", &[Value::Var(var_nr)]));
                        } else {
                            front.extend(db_ops);
                        }
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
                    | Type::Radix(_, _, _)
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

    // @F37 — operator set (arithmetic/comparison/logical/bitwise/unary, precedence, **)
    pub(crate) fn parse_operators(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
    ) -> Type {
        let mut ls = Vec::new();
        if precedence >= OPERATORS.len() {
            // @PLN87 — a PREFIX `&<lvalue>` is a reference-to, distinct from the binary
            // `&` (bitwise-and "Land"). The binary form only ever sits BETWEEN operands,
            // so a `&` seen here at an operand START is the prefix: it flags the `&`-ness
            // via `amp_pending` so `parse_assign_op` can lower a scalar reference to
            // `OpCreateStack`. (`&&` is its own token, so this never mis-fires on
            // logical-and.)
            if self.lexer.has_token("&") {
                self.amp_pending = true;
                let t = self.parse_part(var_tp, code, parent_tp);
                // @PLN87 — `&` is a binding marker, NOT a general operator.  It is valid
                // ONLY as the WHOLE right-hand side of an assignment (`a = &b`), which —
                // since loft statements are `;`-terminated — is always followed by `;`
                // (a block-final reference `{ … &x }` by `}`).  `parse_part` has consumed
                // the whole lvalue, so the NEXT token classifies the use:
                //   `=`/`+=`/…  → `&` on the assignment TARGET (`&var = …`)   [error]
                //   not `;`/`}` → `&` as a sub-expression / argument          [error]
                //   `;`/`}`     → a valid binding RHS → check the operand is a PLACE (#1)
                // (Local peek; avoids the global `amp_pending` flag which leaks.)
                if !self.first_pass {
                    let next_assign = self.lexer.peek_token("=")
                        || self.lexer.peek_token("+=")
                        || self.lexer.peek_token("-=")
                        || self.lexer.peek_token("*=")
                        || self.lexer.peek_token("%=")
                        || self.lexer.peek_token("/=");
                    let next_terminates = self.lexer.peek_token(";") || self.lexer.peek_token("}");
                    // An invalid `&` must not stay "pending": clear the flag in each
                    // error branch so it cannot leak into `parse_assign_op`'s
                    // reference-lowering (1160) or the D-bind-7 bare-statement guard
                    // in `parse_assign` (which would then double-report).  Only the
                    // accepted case (terminates AND is a place) keeps `amp_pending`
                    // true, to be consumed by the binding that follows.
                    if next_assign {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "`&` cannot appear on the left of an assignment — it marks a \
                             binding as a link to its source at the binding site (`x = &src`), \
                             not an assignment target; drop the `&` (the binding is already linked)"
                        );
                        self.amp_pending = false;
                    } else if !next_terminates {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "`&` is not a general operator — it binds a reference only as the \
                             whole right-hand side of an assignment (`a = &b`). Pass a `&` \
                             parameter WITHOUT `&` (`f(x)`, the reference comes from the \
                             parameter type); do not use `&` in an argument or sub-expression"
                        );
                        self.amp_pending = false;
                    } else if !Self::is_amp_place(code, &self.data) {
                        // #1 — a valid binding RHS still needs a PLACE operand.
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "`&` requires an addressable operand — a variable, struct field, \
                             or vector element — not a temporary (a literal, computed value, \
                             or call result)"
                        );
                        self.amp_pending = false;
                    }
                }
                return t;
            }
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
        // @PLN102 pre-freeze — comparison operators are NON-ASSOCIATIVE.  A chain like
        // `a == b == c` (or `a < b < c`) parses as `(a == b) == c`, silently comparing a
        // BOOLEAN to the third operand — a classic footgun.  Reject the second comparison at
        // this level; the explicit `(a == b) == c` still works (its inner compare is a
        // separate parse inside the paren primary), as does `a < b && b < c`.
        let mut compared_at_this_level = false;
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
                        // @PLN25 slice (c): peel `Optional` so a `text?` accumulator enters
                        // the concat path (the nullable-left propagate wrap fires below).
                        current_type.base().clone()
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
                        // @PLN25 slice (c): nullable LEFT accumulator (`text? + …`) PROPAGATES —
                        // if the left is null the whole concat is null (people must be aware of
                        // nulls, not have them silently swept into "\0"/"null"). Build the concat
                        // into a FRESH work-text off the left, then wrap:
                        //   if is_not_null(left) { concat } else { null } : text?
                        // (proven equivalent to `if s==null {null} else {(s??"")+parts}`).
                        if matches!(&current_type, Type::Optional(inner) if matches!(**inner, Type::Text(_)))
                        {
                            let base_text = Type::Text(crate::data::Deps::none());
                            // The null arm must WRITE the null-text sentinel (`OpConvTextFromNull`),
                            // not a bare `Value::Null` — for a buffer-backed return the bare null
                            // leaves the return buffer empty and interp reads "" back instead of
                            // null (the normal `else { null }` flow converts it the same way).
                            let null_arm = |this: &mut Self| {
                                let mut n = Value::Null;
                                this.convert(&mut n, &Type::Null, &base_text);
                                n
                            };
                            if matches!(code.unspan(), Value::Var(_)) {
                                // simple var — reading twice is side-effect-free
                                let mut is_not_null = code.clone();
                                self.convert(&mut is_not_null, &base_text, &Type::Boolean);
                                let mut concat = code.clone();
                                // KEEP parse_append_text's return type — it carries the fresh
                                // work-text's `frame1` dep, without which the text-return
                                // conversion sees an empty work-var list and returns null.
                                let ct =
                                    self.parse_append_text(&mut concat, &base_text, &ls, u16::MAX);
                                let n = null_arm(self);
                                *code = v_if(is_not_null, concat, n);
                                return Type::optional(ct);
                            }
                            // non-trivial left — materialise into a temp to avoid double-eval
                            let tmp = self.create_unique("_nlhs", &current_type);
                            let set_tmp = v_set(tmp, code.clone());
                            let mut is_not_null = Value::Var(tmp);
                            self.convert(&mut is_not_null, &base_text, &Type::Boolean);
                            let mut concat = Value::Var(tmp);
                            let ct = self.parse_append_text(&mut concat, &base_text, &ls, u16::MAX);
                            let opt = Type::optional(ct);
                            let n = null_arm(self);
                            let if_expr = v_if(is_not_null, concat, n);
                            *code = v_block(vec![set_tmp, if_expr], opt.clone(), "nullable_concat");
                            return opt;
                        }
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
            // @PLN102 — non-associative comparison guard (see `compared_at_this_level`).
            if matches!(operator, "==" | "!=" | "<" | "<=" | ">" | ">=") {
                if compared_at_this_level && self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "comparison operators do not chain — `{operator}` follows another \
                         comparison, which would compare a boolean to the next operand; \
                         parenthesise (e.g. `(a == b) == c`) or combine with `&&` \
                         (e.g. `a < b && b < c`)"
                    );
                }
                compared_at_this_level = true;
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
                // @PLN25 slice (c): peel `Optional` so a nullable LEFT operand of `+`
                // (`text?`) accumulates into the concat instead of falling to the generic
                // operator lookup (the "No matching operator '+' on 'text?'" reject).
                current_type.base().clone()
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
                    // @PLN85 (the chained field-of-call class) — a PROJECTION
                    // of the value must COPY into `w`, not alias: argument
                    // lifting later splits the inner call into a `__lift_N`
                    // whose scope-exit free runs INSIDE this block, so an
                    // aliasing `Set(w, GetField(call, ..))` left `w` (and
                    // every further projection) dangling into the freed
                    // store — chain depth ≥ 2 read stale data at ANY
                    // consumption site (bind / argument / return / loop) on
                    // BOTH backends; the depth-1 `materialized_view_return`
                    // path was already copy-then-free and is the spec this
                    // mirrors (the C86 escape rule: a view outliving its
                    // dying temporary materialises).  A direct CALL result
                    // (not a projection) still binds raw — `w` genuinely
                    // adopts that fresh store.
                    let is_projection = matches!(orig.unspan(), Value::Call(pd, _)
                        if matches!(self.data.def(*pd).name(),
                            "OpGetField" | "OpGetVector" | "OpVectorRef" | "OpGetDbRef"));
                    let kt = self.data.def(d_nr).known_type();
                    if is_projection && kt != u16::MAX {
                        let copy_d = self.data.def_nr("OpCopyRecord");
                        *code = v_block(
                            vec![
                                v_set(w, Value::Null),
                                self.cl("OpDatabase", &[Value::Var(w), Value::Int(i32::from(kt))]),
                                Value::Call(
                                    copy_d,
                                    vec![orig, Value::Var(w), Value::Int(i32::from(kt))],
                                ),
                                Value::Var(w),
                            ],
                            Type::Reference(d_nr, crate::data::Deps::frame1(w)),
                            "inline ref copy",
                        );
                    } else {
                        *code = v_block(
                            vec![v_set(w, orig), Value::Var(w)],
                            Type::Reference(d_nr, crate::data::Deps::frame1(w)),
                            "inline ref",
                        );
                    }
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
                if let Type::Function(param_types, ret_type, fn_deps) = t.clone() {
                    // @PLN85 t1 — does the SOURCE fn-ref OWN a freshly-minted
                    // closure?  A closure-factory return (`make_greeter(..)`) carries
                    // a `CalleeFrame` dep (the lambda's `___clos_N` work var); a
                    // borrowed pass-through (`passthru(g)`) or a non-capturing lambda
                    // returns EMPTY deps.  Owned ⇒ this immediately-invoked temp is
                    // the SOLE owner of the freshly-minted closure store (nobody else
                    // frees it), so it must KEEP the ownership dep and be freed at
                    // scope exit; borrowed ⇒ skip_free (the source var frees it, else
                    // a double-free).
                    // Does the SOURCE fn-ref OWN a freshly-minted closure?  A
                    // closure-factory return (`make_greeter(..)`) carries a
                    // `CalleeFrame` dep (the lambda's `___clos_N` work var); a
                    // borrowed pass-through (`passthru(g)`) or a non-capturing lambda
                    // returns EMPTY deps.  Pass 1's type does NOT yet carry the dep
                    // (the lambda propagation runs later), so this reads FALSE there —
                    // fine, because pass-1 IR is discarded and the stamp below is
                    // pass-2-only.
                    let owns_closure = fn_deps
                        .entries()
                        .any(|e| matches!(e, crate::data::DepEntry::CalleeFrame(_)));
                    let fn_type = Type::Function(
                        param_types.clone(),
                        ret_type.clone(),
                        if owns_closure {
                            fn_deps.clone()
                        } else {
                            crate::data::Deps::none()
                        },
                    );
                    // Allocate temp variable on BOTH passes (consistent unique counter).
                    let fn_work = self.create_unique("__fn_ref_tmp", &fn_type);
                    self.vars.defined(fn_work);
                    // @PLN85 t1 — stamp `skip_free` ONLY in pass 2 and ONLY for a
                    // BORROWED source.  `skip_free` is a GLOBAL per-var bit that
                    // persists across passes; a pass-1 stamp (where `owns_closure`
                    // reads FALSE for a fresh closure factory, its dep not yet
                    // propagated) poisons the pass-2 role — the counter-coupling
                    // hazard, cf. t3/p179.  An OWNED source (`make_greeter(..)(..)`)
                    // makes this temp the SOLE owner of the freshly-minted closure
                    // store, so it must be freed at scope exit (no stamp); a BORROWED
                    // source's closure DbRef aliases the source's store, so stamping
                    // avoids a double-free (and blocks the insert_free Return-wrap
                    // that would return `()` from a value block, P249-mirror).
                    if !self.first_pass && !owns_closure {
                        self.vars.set_skip_free(fn_work);
                    }
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

        // @PLN25 slice (b): a `??` discharges null, so its RESULT is the non-null base. Peel
        // any `Optional` from the LHS type here so the null-check builder + the result type
        // see the base (e.g. `Optional<text>` routes exactly as plain `text` did pre-marker).
        *ctp = ctp.base().clone();
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
            // @PLN25 (N-Store): `lhs ?? return ret` returns `ret` into the caller's
            // non-null return slot — an un-discharged nullable `ret` is a violation.
            self.n_store_violation(&t, &r_type, "the return value");
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
        // Thread the LHS BASE type as the default's hint when the context gives
        // none (`var_tp` unknown/null — e.g. a RETURN-TAIL `v[i] ?? []`, where
        // parse_return's `[`-led vector hint cannot fire).  Without it an EMPTY
        // vector default types Unknown and collapses to `null` (the P365 family),
        // which downstream classifies the ncc `if` as a null-arm shape — the
        // return delivery then skips materialisation and native emits `()`.
        let rhs_hint = if matches!(var_tp, Type::Unknown(_) | Type::Null) {
            lhs_type.base()
        } else {
            var_tp
        };
        let rhs_type = self.parse_operators(rhs_hint, &mut rhs, parent_tp, precedence + 1);
        self.known_var_or_type(&rhs, &rhs_pos);

        // The default may be a vector literal (`?? []`, `?? [99]`) that builds into
        // its OWN work-ref `_vec_N` — the last operator of the emitted `"Vector"`
        // block.  That work-ref's only `Set` lives in the else-branch block the
        // slot scan does not walk (the #319 unwalked-block gap — here for the
        // DEFAULT literal, not the subject temp handled below), so without help its
        // slot stays unassigned and codegen panics `Incorrect var _vec_N[65535]`.
        // Register it (function-entry preamble reserves the slot) and clear its
        // borrow-dep so the preamble owned-inits it as the owned vector it is.
        if let Value::Block(bl) = rhs.unspan()
            && bl.name == "Vector"
            && let Some(Value::Var(w)) = bl.operators.last().map(Value::unspan)
        {
            let w = *w;
            self.vars.register_work_ref(w);
            if let Type::Vector(elm, dep) = self.vars.tp(w).clone()
                && !dep.is_empty()
            {
                // An owned literal default borrows nothing — clear the frame-dep so
                // the preamble gate (dep-empty) fires AND `gen_set_first_vector_null`
                // takes the owned-vector alloc path.  `set_type` overwrites the dep;
                // `change_var_type` early-returns on `is_equal` (deps ignored) and
                // only ADDS deps, so it cannot clear.
                self.vars
                    .set_type(w, Type::Vector(elm, crate::data::Deps::none()));
            }
        }

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
        // A checked narrowing cast discharged here (`e as N ?? d` / `e as N? ?? d`):
        // the `??` removes the null, so the surviving value is provably in N's range.
        // Type the RESULT as the narrow `N` — the cast BLOCK stayed `integer` to hold
        // its null sentinel, but past the discharge a `u8` field / return / local can
        // accept it with no second cast — provided the default also fits `N`.
        let checked_narrow = self
            .dn4_checked_narrow
            .take()
            .filter(|nt| self.int_value_fits(&rhs, nt));
        let result_type = if let Some(Type::Integer(nspec)) = &checked_narrow {
            // The discharged value is provably in N's range, but both `??` branches are
            // full 8-byte integers (the cast block keeps its i64 null sentinel). So type
            // the result as a FULL-WIDTH integer (`forced_size: None`) carrying N's RANGE:
            // the range makes a later `u8` field / return / local accept it without a cast
            // (is_narrowing_int = false, exactly like an `integer limit(0,255)` value),
            // while the full width keeps the branches the same native type (no @P316 E0308).
            Type::Integer(IntegerSpec {
                forced_size: None,
                ..*nspec
            })
        } else if widen_ints {
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
            // @PLN102 `??` heap-ownership: distinguish an OWNED Vector subject (a
            // call result / comprehension — EMPTY deps, the value the `??` block
            // must free) from a BORROWED one (`vv[i]`, `s.field` — deps name the
            // source; freeing it is a use-after-free).  loft's core convention:
            // `dep.is_empty()` == owned.  Only the OWNED case migrates to the
            // view-model below; a BORROWED Vector keeps the original skip_free
            // hand-off (its store is owned elsewhere and must NOT be freed here).
            let owned_vector = matches!(lhs_type, Type::Vector(_, dep) if dep.is_empty());
            if matches!(
                lhs_type,
                Type::Text(_)
                    | Type::Reference(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Enum(_, true, _),
            ) || (matches!(lhs_type, Type::Vector(_, _)) && !owned_vector)
            {
                self.vars.set_skip_free(tmp);
            }
            // An OWNED Vector subject OWNS the value `code` produced (typically
            // `mkv()`'s returned store) and MUST be freed at scope exit —
            // `skip_free` would suppress that and every transient consumer
            // (return/append/method/discard) would leak the present-path store.
            // Mark `inline_ref` instead: the de-conflated "borrow the slot, don't
            // ALLOCATE in the preamble" bit (`gen_set_first_vector_null` emits a
            // null sentinel, NOT an empty-vector `OpDatabase` that `code` would
            // immediately orphan), while `get_free_vars` still emits
            // `OpFreeRef(__ncc_N)` (inline_ref is not skip_free).  Exactly the
            // `__lift_N` owns-an-inline-call pattern (`scopes.rs::new_lift_var`).
            // Consumers borrow via the block's view dep (added below), so the
            // adopting assign does not double-free.
            if owned_vector {
                self.vars.mark_inline_ref(tmp);
            }
            // #319 — heap-DbRef ncc temps need a function-entry `Set(tmp,
            // Null)` (the work-ref preamble) to reserve their stack slot:
            // their only Set lives inside the ncc block, which the Zone-2
            // slot scan does not walk.  Shapes whose assigned-var deps reach
            // the temp got a slot via scan_set's dep-prefix; a subject whose
            // dep chain is broken (e.g. a comprehension-built vector) did
            // not — "Incorrect var __ncc_N[65535]" at codegen.  (An OWNED Vector
            // uses the `inline_ref` relocation path in `expressions.rs` for the
            // same slot reservation, so it is intentionally excluded here; a
            // BORROWED Vector keeps the original work-ref registration.)
            if matches!(lhs_type, Type::Reference(_, _) | Type::Enum(_, true, _))
                || (matches!(lhs_type, Type::Vector(_, _)) && !owned_vector)
            {
                self.vars.register_work_ref(tmp);
            }
            let set_tmp = v_set(tmp, code.clone());
            let null_check = null_check_builder(self, &Value::Var(tmp));
            let mut true_branch = Value::Var(tmp);
            self.convert(&mut true_branch, lhs_type, &result_type);
            let if_expr = v_if(null_check, true_branch, rhs);
            // @PLN102 `??` Vector view-model (OWNED subject only): the subject
            // `__ncc_N` is a function-scope OWNER freed by `get_free_vars` (see
            // `inline_ref` above), exactly like the `if`/`match` view-model's
            // `__vdb_N`.  So the block RESULT is a dep-backed VIEW naming
            // `__ncc_N` (`vector<T>["__ncc_N"]`), NOT the bare owner: a consuming
            // assign `x = e ?? d` then makes `x` a BORROW (no second `OpFreeRef`
            // → no double-free), while every transient consumer borrows and never
            // frees.  The default arm keeps its own `__vdb_N` owner (freed
            // independently); the result names the present arm's owner, mirroring
            // `if`'s `["__vdb_1"]`.  A BORROWED Vector subject and every non-Vector
            // heap subject keep the skip_free hand-off, so their result stays the
            // bare owner.
            if owned_vector && let Type::Vector(elm, _) = &result_type {
                let view = Type::Vector(elm.clone(), crate::data::Deps::frame(vec![tmp]));
                *code = v_block(vec![set_tmp, if_expr], view.clone(), "ncc");
                *ctp = view;
                return;
            }
            *code = v_block(vec![set_tmp, if_expr], result_type.clone(), "ncc");
        }
        *ctp = result_type;
    }

    /// @PLN25 DN4 `(N-Cast?)` — lower `e as τ?` (narrowing, not provably-fit) to a
    /// range guard: bind `e` once, yield it if it lies in τ's range, else the
    /// integer null. Pure parse-time desugar over existing ops (`OpLeInt`,
    /// `OpConvIntFromNull`, `if`) — no new runtime op, no runtime error. `src_tp`
    /// is `e`'s type (the temp's slot); the result type is the nullable `τ`.
    pub(crate) fn dn4_checked_cast(&mut self, code: &mut Value, tp: &Type, src_tp: &Type) -> Type {
        let Type::Integer(spec) = tp else {
            return tp.clone();
        };
        let (min, max) = (spec.min, spec.max as i32);
        // Result type is `Optional(τ)` — the narrow target τ, marked nullable. As a Rust
        // VALUE this is carried full-width (`rust_type(Optional(Integer))` is `i64`, so the
        // else-branch's integer null sentinel fits and the value is not truncated); the
        // narrow width is a domain fact used at the store/field seam, which packs it. This
        // preserves τ so the checked cast composes downstream — e.g. `e as u8?` returned into
        // a `u8?` slot is an exact type match, not an implicit narrow of a bare integer.
        // (Earlier this dropped to the full source integer to dodge a native `as u8` on the
        // tail that turned `i64::MIN` into 0 and destroyed the null; that hazard is now fixed
        // at the `rust_type` seam, so the narrow τ is preserved.)
        let res_tp = Type::optional(tp.clone());
        let tmp = self.create_unique("_dn4", src_tp);
        let set = v_set(tmp, code.clone());
        let cond_lo = self.cl("OpLeInt", &[Value::Int(min), Value::Var(tmp)]);
        let cond_hi = self.cl("OpLeInt", &[Value::Var(tmp), Value::Int(max)]);
        let null_lo = self.cl("OpConvIntFromNull", &[]);
        let null_hi = self.cl("OpConvIntFromNull", &[]);
        let inner = v_if(cond_hi, Value::Var(tmp), null_hi);
        let outer = v_if(cond_lo, inner, null_lo);
        *code = v_block(vec![set, outer], res_tp.clone(), "dn4cast");
        res_tp
    }

    /// The constant integer value of `code`, if it is one (literal or const-foldable).
    pub(crate) fn const_int(&self, code: &Value) -> Option<i64> {
        match code.unspan() {
            Value::Int(n) => Some(i64::from(*n)),
            Value::Long(n) => Some(*n),
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Int(n)) => Some(i64::from(n)),
                Some(Value::Long(n)) => Some(n),
                _ => None,
            },
        }
    }

    /// If the operand is provably in `[0, M]` — a non-negative constant, or an
    /// integer type with `min >= 0` — return `M`. Used by `&` range-tracking.
    fn nonneg_bound(&self, ty: &Type, code: &Value) -> Option<u32> {
        if let Some(c) = self.const_int(code)
            && (0..=i64::from(u32::MAX)).contains(&c)
        {
            return Some(u32::try_from(c).unwrap_or(u32::MAX));
        }
        if let Type::Integer(s) = ty
            && s.min >= 0
        {
            return Some(s.max);
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    /// @PLN25 DN3 — is a division/mod DIVISOR provably non-zero (so `a / v` cannot fault)?
    /// True for a constant non-zero literal, a var proven non-zero by an enclosing `if v != 0`
    /// guard (`divisor_nonzero`), or (@PLN102) any expression that const-folds to a non-zero number
    /// — a named constant like `SFX_RATE`, or a cast of one like `RATE as single` (both inline to a
    /// literal that `const_eval` reduces). This closes the gap where a direct `x / 600.0` was
    /// non-null but `x / NAMED_CONST` spuriously typed `τ?`. Anything else can be zero → `τ?`.
    fn divisor_provably_nonzero(&self, divisor: &Value) -> bool {
        match divisor.unspan() {
            Value::Int(n) => *n != 0,
            Value::Long(n) => *n != 0,
            // @PLN102 Phase 3 — a constant non-zero float / single divisor is provably safe too
            // (`1.0 / 2.0` stays non-null). Only consulted for float `/` under LOFT_NULLFLOW; for
            // integer division these arms never match, so the default path is unchanged.
            Value::Float(f) => *f != 0.0,
            Value::Single(f) => *f != 0.0,
            Value::Var(v) => self.divisor_nonzero.contains(v),
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Int(n)) => n != 0,
                Some(Value::Long(n)) => n != 0,
                Some(Value::Float(f)) => f != 0.0,
                Some(Value::Single(f)) => f != 0.0,
                _ => false,
            },
        }
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
        // The checked-narrow target is only for a `??` that DIRECTLY discharges the
        // cast (`e as N ?? d`).  Any other operator between the cast and a later `??`
        // invalidates it — clear so a stale target can't mis-narrow an unrelated `??`.
        if operator != "??" {
            self.dn4_checked_narrow = None;
        }
        // @F2 — ?? null-coalescing operator (incl. `?? return`)
        if operator == "??" {
            self.handle_null_coalesce(var_tp, code, parent_tp, precedence, ctp);
        // @F5 — type conversions: explicit `as` cast (+ implicit / format-only OpConv*)
        } else if operator == "as" {
            self.expr_not_null = false;
            if let Some(tps) = self.lexer.has_identifier() {
                // Parse the target WITHOUT the postfix-`?` consumer, then detect the
                // `?` here — `as τ` and `as τ?` are otherwise indistinguishable (the
                // narrow aliases all carry `not_null:false`).
                let Some(mut tp) = self.parse_type_inner(u32::MAX, &tps, false) else {
                    diagnostic!(self.lexer, Level::Error, "Expect type");
                    return Some(Type::Null);
                };
                let nullable_cast = self.lexer.has_token("?");
                // @PLN25 DN4/DN5 — a scalar cast target has a DOMAIN: its integer value
                // RANGE, and (when it is a plain non-null scalar) that domain EXCLUDES null.
                // A value fits `as τ` (no `?`) only if it lies in the domain on BOTH
                // dimensions.  `null` is simply the reserved out-of-domain element — not a
                // special case — so the range fit (DN4) and the nullness fit (DN5) are ONE
                // containment test.  Peel `Optional` first so the RANGE check sees the
                // underlying integer: a nullable source otherwise slips past
                // `is_narrowing_int` entirely, laundering BOTH dimensions
                // (`integer? as u8` skipped the range check AND the null check).
                //
                // @PLAN48 P2: an explicit `as <narrow-int>` is the sanctioned way to narrow
                // `integer` → `i32`/`u8`/…, so the implicit-narrowing diagnostic in `convert`
                // does NOT fire on an explicit cast.  `as τ?` is the single CHECKED form: it
                // lowers to a range guard yielding the value or the null sentinel, and (below)
                // types as `Optional<τ>` so the laundering stays closed downstream.
                let src_base = ctp.base().clone();
                let src_may_be_null = matches!(ctp, Type::Optional(_) | Type::Null);
                let narrowing = Self::is_narrowing_int(&src_base, &tp);
                if !self.first_pass
                    && crate::keys::pln25_dn1_enabled()
                    && src_may_be_null
                    && !nullable_cast
                    && Self::is_non_null_scalar(&tp)
                {
                    // DN5 — the nullness dimension: a possibly-null value cast to a non-null
                    // scalar.  (Heap targets stay nullable, so `null as SomeStruct` is legal
                    // and never reaches here — `is_non_null_scalar` is scalar-only.)
                    if matches!(ctp, Type::Null) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "cannot cast `null` to the non-null `{tps}` — cast to `{tps}?` \
                             (a typed null), or use a real value",
                        );
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "cannot cast a possibly-null `{}` to the non-null `{tps}` — it may \
                             be null; use `as {tps}?` for a checked cast (value or null), or \
                             discharge first with `?? <default>`",
                            ctp.name(&self.data),
                        );
                    }
                    // Keep `tp` (the non-null target) as the result to bound the cascade.
                } else if narrowing {
                    // @PLN25 DN4 (F5 cutover — UNCONDITIONAL, no opt-out): a narrowing cast
                    // whose value is NOT provably in range is a fit violation, exactly like
                    // DN5's nullness dimension (its sibling, also unconditional). The old
                    // `LOFT_NO_DN4` escape reverted to a SILENT 8-byte width-tag
                    // (`400 as u8 == 400`) — the very truncation the model eliminates — so it
                    // is retired. `as τ` errors (use `as τ?`); `as τ?` is the checked cast.
                    if !self.first_pass && !self.int_value_fits(code, &tp) {
                        // A bare `e as N` immediately left of `??` is a CHECKED cast: the
                        // `??` supplies the out-of-range fallback, so treat it like `e as N?`
                        // rather than rejecting it (the diagnostic already advertised `?? d`).
                        let coalesced = !nullable_cast && self.lexer.peek_token("??");
                        if nullable_cast || coalesced {
                            // Remember the narrow target so the discharging `??` types its
                            // result as `N` (build_null_coalesce_default); the cast BLOCK
                            // itself stays `integer` to keep the null sentinel at full width.
                            self.dn4_checked_narrow = Some(tp.clone());
                            tp = self.dn4_checked_cast(code, &tp, &src_base);
                        } else {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "narrowing cast from {} to {tps} may not fit at runtime; \
                                 use `{tps}?` for a checked cast (value or null), or guard \
                                 the value (`?? d`, mask, or an `if` range check)",
                                self.int_type_name(&src_base),
                            );
                        }
                    }
                    // else: in range, or DN4 off — accept as a width-tag (value
                    // stays in the 8-byte slot), the pre-DN4 behaviour.
                } else {
                    // @PLN99 Arc C — clear the owned-conversion signal, then let `convert`
                    // set it iff it dispatches an allocating user conversion (`fn OpConvTFromS`).
                    self.conv_owned_result = None;
                    // @PLN102 — a nullable SCALAR source (`float?`/`single?`/`integer?`) casts by its
                    // BASE: the null rides in-band (NaN is the float null, C90; the reserved sentinel
                    // for integers) and the cast op propagates it (`NaN as integer` = null), so the
                    // per-type `OpCast` is keyed on the base, not the `Optional`. Peel it here so the
                    // checked `float? as integer?` resolves instead of reporting "Unknown cast"; the
                    // result re-wraps `τ?` below (nullable_cast). A BARE `float? as integer` never
                    // reaches this arm — DN5 rejects it above — so this only enables the checked form.
                    let cast_src: &Type = if src_may_be_null && Self::is_non_null_scalar(&src_base)
                    {
                        &src_base
                    } else {
                        ctp
                    };
                    if !self.convert(code, cast_src, &tp) && !self.cast(code, cast_src, &tp) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown cast from {} to {tps}",
                            &ctp.name(&self.data),
                        );
                    }
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
                // @PLN25 — `as τ?` yields `Optional<τ>`: the checked form's domain includes
                // null, so downstream sees a nullable and `(N-Store)` keeps the hole closed
                // (a later `z: τ = (e as τ?)` still requires a discharge).  `base()` guards
                // against a double-wrap from `dn4_checked_cast`.
                if (nullable_cast || self.dn4_checked_narrow.is_some())
                    && crate::keys::pln25_dn1_enabled()
                    && Self::is_non_null_scalar(rt.base())
                {
                    rt = Type::optional(rt.base().clone());
                }
                // A text→numeric PARSE is fit-failing: unparseable input yields the null sentinel
                // at runtime (C80, no trap). Two regimes:
                //  · @PLN25 DN3 (default / OFF): the result auto-TYPES `τ?` (`s as integer` is
                //    `integer?`; discharge with `?? d` or a `τ?` slot).
                //  · @PLN102 Phase 4 (N-Cast, LOFT_NULLFLOW): a cast `as τ` is an ASSERTION, and a
                //    parse can't be proven, so a BARE `text as τ` is a compile error directing to
                //    the checked `as τ?` (value or null) or the assert-or-default `as τ ?? d`. A
                //    `?? d` immediately after IS that form — allowed, and typed `τ?` so the `??`
                //    discharges it (mirrors the DN4 narrowing `coalesced` rule above).
                if !nullable_cast
                    && crate::keys::pln25_dn1_enabled()
                    && matches!(ctp, Type::Text(_))
                    && matches!(rt.base(), Type::Integer(_) | Type::Float | Type::Single)
                {
                    if crate::keys::nullflow_enabled() && !self.lexer.peek_token("??") {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "a text parse `as {tps}` may fail — use `{tps}?` for a checked cast \
                                 (value or null), or `?? <default>`",
                            );
                        }
                        // Keep `rt` non-null (the asserted target) to bound the cascade.
                    } else {
                        rt = Type::optional(rt.base().clone());
                    }
                }
                if let Some(owned) = self.conv_owned_result.take() {
                    // @PLN99 Arc C — an allocating user conversion produced a FRESH owned
                    // store.  Use the conversion fn's real return type (Owned deps) and do
                    // NOT graft the source's deps: the result is not a view of the source, so
                    // grafting would mark it a borrow and leak the new store at scope end.
                    rt = owned;
                } else {
                    for d in ctp.depend() {
                        rt = rt.depending(d);
                    }
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
            // Match on the BASE type so a nullable vector (`vector<u8>? == null`, e.g.
            // a `read_bytes`/`list_dir` result — @PLN102 H4) is caught, not only a bare
            // `Type::Vector` — the same @PLN99 A5 gap the ref/enum null cases fixed.
            let vec_null = (operator == "==" || operator == "!=")
                && ((matches!(ctp.base(), Type::Vector(_, _)) && second_type == Type::Null)
                    || (*ctp == Type::Null && matches!(second_type.base(), Type::Vector(_, _))));
            // A float/single null is the NaN sentinel, and NaN compares unequal to
            // everything (including itself), so `f == null` can't go through OpEq —
            // it would always be false.  Test validity instead: convert(float, bool)
            // is `!is_nan` (= non-null), so `== null` is its negation.
            // @PLN25: peel `Optional(τ)` — a `float?`/`single?` shares its base's NaN-sentinel
            // null. Without the peel `float? == null` skipped this validity test and fell to
            // OpEqFloat on the NaN sentinel (NaN != everything) → ALWAYS false (a null read as
            // "present"). `Type::Null` itself is never wrapped, so the Null sides stay bare.
            let float_null = (operator == "==" || operator == "!=")
                && ((matches!(ctp.base(), Type::Float | Type::Single)
                    && second_type == Type::Null)
                    || (*ctp == Type::Null
                        && matches!(second_type.base(), Type::Float | Type::Single)));
            // A nullable enum variable holds the reference null sentinel
            // (`store_nr==u16::MAX`), exactly like a struct reference.  Its `== null`
            // must test that sentinel via OpEqRef — the default path reads the
            // discriminant (`OpGetEnum`), which derefs the absent record and OOB-crashes.
            let enum_null = (operator == "==" || operator == "!=")
                && ((matches!(*ctp, Type::Enum(_, _, _)) && second_type == Type::Null)
                    || (*ctp == Type::Null && matches!(second_type, Type::Enum(_, _, _))));
            // @PLN99 A5 — a nullable STRUCT-reference VARIABLE (`s: DT? = null`,
            // typed `Optional(Reference)`) holds the reference null sentinel
            // (`store_nr==u16::MAX`, from OpNullRefSentinel), like a nullable enum variable.
            // Its `== null` must test that sentinel via OpRefIsNull.  Without this case it fell
            // to the generic `==`, which lowered to
            // `OpEqBool(OpConvBoolFromRef(s), OpConvBoolFromNull())` — but OpConvBoolFromRef is
            // `rec != 0` (is-NON-null, 0/1) while OpConvBoolFromNull is the 255 bool sentinel,
            // so the two NEVER compare equal → `s == null` was ALWAYS false (a live `main`
            // bug: `??` and `{s}` saw null but `== null` disagreed).
            // Gate on `Optional`, NOT bare `Reference`: a hash/collection LOOKUP result is a
            // bare `Type::Reference` whose miss is `rec==0` (not the store_nr sentinel) and is
            // handled correctly by the existing path — OpRefIsNull would misread it (p285).
            // INLINE nullable struct fields are `Type::Enum(__nullable<…>)` (enum_null path).
            let opt_ref_l =
                matches!(*ctp, Type::Optional(_)) && matches!(ctp.base(), Type::Reference(_, _));
            let opt_ref_r = matches!(second_type, Type::Optional(_))
                && matches!(second_type.base(), Type::Reference(_, _));
            let ref_null = (operator == "==" || operator == "!=")
                && ((opt_ref_l && second_type == Type::Null) || (*ctp == Type::Null && opt_ref_r));
            // @PLN102 pre-freeze — a boolean and an integer are NOT comparable with `==`/`!=`.
            // The old path coerced the integer to boolean by "is non-null", so `true == 0` was
            // TRUE and `true == 2` was TRUE (nonsense) — while `bool < int` already errored.
            // Reject `==`/`!=` too, so the whole comparison family is consistent.  (`null` is
            // Type::Null, not Integer, so `bool == null` / `bool? == null` are unaffected.)
            if !self.first_pass
                && matches!(operator, "==" | "!=")
                && ((matches!(ctp.base(), Type::Boolean)
                    && matches!(second_type.base(), Type::Integer(_)))
                    || (matches!(ctp.base(), Type::Integer(_))
                        && matches!(second_type.base(), Type::Boolean)))
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot compare a boolean and an integer with `{operator}` — a boolean is \
                     true/false/null, not 0/1; they are different types (convert one explicitly \
                     if that is really what you mean)"
                );
            }
            if vec_null {
                // @PLN25: `vector == null` / `vector != null` tests the null
                // sentinel (store_nr == u16::MAX) via OpVectorIsNull — NOT eq_ref,
                // whose rec==0 null test would also match an empty `[]`.
                if !self.first_pass {
                    let (vec_code, vec_tp) = if matches!(ctp.base(), Type::Vector(_, _)) {
                        (code.clone(), ctp.clone())
                    } else {
                        (second_code, second_type.clone())
                    };
                    // A heap-owning vector TEMP (a call result / non-var) consumed by the
                    // null test would be orphaned: OpVectorIsNull reads only the sentinel and
                    // discards the vector store. Capture it in a work-ref so scopes.rs emits
                    // its OpFreeRef (the nullable-vector-return-consumed-inline leak — a plain
                    // `f() != null` where `f() -> vector<T>?`). A Var operand already owns its
                    // store, so it needs no work-ref.
                    let w = if matches!(vec_code.unspan(), Value::Var(_)) {
                        u16::MAX
                    } else {
                        self.vars.work_refs(&vec_tp, &mut self.lexer)
                    };
                    let is_null = if w == u16::MAX {
                        self.cl("OpVectorIsNull", &[vec_code])
                    } else {
                        self.vars.mark_inline_ref(w);
                        crate::data::v_block(
                            vec![
                                crate::data::v_set(w, vec_code),
                                self.cl("OpVectorIsNull", &[Value::Var(w)]),
                            ],
                            Type::Boolean,
                            "vec-null-workref",
                        )
                    };
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
                    // A VALUE enum (`Color`, no payload variants) is a disc byte whose
                    // null is discriminant 0 — test `OpConvIntFromEnum(v) == 0`, NOT
                    // OpRefIsNull (its store_nr sentinel test on a u8 field-read →
                    // native E0610 `.store_nr` on a primitive).  Read the value/reference
                    // shape from the enum DEFINITION's `returned` type: the ACCESS type
                    // is polluted — a field read of a simple enum comes back
                    // `Enum(_, true, _)` even though the enum itself is a value enum.
                    // Only a genuine struct-enum (`returned` = `Enum(_, true, _)`) is a
                    // DbRef and keeps OpRefIsNull.
                    let value_enum = e_def != u32::MAX
                        && matches!(self.data.def(e_def).returned, Type::Enum(_, false, _));
                    let is_null = if inline {
                        let get_enum = self.cl("OpGetEnum", &[e_code, Value::Int(0)]);
                        let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
                        self.cl("OpEqInt", &[disc, Value::Int(0)])
                    } else if value_enum {
                        // `e_code` is the disc byte already (a field read / a value var);
                        // disc 0 is the absent variant.
                        let disc = self.cl("OpConvIntFromEnum", &[e_code]);
                        self.cl("OpEqInt", &[disc, Value::Int(0)])
                    } else {
                        // A struct-enum reference: null IS the store_nr sentinel
                        // (OpRefIsNull), NOT OpEqRef's rec==0 (a present enum is
                        // inline-represented on native and carries rec==0).
                        self.cl("OpRefIsNull", &[e_code])
                    };
                    *code = if operator == "==" {
                        is_null
                    } else {
                        self.cl("OpNot", &[is_null])
                    };
                }
                *ctp = Type::Boolean;
            } else if ref_null {
                if !self.first_pass {
                    let r_code = if *ctp == Type::Null {
                        second_code
                    } else {
                        code.clone()
                    };
                    // A struct reference variable is a DbRef whose null IS the store_nr
                    // sentinel (OpRefIsNull), NOT rec==0 (a present record can carry rec==0).
                    let is_null = self.cl("OpRefIsNull", &[r_code]);
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
            // `**` is RIGHT-associative — `2 ** 3 ** 2` is `2 ** (3 ** 2)` = 512, matching
            // maths and most languages, so the maker never carries a surprise here.  Its RHS
            // therefore parses at the SAME precedence (it recurses into a following `**`),
            // where every other binary operator is left-associative (RHS at `precedence + 1`).
            let rhs_precedence = if operator == "**" {
                precedence
            } else {
                precedence + 1
            };
            let second_type =
                self.parse_operators(var_tp, &mut second_code, parent_tp, rhs_precedence);
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
            // @PLN25 (N-Arith) range-tracking — capture the operand bounds BEFORE
            // call_op consumes them, so the result range of `&`/`%` can be narrowed
            // (a masked/modded value becomes provably-fit for a later narrowing
            // cast, removing DN4's `(x & 255) as u8` friction).
            let (and_bound, mod_const, lhs_nonneg) = if matches!(operator, "&" | "%") {
                let and_bound = [
                    self.nonneg_bound(ctp, code),
                    self.nonneg_bound(&second_type, &second_code),
                ]
                .into_iter()
                .flatten()
                .min();
                let lhs_nonneg = matches!(ctp, Type::Integer(s) if s.min >= 0);
                (and_bound, self.const_int(&second_code), lhs_nonneg)
            } else {
                (None, None, false)
            };
            // @PLN25 DN3: integer `/` and `%` PRODUCE null at runtime on a zero divisor (the C80
            // sentinel), so the RESULT types `τ?` UNLESS the divisor is provably non-zero — a
            // constant non-zero literal (`a / 2`) or a var proven non-zero by an `if v != 0` guard
            // (the `divisor_nonzero` narrowing stack). Captured HERE, before `call_op` consumes
            // `second_code`; the `τ?` wrap is applied after the range-narrowing below. DN1-gated.
            // Makes the type carry the null so `(N-Store)` forces `?? d` / a `τ?` slot at the store.
            // @PLN102 Phase 3 (N-Domain) — float / single `/` and `%` also fault on a zero
            // divisor, so they type `τ?` too (like integer `/`). Gated on LOFT_NULLFLOW; the
            // `divisor_provably_nonzero` elision keeps `x / 2.0` non-null. Integer stays default-on.
            let div_nullable = (operator == "/" || operator == "%")
                && (matches!(ctp.base(), Type::Integer(_))
                    || (crate::keys::nullflow_enabled()
                        && matches!(ctp.base(), Type::Float | Type::Single)))
                && !self.divisor_provably_nonzero(&second_code);
            // @PLN102 (N-Prop) Phase 2 — capture operand nullability BEFORE `call_op` consumes
            // the operand types. A nullable operand PROPAGATES through a value-preserving scalar
            // op: the runtime already carries the null sentinel / NaN through (`n+5`, `5-n`,
            // `abs(n)` on a null n all stay null). Gated on LOFT_NULLFLOW; the result is wrapped
            // `τ?` below (beside the div wrap). C85 stays the complement — non-null operands are
            // untouched here, so overflow of two non-null values still types non-null.
            let operand_nullable = crate::keys::nullflow_enabled()
                && (matches!(*ctp, Type::Optional(_)) || matches!(second_type, Type::Optional(_)));
            *ctp = self.call_op(
                code,
                operator,
                &[code.clone(), second_code],
                &[ctp.clone(), second_type],
            );
            // Tighten the result range — sound + conservative (never narrower than
            // the operation guarantees), and only ever a tightening of call_op's
            // range. `a & c` (c ≥ 0) ∈ [0, c]; `a % c` ∈ [-(|c|-1), |c|-1], or
            // [0, |c|-1] when `a` is non-negative.
            if let Type::Integer(s) = &*ctp {
                let narrowed = match operator {
                    "&" => and_bound.map(|m| (0i32, m)),
                    "%" => mod_const.filter(|c| *c != 0).map(|c| {
                        let m =
                            u32::try_from(c.unsigned_abs().saturating_sub(1)).unwrap_or(u32::MAX);
                        let lo = if lhs_nonneg {
                            0
                        } else {
                            -i32::try_from(m).unwrap_or(i32::MAX)
                        };
                        (lo, m)
                    }),
                    _ => None,
                };
                if let Some((nmin, nmax)) = narrowed
                    && nmin >= s.min
                    && nmax <= s.max
                {
                    *ctp = Type::Integer(IntegerSpec {
                        min: nmin,
                        max: nmax,
                        ..*s
                    });
                }
            }
            if div_nullable && crate::keys::pln25_dn1_enabled() {
                *ctp = Type::optional(ctp.clone());
            } else if operand_nullable
                && !matches!(*ctp, Type::Optional(_))
                && matches!(ctp.base(), Type::Integer(_) | Type::Float | Type::Single)
                && matches!(
                    operator,
                    "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>"
                )
            {
                // @PLN102 (N-Prop): a nullable operand rode into a value-preserving scalar op —
                // the result is nullable too (the null value already flows through at runtime).
                *ctp = Type::optional(ctp.clone());
            }
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
    /// `(idx_var, vec)` pairs proven safe by an enclosing
    /// `if idx_var < len(vec) { ... }` guard, where `vec` is a `VecKey`
    /// (a bare var OR a struct field read).  Pushed on entry to the
    /// `then` block, popped on exit.  Skip pattern 5.
    guarded_pairs: Vec<(u16, VecKey)>,
    /// Map from local-var slot → vec `VecKey` when the local was bound
    /// via `n = len(vec)`.  Skip pattern 5 with `if i < n { v[i] }` then
    /// becomes equivalent to `if i < len(v) { v[i] }` via the lookup.
    /// Populated when walking `Value::Set(local, Call(len, [vec]))`.
    len_captures: std::collections::HashMap<u16, VecKey>,
    /// Position of the innermost enclosing `Value::Span` — used as the
    /// fault site's source location when we emit a warning.
    last_pos: Option<Position>,
    /// @PLN46 W2 — true while walking an argument that is passed DIRECTLY to a
    /// `#null_safe` function: a fault-prone op that IS this argument tolerates
    /// null by the callee's contract, so it is not flagged.  Reset (to the called
    /// function's own null-safety) on entry to each nested call, so the
    /// suppression never reaches a fault op buried inside a non-null-safe callee.
    arg_to_null_safe_param: bool,
}

#[derive(Copy, Clone)]
enum FaultKind {
    Div,
    Rem,
    VectorIndex,
    TextIndex,
}

/// Identity of a "vector" a bounds guard can prove safe.  Either a bare
/// local/parameter (`Var`) or a struct field read
/// `OpGetField(Var(base), off, tp)`.  Skip-pattern 5 matches the guard's
/// vec against the indexing's vec by this key, so
/// `if i < len(self.v) { self.v[i] }` is proven exactly like the
/// bare-local `if i < len(loc) { loc[i] }` form.  (A struct vector-field
/// read COPIES post-@PLN85 #415, so the old `loc = self.v` alias trick is
/// gone and value-correct code indexes the field directly.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum VecKey {
    Var(u16),
    Field(u16, i32, i32),
}

/// `VecKey` of an expression used as a vector — the indexing's first arg
/// or a `len(...)` arg.  `Some` for a bare `Var` or an
/// `OpGetField(Var(base), Int(off), Int(tp))`; `None` otherwise.
pub(crate) fn vec_key(v: &Value, data: &Data) -> Option<VecKey> {
    match v.unspan() {
        Value::Var(n) => Some(VecKey::Var(*n)),
        Value::Call(def_nr, args) => {
            if data.def(*def_nr).original_name() != "GetField" || args.len() != 3 {
                return None;
            }
            let Value::Var(base) = args[0].unspan() else {
                return None;
            };
            let (Value::Int(off), Value::Int(tp)) = (args[1].unspan(), args[2].unspan()) else {
                return None;
            };
            Some(VecKey::Field(*base, *off, *tp))
        }
        _ => None,
    }
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

    /// @PLN87 P3 (W4) — flag a `&` on a HEAP struct parameter that the body never
    /// REASSIGNS.  For a heap param, field mutation (`o.x = 9`) already propagates
    /// to the caller (reference-default); `&` only changes a WHOLE-BINDING
    /// reassignment (`o = X`) into a write-back.  So a `&Obj` that is only
    /// field-mutated or read gains nothing from the `&` — and it is not free: a
    /// `&`-reference is a DOUBLE-INDIRECT `RefVar` (every access dereferences twice),
    /// materially slower than a plain heap binding that aliases directly.  So the
    /// redundant `&` is a silent efficiency footgun, not just a style nit — hence ON
    /// by default.  Scalar `&` (always load-bearing) and never-read params (already
    /// covered by `test_used`) are not flagged.  Stdlib exempt; silenceable via
    /// `LOFT_NO_WARN_RUNTIME`.
    pub(crate) fn warn_redundant_amp(&mut self, body: &Value) {
        if self.default || self.context == u32::MAX {
            return;
        }
        // ON by default (P3.2): the redundant `&` costs real runtime (double-indirect
        // RefVar access), so users need to see it.  Silenceable with the runtime-warning
        // family switch when a `&` is kept deliberately (e.g. a codegen regression test
        // that exercises the RefVar path).
        if std::env::var("LOFT_NO_WARN_RUNTIME").is_ok_and(|v| v == "1" || v == "true") {
            return;
        }
        let attrs = self.data.def(self.context).attributes().to_vec();
        for a in &attrs {
            // `self` is the method-receiver convention — `&self` is idiomatic and
            // not the "&Object to mutate" misconception this lint targets, so it is
            // never flagged even when only field-mutated.  Hidden synthetic params
            // (return buffers) are not user `&` either.
            if a.hidden || a.name == "self" {
                continue;
            }
            let var = self.vars.var(&a.name);
            if var == u16::MAX
                || self.vars.uses(var) == 0
                || !matches!(self.vars.tp(var), Type::RefVar(inner) if matches!(**inner, Type::Reference(_, _)))
            {
                continue;
            }
            let mut reassigned = false;
            body.walk(&mut |node| {
                if matches!(node, Value::Set(v, _) if *v == var) {
                    reassigned = true;
                }
            });
            if reassigned {
                continue;
            }
            self.lexer.to(self.vars.var_source(var));
            diagnostic!(
                self.lexer,
                Level::Warning,
                "`&` on parameter `{}` only slows it down here — a `&`-reference is \
                 double-indirect (slower on every access), and field mutation already \
                 propagates to the caller without it. Drop the `&` unless you REASSIGN \
                 the whole binding (`{} = …`), which is the one thing `&` is for.",
                a.name,
                a.name,
            );
        }
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
                // @PLN46 W2 — arguments to a `#null_safe` function tolerate null;
                // a fault op that IS such an argument is suppressed.  Override the
                // flag to THIS callee's null-safety so a nested non-null-safe call
                // resets it (the suppression is direct-argument only).
                let saved = ctx.arg_to_null_safe_param;
                ctx.arg_to_null_safe_param = self.data.def(*def_nr).null_safe();
                for arg in args {
                    self.walk_for_warnings(arg, ctx);
                }
                ctx.arg_to_null_safe_param = saved;
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
        // @PLN25 DN3: the fault ops now TYPE `τ?` — the type carries the null and `(N-Store)`
        // forces discharge, so the runtime-null warning is redundant under DN1 (and fired
        // inconsistently vs the `if b != 0` / `if i < len` narrowing). Division/mod flipped
        // first; the index ops (`v[i]`/`s[i]`) are now flipped too (F1a landed), so retire all four.
        if crate::keys::pln25_dn1_enabled()
            && matches!(
                kind,
                FaultKind::Div | FaultKind::Rem | FaultKind::VectorIndex | FaultKind::TextIndex
            )
        {
            return;
        }
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

    /// @PLN46 W3 — auto-set `#null_safe` (function-wide) when EVERY parameter is
    /// entry-guarded by a leading `if p == null { return … }`.  SOUND: an
    /// unguarded parameter leaves the flag unset, so its null is never silently
    /// suppressed at call sites — the plan's "explicit assertion only" rule, here
    /// the explicit guard.  Only SETS the flag, so a `#null_safe` annotation (W2)
    /// is preserved.  Runs on pass 2 after the body, so a forward-defined callee's
    /// flag is set before a caller's warning walk.
    pub(crate) fn infer_function_null_safe(&mut self, body: &Value) {
        if self.first_pass || self.data.def(self.context).null_safe() {
            return;
        }
        let params = self.vars.arguments();
        if params.is_empty() {
            return;
        }
        let guarded = leading_null_guards(&self.data, body);
        if params.iter().all(|p| guarded.contains(p)) {
            self.data.definitions[self.context as usize].null_safe = true;
        }
    }
}

/// @PLN46 W3 — the parameter slots guarded by LEADING `if p == null { return … }`
/// statements (line markers skipped; the first real non-guard ends the entry
/// region, so a recognised guard always precedes any use of its parameter).
fn leading_null_guards(data: &Data, body: &Value) -> std::collections::HashSet<u16> {
    let mut guarded = std::collections::HashSet::new();
    let Value::Block(b) = body.unspan() else {
        return guarded;
    };
    for stmt in &b.operators {
        if let Some(p) = null_guard_param(data, stmt) {
            guarded.insert(p);
        } else if !matches!(stmt.unspan(), Value::Line(_) | Value::Null) {
            break; // first real non-guard ends the entry region (markers are skipped)
        }
    }
    guarded
}

/// The parameter an entry guard `if p == null { return … }` protects, if `stmt`
/// is exactly that shape: an `OpEq*` of `Var(p)` against `null` (either operand
/// order), whose `then` branch returns.
fn null_guard_param(data: &Data, stmt: &Value) -> Option<u16> {
    let Value::If(cond, then, _) = stmt.unspan() else {
        return None;
    };
    if !contains_return(then) {
        return None;
    }
    let Value::Call(op, args) = cond.unspan() else {
        return None;
    };
    if args.len() != 2 || !data.def(*op).name().starts_with("OpEq") {
        return None;
    }
    // A comparison may convert the operand first — e.g. a `character` is compared
    // as int: `OpConvIntFromCharacter(Var(c))`.  Look through one `OpConv*` layer.
    let var_of = |v: &Value| -> Option<u16> {
        match v.unspan() {
            Value::Var(p) => Some(*p),
            Value::Call(c, a) if a.len() == 1 && data.def(*c).name().starts_with("OpConv") => {
                match a[0].unspan() {
                    Value::Var(p) => Some(*p),
                    _ => None,
                }
            }
            _ => None,
        }
    };
    if let Some(p) = var_of(&args[0])
        && is_null_literal(data, &args[1])
    {
        return Some(p);
    }
    if let Some(p) = var_of(&args[1])
        && is_null_literal(data, &args[0])
    {
        return Some(p);
    }
    None
}

/// `null` as it appears in a comparison: a bare `Null`, or a `…FromNull()` cast
/// (the parser converts `null` to the compared type, e.g. `OpConvTextFromNull()`).
fn is_null_literal(data: &Data, v: &Value) -> bool {
    match v.unspan() {
        Value::Null => true,
        Value::Call(op, args) if args.is_empty() => data.def(*op).name().ends_with("FromNull"),
        _ => false,
    }
}

/// Does `v` contain a `return`?  The guard must EXIT on null, not fall through to
/// a use of the (still-possibly-null) parameter.
fn contains_return(v: &Value) -> bool {
    let mut found = false;
    v.walk(&mut |n| {
        if matches!(n, Value::Return(_)) {
            found = true;
        }
    });
    found
}

/// Evaluate the easy-proof skip list against a fault-prone call's args.
/// Returns `true` when a skip pattern matches and the warning should
/// NOT fire.
fn is_easy_proof(kind: FaultKind, args: &[Value], ctx: &WarnCtx, data: &Data) -> bool {
    // @PLN46 W2 — this fault op is a direct argument to a `#null_safe` function,
    // which contracts to handle the possible null.  Skip pattern 6.
    if ctx.arg_to_null_safe_param {
        return true;
    }
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
                && let Some(vk) = vec_key(first, data)
            {
                let unwrapped = unwrap_cond(idx, data);
                if let Value::Var(idx_var) = unwrapped
                    && ctx
                        .guarded_pairs
                        .iter()
                        .any(|(i, v)| i == idx_var && *v == vk)
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
    captures: Option<&std::collections::HashMap<u16, VecKey>>,
) -> Option<(u16, VecKey)> {
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
    // RHS can be either `len(<vec>)` inline (where `<vec>` is a bare var or
    // a struct field read) OR a bare Var captured earlier as
    // `<local> = len(<vec>)`.
    let rhs = args[1].unspan();
    let vec = match rhs {
        Value::Call(len_def, len_args) => {
            let len_raw = data.def(*len_def).original_name();
            let len_name = len_raw.strip_suffix("Nullable").unwrap_or(len_raw.as_str());
            if !matches!(len_name, "len" | "LengthVector") || len_args.len() != 1 {
                return None;
            }
            vec_key(&len_args[0], data)?
        }
        Value::Var(n) => {
            let caps = captures?;
            *caps.get(n)?
        }
        _ => return None,
    };
    Some((*idx_var, vec))
}

/// Collect every `(idx_var, vec_var)` pair that `cond` proves safe.
/// Handles both a single comparison and AND-conjuncted comparisons
/// like `if a < len(u) and b < len(v) { ... }`.  Caller pushes each
/// returned pair onto `ctx.guarded_pairs` for the duration of the
/// then-block, then pops them.
pub(crate) fn collect_guard_pairs(
    cond: &Value,
    data: &Data,
    captures: &std::collections::HashMap<u16, VecKey>,
) -> Vec<(u16, VecKey)> {
    let mut out = Vec::new();
    collect_guard_pairs_into(cond, data, captures, &mut out);
    out
}

fn collect_guard_pairs_into(
    cond: &Value,
    data: &Data,
    captures: &std::collections::HashMap<u16, VecKey>,
    out: &mut Vec<(u16, VecKey)>,
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
fn len_capture_target(src: &Value, data: &Data) -> Option<VecKey> {
    let Value::Call(def_nr, args) = src.unspan() else {
        return None;
    };
    let raw = data.def(*def_nr).original_name();
    let name = raw.strip_suffix("Nullable").unwrap_or(raw.as_str());
    if !matches!(name, "len" | "LengthVector") || args.len() != 1 {
        return None;
    }
    vec_key(&args[0], data)
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
