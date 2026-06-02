// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 arc B — native IR → store materializer (the write path).
//!
//! Walks a native [`crate::data::Value`] tree and writes it into store `Node`
//! records through the [`crate::data_store`] handle layer.  This is the
//! @PLAN28 `ir_schema::data_to_json` walk with its JSON sink swapped for a
//! store-writer sink — exactly the convergence the plan describes
//! (`plans/future/54-data-as-store` § arc B).
//!
//! Coverage: every `Node` variant whose fields are scalars + `vector<Node>`
//! children + the inlined `Block` (30 of 34 variants).  The four that reach
//! into another struct or the `TypeT` half — `Span` (`Position`), `Keys`
//! (`vector<Key>`), `FnRef` (`vector<TypeT>`), `ParFor` (`ParForBody`) — are
//! the next increment (they need the `TypeT`/sub-struct accessors).  A
//! `Box<Value>` single child maps to a one-element `vector<Node>` field
//! (finding 2's box-of-one); a `Vec<Value>` maps to the same field with N
//! elements — both materialize identically.

use crate::data::{Block, Value};
use crate::data_store::{self as ds, Value as Node, ValuesVector};
use crate::database::Stores;

/// Materialize one native `Value` into a freshly-appended slot of `dst`
/// (a `vector<Node>`), recursing into any child vectors.
pub fn materialize_node(stores: &mut Stores, dst: ValuesVector, v: &Value) {
    let slot = dst.push(stores);
    write_into(stores, &slot, v);
}

/// Materialize every element of `items` into the `vector<Node>` `dst`.
fn push_all(stores: &mut Stores, dst: ValuesVector, items: &[Value]) {
    for it in items {
        materialize_node(stores, dst, it);
    }
}

/// Write a native `Block` into `slot` under discriminant `disc` (`NdBlock` or
/// `NdLoop` — identical inlined-`Block` layout).  `Block.result`
/// (`vector<TypeT>`) is deferred to the `TypeT` increment.
fn write_block(stores: &mut Stores, slot: &Node, disc: u8, b: &Block) {
    if disc == ds::DISC_LOOP {
        slot.write_loop(stores, b.name);
    } else {
        slot.write_block(stores, b.name);
    }
    let ops = slot.block_operators();
    push_all(stores, ops, &b.operators);
}

