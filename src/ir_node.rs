// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 G2 / M3.0 — the IR node-walk handle (`IrNode` / `IrType`).
//!
//! ## Why this exists
//!
//! `state/codegen.rs` lowers the IR by pattern-matching the native enums
//! directly — `match val { Value::Call(op, args) => … }` — at ~450 sites.  That
//! couples codegen to the *concrete* `data::Value` / `data::Type` layout, which
//! is exactly what the G2 read-site migration must break: to read the IR from
//! the store instead of the native graph, the match sites have to dispatch on a
//! discriminant and fetch fields through accessors, not destructure a Rust enum.
//!
//! ## The shape
//!
//! The **store** side of this handle already exists: [`crate::data_store::Value`]
//! (a `Node` record handle) answers [`ValueType`] via `value_type()` and exposes
//! typed field accessors (`call_to`, `field_int`, `field_vec`, …); [`read_value`]
//! is the proof it walks the whole IR.  What was missing is a *native* mirror
//! under one backing-agnostic type so codegen can be written **once** and run on
//! **either** backing.  `IrNode<'a>` is that type:
//!
//! ```ignore
//! pub enum IrNode<'a> {
//!     Native(&'a Value),            // borrows the native graph
//!     Store(&'a Stores, Node),      // reads a store record through baked offsets
//! }
//! ```
//!
//! Every accessor `match`es the backing internally; the caller never knows which
//! is live.  `kind()` returns the shared [`ValueType`] discriminant, so a
//! converted codegen site reads:
//!
//! ```ignore
//! match node.kind() {
//!     ValueType::Int  => { let v = node.int_value(); … }
//!     ValueType::Call => { let op = node.call_to(); let args = node.call_args(); … }
//!     …
//! }
//! ```
//!
//! Child nodes come back as `IrNode<'a>` (same backing); child *lists* as
//! [`IrNodeList`] (`len()` + `get(i) -> IrNode`).  The single lifetime `'a` ties
//! to whichever borrow the variant holds (`&'a Value` or `&'a Stores`), so a
//! whole walk shares one lifetime with no per-step allocation.
//!
//! ## Migration contract (how this drives M3.1 → M5)
//!
//! * **M3.0 (here):** define the handle, the `kind()` dispatch (native + store),
//!   and the accessors.  Accessors are filled **just-in-time** — each M3.1 group
//!   adds exactly the ones its sites need, so no accessor is ever dead.  The
//!   bedrock guarantee is the *cross-backing equivalence test*: for the same
//!   node, `IrNode::Native` and `IrNode::Store` must agree on `kind()` and every
//!   accessor (see the tests below).  That is what lets a swap be trusted.
//! * **M3.1…n (native-backed):** convert codegen's `match val { Value::X(..) }`
//!   sites to `match node.kind() { ValueType::X => node.x_*() }`, one
//!   function-group per commit, constructing `IrNode::Native(val)` at the entry.
//!   Behaviour is identical — the backing is still native.  Where the existing
//!   recursion still wants `&Value` (it calls `self.generate(&Value)`), a
//!   converted arm bridges via [`IrNode::native`] until M5.
//! * **M5 (the swap):** once a subsystem is fully on the handle, flip the entry
//!   construction from `IrNode::Native` to `IrNode::Store` and drop the
//!   `native()` bridge; the equivalence harness ([`crate::ir_read::ir_roundtrip_check`])
//!   plus the suite verify the flip.  Codegen itself does **not** change.
//!
//! ## Why an enum, not a trait
//!
//! A `trait NodeView` impl'd by both backings would force every codegen function
//! (deeply recursive `&mut State` methods) to become generic over the node type
//! — monomorphisation blow-up, turbofish noise, and lifetime friction on the
//! `&mut self` recursion.  A concrete two-variant enum keeps codegen
//! non-generic; the backing branch is a single predictable match inside each
//! accessor (and collapses to one arm once M5 makes the backing uniform).

use crate::data::{Type, Value};
use crate::data_store::{
    self as ds, Record, TypeKind, Value as Node, ValueType, ValuesVector, type_kind,
};
use crate::database::Stores;

