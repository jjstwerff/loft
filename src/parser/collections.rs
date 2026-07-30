// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    Context, DefType, I32, Level, LexItem, OutputState, Parser, Parts, Type, Value,
    diagnostic_format, v_block, v_if, v_loop, v_set, var_size,
};
use crate::variables::Function;

/// True when `v` (unspanned) is a CAPTURING fn-ref — `Value::FnRef` whose
/// closure-var slot is set (`!= u16::MAX`).  A bare `return add5;` carries
/// `u16::MAX` (non-capturing) and is fine for par.
fn is_capturing_fnref(v: &Value) -> bool {
    match v.unspan() {
        Value::FnRef(_, clos_var, _) => *clos_var != u16::MAX,
        // A capturing lambda lowers to a `fn_ref_with_closure` block that builds
        // the closure record and yields the `FnRef` as its tail expression.
        Value::Block(bl) | Value::Loop(bl) => bl.operators.last().is_some_and(is_capturing_fnref),
        Value::Insert(ops) => ops.last().is_some_and(is_capturing_fnref),
        _ => false,
    }
}

/// True when a par worker's body returns a capturing closure in a `return` or
/// tail position.  Conservative — only flags closures found directly in return
/// position, so a non-capturing fn-ref return and any closure used only
/// internally (never returned) are never rejected; an indirect return (via a
/// local variable) falls through to the runtime path rather than a false
/// positive.  Used to give a clear diagnostic instead of the dangling-ref panic.
fn worker_returns_capturing_closure(body: &Value) -> bool {
    match body.unspan() {
        Value::Return(inner) => is_capturing_fnref(inner),
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.last().is_some_and(is_capturing_fnref)
                || bl.operators.iter().any(worker_returns_capturing_closure)
        }
        Value::Insert(ops) => {
            ops.last().is_some_and(is_capturing_fnref)
                || ops.iter().any(worker_returns_capturing_closure)
        }
        Value::If(_, t, f) => {
            is_capturing_fnref(t)
                || is_capturing_fnref(f)
                || worker_returns_capturing_closure(t)
                || worker_returns_capturing_closure(f)
        }
        _ => false,
    }
}

/// Plan-06 ARC.md A3 / A3.5 — how to convert the i64 returned by
/// `parallel_buf_get_narrow` back to the worker's actual return type.
enum NarrowWrap {
    /// No conversion needed — raw i64 result is what the body
    /// expects (narrow Integer; body's `r as integer` accepts i64
    /// natively).
    None,
    /// Single-arg Op call: `OpFoo(buf_get_call) -> T`.  Used for
    /// `OpConvCharacterFromInt` (i64 → char via low 4 bytes) and
    /// `OpCastEnumFromInt` (i64 → u8 enum variant; treats i64::MIN
    /// as 255-sentinel).  Op functions are looked up by their bare
    /// name (no `n_` prefix — that's reserved for user-fn `n_<name>`).
    OpCall(&'static str),
    /// `OpNeInt(buf_get_call, 0) -> boolean`.  Boolean's existing
    /// `OpConvBoolFromInt` has null-check semantics (v != i64::MIN),
    /// not value semantics (v != 0) — the buf_get always returns
    /// 0 or 1, never i64::MIN, so the conv would yield true for
    /// both.  `OpNeInt` gives the right 0 → false / 1 → true mapping.
    NeZero,
    /// ARC.md A3.6 — use a different buf_get fn-name instead of
    /// `n_parallel_buf_get_narrow`, and skip the wrap (the named
    /// fn already returns the typed value).  Used for `single`
    /// (routes through `n_parallel_buf_get_single` returning
    /// `single` directly via `f32::from_bits`).
    TypedBufGet(&'static str),
}

/// Plan-06 ARC.md A3 / A3.5 — descriptor for routing a parallel
/// worker's narrow-primitive return through `n_parallel_queue_narrow`
/// instead of the legacy materialised-vector path.
///
/// Returned by [`narrow_route_for`] when the worker's return type is
/// representable in 1/2/4 bytes per row.  `None` otherwise (8-byte
/// integers stay on the regular `n_parallel_queue` path; reference /
/// text / fn-ref returns have their own queue variants).
struct NarrowRoute {
    /// Per-row stride in bytes (1, 2, or 4).
    width: u8,
    /// Sign-extension flag passed to `parallel_buf_get_narrow`.
    /// `true` for signed narrow Integer (i8/i16/i32) — buf_get
    /// reads via `i8`/`i16`/`i32` cast.  `false` for unsigned int,
    /// Boolean, Character, and Enum-no-payload.
    signed: bool,
    /// How to wrap the i64 buf_get result back to the worker's
    /// declared return type (see [`NarrowWrap`]).
    wrap: NarrowWrap,
}

/// Decide whether the worker's return type fits the narrow-Queue
/// route.  Mirrors the original A3 plan's `route_narrow_queue` design:
///
/// | Type                              | width | signed     | wrap                          |
/// |-----------------------------------|-------|------------|-------------------------------|
/// | `Integer(spec)` w/ byte_width 1/2/4 | spec  | spec.min<0 | `None`                        |
/// | `Boolean`                         | 1     | false      | `NeZero` (OpNeInt(_, 0))      |
/// | `Character`                       | 4     | false      | `OpCall(OpConvCharacterFromInt)` |
/// | `Enum(_, false, _)` (no payload)  | 1     | false      | `OpCall(OpCastEnumFromInt)`   |
/// | anything else                     | None — falls through to other routes                     |
///
/// 8-byte Integer + Float go through the regular `n_parallel_queue`
/// (wide u64 rows; Float reads via `parallel_buf_get_float`).
/// Single routes here with `TypedBufGet` — its f32 bit pattern fits
/// stride 4 and `parallel_buf_get_single` recovers the typed value.
fn narrow_route_for(ret_type: &Type) -> Option<NarrowRoute> {
    match ret_type {
        Type::Integer(spec) => match spec.byte_width(true) {
            w @ (1 | 2 | 4) => Some(NarrowRoute {
                width: w,
                signed: spec.min < 0,
                wrap: NarrowWrap::None,
            }),
            _ => None,
        },
        Type::Boolean => Some(NarrowRoute {
            width: 1,
            signed: false,
            wrap: NarrowWrap::NeZero,
        }),
        Type::Character => Some(NarrowRoute {
            width: 4,
            signed: false,
            wrap: NarrowWrap::OpCall("OpConvCharacterFromInt"),
        }),
        Type::Enum(_, false, _) => Some(NarrowRoute {
            width: 1,
            signed: false,
            wrap: NarrowWrap::OpCall("OpCastEnumFromInt"),
        }),
        // ARC.md A3.6 — `single` (f32) routes through the narrow
        // path with stride 4.  No wrap: the typed buf_get fn
        // returns `single` directly via `f32::from_bits` over the
        // same per-row bytes.  Symmetric with Store::set_single.
        Type::Single => Some(NarrowRoute {
            width: 4,
            signed: false,
            wrap: NarrowWrap::TypedBufGet("n_parallel_buf_get_single"),
        }),
        _ => None,
    }
}

impl Parser {
    pub(crate) fn iter_text(
        &mut self,
        code: &mut Value,
        iter_var: u16,
        pre_var: Option<u16>,
    ) -> Value {
        // iter_var is {id}#next — the post-advance byte position (loop driver).
        // pre_var  is {id}#index — saved to the start position of the current char.
        let index_var = pre_var.unwrap();
        let res_var = self
            .vars
            .unique("for_result", &Type::Character, &mut self.lexer);
        let l = self.cl("OpLengthCharacter", &[Value::Var(res_var)]);
        let next = vec![
            // Save current position as #index before advancing.
            v_set(index_var, Value::Var(iter_var)),
            v_set(
                res_var,
                // Plan-07 phase 4 step 4.8 — for-loop iteration over
                // text uses the *Nullable* peer of OpTextCharacter;
                // OOB returns char(0) which the for-loop driver uses
                // as its end-of-iteration signal.  User-facing `text[i]`
                // (parser/fields.rs:750) keeps the raising
                // OpTextCharacter.
                self.cl(
                    "OpTextCharacterNullable",
                    &[code.clone(), Value::Var(iter_var)],
                ),
            ),
            v_set(iter_var, self.cl("OpAddInt", &[Value::Var(iter_var), l])),
            Value::Var(res_var),
        ];
        // Initialise the loop driver at the outer scope.
        // The caller must separately initialise index_var at the same scope level.
        *code = v_set(iter_var, Value::Int(0));
        v_block(next, Type::Character, "for text next")
    }

    #[allow(clippy::too_many_lines)] // sorted/index/spatial iterator setup — splitting would lose context
    /// The ONE home for a vector's per-element ITERATION stride — the byte
    /// step `vector::get_vector(size, idx)` walks per element.  Both the
    /// direct for-loop emission (the `Type::Vector` arm below) and the
    /// generic-instantiation elm-size fixup (`substitute_type_in_value`)
    /// read this; a second derivation is how the generic path silently
    /// drifted (it hand-summed struct field widths → 20 where the schema
    /// strides 4 for a linked struct, so iterating a bounded-generic
    /// method's `vector<Self>` result read garbage past element 0).
    ///
    /// The decision chain, in precedence order:
    /// - linked db type (structs holding vectors/text etc.) → 4-byte rec-id
    /// - narrow-int element → its forced 1/2/4-byte storage width
    /// - anything the shared element-type resolver knows
    ///   (`Data::vector_element_type`) → that schema type's own size: a fn-ref
    ///   is a 4-byte d_nr (@P343), a NESTED vector is a 4-byte handle, a plain
    ///   leaf is its scalar width
    /// - else → the schema's `database.size(db_tp)`
    ///
    /// The nested case used to read `element_stack_size(inner).max(4)` — the
    /// INNER scalar's width, so a `vector<vector<integer>>` strode 8 over rows
    /// that are 4-byte handles.  It agreed with the storage side only while the
    /// storage side made the same mistake; reading both from one derivation is
    /// what keeps reader and writer in step (loft#624 nested).
    pub(crate) fn vector_elem_iter_stride(&mut self, vtp: &Type) -> u16 {
        let vec_tp = self.data.type_def_nr(vtp);
        let db_tp = self.data.def(vec_tp).known_type();
        if self.database.is_linked(db_tp) {
            4
        } else if let Type::Integer(spec) = vtp
            && let Some(n) = spec.vector_narrow_width()
        {
            u16::from(n)
        } else if let Some(elem) = self.data.vector_element_type(vtp, &mut self.database) {
            self.database.size(elem)
        } else {
            self.database.size(db_tp)
        }
    }

    pub(crate) fn iterator(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
        iter_var: u16,
        pre_var: Option<u16>,
    ) -> Value {
        // unwrap &vector<T> / &sorted<T> so the iterator setup
        // matches the underlying collection type.
        if let Type::RefVar(inner) = is_type {
            return self.iterator(code, inner, should, iter_var, pre_var);
        }
        if let Value::Iter(_, start, next, _) = code.clone() {
            if matches!(*next, Value::Block(_)) {
                *code = *start;
                return *next.clone();
            }
            diagnostic!(self.lexer, Level::Error, "Malformed iterator expression");
            return Value::Null;
        }
        if matches!(*is_type, Type::Text(_)) {
            return self.iter_text(code, iter_var, pre_var);
        }
        // CO1.5a: coroutine iterators (from generator function calls) need
        // a next()-based advance. Detect: the call target returns Iterator.
        if let Type::Iterator(inner, _) = is_type
            && !self.first_pass
            && let Value::Call(d_nr, _) = code.unspan()
            && matches!(self.data.def(*d_nr).returned(), Type::Iterator(_, _))
        {
            let gen_var = self.create_unique("__gen", is_type);
            self.vars.defined(gen_var);
            let gen_expr = code.clone();
            *code = v_set(gen_var, gen_expr);
            let op = self.data.def_nr("OpCoroutineNext");
            let yield_tp = (**inner).clone();
            // @P327 / @P328 native — yield-channel dispatch via packed
            // `value_size` (low byte = byte size, high byte = channel
            // tag).  Tag 0 = legacy per-type channel (next_i64 /
            // next_text / next_dbref dispatched by byte size).  Tag 1 =
            // unified `next_into` with tuple-of-(integer|float) rebuild.
            // Tag 2 = unified `next_into` with fn-ref rebuild
            // (`(u32, DbRef)` from `[i64; 2]`).  Interp masks the high
            // byte off in `fill.rs::coroutine_next` and `state/codegen.rs`'s
            // `OpCoroutineNext` arm; only native inspects it.  See
            // `plans/16-coroutine-validation/01-unified-channel.md`.
            let byte_size = i32::from(crate::variables::size(
                &yield_tp,
                &crate::data::Context::Argument,
            ));
            // @PLAN16 phase 02 — a tuple whose every element classifies into a
            // transport slot rides channel 1 (the layout-driven flatten-walk);
            // the per-slot kind codes ride as extra args so the native consumer
            // reconstructs the tuple.  `tuple_kinds` is the SAME decision the
            // producer's `is_tuple_into` makes, so the two ends agree.
            let tkinds = crate::coroutine_layout::tuple_kinds(&yield_tp);
            // #401 — one shared home for the channel decision (float/single/enum
            // need their own tags); see `coroutine_layout::channel_tag`.
            let channel_tag = crate::coroutine_layout::channel_tag(&yield_tp);
            let value_size: i32 = (channel_tag << 8) | byte_size;
            let mut call_args = vec![Value::Var(gen_var), Value::Int(value_size)];
            if let Some(kinds) = &tkinds {
                call_args.extend(kinds.iter().map(|k| Value::Int(k.code())));
            }
            return Value::Call(op, call_args);
        }
        if is_type == should {
            // Non-coroutine pre-existing iterator (sorted/hash/index).
            let orig = code.clone();
            *code = Value::Null;
            return orig;
        }
        if self.first_pass {
            self.reverse_iterator = false;
            return Value::Null;
        }
        if let Type::Iterator(_, _) = should {
            match is_type {
                Type::Vector(vtp, dep) => {
                    let i = Value::Var(iter_var);
                    let vec_tp = self.data.type_def_nr(vtp);
                    let db_tp = self.data.def(vec_tp).known_type();
                    let size = self.vector_elem_iter_stride(vtp);
                    // Plan-07 phase 4 step 4.6 — for-loop iteration uses
                    // the *Nullable* peers; OOB returns a null DbRef which
                    // the loop's pre-body null-check
                    // (parser/collections.rs:1492-1499) converts to false
                    // and breaks.  User-facing `v[i]` (parser/fields.rs:629-640)
                    // keeps the raising OpGetVector / OpVectorRef.
                    let mut ref_expr = self.cl(
                        "OpGetVectorNullable",
                        &[code.clone(), Value::Int(i32::from(size)), i.clone()],
                    );
                    // @PLN25 E2 — a `__nullable<S>` element is `Enum(synth,true)`,
                    // not `Reference`, but in a LINKED collection (an array of
                    // ref slots — e.g. a struct-field vector that shares records
                    // with a sibling hash) it is stored as a 4-byte rec-ref, so
                    // the element read must DEREF via `OpVectorRefNullable` just
                    // like a `Reference` element does.  Without this it keeps the
                    // inline `OpGetVectorNullable(size=4)` and reads the rec-id
                    // slot AS the record → every field offset is junk.  An INLINE
                    // `vector<__nullable<S>>` (not linked) keeps the inline read.
                    let elem_ref_like = matches!(&**vtp, Type::Reference(_, _))
                        || matches!(&**vtp, Type::Enum(d, true, _)
                            if self.data.def(*d).name.starts_with("__nullable<"));
                    if elem_ref_like {
                        if self.database.is_linked(db_tp) {
                            ref_expr = self.cl("OpVectorRefNullable", &[code.clone(), i.clone()]);
                        }
                    } else if matches!(*vtp.clone(), Type::Tuple(_)) {
                        // P189b: vector-of-tuple — keep ref_expr as the
                        // raw `OpGetVector` DbRef.  The loop var is typed
                        // `Reference(__tuple<…>)` (see for_type) so codegen
                        // reads elements via `OpVarRef` + `OpGet*(offset)`
                        // per element type.  Without this skip,
                        // `get_val(Tuple, …)` falls through the field-
                        // type dispatch and errors with "Field access
                        // not supported on type tuple([…])".
                    } else if matches!(*vtp.clone(), Type::Function(_, _, _)) {
                        // @P343: vector-of-fn-ref element read.  Mirror the
                        // working index-apply path (parser/fields.rs:730-752)
                        // — vector elements store only the 4-byte d_nr, so
                        // assemble the stack fn-ref tuple from `OpGetInt4`
                        // (the d_nr) + `OpNullRefSentinel` (the closure half;
                        // vector fn-refs are non-capturing).  Routing through
                        // `get_val(Function, …)` instead (the struct-field
                        // path) reads a non-existent `__closure_rec` child via
                        // `OpRefFromChildRec(OpGetField(elem, 4, 0))` →
                        // garbage closure.  The block name `fn_ref_field_read`
                        // is reused so native codegen's tuple-emit shortcut
                        // (`((d_nr) as u32, closure_DbRef)`) fires here too.
                        let read_dnr = self.cl("OpGetInt4", &[ref_expr, Value::Int(0)]);
                        let read_clos = self.cl("OpNullRefSentinel", &[]);
                        ref_expr = crate::data::v_block(
                            vec![read_dnr, read_clos],
                            *vtp.clone(),
                            "fn_ref_field_read",
                        );
                    } else {
                        // route through `get_val` with the full
                        // element Type — preserves `IntegerSpec.forced_size`
                        // so narrow vectors dispatch to `OpGetShortRaw` /
                        // `OpGetByte` / `OpGetInt4` via the narrow_vec
                        // split.  Previously via `get_field(vec_tp, MAX)`
                        // which looked up `def(integer).returned` and lost
                        // the forced_size → emitted `OpGetInt` (8 bytes)
                        // into a 2-byte slot, producing off-bytes reads.
                        ref_expr = self.get_val(vtp, false, 0, ref_expr, u32::MAX);
                    }
                    let mut tp = *vtp.clone();
                    if matches!(tp, Type::Tuple(_)) {
                        // keep block type aligned with for_type's RefVar(Tuple)
                        // — the next-expression yields a 12-byte DbRef.
                        tp = Type::RefVar(Box::new(tp));
                    }
                    for d in dep {
                        tp = tp.depending(*d);
                    }
                    let reverse = self.reverse_iterator;
                    let step = if reverse {
                        // Decrement, but clamp at i32::MIN to prevent negative-index wrap.
                        // When iter reaches -1, set it to i32::MIN so GetVector returns null.
                        let decremented = self.op("Min", i.clone(), Value::Int(1), I32.clone());
                        let cond = self.op("Le", Value::Int(1), i.clone(), I32.clone());
                        v_block(
                            vec![Value::If(
                                Box::new(cond),
                                Box::new(decremented),
                                Box::new(Value::Int(i32::MIN)),
                            )],
                            I32.clone(),
                            "rev step",
                        )
                    } else {
                        self.op("Add", i.clone(), Value::Int(1), I32.clone())
                    };
                    // P189b: keep block type aligned with for_type's
                    // Reference(__tuple<…>) — the next-expression yields
                    // a 12-byte DbRef into vector storage.
                    let block_tp = if let Type::Tuple(ref elems) = *vtp.clone() {
                        let elems_clone = elems.clone();
                        let tuple_d = self.data.tuple_def(&mut self.lexer, &elems_clone);
                        Type::Reference(tuple_d, crate::data::Deps::none())
                    } else {
                        *vtp.clone()
                    };
                    let next =
                        v_block(vec![v_set(iter_var, step), ref_expr], block_tp, "iter next");
                    self.vars
                        .set_loop(0, self.data.def(vec_tp).known_type(), code);
                    if reverse {
                        // Start at length; the first step gives len-1 (last element).
                        *code = v_set(
                            iter_var,
                            self.cl("OpLengthVector", std::slice::from_ref(code)),
                        );
                    } else {
                        *code = v_set(iter_var, Value::Int(-1));
                    }
                    self.reverse_iterator = false;
                    return next;
                }
                Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _) => {
                    // Derive element type for the block result annotation.
                    let elem_type = match is_type {
                        Type::Sorted(dnr, _, dep)
                        | Type::Index(dnr, _, dep)
                        | Type::Hash(dnr, _, dep)
                        | Type::Radix(dnr, _, dep) => Type::Reference(*dnr, dep.clone()),
                        _ => Type::Null,
                    };
                    // Create a separate Long variable to hold the packed i64 iterator
                    // state (cur << 32 | finish).  iter_var ({id}#index) remains I32
                    // as the user-visible sequential loop counter.
                    // The state var is named "{loop_name}#iter_state" so that iter_op()
                    // can find it by name when generating #remove.
                    let iter_base = self.vars.name(iter_var);
                    let iter_state_name = format!(
                        "{}#iter_state",
                        iter_base.strip_suffix("#index").unwrap_or(iter_base)
                    );
                    let state_var = self.create_var(&iter_state_name, &crate::data::I64);
                    self.vars.defined(state_var);
                    let mut ls = Vec::new();
                    self.fill_iter(&mut ls, code, is_type, true, true);
                    ls.push(Value::Int(0));
                    ls.push(Value::Int(0));
                    let iter_expr = self.cl("OpIterate", &ls);
                    let mut ls = vec![Value::Var(state_var)];
                    self.fill_iter(&mut ls, code, is_type, false, true);
                    // Reset the reverse flag after both fill_iter calls so the second call
                    // also picks up the bit (fill_iter does not reset it itself).
                    self.reverse_iterator = false;
                    let next_expr = self.cl("OpStep", &ls);
                    let incr = self.op("Add", Value::Var(iter_var), Value::Int(1), I32.clone());
                    let iter_next = v_block(
                        vec![v_set(iter_var, incr), next_expr],
                        elem_type,
                        "sorted iter next",
                    );
                    // Use Insert (not v_block+Void) so that state_var and iter_var are
                    // claimed at the outer For-block scope and their stack slots persist
                    // for the duration of the loop.  A Void block would free them on exit.
                    *code = Value::Insert(vec![
                        v_set(state_var, iter_expr),
                        v_set(iter_var, Value::Int(-1)),
                    ]);
                    return iter_next;
                }
                _ => {
                    // @F32 — custom iterators via fn next(self) -> T?
                    // I13: custom iterator protocol — check for fn next(&T) -> Item?
                    let next_d_nr = self.data.find_fn(u16::MAX, "next", is_type);
                    if next_d_nr != u32::MAX {
                        // Store the iterable in a variable so .next() has a stable target.
                        let iter_obj_var = self.create_unique("__iter_obj", is_type);
                        self.vars.defined(iter_obj_var);
                        let obj_expr = code.clone();
                        *code = v_set(iter_obj_var, obj_expr);
                        // The "next" expression is a method call: iter_obj.next()
                        let next_call = Value::Call(next_d_nr, vec![Value::Var(iter_obj_var)]);
                        return next_call;
                    }
                    if self.first_pass {
                        return Value::Null;
                    }
                    // Plan-07 phase 6 (partial) — name the offending type
                    // and the iterables loft accepts, so the user knows
                    // what to substitute.  Old wording "Unknown iterator
                    // type T" left users guessing whether T was the issue
                    // or the syntax.
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot iterate over {}; expected vector, sorted, index, hash, text, or range",
                        is_type.name(&self.data)
                    );
                }
            }
        }
        Value::Null
    }

