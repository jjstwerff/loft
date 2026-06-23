// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    Argument, DefType, Function, I32, Level, Parser, ToString, Type, Value, diagnostic_format,
    field_id, v_block, v_if, v_loop, v_set,
};
use crate::data::Deps;

// Lambda and vector expression parsing.

impl Parser {
    pub(crate) fn parse_append_vector(
        &mut self,
        code: &mut Value,
        tp: &Type,
        parts: &[(Value, Type)],
        orig_var: u16,
    ) -> Type {
        let mut ls = Vec::new();
        let rec_tp = if let Type::Vector(cont, _) = tp {
            // @P314 — narrow-aware element type (see `append_elem_tp`).
            let cont = (**cont).clone();
            self.append_elem_tp(&cont)
        } else {
            i32::MIN
        };
        let var_nr = if orig_var == u16::MAX {
            let vec = self.create_unique("vec", tp);
            let elm_tp = tp.content();
            let db_ops = self.vector_db(&elm_tp, vec);
            // vector concat as an inline expression (not assigned to a variable)
            // creates a temporary with database allocation that corrupts the stack when
            // the result is used inside a compound assignment expression.
            // Emit an error so the user assigns to a variable first.
            if !db_ops.is_empty() && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "vector concatenation in an expression creates a temporary; \
                     assign to a variable first for correct results in compound expressions"
                );
            }
            for l in db_ops {
                ls.push(l);
            }
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(vec), code.clone(), Value::Int(rec_tp)],
            ));
            vec
        } else if let Value::Insert(elms) = code {
            for e in elms {
                ls.push(e.clone());
            }
            orig_var
        } else if matches!(self.vars.tp(orig_var), Type::RefVar(t) if matches!(**t, Type::Vector(_, _)))
        {
            // RefVar(Vector): append directly without an identity Set(v, Var(v)).
            // find_written_vars detects the write via the OpAppendVector in the parts loop.
            // The first operand `code` must BE the accumulator (`out = out + x`,
            // in-place grow); a DIFFERENT first operand (`out = a + x`) is a
            // REPLACEMENT, which the `&`-ref mechanism cannot express — the ref
            // shares the caller's store in place (OpCreateStack/OpGetStackRef);
            // there is no op that repoints it at a different store.  The old code
            // silently dropped `code`/`a` and appended only the trailing parts (a
            // half-wrong `out += x`).  Reject it instead — mirrors the existing
            // `out = a` "& but is never modified" rejection.
            if !self.first_pass && !matches!(code.unspan(), Value::Var(x) if *x == orig_var) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot replace a `&` vector parameter; a `&` ref grows the \
                     caller's vector in place — append with `{} += …`, it cannot be reassigned",
                    self.vars.name(orig_var)
                );
            }
            orig_var
        } else if matches!(code.unspan(), Value::Var(x) if *x != orig_var) {
            // The concat's first operand is a NAMED LOCAL other than the
            // accumulator (`v = a + b`, `u = v + [9]`).  Emitting `Set(v, Var(a))`
            // would repoint v's runtime slot at a's store — aliasing v to a,
            // mutating a when the parts append, AND orphaning the fresh
            // vector_db store create_vector allocated for v.  Deep-COPY a's
            // elements into v's own store instead (OpAppendVector deep-copies),
            // so a stays intact and v is independent.  The self-reference case
            // (`code == Var(orig_var)`, excluded by `x != orig_var`) keeps the
            // `Set(v, Var(v))` below so create_vector's self_ref / in-place path
            // still fires; a fresh-storage temp (Call/Block, not a Var) also
            // keeps `Set` so v adopts the temp's store with no extra copy.
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(orig_var), code.clone(), Value::Int(rec_tp)],
            ));
            orig_var
        } else {
            ls.push(v_set(orig_var, code.clone()));
            orig_var
        };
        for (val, _) in parts {
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(var_nr), val.clone(), Value::Int(rec_tp)],
            ));
        }
        if orig_var == u16::MAX {
            let res = self.vars.tp(var_nr).clone();
            ls.push(Value::Var(var_nr));
            *code = v_block(ls, res.clone(), "Append Vector");
            return res;
        }
        *code = Value::Insert(ls);
        Type::Rewritten(Box::new(tp.clone()))
    }

    pub(crate) fn parse_append_text(
        &mut self,
        code: &mut Value,
        tp: &Type,
        parts: &[(Value, Type)],
        orig_var: u16,
    ) -> Type {
        let mut ls = Vec::new();
        let var_nr = if orig_var == u16::MAX {
            let v = self.vars.work_text(&mut self.lexer);
            if matches!(self.vars.tp(v), Type::RefVar(_)) {
                ls.push(self.cl("OpClearStackText", &[Value::Var(v)]));
                ls.push(self.cl("OpAppendStackText", &[Value::Var(v), code.clone()]));
            } else if tp == &Type::Character {
                ls.push(self.cl("OpClearText", &[Value::Var(v)]));
                ls.push(self.cl("OpAppendCharacter", &[Value::Var(v), code.clone()]));
            } else {
                ls.push(self.cl("OpClearText", &[Value::Var(v)]));
                ls.push(self.cl("OpAppendText", &[Value::Var(v), code.clone()]));
            }
            v
        } else if matches!(self.vars.tp(orig_var), Type::RefVar(_)) {
            ls.push(self.cl("OpAppendStackText", &[Value::Var(orig_var), code.clone()]));
            orig_var
        } else {
            ls.push(self.cl("OpAppendText", &[Value::Var(orig_var), code.clone()]));
            orig_var
        };
        for (val, tp) in parts {
            // Unwrap `RefVar(inner)` for the type-dispatch check below.
            // A `&text` argument (parameter passed by reference) appears
            // as `Type::RefVar(Type::Text(_))` here but the OpAppend* ops
            // accept it directly via the same code path as plain `Text`.
            let dispatch_tp: &Type = if let Type::RefVar(inner) = tp {
                inner.as_ref()
            } else {
                tp
            };
            if matches!(self.vars.tp(var_nr), Type::RefVar(_)) {
                if *dispatch_tp == Type::Character {
                    ls.push(self.cl("OpAppendStackCharacter", &[Value::Var(var_nr), val.clone()]));
                } else if matches!(dispatch_tp, Type::Text(_)) {
                    ls.push(self.cl("OpAppendStackText", &[Value::Var(var_nr), val.clone()]));
                } else {
                    // @P274 — non-text/non-character parts (integer / float /
                    // bool / vector / reference / enum / …) need a format-
                    // dispatch step before append.  `OpAppendStackText`
                    // assumes its argument already evaluates to text on the
                    // stack; passing a raw `i64` from `headers.len()` is what
                    // tripped native E0614 (`type i64 cannot be dereferenced`)
                    // and SIGSEGV in interp.  Route through `append_data`,
                    // which is the same dispatch path used by `"…{x}…"`
                    // format-string interpolation and handles every formattable
                    // type via the matching `OpFormat*` op.
                    self.append_data(
                        dispatch_tp.clone(),
                        &mut ls,
                        var_nr,
                        u16::MAX,
                        val,
                        super::OUTPUT_DEFAULT,
                    );
                }
            } else if *dispatch_tp == Type::Character {
                ls.push(self.cl("OpAppendCharacter", &[Value::Var(var_nr), val.clone()]));
            } else if matches!(dispatch_tp, Type::Text(_)) {
                ls.push(self.cl("OpAppendText", &[Value::Var(var_nr), val.clone()]));
            } else {
                // @P274 — see RefVar branch above.
                self.append_data(
                    dispatch_tp.clone(),
                    &mut ls,
                    var_nr,
                    u16::MAX,
                    val,
                    super::OUTPUT_DEFAULT,
                );
            }
        }
        let tp = Type::Text(Deps::frame1(var_nr));
        if orig_var == u16::MAX || var_nr != orig_var {
            // A new work text was created (either no orig_var, or orig_var was a
            // Character variable) — wrap in a Block so the work text appears on the stack.
            ls.push(Value::Var(var_nr));
            *code = v_block(ls, tp.clone(), "Add text");
            return tp;
        }
        *code = Value::Insert(ls);
        Type::Rewritten(Box::new(tp))
    }

    /// Rewrite boolean operators into an `IF` statement to prevent the calculation of the second
    /// expression when it is unneeded.
    pub(crate) fn boolean_operator(
        &mut self,
        code: &mut Value,
        tp: &Type,
        precedence: usize,
        is_or: bool,
    ) {
        if !self.convert(code, tp, &Type::Boolean) && !self.first_pass {
            self.can_convert(tp, &Type::Boolean);
        }
        let mut second_code = Value::Null;
        let mut parent_tp = Type::Unknown(0);
        let second_pos = self.lexer.peek_pos().clone();
        let second_type = self.parse_operators(
            &Type::Unknown(0),
            &mut second_code,
            &mut parent_tp,
            precedence + 1,
        );
        self.known_var_or_type(&second_code, &second_pos);
        if !self.convert(&mut second_code, &second_type, &Type::Boolean) && !self.first_pass {
            self.can_convert(&second_type, &Type::Boolean);
        }
        *code = v_if(
            code.clone(),
            if is_or {
                Value::Boolean(true)
            } else {
                second_code.clone()
            },
            if is_or {
                second_code
            } else {
                Value::Boolean(false)
            },
        );
    }

    // <single> ::= '!' <expression> |
    //              '(' <expression> ')' |
    //              <vector> |
    //              'if' <if> |
    //              <identifier:var> |
    //              <number> | <float> | <cstring> |
    //              'true' | 'false' | 'null'
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_single(
        &mut self,
        var_tp: &Type,
        val: &mut Value,
        parent_tp: &mut Type,
    ) -> Type {
        if self.lexer.has_token("!") {
            let t = self.parse_part(var_tp, val, parent_tp);
            // #253: `!x` on a non-boolean reads as "is x null?" — the null
            // sentinel is in-band (LOFT.md "!value asymmetry").  On a `not null`
            // operand the value can never BE the sentinel, so `!x` is *always
            // false* — a silent no-op (`f = 0; if !f` never runs, because 0 is a
            // real value, not null).  Warn, don't error: a nullable operand
            // (`!both` in stdlib min/max) is a legitimate null test, and boolean
            // `!` is ordinary negation (`false` is a valid value there).
            let eff = if let Type::RefVar(inner) = &t {
                (**inner).clone()
            } else {
                t.clone()
            };
            let operand_not_null =
                self.expr_not_null || matches!(&eff, Type::Integer(spec) if spec.not_null);
            if !self.first_pass
                && operand_not_null
                && !matches!(eff, Type::Boolean | Type::Null | Type::Unknown(_))
            {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "'!' on a 'not null' {} is always false — '!x' tests whether x \
                     is null, and a 'not null' value is never null; compare \
                     explicitly (e.g. 'x == 0') if you meant a value check",
                    t.name(&self.data)
                );
            }
            let arg = val.clone();
            self.call_op(val, "Not", &[arg], &[t])
        } else if self.lexer.has_token("~") {
            let t = self.parse_part(var_tp, val, parent_tp);
            let arg = val.clone();
            self.call_op(val, "BitNot", &[arg], &[t])
        } else if self.lexer.has_token("-") {
            let t = self.parse_part(var_tp, val, parent_tp);
            let arg = val.clone();
            self.call_op(val, "Min", &[arg], &[t])
        } else if self.lexer.has_token("(") {
            let t = self.expression(val);
            if self.lexer.has_token(",") {
                // T1.2: Tuple literal — (expr, expr, ...)
                let mut values = vec![val.clone()];
                let mut types = vec![t];
                loop {
                    if self.lexer.peek_token(")") {
                        break;
                    }
                    let mut v = Value::Null;
                    let t2 = self.expression(&mut v);
                    values.push(v);
                    types.push(t2);
                    if !self.lexer.has_token(",") {
                        break;
                    }
                }
                self.lexer.token(")");
                if types.len() < 2 {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Tuple literals require at least 2 elements"
                    );
                }
                *val = Value::Tuple(values);
                Type::Tuple(types)
            } else {
                self.lexer.token(")");
                // @P395 — a parenthesised vector concat consumed by a trailing
                // `.method()` reused the OUTER assignment LHS as its accumulator
                // (the inner concat inherited `orig_var` from the live `val`),
                // lowering to a void `Value::Insert` that leaves no stack value;
                // the method then reads a misaligned slot → garbage value / the
                // `codegen.rs:2669` +8 drift.  This is the same shape the P103
                // guard in `parse_append_vector` already rejects for
                // `f([1,2] + [3])` (tests/scripts/102-expected-errors.loft):
                // inline vector concat consumed by a call/sub-expression must be
                // assigned to a variable first.  Fire the same clean error here
                // instead of silently corrupting.  Direct assignment `v = (a+c)`
                // (no trailing `.`) keeps its reuse path; `(a+c)[i]` indexing
                // already lowers correctly and is not guarded.
                if !self.first_pass && matches!(val, Value::Insert(_)) && self.lexer.peek_token(".")
                {
                    let tv = if let Type::Rewritten(inner) = &t {
                        (**inner).clone()
                    } else {
                        t.clone()
                    };
                    if matches!(tv, Type::Vector(_, _)) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "vector concatenation in an expression creates a temporary; \
                             assign to a variable first for correct results in compound expressions"
                        );
                    }
                }
                t
            }
        } else if self.lexer.peek_token("{") {
            self.parse_block("block", val, &Type::Unknown(0))
        } else if self.lexer.has_token("[") {
            // #432 — a bare vector literal in call-argument position arrives with
            // `var_tp` unknown (the parameter type is dropped by `expression`).
            // Build it at the parameter's element width via `vector_hint`, so a
            // `vector<u8>` parameter gets a 1-byte-stride literal instead of a
            // `vector<integer>` (8-byte) the callee silently reinterprets.  A
            // typed context already carries the type in `var_tp` and wins.  Take
            // (clear) the hint so it seeds only this outermost literal — nested
            // literals get their element type threaded through `var_tp`.
            let hint = std::mem::replace(&mut self.vector_hint, Type::Unknown(0));
            let seeded;
            let elem_tp = if var_tp.is_unknown() && matches!(hint, Type::Vector(_, _)) {
                seeded = hint;
                &seeded
            } else {
                var_tp
            };
            self.parse_vector(elem_tp, val, parent_tp)
        } else if self.lexer.has_token("if") {
            self.parse_if(val)
        } else if self.lexer.has_token("match") {
            self.parse_match(val)
        } else if self.lexer.has_token("fn") {
            if self.lexer.peek_token("(") {
                self.parse_lambda(val)
            } else {
                // function references use the bare name, not 'fn name'.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Use the function name directly, without 'fn' prefix"
                );
                self.parse_fn_ref(val)
            }
        } else if self.lexer.has_token("||") {
            // Zero-parameter short lambda: || { body } — `||` already consumed, no closing `|`
            self.parse_lambda_short(val, false)
        } else if self.lexer.has_token("|") {
            // Short lambda with parameters: |x: T, …| { body } — opening `|` consumed
            self.parse_lambda_short(val, true)
        } else if self.lexer.has_token("sizeof") {
            self.lexer.token("(");
            self.parse_size(val)
        } else if self.lexer.has_token("type_name") {
            self.lexer.token("(");
            self.parse_type_name(val)
        } else if self.lexer.has_token("assert") {
            self.lexer.token("(");
            self.parse_intrinsic_call(val, "assert")
        } else if self.lexer.has_token("panic") {
            self.lexer.token("(");
            self.parse_intrinsic_call(val, "panic")
        } else if let Some((name, name_pos)) = self.lexer.has_identifier_pos() {
            // @PLN22 Phase 1 — when the receiver context (`parent_tp`) is not
            // itself an enum, supply the operand's expected enum so a bare
            // value-position variant resolves against it (variants are not in the
            // flat namespace).  `var_tp` carries the enum for typed-local decls,
            // typed reassignment, `==`, and struct-field init; `enum_hint` carries
            // it for call args / return body.  This only feeds parse_var's
            // last-resort variant branch — a name that resolves as a variable,
            // field, function, or `$` is handled by an earlier branch unaffected.
            if !self.enum_context(parent_tp) {
                if self.enum_context(var_tp) {
                    *parent_tp = var_tp.clone();
                } else if self.enum_context(&self.enum_hint) {
                    *parent_tp = self.enum_hint.clone();
                }
            }
            self.parse_var(val, &name, parent_tp, &name_pos)
        } else if self.lexer.peek_token("$") {
            let name_pos = self.lexer.peek_pos().clone();
            self.lexer.has_token("$");
            self.parse_var(val, "$", parent_tp, &name_pos)
        } else if let Some(nr) = self.lexer.has_integer() {
            *val = Value::Int(nr as i32);
            I32.clone()
        } else if let Some(nr) = self.lexer.has_long() {
            *val = Value::Long(nr as i64);
            crate::data::I64.clone()
        } else if let Some(nr) = self.lexer.has_float() {
            *val = Value::Float(nr);
            Type::Float
        } else if let Some(nr) = self.lexer.has_single() {
            *val = Value::Single(nr);
            Type::Single
        } else if let Some(s) = self.lexer.has_cstring() {
            self.parse_string(val, &s)
        } else if let Some(nr) = self.lexer.has_char() {
            *val = self.cl("OpConvCharacterFromInt", &[Value::Int(nr as i32)]);
            Type::Character
        } else if self.lexer.has_token("true") {
            *val = Value::Boolean(true);
            Type::Boolean
        } else if self.lexer.has_token("false") {
            *val = Value::Boolean(false);
            Type::Boolean
        } else if self.lexer.has_token("null") {
            *val = Value::Null;
            Type::Null
        } else {
            Type::Unknown(0)
        }
    }

    // <fn-ref> ::= 'fn' <identifier>
    // Produces a Type::Function value whose runtime representation is the
    // definition number (d_nr) of the named function stored as an i32.
    pub(crate) fn parse_fn_ref(&mut self, code: &mut Value) -> Type {
        let Some(name) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Expect function name after fn");
            }
            return Type::Unknown(0);
        };
        // Try user function (n_<name>) first, then fall back to bare name.
        let d_nr = {
            let prefixed = format!("n_{name}");
            let nr = self.data.def_nr(&prefixed);
            if nr == u32::MAX {
                self.data.def_nr(&name)
            } else {
                nr
            }
        };
        if d_nr == u32::MAX {
            if !self.first_pass {
                if let Some(s) = self.suggest_function_name(&name) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown function '{name}' — did you mean '{s}'?"
                    );
                } else {
                    diagnostic!(self.lexer, Level::Error, "Unknown function '{name}'");
                }
            }
            return Type::Unknown(0);
        }
        if !self.first_pass && !matches!(self.data.def_type(d_nr), DefType::Function) {
            diagnostic!(self.lexer, Level::Error, "'{name}' is not a function");
            return Type::Unknown(0);
        }
        *code = Value::Int(d_nr as i32);
        self.data.def_used(d_nr);
        let n_args = self.data.attributes(d_nr);
        let arg_types: Vec<Type> = (0..n_args).map(|a| self.data.attr_type(d_nr, a)).collect();
        let ret_type = self.data.def(d_nr).returned().clone();
        Type::Function(arg_types, Box::new(ret_type), Deps::none())
    }

    // <lambda> ::= 'fn' '(' [<params>] ')' ['->' <type>] '{' <body> '}'
    // Produces Type::Function; runtime representation is d_nr as i32, same as fn-ref.
    pub(crate) fn parse_lambda(&mut self, code: &mut Value) -> Type {
        let lambda_name = format!("__lambda_{}", self.lambda_counter);
        self.lambda_counter += 1;
        let stored_name = format!("n_{lambda_name}");

        let outer_context = self.context;
        let outer_vars = std::mem::replace(
            &mut self.vars,
            Function::new(&lambda_name, &self.lexer.pos().file),
        );
        let outer_loop = self.in_loop;
        self.in_loop = false;
        // save outer scope variable names/types for capture detection.
        let outer_capture =
            std::mem::replace(&mut self.capture_context, outer_vars.all_names_and_types());
        // clear captured_names so we collect only this lambda's captures.
        let outer_captured = std::mem::take(&mut self.captured_names);

        self.lexer.token("(");
        let mut arguments = Vec::new();
        self.parse_arguments(&lambda_name, &mut arguments);
        self.lexer.token(")");

        self.context = if self.first_pass {
            self.data.add_fn(&mut self.lexer, &lambda_name, &arguments)
        } else {
            self.data.def_nr(&stored_name)
        };
        if self.context == u32::MAX {
            self.context = outer_context;
            self.vars = outer_vars;
            self.in_loop = outer_loop;
            return Type::Unknown(0);
        }
        let d_nr = self.context;

        // Parse optional return type annotation.
        let result = if self.lexer.has_token("->") {
            self.parse_type_full(d_nr, true).unwrap_or(Type::Void)
        } else {
            Type::Void
        };
        if self.first_pass {
            self.data.set_returned(d_nr, result);
        }

        self.vars
            .append(&mut self.data.definitions[d_nr as usize].variables);
        for (a_nr, a) in arguments.iter().enumerate() {
            if self.first_pass {
                let v_nr = self.create_var(&a.name, &a.typedef);
                if v_nr != u16::MAX {
                    self.vars.become_argument(v_nr);
                    self.var_usages(v_nr, false);
                }
            } else {
                self.change_var_type(a_nr as u16, &a.typedef);
            }
        }

        // The codegen (line 40-46) reads definition attributes to assign argument positions.
        let outer_closure_param = self.closure_param;
        if !self.first_pass {
            let closure_rec = self.data.def(d_nr).closure_record();
            if closure_rec != u32::MAX {
                let closure_tp = Type::Reference(closure_rec, Deps::none());
                // Add as definition attribute so codegen positions it on the stack.
                self.data
                    .add_attribute(&mut self.lexer, d_nr, "__closure", closure_tp.clone());
                let v_nr = self.create_var("__closure", &closure_tp);
                self.vars.become_argument(v_nr);
                self.closure_param = v_nr;
            }
        }

        self.parse_code();
        self.closure_param = outer_closure_param;
        self.data.op_code(d_nr);
        self.data.definitions[d_nr as usize]
            .variables
            .append(&mut self.vars);

        // Plan-22 phase 02c (2026-05-12): collect mutations BEFORE
        // synthesize_closure_record so the synthesise step can
        // consult `data.def(d_nr).mutated_captures` to pick the
        // right attribute type (auto-Reference for mutated
        // Reference captures vs inline-bytes for everything else).
        // The collect helper accepts a missing closure_record and
        // derives captured names from the lambda's variable table
        // in that case.
        if !self.captured_names.is_empty() {
            collect_mutated_captures(&mut self.data, d_nr);
            // Plan-22 phase 02d-i (2026-05-12): accumulate scalar
            // mutated-captures onto the parent function's
            // `scalars_to_box` field.  Phase 02d-iii will use this
            // to rewrite outer bindings to hidden cells.  Detection-
            // only at this phase — no behavior change.
            accumulate_scalars_to_box(&mut self.data, outer_context, d_nr, &self.captured_names);
            // Plan-22 phase 02d-ii — ensure a `__cell_<T>` struct
            // exists for every scalar-typed mutated capture, so
            // 02d-iii's outer-binding rewrite has a target type to
            // allocate.  Idempotent across all lambdas in the
            // compilation; gated on first_pass.
            self.synthesize_cell_structs(d_nr);
            // Plan-22 phase 02d-iii.e — pre-box `captured_names`
            // entries for names in the parent's scalars_to_box
            // BEFORE `synthesize_closure_record` builds the
            // record's attributes.  Without this, pass 1 freezes
            // the attribute as the un-flipped scalar (Integer)
            // and the closure record's storage layout uses 8B
            // inline instead of 12B share-by-DbRef — runtime
            // then misroutes writes through `OpSetInt` and
            // crashes on the locked store.
            box_captured_names_for_outer_scalars(
                &mut self.captured_names,
                &self.data,
                outer_context,
            );
            self.synthesize_closure_record(d_nr, &lambda_name);
            // #314: remember this lambda so the enclosing body's end
            // can reject shared-mutable-scalar captures once
            // `scalars_to_box` is complete.
            if self.first_pass {
                self.fn_lambdas.entry(outer_context).or_default().push(d_nr);
            }
        }
        let captured = std::mem::replace(&mut self.captured_names, outer_captured);
        drop(captured);

        self.context = outer_context;
        self.vars = outer_vars;
        self.in_loop = outer_loop;
        self.capture_context = outer_capture;

        self.data.def_used(d_nr);

        self.emit_lambda_code(code, d_nr);

        // Build the user-visible function type from the declared arguments only.
        // Using data.attributes(d_nr) is wrong here: text_return() registers text work
        // variables (e.g. __work_1) as definition attributes for stack allocation, and
        // the second-pass closure injection also adds a hidden __closure attribute.
        // Neither should appear in the public Function type — only declared params do.
        let arg_types: Vec<Type> = arguments.iter().map(|a| a.typedef.clone()).collect();
        let ret_type = self.data.def(d_nr).returned().clone();
        // include the closure work var dep so that get_free_vars knows
        // a local ___clos_N variable owns the closure (and will free it).  Without
        // this dep, the Function arm in get_free_vars would emit a duplicate free.
        let dep = if self.last_closure_work_var == u16::MAX {
            Deps::none()
        } else {
            Deps::frame1(self.last_closure_work_var)
        };
        Type::Function(arg_types, Box::new(ret_type), dep)
    }

    // <short-lambda> ::= '||' ['->' type] block              (expect_close=false)
    //                  | '|' [param {',' param}] '|' ['->' type] block  (expect_close=true)
    // param ::= ident [':' type]
    // `expect_close` is true when the opening `|` was consumed (params may follow);
    // false when `||` was consumed (zero params, no closing `|`).
    // Types are inferred from `lambda_hint` (set by the call-site parser) when omitted.
    // Produces Type::Function; runtime representation is d_nr as i32, same as fn-ref.
    #[allow(clippy::too_many_lines)] // single context save/restore spans the whole body; splitting would need unsafe borrowing
    pub(crate) fn parse_lambda_short(&mut self, code: &mut Value, expect_close: bool) -> Type {
        let lambda_name = format!("__lambda_{}", self.lambda_counter);
        self.lambda_counter += 1;
        let stored_name = format!("n_{lambda_name}");

        // Capture hint types before entering the new context.
        let hint_params_ret = self.lambda_hint.clone();
        let hint_params: Vec<Type> = if let Type::Function(pts, _, _) = &hint_params_ret {
            pts.clone()
        } else {
            Vec::new()
        };

        // Parse parameter list from `|p1 [: T], p2 [: T], …|`.
        // When expect_close=false (`||` was consumed), there are no params and no closing `|`.
        let mut param_names: Vec<String> = Vec::new();
        let mut param_types: Vec<Type> = Vec::new();
        if expect_close {
            while !self.lexer.peek_token("|") && !self.lexer.peek_token("{") {
                let Some(pname) = self.lexer.has_identifier() else {
                    break;
                };
                let idx = param_names.len();
                let tp = if self.lexer.has_token(":") {
                    // type annotations are not allowed in |x| short-form lambdas.
                    // Use the long form fn(x: type) -> ret { body } instead.
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Type annotations are not allowed in |x| lambdas — \
                         use fn({pname}: <type>) {{ ... }} instead \
                         (add `-> <ret>` only for non-void returns; \
                         `-> void` is not a valid type)"
                    );
                    // Consume the type token so parsing can continue.
                    let _ = self.lexer.has_identifier();
                    // Infer from hint to keep parsing viable.
                    hint_params.get(idx).cloned().unwrap_or(Type::Unknown(0))
                } else {
                    // Infer from hint.
                    hint_params.get(idx).cloned().unwrap_or(Type::Unknown(0))
                };
                param_names.push(pname);
                param_types.push(tp);
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("|"); // consume closing `|`
        }

        // Build Argument list for function registration.
        let arguments: Vec<Argument> = param_names
            .iter()
            .zip(param_types.iter())
            .map(|(n, t)| Argument {
                name: n.clone(),
                typedef: t.clone(),
                default: Value::Null,
                constant: false,
            })
            .collect();

        // Error on second pass for any parameter whose type is still Unknown.
        if !self.first_pass {
            for a in &arguments {
                if a.typedef.is_unknown() {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot infer type for lambda parameter '{}'; pass the lambda where the expected type is known, or use fn(name: <type>) {{{{ ... }}}} (add `-> <ret>` only for non-void returns)",
                        a.name
                    );
                }
            }
        }

        let outer_context = self.context;
        let outer_vars = std::mem::replace(
            &mut self.vars,
            Function::new(&lambda_name, &self.lexer.pos().file),
        );
        let outer_loop = self.in_loop;
        self.in_loop = false;
        // save outer scope variable names/types for capture detection.
        let outer_capture =
            std::mem::replace(&mut self.capture_context, outer_vars.all_names_and_types());
        let outer_captured = std::mem::take(&mut self.captured_names);

        self.context = if self.first_pass {
            self.data.add_fn(&mut self.lexer, &lambda_name, &arguments)
        } else {
            self.data.def_nr(&stored_name)
        };
        if self.context == u32::MAX {
            self.context = outer_context;
            self.vars = outer_vars;
            self.in_loop = outer_loop;
            self.capture_context = outer_capture;
            self.captured_names = outer_captured;
            return Type::Unknown(0);
        }
        let d_nr = self.context;

        // return-type annotations are not allowed in |x| short-form lambdas.
        let has_arrow = self.lexer.has_token("->");
        let result = if has_arrow {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Return-type annotations are not allowed in |x| lambdas — \
                 use fn(…) -> <ret> {{ ... }} instead"
            );
            self.parse_type_full(d_nr, true).unwrap_or(Type::Void)
        } else if let Type::Function(_, ret, _) = &hint_params_ret {
            *ret.clone()
        } else {
            Type::Void
        };
        if self.first_pass {
            // On first pass, hint is unavailable — store Void when no annotation.
            self.data.set_returned(
                d_nr,
                if has_arrow {
                    result.clone()
                } else {
                    Type::Void
                },
            );
        } else if !result.is_unknown() && !matches!(result, Type::Void) {
            // On second pass, force-update the return type from hint or annotation.
            self.data.definitions[d_nr as usize].returned = result.clone();
        }

        self.vars
            .append(&mut self.data.definitions[d_nr as usize].variables);
        for (a_nr, a) in arguments.iter().enumerate() {
            if self.first_pass {
                let v_nr = self.create_var(&a.name, &a.typedef);
                if v_nr != u16::MAX {
                    self.vars.become_argument(v_nr);
                    self.var_usages(v_nr, false);
                }
            } else {
                self.change_var_type(a_nr as u16, &a.typedef);
                // Force-update the data definition with the inferred type.
                // `set_attr_type` panics on non-unknown, so write directly.
                // (First pass stored Unknown(0); typedef.rs may have resolved that to a
                // concrete type before the second pass, so we can't rely on is_unknown().)
                if !a.typedef.is_unknown() {
                    self.data.definitions[d_nr as usize].attributes[a_nr].typedef =
                        a.typedef.clone();
                }
            }
        }

        self.parse_code();
        self.data.op_code(d_nr);
        self.data.definitions[d_nr as usize]
            .variables
            .append(&mut self.vars);

        // synthesize closure record if any captures were detected.
        // Plan-22 phase 02c (2026-05-12): collect mutations BEFORE
        // synthesize_closure_record so the synthesise step can
        // consult `data.def(d_nr).mutated_captures` to pick the
        // right attribute type (auto-Reference for mutated
        // Reference captures vs inline-bytes for everything else).
        // The collect helper accepts a missing closure_record and
        // derives captured names from the lambda's variable table
        // in that case.
        if !self.captured_names.is_empty() {
            collect_mutated_captures(&mut self.data, d_nr);
            // Plan-22 phase 02d-i (2026-05-12): accumulate scalar
            // mutated-captures onto the parent function's
            // `scalars_to_box` field.  Phase 02d-iii will use this
            // to rewrite outer bindings to hidden cells.  Detection-
            // only at this phase — no behavior change.
            accumulate_scalars_to_box(&mut self.data, outer_context, d_nr, &self.captured_names);
            // Plan-22 phase 02d-ii — ensure a `__cell_<T>` struct
            // exists for every scalar-typed mutated capture, so
            // 02d-iii's outer-binding rewrite has a target type to
            // allocate.  Idempotent across all lambdas in the
            // compilation; gated on first_pass.
            self.synthesize_cell_structs(d_nr);
            // Plan-22 phase 02d-iii.e — pre-box `captured_names`
            // entries for names in the parent's scalars_to_box
            // BEFORE `synthesize_closure_record` builds the
            // record's attributes.  Without this, pass 1 freezes
            // the attribute as the un-flipped scalar (Integer)
            // and the closure record's storage layout uses 8B
            // inline instead of 12B share-by-DbRef — runtime
            // then misroutes writes through `OpSetInt` and
            // crashes on the locked store.
            box_captured_names_for_outer_scalars(
                &mut self.captured_names,
                &self.data,
                outer_context,
            );
            self.synthesize_closure_record(d_nr, &lambda_name);
            // #314: remember this lambda so the enclosing body's end
            // can reject shared-mutable-scalar captures once
            // `scalars_to_box` is complete.
            if self.first_pass {
                self.fn_lambdas.entry(outer_context).or_default().push(d_nr);
            }
        }
        let captured = std::mem::replace(&mut self.captured_names, outer_captured);
        drop(captured);

        self.context = outer_context;
        self.vars = outer_vars;
        self.in_loop = outer_loop;
        self.capture_context = outer_capture;

        self.data.def_used(d_nr);

        self.emit_lambda_code(code, d_nr);

        let n_args = self.data.attributes(d_nr);
        let arg_types: Vec<Type> = (0..n_args).map(|a| self.data.attr_type(d_nr, a)).collect();
        let ret_type = self.data.def(d_nr).returned().clone();
        // include closure work var dep (same as fn-form lambda).
        let dep = if self.last_closure_work_var == u16::MAX {
            Deps::none()
        } else {
            Deps::frame1(self.last_closure_work_var)
        };
        Type::Function(arg_types, Box::new(ret_type), dep)
    }

    // emit the lambda value — plain Int(d_nr) for non-capturing
    // lambdas, or an Insert block that allocates and populates the closure record.
    #[allow(clippy::similar_names)]
    fn emit_lambda_code(&mut self, code: &mut Value, d_nr: u32) {
        let closure_rec_d = self.data.def(d_nr).closure_record();
        if closure_rec_d != u32::MAX && !self.first_pass {
            // A5.6-1/2 (16-byte fn-ref + embedded closure):
            // Allocate and populate the closure record at lambda DEFINITION time.
            // Embed the closure DbRef into a 16-byte fn-ref slot so the closure
            // travels with the fn-ref even when it escapes its defining scope.
            //
            // Layout of the 16-byte fn-ref frame slot:
            //   bytes  0.. 4: d_nr (i32, function definition number)
            //   bytes  4..16: closure DbRef (12 bytes; null = no closure)
            //
            // w (___clos_N) is added to work_refs so parse_code inserts Set(w,Null)
            // at the START of the enclosing function body, pre-reserving w's slot
            // in the outer scope — FreeStack cannot clobber it.
            //
            // The fn_ref_var (__fn_ref_N) holds [d_nr, closure DbRef] as 16 bytes.
            // At call sites, fn_call_ref reads the embedded DbRef and pushes it as
            // the hidden __closure arg automatically — no explicit injection needed.
            let rec_tp = Type::Reference(closure_rec_d, Deps::none());
            let w = self.create_unique("__clos", &rec_tp);
            self.vars.defined(w);
            // Register w as a work-ref so parse_code inserts Set(w,Null) at fn start.
            self.vars.add_to_work_refs(w);
            let tp_nr = i32::from(self.data.def(closure_rec_d).known_type());
            // Build fn_type for fn_ref_var: visible params (excluding __closure) + ret.
            let n_all_attrs = self.data.attributes(d_nr);
            let has_closure_attr =
                n_all_attrs > 0 && self.data.attr_name(d_nr, n_all_attrs - 1) == "__closure";
            let n_visible = if has_closure_attr {
                n_all_attrs - 1
            } else {
                n_all_attrs
            };
            let visible_params: Vec<Type> = (0..n_visible)
                .map(|aid| self.data.attr_type(d_nr, aid).clone())
                .collect();
            let ret_tp = self.data.def(d_nr).returned().clone();
            // fn-ref depends on closure work var `w` so that
            // get_free_vars does not emit OpFreeRef for the closure record
            // before the fn-ref escapes the defining scope.
            let fn_type = Type::Function(visible_params, Box::new(ret_tp.clone()), Deps::frame1(w));
            let mut alloc_steps: Vec<Value> = Vec::new();
            // Allocate and populate the closure record w.
            alloc_steps.push(crate::data::v_set(w, Value::Null));
            alloc_steps.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp_nr)]));
            let n_attrs = self.data.attributes(closure_rec_d);
            let mut captured_var_nrs: Vec<u16> = Vec::new();
            for aid in 0..n_attrs {
                let cap_name = self.data.attr_name(closure_rec_d, aid);
                let v_nr = self.vars.var(&cap_name);
                if v_nr != u16::MAX {
                    captured_var_nrs.push(v_nr);
                    // mark as captured so test_used does not emit
                    // a false "never read" warning.  Do NOT call var_usages —
                    // that would interfere with the dead-assignment check.
                    self.vars.set_captured(v_nr);
                    alloc_steps.push(self.set_field_no_check(
                        closure_rec_d,
                        aid,
                        0,
                        Value::Var(w),
                        Value::Var(v_nr),
                    ));
                    // P259 / Plan-57 Phase B (Mechanism B): the closure record now
                    // holds a DbRef into the captured heap cell (`Reference(__cell_*,
                    // _)`) via the auto-Reference attribute, and the record OWNS that
                    // cell — `Stores::free_named`'s cascade (allocation.rs:301) frees
                    // it when the closure value dies.  No `OpIncRc` is emitted: the
                    // defining-frame `OpFreeRef` on the cell is suppressed in
                    // `get_free_vars` (scopes.rs) instead, so a captured cell survives
                    // a factory return without an rc bump.  This was the last
                    // load-bearing `OpIncRc`; dropping it unblocks removing the
                    // ref-count entirely (Phase C).
                }
            }
            self.last_closure_captured_vars = captured_var_nrs;
            // Block result: push d_nr (4B via OpConstInt) + closure DbRef (12B via OpVarRef).
            // Together these 16 bytes constitute the fn-ref slot value.
            alloc_steps.push(Value::FnRef(d_nr as i32, w, Box::new(fn_type.clone())));
            *code = crate::data::v_block(alloc_steps, fn_type, "fn_ref_with_closure");
            // A5.6-1/2: closure is embedded in fn-ref — no explicit call-site injection.
            self.last_closure_alloc = None;
            // propagate closure dep and work-buffer info to the
            // enclosing function's declared return type.  Two things are needed:
            // 1. The closure work var `w` in the Function dep list, so
            //    get_free_vars does not emit OpFreeRef for the closure record.
            // 2. The lambda's actual return type (with work-buffer deps), so
            //    try_fn_ref_call at the call site creates the right number of
            //    work buffers.  Without this, cross-scope fn-ref calls to
            //    text-returning lambdas crash because the work buffer is missing.
            if let Type::Function(params, _, _) = self.data.def(self.context).returned() {
                let params = params.clone();
                // H2 step 5: `w` is a FRAME var stored in the DEF-space home
                // (`Definition.returned`) — write it as a tagged
                // callee-frame note so readers decode the space instead of
                // guessing by attr-range position (`Deps::entries`).
                self.data.definitions[self.context as usize].returned =
                    Type::Function(params, Box::new(ret_tp), Deps::callee_frame1(w));
            }
            // record the work var so parse_assign can populate closure_vars
            // (used by write-back and native codegen's closure_var_of lookup).
            self.last_closure_work_var = w;
        } else {
            *code = Value::Int(d_nr as i32);
        }
    }

    /// Synthesize an anonymous struct definition for the captured variables
    /// of a lambda. Emits a diagnostic with the record layout for test verification.
    ///
    /// Plan-22 phase 02c (2026-05-12): for each mutated Reference
    /// capture (per phase 01's `mutated_captures` walker, populated
    /// in pass 1 by phase 02a), the attribute type is
    /// `Reference(d, [u16::MAX])` instead of `Reference(d, [])`.
    /// The non-empty dep activates phase 02b's auto-Reference
    /// storage encoding (12-byte DbRef + OpSetDbRef + OpGetDbRef
    /// instead of inline-bytes + OpCopyRecord + OpGetField).
    /// Mutations made inside the closure body propagate to the
    /// outer scope through the shared store record.
    ///
    /// The dep value `u16::MAX` is a SENTINEL meaning
    /// "auto-Reference share-marker" — it's not a real outer-var
    /// nr (we're inside the lambda's scope at synthesis time, so
    /// the outer var nr isn't directly accessible).  Phase 03
    /// (Case C) refines the dep value to the actual outer-var nr
    /// for proper liveness tracking.  For phase 02c's case-B
    /// scope (closure stays within capture's scope), the sentinel
    /// is sufficient — the closure record itself is freed at the
    /// outer scope's exit, after which the captured value is no
    /// longer reachable.
    /// Plan-22 phase 02d-ii — ensure a `__cell_<T>` struct exists
    /// for every scalar-typed mutated capture in
    /// `self.captured_names`.
    ///
    /// Idempotent: a cell struct created for one lambda is reused
    /// by every subsequent lambda that captures the same scalar
    /// type.  Gated on `self.first_pass` because struct definitions
    /// must be created in pass 1; pass 2 looks them up via
    /// `data.def_nr(name)` (no creation).
    ///
    /// Cells whose `cell_struct_name` returns `None` (exotic
    /// integer widths, Reference / Function / Vector / etc.) are
    /// silently skipped — phase 02d-iii's outer-binding rewrite
    /// detects the missing cell at the use site and falls back to
    /// today's stack-slot codegen.
    fn synthesize_cell_structs(&mut self, lambda_d_nr: u32) {
        if !self.first_pass {
            return;
        }
        let captures = self.captured_names.clone();
        let mutated: Vec<String> = self.data.def(lambda_d_nr).mutated_captures().to_vec();
        for (name, tp) in &captures {
            if !mutated.iter().any(|m| m == name) {
                continue;
            }
            let Some(cell_name) = cell_struct_name(tp, &self.data) else {
                continue;
            };
            // Idempotent: skip if the cell struct already exists.
            if self.data.def_nr(&cell_name) != u32::MAX {
                continue;
            }
            let cell_d_nr = self
                .data
                .add_def(&cell_name, self.lexer.pos(), DefType::Struct);
            let value_tp = cell_value_type(tp);
            self.data
                .add_attribute(&mut self.lexer, cell_d_nr, "value", value_tp);
        }
    }

    /// Plan-22 phase 02d-iii.a — at the start of pass 2 for the
    /// parent function, replace each scalar local in
    /// `Definition.scalars_to_box` with its boxed
    /// `Reference(__cell_<T>, [])` type.  Runs AFTER the
    /// pass-1 vars are restored via `Function::append`, so the
    /// helper sees every local that pass 1 added (including the
    /// names queued by phase 02d-i's accumulator).
    ///
    /// Foundation step: NO read or write rewriting yet.
    /// Subsequent sub-phases (02d-iii.b read auto-deref, 02d-iii.c
    /// first-set allocation, 02d-iii.d closure-body write rewrite,
    /// 02d-iii.e type-check gate) build on this type-flip.
    ///
    /// Today's behavior with this commit alone: the variables
    /// table reports the boxed type, but every emit site still
    /// treats `n` as a stack-slot scalar.  Any function with
    /// mutating scalar captures will hit a type-mismatch
    /// diagnostic at the first `n = …` site (RHS scalar vs LHS
    /// `Reference(__cell_<T>, _)`).  No existing test exercises
    /// mutating-scalar-capture closures (they're broken pre-02d
    /// and the matrix cells are still `#[ignore]`d), so the
    /// regression net stays green.
    ///
    /// `change_var_type` (in `src/parser/expressions.rs`) is
    /// taught to PRESERVE the flipped Reference type when the
    /// new type is the cell's value type; without that guard,
    /// `change_var(to, &s_type)` in `parse_assign_op` would
    /// revert `n` back to Integer on every `n = …` line.
    ///
    /// Idempotent + scoped:
    /// - No-op when `scalars_to_box` is empty (the common case
    ///   for every function in the standard library and existing
    ///   tests).
    /// - Skips arguments — boxing a function parameter would
    ///   change its call-site signature.  Argument boxing is a
    ///   follow-up sub-step (matrix row M / Case-B-on-arg uses
    ///   the explicit `Mutable<T>` path in phase 05).
    /// - Skips names whose `cell_struct_name` returns `None`
    ///   (exotic integer widths) — phase 02d-ii's silent gap.
    /// - Skips names already flipped on a re-entry (defensive).
    #[allow(
        dead_code,
        reason = "Helper exists for 02d-iii.e activation; only invoked from tests for now."
    )]
    pub(crate) fn flip_scalars_to_box_types(&mut self) {
        if self.context == u32::MAX || (self.context as usize) >= self.data.definitions.len() {
            return;
        }
        // Plan-22 phase 02d-vii — skip text-cell boxing when the
        // parent function returns text.  In a text-returning fn,
        // the work-text result-buffer machinery locks the
        // closure record's store before `emit_lambda_code`'s
        // SetDbRef can capture the boxed-text DbRef, panicking
        // with "Write to locked store".  Reverting text vars to
        // their pre-02d-vi behaviour (no flip → text mutation
        // flows through the existing void-return write-back
        // mechanism, which works for the b_d1 shape that
        // text-returning fns are most likely to use).
        let parent_returns_text = matches!(self.data.def(self.context).returned(), Type::Text(_));
        let names = self.data.def(self.context).scalars_to_box().to_vec();
        for name in &names {
            let v_nr = self.vars.var(name);
            if v_nr == u16::MAX {
                continue;
            }
            if self.vars.is_argument(v_nr) {
                continue;
            }
            let original_tp = self.vars.tp(v_nr).clone();
            // Skip if already flipped.
            if matches!(&original_tp, Type::Reference(d, _)
                if self.data.def(*d).name().starts_with("__cell_"))
            {
                continue;
            }
            // Plan-22 phase 02d-vii — text-skip when parent
            // returns text (see above for rationale).  Skips
            // both bare Text and RefVar(Text) (mutable stack
            // text locals).
            let is_text_or_reftext = matches!(&original_tp, Type::Text(_))
                || matches!(&original_tp, Type::RefVar(inner)
                    if matches!(inner.as_ref(), Type::Text(_)));
            if parent_returns_text && is_text_or_reftext {
                continue;
            }
            // Plan-22 phase 02d-iii.e / 02d-v / 02d-vi — all
            // boxable scalar types now flip cleanly:
            //
            // - Direct shapes (Integer / Float / Single /
            //   Character / Text / plain Enum): auto-deref
            //   produces `Call(OpGet<T>, [Var, 0])`,
            //   `maybe_prepend_cell_alloc`'s
            //   `extract_boxed_var_from_lhs` recognises this
            //   shape, `call_to_set_op` maps `OpGet<T>` →
            //   `OpSet<T>` for the write side.
            //
            // - Boolean: auto-deref wraps in
            //   `Call(OpEqInt, [Call(OpGetByte, [Var, 0, 0]),
            //   Int(1)])`; phase 02d-v's `extract_boxed_var_from_lhs`
            //   recognises this nested shape too; writes route
            //   through towards_set's existing boolean branch
            //   (collections.rs:493) which produces the right
            //   OpSetByte IR with bool→byte conversion.
            //
            // - Text: phase 02d-vi added a bypass guard in
            //   `parse_assign_op` that skips the text-special
            //   branch when the LHS is auto-deref'd boxed text.
            //   The general path then handles re-assignment
            //   (`log = "after"`) and append (`log += s` lowered
            //   to `log = log + s`) via `OpSetText` writes
            //   through the cell DbRef.
            let Some(cell_name) = cell_struct_name(&original_tp, &self.data) else {
                continue;
            };
            let cell_d_nr = self.data.def_nr(&cell_name);
            if cell_d_nr == u32::MAX {
                continue;
            }
            self.vars
                .set_type(v_nr, Type::Reference(cell_d_nr, Deps::none()));
        }
    }

    /// #314 (closed by decision — GOALS.md § "Stability trumps
    /// features"): a mutated scalar captured by MORE THAN ONE closure
    /// is rejected at compile time.
    ///
    /// The shape only worked through shared heap cells with no defined
    /// owner ("first death wins" — `free_named` frees the cell at the
    /// first record's death and silently no-ops for the rest), and the
    /// closure-record attribute types freeze at each lambda's pass-1
    /// epilogue while `scalars_to_box` keeps accumulating until the
    /// parent's body end — so a reader lambda parsed before the writer
    /// baked in the unboxed layout and crashed at runtime.  No
    /// consumer needs the shape; shared mutable state belongs in a
    /// struct, which also makes the sharing visible in the source.
    ///
    /// Runs at the parent's pass-1 body end — the first moment the
    /// accumulation is final — for the same parents whose locals
    /// `flip_scalars_to_box_types` flips in pass 2 (named fns; the
    /// caller in `definitions.rs` is the flip's sibling).  The
    /// single-closure accumulator (one record, one owner) stays
    /// supported.
    pub(crate) fn reject_shared_mutable_scalar_captures(&mut self, parent_d: u32) {
        if !self.first_pass
            || parent_d == u32::MAX
            || (parent_d as usize) >= self.data.definitions.len()
        {
            return;
        }
        let Some(lambdas) = self.fn_lambdas.remove(&parent_d) else {
            return;
        };
        let scalars = self.data.def(parent_d).scalars_to_box().to_vec();
        if scalars.is_empty() {
            return;
        }
        for name in &scalars {
            let capturers = lambdas
                .iter()
                .filter(|&&lam| {
                    let rec = self.data.def(lam).closure_record();
                    rec != u32::MAX
                        && (0..self.data.attributes(rec))
                            .any(|a_nr| self.data.attr_name(rec, a_nr) == *name)
                })
                .count();
            if capturers >= 2 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "variable `{name}` is mutated through a closure and captured by \
                     {capturers} closures; sharing a mutable variable between closures \
                     is not supported — hold the shared state in a struct field instead \
                     (e.g. `state = State {{ {name}: ... }}` captured by all closures)"
                );
            }
        }
    }

    fn synthesize_closure_record(&mut self, lambda_d_nr: u32, lambda_name: &str) {
        let record_name = lambda_name.replace("__lambda_", "__closure_");
        let captures = self.captured_names.clone();

        if self.first_pass {
            // Create the struct definition in the first pass.
            let record_d_nr = self
                .data
                .add_def(&record_name, self.lexer.pos(), DefType::Struct);
            for (name, tp) in &captures {
                // P216: ensure the synthetic `__tuple<…>` struct exists
                // for any tuple-typed capture before `fill_database`
                // walks the closure record's attributes — without this,
                // `type_elm(&Type::Tuple(_))` returns `u32::MAX` and the
                // closure record's tuple-typed attribute is silently
                // skipped at `typedef.rs:381`, leaving the closure
                // record with size 0 and the `OpDatabase` allocation
                // panicking with "Incomplete record" at
                // `src/store.rs:227`.
                ensure_tuple_defs_for_capture(&mut self.data, &mut self.lexer, tp);
                let attr_tp = match tp {
                    Type::Reference(d, _) => {
                        // P260 (2026-05-13): ALL Reference captures
                        // store as 12B Parts::DbRef pointing at the
                        // live original (typedef.rs:529 arm fires on
                        // non-empty deps).  Inline-byte storage was
                        // wrong even for read-only captures — the
                        // closure read sees a stale snapshot when the
                        // outer scope mutates a non-scalar field of
                        // the source struct, AND closure-side writes
                        // to compound fields (vector field, nested
                        // struct, vector element) silently no-op
                        // against the inline copy.  Originally this
                        // arm was gated on `is_mutated(name)` (phase
                        // 02c) but the gate is wrong: storage
                        // encoding is an architectural decision
                        // ("don't deep-copy a possibly-large struct
                        // into a closure record"), not a property of
                        // whether THIS closure mutates the capture.
                        // The auto-Reference marker (vec![u16::MAX])
                        // is only consumed by `typedef.rs::fill_database`
                        // — closure records are the sole producer, so
                        // this doesn't affect user-defined struct
                        // fields.
                        Type::Reference(*d, Deps::share_sentinel())
                    }
                    _ => tp.clone(),
                };
                self.data
                    .add_attribute(&mut self.lexer, record_d_nr, name, attr_tp);
            }
            // Store the closure record def_nr on the lambda's definition.
            self.data.definitions[lambda_d_nr as usize].closure_record = record_d_nr;
        } else {
            let record_d_nr = self.data.def_nr(&record_name);
            if record_d_nr != u32::MAX {
                self.data.definitions[lambda_d_nr as usize].closure_record = record_d_nr;
            }
        }
    }

    // <for-vector> ::= 'for' <id> 'in' <range> ['if' <cond>] '{' <expr> '}'
    // Implements [for n in range { body }] vector comprehensions.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn parse_vector_for(
        &mut self,
        vec: u16,
        elm: u16,
        in_t: &mut Type,
        val: &mut Value,
        is_var: bool,
        is_field: bool,
        block: bool,
        parent_tp: &Type,
    ) -> Type {
        let Some(id) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect variable after for");
            return Type::Null;
        };
        self.lexer.token("in");
        let loop_nr = self.vars.start_loop();
        let mut expr = Value::Null;
        let mut in_type = self.parse_in_range(&mut expr, &Value::Null, &id);
        let mut fill = Value::Null;
        if matches!(in_type, Type::Vector(_, _)) {
            let vec_var = self.create_unique("vector", &in_type);
            in_type = in_type.depending(vec_var);
            fill = v_set(vec_var, expr);
            expr = Value::Var(vec_var);
        }
        let var_tp = self.for_type(&in_type);
        let (iter_var, pre_var) = if matches!(in_type, Type::Text(_)) {
            let pos_var = self.create_var(&format!("{id}#next"), &I32);
            self.vars.defined(pos_var);
            let index_var = self.create_var(&format!("{id}#index"), &I32);
            self.vars.defined(index_var);
            (pos_var, Some(index_var))
        } else {
            let iv = self.create_var(&format!("{id}#index"), &I32);
            self.vars.defined(iv);
            (iv, None)
        };
        let for_var = self.create_var(&id, &var_tp);
        self.vars.defined(for_var);
        let if_step = if self.lexer.has_token("if") {
            let mut if_expr = Value::Null;
            self.expression(&mut if_expr);
            if_expr
        } else {
            Value::Null
        };
        let mut create_iter = expr;
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let iter_next = self.iterator(&mut create_iter, &in_type, &it, iter_var, pre_var);
        if !self.first_pass && iter_next == Value::Null {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Need an iterable expression in a for statement"
            );
            return Type::Null;
        }
        let for_next = v_set(for_var, iter_next);
        self.vars.loop_var(for_var);
        let in_loop = self.in_loop;
        self.in_loop = true;
        // Parse body as an expression-returning block: [for n in range { expr }]
        // @PLN25 — when the DECLARED element type is the synthetic `__nullable<S>`
        // enum (a `v: vector<S> = [for … { S{…} }]`, rewritten at the vector-type
        // chokepoint), pass it as the block's expected type so the body's `S{…}`
        // builds the `Some` variant (via parse_block's enum-hint) instead of a
        // bare `S` that then mismatches the rewritten vector type.  Other element
        // types keep `Unknown` — no behaviour change for non-nullable
        // comprehensions.
        let body_expected = if matches!(&*in_t, Type::Enum(e, true, _)
            if self.data.def(*e).name.starts_with("__nullable<"))
        {
            in_t.clone()
        } else if in_t.is_unknown() && self.e2_rewrite_enabled() {
            // @PLN25 — INFERRED comprehension (no element annotation): peek the
            // body for a leading struct-literal `{ S{…} }` and default the
            // element to `__nullable<S>`, mirroring the inferred-literal PEEK in
            // `parse_vector`.  The body then builds `Some` (parse_block enum-hint)
            // and `*in_t` becomes the enum below — so the result matches every
            // DECLARED `vector<S>` (now `vector<__nullable<S>>`).  Without it an
            // inferred `v = [for … { S{…} }]` stayed dense `vector<S>`: `v[i] =
            // null` was a silent no-op and passing `v` to a `vector<S>` parameter
            // mismatched.  A PEEK only (reverted); a body whose first token is not
            // a struct literal (multi-statement, scalar) stays dense.
            let link = self.lexer.link();
            let mut peeked = Type::Unknown(0);
            self.lexer.has_token("{");
            if let Some(name) = self.lexer.has_identifier()
                && self.lexer.peek_token("{")
            {
                let d = self.data.def_nr(&name);
                if d != u32::MAX
                    && self.data.def_type(d) == DefType::Struct
                    && self.data.def(d).synthetic.is_none()
                {
                    let syn = self.data.nullable_enum_for(&mut self.lexer, d);
                    peeked = Type::Enum(syn, true, Deps::none());
                }
            }
            self.lexer.revert(link);
            peeked
        } else {
            Type::Unknown(0)
        };
        let mut body = Value::Null;
        let body_type = self.parse_block("for", &mut body, &body_expected);
        // #319 — a struct-literal body returns `Rewritten(Reference(...))`.
        // The wrapper is a parse-internal marker, not an element type:
        // leaking it into the vector's element type broke every later
        // `qs[i] ?? …` on the comprehension result (the ncc temp lost its
        // dep chain and stack slot — "Incorrect var __ncc_N[65535]").
        *in_t = if let Type::Rewritten(t) = body_type {
            *t
        } else {
            body_type
        };
        self.in_loop = in_loop;
        self.vars.finish_loop(loop_nr);
        // Finalise vector element type (same as parse_vector post-loop)
        let struct_tp = Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
        if !is_field {
            self.vars
                .change_var_type(vec, &struct_tp, &self.data, &mut self.lexer);
            self.data.vector_def(&mut self.lexer, in_t);
        }
        let tp = Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
        if self.first_pass {
            return tp;
        }
        // O8.5: try const-unrolling for [for i in A..B [if cond] { expr(i) }].
        // If the range bounds are const and the body folds for every i,
        // emit a pre-computed literal vector instead of a runtime loop.
        if matches!(in_t, Type::Integer(_))
            && let Some(unrolled) =
                self.try_const_unroll_comprehension(for_var, &create_iter, &body, &if_step, in_t)
        {
            let parent_tp = &Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
            let (tp, ls) = self.build_vector_list(
                val, parent_tp, elm, vec, &unrolled, in_t, tp, is_var, is_field,
            );
            *val = if !is_var && !is_field {
                v_block(ls, tp.clone(), "Const comprehension")
            } else {
                Value::Insert(ls)
            };
            return tp;
        }
        // Second pass: build the append-in-loop bytecode.
        // For field assignments (vec == u16::MAX), pass the original field expression
        // so comprehension code can reference it instead of Value::Var(u16::MAX).
        let vec_expr = if vec == u16::MAX {
            val.clone()
        } else {
            Value::Var(vec)
        };
        self.build_comprehension_code(
            vec,
            &vec_expr,
            elm,
            in_t,
            &in_type,
            &var_tp,
            for_var,
            for_next,
            pre_var,
            fill,
            create_iter,
            if_step,
            body,
            val,
            is_var,
            is_field,
            block,
            tp,
        )
    }

    /// Build the second-pass bytecode for a `[for ... { body }]` vector comprehension.
    // parser helper threading IR-construction params alongside &mut self; no sensible grouping reduces the count
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_comprehension_code(
        &mut self,
        vec: u16,
        vec_expr: &Value,
        elm: u16,
        in_t: &Type,
        in_type: &Type,
        var_tp: &Type,
        for_var: u16,
        for_next: Value,
        pre_var: Option<u16>,
        fill: Value,
        create_iter: Value,
        if_step: Value,
        body: Value,
        val: &mut Value,
        is_var: bool,
        is_field: bool,
        block: bool,
        mut tp: Type,
    ) -> Type {
        // Per-iteration: OpNewRecord / set_field / OpFinishRecord pattern.
        let ed_nr = self.data.type_def_nr(in_t);
        // @PLAN58 cluster IV: use the SAME element-type `known` as the proven
        // `vv += [inner]` path (`new_record`), not `vector(def(ed_nr).known_type)`
        // which over-wraps one level for a nested element — making `record_new`
        // stride the outer slot by the 4-byte handle while the read strides by 8
        // (off-by-one).  `vector_of(in_t)` gives the element type; for a sub-4
        // inner (boolean) pass the outer vector type so the handle strides by 4.
        let elem_known = self.vector_of(in_t);
        let known = Value::Int(i32::from(if elem_known == u16::MAX {
            0
        } else if matches!(in_t, Type::Vector(_, _))
            && self.database.size(self.database.content(elem_known)) < 4
        {
            self.database.vector(elem_known)
        } else {
            elem_known
        }));
        let fld = Value::Int(i32::from(u16::MAX));
        let comp_var = self.create_unique("comp", in_t);
        // @P325 — coroutine comprehensions `[for v in gen() { … }]` had NO
        // termination check in the loop body (the `!matches!(Iterator)`
        // guard below skipped it entirely), so they ran forever appending
        // to the result vector until the underlying store overflowed its
        // 2 GiB word limit (`src/store.rs:643`).  Mirror the @P327 fix in
        // `collections.rs::iter_for`: when iterating a coroutine, emit
        // `OpCoroutineExhausted(__gen_N)` as the loop's break condition.
        // The generator var is the first arg of `OpCoroutineNext` inside
        // `for_next` (`Set(for_var, OpCoroutineNext(__gen_N, value_size))`).
        let coroutine_gen_var = if matches!(in_type, Type::Iterator(_, _))
            && let Value::Set(_, rhs) = &for_next
            && let Value::Call(_, next_args) = rhs.as_ref()
            && let Some(Value::Var(v)) = next_args.first()
        {
            *v
        } else {
            u16::MAX
        };
        let mut lp = vec![for_next];
        if !matches!(in_type, Type::Iterator(_, _)) {
            let mut test_for = Value::Var(for_var);
            self.convert(&mut test_for, var_tp, &Type::Boolean);
            test_for = self.cl("OpNot", &[test_for]);
            lp.push(v_if(
                test_for,
                v_block(vec![Value::Break(0)], Type::Void, "break"),
                Value::Null,
            ));
        } else if coroutine_gen_var != u16::MAX {
            let test_exhausted = self.cl("OpCoroutineExhausted", &[Value::Var(coroutine_gen_var)]);
            lp.push(v_if(
                test_exhausted,
                v_block(vec![Value::Break(0)], Type::Void, "break"),
                Value::Null,
            ));
        }
        if if_step != Value::Null {
            lp.push(v_if(if_step, Value::Null, Value::Continue(0)));
        }
        lp.push(v_set(comp_var, body));
        lp.push(v_set(
            elm,
            self.cl(
                "OpNewRecord",
                &[vec_expr.clone(), known.clone(), fld.clone()],
            ),
        ));
        // @PLAN58 cluster IV: a NESTED comprehension's body is a vector (a 12-byte
        // DbRef handle).  The scalar `set_field(usize::MAX)` path emits `OpSetInt4`
        // (4 of 12 bytes) → eval-stack skew → garbage rec-id into the locked
        // CONST_STORE.  Deep-copy the inner record instead.  Scalar elements keep
        // `set_field`.
        if let Type::Vector(elem_tp, _) = in_t {
            lp.push(self.cl(
                "OpSetInt4",
                &[Value::Var(elm), Value::Int(0), Value::Int(0)],
            ));
            let elem_known = self.database.db_type(elem_tp, &self.data);
            let type_nr = Value::Int(i32::from(self.database.vector(elem_known)));
            lp.push(self.cl(
                "OpCopyRecord",
                &[Value::Var(comp_var), Value::Var(elm), type_nr],
            ));
        } else {
            lp.push(self.set_field(ed_nr, usize::MAX, 0, Value::Var(elm), Value::Var(comp_var)));
        }
        lp.push(self.cl(
            "OpFinishRecord",
            &[vec_expr.clone(), Value::Var(elm), known, fld],
        ));
        let mut for_steps: Vec<Value> = Vec::new();
        if fill != Value::Null {
            for_steps.push(fill);
        }
        if let Some(idx_var) = pre_var {
            for_steps.push(v_set(idx_var, Value::Int(0)));
        }
        for_steps.push(create_iter);
        for_steps.push(v_loop(lp, "For comprehension"));
        let mut ls: Vec<Value> = Vec::new();
        if block {
            ls.extend(self.vector_db(in_t, vec));
            // After vector_db, vec's type carries the db dependency.  Propagate that
            // into tp so that (a) the block's result type keeps the db alive until the
            // block exits, and (b) the caller receives the correct Vector<T,[db]> type,
            // preventing scopes from emitting a redundant OpFreeRef for the result variable.
            if let Type::Vector(elem, _) = &tp {
                tp = Type::Vector(elem.clone(), Deps::frame(self.vars.tp(vec).depend()));
            }
        }
        ls.extend(for_steps);
        if self.vector_needs_db(vec, in_t, is_var) {
            let db = self.insert_new(vec, elm, in_t, &mut ls);
            self.vars.depend(vec, db);
        } else if !is_field && !is_var && *val != Value::Null {
            ls.insert(0, v_set(vec, val.clone()));
        }
        if !is_var && !is_field {
            ls.push(Value::Var(vec));
        }
        *val = if block || (!is_var && !is_field) {
            v_block(ls, tp.clone(), "Vector comprehension")
        } else {
            Value::Insert(ls)
        };
        tp
    }

    /**
    Fill a structure (vector) with values. This can be done in different situations:
    - On a new variable, this creates a variable pointing to a structure with the vector.
    - As a stand-alone expression, this creates a new structure of type vector.
    - On an existing variable, this fills (or replaces) the vector with more elements.
    - On a field inside a structure, this fills any data structure with more elements.
    */
    // <vector> ::= '[' <expr> [ ';' <size-expr>]{ ',' <expr> [ ';' <size-expr> } ']'
    pub(crate) fn parse_vector(
        &mut self,
        var_tp: &Type,
        val: &mut Value,
        parent_tp: &Type,
    ) -> Type {
        let mut assign_tp = var_tp.content();
        // @PLN25 E2 — a KEYED collection's `content()` yields `Reference(__nullable<S>)`
        // (Hash/Sorted/Index wrap the content def in a Reference), but literal-element
        // construction needs the inline `Enum(.., true)` form so each `S{…}` builds the
        // `Some` variant (a vector's `content()` already yields `Enum(.., true)` directly,
        // which is why `vector<S>` fields work but keyed fields hit "cannot store S in
        // vector<__nullable<S>>").  Normalize so `hash<S[k]> += [S{…}]` and keyed-field
        // initialisers take the same Some-construction path as a vector.  Inert gate-off
        // (no content def is ever a `__nullable<` enum).
        if let Type::Reference(d, dep) = &assign_tp
            && self.data.def(*d).name.starts_with("__nullable<")
        {
            assign_tp = Type::Enum(*d, true, dep.clone());
        }
        // @PLN25 — INFERRED struct-literal vector default: with no declared element
        // type (`var_tp` Unknown — an inferred local `v = [Row{…}]`, a fn return
        // body `{ [Row{…}] }`, …) and a first item that is a struct literal `S{…}`,
        // default the element to the synthetic `__nullable<S>` enum so the elements
        // build `Some` — matching the `vector<__nullable<S>>` that every DECLARED
        // site now resolves to (the construction half of the representation).  A
        // PEEK only (reverted); fires solely for an inferred struct-literal vector,
        // so `[1.0]` / `[1,2]`, index expressions, and `not null` / declared vectors
        // are untouched.  Native stdlib (STD_SOURCE) stays dense.
        if assign_tp.is_unknown() && self.e2_rewrite_enabled() {
            let link = self.lexer.link();
            if let Some(first) = self.lexer.has_identifier() {
                // A library-qualified struct literal (`lib::S { … }`) reads as TWO
                // identifiers around `::`; the bare-identifier peek saw only `lib`,
                // missed the `{`, and left the inferred literal DENSE while DECLARED
                // `lib::S` sites are nullable — the type mismatch at `v += [lib::S{…}]`.
                // Skip past `::` to the real struct name (last segment) before the `{`.
                let struct_name = if self.lexer.has_token("::") {
                    self.lexer.has_identifier()
                } else {
                    Some(first)
                };
                if let Some(sname) = struct_name
                    && self.lexer.peek_token("{")
                {
                    let d = self.data.def_nr(&sname);
                    if d != u32::MAX
                        && self.data.def_type(d) == DefType::Struct
                        && self.data.def(d).synthetic.is_none()
                    {
                        let syn = self.data.nullable_enum_for(&mut self.lexer, d);
                        // @PLN25 — for a FORWARD-referenced struct `S` the synth `__nullable<S>`
                        // enum is first created HERE in pass-2 body parse, after `fill_all` ran, so
                        // it is unregistered.  Lay it out NOW (this is the EARLIEST site — before
                        // both the construction and the read bake their payload offset / element
                        // stride) so they bake correct values (371).  No-op once registered.
                        if !self.first_pass {
                            crate::typedef::register_and_lay_out_synth(
                                &mut self.data,
                                &mut self.database,
                                syn,
                            );
                        }
                        assign_tp = Type::Enum(syn, true, Deps::none());
                    }
                }
            }
            self.lexer.revert(link);
        }
        // @P315 — `declared` is true when the element type comes from a typed
        // target (typed local / struct field), false when it is inferred from
        // an untyped literal.  A declared element type must NOT be silently
        // promoted to a wider type by `parse_item` (that changes the element
        // storage width and loses data); require an explicit `as` cast.
        let declared = !assign_tp.is_unknown();
        let is_field = self.is_field(val);
        let is_var = matches!(val, Value::Var(_));
        // Empty `[]`.  A new variable / struct field keeps the lightweight
        // placeholder (its store is zero-initialised elsewhere), as does an
        // UNTYPED standalone `[]` (no element-type hint to size a store).  But a
        // TYPED standalone `[]` — e.g. `return []` with the function's return
        // type threaded in (@P365) — falls through to the normal construction
        // path so it materialises a REAL empty vector store; the placeholder
        // (`Insert([Null])`) otherwise lowers to `()` → native E0308 / interpret
        // garbage-handle crash.
        if self.lexer.peek_token("]") {
            if is_var {
                self.lexer.has_token("]");
                *val = Value::Insert(vec![]);
                return Type::Rewritten(Box::new(var_tp.clone()));
            }
            if is_field {
                // The field is already zero-initialized by OpDatabase; nothing to
                // emit.  Wrapping the OpGetField result in Value::Insert would
                // leave a dangling 12-byte DbRef on the expression stack.
                self.lexer.has_token("]");
                *val = Value::Insert(vec![]);
                return var_tp.clone();
            }
            if assign_tp.is_unknown() {
                self.lexer.has_token("]");
                *val = Value::Insert(vec![val.clone()]);
                return var_tp.clone();
            }
            // Typed standalone empty — fall through; `]` is consumed by the
            // `self.lexer.token("]")` at the end of the construction path.
        }
        let block = !is_field && !matches!(val, Value::Var(_));
        let vec = if is_field {
            u16::MAX
        } else if let Value::Var(nr) = val {
            *nr
        } else {
            self.create_unique(
                "vec",
                &Type::Vector(Box::new(assign_tp.clone()), Deps::frame(parent_tp.depend())),
            )
        };
        let mut in_t = assign_tp.clone();
        let mut res = Vec::new();
        let elm = self.unique_elm_var(parent_tp, &assign_tp, vec);
        if is_field {
            // elm is a reference INTO an existing field's store — the owning struct's
            // variable already emits FreeRef at scope exit.  Suppress FreeRef for elm
            // to prevent a double-free.
            self.vars.set_skip_free(elm);
        }
        // A typed standalone empty `[]` (the @P365 fall-through above) has no
        // items and no `for` comprehension — skip straight to the empty build.
        if !self.lexer.peek_token("]") {
            // Handle [for n in range [if cond] { body }] vector comprehension
            if self.lexer.peek_token("for") {
                self.lexer.has_token("for");
                let tp = self
                    .parse_vector_for(vec, elm, &mut in_t, val, is_var, is_field, block, parent_tp);
                self.lexer.token("]");
                return tp;
            }
            if let Some(early) = self.collect_vector_items(elm, &mut in_t, declared, &mut res) {
                return early;
            }
        }
        // convert parts to the common type
        if in_t == Type::Null {
            return in_t;
        }
        let struct_tp = Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
        if !is_field {
            self.vars
                .change_var_type(vec, &struct_tp, &self.data, &mut self.lexer);
            self.data.vector_def(&mut self.lexer, &in_t);
        }
        let tp = Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
        let (tp, ls) =
            self.build_vector_list(val, parent_tp, elm, vec, &res, &in_t, tp, is_var, is_field);
        self.lexer.token("]");
        if block {
            *val = v_block(ls, tp.clone(), "Vector");
        } else {
            *val = Value::Insert(ls);
        }
        tp
    }

    /// Parse comma-separated vector items inside `[...]`, returning an early error type on failure.
    pub(crate) fn collect_vector_items(
        &mut self,
        elm: u16,
        in_t: &mut Type,
        declared: bool,
        res: &mut Vec<Value>,
    ) -> Option<Type> {
        loop {
            if let Some(value) = self.parse_item(elm, in_t, declared, res) {
                return Some(value);
            }
            if self.lexer.has_token(";")
                && let Some(value) = self.parse_multiply(res)
            {
                return Some(value);
            }
            if !self.lexer.has_token(",") {
                break;
            }
            if self.lexer.peek_token("]") {
                break;
            }
        }
        None
    }

    /// Build the instruction list for a parsed vector literal; returns `(tp, ls)`.
    #[allow(clippy::too_many_arguments)] // parser helper threading IR-construction params alongside &mut self; no sensible grouping reduces the count
    pub(crate) fn build_vector_list(
        &mut self,
        val: &mut Value,
        parent_tp: &Type,
        elm: u16,
        vec: u16,
        res: &[Value],
        in_t: &Type,
        mut tp: Type,
        is_var: bool,
        is_field: bool,
    ) -> (Type, Vec<Value>) {
        let mut ls = Vec::new();
        // Only create a fresh database record here when the variable has no existing
        // one (dep is empty).  For `v += [...]` the variable already has a dep from
        // the initial `=` assignment; calling vector_db again would reset v to an
        // empty record and discard the existing elements.  create_vector handles
        // the `=` re-assignment case by calling vector_db unconditionally.
        if self.vars.tp(vec).depend().is_empty() {
            ls.extend(self.vector_db(in_t, vec));
        }
        // O8.1a: pre-allocate vector capacity when the element count is known
        // at compile time.  This eliminates resize calls in vector_append.
        if !self.first_pass && !res.is_empty() && vec != u16::MAX {
            let ed_nr = self.data.type_def_nr(in_t);
            if ed_nr != u32::MAX {
                let known = self.data.def(ed_nr).known_type();
                if known != u16::MAX {
                    let elem_size = self.database.size(known);
                    if elem_size > 0 {
                        ls.push(self.cl(
                            "OpPreAllocVector",
                            &[
                                Value::Var(vec),
                                Value::Int(res.len() as i32),
                                Value::Int(i32::from(elem_size)),
                            ],
                        ));
                    }
                }
            }
        }
        ls.extend(self.new_record(val, parent_tp, elm, vec, res, in_t));
        if !self.first_pass
            && vec != u16::MAX
            && !self.vars.is_argument(vec)
            && self.vector_needs_db(vec, in_t, is_var)
        {
            let db = self.insert_new(vec, elm, in_t, &mut ls);
            self.vars.depend(vec, db);
            tp = tp.depending(db);
        } else if !is_field && !is_var && *val != Value::Null {
            ls.insert(0, v_set(vec, val.clone()));
        }
        if !is_var && !is_field {
            ls.push(Value::Var(vec));
            for d in self.vars.tp(vec).depend() {
                tp = tp.depending(d);
            }
        }
        (tp, ls)
    }

    /// O8.5: try to const-unroll a comprehension into a literal vector.
    /// Returns Some(vec of folded values) if successful, None to fall back to runtime loop.
    fn try_const_unroll_comprehension(
        &self,
        for_var: u16,
        _create_iter: &Value,
        body: &Value,
        if_step: &Value,
        _in_t: &Type,
    ) -> Option<Vec<Value>> {
        use crate::const_eval::{const_eval, const_eval_with_var};
        // Extract range bounds captured during parse_in_range_body.
        let from = const_eval(self.last_range_from.as_ref()?, &self.data)?;
        let till = const_eval(self.last_range_till.as_ref()?, &self.data)?;
        let (from_i, till_i) = match (&from, &till) {
            (Value::Int(a), Value::Int(b)) => (*a, *b),
            _ => return None,
        };
        if from_i >= till_i {
            return Some(Vec::new()); // empty range
        }
        let count = (till_i - from_i) as u32;
        if count > 10_000 {
            return None; // S7: size limit
        }
        let has_filter = *if_step != Value::Null;
        let mut values = Vec::with_capacity(count as usize);
        for i in from_i..till_i {
            let iv = Value::Int(i);
            // Check filter condition if present.
            if has_filter {
                match const_eval_with_var(if_step, for_var, &iv, &self.data) {
                    Some(Value::Boolean(true)) => {}         // include
                    Some(Value::Boolean(false)) => continue, // skip
                    _ => return None,                        // filter not const-foldable
                }
            }
            let folded = const_eval_with_var(body, for_var, &iv, &self.data)?;
            values.push(folded);
        }
        Some(values)
    }

    pub(crate) fn vector_needs_db(&self, vec: u16, in_t: &Type, is_var: bool) -> bool {
        is_var
            && *in_t != Type::Void
            && self.vars.tp(vec).depend().is_empty()
            && !matches!(self.vars.tp(vec), Type::RefVar(_))
            // Argument vectors already have a caller-provided backing store; do not
            // allocate a local __vdb_N store that would be freed before the return.
            && !self.vars.is_argument(vec)
    }

    pub(crate) fn unique_elm_var(&mut self, parent_tp: &Type, assign_tp: &Type, vec: u16) -> u16 {
        let c_tp = parent_tp.content();
        let was = Type::Reference(
            if c_tp.is_unknown() {
                0
            } else {
                self.data.type_def_nr(&c_tp)
            },
            Deps::frame(parent_tp.depend()),
        );
        // @PLN25 E2 — an ANONYMOUS `{ … }` element against a nullable synth enum
        // `__nullable<S>` has no struct name to drive the transparent-construction
        // path (objects.rs:151, which keys on `S{` matching `__nullable<S>`), so it
        // falls to `parse_block`'s `Reference(r)` record-scan.  Type the element var
        // as the `Some` variant so that scan builds the present payload (the
        // discriminant defaults present via object_init), exactly as the NAMED
        // `S{ … }` path does.  Without this the var falls back to `was`
        // (`Reference(type_def_nr(parent_tp.content))`) and mis-resolves to an
        // arbitrary def → "Unknown field <wrong-def>.<field>".  Gate-inert: a
        // `__nullable<>` enum only exists when the E2 rewrite is active.
        let elm_tp = if let Type::Reference(rd, _) = assign_tp {
            // @PLN25 E2 — a keyed-collection field's `content()` is
            // `Reference(__nullable<S>)` (not `Enum(..)`), so an anon `{ … }`
            // element assigned to a `hash`/`sorted`/`index` field must resolve
            // against the `Some` variant too — same as the `Enum(syn,true)`
            // (vector-element) case below.  Without this it points at the enum
            // and `.field` fails ("Unknown field __nullable<S>.field").
            if self.data.def(*rd).name.starts_with("__nullable<") {
                Type::Reference(
                    self.data.variant_of(*rd, "Some"),
                    Deps::frame(parent_tp.depend()),
                )
            } else {
                assign_tp.clone()
            }
        } else if let Type::Enum(syn, true, _) = assign_tp
            && self.data.def(*syn).name.starts_with("__nullable<")
        {
            Type::Reference(
                self.data.variant_of(*syn, "Some"),
                Deps::frame(parent_tp.depend()),
            )
        } else {
            was
        };
        let elm = self.create_unique("elm", &elm_tp);
        if vec != u16::MAX {
            self.vars.depend(elm, vec);
        }
        for on in Deps::frame(parent_tp.depend()) {
            self.vars.depend(elm, on);
        }
        elm
    }

    pub(crate) fn parse_multiply(&mut self, res: &mut Vec<Value>) -> Option<Type> {
        let mut code = Value::Null;
        let tp = self.parse_operators(&Type::Unknown(0), &mut code, &mut Type::Null, 0);
        if !matches!(tp, Type::Integer(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect a number as the object multiplier"
            );
            return Some(Type::Unknown(0));
        }
        res.push(Value::Return(Box::new(code)));
        None
    }

    // <item> ::== ['for' | <expr> ]
    pub(crate) fn parse_item(
        &mut self,
        elm: u16,
        in_t: &mut Type,
        declared: bool,
        res: &mut Vec<Value>,
    ) -> Option<Type> {
        let mut p = Value::Var(elm);
        // #247: isolate THIS element's capturing-lambda signal.  A capturing
        // lambda makes `emit_lambda_code` set `last_closure_work_var` (and emit
        // a Block, not a bare FnRef); a non-capturing lambda / non-lambda leaves
        // it MAX.  Reset first so a prior element's value can't leak in.
        self.last_closure_work_var = u16::MAX;
        let mut t = if self.lexer.has_token("for") {
            //self.iter_for(&mut p)
            diagnostic!(
                self.lexer,
                Level::Error,
                "For inside a vector is not yet implemented"
            );
            return Some(Type::Unknown(0));
        } else {
            let mut parent_tp = Type::Null;
            // @PLAN58 III-a: propagate the declared element type `in_t` into the
            // element parse so a NESTED literal's inner elements adopt the
            // declared narrow width (`vector<vector<i32>>` → inner `[1,2]` types
            // its elements `i32`, not wide `integer`).  When `in_t` is Unknown
            // (untyped inferred literal) this is identical to the prior
            // `Type::Unknown(0)` behaviour.
            self.parse_operators(&in_t.clone(), &mut p, &mut parent_tp, 0)
        };
        let elem_capturing_lambda = self.last_closure_work_var != u16::MAX;
        if let Type::Rewritten(tp) = in_t {
            *in_t = *tp.clone();
        }
        if let Type::Rewritten(tp) = t {
            t = *tp.clone();
        }
        if in_t.is_unknown() {
            *in_t = t.clone();
        }
        if t.is_unknown() {
            t = in_t.clone();
        }
        if let (Type::Reference(t_nr, _), Type::Reference(in_nr, _)) = (&t, &in_t.clone())
            && let (Type::Enum(t_e, true, _), Type::Enum(in_e, true, _)) = (
                self.data.def(*t_nr).returned(),
                self.data.def(*in_nr).returned(),
            )
            && *t_e == *in_e
        {
            *in_t = Type::Enum(*t_e, true, Deps::none());
        } else if let (Type::Enum(t_e, true, _), Type::Enum(in_e, true, _)) = (&t, &*in_t)
            && *t_e == *in_e
        {
            // @PLN25 E2 — the appended element ALREADY is the vector's nullable
            // enum (`result += [sa_sp]` where `sa_sp: __nullable<S>` and the
            // vector is `vector<__nullable<S>>`).  No conversion is needed — store
            // it as-is; the generic `convert` below does not recognise
            // enum→same-enum and would fire the spurious "would lose precision"
            // diagnostic.  Mirrors the `Reference`-to-same-enum arm above for the
            // case where the element is typed as the enum directly.
        } else if matches!(&*in_t, Type::Enum(syn, true, _) if self.data.def(*syn).name.starts_with("__nullable<"))
            && matches!(t, Type::Null)
        {
            // @PLN25 E2 — a `null` element in a `vector<__nullable<S>>` (e.g.
            // `v += [null]`): the appended element is the Null variant
            // (discriminant 0).  OpNewRecord zero-inits the element to disc 0, so
            // emit an empty construction; the generic convert would otherwise
            // reject `null` → the synthetic enum and fire the "cannot store"
            // diagnostic.
            p = Value::Insert(Vec::new());
            t = in_t.clone();
        } else if !self.first_pass
            && let Type::Enum(syn, true, _) = &*in_t
            && let Type::Reference(s_d, _) = &t
            && self.data.def(*syn).name == format!("__nullable<{}>", self.data.def(*s_d).name())
            && !matches!(p, Value::Insert(_))
        {
            // @PLN25 single-payload — store a DENSE struct value `S` into a
            // `vector<__nullable<S>>` element (`v += [p]`, `v += [make()]`): set the
            // discriminant present and copy the whole dense `S` into the inline `payload`
            // field (one record copy).  A non-Var source is stashed once so it is not
            // re-evaluated.  Gate-inert: `__nullable<>` exists only when E2 is active.
            let syn = *syn;
            let some_d = self.data.variant_of(syn, "Some");
            let mut steps = Vec::new();
            let src = if matches!(p, Value::Var(_)) {
                p.clone()
            } else {
                let tmp = self.create_unique("nbl_src", &t);
                steps.push(v_set(tmp, p.clone()));
                Value::Var(tmp)
            };
            steps.extend(self.build_some_present(some_d, Value::Var(elm), src));
            p = Value::Insert(steps);
            t = in_t.clone();
        } else if !self.convert(&mut p, &t, in_t) {
            if declared {
                // @P315 — the element type is DECLARED (typed local / struct
                // field).  A value that does not convert TO it must be cast
                // EXPLICITLY: silently promoting the declared element type
                // (e.g. Single→Float) would change the element storage WIDTH
                // (a vector<single> packs 4-byte slots; an 8-byte OpSetFloat
                // write overflows them → heap corruption) AND lose data
                // (float→single).  Consistent with scalar / local-vector
                // assignment, which already reject this.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot store {} elements in a vector<{}> (would lose precision); \
                     cast each element explicitly with 'as {}'",
                    t.name(&self.data),
                    in_t.name(&self.data),
                    in_t.name(&self.data)
                );
            } else if self.convert(&mut p, in_t, &t) {
                // INFERRED element type: widen to the common type
                // (e.g. [1, 2.0] → vector<float>).
                *in_t = t.clone();
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "No common type {} for vector {}",
                    t.name(&self.data),
                    in_t.name(&self.data)
                );
            }
        }
        if let Type::Enum(td_nr, true, _) = t
            && let Value::Enum(enum_nr, _) = &p
            && self.lexer.peek_token("{")
        {
            let mut ls = Vec::new();
            self.parse_enum_field(&mut ls, Value::Var(elm), td_nr, 0, *enum_nr);
            ls.push(p.clone());
            p = Value::Insert(ls);
        }
        // #247: a CAPTURING closure stored into a collection is not supported
        // yet (the co-located 16B closure-record layout is deferred —
        // @P213/@P214) and currently CRASHES at runtime ("Write to read-only
        // store"). Cleanly reject the statically-detectable shapes — a direct
        // capturing lambda, or a local that holds one — instead of crashing.
        // (A capturing closure RETURNED from a call, e.g. `[make(1)]`, has type
        // `fn()->T` indistinguishable from a non-capturing fn-ref and still
        // reaches the runtime path — that needs the deferred layout work.)
        if !self.first_pass && matches!(in_t, Type::Function(_, _, _)) {
            let capturing = elem_capturing_lambda
                || match p.unspan() {
                    Value::FnRef(_, clos_var, _) => *clos_var != u16::MAX,
                    Value::Var(v) => self.closure_vars.contains_key(v),
                    // A function that RETURNS a capturing closure carries the
                    // closure work-var in its `returned` Function dep list
                    // (emit_lambda_code, this file ~line 957) — so `[make(1)]`,
                    // a Call whose return type is `fn()->T` indistinguishable by
                    // signature, IS detectable via that non-empty dep list.
                    Value::Call(d_nr, _) => matches!(
                        self.data.def(*d_nr).returned(),
                        Type::Function(_, _, deps) if !deps.is_empty()
                    ),
                    _ => false,
                };
            if capturing {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a capturing closure cannot be stored in a collection yet — the \
                     co-located closure-record layout is deferred (@P213/@P214); hold the \
                     captured state separately (e.g. a struct field) and store a \
                     non-capturing fn that reads it"
                );
            }
        }
        res.push(p.clone());
        None
    }

    pub(crate) fn is_field(&self, val: &Value) -> bool {
        // Plan-07 phase 1: unspan() so wraps on `.` (step 1.12) don't
        // hide the field-access shape from compound-assignment dispatch.
        if let Value::Call(o, _) = val.unspan() {
            *o == self.data.def_nr("OpGetField")
        } else {
            false
        }
    }

    pub(crate) fn new_record_field_op(&mut self, val: &Value, parent_tp: &Type, op: &str) -> Value {
        if let Value::Call(_, ps) = val.unspan() {
            let parent = self.data.def(self.data.type_def_nr(parent_tp)).known_type();
            // @PLN25 single-payload: appending to a NESTED collection field of a `__nullable<S>`
            // element (`b.items += […]` where `b` is a nullable element) — the field-access
            // unwrap already made `ps[0]` the dense-`S` payload sub-ref and `ps[1]` its
            // S-relative offset, so resolve the field number against the payload's `S`, NOT the
            // enum (`field_nr(enum, S_offset)` = 0 → `OpNewRecord(field=0)` = the wrong field).
            // `key_owner` maps a synth `__nullable<S>` to its payload struct; identity otherwise.
            let parent = self.database.key_owner(parent);
            let field_nr = if let Value::Int(pos) = ps[1] {
                self.database.field_nr(parent, pos)
            } else {
                0
            };
            if op == "OpNewRecord" {
                self.cl(
                    "OpNewRecord",
                    &[
                        ps[0].clone(),
                        Value::Int(i32::from(parent)),
                        Value::Int(i32::from(field_nr)),
                    ],
                )
            } else {
                self.cl(
                    "OpFinishRecord",
                    &[
                        ps[0].clone(),
                        Value::Var(0), // placeholder, caller replaces with Value::Var(elm)
                        Value::Int(i32::from(parent)),
                        Value::Int(i32::from(field_nr)),
                    ],
                )
            }
        } else {
            Value::Null
        }
    }

    pub(crate) fn new_record(
        &mut self,
        val: &mut Value,
        parent_tp: &Type,
        elm: u16,
        vec: u16,
        res: &[Value],
        in_t: &Type,
    ) -> Vec<Value> {
        let mut ls = Vec::new();
        let is_field = self.is_field(val);
        let ed_nr = self.data.type_def_nr(in_t);
        assert_ne!(
            ed_nr,
            u32::MAX,
            "Unknown type {} at {}",
            in_t.name(&self.data),
            self.lexer.pos()
        );
        // P188: when the LHS local is a keyed collection
        // (sorted/hash/index/spacial<T[key]>), the container type id
        // must be the keyed-collection's own known_type so OpNewRecord
        // dispatches to sorted_new / hash::add / tree::add / etc.
        // Falling back to `vector_of(in_t)` returns the wrap-`vector<T>`
        // id which would route through `Parts::Vector` → vector_append
        // and crash with index 65535.
        //
        // P188-followup: previously this used
        // `data.def(type_def_nr(lhs_tp)).known_type`, but
        // `type_def_nr` returns the GENERIC alias (`hash` / `index`)
        // not the specific `hash<Score[name]>` instantiation.  The
        // alias's `known_type` happened to be a vector type, so
        // record_finish dispatched through `Parts::Vector` instead of
        // `Parts::Hash`/`Parts::Index` — producing 6 records for 3
        // adds (vector_finish appends without dedup) and bypassing
        // tree::add entirely (1 record for 2 adds).  Fix: register
        // the keyed-collection db type directly (idempotent — same
        // call as gen_set_first_keyed_null and the typedef walker).
        let lhs_known = if !is_field && vec != u16::MAX && !self.first_pass {
            let lhs_tp = self.vars.tp(vec).clone();
            match &lhs_tp {
                Type::Sorted(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    if c == u16::MAX {
                        None
                    } else {
                        Some(self.database.sorted(c, key))
                    }
                }
                Type::Hash(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    if c == u16::MAX {
                        None
                    } else {
                        Some(self.database.hash(c, key))
                    }
                }
                Type::Index(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    if c == u16::MAX {
                        None
                    } else {
                        Some(self.database.index(c, key))
                    }
                }
                Type::Spacial(td, key, _) => {
                    let c = self.data.def(*td).known_type();
                    if c == u16::MAX {
                        None
                    } else {
                        Some(self.database.spacial(c, key))
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        // #246: `vv[i] += [...]` — `is_field` is true (the LHS is an indexed
        // access) but the parent is a VECTOR, not a struct, so there is no
        // struct/field to locate.  `parse_part`'s `[` branch now records the
        // indexed container's type as `parent_tp`, so a `Type::Vector` parent is
        // the signal — covering both `vv[0]` and `h.vv[0]` (a vector element
        // reached through a struct field, where `parent_tp` would otherwise be
        // the stale struct from the preceding `.field`).  Append directly to the
        // inner vector that `val`'s read yields (`ps[0]`), with `fld = u16::MAX`,
        // mirroring the plain-local path — instead of routing through
        // `new_record_field_op` (struct-only, which mis-locates a field).
        let vector_elem_target: Option<Value> = if is_field
            && matches!(parent_tp, Type::Vector(_, _))
            && let Value::Call(_, ps) = val.unspan()
            && !ps.is_empty()
        {
            Some(ps[0].clone())
        } else {
            None
        };
        for p in res {
            // route through `vector_of` so narrow integer
            // aliases (i32, u8) produce the same narrow-element vector
            // db type as struct fields get via `fill_database`.  Without
            // this the literal-append path would register
            // `vector<integer>` (8-byte stride) into a narrow-registered
            // local, and reads would mis-align with writes.
            // @PLAN58 cluster-I (boolean outer-handle stride): for a nested
            // element (`in_t` is a vector), the OUTER vector stores 4-byte rec-id
            // HANDLES.  `vector_of(in_t)` yields the ELEMENT type whose content is
            // the inner scalar — so `record_new` strides the outer slot by the
            // inner scalar size.  ≥4 is fine, but a 1-byte `boolean` inner makes
            // adjacent handles OVERLAP.  When the inner content is <4 bytes, pass
            // the OUTER vector type (`vector(elem)`) so `record_new` strides by the
            // handle size (4).  Integer/single (≥4) and the inner scalar append
            // (`in_t` not a vector) are untouched.
            let elem_known = lhs_known.unwrap_or_else(|| self.vector_of(in_t));
            let known_tp = if matches!(in_t, Type::Vector(_, _))
                && self.database.size(self.database.content(elem_known)) < 4
            {
                self.database.vector(elem_known)
            } else {
                elem_known
            };
            let known = Value::Int(i32::from(known_tp));
            if let Value::Return(multiply) = p {
                let to = if let Value::Call(_, ps) = val {
                    ps[0].clone()
                } else {
                    Value::Var(vec)
                };
                ls.push(self.cl("OpAppendCopy", &[to, *multiply.clone(), known]));
                continue;
            }
            let fld = Value::Int(i32::from(u16::MAX));
            let app_v = if let Some(target) = &vector_elem_target {
                // #246: append directly to the inner vector (the indexed read).
                self.cl("OpNewRecord", &[target.clone(), known.clone(), fld.clone()])
            } else if is_field {
                self.new_record_field_op(val, parent_tp, "OpNewRecord")
            } else {
                self.cl(
                    "OpNewRecord",
                    &[Value::Var(vec), known.clone(), fld.clone()],
                )
            };
            ls.push(v_set(elm, app_v));
            // @P380 (generalized, plan-58 cluster II): a freshly-created
            // vector-of-vectors element is a VECTOR HANDLE (rec-id at offset 0),
            // but `OpNewRecord` default-inits it with the mis-resolved inner
            // scalar's null sentinel.  For an 8-byte inner (`integer`/`float`)
            // the sentinel's low-32 bits are 0 (a harmless empty handle), but a
            // 4-byte `single` NaN (0x7FC00000) is a non-zero garbage rec-id →
            // wild `get_u32_raw` → SIGSEGV when later read as a rec-id.  Zero the
            // handle on EVERY construction path — the literal/`Insert` path lacked
            // it (only the copy branch below had it), so nested `single` literals
            // crashed.  No-op for the already-zero 8-byte cases.
            if !self.first_pass && matches!(in_t, Type::Vector(_, _)) {
                ls.push(self.cl(
                    "OpSetInt4",
                    &[Value::Var(elm), Value::Int(0), Value::Int(0)],
                ));
            }
            if matches!(
                in_t,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            ) {
                let inner_nr = match in_t {
                    Type::Reference(nr, _) => *nr,
                    _ => self.data.type_def_nr(in_t),
                };
                if let Value::Insert(steps) = p {
                    // Inline struct initialization: the steps already write fields into elm.
                    for l in steps {
                        ls.push(l.clone());
                    }
                } else {
                    // Source is a variable, field access, or function call — the bytes
                    // must be explicitly copied into the new element slot.
                    //
                    // When the source is a STRUCT-RETURNING function call (or an
                    // inline struct-literal Object block — same shape as
                    // `copy_ref::is_struct_returning_call`), set the high
                    // bit (0x8000) on the type parameter so OpCopyRecord
                    // also frees the callee's temporary source store after
                    // the deep copy.  Without this, `vec += [fn_call()]`
                    // leaks one store per call (the `__ref_N` that the
                    // callee's `OpDatabase` allocated to build the return
                    // value).  Mirrors the existing fix in `copy_ref` for
                    // the plain-variable assignment path.  Suppressed
                    // under WASM where frame yield/resume creates store
                    // aliases that cannot yet be tracked.
                    #[cfg(not(feature = "wasm"))]
                    let free_source_bit: i32 =
                        if !self.first_pass && self.is_struct_returning_call(p) {
                            0x8000
                        } else {
                            0
                        };
                    #[cfg(feature = "wasm")]
                    let free_source_bit: i32 = 0;
                    let type_nr = if self.first_pass {
                        Value::Int(i32::from(u16::MAX))
                    } else if let Type::Vector(elem_tp, _) = in_t {
                        // For vector-typed elements, resolve via database.vector() to
                        // get the correct Parts::Vector type for deep copy.
                        let elem_known = self.database.db_type(elem_tp, &self.data);
                        Value::Int(i32::from(self.database.vector(elem_known)) | free_source_bit)
                    } else {
                        Value::Int(
                            i32::from(self.data.def(inner_nr).known_type()) | free_source_bit,
                        )
                    };
                    // @P380 handle-zero is now hoisted above (after the element
                    // is created), covering this copy path AND the literal/`Insert`
                    // path; `remove_claims` here sees the already-zeroed handle.
                    ls.push(self.cl("OpCopyRecord", &[p.clone(), Value::Var(elm), type_nr]));
                }
            } else if let Value::Tuple(values) = p {
                // P189c — vector-element tuple literal.  Emit
                // per-attribute writes against the synthetic
                // `__tuple<T1,T2,…>` struct that P189's `tuple_def`
                // registered.  Mirrors the struct-literal path
                // (`Value::Insert` arm below) but tuple literals
                // arrive as `Value::Tuple([v0, v1, …])` without
                // pre-emitted `SetField` steps — `parse_single` at
                // src/parser/vectors.rs:223 builds a bare wrapper —
                // so we emit them here.  `ed_nr` is already the
                // synthetic struct's d_nr (`type_def_nr(Type::Tuple)`),
                // and `set_field` with an explicit attribute index
                // routes through the standard per-field layout
                // dispatch (Integer/Text/Reference/etc.).
                for (i, val) in values.iter().enumerate() {
                    ls.push(self.set_field(ed_nr, i, 0, Value::Var(elm), val.clone()));
                }
            } else if let Value::Insert(steps) = p {
                for l in steps {
                    ls.push(l.clone());
                }
            } else if let Type::Integer(spec) = in_t
                && let Some(n) = spec.vector_narrow_width()
            {
                // narrow integer element write.
                // `set_field(ed_nr=INTEGER_DEF, f_nr=usize::MAX, …)`
                // dispatches through the wide `integer`'s `returned`
                // type and emits `OpSetInt` (8 bytes).  That works
                // for `Parts::Byte` / `Parts::Int` by coincidence
                // (their direct encoding matches the low bytes of a
                // wide little-endian write), and now works for
                // `Parts::ShortRaw` (which is also direct).  Emit
                // the narrow-width opcode directly so writes encode
                // correctly for all narrow widths — defensive against
                // future changes where the wide-write coincidence
                // might break.
                let pos = Value::Int(0);
                let op = match n {
                    1 => {
                        let m = Value::Int(spec.min);
                        self.cl("OpSetByte", &[Value::Var(elm), pos, m, p.clone()])
                    }
                    2 => {
                        let m = Value::Int(spec.min);
                        self.cl("OpSetShortRaw", &[Value::Var(elm), pos, m, p.clone()])
                    }
                    4 => self.cl("OpSetInt4", &[Value::Var(elm), pos, p.clone()]),
                    _ => self.set_field(ed_nr, usize::MAX, 0, Value::Var(elm), p.clone()),
                };
                ls.push(op);
            } else if matches!(in_t, Type::Function(_, _, _)) {
                // Plan-06 phase 4d.A.2 — fn-ref vector elements store
                // the 4-byte i32 d_nr.  Emit OpSetInt4 (4-byte write)
                // not OpSetInt (8-byte) so adjacent element slots
                // aren't corrupted by overflow.
                //
                // #263: the element is a 4-byte d_nr slot, but a fn-ref
                // *value* source (a local `f = dbl`, or a call-returned
                // `getfn()`) is a 20-byte slot ([d_nr 8B][closure DbRef
                // 12B]).  A bare `OpSetInt4(elm, 0, <fn-ref value>)`
                // reads the wrong 4 bytes (the high end of the slot —
                // part of the closure DbRef), storing a garbage d_nr
                // that crashes when later called/freed.  Project the
                // d_nr (the low 8 bytes, truncated to 4 by OpSetInt4):
                //   - a literal `Value::Int(d_nr)` from parse_fn_ref is
                //     already the bare d_nr — write it directly.
                //   - a `Value::Var(v)` (a fn-ref local) → FnRefDnr(v),
                //     which projects the d_nr via OpVarInt (mirrors the
                //     struct-field path at parser/mod.rs:6064).
                //   - a `Value::Call(..)` (a call-returned fn-ref) →
                //     materialise the call into a non-capturing fn-ref
                //     temp first (skip_free so its null closure half is
                //     never dereferenced), then FnRefDnr(tmp).
                // Capturing sources are already rejected above (the #247
                // guard), so discarding the closure half here is lossless.
                let pos = Value::Int(0);
                let dnr_val = match p.unspan() {
                    Value::Int(_) => p.clone(),
                    Value::Var(v) if matches!(self.vars.tp(*v), Type::Function(_, _, _)) => {
                        Value::FnRefDnr(*v)
                    }
                    Value::Call(_, _) => {
                        let fn_type = if let Type::Function(params, ret, _) = in_t {
                            Type::Function(params.clone(), ret.clone(), Deps::none())
                        } else {
                            in_t.clone()
                        };
                        let tmp = self.create_unique("__fn_ref_tmp", &fn_type);
                        self.vars.defined(tmp);
                        // The temp is a borrowed fn-ref value; its closure
                        // half is the null sentinel (non-capturing).  Mark
                        // skip_free so scope-exit cleanup never frees it.
                        self.vars.set_skip_free(tmp);
                        if !self.first_pass {
                            self.change_var_type(tmp, &fn_type);
                        }
                        ls.push(v_set(tmp, p.clone()));
                        Value::FnRefDnr(tmp)
                    }
                    _ => p.clone(),
                };
                ls.push(self.cl("OpSetInt4", &[Value::Var(elm), pos, dnr_val]));
            } else {
                ls.push(self.set_field(ed_nr, usize::MAX, 0, Value::Var(elm), p.clone()));
            }
            let finish = if let Some(target) = &vector_elem_target {
                // #246: finish the direct append into the inner vector.
                self.cl(
                    "OpFinishRecord",
                    &[target.clone(), Value::Var(elm), known, fld],
                )
            } else if is_field {
                let mut finish_v = self.new_record_field_op(val, parent_tp, "OpFinishRecord");
                // Replace placeholder Var(0) with the actual elm variable.
                if let Value::Call(_, ref mut args) = finish_v
                    && args.len() >= 2
                {
                    args[1] = Value::Var(elm);
                }
                finish_v
            } else {
                self.cl(
                    "OpFinishRecord",
                    &[Value::Var(vec), Value::Var(elm), known, fld],
                )
            };
            ls.push(finish);
        }
        ls
    }

    /// Return the database `known_type` of a `main_vector<T>` wrapper struct,
    /// registering it on the spot if it is still unassigned (`u16::MAX`).
    ///
    /// The wrapper is normally registered by `fill_all`'s sweep, which scans
    /// struct fields and (P191) function-local keyed collections — but NOT
    /// function-local plain vectors.  When the element type only resolves on
    /// pass 2 (a cross-package forward reference whose dependency is parsed
    /// after the importing package — #375), the `main_vector<T>` wrapper is
    /// born here during pass-2 codegen, AFTER this file's `fill_all` already
    /// ran, so it never receives a `known_type`.  Codegen would then bake an
    /// `OpDatabase(db_tp=u16::MAX)` operand and crash in `set_default_value` at
    /// runtime.  Mirror the keyed-collection codegen path (`database.hash` /
    /// `database.sorted` register on demand) by filling the wrapper here; the
    /// content type is fully resolved by pass 2.
    ///
    /// `fill_database` registers the wrapper struct and adds its `vector` field,
    /// but the field's POSITION is assigned by `finish_type` (run from
    /// `database.finish()`).  That finish normally runs at end of `parse_file` —
    /// AFTER the codegen that reads the position here — so without finishing now
    /// the `vector` field sits at `u16::MAX` and `OpGetField`/`get_field` read
    /// through a bogus offset, corrupting the interpreter free path (a heap
    /// write that SIGSEGVs at teardown after the correct value prints).  Finish
    /// immediately so the position is laid out before codegen consumes it.
    fn vector_wrapper_known_type(&mut self, vec_def: u32) -> u16 {
        let tp = self.data.def(vec_def).known_type();
        if tp != u16::MAX {
            return tp;
        }
        crate::typedef::fill_database(&mut self.data, &mut self.database, vec_def);
        self.database.finish();
        self.data.def(vec_def).known_type()
    }

    pub(crate) fn vector_db(&mut self, assign_tp: &Type, vec: u16) -> Vec<Value> {
        // @PLN87 P2.4 — a REBIND vector param (`v = [..]` whole-binding replace on
        // a visible vector param, marked via `ensure_rebind_witness`) DOES get a
        // fresh backing: it rebinds locally rather than appending to the caller's
        // store.  Every other argument keeps the caller-provided backing (no
        // local `__vdb` that would be freed before the return).
        let rebind = self.vars.rebind_orig(vec).is_some();
        if self.first_pass || vec == u16::MAX || (self.vars.is_argument(vec) && !rebind) {
            Vec::new()
        } else {
            let mut ls = Vec::new();
            let vec_def = self.data.vector_def(&mut self.lexer, assign_tp);
            let db = self
                .vars
                .work_vec_db(&Type::Reference(vec_def, Deps::none()), &mut self.lexer);
            // The rebind param's fresh backing is freed at function exit by the
            // param's own `OpFreeRefIfDistinct(v, witness)` (it frees v's store);
            // skip_free keeps scopes from ALSO freeing the `__vdb` (a double-free).
            if rebind {
                self.vars.set_skip_free(db);
            }
            self.vars.depend(vec, db);
            let tp = self.vector_wrapper_known_type(vec_def);
            debug_assert_ne!(
                tp,
                u16::MAX,
                "Undefined type {} at {}",
                self.data.def(vec_def).name(),
                self.lexer.pos()
            );
            ls.push(self.cl("OpDatabase", &[Value::Var(db), Value::Int(i32::from(tp))]));
            // Reference to the vector field.
            ls.push(v_set(vec, self.get_field(vec_def, 0, Value::Var(db))));
            // Write 0 into this reference.
            ls.push(self.set_field(vec_def, 0, 0, Value::Var(db), Value::Int(0)));
            ls
        }
    }

    pub(crate) fn insert_new(
        &mut self,
        vec: u16,
        elm: u16,
        in_t: &Type,
        ls: &mut Vec<Value>,
    ) -> u16 {
        // determine the element size by the resulting type
        let vec_def = self.data.vector_def(&mut self.lexer, in_t);
        // Use work_vec_db (separate __vdb_N counter) so that these calls do NOT
        // consume __ref_N counter slots.  Both vector_db and insert_new contribute
        // to the __vdb_N namespace; at any given vector site exactly one of them
        // runs per pass (vector_db is guarded by !first_pass; insert_new is called
        // on first pass when vector_db has not yet created a dep, but on second pass
        // vector_needs_db returns false after vector_db ran, so insert_new is
        // skipped).  The __ref_N counter is reserved exclusively for add_defaults
        // and other return-value work-refs, ensuring ref_return can match the same
        // name across both passes.
        let db = self
            .vars
            .work_vec_db(&Type::Reference(vec_def, Deps::none()), &mut self.lexer);
        self.vars.depend(elm, db);
        self.vars.depend(vec, db);
        let known = Value::Int(i32::from(self.vector_wrapper_known_type(vec_def)));
        ls.insert(0, self.cl("OpDatabase", &[Value::Var(db), known]));
        // Reference to the vector field.
        ls.insert(1, v_set(vec, self.get_field(vec_def, 0, Value::Var(db))));
        // Write 0 into this reference.
        ls.insert(
            2,
            self.set_field(vec_def, 0, 0, Value::Var(db), Value::Int(0)),
        );
        db
    }

    pub(crate) fn type_info(&mut self, in_t: &Type) -> Value {
        Value::Int(i32::from(self.get_type(in_t)))
    }

    pub(crate) fn get_type(&mut self, in_t: &Type) -> u16 {
        if self.first_pass {
            return u16::MAX;
        }
        match in_t {
            Type::Integer(spec) => {
                // honour `forced_size` via
                // `vector_narrow_width`.  Gate covers 1/2/4 bytes.
                // 2-byte uses `Parts::ShortRaw` (direct encoding) —
                // parallel to `Parts::Byte` / `Parts::Int` so that
                // source literal vectors and destination fields share
                // encoding and `vector_add`'s raw-byte copy stays
                // valid.  Narrow path registers on demand so locals /
                // params / returns don't depend on a struct field
                // having registered the name first.
                if let Some(n) = spec.vector_narrow_width() {
                    match n {
                        1 => self.database.byte(spec.min, false),
                        2 => self.database.short_raw(spec.min, false),
                        4 => self.database.int(spec.min, false),
                        _ => self.database.name("integer"),
                    }
                } else {
                    // Bounds heuristic fallback.
                    match in_t.size(false) {
                        1 if spec.min == 0 => self.database.name("byte"),
                        1 => self.database.name(&format!("byte<{},false>", spec.min)),
                        2 => self.database.name(&format!("short<{},false>", spec.min)),
                        _ => self.database.name("integer"),
                    }
                }
            }
            Type::Character => self.database.name("integer"),
            Type::Float => self.database.name("float"),
            Type::Single => self.database.name("single"),
            Type::Text(_) => self.database.name("text"),
            // Plan-06 phase 4d.A.2 — fn-ref in a vector stores as
            // 4-byte i32 d_nr.  Use the same DB type as `i32`
            // (signed 32-bit, registered via `database.int(0, false)`)
            // so vector storage uses the flat narrow-int path.  The
            // semantic difference (d_nr vs. integer) is recovered at
            // read-back time via fn-ref unbox.
            Type::Function(_, _, _) => self.database.int(0, false),
            Type::Reference(r, _) | Type::Enum(r, _, _) => self.data.def(*r).known_type(),
            Type::Hash(tp, key, _) => {
                let mut name = "hash<".to_string() + self.data.def(*tp).name() + "[";
                self.database
                    .field_name(self.data.def(*tp).known_type(), key, &mut name);
                let r = self.database.name(&name);
                if r != u16::MAX {
                    return r;
                }
                // P190 — local-var hash iteration: register on demand.
                let c_tp = self.data.def(*tp).known_type();
                if c_tp == u16::MAX {
                    return u16::MAX;
                }
                self.database.hash(c_tp, key)
            }
            Type::Sorted(tp, key, _) => {
                let mut name = "sorted<".to_string() + self.data.def(*tp).name() + "[";
                field_id(key, &mut name);
                let r = self.database.name(&name);
                if r != u16::MAX {
                    return r;
                }
                let mut ordered = "ordered<".to_string() + self.data.def(*tp).name() + "[";
                field_id(key, &mut ordered);
                let r = self.database.name(&ordered);
                if r != u16::MAX {
                    return r;
                }
                // P190 — local-var keyed collection iteration: the
                // sorted/ordered type wasn't pre-registered by
                // fill_database (which only runs on struct fields).
                // Register on demand here so OpIterate gets the right
                // db type id and `fill_iter` produces all 6 args.
                let c_tp = self.data.def(*tp).known_type();
                if c_tp == u16::MAX {
                    return u16::MAX;
                }
                self.database.sorted(c_tp, key)
            }
            Type::Index(tp, key, _) => {
                let mut name = "index<".to_string() + self.data.def(*tp).name() + "[";
                field_id(key, &mut name);
                let r = self.database.name(&name);
                if r != u16::MAX {
                    return r;
                }
                // P190 — same on-demand registration for local-var index.
                let c_tp = self.data.def(*tp).known_type();
                if c_tp == u16::MAX {
                    return u16::MAX;
                }
                self.database.index(c_tp, key)
            }
            Type::Vector(tp, _) => {
                // route through `vector_of` so narrow-alias
                // content (vector<i32>, vector<u8>) registers the same
                // narrow vector db_tp that `fill_database` registers for
                // struct fields.  Without this, locals / returns / file
                // writes fall back to `database.name(...)` which only
                // succeeds for pre-registered type names and returns
                // `u16::MAX` otherwise — triggering an index-out-of-bounds
                // in `assemble_write_data` when the resulting db_tp is
                // passed to the write path.
                self.vector_of(tp)
            }
            _ => u16::MAX,
        }
    }

    // <children> ::=
}

/// P216: walk a captured variable's `Type` and call `tuple_def` for
/// every `Type::Tuple` (including nested) so the synthetic
/// `__tuple<…>` struct exists by the time `fill_database` walks the
/// closure record's attributes.  Without this, a tuple-typed capture
/// surfaces `u32::MAX` from `type_elm` and the attribute is silently
/// skipped — closure record allocates with size 0 → `OpDatabase`
/// panics "Incomplete record" / native dispatches with field offsets
/// at `u16::MAX` → silent corruption.
fn ensure_tuple_defs_for_capture(
    data: &mut crate::data::Data,
    lexer: &mut crate::lexer::Lexer,
    tp: &Type,
) {
    match tp {
        Type::Tuple(elems) => {
            // Recurse first so nested tuples register inside-out.
            for inner in elems {
                ensure_tuple_defs_for_capture(data, lexer, inner);
            }
            data.tuple_def(lexer, elems);
        }
        Type::Vector(inner, _) | Type::RefVar(inner) => {
            ensure_tuple_defs_for_capture(data, lexer, inner);
        }
        _ => {}
    }
}

/// Plan-22 phase 02d-ii — canonical cell-struct name for a scalar
/// type.
///
/// Returns the conventional name `__cell_<T>` used to box a scalar
/// capture into a 1-field record so closure mutations propagate
/// back through the auto-Reference path (phase 02b/02c encoding).
///
/// Returns `None` for any type the cell-synthesis pass doesn't yet
/// support (exotic integer widths — u8/i8/u16/i16 — and any non-
/// scalar type).  Phase 02d-i may have queued such names in
/// `scalars_to_box` (the accumulator is intentionally inclusive);
/// 02d-iii will detect a missing cell at the rewrite site and
/// fall back to today's stack-slot codegen for those captures.
/// Phase 02d-iv extends the supported set as the need surfaces.
///
/// Naming table (one cell per canonical type, deduped across all
/// captures):
///
/// | Loft type | Cell name |
/// |---|---|
/// | `integer` (4-byte signed) | `__cell_integer` |
/// | `long` / wide integer (8-byte) | `__cell_long` |
/// | `float` | `__cell_float` |
/// | `single` | `__cell_single` |
/// | `boolean` | `__cell_boolean` |
/// | `character` | `__cell_character` |
/// | `text` | `__cell_text` |
/// | plain enum `E` | `__cell_enum_<E>` |
pub(crate) fn cell_struct_name(tp: &Type, data: &crate::data::Data) -> Option<String> {
    match tp {
        Type::Integer(spec) => {
            // Default-nullable byte_width: matches the storage the
            // cell's `value` field will take.  i32 → 8 today (the
            // bounds-range heuristic returns 8 for the I32
            // template; that's the canonical "integer" storage),
            // i64 → 8.  For the foundation phase we only emit two
            // canonical integer cells; exotic forced-size widths
            // (u8/i8/u16/i16) defer to 02d-iv.
            let bw = spec.byte_width(true);
            match (bw, spec.forced_size.is_some()) {
                (8, false) if spec.max == u32::MAX => Some("__cell_long".to_string()),
                (8, false) => Some("__cell_integer".to_string()),
                _ => None,
            }
        }
        Type::Float => Some("__cell_float".to_string()),
        Type::Single => Some("__cell_single".to_string()),
        Type::Boolean => Some("__cell_boolean".to_string()),
        Type::Character => Some("__cell_character".to_string()),
        Type::Text(_) => Some("__cell_text".to_string()),
        Type::Enum(d_nr, false, _) => {
            let enum_name = data.def(*d_nr).name();
            Some(format!("__cell_enum_{enum_name}"))
        }
        _ => None,
    }
}

/// Plan-22 phase 02d-ii — canonical type for the cell's `value`
/// field.
///
/// Strips bound/dep details so multiple captures of "the same
/// underlying type" with slightly different annotations
/// (e.g. `text` with different lifetime deps, or
/// `integer not null` vs `integer`) share a single cell struct.
fn cell_value_type(tp: &Type) -> Type {
    match tp {
        Type::Integer(spec) => {
            // Canonical wide vs narrow templates; bounds + null-flag
            // are dropped to match the cell-name canonicalisation.
            if spec.max == u32::MAX {
                crate::data::I64.clone()
            } else {
                crate::data::I32.clone()
            }
        }
        Type::Text(_) => Type::Text(Deps::none()),
        Type::Enum(d_nr, _, _) => Type::Enum(*d_nr, false, Deps::none()),
        other => other.clone(),
    }
}

/// Plan-22 phase 02d-i — accumulate the names of scalar-typed
/// captures that this lambda mutates onto the PARENT function's
/// `scalars_to_box` field.
///
/// Per-call: one lambda's mutations contribute to the parent
/// function's union.  Names are deduped; types decide
/// scalar-vs-non-scalar.
///
/// "Scalar" set (per phase 02d design):
///   - `Type::Integer(_)`
///   - `Type::Float`
///   - `Type::Single`
///   - `Type::Boolean`
///   - `Type::Character`
///   - `Type::Text(_)`
///   - `Type::Enum(_, false, _)` (plain enum)
///
/// Reference / Function / Vector / Hash / Sorted / Index /
/// Spacial / Tuple captures are NOT scalars — they're handled by
/// other paths (Reference: phase 02c; Function: phase 02c via
/// existing fn-ref machinery; Vector + keyed: rejected by P257).
///
/// `parent_d_nr` is the enclosing function's def_nr; if the
/// lambda is at top-level (no enclosing function — e.g. a lambda
/// in a struct field default value), `parent_d_nr == u32::MAX`
/// and the accumulator is a no-op (top-level binds aren't
/// mutated-captured by their own scope).
/// Plan-22 phase 02d-iii.e — replace `captured_names` entries
/// for names in the parent function's `scalars_to_box` with
/// their boxed `Reference(__cell_<T>, [])` form.
///
/// Called from the lambda parsing site BETWEEN
/// `synthesize_cell_structs` (which needs the original scalar
/// type to compute the cell name) and `synthesize_closure_record`
/// (which uses `captured_names` types directly to set the
/// closure record's attribute types).  In pass 1, this is what
/// makes the closure record's attribute carry
/// `Reference(__cell_int, _)` from creation, so phase 02c's
/// auto-Reference encoding fires (12B share-by-DbRef storage)
/// and the closure body's reads/writes route through the
/// shared cell.
///
/// In pass 2, `captured_names` is typically empty (the lambda's
/// resolve_name takes the closure-redirect arm before the
/// capture arm fires), so this helper is a no-op there.
fn box_captured_names_for_outer_scalars(
    captured_names: &mut [(String, Type)],
    data: &crate::data::Data,
    outer_context: u32,
) {
    if outer_context == u32::MAX || (outer_context as usize) >= data.definitions.len() {
        return;
    }
    let scalars = data.def(outer_context).scalars_to_box().to_vec();
    // Plan-22 phase 02d-vii — symmetric guard with
    // `flip_scalars_to_box_types`: skip text when the parent
    // function returns text (avoid the "Write to locked
    // store" panic at closure-record init in text-returning
    // fns).
    let parent_returns_text = matches!(data.def(outer_context).returned(), Type::Text(_));
    for (name, tp) in captured_names {
        if !scalars.iter().any(|s| s == name) {
            continue;
        }
        let is_text_or_reftext = matches!(tp, Type::Text(_))
            || matches!(tp, Type::RefVar(inner)
                if matches!(inner.as_ref(), Type::Text(_)));
        if parent_returns_text && is_text_or_reftext {
            continue;
        }
        if let Some(cell_name) = cell_struct_name(tp, data) {
            let cell_d_nr = data.def_nr(&cell_name);
            if cell_d_nr != u32::MAX {
                *tp = Type::Reference(cell_d_nr, Deps::none());
            }
        }
    }
}

fn accumulate_scalars_to_box(
    data: &mut crate::data::Data,
    parent_d_nr: u32,
    lambda_d_nr: u32,
    captured_names: &[(String, Type)],
) {
    if parent_d_nr == u32::MAX || (parent_d_nr as usize) >= data.definitions.len() {
        return;
    }
    let mutated = data.def(lambda_d_nr).mutated_captures().to_vec();
    for name in &mutated {
        let Some((_, tp)) = captured_names.iter().find(|(n, _)| n == name) else {
            continue;
        };
        let is_scalar = matches!(
            tp,
            Type::Integer(_)
                | Type::Float
                | Type::Single
                | Type::Boolean
                | Type::Character
                | Type::Text(_)
                | Type::Enum(_, false, _)
        );
        if !is_scalar {
            continue;
        }
        let parent = &mut data.definitions[parent_d_nr as usize];
        if !parent.scalars_to_box.iter().any(|s| s == name) {
            parent.scalars_to_box.push(name.clone());
        }
    }
}

/// Plan-22 phase 01 — walk the lambda body's IR and identify
/// captured bindings whose value is mutated.
///
/// Detection rules:
///   - `Value::Set(slot, _)` where slot's variable name appears
///     in the closure record → whole-binding reassignment.
///   - `Value::Call(d, args)` where `d`'s op name is in the
///     mutating-op set (OpSet* / OpAppend* / OpClear* /
///     OpInsertVector / OpRemoveVector) AND `args[0]` is a
///     `Value::Var(slot)` whose name appears in the closure
///     record → field write through the captured value.
///
/// The closure record's attribute names are the canonical list
/// of captured bindings (populated by `synthesize_closure_record`
/// from `captured_names`).  A name in `mutated_captures` means
/// some write opcode targets that name's underlying slot.
///
/// Stores result on `data.def(lambda_d_nr).mutated_captures`.
/// Phases 02-05 consume this for case classification.
///
/// Detection-only: no IR rewrite, no codegen change, no behavior
/// difference between this commit and the prior state.  The
/// stored result is consulted by later phases.
fn collect_mutated_captures(data: &mut crate::data::Data, lambda_d_nr: u32) {
    // Plan-22 phase 02c (2026-05-12): the captured-name list comes
    // from EITHER the closure record's attributes (when synthesize
    // ran first — the original phase 01 path) OR from the lambda's
    // variables that match a synthesized `__closure_<N>` struct
    // name pattern (not currently used).  The phase-02c flow runs
    // collect_mutated_captures BEFORE synthesize, so we accept the
    // closure_record == MAX case as "not yet built; no names from
    // there" and fall through to using the variables table.
    //
    // Phase 01's existing API stays compatible: when synthesize has
    // already run, this function uses the closure record's
    // attributes (the original behaviour).
    let closure_d_nr = data.def(lambda_d_nr).closure_record();
    let variables = data.def(lambda_d_nr).variables().clone();
    let captured_names: Vec<String> = if closure_d_nr == u32::MAX {
        // Pre-synthesize call site — derive captured names from the
        // lambda's variable table.  Captured names are local
        // placeholder vars created in objects.rs:187 / control.rs:3614
        // during body parsing.  Filter out `__closure` (the closure
        // param), `__work*` (text work buffers), and any argument.
        // The body walker then double-filters via the captured_names
        // membership check in `mark()`, so over-inclusion here is
        // safe (just spurious work).
        (0..variables.count())
            .filter_map(|v| {
                let name = variables.name(v);
                if name == "__closure" || name.starts_with("__work") || variables.is_argument(v) {
                    None
                } else {
                    Some(name.to_string())
                }
            })
            .collect()
    } else {
        data.def(closure_d_nr)
            .attributes
            .iter()
            .map(|a| a.name.clone())
            .collect()
    };
    if captured_names.is_empty() {
        return;
    }
    let body = data.def(lambda_d_nr).code().clone();
    // Resolve the closure-param's var slot — it's the `__closure`
    // argument added at lambda parse time (vectors.rs:417-421).
    // In the body's IR, captured-name reads are
    // `Call(OpGet*, [Var(closure_param), Int(fld_idx)])` and
    // writes are `Call(OpSet*, [Var(closure_param), Int(fld_idx),
    // value])`.  Field indices map to attribute positions in the
    // closure record (`captured_names[fld_idx]`).
    let mut closure_param: Option<u16> = None;
    for v in 0..variables.count() {
        if variables.name(v) == "__closure" {
            closure_param = Some(v);
            break;
        }
    }
    let mut mutated: Vec<String> = Vec::new();
    walk_for_mutations(
        &body,
        &captured_names,
        closure_param,
        &variables,
        data,
        &mut mutated,
    );
    data.definitions[lambda_d_nr as usize].mutated_captures = mutated;
}

/// Op names whose first argument is the target of a mutation.
/// All write through the closure record's field (when first arg
/// is a captured-binding placeholder var).
///
/// `Definition::original_name()` strips the leading 2 chars
/// (the `n_` / `Op` prefix) for functions — these names are
/// the post-strip form, e.g. `OpSetInt` → `SetInt`,
/// `n_my_fn` → `my_fn`.
const MUTATING_OP_NAMES: &[&str] = &[
    "SetInt",
    "SetByte",
    "SetShortRaw",
    "SetInt4",
    "SetFloat",
    "SetSingle",
    "SetText",
    "SetCharacter",
    "SetEnum",
    "AppendVector",
    "AppendText",
    "AppendCharacter",
    "AppendStackText",
    "AppendStackCharacter",
    "ClearVector",
    "ClearText",
    "ClearStackText",
    "InsertVector",
    "RemoveVector",
];

fn walk_for_mutations(
    code: &Value,
    captured_names: &[String],
    closure_param: Option<u16>,
    variables: &crate::variables::Function,
    data: &crate::data::Data,
    out: &mut Vec<String>,
) {
    let mark = |name: &str, out: &mut Vec<String>| {
        if captured_names.iter().any(|c| c == name) && !out.iter().any(|s| s == name) {
            out.push(name.to_string());
        }
    };
    match code {
        Value::Span(boxed) => walk_for_mutations(
            &boxed.1,
            captured_names,
            closure_param,
            variables,
            data,
            out,
        ),
        Value::Set(slot, expr) => {
            if *slot < variables.count() {
                mark(variables.name(*slot), out);
            }
            walk_for_mutations(expr, captured_names, closure_param, variables, data, out);
        }
        Value::Call(d, args) => {
            if (*d as usize) < data.definitions.len() {
                let op_name = data.def(*d).original_name();
                if MUTATING_OP_NAMES.iter().any(|n| *n == op_name)
                    && let Some(first) = args.first()
                {
                    // Three write shapes:
                    //   (a) `OpSet*(Var(captured_local), …)` — pass-1
                    //       form, before closure-param threading.
                    //       Direct local-slot write to the captured
                    //       binding's placeholder var.
                    //   (b) `OpSet*(Var(closure_param), Int(fld_idx),
                    //       …)` — pass-2 direct write to a captured
                    //       primitive's slot in the closure record.
                    //       Map fld_idx → captured_names[fld_idx].
                    //   (c) `OpSet*(Call(GetField, [Var(closure_param),
                    //       Int(fld_idx)]), …)` — pass-2 nested write
                    //       through a captured Reference's loaded
                    //       value.  E.g. `s.x = 7` for captured
                    //       struct `s`.  The mutation targets `s`
                    //       (via its content), so flag s as mutated.
                    match first.unspan() {
                        Value::Var(v) => {
                            if Some(*v) == closure_param
                                && let Some(Value::Int(fld)) = args.get(1).map(Value::unspan)
                                && (*fld as usize) < captured_names.len()
                            {
                                mark(&captured_names[*fld as usize].clone(), out);
                            } else if *v < variables.count() {
                                mark(variables.name(*v), out);
                            }
                        }
                        Value::Call(inner_d, inner_args)
                            if (*inner_d as usize) < data.definitions.len() =>
                        {
                            // Detect nested `Call(GetField, [Var(closure_param), Int(fld)])`
                            let inner_name = data.def(*inner_d).original_name();
                            if (inner_name == "GetField" || inner_name.starts_with("Get"))
                                && let Some(inner_first) = inner_args.first()
                                && let Value::Var(v) = inner_first.unspan()
                                && Some(*v) == closure_param
                                && let Some(Value::Int(fld)) = inner_args.get(1).map(Value::unspan)
                                && (*fld as usize) < captured_names.len()
                            {
                                mark(&captured_names[*fld as usize].clone(), out);
                            }
                        }
                        _ => {}
                    }
                }
            }
            for arg in args {
                walk_for_mutations(arg, captured_names, closure_param, variables, data, out);
            }
        }
        Value::CallRef(_, args) | Value::Insert(args) | Value::Tuple(args) => {
            for arg in args {
                walk_for_mutations(arg, captured_names, closure_param, variables, data, out);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for child in &b.operators {
                walk_for_mutations(child, captured_names, closure_param, variables, data, out);
            }
        }
        Value::If(c, t, f) => {
            walk_for_mutations(c, captured_names, closure_param, variables, data, out);
            walk_for_mutations(t, captured_names, closure_param, variables, data, out);
            walk_for_mutations(f, captured_names, closure_param, variables, data, out);
        }
        Value::Iter(_, init, step, body) => {
            walk_for_mutations(init, captured_names, closure_param, variables, data, out);
            walk_for_mutations(step, captured_names, closure_param, variables, data, out);
            walk_for_mutations(body, captured_names, closure_param, variables, data, out);
        }
        Value::Return(expr)
        | Value::Drop(expr)
        | Value::Yield(expr)
        | Value::TuplePut(_, _, expr)
        | Value::BreakWith(_, expr) => {
            walk_for_mutations(expr, captured_names, closure_param, variables, data, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod plan22_phase01_mutation_detection_tests {
    //! Plan-22 phase 01 — verify `collect_mutated_captures` populates
    //! `data.def(d_nr).mutated_captures` correctly across the
    //! representative shapes phase 02 will consume.

    use crate::parser::Parser;

    /// Helper: parse a snippet and find the first capturing lambda
    /// definition.  Returns its `mutated_captures` clone.
    fn first_capturing_lambda_mutations(source: &str) -> Vec<String> {
        let mut p = Parser::new();
        // Load defaults so primitive types and operators resolve.
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase01_test", false);
        for d_nr in 0..p.data.definitions() {
            let def = p.data.def(d_nr);
            if def.closure_record() != u32::MAX && !def.mutated_captures().is_empty() {
                return def.mutated_captures().to_vec();
            }
        }
        // Fall back to the first capturing lambda even when nothing
        // was detected — lets `assert_eq` show the actual empty vec.
        for d_nr in 0..p.data.definitions() {
            let def = p.data.def(d_nr);
            if def.closure_record() != u32::MAX {
                return def.mutated_captures().to_vec();
            }
        }
        Vec::new()
    }

    #[test]
    fn read_only_capture_yields_empty_set() {
        // Case A baseline — capture `n`, read it, never write.
        // mutated_captures should stay empty (the regression-net
        // signal that phases 02-05 must not flip).
        let mutated = first_capturing_lambda_mutations(
            r"
            fn test() {
                n = 5;
                f = fn(x: integer) -> integer { x + n };
                _ = f(10);
            }
            ",
        );
        assert!(
            mutated.is_empty(),
            "expected no mutations for read-only capture; got {mutated:?}"
        );
    }

    #[test]
    fn whole_binding_reassign_detected() {
        // Case B (basic) — `n = n + 1` rewrites the captured slot.
        // The Set arm of the walker catches this.
        let mutated = first_capturing_lambda_mutations(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
        );
        assert!(
            mutated.iter().any(|s| s == "n"),
            "expected `n` mutation detected; got {mutated:?}"
        );
    }

    #[test]
    fn struct_field_write_detected() {
        // Case B (Reference) — `s.x = ...` desugars to OpSetInt
        // on the captured Reference.  The MUTATING_OP_NAMES arm
        // catches it via `args[0] = Var(captured)`.
        let mutated = first_capturing_lambda_mutations(
            r"
            struct S { x: integer, y: integer }
            fn test() {
                s = S { x: 0, y: 0 };
                f = fn() { s.x = 7; };
                f();
            }
            ",
        );
        assert!(
            mutated.iter().any(|s| s == "s"),
            "expected `s` mutation detected; got {mutated:?}"
        );
    }

    #[test]
    fn text_append_detected() {
        // Case B (text capture) — `s += "x"` desugars to
        // OpAppendText on the captured text slot.
        let mutated = first_capturing_lambda_mutations(
            r#"
            fn test() {
                s = "";
                f = fn() { s += "x"; };
                f();
            }
            "#,
        );
        assert!(
            mutated.iter().any(|s| s == "s"),
            "expected `s` mutation detected; got {mutated:?}"
        );
    }

    #[test]
    fn multiple_captures_only_mutated_one_flagged() {
        // Read `r`, write `w`.  Only `w` should appear in mutated.
        let mutated = first_capturing_lambda_mutations(
            r"
            fn test() {
                r = 100;
                w = 0;
                f = fn() { w = w + r; };
                f();
            }
            ",
        );
        assert!(
            mutated.iter().any(|s| s == "w"),
            "expected `w` mutation detected; got {mutated:?}"
        );
        assert!(
            !mutated.iter().any(|s| s == "r"),
            "did not expect `r` (read-only) in mutated set; got {mutated:?}"
        );
    }
}

#[cfg(test)]
mod plan22_phase02d_i_scalars_to_box_tests {
    //! Plan-22 phase 02d-i — verify the parent-function
    //! `scalars_to_box` field is populated correctly across the
    //! representative shapes phase 02d-iii will consume.  No
    //! behavior change at this phase: the field is detection-only.

    use crate::parser::Parser;

    /// Helper: parse a snippet with a `fn test() { … }` and return
    /// `test`'s `scalars_to_box` field (sorted for stable assertion).
    fn test_fn_scalars_to_box(source: &str) -> Vec<String> {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase02d_i_test", false);
        let test_d_nr = p.data.def_nr("n_test");
        if test_d_nr == u32::MAX {
            return Vec::new();
        }
        let mut names = p.data.def(test_d_nr).scalars_to_box().to_vec();
        names.sort();
        names
    }

    #[test]
    fn read_only_capture_yields_empty_box_set() {
        // Case A — capture `n`, read it, never write.
        // No name should be queued for boxing.
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                n = 5;
                f = fn(x: integer) -> integer { x + n };
                _ = f(10);
            }
            ",
        );
        assert!(
            names.is_empty(),
            "expected empty scalars_to_box for read-only capture; got {names:?}"
        );
    }

    #[test]
    fn integer_mutated_capture_pushed() {
        // The canonical 02d-iii target: `n = 0; f = fn() { n = n + 1; }`.
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
        );
        assert_eq!(
            names,
            vec!["n".to_string()],
            "expected `n` queued for boxing; got {names:?}"
        );
    }

    #[test]
    fn text_mutated_capture_pushed() {
        // `s` is text — boxable per the 02d design (Text is in
        // the scalar set even though it has internal heap storage).
        let names = test_fn_scalars_to_box(
            r#"
            fn test() {
                s = "";
                f = fn() { s += "x"; };
                f();
            }
            "#,
        );
        assert_eq!(
            names,
            vec!["s".to_string()],
            "expected `s` queued for boxing; got {names:?}"
        );
    }

    #[test]
    fn struct_capture_not_pushed() {
        // `s` is a struct (Reference type) — handled by phase 02c
        // (auto-Reference), NOT by 02d-iii's boxing.  The
        // scalars_to_box field must EXCLUDE Reference captures.
        let names = test_fn_scalars_to_box(
            r"
            struct S { x: integer }
            fn test() {
                s = S { x: 0 };
                f = fn() { s.x = 7; };
                f();
            }
            ",
        );
        assert!(
            names.is_empty(),
            "Reference captures must NOT be queued for boxing; got {names:?}"
        );
    }

    #[test]
    fn multiple_captures_only_mutated_scalars_pushed() {
        // Read `r`, write `w`.  Only `w` (the mutated scalar) is
        // queued.  `r` (read-only) is excluded by phase 01's
        // mutated_captures filter.
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                r = 100;
                w = 0;
                f = fn() { w = w + r; };
                f();
            }
            ",
        );
        assert_eq!(
            names,
            vec!["w".to_string()],
            "expected `w` queued (not `r`); got {names:?}"
        );
    }

    #[test]
    fn multi_scalar_capture_all_pushed() {
        // Two scalars, both mutated.  Both should be queued.
        // (Sorted by the helper for stable assertion.)
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                a = 0;
                b = 0;
                f = fn() {
                    a = a + 1;
                    b = b + 2;
                };
                f();
            }
            ",
        );
        assert_eq!(
            names,
            vec!["a".to_string(), "b".to_string()],
            "expected both `a` and `b` queued; got {names:?}"
        );
    }

    #[test]
    fn dedup_when_two_lambdas_mutate_same_capture() {
        // Two separate lambdas in `test` both mutate `n`.  The
        // accumulator must dedup — `n` appears once, not twice.
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                n = 0;
                f1 = fn() { n = n + 1; };
                f2 = fn() { n = n + 2; };
                f1();
                f2();
            }
            ",
        );
        assert_eq!(
            names,
            vec!["n".to_string()],
            "expected `n` queued exactly once (deduped); got {names:?}"
        );
    }
}

