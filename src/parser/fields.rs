// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent)

use super::{DefType, I32, Level, Parser, Parts, Type, Value, diagnostic_format, v_block, v_set};

// Field access, indexing, and iterator operations.

impl Parser {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn field(&mut self, code: &mut Value, tp: Type) -> Type {
        // @PLN102 — a field access can't be MORE non-null than its receiver: if the
        // receiver is nullable (`s: S? = null`, `w.inner` where `inner: Inner?`),
        // reading `s.field` yields the field type's null when the receiver is absent
        // (C80), so `s.field == null` is NOT "always false" and `s.field ?? d` is NOT
        // redundant.  `get_field` sets `expr_not_null` from the FIELD's own nullness;
        // clear it below when the receiver's TYPE is `Optional`.  (The receiver TYPE is
        // the right signal — `expr_not_null` alone is false even for a non-null
        // constructed struct, which would wrongly suppress genuine warnings, e.g.
        // p285.)
        let receiver_nullable = matches!(tp, Type::Optional(_));
        if let Type::Unknown(_) | Type::Never = tp {
            // @P376 — `Type::Never` is the poison an errored struct construction
            // (`p = Plyer { … }` with an unknown `Plyer`) assigns to its
            // variable.  Recover exactly like an unknown field access but
            // WITHOUT a diagnostic: the real `unknown type '…'` was already
            // reported at the construction, and re-reporting here is the
            // cascade #376 is about.  Returning `tp` (Never) keeps the poison
            // flowing so the enclosing format string / sweep stays silent too.
            if !self.first_pass && matches!(tp, Type::Unknown(_)) {
                diagnostic!(self.lexer, Level::Error, "Field of unknown variable");
            }
            // In the first pass, skip the field name token so parsing continues.
            self.lexer.has_identifier();
            // @P281 — when the dot-access is followed by `(args)`
            // (i.e., `s.method(arg1, arg2)`), the parser must
            // consume the ENTIRE call expression so the surrounding
            // statement parser doesn't trip on the unconsumed `(`
            // and fire a spurious "Expect token ;".  Runs in BOTH
            // passes: pass-1 needs it so the body parses cleanly
            // and pass-2 can re-resolve; pass-2 needs it so the
            // already-emitted "Field of unknown variable" doesn't
            // cascade into "Expect token ;".  Each arg routes
            // through `expression` so nested calls / format strings
            // tokenise correctly.
            if self.lexer.has_token("(") {
                if !self.lexer.peek_token(")") {
                    loop {
                        let mut discard = Value::Null;
                        self.expression(&mut discard);
                        if !self.lexer.has_token(",") {
                            break;
                        }
                    }
                }
                self.lexer.has_token(")");
            }
            // wrap `code` in Value::Drop so an unresolved field access
            // (e.g. `x.v` where x's type is not yet known on pass 1) is no
            // longer treated as a plain `Value::Var(x)` by downstream
            // assignment processing.  Without this wrapping, `x.v = 99` in
            // a function whose `x = callee()` references a struct-returning
            // fn defined LATER in the file collapses to `x = 99` on pass 1
            // (because `.v` is silently dropped) — which sets x's inferred
            // type to integer.  Pass 2 then sees x = integer and rejects
            // the now-resolved `x = callee()` returning the struct.
            // Wrapping in Drop keeps `code != Value::Var(x)` so
            // `assign_var_nr` returns u16::MAX and `change_var` skips the
            // type update.
            *code = Value::Drop(Box::new(code.clone()));
            return tp;
        }
        let mut t = tp;
        // @PLN115 S6 — capture the member name's position for the resolution index,
        // but only when recording: `Position` holds a `String`, so an unconditional
        // clone would tax every field access on a normal compile.
        let field_pos = self
            .record_resolutions
            .then(|| self.lexer.peek_pos().clone());
        let Some(field) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect a field name");
            return t;
        };
        let enr = self.data.type_elm(&t);
        if enr == u32::MAX {
            let shown = t.show(&self.data, &self.vars);
            if let Some(s) = self.suggest_type_name(&shown) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown type {shown} — did you mean '{s}'?"
                );
            } else {
                diagnostic!(self.lexer, Level::Error, "Unknown type {shown}");
            }
            return Type::Unknown(0);
        }
        let e_size = i32::from(self.database.size(self.data.def(enr).known_type()));
        if let Type::RefVar(tp) = t {
            t = *tp;
        }
        // @PLN25 E2 — a METHOD CALL or an S-level field access on a `__nullable<S>`
        // receiver (`v[i].method()` / `v[i].struct_method`): unwrap the receiver to
        // dense `S` (the payload offset-ref, gap 2) so the normal `Reference(S)`
        // field/method dispatch below resolves it.  Two cases:
        //  - a trailing `(` is a method call — `find_poly_enum_field` matches the
        //    method's fn-ref entry copied into `Some` and would read it as a FIELD
        //    (→ "Field access not supported on type fn …"); unwrap takes precedence;
        //  - no `Some`-variant field of that name — an S-level access; unwrap.
        // A plain DATA field of `Some` (no `(`) resolves via `find_poly_enum_field`
        // below and is left untouched here.
        if let Type::Enum(enum_d, true, _) = &t
            && self.data.def(*enum_d).name.starts_with("__nullable<")
            && (self.lexer.peek_token("(") || self.find_poly_enum_field(*enum_d, &field).is_none())
        {
            let enum_d = *enum_d;
            // Resolve the dense `S` def from the `Some` variant's inline `payload` field
            // TYPE — NOT by re-parsing the enum name via `def_nr("S")`, which returns
            // `u16::MAX` for a CROSS-LIB struct (its def is source-qualified), the
            // `Unknown field __nullable<S>.field` regression for a library struct.
            let some_d = self.data.variant_of(enum_d, "Some");
            let payload_attr = self.data.attr(some_d, "payload");
            let struct_d = if payload_attr == usize::MAX {
                u32::MAX
            } else {
                match self.data.attr_type(some_d, payload_attr) {
                    Type::Reference(d, _) => d,
                    _ => u32::MAX,
                }
            };
            if struct_d != u32::MAX && self.data.attributes(struct_d) > 0 {
                if !self.first_pass {
                    // Single-payload form: the dense `S` lives in the `Some` variant's
                    // inline `payload` field, so unwrap to a sub-ref at `payload`'s byte
                    // offset.  That sub-ref IS a valid dense `S` (it shares S's offset
                    // table), so the field/method access below re-dispatches on dense `S`
                    // with no copy.
                    let off = self
                        .database
                        .position(self.data.def(some_d).known_type(), "payload");
                    *code = self.get_val(
                        &Type::Reference(struct_d, crate::data::Deps::none()),
                        false,
                        u32::from(off),
                        code.clone(),
                        u32::MAX,
                    );
                }
                let dep = t.depend();
                let mut new_t = Type::Reference(struct_d, crate::data::Deps::none());
                for on in dep {
                    new_t = new_t.depending(on);
                }
                t = new_t;
            }
        }
        let dnr = self.data.type_def_nr(&t);
        if matches!(t, Type::Vector(_, _)) && self.vector_operations(code, &field, e_size) {
            return Type::Void;
        }
        let fnr = self.data.attr(dnr, &field);
        // @PLN86 P6.4 (F4) — record a sandboxed READ of a host field that carries a
        // `#read` capability link, so admission can gate it.  Reads are default-allow,
        // so only a `#read`-linked field is ever recorded.  Second pass only (the base
        // type + attribute have resolved).  (A write LHS also reaches field(); a write
        // to a host field is independently rejected by 2.4, and a field is rarely both
        // read-linked and written — F5 reworks the write path.)
        if fnr != usize::MAX && self.in_sandbox && !self.first_pass {
            let reads: Vec<String> = self
                .member_access
                .get(&(dnr, field.clone()))
                .map(|links| {
                    links
                        .iter()
                        .filter(|t| t.rsplit_once('#').is_some_and(|(_, r)| r == "read"))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let read_count = reads.len();
            if !reads.is_empty() {
                let pos = self.lexer.peek_pos().clone();
                let entry = self.sandbox_field_reads.entry(self.context).or_default();
                for t in reads {
                    entry.push((t, pos.clone()));
                }
            }
            // @PLN86 F5 — remember this field access so a raw write at the assignment
            // site can resolve which field it targets (and un-record the read just
            // logged above, since a write LHS is not a read).
            self.last_field_target = Some((dnr, field.clone(), read_count));
        }
        // Trace point: field/method dispatch entry state.  Captures
        // what type and field name reached `field()`, whether the
        // attribute was found, and which pass we're on.  Recurring
        // vantage during method-dispatch debugging (plan-17 B).
        // Enable with `LOFT_TRACE=field`.
        crate::loft_trace!(
            field,
            "field={} dnr={} fnr={} t={:?} first_pass={}",
            field,
            dnr,
            if fnr == usize::MAX {
                "MAX".to_string()
            } else {
                fnr.to_string()
            },
            t,
            self.first_pass,
        );
        if fnr == usize::MAX {
            // Plan-17 phase 01 (B) — bounded-T method dispatch must run
            // on BOTH passes so the call's return type propagates into
            // first-pass type inference of the enclosing variable.
            //
            // Before the fix, `s = x.to_text()` (where `x: T`,
            // `T: Printable`) typed `s` as `Type::Unknown(0)` because the
            // first-pass branch (just below) consumed the `(...)` args
            // and returned `Type::Unknown(0)` without dispatching to the
            // t-stub.  The variable's Unknown type then survived second
            // pass (`change_var_type` is a no-op when assigning Unknown),
            // and downstream operators like `s + "!"` rejected with
            // "No matching operator '+' on 'unknown(0)' and 'text'".
            //
            // The t-stub `t_<n>T_<method>` is registered when the
            // function's bounds are declared (definitions.rs:670+).  By
            // the time the body parser reaches `x.to_text()` on first
            // pass, `find_fn(u16::MAX, "to_text", Reference(tv_nr, []))`
            // finds the stub.  Dispatching to it on first pass returns
            // the bound's declared return type (`Type::Text(_)` for
            // Printable's `to_text`), so the surrounding assignment
            // types `s` correctly.
            //
            // Pinned by `tests/issues.rs::plan17_b_bounded_method_return_type_propagates`.
            // Likely also closes plan-17 (A) caveat (implicit
            // generic-tuple type inference) — same root cause.
            // Plan-17 phase 01 (B) — bounded-T method dispatch must run
            // on BOTH passes so the call's return type propagates into
            // first-pass type inference of the enclosing variable.  On
            // second pass the t-stub `t_<n>T_<method>` exists (created
            // in `definitions.rs:670+`); on first pass it doesn't.
            // Both branches end up at `parse_method(stub_nr, t)`; the
            // first-pass branch creates the stub on demand if missing.
            if let Some(_tv_name) = self.generic_type_name(&t) {
                let stub_nr = self.data.find_fn(u16::MAX, &field, &t);
                if stub_nr != u32::MAX
                    && self.has_bound_for_method(&field)
                    && self.lexer.has_token("(")
                {
                    return self.parse_method(code, stub_nr, t.clone());
                }
            }
            // P54 Q3 second half — `instance.to_json()` /
            // `instance.to_json_pretty()` on any user struct (or
            // struct-enum variant) lowers to a single call to
            // `n_struct_to_json(self, struct_kt)` via the schema
            // walker in `Stores::show_json` (`src/database/format.rs`).
            // No per-type stub registration needed; the parser
            // synthesises the call when the receiver is a
            // `Type::Reference(struct_d, _)` and the method name
            // matches.  Mirror of `parse_type_parse` (P54 step 5)
            // for the static `T.parse(JsonValue)` form.
            //
            // Runs on BOTH passes so first-pass type inference of the
            // enclosing variable sees the `Type::Text` return type
            // (same reason the bounded-T fallback above does the
            // same).  On first pass we consume the `()` to make
            // parser progress; the Call population only happens on
            // second pass when known_type / def_nr are stable.
            if (field == "to_json" || field == "to_json_pretty")
                && matches!(t, Type::Reference(_, _))
                && self.lexer.peek_token("(")
            {
                self.lexer.token("(");
                self.lexer.token(")");
                if !self.first_pass {
                    let Type::Reference(struct_d, _) = &t else {
                        unreachable!("matches! above guards Reference shape");
                    };
                    let known_tp = self.data.def(*struct_d).known_type();
                    let n_walker = if field == "to_json" {
                        self.data.def_nr("n_struct_to_json")
                    } else {
                        self.data.def_nr("n_struct_to_json_pretty")
                    };
                    if known_tp != u16::MAX && n_walker != u32::MAX {
                        *code = Value::Call(
                            n_walker,
                            vec![code.clone(), Value::Int(i32::from(known_tp))],
                        );
                    }
                }
                return Type::Text(crate::data::Deps::none());
            }
            // Plan-19 phase 03 — method-on-parent-enum dispatch.  When
            // the receiver is a variant value (`Type::Reference(child_d, …)`
            // for a struct enum variant) and the method is declared on
            // the parent enum (e.g. `fn classify(self: Shape)`), look
            // up `t_<n>Shape_classify` on the parent and dispatch
            // there.  Without this fallback, `s.classify()` where
            // `s = Circle { … }` (inferred type `Reference(Circle)`)
            // rejected with "Unknown field Circle.classify" — even
            // though `s: Shape = Circle { … }` followed by
            // `s.classify()` worked.
            //
            // Runs on both passes so the call's return type propagates
            // into first-pass inference of the enclosing variable
            // (same reason plan-17 (B) bounded-T dispatch runs on both
            // passes).
            //
            // Pinned by `tests/issues.rs::plan19_method_on_enum_variant_via_dot`.
            if let Type::Reference(child_d, _) = &t {
                let parent_d = self.data.def(*child_d).parent();
                if parent_d != u32::MAX && matches!(self.data.def_type(parent_d), DefType::Enum) {
                    let parent_name = self.data.def(parent_d).name().to_string();
                    let stub_name = format!("t_{}{}_{}", parent_name.len(), parent_name, field);
                    let md_nr = self.data.def_nr(&stub_name);
                    // Only fire when `t_<Parent>_<field>` is the
                    // user's direct declaration on the enum, NOT the
                    // auto-generated polymorphic dispatcher built
                    // from per-variant impls (which carries
                    // `synthetic = Some("enum_dispatcher")` —
                    // see `Definition.synthetic` doc).  Without
                    // this guard, `r.area()` on a variant lacking
                    // its own impl would bypass the long-standing
                    // "Unknown field Rect.area" error and silently
                    // dispatch through the warning-only stub.
                    if md_nr != u32::MAX
                        && self.data.def(md_nr).synthetic().is_none()
                        && matches!(
                            self.data.def_type(md_nr),
                            DefType::Function | DefType::Generic
                        )
                        && self.lexer.has_token("(")
                    {
                        return self.parse_method(code, md_nr, t.clone());
                    }
                }
            }
            if self.first_pass && self.lexer.has_token("(") {
                self.skip_remaining_args();
            } else if let Type::Enum(enum_d_nr, true, _) = &t
                && let Some((found_d_nr, found_fnr)) = self.find_poly_enum_field(*enum_d_nr, &field)
            {
                // For polymorphic enums (incl. @PLN25 `__nullable<S>`), this field
                // lives in a VARIANT struct, not the enum itself.  Resolve in BOTH
                // passes: the first pass needs the field TYPE so the receiver var
                // is not left as `Var(receiver)` and then re-typed to the field
                // type by `change_var` (the bug behind `for o in v { f(o.items) }`
                // → "o cannot change type to vector<…>").  `get_field` (which needs
                // the layout's `known_type`, assigned in `fill_all`) emits only in
                // the second pass.
                let dep = t.depend();
                t = self.data.attr_type(found_d_nr, found_fnr);
                for on in dep {
                    t = t.depending(on);
                }
                if let Value::Var(nr) = code {
                    t = t.depending(*nr);
                }
                if self.first_pass {
                    // Type-only: leave a non-Var placeholder so the caller's
                    // `change_var` does not re-type the receiver.
                    *code = Value::Null;
                } else {
                    *code = self.get_field(found_d_nr, found_fnr, code.clone());
                    self.data.attr_used(found_d_nr, found_fnr);
                }
                return t;
            } else if !self.first_pass {
                // map/filter/reduce as method syntax on vectors:
                // v.map(fn) → map(v, fn)
                // Unwrap &vector<T> so map/filter/reduce work on ref params.
                let vec_t = if let Type::RefVar(inner) = &t {
                    inner.as_ref().clone()
                } else {
                    t.clone()
                };
                if matches!(vec_t, Type::Vector(_, _))
                    && matches!(field.as_str(), "map" | "filter" | "reduce")
                    && self.lexer.has_token("(")
                {
                    return self.parse_vector_method(code, &vec_t, &field);
                }
                // generic-specific error for field access on T.
                if let Some(tv_name) = self.generic_type_name(&t) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "generic type {tv_name}: field access requires a concrete type",
                    );
                } else {
                    // INC#8 / QUALITY 6c: if a free function `n_<field>` exists
                    // whose first parameter is compatible with the receiver
                    // type, tell the user to call it as a free function
                    // instead of as a method.  The stdlib chooses per
                    // function whether it's `self:` / `both:` / free-only;
                    // readers who don't know that land on "Unknown field
                    // vector.sum_of" without a hint.
                    let free_nr = self.data.def_nr(&format!("n_{field}"));
                    let has_free_hint = free_nr != u32::MAX
                        && !self.data.def(free_nr).attributes().is_empty()
                        && self.data.attr_type(free_nr, 0).is_equal(&t);
                    if has_free_hint {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown field {}.{field} — did you mean the free function `{field}(…)` ? (stdlib declared `{field}` as free-only; see LOFT.md § Methods and function calls)",
                            self.data.def(dnr).name()
                        );
                    } else if let Some(s) = self.suggest_field_name(dnr, &field) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown field {}.{field} — did you mean '{s}'?",
                            self.data.def(dnr).name()
                        );
                        self.lexer.suggest_last(&s);
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown field {}.{field}",
                            self.data.def(dnr).name()
                        );
                    }
                }
                // Consume a trailing `(…)` to avoid cascading parse errors.
                if self.lexer.has_token("(") {
                    self.skip_remaining_args();
                }
            }
            return Type::Unknown(0);
        }
        // @PLN115 S6 — record the resolved member reference at the member name: a
        // Routine attribute is a METHOD, any other attribute a FIELD.  So `text.len`
        // records Method{text, len} and `p.x` records Field{P, x} — a same-spelled
        // member of another type, or a local of the same name, is thereby excluded.
        if !self.first_pass
            && let Some(fp) = &field_pos
        {
            let len = field.chars().count() as u16;
            match self.data.attr_type(dnr, fnr) {
                Type::Routine(r_nr) => self.record(
                    fp,
                    len,
                    crate::resolution::Resolution::Method {
                        recv_type: dnr,
                        method_def: r_nr,
                    },
                ),
                _ => self.record(
                    fp,
                    len,
                    crate::resolution::Resolution::Field {
                        type_def: dnr,
                        attr: fnr as u16,
                    },
                ),
            }
        }
        if let Type::Routine(r_nr) = self.data.attr_type(dnr, fnr) {
            if self.lexer.has_token("(") {
                t = self.parse_method(code, r_nr, t.clone());
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect call of method {}.{}",
                    self.data.def(dnr).name(),
                    self.data.attr_name(dnr, fnr)
                );
            }
        } else if self.data.def(dnr).attributes()[fnr].constant {
            let expr = self.data.attr_value(dnr, fnr);
            // B2-runtime (2026-04-13): `Sig.Idle` on a mixed struct-enum
            // parent resolves `expr` to a bare `Value::Enum(disc, _)` —
            // same type mismatch as the unqualified `Idle` form.  Wrap in
            // the `OpDatabase` + `object_init` record-allocation sequence
            // used by `parse_constant_value` so `var_s: DbRef = …` gets a
            // proper DbRef, not a u8 byte.
            let parent_is_mixed = matches!(self.data.def(dnr).returned(), Type::Enum(_, true, _));
            if parent_is_mixed && !self.first_pass && matches!(expr, Value::Enum(_, _)) {
                let variant_name = self.data.attr_name(dnr, fnr);
                let variant_d_nr = self.data.def_nr(&variant_name);
                if variant_d_nr != u32::MAX && self.data.def(variant_d_nr).known_type() != u16::MAX
                {
                    let ret = self.data.def(dnr).returned().clone();
                    let w = self.vars.work_refs(&ret, &mut self.lexer);
                    let known_type = i32::from(self.data.def(variant_d_nr).known_type());
                    let mut list = Vec::new();
                    list.push(crate::data::v_set(w, Value::Null));
                    list.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(known_type)]));
                    self.object_init(
                        &mut list,
                        variant_d_nr,
                        0,
                        &Value::Var(w),
                        &std::collections::HashSet::new(),
                    );
                    list.push(Value::Var(w));
                    // Mirror the unqualified-form path in parser/objects.rs:
                    // the LHS of assignment owns the store (empty dep); the
                    // work-ref is skip_free so it isn't double-freed.  With
                    // `vec![w]` the LHS got `dep=[__ref_N]` which made it a
                    // borrower — nothing freed the store.
                    self.vars.set_skip_free(w);
                    *code = crate::data::v_block(
                        list,
                        Type::Enum(dnr, true, crate::data::Deps::none()),
                        "EnumUnitLit",
                    );
                    self.data.attr_used(dnr, fnr);
                    return Type::Enum(dnr, true, crate::data::Deps::none());
                }
            }
            *code = Self::replace_record_ref(expr, &code.clone());
            let dep = t.depend();
            t = self.data.attr_type(dnr, fnr);
            for on in dep {
                t = t.depending(on);
            }
        } else {
            let dep = t.depend();
            t = self.data.attr_type(dnr, fnr);
            for on in dep {
                t = t.depending(on);
            }
            if let Value::Var(nr) = code {
                t = t.depending(*nr);
            }
            *code = self.get_field(dnr, fnr, code.clone());
            // Plan-07 phase 4h — count this read for the
            // `not null` field-reminder hint.  Skip stdlib (the
            // suggestion target is user code), skip first pass
            // (counts must be stable; first pass isn't), skip
            // already-`not null` fields (no recommendation
            // possible), skip method-routine reads (constant
            // branch above handled those), and skip when fnr is
            // out of range (defensive — shouldn't happen here).
            if !self.first_pass
                && !self.default
                && fnr != usize::MAX
                && fnr < self.data.def(dnr).attributes().len()
                && self.data.def(dnr).attributes()[fnr].nullable
            {
                let key = (dnr, fnr as u32);
                *self.field_read_counts.entry(key).or_insert(0) += 1;
                // Record this site so `handle_null_coalesce` can
                // mark it defended when `??` follows immediately
                // (`p.field ?? default`).
                self.last_field_read_site = Some(key);
            }
        }
        // A field of a NULLABLE receiver can itself be null (C80), regardless of the
        // field's declared non-nullness — so it is not "not null" for the redundant-
        // check / redundant-coalesce lints.  (Only the lint signal is cleared; the
        // returned type `t` is unchanged — widening it to `Optional` would force
        // `?? d` on every nullable-receiver field read, a far broader change.)
        if receiver_nullable && self.expr_not_null {
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
        }
        self.data.attr_used(dnr, fnr);
        t
    }

    /// Consume remaining function call arguments after `(` has already been consumed.
    /// Handle `v.map(fn)` / `v.filter(fn)` / `v.reduce(fn)` method syntax.
    fn parse_vector_method(&mut self, code: &mut Value, t: &Type, method: &str) -> Type {
        let mut list = vec![code.clone()];
        let mut types = vec![t.clone()];
        let mut m_arg_idx = 1usize;
        loop {
            if let Type::Vector(elm, _) = t {
                let elem = *elm.clone();
                let hint = match (method, m_arg_idx) {
                    ("map", 1) => Some(Type::Function(
                        vec![elem.clone()],
                        Box::new(elem),
                        crate::data::Deps::none(),
                    )),
                    ("filter", 1) => Some(Type::Function(
                        vec![elem],
                        Box::new(Type::Boolean),
                        crate::data::Deps::none(),
                    )),
                    // @P288 — `v.reduce(init, |acc, x| {…})`: the lambda is
                    // ARG 2 (init is arg 1), so the hint goes on m_arg_idx == 2.
                    // Both lambda params take the vector's element type; the
                    // accumulator inherits the init's type but the inference
                    // here uses elm uniformly because every primitive case
                    // (sum, max, min, count) keeps acc and elm in the same
                    // numeric domain.  Heterogeneous reduce can still use the
                    // free-function form which supplies explicit types.
                    ("reduce", 2) => Some(Type::Function(
                        vec![elem.clone(), elem.clone()],
                        Box::new(elem),
                        crate::data::Deps::none(),
                    )),
                    _ => None,
                };
                if let Some(h) = hint {
                    self.expected = h;
                }
            }
            let mut p = Value::Null;
            let pt = self.expression(&mut p);
            self.expected = Type::Unknown(0);
            list.push(p);
            types.push(pt);
            m_arg_idx += 1;
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        match method {
            "map" => self.parse_map(code, &list, &types),
            "filter" => self.parse_filter(code, &list, &types),
            "reduce" => self.parse_reduce(code, &list, &types),
            _ => unreachable!(),
        }
    }

    pub(crate) fn skip_remaining_args(&mut self) {
        loop {
            if self.lexer.peek_token(")") {
                break;
            }
            let mut p = Value::Null;
            self.expression(&mut p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
    }

    /// Search for `field` in the variant structs of a polymorphic enum.
    /// Returns `(variant_d_nr, attr_nr)` if found.
    pub(crate) fn find_poly_enum_field(&self, enum_d_nr: u32, field: &str) -> Option<(u32, usize)> {
        for a_nr in 0..self.data.attributes(enum_d_nr) {
            let a_name = self.data.attr_name(enum_d_nr, a_nr);
            // @PLN22 Phase 1 — resolve the variant within its enum via the
            // variant_of chokepoint, not the bare global def_nr.
            let variant_d_nr = self.data.variant_of(enum_d_nr, &a_name);
            if variant_d_nr == u32::MAX {
                continue;
            }
            let f = self.data.attr(variant_d_nr, field);
            if f != usize::MAX {
                return Some((variant_d_nr, f));
            }
        }
        None
    }

    pub(crate) fn vector_operations(&mut self, code: &mut Value, field: &str, e_size: i32) -> bool {
        if field == "remove" {
            self.lexer.token("(");
            let (tps, ls) = self.parse_parameters();
            let mut cd = ls[0].clone();
            // validate types
            if tps.len() != 1 || !self.convert(&mut cd, &tps[0], &I32) {
                diagnostic!(self.lexer, Level::Error, "Invalid index in remove");
            }
            *code = self.cl("OpRemoveVector", &[code.clone(), Value::Int(e_size), cd]);
            true
        } else {
            false
        }
    }

    pub(crate) fn parse_index(&mut self, code: &mut Value, tp: &Type) -> Type {
        let mut t = tp.clone();
        let mut p = Value::Null;
        self.un_ref(&mut t, &mut p);
        // @PLN25 — index/slice dispatch peels an `Optional` receiver (`s: text?;
        // s[i]`, `s[a..b]`) to its base, mirroring method dispatch which already
        // peels (`s.starts_with(..)` works on a `text?`). Inert gate-OFF: no
        // `Optional` is ever constructed, so `.base()` is a no-op. A null-check
        // discharges the value; indexing the null sentinel behaves as gate-OFF.
        t = t.base().clone();
        let mut elm_type = self.index_type(&t);
        for on in t.depend() {
            elm_type = elm_type.depending(on);
        }
        /*let nr = if self.types.exists("$") {
            self.types.var_nr("$")
        } else {
            self.create_var("$".to_string(), elm_type.clone())
        };
        self.data.definitions[self.context as usize].variables[nr as usize].uses = 0;
         */
        if let Type::Vector(etp, _) = &t {
            let iter_value = self.parse_vector_index(code, &elm_type, etp);
            // A `v[i]` read is nullable — an out-of-bounds index yields the
            // null sentinel (exactly what the OOB defensive-check warns
            // about).  Clear `expr_not_null` (mirroring the keyed-collection
            // arms below, @P285) so `v[i] ?? default` is NOT flagged
            // "Redundant null coalescing" even when the vector's element
            // type — or the vector field itself — is `not null`.  Without
            // this, indexing a `not null` vector hit two contradictory
            // checks: bare `v[i]` warned OOB while `v[i] ?? d` warned
            // redundant, leaving no clean idiom.  Covers BOTH the scalar
            // path (`parse_vector_index` returns `None`, having mutated
            // `code`) and the iterator path (returns `Some`).
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
            if let Some(value) = iter_value {
                return value;
            }
            // @PLN25 DN3 (index): a scalar `v[i]` read is nullable (OOB → the null sentinel) unless
            // the index is provably in-bounds (`parse_vector_index` set `last_index_fit` from a
            // constant / for-loop iter var / `if idx < len(v)` guard). Wrap the element type
            // `Optional` so `(N-Store)` forces a `?? d` / `τ?` slot / guard at the store site.
            // The F1a landing (deps Optional-transparency + the corpus/lib migrations) cleared the
            // blast radius, so this is now folded into the DN1 default (was DEV-GATED `LOFT_INDEX_DEV`).
            if crate::keys::pln25_dn1_enabled() && !self.last_index_fit {
                elm_type = Type::optional(elm_type);
            }
        } else if matches!(t, Type::Text(_)) {
            let index_t = if self.lexer.peek_token("..") {
                p = Value::Int(0);
                I32.clone()
            } else {
                self.expression(&mut p)
            };
            if self.parse_text_index(code, &mut p, &index_t) == Type::Character {
                elm_type = Type::Character;
            }
        } else if let Type::Hash(el, keys, _) | Type::Radix(el, keys, _) = &t {
            // @PLN25 E2 — key fields live in the `Some` variant when the element was
            // rewritten to `__nullable<S>`; resolve names against the key-bearing def.
            let el = crate::typedef::key_bearing_def(&self.data, *el);
            let mut key_types = Vec::new();
            for k in keys {
                key_types.push(self.data.attr_type(el, self.data.attr(el, k)).clone());
            }
            // @PLN48 S3 — a `spatial` RANGE SLICE `xs[(fx,fy)..(tx,ty)]` /
            // `xs[(fx,fy)..:n]` / `xs[(fx,fy)..]`: iterate the records whose Morton
            // code is in the interval (a bounding box is the raw code interval), in
            // natural order.  Lowers to `n_spatial_range(xs, tp, fx, fy, has_till, tx,
            // ty, limit)` — the same scratch path as iteration — and returns the Radix
            // type so `parse_for` iterates the already-built scratch.  A `(` opens the
            // coordinate tuple.
            if matches!(t, Type::Radix(_, _, _)) && self.lexer.peek_token("(") {
                elm_type = self.parse_spatial_slice(code, &t, &key_types);
            } else {
                self.parse_key(code, &t, &key_types);
            }
            // @P285 — a keyed-collection lookup RESULT is nullable (an absent
            // key returns the null record).  `parse_key` parsed the KEY last,
            // so `expr_not_null` still reflects the key (e.g. a `not null`
            // field) — clear it so a following `lookup == null` membership
            // test doesn't fire a bogus "Redundant null check" attributed to
            // the key.
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
        } else if let Type::Sorted(el, keys, _) | Type::Index(el, keys, _) = &t {
            let el = crate::typedef::key_bearing_def(&self.data, *el);
            let mut key_types = Vec::new();
            for (k, _) in keys {
                key_types.push(self.data.attr_type(el, self.data.attr(el, k)).clone());
            }
            self.parse_key(code, &t, &key_types);
            // @P285 — see the Hash/Radix arm above; the lookup result is nullable.
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
        } else if t.is_unknown() {
            // @P278/P281 — pass-1 Unknown receiver: consume the
            // entire bracket content including range syntax
            // (`a..b`, `a..=b`, `a..`, `..b`, `..`) so the outer
            // caller's `]` consumption matches and no spurious
            // "Expect token ]" / "Expect token ;" cascade fires.
            // Pass-2 (with all defs registered) re-parses the body
            // cleanly and dispatches correctly — or emits the real
            // diagnostic if the receiver is genuinely unknown.
            if !self.lexer.peek_token("]") {
                if !self.lexer.peek_token("..") && !self.lexer.peek_token("..=") {
                    let mut p = Value::Null;
                    self.expression(&mut p);
                }
                if (self.lexer.has_token("..") || self.lexer.has_token("..="))
                    && !self.lexer.peek_token("]")
                {
                    let mut p2 = Value::Null;
                    self.expression(&mut p2);
                }
            }
        } else {
            // index_type() already emitted a diagnostic; consume the inner expression
            // so that the caller can still parse the closing `]` without cascading errors.
            let mut p = Value::Null;
            self.expression(&mut p);
        }
        elm_type
    }

    /// @PLN25 DN3 (index) — is a vector index provably in-bounds, so `v[i]` cannot be OOB-null?
    /// True for a non-negative constant literal (the developer typed it), a for-loop iteration
    /// variable (`for i in <range> { v[i] }` — the loop's bound is the contract), or (@PLN102 D1)
    /// an integer-arithmetic index built purely from those trusted leaves (`m[k*4+row]` — the
    /// matrix-indexing contract). Everything else (a general var, an index touching an untrusted
    /// var) can overrun → the read types `τ?`. Mirrors the pass-2 warning walk's skip patterns 2/3;
    /// the `i < len(v)` guard (pattern 5) is added separately.
    fn index_provably_fit(&self, index: &Value, vec: &Value) -> bool {
        // A compile-time-constant index — positive OR negative (`v[-1]` is the Python-style
        // last-element idiom; `-1` lowers to a negation, so use `const_int`, not a literal match)
        // — is the developer's explicit contract: trust it, exactly as the runtime does (a genuine
        // overrun still raises the recoverable OOB fault). A variable index carries no such
        // contract, so it stays `τ?` unless proven fit below.
        if self.const_int(index).is_some() {
            return true;
        }
        match index.unspan() {
            Value::Var(v) => {
                // A for-loop iteration variable (`for i in <range> { v[i] }`) — the vars system
                // already tracks the active loop stack, so no separate parse-time stack is needed.
                if self.vars.is_active_loop_var(*v) {
                    return true;
                }
                // An `if idx < len(vec) { vec[idx] }` guard proved the (idx, vec) pair in-bounds
                // for this branch — match `v` AND the indexed vector's `VecKey`.
                crate::parser::operators::vec_key(vec, &self.data).is_some_and(|vk| {
                    self.index_bounded
                        .iter()
                        .any(|(iv, ik)| *iv == *v && *ik == vk)
                })
            }
            // @PLN102 D1 — a computed index like `m[k*4+row]` is the matrix-indexing contract: an
            // integer-arithmetic tree over constants and active loop vars. Trust it exactly as a
            // bare loop var (a real OOB still faults → null at runtime, C80). This deliberately does
            // NOT thread the `i < len(v)` guard through arithmetic — that proof is specific to
            // `v[i]` and does not survive `v[i*2]` — so `index_arith_trusted` reads only the two
            // by-contract leaves, never `index_bounded`.
            _ => self.index_arith_trusted(index),
        }
    }

    /// @PLN102 D1 — is `index` an integer-arithmetic expression built purely from trusted leaves
    /// (constants and active for-loop iteration variables)? The recursive half of
    /// `index_provably_fit` for computed matrix/vector indices (`k * 4 + row`, `col * stride + k`).
    /// A leaf is a constant (`const_int`) or an active loop var; a node is one of the integer
    /// arithmetic ops. Any other var (a plain local, a guard-bounded var) or non-arithmetic call
    /// (`len(w)`, `f(i)`) breaks the chain → `false`, keeping the read `τ?`.
    fn index_arith_trusted(&self, index: &Value) -> bool {
        if self.const_int(index).is_some() {
            return true;
        }
        match index.unspan() {
            Value::Var(v) => self.vars.is_active_loop_var(*v),
            Value::Call(op, args) if self.is_index_arith_op(*op) => {
                args.iter().all(|a| self.index_arith_trusted(a))
            }
            _ => false,
        }
    }

    /// @PLN102 D1 — is `op` one of the integer/long arithmetic operators an index expression can be
    /// composed from (`+ - * / %`)? Bitwise / shift / comparison ops are intentionally excluded:
    /// the trusted set is ordinary index arithmetic, nothing wider.
    fn is_index_arith_op(&self, op: u32) -> bool {
        matches!(
            self.data.def(op).name.as_str(),
            "OpAddInt"
                | "OpMinInt"
                | "OpMulInt"
                | "OpDivInt"
                | "OpModInt"
                | "OpAddLong"
                | "OpMinLong"
                | "OpMulLong"
                | "OpDivLong"
                | "OpModLong"
        )
    }

    pub(crate) fn index_type(&mut self, t: &Type) -> Type {
        if let Type::Vector(v_t, _) = t {
            *v_t.clone()
        } else if let Type::Sorted(d_nr, _, _)
        | Type::Hash(d_nr, _, _)
        | Type::Index(d_nr, _, _)
        | Type::Radix(d_nr, _, _) = t
        {
            let ret = self.data.def(*d_nr).returned().clone();
            // S16b: struct-enum variants have .returned = Type::Enum(parent, true, []).
            // For collection element access we need Type::Reference(variant_def_nr, [])
            // so that field access and range-query for-loops resolve fields against the
            // variant struct (not the parent enum), and for_type() can map the element type.
            if matches!(ret, Type::Enum(_, true, _)) {
                // @PLN25 E2 — a synth `__nullable<S>` element keeps its `Enum`
                // type (here `d_nr` IS the enum, not a variant), exactly as a
                // `vector<__nullable<S>>` element does, so the field-access
                // unwrap in `field()` resolves S's fields through `Some` and a
                // `lookup[k] == null` test works. Converting it to `Reference`
                // would point at the enum (no payload fields) → "Unknown field".
                if self.data.def(*d_nr).name.starts_with("__nullable<") {
                    ret
                } else {
                    Type::Reference(*d_nr, crate::data::Deps::none())
                }
            } else {
                ret
            }
        } else if matches!(t, Type::Text(_)) {
            t.clone()
        } else if let Type::RefVar(tp) = t {
            *tp.clone()
        } else if t.is_unknown() {
            // First pass: type not yet resolved; suppress error until second pass.
            Type::Unknown(0)
        } else {
            // QUALITY 6d: the "Indexing a non vector" message fires for two
            // very different user intents — real misuse of `[..]` on a
            // scalar, and an attempted generic-constructor
            // (`hash<Row[id]>()`, `sorted<Elm[k]>()`) that the language
            // doesn't support.  The second case leaves readers stuck; point
            // at the type-annotated local idiom that *does* work (a struct
            // field works too, but the local form is usually what they want).
            diagnostic!(
                self.lexer,
                Level::Error,
                "Indexing a non vector — keyed collections (hash/sorted/index/spatial) have no generic-constructor expression; name the key via a type annotation and initialise from a vector literal: `h: hash<Row[id]> = [Row {{ id: 1 }}];` (a struct field `struct Db {{ h: hash<Row[id]> }}` works too)"
            );
            Type::Unknown(0)
        }
    }

    pub(crate) fn parse_vector_index(
        &mut self,
        code: &mut Value,
        elm_type: &Type,
        etp: &Type,
    ) -> Option<Type> {
        let mut p = Value::Null;
        let index_t = self.parse_in_range(&mut p, code, "$");
        // @PLN25 — a nullable `τ?` INDEX is ACCEPTED (not an (N-Store) violation): `v[i]`
        // is already `τ?` (out-of-bounds → null), so the caller must null-check the result
        // regardless, and a null index just propagates to that null result. (N-Store) governs
        // storing null INTO a non-null slot (decl/field/return/typed-store), not passing a
        // nullable THROUGH an op whose result stays honestly nullable. So no rejection here.
        // Pass-1 deferral: a `vector<S>` whose element `S` is not yet registered
        // (forward-referenced or cross-package, e.g. `vector<WallDef>` indexed
        // before `WallDef` is parsed) yields `type_elm == u32::MAX`, which would
        // panic the `def(elm_td)` below.  `etp` is still `Unknown` here, so the
        // op-dispatch matches (Tuple/Function/base) all fall through to the
        // generic linked-handle read.  Substitute the builtin `reference` def so
        // a placeholder read (stride + `OpVectorRef`) builds and the caller's
        // index chain / assignment stays well-formed; pass-2 sees the resolved
        // element, skips this branch, and rebuilds the real op + stride.  Only
        // reachable in pass-1 (pass-2 has every def); a genuinely-undefined
        // element surfaces its error at the type declaration, not here.
        let elm_td = match self.data.type_elm(etp) {
            u32::MAX => self.data.source_nr(0, "reference"),
            td => td,
        };
        let known = self.data.def(elm_td).known_type();
        // honour narrow vector-element stride when the
        // content Type::Integer carries a forced_size AND Phase 2 would
        // register a direct-encoded narrow type (see
        // `IntegerSpec::vector_narrow_width` — currently 1 and 4 bytes).
        // Shorts stay wide until Phase 4 aligns the `Parts::Short`
        // encoding with raw-byte copies.  Falls back to the
        // bounds-heuristic via `database.size(known_type)` otherwise.
        let elm_size_raw = if let Type::Integer(spec) = etp
            && let Some(n) = spec.vector_narrow_width()
        {
            i32::from(n)
        } else if let Some(narrow) = self.data.narrow_vector_content(etp, &mut self.database) {
            // P214: Type::Function vector elements route through
            // `narrow_vector_content` to a `database.int(0, false)`
            // (size 4 d_nr storage).  The previous fallback via
            // `data.def(elm_td).known_type` returned `u16::MAX` for
            // synthetic `i32` defs without a registered known_type,
            // making `database.size(known) = 0` and producing a
            // stride-0 read that always hit slot 0 regardless of
            // index.
            i32::from(self.database.size(narrow))
        } else {
            i32::from(self.database.size(known))
        };
        // @PLAN58 cluster-I (boolean outer-handle stride): a vector-typed element
        // is a 4-byte rec-id HANDLE.  The bounds-heuristic above yields the inner
        // scalar size — fine when ≥4, but a 1-byte `boolean` inner makes adjacent
        // handles overlap on read.  Clamp to ≥4, matching the construction-side
        // `known` fix in `new_record`.  A no-op for ≥4 strides; no classification
        // change (`known` / is_base / is_linked / deref type are untouched).
        let elm_size = if matches!(elm_type, Type::Vector(_, _)) {
            elm_size_raw.max(4)
        } else {
            elm_size_raw
        };
        if let Value::Iter(var, init, next, extra_init) = p {
            if matches!(*next, Value::Block(_)) {
                // Plan-07 phase 4 step 4.6 — this is the for-loop
                // iteration branch (`Value::Iter` carrying the loop's
                // step block).  Use the *Nullable* peers so OOB at
                // end-of-iteration returns a null DbRef instead of
                // raising — matches the loop-driver expectation
                // documented at parser/collections.rs:1492-1499.
                // The explicit `v[i]` branch below (line 629-640) is
                // unchanged and emits the raising OpGetVector / OpVectorRef.
                //
                // Linked structs: array stores 4-byte record pointers → use OpVectorRefNullable
                // which internally uses elm_size=4 and dereferences to the actual record.
                // Base/primitive types: array stores inline values → use OpGetVectorNullable + get_val.
                let op = if self.database.is_linked(known) {
                    self.cl("OpVectorRefNullable", &[code.clone(), *next.clone()])
                } else {
                    let mut v = self.cl(
                        "OpGetVectorNullable",
                        &[code.clone(), Value::Int(elm_size), *next.clone()],
                    );
                    if self.database.is_base(known) {
                        // Pass the ELEMENT's declared nullability (its slot
                        // reserves a sentinel iff `τ?`), NOT the OOB-nullable
                        // read result — a nullable narrow element decodes via
                        // its sentinel op, a `vector<u8>` element stays raw.
                        v = self.get_val(etp, matches!(etp, Type::Optional(_)), 0, v, u32::MAX);
                    } else if let Type::Tuple(elems) = etp {
                        v = self.unbox_tuple_from_dbref(v, elems);
                    }
                    v
                };
                *code = Value::Iter(
                    var,
                    init,
                    Box::new(v_block(vec![op], etp.clone(), "Vector Index")),
                    extra_init,
                );
                return Some(Type::Iterator(
                    Box::new(elm_type.clone()),
                    Box::new(Type::Null),
                ));
            }
            diagnostic!(self.lexer, Level::Error, "Invalid iterator expression");
            return None;
        }
        // @PLN25 DN3 (index): decide this scalar read's element nullability. A `v[i]` is nullable
        // (OOB → the null sentinel) UNLESS the index is provably in-bounds — a non-negative constant
        // (the developer typed a literal), or a for-loop iteration variable (the loop's bound is the
        // contract). `parse_index` reads this flag to wrap the element type `Optional` when unfit.
        self.last_index_fit = self.index_provably_fit(&p, code);
        // @PLN102 strict-index lint (gated by `LOFT_LINT_STRICT_INDEX`, advisory). The index is
        // trusted in-bounds because it is a for-loop iter var — but if that loop is bounded by
        // `len(<other vector>)`, `for i in 0..len(v) { w[i] }` types `w[i]` non-null yet reads
        // C80-null on overrun. Warn on the mismatch; the type stays non-null (a real proof would
        // break the ubiquitous `for i in 0..n { v[i] }` idiom — see `strict_index_lint_enabled`).
        if self.last_index_fit
            && !self.first_pass
            && crate::keys::strict_index_lint_enabled()
            && let Value::Var(iv) = p.unspan()
            && let Some(bound) = self.vars.loop_len_bound(*iv)
            && let Some(indexed) = crate::parser::operators::vec_key(code, &self.data)
            && bound != indexed
        {
            let iname = self.vars.name(*iv).to_string();
            diagnostic!(
                self.lexer,
                Level::Warning,
                "index `{iname}` is bounded by `len(...)` of a different vector than the one \
                 indexed here — a mismatched-vector index is typed non-null but reads null on \
                 overrun (@PLN102 strict-index)"
            );
        }
        if !self.first_pass && !self.convert(&mut p, &index_t, &I32) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Invalid index type {} on vector",
                index_t.show(&self.data, &self.vars)
            );
        }
        // Linked structs: array stores 4-byte record pointers → OpVectorRef dereferences correctly.
        // Base/primitive types: inline data → OpGetVector + get_val reads the primitive value.
        // Plain inline structs: OpGetVector only (field access happens at the next level).
        // Tuple elements: OpGetVector + per-field reads via the DbRef (P189b — see
        // `unbox_tuple_from_dbref`).  Without this the assignment `p = pairs[0]`
        // wrote DbRef bytes into the tuple slot and `p.0` / `p.1` decoded them
        // as garbage integers.
        if self.database.is_linked(known) {
            *code = self.cl("OpVectorRef", &[code.clone(), p]);
        } else {
            *code = self.cl("OpGetVector", &[code.clone(), Value::Int(elm_size), p]);
            if self.database.is_base(known) {
                // Element's DECLARED nullability drives the sentinel decode (see
                // the Nullable-iterator branch above); the OOB-nullable read
                // result is orthogonal (the caller null-checks `v[i]` regardless).
                *code = self.get_val(
                    etp,
                    matches!(etp, Type::Optional(_)),
                    0,
                    code.clone(),
                    u32::MAX,
                );
            } else if let Type::Tuple(elems) = etp {
                *code = self.unbox_tuple_from_dbref(code.clone(), elems);
            } else if matches!(etp, Type::Function(_, _, _)) {
                // P214: vector elements of `fn(...) -> ...` type are
                // stored as 4-byte d_nr only (non-capturing — capturing
                // closures in vectors are deferred).  The variable's
                // stack representation is `(u32, DbRef)` (a fn-ref
                // tuple), so the read assembles the tuple from the
                // 4B slot d_nr + a null closure sentinel.  Mirrors the
                // struct-field non-capturing path in
                // `parser/mod.rs::get_val` Type::Function arm but uses
                // an explicit `OpNullRefSentinel` for the closure half
                // since vectors don't have a separate `__closure_rec`
                // sub-field.
                let slot = code.clone();
                let read_dnr = self.cl("OpGetInt4", &[slot, Value::Int(0)]);
                let read_clos = self.cl("OpNullRefSentinel", &[]);
                *code = crate::data::v_block(
                    vec![read_dnr, read_clos],
                    etp.clone(),
                    // Reuse the field-read block name so native codegen's
                    // tuple-emit shortcut (`((d_nr) as u32, closure_DbRef)`)
                    // fires here too — the block layout is identical.
                    "fn_ref_field_read",
                );
            }
        }
        None
    }

    /// P189b: assemble a stack-tuple from a DbRef pointing to an
    /// inline tuple in vector storage.
    ///
    /// `dbref` is a Value that, at runtime, evaluates to a `DbRef`
    /// pointing at the start of an inline tuple's bytes (typically
    /// `OpGetVector(pairs, stride, idx)`).  This helper stashes the
    /// DbRef in a fresh work-ref of type `Reference(__tuple<…>)` so
    /// per-element loads (`OpGetInt(dbref, off)`, `OpGetText(dbref,
    /// off)`, etc. via `get_val`) read at the right field offsets,
    /// then wraps the per-element loads in `Value::Tuple` so the
    /// caller's `Set(p, …)` produces the correct stack-tuple
    /// representation (each element pushed onto contiguous slots).
    ///
    /// Heap-vs-stack layout differs for `text` and reference fields
    /// (heap stores a 4-byte interned pointer; stack uses a 16-byte
    /// `Str` / `DbRef`).  `get_val`'s per-type dispatch handles the
    /// inflation correctly because each type uses the same opcodes
    /// the struct-field path uses (`OpGetText` reads a 4-byte heap
    /// text pointer and pushes a 16-byte `Str` onto the stack).
    pub(crate) fn unbox_tuple_from_dbref(&mut self, dbref: Value, elems: &[Type]) -> Value {
        let elems_vec = elems.to_vec();
        let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
        let ref_tp = Type::Reference(tuple_d_nr, crate::data::Deps::none());
        let tmp = self.vars.work_refs(&ref_tp, &mut self.lexer);
        if !self.first_pass {
            self.change_var_type(tmp, &ref_tp);
        }
        // Stored tuples MUST use the synthetic `__tuple<…>` struct's
        // post-finish field positions (the same offsets used by
        // `OpGetInt` for ordinary struct fields).  Falls back to the
        // alignment-aware `element_offsets` only on early-parse paths
        // before `finish_type` has run.
        let offsets: Vec<u16> = crate::data::stored_tuple_offsets_for_def(
            &self.data,
            &self.database,
            tuple_d_nr,
            elems_vec.len(),
        )
        .unwrap_or_else(|| {
            crate::data::element_stack_offsets(&elems_vec)
                .into_iter()
                .map(|x| x as u16)
                .collect()
        });
        let mut tuple_elems = Vec::new();
        for (i, et) in elems_vec.iter().enumerate() {
            let off = u32::from(offsets[i]);
            tuple_elems.push(self.get_val(et, false, off, Value::Var(tmp), u32::MAX));
        }
        v_block(
            vec![v_set(tmp, dbref), Value::Tuple(tuple_elems)],
            Type::Tuple(elems_vec),
            "tuple_unbox",
        )
    }

    pub(crate) fn parse_text_index(
        &mut self,
        code: &mut Value,
        p: &mut Value,
        index_t: &Type,
    ) -> Type {
        // @PLN110 3a — snapshot the raw index BEFORE `convert` may wrap it, so the
        // strict-index lint below can still recognise a bare loop variable.
        let raw_index = p.unspan().clone();
        if !self.convert(p, index_t, &I32) {
            diagnostic!(self.lexer, Level::Error, "Invalid index on string");
        }
        let mut other = Value::Null;
        if self.lexer.has_token("..") {
            let incl = self.lexer.has_token("=");
            if self.lexer.peek_token("]") {
                *code = self.cl(
                    "OpGetTextSub",
                    &[code.clone(), p.clone(), Value::Int(i32::MAX)],
                );
            } else {
                let ot_type = self.expression(&mut other);
                if !self.convert(&mut other, &ot_type, &I32) {
                    diagnostic!(self.lexer, Level::Error, "Invalid index on string",);
                }
                if incl {
                    other = self.cl("OpAddInt", &[other.clone(), Value::Int(1)]);
                }
                *code = self.cl("OpGetTextSub", &[code.clone(), p.clone(), other]);
            }
            Type::Text(crate::data::Deps::none())
        } else {
            // @PLN110 3a — `for i in 0..len(s) { s[i] }` is a units error: `len(s)` is a
            // CHARACTER count but `s[i]` is byte-indexed, so the loop walks char-count byte
            // positions and under-runs / misreads multi-byte text. Warn (default-on) when the
            // index is a loop var bounded by `len` of the SAME text being indexed. Advisory —
            // iterate with `for c in s`, or use `0..size(s)` for a byte walk.
            if !self.first_pass
                && crate::keys::text_index_units_lint_enabled()
                && let Value::Var(iv) = &raw_index
                && let Some(bound) = self.vars.loop_len_bound(*iv)
                && crate::parser::operators::vec_key(code, &self.data) == Some(bound)
            {
                let iname = self.vars.name(*iv).to_string();
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "index `{iname}` walks `0..len(text)` (a character count) but `text[{iname}]` \
                     is byte-indexed — this under-runs / misreads multi-byte text; iterate with \
                     `for c in text`, or use `0..size(text)` for a byte walk (@PLN110 strict-index)"
                );
            }
            *code = self.cl("OpTextCharacter", &[code.clone(), p.clone()]);
            Type::Character
        }
    }

    /// @PLN48 S3 — parse a `spatial` range slice `xs[(fx,fy)..(tx,ty)]`,
    /// `xs[(fx,fy)..:n]`, or the open `xs[(fx,fy)..]`, and lower it to an
    /// `n_spatial_range` scratch-builder call.  Returns the Radix `typedef` so the
    /// enclosing `for` iterates the scratch it builds.  `(` has already been peeked.
    fn parse_spatial_slice(
        &mut self,
        code: &mut Value,
        typedef: &Type,
        key_types: &[Type],
    ) -> Type {
        // Parse a `(c0, c1, …)` coordinate tuple with exactly one value per axis of the
        // collection (`key_types.len()`), padded to MAX_AXES with `0` for the fixed-arity
        // `n_spatial_range` call.  The collection's own axis count drives how many the
        // range builder reads, so the padding is inert.
        let axes = key_types.len();
        let max_axes = crate::radix_db::MAX_AXES;
        let parse_tuple = |s: &mut Self| -> Vec<Value> {
            let mut out = Vec::new();
            s.lexer.token("(");
            loop {
                let i = out.len();
                let mut v = Value::Null;
                let vt = s.expression(&mut v);
                if !s.convert(&mut v, &vt, &key_types[i.min(axes - 1)]) {
                    diagnostic!(s.lexer, Level::Error, "Invalid spatial coordinate");
                }
                out.push(v);
                if !s.lexer.has_token(",") {
                    break;
                }
            }
            s.lexer.token(")");
            if out.len() != axes {
                diagnostic!(
                    s.lexer,
                    Level::Error,
                    "a spatial coordinate needs {axes} axes, got {}",
                    out.len()
                );
            }
            out.resize(max_axes, Value::Int(0));
            out
        };
        let from = parse_tuple(self);
        if !self.lexer.has_token("..") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a spatial slice needs a range: `xs[(x,y)..]`, `xs[(x,y)..:n]`, or `xs[(x1,y1)..(x2,y2)]`"
            );
        }
        // The `..` is followed by a till tuple `(tx,ty,…)`, a limit `:n`, or nothing.
        let (has_till, till, limit) = if self.lexer.peek_token("(") {
            (Value::Int(1), parse_tuple(self), Value::Int(-1))
        } else if self.lexer.has_token(":") {
            let mut n = Value::Null;
            let nt = self.expression(&mut n);
            if !self.convert(&mut n, &nt, &crate::data::I64) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "spatial slice limit must be an integer"
                );
            }
            (Value::Int(0), vec![Value::Int(0); max_axes], n)
        } else {
            (Value::Int(0), vec![Value::Int(0); max_axes], Value::Int(-1))
        };
        if !self.first_pass {
            let tp = self.get_type(typedef);
            let fn_nr = self.data.def_nr("n_spatial_range");
            if tp != u16::MAX && fn_nr != u32::MAX {
                let mut args = vec![code.clone(), Value::Int(i32::from(tp))];
                args.extend(from); // fx, fy, fz
                args.push(has_till);
                args.extend(till); // tx, ty, tz
                args.push(limit);
                *code = Value::Call(fn_nr, args);
            }
        }
        typedef.clone()
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_key(&mut self, code: &mut Value, typedef: &Type, key_types: &[Type]) {
        // detect open-start `col[..hi]` or `col[..]` before parsing expression.
        let open_start = self.lexer.peek_token("..") || self.lexer.peek_token("..=");
        let mut p = Value::Null;
        let _index_t = if open_start {
            Type::Null // from=[] → no lower bound
        } else {
            let t = self.expression(&mut p);
            if !self.convert(&mut p, &t, &key_types[0]) {
                diagnostic!(self.lexer, Level::Error, "Invalid index key");
            }
            t
        };
        let known = if self.first_pass {
            Value::Null
        } else {
            self.type_info(typedef)
        };
        let mut nr = usize::from(!open_start);
        let mut key = Vec::new();
        if !open_start {
            key.push(p);
        }
        if key_types.len() > 1 {
            while self.lexer.has_token(",") {
                if nr >= key_types.len() {
                    diagnostic!(self.lexer, Level::Error, "Too many key values on index");
                    break;
                }
                let mut ex = Value::Null;
                let ex_t = self.expression(&mut ex);
                if !self.convert(&mut ex, &ex_t, &key_types[nr]) {
                    diagnostic!(self.lexer, Level::Error, "Invalid index key");
                }
                key.push(ex);
                nr += 1;
            }
        }
        if self.lexer.has_token("..") || open_start {
            // Consume "..=" if present (open_start already peeked but didn't consume)
            let inclusive = if open_start {
                self.lexer.has_token(".."); // consume the ".."
                self.lexer.has_token("=")
            } else {
                self.lexer.has_token("=")
            };
            // D-key-1: a keyed range slice yields a `for`-only iterator (`Value::Iter`).
            // In a value position it would leave an un-consumed Iter (a parse-time panic in
            // `set_loop`, or a codegen "Iter should have been rewritten" panic) — emit one
            // clean diagnostic instead.  `set_loop` tolerates the missing loop so parsing
            // reaches here rather than panicking.
            if !self.first_pass && !self.iterable_context {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a keyed range slice is a `for`-loop iterator, not a value — iterate it \
                     directly (`for x in coll[lo..hi] {{ … }}`) or materialise a vector with a \
                     comprehension (`[for x in coll[lo..hi] {{ x }}]`)"
                );
            }
            let iter = self.create_unique("iter", &crate::data::I64);
            let mut ls = Vec::new();
            if !self.first_pass {
                self.fill_iter(&mut ls, code, typedef, true, inclusive);
                ls.push(Value::Int(nr as i32));
                ls.append(&mut key);
            }
            // open-end — if next token is `]` or `,`, skip upper-bound expression.
            let open_end = self.lexer.peek_token("]") || self.lexer.peek_token(",");
            let mut nr = 0;
            if !open_end {
                let mut n = Value::Null;
                let n_t = self.expression(&mut n);
                if !self.convert(&mut n, &n_t, &key_types[0]) && !self.first_pass {
                    diagnostic!(self.lexer, Level::Error, "Invalid index key");
                }
                key.push(n);
                nr = 1;
            }
            if key_types.len() > 1 {
                while self.lexer.has_token(",") {
                    if nr >= key_types.len() {
                        diagnostic!(self.lexer, Level::Error, "Too many key values on index");
                        break;
                    }
                    let mut ex = Value::Null;
                    let ex_t = self.expression(&mut ex);
                    if !self.convert(&mut ex, &ex_t, &key_types[nr]) {
                        diagnostic!(self.lexer, Level::Error, "Invalid index key");
                    }
                    key.push(ex);
                    nr += 1;
                }
            }
            ls.push(Value::Int(nr as i32));
            ls.append(&mut key);
            let start = v_set(iter, self.cl("OpIterate", &ls));
            let mut ls = vec![Value::Var(iter)];
            self.fill_iter(&mut ls, code, typedef, false, inclusive);
            // S16b: annotate the step-block with the element type, not the collection type,
            // so that IR dumps and any type-driven passes see the correct element type.
            let elem_type = match typedef {
                Type::Sorted(el, _, dep) | Type::Index(el, _, dep) => {
                    Type::Reference(*el, dep.clone())
                }
                _ => typedef.clone(),
            };
            *code = Value::Iter(
                u16::MAX,
                Box::new(start),
                Box::new(v_block(
                    vec![self.cl("OpStep", &ls)],
                    elem_type,
                    "Iterate keys",
                )),
                Box::new(Value::Null),
            );
        } else if matches!(typedef, Type::Index(_, _, _) | Type::Sorted(_, _, _))
            && key_types.len() > 1
            && nr < key_types.len()
        {
            // partial-key match — rewrite idx[k1] as idx[k1..=k1].
            // Uses the existing inclusive-range iteration path with from=till=key.
            // D-key-1: like the range branch, a partial-key match yields a `for`-only
            // iterator — reject it in a value position with the same clean diagnostic.
            if !self.first_pass && !self.iterable_context {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a keyed partial-key match is a `for`-loop iterator, not a value — iterate \
                     it directly (`for x in coll[key] {{ … }}`) or give every key field for a \
                     single-record lookup"
                );
            }
            let inclusive = true;
            let iter = self.create_unique("iter", &crate::data::I64);
            let mut ls = Vec::new();
            if !self.first_pass {
                // fill_iter calls set_loop which requires an active loop context.
                let loop_nr = self.vars.start_loop();
                self.fill_iter(&mut ls, code, typedef, true, inclusive);
                self.vars.finish_loop(loop_nr);
                ls.push(Value::Int(nr as i32));
                let from_key = key.clone();
                ls.append(&mut key);
                // till = same key values as from (inclusive prefix match)
                ls.push(Value::Int(nr as i32));
                ls.extend(from_key);
            }
            let start = v_set(iter, self.cl("OpIterate", &ls));
            let mut ls = vec![Value::Var(iter)];
            {
                let loop_nr = self.vars.start_loop();
                self.fill_iter(&mut ls, code, typedef, false, inclusive);
                self.vars.finish_loop(loop_nr);
            }
            let elem_type = match typedef {
                Type::Sorted(el, _, dep) | Type::Index(el, _, dep) => {
                    Type::Reference(*el, dep.clone())
                }
                _ => typedef.clone(),
            };
            *code = Value::Iter(
                u16::MAX,
                Box::new(start),
                Box::new(v_block(
                    vec![self.cl("OpStep", &ls)],
                    elem_type,
                    "Partial key match",
                )),
                Box::new(Value::Null),
            );
        } else {
            let mut ls = vec![code.clone(), known.clone(), Value::Int(nr as i32)];
            ls.append(&mut key);
            *code = self.cl("OpGetRecord", &ls);
            if matches!(typedef, Type::Hash(_, _, _)) && nr < key_types.len() {
                diagnostic!(self.lexer, Level::Error, "Too few key fields");
            }
        }
    }

    pub(crate) fn fill_iter(
        &mut self,
        ls: &mut Vec<Value>,
        code: &mut Value,
        typedef: &Type,
        add_keys: bool,
        inclusive: bool,
    ) {
        let known = self.get_type(typedef);
        if known == u16::MAX {
            return;
        }
        let mut on;
        let arg;
        match self.database.types[known as usize].parts {
            Parts::Index(_, _, _) => {
                on = 1;
                arg = self.database.fields(known);
            }
            Parts::Sorted(tp, _) => {
                on = 2;
                arg = self.database.size(tp);
            }
            Parts::Ordered(_, _) => {
                on = 3;
                arg = 4;
            }
            Parts::Hash(_, _) | Parts::Radix(_, _) => {
                // C60 piece 3 edit C: route hash iteration through
                // Ordered's on=3 code.  Parser has substituted the
                // iterated expression with a `hash_scratch` ref to a
                // u32-stride rec-nr vector in the hash's store (B+A).
                // @PLN48 — a Radix walks the tree into the same scratch
                // rec-vector (it is already key-ordered), then iterates it.
                on = 3;
                arg = 4;
            }
            _ => {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot iterate; expected vector, sorted, index, text, or range"
                );
                return;
            }
        }
        if inclusive {
            on += 128;
        }
        // for index collections with a descending primary key, the tree
        // in-order is reversed from user-logical order.  XOR the reverse bit
        // so that step() uses previous() instead of next(), matching user order.
        // When the user also applies rev(), the XOR cancels out.
        let desc_primary = on & 63 == 1
            && !self.database.types[known as usize].keys.is_empty()
            && self.database.types[known as usize].keys[0].type_nr < 0;
        if self.reverse_iterator ^ desc_primary {
            on += 64;
        }
        if self.reverse_iterator {
            // Do not reset here — `iterator()` calls fill_iter twice and resets after both.
        }
        ls.push(code.clone());
        ls.push(Value::Int(i32::from(on)));
        ls.push(Value::Int(i32::from(arg)));
        // For Index (on & 63 == 1): store the type index so OpRemove can call
        // database.fields(tp) and database.remove(..., tp) with the correct type.
        // For all other collection types, arg IS the db_tp used by OpRemove.
        let loop_db_tp = if on & 63 == 1 { known } else { arg };
        self.vars.set_loop(on, loop_db_tp, code);
        if add_keys {
            ls.push(Value::Keys(
                self.database.types[known as usize].keys.clone(),
            ));
        }
    }
}
