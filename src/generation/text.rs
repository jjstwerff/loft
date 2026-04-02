// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Text and format-string code generation helpers.

use crate::data::{Data, Type, Value};
use std::io::Write;

use super::{Output, sanitize};

/// O7: count the number of consecutive format/append ops in `ops[start..]`,
/// skipping `Value::Line` nodes.  Used by `clear_stack_text` to emit a
/// `String::with_capacity` hint when ≥ 2 segments are present.
pub(super) fn count_format_ops(ops: &[Value], start: usize, data: &Data) -> usize {
    ops[start..]
        .iter()
        .filter(|v| !matches!(v, Value::Line(_)))
        .take_while(|v| {
            if let Value::Call(d, _) = v {
                let name = &data.def(*d).name;
                name.starts_with("OpFormat") || name.starts_with("OpAppend")
            } else {
                false
            }
        })
        .count()
}

impl Output<'_> {
    pub(super) fn clear_vector(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
    ) -> std::io::Result<()> {
        if let [Value::Var(nr)] = vals {
            let v_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            write!(
                w,
                "if var_{v_nr}.rec != 0 {{ vector::clear_vector(&var_{v_nr}, &mut stores.allocations); }}"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit a single key value as a typed `Content::…` constructor.
    /// `type_nr` is from a `Key` struct; sign indicates sort direction (ignored here),
    /// absolute value indicates the data type:
    /// 1 = integer, 2 = long, 3 = f32, 4 = f64, 5 = bool, 6 = text, 7 = byte.
    pub(super) fn emit_content(
        &mut self,
        w: &mut dyn Write,
        v: &Value,
        type_nr: i8,
    ) -> std::io::Result<()> {
        let expr = self.generate_expr_buf(v)?;
        match type_nr.unsigned_abs() {
            1 | 5 | 7 => write!(w, "Content::Long({expr} as i64)"),
            2 => write!(w, "Content::Long({expr})"),
            3 => write!(w, "Content::Single({expr})"),
            4 => write!(w, "Content::Float({expr})"),
            6 => write!(w, "Content::Str(Str::new(&*({expr})))"),
            _ => write!(w, "Content::Long(0)"),
        }
    }

    /// Use this to emit `OpClearStackText` as a `.clear()` call on the target string variable.
    ///
    /// O7: when the block lookahead (`next_format_count`) indicates ≥ 2 format/append ops
    /// follow, emits a `with_capacity` guard instead of a bare `.clear()` to avoid repeated
    /// `String` reallocations during format-string assembly (significant in WASM linear memory).
    pub(super) fn clear_stack_text(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
    ) -> std::io::Result<()> {
        if let [Value::Var(nr)] = vals {
            let variables = &self.data.def(self.def_nr).variables;
            let s_nr = sanitize(variables.name(*nr));
            // When the variable is a `&mut String` parameter (RefVar(Text)), the capacity
            // re-allocation assignment needs an explicit dereference; auto-deref does not
            // apply to assignment left-hand sides in Rust.
            let deref =
                if variables.is_argument(*nr) && matches!(variables.tp(*nr), Type::RefVar(_)) {
                    "*"
                } else {
                    ""
                };
            let n = self.next_format_count;
            self.next_format_count = 0;
            if n > 1 {
                // avg_element_len = 8 is a conservative estimate for mixed text/integer fields.
                write!(
                    w,
                    "{{ let _cap = {n}_usize * 8; \
                     if var_{s_nr}.capacity() < _cap \
                     {{ {deref}var_{s_nr} = String::with_capacity(_cap); }} \
                     else {{ var_{s_nr}.clear(); }} }}"
                )?;
            } else {
                write!(w, "var_{s_nr}.clear()")?;
            }
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpAppendCharacter` with a null-character guard,
    /// because loft represents characters as integers and zero means no character.
    pub(super) fn append_character(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
    ) -> std::io::Result<()> {
        if let [Value::Var(nr), val] = vals {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            write!(
                w,
                "{{let c = {val_expr}; if c != 0 {{ var_{s_nr}.push(ops::to_char(c)); }} }}"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpAppendText` as a `+=` on the target string variable.
    pub(super) fn append_text(&mut self, w: &mut dyn Write, vals: &[Value]) -> std::io::Result<()> {
        if let [Value::Var(nr), val] = vals {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            write!(w, "var_{s_nr} += &*({val_expr})")?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpFormatText`/`OpFormatStackText` as a call to `ops::format_text`.
    pub(super) fn format_text(&mut self, w: &mut dyn Write, vals: &[Value]) -> std::io::Result<()> {
        if let [
            Value::Var(nr),
            val,
            width,
            Value::Int(dir),
            Value::Int(token),
        ] = vals
        {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            // All text-returning calls produce either `Str` or `String` (never `&str`).
            // Wrap with `&*` so `format_text` (which expects `&str`) always gets the right type.
            // `&*Str` and `&*String` both deref to `&str` via their `Deref<Target=str>` impls.
            let val_str = if let Value::Call(d, _) = val
                && matches!(self.data.def(*d).returned, Type::Text(_))
            {
                format!("&*({val_expr})")
            } else if let Value::CallRef(v_nr, _) = val
                && let Type::Function(_, ret, _) = self.data.def(self.def_nr).variables.tp(*v_nr)
                && matches!(**ret, Type::Text(_))
            {
                format!("&*({val_expr})")
            } else {
                val_expr
            };
            let width_expr = self.generate_expr_buf(width)?;
            write!(
                w,
                "ops::format_text(&mut var_{s_nr}, {val_str}, {width_expr}, {dir}, {token})"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpFormatLong` as a call to `ops::format_long`.
    pub(super) fn format_long(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
        stack: bool,
    ) -> std::io::Result<()> {
        if let [
            Value::Var(nr),
            val,
            Value::Int(radix),
            width,
            Value::Int(token),
            Value::Boolean(plus),
            Value::Boolean(note),
            Value::Int(dir),
        ] = vals
        {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            let width_expr = self.generate_expr_buf(width)?;
            let prefix = if stack { "" } else { "&mut " };
            write!(
                w,
                "ops::format_long({prefix}var_{s_nr}, {val_expr}, {radix} as u8, {width_expr}, {token} as u8, {plus}, {note}, {dir} as i8)"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    pub(super) fn format_float(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
        stack: bool,
    ) -> std::io::Result<()> {
        if let [Value::Var(nr), val, width, prec, dir] = vals {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            let width_expr = self.generate_expr_buf(width)?;
            let prec_expr = self.generate_expr_buf(prec)?;
            let dir_expr = self.generate_expr_buf(dir)?;
            let prefix = if stack { "" } else { "&mut " };
            write!(
                w,
                "ops::format_float({prefix}var_{s_nr}, {val_expr}, {width_expr}, {prec_expr}, {dir_expr} as i8)"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpFormatSingle`/`OpFormatStackSingle` as a call to `ops::format_single`.
    pub(super) fn format_single(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
        stack: bool,
    ) -> std::io::Result<()> {
        if let [Value::Var(nr), val, width, prec, dir] = vals {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            let width_expr = self.generate_expr_buf(width)?;
            let prec_expr = self.generate_expr_buf(prec)?;
            let dir_expr = self.generate_expr_buf(dir)?;
            let prefix = if stack { "" } else { "&mut " };
            write!(
                w,
                "ops::format_single({prefix}var_{s_nr}, {val_expr}, {width_expr}, {prec_expr}, {dir_expr} as i8)"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }
}
