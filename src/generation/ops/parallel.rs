// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan 09 phase 03 — parallel-for emitter family.
//!
//! Replaces the 95-line special case in
//! `src/generation/dispatch.rs` (the `"n_parallel_for" |
//! "n_parallel_for_light" =>` arm) with a per-Op `OpEmitter` impl.
//!
//! What the emission does (unchanged from the pre-phase-03 special
//! case):
//!
//! 1. Pull the worker fn def out of `vals[4]` (an i32 d_nr literal)
//!    and read its return type.
//! 2. Pick which native helper to call (`n_parallel_for_native`,
//!    `n_parallel_for_ref_native`, or `n_parallel_for_text_native`)
//!    based on whether the worker returns text, heap-typed, or scalar.
//! 3. Synthesise extra-arg `let _ex0 = …;` bindings so the closure
//!    can capture them.
//! 4. Emit the parallel-helper call with a closure of the correct
//!    shape (text / heap-ref / float / scalar).
//! 5. Close N braces matching the let-bindings.
//!
//! The emitter takes over because of phase 00 step 0.6's registry-
//! first guard at the top of `output_call_inner`: when this emitter
//! is registered for `n_parallel_for` / `n_parallel_for_light`,
//! dispatch routes here BEFORE the legacy match arm runs.  Phase 03
//! deletes the legacy match arm; this file becomes the sole
//! emission site for parallel-for codegen.
//!
//! Phase 06 (P202 — threading queue runtime fns) extends this
//! family with `n_parallel_queue*` emitters that reuse the helper
//! functions defined here.

use super::{EmitCtx, OpEmitter};
use crate::data::{Type, Value};
use std::io;

/// Closure shape for the worker — determines the conversion applied
/// to the worker's return value before it's stored in the result vec.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ClosureShape {
    /// Worker returns `String`; closure synthesises a write buffer.
    Text,
    /// Worker returns `DbRef` (heap struct / struct-enum / vector / etc.).
    HeapRef,
    /// Worker returns `f64` / `f32`; closure converts via `to_bits() as i64`.
    Float,
    /// Worker returns scalar integer / boolean; closure casts `as i64`.
    Scalar,
}

fn closure_shape(ret: &Type) -> ClosureShape {
    if matches!(ret, Type::Text(_)) {
        ClosureShape::Text
    } else if ret.heap_def_nr().is_some() {
        ClosureShape::HeapRef
    } else if matches!(ret, Type::Float | Type::Single) {
        ClosureShape::Float
    } else {
        ClosureShape::Scalar
    }
}

fn helper_name(shape: ClosureShape) -> &'static str {
    match shape {
        ClosureShape::Text => "n_parallel_for_text_native",
        ClosureShape::HeapRef => "n_parallel_for_ref_native",
        // Float and Scalar both use the primitive helper — they
        // differ only in the closure body's return-value transform.
        ClosureShape::Float | ClosureShape::Scalar => "n_parallel_for_native",
    }
}

/// Phase 06 — runtime helper-name selection for the queue family.
/// Mirrors `helper_name` but maps to the `n_parallel_queue_*_native`
/// fns introduced in plan-09 phase 06 (P202 close).
fn queue_helper_name(shape: ClosureShape) -> &'static str {
    match shape {
        ClosureShape::Text => "n_parallel_queue_text_native",
        ClosureShape::HeapRef => "n_parallel_queue_ref_native",
        ClosureShape::Float | ClosureShape::Scalar => "n_parallel_queue_native",
    }
}

/// Plan-06 ARC.md A3 — narrow-Integer queue helper name.  Narrow
/// integer returns (byte_width 1/2/4) need the byte-packing variant
/// instead of the wide `Vec<u64>` queue.
fn queue_narrow_helper_name() -> &'static str {
    "n_parallel_queue_narrow_native"
}

/// True when the worker's return type is a narrow Integer
/// (byte_width 1/2/4).  Used by `ParallelQueueEmitter` to swap the
/// helper to the narrow variant.
fn is_narrow_int_return(ret: &Type) -> bool {
    matches!(ret, Type::Integer(spec) if matches!(spec.byte_width(true), 1 | 2 | 4))
}

/// `n_parallel_for` / `n_parallel_for_light` emitter.
///
/// Lifts the legacy match-arm body verbatim into a custom emitter
/// while producing byte-identical output.
pub struct ParallelForEmitter;