/// A backing-agnostic handle to one IR node — either a borrow of the native
/// [`Value`] graph or a store [`Node`] record read through baked offsets.  See
/// the module docs for the migration contract.
#[derive(Clone, Copy)]
pub enum IrNode<'a> {
    /// Borrows the native IR graph.
    Native(&'a Value),
    /// Reads a store `Node` record (carries `&Stores` for the accessors).
    Store(&'a Stores, Node),
}

/// A backing-agnostic handle to a `vector<Node>` child list.
#[derive(Clone, Copy)]
pub enum IrNodeList<'a> {
    /// A native `&[Value]` slice.
    Native(&'a [Value]),
    /// A store `vector<Node>` header.
    Store(&'a Stores, ValuesVector),
}

impl<'a> IrNode<'a> {
    /// The node's variant discriminant — the value codegen `match`es on.
    #[must_use]
    pub fn kind(&self) -> ValueType {
        match *self {
            IrNode::Native(v) => native_value_kind(v),
            IrNode::Store(s, n) => n.value_type(s),
        }
    }

    /// Escape hatch for the M3.1→M4 transition: the underlying native node, or
    /// `None` when store-backed.  A converted codegen arm uses this to feed the
    /// not-yet-converted recursion (`self.generate(&Value)`).  Removed at M5,
    /// when the recursion itself takes `IrNode`.
    #[must_use]
    pub fn native(&self) -> Option<&'a Value> {
        match *self {
            IrNode::Native(v) => Some(v),
            IrNode::Store(..) => None,
        }
    }

    // ── scalar accessors (representative spread — categories proven; the rest
    //    land just-in-time with their M3.1 consumer) ──────────────────────────

    /// `Int` / `Long` payload, widened to `i64` (matches the store's width).
    #[must_use]
    pub fn int_value(&self) -> i64 {
        match *self {
            IrNode::Native(Value::Int(v)) => i64::from(*v),
            IrNode::Native(Value::Long(v)) => *v,
            IrNode::Store(s, n) => n.int_value(s),
            IrNode::Native(_) => kind_panic("int_value", self),
        }
    }

    /// `Var` slot number.
    #[must_use]
    pub fn var_nr(&self) -> u16 {
        match *self {
            IrNode::Native(Value::Var(v)) => *v,
            IrNode::Store(s, n) => n.field_int(s, ds::NDVAR_N) as u16,
            IrNode::Native(_) => kind_panic("var_nr", self),
        }
    }

    /// `Enum` ordinal and database-type pair.
    #[must_use]
    pub fn enum_pair(&self) -> (u8, u16) {
        match *self {
            IrNode::Native(Value::Enum(ord, tp)) => (*ord, *tp),
            IrNode::Store(s, n) => (
                n.field_int(s, ds::NDENUM_ORD) as u8,
                n.field_int(s, ds::NDENUM_TP) as u16,
            ),
            IrNode::Native(_) => kind_panic("enum_pair", self),
        }
    }

    /// `Single` (f32) payload.
    #[must_use]
    pub fn single_value(&self) -> f32 {
        match *self {
            IrNode::Native(Value::Single(v)) => *v,
            IrNode::Store(s, n) => n.field_single(s, ds::NDSINGLE_F),
            IrNode::Native(_) => kind_panic("single_value", self),
        }
    }

    /// `Float` (f64) payload.
    #[must_use]
    pub fn float_value(&self) -> f64 {
        match *self {
            IrNode::Native(Value::Float(v)) => *v,
            IrNode::Store(s, n) => n.field_float(s, ds::NDFLOAT_F),
            IrNode::Native(_) => kind_panic("float_value", self),
        }
    }

    /// `Boolean` payload.
    #[must_use]
    pub fn bool_value(&self) -> bool {
        match *self {
            IrNode::Native(Value::Boolean(v)) => *v,
            IrNode::Store(s, n) => n.field_bool(s, ds::NDBOOLEAN_B),
            IrNode::Native(_) => kind_panic("bool_value", self),
        }
    }

    /// `Break` loop number.
    #[must_use]
    pub fn break_nr(&self) -> u16 {
        match *self {
            IrNode::Native(Value::Break(v)) => *v,
            IrNode::Store(s, n) => n.field_int(s, ds::NDBREAK_N) as u16,
            IrNode::Native(_) => kind_panic("break_nr", self),
        }
    }

    /// `Continue` loop number.
    #[must_use]
    pub fn continue_nr(&self) -> u16 {
        match *self {
            IrNode::Native(Value::Continue(v)) => *v,
            IrNode::Store(s, n) => n.field_int(s, ds::NDCONTINUE_N) as u16,
            IrNode::Native(_) => kind_panic("continue_nr", self),
        }
    }

    /// `Line` source-line number.
    #[must_use]
    pub fn line_nr(&self) -> u32 {
        match *self {
            IrNode::Native(Value::Line(v)) => *v,
            IrNode::Store(s, n) => n.field_int(s, ds::NDLINE_N) as u32,
            IrNode::Native(_) => kind_panic("line_nr", self),
        }
    }

    /// `Text` / `RawExpr` string payload (no allocation — borrows for `'a`).
    #[must_use]
    pub fn text(&self) -> &'a str {
        match *self {
            IrNode::Native(Value::Text(t) | Value::RawExpr(t)) => t.as_str(),
            IrNode::Store(s, n) => n.field_str(s, ds::NDTEXT_S),
            IrNode::Native(_) => kind_panic("text", self),
        }
    }

    // ── Call: scalar + child-list ───────────────────────────────────────────

    /// `Call` target function/operator number.
    #[must_use]
    pub fn call_to(&self) -> u32 {
        match *self {
            IrNode::Native(Value::Call(op, _)) => *op,
            IrNode::Store(s, n) => n.call_to(s),
            IrNode::Native(_) => kind_panic("call_to", self),
        }
    }

    /// `Call` argument list.
    #[must_use]
    pub fn call_args(&self) -> IrNodeList<'a> {
        match *self {
            IrNode::Native(Value::Call(_, args)) => IrNodeList::Native(args),
            IrNode::Store(s, n) => IrNodeList::Store(s, n.call_parameters()),
            IrNode::Native(_) => kind_panic("call_args", self),
        }
    }

    // ── single boxed children ───────────────────────────────────────────────

    /// `Return` inner expression.
    #[must_use]
    pub fn return_inner(&self) -> IrNode<'a> {
        match *self {
            IrNode::Native(Value::Return(b)) => IrNode::Native(b),
            IrNode::Store(s, n) => store_child(s, n, ds::NDRETURN_INNER),
            IrNode::Native(_) => kind_panic("return_inner", self),
        }
    }

    /// `Drop` inner expression.
    #[must_use]
    pub fn drop_inner(&self) -> IrNode<'a> {
        match *self {
            IrNode::Native(Value::Drop(b)) => IrNode::Native(b),
            IrNode::Store(s, n) => store_child(s, n, ds::NDDROP_INNER),
            IrNode::Native(_) => kind_panic("drop_inner", self),
        }
    }

    // ── If: three boxed children ────────────────────────────────────────────

    /// `If` condition expression.
    #[must_use]
    pub fn if_cond(&self) -> IrNode<'a> {
        match *self {
            IrNode::Native(Value::If(c, _, _)) => IrNode::Native(c),
            IrNode::Store(s, n) => store_child(s, n, ds::NDIF_COND),
            IrNode::Native(_) => kind_panic("if_cond", self),
        }
    }

    /// `If` then-branch.
    #[must_use]
    pub fn if_then(&self) -> IrNode<'a> {
        match *self {
            IrNode::Native(Value::If(_, t, _)) => IrNode::Native(t),
            IrNode::Store(s, n) => store_child(s, n, ds::NDIF_T),
            IrNode::Native(_) => kind_panic("if_then", self),
        }
    }

    /// `If` else-branch (`Value::Null` when the source had no `else`).
    #[must_use]
    pub fn if_else(&self) -> IrNode<'a> {
        match *self {
            IrNode::Native(Value::If(_, _, f)) => IrNode::Native(f),
            IrNode::Store(s, n) => store_child(s, n, ds::NDIF_F),
            IrNode::Native(_) => kind_panic("if_else", self),
        }
    }

    // ── Span passthrough ────────────────────────────────────────────────────

    /// See through any number of `Span` wrappers to the inner non-`Span` node —
    /// the handle counterpart of [`Value::unspan`].
    #[must_use]
    pub fn unspan(&self) -> IrNode<'a> {
        let mut cur = *self;
        while matches!(cur.kind(), ValueType::Span) {
            cur = cur.span_inner();
        }
        cur
    }

    /// The `Value` wrapped by a `Span` node (one level).
    fn span_inner(&self) -> IrNode<'a> {
        match *self {
            IrNode::Native(Value::Span(b)) => IrNode::Native(&b.1),
            IrNode::Store(s, n) => store_child(s, n, ds::NDSPAN_INNER),
            IrNode::Native(_) => kind_panic("span_inner", self),
        }
    }
}