/// Write `v` into the already-allocated `slot` (its bytes are zeroed).
fn write_into(stores: &mut Stores, slot: &Node, v: &Value) {
    match v {
        // ── leaves ──────────────────────────────────────────────────────────
        Value::Null => slot.write_null(stores),
        Value::Line(n) => {
            slot.set_discriminant(stores, ds::DISC_LINE);
            slot.set_field_int(stores, ds::NDLINE_N, i64::from(*n));
        }
        Value::Int(n) => slot.write_int(stores, i64::from(*n)),
        Value::Enum(ord, tp) => {
            slot.set_discriminant(stores, ds::DISC_ENUM);
            slot.set_field_int(stores, ds::NDENUM_ORD, i64::from(*ord));
            slot.set_field_int(stores, ds::NDENUM_TP, i64::from(*tp));
        }
        Value::Boolean(b) => {
            slot.set_discriminant(stores, ds::DISC_BOOLEAN);
            slot.set_field_bool(stores, ds::NDBOOLEAN_B, *b);
        }
        Value::Float(f) => {
            slot.set_discriminant(stores, ds::DISC_FLOAT);
            slot.set_field_float(stores, ds::NDFLOAT_F, *f);
        }
        Value::Long(n) => {
            slot.set_discriminant(stores, ds::DISC_LONG);
            slot.set_field_int(stores, ds::NDLONG_N, *n);
        }
        Value::Single(f) => {
            slot.set_discriminant(stores, ds::DISC_SINGLE);
            slot.set_field_single(stores, ds::NDSINGLE_F, *f);
        }
        Value::Text(s) => {
            slot.set_discriminant(stores, ds::DISC_TEXT);
            slot.set_field_str(stores, ds::NDTEXT_S, s);
        }
        Value::Var(n) => {
            slot.set_discriminant(stores, ds::DISC_VAR);
            slot.set_field_int(stores, ds::NDVAR_N, i64::from(*n));
        }
        Value::Break(n) => {
            slot.set_discriminant(stores, ds::DISC_BREAK);
            slot.set_field_int(stores, ds::NDBREAK_N, i64::from(*n));
        }
        Value::Continue(n) => {
            slot.set_discriminant(stores, ds::DISC_CONTINUE);
            slot.set_field_int(stores, ds::NDCONTINUE_N, i64::from(*n));
        }
        Value::FnRefDnr(n) => {
            slot.set_discriminant(stores, ds::DISC_FN_REF_DNR);
            slot.set_field_int(stores, ds::NDFNREFDNR_N, i64::from(*n));
        }
        Value::TupleGet(var, idx) => {
            slot.set_discriminant(stores, ds::DISC_TUPLE_GET);
            slot.set_field_int(stores, ds::NDTUPLEGET_VAR, i64::from(*var));
            slot.set_field_int(stores, ds::NDTUPLEGET_IDX, i64::from(*idx));
        }
        Value::RawExpr(s) => {
            slot.set_discriminant(stores, ds::DISC_RAW_EXPR);
            slot.set_field_str(stores, ds::NDRAWEXPR_S, s);
        }
        // ── scalar(s) + vector<Node> children ────────────────────────────────
        Value::Call(def_nr, args) => {
            slot.write_call(stores, *def_nr);
            push_all(stores, slot.call_parameters(), args);
        }
        Value::CallRef(var, args) => {
            slot.set_discriminant(stores, ds::DISC_CALL_REF);
            slot.set_field_int(stores, ds::NDCALLREF_VAR, i64::from(*var));
            push_all(stores, slot.field_vec(ds::NDCALLREF_ARGS), args);
        }
        Value::Insert(items) => {
            slot.set_discriminant(stores, ds::DISC_INSERT);
            push_all(stores, slot.field_vec(ds::NDINSERT_ITEMS), items);
        }
        Value::Set(var, inner) => {
            slot.set_discriminant(stores, ds::DISC_SET);
            slot.set_field_int(stores, ds::NDSET_VAR, i64::from(*var));
            materialize_node(stores, slot.field_vec(ds::NDSET_INNER), inner);
        }
        Value::Return(inner) => {
            slot.set_discriminant(stores, ds::DISC_RETURN);
            materialize_node(stores, slot.field_vec(ds::NDRETURN_INNER), inner);
        }
        Value::BreakWith(n, inner) => {
            slot.set_discriminant(stores, ds::DISC_BREAK_WITH);
            slot.set_field_int(stores, ds::NDBREAKWITH_N, i64::from(*n));
            materialize_node(stores, slot.field_vec(ds::NDBREAKWITH_INNER), inner);
        }
        Value::If(cond, t, f) => {
            slot.set_discriminant(stores, ds::DISC_IF);
            materialize_node(stores, slot.field_vec(ds::NDIF_COND), cond);
            materialize_node(stores, slot.field_vec(ds::NDIF_T), t);
            materialize_node(stores, slot.field_vec(ds::NDIF_F), f);
        }
        Value::Drop(inner) => {
            slot.set_discriminant(stores, ds::DISC_DROP);
            materialize_node(stores, slot.field_vec(ds::NDDROP_INNER), inner);
        }
        Value::Iter(var, create, next, init) => {
            slot.set_discriminant(stores, ds::DISC_ITER);
            slot.set_field_int(stores, ds::NDITER_VAR, i64::from(*var));
            materialize_node(stores, slot.field_vec(ds::NDITER_CREATE), create);
            materialize_node(stores, slot.field_vec(ds::NDITER_NEXT), next);
            materialize_node(stores, slot.field_vec(ds::NDITER_INIT), init);
        }
        Value::Tuple(items) => {
            slot.set_discriminant(stores, ds::DISC_TUPLE);
            push_all(stores, slot.field_vec(ds::NDTUPLE_ITEMS), items);
        }
        Value::TuplePut(var, idx, inner) => {
            slot.set_discriminant(stores, ds::DISC_TUPLE_PUT);
            slot.set_field_int(stores, ds::NDTUPLEPUT_VAR, i64::from(*var));
            slot.set_field_int(stores, ds::NDTUPLEPUT_IDX, i64::from(*idx));
            materialize_node(stores, slot.field_vec(ds::NDTUPLEPUT_INNER), inner);
        }
        Value::Yield(inner) => {
            slot.set_discriminant(stores, ds::DISC_YIELD);
            materialize_node(stores, slot.field_vec(ds::NDYIELD_INNER), inner);
        }
        Value::Parallel(arms) => {
            slot.set_discriminant(stores, ds::DISC_PARALLEL);
            push_all(stores, slot.field_vec(ds::NDPARALLEL_ARMS), arms);
        }
        // ── inlined Block ────────────────────────────────────────────────────
        Value::Block(b) => write_block(stores, slot, ds::DISC_BLOCK, b),
        Value::Loop(b) => write_block(stores, slot, ds::DISC_LOOP, b),
        // ── inlined sub-structs (no TypeT) ────────────────────────────────────
        Value::Span(boxed) => {
            let (position, inner) = &**boxed;
            slot.set_discriminant(stores, ds::DISC_SPAN);
            slot.set_field_int(stores, ds::SPAN_POS_LINE, i64::from(position.line));
            slot.set_field_int(stores, ds::SPAN_POS_POS, i64::from(position.pos));
            slot.set_field_str(stores, ds::SPAN_POS_FILE, &position.file);
            materialize_node(stores, slot.field_vec(ds::NDSPAN_INNER), inner);
        }
        Value::ParFor(b) => {
            slot.set_discriminant(stores, ds::DISC_PAR_FOR);
            slot.set_field_int(stores, ds::PARFOR_X_VAR, i64::from(b.x_var));
            slot.set_field_int(stores, ds::PARFOR_R_VAR, i64::from(b.r_var));
            slot.set_field_int(stores, ds::PARFOR_STITCH_ID, i64::from(b.stitch_id));
            materialize_node(stores, slot.field_vec(ds::PARFOR_INPUT), &b.input);
            materialize_node(stores, slot.field_vec(ds::PARFOR_WORKER), &b.worker);
            materialize_node(stores, slot.field_vec(ds::PARFOR_THREADS), &b.threads);
            materialize_node(stores, slot.field_vec(ds::PARFOR_BODY), &b.body);
        }
        // ── deferred: reach into Key (vector<Key>) / the TypeT half ────────────
        Value::Keys(_) | Value::FnRef(..) => {
            unimplemented!(
                "@PLAN54 arc B: materialize_node does not yet cover {v:?} \
                 (Keys needs vector<Key>; FnRef needs the TypeT half — next increment)"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Block, ParForBody, Type};
    use crate::data_store::{self as ds, ValueType};
    use crate::ir_schema_gen::register_ir_schema;
    use crate::lexer::Position;

    /// A standalone host record whose offset-0 field is the root
    /// `vector<Node>` the materializer writes into.
    fn root_vector(stores: &mut Stores) -> ValuesVector {
        ValuesVector::new(stores.database(16))
    }

    #[test]
    fn materialize_call_tree_round_trips() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::Call(144, vec![Value::Int(7), Value::Int(9)]);

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let got = root.get(0, &stores);
        assert_eq!(got.value_type(&stores), ValueType::Call);
        assert_eq!(got.call_to(&stores), 144);
        let params = got.call_parameters();
        assert_eq!(params.len(&stores), 2);
        assert_eq!(params.get(0, &stores).int_value(&stores), 7);
        assert_eq!(params.get(1, &stores).int_value(&stores), 9);
    }

    #[test]
    fn materialize_block_round_trips() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::Block(Box::new(Block {
            name: "loop_body",
            operators: vec![Value::Call(5, vec![Value::Int(1)]), Value::Null],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let got = root.get(0, &stores);
        assert_eq!(got.value_type(&stores), ValueType::Block);
        assert_eq!(got.block_name(&stores), "loop_body");

        let ops = got.block_operators();
        assert_eq!(ops.len(&stores), 2);

        let call = ops.get(0, &stores);
        assert_eq!(call.value_type(&stores), ValueType::Call);
        assert_eq!(call.call_to(&stores), 5);
        assert_eq!(call.call_parameters().get(0, &stores).int_value(&stores), 1);

        assert_eq!(ops.get(1, &stores).value_type(&stores), ValueType::Null);
    }

    #[test]
    fn materialize_leaf_scalars_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // One of every leaf-scalar variant, wrapped in a tuple so they share a
        // single root vector.
        let native = Value::Tuple(vec![
            Value::Line(3),
            Value::Long(99),
            Value::Float(2.5),
            Value::Single(1.5),
            Value::Boolean(true),
            Value::Text("hi".into()),
            Value::Var(7),
            Value::Break(2),
            Value::Continue(1),
            Value::FnRefDnr(4),
            Value::Enum(5, 6),
            Value::TupleGet(1, 2),
            Value::RawExpr("x".into()),
        ]);

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let tuple = root.get(0, &stores);
        assert_eq!(tuple.value_type(&stores), ValueType::Tuple);
        let items = tuple.field_vec(ds::NDTUPLE_ITEMS);
        assert_eq!(items.len(&stores), 13);

        let e = |i: u32| items.get(i, &stores);
        assert_eq!(e(0).value_type(&stores), ValueType::Line);
        assert_eq!(e(0).field_int(&stores, ds::NDLINE_N), 3);
        assert_eq!(e(1).value_type(&stores), ValueType::Long);
        assert_eq!(e(1).field_int(&stores, ds::NDLONG_N), 99);
        assert_eq!(e(2).value_type(&stores), ValueType::Float);
        assert_eq!(
            e(2).field_float(&stores, ds::NDFLOAT_F).to_bits(),
            2.5_f64.to_bits()
        );
        assert_eq!(e(3).value_type(&stores), ValueType::Single);
        assert_eq!(
            e(3).field_single(&stores, ds::NDSINGLE_F).to_bits(),
            1.5_f32.to_bits()
        );
        assert_eq!(e(4).value_type(&stores), ValueType::Boolean);
        assert!(e(4).field_bool(&stores, ds::NDBOOLEAN_B));
        assert_eq!(e(5).value_type(&stores), ValueType::Text);
        assert_eq!(e(5).field_str(&stores, ds::NDTEXT_S), "hi");
        assert_eq!(e(6).value_type(&stores), ValueType::Var);
        assert_eq!(e(6).field_int(&stores, ds::NDVAR_N), 7);
        assert_eq!(e(7).value_type(&stores), ValueType::Break);
        assert_eq!(e(7).field_int(&stores, ds::NDBREAK_N), 2);
        assert_eq!(e(8).value_type(&stores), ValueType::Continue);
        assert_eq!(e(9).value_type(&stores), ValueType::FnRefDnr);
        assert_eq!(e(9).field_int(&stores, ds::NDFNREFDNR_N), 4);
        assert_eq!(e(10).value_type(&stores), ValueType::Enum);
        assert_eq!(e(10).field_int(&stores, ds::NDENUM_ORD), 5);
        assert_eq!(e(10).field_int(&stores, ds::NDENUM_TP), 6);
        assert_eq!(e(11).value_type(&stores), ValueType::TupleGet);
        assert_eq!(e(11).field_int(&stores, ds::NDTUPLEGET_VAR), 1);
        assert_eq!(e(11).field_int(&stores, ds::NDTUPLEGET_IDX), 2);
        assert_eq!(e(12).value_type(&stores), ValueType::RawExpr);
        assert_eq!(e(12).field_str(&stores, ds::NDRAWEXPR_S), "x");
    }

    #[test]
    fn materialize_control_flow_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // if 1 { return 2 } else { drop 3 }
        let native = Value::If(
            Box::new(Value::Int(1)),
            Box::new(Value::Return(Box::new(Value::Int(2)))),
            Box::new(Value::Drop(Box::new(Value::Int(3)))),
        );

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let if_node = root.get(0, &stores);
        assert_eq!(if_node.value_type(&stores), ValueType::If);

        let cond = if_node.field_vec(ds::NDIF_COND).get(0, &stores);
        assert_eq!(cond.int_value(&stores), 1);

        let then = if_node.field_vec(ds::NDIF_T).get(0, &stores);
        assert_eq!(then.value_type(&stores), ValueType::Return);
        let ret_inner = then.field_vec(ds::NDRETURN_INNER).get(0, &stores);
        assert_eq!(ret_inner.int_value(&stores), 2);

        let els = if_node.field_vec(ds::NDIF_F).get(0, &stores);
        assert_eq!(els.value_type(&stores), ValueType::Drop);
        let drop_inner = els.field_vec(ds::NDDROP_INNER).get(0, &stores);
        assert_eq!(drop_inner.int_value(&stores), 3);
    }

    #[test]
    fn materialize_box_of_one_and_multi_vector_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // A mix: Set (scalar + box-of-one), CallRef (scalar + Vec),
        // Iter (scalar + three box-of-one children).
        let native = Value::Tuple(vec![
            Value::Set(9, Box::new(Value::Long(42))),
            Value::CallRef(3, vec![Value::Int(1), Value::Int(2)]),
            Value::Iter(
                5,
                Box::new(Value::Int(10)),
                Box::new(Value::Int(11)),
                Box::new(Value::Int(12)),
            ),
        ]);

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);
        let items = root.get(0, &stores).field_vec(ds::NDTUPLE_ITEMS);

        let set = items.get(0, &stores);
        assert_eq!(set.value_type(&stores), ValueType::Set);
        assert_eq!(set.field_int(&stores, ds::NDSET_VAR), 9);
        assert_eq!(
            set.field_vec(ds::NDSET_INNER)
                .get(0, &stores)
                .field_int(&stores, ds::NDLONG_N),
            42
        );

        let call_ref = items.get(1, &stores);
        assert_eq!(call_ref.value_type(&stores), ValueType::CallRef);
        assert_eq!(call_ref.field_int(&stores, ds::NDCALLREF_VAR), 3);
        let args = call_ref.field_vec(ds::NDCALLREF_ARGS);
        assert_eq!(args.len(&stores), 2);
        assert_eq!(args.get(1, &stores).int_value(&stores), 2);

        let iter = items.get(2, &stores);
        assert_eq!(iter.value_type(&stores), ValueType::Iter);
        assert_eq!(iter.field_int(&stores, ds::NDITER_VAR), 5);
        assert_eq!(
            iter.field_vec(ds::NDITER_CREATE)
                .get(0, &stores)
                .int_value(&stores),
            10
        );
        assert_eq!(
            iter.field_vec(ds::NDITER_NEXT)
                .get(0, &stores)
                .int_value(&stores),
            11
        );
        assert_eq!(
            iter.field_vec(ds::NDITER_INIT)
                .get(0, &stores)
                .int_value(&stores),
            12
        );
    }

    #[test]
    fn materialize_loop_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::Loop(Box::new(Block {
            name: "spin",
            operators: vec![Value::Break(0)],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let lp = root.get(0, &stores);
        assert_eq!(lp.value_type(&stores), ValueType::Loop);
        assert_eq!(lp.block_name(&stores), "spin");
        let ops = lp.block_operators();
        assert_eq!(ops.len(&stores), 1);
        assert_eq!(ops.get(0, &stores).value_type(&stores), ValueType::Break);
    }

    #[test]
    fn materialize_span_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::Span(Box::new((
            Position {
                file: "f.loft".into(),
                line: 12,
                pos: 3,
            },
            Value::Int(7),
        )));

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let span = root.get(0, &stores);
        assert_eq!(span.value_type(&stores), ValueType::Span);
        assert_eq!(span.field_int(&stores, ds::SPAN_POS_LINE), 12);
        assert_eq!(span.field_int(&stores, ds::SPAN_POS_POS), 3);
        assert_eq!(span.field_str(&stores, ds::SPAN_POS_FILE), "f.loft");
        assert_eq!(
            span.field_vec(ds::NDSPAN_INNER)
                .get(0, &stores)
                .int_value(&stores),
            7
        );
    }

    #[test]
    fn materialize_par_for_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::ParFor(Box::new(ParForBody {
            input: Value::Int(1),
            x_var: 2,
            r_var: 3,
            worker: Value::Int(4),
            threads: Value::Int(5),
            body: Value::Int(6),
            stitch_id: 1,
        }));

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let pf = root.get(0, &stores);
        assert_eq!(pf.value_type(&stores), ValueType::ParFor);
        assert_eq!(pf.field_int(&stores, ds::PARFOR_X_VAR), 2);
        assert_eq!(pf.field_int(&stores, ds::PARFOR_R_VAR), 3);
        assert_eq!(pf.field_int(&stores, ds::PARFOR_STITCH_ID), 1);
        let child = |off: u32| pf.field_vec(off).get(0, &stores).int_value(&stores);
        assert_eq!(child(ds::PARFOR_INPUT), 1);
        assert_eq!(child(ds::PARFOR_WORKER), 4);
        assert_eq!(child(ds::PARFOR_THREADS), 5);
        assert_eq!(child(ds::PARFOR_BODY), 6);
    }
}
