// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 arc C — store → native IR reader (the read path).
//!
//! The inverse of [`crate::ir_store`]: walks store `Node` / `TypeT` records
//! through the [`crate::data_store`] handle layer and rebuilds the native
//! [`crate::data::Value`] / [`crate::data::Type`] graph.  Together the two
//! halves give a full round-trip — `native → store → native` — so the
//! materialised store can be validated against the original with the IR's own
//! derived `PartialEq`, the strongest oracle available (stronger than the
//! @PLAN28 JSON re-encode comparison, and needing no JSON at all).
//!
//! Scope of this slice: the two recursive enums (`Value`, 34 variants;
//! `Type`, 24 variants) and every sub-struct reachable from them — `Block`,
//! `ParForBody`, `Position`, `Key`, `IntegerSpec`, plus `SortKey`/`NameRef`
//! key lists and `vector<integer>` dep lists.  The `Definition`-level
//! components (`Attribute` / `Function` / `Variable` / `Definition` / `Data`)
//! and the whole-`Data` reader are the next increment.
//!
//! A one-element `vector<Node>` / `vector<TypeT>` field reads back as a single
//! `Box<Value>` / `Box<Type>` (the box-of-one the writer used for single
//! recursive children); an N-element vector reads back as a `Vec`.  `Block.name`
//! is a `&'static str` in the native IR — reconstructed via a bounded
//! [`Box::leak`], exactly as the @PLAN28 JSON decoder does (a loaded image holds
//! a small fixed set of block names that live for the whole process anyway).

use crate::data::{Block, IntegerSpec, ParForBody, Type, Value};
use crate::data_store::{
    self as ds, Record, TypeKind, Value as Node, ValueType, ValuesVector, type_kind,
};
use crate::database::Stores;
use crate::keys::Key;
use crate::lexer::Position;
use std::num::NonZeroU8;