impl<'a> IrNodeList<'a> {
    /// Number of child nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        match *self {
            IrNodeList::Native(s) => s.len(),
            IrNodeList::Store(st, v) => v.len(st) as usize,
        }
    }

    /// Whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `i`-th child as a same-backed [`IrNode`].
    #[must_use]
    pub fn get(&self, i: usize) -> IrNode<'a> {
        match *self {
            IrNodeList::Native(s) => IrNode::Native(&s[i]),
            IrNodeList::Store(st, v) => IrNode::Store(st, v.get(i as u32, st)),
        }
    }

    /// Iterate the children left-to-right.
    pub fn iter(&self) -> impl Iterator<Item = IrNode<'a>> + '_ {
        (0..self.len()).map(move |i| self.get(i))
    }
}

/// A backing-agnostic handle to one IR static [`Type`].  Codegen matches `Type`
/// even more than `Value` (~280 sites); M4 converts those.  M3.0 lands the
/// `kind()` dispatch — the foundation those conversions build on.
#[derive(Clone, Copy)]
pub enum IrType<'a> {
    /// Borrows the native `Type`.
    Native(&'a Type),
    /// Reads a store `TypeT` record.
    Store(&'a Stores, Record),
}

impl<'a> IrType<'a> {
    /// The type's variant discriminant.
    #[must_use]
    pub fn kind(&self) -> TypeKind {
        match *self {
            IrType::Native(t) => native_type_kind(t),
            IrType::Store(s, r) => type_kind(r.discriminant(s)),
        }
    }

    /// Escape hatch for the transition: the underlying native type, or `None`
    /// when store-backed.
    #[must_use]
    pub fn native(&self) -> Option<&'a Type> {
        match *self {
            IrType::Native(t) => Some(t),
            IrType::Store(..) => None,
        }
    }
}

