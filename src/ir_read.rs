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

use crate::data::{
    Attribute, Block, Data, DefType, Definition, ImpureCategory, IntegerSpec, LinkedFieldGroup,
    LinkedFieldKind, ParForBody, Purity, Type, Value,
};
use crate::data_store::{
    self as ds, Record, TypeKind, Value as Node, ValueType, ValuesVector, type_kind,
};
use crate::database::Stores;
use crate::keys::{DbRef, Key};
use crate::lexer::Position;
use crate::variables::{Function, RestoredVar};
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

// ─── Definition-level reader (Attribute / Function / Definition / Data) ───────

/// Rebuild a whole native [`Data`] from the store record `root` (the `DbRef`
/// returned by [`crate::ir_store::materialize_data`]).  Derived indices are
/// re-derived via [`Data::rebuild_indices`], mirroring the @PLAN28 JSON loader.
#[must_use]
pub fn read_data(stores: &Stores, root: DbRef) -> Data {
    let r = Record::new(root);
    let mut data = Data::new();
    data.source = r.field_int(stores, ds::DATA_SOURCE) as u16;
    let defs = r.field_recvec(ds::DATA_DEFINITIONS, ds::DEFINITION_STRIDE);
    for i in 0..defs.len(stores) {
        data.definitions
            .push(read_definition(stores, defs.get(i, stores)));
    }
    data.rebuild_indices();
    data
}

/// Rebuild one native [`Definition`] from a store `Definition` record.
///
/// `attr_names` is re-derived from the restored attribute list (a derived
/// index, never stored); `code_position` / `code_length` / `const_ref` are
/// codegen-derived and reset (recomputed by the compile pass on load), exactly
/// as the @PLAN28 JSON decoder does.
///
/// # Panics
/// If a stored `def_type` / `purity` code is out of range — only possible on a
/// record not written by [`crate::ir_store`] (a corrupt or foreign store).
#[must_use]
pub fn read_definition(stores: &Stores, r: Record) -> Definition {
    let name = r.field_str(stores, ds::DEF_NAME).to_string();
    let position = Position {
        file: r
            .field_str(stores, ds::DEF_POSITION + ds::POS_FILE)
            .to_string(),
        line: r.field_int(stores, ds::DEF_POSITION + ds::POS_LINE) as u32,
        pos: r.field_int(stores, ds::DEF_POSITION + ds::POS_POS) as u32,
    };
    let attributes = read_attributes(stores, r);
    let attr_names = attributes
        .iter()
        .enumerate()
        .map(|(i, a)| (a.name.clone(), i))
        .collect();
    let forced = r.field_int(stores, ds::DEF_FORCED_SIZE);
    let synthetic_s = r.field_str(stores, ds::DEF_SYNTHETIC);
    Definition {
        variables: read_function(stores, r, ds::DEF_VARIABLES),
        attributes,
        attr_names,
        // codegen-derived: recomputed by the compile pass on load.
        code_position: 0,
        code_length: 0,
        const_ref: None,
        source: r.field_int(stores, ds::DEF_SOURCE) as u16,
        def_type: def_type_from_code(r.field_int(stores, ds::DEF_DEF_TYPE)),
        parent: r.field_int(stores, ds::DEF_PARENT) as u32,
        code: read_node_child(stores, r.field_vec(ds::DEF_CODE)),
        returned: read_type_child(stores, r.field_recvec(ds::DEF_RETURNED, ds::TYPET_STRIDE)),
        returned_not_null: r.field_bool(stores, ds::DEF_RETURNED_NOT_NULL),
        rust: r.field_str(stores, ds::DEF_RUST).to_string(),
        native: r.field_str(stores, ds::DEF_NATIVE).to_string(),
        op_code: r.field_int(stores, ds::DEF_OP_CODE) as u16,
        known_type: r.field_int(stores, ds::DEF_KNOWN_TYPE) as u16,
        pub_visible: r.field_bool(stores, ds::DEF_PUB_VISIBLE),
        closure_record: r.field_int(stores, ds::DEF_CLOSURE_RECORD) as u32,
        mutated_captures: read_name_list(
            stores,
            r.field_recvec(ds::DEF_MUTATED_CAPTURES, ds::NAMEREF_STRIDE),
        ),
        scalars_to_box: read_name_list(
            stores,
            r.field_recvec(ds::DEF_SCALARS_TO_BOX, ds::NAMEREF_STRIDE),
        ),
        bounds: read_u32_list(stores, r.field_recvec(ds::DEF_BOUNDS, ds::INT_STRIDE)),
        forced_size: if forced == 0 {
            None
        } else {
            Some(forced as u8)
        },
        purity: purity_from_code(r.field_int(stores, ds::DEF_PURITY)),
        field_groups: read_field_groups(stores, r),
        synthetic: if synthetic_s.is_empty() {
            None
        } else {
            Some(Box::leak(synthetic_s.to_owned().into_boxed_str()))
        },
        name,
        position,
    }
}