    /// Convert a type to another type when possible
    /// Returns false when impossible. However, the other way round might still be possible.
    pub(crate) fn towards_set_hash_remove(
        &mut self,
        to: &Value,
        val: &Value,
        op: &str,
    ) -> Option<Value> {
        if !self.first_pass && *val == Value::Null && op == "=" {
            // Partial-key lookup produces an iteration (Value::Iter), not a single record.
            // Assigning null to an iteration has no defined semantics — require all key fields.
            if matches!(to, Value::Iter(..)) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot assign null to a partial-key lookup — \
                     provide all key fields to remove a single entry"
                );
                return Some(Value::Null);
            }
            if let Value::Call(get_nr, get_args) = to.unspan()
                && self.data.def(*get_nr).name() == "OpGetRecord"
                && let Some(Value::Int(db_tp_val)) = get_args.get(1)
                && (*db_tp_val as usize) < self.database.types.len()
                && matches!(
                    self.database.types[*db_tp_val as usize].parts,
                    Parts::Hash(_, _) | Parts::Index(_, _, _) | Parts::Sorted(_, _)
                )
            {
                let db_tp = *db_tp_val;
                let get_args = get_args.clone();
                let get_rec = self.cl("OpGetRecord", &get_args);
                return Some(self.cl(
                    "OpHashRemove",
                    &[get_args[0].clone(), get_rec, Value::Int(db_tp)],
                ));
            }
        }
        None
    }

    pub(crate) fn towards_set(
        &mut self,
        to: &Value,
        val: &Value,
        f_type: &Type,
        src_tp: &Type,
        op: &str,
    ) -> Value {
        // Intercept `h[key] = null` → remove the key from hash/index/sorted
        if let Some(result) = self.towards_set_hash_remove(to, val, op) {
            return result;
        }
        // @P305 — `coll[key] = value` insert-or-replace for a KEYED
        // collection.  The LHS parsed to `OpGetRecord(coll, db_tp, key…)`,
        // which returns the existing slot or null; the default reference
        // copy below (`OpCopyRecord(value, OpGetRecord(…))`) UPDATES an
        // existing key but silently NO-OPs when the key is absent (copy into
        // a null lookup), so it could never INSERT.  Route to `OpSetKeyed`,
        // which finds-or-inserts at runtime (dedup by `value`'s key),
        // uniformly for local / field / `&`-param collections.  (The
        // `coll[key] = null` removal is intercepted earlier in this fn.)
        //
        // @PLN25 E2 — gate-on, a keyed collection over a nullable element
        // (`hash<S[k]>` rewritten to `hash<__nullable<S>[k]>`) has element
        // type `Enum(__nullable<S>, true)` (the inline-nullable form, see
        // `index_type`), NOT `Reference(S)`.  Accept it too so the keyed-set
        // still routes to `OpSetKeyed`: `set_keyed` reads `value`'s key via
        // the SAME `key_owner`-resolved key descriptors the lookup uses, so
        // insert and lookup agree.  Without this the set falls through to the
        // update-only `OpCopyRecord`, which no-ops on the insert-miss and
        // leaves every lookup returning null.  Inert gate-off (no element
        // type is ever a `__nullable<` enum).
        let nullable_elem = matches!(
            f_type,
            Type::Enum(e, true, _) if self.data.def(*e).name.starts_with("__nullable<")
        );
        if op == "="
            && (matches!(f_type, Type::Reference(_, _)) || nullable_elem)
            && let Value::Call(get_nr, get_args) = to.unspan()
            && self.data.def(*get_nr).name() == "OpGetRecord"
            && let Some(Value::Int(db_tp)) = get_args.get(1)
            && (*db_tp as usize) < self.database.types.len()
            && matches!(
                self.database.types[*db_tp as usize].parts,
                Parts::Hash(_, _) | Parts::Sorted(_, _) | Parts::Index(_, _, _)
            )
        {
            let db_tp = *db_tp;
            let coll = get_args[0].clone();
            // Multi-index guard: a keyed STRUCT FIELD cross-linked with a
            // sibling index (two+ keyed fields sharing an element type,
            // auto-linked in types.rs) can't be maintained by `OpSetKeyed`
            // (it lacks the struct + field context to update the siblings).
            // Detect it and fall through to the update-only `copy_ref` below
            // (which keeps the shared record consistent — no insert, no
            // corruption — matching the pre-@P305 behaviour for that case).
            let mut multi_index = false;
            if let Value::Call(gf_nr, gf_args) = coll.unspan()
                && self.data.def(*gf_nr).name() == "OpGetField"
                && let Some(Value::Int(byte_off)) = gf_args.get(1)
                && let Value::Var(sv) = gf_args[0].unspan()
            {
                let d_nr = match self.vars.tp(*sv) {
                    Type::Reference(d, _) => *d,
                    Type::RefVar(inner) => match &**inner {
                        Type::Reference(d, _) => *d,
                        _ => u32::MAX,
                    },
                    _ => u32::MAX,
                };
                if d_nr != u32::MAX {
                    let struct_tp = self.data.def(d_nr).known_type();
                    multi_index = self
                        .database
                        .keyed_field_is_linked(struct_tp, *byte_off as u16);
                }
            }
            if multi_index {
                // `coll[key] = value` on a multi-indexed field would desync
                // the sibling indexes (OpSetKeyed) or `copy_block` into a
                // null lookup on an insert-miss (copy_ref) — both corrupt.
                // Direct the user to `+= [value]`, which maintains every
                // sibling index via record_finish's `other_indexes` path.
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "`coll[key] = value` is not supported on a multi-indexed \
                         field (a struct with two-or-more keyed fields sharing an \
                         element type); use `coll += [value]` instead"
                    );
                }
                return Value::Null;
            }
            // @P311: do NOT set the 0x8000 "free source" bit here.  Unlike the
            // field-assignment path (copy_ref → OpCopyRecord), `set_keyed`
            // already takes a deep copy of the value into the freshly-claimed
            // collection record, and the inline-literal/struct-call work-ref
            // that produced `val` still carries its own scope `OpFreeRef`.
            // Freeing it again from set_keyed is a double free: the work-ref's
            // store is released while still owned, then reused by the next
            // iteration's OpDatabase, corrupting the nested-vector backings of
            // every entry inserted after the first (silent data loss on the
            // interpreter, use-after-free SIGSEGV once the store is recycled).
            let tp_val = db_tp;
            return self.cl("OpSetKeyed", &[coll, val.clone(), Value::Int(tp_val)]);
        }
        // #328: a `reference<T>` field is a POINTER — assignment repoints
        // the field; it must never deep-copy record bytes into the current
        // referent (which both clobbered the referent and left `= null` a
        // no-op).  The discriminator is the lvalue SHAPE: `get_val` emits
        // `OpGetDbRef(host, pos)` only for pointer fields (the parse-time
        // share marker is rewritten to liveness deps by the expression
        // layer, so the TYPE cannot be tested here).  Closure-capture reads
        // also produce OpGetDbRef but on the hidden `__closure` param —
        // excluded so captured-Reference reassignment keeps its existing
        // copy semantics.
        if matches!(f_type, Type::Reference(_, _))
            && op == "="
            && let Value::Call(d, args) = to.unspan()
            && self.data.def(*d).name() == "OpGetDbRef"
            && args.len() == 2
            && !matches!(args[0].unspan(), Value::Var(v) if *v == self.closure_param)
        {
            let (r, p) = (args[0].clone(), args[1].clone());
            // `= null` clears the pointer: write the canonical null
            // sentinel DbRef (a bare `Value::Null` here makes codegen
            // allocate a typeless temp record that then leaks).
            let v = if matches!(val.unspan(), Value::Null) {
                self.cl("OpNullRefSentinel", &[])
            } else {
                val.clone()
            };
            return self.cl("OpSetDbRef", &[r, p, v]);
        }
        // @PLN25 E2a.5 — `lvalue = null` for a nullable inline struct field /
        // vector element.  The field/element is a synthetic `__nullable<S>` enum
        // stored inline, so null is discriminant 0 (NOT the variable store_nr
        // sentinel — an inline slot has no DbRef of its own).  `copy_ref` would
        // emit `OpCopyRecord(null, dest, …)`: a silent no-op on the interpreter
        // (the null source copies nothing) and a hard type error on native
        // (`OpCopyRecord(cell, (), …)` — `()` where a DbRef is expected).  Write
        // the discriminant at offset 0 to 0 instead; the inline form of `== null`
        // reads exactly that (operators.rs `enum_null`).  Inert with the synthesis
        // gate off — no `__nullable<` enums exist, so the name check never matches.
        //
        // A present `Some` carrying heap payload (text / nested vector) is freed
        // first via `OpClearKeyed` (→ `remove_claims`), which reads the
        // discriminant and no-ops on an already-null / payload-less element — so
        // it is safe regardless of prior state.  Without it the old payload leaks
        // until the host store is freed (@PLN25 E2 leak, was tracked in
        // embedded-record-null.md).
        if op == "="
            && matches!(val.unspan(), Value::Null)
            && let Type::Enum(syn, true, _) = f_type
            && self.data.def(*syn).name.starts_with("__nullable<")
            && !matches!(to, Value::Var(_))
        {
            let set_null = self.cl(
                "OpSetEnum",
                &[to.clone(), Value::Int(0), Value::Enum(0, u16::MAX)],
            );
            let kt = self.data.def(*syn).known_type();
            if !self.first_pass && kt != u16::MAX {
                let free = self.cl("OpClearKeyed", &[to.clone(), Value::Int(i32::from(kt))]);
                return v_block(vec![free, set_null], Type::Void, "nullable_elem_set_null");
            }
            return set_null;
        }
        // @PLN25 E2 — `inline_nullable = <expression source of type S>`: a nullable
        // `__nullable<S>` field / vector element assigned from an EXPRESSION whose
        // type is `Reference(S)` (a call / variable, possibly the null sentinel).
        // `copy_ref` → `OpCopyRecord` would copy S's flat layout (`id@0,tag@8`) into
        // the packed `Some` element (`disc@0,tag@4,id@8`) — garbage on a present
        // source, and `allocation.rs:560` OOB on a null source (no store to read).
        // Instead: when the source is present, build the `Some` variant (disc 2 +
        // per-field copy at the Some offsets); when null, set discriminant 0.  This
        // is `handle_field`'s null-source convert lifted to the `[i] = expr` / field
        // store site.  A literal `S{…}` already built `Some` in `parse_var`, so its
        // `src_tp` is the enum (not `Reference(S)`) and it does not reach here.
        if op == "="
            && !self.first_pass
            && !matches!(to, Value::Var(_))
            && let Type::Enum(syn, true, _) = f_type
            && self.data.def(*syn).name.starts_with("__nullable<")
            && let Type::Reference(src_d, _) = src_tp
            && self.data.def(*syn).name == format!("__nullable<{}>", self.data.def(*src_d).name())
        {
            let syn = *syn;
            let some_d = self.data.variant_of(syn, "Some");
            let src_var = self.vars.work_refs(src_tp, &mut self.lexer);
            // Single-payload: set the discriminant present and copy the whole dense `S`
            // source into the inline `payload` field (Null=1, Some=2; 0 = absent).
            let present = self.build_some_present(some_d, to.clone(), Value::Var(src_var));
            // null branch — set discriminant 0 (the element may already hold a
            // `Some`; unlike the construction path it is not freshly zero-inited).
            let null_set = self.cl(
                "OpSetEnum",
                &[to.clone(), Value::Int(0), Value::Enum(0, u16::MAX)],
            );
            let is_null = self.cl("OpRefIsNull", &[Value::Var(src_var)]);
            let not_null = self.cl("OpNot", &[is_null]);
            // Free the element's prior `Some` payload before overwriting it with
            // either the new `Some` or null — both branches below clobber the
            // inline slot in place, so without this the old heap payload (text /
            // nested vector) leaks.  `remove_claims` no-ops on an already-null
            // element, so it is safe even on first assignment.  Free FIRST: the
            // source (a plain `Reference(S)` value) cannot alias this enum-typed
            // inline slot, so clearing it does not disturb the source read.
            let kt = self.data.def(syn).known_type();
            let mut body = Vec::with_capacity(3);
            if kt != u16::MAX {
                body.push(self.cl("OpClearKeyed", &[to.clone(), Value::Int(i32::from(kt))]));
            }
            body.push(v_set(src_var, val.clone()));
            body.push(v_if(not_null, Value::Insert(present), null_set));
            return v_block(body, Type::Void, "nullable_elem_convert");
        }
        // @PLN25 index flip — an element WRITE `v[i] = h` is an lvalue slot, not a nullable
        // read: under the flip `v[i]` types `Optional(Reference/Enum)`, but the slot itself
        // holds the base record, so a whole-element assign is still a `copy_ref` (OpCopyRecord).
        // Peel `.base()` so the Optional read-nullability marker doesn't drop it to the generic
        // op-name path (which errors "Cannot assign to attribute on OpGetVector"). Gate-OFF
        // inert (no Optional exists → `.base()` is identity → byte-identical).
        if matches!(
            f_type.base(),
            Type::Enum(_, true, _) | Type::Reference(_, _)
        ) && op == "="
            && !matches!(to, Value::Var(_))
        {
            return self.copy_ref(to, val, f_type.base());
        }
        if matches!(
            *f_type,
            Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
        ) {
            if let Value::Var(nr) = to.unspan() {
                if self.const_write_blocked(*nr, op) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot modify {} '{}'; remove 'const' or use a local copy",
                        self.vars.const_kind(*nr),
                        self.vars.name(*nr)
                    );
                }
                return v_set(*nr, val.clone());
            }
            // @P308 — a KEYED-collection FIELD whole-assignment `s.h = expr`
            // (hash/sorted/index, RHS an expression) must be DEEP-COPIED into
            // the field; before this it fell to the bare `return val.clone()`
            // below → silent no-op (empty field).  `OpReplaceKeyed(val, to,
            // kt)` against the field ref = remove_claims(field) +
            // copy_claims(val, field); `0x8000` frees a fresh-storage call
            // source.  (Empty `s.h = []` → OpClearKeyed and `s.h[k]=v` →
            // OpSetKeyed are handled earlier; this is the remaining
            // whole-value case.)  Vector/Radix keep the bare return —
            // vector field-replace lives in parse_assign_op, and Radix's
            // copy_claims is unimplemented (per @P295), so `keyed_field_kt`
            // returns None for both.
            if op == "="
                && !matches!(val, Value::Insert(_) | Value::Null)
                && let Some(kt) = self.keyed_field_kt(f_type)
            {
                #[cfg(not(feature = "wasm"))]
                let tp_val = if self.is_struct_returning_call(val) {
                    i32::from(kt) | 0x8000
                } else {
                    i32::from(kt)
                };
                #[cfg(feature = "wasm")]
                let tp_val = i32::from(kt);
                return self.cl(
                    "OpReplaceKeyed",
                    &[val.clone(), to.clone(), Value::Int(tp_val)],
                );
            }
            // LHS is a field access (e.g. `s.v = fresh`).  Pre-fix this
            // returned bare `val` and the assignment was silently discarded.
            // The full clear-then-append pair lives in parse_assign_op where
            // the RHS type is in scope (so we can avoid emitting OpAppendVector
            // when the RHS is not actually a vector — e.g. `b.data = f#read(...)`
            // where f#read returns text — which would mismatch types in
            // codegen).  Empty literal `[]` is also handled there.
            return val.clone();
        }
        if let Type::RefVar(tp) = f_type
            && matches!(**tp, Type::Vector(_, _) | Type::Sorted(_, _, _))
        {
            if let Value::Var(nr) = to.unspan() {
                if self.vars.uses(*nr) > 0 {
                    return val.clone();
                }
            } else {
                return val.clone();
            }
        }
        // @PLN17: boolean element/field writes now go through the generic
        // call_to_set_op path (OpGetBoolean -> OpSetBoolean, with try_swap on the
        // inner OpGetVector) — identical to how plain enums are handled.  The old
        // special-case here destructured the two-level OpEqInt(OpGetByte(…)) read
        // shape and is obsolete (it mis-read the single-level OpGetBoolean shape).
        let code = self.compute_op_code(op, to, val, f_type);
        if let Value::Call(d_nr, args) = to.unspan() {
            let name = self.data.def(*d_nr).name().to_string();
            let args = args.clone();
            self.call_to_set_op(&name, &args, code, op)
        } else if let Value::Var(nr) = to.unspan() {
            if self.const_write_blocked(*nr, op) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot modify {} '{}'; remove 'const' or use a local copy",
                    self.vars.const_kind(*nr),
                    self.vars.name(*nr)
                );
            }
            // This variable was created here and thus not yet used.
            self.var_usages(*nr, false);
            v_set(*nr, code)
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Not implemented operation {op} for type {}",
                    f_type.show(&self.data, &self.vars)
                );
            }
            Value::Null
        }
    }

    /// Compute the RHS value after applying `op` to `to` and `val`.
    pub(crate) fn iter_op_count_or_first(
        &mut self,
        code: &mut Value,
        name: &str,
        t: &mut Type,
        is_first: bool,
    ) {
        let count_var = format!("{name}#count");
        let count = if self.vars.name_exists(&count_var) {
            self.vars.var(&count_var)
        } else {
            self.create_var(&count_var, &I32)
        };
        self.vars.loop_count(count);
        self.vars.defined(count);
        if is_first {
            *code = self.cl("OpEqInt", &[Value::Var(count), Value::Int(0)]);
            *t = Type::Boolean;
        } else {
            *code = Value::Var(count);
            *t = I32.clone();
        }
    }

    #[allow(clippy::too_many_lines)] // iterator operation dispatch — splitting would lose context
    pub(crate) fn iter_op(&mut self, code: &mut Value, name: &str, t: &mut Type, index_var: u16) {
        // File variables handle their own # operations before iterator operations.
        if self.is_file_var(index_var) {
            self.file_op(code, t, index_var);
            return;
        }
        // detect #fields for compile-time field iteration.
        if self.lexer.has_keyword("fields") {
            let var = self.vars.var(name);
            let var_type = if var == u16::MAX {
                Type::Unknown(0)
            } else {
                self.vars.tp(var).clone()
            };
            if let Type::Reference(d, _) = &var_type {
                self.fields_of = *d;
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "#fields requires a struct variable, got {}",
                    var_type.name(&self.data)
                );
            }
            // Set code to the source variable so parse_field_iteration receives it.
            if var != u16::MAX {
                *code = Value::Var(var);
            }
            *t = Type::Void;
            return;
        }
        if self.lexer.has_keyword("index") {
            // For index<T> collections, {name}#index holds an internal B-tree record number,
            // not a sequential 0-based counter.  Reject it at compile time.
            if self.vars.loop_on(index_var) & 63 == 1 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "#index is not supported on index<T> collections \
(it holds an internal record number, not a sequential counter); \
use #count instead"
                );
                *t = Type::Unknown(0);
            } else {
                let i_name = &format!("{name}#index");
                if self.vars.name_exists(i_name) {
                    let v = self.vars.var(i_name);
                    *t = self.vars.tp(v).clone();
                    *code = Value::Var(v);
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Incorrect #index variable on {}",
                        name
                    );
                    *t = Type::Unknown(0);
                }
            }
        } else if self.lexer.has_keyword("next") {
            let n_name = format!("{name}#next");
            if self.vars.name_exists(&n_name) {
                let v = self.vars.var(&n_name);
                *t = self.vars.tp(v).clone();
                *code = Value::Var(v);
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Incorrect #next variable on {} (only valid in text loops)",
                    name
                );
                *t = Type::Unknown(0);
            }
        } else if self.lexer.has_token("break") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot continue outside a loop");
            }
            *code = Value::Break(self.vars.loop_nr(name));
            *t = Type::Void;
        } else if self.lexer.has_token("continue") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot continue outside a loop");
            }
            *code = Value::Continue(self.vars.loop_nr(name));
            *t = Type::Void;
        } else if self.lexer.has_keyword("count") {
            self.iter_op_count_or_first(code, name, t, false);
        } else if self.lexer.has_keyword("first") {
            self.iter_op_count_or_first(code, name, t, true);
        } else if self.lexer.has_keyword("remove") {
            // CO1.5c: #remove on generator iterators is already rejected by the
            // loop_value == Null check below — coroutine for-loops never call set_loop.
            if !self.first_pass && *self.vars.loop_value(index_var) == Value::Null {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}#remove' is only valid on a loop iteration variable (e.g. 'for {} in collection {{ {}#remove }}')",
                    name,
                    name,
                    name
                );
                *t = Type::Void;
                return;
            }
            // C60 Step 9: reject #remove on hash iteration.  The parser
            // substitutes hash iteration with a scratch rec-nr vector
            // (see parse_for, the `{id}#hash_scratch` variable), so
            // #remove would remove from the snapshot, not the hash —
            // silently diverging from the user's intent.
            if !self.first_pass {
                let coll = self.vars.loop_coll_var(index_var);
                if coll != u16::MAX && self.vars.name(coll).contains("hash_scratch") {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "#remove is not supported on hash iteration — the \
                         iterated vector is a sorted snapshot; use \
                         `hash[key] = null` to remove from the hash"
                    );
                    *t = Type::Void;
                    return;
                }
            }
            let on = self.vars.loop_on(index_var);
            let state_name = if on & 63 >= 1 && on & 63 <= 3 {
                let state_key = format!("{name}#iter_state");
                if self.vars.name_exists(&state_key) {
                    state_key
                } else {
                    format!("{name}#index")
                }
            } else {
                format!("{name}#index")
            };
            *code = self.cl(
                "OpRemove",
                &[
                    Value::Var(self.vars.var(&state_name)),
                    self.vars.loop_value(index_var).clone(),
                    Value::Int(i32::from(on)),
                    Value::Int(i32::from(self.vars.loop_db_tp(index_var))),
                ],
            );
            *t = Type::Void;
        } else if self.lexer.has_keyword("lock") {
            // d#lock — read the lock state of the store containing a reference or vector variable.
            // Assignment d#lock = true/false is resolved in towards_set.
            if !self.first_pass
                && !matches!(
                    self.vars.tp(index_var),
                    Type::Reference(_, _) | Type::Vector(_, _)
                )
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "#lock is only valid on reference or vector variables, not on '{}'",
                    name
                );
                *t = Type::Unknown(0);
            } else {
                *code = self.cl("n_get_store_lock", &[Value::Var(index_var)]);
                *t = Type::Boolean;
            }
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown loop attribute '#{name}'; use #index, #count, #first, #last, or #break"
            );
            *t = Type::Unknown(0);
        }
    }

    pub(crate) fn append_data_fp(state: OutputState, fmt: Value) -> (Value, Value, Value) {
        let mut a_width = state.width;
        let mut p_rec = Value::Int(-1); // -1 = no precision specified; 0 = :.0
        if let Value::Float(w) = a_width {
            let s = format!("{w}");
            let mut split = s.split('.');
            a_width = Value::Int(split.next().unwrap().parse::<i32>().unwrap());
            // `{m:1.0}` — precision ZERO: the dotted spec was parsed into a
            // FLOAT whose Display drops the trailing `.0`, so the fraction
            // part is absent here.  Absent means a zero fraction (a dotted
            // spec is the only way the width arrives as Float), not "no
            // precision" — unwrapping it was a parser ICE on `:N.0`.
            p_rec = Value::Int(split.next().map_or(0, |p| p.parse::<i32>().unwrap_or(0)));
        }
        if state.float {
            p_rec = a_width;
            a_width = Value::Int(0);
        }
        (fmt, a_width, p_rec)
    }

    pub(crate) fn append_data_long(
        &mut self,
        list: &mut Vec<Value>,
        start: &str,
        var: Value,
        fmt: Value,
        state: OutputState,
    ) {
        list.push(self.cl(
            &(start.to_owned() + "Int"),
            &[
                var,
                fmt,
                Value::Int(state.radix),
                state.width,
                Value::Int(i32::from(state.token.as_bytes()[0])),
                Value::Boolean(state.plus),
                Value::Boolean(state.note),
                Value::Int(state.dir),
            ],
        ));
    }

    pub(crate) fn append_data_text(
        &mut self,
        list: &mut Vec<Value>,
        start: &str,
        var: Value,
        fmt: Value,
        state: OutputState,
    ) {
        list.push(self.cl(
            &(start.to_owned() + "Text"),
            &[
                var,
                fmt,
                state.width,
                Value::Int(state.dir),
                Value::Int(i32::from(state.token.as_bytes()[0])),
            ],
        ));
    }

    #[allow(clippy::too_many_lines)]
    /// P242: when `d_nr` is the type variable of the current
    /// generic-function context AND that variable's bound
    /// supplies a `to_text(self: Self) -> text` method, return a
    /// `Value::Call(t_<len><tvname>_to_text, [format])` IR node.
    /// Otherwise return `None`.
    ///
    /// Used by `append_data` to route generic-T format-string
    /// interpolation (`println("{x}")` where `x: T`) through the
    /// bound's stub method.  At monomorphisation time
    /// `re_resolve_call` substitutes the stub with the concrete
    /// type's implementation, just like an explicit
    /// `x.to_text()` call site.
    fn try_bound_to_text_call(&mut self, d_nr: u32, format: &Value, spec: &str) -> Option<Value> {
        // @PLN99 Arc B — route `{x}` / `{x:spec}` through the type's OWN
        // `to_text(self, …)` for ANY struct, not only the current bounded-
        // generic's type variable (the old gate required `context == Generic`).
        // A struct with no `t_<len><Type>_to_text` method (`stub_nr == u32::MAX`
        // below) falls back to the generic `OpFormatDatabase` dump — nothing
        // regresses.
        if self.data.def_type(d_nr) != DefType::Struct {
            return None;
        }
        let tv_name = self.data.def(d_nr).name().to_string();
        if tv_name.is_empty() {
            return None;
        }
        let stub_name = format!("t_{}{}_{}", tv_name.len(), tv_name, "to_text");
        let stub_nr = self.data.def_nr(&stub_name);
        if stub_nr == u32::MAX {
            return None;
        }
        // Classify the stub's params by TYPE, not arity.  A `to_text` may or may
        // not carry a user `spec: text` param (@PLN99 Arc B — the value owns its
        // `{x:spec}` DSL), and INDEPENDENTLY may or may not carry the hidden
        // text-return work buffer (`RefVar(Text)`, added by `parse_function`'s
        // I9-text path so the call mirrors an auto-generated `convert(text →
        // &text)`).  Counting attributes conflates `(self, spec)` with
        // `(self, __work)` — both are 2 — so a `to_text(self, spec)` whose body
        // returns text directly (a tail `if`, a bare literal: no work buffer)
        // silently lost its spec and received the empty work buffer in its place
        // (#533).  The buffer is also renamed to a promoted local (`r`) by
        // text_return, so its `__`-name is not reliable — only the type is.  self
        // is attr 0 (the struct being formatted, never text/RefVar).
        let mut has_spec = false;
        let mut has_work = false;
        for a in 1..self.data.attributes(stub_nr) {
            match self.data.attr_type(stub_nr, a) {
                Type::Text(_) => has_spec = true,
                Type::RefVar(_) => has_work = true,
                _ => {}
            }
        }
        let mut args = vec![format.clone()];
        if has_spec {
            args.push(Value::str(spec));
        }
        if has_work {
            let wv = self.vars.work_text(&mut self.lexer);
            args.push(v_block(
                vec![
                    v_set(wv, Value::Text(String::new())),
                    self.cl("OpCreateStack", &[Value::Var(wv)]),
                ],
                Type::Reference(self.data.def_nr("reference"), crate::data::Deps::frame1(wv)),
                "p242_to_text_work",
            ));
        }
        Some(Value::Call(stub_nr, args))
    }

    pub(crate) fn append_data(
        &mut self,
        tp: Type,
        list: &mut Vec<Value>,
        append: u16,
        append_value: u16,
        format: &Value,
        state: OutputState,
    ) {
        // @PLN25 — format dispatch peels an `Optional` value type (`"{s}"` where
        // `s: text?`) to its base, matching index/method dispatch which already
        // peel; inert gate-OFF (no `Optional` is ever constructed). A null-holding
        // value formats via its base-text/scalar sentinel exactly as gate-OFF.
        let tp = match tp {
            Type::Optional(inner) => *inner,
            other => other,
        };
        let var = Value::Var(append);
        let start = if matches!(self.vars.tp(append), Type::RefVar(_)) {
            "OpFormatStack"
        } else {
            "OpFormat"
        };
        // L9: escalate format-specifier mismatches to compile errors.
        // A specifier that can never have any effect on the value type is always a bug.
        if !self.first_pass {
            let is_text = matches!(tp, Type::Text(_));
            let is_bool = matches!(tp, Type::Boolean);
            if state.radix != 10 && (is_text || is_bool) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Format specifier has no effect on {}",
                    tp.name(&self.data)
                );
            } else if is_text && state.token == "0" && state.width != Value::Int(0) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Zero-padding has no effect on text"
                );
            }
        }
        match tp {
            Type::Integer(_) => {
                self.append_data_long(list, start, var, format.clone(), state);
            }
            Type::Boolean => {
                let value = self.cl("OpCastTextFromBool", std::slice::from_ref(format));
                self.append_data_text(list, start, var, value, state);
            }
            Type::Text(_) => {
                self.append_data_text(list, start, var, format.clone(), state);
            }
            Type::Character => {
                list.push(self.cl("OpAppendCharacter", &[var, format.clone()]));
            }
            Type::Float => {
                let dir = Value::Int(state.dir);
                let (fmt, a_width, p_rec) = Self::append_data_fp(state, format.clone());
                list.push(self.cl(
                    &(start.to_owned() + "Float"),
                    &[var, fmt, a_width, p_rec, dir],
                ));
            }
            Type::Single => {
                let dir = Value::Int(state.dir);
                let (fmt, a_width, p_rec) = Self::append_data_fp(state, format.clone());
                list.push(self.cl(
                    &(start.to_owned() + "Single"),
                    &[var, fmt, a_width, p_rec, dir],
                ));
            }
            Type::Vector(cont, _) => {
                let fmt = format.clone();
                let d_nr = self.data.type_def_nr(&cont);
                // #250's recursive resolution, the FORMAT twin of `db_type`'s
                // Vector arm, for NESTED content only: `vector<vector<X>>`
                // content must resolve recursively — the def-level
                // `known_type()` fallback returns whichever vector row
                // registered first, a layout-coincident id that broke (#483
                // SIGSEGV in `ShowDb::write_list`) as soon as new stdlib
                // content shifted the type table.  Non-vector content keeps
                // `known_type()`: an enum's row carries the VARIANT NAMES the
                // format needs, where `db_type` returns the generic byte
                // storage row.
                // #624 — a NARROW element (`u8` / `u16` / a 4-byte `integer`
                // subtype) is stored packed at 1/2/4 bytes, but the def-level
                // `known_type()` below resolves the WIDE integer row.  Dumping
                // through that row strides 8 bytes over packed data: `{v}` on a
                // `vector<u8>` printed the first eight elements packed into one
                // i64 followed by zeros.  Resolve the narrow storage row the way
                // the element READ and the `+=` append already do, and skip
                // `check_vector` — that registers `vector<integer>`'s own row,
                // which a narrow element does not own and must not overwrite.
                let vec_tp = if let Some(narrow) =
                    self.data.narrow_vector_content(&cont, &mut self.database)
                {
                    self.database.vector(narrow)
                } else {
                    let db_tp = if matches!(*cont, Type::Vector(_, _)) {
                        self.database.db_type(&cont, &self.data)
                    } else {
                        self.data.def(d_nr).known_type()
                    };
                    if db_tp == u16::MAX {
                        0
                    } else {
                        let v = self.database.vector(db_tp);
                        self.data.check_vector(d_nr, v, self.lexer.pos());
                        v
                    }
                };
                list.push(self.cl(
                    &(start.to_owned() + "Database"),
                    &[
                        var,
                        fmt,
                        Value::Int(i32::from(vec_tp)),
                        Value::Int(state.db_format()),
                    ],
                ));
            }
            Type::Iterator(vtp, _) => {
                self.append_iter(list, append, append_value, vtp.as_ref(), format, state);
            }
            Type::Reference(d_nr, _) => {
                // P242 fix: when `d_nr` is the current generic
                // function's type variable AND the bound supplies
                // a `to_text` method, route the format through it
                // before dispatching as text.  Without this, the
                // codegen falls back to OpFormatDatabase with a
                // non-DbRef arg at monomorphisation time (interp
                // prints "null" then panics; native rejects with
                // rustc E0308 "expected DbRef").  The bound's
                // `to_text` stub (`t_<len><tvname>_to_text`) is
                // already created by `parse_function`'s I7/I8.1
                // path; `re_resolve_call` substitutes it with the
                // concrete type's impl at instantiation time.
                if let Some(text_call) = self.try_bound_to_text_call(d_nr, format, state.spec) {
                    self.append_data_text(list, start, var, text_call, state);
                } else {
                    let fmt = format.clone();
                    let db_tp = self.data.def(d_nr).known_type();
                    list.push(self.cl(
                        &(start.to_owned() + "Database"),
                        &[
                            var,
                            fmt,
                            Value::Int(i32::from(db_tp)),
                            Value::Int(state.db_format()),
                        ],
                    ));
                }
            }
            Type::Enum(d_nr, is_ref, _) => {
                let fmt = format.clone();
                let e_tp = self.data.def(d_nr).known_type();
                if e_tp == u16::MAX || !is_ref {
                    let e_val = self.cl("OpCastTextFromEnum", &[fmt, Value::Int(i32::from(e_tp))]);
                    self.append_data_text(list, start, var, e_val, state);
                } else {
                    list.push(self.cl(
                        &(start.to_owned() + "Database"),
                        &[
                            var,
                            fmt,
                            Value::Int(i32::from(e_tp)),
                            Value::Int(state.db_format()),
                        ],
                    ));
                }
            }
            _ => {
                // @P376 — `Type::Never` is the poison an errored struct
                // construction assigns; the real `unknown type '…'` was already
                // reported, so silently skip formatting it instead of adding a
                // cascade "Cannot format type never".
                if !self.first_pass && !matches!(tp, Type::Never) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot format type {}",
                        tp.name(&self.data)
                    );
                }
            }
        }
    }

    pub(crate) fn append_iter(
        &mut self,
        list: &mut Vec<Value>,
        append: u16,
        append_value: u16,
        var_type: &Type,
        value: &Value,
        state: OutputState,
    ) {
        if let Value::Iter(var, init, next, extra_init) = value
            && matches!(**next, Value::Block(_))
        {
            let count = if *var == u16::MAX {
                self.create_unique("count", &I32)
            } else {
                let count_name = format!("{}#count", self.vars.name(*var));
                let c = self.vars.var(&count_name);
                if c == u16::MAX {
                    self.create_var(&count_name, &I32)
                } else {
                    c
                }
            };
            list.push(self.cl("OpAppendText", &[Value::Var(append), Value::str("[")]));
            list.push(*init.clone());
            if !matches!(**extra_init, Value::Null) {
                list.push(*extra_init.clone());
            }
            list.push(v_set(count, Value::Int(0)));
            let mut append_var = append_value;
            if append_value == u16::MAX {
                append_var = self.create_unique("val", var_type);
            }
            let mut steps = Vec::new();
            steps.push(v_set(append_var, *next.clone()));
            steps.push(v_if(
                self.cl("OpLtInt", &[Value::Int(0), Value::Var(count)]),
                self.cl("OpAppendText", &[Value::Var(append), Value::str(",")]),
                Value::Null,
            ));
            steps.push(v_set(
                count,
                self.cl("OpAddInt", &[Value::Var(count), Value::Int(1)]),
            ));
            self.append_data(
                var_type.clone(),
                &mut steps,
                append,
                append_var,
                &Value::Var(append_var),
                state,
            );
            list.push(v_loop(steps, "Append Iter"));
            list.push(self.cl("OpAppendText", &[Value::Var(append), Value::str("]")]));
        }
    }

    // <object> ::= [ <identifier> ':' <expression> { ',' <identifier> ':' <expression> } ] '}'
    /// Parse a single `field: value` entry in an object literal.
    /// Returns `None` if parsing should abort (lexer reverted), `Some(false)` on unknown field,
    /// `Some(true)` on success.
    /// Parse a single `field: value` entry in an object literal.
    /// Returns false if no identifier found or `:` missing (caller handles revert).
    pub(crate) fn parse_for_iter_setup(
        &mut self,
        id: &str,
        in_type: &Type,
        expr: Value,
    ) -> (u16, Option<u16>, u16, Value, Value, Value) {
        let var_tp = self.for_type(in_type);
        // For text loops: {id}#next drives the loop; {id}#index is saved per-iteration.
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
        // error if the loop variable reuses a name with a different type.
        // Same-type reuse is idiomatic in loft (flat variable scoping).
        // `_` is exempt — it's the universal "unused" name and must work
        // across different element types within the same function.
        //
        // loft#690 — this compares with `is_equal`, NOT `is_same`.  `is_same` answers
        // "same KIND of type": it reports ANY two `Reference`s as the same whatever
        // struct they name, so `for r in as_a { … }  for r in as_b { … }` over two
        // different structs slipped through.  `add_variable` then handed the second
        // loop the FIRST binding — old var, old type, old dep — and the body read B's
        // records through A's layout: no diagnostic, no crash, just wrong numbers
        // (`m=8589934636` for a sum of 3).  `is_equal` compares the struct a
        // `Reference` names while keeping the looseness that matters here — differing
        // integer ranges, text deps, and fn-ref capture lists still count as the same
        // type, so same-struct reuse stays idiomatic.
        let existing_var = self.vars.var(id);
        if !self.first_pass
            && id != "_"
            && existing_var != u16::MAX
            && self.vars.is_defined(existing_var)
            && !self.vars.var_type(existing_var).is_equal(&var_tp)
            && !self.vars.var_type(existing_var).is_unknown()
            // text_return converts text variables to RefVar(Text) work buffers
            // for the return path.  When a for-loop variable was converted this
            // way, the iterator still writes into it as text — this is correct
            // (the work buffer IS the variable) so suppress the mismatch.
            && !matches!(self.vars.var_type(existing_var), Type::RefVar(_))
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "loop variable '{id}' has type {} but was previously used as {}",
                var_tp.name(&self.data),
                self.vars.var_type(existing_var).name(&self.data)
            );
        }
        // C61: reject two classes of shadow that both produce silent wrong
        // values today:
        //   * Nested same-name loops (`for i { for i { } }`) — the inner
        //     iterator rewrites the outer's `#index` companion, detected
        //     on pass 2 via the active-loop chain.
        //   * Outer-local shadow (`x = 5; for x in …`) — the loop
        //     silently clobbers `x`; detected on pass 1 via the
        //     `was_loop_var` flag.  A plain local's slot has never served
        //     as a loop variable, so the prior binding is unambiguously a
        //     local.
        // Sequential same-name loops stay legal because the prior slot
        // carries `was_loop_var = true`.  `_` is exempt.
        if id != "_" && existing_var != u16::MAX {
            if !self.first_pass && self.vars.is_active_loop_var(existing_var) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "loop variable '{id}' shadows the enclosing loop's '{id}' — \
                     rename the inner loop variable (e.g. inner_{id}); loft does \
                     not support nested same-name loops"
                );
            } else if self.first_pass
                && !self.vars.was_loop_var(existing_var)
                && !self.vars.is_active_loop_var(existing_var)
                && !matches!(self.vars.var_type(existing_var), Type::RefVar(_))
                && (self.vars.var_type(existing_var).is_same(&var_tp)
                    || self.vars.var_type(existing_var).is_unknown())
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "loop variable '{id}' shadows a local named '{id}' — \
                     rename the loop variable (e.g. loop_{id}) or drop the \
                     outer `{id}` if it was a dead placeholder; loft does \
                     not block-scope loop variables"
                );
            }
        }
        let for_var = self.create_var(id, &var_tp);
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
        let iter_next = self.iterator(&mut create_iter, in_type, &it, iter_var, pre_var);
        (iter_var, pre_var, for_var, if_step, create_iter, iter_next)
    }

    // @F28 — for-in loops (ranges, loop attributes, filtered, rev())
    pub(crate) fn parse_for(&mut self, code: &mut Value) {
        // P235: tuple destructure — `for (a, b, ...) in items { ... }`.
        // Parse the parenthesised name list now; later (after the iter
        // type is known) we synthesize a temp loop var and prepend
        // `Set(name_i, loop_var.<i>)` to the body so `a` / `b` resolve
        // as if the user had written them.  Closes both par and
        // non-par destructure with one rewrite (the par variant
        // dispatches into `parse_parallel_for_loop` which inherits
        // `id` from us).
        let destructure_names: Option<Vec<String>> = if self.lexer.peek_token("(") {
            self.lexer.token("(");
            let mut names = Vec::new();
            loop {
                if let Some(n) = self.lexer.has_identifier() {
                    names.push(n);
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Expect identifier in for-destructure pattern"
                    );
                    let _ = self.lexer.has_token(")");
                    return;
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            if !self.lexer.has_token(")") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect ')' to close for-destructure pattern"
                );
                return;
            }
            if names.len() < 2 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "for-destructure pattern requires at least 2 names; got {}",
                    names.len()
                );
                return;
            }
            Some(names)
        } else {
            None
        };

        // @PLN115 tail — capture the simple `for id` binder's position before it is
        // consumed, to record its DECLARATION once the loop var exists (only when
        // recording; destructure binders are synthesized, so they are excluded).
        let binder_pos = (destructure_names.is_none() && self.record_resolutions)
            .then(|| self.lexer.peek_pos().clone());
        // P235: when destructuring, synthesize a loop var name from
        // the source line/column; the user-named binders are defined
        // later as proper variables and prepended to the body.
        let id_opt: Option<String> = if destructure_names.is_some() {
            let pos = self.lexer.peek().position.clone();
            Some(format!("__destructure_t_{}_{}", pos.line, pos.pos))
        } else {
            self.lexer.has_identifier()
        };
        if let Some(id) = id_opt {
            // @P345: a loop variable's type is fully determined by the
            // iterable's element type, so an annotation is always redundant
            // (and unsupported).  Catch `for i: T in …` here and emit one
            // clear message instead of the misleading `Expect token in` →
            // `{` → `;` cascade the bare `token("in")` would produce.
            if self.lexer.peek_token(":") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "loop variable '{id}' is type-inferred from the iterable — remove the ': <type>' annotation (write `for {id} in …`)"
                );
                // Recover: skip the (possibly compound) annotation up to `in`
                // so the loop body still parses and no spurious token cascade
                // follows the clear message above.
                self.lexer.has_token(":");
                while !self.lexer.peek_token("in")
                    && !self.lexer.peek_token("{")
                    && !self.lexer.peek_token("")
                {
                    self.lexer.cont();
                }
            }
            self.lexer.token("in");
            let loop_nr = self.vars.start_loop();
            let mut expr = Value::Null;
            let mut in_type = self.parse_in_range(&mut expr, &Value::Null, &id);
            // if #fields was detected, take the compile-time unrolling path.
            if self.fields_of != u32::MAX {
                let struct_def_nr = self.fields_of;
                self.fields_of = u32::MAX;
                self.vars.finish_loop(loop_nr);
                self.parse_field_iteration(&id, struct_def_nr, &expr, code);
                return;
            }
            let mut fill = Value::Null;
            // For vector loops, the iterator runs on a unique temp copy so that the loop
            // variable does not alias the user-visible collection.  Record the original
            // variable number so that mutation of the original can be detected later.
            let orig_coll_var = if let Value::Var(v) = &expr {
                *v
            } else {
                u16::MAX
            };
            // Save the original collection expression before the vector temp-copy substitution
            // so that is_iterated_value() can match field-access patterns like `db.items`.
            let orig_coll_expr = expr.clone();
            // C60 piece 3 edit B (re-attempt with typed scratch): when
            // iterating a hash, substitute the collection expression
            // with a call to `hash_sorted(h, tp_id)` that builds a
            // u32-stride rec-nr scratch in the hash's own store
            // (edit A).  `in_type` stays Type::Hash so fill_iter hits
            // the Hash arm and emits on=3 (edit C); the empty-bounds
            // guard in iterate on=3 (edit E) handles unbounded.
            //
            // Key fix from prior segfault attempt: type the scratch
            // variable with the hash's actual content def-nr
            // (`Type::Reference(content, dep)`), not `Reference(0)`.
            // Downstream type-size + free-cleanup machinery reads
            // `self.data.def(content)`; passing 0 gave whatever
            // definition happens to sit at index 0 and corrupted
            // stack layout.
            // Set to the `hash_scratch` var for on=4 (Hash/Radix) iteration, so the
            // loop epilogue can free a read-only source's DEDICATED scratch store
            // (`OpFreeScratch`, a no-op when co-located).  u16::MAX = not on=4.
            let mut hash_scratch_var: u16 = u16::MAX;
            if let Type::Hash(content, _, dep) | Type::Radix(content, _, dep) = in_type.clone() {
                let is_radix = matches!(in_type, Type::Radix(_, _, _));
                let scratch_tp = Type::Reference(content, dep.clone());
                let scratch_var = self.create_unique("hash_scratch", &scratch_tp);
                hash_scratch_var = scratch_var;
                let hash_tp_id = self.get_type(&in_type);
                let tp_arg = if hash_tp_id == u16::MAX {
                    0
                } else {
                    i32::from(hash_tp_id)
                };
                // A `par` loop discards a hash's order across worker threads, so it
                // skips the O(n log n) key sort and walks the buckets raw.  A radix
                // has a natural order and its walk is already ordered (no sort), so
                // it uses the same builder in every case (@PLN48).
                let is_par =
                    matches!(&self.lexer.peek().has, LexItem::Identifier(kw) if kw == "par");
                let scratch_fn_name = if is_radix {
                    "n_radix_sorted"
                } else if is_par {
                    "n_hash_unsorted"
                } else {
                    "n_hash_sorted"
                };
                // @PLN48 S3 — `for m in xs.within(…)`: the `.within` intercept already
                // rewrote `expr` to an `n_spatial_within(…)` call that BUILDS the
                // ordered scratch.  Use it directly — do not wrap it in n_radix_sorted
                // (which would walk the scratch as if it were a tree).
                let already_scratch = matches!(
                    expr.unspan(),
                    Value::Call(d, _) if self.data.def(*d).name() == "n_spatial_range"
                );
                if already_scratch {
                    fill = v_set(scratch_var, expr.clone());
                    expr = Value::Var(scratch_var);
                    if !self.first_pass {
                        self.vars.set_type(scratch_var, scratch_tp);
                    }
                } else {
                    let hash_sorted_fn = self.data.def_nr(scratch_fn_name);
                    if hash_sorted_fn != u32::MAX {
                        let call =
                            Value::Call(hash_sorted_fn, vec![expr.clone(), Value::Int(tp_arg)]);
                        fill = v_set(scratch_var, call);
                        expr = Value::Var(scratch_var);
                        if !self.first_pass {
                            self.vars.set_type(scratch_var, scratch_tp);
                        }
                    }
                }
            }
            if matches!(in_type, Type::Vector(_, _)) {
                let vec_var = self.create_unique("vector", &in_type);
                // On the second pass in_type may carry __vdb_N dependencies that
                // were not present on the first pass (vector_db only runs on pass 2).
                // Update the temp variable's type so that get_free_vars sees the
                // deps and does NOT emit OpFreeRef for the temp — the __vdb_N
                // variable at the outer scope owns the store and will free it.
                if !self.first_pass {
                    self.vars.set_type(vec_var, in_type.clone());
                }
                in_type = in_type.depending(vec_var);
                fill = v_set(vec_var, expr);
                expr = Value::Var(vec_var);
            }
            // Optional parallel clause: par(result=worker(elem), threads)
            if let LexItem::Identifier(kw) = &self.lexer.peek().has
                && kw == "par"
            {
                self.lexer.has_identifier(); // consume "par"
                // Plan-06 phase 4d.B — par-over-keyed-collection
                // desugar.  When the input is sorted/hash/index/
                // spatial, the par dispatcher's flat-vector iteration
                // doesn't know how to walk the tree/hashmap layout.
                // Pre-materialise into a `vector<reference<T>>` and
                // re-route par() to use the materialised vector.
                if matches!(
                    in_type,
                    Type::Sorted(_, _, _)
                        | Type::Hash(_, _, _)
                        | Type::Index(_, _, _)
                        | Type::Radix(_, _, _)
                ) && let Some((mat_fill_ir, mat_var, mat_in_type)) =
                    self.materialise_keyed_for_par(&in_type, &expr)
                {
                    let combined_fill = if fill == Value::Null {
                        mat_fill_ir
                    } else {
                        // Inline (Insert), not a scoped Block — see the
                        // materialise_keyed_for_par note: native codegen must see
                        // `__par_mat`'s `let` in the enclosing function scope.
                        Value::Insert(vec![fill, mat_fill_ir])
                    };
                    self.parse_parallel_for_loop(
                        code,
                        &id,
                        &mat_in_type,
                        &Value::Var(mat_var),
                        combined_fill,
                        loop_nr,
                        destructure_names.as_deref(),
                    );
                    return;
                }
                // A range / `iterator<T>` / text input has no flat vector for
                // the par dispatcher to partition; materialise it into one via
                // the same iterate-and-append the comprehension uses, then
                // re-route par() over the materialised vector.
                if let Some((mat_fill_ir, mat_var, mat_in_type)) =
                    self.materialise_iter_for_par(&in_type, &expr, loop_nr)
                {
                    let combined_fill = if fill == Value::Null {
                        mat_fill_ir
                    } else {
                        Value::Insert(vec![fill, mat_fill_ir])
                    };
                    self.parse_parallel_for_loop(
                        code,
                        &id,
                        &mat_in_type,
                        &Value::Var(mat_var),
                        combined_fill,
                        loop_nr,
                        destructure_names.as_deref(),
                    );
                    return;
                }
                self.parse_parallel_for_loop(
                    code,
                    &id,
                    &in_type,
                    &expr,
                    fill,
                    loop_nr,
                    destructure_names.as_deref(),
                );
                return;
            }
            // CO1.5: detect coroutine for-loop before parse_for_iter_setup consumes expr.
            let is_coroutine_loop = matches!(&in_type, Type::Iterator(_, _))
                && !self.first_pass
                && matches!(expr.unspan(), Value::Call(d, _) if matches!(self.data.def(*d).returned(), Type::Iterator(_, _)));
            let (iter_var, pre_var, for_var, if_step, create_iter, iter_next) =
                self.parse_for_iter_setup(&id, &in_type, expr);
            // @PLN115 tail — record the loop binder's DECLARATION (pass 2, recording
            // on): `Local{fn_def, for_var}` at the binder name, so a `for i` binder's
            // references/rename take S4's precise path instead of the F-v1 fallback.
            if let Some(pos) = &binder_pos
                && !self.first_pass
                && for_var != u16::MAX
            {
                self.record_decl(
                    pos,
                    id.chars().count() as u16,
                    crate::resolution::Resolution::Local {
                        fn_def: self.context,
                        var_nr: for_var,
                    },
                );
            }
            let var_tp = self.for_type(&in_type);
            // For vector loops: set_loop stores the temp-copy var; override with the
            // original so that `orig += elem` is correctly identified as a mutation.
            if matches!(in_type, Type::Vector(_, _)) {
                if orig_coll_var != u16::MAX {
                    self.vars.set_coll_var(orig_coll_var);
                }
                // Always restore the original collection expression so that
                // is_iterated_value() can match field-access forms like `db.items`.
                self.vars.set_coll_value(orig_coll_expr.clone());
            }
            if !self.first_pass && iter_next == Value::Null {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Need an iterable expression in a for statement"
                );
                // Balance the loop stack before bailing: `start_loop` (above)
                // has already pushed this loop's scope, so a bare `return`
                // leaves `current_loop` pointing at it and the ENCLOSING loop's
                // `finish_loop` then trips the "Incorrect loop finish" assert
                // (a hard panic masking this diagnostic).  Mirrors the `#fields`
                // early-return at the top of this function.  Reached e.g. when a
                // loop variable is reused across two sequential loops of
                // different element types (`for t in a {…} for t in b {…}`): the
                // reused name keeps the first type, so a field access on the
                // second loop var fails to resolve and yields no iterator.
                self.vars.finish_loop(loop_nr);
                return;
            }
            // P235 step 2: with for_var resolved, define each destructured
            // binder as a proper variable typed as the matching tuple
            // element.  The Set(name_i, TupleGet(for_var, i)) ops will be
            // prepended to the body block once `parse_block` returns.
            // Defining the variables BEFORE `parse_block` runs is essential
            // — the body's references to `a` / `b` / etc. resolve through
            // the parser's scope at parse time.
            let destructure_setup: Vec<Value> = if let Some(names) = &destructure_names {
                // Tuple element types come from one of two shapes:
                //   - `Type::Tuple([T1, T2, ...])` — direct tuple type
                //     (uncommon for for-loops: would require iterating
                //     over a "tuple of …" rather than a vector<tuple>).
                //   - `Type::Reference(d_nr, _)` where `def(d_nr).name`
                //     starts with `__tuple<` — synthetic struct created
                //     by `tuple_def`; element types live as attributes.
                //     This is the common shape for `vector<(T1, T2)>`
                //     iteration, mirroring P189b's element-access path
                //     in `src/parser/operators.rs:608-658`.
                let (elem_types_opt, ref_def_nr): (Option<Vec<Type>>, u32) = match &var_tp {
                    Type::Tuple(elems) => (Some(elems.clone()), u32::MAX),
                    Type::Reference(d_nr, _)
                        if self.data.def(*d_nr).name().starts_with("__tuple<") =>
                    {
                        let elems: Vec<Type> = self
                            .data
                            .def(*d_nr)
                            .attributes
                            .iter()
                            .map(|a| a.typedef.clone())
                            .collect();
                        (Some(elems), *d_nr)
                    }
                    _ => (None, u32::MAX),
                };
                if let Some(elem_types) = elem_types_opt {
                    if elem_types.len() == names.len() {
                        // Build per-element read.  Two shapes:
                        //   - Direct Tuple: `Value::TupleGet(for_var, i)`
                        //     — reads from the var's stack-resident
                        //     tuple slot.
                        //   - Reference(__tuple<…>): use `get_val` with
                        //     the synthetic struct's per-attribute byte
                        //     offset — same path P189b's `.0` / `.1`
                        //     element access takes.
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, name)| {
                                let elem_tp = elem_types[i].clone();
                                let var = self.create_var(name, &elem_tp);
                                self.vars.defined(var);
                                self.vars.in_use(var, true);
                                let read = if ref_def_nr == u32::MAX {
                                    Value::TupleGet(for_var, i as u16)
                                } else {
                                    let elem_offset = if let Some(offs) =
                                        crate::data::stored_tuple_offsets_for_def(
                                            &self.data,
                                            &self.database,
                                            ref_def_nr,
                                            elem_types.len(),
                                        ) {
                                        u32::from(offs[i])
                                    } else {
                                        crate::data::element_stack_offsets(&elem_types)[i] as u32
                                    };
                                    self.get_val(
                                        &elem_tp,
                                        false,
                                        elem_offset,
                                        Value::Var(for_var),
                                        u32::MAX,
                                    )
                                };
                                v_set(var, read)
                            })
                            .collect()
                    } else {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "for-destructure: pattern has {} names but iterated tuple has {} elements",
                                names.len(),
                                elem_types.len()
                            );
                        }
                        Vec::new()
                    }
                } else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "for-destructure requires a tuple element type, got {}",
                            var_tp.name(&self.data)
                        );
                    }
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            // Extract the generator var (first arg of OpCoroutineNext) before
            // `iter_next` is consumed by `for_next` — @P327 needs it for the
            // tuple-yield exhaustion check below.
            let gen_var = if let Value::Call(_, args) = &iter_next
                && let Some(Value::Var(v)) = args.first()
            {
                *v
            } else {
                u16::MAX
            };
            // #481 (the #306-message half): a heap-ref coroutine yield is a
            // DbRef INTO the generator's state store — both for yielded views
            // of generator locals AND for records constructed in the
            // generator (they allocate in the state store, which is freed
            // with the generator).  Bind the loop var as a BORROW of the
            // generator (dep on gen_var): the consumer's scope machinery
            // must never emit a per-iteration OpFreeRef for it, which
            // whole-store-freed the generator's state store (the store
            // recycled under allocation pressure → corrupted worklists) and
            // tripped the #306 stack-store guard on the exhausted null.
            if gen_var != u16::MAX
                && matches!(in_type, Type::Iterator(_, _))
                && matches!(
                    var_tp,
                    Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
                )
            {
                let dep_tp = match var_tp.clone() {
                    Type::Reference(d, _) => Type::Reference(d, crate::data::Deps::frame1(gen_var)),
                    Type::Enum(d, m, _) => Type::Enum(d, m, crate::data::Deps::frame1(gen_var)),
                    Type::Vector(e, _) => Type::Vector(e, crate::data::Deps::frame1(gen_var)),
                    other => other,
                };
                self.change_var_type(for_var, &dep_tp);
            }
            // @PLN93 (#511): iterating a CAPTURED collection (`for e in h`, `h` captured
            // into this lambda).  The element is a DbRef INTO the captured (shared) store,
            // reached via the hidden `__closure` param — exactly the #481 coroutine shape.
            // Bind the loop var as a BORROW of the closure so the scope machinery never
            // emits a per-iteration OpFreeRef for it: that free calls `free_named` on the
            // element's store_nr, which whole-store-frees the shared collection — a later
            // closure capturing the same collection then reads an empty store (native
            // only; interp already treats the element as a borrow).
            if self.closure_param != u16::MAX
                && !self.first_pass
                && Self::is_collection_type(&in_type)
                && in_type.depend().contains(&self.closure_param)
                && matches!(
                    var_tp,
                    Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
                )
            {
                let dep = crate::data::Deps::frame1(self.closure_param);
                let dep_tp = match var_tp.clone() {
                    Type::Reference(d, _) => Type::Reference(d, dep),
                    Type::Enum(d, m, _) => Type::Enum(d, m, dep),
                    Type::Vector(e, _) => Type::Vector(e, dep),
                    other => other,
                };
                self.change_var_type(for_var, &dep_tp);
            }
            // For length-based vector termination below: pull the collection the
            // element fetch actually reads from (the materialised iteration temp),
            // not the original collection expression.  Re-reading the original each
            // iteration would re-run a side-effecting source (`for x in make()`); the
            // fetch temp is already materialised once, so its length is cheap + stable.
            let vec_fetch_coll = if matches!(in_type, Type::Vector(_, _)) {
                fn find_vec_coll(v: &Value, gvn: u32, vrn: u32) -> Option<Value> {
                    match v {
                        Value::Call(op, args) if *op == gvn || *op == vrn => args.first().cloned(),
                        Value::Call(_, args)
                        | Value::Insert(args)
                        | Value::Tuple(args)
                        | Value::Parallel(args) => {
                            args.iter().find_map(|a| find_vec_coll(a, gvn, vrn))
                        }
                        Value::Block(bl) | Value::Loop(bl) => {
                            bl.operators.iter().find_map(|o| find_vec_coll(o, gvn, vrn))
                        }
                        Value::If(c, t, e) => find_vec_coll(c, gvn, vrn)
                            .or_else(|| find_vec_coll(t, gvn, vrn))
                            .or_else(|| find_vec_coll(e, gvn, vrn)),
                        Value::Set(_, x)
                        | Value::Return(x)
                        | Value::Drop(x)
                        | Value::Yield(x)
                        | Value::BreakWith(_, x) => find_vec_coll(x, gvn, vrn),
                        Value::Span(b) => find_vec_coll(&b.1, gvn, vrn),
                        _ => None,
                    }
                }
                let gvn = self.data.def_nr("OpGetVectorNullable");
                let vrn = self.data.def_nr("OpVectorRefNullable");
                find_vec_coll(&iter_next, gvn, vrn)
            } else {
                None
            };
            let for_next = v_set(for_var, iter_next);
            self.vars.loop_var(for_var);
            let in_loop = self.in_loop;
            self.in_loop = true;
            let mut block = Value::Null;
            let loop_write_state = self.vars.save_and_clear_write_state();
            self.vars.clear_write_state();
            self.parse_block("for", &mut block, &Type::Void);
            // P235 step 3: prepend the destructure Set ops so each
            // iteration unpacks the loop var into the user-named binders
            // before the user's body runs.
            if !destructure_setup.is_empty() {
                if let Value::Block(ref mut bl) = block {
                    for s in destructure_setup.into_iter().rev() {
                        bl.operators.insert(0, s);
                    }
                } else {
                    let inner = std::mem::replace(&mut block, Value::Null);
                    let mut ops = destructure_setup;
                    ops.push(inner);
                    block = v_block(ops, Type::Void, "destructure_for_body");
                }
            }
            self.vars.restore_write_state(&loop_write_state);
            let count = self.vars.loop_counter();
            self.in_loop = in_loop;
            self.vars.finish_loop(loop_nr);
            let mut for_steps = Vec::new();
            if fill != Value::Null {
                for_steps.push(fill);
            }
            // For text loops, initialise {id}#index at the FOR block scope so its live
            // interval covers the entire loop (not just the inner "for text next" block).
            if let Some(idx_var) = pre_var {
                for_steps.push(v_set(idx_var, Value::Int(0)));
            }
            for_steps.push(create_iter);
            let mut lp = vec![for_next];
            // CO1.5b: coroutine iterators also need a termination check.
            //
            // @P327 — for-yield types without a single-value null sentinel
            // (Tuple today; the same shape would catch any future composite
            // yielded type) MUST check the iterator's exhausted state, NOT
            // the yielded value.  Without this branch, `convert(tuple,
            // Boolean)` finds no OpConv* match → `test_for` stays as
            // `Var(for_var)` → `OpNot(tuple)` reads one byte of the tuple's
            // storage as boolean and inverts → silent wrong answer (loop
            // iterates 0 or N times depending on which byte aligns to 0).
            // OpCoroutineExhausted(gen) reads the coroutine status, which
            // CoroutineNext sets to Exhausted on the post-last-yield call.
            // `gen_var` is captured above from `iter_next`'s first arg
            // (the coroutine path never assigns a slot to the index var).
            //
            // @PLAN16 phase 05 — closures (`Type::Function`) join tuples on
            // this path: a yielded fn-ref has no null sentinel that
            // `OpConv*FromX → OpNot` can drive — `OpNot(fnref)` reads bytes
            // of the 20-byte fn-ref slot as boolean (SIGBUS on interp,
            // E0600 on native).  Same fix: terminate via the coroutine's
            // own exhausted state.
            // #401 — every coroutine loop terminates via the iterator's own
            // exhausted state, NOT a value-sentinel.  The value-sentinel path
            // below (`convert(value, Boolean)` → `OpNot`) only terminates when
            // the yielded type's null sentinel matches the transport channel's
            // exhaustion sentinel (`i64::MIN`): true for int/text/ref, but NOT
            // for `float`/`single` (null = NaN) or `enum` — `coroutine_next`
            // returns `i64::MIN`, whose f64 bit-pattern is not NaN, so the break
            // never fires and the loop spins forever (and the interp codegen of
            // that doomed check hangs).  Originally this used the state check
            // only for composite yields (Tuple/Function, which have no
            // single-value sentinel at all); it is correct for every element type.
            if is_coroutine_loop && gen_var != u16::MAX {
                let test_exhausted = self.cl("OpCoroutineExhausted", &[Value::Var(gen_var)]);
                lp.push(v_if(
                    test_exhausted,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            } else if matches!(in_type, Type::Vector(_, _))
                && matches!(&var_tp, Type::Function(_, _, _))
            {
                // @P343: a `vector<fn(...)>` loop terminates by testing the
                // d_nr half of the loop's fn-ref (see the
                // `fn_ref_field_read` branch in `iterator()`): break once it
                // is no longer a valid (callable, > 0) function reference.
                // At out-of-bounds the element read sets the d_nr to the
                // backend's invalid sentinel — i64::MIN on the interpreter
                // (`OpGetInt4` on the `OpGetVectorNullable` null sentinel)
                // and 0 on `--native` (that i64::MIN truncates to `0u32` in
                // the `(u32, DbRef)` fn-ref tuple, which is also native's
                // "invalid fn-ref" marker).  Both are `<= 0`, and every
                // real fn d_nr is `> 0`, so `d_nr > 0` (encoded as
                // `OpLtInt(0, d_nr)`) is the one test that terminates
                // correctly on both backends.  `convert(Function, Boolean)`
                // has no rule, so the generic branch below would leave the
                // raw 20-byte fn-ref for `OpNot` to misread — testing the
                // always-null closure half, which broke the loop on
                // iteration 0.
                let dnr = Value::FnRefDnr(for_var);
                let in_bounds = self.cl("OpLtInt", &[Value::Int(0), dnr]);
                let test_for = self.cl("OpNot", &[in_bounds]);
                lp.push(v_if(
                    test_for,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            } else if matches!(in_type, Type::Vector(_, _)) {
                // Length-based termination: yield exactly len(coll) elements,
                // independent of the element value, so a null ELEMENT (which shares
                // the OOB null sentinel) no longer ends the loop early.  The length is
                // re-read EACH iteration (not hoisted) so in-loop `x#remove` — which
                // shrinks the vector and decrements the index (state/io.rs remove) —
                // still terminates: when the vector drains, len falls to the index.
                // Direction-agnostic: forward ends at index == len, reverse ends at
                // index < 0 (the i32::MIN sentinel the reverse step sets).
                let coll = vec_fetch_coll
                    .clone()
                    .unwrap_or_else(|| orig_coll_expr.clone());
                let len = self.cl("OpLengthVector", std::slice::from_ref(&coll));
                let past_end = self.cl("OpLeInt", &[len, Value::Var(iter_var)]);
                lp.push(v_if(
                    past_end,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
                let before_start = self.cl("OpLtInt", &[Value::Var(iter_var), Value::Int(0)]);
                lp.push(v_if(
                    before_start,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            } else if !matches!(in_type, Type::Iterator(_, _)) || is_coroutine_loop {
                let mut test_for = Value::Var(for_var);
                self.convert(&mut test_for, &var_tp, &Type::Boolean);
                test_for = self.cl("OpNot", &[test_for]);
                lp.push(v_if(
                    test_for,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            }
            if if_step != Value::Null {
                lp.push(v_if(if_step, Value::Null, Value::Continue(0)));
            }
            lp.push(block);
            if count != u16::MAX {
                for_steps.insert(0, v_set(count, Value::Int(0)));
                lp.push(v_set(
                    count,
                    self.cl("OpAddInt", &[Value::Var(count), Value::Int(1)]),
                ));
            }
            for_steps.push(v_loop(lp, "For loop"));
            // on=4 epilogue — free a read-only source's DEDICATED scratch store on loop
            // exit.  Runs on completion AND break (break leaves the v_loop to here), and
            // once per (re-)entry so a loop-in-loop frees each build.  Conditional:
            // OpFreeScratch frees only when the scratch store differs from the source
            // recorded in the header, so a co-located scratch (writable source) is a
            // no-op.  A `return` out of the loop still bypasses this — a bounded residual
            // (expose-iteration-scratch.md Open question A).
            if hash_scratch_var != u16::MAX {
                for_steps.push(self.cl("OpFreeScratch", &[Value::Var(hash_scratch_var)]));
                // Null the scratch var so the scope-exit OpFreeScratch (emitted by
                // get_free_vars, which catches the `return`-out-of-loop path) is a no-op
                // here — its rec==0 guard skips a nulled ref, so the two never double-free.
                for_steps.push(v_set(hash_scratch_var, Value::Null));
            }
            *code = v_block(for_steps, Type::Void, "For block");
        } else {
            diagnostic!(self.lexer, Level::Error, "Expect variable after for");
        }
    }

    /// Plan-06 phase 4d.B — materialise a keyed-collection input
    /// (`sorted/hash/index/spatial<T[key]>`) into a temporary
    /// `vector<reference<T>>` so the par dispatcher's flat-vector
    /// path can iterate it.  Returns `(fill_ir, mat_var, mat_in_type)`
    /// or None if the source can't be unwrapped.  Mirrors the IR
    /// shape that the parser emits for the manual workaround
    /// `refs += [s]`: OpPreAllocVector + OpNewRecord + OpCopyRecord
    /// + OpFinishRecord per loop iteration.
    pub(crate) fn materialise_keyed_for_par(
        &mut self,
        in_type: &Type,
        source_expr: &Value,
    ) -> Option<(Value, u16, Type)> {
        let (content_d, dep) = match in_type {
            Type::Sorted(c, _, dep)
            | Type::Hash(c, _, dep)
            | Type::Index(c, _, dep)
            | Type::Radix(c, _, dep) => (*c, dep.clone()),
            _ => return None,
        };
        let elem_ref_tp = Type::Reference(content_d, dep);
        let vec_ref_tp = Type::Vector(Box::new(elem_ref_tp.clone()), crate::data::Deps::none());
        let mat_var = self.create_unique("__par_mat", &vec_ref_tp);
        self.vars.defined(mat_var);
        // Register the wrapper struct EARLY (both passes) so the
        // typedef pass between pass 1 and pass 2 runs fill_database
        // and assigns a real `known_type`.
        let _ = self.data.vector_def(&mut self.lexer, &elem_ref_tp);
        if self.first_pass {
            return Some((Value::Null, mat_var, vec_ref_tp));
        }
        // Allocate the backing store for __par_mat.
        let db_setup = self.vector_db(&elem_ref_tp, mat_var);
        let iter_idx = self.create_unique("__par_mat_idx", &I32);
        self.vars.defined(iter_idx);
        let elm_var = self.create_unique("__par_mat_e", &elem_ref_tp);
        self.vars.defined(elm_var);
        // Drive the keyed-collection iterator via iterator() — same
        // helper that powers `for x in sorted_items`.
        let mut create_iter = source_expr.clone();
        let it_marker = Type::Iterator(Box::new(elem_ref_tp.clone()), Box::new(Type::Null));
        let iter_next = self.iterator(&mut create_iter, in_type, &it_marker, iter_idx, None);
        if iter_next == Value::Null {
            return None;
        }
        // Build the per-element append IR.  Mirrors the manual case
        // `refs += [s]` which lowers to:
        //   OpPreAllocVector(refs, 1, elem_size)
        //   tmp = OpNewRecord(refs, vec_tp, u16::MAX)
        //   OpCopyRecord(elm_var, tmp, content_tp)
        //   OpFinishRecord(refs, tmp, vec_tp, u16::MAX)
        let content_known = self.data.def(content_d).known_type();
        let elem_size = if content_known == u16::MAX {
            8
        } else {
            i32::from(self.database.size(content_known))
        };
        let vec_known = i32::from(self.vector_of(&elem_ref_tp));
        let prealloc = self.cl(
            "OpPreAllocVector",
            &[Value::Var(mat_var), Value::Int(1), Value::Int(elem_size)],
        );
        let tmp_var = self.create_unique("__par_mat_t", &elem_ref_tp);
        self.vars.defined(tmp_var);
        let new_rec = self.cl(
            "OpNewRecord",
            &[
                Value::Var(mat_var),
                Value::Int(vec_known),
                Value::Int(i32::from(u16::MAX)),
            ],
        );
        let copy_rec = self.cl(
            "OpCopyRecord",
            &[
                Value::Var(elm_var),
                Value::Var(tmp_var),
                Value::Int(i32::from(content_known)),
            ],
        );
        let finish_rec = self.cl(
            "OpFinishRecord",
            &[
                Value::Var(mat_var),
                Value::Var(tmp_var),
                Value::Int(vec_known),
                Value::Int(i32::from(u16::MAX)),
            ],
        );
        let body = Value::Insert(vec![
            prealloc,
            v_set(tmp_var, new_rec),
            copy_rec,
            finish_rec,
        ]);
        // Loop body: read next via iter_next into elm_var, break on
        // null, otherwise append (body).
        let for_next = v_set(elm_var, iter_next);
        let mut lp = vec![for_next];
        let mut test_for = Value::Var(elm_var);
        self.convert(&mut test_for, &elem_ref_tp, &Type::Boolean);
        test_for = self.cl("OpNot", &[test_for]);
        lp.push(v_if(
            test_for,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        ));
        lp.push(body);
        // Assemble the materialisation Block.
        let mut for_steps: Vec<Value> = Vec::new();
        for s in &db_setup {
            for_steps.push(s.clone());
        }
        for_steps.push(create_iter);
        for_steps.push(v_loop(lp, "Materialise par input"));
        // Splice the steps inline (Insert), NOT a v_block: native codegen emits
        // a Block as a Rust `{ }` scope, which would confine the `__par_mat`
        // `let` to that scope so the following par dispatch can't see it
        // (E0425).  The vector-input path likewise feeds a bare statement, not a
        // scoped block.
        let fill_ir = Value::Insert(for_steps);
        let mat_in_type = vec_ref_tp.depending(mat_var);
        Some((fill_ir, mat_var, mat_in_type))
    }

    /// Materialise a non-keyed iterable — a range / `iterator<T>` / text — into
    /// a flat `vector<T>` so the par dispatcher's index-partitioned walk can run
    /// over it.  This is the same "iterate, append" that the comprehension
    /// `[for e in src { e }]` performs; we reuse `build_comprehension_code` so
    /// element append is correct per kind (scalar / text / ref) on both
    /// backends.  Returns `(fill_ir, mat_var, mat_in_type)`, or None when the
    /// source isn't one of these iterables (a flat `vector` is dispatched
    /// directly; keyed collections go through `materialise_keyed_for_par`).
    pub(crate) fn materialise_iter_for_par(
        &mut self,
        in_type: &Type,
        source_expr: &Value,
        loop_nr: u16,
    ) -> Option<(Value, u16, Type)> {
        if !matches!(in_type, Type::Iterator(_, _) | Type::Text(_)) {
            return None;
        }
        let elem_tp = self.for_type(in_type);
        let vec_tp = Type::Vector(Box::new(elem_tp.clone()), crate::data::Deps::none());
        // Register the element vector type EARLY (both passes) so the typedef
        // pass between pass 1 and pass 2 assigns it a real `known_type`.
        let _ = self.data.vector_def(&mut self.lexer, &elem_tp);
        // Name these vars by the stable `loop_nr` (NOT the global create_unique
        // counter): the materialise body is built pass-2-only, so a counter-named
        // var advances the counter only on pass 2, which desyncs numbering for
        // sibling materialise loops — two loops' `__par_mat` then collide on one
        // name (#282).  When their element types differ (e.g. integer range vs
        // character text-source) the merged var takes one type, so the other loop
        // reads its store at the wrong stride → silent garbage.  A loop-keyed name
        // is unique per loop and identical across both passes, so no collision.
        let mk = |p: &mut Self, name: &str, tp: &Type| -> u16 {
            let v = p
                .vars
                .add_variable(&format!("_par_{name}_l{loop_nr}"), tp, &mut p.lexer);
            p.vars.defined(v);
            v
        };
        let mat_var = mk(self, "mat", &vec_tp);
        if self.first_pass {
            return Some((Value::Null, mat_var, vec_tp));
        }
        // Iterator state vars — text drives a (pos, index) pair, every other
        // iterator a single index — mirroring parse_vector_for.
        let (iter_var, pre_var) = if matches!(in_type, Type::Text(_)) {
            let pos = mk(self, "mat_next", &I32);
            let idx = mk(self, "mat_index", &I32);
            (pos, Some(idx))
        } else {
            let iv = mk(self, "mat_index", &I32);
            (iv, None)
        };
        let for_var = mk(self, "mat_e", &elem_tp);
        let mut create_iter = source_expr.clone();
        let it = Type::Iterator(Box::new(elem_tp.clone()), Box::new(Type::Null));
        let iter_next = self.iterator(&mut create_iter, in_type, &it, iter_var, pre_var);
        if iter_next == Value::Null {
            return None;
        }
        let for_next = v_set(for_var, iter_next);
        let elm = self.unique_elm_var(&vec_tp, &elem_tp, mat_var);
        // Identity comprehension `[for e in src { e }]` filling mat_var.  is_var
        // mode emits a Value::Insert (no Rust `{ }` scope), so the `let mat_var`
        // lands in the enclosing function scope where par dispatch reads it —
        // the same scoping the keyed materialiser relies on.
        let mut fill_expr = Value::Var(mat_var);
        self.build_comprehension_code(
            mat_var,
            &Value::Var(mat_var),
            elm,
            &elem_tp,
            in_type,
            &elem_tp,
            for_var,
            for_next,
            pre_var,
            Value::Null,
            create_iter,
            Value::Null,
            Value::Var(for_var),
            &mut fill_expr,
            true,
            false,
            false,
            vec_tp.clone(),
        );
        let mat_in_type = self.vars.tp(mat_var).clone();
        Some((fill_expr, mat_var, mat_in_type))
    }

    /// P235 par half — parse the worker call inside a destructured
    /// par expression and synthesize a wrapper fn that bridges the
    /// gap between par dispatch's "one per-iteration arg" model and
    /// the user's multi-arg-from-tuple call shape.
    ///
    /// Given `for (a, b) in pairs par(r = work(a, b), N) { ... }`
    /// (a, b already defined in scope as tuple-element vars by
    /// `parse_parallel_for_loop`), this method parses `work(a, b)`
    /// and synthesizes:
    ///
    /// ```text
    /// fn __par_destructure_w_<N>(t: tuple_type) -> ret_type {
    ///     work(t.<a_idx>, t.<b_idx>, ...)
    /// }
    /// ```
    ///
    /// Each user arg that is `Var(destructure_var_nrs[i])` becomes
    /// a tuple element read at the matching tuple position; other
    /// args (e.g. context constants) pass through verbatim.  The
    /// par dispatch then calls the wrapper with the tuple loop
    /// element as its single per-iteration arg.
    ///
    /// Returns the wrapper's d_nr (or u32::MAX on error), the
    /// return type, and empty extras (no context args — they're
    /// baked into the wrapper body).
    pub(crate) fn parse_destructure_par_worker(
        &mut self,
        tuple_tp: &Type,
        destructure_var_nrs: &[u16],
    ) -> (u32, Type, Vec<Value>, Vec<Type>) {
        // Parse worker fn name + ( arg, arg, ... )
        let Some(work_id) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect function name in par(...) destructure worker"
                );
            }
            return (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new());
        };
        let work_d_nr = {
            let prefixed = format!("n_{work_id}");
            let nr = self.data.def_nr(&prefixed);
            if nr == u32::MAX {
                self.data.def_nr(&work_id)
            } else {
                nr
            }
        };
        if !self.lexer.has_token("(") {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect '(' after worker name '{work_id}'"
                );
            }
            return (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new());
        }
        let mut user_args: Vec<Value> = Vec::new();
        if !self.lexer.peek_token(")") {
            loop {
                let mut arg = Value::Null;
                self.expression(&mut arg);
                user_args.push(arg);
                if !self.lexer.has_token(",") {
                    break;
                }
            }
        }
        self.lexer.token(")");

        if work_d_nr == u32::MAX {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Unknown function '{work_id}'");
            }
            return (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new());
        }
        if !self.first_pass && !matches!(self.data.def_type(work_d_nr), DefType::Function) {
            diagnostic!(self.lexer, Level::Error, "'{work_id}' is not a function");
            return (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new());
        }
        let ret_type = self.data.def(work_d_nr).returned().clone();
        if self.first_pass {
            // First pass: return the user worker so the parser's
            // downstream type-shape decisions (return_size, ladder
            // routing) see realistic values.  Argcount checks in
            // build_parallel_for_ir are gated on !first_pass, so
            // mismatched extras don't fire here.  Second pass
            // synthesizes the real wrapper.
            return (work_d_nr, ret_type, Vec::new(), Vec::new());
        }
        self.data.def_used(work_d_nr);

        // Resolve tuple element types + def_nr for offsets
        let elem_types: Vec<Type> = match tuple_tp {
            Type::Tuple(elems) => elems.clone(),
            Type::Reference(d, _) => self
                .data
                .def(*d)
                .attributes
                .iter()
                .map(|a| a.typedef.clone())
                .collect(),
            _ => Vec::new(),
        };
        let tuple_d_nr = match tuple_tp {
            Type::Reference(d, _) => *d,
            _ => u32::MAX,
        };

        // Allocate wrapper def
        let wrapper_pos = self.lexer.pos().clone();
        let wrapper_file = wrapper_pos.file.clone();
        // Use lexer line:col + work fn name for a stable, unique
        // synthetic name (avoids needing a Parser-level counter).
        let wrapper_name = format!(
            "__par_destructure_w_{}_{}_{work_id}",
            wrapper_pos.line, wrapper_pos.pos
        );
        let wrapper_d_nr = self
            .data
            .add_def(&wrapper_name, &wrapper_pos, DefType::Function);
        let _ = self
            .data
            .add_attribute(&mut self.lexer, wrapper_d_nr, "t", tuple_tp.clone());
        self.data.set_returned(wrapper_d_nr, ret_type.clone());

        // Build wrapper variable table
        let mut wrapper_vars = Function::new(&wrapper_name, &wrapper_file);
        let t_var = wrapper_vars.add_variable("t", tuple_tp, &mut self.lexer);
        wrapper_vars.become_argument(t_var);
        wrapper_vars.defined(t_var);

        // Translate user_args: Var(destructure_var_nrs[i]) → tuple
        // element read; other shapes pass through verbatim.
        let mut wrapper_call_args: Vec<Value> = Vec::with_capacity(user_args.len());
        for arg in &user_args {
            let arg_var_nr = if let Value::Var(v) = arg.unspan() {
                Some(*v)
            } else {
                None
            };
            if let Some(v) = arg_var_nr
                && let Some(idx) = destructure_var_nrs.iter().position(|&dv| dv == v)
            {
                let elem_offset = if tuple_d_nr == u32::MAX {
                    crate::data::element_stack_offsets(&elem_types)[idx] as u32
                } else if let Some(offs) = crate::data::stored_tuple_offsets_for_def(
                    &self.data,
                    &self.database,
                    tuple_d_nr,
                    elem_types.len(),
                ) {
                    u32::from(offs[idx])
                } else {
                    crate::data::element_stack_offsets(&elem_types)[idx] as u32
                };
                let read = self.get_val(
                    &elem_types[idx],
                    false,
                    elem_offset,
                    Value::Var(t_var),
                    u32::MAX,
                );
                wrapper_call_args.push(read);
            } else {
                wrapper_call_args.push(arg.clone());
            }
        }

        let body_call = Value::Call(work_d_nr, wrapper_call_args);
        let body = v_block(
            vec![Value::Return(Box::new(body_call))],
            ret_type.clone(),
            "destructure_wrapper",
        );
        self.data.definitions[wrapper_d_nr as usize].code = body;
        self.data.definitions[wrapper_d_nr as usize].variables = wrapper_vars;

        (wrapper_d_nr, ret_type, Vec::new(), Vec::new())
    }

    // Desugar `for a in vec par(b = worker(a), N) { body }` into an
    // index-based loop over the `parallel_for` result vector.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse_parallel_for_loop(
        &mut self,
        code: &mut Value,
        elem_var: &str,
        in_type: &Type,
        vec_expr: &Value,
        fill: Value,
        loop_nr: u16,
        destructure_names: Option<&[String]>,
    ) {
        // Consume opening '('.
        self.lexer.token("(");

        // Validate: parallel syntax requires a vector input.
        let elem_tp = if let Type::Vector(_, _) = in_type {
            self.for_type(in_type)
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "par(...) requires a vector<T> input, not {}",
                    in_type.name(&self.data)
                );
            }
            self.skip_to_parallel_body();
            self.vars.finish_loop(loop_nr);
            return;
        };

        // Parse: result_name = worker_call , threads )
        let Some(result_name) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect result variable name after 'par('"
                );
            }
            self.skip_to_parallel_body();
            self.vars.finish_loop(loop_nr);
            return;
        };
        if !self.lexer.has_token("=") {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect '=' after result name '{}' in par(...)",
                    result_name
                );
            }
            self.skip_to_parallel_body();
            self.vars.finish_loop(loop_nr);
            return;
        }

        // Create the element variable so the worker call expression can resolve it.
        // (e.g. `calc(a)` needs `a` in scope during parsing even though the body
        // never runs `a` directly — the parallel map handles that.)
        //
        // Plan-04 B.3 follow-up: the body of
        // `for a in items par(b = worker(a), N) { ... a.iv ... }`
        // is parsed against this `elem_var_nr`, but the desugared
        // loop iterates over an `idx` counter and never writes `a` —
        // so the slot allocator would never place it.  `a` is treated
        // as an inline alias for `OpGetVector(items, idx)`, same as
        // `b` → `OpGetVector(results, idx)`.  build_parallel_for_ir
        // performs the actual Var→accessor rewrite after body parse,
        // once `idx_var` exists.
        let elem_var_nr = self.create_var(elem_var, &elem_tp);
        self.vars.defined(elem_var_nr);
        if matches!(elem_tp, Type::Integer(_)) {
            self.vars.in_use(elem_var_nr, true);
        }

        // P235 par half: when the for-loop binds a tuple destructure
        // (`for (a, b) in pairs par(r = work(a, b), N) { ... }`),
        // the destructured names need to be in scope BEFORE
        // parse_parallel_worker parses the worker call.  Define them
        // here as proper variables typed from the tuple's element
        // types — same pattern as the non-par destructure setup at
        // parse_for:1289-1346.  The variables persist through the
        // body too (matching non-par destructure semantics).
        //
        // The worker call itself is parsed via a destructure-aware
        // path (`parse_destructure_par_worker`) that captures ALL
        // user args (parse_parallel_worker dummies the first arg)
        // and synthesizes a wrapper fn `__par_destructure_w_<N>(t)
        // -> ret { work(t.0, t.1, ...) }`.  Par dispatch then calls
        // the wrapper with the tuple loop element as its single arg.
        let destructure_var_nrs: Option<Vec<u16>> = if let Some(names) = destructure_names {
            let elem_types_opt: Option<Vec<Type>> = match &elem_tp {
                Type::Tuple(elems) => Some(elems.clone()),
                Type::Reference(d_nr, _) if self.data.def(*d_nr).name().starts_with("__tuple<") => {
                    Some(
                        self.data
                            .def(*d_nr)
                            .attributes
                            .iter()
                            .map(|a| a.typedef.clone())
                            .collect(),
                    )
                }
                _ => None,
            };
            if let Some(elem_types) = elem_types_opt {
                if elem_types.len() == names.len() {
                    Some(
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, name)| {
                                let var = self.create_var(name, &elem_types[i]);
                                self.vars.defined(var);
                                self.vars.in_use(var, true);
                                var
                            })
                            .collect(),
                    )
                } else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "for-destructure: pattern has {} names but iterated tuple has {} elements",
                            names.len(),
                            elem_types.len()
                        );
                    }
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Resolve worker function: consumes the worker call tokens up to the ','.
        let (fn_d_nr, ret_type, extra_vals, _extra_types) =
            if let Some(d_var_nrs) = destructure_var_nrs.as_ref() {
                self.parse_destructure_par_worker(&elem_tp, d_var_nrs)
            } else {
                self.parse_parallel_worker(elem_var, &elem_tp)
            };

        // Plan-06 phase 5b' — par-safety DEEP check at ERROR level.
        // Recurses through user-fn callees until it hits a direct
        // call to a `Purity::Impure(ParentWrite)` stdlib fn, or
        // until every path bottoms out in pure/host_io/prng/io/
        // par_call/native primitives.  Unannotated declared-only
        // natives (Op*, n_*) are treated as safe — they're C-level
        // primitives, and the ones that DO write to parent state
        // are explicitly tagged in `default/01_code.loft`.
        //
        // Emits Level::Error: writes from a worker to parent state
        // silently vanish at thread join (D2.0).  The error gives
        // the full reachability chain so the user can see exactly
        // which helper introduces the offending call.
        if !self.first_pass
            && fn_d_nr != u32::MAX
            && let Some(chain) = crate::scopes::worker_calls_parent_write_deep(&self.data, fn_d_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "par() worker {} — a par worker's captured state is READ-ONLY (@PLN102 C93): a \
                     write to shared parent state is a data race, which loft disallows rather \
                     than run.  Return the value from the worker and accumulate it in the fold \
                     body instead.",
                chain
            );
        }

        // Comma separating worker from thread count.
        self.lexer.token(",");
        let mut threads_expr = Value::Null;
        self.expression(&mut threads_expr);
        // Closing ')'.
        self.lexer.token(")");

        // Compute element size from the return type.
        // return_size =  0 signals text mode to n_parallel_for.
        // return_size = -1 signals reference (struct) mode.
        let return_size: i32 = if matches!(&ret_type, Type::Text(_)) {
            0 // sentinel: text mode — workers collect Strings, main thread stores refs
        } else if matches!(
            &ret_type,
            Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
        ) {
            // Reference mode — workers return a DbRef into their own
            // store; main deep-copies via copy_from_worker.  Plan-06
            // phase 1 G1: struct-enum returns (Enum variants with
            // payload, e.g. `Verdict::Pass{score}`) are heap-typed
            // (`heap_def_nr().is_some()`) so they share the ref path
            // verbatim.  This closes the size-8 gate for variant payloads.
            // Plan-06 phase 1 G6: vector<T> returns also route here —
            // the worker constructs the vector in its own output
            // store and the main thread deep-copies it via the same
            // copy_from_worker mechanism.
            // Plan-06 ARC.md A6.d: keyed collections (Sorted / Hash /
            // Index / Radix) are stored as DbRefs to their backing
            // records and route through the same ref path; the rebase
            // walk in `data::owned_elements` already enumerates their
            // internal owned-DbRef fields.
            -1
        } else {
            let sz = i32::from(var_size(&ret_type, &Context::Argument));
            // Plan-06 phase 1 G4 — accept Type::Function returns
            // (size 20 = 8B d_nr + 12B closure DbRef).  Workers
            // write the 20-byte fn-ref into per-worker output
            // slots; main thread copies bytes back via the
            // execute_at_raw_to path in run_parallel_direct.
            let is_fn_ref = matches!(&ret_type, Type::Function(_, _, _));
            if !self.first_pass && fn_d_nr != u32::MAX && (sz == 0 || (sz > 8 && !is_fn_ref)) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Parallel worker return type '{}' (size {sz}) is not supported",
                    ret_type.name(&self.data)
                );
            }
            // A non-capturing fn-ref return (e.g. `return add5;`) is fine, but a
            // CAPTURING closure can't be returned from a par worker: its captured
            // environment lives in the worker's per-thread store and is dropped at
            // join, so the fn-ref would dangle.  Reject it with a clear message
            // instead of the raw out-of-bounds panic the dangling ref triggers.
            if is_fn_ref
                && !self.first_pass
                && fn_d_nr != u32::MAX
                && worker_returns_capturing_closure(self.data.def(fn_d_nr).code())
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a parallel worker cannot return a capturing closure — its \
                     captured state lives in the worker thread's store and cannot \
                     cross back.  Return a non-capturing function reference, or \
                     compute the result directly in the worker."
                );
            }
            sz.max(1) // fallback to 1 if unknown
        };
        // Use the actual inline element size from the database (e.g. 4 for Score{value:integer},
        // 8 for Range{lo,hi:integer}).  var_size() returns size_of::<DbRef>() for reference types,
        // which is wrong for inline vector element storage.
        let elem_size = {
            let elm_td = self.data.type_elm(&elem_tp);
            // Plan-06 phase 1 G2.1 — narrow-integer vector inputs
            // (vector<u8>, vector<i32>) store one element per
            // forced_size byte slot.  IntegerSpec::vector_narrow_width()
            // returns 1/2/4 for u8/i16/i32 and matches the iterator
            // dispatch in collections.rs:105-113 for non-par for loops.
            // Without this, var_size() returned 8 for any Integer
            // and par read garbage at row_idx*8 instead of row_idx*4.
            if let Type::Integer(spec) = &elem_tp
                && let Some(n) = spec.vector_narrow_width()
            {
                i32::from(n)
            } else if matches!(&elem_tp, Type::Function(_, _, _)) {
                // Plan-06 phase 4d.A.2 — fn-ref vector storage is
                // 4-byte i32 d_nr (matches `data::element_stack_size(Type::Function)`).
                // The known_type / db_size lookup below would return
                // var_size(.., Argument) = 20 (the wide stack-slot
                // width for fn-refs), which is wrong for vector
                // stride.  Hard-code 4 here so par steps through the
                // vector in 4-byte increments matching `OpSetInt4`'s
                // narrow writes.
                4
            } else {
                let known = self.data.def(elm_td).known_type();
                let db_size = i32::from(self.database.size(known));
                if db_size > 0 {
                    db_size
                } else {
                    i32::from(var_size(&elem_tp, &Context::Argument))
                }
            }
        };

        self.build_parallel_for_ir(
            code,
            &result_name,
            fn_d_nr,
            &ret_type,
            elem_size,
            return_size,
            vec_expr,
            threads_expr,
            fill,
            loop_nr,
            extra_vals,
            elem_var_nr,
            &elem_tp,
        );
    }

    // parallel_for IR builder; threads unrelated IR params alongside &mut self — no sensible grouping
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn build_parallel_for_ir(
        &mut self,
        code: &mut Value,
        result_name: &str,
        fn_d_nr: u32,
        ret_type: &Type,
        elem_size: i32,
        return_size: i32,
        vec_expr: &Value,
        threads_expr: Value,
        fill: Value,
        loop_nr: u16,
        extra_args: Vec<Value>,
        elem_var: u16,
        elem_tp: &Type,
    ) {
        let ref_d_nr = self.data.def_nr("reference");
        let results_ref_type = Type::Reference(ref_d_nr, crate::data::Deps::none());
        let par_for_d_nr = self.data.def_nr("n_parallel_for");

        // Plan-06 PRIORITY.md spine step 8c — compute the queue gate
        // up front so we can avoid declaring a `par_results` slot for
        // the streaming queue path.  In 8b/c that slot was created
        // and `defined()` unconditionally; on the queue path nothing
        // ever wrote to it, so scope-analysis + `generate_block`'s
        // eval-stack residue check emitted a phantom `OpFreeStack(8)`
        // at the par-block tail.  The discard underflowed below the
        // function frame, corrupted `Return discard`'s wraparound,
        // and SIGSEGV'd on the resulting bogus `code_pos`.  Skipping
        // the slot keeps the codegen tracker honest.
        //
        // The buf_get / buf_drop d_nrs are looked up once here; the
        // dispatch site below clones from these.
        let queue_d_nr = self.data.def_nr("n_parallel_queue");
        let buf_get_d_nr = self.data.def_nr("n_parallel_buf_get");
        let buf_drop_d_nr = self.data.def_nr("n_parallel_buf_drop");
        let queue_narrow_d_nr = self.data.def_nr("n_parallel_queue_narrow");
        let buf_get_narrow_d_nr = self.data.def_nr("n_parallel_buf_get_narrow");
        let buf_drop_narrow_d_nr = self.data.def_nr("n_parallel_buf_drop_narrow");
        let queue_text_d_nr = self.data.def_nr("n_parallel_queue_text");
        let buf_get_text_d_nr = self.data.def_nr("n_parallel_buf_get_text");
        let buf_drop_text_d_nr = self.data.def_nr("n_parallel_buf_drop_text");
        let queue_ref_d_nr = self.data.def_nr("n_parallel_queue_ref");
        let buf_get_ref_d_nr = self.data.def_nr("n_parallel_buf_get_ref");
        let buf_drop_ref_d_nr = self.data.def_nr("n_parallel_buf_drop_ref");
        let queue_fn_d_nr = self.data.def_nr("n_parallel_queue_fn");
        let buf_get_fn_d_nr = self.data.def_nr("n_parallel_buf_get_fn");
        let buf_drop_fn_d_nr = self.data.def_nr("n_parallel_buf_drop_fn");
        let early_is_primitive_return = !matches!(
            ret_type,
            Type::Text(_)
                | Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Function(_, _, _)
                | Type::Vector(_, _)
                | Type::Unknown(_)
        );
        // ARC.md A3.6 — `float` (8B f64) joins 8B Integer on the
        // wide-Queue path; the body's read goes through
        // `parallel_buf_get_float` for typed `f64::from_bits` recovery.
        let early_ret_size_8 = matches!(ret_type, Type::Integer(spec) if u32::from(crate::variables::size(
            &Type::Integer(*spec),
            &Context::Argument,
        )) == 8)
            || matches!(ret_type, Type::Float);
        // Plan-06 ARC.md A3 / A3.5 — narrow primitive returns route
        // through n_parallel_queue_narrow.  The buf_get_narrow result
        // is i64; for non-Integer shapes we wrap with a conversion Op
        // that maps i64 → bool / character / enumerate.  Single /
        // Float (A3.6) still need new IR bit-cast Ops.
        //
        // Shape coverage:
        //   Integer narrow (forced_size 1/2/4) — A3, no wrap needed.
        //   Boolean — A3.5, wrap with OpConvBoolFromInt.
        //   Character — A3.5, wrap with OpConvCharacterFromInt.
        //   Enum (no payload) — A3.5, wrap with OpCastEnumFromInt.
        //   Single / Float — A3.6, deferred (no bit-cast IR Op).
        let early_narrow_route = narrow_route_for(ret_type);
        let early_route_int_queue = early_is_primitive_return
            && fn_d_nr != u32::MAX
            && early_ret_size_8
            && queue_d_nr != u32::MAX
            && buf_get_d_nr != u32::MAX
            && buf_drop_d_nr != u32::MAX;
        let early_route_narrow_queue = early_is_primitive_return
            && fn_d_nr != u32::MAX
            && early_narrow_route.is_some()
            && queue_narrow_d_nr != u32::MAX
            && buf_get_narrow_d_nr != u32::MAX
            && buf_drop_narrow_d_nr != u32::MAX;
        let early_route_text_queue = matches!(ret_type, Type::Text(_))
            && fn_d_nr != u32::MAX
            && queue_text_d_nr != u32::MAX
            && buf_get_text_d_nr != u32::MAX
            && buf_drop_text_d_nr != u32::MAX;
        // 8d.3: route Reference + struct-enum-payload + Vector
        // returns through Queue.  All three share the adopt-and-
        // rebase contract; the per-thread reserved-slot-range
        // allocator (8d.3 in `run_parallel_queue_ref`) ensures
        // worker-written DbRefs already live in parent namespace
        // so cross-worker collision is eliminated.
        let early_route_ref_queue = matches!(
            ret_type,
            Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
        ) && fn_d_nr != u32::MAX
            && queue_ref_d_nr != u32::MAX
            && buf_get_ref_d_nr != u32::MAX
            && buf_drop_ref_d_nr != u32::MAX;
        // ARC.md A6.b — fn-ref returns route through a packed-buffer
        // queue (par_fn_buffer_stack) instead of the heap result
        // vector.  See run_parallel_queue_fn for the runtime; the
        // body substitution diverges below to emit
        // `Set(b_var, Call(buf_get_fn, [idx]))` per iteration
        // instead of inline-expanding b_var via replace_var_in_ir.
        let early_route_fn_queue = matches!(ret_type, Type::Function(_, _, _))
            && fn_d_nr != u32::MAX
            && queue_fn_d_nr != u32::MAX
            && buf_get_fn_d_nr != u32::MAX
            && buf_drop_fn_d_nr != u32::MAX;
        let early_route_through_queue = early_route_int_queue
            || early_route_narrow_queue
            || early_route_text_queue
            || early_route_ref_queue
            || early_route_fn_queue;

        // Create result-reference variable — only on the materialised
        // path.  The queue path stores results in
        // `Stores::par_buffer_stack` / `_text_buffer_stack` and never
        // touches a heap result vector.
        let results_var = if early_route_through_queue {
            None
        } else {
            let v = self.create_unique("par_results", &results_ref_type);
            self.vars.defined(v);
            Some(v)
        };

        // Create index variable (b#index).
        let idx_var = self.create_var(&format!("{result_name}#index"), &I32);
        self.vars.defined(idx_var);
        self.vars.in_use(idx_var, true);

        // Create length variable (par_len#N).
        let len_var = self.create_unique("par_len", &I32);
        self.vars.defined(len_var);
        self.vars.in_use(len_var, true);

        // Create the result element variable (b) with the worker's return type.
        // On the first pass fn_d_nr is u32::MAX; use Type::Unknown so that the second
        // pass can update the type to the correct one (Float, Boolean, etc.) via
        // add_variable's "update if unknown" logic.  Using I32 here caused the type
        // to stick as integer even when the worker returns float or boolean.
        let b_type = if matches!(ret_type, Type::Unknown(_)) {
            I32.clone()
        } else if fn_d_nr == u32::MAX {
            // First pass: placeholder — will be replaced on second pass.
            Type::Unknown(u32::MAX)
        } else if let Type::Text(_) = ret_type {
            // Strip worker-internal deps — they reference variables in the worker scope.
            Type::Text(crate::data::Deps::none())
        } else {
            ret_type.clone()
        };
        // Plan-04 B.3 follow-up v2 (b3-par-inline.md): each par block gets
        // its OWN uniquely-named `b_var`, so two par blocks sharing the
        // user's loop-variable name can no longer collide on a single
        // `Function::variables` entry.  During body parsing the user's
        // name is aliased to this `b_var` via `set_name` (same mechanism
        // as match-arm field aliases in `control.rs:867`).  After the
        // body parses, every `Value::Var(b_var)` in the body is rewritten
        // to the element-accessor expression (see post-parse rewrite
        // below) — `b` becomes an inline alias rather than a runtime
        // slot, so there is no `Set(b_var, …)`, no `OpPut*`, and no
        // type-width mismatch to drift the stack.
        //
        // Key it on the stable `loop_nr` (identical across both parser
        // passes), NOT the global `create_unique` counter: that counter can
        // accumulate a different number of increments between pass 1 and
        // pass 2 across many par loops, so a counter-named `b_var` failed to
        // reuse its pass-1 entry — the user name then aliased to a wrong-typed
        // var (`r.len()` on a `text` result seen as `integer`).  A loop-keyed
        // name reuses the same entry on both passes.
        let b_var_name = format!("_{result_name}_par{loop_nr}");
        let b_var = self
            .vars
            .add_variable(&b_var_name, &b_type, &mut self.lexer);
        self.vars.defined(b_var);
        let prior_name_target = self.vars.set_name(result_name, b_var);
        if matches!(b_type, Type::Integer(_) | Type::Unknown(_)) {
            self.vars.in_use(b_var, true);
        }

        // Parse the body block.
        self.vars.loop_var(b_var);
        let in_loop = self.in_loop;
        self.in_loop = true;
        // M11-a: flag that we are inside a par() body so that any `yield`
        // encountered during parsing can emit a compile-time error.
        let outer_par = self.in_par_body;
        self.in_par_body = true;
        let mut block = Value::Null;
        self.parse_block("parallel for", &mut block, &Type::Void);
        let count = self.vars.loop_counter();
        self.in_par_body = outer_par;
        self.in_loop = in_loop;
        self.vars.finish_loop(loop_nr);
        // Restore prior `result_name` alias (or remove ours if none).
        match prior_name_target {
            Some(nr) => {
                self.vars.set_name(result_name, nr);
            }
            None => self.vars.remove_name(result_name),
        }

        // Build IR only when we have a valid function reference.
        if fn_d_nr == u32::MAX || par_for_d_nr == u32::MAX {
            // Errors already reported; emit nothing useful.
            *code = Value::Null;
            return;
        }

        // Plan-06 PRIORITY.md spine step 3 (Discard detection) — when the
        // body never references the worker's result name (`b_var`) and
        // never references the loop variable (`elem_var`), the user
        // doesn't need a materialised result vector.  Lower to a direct
        // call into `n_parallel_discard` (spine step 2 / 3b) which runs
        // workers, drops results, allocates no result vector.
        //
        // Tighter conditions surface in steps 4 (Queue) and 9 (Reduce);
        // until then, only the empty-body case routes here.  Body shapes
        // like `{ log("done"); }` (uses neither r nor x) also qualify
        // but are gated behind a per-Var walk in a later step.
        let body_is_empty = matches!(&block, Value::Null)
            || matches!(&block, Value::Block(bl) if bl.operators.is_empty())
            || matches!(&block, Value::Insert(ops) if ops.is_empty());
        let discard_d_nr = self.data.def_nr("n_parallel_discard");
        if !self.first_pass && body_is_empty && discard_d_nr != u32::MAX && extra_args.is_empty() {
            // Build a Discard call with the same arg layout as
            // n_parallel_for (so the native fn pops in the same order).
            // The body and per-element accessor (b/a inline aliases)
            // are not needed — workers run, results dropped.
            let n_extra_v = Value::Int(0);
            let pf_args = vec![
                vec_expr.clone(),
                Value::Int(elem_size),
                Value::Int(return_size),
                threads_expr,
                Value::Int(fn_d_nr as i32),
                n_extra_v,
            ];
            *code = v_block(
                vec![fill, Value::Call(discard_d_nr, pf_args)],
                Type::Void,
                "par_discard",
            );
            // Loop-counter accounting from the empty body.
            let _ = count;
            return;
        }

        // A14.5/A14.6: auto-select light path for eligible workers.
        // Heap-typed returns (Reference, struct-enum, Text, Unknown) need the
        // heavy path's deep-copy machinery — the light path's `execute_at_raw`
        // memcpy only handles inline returns ≤ 8 bytes.
        // Plan-06 phase 1 G4 — fn-ref returns (Type::Function, 20 bytes)
        // also need the heavy path's per-worker output slot mechanism;
        // the light path writes via 8-byte execute_at_raw and would
        // truncate the closure DbRef.
        let is_primitive_return = !matches!(
            ret_type,
            Type::Text(_)
                | Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Function(_, _, _)
                | Type::Vector(_, _)
                | Type::Unknown(_)
        );
        // ARC.md A4 (closed 2026-05-07) — `n_parallel_for_light` was
        // retired; every primitive return type now routes through the
        // Queue family (route_int_queue / route_narrow_queue / etc.).
        // `actual_par_d_nr` falls back to `n_parallel_for` only for
        // shapes the Queue family doesn't cover (Tuple returns,
        // pending A7).  The native body of `n_parallel_for` panics
        // with a clear diagnostic if it ever runs.
        let _ = is_primitive_return; // light_m elimination retained for future scope-analysis hooks
        let actual_par_d_nr = par_for_d_nr;

        // parallel_for(input, elem_size, return_size, threads, fn_d_nr, [pool_m], extra1, ..., n_extra)
        // n_extra is pushed LAST so it's on top of the stack for popping first.
        let n_extra = extra_args.len();
        let mut pf_args = vec![
            vec_expr.clone(),
            Value::Int(elem_size),
            Value::Int(return_size),
            threads_expr,
            Value::Int(fn_d_nr as i32),
        ];
        // pool_m is hardcoded in the native function (avoids stack-ordering complexity)
        // ARC.md A4: light_m elimination — `n_parallel_for_light` was retired
        pf_args.extend(extra_args);
        pf_args.push(Value::Int(n_extra as i32));

        // Plan-06 PRIORITY.md spine step 8 — primitive 8-byte integer
        // returns dispatch through the streaming Queue path (8a's
        // `n_parallel_queue` + `n_parallel_buf_get`/`_drop`) instead
        // of allocating a heap result vector.
        //
        // After 8b' (this commit) `run_parallel_queue` mirrors
        // `run_parallel_direct`'s input-kind dispatch (DbRef, text,
        // primitive, wide-inline / tuple / fn-ref), so the parser
        // gate no longer needs to filter by input kind — every
        // `is_primitive_return` worker now routes correctly.
        //
        // Gating today:
        //   - `is_primitive_return` (no Text/Reference/Enum-payload/
        //     Function/Vector/Unknown — handled by 8c/8d).
        //   - `Type::Integer(_)` with full 8-byte width.  Narrow
        //     integer widths (u8, i32, etc.) and other primitive
        //     return types (bool, single, character, enum-no-
        //     payload, tuple) need per-size buf_get variants to
        //     mask the high bits and land cleanly into the body's
        //     b_var slot.  Deferred to a future per-size sub-step.
        //   - `fn_d_nr` resolved (partial-parse failures fall
        //     through to the legacy path).
        //   - `n_parallel_queue` / `_buf_get` / `_buf_drop`
        //     registered (defensively, so a stripped stdlib still
        //     parses).
        //
        // Trade-off: workers eligible for the light path
        // (`light_m.is_some()`) skip its per-thread pool optimisation
        // when routed through Queue.  A later sub-step combines
        // light + queue once the architecture stabilises.
        let queue_d_nr = self.data.def_nr("n_parallel_queue");
        let buf_get_d_nr = self.data.def_nr("n_parallel_buf_get");
        let buf_drop_d_nr = self.data.def_nr("n_parallel_buf_drop");
        let queue_narrow_d_nr = self.data.def_nr("n_parallel_queue_narrow");
        let buf_get_narrow_d_nr = self.data.def_nr("n_parallel_buf_get_narrow");
        let buf_drop_narrow_d_nr = self.data.def_nr("n_parallel_buf_drop_narrow");
        let queue_text_d_nr = self.data.def_nr("n_parallel_queue_text");
        let buf_get_text_d_nr = self.data.def_nr("n_parallel_buf_get_text");
        let buf_drop_text_d_nr = self.data.def_nr("n_parallel_buf_drop_text");
        let queue_ref_d_nr = self.data.def_nr("n_parallel_queue_ref");
        let buf_get_ref_d_nr = self.data.def_nr("n_parallel_buf_get_ref");
        let buf_drop_ref_d_nr = self.data.def_nr("n_parallel_buf_drop_ref");
        let queue_fn_d_nr = self.data.def_nr("n_parallel_queue_fn");
        let buf_get_fn_d_nr = self.data.def_nr("n_parallel_buf_get_fn");
        let buf_drop_fn_d_nr = self.data.def_nr("n_parallel_buf_drop_fn");
        // ARC.md A3.6 — `float` (8B f64) joins 8B Integer on the
        // wide-Queue path (mirrors `early_ret_size_8`).
        let ret_size_8 = matches!(ret_type, Type::Integer(spec) if u32::from(crate::variables::size(
            &Type::Integer(*spec),
            &Context::Argument,
        )) == 8)
            || matches!(ret_type, Type::Float);
        // Plan-06 ARC.md A3 / A3.5 — narrow primitive return routing.
        // Mirrors the early-gate logic; see comment above for shape
        // coverage.
        let narrow_route = narrow_route_for(ret_type);
        // 8b: integer-i64 returns route through `n_parallel_queue` +
        // `par_buffer_stack`.
        // 8c: text returns route through `n_parallel_queue_text` +
        // `par_text_buffer_stack` (sibling stack — keeps the per-row
        // read path tight by avoiding an enum match per element).
        // 8d.2: reference / struct-enum-payload returns route through
        // `n_parallel_queue_ref` + `par_ref_buffer_stack` — workers
        // return DbRefs into their own output stores, the dispatcher
        // adopts those stores into the parent and rebases the DbRefs
        // via `Stores::adopt_worker_excess` + `rebase_walk_record`.
        // Vector returns stay on the legacy materialised path for
        // now — they're treated as a Reference but the
        // `parallel_execute_and_collect` branch picks the heap-vector
        // ownership invariants and matches its element-stride
        // accounting differently; 8d.3 generalises Queue dispatch
        // to vector returns.
        // ARC.md A6.b: fn-ref returns route through `n_parallel_queue_fn`
        // + `par_fn_buffer_stack` (packed Vec<u8>, 20 bytes per row).
        // The body substitution diverges: instead of inline-expanding
        // b_var via `replace_var_in_ir`, b_var is kept as a real
        // variable and `Set(b_var, Call(buf_get_fn, [idx]))` is emitted
        // at the top of each iteration body.  This works around the
        // `replace_var_in_ir` limitation that doesn't substitute the
        // u16 fn-ref var index inside `Value::CallRef`.
        let route_int_queue = is_primitive_return
            && fn_d_nr != u32::MAX
            && ret_size_8
            && queue_d_nr != u32::MAX
            && buf_get_d_nr != u32::MAX
            && buf_drop_d_nr != u32::MAX;
        // Plan-06 ARC.md A3 / A3.5 — narrow primitive Queue route.
        // Same gate as route_int_queue but for narrow Integer + bool /
        // character / enum-no-payload (each shape fits 1 or 4 bytes).
        let route_narrow_queue = is_primitive_return
            && fn_d_nr != u32::MAX
            && narrow_route.is_some()
            && queue_narrow_d_nr != u32::MAX
            && buf_get_narrow_d_nr != u32::MAX
            && buf_drop_narrow_d_nr != u32::MAX;
        let route_text_queue = matches!(ret_type, Type::Text(_))
            && fn_d_nr != u32::MAX
            && queue_text_d_nr != u32::MAX
            && buf_get_text_d_nr != u32::MAX
            && buf_drop_text_d_nr != u32::MAX;
        // Late-gate matches early-gate (see comment above).
        // ARC.md A6.d: keyed-collection returns share the ref path
        // (DbRef to backing record + type-driven rebase walk).
        let route_ref_queue = matches!(
            ret_type,
            Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
        ) && fn_d_nr != u32::MAX
            && queue_ref_d_nr != u32::MAX
            && buf_get_ref_d_nr != u32::MAX
            && buf_drop_ref_d_nr != u32::MAX;
        let route_fn_queue = matches!(ret_type, Type::Function(_, _, _))
            && fn_d_nr != u32::MAX
            && queue_fn_d_nr != u32::MAX
            && buf_get_fn_d_nr != u32::MAX
            && buf_drop_fn_d_nr != u32::MAX;
        let route_through_queue = route_int_queue
            || route_narrow_queue
            || route_text_queue
            || route_ref_queue
            || route_fn_queue;

        let stop_cond = self.cl("OpLeInt", &[Value::Var(len_var), Value::Var(idx_var)]);
        let stop = v_if(
            stop_cond,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );

        // Build the body's `b` accessor.  For the queue paths it's
        // `n_parallel_buf_get[_text/_ref/_fn](idx)`; for the materialised
        // path it's `OpGetVector + get_field` indexing the heap
        // result vector.
        // Plan-06 ARC.md A3 / A3.5 — narrow takes priority over wide
        // int.  var_size() returns 8 for all Integer stack slots
        // (post-2c), so route_int_queue would also fire for narrow
        // returns; the narrow path is what we want when storage
        // width is < 8 OR the return type is Boolean / Character /
        // Enum-no-payload.
        let get_call = if route_narrow_queue {
            // Narrow reader takes (idx, return_size, signed).  Returns
            // i64; for non-Integer shapes we wrap with the matching
            // conversion (Op call or OpNeInt-vs-zero) so the body's
            // `r` substitution lands correctly typed.
            let route = narrow_route
                .as_ref()
                .expect("narrow_route set when route_narrow_queue is true");
            // ARC.md A3.6 — TypedBufGet bypasses the i64 reader
            // entirely.  The named buf_get fn is signature-typed
            // (returns `single` etc.) and reads the same per-row
            // bytes via a typed memcpy.  No wrap needed.
            if let NarrowWrap::TypedBufGet(typed_fn_name) = &route.wrap {
                let typed_d_nr = self.data.def_nr(typed_fn_name);
                if typed_d_nr == u32::MAX {
                    // Defensive: stripped stdlib without the typed
                    // reader.  Fall back to the raw narrow buf_get
                    // (will type-mismatch downstream — louder than
                    // a silent miscompile).
                    Value::Call(
                        buf_get_narrow_d_nr,
                        vec![
                            Value::Var(idx_var),
                            Value::Int(i32::from(route.width)),
                            Value::Int(i32::from(route.signed)),
                        ],
                    )
                } else {
                    Value::Call(typed_d_nr, vec![Value::Var(idx_var)])
                }
            } else {
                let raw_call = Value::Call(
                    buf_get_narrow_d_nr,
                    vec![
                        Value::Var(idx_var),
                        Value::Int(i32::from(route.width)),
                        Value::Int(i32::from(route.signed)),
                    ],
                );
                match &route.wrap {
                    NarrowWrap::None => raw_call,
                    NarrowWrap::OpCall(conv_name) => {
                        let conv_d_nr = self.data.def_nr(conv_name);
                        if conv_d_nr == u32::MAX {
                            // Defensive: stripped stdlib without the conv
                            // Op.  Skip the wrap and let downstream type-
                            // check catch the mismatch — much louder than
                            // a silent miscompile.
                            raw_call
                        } else {
                            Value::Call(conv_d_nr, vec![raw_call])
                        }
                    }
                    NarrowWrap::NeZero => {
                        // Boolean wrap: `OpNeInt(buf_get, 0) -> boolean`.
                        let ne_d_nr = self.data.def_nr("OpNeInt");
                        if ne_d_nr == u32::MAX {
                            raw_call
                        } else {
                            Value::Call(ne_d_nr, vec![raw_call, Value::Int(0)])
                        }
                    }
                    NarrowWrap::TypedBufGet(_) => unreachable!("handled above"),
                }
            }
        } else if route_int_queue {
            // ARC.md A3.6 — Float returns reuse the wide u64-row
            // queue but the body needs `parallel_buf_get_float` for
            // typed `f64::from_bits` recovery.
            if matches!(ret_type, Type::Float) {
                let buf_get_float_d_nr = self.data.def_nr("n_parallel_buf_get_float");
                if buf_get_float_d_nr == u32::MAX {
                    Value::Call(buf_get_d_nr, vec![Value::Var(idx_var)])
                } else {
                    Value::Call(buf_get_float_d_nr, vec![Value::Var(idx_var)])
                }
            } else {
                Value::Call(buf_get_d_nr, vec![Value::Var(idx_var)])
            }
        } else if route_text_queue {
            Value::Call(buf_get_text_d_nr, vec![Value::Var(idx_var)])
        } else if route_ref_queue {
            // 8d.2: returns a rebased DbRef (12 bytes) — body field
            // accesses route through the standard OpGetField path,
            // same as Var(struct_var).foo.
            Value::Call(buf_get_ref_d_nr, vec![Value::Var(idx_var)])
        } else if route_fn_queue {
            // ARC.md A6.b: returns a 20-byte fn-ref blob.  Used as
            // the RHS of `Set(b_var, ...)` below — NOT inline-substituted
            // via `replace_var_in_ir` because the body's `f(10)`
            // parses as `CallRef(b_var, [Int(10)])` and `replace_var_in_ir`
            // doesn't substitute the u16 fn-ref var index inside CallRef.
            Value::Call(buf_get_fn_d_nr, vec![Value::Var(idx_var)])
        } else {
            // Use OpGetVector + get_field to extract the element from the result
            // vector. This works for all return types (int, long, float, bool, text)
            // without per-type getter functions.
            let result_elem_size = match return_size {
                0 => 4, // text: 4-byte string pointer per element
                -1 => {
                    // reference: inline struct size from the database
                    let ret_td = self.data.type_def_nr(ret_type);
                    let known = self.data.def(ret_td).known_type();
                    i32::from(self.database.size(known))
                }
                other => other,
            };
            // Plan-07 phase 4 step 4.6 — par-worker iteration over the
            // result vector uses the Nullable peer (iteration site).
            // `idx_var` is bounded by the parallel-loop driver so OOB
            // shouldn't fire here, but keeping the Nullable shape
            // makes the design rule "every iteration site emits
            // Nullable" hold uniformly.
            let get_vec = self.cl(
                "OpGetVectorNullable",
                &[
                    Value::Var(results_var.expect(
                        "materialised path requires results_var; queue gate let it slip through",
                    )),
                    Value::Int(result_elem_size),
                    Value::Var(idx_var),
                ],
            );
            if matches!(ret_type, Type::Reference(_, _)) || fn_d_nr == u32::MAX {
                // fn_d_nr == u32::MAX: worker was rejected (e.g. S23 generator check);
                // skip the type-based field access to avoid crashing on Unknown type.
                get_vec
            } else {
                let vec_tp = self.data.type_def_nr(ret_type);
                if vec_tp == u32::MAX {
                    // Unsupported return type (e.g. iterator<T> in first pass before S23
                    // diagnostic fires): fall back to raw vector access to prevent crash.
                    get_vec
                } else {
                    self.get_field(vec_tp, usize::MAX, get_vec)
                }
            }
        };
        // Plan-04 B.3 follow-up v2 (b3-par-inline.md): rewrite every
        // `Value::Var(b_var)` in the body with a clone of `get_call`.
        // `b` is no longer a runtime variable — each reference expands
        // inline to the accessor expression.  No `Set(b_var, get_call)`
        // is emitted; the body references ARE the reads.  Under the B.3
        // atomic bundle's slot-aware `OpPut*` dispatch this eliminates
        // the type-width mismatch and the stack drift.
        //
        // ARC.md A6.b: fn-ref returns diverge — `f(10)` parses as
        // `CallRef(b_var, [Int(10)])` and `replace_var_in_ir`
        // (src/parser/collections.rs:replace_var_in_ir) walks
        // `CallRef(_, args)` into `args` but doesn't substitute the
        // first u16 (the fn-ref var index).  Inline substitution
        // would leave b_var as a dangling reference.  Instead, keep
        // b_var as a real variable with a 20-byte slot, and prepend
        // `Set(b_var, Call(buf_get_fn, [idx]))` to the body so each
        // iteration's CallRef reads the fresh fn-ref blob.
        if route_fn_queue {
            // Mark b_var as in_use so the slot allocator reserves
            // a 20-byte slot.  The body's `f(10)` (CallRef) uses
            // b_var implicitly through the u16 var index, which
            // doesn't increment the standard use-count tracker.
            self.vars.in_use(b_var, true);
            let init = v_set(b_var, get_call.clone());
            // Prepend init to the body block.  If the body is not
            // already a Block, wrap it in one.
            if let Value::Block(ref mut bl) = block {
                bl.operators.insert(0, init);
            } else {
                let inner = std::mem::replace(&mut block, Value::Null);
                block = v_block(vec![init, inner], Type::Void, "fn-queue body");
            }
        } else {
            replace_var_in_ir(&mut block, b_var, &get_call);
        }

        // apply the same inline-alias treatment to the outer
        // iterator variable `a`.  The desugared loop increments `idx`;
        // `a` is logically `items[idx]` on every iteration.  Rewriting
        // `Var(a)` → `OpGetVectorNullable(items, elem_size, idx)` (plus
        // `get_field` for non-Reference element types, mirroring the
        // `b` path) means `a` never needs a slot.  Without this the
        // allocator leaves `a` at `stack_pos == u16::MAX` and codegen
        // panics `Incorrect var a[65535] versus N`.
        //
        // Plan-07 phase 4 step 4.6 — fused-for-par iteration site →
        // Nullable peer (consistent with all other iteration sites).
        let a_get_vec = self.cl(
            "OpGetVectorNullable",
            &[vec_expr.clone(), Value::Int(elem_size), Value::Var(idx_var)],
        );
        let a_accessor = if matches!(elem_tp, Type::Reference(_, _)) {
            a_get_vec
        } else {
            let elm_td = self.data.type_def_nr(elem_tp);
            if elm_td == u32::MAX {
                a_get_vec
            } else {
                self.get_field(elm_td, usize::MAX, a_get_vec)
            }
        };
        replace_var_in_ir(&mut block, elem_var, &a_accessor);
        let idx_inc = v_set(
            idx_var,
            self.cl("OpAddInt", &[Value::Var(idx_var), Value::Int(1)]),
        );

        let mut lp = vec![stop, block, idx_inc];
        if count != u16::MAX {
            lp.insert(
                3,
                v_set(
                    count,
                    self.cl("OpAddInt", &[Value::Var(count), Value::Int(1)]),
                ),
            );
        }

        let mut for_steps = Vec::new();
        if count != u16::MAX {
            for_steps.push(v_set(count, Value::Int(0)));
        }
        if fill != Value::Null {
            for_steps.push(fill);
        }
        if route_through_queue {
            // Queue path: `n_parallel_queue[_text]` returns the row
            // count directly, doubling as `par_len` and saving the
            // `OpLengthVector(input)` call.  The result heap vector
            // (`results_var`) is never allocated; per-iteration reads
            // come from `stores.par_buffer_stack` (int),
            // `par_text_buffer_stack` (text), or
            // `par_ref_buffer_stack` (ref).  After the loop, pop
            // the buffer with `n_parallel_buf_drop[_text/_ref]()` so
            // the next par-call doesn't see a stale buffer
            // underneath; the ref drop also frees adopted worker
            // stores.
            let (call_d_nr, drop_d_nr, label_loop, label_block) = if route_text_queue {
                (
                    queue_text_d_nr,
                    buf_drop_text_d_nr,
                    "Parallel for loop (queue text)",
                    "Parallel for block (queue text)",
                )
            } else if route_narrow_queue {
                // Plan-06 ARC.md A3 / A3.5 — for the narrow path,
                // pf_args[2] (return_size) was set by var_size which
                // returns the wide stack-slot width (8 bytes); the
                // narrow runtime expects the actual stride (1/2/4).
                // Patch in place before the queue_call below grabs
                // `pf_args`.
                let route = narrow_route
                    .as_ref()
                    .expect("narrow_route set when route_narrow_queue is true");
                pf_args[2] = Value::Int(i32::from(route.width));
                (
                    queue_narrow_d_nr,
                    buf_drop_narrow_d_nr,
                    "Parallel for loop (queue narrow)",
                    "Parallel for block (queue narrow)",
                )
            } else if route_ref_queue {
                (
                    queue_ref_d_nr,
                    buf_drop_ref_d_nr,
                    "Parallel for loop (queue ref)",
                    "Parallel for block (queue ref)",
                )
            } else if route_fn_queue {
                (
                    queue_fn_d_nr,
                    buf_drop_fn_d_nr,
                    "Parallel for loop (queue fn)",
                    "Parallel for block (queue fn)",
                )
            } else {
                (
                    queue_d_nr,
                    buf_drop_d_nr,
                    "Parallel for loop (queue)",
                    "Parallel for block (queue)",
                )
            };
            let queue_call = Value::Call(call_d_nr, pf_args);
            for_steps.push(v_set(len_var, queue_call));
            for_steps.push(v_set(idx_var, Value::Int(0)));
            for_steps.push(v_loop(lp, label_loop));
            for_steps.push(Value::Call(drop_d_nr, Vec::new()));
            *code = v_block(for_steps, Type::Void, label_block);
        } else {
            // Materialised path (text / ref / fn-ref / vector / non-
            // integer primitives).  Allocates a heap result vector
            // and indexes it via `OpGetVector`.  Step 8c/8d/8b' will
            // route the remaining cases through Queue; step 8e then
            // retires `parallel_execute_and_collect` and the
            // `Stitch::Concat` arm.
            let pf_call = Value::Call(actual_par_d_nr, pf_args);
            // len(input_vec) — compute once before the loop.
            let len_call = self.cl("OpLengthVector", std::slice::from_ref(vec_expr));
            for_steps.push(v_set(len_var, len_call));
            for_steps.push(v_set(
                results_var.expect(
                    "materialised path requires results_var; queue gate let it slip through",
                ),
                pf_call,
            ));
            for_steps.push(v_set(idx_var, Value::Int(0)));
            for_steps.push(v_loop(lp, "Parallel for loop"));
            *code = v_block(for_steps, Type::Void, "Parallel for block");
        }
    }

    // Consume the remaining `par(...)` tokens and then the body block so the
    // parser can recover after an error in the parallel clause.
    // Called after '(' has already been consumed, so this drains to ')'.
    /// Compiler special-case for `map(v: vector<T>, f: fn(T) -> U) -> vector<U>`.
    /// Generates inline bytecode equivalent to `[for elm in v { f(elm) }]`.
    #[allow(clippy::too_many_lines)]
    // @F24 — higher-order functions (map / filter / reduce)
    pub(crate) fn parse_map(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        let placeholder = Type::Vector(Box::new(Type::Unknown(0)), crate::data::Deps::none());
        // On first pass, return the concrete output vector type derived from the function's
        // return type so that downstream variables (e.g. `r = map(...)`) get the right type
        // and subsequent `for x in r` iterations resolve correctly.
        // We must NOT create unique variables here — only determine the type.
        if self.first_pass {
            // On first pass, infer output element type from the input vector.
            // The lambda return type may not be fully resolved yet; defaulting
            // to the input element type is correct for most cases (e.g. x * 10)
            // and lets downstream code like r[0] type-check.
            if let Type::Vector(elm, _) = &types[0] {
                return Type::Vector(elm.clone(), crate::data::Deps::none());
            }
            return placeholder;
        }
        if list.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map requires 2 arguments: map(vector, fn f)"
            );
            return placeholder;
        }
        let _in_elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: first argument must be a vector"
            );
            return placeholder;
        };
        let (fn_param_types, fn_ret_type) = if let Type::Function(params, ret, _) = &types[1] {
            (params.clone(), *ret.clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: second argument must be a function reference (use fn <name>)"
            );
            return placeholder;
        };
        if fn_param_types.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: function must take exactly one argument"
            );
            return placeholder;
        }
        // D-clo-2 — a stored short `|x|` lambda whose types could not be inferred (it was
        // assigned without a type context — `g = |y| { y*2 }`) arrives here as
        // `Function([Unknown], Unknown)`.  Building a result vector of an Unknown element type
        // panics in `build_comprehension_code` (`def(u32::MAX)`).  Emit the SAME guiding
        // diagnostic the lambda already gets when used standalone / called directly, instead of
        // crashing — the inline `.map(|y| …)` form (which has the element-type hint) is unaffected.
        // D-clo-2 — a stored short `|x|` lambda assigned without a type context (`g = |y| {…}`)
        // is un-inferrable: it arrives here with a GARBAGE signature (a `text`/`void` default,
        // or `Unknown`).  `map`'s result element is the lambda's return type, so a `void`/unknown
        // return builds a `vector<void>` and panics in `build_comprehension_code`
        // (`def(u32::MAX)`).  Emit the SAME guiding diagnostic the lambda already gets standalone,
        // instead of crashing.  The inline `.map(|y| …)` form has the element-type hint and a
        // real return type, so it is unaffected.
        if !self.first_pass
            && (fn_ret_type.is_unknown()
                || matches!(fn_ret_type, Type::Void)
                || fn_param_types.iter().any(Type::is_unknown))
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "cannot infer the type of the function passed to `map` — a short `|x|` lambda \
                 stored in a variable has no type context. Pass it inline to `.map(…)`, or use \
                 the long form `fn(x: <type>) -> <ret> {{ … }}` which declares its types"
            );
            return placeholder;
        }
        // accept both static fn-refs (Value::Int) and fn-ref variables/lambdas.
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            Some(*d as u32)
        } else {
            None // fn-ref variable or lambda — will use CallRef
        };
        // For CallRef path, store the fn-ref value in a local variable.
        let fn_ref_var = if fn_d_nr.is_none() {
            let v = self.create_unique("map_fn", &types[1]);
            self.vars.defined(v);
            Some(v)
        } else {
            None
        };

        let mut in_type = types[0].clone();
        let vec_copy_var = self.create_unique("map_vec", &in_type);
        in_type = in_type.depending(vec_copy_var);

        let iter_var = self.create_unique("map_idx", &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique("map_elm", &var_tp);
        self.vars.defined(for_var);

        let out_elem = fn_ret_type.clone();
        let result_type = Type::Vector(Box::new(out_elem.clone()), crate::data::Deps::none());
        let result_vec = self.create_unique("map_result", &result_type);
        let elm = self.unique_elm_var(&result_type, &out_elem, result_vec);

        let mut create_iter_code = Value::Var(vec_copy_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter_code, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut fill = v_set(vec_copy_var, list[0].clone());
        // for CallRef path, assign the fn-ref value before the loop.
        if let Some(fv) = fn_ref_var {
            fill = Value::Insert(vec![fill, v_set(fv, list[1].clone())]);
        }

        let body = if let Some(d) = fn_d_nr {
            Value::Call(d, vec![Value::Var(for_var)])
        } else {
            Value::CallRef(fn_ref_var.unwrap(), vec![Value::Var(for_var)])
        };

        self.data.vector_def(&mut self.lexer, &out_elem);

        let tp = result_type.clone();
        // Reset val so build_comprehension_code creates a fresh result vector rather than
        // pre-seeding it with the LHS variable (which would cause a self-reference panic).
        *val = Value::Null;
        self.build_comprehension_code(
            result_vec,
            &Value::Var(result_vec),
            elm,
            &out_elem,
            &in_type,
            &var_tp,
            for_var,
            for_next,
            None,
            fill,
            create_iter_code,
            Value::Null,
            body,
            val,
            false,
            false,
            true,
            tp,
        )
    }

    /// Validate `filter` arguments and extract `(in_elem_type, fn_d_nr)`.
    /// Returns `Err(placeholder)` on validation failure.
    pub(crate) fn parse_filter_validate(
        &mut self,
        list: &[Value],
        types: &[Type],
    ) -> Result<(Type, Option<u32>), Type> {
        let placeholder = Type::Vector(Box::new(Type::Unknown(0)), crate::data::Deps::none());
        if list.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter requires 2 arguments: filter(vector, fn pred)"
            );
            return Err(placeholder);
        }
        let in_elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: first argument must be a vector"
            );
            return Err(placeholder);
        };
        let (fn_param_types, fn_ret_type) = if let Type::Function(params, ret, _) = &types[1] {
            (params.clone(), *ret.clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: second argument must be a function reference (use fn <name>)"
            );
            return Err(placeholder);
        };
        if fn_param_types.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: predicate must take exactly one argument"
            );
            return Err(placeholder);
        }
        if fn_ret_type != Type::Boolean {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: predicate must return boolean"
            );
            return Err(placeholder);
        }
        // accept both static fn-refs and fn-ref variables/lambdas.
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            Some(*d as u32)
        } else {
            None
        };
        Ok((in_elem_type, fn_d_nr))
    }

    pub(crate) fn parse_filter(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        let placeholder = Type::Vector(Box::new(Type::Unknown(0)), crate::data::Deps::none());
        // On first pass, return the concrete output type from the input vector's element type.
        if self.first_pass {
            if !types.is_empty()
                && let Type::Vector(elm, _) = &types[0]
            {
                return Type::Vector(elm.clone(), crate::data::Deps::none());
            }
            return placeholder;
        }
        let (in_elem_type, fn_d_nr) = match self.parse_filter_validate(list, types) {
            Ok(v) => v,
            Err(t) => return t,
        };
        // for CallRef path, store the fn-ref value in a local variable.
        let fn_ref_var = if fn_d_nr.is_none() {
            let v = self.create_unique("filter_fn", &types[1]);
            self.vars.defined(v);
            Some(v)
        } else {
            None
        };

        let mut in_type = types[0].clone();
        let vec_copy_var = self.create_unique("filter_vec", &in_type);
        in_type = in_type.depending(vec_copy_var);

        let iter_var = self.create_unique("filter_idx", &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique("filter_elm", &var_tp);
        self.vars.defined(for_var);

        let out_elem = in_elem_type.clone();
        let result_type = Type::Vector(Box::new(out_elem.clone()), crate::data::Deps::none());
        let result_vec = self.create_unique("filter_result", &result_type);
        let elm = self.unique_elm_var(&result_type, &out_elem, result_vec);

        let mut create_iter_code = Value::Var(vec_copy_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter_code, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut fill = v_set(vec_copy_var, list[0].clone());
        if let Some(fv) = fn_ref_var {
            fill = Value::Insert(vec![fill, v_set(fv, list[1].clone())]);
        }

        let if_step = if let Some(d) = fn_d_nr {
            Value::Call(d, vec![Value::Var(for_var)])
        } else {
            Value::CallRef(fn_ref_var.unwrap(), vec![Value::Var(for_var)])
        };

        let body = Value::Var(for_var);

        self.data.vector_def(&mut self.lexer, &out_elem);

        let tp = result_type.clone();
        // Reset val so build_comprehension_code creates a fresh result vector.
        *val = Value::Null;
        self.build_comprehension_code(
            result_vec,
            &Value::Var(result_vec),
            elm,
            &out_elem,
            &in_type,
            &var_tp,
            for_var,
            for_next,
            None,
            fill,
            create_iter_code,
            if_step,
            body,
            val,
            false,
            false,
            true,
            tp,
        )
    }

    /// Build ops to construct a struct/struct-enum instance, replicating the IR that
    /// `parse_object` produces. Returns the ops list and the work variable holding the result.
    fn build_object_ops(&mut self, td_nr: u32, fields: &[(usize, Value)]) -> (Vec<Value>, u16) {
        let ret = self.data.def(td_nr).returned().clone();
        let w = self.vars.work_refs(&ret, &mut self.lexer);
        self.data.set_referenced(td_nr, self.context, Value::Null);
        let tp = i32::from(self.data.def(td_nr).known_type());
        let mut list: Vec<Value> = vec![
            v_set(w, Value::Null),
            self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp)]),
        ];
        for &(f_nr, ref val) in fields {
            list.push(self.set_field_no_check(td_nr, f_nr, 0, Value::Var(w), val.clone()));
        }
        (list, w)
    }

    /// Compile-time unroll `for f in s#fields` into one block per field.
    fn parse_field_iteration(
        &mut self,
        loop_var_name: &str,
        struct_def_nr: u32,
        source_expr: &Value,
        code: &mut Value,
    ) {
        let field_def_nr = self.data.def_nr("StructField");
        let field_type = Type::Reference(field_def_nr, crate::data::Deps::none());
        let loop_var = self.create_var(loop_var_name, &field_type);
        self.vars.defined(loop_var);

        let mut body = Value::Null;
        self.parse_block("fields", &mut body, &Type::Void);

        // @PLN85 45-field-iter — OWNED-text locals bound in the body (a text
        // match-payload binding `is FvText { v }`, or a `"{v}"` interpolation
        // `__work`) are PER-ITERATION temporaries.  The body is parsed ONCE and
        // cloned per field below, so all clones would share ONE var; the scope
        // machinery then frees it only at its LAST textual use — which sits inside
        // a CONDITIONAL match arm — and when the last field doesn't take that arm
        // the owned copy from an earlier text field orphans on the interpreter
        // (native RAII-drops it).  Give each field-block its OWN copy so each is
        // freed at its own block's use, exactly like a real loop's per-iteration
        // scope.  Identify them as owned-text vars ASSIGNED (`Set`) inside the body
        // — an outer accumulator (`r += v`) is an `OpAppendText`, not a `Set`, so it
        // is left shared; a borrow/skip_free binding owns no allocation.
        let mut set_targets: Vec<u16> = Vec::new();
        Self::collect_set_targets(&body, &mut set_targets);
        let owned_text_locals: Vec<u16> = set_targets
            .into_iter()
            .filter(|&v| {
                (v as usize) < self.vars.count() as usize
                    && matches!(self.vars.tp(v).base(), Type::Text(_))
                    // OWNED text (empty deps) — a payload-copy binding, not a
                    // borrow view (a `["mf"]`-typed view owns no allocation).
                    && self.vars.tp(v).depend().is_empty()
                    && !self.vars.is_argument(v)
                    // ONLY the match-payload binding (`_mv_<field>`): its free
                    // lands at the OUTER last-use (a conditional arm) so it orphans
                    // across the unroll.  An interpolation `__work` buffer is freed
                    // per-statement WITHIN its arm already — remapping it splits the
                    // var away from that free and re-introduces a leak.
                    && self.vars.name(v).starts_with("_mv_")
            })
            .collect();

        let num_attrs = self.data.attributes(struct_def_nr);
        let mut blocks: Vec<Value> = Vec::new();

        // work_checkpoint + clean_work_refs removed — see comment at the
        // end of this loop explaining why skip_free must NOT be set here.
        for a in 0..num_attrs {
            let attr_name = self.data.attr_name(struct_def_nr, a);
            let attr_type = self.data.attr_type(struct_def_nr, a);

            let variant_name = match &attr_type {
                Type::Boolean => "FvBool",
                // Post-2c round 10c: wide Type::Integer (former Type::Long)
                // maps to FvLong; narrow range maps to FvInt.
                Type::Integer(s) if s.is_wide() => "FvLong",
                Type::Integer(_) => "FvInt",
                Type::Float => "FvFloat",
                Type::Single => "FvSingle",
                Type::Character => "FvChar",
                Type::Text(_) => "FvText",
                _ => continue,
            };

            let field_read = self.get_field(struct_def_nr, a, source_expr.clone());
            // @PLN22 Phase 1 — the FieldValue reflection variants (FvBool, FvInt,
            // …) resolve within the FieldValue enum via the variant_of chokepoint,
            // not the bare global def_nr (the enum itself stays globally keyed).
            let fv_enum = self.data.def_nr("FieldValue");
            let variant_def_nr = self.data.variant_of(fv_enum, variant_name);
            let disc_val = self.data.def(variant_def_nr).attributes()[0].value.clone();

            // Construct FieldValue variant as Value::Insert (flat ops list).
            let (fv_ops, fv_work) =
                self.build_object_ops(variant_def_nr, &[(0, disc_val), (1, field_read)]);
            let fv_insert = Value::Insert(fv_ops);

            // Construct StructField: the FieldValue is passed as Value::Var(fv_work)
            // after the Insert has executed.
            let (sf_ops, sf_work) = self.build_object_ops(
                field_def_nr,
                &[(0, Value::Text(attr_name)), (1, Value::Var(fv_work))],
            );
            let sf_insert = Value::Insert(sf_ops);

            blocks.push(fv_insert);
            blocks.push(sf_insert);
            blocks.push(v_set(loop_var, Value::Var(sf_work)));
            // Fresh per-field copies of the owned-text bindings so each field-block
            // frees its own (see `owned_text_locals` above).
            let mut this_body = body.clone();
            for &v in &owned_text_locals {
                let fresh = self.vars.copy_variable(v);
                Self::remap_var_deep(&mut this_body, v, fresh);
            }
            blocks.push(this_body);
        }
        // do NOT call clean_work_refs here.  The unrolled loop
        // creates 2 work-refs per iteration (FvFloat/etc + StructField)
        // and assigns the latter to loop_var via v_set.  Only the LAST
        // iteration's work-refs feed loop_var; earlier ones are orphaned.
        // Marking them all skip_free prevented get_free_vars from
        // emitting OpFreeRef at scope exit, leaking 1 store per
        // orphaned work-ref (8 stores for a 3-field + 4-field struct).
        // The scan_set var-copy companion (Set(v, Var(src)) path) already
        // strips loop_var's deps so it gets its own OpFreeRef; the
        // work-refs themselves pass get_free_vars's is_work_ref check.

        if blocks.is_empty() {
            *code = Value::Null;
        } else {
            *code = v_block(blocks, Type::Void, "field_iter");
        }
    }

    /// Deep-remap every occurrence of var `from` → `to` in `val`, recursing ALL
    /// container variants (unlike `remap_var_nr`, which only walks
    /// `Call`/`Set`/`Insert` for flat default-expression trees).  Used by
    /// `parse_field_iteration` to give each unrolled field-block its own copy of a
    /// body-local text binding.
    /// @PLN104 — renumber a FRAME variable `from` → `to` through a `Value` IR tree,
    /// moving BOTH the `Value::Var`/`Set`/… references AND the frame deps inside every
    /// embedded type (`Block.result`, `FnRef`).  This is the type-dep-aware companion
    /// `remap_var_deep` lacks: a variable swap that renumbers only the IR references
    /// desyncs the cref-buffer / block-result type deps (loft-lang/loft#568).  The
    /// CALLER must also renumber the variable-table typedefs (`Type::renumber_frame_deps`
    /// on each `vars.tp`) and swap the `Variable` structs — see the retbuf swap in
    /// `block_result`.  Do a 3-way `from→TMP→to` to swap two live indices atomically.
    pub(crate) fn renumber_frame_var(val: &mut Value, from: u16, to: u16) {
        match val {
            Value::Var(n) if *n == from => *n = to,
            Value::Set(v, inner) => {
                if *v == from {
                    *v = to;
                }
                Self::renumber_frame_var(inner, from, to);
            }
            // `CallRef`'s first field is the fn-ref VARIABLE (remap it); `Call`'s is a
            // def_nr (a definition, NOT a variable — leave it). Missing the CallRef var
            // desyncs it from the swapped variable table → codegen's `generate_call_ref`
            // reads a non-`Function` slot and panics (loft-lang/loft#568 default-on).
            Value::CallRef(v, xs) => {
                if *v == from {
                    *v = to;
                }
                for x in xs {
                    Self::renumber_frame_var(x, from, to);
                }
            }
            Value::Call(_, xs) | Value::Insert(xs) | Value::Tuple(xs) | Value::Parallel(xs) => {
                for x in xs {
                    Self::renumber_frame_var(x, from, to);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                bl.result.renumber_frame_deps(from, to);
                for op in &mut bl.operators {
                    Self::renumber_frame_var(op, from, to);
                }
            }
            Value::If(c, t, e) => {
                Self::renumber_frame_var(c, from, to);
                Self::renumber_frame_var(t, from, to);
                Self::renumber_frame_var(e, from, to);
            }
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                Self::renumber_frame_var(inner, from, to);
            }
            Value::BreakWith(_, inner) => Self::renumber_frame_var(inner, from, to),
            Value::Span(b) => Self::renumber_frame_var(&mut b.1, from, to),
            Value::Iter(v, a, b, c) => {
                if *v == from {
                    *v = to;
                }
                Self::renumber_frame_var(a, from, to);
                Self::renumber_frame_var(b, from, to);
                Self::renumber_frame_var(c, from, to);
            }
            Value::TupleGet(v, _) => {
                if *v == from {
                    *v = to;
                }
            }
            Value::TuplePut(v, _, inner) => {
                if *v == from {
                    *v = to;
                }
                Self::renumber_frame_var(inner, from, to);
            }
            Value::FnRef(_, v, ty) => {
                if *v == from {
                    *v = to;
                }
                ty.renumber_frame_deps(from, to);
            }
            _ => {}
        }
    }

    fn remap_var_deep(val: &mut Value, from: u16, to: u16) {
        match val {
            Value::Var(n) if *n == from => *n = to,
            Value::Set(v, inner) => {
                if *v == from {
                    *v = to;
                }
                Self::remap_var_deep(inner, from, to);
            }
            Value::Call(_, xs)
            | Value::CallRef(_, xs)
            | Value::Insert(xs)
            | Value::Tuple(xs)
            | Value::Parallel(xs) => {
                for x in xs {
                    Self::remap_var_deep(x, from, to);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &mut bl.operators {
                    Self::remap_var_deep(op, from, to);
                }
            }
            Value::If(c, t, e) => {
                Self::remap_var_deep(c, from, to);
                Self::remap_var_deep(t, from, to);
                Self::remap_var_deep(e, from, to);
            }
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                Self::remap_var_deep(inner, from, to);
            }
            Value::BreakWith(_, inner) => Self::remap_var_deep(inner, from, to),
            Value::Span(b) => Self::remap_var_deep(&mut b.1, from, to),
            Value::Iter(v, a, b, c) => {
                if *v == from {
                    *v = to;
                }
                Self::remap_var_deep(a, from, to);
                Self::remap_var_deep(b, from, to);
                Self::remap_var_deep(c, from, to);
            }
            Value::TupleGet(v, _) => {
                if *v == from {
                    *v = to;
                }
            }
            Value::TuplePut(v, _, inner) => {
                if *v == from {
                    *v = to;
                }
                Self::remap_var_deep(inner, from, to);
            }
            _ => {}
        }
    }

    /// Collect the var numbers that are the TARGET of a `Set` anywhere in `val`
    /// (recursing every child).  Used by `parse_field_iteration` to find the
    /// body-local temporaries an unrolled field-block assigns (and so must own a
    /// private copy of).  Duplicates are harmless — the caller filters + copies
    /// each once.
    fn collect_set_targets(val: &Value, out: &mut Vec<u16>) {
        match val.unspan() {
            Value::Set(v, inner) => {
                if !out.contains(v) {
                    out.push(*v);
                }
                Self::collect_set_targets(inner, out);
            }
            Value::Block(bl) => {
                for op in &bl.operators {
                    Self::collect_set_targets(op, out);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    Self::collect_set_targets(op, out);
                }
            }
            Value::If(c, t, e) => {
                Self::collect_set_targets(c, out);
                Self::collect_set_targets(t, out);
                Self::collect_set_targets(e, out);
            }
            Value::Return(inner) | Value::Drop(inner) => Self::collect_set_targets(inner, out),
            Value::Call(_, args) => {
                for a in args {
                    Self::collect_set_targets(a, out);
                }
            }
            _ => {}
        }
    }

    /// Compute the in-store byte size of a vector element type.
    pub(crate) fn element_store_size(&self, elm: &Type) -> i32 {
        let elm_td = self.data.type_elm(elm);
        // Post-2c: honor size(N) on integer aliases.  Must run before the
        // generic `known_type → database.size(...)` path below, because
        // database.size for the 8-byte integer base returns 8 regardless.
        if matches!(elm, Type::Integer(_))
            && let Some(n) = self.data.forced_size(elm_td)
        {
            return i32::from(n);
        }
        // B5 (2026-04-13): for a mixed struct-enum element type
        // (`Type::Enum(_, true, _)`), the parent enum's `known_type` is
        // a byte-sized enumerate (size 1) — wrong for vector storage,
        // since instances are records.  Use the size of the largest
        // variant's structure type instead.  Without this, recursive
        // struct-enums (`vector<Tree>` inside Tree's own variant) trip
        // `OpDatabase(db_tp=u16::MAX)` panics in `Store::claim`.
        if let Type::Enum(parent_d_nr, true, _) = elm
            && elm_td != u32::MAX
        {
            let mut max_size = 0i32;
            for a_nr in 0..self.data.attributes(*parent_d_nr) {
                let variant_name = self.data.attr_name(*parent_d_nr, a_nr);
                let variant_d_nr = self.data.def_nr(&variant_name);
                if variant_d_nr != u32::MAX {
                    let variant_known = self.data.def(variant_d_nr).known_type();
                    let s = i32::from(self.database.size(variant_known));
                    if s > max_size {
                        max_size = s;
                    }
                }
            }
            if max_size > 0 {
                return max_size;
            }
        }
        if elm_td != u32::MAX {
            let known = self.data.def(elm_td).known_type();
            let db_size = i32::from(self.database.size(known));
            if db_size > 0 {
                return db_size;
            }
        }
        // Fallback for primitive types
        match elm {
            Type::Single | Type::Boolean | Type::Character | Type::Text(_) => 4,
            Type::Integer(_) | Type::Float => 8,
            _ => 12, // DbRef size for reference types
        }
    }

    /// Compiler special-case for `sort(v: vector<T>)`.
    /// Emits `OpSortVector(v, db_tp)` which sorts in-place at runtime, dispatching
    /// on the database element type.
    pub(crate) fn parse_sort(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "sort requires 1 argument: sort(vector)"
            );
            return Type::Void;
        }
        if let Type::Vector(elm, _) = &types[0] {
            if !matches!(
                elm.as_ref(),
                Type::Integer(_) | Type::Float | Type::Single | Type::Text(_)
            ) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "sort is not supported for vector<{}>; use integer, long, float, single, or text",
                    elm.name(&self.data)
                );
                return Type::Void;
            }
            let info = self.type_info(elm);
            *val = self.cl("OpSortVector", &[list[0].clone(), info]);
        } else {
            diagnostic!(self.lexer, Level::Error, "sort requires a vector argument");
        }
        Type::Void
    }

    /// Compiler special-case for `insert(v: vector<T>, idx: integer, elem: T)`.
    /// Emits `OpInsertVector` to create space, then the appropriate `OpSet*` to write the value.
    pub(crate) fn parse_insert(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 3 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "insert requires 3 arguments: insert(vector, index, element)"
            );
            return Type::Void;
        }
        let elm_tp = if let Type::Vector(elm, _) = &types[0] {
            (**elm).clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "insert requires a vector as first argument"
            );
            return Type::Void;
        };
        let elm_size = Value::Int(self.element_store_size(&elm_tp));
        let db_tp = self.type_info(&elm_tp);
        let ed_nr = self.data.type_def_nr(&elm_tp);
        // Create a temp var with dependency on the vector to prevent premature free
        let ref_tp = Type::Reference(ed_nr, crate::data::Deps::frame(types[0].depend()));
        let tmp = self.create_unique("ins", &ref_tp);
        if let Value::Var(vec_var) = &list[0] {
            self.vars.depend(tmp, *vec_var);
        }
        // tmp = OpInsertVector(v, elem_size, idx, db_tp)
        let insert_call = self.cl(
            "OpInsertVector",
            &[list[0].clone(), elm_size, list[1].clone(), db_tp],
        );
        let set_val = self.set_field(ed_nr, usize::MAX, 0, Value::Var(tmp), list[2].clone());
        *val = v_block(vec![v_set(tmp, insert_call), set_val], Type::Void, "insert");
        Type::Void
    }

    /// Compiler special-case for `reverse(v: vector<T>)`.
    /// Dispatches to `OpReverseVector` which works for any element type.
    pub(crate) fn parse_reverse(
        &mut self,
        val: &mut Value,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reverse requires 1 argument: reverse(vector)"
            );
            return Type::Void;
        }
        let elm_size = if let Type::Vector(elm, _) = &types[0] {
            self.element_store_size(elm)
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reverse requires a vector argument"
            );
            return Type::Void;
        };
        *val = self.cl("OpReverseVector", &[list[0].clone(), Value::Int(elm_size)]);
        Type::Void
    }

    /// Validate arguments for `any`/`all`/`count_if`: (vector, fn-pred→boolean).
    fn validate_predicate_args(
        &mut self,
        name: &str,
        list: &[Value],
        types: &[Type],
    ) -> Option<(Type, u32)> {
        if list.len() != 2 {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name} requires 2 arguments: {name}(vector, fn pred)"
                );
            }
            return None;
        }
        let elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name}: first argument must be a vector"
                );
            }
            return None;
        };
        if let Type::Function(params, ret, _) = &types[1] {
            if params.len() != 1 && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name}: predicate must take exactly one argument"
                );
            }
            if **ret != Type::Boolean && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name}: predicate must return boolean"
                );
            }
        } else if !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "{name}: second argument must be a function reference (use fn <name>)"
            );
            return None;
        }
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            *d as u32
        } else {
            return None;
        };
        Some((elem_type, fn_d_nr))
    }

    /// Build the iteration preamble shared by `any`/`all`/`count_if`: copies the
    /// vector, creates an iterator, and returns the loop scaffolding.
    fn predicate_loop_scaffold(
        &mut self,
        name: &str,
        list: &[Value],
        types: &[Type],
    ) -> (Vec<Value>, u16, Value, Value) {
        let mut in_type = types[0].clone();
        let vec_var = self.create_unique(&format!("{name}_vec"), &in_type);
        in_type = in_type.depending(vec_var);

        let iter_var = self.create_unique(&format!("{name}_idx"), &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique(&format!("{name}_elm"), &var_tp);
        self.vars.defined(for_var);

        let mut create_iter = Value::Var(vec_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut test_for = Value::Var(for_var);
        self.convert(&mut test_for, &var_tp, &Type::Boolean);
        let not_test = self.cl("OpNot", &[test_for]);
        let break_if_done = v_if(
            not_test,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );

        let preamble = vec![v_set(vec_var, list[0].clone()), create_iter];
        // N8a.4: return for_next and break_if_done as separate values so callers
        // inline them directly in the loop body.  A v_block wrapper would declare
        // `for_var` inside a nested Rust `{ }` block, making it invisible to the
        // short_circuit/count_step expression that follows in native code.
        (preamble, for_var, for_next, break_if_done)
    }

    /// `any(vec, pred)` — true if pred returns true for any element.
    pub(crate) fn parse_any(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Boolean;
        }
        let Some((_, fn_d_nr)) = self.validate_predicate_args("any", list, types) else {
            return Type::Boolean;
        };

        let acc = self.create_unique("any_acc", &Type::Boolean);
        self.vars.defined(acc);

        let (preamble, for_var, for_next, break_if_done) =
            self.predicate_loop_scaffold("any", list, types);

        // if pred(elem) { acc = true; break }
        let pred_call = Value::Call(fn_d_nr, vec![Value::Var(for_var)]);
        let short_circuit = v_if(
            pred_call,
            v_block(
                vec![v_set(acc, Value::Boolean(true)), Value::Break(0)],
                Type::Void,
                "any_hit",
            ),
            Value::Null,
        );

        let loop_body = vec![for_next, break_if_done, short_circuit];
        let mut stmts = vec![v_set(acc, Value::Boolean(false))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "any"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, Type::Boolean, "any");
        Type::Boolean
    }

    /// `all(vec, pred)` — true if pred returns true for every element.
    pub(crate) fn parse_all(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Boolean;
        }
        let Some((_, fn_d_nr)) = self.validate_predicate_args("all", list, types) else {
            return Type::Boolean;
        };

        let acc = self.create_unique("all_acc", &Type::Boolean);
        self.vars.defined(acc);

        let (preamble, for_var, for_next, break_if_done) =
            self.predicate_loop_scaffold("all", list, types);

        // if !pred(elem) { acc = false; break }
        let pred_call = Value::Call(fn_d_nr, vec![Value::Var(for_var)]);
        let not_pred = self.cl("OpNot", &[pred_call]);
        let short_circuit = v_if(
            not_pred,
            v_block(
                vec![v_set(acc, Value::Boolean(false)), Value::Break(0)],
                Type::Void,
                "all_miss",
            ),
            Value::Null,
        );

        let loop_body = vec![for_next, break_if_done, short_circuit];
        let mut stmts = vec![v_set(acc, Value::Boolean(true))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "all"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, Type::Boolean, "all");
        Type::Boolean
    }

    /// `count_if(vec, pred)` — count of elements where pred returns true.
    pub(crate) fn parse_count_if(
        &mut self,
        val: &mut Value,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        if self.first_pass {
            return I32.clone();
        }
        let Some((_, fn_d_nr)) = self.validate_predicate_args("count_if", list, types) else {
            return I32.clone();
        };

        let acc = self.create_unique("cntif_acc", &I32);
        self.vars.defined(acc);

        let (preamble, for_var, for_next, break_if_done) =
            self.predicate_loop_scaffold("count_if", list, types);

        // if pred(elem) { acc += 1 }
        let pred_call = Value::Call(fn_d_nr, vec![Value::Var(for_var)]);
        let inc = v_set(acc, self.cl("OpAddInt", &[Value::Var(acc), Value::Int(1)]));
        let count_step = v_if(pred_call, inc, Value::Null);

        let loop_body = vec![for_next, break_if_done, count_step];
        let mut stmts = vec![v_set(acc, Value::Int(0))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "count_if"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, I32.clone(), "count_if");
        I32.clone()
    }
}