impl OpEmitter for ParallelForEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Guard: the special case requires at least 5 args with vals[4]
        // a non-negative i32 (the worker fn's def_nr).  When violated,
        // fall through to the default emitter — preserves pre-phase-03
        // behaviour where the special case `if` clause didn't match.
        let fn_d_nr = match args.get(4) {
            Some(Value::Int(d)) if *d >= 0 => (*d).cast_unsigned(),
            _ => {
                return super::default::DefaultEmitter.emit(ctx, args);
            }
        };
        if args.len() < 5 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }

        let worker_def = ctx.output.data.def(fn_d_nr);
        let worker_name = worker_def.name.clone();
        let worker_ret = worker_def.returned.clone();
        let shape = closure_shape(&worker_ret);

        // Extra context args: vals[5..len-1].  The trailing element
        // is `n_extra` (the count); we don't read it directly because
        // arg arithmetic gives us the same number.
        let n_extra = if args.len() > 6 { args.len() - 6 } else { 0 };

        // Emit let-bindings for extra args so they're captured by name
        // (and not re-evaluated) inside the closure.  Each binding
        // wraps the rest of the emission in a `{ … }` block; the
        // matching `}` characters are emitted at the end.
        for i in 0..n_extra {
            write!(ctx.w, "{{ let _ex{i} = ")?;
            ctx.emit(&args[5 + i])?;
            write!(ctx.w, "; ")?;
        }

        // Helper call: `name(cell, input, elem_size, …, threads`
        let par_fn = helper_name(shape);
        write!(ctx.w, "{par_fn}(cell, ")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[1])?;
        write!(ctx.w, ", ")?;
        if shape == ClosureShape::HeapRef {
            // Ref mode: emit struct_size and known_type instead of
            // return_size.  Both Type::Reference and the heap-allocated
            // struct-enum (`Type::Enum(_, true, _)`) route here;
            // `heap_def_nr()` returns the def for both.
            let (struct_size, known_type) = if let Some(d_nr) = worker_ret.heap_def_nr() {
                let kt = ctx.output.data.def(d_nr).known_type;
                (i32::from(ctx.output.stores.size(kt)), i32::from(kt))
            } else {
                (0, 0)
            };
            write!(ctx.w, "{struct_size}, {known_type}, ")?;
        } else {
            ctx.emit_i32_slot(&args[2])?;
            write!(ctx.w, ", ")?;
        }
        ctx.emit_i32_slot(&args[3])?;

        // Closure args: `, _ex0, _ex1, …` appended after `elm`.
        let extras = {
            use std::fmt::Write as _;
            let mut s = String::new();
            for i in 0..n_extra {
                write!(s, ", _ex{i}").unwrap();
            }
            s
        };

        // Closure body — return-shape-specific.  P199 — worker
        // closures receive `&UnsafeCell<Stores>` from the parallel-
        // runner helpers; user-fn calls take `cell`, so the closure
        // parameter is named `cell` and threaded through verbatim.
        match shape {
            ClosureShape::Text => write!(
                ctx.w,
                ", |cell, elm| {{ let mut _w = String::new(); {worker_name}(cell, elm{extras}, &mut _w); _w }})"
            )?,
            ClosureShape::HeapRef => write!(
                ctx.w,
                ", |cell, elm| {{ {worker_name}(cell, elm{extras}) }})"
            )?,
            ClosureShape::Float => write!(
                ctx.w,
                ", |cell, elm| {{ {worker_name}(cell, elm{extras}).to_bits() as i64 }})"
            )?,
            ClosureShape::Scalar => {
                // Plan-06 ARC.md A3.5 — Boolean returns map to Rust
                // `bool`, which cannot cast directly to i64; insert
                // an `as u8` bridge.  Other Scalar shapes (Integer
                // narrow / wide, Character → i32, Enum-no-payload →
                // u8) all support `as i64` natively.
                if matches!(worker_ret, Type::Boolean) {
                    write!(
                        ctx.w,
                        ", |cell, elm| {{ {worker_name}(cell, elm{extras}) as u8 as i64 }})"
                    )?;
                } else {
                    write!(
                        ctx.w,
                        ", |cell, elm| {{ {worker_name}(cell, elm{extras}) as i64 }})"
                    )?;
                }
            }
        }

        // Close the `{ let _ex0 = … ;` blocks opened above.
        for _ in 0..n_extra {
            write!(ctx.w, " }}")?;
        }
        Ok(())
    }
}

/// Phase 06 — `n_parallel_queue` / `_text` / `_ref` emitter family.
///
/// Closes P202 (native missing `n_parallel_queue` family).  Mirrors
/// `ParallelForEmitter`'s closure-shape logic but routes calls to
/// `n_parallel_queue_*_native` runtime fns (defined in
/// `src/codegen_runtime.rs`).  Queue variants return `i64` (row count)
/// instead of `DbRef`; the result feeds a fused for-loop that reads
/// rows via `n_parallel_buf_get_*_native`.
///
/// The arg layout is identical to `n_parallel_for` (input, elem_size,
/// return_size, threads, func, extras..., n_extra) so the emitter
/// reuses the same parsing + closure-emission scaffolding.
pub struct ParallelQueueEmitter;