#[cfg(test)]
mod plan22_phase02d_ii_cell_struct_synthesis_tests {
    //! Plan-22 phase 02d-ii — verify `synthesize_cell_structs`
    //! creates the canonical `__cell_<T>` structs in `Data` after
    //! parsing snippets that mutate scalar captures.  The structs
    //! are foundation infrastructure for 02d-iii's outer-binding
    //! rewrite; at this phase they exist but are never allocated
    //! or referenced (no behavior change).

    use crate::data::DefType;
    use crate::parser::Parser;

    /// Helper: parse a snippet and return the `(name, def_type)`
    /// of every `__cell_*` definition that exists in `Data`.
    fn cells_after(source: &str) -> Vec<(String, DefType)> {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase02d_ii_test", false);
        let mut cells = Vec::new();
        for d_nr in 0..p.data.definitions() {
            let def = p.data.def(d_nr);
            if def.name().starts_with("__cell_") {
                cells.push((def.name().to_string(), def.def_type().clone()));
            }
        }
        cells.sort_by(|a, b| a.0.cmp(&b.0));
        cells
    }

    /// Helper: parse a snippet and return `value` field's type
    /// signature for the named cell struct.  Panics if the cell or
    /// its `value` attribute is missing — those failures should
    /// surface as test failures, not silent `None`s.
    fn cell_value_signature(source: &str, cell_name: &str) -> String {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase02d_ii_test", false);
        let d_nr = p.data.def_nr(cell_name);
        assert_ne!(d_nr, u32::MAX, "cell `{cell_name}` not found");
        let def = p.data.def(d_nr);
        let attr = def
            .attributes
            .iter()
            .find(|a| a.name == "value")
            .unwrap_or_else(|| panic!("`value` attribute missing on `{cell_name}`"));
        format!("{:?}", attr.typedef)
    }

