// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Function call code generation: user-defined functions and `#rust` template calls.

use crate::data::{Context, Data, Definition, Type, Value};
use std::io::Write;

use super::{Output, narrow_int_cast, rust_type, sanitize};

/// Check if a Value tree contains a Call to OpDatabase (which mutates stores).
pub(super) fn contains_op_database(val: &Value, data: &Data) -> bool {
    match val {
        Value::Call(d_nr, args) => {
            if data.def(*d_nr).name == "OpDatabase" {
                return true;
            }
            args.iter().any(|a| contains_op_database(a, data))
        }
        Value::Block(bl) => bl.operators.iter().any(|v| contains_op_database(v, data)),
        Value::Insert(ops) => ops.iter().any(|v| contains_op_database(v, data)),
        Value::Set(_, to) => contains_op_database(to, data),
        Value::If(t, a, b) => {
            contains_op_database(t, data)
                || contains_op_database(a, data)
                || contains_op_database(b, data)
        }
        _ => false,
    }
}

impl Output<'_> {
    pub(super) fn output_call_user_fn(
        &mut self,
        w: &mut dyn Write,
        def_fn: &Definition,
        vals: &[Value],
    ) -> std::io::Result<()> {
        // N8b.2: generator function calls produce a DbRef via the coroutine table.
        let is_generator = matches!(def_fn.returned, Type::Iterator(_, _));
        if is_generator {
            write!(w, "loft::codegen_runtime::alloc_coroutine(")?;
        }
        write!(w, "{}(stores", def_fn.name)?;
        for (idx, v) in vals.iter().enumerate() {
            write!(w, ", ")?;
            if let Some(vr) = self.create_stack_var(v) {
                let name = sanitize(self.data.def(self.def_nr).variables.name(vr));
                write!(w, "&mut var_{name}")?;
            } else {
                // wrap i32 literal into (u32, null_DbRef) for fn-ref params.
                let param_is_fnref = idx < def_fn.attributes.len()
                    && matches!(def_fn.attributes[idx].typedef, Type::Function(_, _, _));
                let param_is_routine = idx < def_fn.attributes.len()
                    && matches!(def_fn.attributes[idx].typedef, Type::Routine(_));
                if param_is_fnref && matches!(v, Value::Int(_)) {
                    let mut buf = Vec::new();
                    self.output_code_inner(&mut buf, v)?;
                    let s = String::from_utf8(buf).unwrap();
                    write!(
                        w,
                        "({s} as u32, loft::keys::DbRef {{ store_nr: u16::MAX, rec: 0, pos: 0 }})"
                    )?;
                } else if param_is_routine && matches!(v, Value::Int(_)) {
                    let mut buf = Vec::new();
                    self.output_code_inner(&mut buf, v)?;
                    let s = String::from_utf8(buf).unwrap();
                    write!(w, "{s} as u32")?;
                } else {
                    // B7-native: text-returning user fn calls produce `Str`,
                    // but callees expect `&str`.  Wrap with `&*` to deref.
                    let needs_deref = idx < def_fn.attributes.len()
                        && matches!(def_fn.attributes[idx].typedef, Type::Text(_))
                        && matches!(v, Value::Call(d, _) if
                            matches!(self.data.def(*d).returned, Type::Text(_))
                            && self.data.def(*d).rust.is_empty()
                            && !self.data.def(*d).name.starts_with("Op"));
                    if needs_deref {
                        write!(w, "&*(")?;
                    }
                    self.output_code_inner(w, v)?;
                    if needs_deref {
                        write!(w, ")")?;
                    }
                }
            }
        }
        write!(w, ")")?;
        if is_generator {
            write!(w, ")")?; // close alloc_coroutine(...)
        } else if narrow_int_cast(&def_fn.returned).is_some() {
            // Narrow integer return types (u8/u16/i8/i16) must be widened to i32 so that
            // assignments and comparisons with i32 expressions type-check in Rust.
            write!(w, " as i32")?;
        }
        Ok(())
    }

    /// Use this to inline a `#rust` template operator by substituting `@param` placeholders
    /// with generated argument expressions.
    #[allow(clippy::too_many_lines)]
    pub(super) fn output_call_template(
        &mut self,
        w: &mut dyn Write,
        def_fn: &Definition,
        vals: &[Value],
    ) -> std::io::Result<()> {
        let mut res = def_fn.rust.clone();
        // Bytecode templates wrap text values in Str::new(...) for put_stack compatibility.
        // Native code uses &str directly — strip the wrapper by extracting its argument.
        // Must be done before @param substitution so argument expressions are not affected.
        while let Some(start) = res.find("Str::new(") {
            let arg_start = start + "Str::new(".len();
            let mut depth = 1usize;
            let mut end = arg_start;
            for (i, c) in res[arg_start..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = arg_start + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            res = format!(
                "{}{}{}",
                &res[..start],
                &res[arg_start..end],
                &res[end + 1..]
            );
        }
        for (a_nr, a) in def_fn.attributes.iter().enumerate() {
            let name = "@".to_string() + &a.name;
            if a_nr < vals.len() {
                // For enum-typed parameters, Value::Null means the null enum byte (255).
                if matches!(a.typedef, Type::Enum(_, _, _)) && matches!(vals[a_nr], Value::Null) {
                    res = res.replace(&name, "(255u8)");
                    continue;
                }
                // For reference-typed parameters, Value::Null means the null DbRef sentinel.
                if matches!(
                    a.typedef,
                    Type::Reference(_, _)
                        | Type::Vector(_, _)
                        | Type::Sorted(_, _, _)
                        | Type::Hash(_, _, _)
                        | Type::Index(_, _, _)
                        | Type::Enum(_, true, _)
                ) && matches!(vals[a_nr], Value::Null)
                {
                    res = res.replace(&name, "(DbRef { store_nr: u16::MAX, rec: 0, pos: 8 })");
                    continue;
                }
                // For character-typed parameters, Value::Int means a character code point.
                if matches!(a.typedef, Type::Character)
                    && let Value::Int(n) = vals[a_nr]
                {
                    let with = format!("char::from_u32({n}_u32).unwrap_or('\\0')");
                    res = res.replace(&name, &format!("({with})"));
                    continue;
                }
                // For character-typed parameters, a variable holding an i32 char needs
                // ops::to_char() because the template expects a `char`, not `i32`.
                if matches!(a.typedef, Type::Character)
                    && let Value::Var(n) = vals[a_nr]
                    && matches!(self.data.def(self.def_nr).variables.tp(n), Type::Character)
                {
                    let inner = self.generate_expr_buf(&vals[a_nr])?;
                    res = res.replace(&name, &format!("(ops::to_char({inner}))"));
                    continue;
                }
                // For character-typed parameters, a call returning character yields `i32`
                // (due to the `as u32 as i32` auto-cast), so wrap with ops::to_char().
                if matches!(a.typedef, Type::Character)
                    && let Value::Call(d, _) = &vals[a_nr]
                    && matches!(self.data.def(*d).returned, Type::Character)
                {
                    let inner = self.generate_expr_buf(&vals[a_nr])?;
                    res = res.replace(&name, &format!("(ops::to_char({inner}))"));
                    continue;
                }
                // Text-typed parameters: all text-returning calls produce `Str` or `String`,
                // but templates expect `&str`. Deref with `&*` to get `&str` in all cases.
                if matches!(a.typedef, Type::Text(_))
                    && let Value::Call(d, _) = &vals[a_nr]
                    && matches!(self.data.def(*d).returned, Type::Text(_))
                {
                    let inner = self.generate_expr_buf(&vals[a_nr])?;
                    res = res.replace(&name, &format!("(&*({inner}))"));
                    continue;
                }
                let mut with = self.generate_expr_buf(&vals[a_nr])?;
                // Integer parameter receiving a char value needs explicit cast.
                if matches!(a.typedef, Type::Integer(_, _, _)) {
                    let val_is_char = match &vals[a_nr] {
                        Value::Var(n) => {
                            matches!(self.data.def(self.def_nr).variables.tp(*n), Type::Character)
                        }
                        Value::Call(d, _) => {
                            matches!(self.data.def(*d).returned, Type::Character)
                        }
                        _ => false,
                    };
                    if val_is_char {
                        with += " as u32 as i32";
                    }
                }
                // Templates use u32::from(@name) for field offsets; that was written for u16
                // parameters (fill.rs).  Native codegen emits i32 literals, so substitute the
                // entire u32::from(@name) pattern with (@value) as u32 to stay type-correct.
                let u32_from_pat = format!("u32::from({name})");
                if res.contains(&u32_from_pat) {
                    res = res.replace(&u32_from_pat, &format!("({with}) as u32"));
                } else {
                    // When the template parameter expects a narrow unsigned integer (u8/u16),
                    // native codegen emits i32 literals.  Add a cast so the types match.
                    // Use Context::Result to get the precise narrow type (e.g. u16) since
                    // Context::Variable returns i32 for narrow integers.
                    let tp_str = rust_type(&a.typedef, &Context::Result);
                    if matches!(tp_str.as_str(), "u8" | "u16") {
                        let typed_with = if with.ends_with("_i32") {
                            format!("{}_{tp_str}", &with[..with.len() - 4])
                        } else {
                            format!("({with}) as {tp_str}")
                        };
                        res = res.replace(&name, &format!("({typed_with})"));
                    } else {
                        res = res.replace(&name, &format!("({with})"));
                    }
                }
            } else {
                println!(
                    "Problem def_fn {def_fn} attributes {:?} vals {vals:?}",
                    def_fn.attributes
                );
                break;
            }
        }
        // Templates use `s.database.` and `s.` for bytecode interpreter (State).
        // In generated native code, `stores` is the direct Stores reference.
        res = res.replace("s.database.", "stores.");
        res = res.replace("s.db_from_text(", "db_from_text(stores, ");
        res = res.replace("crate::state::", "loft::state::");
        // loft represents `character` as `i32`; template functions that return `char`
        // (like `ops::text_character`) need an explicit cast at the call site.
        // Narrow integer returns (u8/u16/i8/i16) must be widened to i32 so that
        // pre-eval bindings (`let _pre_N = narrow_func()`) do not cause type-mismatch
        // errors when compared against i32 literals.
        // Multi-statement template bodies (containing `;`) are wrapped in `{...}` so
        // they are valid in expression position when inlined as function arguments.
        if matches!(def_fn.returned, Type::Character) {
            write!(w, "({res}) as u32 as i32")
        } else if narrow_int_cast(&def_fn.returned).is_some() {
            if res.contains(';') {
                write!(w, "({{{res}}}) as i32")
            } else {
                write!(w, "({res}) as i32")
            }
        } else if res.contains(';') {
            write!(w, "{{{res}}}")
        } else {
            write!(w, "{res}")
        }
    }
}