/// Rebuild a native [`Value`] from one store `Node` record.
///
/// # Panics
/// If `slot`'s discriminant byte is not a known `Node` variant — only possible
/// on a record not written by [`crate::ir_store`] (a corrupt or foreign store).
#[must_use]
pub fn read_value(stores: &Stores, slot: Node) -> Value {
    match slot.value_type(stores) {
        // ── leaves ───────────────────────────────────────────────────────────
        ValueType::Null => Value::Null,
        ValueType::Line => Value::Line(slot.field_int(stores, ds::NDLINE_N) as u32),
        ValueType::Int => Value::Int(slot.int_value(stores) as i32),
        ValueType::Enum => Value::Enum(
            slot.field_int(stores, ds::NDENUM_ORD) as u8,
            slot.field_int(stores, ds::NDENUM_TP) as u16,
        ),
        ValueType::Boolean => Value::Boolean(slot.field_bool(stores, ds::NDBOOLEAN_B)),
        ValueType::Float => Value::Float(slot.field_float(stores, ds::NDFLOAT_F)),
        ValueType::Long => Value::Long(slot.field_int(stores, ds::NDLONG_N)),
        ValueType::Single => Value::Single(slot.field_single(stores, ds::NDSINGLE_F)),
        ValueType::Text => Value::Text(slot.field_str(stores, ds::NDTEXT_S).to_string()),
        ValueType::Var => Value::Var(slot.field_int(stores, ds::NDVAR_N) as u16),
        ValueType::Break => Value::Break(slot.field_int(stores, ds::NDBREAK_N) as u16),
        ValueType::Continue => Value::Continue(slot.field_int(stores, ds::NDCONTINUE_N) as u16),
        ValueType::FnRefDnr => Value::FnRefDnr(slot.field_int(stores, ds::NDFNREFDNR_N) as u16),
        ValueType::TupleGet => Value::TupleGet(
            slot.field_int(stores, ds::NDTUPLEGET_VAR) as u16,
            slot.field_int(stores, ds::NDTUPLEGET_IDX) as u16,
        ),
        ValueType::RawExpr => Value::RawExpr(slot.field_str(stores, ds::NDRAWEXPR_S).to_string()),
        // ── scalar(s) + vector<Node> children ────────────────────────────────
        ValueType::Call => Value::Call(
            slot.call_to(stores),
            read_node_list(stores, slot.call_parameters()),
        ),
        ValueType::CallRef => Value::CallRef(
            slot.field_int(stores, ds::NDCALLREF_VAR) as u16,
            read_node_list(stores, slot.field_vec(ds::NDCALLREF_ARGS)),
        ),
        ValueType::Insert => {
            Value::Insert(read_node_list(stores, slot.field_vec(ds::NDINSERT_ITEMS)))
        }
        ValueType::Set => Value::Set(
            slot.field_int(stores, ds::NDSET_VAR) as u16,
            Box::new(read_node_child(stores, slot.field_vec(ds::NDSET_INNER))),
        ),
        ValueType::Return => Value::Return(Box::new(read_node_child(
            stores,
            slot.field_vec(ds::NDRETURN_INNER),
        ))),
        ValueType::BreakWith => Value::BreakWith(
            slot.field_int(stores, ds::NDBREAKWITH_N) as u16,
            Box::new(read_node_child(
                stores,
                slot.field_vec(ds::NDBREAKWITH_INNER),
            )),
        ),
        ValueType::If => Value::If(
            Box::new(read_node_child(stores, slot.field_vec(ds::NDIF_COND))),
            Box::new(read_node_child(stores, slot.field_vec(ds::NDIF_T))),
            Box::new(read_node_child(stores, slot.field_vec(ds::NDIF_F))),
        ),
        ValueType::Drop => Value::Drop(Box::new(read_node_child(
            stores,
            slot.field_vec(ds::NDDROP_INNER),
        ))),
        ValueType::Iter => Value::Iter(
            slot.field_int(stores, ds::NDITER_VAR) as u16,
            Box::new(read_node_child(stores, slot.field_vec(ds::NDITER_CREATE))),
            Box::new(read_node_child(stores, slot.field_vec(ds::NDITER_NEXT))),
            Box::new(read_node_child(stores, slot.field_vec(ds::NDITER_INIT))),
        ),
        ValueType::Tuple => Value::Tuple(read_node_list(stores, slot.field_vec(ds::NDTUPLE_ITEMS))),
        ValueType::TuplePut => Value::TuplePut(
            slot.field_int(stores, ds::NDTUPLEPUT_VAR) as u16,
            slot.field_int(stores, ds::NDTUPLEPUT_IDX) as u16,
            Box::new(read_node_child(
                stores,
                slot.field_vec(ds::NDTUPLEPUT_INNER),
            )),
        ),
        ValueType::Yield => Value::Yield(Box::new(read_node_child(
            stores,
            slot.field_vec(ds::NDYIELD_INNER),
        ))),
        ValueType::Parallel => {
            Value::Parallel(read_node_list(stores, slot.field_vec(ds::NDPARALLEL_ARMS)))
        }
        // ── inlined Block ────────────────────────────────────────────────────
        ValueType::Block => Value::Block(Box::new(read_block(stores, slot))),
        ValueType::Loop => Value::Loop(Box::new(read_block(stores, slot))),
        // ── inlined sub-structs ───────────────────────────────────────────────
        ValueType::Span => {
            let position = Position {
                file: slot.field_str(stores, ds::SPAN_POS_FILE).to_string(),
                line: slot.field_int(stores, ds::SPAN_POS_LINE) as u32,
                pos: slot.field_int(stores, ds::SPAN_POS_POS) as u32,
            };
            let inner = read_node_child(stores, slot.field_vec(ds::NDSPAN_INNER));
            Value::Span(Box::new((position, inner)))
        }
        ValueType::ParFor => Value::ParFor(Box::new(read_par_for(stores, slot))),
        // ── vector of a non-Node struct ───────────────────────────────────────
        ValueType::Keys => Value::Keys(read_keys(stores, slot)),
        // ── carries a vector<TypeT> ────────────────────────────────────────────
        ValueType::FnRef => Value::FnRef(
            slot.field_int(stores, ds::NDFNREF_DEF_NR) as i32,
            slot.field_int(stores, ds::NDFNREF_VAR) as u16,
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::NDFNREF_T, ds::TYPET_STRIDE),
            )),
        ),
        ValueType::Other(d) => panic!("ir_read: unknown Node discriminant {d}"),
    }
}