    #[test]
    fn integer_capture_creates_cell_integer() {
        let cells = cells_after(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
        );
        assert!(
            cells
                .iter()
                .any(|(n, t)| n == "__cell_integer" && *t == DefType::Struct),
            "expected `__cell_integer` Struct in Data; got {cells:?}"
        );
    }

    #[test]
    fn text_capture_creates_cell_text() {
        let cells = cells_after(
            r#"
            fn test() {
                s = "";
                f = fn() { s += "x"; };
                f();
            }
            "#,
        );
        assert!(
            cells
                .iter()
                .any(|(n, t)| n == "__cell_text" && *t == DefType::Struct),
            "expected `__cell_text` Struct in Data; got {cells:?}"
        );
    }

    #[test]
    fn read_only_capture_creates_no_cell() {
        let cells = cells_after(
            r"
            fn test() {
                n = 5;
                f = fn(x: integer) -> integer { x + n };
                _ = f(10);
            }
            ",
        );
        assert!(
            cells.is_empty(),
            "no cell should be synthesised for read-only capture; got {cells:?}"
        );
    }

    #[test]
    fn struct_capture_creates_no_cell() {
        // Struct (Reference) captures are handled by phase 02c's
        // auto-Reference path, NOT by 02d's boxing.  No `__cell_*`
        // should appear.
        let cells = cells_after(
            r"
            struct S { x: integer }
            fn test() {
                s = S { x: 0 };
                f = fn() { s.x = 7; };
                f();
            }
            ",
        );
        assert!(
            cells.is_empty(),
            "no cell should be synthesised for struct capture; got {cells:?}"
        );
    }