impl OpEmitter for ParallelQueueEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Guard: same as for-par — at least 5 args with vals[4] a
        // non-negative i32 (worker fn's def_nr).  Otherwise fall through.
        let fn_d_nr = match args.get(4) {
            Some(Value::Int(d)) if *d >= 0 => (*d).cast_unsigned(),
            _ => {
                return super::default::DefaultEmitter.emit(ctx, args);
            }
        };
        if args.len() < 5 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }

        let worker_def = ctx.output.data.def(fn_d_nr);
        let worker_name = worker_def.name.clone();
        let worker_ret = worker_def.returned.clone();
        let shape = closure_shape(&worker_ret);

        // Extras: args[5..len-1]; trailing args[len-1] is the n_extra count.
        let n_extra = if args.len() > 6 { args.len() - 6 } else { 0 };

        for i in 0..n_extra {
            write!(ctx.w, "{{ let _ex{i} = ")?;
            ctx.emit(&args[5 + i])?;
            write!(ctx.w, "; ")?;
        }

        // Plan-06 ARC.md A3 — narrow-Integer returns route through
        // `n_parallel_queue_narrow_native` (byte-packed buffer);
        // wide / non-Integer scalars stay on `n_parallel_queue_native`.
        let par_fn = if is_narrow_int_return(&worker_ret) {
            queue_narrow_helper_name()
        } else {
            queue_helper_name(shape)
        };
        write!(ctx.w, "{par_fn}(cell, ")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[1])?;
        write!(ctx.w, ", ")?;
        if shape == ClosureShape::HeapRef {
            let (struct_size, known_type) = if let Some(d_nr) = worker_ret.heap_def_nr() {
                let kt = ctx.output.data.def(d_nr).known_type;
                (i32::from(ctx.output.stores.size(kt)), i32::from(kt))
            } else {
                (0, 0)
            };
            write!(ctx.w, "{struct_size}, {known_type}, ")?;
        } else {
            ctx.emit_i32_slot(&args[2])?;
            write!(ctx.w, ", ")?;
        }
        ctx.emit_i32_slot(&args[3])?;

        let extras = {
            use std::fmt::Write as _;
            let mut s = String::new();
            for i in 0..n_extra {
                write!(s, ", _ex{i}").unwrap();
            }
            s
        };

        match shape {
            ClosureShape::Text => write!(
                ctx.w,
                ", |cell, elm| {{ let mut _w = String::new(); {worker_name}(cell, elm{extras}, &mut _w); _w }})"
            )?,
            ClosureShape::HeapRef => write!(
                ctx.w,
                ", |cell, elm| {{ {worker_name}(cell, elm{extras}) }})"
            )?,
            ClosureShape::Float => write!(
                ctx.w,
                ", |cell, elm| {{ {worker_name}(cell, elm{extras}).to_bits() as i64 }})"
            )?,
            ClosureShape::Scalar => {
                // Plan-06 ARC.md A3.5 — Boolean returns map to Rust
                // `bool`, which cannot cast directly to i64; insert
                // an `as u8` bridge.  Other Scalar shapes (Integer
                // narrow / wide, Character → i32, Enum-no-payload →
                // u8) all support `as i64` natively.
                if matches!(worker_ret, Type::Boolean) {
                    write!(
                        ctx.w,
                        ", |cell, elm| {{ {worker_name}(cell, elm{extras}) as u8 as i64 }})"
                    )?;
                } else {
                    write!(
                        ctx.w,
                        ", |cell, elm| {{ {worker_name}(cell, elm{extras}) as i64 }})"
                    )?;
                }
            }
        }

        for _ in 0..n_extra {
            write!(ctx.w, " }}")?;
        }
        Ok(())
    }
}

/// Phase 06 — `n_parallel_buf_get` / `_text` / `_ref` and
/// `n_parallel_buf_drop` / `_text` / `_ref` emitter.  Renames the
/// call site to the corresponding `_native` runtime fn; args pass
/// through unchanged.
///
/// These fns have no closure transformation — they're plain reads
/// from / pops of the active par-buffer.  The emitter exists only
/// to disambiguate the runtime fn name from the loft-side stub
/// (which has a `todo!()` body in generated Rust).
pub struct ParallelBufRenameEmitter;

impl OpEmitter for ParallelBufRenameEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let native_name = format!("{}_native", ctx.def_fn.name);
        write!(ctx.w, "{native_name}(cell")?;
        for arg in args {
            write!(ctx.w, ", ")?;
            ctx.emit(arg)?;
        }
        write!(ctx.w, ")")
    }
}
