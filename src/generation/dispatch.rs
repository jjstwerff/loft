// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Variable assignment and function call dispatch code generation.

use crate::data::{Context, Type, Value};
use crate::ir_node::IrNode;
use std::io::Write;

use super::calls::contains_op_database;
use super::{
    Output, block_needs_i64_widen, default_native_value, narrow_int_cast, rust_type, sanitize,
};

impl Output<'_> {
    /// @PLN90 #495 — variable-assignment entry.  A "runtime-Join" local (owned
    /// init + ≥1 ncc-borrow reassign; see [`Output::witness_vars`]) is routed
    /// through the owned-store-tracker path so neither free-site whole-store-frees
    /// a borrowed view.  Every other var goes straight to [`Self::output_set_body`].
    pub(super) fn output_set(
        &mut self,
        w: &mut dyn Write,
        var: u16,
        to: &Value,
    ) -> std::io::Result<()> {
        if crate::keys::join_own_enabled() && self.witness_vars.contains(&var) {
            return self.output_set_witnessed(w, var, to);
        }
        self.output_set_body(w, var, to)
    }

    /// The owned-store-tracker emission for a runtime-Join local.  An OWNED assign
    /// (the init) emits normally, then points `_own_store_<name>` at the store r
    /// now owns.  A BORROW reassign (the ncc) emits the plain assignment, frees the
    /// tracked OWNED store it displaces (never r's new view), and NULLs the tracker.
    fn output_set_witnessed(
        &mut self,
        w: &mut dyn Write,
        var: u16,
        to: &Value,
    ) -> std::io::Result<()> {
        let variables = self.data.def(self.def_nr).variables();
        let name = sanitize(variables.name(var));
        // A whole-value Var-copy OWNS its fresh store (C86) even though the oracle
        // reports the SOURCE var as Borrowed — mirror collect_witness_vars.
        let is_var_copy = matches!(
            to.unspan(),
            Value::Var(src) if variables.tp(*src).heap_def_nr().is_some()
        );
        let owned = is_var_copy
            || matches!(
                crate::use_analysis::ownership_of(self.data, self.def_nr, to),
                crate::use_analysis::Own::Owned
            );
        let reassign = self.declared.contains(&var);
        if reassign && !owned {
            write!(w, "{{ ")?;
            self.output_set_inner(w, var, to)?;
            write!(
                w,
                "; if _own_store_{name}.store_nr != u16::MAX \
                 && _own_store_{name}.store_nr != var_{name}.store_nr \
                 {{ OpFreeRef(cell, _own_store_{name}, \"{name}(owned)\"); }} \
                 _own_store_{name} = DbRef::NULL; }}"
            )?;
            return Ok(());
        }
        if reassign && owned {
            // An OWNED reassign of a runtime-Join local (a 2nd+ owned assign).
            // r currently holds a store that may be a BORROWED view — so free the
            // tracked OWNED store this displaces, then RESET var_r to the null
            // sentinel so `output_set_body`'s in-place `OpDatabase(var_r)` reuse /
            // `owned_ref_reassign` displaced-free allocate FRESH and no-op the
            // free (never touching the view).  The new owned store becomes the
            // tracked one.
            write!(
                w,
                "{{ if _own_store_{name}.store_nr != u16::MAX \
                 {{ OpFreeRef(cell, _own_store_{name}, \"{name}(owned)\"); }} \
                 var_{name}.store_nr = u16::MAX; "
            )?;
            self.output_set_body(w, var, to)?;
            write!(w, "; _own_store_{name} = var_{name}; }}")?;
            return Ok(());
        }
        // First-decl.  `collect_witness_vars` requires ≥1 owned assign, and the
        // init is owned (a `first = v[i] ?? d` borrow-only local is borrow-TYPED,
        // never a candidate), so this is the owned init — a fresh store, no prior.
        self.output_set_body(w, var, to)?;
        if owned {
            write!(w, "; _own_store_{name} = var_{name}")?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn output_set_body(
        &mut self,
        w: &mut dyn Write,
        var: u16,
        to: &Value,
    ) -> std::io::Result<()> {
        // REASSIGNING an owned heap ref orphans its previous store unless
        // someone frees it — the interpreter's Set emits a pre-free
        // (state/codegen.rs `owned_ref`), but native callees mint a FRESH
        // store per call, so a plain `var_x = n_f(...)` in a loop leaked one
        // store per iteration (the `sweep`/`hexn` tuple leak that blocked
        // #354, and `s = grow(s)` per-iteration struct leaks).  Mirror the
        // interpreter at the same chokepoint with the post-free shape: stash
        // the old DbRef, run the assignment, free the stash when the new
        // value is a different store (an adopting callee returns the SAME
        // store — the free must then no-op, and a never-assigned sentinel
        // store_nr is a no-op inside OpFreeRef).  Excluded, matching the
        // interpreter's predicate: borrowed views (non-empty dep), the
        // fn's hidden return buffer (the CALLER owns that store), and
        // coroutine-persistent fields (no `var_x` local exists).
        {
            let variables = self.data.def(self.def_nr).variables();
            let is_retbuf_attr =
                self.data.def(self.def_nr).attributes().iter().any(|a| {
                    a.hidden && self.data.def(self.def_nr).variables().var(&a.name) == var
                });
            // A retbuf-attr return-local is normally EXCLUDED (the caller owns its
            // store).  But when it has an entry-buffer witness, a CONDITIONAL
            // reassignment must free the orphaned fn-owned intermediate — guarded
            // (below) against the witness so the caller's buffer is never freed.
            let owned_ref_reassign = self.declared.contains(&var)
                && matches!(
                    variables.tp(var),
                    Type::Reference(_, _) | Type::Enum(_, true, _)
                )
                && variables.tp(var).depend().is_empty()
                && !self.coroutine_persistent_vars.contains(&var)
                // A fresh-store-producing rhs: a call, an inline object `Insert`,
                // or a `Block` that builds a new store (the `nullable_unwrap_copy`
                // / `ncc` materialisers — `chosen = v[i] ?? d`).  A bare `Var` rhs
                // (a borrow / move) is excluded — `depend().is_empty()` above
                // already gates out borrowed locals.
                && matches!(
                    to.unspan(),
                    Value::Call(_, _) | Value::Insert(_) | Value::Block(_)
                )
                && (!is_retbuf_attr || self.retbuf_witness.contains(&var));
            if owned_ref_reassign {
                let name = sanitize(variables.name(var));
                // For a witnessed retbuf-attr, also exclude the caller's entry
                // buffer (`_rb_w_<name>`): freeing it would orphan the buffer the
                // caller passed (an over-free / UAF).  Only a fn-owned
                // intermediate (distinct from both the new value AND the witness)
                // is freed.
                let witness_guard = if is_retbuf_attr {
                    format!(" && _old_{name}.store_nr != _rb_w_{name}.store_nr")
                } else {
                    String::new()
                };
                write!(w, "{{ let _old_{name}: DbRef = var_{name}; ")?;
                self.output_set_inner(w, var, to)?;
                write!(
                    w,
                    "; if _old_{name}.store_nr != var_{name}.store_nr{witness_guard} \
                     {{ OpFreeRef(cell, _old_{name}, \"var_{name}(prev)\"); }} }}"
                )?;
                return Ok(());
            }
        }
        self.output_set_inner(w, var, to)
    }

    fn output_set_inner(&mut self, w: &mut dyn Write, var: u16, to: &Value) -> std::io::Result<()> {
        let variables = self.data.def(self.def_nr).variables();
        // P224: writes to coroutine-persistent locals target the struct
        // field directly so the value survives across `next_*` calls.
        // The same Var/Set pair would otherwise produce a state-arm-scoped
        // `let mut var_X = …` shadow that arm 1+ cannot see.
        if self.coroutine_persistent_vars.contains(&var) {
            let name = sanitize(variables.name(var));
            // @PLN25: a `text?` var stores as `String` like plain `text` — peel so the literal→String
            // `.to_string()` conversion fires for an Optional(Text) local.
            let needs_to_string = matches!(variables.tp(var).base(), Type::Text(_));
            write!(w, "self.var_{name} = ")?;
            if needs_to_string {
                write!(w, "(")?;
            }
            self.output_code_inner(w, to)?;
            if needs_to_string {
                write!(w, ").to_string()")?;
            }
            return Ok(());
        }
        if variables.is_argument(var)
            && let Type::RefVar(inner) = variables.tp(var)
        {
            if to != &Value::Null {
                let name = sanitize(variables.name(var));
                // @PLN87 P2.2 / @PLN85 t4 — a `&`-param whole-record write-back that
                // installs a fresh OWNED store (`o = Obj{..}` literal OR `o = mk()`
                // owned-returning call) must FREE the DISPLACED caller store
                // (`*var_o`), else it orphans.  P2.2 covered only the literal (via
                // `is_skip_free`); t4 is the call twin, whose NRVO buffer `__ref_N`
                // is not skip_free — the ownership oracle's `Own::Owned` names both.
                // Aliasing-safe: stash the OLD DbRef by value, install the new store,
                // then free the old one only if distinct — a self-reading RHS
                // (`o = mk_from(o)`) keeps the old store live across the eval (no
                // free-before UAF) and a same-store install is a no-op.  The native
                // twin of the interp stash + `OpFreeRefIfDistinct` at the RefVar-set
                // site (codegen.rs).  Heap inner type only; a `RefVar(Text)` buffer
                // has no such displaced store.
                let amp_owned_writeback = matches!(
                    **inner,
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                ) && matches!(
                    crate::use_analysis::ownership_of(self.data, self.def_nr, to),
                    crate::use_analysis::Own::Owned
                );
                let needs_text_coerce = matches!(**inner, Type::Text(_));
                if amp_owned_writeback {
                    write!(w, "{{ let _old_disp = *var_{name}; *var_{name} = ")?;
                } else {
                    write!(w, "*var_{name} = ")?;
                    if needs_text_coerce {
                        // P223: wrap the RHS in `(...)` so that `.to_string()`
                        // attaches to the whole expression — without parens
                        // an inner `Var(text_local)` (which emits `&var_x`)
                        // would parse as `&(var_x.to_string())` (E0308:
                        // `&String` vs `String`) per Rust method-call
                        // precedence.  The parens are harmless for other
                        // RHS shapes that already produce `String`/`&str`.
                        write!(w, "(")?;
                    }
                }
                self.output_code_inner(w, to)?;
                if amp_owned_writeback {
                    write!(
                        w,
                        "; if _old_disp.store_nr != var_{name}.store_nr {{ \
                         OpFreeRef(cell, _old_disp, \"_old_disp\"); }} }}"
                    )?;
                } else if needs_text_coerce {
                    write!(w, ").to_string()")?;
                }
            }
            return Ok(());
        }
        // @PLN87 L1 — a local SCALAR `&`-link.  `b = &a` lowers to
        // `b: &T = OpCreateStack(a)`; native represents it as a Rust mutable borrow
        // (`&mut i64`), so reads/writes of `b` deref to `a`'s slot (the same shape a
        // `&integer` PARAMETER already uses).  First-Set (the `OpCreateStack` value)
        // (re)binds the reference to the source local; any other value is a
        // write-THROUGH (`*var_b = …`).
        if !variables.is_argument(var)
            && let Type::RefVar(inner) = variables.tp(var)
            && matches!(
                **inner,
                Type::Integer(..) | Type::Float | Type::Single | Type::Boolean | Type::Character
            )
        {
            let name = sanitize(variables.name(var));
            // A RAW pointer (`*mut T`), not `&mut T`: the source local stays usable
            // and assignable while the link is alive (loft allows the aliasing that
            // Rust's borrow checker forbids), matching the interpreter and loft's
            // internal unchecked-aliasing model.  Scalars don't move, so the pointer
            // stays valid for the source's scope.
            let base = rust_type(inner, &Context::Variable);
            // Dispatch on the construction VALUE, which is in the IR (so it survives a
            // snapshot round-trip) — no per-variable flag needed: an `OpGetField` value
            // is an L3 struct-field link, `OpGetVector`/`OpVectorRef` an L4 element link,
            // `OpCreateStack` an L1 local link, anything else a write-through.  All share
            // the `*mut T` representation; only the construction differs.
            if let Value::Call(d_nr, _) = to.unspan()
                && matches!(
                    self.data.def(*d_nr).name(),
                    "OpGetField" | "OpGetVector" | "OpVectorRef"
                )
            {
                // @PLN87 L3/L4 — a heap field (`r = &s.x`) / element (`c = &v[0]`)
                // `&`-ref.  `r`/`c` is the SAME `*mut T` shape as L1, so reads/writes
                // (`*var`) stay uniform; only construction differs — the value yields the
                // place's DbRef (`OpGetField` → record+offset; `OpGetVector` → the inline
                // element location), and we take a `*mut` into that store slot.  Aliases
                // unchecked like L1; a realloc / move of the backing staleness-invalidates
                // it, the same as the interpreter's DbRef.
                if self.declared.contains(&var) {
                    write!(w, "var_{name} = ")?;
                } else {
                    self.declared.insert(var);
                    write!(w, "let mut var_{name}: *mut {base} = ")?;
                }
                write!(w, "unsafe {{ let __ed = ")?;
                self.output_code_inner(w, to)?;
                write!(
                    w,
                    "; stores.store_mut(&__ed).addr_mut::<{base}>(__ed.rec, __ed.pos) \
                     as *mut {base} }}"
                )?;
            } else if let Value::Call(d_nr, cargs) = to.unspan()
                && self.data.def(*d_nr).name() == "OpCreateStack"
                && let [src_arg] = cargs.as_slice()
                && let Value::Var(src) = src_arg.unspan()
            {
                let src_name = sanitize(variables.name(*src));
                if self.declared.contains(&var) {
                    write!(w, "var_{name} = std::ptr::addr_of_mut!(var_{src_name})")?;
                } else {
                    self.declared.insert(var);
                    write!(
                        w,
                        "let mut var_{name}: *mut {base} = std::ptr::addr_of_mut!(var_{src_name})"
                    )?;
                }
            } else if let Value::Var(src) = to.unspan()
                && matches!(variables.tp(*src), Type::RefVar(_))
            {
                // @PLN87 L7 — ref-to-ref (`c = &b`, `b` a scalar reference): `c` copies
                // `b`'s pointer, referencing the same source `b` does (the scalar analogue
                // of the struct ref-to-ref the #257 alias already handles).
                let src_name = sanitize(variables.name(*src));
                if self.declared.contains(&var) {
                    write!(w, "var_{name} = var_{src_name}")?;
                } else {
                    self.declared.insert(var);
                    write!(w, "let mut var_{name}: *mut {base} = var_{src_name}")?;
                }
            } else {
                // write-THROUGH to the linked source
                write!(w, "unsafe {{ *var_{name} = ")?;
                self.output_code_inner(w, to)?;
                write!(w, " }}")?;
            }
            return Ok(());
        }
        // @PLN87 L5 — a heap whole-value reference (`p = &o`): `OpCreateStack(o)` with a
        // Reference referent.  The interp form is a stack-ref to `o`'s slot; native aliases
        // the record BY VALUE (the #257 alias shape, read via the record DbRef) — store
        // `o`'s DbRef.  `p` is non-owning (skip_free at the bind site); a realloc/rebind of
        // `o` is the same L7 edge the #257 alias has.
        if !variables.is_argument(var)
            && let Type::RefVar(inner) = variables.tp(var)
            && matches!(**inner, Type::Reference(..))
            && let Value::Call(d_nr, cargs) = to.unspan()
            && self.data.def(*d_nr).name() == "OpCreateStack"
            && let [src_arg] = cargs.as_slice()
            && let Value::Var(src) = src_arg.unspan()
        {
            let name = sanitize(variables.name(var));
            let src_name = sanitize(variables.name(*src));
            if self.declared.contains(&var) {
                write!(w, "var_{name} = var_{src_name}")?;
            } else {
                self.declared.insert(var);
                write!(w, "let mut var_{name}: DbRef = var_{src_name}")?;
            }
            return Ok(());
        }
        // #257: aliasing a `&ref` param into a fresh local (`snap = s`), both
        // RefVars.  A local RefVar alias is non-owning and never written back
        // through, so storing the record `DbRef` by value (a pointer triple,
        // `Copy`) is enough — and avoids a `&mut DbRef` reborrow that would
        // freeze the source for as long as the alias lives (so `s` could not be
        // read after `snap = s`).  `output_code_inner` yields the record DbRef:
        // `*var_s` for a `&mut DbRef` arg, `var_src` for a local DbRef alias.
        if !variables.is_argument(var)
            && matches!(variables.tp(var), Type::RefVar(_))
            && let Value::Var(src) = to.unspan()
            && matches!(variables.tp(*src), Type::RefVar(_))
        {
            let name = sanitize(variables.name(var));
            if self.declared.contains(&var) {
                write!(w, "var_{name} = ")?;
            } else {
                self.declared.insert(var);
                write!(w, "let mut var_{name}: DbRef = ")?;
            }
            self.output_code_inner(w, to)?;
            return Ok(());
        }
        // @PLN25: a `text?` var stores as `String` like plain `text` — peel so the literal→String
        // `.to_string()` conversion fires for an Optional(Text) local.
        let needs_to_string = matches!(variables.tp(var).base(), Type::Text(_));
        let name = sanitize(variables.name(var));
        // P198 — most operators are wrapped in Value::Span by the parser.
        // Unwrap before pattern-matching so the deep-copy emission below
        // fires for Span(Call(...)) / Span(Var(...)) RHS values.  Without
        // this, native codegen falls through to a plain assignment that
        // aliases the parameter's store, breaking the loft invariant that
        // each ref-typed variable owns its own store
        // (tests/scripts/95-alias-copy.loft assert orig.x == 1.0 fails
        // because mutating ac_copy mutates ac_orig too).
        let to_unspanned = to.unspan();
        // A call whose return is NOT a fresh-adopt store (it borrows a visible
        // param OR is a reused hidden buffer) must be deep-copied into a store
        // this binding owns — else the returned DbRef aliases the callee's
        // arg/buffer and a later `OpFreeRef(var)` whole-store-frees it.
        // Cluster-A A.3: reads the canonical `return_adopts_fresh_store()` fact,
        // the SAME gate the interpreter uses (`state/codegen.rs`
        // gen_set_first_at_tos), instead of the coarse "callee has a visible
        // Reference/Enum param" proxy — which MISSED a borrowed VECTOR-element
        // view (`fn pick(t: vector<M>) -> M { t[i] }`: no Reference param, yet
        // the return aliases `t`, so freeing the bound local freed the caller's
        // vector).
        if let (Some(d_nr), Value::Call(fn_nr, args)) =
            (variables.tp(var).heap_def_nr(), to_unspanned)
            && self.data.def(*fn_nr).name().starts_with("n_")
            && *self.data.def(*fn_nr).code() != Value::Null
            && !self.data.def(*fn_nr).return_adopts_fresh_store()
        {
            let tp_nr = self.data.def(d_nr).known_type();
            if !self.declared.contains(&var) {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                writeln!(
                    w,
                    "let mut var_{name}: {tp_str} = stores.null_named(\"var_{name}\");"
                )?;
                self.indent(w)?;
            }
            // @P298 / @P297 (native half) — free the callee's return store
            // after the deep copy by OR-ing the `0x8000` source-free bit into
            // the OpCopyRecord type-nr (the runtime `OpCopyRecord` honours it).
            // Without this, `_src` (the freshly-allocated struct the callee
            // returned) is copied into `var_{name}` and then leaked — one
            // store per call.  Mirrors the interpreter's
            // `gen_set_first_ref_call_copy` (`src/state/codegen.rs`).
            //
            // Clear the bit when the callee returns a BORROWED view (its
            // return type carries a `dep` chain naming one of its args): the
            // "source" is then a slice of an arg's store, and freeing it would
            // corrupt the caller.  Return-dep inference tags these correctly.
            // A dep naming only HIDDEN attrs is NOT a borrow — it is the
            // one-buffer return marker (`["??"]`): the callee minted a fresh
            // store into its buffer param and nobody else owns it, so the
            // source-free bit must stay set (skipping it leaked one store
            // per `s = grow(s)` loop iteration).
            // Cluster-A A.4: ONE return-ownership query, shared with the
            // interpreter (`state/codegen.rs`).  Both backends read the same
            // fact, so they cannot diverge on the hidden-only / out-of-range edge.
            let is_borrowed_view = self.data.def(*fn_nr).returns_borrowed_view();
            let tp_with_free: i32 = if is_borrowed_view {
                i32::from(tp_nr)
            } else {
                i32::from(tp_nr) | 0x8000
            };
            // @P290 — evaluate the call BEFORE touching the destination.  The
            // call's args can reference `var_{name}` itself (e.g. `g = id(g)`),
            // and `OpDatabase` clears the destination's store IN PLACE — so the
            // old code (`OpDatabase(var)` then the call) handed the callee an
            // already-emptied destination.  Compute `_src` first against the
            // live `_dst`, then re-init + deep-copy.  When `_src` lives in the
            // destination's OWN store (the callee returned its arg — a borrowed
            // view), clearing that store would wipe the very data we copy, so
            // pass the reference through unchanged instead (the interpreter's
            // PutRef path in `gen_set_first_ref_call_copy`).
            // P198 — the inner user-fn call uses the new `cell` ABI; the
            // outer OpCopyRecord wraps `cell` to a fresh `&mut Stores`.
            let callee = self.data.def(*fn_nr);
            // @PLN85 unification — collapse the native first-bind onto the ownership
            // oracle. A `??`-JOIN return needs the runtime ARG-ALIASING guard
            // witnessed by the oracle's interprocedurally-resolved base (the borrowed
            // arg): adopt the owned arm (`_src` fresh, not aliasing the witness) and
            // materialise the borrow arm (`_src` aliases the witness). The old
            // `_src == _dst` guard re-derived this and LEAKED the owned arm — `_src`
            // (a fresh `m_none()`) never equals `_dst` (the old slot), so it
            // materialised + dropped the owned store.
            let join_witness = if crate::keys::join_own_enabled()
                && let crate::use_analysis::Own::Join { base } =
                    crate::use_analysis::ownership_of(self.data, self.def_nr, to)
                && base != u16::MAX
            {
                Some(sanitize(variables.name(base)))
            } else {
                None
            };
            write!(
                w,
                "{{ let _dst = var_{name}; let _src = {}(cell",
                callee.name()
            )?;
            // Emit each arg through the shared `emit_call_arg` helper so the
            // ABI-B call applies the same per-parameter coercions (boolean→u8,
            // narrow-int, text deref, typed-null, fn-ref) as the normal call
            // path.  Re-deriving arg emission here is what dropped the
            // boolean→u8 wrap and tripped rustc E0308 (issue #366).
            for (idx, arg) in args.iter().enumerate() {
                write!(w, ", ")?;
                self.emit_call_arg(w, callee, idx, arg)?;
            }
            // The ADOPT condition. Default (`_src == _dst`): adopt a null return or
            // a same-store NRVO alias, else deep-copy. JOIN (witnessed): adopt a
            // null/fresh `_src` that does NOT alias the borrowed arg `witness`, else
            // (it aliases the witness) materialise — the join's owned/borrow split.
            let adopt = match &join_witness {
                Some(witness) => {
                    format!("_src.store_nr == u16::MAX || _src.store_nr != var_{witness}.store_nr")
                }
                None => "_src.store_nr == u16::MAX || _src.store_nr == _dst.store_nr".to_string(),
            };
            // @PLN85 (the adopt-arm placeholder leak) — the ADOPT arm replaces
            // `var_{name}`'s slot with `_src`, orphaning `_dst` when it is a
            // REAL store (the first-bind `null_named` pre-allocation, or a
            // displaced prior store on reassignment): one store leaked per
            // adopting bind (`d = choose(..)`).  Free the real, distinct
            // placeholder first — the same exclusive-ownership assumption the
            // COPY arm already makes (it clears `_dst` in place via
            // `OpDatabase`).  A same-store adopt (the NRVO alias) and the
            // null-sentinel `_dst` are excluded by the guard.
            write!(
                w,
                "); if {adopt} {{ if _dst.store_nr != u16::MAX \
                 && _dst.store_nr != _src.store_nr \
                 {{ OpFreeRef(cell, _dst, \"{name}(displaced)\"); }} \
                 var_{name} = _src; }} \
                 else {{ var_{name} = OpDatabase(cell, _dst, {tp_nr}_i32); \
                 OpCopyRecord(cell,_src, var_{name}, {tp_with_free}_i32); }} }}"
            )?;
            return Ok(());
        }
        // When assigning a reference to a reference variable, a pointer copy is not
        // sufficient — emit an OpCopyRecord call for a deep copy.
        // For a first declaration, we also need to allocate a fresh store via
        // OpDatabase(null_named(…)) so the destination has its own record to copy into.
        // For reassignment, the existing destination record is reused in-place.
        if let (Some(d_nr), Value::Var(src)) = (variables.tp(var).heap_def_nr(), to_unspanned)
            && variables.tp(*src).heap_def_nr().is_some()
        {
            let src_name = sanitize(variables.name(*src));
            let tp_nr = self.data.def(d_nr).known_type();
            if self.declared.contains(&var) {
                // Reassignment: the variable was pre-declared via null_named
                // (Set(var, Null)) at function entry.  OpDatabase below
                // ensures it has a valid allocated record.
            } else {
                self.declared.insert(var);
                let var_tp = variables.tp(var);
                let tp_str = rust_type(var_tp, &Context::Variable);
                // Two statements: null_named and OpDatabase cannot share a &mut stores borrow.
                writeln!(
                    w,
                    "let mut var_{name}: {tp_str} = stores.null_named(\"var_{name}\");"
                )?;
                self.indent(w)?;
            }
            writeln!(w, "var_{name} = OpDatabase(cell,var_{name}, {tp_nr}_i32);")?;
            self.indent(w)?;
            write!(
                w,
                "OpCopyRecord(cell,var_{src_name}, var_{name}, {tp_nr}_i32)"
            )?;
            return Ok(());
        }
        // For text/reference block assignments, pre-declare the variable so that
        // any drop(@var) inside the block (e.g., on break) can reference it.
        if !self.declared.contains(&var) && matches!(to, Value::Block(_)) {
            let var_tp = variables.tp(var);
            // @PLN25: peel `Optional(Text)` — a `text?` block-assigned local is `String`-typed.
            if matches!(var_tp.base(), Type::Text(_)) {
                self.declared.insert(var);
                write!(w, "let mut var_{name} = ")?;
                self.output_code_inner(w, to)?;
                if needs_to_string {
                    write!(w, ".to_string()")?;
                }
                return Ok(());
            }
        }
        // S35: Set(var, Insert([stmt1, ..., last_expr])) — hoist all-but-last ops as
        // statements before the declaration, then assign only from the final expression.
        // Without this, the inner Set ops are emitted inline inside an expression context,
        // producing malformed Rust like `let mut var_rv: DbRef = let mut var__read: DbRef = …`.
        //
        // @P321g: unspan `to` first.  The parser wraps an assignment RHS in
        // `Value::Span` for source-position tracking, so `x = route_click(p,
        // st.es_tools, …)` — where the `&`-ref arg `st.es_tools` materialises a
        // `Set(__ref_N, …)` statement ahead of the call — arrives here as
        // `Span(Insert([Set(__ref_N, …), Call(…)]))`.  Matching the bare
        // `Insert` missed it, falling through to the brace-less `Insert` arm in
        // `output_code_inner` and re-emitting the exact malformed shape above.
        if let Value::Insert(ops) = to_unspanned
            && !ops.is_empty()
        {
            for op in &ops[..ops.len() - 1] {
                self.indent(w)?;
                self.output_code_inner(w, op)?;
                writeln!(w, ";")?;
            }
            self.indent(w)?;
            if self.declared.contains(&var) {
                write!(w, "var_{name} = ")?;
            } else {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                write!(w, "let mut var_{name}: {tp_str} = ")?;
            }
            self.output_code_inner(w, &ops[ops.len() - 1])?;
            return Ok(());
        }
        // Hoist call arguments that mutate stores into temporaries to prevent
        // double-mutable-borrow of `stores` in the call expression.
        //
        // ONLY for ops emitted as an actual CALL — a user fn or a `codegen_runtime`
        // Op stub (`rust().is_empty()`).  An inline `#rust` op (non-empty template,
        // e.g. a byte/short field read like `OpGetByteNullable`) has NO callable
        // fn: emitting `OpGetByteNullable(cell, _harg, …)` is unresolved.  Those
        // fall through to the normal emit, which inlines the `#rust` body — whose
        // own `let db = @v1; …` sequences the mutating arg before the read borrow,
        // so no double-borrow (and the receiver was already pre-eval-hoisted).
        if let Value::Call(call_dnr, args) = to.unspan()
            && self.data.def(*call_dnr).rust().is_empty()
            && args
                .iter()
                .any(|a| contains_op_database(IrNode::Native(a), self.data))
        {
            let def_fn = self.data.def(*call_dnr);
            let mut hoisted: Vec<Option<String>> = vec![None; args.len()];
            for (idx, arg) in args.iter().enumerate() {
                if contains_op_database(IrNode::Native(arg), self.data) {
                    let param_tp = if idx < def_fn.attributes().len() {
                        rust_type(&def_fn.attributes()[idx].typedef, &Context::Argument)
                    } else {
                        "DbRef".to_string()
                    };
                    let tmp = format!("_harg_{name}_{idx}");
                    write!(w, "let {tmp}: {param_tp} = ")?;
                    self.output_code_inner(w, arg)?;
                    writeln!(w, ";")?;
                    self.indent(w)?;
                    hoisted[idx] = Some(tmp);
                }
            }
            if self.declared.contains(&var) {
                write!(w, "var_{name} = ")?;
            } else {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                write!(w, "let mut var_{name}: {tp_str} = ")?;
            }
            // P199 — user-fn / Op-stub callees take `&UnsafeCell<Stores>`
            // (cell), not `&mut Stores` (stores).  Plan 09 phase 01 added
            // per-fn ABI tagging via `crate::codegen_runtime::abi_of`:
            //   - Cell  → `name(cell, args...)`     (default)
            //   - None  → `name(args...)`           (no implicit Stores)
            let abi = if *def_fn.code() == Value::Null {
                crate::codegen_runtime::abi_of(def_fn.name())
            } else {
                crate::codegen_runtime::Abi::Cell
            };
            write!(w, "{}(", self.fn_ident(def_fn))?;
            let mut first_arg = true;
            if matches!(abi, crate::codegen_runtime::Abi::Cell) {
                write!(w, "cell")?;
                first_arg = false;
            }
            for (idx, arg) in args.iter().enumerate() {
                if !first_arg {
                    write!(w, ", ")?;
                }
                first_arg = false;
                // A store-mutating arg was hoisted to a typed temporary above;
                // emit its name.  Every other arg goes through the shared
                // `emit_call_arg` so this path applies the same per-parameter
                // coercions (boolean→u8, …) as the normal + ABI-B call paths
                // (issue #366 — keep all three call paths in lockstep).
                if let Some(ref tmp) = hoisted[idx] {
                    write!(w, "{tmp}")?;
                } else {
                    self.emit_call_arg(w, def_fn, idx, arg)?;
                }
            }
            write!(w, ")")?;
            if needs_to_string {
                write!(w, ".to_string()")?;
            }
            return Ok(());
        }
        // @P302 — capture first-vs-reassign BEFORE the `declared` insert below
        // (the first-decl branch inserts `var`, so the flag must be read now).
        // A #260-predeclared `__vdb` counts as FIRST here (consume the entry):
        // its prologue `let` bound only the sentinel, so this Set still emits
        // the named-store `null_named` + `OpDatabase` pair.
        // #354: the `_` discard loop var shares ONE table entry across all
        // the fn's loops, so a later loop whose iter-value type differs from
        // the table type (an integer range after a float range gives an i64
        // value into the f64 `var__`) fails E0308.  `_` is a discard, so
        // emit a fresh shadowing `let` typed from THIS loop's own iter
        // value.  Restricted to a SCALAR table type AND scalar iter value:
        // a collection `for _ in <vec>` binds a DbRef view whose OpFreeRef
        // (emitted by scope analysis against the table type) must keep
        // seeing a DbRef — re-typing it scalar there orphaned the store (a
        // leak, crawler's hex/sim libs).
        fn is_scalar(t: &Type) -> bool {
            matches!(
                t,
                Type::Integer(_)
                    | Type::Float
                    | Type::Single
                    | Type::Boolean
                    | Type::Character
                    | Type::Enum(_, false, _)
            )
        }
        let discard_loop_var = variables.name(var) == "_"
            && is_scalar(variables.tp(var))
            && matches!(to.unspan(), Value::Block(bl) if is_scalar(&bl.result));
        let first_assign = !self.declared.contains(&var) || self.predeclared.remove(&var);
        if self.declared.contains(&var) && !discard_loop_var {
            write!(w, "var_{name} = ")?;
        } else {
            self.declared.insert(var);
            let var_tp = if discard_loop_var && let Value::Block(bl) = to.unspan() {
                bl.result.clone()
            } else {
                variables.tp(var).clone()
            };
            let tp_str = rust_type(&var_tp, &Context::Variable);
            write!(w, "let mut var_{name}: {tp_str} = ")?;
        }
        if matches!(to, Value::Null) && rust_type(variables.tp(var), &Context::Variable) == "DbRef"
        {
            self.emit_null_dbref(w, var, &name, first_assign)?;
        } else if to == &Value::Null {
            // Emit the null sentinel for the variable's type, not bare `()`.
            let null_val = default_native_value(variables.tp(var));
            write!(w, "{null_val}")?;
        } else {
            // @PLN17: a boolean variable's storage form is u8.  The RHS may be a
            // compound `bool` expression (`!b` → `(..) != 1`, `a == b`), so the
            // ` as u8` coercion must WRAP the whole RHS — a bare suffix binds to
            // the last token (`!= 1 as u8`) and leaves a `bool`.  Open the paren
            // here; the narrow-cast suffix below closes it with `) as u8`.
            let wrap_bool =
                !matches!(to, Value::Null) && matches!(variables.tp(var).base(), Type::Boolean);
            // #433 — a narrow-int value-block (`vec<u8>[i] ?? <int>` ncc) assigned to
            // a plain `integer` (i64) variable needs an `as i64` widen, same as the
            // return seam (see block_needs_i64_widen).  Open the wrapping paren here;
            // the suffix closes it below.
            let widen_block = block_needs_i64_widen(to, variables.tp(var));
            if wrap_bool || widen_block {
                write!(w, "(")?;
            }
            // O7: when this text assignment opens a multi-segment format string,
            // pre-allocate capacity to avoid repeated reallocations.
            if needs_to_string
                && self.next_format_count > 1
                && let Value::Text(initial) = to
            {
                let n = self.next_format_count;
                self.next_format_count = 0;
                let cap = initial.len() + n * 8;
                if initial.is_empty() {
                    write!(w, "String::with_capacity({cap}_usize)")?;
                } else {
                    write!(
                        w,
                        "{{ let mut _s = String::with_capacity({cap}_usize); \
                         _s.push_str({initial:?}); _s }}"
                    )?;
                }
            } else {
                // wrap plain Int or If-with-Int values assigned to Function vars.
                let is_fn_ref_var = matches!(variables.tp(var), Type::Function(_, _, _));
                let wrap_fn_ref = is_fn_ref_var && matches!(to, Value::Int(_));
                if wrap_fn_ref {
                    write!(w, "(")?;
                }
                // set fn_ref_context so if-else branches with bare Int
                // values produce (u32, null_DbRef) tuples.  Cleared inside
                // Call argument processing to avoid wrapping OpDatabase args.
                let prev_ctx = self.fn_ref_context;
                if is_fn_ref_var && !wrap_fn_ref {
                    self.fn_ref_context = true;
                }
                // When assigning to a `(String, …)` tuple variable, the
                // element values that emit as `&str` literals need a
                // `.to_string()` wrap so the tuple's runtime type
                // matches its declared `(String, …)` shape.  Without
                // this the Rust compiler rejects `("a", "b")` against
                // `(String, String)`.  See `Value::Text` in emit.rs.
                let prev_tuple_text = self.tuple_text_to_string;
                if let Type::Tuple(elems) = variables.tp(var)
                    && tuple_has_text_leaf(elems)
                {
                    // Recurse through nested tuples so `((i64, String),
                    // (i64, String))` triggers the flag too — without
                    // this, plan-14 phase-02 cells like
                    // `((1, "a"), (2, "b"))` emitted `"a"` (`&str`)
                    // against a declared `String` slot and rustc raised
                    // E0308.
                    self.tuple_text_to_string = true;
                }
                // When assigning to a String variable from a text-local source,
                // output_code_inner emits `&var_name` (borrow to &str), and
                // appending `.to_string()` yields `&String` not `String`.
                // Detect this case and emit `.clone()` on the owned String directly.
                // Unspan the RHS so the detection fires for Span-wrapped Var
                // (the common case for assignment RHS — same shape as P228).
                let text_local_clone = needs_to_string
                    && matches!(to.unspan(), Value::Var(v) if {
                        let vars = self.data.def(self.def_nr).variables();
                        // @PLN25 slice (c): `.base()` — a `text?` local source is a `String`
                        // just like plain `text`, so it must emit `var.clone()` here. Without
                        // the peel it fell through to `&var.to_string()` (E0308: `&String`).
                        !vars.is_argument(*v) && matches!(vars.tp(*v).base(), Type::Text(_))
                    });
                // @P283 — source is a `RefVar(Text)` argument (`&mut String`).
                // `output_code_inner` for `Value::Var` in this case emits
                // `&*var_X` (emit.rs:141); appending `.to_string()` then parses
                // as `&*(var_X.to_string())` per Rust method-call precedence,
                // which evaluates to `&str` and breaks the `String`-typed
                // assignment with E0308.  Emit `var_X.to_string()` directly —
                // auto-deref through `&mut String` produces a fresh owned
                // `String` of the correct type.
                let refvar_text_clone = needs_to_string
                    && matches!(to.unspan(), Value::Var(v) if {
                        let vars = self.data.def(self.def_nr).variables();
                        vars.is_argument(*v)
                            && matches!(vars.tp(*v), Type::RefVar(inner) if matches!(**inner, Type::Text(_)))
                    });
                // T1.8a: same pattern when the source is a `TupleGet` of a
                // text element from a Variable-context tuple — the tuple's
                // text fields are `String`, and `output_code_inner` for
                // `Value::TupleGet` of a text element emits `&var_t.0`
                // (borrow to &str).  Appending `.to_string()` yields
                // `&String` not `String`, breaking destructuring of a
                // tuple-of-text return.  Emit `var_t.0.clone()` instead.
                //
                // P228: unspan `to` before pattern-matching so the
                // detection fires when the parser wraps the TupleGet in
                // a `Value::Span` (the common case for assignment RHS).
                // Without the unspan, plain `label = t.0` (where `t` is
                // a tuple with a text element) emitted
                // `let mut var_label: String = &var_t.0.to_string();` —
                // E0308 because `&var_t.0.to_string()` parses as
                // `&(var_t.0.to_string())` per Rust method-call
                // precedence (`&String` vs declared `String`).
                let to_inner = to.unspan();
                let tuple_text_elem_clone = needs_to_string
                    && matches!(to_inner, Value::TupleGet(v, idx) if {
                        let vars = self.data.def(self.def_nr).variables();
                        !vars.is_argument(*v)
                            && matches!(vars.tp(*v),
                                Type::Tuple(elems)
                                if elems.get(*idx as usize).is_some_and(|e| matches!(e, Type::Text(_))))
                    });
                // P247 — destination is a tuple-typed work var (e.g.
                // `__ref_N: (i64, String)` materialised by the
                // operators.rs nested-TupleGet read path) AND the
                // source is a `TupleGet(parent, idx)` whose result
                // type contains a non-Copy leaf (Text / Reference).
                // Default emission `let __ref_N = var_t.0;` MOVES
                // `var_t.0`, invalidating subsequent reads of
                // `var_t.0.X` in the same expression.  Emit
                // `var_t.0.clone()` instead so each chained access
                // gets its own owned copy.
                let nested_tuple_clone = matches!(variables.tp(var), Type::Tuple(elems)
                    if tuple_has_non_copy_leaf(elems))
                    && matches!(to_inner, Value::TupleGet(v, _) if {
                        let vars = self.data.def(self.def_nr).variables();
                        !vars.is_argument(*v) && matches!(vars.tp(*v), Type::Tuple(_))
                    });
                if text_local_clone {
                    if let Value::Var(v) = to.unspan() {
                        let src_name = sanitize(self.data.def(self.def_nr).variables().name(*v));
                        write!(w, "var_{src_name}.clone()")?;
                    }
                } else if refvar_text_clone {
                    if let Value::Var(v) = to.unspan() {
                        let src_name = sanitize(self.data.def(self.def_nr).variables().name(*v));
                        write!(w, "var_{src_name}.to_string()")?;
                    }
                } else if tuple_text_elem_clone {
                    // P228: read through the same unspan as the detection above.
                    if let Value::TupleGet(v, idx) = to.unspan() {
                        let src_name = sanitize(self.data.def(self.def_nr).variables().name(*v));
                        write!(w, "var_{src_name}.{idx}.clone()")?;
                    }
                } else if nested_tuple_clone {
                    if let Value::TupleGet(v, idx) = to.unspan() {
                        let src_name = sanitize(self.data.def(self.def_nr).variables().name(*v));
                        write!(w, "var_{src_name}.{idx}.clone()")?;
                    }
                } else {
                    self.output_code_inner(w, to)?;
                }
                self.fn_ref_context = prev_ctx;
                self.tuple_text_to_string = prev_tuple_text;
                if needs_to_string
                    && !text_local_clone
                    && !refvar_text_clone
                    && !tuple_text_elem_clone
                    && !nested_tuple_clone
                {
                    write!(w, ".to_string()")?;
                } else if wrap_fn_ref {
                    write!(w, " as u32, loft::keys::DbRef::NULL)")?;
                } else if matches!(variables.tp(var), Type::Routine(_))
                    && !matches!(to, Value::Null)
                {
                    write!(w, " as u32")?;
                } else if to != &Value::Null && narrow_int_cast(variables.tp(var)).is_some() {
                    // Variable is a narrow integer type, but the RHS expression
                    // (a function returning u16 or an iterator block returning as u16) produces
                    // the narrow type.  Post-2c: widen to i64 to match the default Integer.
                    // @PLN17: a boolean variable's storage form is u8 (not i64) — cast the
                    // RHS (`bool` literal/comparison or `u8`) to u8, not i64.
                    if matches!(variables.tp(var).base(), Type::Boolean) {
                        write!(w, ") as u8")?; // closes the `(` opened before the RHS
                    } else {
                        write!(w, " as i64")?;
                    }
                } else if widen_block {
                    // #433 — close the paren opened above and widen the narrow-int
                    // value-block to the `integer` (i64) variable's slot width.
                    write!(w, ") as i64")?;
                } else if let Value::Call(d_nr, _) = to.unspan() {
                    // When the variable type and the called function's return type differ
                    // (e.g., multiple parallel-for loops reusing `b` with different worker types),
                    // add a cast so Rust accepts the assignment.
                    let var_tp_str = rust_type(variables.tp(var), &Context::Variable);
                    let ret = self.data.def(*d_nr).returned();
                    let ret_str = rust_type(ret, &Context::Variable);
                    if ret_str != var_tp_str && !matches!(ret, Type::Void) {
                        write!(w, " as {var_tp_str}")?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Emit a null-initialised `DbRef` variable, matching the interpreter's pre-init order.
    ///
    /// `first == false` is an @P302 keyed-collection reassignment (`s = []`
    /// clear): the slot already holds a live `DbRef`, so emit `OpDatabase`
    /// ALONE (in-place clear, reuses the store, no leak) — skip `null_named`,
    /// which would reset to a sentinel and leak the old store.
    fn emit_null_dbref(
        &mut self,
        w: &mut dyn Write,
        var: u16,
        name: &str,
        first: bool,
    ) -> std::io::Result<()> {
        let variables = self.data.def(self.def_nr).variables();
        let var_raw_name = variables.name(var);
        let is_elm = var_raw_name.starts_with("_elm");
        // A `skip_free` variable is a BORROWED VIEW by construction — the parser /
        // scope analysis set the flag precisely to suppress its `OpFreeRef` because
        // it does not own a store (e.g. a match-arm binding `match e { V { v } =>
        // … }`, which aliases the subject's payload record).  Such a slot is always
        // overwritten by the borrowing read (`InitRefSentinel` → `OpGetField`), so
        // allocating a backing store here would orphan it: no free is emitted, so
        // the store leaks (one per evaluation).  The interpreter's `InitRefSentinel`
        // never allocates; this reads the same ownership fact rather than
        // re-deriving it from the dep list — a vector-typed match-bind carries an
        // EMPTY dep (it is not dep-tracked), so `owns_store` below would wrongly
        // read it as owning.  The `??`-coalesce borrow (`__ncc_*`) instead carries a
        // non-empty dep, so it already lands on the sentinel via `owns_store`.
        let is_skip_free = variables.is_skip_free(var);
        let owns_store = match variables.tp(var) {
            Type::Reference(_, dep) | Type::Vector(_, dep) | Type::Enum(_, true, dep) => {
                dep.is_empty()
            }
            // @P302 — a keyed local backed by its own store carries a self-dep
            // `[var]` (added by the `s = []` clear path so a later `s += …`
            // re-inits in place).  That is an ownership marker, not a borrow.
            Type::Sorted(_, _, dep)
            | Type::Hash(_, _, dep)
            | Type::Index(_, _, dep)
            | Type::Radix(_, _, dep) => dep.is_empty() || (dep.len() == 1 && dep[0] == var),
            _ => false,
        };
        if is_elm || variables.is_inline_ref(var) || is_skip_free || !owns_store {
            write!(w, "DbRef::NULL")?;
        } else {
            let ref_buf_type_id = {
                let var_tp = variables.tp(var).clone();
                match &var_tp {
                    Type::Vector(elm_tp, _) => {
                        let elm_name = elm_tp.name(self.data);
                        self.data.name_type(&format!("main_vector<{elm_name}>"), 0)
                    }
                    // P188: keyed-collection locals need an OpDatabase
                    // call against the specific keyed-collection type so
                    // the backing store is allocated and the root pointer
                    // is zero-initialised.  Resolves to the same database
                    // type id that struct-field registration uses.
                    Type::Sorted(td, key, _) | Type::Index(td, key, _) => {
                        let c = self.data.def(*td).known_type();
                        if c == u16::MAX {
                            u16::MAX
                        } else {
                            let prefix = match &var_tp {
                                Type::Sorted(_, _, _) => "sorted",
                                Type::Index(_, _, _) => "index",
                                _ => unreachable!(),
                            };
                            let mut name =
                                format!("{prefix}<{}[", self.stores.types[c as usize].name);
                            for (k_nr, (k, asc)) in key.iter().enumerate() {
                                if k_nr > 0 {
                                    name += ",";
                                }
                                if !*asc {
                                    name += "-";
                                }
                                name += k;
                            }
                            name += "]>";
                            self.stores.name(&name)
                        }
                    }
                    Type::Hash(td, key, _) | Type::Radix(td, key, _) => {
                        let c = self.data.def(*td).known_type();
                        if c == u16::MAX {
                            u16::MAX
                        } else {
                            let prefix = match &var_tp {
                                Type::Hash(_, _, _) => "hash",
                                Type::Radix(_, _, _) => "spatial",
                                _ => unreachable!(),
                            };
                            let mut name =
                                format!("{prefix}<{}[", self.stores.types[c as usize].name);
                            for (k_nr, k) in key.iter().enumerate() {
                                if k_nr > 0 {
                                    name += ",";
                                }
                                name += k;
                            }
                            name += "]>";
                            self.stores.name(&name)
                        }
                    }
                    _ => u16::MAX,
                }
            };
            if ref_buf_type_id == u16::MAX {
                // @P317 — a struct `Reference` local with no resolvable
                // backing-store type id: emit the NULL SENTINEL, not
                // `null_named` (which allocates a real store slot).  Every
                // use of such a local is preceded by either an `OpDatabase`
                // (which allocates fresh from the sentinel — see
                // codegen_runtime::OpDatabase's `store_nr == u16::MAX` arm)
                // or a reassignment from a call return (which supplies its
                // own store).  Allocating a `null_named` placeholder here
                // LEAKS one store per reassignment-from-call: the placeholder
                // slot is overwritten by the call's DbRef and never freed
                // (e.g. `nk = chunk_of(...)` first-assigned inside an `if` in
                // a loop — the C3-incremental store exhaustion that tripped
                // `assert!(store.free)`).  Matches the interpreter, which
                // null-inits ref locals to a sentinel, not an allocation.
                write!(w, "DbRef::NULL")?;
            } else if first {
                writeln!(w, "stores.null_named(\"var_{name}\");")?;
                self.indent(w)?;
                write!(
                    w,
                    "var_{name} = OpDatabase(cell,var_{name}, {ref_buf_type_id}_i32)"
                )?;
            } else {
                // @P302 reassignment / `s = []` clear: the slot already holds a
                // live DbRef → OpDatabase clears that store in place (no
                // null_named, which would reset to a sentinel and leak the
                // old store).  The caller already wrote the `var_{name} = `.
                write!(w, "OpDatabase(cell,var_{name}, {ref_buf_type_id}_i32)")?;
            }
        }
        Ok(())
    }

    /// Use this to dispatch a `Value::Call` to either the user-function or template emitter.
    /// Certain built-in text operations are intercepted here because their generated Rust
    /// differs structurally from both a regular call and a template substitution.
    #[allow(clippy::too_many_lines)] // large opcode dispatch — splitting would lose context
    pub(super) fn output_call(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
        vals: &[Value],
    ) -> std::io::Result<()> {
        // clear fn_ref_context inside calls — arguments like OpDatabase's
        // type number are plain integers, not fn-ref d_nr values.
        let saved_ctx = self.fn_ref_context;
        self.fn_ref_context = false;
        // P238: clear tuple_text_to_string inside calls — call arguments
        // bind to the callee's parameter signature (typically `&str` for
        // text params), not to the outer assignment target's tuple slot
        // type.  Without this clear, `let var_s: (String, String) =
        // t_4text_pair_t(cell, "hi")` would emit `"hi".to_string()` for
        // the arg because the outer `(String, String)` flag propagated
        // into `Value::Text` rendering.
        let saved_tuple = self.tuple_text_to_string;
        self.tuple_text_to_string = false;
        let result = self.output_call_inner(w, def_nr, vals);
        self.fn_ref_context = saved_ctx;
        self.tuple_text_to_string = saved_tuple;
        result
    }

    #[allow(clippy::too_many_lines)]
    fn output_call_inner(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
        vals: &[Value],
    ) -> std::io::Result<()> {
        let def_fn = self.data.def(def_nr);
        let name: &str = def_fn.name();
        // Phase 09 phase 00 step 0.6: registry-first dispatch.  When a
        // custom emitter is registered for this Op, run it instead of
        // the special-case match arms below.  Today the registry is
        // empty so every Op falls through to the existing dispatch.
        // Future phases register per-Op emitters that take over these
        // emissions one Op at a time (without touching the bulk match).
        if crate::generation::ops::has_custom_emitter(name) {
            let name_owned = name.to_string();
            let mut ctx = crate::generation::ops::EmitCtx {
                w,
                def_fn,
                output: self,
            };
            return crate::generation::ops::emit_op(&mut ctx, &name_owned, vals);
        }
        if def_fn.rust().is_empty() {
            self.output_call_user_fn(w, def_fn, vals)
        } else {
            self.output_call_template(w, def_fn, vals)
        }
    }
}

/// True iff `elems` contains a `Type::Text` element at any depth —
/// includes text reached through nested `Type::Tuple`.  Used by
/// `tuple_text_to_string` activation in `output_set` so a tuple
/// destination like `((i64, String), (i64, String))` triggers the
/// `"a".to_string()` wrap on the inner literals (plan-14 phase 02).
fn tuple_has_text_leaf(elems: &[Type]) -> bool {
    for e in elems {
        match e {
            Type::Text(_) => return true,
            Type::Tuple(inner) if tuple_has_text_leaf(inner) => return true,
            _ => {}
        }
    }
    false
}

/// True iff `elems` contains a leaf type that doesn't impl `Copy` in
/// generated Rust — text, references, vectors, hash/index/sorted/
/// spatial, iterators, struct-enums.  Used by `nested_tuple_clone`
/// (P247) to decide whether `let __ref_N = var_t.0;` needs a
/// `.clone()` to avoid moving non-Copy data out of the parent tuple.
/// Plain integers / floats / booleans / characters / plain enums
/// are Copy and don't need cloning.
fn tuple_has_non_copy_leaf(elems: &[Type]) -> bool {
    for e in elems {
        match e {
            Type::Text(_)
            | Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Iterator(_, _)
            | Type::Enum(_, true, _) => return true,
            Type::Tuple(inner) if tuple_has_non_copy_leaf(inner) => return true,
            _ => {}
        }
    }
    false
}