    #[test]
    fn dedup_two_integer_captures_share_one_cell() {
        // Two separate scalar bindings, both integer, both
        // mutated.  Exactly one `__cell_integer` should appear.
        let cells = cells_after(
            r"
            fn test() {
                a = 0;
                b = 0;
                f = fn() {
                    a = a + 1;
                    b = b + 2;
                };
                f();
            }
            ",
        );
        let count = cells.iter().filter(|(n, _)| n == "__cell_integer").count();
        assert_eq!(
            count, 1,
            "expected exactly one `__cell_integer`; got {count} (cells={cells:?})"
        );
    }

    #[test]
    fn distinct_types_create_distinct_cells() {
        // Mix of scalar types — each should get its own cell.
        let cells = cells_after(
            r#"
            fn test() {
                n = 0;
                s = "";
                b = false;
                f = fn() {
                    n = n + 1;
                    s += "x";
                    b = !b;
                };
                f();
            }
            "#,
        );
        let names: Vec<String> = cells.iter().map(|(n, _)| n.clone()).collect();
        assert!(
            names.contains(&"__cell_integer".to_string()),
            "expected __cell_integer; got {names:?}"
        );
        assert!(
            names.contains(&"__cell_text".to_string()),
            "expected __cell_text; got {names:?}"
        );
        assert!(
            names.contains(&"__cell_boolean".to_string()),
            "expected __cell_boolean; got {names:?}"
        );
    }

