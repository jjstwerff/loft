// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    DefType, HashSet, I32, IntegerSpec, Level, LexItem, LexResult, Mode, OUTPUT_DEFAULT,
    OutputState, Parser, Position, SKIP_TOKEN, SKIP_WIDTH, ToString, Type, Value,
    diagnostic_format, to_default, v_block, v_if, v_set,
};

// Variable resolution, struct construction, and object parsing.

impl Parser {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_var(
        &mut self,
        code: &mut Value,
        name: &str,
        parent_tp: &mut Type,
        name_pos: &Position,
    ) -> Type {
        // '$' refers to the current record in struct field default expressions
        if name == "$" && matches!(self.data.def_type(self.context), DefType::Struct) {
            *code = Value::Var(0);
            return Type::Reference(self.context, Vec::new());
        }
        let mut source = u16::MAX;
        let qualified = self.lexer.has_token("::");
        let nm = if qualified {
            source = self.data.get_source(name);
            if let Some(id) = self.lexer.has_identifier() {
                id
            } else {
                diagnostic!(self.lexer, Level::Error, "Expecting identifier after ::");
                name.to_string()
            }
        } else {
            name.to_string()
        };
        // Tier-0 auto-`use`: a qualified `name::…` whose lowercase `name` is
        // neither a loaded library (`source` == MAX) nor a known definition is
        // an unknown library.  After the use-region's pre-scan, any *available*
        // library would already be loaded, so this is the genuine "no such
        // library" case — report it directly instead of the downstream "Unknown
        // function" (reported once types are known, in the second pass).
        if qualified
            && source == u16::MAX
            && !self.first_pass
            && self.data.def_nr(name) == u32::MAX
            && name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        {
            diagnostic!(self.lexer, Level::Error, "Unknown library '{name}'");
            // Consume a trailing call so the parser does not also choke on the
            // arguments and emit a second, less-helpful error.
            if self.lexer.has_token("(") && !self.lexer.has_token(")") {
                loop {
                    let mut arg = Value::Null;
                    self.expression(&mut arg);
                    if !self.lexer.has_token(",") {
                        self.lexer.has_token(")");
                        break;
                    }
                }
            }
            return Type::Unknown(0);
        }
        // vector<T>.parse(text) — parse a JSON array into a vector of T.
        if nm == "vector" && self.lexer.has_token("<") {
            if let Some(elem_name) = self.lexer.has_identifier() {
                let elem_d_nr = self.data.def_nr(&elem_name);
                self.lexer.token(">");
                if self.lexer.has_token(".") && self.lexer.has_keyword("parse") {
                    if elem_d_nr != u32::MAX {
                        return self.parse_vector_parse(elem_d_nr, code);
                    }
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown type '{elem_name}' in vector<{elem_name}>.parse()"
                        );
                    }
                    return Type::Unknown(0);
                }
            }
            // Not a vector<T>.parse() — cannot recover tokens, report error.
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expected '.parse(' after vector<T>"
                );
            }
            return Type::Unknown(0);
        }
        let mut t = self.parse_constant_value(code, source, &nm, name_pos);
        if t != Type::Null {
            return t;
        }
        if self.lexer.has_token("(") {
            if name == "sizeof" {
                t = self.parse_size(code);
            } else if name == "type_name" {
                t = self.parse_type_name(code);
            } else if name == "typedef" {
                let mut p = Value::Null;
                let et = self.expression(&mut p);
                self.lexer.token(")");
                let tp = self.data.def(self.data.type_def_nr(&et)).known_type();
                t = Type::Integer(IntegerSpec {
                    min: 0,
                    max: 65536,
                    not_null: false,
                    forced_size: None,
                });
                *code = Value::Int(i32::from(tp));
            } else {
                t = self.parse_call(code, source, &nm);
            }
        } else if self.closure_param != u16::MAX
            && !self.first_pass
            && self.data.def(self.context).closure_record() != u32::MAX
            && self
                .data
                .attr(self.data.def(self.context).closure_record(), name)
                != usize::MAX
        {
            // A5.3/A5.4: redirect captured variable reads to closure record field.
            let closure_d_nr = self.data.def(self.context).closure_record();
            let fnr = self.data.attr(closure_d_nr, name);
            *code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
            t = self.data.attr_type(closure_d_nr, fnr);
            // closure record is a struct — add __closure as dep so the
            // store allocation stays alive while derived text/references are in use.
            t = t.depending(self.closure_param);
        } else if self.vars.name_exists(name) {
            let index_var = self.vars.var(name);
            // on pass 2, if a variable has Unknown type, it may be a pass-1
            // placeholder for a forward-declared function. Try fn-ref resolution.
            if !self.first_pass && self.vars.tp(index_var).is_unknown() {
                let prefixed = format!("n_{nm}");
                let fn_d_nr = self.data.def_nr(&prefixed);
                if fn_d_nr != u32::MAX && matches!(self.data.def_type(fn_d_nr), DefType::Function) {
                    // Suppress "never read" warning on the pass-1 placeholder.
                    self.var_usages(index_var, true);
                    *code = Value::Int(fn_d_nr as i32);
                    self.data.def_used(fn_d_nr);
                    let n_args = self.data.attributes(fn_d_nr);
                    let arg_types: Vec<Type> = (0..n_args)
                        .map(|a| self.data.attr_type(fn_d_nr, a))
                        .collect();
                    let ret_type = self.data.def(fn_d_nr).returned().clone();
                    return Type::Function(arg_types, Box::new(ret_type), vec![]);
                }
            }
            if self.lexer.has_token("#") {
                self.var_usages(index_var, true);
                if self.lexer.has_keyword("errors") {
                    // s#errors — return the parse errors from the last Type.parse() call.
                    let fn_nr = self.data.def_nr("i_parse_errors");
                    if fn_nr != u32::MAX {
                        *code = Value::Call(fn_nr, vec![]);
                        t = Type::Text(Vec::new());
                    }
                    return t;
                }
                self.iter_op(code, name, &mut t, index_var);
            } else if let Value::Var(into) = code {
                let v_nr = self.vars.var(name);
                if matches!(self.vars.tp(v_nr), Type::Text(_)) {
                    t = self.vars.tp(v_nr).clone();
                } else {
                    t = self.vars.tp(v_nr).depending(v_nr);
                }
                self.var_usages(v_nr, true);
                if let Type::Reference(d_nr, _) = self.vars.tp(*into)
                    && let Type::Reference(vd_nr, _) = self.vars.tp(v_nr)
                    && d_nr == vd_nr
                {
                    // Don't create OpCopyRecord here: generate_set handles the copy when
                    // value=Var(src). Using Var(v_nr) directly lets method calls like
                    // `d = c.double()` pass c as `self` without the broken CopyRecord-as-self
                    // pattern that was causing garbage store_nr crashes (Issue 1).
                    let d_nr = *d_nr;
                    let into_var = *into;
                    self.vars.make_independent(into_var, v_nr);
                    *code = Value::Var(v_nr);
                    return Type::Reference(d_nr, Vec::new());
                }
                *code = Value::Var(v_nr);
            } else {
                let v_nr = self.vars.var(name);
                t = self.vars.tp(v_nr).depending(v_nr);
                self.var_usages(v_nr, true);
                *code = Value::Var(v_nr);
            }
        } else if let Some((_cname, ctype)) = self
            .capture_context
            .iter()
            .find(|(n, _)| n == name)
            .cloned()
        {
            // P257 (2026-05-12): reject collection-typed captures with a
            // clean parse-time diagnostic.  The closure-record layout
            // (16-byte fn-ref slot: 4B d_nr + 12B closure DbRef) holds
            // the captured payload as a flat list of attributes; vectors
            // and other keyed collections need an additional level of
            // indirection (their content type) that the closure record
            // doesn't currently model.  Without this rejection the
            // failure mode is unstable: interp panics with `Write to
            // locked store` (the closure record write trips the
            // collection's internal lock), native rejects with rustc
            // E0308 + E0605 (the generated code casts a tuple-shaped
            // value as i32).  The bind-the-element-before-the-lambda
            // workaround applies for any value the closure body
            // actually needs.
            if matches!(
                ctype,
                Type::Vector(_, _)
                    | Type::Hash(_, _, _)
                    | Type::Sorted(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Spacial(_, _, _)
            ) {
                let kind = match &ctype {
                    Type::Vector(_, _) => "vector",
                    Type::Hash(_, _, _) => "hash",
                    Type::Sorted(_, _, _) => "sorted",
                    Type::Index(_, _, _) => "index",
                    Type::Spacial(_, _, _) => "spacial",
                    _ => "collection",
                };
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{kind} variable '{name}' cannot be captured into a closure body; bind the element you need before the lambda (e.g. `x = {name}[i]; f = fn(...) {{ ... x ... }}`) — collection capture is not supported because the closure record layout doesn't model the content type"
                );
                t = ctype.clone();
                *code = Value::Null;
                return t;
            }
            // record the capture for closure record synthesis.
            if !self.captured_names.iter().any(|(n, _)| n == name) {
                self.captured_names.push((name.to_string(), ctype.clone()));
            }
            // if we have a closure parameter (second pass), emit field read
            // from the closure record.  Otherwise create a placeholder variable.
            // if we have a closure parameter (second pass), emit field read.
            let closure_d_nr = if self.closure_param == u16::MAX || self.first_pass {
                u32::MAX
            } else {
                self.data.def(self.context).closure_record()
            };
            let fnr = if closure_d_nr == u32::MAX {
                usize::MAX
            } else {
                self.data.attr(closure_d_nr, name)
            };
            if fnr == usize::MAX {
                // First pass, no closure param, or field not found — placeholder variable.
                let v_nr = self.create_var(name, &ctype);
                self.var_usages(v_nr, true);
                t = ctype;
                *code = Value::Var(v_nr);
            } else {
                *code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
                t = self.data.attr_type(closure_d_nr, fnr);
                // closure record is a struct — add __closure as dep.
                t = t.depending(self.closure_param);
            }
        } else if self.data.def_nr(name) != u32::MAX
            && (!self.lexer.peek_token("=") || self.lexer.peek_token("=="))
            && !matches!(
                self.data.def_type(self.data.def_nr(name)),
                DefType::Function
            )
        {
            // @P335: functions are stored mangled as `n_<name>` and are reached
            // ONLY via the `n_`+ident lookup below — never by matching the RAW
            // identifier here.  Without this guard a user variable spelled
            // `n_day` raw-matches function `day` (stored `n_day`) and the
            // declaration mis-parses ("Expect token ;").  Enums / types stored
            // under their plain name still resolve here.
            let dnr = self.data.def_nr(name);
            if self.data.def_type(dnr) == DefType::Enum {
                t = self.data.def(dnr).returned().clone();
            } else if self.data.def_type(dnr) == DefType::EnumValue {
                t = Type::Enum(self.data.def(dnr).parent(), true, Vec::new());
            } else {
                t = Type::Null;
            }
        } else if matches!(self.data.def_type(self.context), DefType::Struct)
            && self.data.attr(self.context, name) != usize::MAX
        {
            let fnr = self.data.attr(self.context, name);
            *code = self.get_field(self.context, fnr, Value::Var(0));
            t = self.data.attr_type(self.context, fnr);
        } else if let Type::Enum(enr, _, _) = parent_tp
            && let Some(a_nr) = self.data.def(*enr).attr_names.get(name)
        {
            *code = self.data.attr_value(*enr, *a_nr);
            t = parent_tp.clone();
        } else {
            // try resolving as a bare function reference.
            // On the first pass, only do this when the identifier is NOT followed
            // by '=' (assignment position), so that `double = 5` still creates a
            // local variable that shadows the function name.
            let fn_d_nr = {
                let prefixed = format!("n_{nm}");
                let nr = self.data.def_nr(&prefixed);
                if nr == u32::MAX {
                    // @P335: the RAW fallback resolves names stored un-prefixed,
                    // but must NOT match a FUNCTION — otherwise a user identifier
                    // spelled `n_<x>` aliases function `<x>` (stored `n_<x>`).
                    // Functions are only reachable via the `n_`+ident form above.
                    let raw = self.data.def_nr(&nm);
                    if raw != u32::MAX && !matches!(self.data.def_type(raw), DefType::Function) {
                        raw
                    } else {
                        u32::MAX
                    }
                } else {
                    nr
                }
            };
            if fn_d_nr != u32::MAX && matches!(self.data.def_type(fn_d_nr), DefType::Function) {
                // @P392: the un-annotated form `<name> = …` is caught by the
                // `=` check; the typed-local form `<name>: T = …` lands here
                // with the lexer parked on `:`.  Without the typed-position
                // branch the parser silently produces a function-ref
                // `Value::Int`, back in parse_assign the `if let Value::Var(_)
                // = code` arm doesn't match, the `:` is never consumed, and
                // the user sees a confusing `Expect token ;` at the `:`.
                let un_annotated = self.lexer.peek_token("=") && !self.lexer.peek_token("==");
                let typed_local = self.lexer.peek_token(":");
                if un_annotated || typed_local {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot redefine function '{nm}' as a variable"
                    );
                    // RECOVER as a Var so parse_assign's downstream arms
                    // (typed-annotation `:` consumer; bare `=` assignment)
                    // run on a sane shape — otherwise the function-ref
                    // `Value::Int` below leaves the `:`/`=` un-consumed and
                    // we double-emit `Expect token ;`.  Both un-annotated
                    // and typed-local now recover with a single, clear
                    // diagnostic.
                    *code = Value::Var(self.create_var(name, &Type::Unknown(0)));
                    t = Type::Unknown(0);
                } else {
                    *code = Value::Int(fn_d_nr as i32);
                    self.data.def_used(fn_d_nr);
                    let n_args = self.data.attributes(fn_d_nr);
                    let arg_types: Vec<Type> = (0..n_args)
                        .map(|a| self.data.attr_type(fn_d_nr, a))
                        .collect();
                    let ret_type = self.data.def(fn_d_nr).returned().clone();
                    t = Type::Function(arg_types, Box::new(ret_type), vec![]);
                }
            } else if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Unknown variable '{}'", name);
                t = Type::Unknown(0);
            } else {
                *code = Value::Var(self.create_var(name, &Type::Unknown(0)));
                t = Type::Unknown(0);
            }
        }
        // Plan-22 phase 02d-iii.b — read auto-deref for boxed
        // scalars.  When `t` is `Reference(__cell_<T>, _)`
        // (set by phase 02d-iii.a's flip helper for mutated-
        // scalar-capture locals), wrap the resolved IR in
        // `Call(OpGet<T>, [code, Int(0)])` so reads load the
        // cell's `value` field instead of yielding the bare
        // DbRef.  Dormant in production until 02d-iii.e
        // activates the flip — no production variable has the
        // trigger type today, so the hook is a no-op for every
        // read.
        self.auto_deref_boxed_scalar(code, t)
    }

    /// Plan-22 phase 02d-iii.b — wrap a resolved variable read
    /// in an `OpGet<T>(code, 0)` call when its type is
    /// `Reference(__cell_<T>, _)` (a boxed scalar created by
    /// phase 02d-iii.a's flip).
    ///
    /// Returns the underlying scalar type (the cell's `value`
    /// field type).  No-op for any other input type.
    ///
    /// Mirrors `wrap_vector_get_val` in `parser/mod.rs:2409` —
    /// the same opcode-per-type table, the same boolean
    /// `OpGetByte` + `OpEqInt` shape.
    ///
    /// Limitations (acceptable for 02d-iii.b foundation):
    /// - Doesn't fire from `parse_var`'s early-return paths
    ///   (Reference-aliasing arm at lines 141-154, the special
    ///   `$` ref at line 18, etc.).  Those paths handle other
    ///   concerns and aren't on the critical path for
    ///   boxed-scalar reads.
    /// - Cells whose `value` field type isn't in the supported
    ///   primitive set (Integer / Float / Single / Boolean /
    ///   Character / Text / plain Enum) fall through with the
    ///   bare DbRef — phase 02d-ii's silent gap for exotic
    ///   shapes carries through here.
    pub(crate) fn auto_deref_boxed_scalar(&mut self, code: &mut Value, t: Type) -> Type {
        let Type::Reference(d_nr, _) = &t else {
            return t;
        };
        if !self.data.def(*d_nr).name().starts_with("__cell_") {
            return t;
        }
        let Some(value_attr) = self.data.def(*d_nr).attributes().first() else {
            return t;
        };
        if value_attr.name != "value" {
            return t;
        }
        let value_tp = value_attr.typedef.clone();
        let pos = Value::Int(0);
        let (op_name, is_bool) = match &value_tp {
            Type::Integer(_) => ("OpGetInt", false),
            Type::Float => ("OpGetFloat", false),
            Type::Single => ("OpGetSingle", false),
            Type::Text(_) => ("OpGetText", false),
            Type::Character => ("OpGetCharacter", false),
            Type::Enum(_, false, _) => ("OpGetEnum", false),
            // @PLN17: byte-stored boolean read, preserving 0/1/255 (like enum).
            Type::Boolean => ("OpGetBoolean", false),
            _ => return t,
        };
        let op_d_nr = self.data.def_nr(op_name);
        if op_d_nr == u32::MAX {
            return t;
        }
        if is_bool {
            let byte_call = Value::Call(op_d_nr, vec![code.clone(), pos, Value::Int(0)]);
            let eq_d_nr = self.data.def_nr("OpEqInt");
            if eq_d_nr == u32::MAX {
                *code = byte_call;
            } else {
                *code = Value::Call(eq_d_nr, vec![byte_call, Value::Int(1)]);
            }
        } else {
            *code = Value::Call(op_d_nr, vec![code.clone(), pos]);
        }
        value_tp
    }

    pub(crate) fn is_file_var(&self, var_nr: u16) -> bool {
        let file_def = self.data.def_nr("File");
        matches!(self.vars.tp(var_nr), Type::Reference(d, _) if *d == file_def)
    }

    pub(crate) fn file_op(&mut self, code: &mut Value, t: &mut Type, var_nr: u16) {
        self.vars.in_use(var_nr, true);
        if self.lexer.has_keyword("format") {
            let file_ref = Value::Var(var_nr);
            *code = self.cl("OpGetEnum", &[file_ref, Value::Int(32)]);
            let fmt_def = self.data.def_nr("Format");
            *t = Type::Enum(fmt_def, false, Vec::new());
        } else if self.lexer.has_keyword("exists") {
            let file_ref = Value::Var(var_nr);
            let fmt = self.cl("OpGetEnum", &[file_ref, Value::Int(32)]);
            let fmt_def = self.data.def_nr("Format");
            let enum_tp = Type::Enum(fmt_def, false, Vec::new());
            let ne_val = if let Some(&a_nr) = self.data.def(fmt_def).attr_names.get("NotExists") {
                self.data.attr_value(fmt_def, a_nr)
            } else {
                diagnostic!(self.lexer, Level::Error, "Format.NotExists not found");
                Value::Null
            };
            self.call_op(code, "!=", &[fmt, ne_val], &[enum_tp.clone(), enum_tp]);
            *t = Type::Boolean;
        } else if self.lexer.has_keyword("size") {
            *code = self.cl("OpSizeFile", &[Value::Var(var_nr)]);
            *t = crate::data::I64.clone();
        } else if self.lexer.has_keyword("index") {
            // Read the current field at offset 8 (pre-2c layout restored now that i32 is 4B)
            *code = self.cl("OpGetInt", &[Value::Var(var_nr), Value::Int(8)]);
            *t = crate::data::I64.clone();
        } else if self.lexer.has_keyword("next") {
            // Read the next field at offset 16 (pre-2c layout restored)
            *code = self.cl("OpGetInt", &[Value::Var(var_nr), Value::Int(16)]);
            *t = crate::data::I64.clone();
        } else if self.lexer.has_keyword("read") {
            // Size argument is optional: `f#read(n) as T` reads n bytes,
            // `f#read as T` derives n from T's fixed byte width.  Bare
            // `f#read as text` is rejected — text has no fixed width and
            // requires the explicit `(n)`.
            let has_explicit_size = self.lexer.has_token("(");
            let mut n_code = Value::Null;
            if has_explicit_size {
                self.expression(&mut n_code);
                self.lexer.token(")");
            }
            // Determine read type from optional "as T", remembering the
            // type's natural byte width so the size-less form can infer.
            // If no `as T` is given but the surrounding assignment has a
            // known destination type (`s.field = f#read`), use THAT — the
            // destination field's declared type drives the byte width,
            // symmetric with how `f += s.field` already takes its width
            // from the field's type.
            let has_cast = self.lexer.has_token("as");
            let target_hint = if !has_cast && !has_explicit_size {
                let hint = self.read_target_type.clone();
                if matches!(hint, Type::Integer(_) | Type::Float | Type::Single) {
                    Some(hint)
                } else {
                    None
                }
            } else {
                None
            };
            let (read_type, db_tp, inferred_size) = if has_cast {
                if let Some(type_name) = self.lexer.has_identifier() {
                    // Capture the alias def_nr so size(N) can pick Parts::Int.
                    let alias_nr = self.data.def_nr(&type_name);
                    let tp = self
                        .parse_type(u32::MAX, &type_name, false)
                        .unwrap_or(Type::Text(vec![]));
                    if let Type::Reference(d_nr, _) = &tp
                        && let Some(field) = Self::first_collection_field(*d_nr, &self.data)
                    {
                        let tname = self.data.def(*d_nr).name().to_string();
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "read_file: '{}' has collection field '{}'; use a plain struct for serialisation",
                            tname,
                            field
                        );
                    }
                    self.ensure_io_type(&tp.clone());
                    // Post-2c: honor `as i32` by routing to Parts::Int (4B) when
                    // the alias has size(4).
                    let forced = self.data.forced_size(alias_nr);
                    let id = if let Type::Integer(IntegerSpec { min, .. }) = &tp
                        && forced == Some(4)
                    {
                        if self.first_pass {
                            u16::MAX
                        } else {
                            self.database.int(*min, false)
                        }
                    } else if let Type::Integer(IntegerSpec { min, .. }) = &tp
                        && forced == Some(1)
                    {
                        if self.first_pass {
                            u16::MAX
                        } else {
                            self.database.byte(*min, false)
                        }
                    } else if let Type::Integer(IntegerSpec { min, .. }) = &tp
                        && forced == Some(2)
                    {
                        if self.first_pass {
                            u16::MAX
                        } else {
                            self.database.short(*min, false)
                        }
                    } else {
                        self.get_type(&tp)
                    };
                    // Byte width inferred from the cast — reuses the
                    // three-tier resolution that `sizeof(T)` already
                    // uses (parse_size in control.rs): forced_size of
                    // the alias > packed size of a range-constrained
                    // integer > database-allocated size of the base
                    // type.  Variable-width types (text, struct refs
                    // with collection fields) fall back to None and
                    // still require explicit `(n)`.
                    let nat_size = if self.first_pass {
                        Some(0_i64)
                    } else if let Some(n) = forced {
                        Some(i64::from(n))
                    } else {
                        let packed = tp.size(false);
                        if packed > 0 {
                            Some(i64::from(packed))
                        } else if matches!(tp, Type::Text(_)) {
                            None
                        } else {
                            let db_sz = self
                                .database
                                .size(self.data.def(self.data.type_elm(&tp)).known_type());
                            if db_sz == 0 {
                                None
                            } else {
                                Some(i64::from(db_sz))
                            }
                        }
                    };
                    (tp, id, nat_size)
                } else {
                    let text_tp = Type::Text(vec![]);
                    let id = self.get_type(&text_tp);
                    (text_tp, id, None)
                }
            } else if let Some(hint) = target_hint {
                // No `as T` — use the assignment-LHS type as the cast.
                // `IntegerSpec` carries TWO pieces of width info:
                // (a) `forced_size` (set by `pub type i32 = …size(4)`
                // typedefs — fixed-width regardless of range), and
                // (b) the implicit packed size derived from `min`/`max`
                // (`size(false)` returns 1/2/8 based on range).  The
                // forced size wins: an `i32` field has range covering
                // all 32-bit values which packs to 8, but its forced
                // size is 4.  Falls back to packed when no forced, and
                // to the database-allocated size otherwise.
                let forced_width: Option<u8> = if let Type::Integer(spec) = &hint {
                    spec.forced_size.map(std::num::NonZero::get)
                } else {
                    None
                };
                let nat = if let Some(n) = forced_width {
                    Some(i64::from(n))
                } else {
                    let packed = hint.size(false);
                    if packed > 0 {
                        Some(i64::from(packed))
                    } else {
                        let db_sz = self
                            .database
                            .size(self.data.def(self.data.type_elm(&hint)).known_type());
                        if db_sz == 0 {
                            None
                        } else {
                            Some(i64::from(db_sz))
                        }
                    }
                };
                let id = if self.first_pass {
                    u16::MAX
                } else if let Type::Integer(IntegerSpec { min, .. }) = &hint
                    && forced_width == Some(4)
                {
                    self.database.int(*min, false)
                } else if let Type::Integer(IntegerSpec { min, .. }) = &hint
                    && (forced_width == Some(1) || hint.size(false) == 1)
                {
                    self.database.byte(*min, false)
                } else if let Type::Integer(IntegerSpec { min, .. }) = &hint
                    && (forced_width == Some(2) || hint.size(false) == 2)
                {
                    self.database.short(*min, false)
                } else {
                    self.get_type(&hint)
                };
                (hint, id, nat)
            } else {
                let text_tp = Type::Text(vec![]);
                let id = self.get_type(&text_tp);
                (text_tp, id, None)
            };
            // Resolve the final size expression: explicit `(n)` wins;
            // otherwise inferred from the cast type.  Bare `f#read`
            // with no `as T` or with a variable-width cast (`as text`)
            // is a parse error.
            if !has_explicit_size {
                if let Some(n) = inferred_size {
                    n_code = Value::Int(n as i32);
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "f#read without (size) requires a fixed-width 'as <type>' cast (i32, u8, u16, integer, single, float, ...)"
                    );
                    n_code = Value::Int(0);
                }
            }
            let mut ls = Vec::new();
            let temp_var = if let Type::Text(_) = read_type {
                self.vars.work_text(&mut self.lexer)
            } else {
                let t = self.vars.unique("read", &read_type, &mut self.lexer);
                ls.push(v_set(t, self.null(&read_type)));
                t
            };
            let var_ref = self.cl("OpCreateStack", &[Value::Var(temp_var)]);
            ls.push(self.cl(
                "OpReadFile",
                &[
                    Value::Var(var_nr),
                    var_ref,
                    n_code,
                    Value::Int(i32::from(db_tp)),
                ],
            ));
            ls.push(Value::Var(temp_var));
            *code = v_block(ls, read_type.clone(), "reading file");
            *t = read_type;
        } else {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Unknown # operation on File");
            }
            *t = Type::Unknown(0);
        }
    }

    pub(crate) fn is_file_var_type(&self, tp: &Type) -> bool {
        let file_def = self.data.def_nr("File");
        matches!(tp, Type::Reference(d, _) if *d == file_def)
    }

    /// Ensure byte/short integer types used in file I/O are registered in the database.
    pub(crate) fn ensure_io_type(&mut self, t: &Type) {
        match t {
            Type::Integer(IntegerSpec { min, .. }) => match t.size(false) {
                1 => {
                    self.database.byte(*min, false);
                }
                2 => {
                    self.database.short(*min, false);
                }
                _ => {}
            },
            Type::Vector(tp, _) => {
                let tp = tp.clone();
                self.ensure_io_type(&tp);
            }
            _ => {}
        }
    }

    /// Return the name of the first collection-type field in `d_nr`, or `None`.
    /// Collection fields (sorted/index/hash/spacial) cannot be serialised by the binary
    /// file I/O routines; callers should emit a compile-time error when this returns `Some`.
    fn first_collection_field(d_nr: u32, data: &super::Data) -> Option<String> {
        for a in data.def(d_nr).attributes() {
            if matches!(
                a.typedef,
                Type::Sorted(..) | Type::Index(..) | Type::Hash(..) | Type::Spacial(..)
            ) {
                return Some(a.name.clone());
            }
        }
        None
    }

    pub(crate) fn write_to_file(
        &mut self,
        file_var: u16,
        val: Value,
        val_type: &Type,
        cast_alias: u32,
    ) -> Value {
        if let Type::Reference(d_nr, _) = val_type
            && let Some(field) = Self::first_collection_field(*d_nr, &self.data)
        {
            let type_name = self.data.def(*d_nr).name().to_string();
            diagnostic!(
                self.lexer,
                Level::Error,
                "write_file: '{}' has collection field '{}'; use a plain struct for serialisation",
                type_name,
                field
            );
            return Value::Null;
        }
        let val_type_clone = val_type.clone();
        self.ensure_io_type(&val_type_clone);
        // Post-2c: if the value was written as `… as <alias>` and the alias
        // has size(N), narrow the serialisation to the alias's db type.
        let db_tp = if let Type::Integer(IntegerSpec { min, .. }) = val_type
            && let Some(n) = self.data.forced_size(cast_alias)
        {
            if self.first_pass {
                u16::MAX
            } else {
                match n {
                    1 => self.database.byte(*min, false),
                    2 => self.database.short(*min, false),
                    4 => self.database.int(*min, false),
                    _ => self.get_type(val_type),
                }
            }
        } else {
            // Post-2c lint: a bare `f += <integer>` writes 8 bytes.  For
            // binary file formats (BigEndian / LittleEndian) that silently
            // breaks record alignment with older specs that assumed i32;
            // for text files 8 bytes of decimal is fine.  We can't tell
            // the file format at parse time, so warn generically — the
            // user silences the lint by writing `f += x as i32` (or the
            // correct byte-width alias).  Skip on the stdlib (`!self.default`)
            // and on explicit `as integer` (full 8-byte) where `cast_alias`
            // is the integer base — those are intentional wide writes.
            if !self.first_pass
                && !self.default
                && matches!(val_type, Type::Integer(_))
                && cast_alias == u32::MAX
            {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "`f += <integer>` without a width cast writes 8 bytes; \
                     for binary files (BigEndian / LittleEndian) add `as i8` \
                     / `as i16` / `as i32` / `as u8` / `as u16` / `as u32` to \
                     pick the exact byte width.  Use `as integer` to silence \
                     this warning when 8-byte writes are intentional"
                );
            }
            self.get_type(val_type)
        };
        let temp_var = self.vars.unique("wf", val_type, &mut self.lexer);
        for d in val_type.depend() {
            self.vars.depend(temp_var, d);
        }
        let assign = v_set(temp_var, val);
        let var_ref = self.cl("OpCreateStack", &[Value::Var(temp_var)]);
        let write = self.cl(
            "OpWriteFile",
            &[Value::Var(file_var), var_ref, Value::Int(i32::from(db_tp))],
        );
        Value::Insert(vec![assign, write])
    }

    pub(crate) fn parse_constant_value(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
        name_pos: &Position,
    ) -> Type {
        let mut t;
        let d_nr = if source == u16::MAX {
            self.data.def_nr(name)
        } else {
            self.data.source_nr(source, name)
        };
        if d_nr != u32::MAX {
            self.data.def_used(d_nr);
            t = self.data.def(d_nr).returned().clone();
            if self.data.def_type(d_nr) == DefType::Function {
                t = Type::Routine(d_nr);
            } else if matches!(
                self.data.def_type(d_nr),
                DefType::Struct | DefType::EnumValue
            ) && !matches!(self.data.def(d_nr).returned(), Type::Enum(_, false, _))
            {
                if self.lexer.peek_token("{") {
                    let tp = self.parse_object(d_nr, code);
                    if tp != Type::Unknown(0) {
                        return tp;
                    }
                } else if self.lexer.peek_token(".") {
                    self.lexer.cont();
                    if self.lexer.has_keyword("parse") {
                        return self.parse_type_parse(d_nr, code);
                    }
                }
            // Type.parse() for struct-enums.  Must not consume the
            // "." unless "parse" follows — `Enum.Variant` is a qualified
            // variant reference, not a method call.  Save a link and
            // revert if "parse" doesn't follow.
            } else if self.data.def_type(d_nr) == DefType::Enum
                && matches!(self.data.def(d_nr).returned(), Type::Enum(_, true, _))
                && self.lexer.peek_token(".")
            {
                let link = self.lexer.link();
                self.lexer.cont();
                if self.lexer.has_keyword("parse") {
                    return self.parse_type_parse(d_nr, code);
                }
                self.lexer.revert(link);
            } else if self.data.def_type(d_nr) == DefType::Constant {
                let const_code = self.data.def(d_nr).code().clone();
                let const_tp = self.data.def(d_nr).returned().clone();
                // vector constants are pre-built in CONST_STORE during
                // byte_code(). Emit OpConstRef + OpCopyRecord to deep-copy
                // from the constant store into a fresh runtime store.
                // On pass 1 const_ref is None but we still emit the same IR
                // shape so create_unique runs on both passes (counter sync).
                if matches!(const_tp, Type::Vector(_, _)) && matches!(const_code, Value::Block(_)) {
                    // Emit a simple Call to OpConstRef. The constant's DbRef
                    // will be deep-copied at the call site — the caller's
                    // gen_set_first_ref_call_copy handles the CopyRecord.
                    *code = self.cl("OpConstRef", &[Value::Int(d_nr as i32)]);
                    return const_tp;
                }
                *code = const_code;
                return const_tp;
            }
            if let Type::Enum(en, _, _) = t {
                for a_nr in 0..self.data.attributes(en) {
                    if self.data.attr_name(en, a_nr) == name {
                        // B2-runtime (2026-04-13): in a mixed struct-enum,
                        // a bare-identifier unit-variant literal (`s = Idle`)
                        // must produce a DbRef to a freshly allocated record
                        // with the discriminant set at offset 0, not a bare
                        // `Value::Enum(u8)`.  Without this, the receiving
                        // slot is typed DbRef but holds a u8 — native emit
                        // produces `let var_s: DbRef = 2_u8;` (rustc E0308)
                        // and the interpreter double-frees at exit.  Emit
                        // the same `OpDatabase` + field-init sequence that
                        // `parse_object` would for the struct-variant form.
                        let parent_is_mixed =
                            matches!(self.data.def(en).returned(), Type::Enum(_, true, _));
                        if parent_is_mixed && !self.first_pass {
                            let e_nr = self.data.def_nr(name);
                            if e_nr != u32::MAX && self.data.def(e_nr).known_type() != u16::MAX {
                                let ret = self.data.def(en).returned().clone();
                                let w = self.vars.work_refs(&ret, &mut self.lexer);
                                let known_type = i32::from(self.data.def(e_nr).known_type());
                                let mut list = Vec::new();
                                list.push(v_set(w, Value::Null));
                                list.push(
                                    self.cl("OpDatabase", &[Value::Var(w), Value::Int(known_type)]),
                                );
                                self.object_init(
                                    &mut list,
                                    e_nr,
                                    0,
                                    &Value::Var(w),
                                    &HashSet::new(),
                                );
                                list.push(Value::Var(w));
                                // The work-ref's DbRef is copied into the
                                // receiving slot (the LHS of assignment).
                                // The LHS OWNS the store (empty dep); the
                                // work-ref is skip_free (same store, no
                                // double-free).
                                self.vars.set_skip_free(w);
                                *code = v_block(list, Type::Enum(en, true, vec![]), "EnumUnitLit");
                                return t;
                            }
                        }
                        *code = self.data.attr_value(en, a_nr);
                        return t;
                    }
                }
            }
        }
        // #271 — a brace-construction `Name { … }` of a type that exists in a
        // `use`d library but is NOT `pub` arrives here unresolved (only pub names
        // import), so the `{` would otherwise trip a baffling "Expect token ;".
        // Name the real cause instead.  Pass-2 only, to emit once.
        // #271 — a brace-construction `Name { … }` of a type that exists in a
        // `use`d library but is NOT `pub` arrives here unresolved (`use` only
        // imports pub names), so the `{` would otherwise trip a baffling
        // "Expect token ;".  Name the real cause and recover by consuming the
        // balanced `{ … }` so neither pass cascades.  The structural error fires
        // on pass 1 (before pass 2 runs), so emit there; recover on both passes.
        if d_nr == u32::MAX && self.lexer.peek_token("{") && self.data.has_private_type(name) {
            if self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "type '{name}' is private — declare it `pub` to construct it outside its library"
                );
            }
            self.lexer.has_token("{");
            let mut depth = 1u32;
            while depth > 0 {
                let before = self.lexer.peek().position;
                if self.lexer.has_token("{") {
                    depth += 1;
                } else if self.lexer.has_token("}") {
                    depth -= 1;
                } else {
                    self.lexer.cont();
                }
                if self.lexer.peek().position == before {
                    break; // no forward progress (EOF) — bail rather than spin
                }
            }
            return Type::Unknown(0);
        }
        // A brace-construction `Name { field: … }` of a type that does not
        // exist at all.  Without naming the real cause, the unconsumed `{`
        // trips a baffling `Expect token ;` mid-literal.  Recover by consuming
        // the balanced `{ … }` so the statement parses.  `!name_exists` gates
        // out known variables (`items { … }` opening a loop body, `b { … }` an
        // if body) BEFORE the stateful struct-shape lookahead — a genuinely
        // unknown type has not been turned into a placeholder variable yet, so
        // it still passes — and the `ident :` / `ident ,` shape check (the same
        // `parse_block` uses) keeps a control-flow block from matching.
        if d_nr == u32::MAX
            && !self.vars.name_exists(name)
            && self.lexer.peek_token("{")
            && self.peek_struct_literal_body()
        {
            // Emit on pass 1 (matching the private-type branch above) so the
            // error fires once; recover on both passes so neither cascades.
            if self.first_pass {
                diagnostic_at!(self.lexer, name_pos, Level::Error, "unknown type '{name}'");
            }
            self.lexer.has_token("{");
            let mut depth = 1u32;
            while depth > 0 {
                let before = self.lexer.peek().position;
                if self.lexer.has_token("{") {
                    depth += 1;
                } else if self.lexer.has_token("}") {
                    depth -= 1;
                } else {
                    self.lexer.cont();
                }
                if self.lexer.peek().position == before {
                    break; // no forward progress (EOF) — bail rather than spin
                }
            }
            return Type::Unknown(0);
        }
        Type::Null
    }

    /// Peek past a `{` to decide whether it opens a struct literal (`{ field:
    /// … }` / `{ field, … }`) rather than a control-flow block.  Non-consuming
    /// (uses a lexer link/revert); mirrors the disambiguation in `parse_block`.
    fn peek_struct_literal_body(&mut self) -> bool {
        let link = self.lexer.link();
        self.lexer.token("{");
        let looks_like_struct = self.lexer.has_identifier().is_some()
            && ((self.lexer.peek_token(":") && !self.lexer.peek_token(":="))
                || self.lexer.peek_token(","));
        self.lexer.revert(link);
        looks_like_struct
    }

    pub(crate) fn known_var_or_type(&mut self, code: &Value, pos: &Position) {
        if let Value::Var(nr) = code {
            if !self.vars.exists(*nr) {
                return;
            }
            if self.default && matches!(self.vars.tp(*nr), Type::Vector(_, _)) {
                return;
            }
            if !self.first_pass && (self.vars.tp(*nr).is_unknown() || !self.vars.is_defined(*nr)) {
                let name = self.vars.name(*nr).to_string();
                let candidates: Vec<&str> = (0..self.vars.count())
                    .filter(|&v| {
                        v != *nr && self.vars.is_defined(v) && !self.vars.tp(v).is_unknown()
                    })
                    .map(|v| self.vars.name(v))
                    .collect();
                // Plan-07 phase 5: skip suggestions for very short names
                // (1 char) where typos are too ambiguous to be meaningful
                // (`x` vs `y` is a coin flip).  Standard `suggest_similar`
                // (distance ≤ 2) used here instead of the more aggressive
                // `suggest_similar_capped` because variable typos like
                // `result` vs `reuslt` (6-char transposition, distance 2)
                // are common and worth suggesting; the capped version's
                // `min(2, n/4)` formula is too strict for short names.
                let suggestion = if name.chars().count() <= 1 {
                    None
                } else {
                    crate::diagnostics::suggest_similar(&name, &candidates)
                };
                if let Some(s) = suggestion {
                    diagnostic_at!(
                        self.lexer,
                        pos,
                        Level::Error,
                        "Unknown variable '{}' — did you mean '{}'?",
                        name,
                        s
                    );
                } else {
                    diagnostic_at!(self.lexer, pos, Level::Error, "Unknown variable '{}'", name);
                }
            }
        }
    }

    /// `Type.parse(arg)` — populate a struct from a JsonValue.
    ///
    /// Single-walker design: regardless of the struct's
    /// shape, this emits exactly one IR call to
    /// `n_struct_from_jsonvalue(arg, struct_kt)`.  The walker uses
    /// `stores.types[struct_kt].parts` at runtime to dispatch on each
    /// field's declared type — primitives get extracted with
    /// path-qualified Q1 schema-side type checks, nested struct
    /// fields recurse, JsonValue-passthrough fields byte-copy, and
    /// vector fields iterate the JArray and recurse per element for
    /// struct elements.
    ///
    /// **Auto-wrap:** when the argument is plain
    /// text, transparently wrap with `json_parse(text)` first so
    /// legacy `Struct.parse(text)` keeps compiling but routes
    /// through the typed-tree pipeline (malformed input populates
    /// `json_errors()` instead of silently zero-filling the struct).
    /// Users wanting explicit staging can write
    /// `Struct.parse(json_parse(text))` themselves.
    fn parse_type_parse(&mut self, d_nr: u32, code: &mut Value) -> Type {
        self.lexer.token("(");
        let mut arg_expr = Value::Null;
        let arg_tp = self.expression(&mut arg_expr);
        self.lexer.token(")");
        if !self.first_pass {
            // JsonValue resolves to either Type::Reference (if a
            // user-declared alias) or Type::Enum(_, true, _) (mixed
            // struct-enum, the actual stdlib decl shape).
            let is_jsonvalue = match &arg_tp {
                Type::Reference(d, _) | Type::Enum(d, true, _) => {
                    self.data.def(*d).name() == "JsonValue"
                }
                _ => false,
            };
            if is_jsonvalue {
                // Direct JsonValue → walker.  This is the new
                // typed-tree path used by `Struct.parse(json_parse(text))`
                // and by the `Struct.parse(JsonValue)` codegen elsewhere.
                let n_walker = self.data.def_nr("n_struct_from_jsonvalue");
                debug_assert_ne!(
                    n_walker,
                    u32::MAX,
                    "n_struct_from_jsonvalue must be registered in NATIVE_FNS"
                );
                let known_tp = self.data.def(d_nr).known_type();
                *code = Value::Call(n_walker, vec![arg_expr, Value::Int(i32::from(known_tp))]);
            } else {
                // Text or other → legacy lenient text-parse path
                // (`OpCastVectorFromText` calls
                // `src/database/structures.rs::parsing`, which accepts
                // both standard JSON and loft-native bare-key syntax).
                // Preserves the legacy data-import semantics.
                let mut text_expr = arg_expr;
                if !matches!(arg_tp, Type::Text(_)) {
                    self.convert(&mut text_expr, &arg_tp, &Type::Text(Vec::new()));
                }
                let known_tp = self.data.def(d_nr).known_type();
                *code = self.cl(
                    "OpCastVectorFromText",
                    &[text_expr, Value::Int(i32::from(known_tp))],
                );
            }
        }
        Type::Reference(d_nr, Vec::new())
    }

    /// Parse `vector<T>.parse(text)` — parse a JSON array into a vector of T.
    /// Returns `Type::Vector(T)` so the result is directly iterable.
    fn parse_vector_parse(&mut self, elem_d_nr: u32, code: &mut Value) -> Type {
        self.lexer.token("(");
        let mut text_expr = Value::Null;
        let tp = self.expression(&mut text_expr);
        self.lexer.token(")");
        let elem_tp = Type::Reference(elem_d_nr, Vec::new());
        let vec_type = Type::Vector(Box::new(elem_tp.clone()), Vec::new());
        if !self.first_pass {
            if !matches!(tp, Type::Text(_)) {
                self.convert(&mut text_expr, &tp, &Type::Text(Vec::new()));
            }
            // Get the database vector type for vector<elem>.
            let elem_kt = self.data.def(elem_d_nr).known_type();
            let vec_kt = self.database.vector(elem_kt);
            let parse_call = self.cl(
                "OpCastVectorFromText",
                &[text_expr, Value::Int(i32::from(vec_kt))],
            );
            // The parse returns a DbRef to the wrapper struct main_vector<T>.
            // Extract the vector field (at position 0) so the result is directly iterable.
            let wrapper_name = format!("main_vector<{}>", self.data.def(elem_d_nr).name());
            let wrapper_d_nr = self.data.def_nr(&wrapper_name);
            if wrapper_d_nr == u32::MAX {
                *code = parse_call;
            } else {
                *code = self.get_field(wrapper_d_nr, 0, parse_call);
            }
        }
        // Ensure the vector def exists for type resolution.
        self.data.vector_def(&mut self.lexer, &elem_tp);
        vec_type
    }

    pub(crate) fn parse_string(&mut self, code: &mut Value, string: &str) -> Type {
        let mut append_value = u16::MAX;
        *code = Value::str(string);
        let mut var = u16::MAX;
        let mut list = vec![];
        if self.lexer.mode() == Mode::Formatting {
            // Define a new variable to append to
            var = self.vars.work_text(&mut self.lexer);
            list.push(v_set(var, code.clone()));
        }
        while self.lexer.mode() == Mode::Formatting {
            self.lexer.set_mode(Mode::Code);
            let mut format = Value::Null;
            let saved_in_fmt = self.in_format_expr;
            self.in_format_expr = true;
            let mut tp = if self.lexer.has_token("for") {
                self.iter_for(&mut format, &mut append_value)
            } else {
                self.expression(&mut format)
            };
            self.in_format_expr = saved_in_fmt;
            // Plan-07 phase 4e.1 — format strings are the user's
            // observability surface and must NEVER halt, log, or warn
            // (per C66 + DESIGN_DECISIONS 2026-05-11).  Walk the
            // interpolated expression tree and swap every fault-prone
            // op to its Nullable peer so `println("{a / b}")` /
            // `println("{user.name}")` / `println("{v[i]}")` always
            // render the silent sentinel ("null") instead of taking
            // out the print statement that the developer is using to
            // diagnose the problem in the first place.
            //
            // Phase 4e.3 — when the OUTERMOST swapped op is
            // fault-prone (div / mod / vector-index / text-index),
            // append an `OpTagFault(kind)` SIBLING statement to the
            // statement list BEFORE the format-conversion op so the
            // conversion op sees the tag and renders `null(<reason>)`
            // instead of bare `null` on the null sentinel.  Inner
            // faults (`"{a + v[i] / b}"`) get their Nullable peer
            // swap from the recursion but do NOT tag — there's no
            // renderer to consume their tag in mid-expression.
            let outer_fault_kind = if self.first_pass {
                None
            } else {
                Self::rewrite_subtree_to_nullable_kind(&mut format, &self.data)
            };
            if let Some(kind) = outer_fault_kind
                && self.data.def_nr("OpTagFault") != u32::MAX
            {
                let tag_call = self.cl("OpTagFault", &[Value::Int(i32::from(kind))]);
                list.push(tag_call);
            }
            self.un_ref(&mut tp, &mut format);
            if !self.first_pass && tp.is_unknown() {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Incorrect expression in string was {tp:?}"
                );
                return Type::Void;
            }
            self.lexer.set_mode(Mode::Formatting);
            let mut state = OUTPUT_DEFAULT;
            let mut token = "0".to_string();
            if self.lexer.has_token(":") {
                if let LexResult {
                    has: LexItem::Token(t),
                    position: _pos,
                } = self.lexer.peek()
                {
                    let st: &str = &t;
                    if !SKIP_TOKEN.contains(&st) {
                        token.clear();
                        token += &t;
                        state.token = &token;
                        self.lexer.cont();
                    }
                }
                self.string_states(&mut state);
                let LexResult {
                    has: h,
                    position: _pos,
                } = self.lexer.peek();
                if match h {
                    LexItem::Token(st) | LexItem::Identifier(st) => {
                        let s: &str = &st;
                        !SKIP_WIDTH.contains(&s)
                    }
                    LexItem::Integer(_, _) | LexItem::Float(_) => true,
                    _ => false,
                } {
                    if let LexResult {
                        has: LexItem::Integer(_, true),
                        position: _pos,
                    } = self.lexer.peek()
                    {
                        state.token = "0";
                    }
                    self.lexer.set_mode(Mode::Code);
                    self.expression(&mut state.width);
                    self.lexer.set_mode(Mode::Formatting);
                }
                state.radix = self.get_radix();
            }
            self.append_data(tp, &mut list, var, append_value, &format, state);
            if let Some(text) = self.lexer.has_cstring() {
                if !text.is_empty() {
                    let call = if matches!(self.vars.tp(var), Type::RefVar(_)) {
                        "OpAppendStackText"
                    } else {
                        "OpAppendText"
                    };
                    list.push(self.cl(call, &[Value::Var(var), Value::str(&text)]));
                }
            } else {
                diagnostic!(self.lexer, Level::Error, "Formatter error");
                return Type::Void;
            }
        }
        if var < u16::MAX {
            list.push(Value::Var(var));
            *code = v_block(list, Type::Text(vec![var]), "Formatted string");
            Type::Text(vec![var])
        } else {
            Type::Text(Vec::new())
        }
    }

    pub(crate) fn string_states(&mut self, state: &mut OutputState) {
        if self.lexer.has_token("<") {
            state.dir = -1;
        } else if self.lexer.has_token("^") {
            state.dir = 0;
        } else if self.lexer.has_token(">") {
            state.dir = 1;
        }
        if self.lexer.has_token("+") {
            state.plus = true;
        }
        if self.lexer.has_token("#") {
            // show 0x 0b or 0o in front of numbers when applicable
            state.note = true;
        }
        if self.lexer.has_token(".") {
            state.float = true;
        }
    }

    pub(crate) fn get_radix(&mut self) -> i32 {
        if let Some(id) = self.lexer.has_identifier() {
            if id.to_lowercase() == "j" || id.to_lowercase() == "json" {
                -1
            } else if id == "x" || id == "X" {
                16
            } else if id == "b" {
                2
            } else if id == "o" {
                8
            } else if id == "e" {
                1
            } else if id == "d" || id == "f" {
                10
            } else {
                diagnostic!(self.lexer, Level::Error, "Unexpected formatting type: {id}");
                10
            }
        } else {
            10
        }
    }

    // Iterator for
    // <for> ::= <identifier> 'in' <range> '{' <block>
    pub(crate) fn iter_for(&mut self, val: &mut Value, append_value: &mut u16) -> Type {
        if let Some(id) = self.lexer.has_identifier() {
            // Create {id}#index first (always needed, regardless of type).
            let index_var = self.create_var(&format!("{id}#index"), &I32);
            self.vars.defined(index_var);
            self.lexer.token("in");
            let loop_nr = self.vars.start_loop();
            let mut expr = Value::Null;
            let in_type = self.parse_in_range(&mut expr, &Value::Null, &id);
            // For text loops: {id}#next drives the loop; {id}#index is saved per-iteration.
            let (iter_var, pre_var) = if matches!(in_type, Type::Text(_)) {
                let pos_var = self.create_var(&format!("{id}#next"), &I32);
                self.vars.defined(pos_var);
                (pos_var, Some(index_var))
            } else {
                (index_var, None)
            };
            let var_tp = self.for_type(&in_type);
            *append_value = self.create_unique("val", &Type::Unknown(0));
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
            let mut block = Value::Null;
            let format_type = self.parse_block("for", &mut block, &Type::Unknown(0));
            self.change_var_type(*append_value, &format_type);
            self.in_loop = in_loop;
            let mut lp = vec![for_next];
            if !matches!(in_type, Type::Iterator(_, _)) {
                lp.push(v_if(
                    self.single_op("!", Value::Var(for_var), var_tp.clone()),
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            }
            if if_step != Value::Null {
                lp.push(v_if(if_step, Value::Null, Value::Continue(0)));
            }
            let result_tp = if let Value::Block(bl) = &block {
                bl.result.clone()
            } else {
                var_tp.clone()
            };
            lp.push(block);
            let tp = Type::Iterator(Box::new(format_type), Box::new(Type::Null));
            // For text loops, extra_init holds v_set(index_var, 0) which must be emitted at
            // the same scope level as the iterator init (outside the loop) so the slot
            // assigner sees {id}#index as live across the entire loop body.
            let extra_init = if let Some(idx_var) = pre_var {
                Box::new(v_set(idx_var, Value::Int(0)))
            } else {
                Box::new(Value::Null)
            };
            *val = Value::Iter(
                for_var,
                Box::new(create_iter),
                Box::new(v_block(lp, result_tp, "Iter For")),
                extra_init,
            );
            self.vars.finish_loop(loop_nr);
            return tp;
        }
        diagnostic!(self.lexer, Level::Error, "Expect variable after for");
        Type::Null
    }

    // range ::= rev(<expr> '..' ['='] <expr>) | <expr> [ '..' ['='] <expr> ]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_in_range_body(
        &mut self,
        expr: &mut Value,
        data: &Value,
        name: &str,
        in_type: Type,
        reverse: bool,
    ) -> Type {
        let incl = self.lexer.has_token("=");
        // O8.5: capture range bounds for const-unroll detection.
        self.last_range_from = Some(expr.clone());
        let mut till = Value::Null;
        let till_tp = if self.lexer.peek_token("]") {
            till = if *data == Value::Null {
                Value::Int(i32::MAX)
            } else {
                self.cl("OpLengthVector", std::slice::from_ref(data))
            };
            in_type.clone()
        } else {
            self.expression(&mut till)
        };
        // O8.5: store till value (adjusted for inclusive ranges).
        if incl {
            // 0..=9 means till is 9, but the range includes 9.
            // Store till+1 so the unroller can use from..till_exclusive.
            if let Value::Int(t) = &till {
                self.last_range_till = Some(Value::Int(t + 1));
            } else {
                self.last_range_till = None;
            }
        } else {
            self.last_range_till = Some(till.clone());
        }
        let ivar = if name == "$" {
            self.create_unique("index", &in_type.clone())
        } else {
            self.create_var(&format!("{name}#index"), &in_type)
        };
        let mut ls = Vec::new();
        let test = if reverse {
            if incl {
                ls.push(v_set(
                    ivar,
                    v_if(
                        self.single_op("!", Value::Var(ivar), in_type.clone()),
                        till,
                        self.conv_op(
                            "-",
                            Value::Var(ivar),
                            Value::Int(1),
                            in_type.clone(),
                            I32.clone(),
                        ),
                    ),
                ));
            } else {
                ls.push(v_if(
                    self.single_op("!", Value::Var(ivar), in_type.clone()),
                    v_set(ivar, till),
                    Value::Null,
                ));
                ls.push(v_set(
                    ivar,
                    self.conv_op(
                        "-",
                        Value::Var(ivar),
                        Value::Int(1),
                        in_type.clone(),
                        I32.clone(),
                    ),
                ));
            }
            self.conv_op(
                "<",
                Value::Var(ivar),
                expr.clone(),
                in_type.clone(),
                till_tp,
            )
        } else {
            ls.push(v_set(
                ivar,
                v_if(
                    self.single_op("!", Value::Var(ivar), in_type.clone()),
                    expr.clone(),
                    self.conv_op(
                        "+",
                        Value::Var(ivar),
                        Value::Int(1),
                        in_type.clone(),
                        I32.clone(),
                    ),
                ),
            ));
            self.conv_op(
                if incl { "<" } else { "<=" },
                till,
                Value::Var(ivar),
                till_tp,
                in_type.clone(),
            )
        };
        ls.push(v_if(test, Value::Break(0), Value::Null));
        ls.push(Value::Var(ivar));
        *expr = Value::Iter(
            u16::MAX,
            Box::new(v_set(ivar, self.null(&in_type))),
            Box::new(v_block(ls, in_type.clone(), "Iter range")),
            Box::new(Value::Null),
        );
        if reverse {
            self.lexer.token(")");
            self.reverse_iterator = false;
        }
        Type::Iterator(Box::new(in_type), Box::new(Type::Null))
    }

    pub(crate) fn parse_in_range(&mut self, expr: &mut Value, data: &Value, name: &str) -> Type {
        let mut reverse = false;
        if let LexItem::Identifier(rev) = self.lexer.peek().has
            && &rev == "rev"
        {
            self.lexer.has_identifier();
            self.lexer.token("(");
            reverse = true;
            // set the reverse flag BEFORE parsing the inner expression so that
            // rev(col[lo..hi]) passes the flag through parse_key → fill_iter.
            self.reverse_iterator = true;
        }
        let in_type = if self.lexer.peek_token("..") || self.lexer.peek_token("..=") {
            // Open-start range: treat missing start as 0.
            *expr = Value::Int(0);
            I32.clone()
        } else {
            self.expression(expr)
        };
        if !self.lexer.has_token("..") {
            if reverse {
                // if the inner expression was a subscript that already produced
                // a range iterator (parse_key consumed the `..`), the Value::Iter is
                // ready with the reverse flag — just consume ')' and return.
                if matches!(expr, Value::Iter(_, _, _, _)) {
                    self.lexer.token(")");
                    self.reverse_iterator = false;
                    return in_type;
                }
                // rev() wrapping a bare collection (not a range subscript).
                if matches!(
                    in_type,
                    Type::Sorted(_, _, _) | Type::Index(_, _, _) | Type::Vector(_, _)
                ) {
                    // reverse_iterator stays set; consumed and reset by iterator()
                } else if !matches!(in_type, Type::Null) {
                    self.reverse_iterator = false;
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "rev() on a non-range expression must wrap a sorted, index, or vector collection"
                    );
                }
                self.lexer.token(")");
            }
            return in_type;
        }
        self.parse_in_range_body(expr, data, name, in_type, reverse)
    }

    pub(crate) fn parse_object_field(
        &mut self,
        td_nr: u32,
        code: &mut Value,
        list: &mut Vec<Value>,
        found_fields: &mut HashSet<String>,
        in_place_var: Option<u16>,
        hoists: &mut Vec<Value>,
    ) -> bool {
        // Accept both bare identifiers and JSON-style quoted strings as field names.
        let field = if let Some(id) = self.lexer.has_identifier() {
            id
        } else if let Some(s) = self.lexer.has_cstring() {
            s
        } else {
            return false;
        };
        if !self.lexer.has_token(":") {
            return false;
        }
        let nr = self.data.attr(td_nr, &field);
        if nr == usize::MAX {
            if let Some(s) = self.suggest_field_name(td_nr, &field) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown field {}.{field} — did you mean '{s}'?",
                    self.data.def(td_nr).name()
                );
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown field {}.{field}",
                    self.data.def(td_nr).name()
                );
            }
        } else {
            let td = self.data.attr_type(td_nr, nr);
            let pos = self
                .database
                .position(self.data.def(td_nr).known_type(), &field);
            found_fields.insert(field.clone());
            let mut value = if let Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Spacial(_, _, _)
            | Type::Enum(_, true, _)
            | Type::Index(_, _, _) = td
            {
                // Collection/enum-big header is a 4-byte u32 record pointer.
                // Post-2c `OpSetInt` writes 8 bytes and overflows the field.
                list.push(self.cl(
                    "OpSetInt4",
                    &[code.clone(), Value::Int(i32::from(pos)), Value::Int(0)],
                ));
                let info = self.type_info(&td);
                self.cl(
                    "OpGetField",
                    &[code.clone(), Value::Int(i32::from(pos)), info],
                )
            } else {
                Value::Null
            };
            let mut parent_tp = Type::Reference(td_nr, Vec::new());
            if let Value::Var(v) = code {
                parent_tp = parent_tp.depending(*v);
            }
            // Collection fields prime `value` with an in-field write target
            // (the literal writes THROUGH the field) — those must not be
            // hoisted; only pure value expressions (`value` started Null).
            let primed = !matches!(value, Value::Null);
            let exp_tp = self.parse_operators(&td, &mut value, &mut parent_tp, 0);
            // #330: an initialiser that READS the in-place target is hoisted
            // into a typed temp; the temps run before the OpDatabase re-init
            // (spliced in parse_object), so they see the OLD record.
            if let Some(xv) = in_place_var
                && !primed
                && value.reads_var(xv)
            {
                let tmp = self.vars.work_refs(&exp_tp, &mut self.lexer);
                if !self.first_pass {
                    self.change_var_type(tmp, &exp_tp);
                }
                let prev = std::mem::replace(&mut value, Value::Var(tmp));
                hoists.push(v_set(tmp, prev));
            }
            self.handle_field(td_nr, code, list, &field, &mut value, &exp_tp);
        }
        true
    }

    pub(crate) fn parse_object(&mut self, td_nr: u32, code: &mut Value) -> Type {
        let link = self.lexer.link();
        if !self.lexer.has_token("{") {
            self.lexer.revert(link);
            return Type::Unknown(0);
        }
        let mut list = Vec::new();
        let mut new_object = false;
        let mut in_place_var: Option<u16> = None;
        let mut hoists: Vec<Value> = Vec::new();
        let work = self.vars.work_ref();
        if let Value::Var(v_nr) = code {
            let var_tp = self.vars.tp(*v_nr).clone();
            let type_matches =
                var_tp.is_unknown() || matches!(&var_tp, Type::Reference(d, _) if *d == td_nr);
            if self.vars.is_independent(*v_nr) && type_matches {
                // #330: remember the in-place target — a field initialiser
                // that READS it must be hoisted ABOVE the OpDatabase re-init
                // (see the hoist in parse_object_field and the splice after
                // the field loop), because the re-init clears the record
                // before the initialisers run.
                if !self.first_pass && !self.vars.is_compiler_generated(*v_nr) {
                    in_place_var = Some(*v_nr);
                }
                if !self.vars.is_argument(*v_nr) {
                    list.push(v_set(*v_nr, Value::Null));
                }
                self.data.set_referenced(td_nr, self.context, Value::Null);
                let tp = i32::from(self.data.def(td_nr).known_type());
                list.push(self.cl("OpDatabase", &[Value::Var(*v_nr), Value::Int(tp)]));
            } else if (!type_matches
                || (!self.vars.is_independent(*v_nr) && !self.vars.is_compiler_generated(*v_nr)))
                && !self.first_pass
            {
                // Two shapes route here:
                // - LHS variable already has an incompatible type (e.g. integer from a
                //   prior pass) — `!type_matches`.
                // - LHS variable is a user-declared local whose type was inferred
                //   as dependent on some other variable (e.g. `x` had `x = bs[i]` in a
                //   later statement, giving x type `Reference(T, [bs])`), so
                //   `is_independent` returns false even though `type_matches` is true
                //   for this struct-literal assignment.  Without this branch, the
                //   in-place `v_set(x, Null) + OpDatabase` init above was skipped,
                //   leaving only field-init calls that wrote into uninitialised storage
                //   — the subsequent codegen then asserted
                //   `Incorrect var x[N] versus M` when x's slot was read later.
                //
                // The `is_compiler_generated` guard excludes internal aliases like
                // `_elm_N` / `__ref_N` / `_vector_N` (created by parser helpers via
                // `Function::unique`) whose storage is already allocated by the
                // enclosing vector slot or struct field — they correctly receive
                // field-inits without v_set/OpDatabase, so routing them through
                // new_object would break the aliasing and create an orphan allocation.
                //
                // Falls through to new_object so the struct gets a fresh work ref and
                // the result is a proper Value::Block — not a Value::Insert — which can be
                // used safely as a method-call argument.
                new_object = true;
                self.data.set_referenced(td_nr, self.context, Value::Null);
                let ret = self.data.def(td_nr).returned();
                let w = self.vars.work_refs(ret, &mut self.lexer);
                let tp = i32::from(self.data.def(td_nr).known_type());
                list.push(v_set(w, Value::Null));
                list.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp)]));
                *code = Value::Var(w);
            }
        } else if !self.first_pass && !self.is_field(code) {
            new_object = true;
            self.data.set_referenced(td_nr, self.context, Value::Null);
            let ret = self.data.def(td_nr).returned();
            let w = self.vars.work_refs(ret, &mut self.lexer);
            let tp = i32::from(self.data.def(td_nr).known_type());
            list.push(v_set(w, Value::Null));
            list.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp)]));
            *code = Value::Var(w);
        }
        let mut found_fields = HashSet::new();
        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            if !self.parse_object_field(
                td_nr,
                code,
                &mut list,
                &mut found_fields,
                in_place_var,
                &mut hoists,
            ) {
                self.lexer.revert(link);
                self.vars.clean_work_refs(work);
                return Type::Unknown(0);
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("}");
        // #330 splice: run the hoisted self-reading field values BEFORE the
        // in-place `Set(x, Null) + OpDatabase(x)` prelude clears the record.
        if !hoists.is_empty() {
            hoists.append(&mut list);
            list = hoists;
        }
        if !self.first_pass {
            self.object_init(&mut list, td_nr, 0, code, &found_fields);
            // emit all field constraint checks after construction completes.
            let assert_dnr = self.data.def_nr("n_assert");
            for a_nr in 0..self.data.def(td_nr).attributes().len() {
                let check = self.data.def(td_nr).attributes()[a_nr].check.clone();
                if check != Value::Null {
                    let bound = Self::replace_record_ref(check, code);
                    let nm = self.data.attr_name(td_nr, a_nr);
                    let msg = match &self.data.def(td_nr).attributes()[a_nr].check_message {
                        Value::Text(s) => Value::Text(s.clone()),
                        _ => Value::Text(format!(
                            "field constraint failed on {}.{nm}",
                            self.data.def(td_nr).name()
                        )),
                    };
                    let pos = self.lexer.pos();
                    list.push(Value::Call(
                        assert_dnr,
                        vec![
                            bound,
                            msg,
                            Value::Text(pos.file.clone()),
                            Value::Int(pos.line as i32),
                        ],
                    ));
                }
            }
        }
        if new_object && let Value::Var(v) = code {
            list.push(Value::Var(*v));
            *code = v_block(list, Type::Reference(td_nr, vec![*v]), "Object");
            Type::Reference(td_nr, Vec::new())
        } else {
            *code = Value::Insert(list);
            Type::Rewritten(Box::new(Type::Reference(td_nr, Vec::new())))
        }
    }

    /// Recursively replace `Value::Var(0)` (the record placeholder used in field default
    /// expressions) with the actual record reference from the calling context.
    pub(crate) fn replace_record_ref(mut val: Value, record: &Value) -> Value {
        val.map_nodes(&mut |n| {
            if matches!(n, Value::Var(0)) {
                *n = record.clone();
            }
        });
        val
    }

    // fill the not mentioned fields with their default value
    pub(crate) fn object_init(
        &mut self,
        list: &mut Vec<Value>,
        td_nr: u32,
        pos: u16,
        code: &Value,
        found_fields: &HashSet<String>,
    ) {
        for aid in 0..self.data.attributes(td_nr) {
            let tp = self.data.attr_type(td_nr, aid);
            let nm = self.data.attr_name(td_nr, aid);
            let fld = self
                .database
                .position(self.data.def(td_nr).known_type(), &nm);
            // Skip computed fields (not stored) and already-provided fields.
            if found_fields.contains(&nm)
                || matches!(tp, Type::Routine(_))
                || self.data.def(td_nr).attributes()[aid].constant
            {
                continue;
            }
            let mut default = self.data.attr_value(td_nr, aid);
            // #328/#332: a POINTER field (`reference<T>`, the u16::MAX share
            // marker) is a 12-byte DbRef — its omitted default is the null
            // sentinel.  The inline recursion below would write the INNER
            // struct's field defaults over the DbRef bytes (it only looked
            // harmless while integer defaults were zeros).
            if let Type::Reference(_, deps) = &tp
                && deps.contains(&u16::MAX)
                && default == Value::Null
            {
                let sentinel = self.cl("OpNullRefSentinel", &[]);
                list.push(self.set_field_no_check(td_nr, aid, pos, code.clone(), sentinel));
                continue;
            }
            if let Type::Reference(tp, _) = tp
                && default == Value::Null
            {
                self.object_init(list, tp, pos + fld, code, &HashSet::new());
                continue;
            } else if default == Value::Null {
                // LOFT.md § constructors: an omitted field gets "the zero
                // value for its type" — numerics default to 0 (NOT null;
                // tests/scripts/06-structs.loft locks this).  Pointer
                // fields take the sentinel branch above: a pointer's zero
                // value IS null.
                default = to_default(&tp, &self.data);
            } else {
                default = Self::replace_record_ref(default, code);
            }
            list.push(self.set_field_no_check(td_nr, aid, pos, code.clone(), default));
        }
    }

    /// @P308 — the specific keyed-collection db type id (the id
    /// `OpReplaceKeyed` / `copy_claims` need) to deep-copy a `hash`/`sorted`/
    /// `index` field from an expression, else `None` (the caller keeps the
    /// bare-push).  `spacial` is excluded (`copy_claims` unimplemented, per
    /// @P295).  Mirrors the keyed-LOCAL `keyed_kt` logic in
    /// `expressions.rs::parse_assign_op`.  (Sorted/index were briefly
    /// HASH-only while @P309 — a deep-copy data-loss/hang when `index<T>`
    /// grew the shared element struct — was open; now fixed in
    /// `copy_claims_array_body`.)
    pub(crate) fn keyed_field_kt(&mut self, td: &Type) -> Option<u16> {
        match td {
            Type::Hash(d, key, _) => {
                let c = self.data.def(*d).known_type();
                (c != u16::MAX).then(|| self.database.hash(c, key))
            }
            Type::Sorted(d, key, _) => {
                let c = self.data.def(*d).known_type();
                (c != u16::MAX).then(|| self.database.sorted(c, key))
            }
            Type::Index(d, key, _) => {
                let c = self.data.def(*d).known_type();
                (c != u16::MAX).then(|| self.database.index(c, key))
            }
            _ => None,
        }
    }

    pub(crate) fn handle_field(
        &mut self,
        td_nr: u32,
        code: &mut Value,
        list: &mut Vec<Value>,
        field: &str,
        value: &mut Value,
        exp_tp: &Type,
    ) {
        let nr = self.data.attr(td_nr, field);
        let td = self.data.attr_type(td_nr, nr);
        if matches!(
            td,
            Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Spacial(_, _, _)
                | Type::Index(_, _, _)
        ) {
            // Issue #120: for vector fields assigned from a bare variable
            // (e.g. `BigBox { data: d }`), parse_operators overwrites the
            // field ref with Var(d) — no copy operation is generated.
            // Emit OpAppendVector to deep-copy the source vector into the
            // struct's field so the data is independent of the source store.
            //
            // the same holds for any non-Insert vector-typed
            // expression (e.g. `C { v: build() }` where `build` returns a
            // vector).  Before this was a plain push, which left the field
            // uninitialised.
            if let Type::Vector(ref content, _) = td {
                if !self.first_pass && !matches!(value, Value::Insert(_) | Value::Null) {
                    let pos = self
                        .database
                        .position(self.data.def(td_nr).known_type(), field);
                    // `vector_of` consults
                    // `Data::narrow_vector_content` and registers a
                    // narrow element type when the content is a narrow
                    // alias (i32, u8).  `elem_tp` is the narrow type
                    // (Parts::Byte / Parts::Int) when applicable.
                    let vec_tp = self.vector_of(content);
                    let elem_tp = self.database.content(vec_tp);
                    let field_ref = self.cl(
                        "OpGetField",
                        &[
                            code.clone(),
                            Value::Int(i32::from(pos)),
                            Value::Int(i32::from(vec_tp)),
                        ],
                    );
                    list.push(self.cl(
                        "OpAppendVector",
                        &[field_ref, value.clone(), Value::Int(i32::from(elem_tp))],
                    ));
                } else {
                    list.push(value.clone());
                }
            } else if let Some(kt) = self.keyed_field_kt(&td)
                && !self.first_pass
                && !matches!(value, Value::Insert(_) | Value::Null)
            {
                // @P308 — a keyed-collection field (hash/sorted/index)
                // initialised from an EXPRESSION (`F{ h: build() }` /
                // `F{ h: c }`) must be DEEP-COPIED into the field, exactly
                // like the Vector branch above deep-copies via OpAppendVector.
                // Before this it fell to the bare `list.push(value)` below,
                // which left the field empty (the value's result was dropped)
                // and leaked a call source's store.  `OpReplaceKeyed(src,
                // field_ref, kt)` = remove_claims(field) (no-op on the
                // zero-inited field) + copy_claims(src, field) — the same
                // deep-copy that runs when a struct with a keyed field is
                // copied.  The `0x8000` bit frees a fresh-storage call source.
                // An empty/literal `[]` keeps the bare push (field stays
                // empty — correct).  Spacial is excluded by `keyed_field_kt`
                // (copy_claims panics for it, per @P295).
                let pos = self
                    .database
                    .position(self.data.def(td_nr).known_type(), field);
                let field_ref = self.cl(
                    "OpGetField",
                    &[
                        code.clone(),
                        Value::Int(i32::from(pos)),
                        Value::Int(i32::from(kt)),
                    ],
                );
                let tp_val = if self.is_struct_returning_call(value) {
                    i32::from(kt) | 0x8000
                } else {
                    i32::from(kt)
                };
                list.push(self.cl(
                    "OpReplaceKeyed",
                    &[value.clone(), field_ref, Value::Int(tp_val)],
                ));
            } else {
                list.push(value.clone());
            }
        } else if let Value::Insert(ops) = value {
            for o in ops {
                list.push(o.clone());
            }
        } else {
            // @P279 — when pass-1 sees an Unknown value being assigned
            // to a typed field, suppress the diagnostic.  Unknown
            // values in pass-1 almost always come from a forward fn
            // ref whose return type hasn't been registered yet;
            // pass-2 re-runs this check with all defs visible and
            // fires the diagnostic for any GENUINE mismatch.  Mirrors
            // the pass-1 tolerance in `field()` / `parse_index()`
            // that closed @P281 / @P278 (same architectural fix:
            // pass-1 mustn't emit errors pass-2 will naturally
            // resolve).  `set_field_no_check` still runs so codegen
            // stays consistent with pass-2.
            if (!self.first_pass || !exp_tp.is_unknown()) && !self.convert(value, exp_tp, &td) {
                // Plan-07 phase 6 (partial) — name the value side first
                // ("cannot assign <got> to <expected>"), the field-type
                // side last.  Old shape "Cannot write {field_type} on
                // field {S}.{f}:{value_type}" used a colon that read
                // as "field declared as <value_type>" — backwards.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot assign {} to field {}.{field} of type {}",
                    exp_tp.show(&self.data, &self.vars),
                    self.data.def(td_nr).name(),
                    td.show(&self.data, &self.vars)
                );
            }
            list.push(self.set_field_no_check(td_nr, nr, 0, code.clone(), value.clone()));
        }
    }

    pub(crate) fn parse_enum_field(
        &mut self,
        list: &mut Vec<Value>,
        into: Value,
        d_nr: u32,
        pos: u16,
        enum_nr: u8,
    ) {
        let e_nr = self
            .data
            .def_nr(&self.data.def(d_nr).attributes()[enum_nr as usize - 1].name);
        let tp = self.data.def(e_nr).returned().clone();
        let v = self.create_unique("enum", &tp);
        let mut cd = if pos != 0 {
            list.push(v_set(
                v,
                self.cl("OpGetField", &[into, Value::Int(i32::from(pos))]),
            ));
            Value::Var(v)
        } else {
            into.clone()
        };
        self.parse_object(e_nr, &mut cd);
        if let Value::Insert(ls) = &cd {
            for l in ls {
                list.push(l.clone());
            }
        }
    }
}