/// Rebuild a native [`Type`] from one store `TypeT` record.
///
/// # Panics
/// If `slot`'s discriminant byte is not a known `TypeT` variant — only possible
/// on a record not written by [`crate::ir_store`] (a corrupt or foreign store).
#[must_use]
pub fn read_type(stores: &Stores, slot: Record) -> Type {
    match type_kind(slot.discriminant(stores)) {
        TypeKind::Unknown => Type::Unknown(slot.field_int(stores, ds::TYUNKNOWN_N) as u32),
        TypeKind::Null => Type::Null,
        TypeKind::Void => Type::Void,
        TypeKind::Never => Type::Never,
        TypeKind::Integer => Type::Integer(read_int_spec(stores, slot)),
        TypeKind::Boolean => Type::Boolean,
        TypeKind::Float => Type::Float,
        TypeKind::Single => Type::Single,
        TypeKind::Character => Type::Character,
        TypeKind::Text => Type::Text(read_dep_list(stores, slot, ds::TYTEXT_DEP)),
        TypeKind::Keys => Type::Keys,
        TypeKind::Enum => Type::Enum(
            slot.field_int(stores, ds::TYENUM_N) as u32,
            slot.field_bool(stores, ds::TYENUM_IS_REF),
            read_dep_list(stores, slot, ds::TYENUM_DEP),
        ),
        TypeKind::Reference => Type::Reference(
            slot.field_int(stores, ds::TYREF_N) as u32,
            read_dep_list(stores, slot, ds::TYREF_DEP),
        ),
        TypeKind::RefVar => Type::RefVar(Box::new(read_type_child(
            stores,
            slot.field_recvec(ds::TYREFVAR_INNER, ds::TYPET_STRIDE),
        ))),
        TypeKind::Vector => Type::Vector(
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::TYVECTOR_INNER, ds::TYPET_STRIDE),
            )),
            read_dep_list(stores, slot, ds::TYVECTOR_DEP),
        ),
        TypeKind::Routine => Type::Routine(slot.field_int(stores, ds::TYROUTINE_N) as u32),
        TypeKind::Iterator => Type::Iterator(
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::TYITER_STEP, ds::TYPET_STRIDE),
            )),
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::TYITER_INNER, ds::TYPET_STRIDE),
            )),
        ),
        TypeKind::Sorted => Type::Sorted(
            slot.field_int(stores, ds::TYSORTED_N) as u32,
            read_sort_keys(
                stores,
                slot.field_recvec(ds::TYSORTED_KEYS, ds::SORTKEY_STRIDE),
            ),
            read_dep_list(stores, slot, ds::TYSORTED_DEP),
        ),
        TypeKind::Index => Type::Index(
            slot.field_int(stores, ds::TYINDEX_N) as u32,
            read_sort_keys(
                stores,
                slot.field_recvec(ds::TYINDEX_KEYS, ds::SORTKEY_STRIDE),
            ),
            read_dep_list(stores, slot, ds::TYINDEX_DEP),
        ),
        TypeKind::Spacial => Type::Spacial(
            slot.field_int(stores, ds::TYSPACIAL_N) as u32,
            read_name_list(
                stores,
                slot.field_recvec(ds::TYSPACIAL_NAMES, ds::NAMEREF_STRIDE),
            ),
            read_dep_list(stores, slot, ds::TYSPACIAL_DEP),
        ),
        TypeKind::Hash => Type::Hash(
            slot.field_int(stores, ds::TYHASH_N) as u32,
            read_name_list(
                stores,
                slot.field_recvec(ds::TYHASH_NAMES, ds::NAMEREF_STRIDE),
            ),
            read_dep_list(stores, slot, ds::TYHASH_DEP),
        ),
        TypeKind::Function => Type::Function(
            read_type_list(stores, slot.field_recvec(ds::TYFUNC_ARGS, ds::TYPET_STRIDE)),
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::TYFUNC_RESULT, ds::TYPET_STRIDE),
            )),
            read_dep_list(stores, slot, ds::TYFUNC_DEP),
        ),
        TypeKind::Rewritten => Type::Rewritten(Box::new(read_type_child(
            stores,
            slot.field_recvec(ds::TYREWRITTEN_INNER, ds::TYPET_STRIDE),
        ))),
        TypeKind::Tuple => Type::Tuple(read_type_list(
            stores,
            slot.field_recvec(ds::TYTUPLE_ELEMS, ds::TYPET_STRIDE),
        )),
        TypeKind::Other(d) => panic!("ir_read: unknown TypeT discriminant {d}"),
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Read the single element of a box-of-one `vector<Node>` as a `Value`.
fn read_node_child(stores: &Stores, v: ValuesVector) -> Value {
    read_value(stores, v.get(0, stores))
}