    #[test]
    fn cell_value_field_carries_canonical_type() {
        // Sanity check: the cell's `value` field exists and the
        // type signature renders as `Integer(...)` (the canonical
        // I32 template).  Phase 02d-iii will read/write through
        // this field.
        let sig = cell_value_signature(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
            "__cell_integer",
        );
        assert!(
            sig.starts_with("Integer("),
            "cell `value` field should be Integer-typed; got `{sig}`"
        );
    }

    #[test]
    fn dedup_across_two_lambdas() {
        // Two lambdas, each mutating its own integer capture.
        // Both reuse the single `__cell_integer` struct (the cell
        // is keyed by type, not by binding).
        let cells = cells_after(
            r"
            fn test() {
                a = 0;
                b = 0;
                f1 = fn() { a = a + 1; };
                f2 = fn() { b = b + 1; };
                f1();
                f2();
            }
            ",
        );
        let count = cells.iter().filter(|(n, _)| n == "__cell_integer").count();
        assert_eq!(
            count, 1,
            "expected single shared `__cell_integer` across two lambdas; got {count} (cells={cells:?})"
        );
    }
}

#[cfg(test)]
mod plan22_phase02d_iii_a_type_flip_tests {
    //! Plan-22 phase 02d-iii.a — verify
    //! `flip_scalars_to_box_types` replaces each boxed scalar
    //! local's type in the variables table with its
    //! `Reference(__cell_<T>, [])` form when invoked.
    //!
    //! Foundation step: the helper is shipped but
    //! INTENTIONALLY NOT WIRED into the parse_function pass-2
    //! entry yet.  The existing void-return-closure write-back
    //! mechanism in `parse_call_ref` (control.rs lines
    //! 3729-3755) handles today's `p86_lambda_capture_*`
    //! cases by copying the closure-record's scalar attribute
    //! back to the outer slot after each call.  Activating the
    //! flip in this commit would break that path
    //! (the outer slot's shape changes from 8B Integer to 12B
    //! DbRef, segfaulting the write-back's `v_set`).
    //!
    //! 02d-iii.e activates the flip from `parse_function`
    //! AFTER 02d-iii.b-d have wired cell-based propagation
    //! (auto-Reference closure-record attribute + shared-DbRef
    //! reads/writes), at which point the write-back path can
    //! be removed without a regression.
    //!
    //! These tests invoke the helper EXPLICITLY on a parsed
    //! parser state — they verify the helper's logic in
    //! isolation, independent of the integration path.