/// Read a `Definition`'s `vector<Attribute>`.
fn read_attributes(stores: &Stores, parent: Record) -> Vec<Attribute> {
    let v = parent.field_recvec(ds::DEF_ATTRIBUTES, ds::ATTRIBUTE_STRIDE);
    (0..v.len(stores))
        .map(|i| read_attribute(stores, v.get(i, stores)))
        .collect()
}

/// Read one `Attribute` record.
fn read_attribute(stores: &Stores, r: Record) -> Attribute {
    Attribute {
        name: r.field_str(stores, ds::ATTR_NAME).to_string(),
        typedef: read_type_child(stores, r.field_recvec(ds::ATTR_TYPEDEF, ds::TYPET_STRIDE)),
        mutable: r.field_bool(stores, ds::ATTR_MUTABLE),
        constant: r.field_bool(stores, ds::ATTR_CONSTANT),
        init: r.field_bool(stores, ds::ATTR_INIT),
        nullable: r.field_bool(stores, ds::ATTR_NULLABLE),
        primary: r.field_bool(stores, ds::ATTR_PRIMARY),
        hidden: r.field_bool(stores, ds::ATTR_HIDDEN),
        value: read_node_child(stores, r.field_vec(ds::ATTR_VALUE)),
        check: read_node_child(stores, r.field_vec(ds::ATTR_CHECK)),
        check_message: read_node_child(stores, r.field_vec(ds::ATTR_CHECK_MESSAGE)),
        alias_d_nr: r.field_int(stores, ds::ATTR_ALIAS_D_NR) as u32,
        assigned_lambda_d_nr: r.field_int(stores, ds::ATTR_ASSIGNED_LAMBDA_D_NR) as u32,
    }
}

/// Read a `Definition`'s `vector<LinkedFieldGroup>`.
fn read_field_groups(stores: &Stores, parent: Record) -> Vec<LinkedFieldGroup> {
    let v = parent.field_recvec(ds::DEF_FIELD_GROUPS, ds::LFG_STRIDE);
    (0..v.len(stores))
        .map(|i| {
            let r = v.get(i, stores);
            LinkedFieldGroup {
                kind: if r.field_int(stores, ds::LFG_KIND) == 0 {
                    LinkedFieldKind::Tuple
                } else {
                    LinkedFieldKind::Index
                },
                instance: r.field_int(stores, ds::LFG_INSTANCE) as u16,
                field_indices: read_dep_list(stores, r, ds::LFG_FIELD_INDICES),
                alignment: r.field_int(stores, ds::LFG_ALIGNMENT) as u8,
                size: r.field_int(stores, ds::LFG_SIZE) as u16,
            }
        })
        .collect()
}

/// Read the inlined `Function` (at `base`) of a `Definition` record: `name` /
/// `file`, the nine codegen-read fields of each `Variable`, the `names` map
/// (name → var_nr), and the `inline_ref_vars` set — all from the store, so the
/// `Function` is reconstructed losslessly (the store grew to hold `names` /
/// `inline_refs` because neither is reconstructible from the variable list and
/// codegen reads both on the load path; see plan README § arc C).
fn read_function(stores: &Stores, parent: Record, base: u32) -> Function {
    let name = parent.field_str(stores, base + ds::FN_NAME).to_string();
    let file = parent.field_str(stores, base + ds::FN_FILE).to_string();
    let vars_vec = parent.field_recvec(base + ds::FN_VARIABLES, ds::VARIABLE_STRIDE);
    let mut vars = Vec::with_capacity(vars_vec.len(stores) as usize);
    for i in 0..vars_vec.len(stores) {
        let vr = vars_vec.get(i, stores);
        vars.push(RestoredVar {
            name: vr.field_str(stores, ds::VAR_NAME).to_string(),
            type_def: read_type_child(stores, vr.field_recvec(ds::VAR_TYPE_DEF, ds::TYPET_STRIDE)),
            stack_pos: vr.field_int(stores, ds::VAR_STACK_POS) as u16,
            uses: vr.field_int(stores, ds::VAR_USES) as u16,
            argument: vr.field_bool(stores, ds::VAR_ARGUMENT),
            stack_allocated: vr.field_bool(stores, ds::VAR_STACK_ALLOCATED),
            skip_free: vr.field_bool(stores, ds::VAR_SKIP_FREE),
            captured: vr.field_bool(stores, ds::VAR_CAPTURED),
            caller_hidden_buf: vr.field_bool(stores, ds::VAR_CALLER_HIDDEN_BUF),
        });
    }
    let names_vec = parent.field_recvec(base + ds::FN_NAMES, ds::NAMENR_STRIDE);
    let names = (0..names_vec.len(stores))
        .map(|i| {
            let e = names_vec.get(i, stores);
            (
                e.field_str(stores, ds::NAMENR_NAME).to_string(),
                e.field_int(stores, ds::NAMENR_NR) as u16,
            )
        })
        .collect();
    let inline_refs = read_dep_list(stores, parent, base + ds::FN_INLINE_REFS);
    Function::from_snapshot(&name, &file, vars, names, inline_refs)
}