/// Read every element of a `vector<Node>` as a `Vec<Value>`.
fn read_node_list(stores: &Stores, v: ValuesVector) -> Vec<Value> {
    (0..v.len(stores))
        .map(|i| read_value(stores, v.get(i, stores)))
        .collect()
}

/// Read the single element of a box-of-one `vector<TypeT>` as a `Type`.
fn read_type_child(stores: &Stores, v: ds::RecVector) -> Type {
    read_type(stores, v.get(0, stores))
}

/// Read every element of a `vector<TypeT>` as a `Vec<Type>`.
fn read_type_list(stores: &Stores, v: ds::RecVector) -> Vec<Type> {
    (0..v.len(stores))
        .map(|i| read_type(stores, v.get(i, stores)))
        .collect()
}

/// Read a `vector<integer>` dep list (field `off` of `slot`) as a `Vec<u16>`.
fn read_dep_list(stores: &Stores, slot: Record, off: u32) -> Vec<u16> {
    let v = slot.field_recvec(off, ds::INT_STRIDE);
    (0..v.len(stores))
        .map(|i| v.get(i, stores).field_int(stores, 0) as u16)
        .collect()
}

/// Read a `vector<SortKey>` as a `Vec<(String, bool)>`.
fn read_sort_keys(stores: &Stores, v: ds::RecVector) -> Vec<(String, bool)> {
    (0..v.len(stores))
        .map(|i| {
            let e = v.get(i, stores);
            (
                e.field_str(stores, ds::SORTKEY_NAME).to_string(),
                e.field_bool(stores, ds::SORTKEY_ASC),
            )
        })
        .collect()
}

/// Read a `vector<NameRef>` as a `Vec<String>`.
fn read_name_list(stores: &Stores, v: ds::RecVector) -> Vec<String> {
    (0..v.len(stores))
        .map(|i| {
            v.get(i, stores)
                .field_str(stores, ds::NAMEREF_NAME)
                .to_string()
        })
        .collect()
}

/// Read an inlined `IntegerSpec` from a `TyInteger` record.  `forced_size`
/// sentinel `0` decodes to `None` (mirrors the writer).
fn read_int_spec(stores: &Stores, slot: Record) -> IntegerSpec {
    IntegerSpec {
        min: slot.field_int(stores, ds::TYINTEGER_MIN) as i32,
        max: slot.field_int(stores, ds::TYINTEGER_MAX) as u32,
        not_null: slot.field_bool(stores, ds::TYINTEGER_NOT_NULL),
        forced_size: NonZeroU8::new(slot.field_int(stores, ds::TYINTEGER_FORCED) as u8),
    }
}

/// Read an inlined `Block` from an `NdBlock` / `NdLoop` record.  `Block.name`
/// is `&'static str` — reconstructed via a bounded leak (see module note).
fn read_block(stores: &Stores, slot: Node) -> Block {
    let name: &'static str = Box::leak(slot.block_name(stores).to_owned().into_boxed_str());
    Block {
        name,
        operators: read_node_list(stores, slot.block_operators()),
        result: read_type_child(
            stores,
            slot.field_recvec(ds::NDBLOCK_BLOCK + ds::BLOCK_RESULT, ds::TYPET_STRIDE),
        ),
        scope: slot.field_int(stores, ds::NDBLOCK_BLOCK + ds::BLOCK_SCOPE) as u16,
        var_size: slot.field_int(stores, ds::NDBLOCK_BLOCK + ds::BLOCK_VAR_SIZE) as u16,
    }
}