    use crate::data::Type;
    use crate::parser::Parser;

    /// Helper: parse a snippet, restore pass-2 entry state for
    /// `n_test` (set context, restore vars), invoke the flip
    /// helper, and return the type of `var_name` after the
    /// flip.  Panics if the function or variable is missing.
    fn flipped_type_of_var(source: &str, var_name: &str) -> Type {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase02d_iii_a_test", false);
        let test_d_nr = p.data.def_nr("n_test");
        assert_ne!(test_d_nr, u32::MAX, "function `n_test` not found");
        // Restore parse_function pass-2 entry state for the
        // test fn: set context + drain the def's saved vars
        // back into self.vars (the inverse of the save at the
        // end of parse_function).
        p.context = test_d_nr;
        // Reset p.vars to a fresh Function and drain the saved
        // pass-2 variables back into it (mirrors parse_function's
        // line-547 `Function::new` + line-853 `vars.append`).
        p.vars = crate::variables::Function::new("phase02d_iii_a_test", "test.loft");
        p.vars
            .append(&mut p.data.definitions[test_d_nr as usize].variables);
        // Invoke the flip helper under test.
        p.flip_scalars_to_box_types();
        let v_nr = p.vars.var(var_name);
        assert_ne!(v_nr, u16::MAX, "variable `{var_name}` not found");
        p.vars.tp(v_nr).clone()
    }