/// Read a `vector<integer>` (field `off` of `slot`) as a `Vec<u32>`.
fn read_u32_list(stores: &Stores, v: ds::RecVector) -> Vec<u32> {
    (0..v.len(stores))
        .map(|i| v.get(i, stores).field_int(stores, 0) as u32)
        .collect()
}

/// `DefType` from its stored integer code (inverse of `ir_store::def_type_code`).
fn def_type_from_code(c: i64) -> DefType {
    match c {
        0 => DefType::Unknown,
        1 => DefType::Function,
        2 => DefType::Dynamic,
        3 => DefType::Enum,
        4 => DefType::EnumValue,
        5 => DefType::Struct,
        6 => DefType::Vector,
        7 => DefType::Type,
        8 => DefType::Constant,
        9 => DefType::Generic,
        10 => DefType::Interface,
        other => panic!("ir_read: unknown DefType code {other}"),
    }
}

/// `Purity` from its stored integer code (inverse of `ir_store::purity_code`):
/// `0` Unknown, `1` Pure, `2 + cat` Impure with `ImpureCategory` `cat`.
fn purity_from_code(c: i64) -> Purity {
    match c {
        0 => Purity::Unknown,
        1 => Purity::Pure,
        n => Purity::Impure(match n - 2 {
            0 => ImpureCategory::HostIo,
            1 => ImpureCategory::Prng,
            2 => ImpureCategory::Io,
            3 => ImpureCategory::ParentWrite,
            4 => ImpureCategory::ParCall,
            other => panic!("ir_read: unknown Purity/ImpureCategory code {}", other + 2),
        }),
    }
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

    // ── Definition / Data reader (whole-stdlib round-trip) ──────────────────

    use crate::data::Data;
    use crate::ir_schema::{compare_data, definition_to_json};
    use crate::ir_store::materialize_data;

    /// Parse the real `default/` stdlib once, materialize, read back.
    fn stdlib_round_trip() -> (Data, Data) {
        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false)
            .expect("parse default/ stdlib");
        let fresh = p.data;
        let mut ir = Stores::new();
        let _ids = register_ir_schema(&mut ir);
        let root = materialize_data(&mut ir, &fresh);
        let loaded = read_data(&ir, root);
        (fresh, loaded)
    }

    /// Capstone: the whole real `default/` stdlib round-trips
    /// `native → store → native` **bit-for-bit**.  The full `compare_data`
    /// oracle — every `Definition` field including the per-function variable
    /// table (`vars` + `names` + `inline_refs`) — is green across all
    /// definitions.  This is arc C's read-path milestone: a store-materialised
    /// `Data` is indistinguishable from a fresh parse.
    #[test]
    fn read_whole_stdlib_compare_data_green() {
        let (fresh, loaded) = stdlib_round_trip();
        assert!(
            fresh.definitions() > 100,
            "stdlib should have many definitions, got {}",
            fresh.definitions()
        );
        if let Err(diff) = compare_data(&fresh, &loaded) {
            panic!("store round-trip diverged from fresh parse: {diff:?}");
        }
    }

    /// Spot-check that a function-body definition's variable table — the part
    /// that needed the schema growth — survives intact: `vars` count, the
    /// `names` map, and `inline_ref_vars` all re-encode identically.
    #[test]
    fn read_stdlib_function_variables_round_trip() {
        let (fresh, loaded) = stdlib_round_trip();
        let mut checked = 0u32;
        for d_nr in 0..fresh.definitions() {
            let rf = fresh.def(d_nr);
            if rf.variables.snapshot_len() == 0 {
                continue;
            }
            // definition_to_json embeds the full variable block (vars + names +
            // inline_refs); equality here proves the populated table round-trips.
            assert_eq!(
                definition_to_json(rf),
                definition_to_json(loaded.def(d_nr)),
                "function def {d_nr} ({}) variable table differs",
                rf.name
            );
            checked += 1;
        }
        assert!(checked > 20, "expected many function defs, got {checked}");
    }
}