/// Read an inlined `ParForBody` from an `NdParFor` record.
fn read_par_for(stores: &Stores, slot: Node) -> ParForBody {
    ParForBody {
        input: read_node_child(stores, slot.field_vec(ds::PARFOR_INPUT)),
        x_var: slot.field_int(stores, ds::PARFOR_X_VAR) as u16,
        r_var: slot.field_int(stores, ds::PARFOR_R_VAR) as u16,
        worker: read_node_child(stores, slot.field_vec(ds::PARFOR_WORKER)),
        threads: read_node_child(stores, slot.field_vec(ds::PARFOR_THREADS)),
        body: read_node_child(stores, slot.field_vec(ds::PARFOR_BODY)),
        stitch_id: slot.field_int(stores, ds::PARFOR_STITCH_ID) as u8,
    }
}

/// Read an `NdKeys` record's `vector<Key>` as a `Vec<Key>`.
fn read_keys(stores: &Stores, slot: Node) -> Vec<Key> {
    let kv = slot.field_recvec(ds::NDKEYS_KEYS, ds::KEY_STRIDE);
    (0..kv.len(stores))
        .map(|i| {
            let e = kv.get(i, stores);
            Key {
                type_nr: e.field_int(stores, ds::KEY_TYPE_NR) as i8,
                position: e.field_int(stores, ds::KEY_POSITION) as u16,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_store::{RecVector, ValuesVector};
    use crate::ir_schema_gen::register_ir_schema;
    use crate::ir_store::{materialize_node, materialize_type};

    /// `native Value → store → native Value` must round-trip identically.
    fn round_trip_value(v: &Value) {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);
        let root = ValuesVector::new(stores.database(16));
        materialize_node(&mut stores, root, v);
        let back = read_value(&stores, root.get(0, &stores));
        assert_eq!(*v, back, "Value round-trip mismatch");
    }

    /// `native Type → store → native Type` must round-trip identically.
    fn round_trip_type(t: &Type) {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);
        let root = RecVector::new(stores.database(16), ds::TYPET_STRIDE);
        materialize_type(&mut stores, root, t);
        let back = read_type(&stores, root.get(0, &stores));
        assert_eq!(*t, back, "Type round-trip mismatch");
    }

    #[test]
    fn value_leaves_round_trip() {
        for v in [
            Value::Null,
            Value::Line(42),
            Value::Int(-7),
            Value::Enum(5, 6),
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Float(2.5),
            Value::Long(-99),
            Value::Single(1.5),
            Value::Text("hi".into()),
            Value::Var(7),
            Value::Break(2),
            Value::Continue(1),
            Value::FnRefDnr(4),
            Value::TupleGet(1, 2),
            Value::RawExpr("x".into()),
        ] {
            round_trip_value(&v);
        }
    }

    #[test]
    fn value_recursive_round_trips() {
        // Call / CallRef / Insert / Tuple / Parallel (scalar + Vec<Value>).
        round_trip_value(&Value::Call(144, vec![Value::Int(7), Value::Int(9)]));
        round_trip_value(&Value::CallRef(3, vec![Value::Int(1), Value::Null]));
        round_trip_value(&Value::Insert(vec![Value::Int(1), Value::Int(2)]));
        round_trip_value(&Value::Tuple(vec![Value::Boolean(true), Value::Long(8)]));
        round_trip_value(&Value::Parallel(vec![Value::Int(1)]));
        // Box-of-one children.
        round_trip_value(&Value::Set(9, Box::new(Value::Long(42))));
        round_trip_value(&Value::Return(Box::new(Value::Int(3))));
        round_trip_value(&Value::BreakWith(1, Box::new(Value::Int(4))));
        round_trip_value(&Value::Drop(Box::new(Value::Int(5))));
        round_trip_value(&Value::Yield(Box::new(Value::Int(6))));
        round_trip_value(&Value::TuplePut(2, 1, Box::new(Value::Int(7))));
        round_trip_value(&Value::If(
            Box::new(Value::Int(1)),
            Box::new(Value::Return(Box::new(Value::Int(2)))),
            Box::new(Value::Drop(Box::new(Value::Int(3)))),
        ));
        round_trip_value(&Value::Iter(
            5,
            Box::new(Value::Int(10)),
            Box::new(Value::Int(11)),
            Box::new(Value::Int(12)),
        ));
    }

    #[test]
    fn value_block_and_loop_round_trip() {
        round_trip_value(&Value::Block(Box::new(Block {
            name: "loop_body",
            operators: vec![Value::Call(5, vec![Value::Int(1)]), Value::Null],
            result: Type::Boolean,
            scope: 3,
            var_size: 16,
        })));
        round_trip_value(&Value::Loop(Box::new(Block {
            name: "spin",
            operators: vec![Value::Break(0)],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        })));
    }

    #[test]
    fn value_inline_substructs_round_trip() {
        round_trip_value(&Value::Span(Box::new((
            Position {
                file: "f.loft".into(),
                line: 12,
                pos: 3,
            },
            Value::Int(7),
        ))));
        round_trip_value(&Value::ParFor(Box::new(ParForBody {
            input: Value::Int(1),
            x_var: 2,
            r_var: 3,
            worker: Value::Int(4),
            threads: Value::Int(5),
            body: Value::Int(6),
            stitch_id: 1,
        })));
        round_trip_value(&Value::Keys(vec![
            Key {
                type_nr: 3,
                position: 7,
            },
            Key {
                type_nr: -1,
                position: 42,
            },
        ]));
        round_trip_value(&Value::FnRef(42, 3, Box::new(Type::Boolean)));
    }

    #[test]
    fn type_all_variants_round_trip() {
        for t in [
            Type::Unknown(7),
            Type::Null,
            Type::Void,
            Type::Never,
            Type::Boolean,
            Type::Float,
            Type::Single,
            Type::Character,
            Type::Keys,
            Type::Integer(IntegerSpec::wide()),
            Type::Integer(IntegerSpec::u8()),
            Type::Integer(IntegerSpec::signed32()),
            Type::Text(vec![]),
            Type::Text(vec![1, 2, 3]),
            Type::Enum(4, true, vec![5]),
            Type::Enum(4, false, vec![]),
            Type::Reference(9, vec![0, 1]),
            Type::RefVar(Box::new(Type::Boolean)),
            Type::Vector(Box::new(Type::Text(vec![])), vec![2]),
            Type::Routine(11),
            Type::Iterator(
                Box::new(Type::Integer(IntegerSpec::wide())),
                Box::new(Type::Null),
            ),
            Type::Sorted(
                3,
                vec![("name".into(), true), ("age".into(), false)],
                vec![1],
            ),
            Type::Index(3, vec![("key".into(), true)], vec![]),
            Type::Spacial(3, vec!["x".into(), "y".into()], vec![1]),
            Type::Hash(3, vec!["id".into()], vec![]),
            Type::Function(
                vec![Type::Integer(IntegerSpec::wide()), Type::Text(vec![])],
                Box::new(Type::Boolean),
                vec![0],
            ),
            Type::Rewritten(Box::new(Type::Text(vec![]))),
            Type::Tuple(vec![Type::Integer(IntegerSpec::wide()), Type::Text(vec![])]),
        ] {
            round_trip_type(&t);
        }
    }

    #[test]
    fn type_nested_recursion_round_trips() {
        // vector<vector<reference>> and a function returning a vector.
        round_trip_type(&Type::Vector(
            Box::new(Type::Vector(Box::new(Type::Reference(2, vec![])), vec![])),
            vec![],
        ));
        round_trip_type(&Type::Function(
            vec![Type::Integer(IntegerSpec::i32()), Type::Text(vec![1, 2])],
            Box::new(Type::Vector(Box::new(Type::Boolean), vec![3])),
            vec![7],
        ));
    }

    /// `forced_size` is ignored by `IntegerSpec`'s `PartialEq`, so the `==`
    /// round-trip above can't catch a wrong width — assert it explicitly.
    #[test]
    fn integer_forced_size_round_trips_exactly() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);
        let root = RecVector::new(stores.database(16), ds::TYPET_STRIDE);
        materialize_type(&mut stores, root, &Type::Integer(IntegerSpec::u8()));
        let Type::Integer(spec) = read_type(&stores, root.get(0, &stores)) else {
            panic!("expected Integer");
        };
        assert_eq!(spec.forced_size, NonZeroU8::new(1));
    }
}