/// Build a single-child [`IrNode`] from a store node's boxed-child field (a
/// length-1 `vector<Node>` slot), mirroring [`read_node_child`].
#[inline]
fn store_child(stores: &Stores, node: Node, off: u32) -> IrNode<'_> {
    IrNode::Store(stores, node.field_vec(off).get(0, stores))
}

/// Diverge with a clear message when an accessor is called on the wrong variant
/// — a codegen bug (the caller must `match kind()` first), not a data fault.
#[cold]
#[inline(never)]
fn kind_panic(accessor: &str, node: &IrNode<'_>) -> ! {
    panic!("IrNode::{accessor} called on {:?}", node.kind());
}

/// Native `Value` → its [`ValueType`] discriminant.  The native half of the
/// `kind()` dispatch (the store half is `Node::value_type`).
fn native_value_kind(v: &Value) -> ValueType {
    use ValueType as K;
    match v {
        Value::Null => K::Null,
        Value::Line(_) => K::Line,
        Value::Span(_) => K::Span,
        Value::Int(_) => K::Int,
        Value::Enum(_, _) => K::Enum,
        Value::Boolean(_) => K::Boolean,
        Value::Float(_) => K::Float,
        Value::Long(_) => K::Long,
        Value::Single(_) => K::Single,
        Value::Text(_) => K::Text,
        Value::Call(_, _) => K::Call,
        Value::CallRef(_, _) => K::CallRef,
        Value::Block(_) => K::Block,
        Value::Insert(_) => K::Insert,
        Value::Var(_) => K::Var,
        Value::Set(_, _) => K::Set,
        Value::Return(_) => K::Return,
        Value::Break(_) => K::Break,
        Value::BreakWith(_, _) => K::BreakWith,
        Value::Continue(_) => K::Continue,
        Value::If(_, _, _) => K::If,
        Value::Loop(_) => K::Loop,
        Value::Drop(_) => K::Drop,
        Value::Iter(_, _, _, _) => K::Iter,
        Value::Keys(_) => K::Keys,
        Value::Tuple(_) => K::Tuple,
        Value::TupleGet(_, _) => K::TupleGet,
        Value::TuplePut(_, _, _) => K::TuplePut,
        Value::Yield(_) => K::Yield,
        Value::FnRef(_, _, _) => K::FnRef,
        Value::FnRefDnr(_) => K::FnRefDnr,
        Value::Parallel(_) => K::Parallel,
        Value::ParFor(_) => K::ParFor,
        Value::RawExpr(_) => K::RawExpr,
    }
}