/// Recursively rename variable `from` to `to` everywhere in `val` — as a READ
/// (`Value::Var` and the other var-carrying reads) AND as a BINDING TARGET (the
/// slot of `Set`/`BreakWith`/`TuplePut`/`Iter` and a block's `scope`).  Unlike
/// `replace_var_in_ir` (which rewrites reads only, leaving binding targets), this
/// is a total substitution: after it, `from` no longer appears anywhere.
///
/// Used by the vector-literal receiver fix (loft#501): when a literal reused the
/// outer assignment LHS as its build accumulator but a `.`/`[` chain follows, the
/// accumulator is renamed to a fresh synthetic local so the LHS is free to receive
/// the chain's result.
pub(crate) fn rename_var(val: &mut Value, from: u16, to: u16) {
    match val {
        Value::Var(v) | Value::TupleGet(v, _) | Value::FnRefDnr(v) | Value::FnRef(_, v, _) => {
            if *v == from {
                *v = to;
            }
        }
        Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Text(_)
        | Value::Enum(_, _)
        | Value::Line(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::Null
        | Value::RawExpr(_) => {}
        Value::CallRef(v, args) => {
            if *v == from {
                *v = to;
            }
            for a in args.iter_mut() {
                rename_var(a, from, to);
            }
        }
        Value::Call(_, args) | Value::Insert(args) | Value::Tuple(args) | Value::Parallel(args) => {
            for a in args.iter_mut() {
                rename_var(a, from, to);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            if bl.scope == from {
                bl.scope = to;
            }
            for op in &mut bl.operators {
                rename_var(op, from, to);
            }
        }
        Value::Set(t, body) | Value::BreakWith(t, body) | Value::TuplePut(t, _, body) => {
            if *t == from {
                *t = to;
            }
            rename_var(body, from, to);
        }
        Value::Return(body) | Value::Drop(body) | Value::Yield(body) => {
            rename_var(body, from, to);
        }
        Value::If(cond, t, f) => {
            rename_var(cond, from, to);
            rename_var(t, from, to);
            rename_var(f, from, to);
        }
        Value::Iter(t, a, b, c) => {
            if *t == from {
                *t = to;
            }
            rename_var(a, from, to);
            rename_var(b, from, to);
            rename_var(c, from, to);
        }
        Value::Span(b) => rename_var(&mut b.1, from, to),
        Value::ParFor(b) => {
            rename_var(&mut b.input, from, to);
            rename_var(&mut b.worker, from, to);
            rename_var(&mut b.threads, from, to);
            rename_var(&mut b.body, from, to);
        }
    }
}

/// Plan-04 B.3 follow-up v2: recursively walk `val` and replace every
/// `Value::Var(target)` with a clone of `replacement`.  Used by
/// `build_parallel_for_ir` to inline-expand the par loop variable `b`
/// to its element-accessor expression, so that `b` is a parse-time
/// alias rather than a runtime slot.  See
/// `doc/claude/plans/finished/04-slot-assignment-redesign/b3-par-inline.md`.
fn replace_var_in_ir(val: &mut Value, target: u16, replacement: &Value) {
    match val {
        Value::Var(v) if *v == target => {
            *val = replacement.clone();
        }
        Value::Var(_)
        | Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Text(_)
        | Value::Enum(_, _)
        | Value::Line(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _)
        | Value::FnRef(_, _, _)
        | Value::FnRefDnr(_)
        | Value::Null => {}
        Value::Call(_, args)
        | Value::CallRef(_, args)
        | Value::Insert(args)
        | Value::Tuple(args)
        | Value::Parallel(args) => {
            for a in args.iter_mut() {
                replace_var_in_ir(a, target, replacement);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &mut bl.operators {
                replace_var_in_ir(op, target, replacement);
            }
        }
        Value::Set(_, body)
        | Value::Return(body)
        | Value::BreakWith(_, body)
        | Value::Drop(body)
        | Value::TuplePut(_, _, body)
        | Value::Yield(body) => {
            replace_var_in_ir(body, target, replacement);
        }
        Value::If(cond, t, f) => {
            replace_var_in_ir(cond, target, replacement);
            replace_var_in_ir(t, target, replacement);
            replace_var_in_ir(f, target, replacement);
        }
        Value::Iter(_, a, b, c) => {
            replace_var_in_ir(a, target, replacement);
            replace_var_in_ir(b, target, replacement);
            replace_var_in_ir(c, target, replacement);
        }
        // Plan-07 phase 1 — Span is transparent; recurse into the
        // wrapped node.
        Value::Span(b) => replace_var_in_ir(&mut b.1, target, replacement),
        // Plan-06 spine step 3 — recurse into all child Values.
        Value::ParFor(b) => {
            replace_var_in_ir(&mut b.input, target, replacement);
            replace_var_in_ir(&mut b.worker, target, replacement);
            replace_var_in_ir(&mut b.threads, target, replacement);
            replace_var_in_ir(&mut b.body, target, replacement);
        }
        // Phase 09 phase 00 step 0.7 — RawExpr is a codegen-internal
        // synthetic value; the parser walker never produces or sees it.
        Value::RawExpr(_) => {}
    }
}