    #[test]
    fn integer_mutated_capture_flipped_to_reference_cell_integer() {
        // The canonical 02d-iii target.  After explicit flip,
        // `n`'s type becomes `Reference(__cell_integer, [])`.
        let tp = flipped_type_of_var(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
            "n",
        );
        match tp {
            Type::Reference(_, ref deps) => {
                assert!(
                    deps.is_empty(),
                    "boxed-scalar Reference should be heap-owned (empty deps); got {deps:?}"
                );
            }
            other => panic!("expected Reference(__cell_integer, []); got {other:?}"),
        }
    }

    #[test]
    fn read_only_capture_keeps_integer_type() {
        // Case A — capture `n` for reading only.  Type stays as
        // Integer; `flip_scalars_to_box_types` must NOT touch it.
        let tp = flipped_type_of_var(
            r"
            fn test() {
                n = 5;
                f = fn(x: integer) -> integer { x + n };
                _ = f(10);
            }
            ",
            "n",
        );
        assert!(
            matches!(tp, Type::Integer(_)),
            "read-only capture must NOT be flipped; got {tp:?}"
        );
    }

    #[test]
    fn struct_capture_keeps_struct_reference_type() {
        // Case B (Reference) — `s` is a struct; phase 02c handles
        // the auto-Reference encoding, NOT 02d-iii's boxing.
        let tp = flipped_type_of_var(
            r"
            struct S { x: integer }
            fn test() {
                s = S { x: 0 };
                f = fn() { s.x = 7; };
                f();
            }
            ",
            "s",
        );
        match tp {
            Type::Reference(_, _) => {
                // d_nr should NOT be a __cell_* struct; it's S.
                // (The flip helper doesn't touch struct
                // captures because `cell_struct_name` returns
                // `None` for `Type::Reference`.)
            }
            other => panic!("expected Reference(S, _); got {other:?}"),
        }
    }

    #[test]
    fn no_mutation_no_flip() {
        // Local `n` exists but is never captured by any closure —
        // scalars_to_box is empty for `n_test`, so the helper
        // is a no-op and `n` stays as Integer.
        let tp = flipped_type_of_var(
            r"
            fn test() {
                n = 5;
                _ = n + 1;
            }
            ",
            "n",
        );
        assert!(
            matches!(tp, Type::Integer(_)),
            "uncaptured local must NOT be flipped; got {tp:?}"
        );
    }

    #[test]
    fn multi_scalar_capture_flips_each() {
        // Two distinct scalar captures, both mutated.  Both
        // should be flipped to their respective cell References
        // when the helper is invoked.
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(
            r#"
            fn test() {
                n = 0;
                s = "";
                f = fn() {
                    n = n + 1;
                    s += "x";
                };
                f();
            }
            "#,
            "phase02d_iii_a_test",
            false,
        );
        let test_d_nr = p.data.def_nr("n_test");
        let cell_int = p.data.def_nr("__cell_integer");
        let cell_text = p.data.def_nr("__cell_text");
        assert_ne!(cell_int, u32::MAX, "__cell_integer missing");
        assert_ne!(cell_text, u32::MAX, "__cell_text missing");
        // Restore pass-2 entry state and invoke the flip helper.
        p.context = test_d_nr;
        // Reset p.vars to a fresh Function and drain the saved
        // pass-2 variables back into it (mirrors parse_function's
        // line-547 `Function::new` + line-853 `vars.append`).
        p.vars = crate::variables::Function::new("phase02d_iii_a_test", "test.loft");
        p.vars
            .append(&mut p.data.definitions[test_d_nr as usize].variables);
        p.flip_scalars_to_box_types();
        let n_nr = p.vars.var("n");
        let s_nr = p.vars.var("s");
        assert_ne!(n_nr, u16::MAX, "`n` not found in vars");
        assert_ne!(s_nr, u16::MAX, "`s` not found in vars");
        match p.vars.tp(n_nr) {
            Type::Reference(d, deps) => {
                assert_eq!(*d, cell_int, "`n` should point at __cell_integer");
                assert!(deps.is_empty());
            }
            other => panic!("`n` not flipped: {other:?}"),
        }
        // Plan-22 phase 02d-vi — text is now flipped via the
        // bypass guard added in `parse_assign_op` for boxed-text
        // LHS shape.  `s` should be Reference(__cell_text, []).
        match p.vars.tp(s_nr) {
            Type::Reference(d, deps) => {
                assert_eq!(*d, cell_text, "`s` should point at __cell_text");
                assert!(deps.is_empty());
            }
            other => panic!("`s` not flipped to __cell_text: {other:?}"),
        }
    }
}

