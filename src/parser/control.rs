// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use std::collections::HashSet;

use super::{
    DefType, I32, Level, LexItem, Parser, Position, Type, Value, diagnostic_format,
    merge_dependencies, v_block, v_if, v_loop, v_set,
};

/// Check if the last meaningful expression in a block is divergent.
fn is_block_divergent(ops: &[Value]) -> bool {
    ops.iter()
        .rev()
        .any(|v| matches!(v, Value::Return(_) | Value::Break(_) | Value::Continue(_)))
}

/// Collected match arm data for enum/struct-enum match expressions.
struct EnumArm {
    /// discriminants for this arm — Vec allows or-patterns (multiple variants per arm).
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
    #[allow(clippy::too_many_lines)]
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
                    if matches!(te, Type::Null) && matches!(ex, Type::Integer(_, _, true)) {
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
                        && matches!(elem_tp, Type::Integer(_, _, true))
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
            // CO1.3c: generator bodies return void (values come from yield),
            // so suppress the void-vs-iterator mismatch.
            let is_generator = matches!(result, Type::Iterator(_, _));
            let ignore = is_generator
                || (matches!(*t, Type::Void | Type::Never)
                    && (matches!(l[last], Value::Return(_)) || definitely_returns(&l[last])));
            if !self.convert(&mut l[last], t, result) && !ignore {
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
                    self.validate_convert(context, t, result);
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
        if self.data.def_type(self.context) != DefType::Generic {
            if let Type::Text(ls) = t {
                self.text_return(ls);
            } else if let Type::Vector(_, ls) = t {
                self.ref_return(ls);
            } else if let Type::Reference(_, ls) = t {
                // Issue #120: when filter_hidden stripped the deps from a
                // Reference return type, recover work-ref variables from the
                // return expression. First try Call arguments, then fall back
                // to promoting ALL non-argument __ref_N work-refs.
                if ls.is_empty() && !l.is_empty() {
                    let last = &l[l.len() - 1];
                    let extra = Self::collect_hidden_ref_args(last, &self.data);
                    if !extra.is_empty() {
                        self.ref_return(&extra);
                    }
                } else {
                    self.ref_return(ls);
                }
            }
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
                if matches!(true_type, Type::Null | Type::Never) {
                    true_type = Type::Unknown(0);
                }
                false_type = self.parse_block("else", &mut false_code, &true_type);
                if true_type == Type::Unknown(0) {
                    // Only patch the true block with a null value if the last
                    // expression is NOT a divergent expression (return/break/continue).
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
                let parent = self.data.def(*d_nr).parent;
                (parent, true, true, false)
            }
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::Struct => {
                (*d_nr, true, true, true)
            }
            // scalar types — dispatch to scalar match handler.
            Type::Integer(_, _, _)
            | Type::Long
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
                let (arm, is_exhaustive) = self.parse_match_wildcard_arm(&mut result_type);
                has_wildcard = is_exhaustive;
                arms.push(arm);
                self.lexer.has_token(","); // optional trailing comma
                if !has_wildcard {
                    continue;
                }
                break;
            }

            // Look up the variant definition.
            let variant_def_nr = self.data.def_nr(&pattern_name);

            // for plain struct match, the pattern name must match the struct type.
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

            // or-patterns — collect additional variants separated by `|`.
            // Only for plain enum arms without field bindings.
            let mut all_discs = vec![disc];
            while self.lexer.has_token("|") {
                let Some(next_name) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "expect variant name after '|'");
                    }
                    break;
                };
                let next_def_nr = self.data.def_nr(&next_name);
                if !self.first_pass
                    && (next_def_nr == u32::MAX
                        || self.data.def_type(next_def_nr) != DefType::EnumValue
                        || self.data.def(next_def_nr).parent != e_nr)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        next_name,
                        self.data.def(e_nr).name
                    );
                } else {
                    let next_disc = if is_struct {
                        if let Value::Enum(nr, _) = self.data.def(next_def_nr).attributes[0].value {
                            i32::from(nr)
                        } else {
                            0
                        }
                    } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&next_name) {
                        if let Value::Enum(nr, _) = self.data.def(e_nr).attributes[*a_nr].value {
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

            // S15: restore name mappings after arm body so the next arm can
            // create its own alias for the same field name.
            for (name, old) in name_aliases.drain(..) {
                if let Some(old_nr) = old {
                    self.vars.set_name(&name, old_nr);
                } else {
                    self.vars.remove_name(&name);
                }
            }

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
                    "Error: match on {} is not exhaustive — missing: {}; add the missing variants or a '_ =>' wildcard",
                    self.data.def(e_nr).name,
                    missing.join(", ")
                );
                self.lexer.pos_diagnostic(Level::Error, &match_pos, &msg);
            }
        }

        // Build the if-chain from the collected arms (last to first).
        // Value::Null is the base case — reached only when no arm matches
        // (only possible if exhaustiveness fails, which is a compile error).
        let mut chain = Value::Null;
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
        self.lexer.token("=>");
        let mut arm_code = Value::Null;
        let arm_type = if self.lexer.peek_token("{") {
            self.parse_block("match_arm", &mut arm_code, &Type::Unknown(0))
        } else {
            self.expression(&mut arm_code)
        };
        if *result_type == Type::Void {
            *result_type = arm_type.clone();
        } else if !self.first_pass && arm_type != Type::Void && !result_type.is_same(&arm_type) {
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
                if let Value::Enum(nr, _) = self.data.def(*e_nr).attributes[*a_nr].value {
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
                        self.data.def(*e_nr).name
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
                        if let Value::Enum(nr, _) = self.data.def(*e_nr).attributes[*a_nr].value {
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
                                self.data.def(*e_nr).name
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
        let mut lit = Value::Null;
        let negate = self.lexer.has_token("-");
        let lit_type = if let Some(n) = self.lexer.has_integer() {
            let v = n as i32;
            lit = Value::Int(if negate { -v } else { v });
            Type::Integer(i32::MIN + 1, i32::MAX as u32, false)
        } else if let Some(n) = self.lexer.has_long() {
            let v = n as i64;
            lit = Value::Long(if negate { -v } else { v });
            Type::Long
        } else if let Some(n) = self.lexer.has_float() {
            lit = Value::Float(if negate { -n } else { n });
            Type::Float
        } else if let Some(s) = self.lexer.has_cstring() {
            lit = Value::Text(s);
            Type::Text(Vec::new())
        } else if let Some(c) = self.lexer.has_char() {
            lit = Value::Int(c as i32);
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

    /// parse a match expression over a scalar (integer, text, boolean, etc.).
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
            let guard_opt = if self.lexer.has_token("if") {
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

    /// parse a match expression over a vector subject.
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
                        &[Type::Integer(0, 0, false), Type::Integer(0, 0, false)],
                    );
                } else {
                    // length == fixed
                    self.call_op(
                        cond.get_or_insert(Value::Null),
                        "==",
                        &[len_call, Value::Int(fixed)],
                        &[Type::Integer(0, 0, false), Type::Integer(0, 0, false)],
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
            self.lexer.token("=>");
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

            self.lexer.token("=>");

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

    /// build a boolean condition for a single scalar pattern value.
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
        } else if let Type::Reference(_, _) | Type::Integer(_, _, _) | Type::Long = in_type {
            // I13: check for custom iterator protocol before falling back.
            let next_d_nr = self.data.find_fn(u16::MAX, "next", in_type);
            if next_d_nr != u32::MAX {
                return self.data.def(next_d_nr).returned.clone();
            }
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
                // A5.6b.1: captured text variables are read from the closure record at
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

    /// Walk a return expression to find work-ref variables passed as hidden
    /// Reference arguments to struct-returning calls.  Used by `block_result`
    /// to recover deps that `filter_hidden` stripped from the return type.
    /// Issue #120: without this, the work-ref stays a local and gets freed
    /// before the caller reads the return value.
    fn collect_hidden_ref_args(val: &Value, data: &crate::data::Data) -> Vec<u16> {
        match val {
            Value::Call(d_nr, args) => {
                let mut result = Vec::new();
                let attrs = &data.def(*d_nr).attributes;
                for (i, attr) in attrs.iter().enumerate() {
                    if attr.hidden && matches!(attr.typedef, Type::Reference(_, _)) {
                        if let Some(Value::Var(v)) = args.get(i) {
                            result.push(*v);
                        }
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
            Value::If(_, t, f) => {
                let mut r = Self::collect_hidden_ref_args(t, data);
                r.extend(Self::collect_hidden_ref_args(f, data));
                r
            }
            _ => vec![],
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
                // P117: mark as hidden return-mechanism parameter
                self.data.definitions[self.context as usize].attributes[a].hidden = true;
                self.vars.become_argument(*v);
                dep.push(a as u16);
            }
            self.data.definitions[self.context as usize].returned = match ret {
                Type::Vector(it, _) => Type::Vector(it, dep),
                Type::Reference(td, _) => Type::Reference(td, dep),
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
        let r_type = self.data.def(self.context).returned.clone();
        if !self.lexer.peek_token(";") && !self.lexer.peek_token("}") {
            // T1.7: save the position of the first token in the return expression,
            // used to report `not null` violations at the tuple literal site.
            let expr_start = self.lexer.peek();
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
            // T1.7: check for null assigned to `integer not null` tuple elements.
            if !self.first_pass
                && let (Value::Tuple(elems), Type::Tuple(expected)) = (&v, &r_type)
            {
                for (elem_val, elem_tp) in elems.iter().zip(expected.iter()) {
                    if matches!(elem_val, Value::Null)
                        && matches!(elem_tp, Type::Integer(_, _, true))
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
            if t == Type::Null {
                v = self.null(&r_type);
            } else if !self.convert(&mut v, &t, &r_type) {
                self.validate_convert("return", &t, &r_type);
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
                    if ref1_var != u16::MAX
                        && self.vars.is_argument(ref1_var)
                        && !dep.contains(&ref1_var)
                    {
                        let rec_tp =
                            i32::from(self.data.def(self.data.type_def_nr(elm_tp)).known_type);
                        let append = self.cl(
                            "OpAppendVector",
                            &[Value::Var(ref1_var), v, Value::Int(rec_tp)],
                        );
                        *val = Value::Insert(vec![
                            append,
                            Value::Return(Box::new(Value::Var(ref1_var))),
                        ]);
                        return;
                    }
                }
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
        self.parse_call_diagnostic(val, name, &list, &types, &call_pos)
    }

    /// Extract the assert condition expression from the source line.
    /// Reads the line at `pos.file:pos.line`, finds `assert(`, and extracts
    /// the text up to the matching `)`.
    fn extract_assert_expr(&self, pos: &crate::lexer::Position) -> String {
        let line = self.read_source_line(&pos.file, pos.line);
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
    #[allow(clippy::unused_self)]
    fn read_source_line(&self, file: &str, line: u32) -> String {
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
                let expr = self.extract_assert_expr(call_pos);
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
        if self.lexer.has_token(")") {
            // Check for zero-argument fn-ref call
            if self.vars.name_exists(name) {
                let v_nr = self.vars.var(name);
                if let Type::Function(param_types, ret_type, _) = self.vars.tp(v_nr).clone()
                    && param_types.is_empty()
                {
                    // A5.6b.2: create/find work-buffer text variables for text-returning fn-ref calls.
                    // work_text() adds each var to work_texts; parse_code inserts v_set(wv, Text(""))
                    // so Zone 2 slot assignment fires.  Must run on both passes for counter sync.
                    let work_vars: Vec<u16> = if let Type::Text(deps) = ret_type.as_ref() {
                        (0..deps.len())
                            .map(|_| self.vars.work_text(&mut self.lexer))
                            .collect()
                    } else {
                        vec![]
                    };
                    if !self.first_pass {
                        self.var_usages(v_nr, true);
                        let mut args = vec![];
                        // A5.6b.2: inject work-buffer DbRef blocks before __closure (zero-param case).
                        // A5.6f: clear the work buffer before each call so loop iterations start fresh.
                        let ref_def = self.data.def_nr("reference");
                        for &wv in &work_vars {
                            args.push(v_block(
                                vec![
                                    crate::data::v_set(wv, Value::Text(String::new())),
                                    self.cl("OpCreateStack", &[Value::Var(wv)]),
                                ],
                                Type::Reference(ref_def, vec![wv]),
                                "cref_work_buf",
                            ));
                        }
                        // A5.6-3: closure is embedded in the 16-byte fn-ref slot; fn_call_ref
                        // pushes it automatically — no explicit injection needed here.
                        // A5.6d: mark captured vars as read at the call site
                        for &cv in &std::mem::take(&mut self.last_closure_captured_vars) {
                            self.var_usages(cv, true);
                        }
                        *val = Value::CallRef(v_nr, args);
                    }
                    return *ret_type;
                }
            }
            return self.call(val, source, name, &list, &Vec::new(), &[]);
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
                let mut p = Value::Null;
                let t = self.expression(&mut p);
                named_args.push((arg_name, p, t));
                if !self.lexer.has_token(",") {
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
                    self.lambda_hint = expected;
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
                    ("map", 1) => Some(Type::Function(vec![elem.clone()], Box::new(elem), vec![])),
                    ("filter" | "any" | "all" | "count_if", 1) => {
                        Some(Type::Function(vec![elem], Box::new(Type::Boolean), vec![]))
                    }
                    ("reduce", 2) => {
                        let init_tp = types.get(1).cloned().unwrap_or(elem.clone());
                        Some(Type::Function(
                            vec![init_tp.clone(), elem],
                            Box::new(init_tp),
                            vec![],
                        ))
                    }
                    _ => None,
                };
                if let Some(h) = hint {
                    self.lambda_hint = h;
                }
            }
            let mut p = Value::Null;
            let t = self.expression(&mut p);
            self.lambda_hint = Type::Unknown(0);
            types.push(t);
            list.push(p);
            arg_idx += 1;
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        self.dispatch_call(val, source, name, &list, &types, &named_args, &call_pos)
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
    ) -> Type {
        if matches!(
            name,
            "assert" | "panic" | "log_info" | "log_warn" | "log_error" | "log_fatal"
        ) {
            return self.parse_call_diagnostic(val, name, list, types, call_pos);
        }
        match name {
            "parallel_for" => return self.parse_parallel_for(val, list, types),
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
                if let Type::Iterator(inner, _) = &types[0] {
                    let yield_tp = (**inner).clone();
                    let value_size =
                        crate::variables::size(&yield_tp, &crate::data::Context::Argument);
                    let op = self.data.def_nr("OpCoroutineNext");
                    let mut args = list.to_vec();
                    args.push(Value::Int(i32::from(value_size)));
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
        self.call(val, source, name, list, types, named_args)
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
        if !self.vars.name_exists(name) {
            return None;
        }
        let v_nr = self.vars.var(name);
        let Type::Function(param_types, ret_type, _) = self.vars.tp(v_nr).clone() else {
            return None;
        };
        // A5.6b.2: create/find work-buffer text variables for text-returning fn-ref calls.
        // work_text() adds each var to work_texts; parse_code inserts v_set(wv, Text(""))
        // so Zone 2 slot assignment fires.  Must run on both passes for counter sync.
        let work_vars: Vec<u16> = if let Type::Text(deps) = ret_type.as_ref() {
            (0..deps.len())
                .map(|_| self.vars.work_text(&mut self.lexer))
                .collect()
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
            // A5.6b.2: inject hidden work-buffer DbRef args for text-returning lambdas.
            // Each block emits OpCreateStack → 12-byte DbRef, matching callee's &text param.
            // Order: visible params → work bufs → __closure (must match callee slot layout).
            // A5.6f: prepend v_set(wv, "") to clear the buffer so loop iterations start fresh.
            let ref_def = self.data.def_nr("reference");
            for &wv in &work_vars {
                converted.push(v_block(
                    vec![
                        crate::data::v_set(wv, Value::Text(String::new())),
                        self.cl("OpCreateStack", &[Value::Var(wv)]),
                    ],
                    Type::Reference(ref_def, vec![wv]),
                    "cref_work_buf",
                ));
            }
            // A5.3: inject hidden __closure argument — the closure allocation
            // expression is generated inline so it runs at the call site, avoiding
            // A5.6-3: closure is embedded in the 16-byte fn-ref slot; fn_call_ref
            // pushes it automatically — no explicit injection needed at call sites.
            // A5.6d: mark captured vars as read at the call site
            for &cv in &std::mem::take(&mut self.last_closure_captured_vars) {
                self.var_usages(cv, true);
            }
            self.var_usages(v_nr, true);
            *val = Value::CallRef(v_nr, converted);
            // A5.6c: for void-return capturing lambdas, write updated closure
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
                    if outer_v != u16::MAX {
                        let field_val = self.get_field(closure_rec_d, aid, Value::Var(closure_w));
                        block.push(v_set(outer_v, field_val));
                    }
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
                        let packed = tp.size(false);
                        *val = if packed > 0 {
                            // Range-constrained integer: use packed field size
                            Value::Int(i32::from(packed))
                        } else {
                            Value::Int(i32::from(
                                self.database
                                    .size(self.data.def(self.data.type_elm(&tp)).known_type),
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
        Type::Text(Vec::new())
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

    /// A15: Parse `parallel { arm1; arm2; ... }`.
    /// Each semicolon-separated expression in the block becomes one concurrent arm.
    pub(crate) fn parse_parallel(&mut self, code: &mut Value) {
        self.lexer.token("{");
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
        if arms.is_empty() && !self.first_pass {
            diagnostic!(self.lexer, Level::Warning, "Empty parallel block");
        }
        *code = Value::Parallel(arms);
    }
}