/// Native `Type` → its [`TypeKind`] discriminant.  The native half of
/// `IrType::kind()` (the store half is `type_kind(disc)`).
fn native_type_kind(t: &Type) -> TypeKind {
    use TypeKind as K;
    match t {
        Type::Unknown(_) => K::Unknown,
        Type::Null => K::Null,
        Type::Void => K::Void,
        Type::Never => K::Never,
        Type::Integer(_) => K::Integer,
        Type::Boolean => K::Boolean,
        Type::Float => K::Float,
        Type::Single => K::Single,
        Type::Character => K::Character,
        Type::Text(_) => K::Text,
        Type::Keys => K::Keys,
        Type::Enum(_, _, _) => K::Enum,
        Type::Reference(_, _) => K::Reference,
        Type::RefVar(_) => K::RefVar,
        Type::Vector(_, _) => K::Vector,
        Type::Routine(_) => K::Routine,
        Type::Iterator(_, _) => K::Iterator,
        Type::Sorted(_, _, _) => K::Sorted,
        Type::Index(_, _, _) => K::Index,
        Type::Spacial(_, _, _) => K::Spacial,
        Type::Hash(_, _, _) => K::Hash,
        Type::Function(_, _, _) => K::Function,
        Type::Rewritten(_) => K::Rewritten,
        Type::Tuple(_) => K::Tuple,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Block, Position};
    use crate::ir_schema_gen::register_ir_schema;
    use crate::ir_store::materialize_node;

    fn pos() -> Position {
        Position {
            file: "f.loft".into(),
            line: 1,
            pos: 1,
        }
    }

    fn blk() -> Block {
        Block {
            name: "b",
            operators: vec![Value::Null],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }
    }

    /// Materialise `v` into a fresh store and hand back both backings of the
    /// SAME node: `(stores, native, store-node)`.  The store outlives the call
    /// via the returned owner so the handles can borrow it.
    fn both_backings(v: &Value) -> (Stores, ValuesVector) {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);
        let root = ValuesVector::new(stores.database(16));
        materialize_node(&mut stores, root, v);
        (stores, root)
    }

    /// The M3.0 bedrock guarantee: for every representative node, the native and
    /// store backings agree on `kind()` and every implemented accessor.  This is
    /// the per-step oracle every M3.1+ conversion relies on.
    #[test]
    fn cross_backing_accessors_agree() {
        let samples = vec![
            Value::Null,
            Value::Int(-7),
            Value::Long(1234),
            Value::Single(2.5),
            Value::Float(3.5),
            Value::Boolean(true),
            Value::Var(9),
            Value::Enum(3, 42),
            Value::Break(1),
            Value::Continue(2),
            Value::Line(7),
            Value::Keys(Vec::new()),
            Value::Text("hello".into()),
            Value::Call(17, vec![Value::Int(1), Value::Var(2), Value::Int(3)]),
            Value::Return(Box::new(Value::Int(99))),
            Value::Drop(Box::new(Value::Var(4))),
            Value::If(
                Box::new(Value::Boolean(true)),
                Box::new(Value::Int(1)),
                Box::new(Value::Null),
            ),
            Value::with_span(pos(), Value::Int(55)),
        ];
        for v in &samples {
            let (stores, root) = both_backings(v);
            let nat = IrNode::Native(v);
            let sto = IrNode::Store(&stores, root.get(0, &stores));
            assert_eq!(nat.kind(), sto.kind(), "kind disagrees for {v:?}");

            match nat.kind() {
                // payload-free leaves — `kind()` agreement (asserted above) is
                // the whole contract.
                ValueType::Null | ValueType::Keys => {}
                ValueType::Int | ValueType::Long => {
                    assert_eq!(nat.int_value(), sto.int_value());
                }
                // bit-exact: a materialise→read round-trip must not perturb a float.
                ValueType::Single => {
                    assert_eq!(nat.single_value().to_bits(), sto.single_value().to_bits());
                }
                ValueType::Float => {
                    assert_eq!(nat.float_value().to_bits(), sto.float_value().to_bits());
                }
                ValueType::Boolean => assert_eq!(nat.bool_value(), sto.bool_value()),
                ValueType::Var => assert_eq!(nat.var_nr(), sto.var_nr()),
                ValueType::Enum => assert_eq!(nat.enum_pair(), sto.enum_pair()),
                ValueType::Break => assert_eq!(nat.break_nr(), sto.break_nr()),
                ValueType::Continue => assert_eq!(nat.continue_nr(), sto.continue_nr()),
                ValueType::Line => assert_eq!(nat.line_nr(), sto.line_nr()),
                ValueType::Text => assert_eq!(nat.text(), sto.text()),
                ValueType::Call => {
                    assert_eq!(nat.call_to(), sto.call_to());
                    let (na, sa) = (nat.call_args(), sto.call_args());
                    assert_eq!(na.len(), sa.len());
                    for i in 0..na.len() {
                        assert_eq!(na.get(i).kind(), sa.get(i).kind(), "arg {i} kind");
                    }
                }
                ValueType::Return => {
                    assert_eq!(nat.return_inner().kind(), sto.return_inner().kind());
                    assert_eq!(
                        nat.return_inner().int_value(),
                        sto.return_inner().int_value()
                    );
                }
                ValueType::Drop => {
                    assert_eq!(nat.drop_inner().var_nr(), sto.drop_inner().var_nr());
                }
                ValueType::If => {
                    assert_eq!(nat.if_cond().kind(), sto.if_cond().kind());
                    assert_eq!(nat.if_then().int_value(), sto.if_then().int_value());
                    assert_eq!(nat.if_else().kind(), sto.if_else().kind());
                }
                ValueType::Span => {
                    // The wrapper itself is the node — unspan must agree across
                    // backings and reach the same inner leaf.
                    assert_eq!(nat.unspan().kind(), sto.unspan().kind());
                    assert_eq!(nat.unspan().int_value(), sto.unspan().int_value());
                }
                other => panic!("unhandled sample kind {other:?}"),
            }
        }
    }

    /// `unspan()` sees through `Span` wrappers identically on both backings.
    #[test]
    fn cross_backing_unspan_agrees() {
        let v = Value::with_span(pos(), Value::Var(8));
        let (stores, root) = both_backings(&v);
        let nat = IrNode::Native(&v);
        let sto = IrNode::Store(&stores, root.get(0, &stores));
        assert_eq!(nat.unspan().kind(), ValueType::Var);
        assert_eq!(sto.unspan().kind(), ValueType::Var);
        assert_eq!(nat.unspan().var_nr(), sto.unspan().var_nr());
    }

    /// The `kind()` dispatch is total for both enums — every native variant maps
    /// to a non-`Other` discriminant (a missing arm would surface as `Other`).
    #[test]
    fn native_kind_is_total() {
        // A node-bearing block exercises the Block/Loop discriminants too.
        let block = Value::Block(Box::new(blk()));
        assert_eq!(native_value_kind(&block), ValueType::Block);
        assert_eq!(
            native_value_kind(&Value::Loop(Box::new(blk()))),
            ValueType::Loop
        );
        assert!(!matches!(native_type_kind(&Type::Void), TypeKind::Other(_)));
        assert!(!matches!(
            native_type_kind(&Type::Boolean),
            TypeKind::Other(_)
        ));
    }
}
