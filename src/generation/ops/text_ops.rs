// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Text / format / buffer Op emitters — the refvar-dependent family.
//!
//! Relocated VERBATIM from the `output_call_inner` match (`src/generation/
//! dispatch.rs`) to retire that match.  These ops delegate to the
//! `Output::format_*` / `append_*` / `clear_*` helpers (`src/generation/text.rs`)
//! and depend on the @P283 refvar→`Stack` rewrite: the ORIGINAL op name
//! (`ctx.def_fn.name`) sets each helper's `stack` flag, while the rewritten
//! `dispatch` name selects the case.  This single emitter reproduces that
//! rewrite internally + the whole sub-match, and is registered for every name
//! below (base + `Stack` variants) so the registry-first guard routes them here.

use super::{EmitCtx, OpEmitter};
use crate::data::{Type, Value};
use std::io;

pub struct TextDispatchEmitter;

impl OpEmitter for TextDispatchEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let name = ctx.def_fn.name().to_string();
        // @P283 — mirror src/state/codegen.rs: a first arg that is a Var of type
        // RefVar(Text) (a `&mut String` work-buffer) rewrites the op to its
        // `Stack` variant for case selection.
        let refvar_text_first = matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if {
            matches!(
                ctx.output.data.def(ctx.output.def_nr).variables().tp(*v),
                Type::RefVar(inner) if matches!(**inner, Type::Text(_))
            )
        });
        let dispatch: &str = if refvar_text_first {
            super::refvar_text_stack_variant(&name).unwrap_or(&name)
        } else {
            &name
        };
        match dispatch {
            "OpFormatInt" | "OpFormatStackInt" => {
                ctx.output
                    .format_long(&mut *ctx.w, args, name == "OpFormatStackInt")
            }
            "OpFormatFloat" | "OpFormatStackFloat" => {
                ctx.output
                    .format_float(&mut *ctx.w, args, name == "OpFormatStackFloat")
            }
            "OpFormatSingle" | "OpFormatStackSingle" => {
                ctx.output
                    .format_single(&mut *ctx.w, args, name == "OpFormatStackSingle")
            }
            "OpFormatText" | "OpFormatStackText" => ctx.output.format_text(&mut *ctx.w, args),
            "OpAppendText" => ctx.output.append_text(&mut *ctx.w, args),
            "OpAppendStackText" => {
                write!(ctx.w, "*")?;
                ctx.output.append_text(&mut *ctx.w, args)
            }
            "OpAppendCharacter" | "OpAppendStackCharacter" => {
                ctx.output.append_character(&mut *ctx.w, args)
            }
            "OpClearStackText" | "OpClearText" => ctx.output.clear_stack_text(&mut *ctx.w, args),
            "OpClearVector" => ctx.output.clear_vector(&mut *ctx.w, args),
            "OpAppendVector" => ctx.output.append_vector(&mut *ctx.w, args),
            "OpFreeText" | "OpCreateStack" => Ok(()),
            "OpFormatDatabase" | "OpFormatStackDatabase" => {
                // OpFormatDatabase takes a &mut String as the output buffer.
                if let [work_val, record_val, tp_val, fmt_val] = args {
                    write!(ctx.w, "OpFormatDatabase(cell,&mut ")?;
                    // work_val is Var(nr) — strip the leading & that emit() adds.
                    if let Value::Var(nr) = work_val {
                        let nm = super::super::sanitize(
                            ctx.output.data.def(ctx.output.def_nr).variables().name(*nr),
                        );
                        write!(ctx.w, "var_{nm}")?;
                    } else {
                        ctx.emit(work_val)?;
                    }
                    write!(ctx.w, ", ")?;
                    ctx.emit(record_val)?;
                    write!(ctx.w, ", ")?;
                    ctx.emit_i32_slot(tp_val)?;
                    write!(ctx.w, ", ")?;
                    ctx.emit_i32_slot(fmt_val)?;
                    write!(ctx.w, ")")?;
                }
                Ok(())
            }
            // Registered only for the names above, so this is unreachable; fall
            // back to the template path for safety.
            _ => super::default::DefaultEmitter.emit(ctx, args),
        }
    }
}