#[cfg(test)]
mod plan22_phase02d_iii_b_read_auto_deref_tests {
    //! Plan-22 phase 02d-iii.b — verify
    //! `auto_deref_boxed_scalar` wraps reads of
    //! `Reference(__cell_<T>, _)` variables in
    //! `Call(OpGet<T>, [code, Int(0)])` and returns the
    //! cell's value-field type.
    //!
    //! Foundation step: the helper IS hooked into `parse_var`'s
    //! natural-return path, but it's a no-op in production
    //! because no variable carries the trigger type yet (phase
    //! 02d-iii.a's flip is dormant — see its test module).
    //! Phase 02d-iii.e activates the flip + this hook fires for
    //! real on every captured-scalar read in the parent body
    //! and the closure body.
    //!
    //! These tests invoke the helper directly with constructed
    //! inputs — they verify the wrapping logic in isolation,
    //! independent of the integration path.

    use crate::data::{Deps, Type, Value};
    use crate::parser::Parser;

    /// Helper: build a parser with defaults loaded so the OpGet*
    /// definitions exist in `Data`, then synthesise the named
    /// cell struct via the same machinery phase 02d-ii uses.
    fn parser_with_cell(value_tp: &Type, cell_name: &str) -> (Parser, u32) {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        // Build the cell struct directly (mirrors
        // `synthesize_cell_structs`).
        let cell_d_nr = p
            .data
            .add_def(cell_name, p.lexer.pos(), crate::data::DefType::Struct);
        p.data
            .add_attribute(&mut p.lexer, cell_d_nr, "value", value_tp.clone());
        (p, cell_d_nr)
    }

    #[test]
    fn integer_cell_wraps_with_op_get_int() {
        let (mut p, cell_d_nr) = parser_with_cell(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
        );
        let mut code = Value::Var(7);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        // Expect: Call(OpGetInt, [Var(7), Int(0)])
        let op_d_nr = p.data.def_nr("OpGetInt");
        match &code {
            Value::Call(d, args) => {
                assert_eq!(*d, op_d_nr, "expected OpGetInt; got d_nr={d}");
                assert_eq!(args.len(), 2);
                assert!(matches!(args[0], Value::Var(7)));
                assert!(matches!(args[1], Value::Int(0)));
            }
            other => panic!("expected Call(OpGetInt, …); got {other:?}"),
        }
        assert!(
            matches!(new_t, Type::Integer(_)),
            "expected Integer return type; got {new_t:?}"
        );
    }

    #[test]
    fn text_cell_wraps_with_op_get_text() {
        let (mut p, cell_d_nr) = parser_with_cell(&Type::Text(Deps::none()), "__cell_text");
        let mut code = Value::Var(3);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        let op_d_nr = p.data.def_nr("OpGetText");
        match &code {
            Value::Call(d, args) => {
                assert_eq!(*d, op_d_nr);
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected Call(OpGetText, …); got {other:?}"),
        }
        assert!(matches!(new_t, Type::Text(_)));
    }

    #[test]
    fn boolean_cell_wraps_byte_then_eq() {
        let (mut p, cell_d_nr) = parser_with_cell(&Type::Boolean, "__cell_boolean");
        let mut code = Value::Var(5);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        // @PLN17: boolean now reads its byte directly (0/1/255), like a plain enum —
        // Call(OpGetBoolean, [Var(5), Int(0)]) — not the old OpEqInt(OpGetByte, 1).
        let get_bool_d_nr = p.data.def_nr("OpGetBoolean");
        match &code {
            Value::Call(d, args) => {
                assert_eq!(
                    *d, get_bool_d_nr,
                    "boolean cell read should be OpGetBoolean"
                );
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected Call(OpGetBoolean, …); got {other:?}"),
        }
        assert_eq!(new_t, Type::Boolean);
    }

    #[test]
    fn float_cell_wraps_with_op_get_float() {
        let (mut p, cell_d_nr) = parser_with_cell(&Type::Float, "__cell_float");
        let mut code = Value::Var(2);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        let op_d_nr = p.data.def_nr("OpGetFloat");
        if let Value::Call(d, _) = &code {
            assert_eq!(*d, op_d_nr);
        } else {
            panic!("expected Call(OpGetFloat, …); got {code:?}");
        }
        assert_eq!(new_t, Type::Float);
    }

    #[test]
    fn character_cell_wraps_with_op_get_character() {
        let (mut p, cell_d_nr) = parser_with_cell(&Type::Character, "__cell_character");
        let mut code = Value::Var(4);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        let op_d_nr = p.data.def_nr("OpGetCharacter");
        if let Value::Call(d, _) = &code {
            assert_eq!(*d, op_d_nr);
        } else {
            panic!("expected Call(OpGetCharacter, …); got {code:?}");
        }
        assert_eq!(new_t, Type::Character);
    }

    #[test]
    fn non_cell_reference_returns_unchanged() {
        // A regular struct Reference (not a __cell_*) must NOT
        // be auto-dereffed.  This is the dormancy guarantee for
        // production code with no boxed scalars.
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        // Synthesise a non-cell struct directly so the test
        // doesn't depend on which structs the defaults provide.
        let s_d_nr = p
            .data
            .add_def("MyStruct", p.lexer.pos(), crate::data::DefType::Struct);
        let mut code = Value::Var(1);
        let original_code = code.clone();
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(s_d_nr, Deps::none()));
        assert_eq!(
            code, original_code,
            "non-cell Reference must not be wrapped"
        );
        assert!(
            matches!(new_t, Type::Reference(_, _)),
            "non-cell Reference type must pass through unchanged; got {new_t:?}"
        );
    }

    #[test]
    fn non_reference_type_returns_unchanged() {
        // Passing a bare Integer through the helper must be a
        // no-op (no Reference, nothing to deref).
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let mut code = Value::Int(42);
        let original_code = code.clone();
        let new_t = p.auto_deref_boxed_scalar(
            &mut code,
            Type::Integer(crate::data::IntegerSpec::signed32()),
        );
        assert_eq!(code, original_code, "Integer input must not be wrapped");
        assert!(matches!(new_t, Type::Integer(_)));
    }

    #[test]
    fn parse_var_path_no_op_for_non_cell_locals() {
        // Smoke test through the parse_var integration path: a
        // normal local Integer goes through `parse_var` ->
        // `auto_deref_boxed_scalar` -> returns unchanged.
        // Verifies the production hook is dormant for non-boxed
        // variables.
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(
            r"
            fn test() {
                n = 42;
                _ = n + 1;
            }
            ",
            "phase02d_iii_b_test",
            false,
        );
        let test_d_nr = p.data.def_nr("n_test");
        let vars = p.data.def(test_d_nr).variables();
        let mut found_n = false;
        for v_nr in 0..vars.next_var() {
            if vars.name(v_nr) == "n" {
                assert!(
                    matches!(vars.tp(v_nr), Type::Integer(_)),
                    "non-boxed `n` must stay Integer; got {:?}",
                    vars.tp(v_nr)
                );
                found_n = true;
            }
        }
        assert!(found_n, "`n` not found in vars table");
    }
}

#[cfg(test)]
mod plan22_phase02d_iii_c_assign_rewrite_tests {
    //! Plan-22 phase 02d-iii.c — verify
    //! `boxed_scalar_assign_rewrite` builds the correct IR for
    //! first vs subsequent assignments to a boxed-scalar local,
    //! plus `change_var_type` guard preserves the flipped type.
    //!
    //! Foundation step: helpers are shipped as infrastructure; a
    //! `parse_assign_op` hook activates them with 02d-iii.e
    //! (after 02d-iii.d wires the closure-body write rewrite).
    //! With the flip dormant in production, no variable carries
    //! the trigger type and the helpers return None / no-op for
    //! every assignment in real code.

    use crate::data::{Type, Value};
    use crate::parser::Parser;

    /// Helper: build a parser with defaults loaded, synthesise a
    /// `__cell_<T>` struct with the given value-field type, and
    /// add a fresh variable named `name` with type
    /// `Reference(cell_d_nr, [])`.
    fn parser_with_boxed_local(
        value_tp: &Type,
        cell_name: &str,
        var_name: &str,
    ) -> (Parser, u32, u16) {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let cell_d_nr = p
            .data
            .add_def(cell_name, p.lexer.pos(), crate::data::DefType::Struct);
        p.data
            .add_attribute(&mut p.lexer, cell_d_nr, "value", value_tp.clone());
        let v_nr = p.vars.add_variable(
            var_name,
            &Type::Reference(cell_d_nr, crate::data::Deps::none()),
            &mut p.lexer,
        );
        (p, cell_d_nr, v_nr)
    }

    #[test]
    fn first_set_integer_emits_alloc_and_fill() {
        let (p, cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        let ir = p
            .boxed_scalar_assign_rewrite(v_nr, "=", Value::Int(42))
            .expect("expected rewrite IR");
        let op_db = p.data.def_nr("OpDatabase");
        let op_set = p.data.def_nr("OpSetInt");
        let cell_kt = i32::from(p.data.def(cell_d_nr).known_type());
        let Value::Insert(ops) = ir else {
            panic!("expected Insert");
        };
        assert_eq!(ops.len(), 3);
        match &ops[0] {
            Value::Set(set_v, val) => {
                assert_eq!(*set_v, v_nr);
                assert!(matches!(**val, Value::Null));
            }
            other => panic!("op[0] should be Set(n, Null); got {other:?}"),
        }
        match &ops[1] {
            Value::Call(d, args) => {
                assert_eq!(*d, op_db);
                assert_eq!(args.len(), 2);
                assert!(matches!(args[0], Value::Var(x) if x == v_nr));
                assert!(matches!(args[1], Value::Int(kt) if kt == cell_kt));
            }
            other => panic!("op[1] should be OpDatabase(n, kt); got {other:?}"),
        }
        match &ops[2] {
            Value::Call(d, args) => {
                assert_eq!(*d, op_set);
                assert_eq!(args.len(), 3);
                assert!(matches!(args[0], Value::Var(x) if x == v_nr));
                assert!(matches!(args[1], Value::Int(0)));
                assert!(matches!(args[2], Value::Int(42)));
            }
            other => panic!("op[2] should be OpSetInt(n, 0, 42); got {other:?}"),
        }
    }

    #[test]
    fn subsequent_set_integer_emits_field_write_only() {
        let (mut p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        p.vars.defined(v_nr);
        let ir = p
            .boxed_scalar_assign_rewrite(v_nr, "=", Value::Int(7))
            .expect("expected rewrite IR");
        let op_set = p.data.def_nr("OpSetInt");
        match ir {
            Value::Call(d, args) => {
                assert_eq!(d, op_set);
                assert_eq!(args.len(), 3);
                assert!(matches!(args[0], Value::Var(x) if x == v_nr));
                assert!(matches!(args[1], Value::Int(0)));
                assert!(matches!(args[2], Value::Int(7)));
            }
            other => panic!("expected Call(OpSetInt, ...); got {other:?}"),
        }
    }

    #[test]
    fn text_first_set_uses_op_set_text() {
        let (p, _cell_d_nr, v_nr) =
            parser_with_boxed_local(&Type::Text(crate::data::Deps::none()), "__cell_text", "s");
        let op_set = p.data.def_nr("OpSetText");
        let ir = p
            .boxed_scalar_assign_rewrite(v_nr, "=", Value::Text("hi".to_string()))
            .expect("expected rewrite IR");
        let Value::Insert(ops) = ir else {
            panic!("expected Insert");
        };
        if let Value::Call(d, _) = &ops[2] {
            assert_eq!(*d, op_set);
        } else {
            panic!("op[2] not a Call");
        }
    }

    #[test]
    fn float_first_set_uses_op_set_float() {
        let (p, _cell_d_nr, v_nr) = parser_with_boxed_local(&Type::Float, "__cell_float", "f");
        let op_set = p.data.def_nr("OpSetFloat");
        let ir = p
            .boxed_scalar_assign_rewrite(v_nr, "=", Value::Float(2.5))
            .expect("expected rewrite IR");
        if let Value::Insert(ops) = &ir
            && let Value::Call(d, _) = &ops[2]
        {
            assert_eq!(*d, op_set);
        } else {
            panic!("expected Insert([_, _, Call(OpSetFloat, _)]); got {ir:?}");
        }
    }

    #[test]
    fn non_boxed_var_returns_none() {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let v_nr = p.vars.add_variable(
            "n",
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            &mut p.lexer,
        );
        let result = p.boxed_scalar_assign_rewrite(v_nr, "=", Value::Int(0));
        assert!(
            result.is_none(),
            "non-boxed Integer must not be rewritten; got {result:?}"
        );
    }

    #[test]
    fn non_cell_reference_returns_none() {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let s_d_nr = p
            .data
            .add_def("MyStruct", p.lexer.pos(), crate::data::DefType::Struct);
        let v_nr = p.vars.add_variable(
            "s",
            &Type::Reference(s_d_nr, crate::data::Deps::none()),
            &mut p.lexer,
        );
        let result = p.boxed_scalar_assign_rewrite(v_nr, "=", Value::Null);
        assert!(
            result.is_none(),
            "non-cell Reference must not be rewritten; got {result:?}"
        );
    }

    #[test]
    fn compound_op_returns_none() {
        let (p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        for op in &["+=", "-=", "*=", "/=", "%="] {
            let result = p.boxed_scalar_assign_rewrite(v_nr, op, Value::Int(1));
            assert!(
                result.is_none(),
                "op `{op}` must not be rewritten by 02d-iii.c; got {result:?}"
            );
        }
    }

    #[test]
    fn boolean_falls_through() {
        // OpSetByte takes (ref, fld, min, val); this helper
        // doesn't yet handle the 4-arg shape.  Phase 02d-iii.e
        // (or later) extends boolean.
        let (p, _cell_d_nr, v_nr) = parser_with_boxed_local(&Type::Boolean, "__cell_boolean", "b");
        let result = p.boxed_scalar_assign_rewrite(v_nr, "=", Value::Boolean(true));
        assert!(
            result.is_none(),
            "boolean cell falls through in 02d-iii.c; got {result:?}"
        );
    }

    #[test]
    fn change_var_type_guard_preserves_flipped_type() {
        // The `change_var_type` guard added in 02d-iii.c: if the
        // variable's current type is `Reference(__cell_integer, _)`,
        // calling `change_var_type` with an Integer arg must NOT
        // revert it.  Without this guard, parse_assign_op's
        // `change_var(to, &s_type)` would undo the flip on every
        // `n = expr`.
        let (mut p, cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        // The guard only fires when !first_pass.
        p.first_pass = false;
        p.change_var_type(v_nr, &Type::Integer(crate::data::IntegerSpec::signed32()));
        match p.vars.tp(v_nr) {
            Type::Reference(d, _) => {
                assert_eq!(*d, cell_d_nr, "type was reverted; flip not preserved");
            }
            other => panic!("type was reverted to {other:?}"),
        }
    }
}

#[cfg(test)]
mod plan22_phase02d_iii_d_alloc_prepend_tests {
    //! Plan-22 phase 02d-iii.d — verify
    //! `maybe_prepend_cell_alloc` wraps a first-set
    //! `Call(OpSet<T>, [Var(n), 0, rhs])` with the cell
    //! allocation preamble, and is a no-op for subsequent sets,
    //! closure-body writes (where LHS inner is non-Var), and
    //! non-boxed locals.
    //!
    //! The closure-body write rewrite is delivered FOR FREE by
    //! 02d-iii.b's auto-deref + `call_to_set_op` in
    //! `parser/operators.rs:283` — the existing machinery
    //! already maps `OpGetInt` → `OpSetInt` when the LHS shape
    //! is `Call(OpGetInt, [<inner>, Int(pos)])`.  This helper
    //! delivers the OUTER-binding alloc that the auto-deref +
    //! `call_to_set_op` path doesn't provide.
    //!
    //! Foundation step: helper is shipped as infrastructure;
    //! a `parse_assign_op` hook activates it with 02d-iii.e.

    use crate::data::{Type, Value};
    use crate::parser::Parser;

    /// Helper: build a parser with defaults loaded, synthesise a
    /// `__cell_<T>` struct, and add a fresh boxed-scalar local.
    fn parser_with_boxed_local(
        value_tp: &Type,
        cell_name: &str,
        var_name: &str,
    ) -> (Parser, u32, u16) {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let cell_d_nr = p
            .data
            .add_def(cell_name, p.lexer.pos(), crate::data::DefType::Struct);
        p.data
            .add_attribute(&mut p.lexer, cell_d_nr, "value", value_tp.clone());
        let v_nr = p.vars.add_variable(
            var_name,
            &Type::Reference(cell_d_nr, crate::data::Deps::none()),
            &mut p.lexer,
        );
        (p, cell_d_nr, v_nr)
    }

    #[test]
    fn first_set_wraps_with_alloc() {
        let (mut p, cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let op_db = p.data.def_nr("OpDatabase");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(0)]);
        let result = Value::Call(
            op_set,
            vec![Value::Var(v_nr), Value::Int(0), Value::Int(42)],
        );
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        let cell_kt = i32::from(p.data.def(cell_d_nr).known_type());
        let Value::Insert(ops) = wrapped else {
            panic!("expected Insert wrap; got non-Insert");
        };
        assert_eq!(ops.len(), 3);
        match &ops[0] {
            Value::Set(set_v, val) => {
                assert_eq!(*set_v, v_nr);
                assert!(matches!(**val, Value::Null));
            }
            other => panic!("op[0] should be Set(n, Null); got {other:?}"),
        }
        match &ops[1] {
            Value::Call(d, args) => {
                assert_eq!(*d, op_db);
                assert!(matches!(args[0], Value::Var(x) if x == v_nr));
                assert!(matches!(args[1], Value::Int(kt) if kt == cell_kt));
            }
            other => panic!("op[1] should be OpDatabase(n, kt); got {other:?}"),
        }
        assert_eq!(ops[2], result, "op[2] should be the original OpSetInt call");
        assert!(
            p.vars.is_defined(v_nr),
            "variable should be marked defined after first-set alloc"
        );
    }

    #[test]
    fn subsequent_set_unchanged() {
        let (mut p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        p.vars.defined(v_nr);
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(0)]);
        let result = Value::Call(op_set, vec![Value::Var(v_nr), Value::Int(0), Value::Int(7)]);
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(wrapped, result, "subsequent set should not be wrapped");
    }

    #[test]
    fn closure_body_lhs_is_no_op() {
        // When the LHS auto-deref's inner is a Call (e.g.
        // get_field(closure, n_field)) instead of Var(n), the
        // helper is a no-op — the cell was already allocated by
        // the parent.
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let fake_get_field = Value::Call(op_get, vec![Value::Var(99), Value::Int(8)]);
        let lhs = Value::Call(op_get, vec![fake_get_field.clone(), Value::Int(0)]);
        let result = Value::Call(op_set, vec![fake_get_field, Value::Int(0), Value::Int(11)]);
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(
            wrapped, result,
            "closure-body LHS (non-Var inner) should not be wrapped"
        );
    }

    #[test]
    fn non_boxed_lhs_is_no_op() {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let s_d_nr = p
            .data
            .add_def("MyStruct", p.lexer.pos(), crate::data::DefType::Struct);
        let v_nr = p.vars.add_variable(
            "s",
            &Type::Reference(s_d_nr, crate::data::Deps::none()),
            &mut p.lexer,
        );
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(0)]);
        let result = Value::Call(
            op_set,
            vec![Value::Var(v_nr), Value::Int(0), Value::Int(99)],
        );
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(
            wrapped, result,
            "non-cell Reference LHS should not be wrapped"
        );
    }

    #[test]
    fn non_call_lhs_is_no_op() {
        let (mut p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        let lhs = Value::Var(v_nr);
        let result = Value::Set(v_nr, Box::new(Value::Int(5)));
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(
            wrapped, result,
            "non-Call LHS (no auto-deref pattern) should not be wrapped"
        );
    }

    #[test]
    fn lhs_with_non_zero_offset_is_no_op() {
        // Non-zero offset means struct field read, not cell
        // value read.  Helper returns unchanged.
        let (mut p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(8)]);
        let result = Value::Call(op_set, vec![Value::Var(v_nr), Value::Int(8), Value::Int(3)]);
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(wrapped, result, "non-zero-offset LHS should not be wrapped");
    }

    #[test]
    fn text_cell_first_set_uses_correct_kt() {
        let (mut p, cell_d_nr, v_nr) =
            parser_with_boxed_local(&Type::Text(crate::data::Deps::none()), "__cell_text", "s");
        let op_get = p.data.def_nr("OpGetText");
        let op_set = p.data.def_nr("OpSetText");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(0)]);
        let result = Value::Call(
            op_set,
            vec![Value::Var(v_nr), Value::Int(0), Value::Text("hi".into())],
        );
        let wrapped = p.maybe_prepend_cell_alloc(result, &lhs);
        let cell_kt = i32::from(p.data.def(cell_d_nr).known_type());
        if let Value::Insert(ops) = wrapped
            && let Value::Call(_, args) = &ops[1]
        {
            assert!(matches!(args[1], Value::Int(kt) if kt == cell_kt));
        } else {
            panic!("expected Insert with OpDatabase op[1]");
        }
    }
}
