// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use std::collections::HashSet;

use super::{
    DefType, I32, Level, Parser, Position, Type, Value, diagnostic_format, merge_dependencies,
    v_block, v_if, v_loop, v_set,
};

/// Returns true if the given AST value definitely returns on all code paths.
/// A block definitely-returns if its last statement is a `return`, or if it is
/// an `if` with an `else` where both branches definitely-return (recursive).
pub(crate) fn definitely_returns(val: &Value) -> bool {
    match val {
        Value::Return(_) => true,
        Value::Block(bl) => {
            // A block definitely-returns if its last non-Line statement does.
            bl.operators
                .iter()
                .rev()
                .find(|v| !matches!(v, Value::Line(_)))
                .is_some_and(definitely_returns)
        }
        Value::If(_, t_branch, f_branch) => {
            // Both branches must definitely-return, and the else must not be null.
            !matches!(**f_branch, Value::Null)
                && definitely_returns(t_branch)
                && definitely_returns(f_branch)
        }
        _ => false,
    }
}

impl Parser {
    // <block> ::= '}' | <expression> {';' <expression} '}'
    pub(crate) fn parse_block(&mut self, context: &str, val: &mut Value, result: &Type) -> Type {
        if let Value::Var(v) = val
            && let Type::Reference(r, _) = self.vars.tp(*v).clone()
            && context == "block"
        {
            // We actually scan a record here instead of a block of statement
            self.parse_object(r, val);
            return Type::Reference(r, Vec::new());
        }
        self.lexer.token("{");
        if self.lexer.has_token("}") {
            *val = v_block(Vec::new(), Type::Void, "empty block");
            return Type::Void;
        }
        let mut t = Type::Void;
        let mut l = Vec::new();
        let mut terminated: Option<&str> = None;
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
            // Warn about unreachable code after an unconditional terminator.
            if let Some(kind) = terminated {
                if !self.first_pass {
                    diagnostic!(self.lexer, Level::Warning, "Unreachable code after {kind}");
                }
                // Only warn once per terminator
                terminated = None;
            }
            let mut n = Value::Null;
            t = self.expression(&mut n);
            // Track unconditional terminators at block scope.
            // if/else/loop/match contain terminators inside branches — not unconditional.
            match &n {
                Value::Return(_) => terminated = Some("return"),
                Value::Break(_) => terminated = Some("break"),
                Value::Continue(_) => terminated = Some("continue"),
                _ => {}
            }
            if let Value::Insert(ls) = n {
                Self::move_insert_elements(&mut l, ls);
                t = Type::Void;
            } else if t != Type::Void && (self.lexer.peek_token(";") || *result == Type::Void) {
                l.push(Value::Drop(Box::new(n)));
            } else {
                l.push(n);
            }
            if self.lexer.peek_token("}") {
                break;
            }
            t = Type::Void;
            match l.last() {
                Some(Value::If(_, _, _) | Value::Loop(_) | Value::Block(_)) => (),
                _ => {
                    if !self.lexer.token(";") {
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
        t = self.block_result(context, result, &t, &mut l);
        *val = v_block(l, t.clone(), "block");
        t
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
    ) -> Type {
        let mut tp = t.clone();
        if *result != Type::Void && !matches!(*result, Type::Unknown(_)) {
            let last = l.len() - 1;
            let ignore = *t == Type::Void
                && (matches!(l[last], Value::Return(_)) || definitely_returns(&l[last]));
            if !self.convert(&mut l[last], t, result) && !ignore {
                // T1-22: for function bodies with `not null` return, downgrade to a warning.
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
                    self.validate_convert(context, t, result);
                }
            }
            tp = result.clone();
        }
        if let Type::Text(ls) = t {
            self.text_return(ls);
        } else if let Type::Reference(_, ls) | Type::Vector(_, ls) = t {
            self.ref_return(ls);
        }
        tp
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
        let mut true_code = Value::Null;
        let write_state = self.vars.save_and_clear_write_state();
        self.vars.clear_write_state();
        let mut true_type = self.parse_block("if", &mut true_code, &Type::Unknown(0));
        let mut false_type = Type::Void;
        let mut false_code = Value::Null;
        if self.lexer.has_token("else") {
            self.vars.restore_write_state(&write_state);
            self.vars.clear_write_state();
            if self.lexer.has_token("if") {
                self.parse_if(&mut false_code);
            } else {
                if true_type == Type::Null {
                    true_type = Type::Unknown(0);
                }
                false_type = self.parse_block("else", &mut false_code, &true_type);
                if true_type == Type::Unknown(0) {
                    if let Value::Block(bl) = &mut true_code {
                        let p = bl.operators.len() - 1;
                        bl.operators[p] = self.null(&false_type);
                        bl.result = false_type.clone();
                    }
                    true_type = false_type.clone();
                }
            }
        } else {
            self.vars.restore_write_state(&write_state);
            if true_type != Type::Void {
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
        // 1. Parse the subject expression.
        let mut subject = Value::Null;
        let subject_type = self.expression(&mut subject);

        // Resolve type info from the subject.
        // Accepts: plain enums, struct-enums, struct-enum variants, and plain structs (T1-18).
        let (e_nr, is_struct, valid_enum, is_plain_struct) = match &subject_type {
            Type::Enum(nr, s, _) => (*nr, *s, true, false),
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::EnumValue => {
                let parent = self.data.def(*d_nr).parent;
                (parent, true, true, false)
            }
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::Struct => {
                (*d_nr, true, true, true)
            }
            // T1-14: scalar types — dispatch to scalar match handler.
            Type::Integer(_, _)
            | Type::Long
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Text(_) => {
                return self.parse_scalar_match(subject, &subject_type, code);
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

        // arms: (discriminant or None for wildcard, arm code, arm type)
        let mut arms: Vec<(Option<i32>, Value, Type)> = Vec::new();
        let mut covered: HashSet<u32> = HashSet::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;

        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            let Some(pattern_name) = self.lexer.has_identifier() else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expect variant name or '_' in match arm"
                    );
                }
                break;
            };

            if pattern_name == "_" {
                // Wildcard arm — must be last.
                has_wildcard = true;
                self.lexer.token("=>");
                let mut arm_code = Value::Null;
                let arm_type = if self.lexer.peek_token("{") {
                    self.parse_block("match_arm", &mut arm_code, &Type::Unknown(0))
                } else {
                    self.expression(&mut arm_code)
                };
                if result_type == Type::Void {
                    result_type = arm_type.clone();
                } else if !self.first_pass
                    && arm_type != Type::Void
                    && !result_type.is_same(&arm_type)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot unify: {} and {}",
                        result_type.name(&self.data),
                        arm_type.name(&self.data)
                    );
                }
                arms.push((None, arm_code, arm_type));
                self.lexer.has_token(","); // optional trailing comma
                break;
            }

            // Look up the variant definition.
            let variant_def_nr = self.data.def_nr(&pattern_name);

            // T1-18: for plain struct match, the pattern name must match the struct type.
            // There is no discriminant — the arm always matches.
            if is_plain_struct {
                if variant_def_nr != e_nr && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' does not match struct type {}",
                        pattern_name,
                        self.data.def(e_nr).name
                    );
                }
                // Parse field bindings and arm body, emit as always-matching.
                let mut arm_stmts: Vec<Value> = Vec::new();
                if self.lexer.peek_token("{") {
                    self.lexer.token("{");
                    while !self.lexer.peek_token("}") {
                        if let Some(field_name) = self.lexer.has_identifier() {
                            let attr_idx = self.data.attr(e_nr, &field_name);
                            if attr_idx != usize::MAX {
                                let field_val = self.get_field(e_nr, attr_idx, subject_val.clone());
                                let field_type = self.data.attr_type(e_nr, attr_idx);
                                let v = self.create_var(&field_name, &field_type);
                                self.vars.defined(v);
                                arm_stmts.push(v_set(v, field_val));
                            } else if !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "unknown field '{}' on struct {}",
                                    field_name,
                                    self.data.def(e_nr).name
                                );
                            }
                        }
                        if !self.lexer.has_token(",") {
                            break;
                        }
                    }
                    self.lexer.token("}");
                }
                self.lexer.token("=>");
                let mut arm_code = Value::Null;
                let arm_type = if self.lexer.peek_token("{") {
                    self.parse_block("match_arm", &mut arm_code, &Type::Unknown(0))
                } else {
                    self.expression(&mut arm_code)
                };
                arm_stmts.push(arm_code);
                let block = v_block(arm_stmts, arm_type.clone(), "struct_match");
                if result_type == Type::Void {
                    result_type = arm_type;
                }
                arms.push((None, block, result_type.clone()));
                has_wildcard = true; // plain struct arm always matches
                break;
            }

            let bad_variant = e_nr == u32::MAX
                || variant_def_nr == u32::MAX
                || self.data.def_type(variant_def_nr) != DefType::EnumValue
                || self.data.def(variant_def_nr).parent != e_nr;
            if bad_variant {
                if !self.first_pass && valid_enum && variant_def_nr != u32::MAX {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        pattern_name,
                        self.data.def(e_nr).name
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
                self.lexer.token("=>");
                let mut arm_code = Value::Null;
                self.expression(&mut arm_code);
                continue;
            }

            // Duplicate arm detection.
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

            // Get the discriminant integer for this variant.
            let disc: i32 = if is_struct {
                // Struct enum: discriminant is attributes[0].value of the EnumValue def.
                if let Value::Enum(nr, _) = self.data.def(variant_def_nr).attributes[0].value {
                    i32::from(nr)
                } else {
                    0
                }
            } else {
                // Plain enum: discriminant is stored in the parent enum's attributes.
                if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&pattern_name) {
                    if let Value::Enum(nr, _) = self.data.def(e_nr).attributes[*a_nr].value {
                        i32::from(nr)
                    } else {
                        0
                    }
                } else {
                    0
                }
            };

            // Parse optional field bindings for struct-enum arms: `{ field1, field2, ... }`.
            let mut arm_stmts: Vec<Value> = Vec::new();
            if is_struct && self.lexer.peek_token("{") {
                self.lexer.token("{");
                let mut seen_fields: HashSet<String> = HashSet::new();
                loop {
                    let Some(field_name) = self.lexer.has_identifier() else {
                        break;
                    };
                    if !self.first_pass && seen_fields.contains(&field_name) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "duplicate field binding '{}' in match arm",
                            field_name
                        );
                    }
                    seen_fields.insert(field_name.clone());

                    // Find the field in the variant's attributes (skip attr 0 = "enum" disc).
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
                            let v_nr = self.create_var(&field_name, &field_type);
                            if v_nr != u16::MAX {
                                self.vars.defined(v_nr);
                                let field_read =
                                    self.get_field(variant_def_nr, attr_idx, subject_val.clone());
                                arm_stmts.push(v_set(v_nr, field_read));
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

            self.lexer.token("=>");

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

            // Type unification across arms.
            if result_type == Type::Void {
                result_type = arm_type.clone();
            } else if !self.first_pass && arm_type != Type::Void && !result_type.is_same(&arm_type)
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot unify: {} and {}",
                    result_type.name(&self.data),
                    arm_type.name(&self.data)
                );
            }

            // Wrap field-reads + body into a block if there are field reads.
            let arm_code = if arm_stmts.is_empty() {
                arm_body
            } else {
                arm_stmts.push(arm_body);
                v_block(arm_stmts, arm_type.clone(), "match_arm")
            };

            arms.push((Some(disc), arm_code, arm_type));
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
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "match on {} is not exhaustive — missing: {}",
                    self.data.def(e_nr).name,
                    missing.join(", ")
                );
            }
        }

        // Build the if-chain from the collected arms (last to first).
        // Value::Null is the base case — reached only when no arm matches
        // (only possible if exhaustiveness fails, which is a compile error).
        let mut chain = Value::Null;
        for (disc_opt, arm_code, _) in arms.iter().rev() {
            match disc_opt {
                None => {
                    // Wildcard — always taken; becomes the else branch of the chain.
                    chain = arm_code.clone();
                }
                Some(disc_nr) => {
                    let cmp = self.cl("OpEqInt", &[disc_expr.clone(), Value::Int(*disc_nr)]);
                    chain = v_if(cmp, arm_code.clone(), chain);
                }
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
        *code = if let Some((v, init)) = preamble {
            v_block(vec![v_set(v, init), chain], result_type.clone(), "match")
        } else {
            chain
        };
        result_type
    }

    /// T1-14: parse a match expression over a scalar (integer, text, boolean, etc.).
    /// Builds an if/else chain: `if subject == lit1 { arm1 } else if subject == lit2 { arm2 } else { wildcard }`
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

        // Collect arms: (literal_value, arm_code, arm_type)
        let mut arms: Vec<(Option<Value>, Value, Type)> = Vec::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;

        loop {
            if self.lexer.peek_token("}") {
                break;
            }

            // Parse pattern: literal, `true`, `false`, `_`, or string.
            let mut pattern_val: Option<Value> = None;

            // Check for wildcard `_` — it's an identifier, not a token.
            let link = self.lexer.link();
            if self.lexer.has_identifier().as_deref() == Some("_") && self.lexer.peek_token("=>") {
                has_wildcard = true;
            } else {
                self.lexer.revert(link);
                // Parse a literal pattern (integer, float, text, true, false, etc.)
                let mut lit = Value::Null;
                let lit_type = self.expression(&mut lit);
                if !self.first_pass && lit_type != Type::Null && !lit_type.is_same(subject_type) {
                    self.can_convert(&lit_type, subject_type);
                }
                pattern_val = Some(lit);
            }

            self.lexer.token("=>");
            let mut arm_code = Value::Null;
            let arm_type = if self.lexer.peek_token("{") {
                self.parse_block("match_arm", &mut arm_code, &Type::Unknown(0))
            } else {
                self.expression(&mut arm_code)
            };
            if result_type == Type::Void {
                result_type = arm_type.clone();
            }
            arms.push((pattern_val, arm_code, arm_type));
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

        // Build if/else chain from last arm to first.
        // The last arm without a pattern (wildcard) is the else fallback.
        let fallback = if has_wildcard {
            let (_, arm_code, _) = arms.pop().unwrap();
            arm_code
        } else {
            // No wildcard — result is nullable (null if no arm matches).
            self.null(&result_type)
        };

        let mut chain = fallback;
        for (pattern_val, arm_code, _) in arms.into_iter().rev() {
            if let Some(lit) = pattern_val {
                let mut cond = Value::Null;
                let cond_tp = self.call_op(
                    &mut cond,
                    "==",
                    &[Value::Var(v), lit],
                    &[subject_type.clone(), subject_type.clone()],
                );
                if cond_tp == Type::Null {
                    // Comparison not found — fall through without condition.
                    chain = arm_code;
                } else {
                    chain = v_if(cond, arm_code, chain);
                }
            } else {
                chain = arm_code;
            }
        }

        *code = v_block(
            vec![v_set(v, subject), chain],
            result_type.clone(),
            "scalar_match",
        );
        result_type
    }

    // <for> ::= <identifier> 'in' <expression> [ 'par' '(' <id> '=' <worker> ',' <threads> ')' ] '{' <block>
    //
    // The optional parallel clause `par(b=worker(a), N)` desugars to a parallel map
    // followed by an index-based loop over the results.  Three worker call forms
    // are supported — see `parse_parallel_for_loop` for details.
    /// Set up iterator variables for a for-loop header and return
    /// `(iter_var, pre_var, for_var, if_step, create_iter, iter_next)`.
    pub(crate) fn for_type(&mut self, in_type: &Type) -> Type {
        if let Type::Vector(t_nr, dep) = &in_type {
            let mut t = *t_nr.clone();
            if let Type::Enum(nr, true, _) = t {
                t = Type::Reference(nr, vec![]);
            }
            for d in dep {
                t = t.depending(*d);
            }
            t
        } else if let Type::Sorted(dnr, _, dep) | Type::Index(dnr, _, dep) = &in_type {
            Type::Reference(*dnr, dep.clone())
        } else if let Type::Iterator(i_tp, _) = &in_type {
            if **i_tp == Type::Null {
                I32.clone()
            } else {
                *i_tp.clone()
            }
        } else if let Type::Text(_) = in_type {
            Type::Character
        } else if let Type::Reference(_, _) | Type::Integer(_, _) | Type::Long = in_type {
            in_type.clone()
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown in expression type {}",
                    in_type.name(&self.data)
                );
            }
            Type::Null
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
                if matches!(tp, Type::Text(_)) {
                    // create a new attribute with this name
                    let a = self.data.add_attribute(
                        &mut self.lexer,
                        self.context,
                        n,
                        Type::RefVar(Box::new(Type::Text(Vec::new()))),
                    );
                    self.vars.become_argument(*v);
                    dep.push(a as u16);
                    self.vars
                        .set_type(*v, Type::RefVar(Box::new(Type::Text(Vec::new()))));
                } else {
                    let a = self
                        .data
                        .add_attribute(&mut self.lexer, self.context, n, tp.clone());
                    self.vars.become_argument(*v);
                    dep.push(a as u16);
                }
            }
            self.data.definitions[self.context as usize].returned = Type::Text(dep);
        }
    }

    pub(crate) fn ref_return(&mut self, ls: &[u16]) {
        let ret = self.data.definitions[self.context as usize]
            .returned
            .clone();
        if let Type::Vector(_, cur) | Type::Reference(_, cur) = &ret {
            let mut dep = cur.clone();
            for v in ls {
                let n = self.vars.name(*v);
                // skip related variables that are already attributes
                if let Some(a) = self.data.def(self.context).attr_names.get(n) {
                    if !dep.contains(&(*a as u16)) {
                        dep.push(*a as u16);
                    }
                    continue;
                }
                // create a new attribute with this name
                let a = self
                    .data
                    .add_attribute(&mut self.lexer, self.context, n, ret.clone());
                self.vars.become_argument(*v);
                dep.push(a as u16);
            }
            self.data.definitions[self.context as usize].returned = match ret {
                Type::Vector(it, _) => Type::Vector(it, dep),
                Type::Reference(td, _) => Type::Reference(td, dep),
                _ => unreachable!("ref_return called with non-Vector/Reference return type"),
            };
        }
    }

    // <return> ::= [ <expression> ]
    pub(crate) fn parse_return(&mut self, val: &mut Value) {
        // validate if there is a defined return value
        let mut v = Value::Null;
        let r_type = self.data.def(self.context).returned.clone();
        if !self.lexer.peek_token(";") && !self.lexer.peek_token("}") {
            let t = self.expression(&mut v);
            if r_type == Type::Void {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect no expression after return"
                );
                *val = Value::Return(Box::new(Value::Null));
                return;
            }
            if t == Type::Null {
                v = self.null(&r_type);
            } else if !self.convert(&mut v, &t, &r_type) {
                self.validate_convert("return", &t, &r_type);
            }
            if let Type::Text(ls) = t {
                self.text_return(&ls);
            }
        } else if !self.first_pass && r_type != Type::Void {
            diagnostic!(self.lexer, Level::Error, "Expect expression after return");
        }
        *val = Value::Return(Box::new(v));
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
                Value::str("assert failure")
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

    pub(crate) fn parse_call(&mut self, val: &mut Value, source: u16, name: &str) -> Type {
        let call_pos = self.lexer.pos().clone();
        let mut list = Vec::new();
        let mut types = Vec::new();
        if self.lexer.has_token(")") {
            // Check for zero-argument fn-ref call
            if self.vars.name_exists(name) {
                let v_nr = self.vars.var(name);
                if let Type::Function(param_types, ret_type) = self.vars.tp(v_nr).clone()
                    && param_types.is_empty()
                {
                    if !self.first_pass {
                        self.var_usages(v_nr, true);
                        *val = Value::CallRef(v_nr, vec![]);
                    }
                    return *ret_type;
                }
            }
            return self.call(val, source, name, &list, &Vec::new());
        }
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
        if matches!(
            name,
            "assert" | "panic" | "log_info" | "log_warn" | "log_error" | "log_fatal"
        ) {
            return self.parse_call_diagnostic(val, name, &list, &types, &call_pos);
        }
        if name == "parallel_for" {
            return self.parse_parallel_for(val, &list, &types);
        }
        if name == "map" {
            return self.parse_map(val, &list, &types);
        }
        if name == "filter" {
            return self.parse_filter(val, &list, &types);
        }
        if name == "reduce" {
            return self.parse_reduce(val, &list, &types);
        }
        // If the name refers to a fn-ref variable, emit a dynamic call through it.
        if self.vars.name_exists(name) {
            let v_nr = self.vars.var(name);
            if let Type::Function(param_types, ret_type) = self.vars.tp(v_nr).clone() {
                if !self.first_pass {
                    if list.len() != param_types.len() {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Function reference '{name}' expects {} argument(s), got {}",
                            param_types.len(),
                            list.len()
                        );
                        return *ret_type;
                    }
                    let mut converted = list.clone();
                    for (i, expected) in param_types.iter().enumerate() {
                        self.convert(&mut converted[i], &types[i], expected);
                    }
                    self.var_usages(v_nr, true);
                    *val = Value::CallRef(v_nr, converted);
                }
                return *ret_type;
            }
        }
        self.call(val, source, name, &list, &types)
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
        let (fn_param_types, _fn_ret_type) = if let Type::Function(params, ret) = &types[2] {
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
                        *val = Value::Int(i32::from(
                            self.database
                                .size(self.data.def(self.data.type_elm(&tp)).known_type),
                        ));
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
                        self.database.size(self.data.def(e_tp).known_type),
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

    // <call> ::= [ <expression> { ',' <expression> } ] ')'
    pub(crate) fn parse_method(&mut self, val: &mut Value, md_nr: u32, on: Type) -> Type {
        let mut list = vec![val.clone()];
        let mut types = vec![on];
        if self.lexer.has_token(")") {
            return self.call_nr(val, md_nr, &list, &types, true);
        }
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
        self.call_nr(val, md_nr, &list, &types, true)
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
}
