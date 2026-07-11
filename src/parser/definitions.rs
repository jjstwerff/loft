// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    Argument, DefType, Function, HashMap, HashSet, IntegerSpec, Level, Link, Parser, Position,
    ToString, Type, Value, complete_definition, diagnostic_format, is_camel, is_lower, is_op,
    is_upper, rename, v_block, v_if,
};

impl Parser {
    /// Consume an optional `not null` annotation, warning that it is deprecated.
    /// Returns `true` when it was present.
    ///
    /// @PLN25 F2: a type is non-null by DEFAULT now (`τ?` is the nullable form),
    /// so `not null` carries nothing — it stays parseable as an accepted no-op for
    /// back-compat, but every use gets a deprecation warning. That drives the
    /// warning-gate attrition (packages fail CI on the warning unless they carry
    /// `.allow_warnings`) toward the eventual hard "retired" error, which stays
    /// blocked on the registry republish (RESUME.md § F2 task #4). Warns once
    /// (pass 2 only) so a definition parsed in both passes reports a single note.
    fn has_deprecated_not_null(&mut self) -> bool {
        if self.lexer.has_keyword("not") {
            self.lexer.token("null");
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "`not null` is deprecated and has no effect — a type is non-null by \
                     default now; delete `not null` (write `T?` if the type should allow null)"
                );
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn warn_missing_enum_variants(&mut self, e_nr: u32, nrs: &[usize], name: &str) {
        let implemented: HashSet<u32> = nrs
            .iter()
            .filter_map(|nr| {
                if let Type::Reference(a_nr, _) = self.data.def(*nr as u32).attributes()[0].typedef
                {
                    Some(a_nr)
                } else {
                    None
                }
            })
            .collect();
        let missing: Vec<(String, Position)> = self
            .data
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, v)| v.def_type == DefType::EnumValue && v.parent == e_nr)
            .filter(|(v_nr, _)| !implemented.contains(&(*v_nr as u32)))
            .map(|(_, v)| (v.name.clone(), v.position.clone()))
            .collect();
        for (variant_name, pos) in &missing {
            self.lexer.pos_diagnostic(
                Level::Warning,
                pos,
                &format!("no implementation of '{name}' for variant '{variant_name}'"),
            );
        }
    }

    // @F20 — variant-based dynamic dispatch (synthesised enum dispatcher)
    pub(crate) fn create_enum_dispatch_fn(&mut self, e_nr: u32, nrs: &[usize]) {
        let from_nr = nrs[0] as u32;
        let name = self.data.def(from_nr).original_name().clone();
        let attrs = self.data.def(from_nr).attributes()[1..].to_vec();
        let mut common = attrs.len();
        for nr in &nrs[1..] {
            let mut c = 0;
            for a in &self.data.def(*nr as u32).attributes()[1..] {
                for o in &attrs {
                    if a.name == o.name && a.typedef == o.typedef {
                        c += 1;
                    }
                }
            }
            if c < common {
                common = c;
            }
        }
        for nr in nrs {
            if self.data.def(*nr as u32).attributes().len() > common + 1 {
                for a in &self.data.def(*nr as u32).attributes()[common + 1..] {
                    if a.value == Value::Null {
                        return;
                    }
                }
            }
        }
        let mut args = Vec::new();
        args.push(Argument {
            name: "self".to_string(),
            typedef: Type::Enum(e_nr, true, crate::data::Deps::none()),
            default: Value::Null,
            constant: false,
        });
        for a in &attrs[..common] {
            args.push(Argument {
                name: a.name.clone(),
                typedef: a.typedef.clone(),
                default: a.value.clone(),
                constant: false,
            });
        }
        let fn_nr = self.data.add_fn(&mut self.lexer, &name, &args);
        self.data.mark_synthetic(fn_nr, "enum_dispatcher");
        self.context = fn_nr;
        self.vars = Function::new(&name, &self.data.def(from_nr).position().file);
        self.data
            .set_returned(fn_nr, self.data.def(from_nr).returned().clone());
        for a in &args {
            let v_nr = self.create_var(&a.name, &a.typedef);
            if v_nr != u16::MAX {
                self.vars.become_argument(v_nr);
            }
        }
        // Build forwarding args for extra (non-self) attributes (e.g. RefVar(Text) buffers).
        // Variant calls must write into the dispatcher's own text-buffer argument, not a
        // freshly-allocated work_text that has no stack slot yet.
        let mut extra_call_args: Vec<Value> = Vec::new();
        let mut extra_call_types: Vec<Type> = Vec::new();
        for a in &args[1..] {
            let v = self.vars.var(&a.name);
            if v != u16::MAX {
                extra_call_args.push(Value::Var(v));
                extra_call_types.push(a.typedef.clone());
            }
        }
        let mut ls = Vec::new();
        let get_enum = self.cl("OpGetEnum", &[Value::Var(0), Value::Int(0)]);
        let get_int = self.cl("OpConvIntFromEnum", &[get_enum]);
        self.enum_numbers(
            nrs.to_vec(),
            &name,
            &mut ls,
            &get_int,
            &extra_call_args,
            &extra_call_types,
        );
        // No-variant-matched fallback: an explicit `return null`, not a bare
        // `Null` tail. As the tail of a value-typed (e.g. text) block the bare
        // Null was wrapped in `Str::new(<dispatch if>)` and emitted `Str::new(())`
        // (E0308) under --native; `Return(Null)` routes through the typed-null
        // return path (STRING_NULL for text, i64::MIN for int, …) on both backends.
        ls.push(Value::Return(Box::new(Value::Null)));
        self.data.definitions[fn_nr as usize].code =
            v_block(ls, self.data.def(from_nr).returned().clone(), "dynamic_fn");
        self.data.definitions[self.context as usize].variables = self.vars.clone();
        self.warn_missing_enum_variants(e_nr, nrs, &name);
    }

    pub(crate) fn enum_fn(&mut self) {
        if !self.first_pass {
            return;
        }
        let mut todo = HashMap::new();
        for (d_nr, d) in self.data.definitions.iter().enumerate() {
            if d.def_type != DefType::Function || d.attributes.is_empty() {
                continue;
            }
            if let Type::Reference(e_tp, _) = &d.attributes[0].typedef
                && matches!(self.data.def(*e_tp).returned(), Type::Enum(_, true, _))
                && self.data.find_fn(
                    u16::MAX,
                    &d.original_name(),
                    self.data.def(*e_tp).returned(),
                ) == u32::MAX
                && let Type::Enum(e_nr, true, _) = self.data.def(*e_tp).returned()
            {
                todo.entry(*e_nr).or_insert(vec![]).push(d_nr);
            }
        }
        for (e_nr, nrs) in todo {
            self.create_enum_dispatch_fn(e_nr, &nrs);
        }
    }

    pub(crate) fn enum_numbers(
        &mut self,
        nrs: Vec<usize>,
        name: &str,
        ls: &mut Vec<Value>,
        get_int: &Value,
        extra_args: &[Value],
        extra_types: &[Type],
    ) {
        for nr in nrs {
            let d_nr = nr as u32;
            let a_nr = if let Type::Reference(nr, _) = self.data.def(d_nr).attributes()[0].typedef {
                nr
            } else {
                0
            };
            let e_nr = if let Value::Enum(nr, _) = self.data.def(a_nr).attributes()[0].value {
                nr
            } else {
                0
            };
            let self_type = self.data.def(d_nr).attributes()[0].typedef.clone();
            let mut call_args = vec![Value::Var(0)];
            call_args.extend_from_slice(extra_args);
            let mut call_types = vec![self_type];
            call_types.extend_from_slice(extra_types);
            let mut code = Value::Null;
            self.call(&mut code, u16::MAX, name, &call_args, &call_types, &[], &[]);
            let ret_call = v_block(
                vec![Value::Return(Box::new(code.clone()))],
                Type::Void,
                "ret",
            );
            ls.push(v_if(
                self.cl("OpEqInt", &[get_int.clone(), Value::Int(i32::from(e_nr))]),
                ret_call,
                Value::Null,
            ));
        }
    }

    /// Parse the `{ Value { fields }, Value, ... }` body of an enum definition.
    /// Returns false if a fatal parse error occurred and parsing should stop.
    pub(crate) fn parse_enum_values(&mut self, d_nr: u32) -> bool {
        let mut nr: u8 = 0;
        loop {
            let Some(value_name) = self.lexer.has_identifier() else {
                diagnostic!(self.lexer, Level::Error, "Expect name in type definition");
                return false;
            };
            if !is_camel(&value_name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect enum values to be in camel case style"
                );
            }
            let v_nr = if self.first_pass {
                let v = self
                    .data
                    .add_def(&value_name, self.lexer.pos(), DefType::EnumValue);
                self.data.definitions[v as usize].parent = d_nr;
                v
            } else {
                // @PLN22 Phase 1 — the second-pass re-resolve goes through the
                // enum (d_nr) we are parsing via the variant_of chokepoint, not
                // the bare global def_nr.  Non-breaking while variants are still
                // globally keyed (the variant is a child of d_nr); required once
                // de-globalize lands (step 4), when def_nr no longer keys variants.
                self.data.variant_of(d_nr, &value_name)
            };
            if self.lexer.has_token("{") {
                if self.first_pass {
                    self.data.definitions[d_nr as usize].returned =
                        Type::Enum(d_nr, true, crate::data::Deps::none());
                    self.data
                        .set_returned(v_nr, Type::Enum(d_nr, true, crate::data::Deps::none()));
                    self.data.add_attribute(
                        &mut self.lexer,
                        d_nr,
                        &value_name,
                        Type::Enum(d_nr, true, crate::data::Deps::none()),
                    );
                    self.data.definitions[d_nr as usize].attributes[nr as usize].constant = true;
                    // Enum values start with 1 as 0 is de null/undefined value.
                    self.data
                        .set_attr_value(d_nr, nr as usize, Value::Enum(nr + 1, u16::MAX));
                    // Create an "enum" field inside the new structure
                    let e_attr = self.data.add_attribute(
                        &mut self.lexer,
                        v_nr,
                        "enum",
                        Type::Enum(
                            self.data.def_nr("enumerate"),
                            false,
                            crate::data::Deps::none(),
                        ),
                    );
                    // Enum values start with 1 as 0 is de null/undefined value.
                    self.data
                        .set_attr_value(v_nr, e_attr, Value::Enum(nr + 1, u16::MAX));
                }
                loop {
                    // @PLN35 — `#lexeme` marks the field carrying this token variant's surface
                    // text, so a bare literal in a slice pattern (`[ "fn", … ]`) matches against
                    // it.  It precedes the field name: `Keyword { #lexeme name: text }`.
                    let is_lexeme = if self.lexer.has_token("#") {
                        match self.lexer.has_identifier().as_deref() {
                            Some("lexeme") => true,
                            other => {
                                if !self.first_pass {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "unknown field annotation `#{}` (expected `#lexeme`)",
                                        other.unwrap_or("")
                                    );
                                }
                                false
                            }
                        }
                    } else {
                        false
                    };
                    let Some(a_name) = self.lexer.has_identifier() else {
                        diagnostic!(self.lexer, Level::Error, "Expect attribute");
                        return true;
                    };
                    if self.first_pass && self.data.attr(v_nr, &a_name) != usize::MAX {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "field `{}` is already declared",
                            a_name
                        );
                    }
                    self.lexer.token(":");
                    self.parse_field(v_nr, &a_name);
                    if is_lexeme {
                        let idx = self.data.attr(v_nr, &a_name);
                        if idx != usize::MAX {
                            self.data.definitions[v_nr as usize].attributes[idx].lexeme = true;
                        }
                    }
                    // accept trailing comma after the last field,
                    // matching struct parsing (line 1380).
                    if !self.lexer.has_token(",") || self.lexer.peek_token("}") {
                        break;
                    }
                }
                self.lexer.token("}");
            } else if self.first_pass {
                self.data
                    .set_returned(v_nr, Type::Enum(d_nr, false, crate::data::Deps::none()));
                self.data.add_attribute(
                    &mut self.lexer,
                    d_nr,
                    &value_name,
                    Type::Enum(d_nr, false, crate::data::Deps::none()),
                );
                self.data.definitions[d_nr as usize].attributes[nr as usize].constant = true;
                // Enum values start with 1 as 0 is de null/undefined value.
                self.data
                    .set_attr_value(d_nr, nr as usize, Value::Enum(nr + 1, u16::MAX));
            } else if *self.data.def(d_nr).returned() != *self.data.def(v_nr).returned() {
                self.data.definitions[v_nr as usize].returned =
                    self.data.def(d_nr).returned().clone();
            }
            // accept trailing comma after the last variant,
            // matching the trailing-comma guard on the field-list loop above.
            if !self.lexer.has_token(",") || self.lexer.peek_token("}") {
                break;
            }
            if nr == 255 {
                self.lexer
                    .diagnostic(Level::Error, "Too many enumerate values");
                break;
            }
            nr += 1;
        }
        // B2 fix: in a mixed-kind enum (some unit variants, some struct-
        // field variants), the unit variants processed *before* the
        // first struct variant got typed as Enum(d_nr, false, _) because
        // the parent enum had not yet been upgraded to struct-enum.
        // Sync both each variant's `returned` type and the parent's
        // per-variant attribute types to the final parent.returned so
        // pattern match / construction / return paths all see the same
        // struct-enum discriminator width.
        if self.first_pass {
            let parent_returned = self.data.def(d_nr).returned().clone();
            if matches!(parent_returned, Type::Enum(_, true, _)) {
                let num_variants = self.data.def(d_nr).attributes().len();
                for a_nr in 0..num_variants {
                    let v_name = self.data.def(d_nr).attributes()[a_nr].name.clone();
                    let v_nr = self.data.def_nr(&v_name);
                    if v_nr != u32::MAX {
                        self.data.definitions[v_nr as usize].returned = parent_returned.clone();
                    }
                    self.data.definitions[d_nr as usize].attributes[a_nr].typedef =
                        parent_returned.clone();
                }
            }
        }
        true
    }

    /// @PLN22 Phase 2 — true if `name` resolves ONLY through the source-0 prelude
    /// fallback (a stdlib / wildcard-imported name) and is NOT present in the
    /// source being parsed — so a definition of it here SHADOWS the prelude one
    /// rather than being rejected as a redefinition; the shadowed one stays
    /// reachable via qualification (`std::E`).
    ///
    /// The membership test is by NAME against the current source's namespace
    /// (`source_nr(cur, name)`), NOT by the found def's physical `.source`: a
    /// cross-file forward-ref type (p173) lives physically in another file's
    /// source yet is imported INTO this source's namespace, so it must be filled,
    /// not shadowed.  Built-in type-keywords (`integer`, `vector`, … — the
    /// stdlib's `DefType::Type` at source 0) are NEVER shadowable (shadowing them
    /// would re-point the language's own types).  Every other prelude/import kind
    /// (const, struct, enum, library typedef) is shadowable.
    fn prelude_shadowed(&self, name: &str) -> bool {
        let cur = self.data.source;
        // the stdlib itself (source 0) never shadows; and a name already in THIS
        // source's namespace (a real def or a cross-file forward-ref imported
        // into it) is filled/conflicts, not shadowed.
        if cur == 0 || self.data.source_nr(cur, name) != u32::MAX {
            return false;
        }
        // resolvable only via the source-0 prelude fallback?
        let prelude = self.data.def_nr(name);
        prelude != u32::MAX
            && self.data.def_type(prelude) != DefType::Unknown
            // built-in type-keyword (stdlib typedef) — sacred.
            && !(self.data.def_type(prelude) == DefType::Type
                && self.data.def(prelude).source == crate::data::STD_SOURCE)
    }

    // <enum> ::= 'enum' <identifier> '{' <value> {, <value>} '}' [';']
    // @F13 — simple enums (ordered value types)
    // @F14 — polymorphic struct-enums (per-variant fields)
    // @F15 — enum-scoped variant names + context inference
    pub(crate) fn parse_enum(&mut self) -> bool {
        if !self.lexer.has_token("enum") {
            return false;
        }
        let Some(type_name) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect name in type definition");
            return false;
        };
        if !is_camel(&type_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect enum definitions to be in camel case style"
            );
        }
        let mut d_nr = self.data.def_nr(&type_name);
        // @PLN22 Phase 2 — shadow a prelude/import name of the same key.
        if self.prelude_shadowed(&type_name) {
            d_nr = u32::MAX;
        }
        let mut conflict = false;
        if d_nr == u32::MAX {
            let pos = self.lexer.pos();
            d_nr = self.data.add_def(&type_name, pos, DefType::Enum);
        } else if self.first_pass && self.data.def_type(d_nr) == DefType::Unknown {
            self.data.definitions[d_nr as usize].def_type = DefType::Enum;
            self.data.definitions[d_nr as usize].position = self.lexer.pos().clone();
        } else if self.first_pass {
            // a name that already exists must not be reused — that
            // would overwrite the existing definition's type and crash in
            // `set_returned` below.  Emit a clear diagnostic naming the
            // existing definition's location.
            let prev_pos = self.data.def(d_nr).position().clone();
            let prev_kind = format!("{:?}", self.data.def(d_nr).def_type()).to_lowercase();
            diagnostic!(
                self.lexer,
                Level::Error,
                "enum '{type_name}' conflicts with a {prev_kind} of the same name \
                 already defined at {prev_pos} — pick a different name"
            );
            conflict = true;
        }
        if self.first_pass && !conflict {
            self.data
                .set_returned(d_nr, Type::Enum(d_nr, false, crate::data::Deps::none()));
        }
        if !self.lexer.token("{") {
            return false;
        }
        if !self.parse_enum_values(d_nr) {
            return false;
        }
        // Skip type-completion when this enum conflicts with a builtin of the
        // same name (e.g. `enum hash`): `d_nr` is then the existing builtin, and
        // `complete_definition` would re-`set_returned` an already-typed def and
        // panic.  The conflict diagnostic above is the user-facing result.
        if self.first_pass && !conflict {
            complete_definition(&mut self.lexer, &mut self.data, d_nr);
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        true
    }

    // <typedef> ::= 'type' <identifier> '=' <type_def> [ 'size' '(' <integer> ')' ] ';'
    // @F46 — type aliases (type X = …)
    pub(crate) fn parse_typedef(&mut self) -> bool {
        if !self.lexer.has_token("type") {
            return false;
        }
        let Some(type_name) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect name in type definition");
            return false;
        };
        if !self.default && !is_camel(&type_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect type definitions to be in camel case style"
            );
        }
        // detect a name collision before calling `add_def`, which
        // would otherwise panic with `Dual definition of <name>`.  Emit a
        // clear diagnostic citing the prior definition's location.
        let mut conflict = false;
        if self.first_pass {
            let mut existing = self.data.def_nr(&type_name);
            // @PLN22 Phase 2 — shadow a prelude/import name (but not a built-in
            // type-keyword, which prelude_shadowed excludes).
            if self.prelude_shadowed(&type_name) {
                existing = u32::MAX;
            }
            if existing != u32::MAX {
                let prev_pos = self.data.def(existing).position().clone();
                let prev_kind = format!("{:?}", self.data.def(existing).def_type()).to_lowercase();
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "type '{type_name}' conflicts with a {prev_kind} of the same name \
                     already defined at {prev_pos} — pick a different name"
                );
                conflict = true;
            }
        }
        let d_nr = if self.first_pass && !conflict {
            self.data
                .add_def(&type_name, self.lexer.pos(), DefType::Type)
        } else {
            self.data.def_nr(&type_name)
        };
        if self.lexer.has_token("=") {
            if let Some(tp) = self.parse_type_full(d_nr, false) {
                if self.first_pass && !conflict && d_nr != u32::MAX {
                    self.data.set_returned(d_nr, tp);
                }
            } else if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Expected a type after =");
            }
        }
        if self.lexer.has_keyword("size") {
            self.lexer.token("(");
            if let Some(n) = self.lexer.has_integer() {
                // Only 1/2/4/8 are meaningful for integer subtypes.  Larger
                // values (e.g. size(12) on the built-in `reference` alias)
                // are accepted silently — forced_size is only consulted for
                // integer types, so non-integer annotations are harmless.
                if matches!(n, 1 | 2 | 4 | 8) && self.first_pass && d_nr != u32::MAX {
                    self.data.definitions[d_nr as usize].forced_size = Some(n as u8);
                }
            }
            self.lexer.token(")");
        }
        // Same guard as `parse_enum`: a conflicting `type hash = …` leaves `d_nr`
        // pointing at the existing builtin, so re-completing it would panic.
        if self.first_pass && !conflict {
            complete_definition(&mut self.lexer, &mut self.data, d_nr);
        }
        self.lexer.token(";");
        true
    }

    // <constant>
    // Accepts either `NAME = expr;` or `NAME: type = expr;`. The optional
    // type annotation is parsed (so the parser doesn't reject the form)
    // but the inferred type from the initialiser is the source of truth.
    pub(crate) fn parse_constant(&mut self) -> bool {
        // P246 — accept the optional `const` keyword at file scope as
        // a synonym for the bare-name form (`const PI = 3.14;` ===
        // `PI = 3.14;`).  Pre-fix the leading `const` swallowed
        // identifiable name, the parser fell through to
        // `expression()`, and `change_var_type` panicked on an empty
        // file-scope variable table.  The two forms are identical at
        // every level — same definition kind, same UPPER_CASE check,
        // same code path — so the keyword is purely an explicitness
        // signal at the declaration site (matches the in-fn `const`
        // syntax and lib/hex_world/src/wall.loft's existing usage).
        let _explicit_const = self.lexer.has_keyword("const");
        if let Some(id) = self.lexer.has_identifier() {
            // Optional `: type` annotation between the identifier and `=`.
            // Parsed and discarded — the literal's element type is used.
            // A future enhancement could validate the inferred type matches
            // the annotation (after dep-list normalisation).
            if self.lexer.has_token(":") {
                let _ = self.parse_type_full(u32::MAX, false);
            }
            self.lexer.token("=");
            if !is_upper(&id) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect constants to be in upper case style"
                );
            }
            let mut val = Value::Null;
            let tp = self.expression(&mut val);
            if self.first_pass {
                // detect a name collision before calling `add_def`,
                // which would otherwise panic with `Dual definition of <name>`.
                let mut existing = self.data.def_nr(&id);
                // @PLN22 Phase 2 — shadow a prelude/import name of the same key.
                if self.prelude_shadowed(&id) {
                    existing = u32::MAX;
                }
                if existing == u32::MAX {
                    let c_nr = self.data.add_def(&id, self.lexer.pos(), DefType::Constant);
                    self.data.set_returned(c_nr, tp);
                    self.data.definitions[c_nr as usize].code = val;
                } else {
                    let prev_pos = self.data.def(existing).position().clone();
                    let prev_kind =
                        format!("{:?}", self.data.def(existing).def_type()).to_lowercase();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "constant '{id}' conflicts with a {prev_kind} of the same name \
                         already defined at {prev_pos} — pick a different name"
                    );
                }
            }
            self.lexer.token(";");
            true
        } else {
            false
        }
    }

    /// Read the function name after `fn`.  In user code only identifiers are accepted.
    /// In the default library, `assert` and `panic` are also allowed even though they are
    /// keywords — they remain real functions with call-site file/line injection.
    fn parse_fn_name(&mut self) -> Option<String> {
        if let Some(name) = self.lexer.has_identifier() {
            return Some(name);
        }
        if self.default {
            if self.lexer.has_token("assert") {
                return Some("assert".to_string());
            }
            if self.lexer.has_token("panic") {
                return Some("panic".to_string());
            }
        }
        diagnostic!(
            self.lexer,
            Level::Error,
            "Expect name in function definition"
        );
        None
    }

    #[allow(clippy::too_many_lines)]
    // @F16 — functions & declarations (pub, parameters, return)
    pub(crate) fn parse_function(&mut self) -> bool {
        if !self.lexer.has_token("fn") {
            return false;
        }
        let Some(fn_name) = self.parse_fn_name() else {
            return false;
        };
        self.vars = Function::new(&fn_name, &self.lexer.pos().file);
        // @PLN25 E2 — clear any type-var from a previous function before parsing
        // this one; set below if this function is generic.
        self.cur_type_var = u32::MAX;
        if !self.default && !is_lower(&fn_name) && !is_op(&fn_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect function names to be in lower case style"
            );
        }
        // detect `<T>` type parameter after function name.
        // @F25 — generics: single type variable <T>, inferred
        let mut is_generic = false;
        let mut type_var_name = String::new();
        // I4: bound names collected from `<T: A + B>` — resolved to def_nrs in the second pass.
        let mut pending_bounds: Vec<String> = Vec::new();
        if self.lexer.has_token("<") {
            if let Some(tv) = self.lexer.has_identifier() {
                if !is_camel(&tv) && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Type variable '{}' must be CamelCase",
                        tv
                    );
                }
                type_var_name = tv;
                is_generic = true;
                // I4: parse `<T: A + B>` bound list; collect raw names here, resolve in second pass.
                if self.lexer.has_token(":") {
                    loop {
                        if let Some(bound_name) = self.lexer.has_identifier() {
                            pending_bounds.push(bound_name);
                        } else if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Expected interface name in type bound"
                            );
                        }
                        if !self.lexer.has_token("+") {
                            break;
                        }
                    }
                }
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expected type variable name after '<'"
                );
            }
            self.lexer.closing_angle();
        }
        let mut arguments = Vec::new();
        if self.lexer.token("(") {
            // register the type variable as a struct so parse_type
            // resolves it to Reference(d, []).  The definition is never
            // compiled — it only exists for the template's type resolution.
            if is_generic {
                let existing = self.data.def_nr(&type_var_name);
                // A prior generic's type-var placeholder is an attribute-less `Struct`, safe to
                // reuse (that is how `<T>` is shared across functions). Any OTHER existing def
                // — a constant (e.g. `E`), a function, an enum, or a real struct/type — is a
                // COLLISION: loft has one flat namespace, so a generic parameter cannot share a
                // name. Report it (mirroring the `type X conflicts with …` diagnostic) instead
                // of silently binding the parameter to that def and panicking later in
                // `predict_generic_return_type`.
                let collision = existing != u32::MAX
                    && !(self.data.def(existing).def_type() == DefType::Struct
                        && self.data.def(existing).attributes().is_empty());
                if collision {
                    if self.first_pass {
                        let ed = self.data.def(existing);
                        let prev_pos = ed.position().clone();
                        let prev_kind = format!("{:?}", ed.def_type()).to_lowercase();
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "generic type parameter '{type_var_name}' conflicts with a \
                             {prev_kind} of the same name already defined at {prev_pos} — \
                             pick a different name"
                        );
                    }
                    // Stop treating the function as generic so the unresolved parameter never
                    // reaches the generic type-resolution path (which would panic).
                    is_generic = false;
                } else if self.first_pass && existing == u32::MAX {
                    // register the type variable as a struct so parse_type
                    // resolves it to Reference(d, []).  The definition is never
                    // compiled — it only exists for the template's type resolution.
                    let tv_nr =
                        self.data
                            .add_def(&type_var_name, self.lexer.pos(), DefType::Struct);
                    self.data
                        .set_returned(tv_nr, Type::Reference(tv_nr, crate::data::Deps::none()));
                }
            }
            // @PLN25 E2 — record the type-var def_nr (valid in both passes: the
            // stub was just added in the first pass, already exists in the
            // second) so `e2_nullable_elem` leaves a generic `vector<T>` dense.
            if is_generic {
                self.cur_type_var = self.data.def_nr(&type_var_name);
            }
            if !self.parse_arguments(&fn_name, &mut arguments) {
                return true;
            }
            self.lexer.token(")");
        }
        // validate that the type variable appears in the first parameter.
        if is_generic && !arguments.is_empty() {
            let tv_nr = self.data.def_nr(&type_var_name);
            let has_tv = arguments[0].typedef.contains_def(tv_nr);
            if !has_tv && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Type variable {} must appear in the first parameter — \
                     move {} to the first parameter position",
                    type_var_name,
                    type_var_name
                );
            }
        } else if is_generic && arguments.is_empty() && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Generic function must have at least one parameter of type {}",
                type_var_name
            );
        }
        self.context = if self.default && self.first_pass && is_op(&fn_name) {
            self.data.add_op(&mut self.lexer, &fn_name, &arguments)
        } else if self.first_pass {
            let d = self.data.add_fn(&mut self.lexer, &fn_name, &arguments);
            if is_generic && d != u32::MAX {
                self.data.definitions[d as usize].def_type = DefType::Generic;
            }
            // @PLN99 Arc C — a user-defined conversion `fn OpConvXFromY` / `fn OpCastXFromY`
            // is a global stored `n_OpConv…`, so it skipped `add_op` and never entered the
            // `possible` map that `convert`/`cast` search.  Register it (by type-matched
            // prefix) so `value as T` and implicit conversions dispatch a user `S → T`,
            // exactly like a built-in — the loop in `convert` still matches on arg/return type.
            if d != u32::MAX {
                if fn_name.starts_with("OpConv") {
                    self.data.register_possible("OpConv", d);
                } else if fn_name.starts_with("OpCast") {
                    self.data.register_possible("OpCast", d);
                }
            }
            d
        } else if self.default && is_op(&fn_name) {
            self.data.def_nr(&fn_name)
        } else {
            self.data.get_fn(&fn_name, &arguments)
        };
        if self.context == u32::MAX {
            return false;
        }
        // @PLN86 §7.2 (F7) — now the function's def_nr exists, key each parsed parameter
        // `…#default` lock by `(this fn, param index)`.  First pass only (definitions
        // resolve there); the call-site gate reads `param_locks` on the second pass.
        if self.first_pass {
            for (idx, token) in std::mem::take(&mut self.pending_param_locks) {
                self.param_locks.insert((self.context, idx as u32), token);
            }
        }
        // @PLN86 step 1.2 — record the sandbox profile for a host-designated
        // function so the admission walk (and the nesting guard, 0.1) know this
        // def is restricted.  Designation is host-controlled (`fn:<name>` here;
        // file globs later) — never from the source, so a script cannot mark
        // itself.  Re-derived on every pass (def_sandbox is cleared at parse start).
        if let Some(profile) = self.sandbox.fn_designation(&fn_name).map(str::to_string) {
            self.def_sandbox.insert(self.context, profile);
            // @PLN86 step 0.1 — enter restricted parsing for this def's body: the
            // nesting guard activates and its depth state starts fresh.
            self.in_sandbox = true;
            self.parse_depth = 0;
            self.depth_overflowed = false;
        } else {
            self.in_sandbox = false;
        }
        // Plan-17 phase 01 (B) — bound resolution + t-stub creation now
        // happens on BOTH passes.  Before, this block was gated on
        // `!self.first_pass`, leaving `definitions[ctx].bounds` empty
        // on first pass.  The body parser's bounded-T method-dispatch
        // path (`fields.rs::field` I7) then couldn't find the t-stub,
        // returned `Type::Unknown(0)`, and the receiving variable
        // stayed Unknown — `s + "!"` after `s = x.to_text()` then
        // failed with "No matching operator '+' on 'unknown(0)' and
        // 'text'".  By resolving on first pass too, the t-stub exists
        // when the body parses on first pass and the dispatch returns
        // the bound's declared return type.
        //
        // First-pass forward-decl tolerance: if a bound's interface
        // hasn't been declared yet (forward reference), `def_nr` returns
        // u32::MAX.  We skip the diagnostic on first pass (it'll fire
        // again on second pass with all defs visible) but still install
        // any bounds we CAN resolve so the body can dispatch.
        // I4: resolve pending bound names to interface def_nrs.
        if !pending_bounds.is_empty() {
            let mut bounds = Vec::new();
            for bname in &pending_bounds {
                let b_nr = self.data.def_nr(bname);
                if b_nr == u32::MAX {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "'{}' is not a known interface",
                            bname
                        );
                    }
                    // First pass: silent skip — interface may be a
                    // forward declaration; second pass will catch
                    // genuinely-unknown ones.
                } else if !matches!(self.data.def_type(b_nr), DefType::Interface) {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "'{}' is not an interface — bounds must be interface names",
                            bname
                        );
                    }
                } else {
                    bounds.push(b_nr);
                }
            }
            self.data.definitions[self.context as usize].bounds = bounds;
            // I7/I8.1: Create T-parameterized stubs for each bound interface's methods so
            // the body parser can emit `Value::Call(t_stub_nr, ...)` for method/op calls on T.
            // `re_resolve_call` then substitutes these with the concrete type's implementation.
            let tv_nr = self.data.def_nr(&type_var_name);
            let self_nr = self.data.def_nr("Self");
            if tv_nr != u32::MAX && self_nr != u32::MAX {
                let self_prefix = format!("t_{}Self_", "Self".len());
                let iface_nrs: Vec<u32> =
                    self.data.definitions[self.context as usize].bounds.clone();
                for iface_nr in iface_nrs {
                    let children: Vec<u32> = self.data.children_of(iface_nr).collect();
                    for child_nr in children {
                        let child_name = self.data.def(child_nr).name().to_string();
                        // Extract method name from interface-scoped stub names:
                        // "__iface_{d_nr}_{method}" → "method"
                        // Also handle legacy "t_4Self_{method}" format.
                        let method_suffix = if let Some(rest) = child_name.strip_prefix("__iface_")
                        {
                            rest.split_once('_')
                                .map_or(rest.to_string(), |(_, m)| m.to_string())
                        } else if child_name.starts_with(&self_prefix) {
                            child_name[self_prefix.len()..].to_string()
                        } else {
                            child_name.clone()
                        };
                        let t_stub_name = format!(
                            "t_{}{}_{}",
                            type_var_name.len(),
                            type_var_name,
                            method_suffix
                        );
                        if self.data.def_nr(&t_stub_name) != u32::MAX {
                            continue; // already created (e.g. multiple bounds share a method)
                        }
                        let attrs_count = self.data.def(child_nr).attributes().len();
                        let t_stub_nr =
                            self.data
                                .add_def(&t_stub_name, self.lexer.pos(), DefType::Function);
                        for a_nr in 0..attrs_count {
                            let a_name = self.data.attr_name(child_nr, a_nr);
                            let a_type = self.data.attr_type(child_nr, a_nr);
                            let new_type = Self::substitute_type(
                                a_type,
                                self_nr,
                                &crate::data::Type::Reference(tv_nr, crate::data::Deps::none()),
                            );
                            self.data
                                .add_attribute(&mut self.lexer, t_stub_nr, &a_name, new_type);
                        }
                        let ret_type = self.data.def(child_nr).returned().clone();
                        let t_ret_type = Self::substitute_type(
                            ret_type,
                            self_nr,
                            &crate::data::Type::Reference(tv_nr, crate::data::Deps::none()),
                        );
                        self.data.set_returned(t_stub_nr, t_ret_type.clone());
                        // I9-text: if the interface method returns text, add the hidden
                        // __work_1 parameter that text_return would add for concrete
                        // implementations.  Without this, the call-site argument count
                        // won't match after re_resolve_call substitutes the concrete
                        // text-returning method (which has the hidden param).
                        if matches!(t_ret_type, crate::data::Type::Text(_)) {
                            self.data.add_attribute(
                                &mut self.lexer,
                                t_stub_nr,
                                "__work_1",
                                crate::data::Type::RefVar(Box::new(crate::data::Type::Text(
                                    crate::data::Deps::none(),
                                ))),
                            );
                        }
                        // The @PLAN59 twin of the I9-text arm: a concrete method
                        // returning Reference / Vector / struct-Enum carries the
                        // hidden `__retbuf` attribute (the H1 heap-return ABI,
                        // added at signature parse), so the stub must carry it
                        // too — otherwise the template call site parses one
                        // argument short of the concrete implementation that
                        // `re_resolve_call` substitutes, and byte_code's arity
                        // assert fires ("Too few parameters … got 2, need 3").
                        // `add_defaults` fills the slot with a caller-side
                        // work-ref exactly like a direct call; when T
                        // instantiates to a primitive whose operator has no
                        // `__retbuf` (a `#rust` op), the trailing-argument trim
                        // in `substitute_type_in_value` drops it again.
                        if matches!(
                            t_ret_type,
                            crate::data::Type::Reference(_, _)
                                | crate::data::Type::Vector(_, _)
                                | crate::data::Type::Enum(_, true, _)
                        ) {
                            let a = self.data.add_attribute(
                                &mut self.lexer,
                                t_stub_nr,
                                "__retbuf",
                                t_ret_type.clone(),
                            );
                            self.data.definitions[t_stub_nr as usize].attributes[a].hidden = true;
                            // Mirror ref_return's finalisation on concrete
                            // implementations ("returned = {__retbuf}"): the
                            // stub RETURNS its retbuf, so the returned type's
                            // deps must name the retbuf attribute.  The call
                            // site then binds the result as a BORROW of the
                            // caller-side buffer (one free at scope end).
                            // Without it the bind was owned — the buffer's
                            // store was freed twice-in-name and the second
                            // local vector's store never freed (#482, one
                            // main_vector leaked per call).  `returned` is a
                            // set-once field, so write the dep directly.
                            let dep = crate::data::Deps::attrs(vec![a as u16]);
                            let dep_ret = match t_ret_type.clone() {
                                crate::data::Type::Reference(d, _) => {
                                    crate::data::Type::Reference(d, dep)
                                }
                                crate::data::Type::Vector(e, _) => {
                                    crate::data::Type::Vector(e, dep)
                                }
                                crate::data::Type::Enum(d, m, _) => {
                                    crate::data::Type::Enum(d, m, dep)
                                }
                                other => other,
                            };
                            self.data.definitions[t_stub_nr as usize].returned = dep_ret;
                        }
                    }
                }
            }
        }
        let mut returned_not_null = false;
        let mut result = if self.lexer.has_token("->") {
            // Will be the correct def_nr on the second pass
            if let Some(tp) = self.parse_type_full(self.data.def_nr(&fn_name), true) {
                if self.has_deprecated_not_null() {
                    returned_not_null = true;
                }
                tp
            } else {
                // message
                Type::Void
            }
        } else {
            Type::Void
        };
        // @PLN102 Phase 3 (N-Domain) — the domain-partial math fns are declared `-> τ?` in the
        // stdlib (they yield the reserved null out of their real domain). When LOFT_NULLFLOW is
        // OFF, strip the `?` so their return stays non-null and the default surface is byte-
        // identical until the flag flips default-on. Self-contained fns only (no desugar cascade):
        // sqrt / asin / acos. (pow / log — and their desugar consumers exp / ln / log2 / log10 —
        // land with the constant-in-domain elision, step 3.5.)
        if !crate::keys::nullflow_enabled()
            && matches!(result, Type::Optional(_))
            && matches!(
                fn_name.as_str(),
                "sqrt" | "asin" | "acos" | "ln" | "log" | "log2" | "log10" | "pow"
            )
        {
            result = result.base().clone();
        }
        // @PLN86 P6.2 — the call-gate capability link in the SIGNATURE, after the output
        // (`-> int fs#read`, or a void fn's `) fs#update`): a first-class part of the
        // contract beside the params + return, NOT in the `#native`/`#impure`/`#wasm`
        // implementation plumbing.  A restricted caller needs this granted to CALL the
        // function (`admit_capabilities`); passing arguments is part of the call.  After
        // an output, only `;` / `{` / a link is legal, so a leading identifier is
        // unambiguously a link.  Re-read each pass, so set unconditionally.
        if let Some(token) = self.try_cap_link() {
            self.data.definitions[self.context as usize].cap = token;
        }
        // Plan-14 phase 07 (P234 runtime): when the declared return type
        // is `Type::Tuple(elems)` and any element carries a lifetime
        // concern (Text, Reference, Vector, Enum-struct, keyed
        // collection, RefVar, or a nested tuple containing one of
        // those), rewrite the return type to `Reference(__tuple<…>)`
        // — the synthetic struct that loft already creates via
        // `data.tuple_def(...)` for stored tuples (P189b path).
        // The function then returns a DbRef and all existing
        // `ref_return` / `text_return` ownership-transfer machinery
        // applies unchanged.  Pure-value tuples skip the rewrite and
        // keep using Rust's tuple ABI (the T1.8a path for shapes
        // like `(integer, integer)`).
        //
        // Skip for generic templates — `T` resolves later to a concrete
        // type which may or may not have lifetime concerns; rewriting
        // pre-specialisation would freeze the wrong shape.  Mirrors the
        // generic-template guard in `block_result` (control.rs ~line 426)
        // that excludes generic templates from `ref_return` /
        // `text_return`.
        let is_generic_template =
            self.context != u32::MAX && self.data.def_type(self.context) == DefType::Generic;
        // A7.1: also rewrite pure-value tuples wider than the 8-byte
        // primitive return slot.  Three- and four-arity tuples and
        // nested tuples don't fit in a single eval-stack slot under
        // par dispatch; routing them through the synthetic struct
        // unifies the par-tuple-return path with the lifetime-bearing
        // case from Phase 07.  Safe after P236's fix (work-ref
        // unification across If branches in
        // `parser/control.rs::unify_if_branches_work_refs`); without
        // it, `min_max(...) -> (integer, integer) { if cond { (a, b) }
        // else { (c, d) } }` regressed on `--native` because each
        // branch's separate synthetic-struct work-ref dropped the
        // if/else's value.
        //
        // P196 follow-up (2026-05-12): exclude tuples that contain a
        // `Type::Function` element from the size>8 trigger.  Function
        // values are 16 bytes (u32 d_nr + DbRef closure ref), so any
        // tuple containing one trips size>8 even when the OTHER
        // elements are pure primitives — but the synthetic struct
        // wrapping breaks at the assignment site `Pair { v: pp }` where
        // `Pair.v: (fn, integer)` stays as a bare tuple type but
        // `pp: Reference(__tuple<fn, integer>)` after the rewrite.
        // Function-element tuple returns worked correctly BEFORE
        // commit 44fdd098 added the size>8 trigger, so excluding them
        // preserves the original P196 codegen path while keeping the
        // A7.1 win for pure-primitive 3+ arity tuples.  The
        // `has_lifetime_concern` arm still fires for Text / Reference /
        // Vector / etc. elements that genuinely need by-reference
        // passing; only the size-driven trigger is narrowed.
        let needs_tuple_rewrite = matches!(&result, crate::data::Type::Tuple(elems)
            if elems.iter().any(crate::data::has_lifetime_concern)
                || (u32::from(crate::variables::size(&result, &crate::data::Context::Argument)) > 8
                    && !elems.iter().any(|e| matches!(e, crate::data::Type::Function(_, _, _)))));
        // @PLN85 generic-tuple-return-fix.md — a generic template whose return SHAPE
        // is already concrete (`-> (text, text)`, no `T` in any element) is not the
        // "T resolves later" case the skip guards; let it ride the same promotion the
        // non-generic gets so the monomorph inherits it (sites 1 + 3 + block_result).
        // Only a shape that DEPENDS on `T` (`-> (T, T)`) defers to instantiation.
        let generic_return_promotable =
            !is_generic_template || !self.return_shape_depends_on_type_var(&result);
        if generic_return_promotable
            && needs_tuple_rewrite
            && let crate::data::Type::Tuple(elems) = &result
        {
            let elems_clone = elems.clone();
            let synthetic_d_nr = self.data.tuple_def(&mut self.lexer, &elems_clone);
            result = crate::data::Type::Reference(synthetic_d_nr, crate::data::Deps::none());
        }
        self.vars
            .append(&mut self.data.definitions[self.context as usize].variables);
        if self.first_pass {
            self.data.set_returned(self.context, result);
            self.data.definitions[self.context as usize].returned_not_null = returned_not_null;
        }
        // Dep inference for native methods: if a native fn (no body, `;`-terminated)
        // has a `self` parameter and returns the same struct-enum type, the return
        // borrows from self's store.  Mark dep=[0] (self attribute) so
        // inline_struct_return can distinguish accessors (dep non-empty, borrow)
        // from constructors (dep empty, own).
        if self.first_pass && self.lexer.peek_token(";") {
            let def = &self.data.definitions[self.context as usize];
            if let Some(self_attr) = def.attributes().first()
                && self_attr.name == "self"
                && let Type::Enum(ret_nr, true, dep) = def.returned()
                && dep.is_empty()
                && let Type::Enum(self_nr, true, _) = &self_attr.typedef
                && ret_nr == self_nr
            {
                self.data.definitions[self.context as usize].returned =
                    Type::Enum(*ret_nr, true, crate::data::Deps::attrs(vec![0]));
            }
        }
        if !self.lexer.has_token(";") {
            for (a_nr, a) in arguments.iter().enumerate() {
                if self.first_pass {
                    let v_nr = self.create_var(&a.name, &a.typedef);
                    if v_nr != u16::MAX {
                        self.vars.become_argument(v_nr);
                        self.var_usages(v_nr, false);
                    }
                } else {
                    self.change_var_type(a_nr as u16, &a.typedef);
                    if a.constant {
                        self.vars.set_const_param(a_nr as u16);
                    }
                }
            }
            // @PLAN59 / H1 phase 1 — the unconditional heap-return buffer:
            // every BODY-carrying plain fn returning Reference / Vector /
            // struct-Enum gets its hidden `__retbuf` attribute + backing
            // argument var at signature parse, so arity is fixed before ANY
            // caller parses.  `ref_return` binds a promoted local to it by
            // renaming the ATTR and retiring this placeholder var (probes
            // C3/C6 in plans/59-return-abi).  Excluded: native decls (`;`,
            // handled by the enclosing branch), ops / `#rust`-templated fns
            // (implemented in Rust, never promoted — their ABI must not
            // change), generic templates (specialisations never promote,
            // I9-var), lambdas (separate parse path, no earlier callers).
            if self.first_pass
                && generic_return_promotable
                && matches!(
                    self.data.def_type(self.context),
                    DefType::Function | DefType::Generic
                )
                && self.data.def(self.context).rust().is_empty()
                && matches!(
                    self.data.def(self.context).returned(),
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                )
            {
                let ret = self.data.def(self.context).returned().clone();
                let a =
                    self.data
                        .add_attribute(&mut self.lexer, self.context, "__retbuf", ret.clone());
                self.data.definitions[self.context as usize].attributes[a].hidden = true;
                let v = self.create_var("__retbuf", &ret);
                if v != u16::MAX {
                    self.vars.become_argument(v);
                    self.var_usages(v, false);
                    self.vars.mark_used(v);
                }
            }
            // re-apply name remaps for promoted text arguments in second pass.
            if !self.first_pass {
                for (shadow, original) in self.vars.promoted_text_args() {
                    let orig_name = self.vars.name(original).to_string();
                    self.vars.remap_name(&orig_name, shadow);
                    // Mark original as used so test_used doesn't warn.
                    self.vars.mark_used(original);
                }
                // Plan-22 phase 02d-iii.e — ACTIVATE the type
                // flip.  After this point, every mutated-scalar-
                // capture local in this function carries
                // `Reference(__cell_<T>, [])` instead of its
                // original scalar type.  Lambdas inside the body
                // will snapshot the flipped type into
                // `capture_context`, so phase 02c's auto-Reference
                // closure-record encoding fires correctly.
                //
                // Reads (parent body + closure body): wrapped by
                // 02d-iii.b's `auto_deref_boxed_scalar` hook in
                // `parse_var`.  Writes (parent body): wrapped by
                // 02d-iii.d's `maybe_prepend_cell_alloc` hook in
                // `parse_assign_op` (alloc on first-set; existing
                // `towards_set` → `call_to_set_op` machinery
                // emits the OpSet<T> via the auto-deref'd LHS
                // pattern).  Writes (closure body): handled
                // for free by the same machinery.
                //
                // The void-return write-back path in
                // `parse_call_ref` is gated to skip boxed-scalar
                // attributes — propagation goes through the
                // shared cell DbRef, not via per-call slot copy.
                self.flip_scalars_to_box_types();
                // #318 sink R1: a fn cannot RETURN a closure-carrying
                // struct — the value's closure record holds raw DbRefs
                // into this frame's stores, which die at return (the
                // caller then silently corrupts whatever reuses the
                // slots).  Checked in pass 2 only: by then pass 1 has
                // recorded every capturing assignment on the struct's
                // attributes, so the predicate is complete.  Returning
                // a BARE capturing closure stays supported (the case-C
                // factory transfer owns the record + captures).
                let returned = self.data.def(self.context).returned().clone();
                if self.type_carries_closure(&returned) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "function returns a struct type that holds a capturing closure; \
                         the closure references state owned by this function's frame, so \
                         the value cannot outlive it — construct the struct in the frame \
                         that owns the captured state and pass it down, or return the \
                         closure itself (#318)"
                    );
                }
            }
            self.parse_code();
            // #314 — pass-1 sibling of the pass-2 flip above: now that
            // the whole body is parsed, `scalars_to_box` is final;
            // reject any mutated scalar that more than one closure
            // captures (GOALS.md § "Stability trumps features").
            if self.first_pass {
                self.reject_shared_mutable_scalar_captures(self.context);
            }
            // reset transient closure state after each function body.
            // Without this, a lambda inside make_adder leaks last_closure_work_var
            // into the next function parsed (main), causing closure_var_of to
            // return a stale value for add5 = make_adder(5).
            self.last_closure_work_var = u16::MAX;
            if !self.first_pass {
                self.check_ref_mutations(&arguments);
                // Plan-06 PRIORITY.md spine step 5 — analyse each
                // `let r = parallel_for(...)` site for materialising
                // uses of r and emit a deprecation warning pointing
                // users at the streaming form (fused for-par) or
                // explicit `par_to_vec(...)` (planned phase 11).
                self.check_par_result_singlepass();
            }
        }
        if !self.first_pass {
            // Stub functions with an empty body `{ }` and a `self` parameter are intentional
            // skips (e.g. to silence the "no implementation for variant" warning).
            // Don't warn about unused parameters in that case.
            let is_stub = {
                let def = &self.data.definitions[self.context as usize];
                let body_empty = matches!(def.code(), Value::Block(bl) if bl.operators.is_empty());
                // @F19 — method dispatch via self / both
                let first_is_self = def.attributes().first().is_some_and(|a| a.name == "self");
                body_empty && first_is_self
            };
            // Each warning pass below seeks the lexer to a diagnostic site
            // (`lexer.to(..)`) without rewinding the actual read cursor.
            // `to()` only moves the *reporting* line/pos, but the tokenizer
            // increments that line on every subsequent physical line it
            // pulls — so a backward seek here silently offsets the line
            // number of every diagnostic emitted for the code that follows
            // (e.g. a later dead-assignment).  Save the true position and
            // restore it once the passes finish so they are position-neutral.
            let warn_pos = self.lexer.at();
            if !is_stub {
                self.vars.test_used(&mut self.lexer, &self.data);
            }
            // P246 follow-up — UPPER_CASE locals without `const`
            // violate the "UPPER_CASE means immutable constant"
            // convention.  Run once per function in the second pass
            // (after const_param flags are settled).
            self.vars.warn_upper_case_locals(&mut self.lexer);
            // Plan-07 phase 4e.2 — undefended fault-site warning.
            // Walks this function's body looking for fault-prone op
            // calls (OpDivInt / OpRemInt / OpGetVector / OpVectorRef /
            // OpTextCharacter) that survived the 4d.1 / 4d.2 / 4e.1
            // swap passes; emits `Level::Warning` unless an easy-proof
            // skip pattern applies.  Silenceable via
            // `LOFT_NO_WARN_RUNTIME=1` env var.  Second-pass only —
            // first pass doesn't have the swap-pass results yet.
            let body = self.data.definitions[self.context as usize].code.clone();
            self.warn_undefended_fault_sites(&body);
            // @PLN87 P3 (W4) — a `&` on a heap struct param that is never reassigned
            // has no effect (field mutation propagates regardless).
            self.warn_redundant_amp(&body);
            // @PLN46 W3 — auto-infer `#null_safe` from entry guards (after the warn
            // pass, so this fn's flag is set for LATER callers' walks).
            self.infer_function_null_safe(&body);
            self.lexer.to(warn_pos);
        }
        self.lexer.has_token(";");
        self.parse_rust();
        self.data.op_code(self.context);
        self.data.definitions[self.context as usize]
            .variables
            .append(&mut self.vars);
        self.context = u32::MAX;
        // @PLN86 step 0.1 — leave restricted parsing; trusted top-level code that
        // follows is never depth-guarded.
        self.in_sandbox = false;
        true
    }

    // <rust> ::= { '#rust' <string> | '#iterator' <string> <string> }
    // <native> ::= '#native' <string>   (any file)
    pub(crate) fn parse_rust(&mut self) {
        loop {
            if !self.lexer.peek_token("#") {
                break;
            }
            // Speculatively consume `#`; revert if the annotation is not recognised.
            let link = self.lexer.link();
            self.lexer.has_token("#");
            let id = self.lexer.has_identifier();
            if id == Some("native".to_string()) {
                if let Some(sym) = self.lexer.has_cstring() {
                    // Explicit override — for the rare case where the native
                    // symbol differs from the loft fn name (e.g. a
                    // `foo_native` loft wrapper backing the `n_foo` symbol).
                    // @PLAN12 — if the explicit string EQUALS the canonical
                    // default (the fn's own stored name, `n_<fn>` for a free
                    // fn), it's redundant: a bare `#native` derives the same
                    // symbol.  Reject it so the redundant form can't drift
                    // back in — bare `#native` is the only spelling for the
                    // common case; an explicit string is reserved for a
                    // symbol that genuinely DIFFERS from the fn name.
                    let canonical = self.data.def(self.context).name().to_string();
                    if sym == canonical {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "redundant `#native \"{sym}\"` — the symbol equals the \
                             function name; write a bare `#native` instead (an \
                             explicit string is only for a symbol that differs)"
                        );
                    }
                    self.data.definitions[self.context as usize].native = sym;
                } else {
                    // @PLAN12 — bare `#native` defaults the symbol to the
                    // function's own name.  A free `fn foo(...)` is stored as
                    // `n_foo` (see `Data::add_fn`), which IS the conventional
                    // native symbol, so the binding needs no separate string.
                    let default_sym = self.data.def(self.context).name().to_string();
                    self.data.definitions[self.context as usize].native = default_sym;
                }
            } else if self.default && id == Some("rust".to_string()) {
                if let Some(c) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust = c;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust string");
                }
            } else if self.default && id == Some("iterator".to_string()) {
                if let Some(init) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust = init;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust init string");
                }
                if let Some(next) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust += "#";
                    self.data.definitions[self.context as usize].rust += &next;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust next string");
                }
            } else if id == Some("null_safe".to_string()) {
                // @PLN46 W2 — `#null_safe` asserts every nullable parameter
                // tolerates null and yields a defined result, so a fault-prone
                // expression (`s[i]`) passed DIRECTLY as an argument is not flagged
                // at the call site (the possible-null is the callee's contract).
                self.data.definitions[self.context as usize].null_safe = true;
            } else if id == Some("pure".to_string()) {
                // Plan-06 phase 5a (DESIGN.md D8.1): `#pure`
                // declares "no observable side effects, no
                // parent-store writes".  Always par-safe.
                self.data.definitions[self.context as usize].purity = crate::data::Purity::Pure;
            } else if id == Some("impure".to_string()) {
                // Plan-06 phase 5a (DESIGN.md D8.1):
                // `#impure(category)` classifies the side effect.
                // Five categories: host_io, prng, io, parent_write,
                // par_call.
                if !self.lexer.has_token("(") {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Expect '(' after #impure — use #impure(host_io|prng|io|parent_write|par_call)"
                    );
                    self.lexer.revert(link);
                    break;
                }
                let category = self.lexer.has_identifier();
                let cat = match category.as_deref() {
                    Some("host_io") => crate::data::ImpureCategory::HostIo,
                    Some("prng") => crate::data::ImpureCategory::Prng,
                    Some("io") => crate::data::ImpureCategory::Io,
                    Some("parent_write") => crate::data::ImpureCategory::ParentWrite,
                    Some("par_call") => crate::data::ImpureCategory::ParCall,
                    other => {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown #impure category {:?} — expected host_io|prng|io|parent_write|par_call",
                            other.unwrap_or("(missing)")
                        );
                        self.lexer.revert(link);
                        break;
                    }
                };
                if !self.lexer.has_token(")") {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Expect ')' after #impure category"
                    );
                    self.lexer.revert(link);
                    break;
                }
                self.data.definitions[self.context as usize].purity =
                    crate::data::Purity::Impure(cat);
            } else {
                // Not a recognised annotation — put the `#` back and stop.
                self.lexer.revert(link);
                break;
            }
        }
    }

    pub(crate) fn parse_arguments(&mut self, fn_name: &str, arguments: &mut Vec<Argument>) -> bool {
        // @PLN86 §7.2 (F7) — collect this list's `…#default` parameter locks fresh; the
        // caller (`parse_function`) records them once the function's def_nr exists.
        self.pending_param_locks.clear();
        loop {
            if self.lexer.peek_token(")") {
                break;
            }
            let Some(attr_name) = self.lexer.has_identifier() else {
                diagnostic!(self.lexer, Level::Error, "Expect attribute");
                return false;
            };
            if !is_lower(&attr_name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect function attributes to be in lower case style"
                );
            }
            for a in arguments.iter() {
                if attr_name == a.name {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Double attribute '{fn_name}.{attr_name}'"
                    );
                }
            }
            let mut constant = false;
            let mut reference = false;
            let typedef = if self.lexer.has_token(":") {
                if self.lexer.has_token("&") {
                    reference = true;
                }
                // Will be the correct def_nr on the second pass
                if self.lexer.has_keyword("const") {
                    constant = true;
                }
                if let Some(tp) = self.parse_type_full(self.data.def_nr(fn_name), false) {
                    // @PLN25 E2/E3 — a `vector<Struct>` PARAM is already rewritten by
                    // the vector-type-resolution chokepoint (`sub_type` `vector` arm),
                    // so no per-site hook here.
                    if reference {
                        Type::RefVar(Box::new(tp))
                    } else {
                        tp
                    }
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expecting a type");
                    return true;
                }
            } else {
                Type::Unknown(0)
            };
            // if this parameter has `= expr`, the expression may
            // reference earlier parameters of the same function.  Inject
            // those earlier params into `self.vars` before parsing the
            // default, track which var_nr each maps to, then rewrite the
            // parsed Value tree so references use the *argument index*
            // (0, 1, …) rather than the parser's internal var_nr.
            // `fill_defaults` in src/parser/mod.rs::substitute_param_refs
            // replaces `Var(argument_index)` with the caller's actual arg.
            let injected: Vec<(String, u16, u16)> = if self.lexer.peek_token("=") {
                let mut mapping = Vec::new();
                for (i, a) in arguments.iter().enumerate() {
                    if a.typedef.is_unknown() {
                        continue;
                    }
                    if self.vars.var(&a.name) != u16::MAX {
                        continue;
                    }
                    let v = self.vars.add_variable(&a.name, &a.typedef, &mut self.lexer);
                    if v != u16::MAX {
                        self.vars.become_argument(v);
                        self.vars.defined(v);
                        mapping.push((a.name.clone(), v, i as u16));
                    }
                }
                mapping
            } else {
                Vec::new()
            };
            let val = if self.lexer.has_token("=") {
                let mut t = Value::Var(arguments.len() as u16);
                self.expression(&mut t);
                // Rewrite Var(injected_slot) → Var(arg_index) so the stored
                // default is portable across call sites.
                for (_name, slot, arg_idx) in &injected {
                    t = Self::remap_var_nr(t, *slot, *arg_idx);
                }
                t
            } else {
                Value::Null
            };
            for (name, _, _) in &injected {
                self.vars.remove_name(name);
            }
            // @PLN86 §7.2 (F7) — an optional `group#default` lock after the parameter
            // (its default): `count: int = 1 spawn.count#default`.  Consumed on BOTH
            // passes (else the second pass chokes on the token); the index is the slot
            // this parameter is about to occupy (`arguments.len()`).  `try_cap_link` is
            // non-destructive, so a `,`/`)` after the default is never mis-consumed.
            if let Some(token) = self.try_cap_link() {
                self.pending_param_locks.push((arguments.len(), token));
            }
            if !self.first_pass
                && typedef.is_unknown()
                && val == Value::Null
                && (!self.default || !matches!(typedef, Type::Vector(_, _)))
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expecting a clear type, found {}",
                    typedef.name(&self.data)
                );
            }
            (*arguments).push(Argument {
                name: attr_name,
                typedef,
                default: val,
                constant,
            });
            if !self.lexer.has_token(",") {
                break;
            }
        }
        true
    }

    pub(crate) fn parse_fn_type(&mut self, d_nr: u32) -> Type {
        let mut r_type = Type::Void;
        let mut args = Vec::new();
        self.lexer.token("(");
        loop {
            if self.lexer.peek_token(")") {
                break;
            }
            if let Some(tp) = self.parse_type_full(d_nr, false) {
                args.push(tp);
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        if self.lexer.has_token("->")
            && let Some(tp2) = self.parse_type_full(d_nr, false)
        {
            r_type = tp2;
        }
        Type::Function(args, Box::new(r_type), crate::data::Deps::none())
    }

    // <type> ::= <identifier> [::<identifier>] [ '<' ( <sub_type> | <type> ) '>' ] [ <depend> ] [ '?' ]
    //
    // @PLN25 Phase 0 (EXPAND): accept a postfix `?` on ANY scalar / struct type
    // (`integer?`, `text?`, `S?`) as the nullable opt-in. Today plain types are
    // ALREADY nullable by default, so `?` is a behaviour-preserving no-op — its
    // job in EXPAND is purely to let nullable sites be pre-annotated (MIGRATE)
    // before the Phase-2 default flip gives the marker teeth. The vector ELEMENT
    // `?` (`vector<S?>`) is consumed earlier in `sub_type_inner` (before this
    // returns), so this wrapper never steals it — it only catches the outer `?`.
    pub(crate) fn parse_type(
        &mut self,
        on_d: u32,
        type_name: &str,
        returned: bool,
    ) -> Option<Type> {
        let t = self.parse_type_inner(on_d, type_name, returned)?;
        if self.lexer.has_token("?") {
            // @PLN101 — a `value struct` is stored INLINE (bytes, no `DbRef`), so it has no
            // `store_nr` null sentinel: `<value struct>?` cannot be represented. Reject it with
            // a clear diagnostic; fall through as the plain (non-null) type to avoid a cascade.
            if let Type::Reference(p, _) = &t
                && self.data.is_value_struct(*p)
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{type_name}?` is not allowed — a `value struct` is stored inline and has \
                     no null; use a plain `{type_name}`, or a reference `struct` for nullability"
                );
                return Some(t);
            }
            // @PLN25 slice (a): the postfix `?` constructs the real `Optional` former
            // (idempotent + normalising via `Type::optional`). GATED on `LOFT_PLN25_OPT`
            // while the slice-(b) peel audit is incomplete — OFF keeps the Phase-0 no-op
            // (suite byte-identical), ON surfaces the remaining consuming-site mis-routes.
            if crate::keys::pln25_optional_enabled() {
                return Some(Type::optional(t));
            }
            // Gate OFF — Phase-0 behaviour: Integer records nullable explicitly; other
            // scalars have no flag yet, so accept-and-ignore (the base `t` falls through).
            if let Type::Integer(spec) = &t {
                return Some(Type::Integer(IntegerSpec {
                    not_null: false,
                    ..*spec
                }));
            }
        }
        Some(t)
    }

    // `pub(crate)` so the `as`-cast (operators.rs) can parse the target type
    // WITHOUT the postfix-`?` consumer — the cast detects the `?` itself to tell
    // `as τ` (fit-checked) from `as τ?` (checked cast); see DN4.
    pub(crate) fn parse_type_inner(
        &mut self,
        on_d: u32,
        type_name: &str,
        returned: bool,
    ) -> Option<Type> {
        // Phase 2c round 10c: `long` has been removed as a user-facing
        // type.  Callers now use `integer` everywhere; if anyone still
        // writes `long` it parses as an unknown identifier and fails
        // normally via the standard `data.def_nr` lookup path below.
        let tp_nr = if self.lexer.has_token("::") {
            if let Some(name) = self.lexer.has_identifier() {
                let source = self.data.get_source(type_name);
                self.data.source_nr(source, &name)
            } else {
                diagnostic!(self.lexer, Level::Error, "Expect type from {type_name}");
                return None;
            }
        } else {
            self.data.def_nr(type_name)
        };
        if self.first_pass && tp_nr == u32::MAX && type_name != "spatial" {
            // @P296-sibling — for a qualified `lib::Type` reference, `tp_nr`
            // was computed as `source_nr(source, name)` (the type, e.g.
            // `CellSnap`), but the pass-1 placeholder is keyed on
            // `type_name` (the lib prefix, e.g. `audience_crystal`).  When
            // the lib isn't resolvable yet in pass-1 (e.g. Windows lib-path
            // resolution lands the load after this reference), a SECOND
            // unresolved `lib::Other` ref would re-`add_def(lib_prefix)` →
            // "Dual definition" panic.  Reuse an existing same-named Unknown
            // placeholder instead of re-adding it; pass-2 resolves the real
            // type once the lib is fully parsed.  (The non-qualified path
            // already dedups via `tp_nr` below, so this only affects the
            // qualified shape.)
            let existing = self.data.def_nr(type_name);
            let u_nr = if existing != u32::MAX && self.data.def_type(existing) == DefType::Unknown {
                existing
            } else {
                self.data
                    .add_def(type_name, self.lexer.pos(), DefType::Unknown)
            };
            return Some(Type::Unknown(u_nr));
        }
        if tp_nr != u32::MAX && self.data.def_type(tp_nr) == DefType::Unknown {
            return Some(Type::Unknown(tp_nr));
        }
        let link = self.lexer.link();
        if self.lexer.has_token("<")
            && let Some(value) = self.sub_type(on_d, type_name, link)
        {
            return Some(value);
        }
        let mut dep = Vec::new();
        self.parse_depended(returned, &mut dep);
        let mut min = i32::MIN + 1;
        let mut max = i32::MAX as u32;
        if type_name == "integer" {
            let has_limit = self.parse_type_limit(&mut min, &mut max);
            // T1.7: check for `not null` annotation after the integer type
            let not_null = self.has_deprecated_not_null();
            if has_limit || not_null {
                // Phase 2c round 10c — all integer ranges stay as Type::Integer
                // (i64 storage + i64 arithmetic at rest).  Narrow-bounded
                // ranges (u8/u16/i8/i16/i32-range) get packed storage via
                // `forced_size`; wide ranges (up to u32::MAX) use full
                // 8-byte storage.  Type::Long is no longer produced.
                return Some(Type::Integer(IntegerSpec {
                    min,
                    max,
                    not_null,
                    forced_size: None,
                }));
            }
        }
        let dt = self.data.def_type(tp_nr);
        if tp_nr != u32::MAX
            && matches!(
                dt,
                DefType::Type | DefType::Enum | DefType::EnumValue | DefType::Struct
            )
        {
            if matches!(dt, DefType::EnumValue)
                || (self.first_pass && matches!(dt, DefType::Struct))
            {
                Some(Type::Reference(tp_nr, crate::data::Deps::unknown(dep)))
            } else if matches!(self.data.def(tp_nr).returned(), Type::Text(_)) {
                Some(Type::Text(crate::data::Deps::unknown(dep)))
            } else {
                // when a user-typed integer alias carries an
                // explicit `size(N)` annotation (e.g. `i32`, `u8`, `u16`),
                // stamp the forced width onto the returned Type::Integer so
                // the signal flows through `Box<Type>` in `Type::Vector` /
                // `Hash` / `Sorted` / `Index` to the element resolver
                // (Phase 2) and the indexing codegen (Phase 3).
                //
                // Skip the base `integer` primitive: its `forced_size = 8`
                // matches the default heuristic; stamping would clutter
                // every `Type::Integer` with `Some(8)` for no benefit.
                let mut tp = self.data.def(tp_nr).returned().clone();
                if type_name != "integer"
                    && let Type::Integer(mut spec) = tp
                    && let Some(forced) = self.data.forced_size(tp_nr)
                    && let Some(nz) = std::num::NonZeroU8::new(forced)
                    && forced != 8
                {
                    spec.forced_size = Some(nz);
                    tp = Type::Integer(spec);
                }
                Some(tp)
            }
        } else {
            None
        }
    }

    /// Parse a type expression that may be a tuple `(T1, T2, ...)` or an identifier-based type.
    /// This is the entry point for type positions (return types, parameter types, annotations).
    pub(crate) fn parse_type_full(&mut self, on_d: u32, returned: bool) -> Option<Type> {
        if self.lexer.has_token("(") {
            // Tuple type: (T1, T2, ...)
            let mut types = Vec::new();
            loop {
                if self.lexer.peek_token(")") {
                    break;
                }
                if let Some(tp) = self.parse_type_full(on_d, false) {
                    types.push(tp);
                } else {
                    break;
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token(")");
            if types.len() < 2 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Tuple types require at least 2 elements"
                );
                return types.into_iter().next();
            }
            // Plan-06 phase 4d: register the synthetic `__tuple<…>`
            // struct as soon as the tuple type appears in a type
            // position (struct-field declaration, parameter, return
            // type, …).  Without this, `fill_database` later sees
            // `Type::Tuple` with `type_elm == u32::MAX` and silently
            // skips registering the host struct's tuple field —
            // leaving `database.position("v")` as `u16::MAX` when
            // codegen needs the host field offset.  Mirrors the
            // `sub_type` tuple arm below.  Idempotent.
            self.data.tuple_def(&mut self.lexer, &types);
            Some(Type::Tuple(types))
        } else if self.lexer.has_token("fn") {
            Some(self.parse_fn_type(on_d))
        } else if let Some(id) = self.lexer.has_identifier() {
            self.parse_type(on_d, &id, returned)
        } else {
            None
        }
    }

    pub(crate) fn sub_type(&mut self, on_d: u32, type_name: &str, link: Link) -> Option<Type> {
        let tp = self.sub_type_inner(on_d, type_name, link)?;
        // #318 sink R3: no collection of a closure-carrying struct.
        // An element copy embeds a closure record whose raw DbRefs
        // point into the constructing frame (silent corruption once
        // the frame dies and slots are reused) — the same reason the
        // plan-15 matrix CLOSED `vector<capturing fn>`; a struct
        // wrapper was a loophole around that decision.  Checked in
        // pass 2 (layout complete).
        if !self.first_pass
            && matches!(
                tp,
                Type::Vector(_, _)
                    | Type::Hash(_, _, _)
                    | Type::Sorted(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Radix(_, _, _)
            )
            && self.type_carries_closure(&tp)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "collection of a struct type that holds a capturing closure is not \
                 supported — element copies would dangle into the constructing \
                 function's frame; keep closure holders in local variables and pass \
                 them down as arguments (#318)"
            );
        }
        Some(tp)
    }

    fn sub_type_inner(&mut self, on_d: u32, type_name: &str, link: Link) -> Option<Type> {
        // Plan-06 phase 4d.A — accept tuple as the inner type of
        // `vector<(T1, T2, ...)>` (and reserve the same shape for
        // `iterator<(T1, T2)>` once that lands).  Without this, the
        // identifier-only check below would reject `(` and the parser
        // would mis-parse the rest of `<(...)>` as a less-than
        // expression on a bare `vector` type.
        if self.lexer.peek_token("(") {
            let tp = self.parse_type_full(on_d, false)?;
            // P189: register a synthetic tuple struct so the rest of
            // the type system (fill_database, type_def_nr, vector_of)
            // can treat the tuple identically to a named struct.
            // Idempotent — the same tuple shape resolves to the same
            // def_nr across the program.
            if let Type::Tuple(types) = &tp {
                self.data.tuple_def(&mut self.lexer, types);
            }
            return Some(match type_name {
                "vector" => {
                    self.lexer.closing_angle();
                    Type::Vector(Box::new(tp), crate::data::Deps::none())
                }
                "iterator" => {
                    self.lexer.closing_angle();
                    Type::Iterator(Box::new(tp), Box::new(Type::Null))
                }
                _ => {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "{type_name}<(...)> not supported — tuple element types only \
                         allowed on vector / iterator"
                    );
                    self.lexer.has_closing_angle();
                    Type::Unknown(0)
                }
            });
        }
        // Plan-06 phase 4d.A.2 — recognise `fn` as a type-keyword inside
        // `vector<...>` and `iterator<...>`.  Before this, the parser
        // saw `vector<fn(integer) -> integer>`, sub_type's
        // identifier-only check rejected `fn`, the lexer reverted
        // past `<`, and the caller's annotation parser
        // (`parse_assign:1009`) entered a tight retry loop on the
        // unconsumed `<` — `loft --dump file.loft` hung at 100% CPU.
        //
        // Storage uses 4-byte i32 d_nr — `data::type_def_nr` and
        // `type_elm` route Type::Function to `i32`'s def_nr, and
        // `parser/vectors::get_type` returns `database.int(0, false)`
        // so vector elements are written as raw 4-byte d_nrs (the same
        // path `vector<i32>` uses).  At read-back time, the par
        // dispatcher's `read_tuple_at_wide` (with a single-element
        // `[Type::Function]` shape) inflates each row's 4 bytes into
        // the worker's 20-byte fn-ref slot ([8B i64 d_nr][12B null
        // closure DbRef]).
        if self.lexer.peek_token("fn") {
            self.lexer.token("fn");
            let tp = self.parse_fn_type(on_d);
            return Some(match type_name {
                "vector" => {
                    self.lexer.closing_angle();
                    Type::Vector(Box::new(tp), crate::data::Deps::none())
                }
                "iterator" => {
                    self.lexer.closing_angle();
                    Type::Iterator(Box::new(tp), Box::new(Type::Null))
                }
                _ => {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "{type_name}<fn(...)> not supported — fn-ref element types \
                         only allowed on vector / iterator"
                    );
                    self.lexer.has_closing_angle();
                    Type::Unknown(0)
                }
            });
        }
        if let Some(sub_name) = self.lexer.has_identifier() {
            // @PLN25 storage-vs-access-nullability — a POSTFIX `?` on the element type
            // (`vector<S?>`) is the nullable-element opt-IN. Dense is the default: a
            // struct / enum element stores inline; `S?` synthesises the `__nullable<S>`
            // enum. The `?` is a flag on the type, not the headline — hence postfix,
            // pairing with `x ?? d`. Keyed collections stay dense (a key denotes
            // presence) — `?` there is an error.
            let nullable_elem = self.lexer.has_token("?");
            if nullable_elem && type_name != "vector" {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`?` (nullable element) is only valid on `vector` — `{type_name}` \
                     elements are always dense (a key / slot denotes presence)"
                );
            }
            // before trying to resolve the element type, fail fast if the
            // identifier shadows a non-type definition (constant, function).
            // parse_type silently returns None in that case; sub_type's later
            // assert!(self.first_pass) masks the issue in pass 1 and
            // typedef.rs::fill_database panics later when a struct-def happens
            // to carry the same name without being a real type.
            let dn = self.data.def_nr(&sub_name);
            if dn != u32::MAX {
                let dt = self.data.def_type(dn);
                if !matches!(
                    dt,
                    DefType::Struct
                        | DefType::Enum
                        | DefType::EnumValue
                        | DefType::Type
                        | DefType::Unknown
                ) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is a {:?}, not a type — the element of {}<T> must \
                         be a struct or enum (defined at {})",
                        sub_name,
                        dt,
                        type_name,
                        self.data.def(dn).position()
                    );
                    // Consume the rest of the <...> so the parser stays
                    // synchronised on the next token.
                    self.lexer.recover_to(&[">", ";", "}"]);
                    self.lexer.has_closing_angle();
                    return Some(Type::Unknown(0));
                }
            }
            if let Some(tp) = self.parse_type(on_d, &sub_name, false) {
                let sub_nr = if let Type::Unknown(d) = tp {
                    d
                } else {
                    self.data.type_def_nr(&tp)
                };
                let mut fields = Vec::new();
                return Some(match type_name {
                    "index" => {
                        // @PLN25 — keyed collections (`index`/`hash`/`sorted`) are DENSE:
                        // nullability is a SEQUENCE concept (`vector`/`array`) only, so a keyed
                        // element is implicitly `not null` (a key denotes presence).  Accept an
                        // explicit `not null` in the definition as a no-op.  (Design:
                        // single-payload-refactor.md § "DESIGN DECISION (2026-06-20)".)
                        self.has_deprecated_not_null();
                        self.parse_fields(true, &mut fields);
                        Type::Index(
                            self.data.type_def_nr(&tp),
                            fields,
                            crate::data::Deps::none(),
                        )
                    }
                    "hash" => {
                        // Dense (implicitly `not null`) — see the `index` arm.
                        self.has_deprecated_not_null();
                        self.parse_fields(false, &mut fields);
                        self.data.set_referenced(sub_nr, on_d, Value::Null);
                        let mut f = Vec::new();
                        for (field, _) in fields {
                            f.push(field);
                        }
                        Type::Hash(sub_nr, f, crate::data::Deps::none())
                    }
                    "vector" => {
                        // @PLN25 E2/E3 — the ONE chokepoint where every inline
                        // `vector<S>` element type resolves (local / param / return /
                        // field / nested all reach here).  Default = nullable: rewrite
                        // a struct element `Reference(S)` to the synthetic
                        // `__nullable<S>` enum.  A `not null` after a NAMED element
                        // (`vector<Row not null>`) is the dense opt-out — consume it
                        // and skip the rewrite, leaving the bare inline struct.
                        // (Scalar elements consume their own `not null` in the type
                        // parse above, so this fires only for a named element.)
                        // `not null` still accepted (now redundant — dense IS the
                        // default) as a no-op, for back-compat with existing source.
                        self.has_deprecated_not_null();
                        self.lexer.closing_angle();
                        // @PLN25 storage-vs-access-nullability: DENSE by default; the
                        // leading `?` (`vector<?S>`) opts in to a nullable element,
                        // synthesising the `__nullable<S>` enum.
                        let elem = if nullable_elem {
                            self.e2_nullable_elem(tp)
                        } else {
                            tp
                        };
                        Type::Vector(Box::new(elem), crate::data::Deps::none())
                    }
                    "sorted" => {
                        // Dense (implicitly `not null`) — see the `index` arm.
                        self.has_deprecated_not_null();
                        self.parse_fields(true, &mut fields);
                        Type::Sorted(sub_nr, fields, crate::data::Deps::none())
                    }
                    "spatial" => {
                        // @PLN48 S2 — `spatial<T[x, y]>` lowers to the shared `Radix`
                        // runtime kind (RADIX_TREE.md §8.1): the coordinate key fields
                        // become the Morton-interleaved axes.  Mirrors the `hash` arm;
                        // a spatial index needs its coordinate keys, so a bare
                        // `spatial<T>` (no key-spec) is a helpful error rather than a
                        // silent empty key.
                        self.has_deprecated_not_null();
                        if self.lexer.peek_token("[") {
                            self.parse_fields(false, &mut fields);
                            self.data.set_referenced(sub_nr, on_d, Value::Null);
                            let mut f = Vec::new();
                            for (field, _) in fields {
                                f.push(field);
                            }
                            // The Morton code interleaves at most `MAX_AXES` axes; a wider
                            // key would index past the `[u64; MAX_AXES]` array at runtime
                            // (a production panic).  Reject it here with a clean message.
                            if f.len() > crate::radix_db::MAX_AXES {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "spatial<T[…]> supports at most {} coordinate axes, got {}",
                                    crate::radix_db::MAX_AXES,
                                    f.len()
                                );
                            }
                            Type::Radix(sub_nr, f, crate::data::Deps::none())
                        } else {
                            self.lexer.closing_angle();
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "spatial<T[x, y]> needs coordinate key fields, e.g. spatial<Mob[x, y]>"
                            );
                            Type::Unknown(0)
                        }
                    }
                    "reference" => {
                        self.lexer.closing_angle();
                        self.data.set_referenced(sub_nr, on_d, Value::Null);
                        // #328: in struct-field position `reference<T>` is the
                        // documented POINTER — the `u16::MAX` dep is the
                        // auto-Reference share marker that selects the 12-byte
                        // `Parts::DbRef` layout in `fill_database` and the
                        // OpGetDbRef / OpSetDbRef read/write arms.  Without it
                        // the parse erased the pointer-ness: the field laid
                        // out as INLINE `T` bytes (a silent deep copy on
                        // write), `next: null` construction panicked on the
                        // unpositioned-field marker, and `reference<Self>`
                        // could not exist at all.  Non-field positions
                        // (locals, parameters, return types) keep the plain
                        // shape — their semantics are unchanged by #328.
                        if on_d != u32::MAX
                            && matches!(
                                self.data.def_type(on_d),
                                DefType::Struct | DefType::EnumValue
                            )
                        {
                            Type::Reference(sub_nr, crate::data::Deps::pointer_marker())
                        } else {
                            Type::Reference(sub_nr, crate::data::Deps::none())
                        }
                    }
                    "iterator" => {
                        // CO1.3c: comma and second type are optional for generators.
                        // iterator<T> = generator yield type; iterator<T, I> = collection iterator.
                        let mut it_tp = Type::Null;
                        if self.lexer.has_token(",") {
                            if let Some(iter) = self.lexer.has_identifier() {
                                if let Some(it) = self.parse_type(on_d, &iter, false) {
                                    self.data.set_referenced(sub_nr, on_d, Value::Null);
                                    it_tp = it;
                                } else {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "Expect an iterator type"
                                    );
                                }
                            } else {
                                diagnostic!(self.lexer, Level::Error, "Expect an iterator type");
                            }
                        }
                        self.lexer.closing_angle();
                        Type::Iterator(Box::new(tp), Box::new(it_tp))
                    }
                    _ => {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Subtype only allowed on structures"
                        );
                        Type::Unknown(0)
                    }
                });
            }
            assert!(self.first_pass, "Incorrect handling of unknown types");
        } else {
            self.lexer.revert(link);
        }
        None
    }

    // <depend> ::= '[' { <field> [ ',' ] } ']'
    pub(crate) fn parse_depended(&mut self, returned: bool, dep: &mut Vec<u16>) {
        if self.default && returned && self.lexer.has_token("[") && self.context != u32::MAX {
            loop {
                if let Some(id) = self.lexer.has_identifier() {
                    if let Some(nr) = self.data.def(self.context).attr_names.get(&id) {
                        dep.push(*nr as u16);
                    } else {
                        diagnostic!(self.lexer, Level::Error, "Unknown field name '{id}'");
                    }
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expected a field name");
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("]");
        }
    }

    pub(crate) fn parse_fields(&mut self, directions: bool, result: &mut Vec<(String, bool)>) {
        self.lexer.token("[");
        loop {
            let desc = self.lexer.has_token("-");
            if !directions && desc {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Structure doesn't support descending fields"
                );
            }
            if let Some(field) = self.lexer.has_identifier() {
                result.push((field, !desc));
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("]");
        self.lexer.closing_angle();
    }

    // <field_limit> ::= 'limit' '(' [ '-' ] <min-integer> ',' [ '-' ] <max-integer> ')'
    pub(crate) fn parse_type_limit(&mut self, min: &mut i32, max: &mut u32) -> bool {
        if self.lexer.has_keyword("limit") {
            self.lexer.token("(");
            let min_neg = self.lexer.has_token("-");
            if let Some(nr) = self.lexer.has_integer() {
                *min = if min_neg { -(nr as i32) } else { nr as i32 };
            }
            self.lexer.token(",");
            // C54.A incremental 2a — accept both Integer and Long literals.
            // Values > i32::MAX now tokenise as Long (so u32-range bounds
            // like `limit(0, 4_294_967_294)` work).  Truncate to u32
            // (current `max: u32` param); future phases can widen to i64
            // if signed-bound support for > i32 ranges is needed.
            if let Some(nr) = self.lexer.has_integer() {
                *max = nr;
            } else if let Some(nr) = self.lexer.has_long() {
                *max = nr as u32;
            }
            self.lexer.token(")");
            true
        } else {
            false
        }
    }

    // <struct> = 'struct' <identifier> [ ':' <type> ] '{' <param-id> ':' <field> { ',' <param-id> ':' <field> } '}'
    /// @PLN86 P6.1 — a `capability <dotted.name>` top-level declaration.  Registers
    /// a namespaced capability group that functions + data members link to via the
    /// `group#right` notation (P6.2 / P6.4); the dotted name IS the namespace (matched
    /// hierarchically on the grant side, like the existing `fs.read` groups).  A
    /// capability has no code or type — it is a pure annotation target — so this only
    /// records the name; a `group#right` link to an UNDECLARED group is a load error,
    /// validated at admission once every declaration is registered.  `capability` is a
    /// contextual keyword (not reserved), so it never shadows an identifier.
    pub(crate) fn parse_capability(&mut self) -> bool {
        if !self.lexer.has_keyword("capability") {
            return false;
        }
        let Some(mut name) = self.lexer.has_identifier() else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect a capability name after `capability`"
            );
            return true;
        };
        // A dotted name (`cmd.move`) is the namespace, like the `fs.read` groups.
        while self.lexer.has_token(".") {
            let Some(seg) = self.lexer.has_identifier() else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect an identifier after `.` in a capability name"
                );
                return true;
            };
            name.push('.');
            name.push_str(&seg);
        }
        self.declared_capabilities.insert(name);
        true
    }

    /// @PLN86 P6.2 — try to parse a `group#right` capability-link token in a position
    /// where one is OPTIONAL: a dotted group, `#`, then one of `read`/`update`/`append`.
    /// Returns the canonical `"group#right"` string; returns `None` **silently** when
    /// no link is present (the next token is not an identifier — e.g. the `;`/`{`
    /// terminator after a signature, or a field separator); errors only on a *malformed*
    /// link (a group with no `#right`).  The group's existence as a declared `capability`
    /// is validated at admission, so forward + cross-file declarations resolve.  Shared
    /// by the function call gate (P6.2, in the signature) and a struct-field link (P6.4).
    pub(crate) fn try_cap_link(&mut self) -> Option<String> {
        // NON-DESTRUCTIVE: in this optional position the next token might instead be
        // a separator, a default `=`, or a field-modifier keyword (`not`, `assert`)
        // — all of which lex as ordinary tokens/identifiers.  So consume nothing
        // unless this is really a link: save the cursor and revert on a miss.  Only
        // a real `#` followed by a bad right is a hard error.
        let saved = self.lexer.link();
        let Some(mut group) = self.lexer.has_identifier() else {
            return None; // no identifier consumed
        };
        while self.lexer.has_token(".") {
            let Some(seg) = self.lexer.has_identifier() else {
                self.lexer.revert(saved);
                return None;
            };
            group.push('.');
            group.push_str(&seg);
        }
        if !self.lexer.has_token("#") {
            // not a link (e.g. `not null`, `assert(...)`, the next field) — un-consume.
            self.lexer.revert(saved);
            return None;
        }
        let Some(right) = self.lexer.has_identifier() else {
            self.lexer.revert(saved);
            return None;
        };
        if crate::sandbox::Right::parse(&right).is_none() {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown capability right `{right}` — expected read, update, or append"
            );
            return None;
        }
        Some(format!("{group}#{right}"))
    }

    /// @PLN86 P6.4 — consume zero-or-more `group#right` capability links after a
    /// struct field's type (`health: int stats#read stats#update`), recording each
    /// on `(struct, field)` for the admission walk.  Consumed every pass, recorded
    /// once (first pass) so the per-field link list does not double on re-parse.
    pub(crate) fn parse_field_links(&mut self, d_nr: u32, a_name: &str) {
        while let Some(token) = self.try_cap_link() {
            if self.first_pass {
                self.record_member_link(d_nr, a_name, token);
            }
        }
    }

    // @F12 — struct records (fields, `= default`, `computed`, `limit`/`not null`/`assert`)
    pub(crate) fn parse_struct(&mut self) -> bool {
        // @PLN101 — optional `value` modifier: `value struct T {…}` marks T a value (copy,
        // inline, non-null) type. `value` is a plain IDENTIFIER (not a keyword), so peek it
        // (`has_token` only matches Token lexemes) and consume only the `value struct` prefix.
        let is_value = matches!(
            self.lexer.peek().has,
            crate::lexer::LexItem::Identifier(ref t) if t == "value"
        );
        if is_value {
            self.lexer.cont();
        }
        if !self.lexer.has_token("struct") {
            if is_value {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`value` must be followed by `struct`"
                );
                return true;
            }
            return false;
        }
        let Some(id) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect attribute");
            return true;
        };
        let mut d_nr = self.data.def_nr(&id);
        // @PLN22 Phase 2 — shadow a prelude/import struct of the same key.  This
        // includes the stdlib's generic type-var marker (`<T>`): a user `struct T`
        // shadows it just like the enum path does, so `T` is a usable type name.
        // (A SAME-SOURCE clash — the user's own `struct T` plus their own `fn foo<T>`
        // — keeps `prelude_shadowed` false, so it still hits the "reserved" arm below.)
        if self.prelude_shadowed(&id) {
            d_nr = u32::MAX;
        }
        if d_nr == u32::MAX {
            d_nr = self.data.add_def(&id, self.lexer.pos(), DefType::Struct);
            self.data.definitions[d_nr as usize].returned =
                Type::Reference(d_nr, crate::data::Deps::none());
        } else if self.first_pass {
            // fix-tvscope: a SAME-SOURCE type-var placeholder (the user's own
            // `fn foo<T>` before their `struct T`) blocks the struct — a genuine
            // clash worth the dedicated diagnostic rather than the confusing
            // "Redefined struct".  (A cross-source stdlib `<T>` was shadowed above.)
            if self.data.is_type_var_placeholder(d_nr) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}' is reserved as a generic type variable — choose a different struct name",
                    id
                );
            } else if self.data.def_type(d_nr) == DefType::Unknown
                && matches!(
                    self.data.definitions[d_nr as usize].returned,
                    Type::Unknown(_)
                )
            {
                // Adopt a genuine same-file forward-reference placeholder (an
                // `add_def(.., DefType::Unknown)` stub left by a use-before-def).
                // A reserved builtin type-keyword forward-declared in the stdlib
                // (e.g. `type iterator;`) is `DefType::Type` with an Unknown
                // returned type — it must NOT be adopted, or `struct iterator`
                // would silently shadow the builtin.  Without the def_type guard
                // it fell through here; now it lands in the conflict arm below.
                self.data.definitions[d_nr as usize].position = self.lexer.pos().clone();
                self.data.definitions[d_nr as usize].def_type = DefType::Struct;
                self.data.definitions[d_nr as usize].returned =
                    Type::Reference(d_nr, crate::data::Deps::none());
            } else {
                let prev_pos = self.data.def(d_nr).position().clone();
                let prev_kind = format!("{:?}", self.data.def(d_nr).def_type()).to_lowercase();
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "struct '{id}' conflicts with a {prev_kind} of the same name \
                     already defined at {prev_pos} — pick a different name"
                );
            }
        }
        let context = self.context;
        self.context = d_nr;
        // @PLN101 — mark the value-struct kind now that d_nr is the confirmed struct def.
        if is_value {
            self.data.value_structs.insert(d_nr);
        }
        self.lexer.token("{");
        // #91: collect init field dependency info for circular detection.
        let mut init_deps: Vec<(String, Vec<String>)> = Vec::new();
        loop {
            self.lexer.has_token("pub");
            // @P386: `const` struct fields are a planned feature (@PLAN33), not yet
            // supported.  Reject with ONE clear diagnostic and consume the keyword
            // so the field still parses as `name: type` — without this, `const` is
            // read as the field NAME and the real field cascades into 4 errors.
            if self.lexer.has_keyword("const") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "const struct fields are not yet supported (planned — @PLAN33); \
                     remove `const` for now"
                );
            }
            let Some(a_name) = self.lexer.has_identifier() else {
                diagnostic!(self.lexer, Level::Error, "Expect attribute");
                self.context = context;
                return true;
            };
            if self.first_pass && self.data.attr(d_nr, &a_name) != usize::MAX {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "field `{}` is already declared",
                    a_name
                );
            }
            self.lexer.token(":");
            self.init_field_deps.clear();
            self.parse_field(d_nr, &a_name);
            if !self.init_field_deps.is_empty() {
                init_deps.push((a_name.clone(), self.init_field_deps.clone()));
            }
            if !self.lexer.has_token(",") || self.lexer.peek_token("}") {
                break;
            }
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        self.link_shared_nullable_hash(d_nr);
        // #91: check for circular init dependencies (second pass, all fields known).
        if !self.first_pass {
            self.check_circular_init(&init_deps);
        }
        self.context = context;
        true
    }

    /// @PLN25 Scope B — a keyed HASH field that shares its record set with a sibling NULLABLE
    /// vector (the `other_indexes` "two views, one record set" pattern, e.g.
    /// `struct Db { entries: vector<S>, lookup: hash<S[k]> }`) must index the `Some`-wrapped
    /// records, not dense `S`.  Rewrite such a hash's element from `S` to the sibling's
    /// `__nullable<S>` enum so the parser type, db storage, lookup type-id, and field-access all
    /// agree on ONE type: the db link then matches by content, `determine_keys` bakes the key at
    /// the payload offset, and `c.lookup[k].field` unwraps via the `Some` payload sub-ref — all
    /// reusing the kept nullable machinery.  Gate-OFF-inert: a dense vector sibling's element is
    /// `Reference(S)` (not the `Enum`), so nothing matches.  (Sorted/Index sharing is left dense —
    /// no consumer exercises it and it needs the index bookkeeping on the `Some` variant.)
    fn link_shared_nullable_hash(&mut self, d_nr: u32) {
        let n = self.data.definitions[d_nr as usize].attributes.len();
        // payload struct `S` -> its `__nullable<S>` enum, gathered from nullable vector siblings.
        let mut nullable_of: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for a in 0..n {
            if let Type::Vector(inner, _) = self.data.attr_type(d_nr, a)
                && let Type::Enum(nd, true, _) = *inner
                // ONLY the synth `__nullable<S>` enum — not an arbitrary user struct-enum
                // element (`vector<Shape>`), whose `variant_of(.., "Some")` would be MAX.
                && self.data.def(nd).name().starts_with("__nullable<")
            {
                let some = self.data.variant_of(nd, "Some");
                let pa = self.data.attr(some, "payload");
                if pa != usize::MAX
                    && let Type::Reference(s, _) = self.data.attr_type(some, pa)
                {
                    nullable_of.insert(s, nd);
                }
            }
        }
        if nullable_of.is_empty() {
            return;
        }
        for a in 0..n {
            if let Type::Hash(h_elem, keys, deps) = self.data.attr_type(d_nr, a)
                && let Some(&nd) = nullable_of.get(&h_elem)
            {
                self.data.definitions[d_nr as usize].attributes[a].typedef =
                    Type::Hash(nd, keys, deps);
            }
        }
    }

    /// I3: parse an `interface` declaration and register it as `DefType::Interface`.
    ///
    /// Syntax: `interface Name { fn method(params) -> type [;] ... }`
    ///
    /// Method signatures are parsed for syntactic correctness (param/return types
    /// resolved against the current scope).  `Self` is a placeholder type that
    /// refers to the concrete satisfying type at instantiation (I6).
    ///
    /// This first-pass implementation registers the interface definition and
    /// verifies syntax; semantic satisfaction checking comes in I5/I6.
    #[allow(clippy::too_many_lines)]
    // @F26 — interfaces & bounded generics (<T: A + B>, operator interfaces)
    pub(crate) fn parse_interface(&mut self) -> bool {
        if !self.lexer.has_token("interface") {
            return false;
        }
        let Some(id) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect interface name");
            return true;
        };
        if !is_camel(&id) && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Interface name '{}' must be CamelCase",
                id
            );
        }
        // Register or locate the interface definition.
        let mut d_nr = self.data.def_nr(&id);
        if d_nr == u32::MAX {
            if self.first_pass {
                d_nr = self.data.add_def(&id, self.lexer.pos(), DefType::Interface);
            }
        } else if self.first_pass {
            diagnostic!(self.lexer, Level::Error, "Cannot redefine interface '{id}'");
        }
        // I3: register 'Self' as a type placeholder for method signature parsing.
        // 'Self' resolves to its own definition (like a generic type variable) so
        // that parse_type_full succeeds.  I6 substitutes the concrete satisfying type.
        if self.first_pass && self.data.def_nr("Self") == u32::MAX {
            let self_nr = self.data.add_def("Self", self.lexer.pos(), DefType::Struct);
            self.data
                .set_returned(self_nr, Type::Reference(self_nr, crate::data::Deps::none()));
        }
        let context = self.context;
        if d_nr != u32::MAX {
            self.context = d_nr;
        }
        if !self.lexer.token("{") {
            self.context = context;
            return true;
        }
        // Parse zero or more method/operator signatures.
        while !self.lexer.peek_token("}") {
            if self.lexer.peek().has == crate::lexer::LexItem::None {
                break;
            }
            // I3.1: `op <token> (params) -> type` desugars to an `OpCamelCase` method stub.
            let method_name = if self.lexer.has_keyword("op") {
                if let crate::lexer::LexItem::Token(tok) = self.lexer.peek().has.clone() {
                    self.lexer.cont();
                    format!("Op{}", rename(&tok))
                } else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Expected operator symbol after 'op' in interface body"
                        );
                    }
                    self.lexer.cont();
                    continue;
                }
            } else {
                if !self.lexer.has_token("fn") {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "Expected 'fn' in interface body");
                    }
                    self.lexer.cont();
                    continue;
                }
                let Some(name) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Expected method name in interface"
                        );
                    }
                    break;
                };
                name
            };
            let mut args = Vec::new();
            if self.lexer.token("(") {
                self.parse_arguments(&method_name, &mut args);
                self.lexer.token(")");
            }
            let return_tp = if self.lexer.has_token("->") {
                self.parse_type_full(d_nr, true)
            } else {
                None
            };
            // I6/I9-stub: register method stubs as children of the interface.
            // Use interface-scoped names (`__iface_{d_nr}_{method}`) to avoid
            // collision when multiple interfaces declare the same operator.
            // `children_of(d_nr)` enumerates them for satisfaction checking;
            // T-stub creation strips the prefix to extract the method name.
            if self.first_pass && d_nr != u32::MAX {
                let stub_name = format!("__iface_{d_nr}_{method_name}");
                if self.data.def_nr(&stub_name) == u32::MAX {
                    let stub_nr =
                        self.data
                            .add_def(&stub_name, self.lexer.pos(), DefType::Function);
                    for a in &args {
                        self.data.add_attribute(
                            &mut self.lexer,
                            stub_nr,
                            &a.name,
                            a.typedef.clone(),
                        );
                    }
                    self.data.definitions[stub_nr as usize].parent = d_nr;
                    if let Some(ref rt) = return_tp {
                        self.data.set_returned(stub_nr, rt.clone());
                    }
                }
            }
            // I5 (phase 1): factory methods (Self in return without self: Self first param)
            // are not yet supported.  Emit a clear diagnostic rather than silently producing
            // wrong code when I6 lands.
            if !self.first_pass {
                let self_nr = self.data.def_nr("Self");
                if self_nr != u32::MAX
                    && let Some(Type::Reference(ret_nr, _)) = &return_tp
                    && *ret_nr == self_nr
                {
                    let has_self_param = args.first().is_some_and(|a| {
                        a.name == "self"
                            && matches!(&a.typedef, Type::Reference(nr, _) if *nr == self_nr)
                    });
                    if !has_self_param {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "factory methods not yet supported: '{}' returns Self without a 'self: Self' parameter",
                            method_name
                        );
                    }
                }
            }
            self.lexer.has_token(";");
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        self.context = context;
        true
    }

    /// #91: DFS cycle detection on init field dependencies.
    fn check_circular_init(&mut self, init_deps: &[(String, Vec<String>)]) {
        let names: HashSet<String> = init_deps.iter().map(|(n, _)| n.clone()).collect();
        for (start, deps) in init_deps {
            let mut visited: Vec<String> = vec![start.clone()];
            let mut stack = deps.clone();
            while let Some(dep) = stack.pop() {
                if dep == *start {
                    visited.push(start.clone());
                    let path = visited.join(" -> ");
                    diagnostic!(self.lexer, Level::Error, "circular init dependency: {path}");
                    break;
                }
                if names.contains(&dep) && !visited.contains(&dep) {
                    visited.push(dep.clone());
                    if let Some((_, subdeps)) = init_deps.iter().find(|(n, _)| *n == dep) {
                        stack.extend(subdeps.clone());
                    }
                }
            }
        }
    }

    // <field> ::= { <field_limit> | 'not' 'null' | <field_default> | 'check' '(' <expr> ')' | <type-id> [ '[' ['-'] <field> { ',' ['-'] <field> } ']' ] } }
    #[allow(clippy::too_many_lines)] // pre-existing length; T1.11a added one branch
    pub(crate) fn parse_field(&mut self, d_nr: u32, a_name: &String) {
        let mut a_type: Type = Type::Unknown(0);
        let mut defined = false;
        let mut value = Value::Null;
        let mut check = Value::Null;
        let mut check_message = Value::Null;
        let mut nullable = true;
        let mut is_computed = false;
        let mut is_init = false;
        // Post-2c: remember the integer alias name the user typed (e.g. `i32`)
        // so `fill_database` / codegen can consult `forced_size(alias)` even
        // though the resolved Type::Integer collapses the alias info.
        let mut alias_d_nr: u32 = u32::MAX;
        loop {
            // @PLN25 F2 — `not null` is RETIRED but still ACCEPTED as a no-op (a scalar field
            // is non-null by DEFAULT now; `is_optional` below sets the attribute non-null and
            // the `not_null` flag is stamped for the range). `has_deprecated_not_null` consumes
            // it and emits the deprecation warning; the hard "retired" error stays blocked on
            // the registry republish (RESUME.md § F2 task #4).
            if self.has_deprecated_not_null() {
                nullable = false;
            }
            {
                let (comp, init) =
                    self.parse_field_default(&mut value, &mut a_type, d_nr, a_name, &mut defined);
                is_computed |= comp;
                is_init |= init;
            }
            if self.lexer.has_token("assert") {
                // assert(condition) or assert(condition, message) on struct fields.
                self.lexer.token("(");
                self.expression(&mut check);
                if self.lexer.has_token(",") {
                    self.expression(&mut check_message);
                }
                self.lexer.token(")");
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "CHECK" {
                    // Legacy CHECK syntax — parse and discard for backward compat
                    self.lexer.token("(");
                    let mut p = Value::Null;
                    self.expression(&mut p);
                    if self.lexer.has_token(",") {
                        let mut q = Value::Null;
                        self.expression(&mut q);
                    }
                    self.lexer.token(")");
                } else if let Some(tp) = self.parse_type(d_nr, &id, false) {
                    defined = true;
                    // If the type carries a not-null flag (e.g. integer not null),
                    // propagate it to the field's nullable flag so is_null and
                    // redundant-null-check warnings work correctly.
                    if let Type::Integer(IntegerSpec { not_null: true, .. }) = &tp {
                        nullable = false;
                    }
                    // Capture the alias def_nr for size(N) routing.  Only
                    // real aliases (i32, u8, etc.) — "integer" is the base type
                    // and its forced_size is 8, which would override the narrow
                    // limit()-based heuristic for `integer limit(0, 255)`.
                    // @PLN25: peel `Optional(τ)` so a nullable narrow field (`u8?`)
                    // captures its alias and stores at the narrow width like `u8`.
                    if matches!(tp.base(), Type::Integer(_)) && id != "integer" {
                        alias_d_nr = self.data.def_nr(&id);
                    }
                    a_type = tp;
                    // '= expr' shorthand for a field default value
                    if self.lexer.has_token("=") {
                        // #91: enable dep tracking so $.field accesses are recorded
                        // for circular-init detection (same as init(expr) path).
                        self.init_field_tracking = true;
                        self.init_field_deps.clear();
                        // @PLN22 Phase 1 — hint the field's enum so a bare variant
                        // default (`level: Level = Warning`) resolves against the
                        // declared field type.
                        if self.enum_context(&a_type) {
                            self.expected = a_type.clone();
                        }
                        let tp = self.expression(&mut value);
                        self.expected = Type::Unknown(0);
                        self.init_field_tracking = false;
                        if a_type.is_unknown() {
                            a_type = tp;
                        }
                    }
                    // @PLN86 P6.4 — links after a scalar/named field type.
                    self.parse_field_links(d_nr, a_name);
                }
            } else if let Some(tp) = self.parse_type_full(d_nr, false) {
                // Plan-06 phase 4d: tuple-typed struct fields are now
                // accepted.  Storage layout uses the synthetic
                // `__tuple<…>` struct's positions (registered via
                // `tuple_def`); set/get codegen routes element access
                // through the same OpInt variants used for ordinary
                // struct fields.  See `parser/mod.rs::set_field_check`
                // and `get_val` for the per-element write/read paths.
                defined = true;
                a_type = tp;
                // @PLN86 P6.4 — links after a vector/generic/tuple field type.
                self.parse_field_links(d_nr, a_name);
                break;
            } else {
                break;
            }
        }
        if !defined {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Attribute {a_name} needs type or definition"
            );
        }
        if self.first_pass {
            // @PLN25 F2: a scalar field's nullability is carried by its `Optional` wrapper (DN1's
            // single source of truth), NOT the pre-DN1 parser default (`nullable = true`) or the
            // `not null` keyword.  So a plain scalar field is NON-null (an Integer field also gets
            // the FULL range — no reserved sentinel; only a `u8?` field reserves the top value),
            // and `not null` is redundant on EVERY scalar field — the prerequisite for retiring
            // it.  Now covers all scalars (Integer/Text/Boolean/Float/Single/Character): the
            // attribute flag was the last place a plain non-Integer scalar field still read as
            // nullable (inconsistently — `(N-Store)` already rejected a `null` store into it).
            if crate::keys::pln25_f2_enabled() && Self::is_non_null_scalar(a_type.base()) {
                nullable = matches!(a_type, Type::Optional(_));
            }
            // H6: stamp the field's nullability onto its integer spec so the
            // STORED attribute type is self-describing.  The alias path
            // (`u8 not null`) otherwise leaves `not_null` at the alias default
            // (`false`), and the literal range-check (`int_value_fits`) reads the
            // usable bounds off `not_null` — so a NOT-NULL `u8` field must report
            // `not_null:true` or its `255` would be wrongly rejected.  `nullable`
            // here is exactly what `set_attr_nullable` records, so the spec flag
            // and the attribute flag cannot disagree.
            if let Type::Integer(ref mut spec) = a_type {
                spec.not_null = !nullable;
            }
            let a = self
                .data
                .add_attribute(&mut self.lexer, d_nr, a_name, a_type);
            self.data.set_attr_nullable(d_nr, a, nullable);
            self.data.set_attr_value(d_nr, a, value);
            if alias_d_nr != u32::MAX {
                self.data.definitions[d_nr as usize].attributes[a].alias_d_nr = alias_d_nr;
            }
            if is_computed {
                self.data.definitions[d_nr as usize].attributes[a].constant = true;
            }
            if is_init {
                self.data.definitions[d_nr as usize].attributes[a].init = true;
            }
            if check != Value::Null {
                self.data.definitions[d_nr as usize].attributes[a].check = check;
                self.data.definitions[d_nr as usize].attributes[a].check_message = check_message;
            }
        } else {
            let a = self.data.attr(d_nr, a_name);
            if is_computed {
                self.data.definitions[d_nr as usize].attributes[a].constant = true;
            }
            if is_init {
                self.data.definitions[d_nr as usize].attributes[a].init = true;
            }
            if value != Value::Null {
                self.data.set_attr_value(d_nr, a, value);
            }
            if check != Value::Null {
                self.data.definitions[d_nr as usize].attributes[a].check = check;
                self.data.definitions[d_nr as usize].attributes[a].check_message = check_message;
            }
        }
    }

    // <field_default> ::= 'virtual' <value-expr> | 'init' '(' <value-expr> ')'
    //                   | 'default' '(' <value-expr> ')'
    // Returns (is_computed, is_init).
    pub(crate) fn parse_field_default(
        &mut self,
        value: &mut Value,
        a_type: &mut Type,
        _d_nr: u32,
        _a_name: &String,
        defined: &mut bool,
    ) -> (bool, bool) {
        let mut is_computed = false;
        let mut is_init = false;
        if self.lexer.has_keyword("computed") || self.lexer.has_keyword("virtual") {
            is_computed = true;
            // Computed field: calculate on every access, no store space.
            self.lexer.token("(");
            let tp = self.expression(value);
            if a_type.is_unknown() {
                *a_type = tp;
                *defined = true;
            } else {
                self.convert(value, &tp, a_type);
            }
            self.lexer.token(")");
        }
        if self.lexer.has_keyword("init") {
            is_init = true;
            // L7: init(expr) — stored at creation, writable after. $ allowed.
            // #91: enable dep tracking for circular-init detection.
            self.init_field_tracking = true;
            self.init_field_deps.clear();
            self.lexer.token("(");
            let tp = self.expression(value);
            if a_type.is_unknown() {
                *a_type = tp;
                *defined = true;
            } else {
                self.convert(value, &tp, a_type);
            }
            self.lexer.token(")");
            self.init_field_tracking = false;
        }
        if self.lexer.has_keyword("default") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "default(expr) is removed; use 'computed(expr)' for calculated fields or '= expr' for stored defaults"
            );
            self.lexer.token("(");
            self.expression(value);
            self.lexer.token(")");
        }
        (is_computed, is_init)
    }
}
