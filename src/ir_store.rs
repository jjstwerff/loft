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
//! Coverage so far mirrors the handle layer's read surface — `Null` / `Int` /
//! `Call` / `Block`.  Every other native variant is `unimplemented!` until its
//! read+write accessors land in `data_store.rs`; each is a small, mechanical
//! follow-up increment (add the offset const + guard assertion + the two
//! accessors, then a match arm here).

use crate::data::Value;
use crate::data_store::{Value as Node, ValuesVector};
use crate::database::Stores;

/// Materialize one native `Value` into a freshly-appended slot of `dst`
/// (a `vector<Node>`), recursing into any child vectors.
pub fn materialize_node(stores: &mut Stores, dst: ValuesVector, v: &Value) {
    let slot = dst.push(stores);
    write_into(stores, &slot, v);
}

/// Write `v` into the already-allocated `slot` (its bytes are zeroed).
fn write_into(stores: &mut Stores, slot: &Node, v: &Value) {
    match v {
        Value::Null => slot.write_null(stores),
        Value::Int(n) => slot.write_int(stores, i64::from(*n)),
        Value::Call(def_nr, args) => {
            slot.write_call(stores, *def_nr);
            let params = slot.call_parameters();
            for a in args {
                materialize_node(stores, params, a);
            }
        }
        Value::Block(b) => {
            slot.write_block(stores, b.name);
            let ops = slot.block_operators();
            for op in &b.operators {
                materialize_node(stores, ops, op);
            }
        }
        other => {
            unimplemented!("@PLAN54 arc B: materialize_node does not yet cover {other:?}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Block, Type};
    use crate::data_store::ValueType;
    use crate::ir_schema_gen::register_ir_schema;

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
}
