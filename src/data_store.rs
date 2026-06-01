// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 — store-backed compiler IR (`Data`).
//!
//! Simple wrappers around the IR's record/vector definitions: each handle hides
//! a `DbRef` and the methods retrieve fields via the already-written
//! Store/`vector` functions.  The wrappers do no Database re-derivation; they
//! read/write a field at its known offset with an existing primitive.
//!
//! The layout — variant discriminants, field byte offsets, the `Node` element
//! stride — is **hard-coded** here, mirroring exactly what loft's schema
//! ([`crate::ir_schema_gen::register_ir_schema`]) lays out.  Hand-written for
//! now to get the shape clean; later these consts come straight from the
//! generated layout, so the access is a folded constant with no lookup.

use crate::database::Stores;
use crate::keys::DbRef;
use crate::vector;

// ─── Hard-coded layout (mirrors the loft schema) ─────────────────────────────

/// `Node` variant discriminants (1-based), as `structure("NdInt", 4)` … assign.
const DISC_NULL: u8 = 1;
const DISC_INT: u8 = 4;
const DISC_CALL: u8 = 11;
const DISC_BLOCK: u8 = 13;

/// Field byte offsets within their record, relative to the record's `pos`.
const NDCALL_ARGS: u32 = 4;
const NDCALL_DEF_NR: u32 = 8;
const NDBLOCK_BLOCK: u32 = 8;
const BLOCK_NAME: u32 = 16;
const BLOCK_OPERATORS: u32 = 20;

/// Element stride of a `vector<Node>` (the `Node` enum's record size).
const NODE_STRIDE: u32 = 48;

/// Which `Node` variant a [`Value`] is — the subset the IR-walker matches on so
/// far.  Anything else surfaces as `Other(discriminant)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Null,
    Int,
    Call,
    Block,
    Other(u8),
}

/// A handle to one IR `Value` (a `Node` record), hiding its [`DbRef`].
#[derive(Debug, Clone, Copy)]
pub struct Value {
    rec: DbRef,
}

/// A handle to a `vector<Node>` field, hiding the vector internals.  `rec`
/// points at the field slot in the owning record (where the vector header
/// lives), as the `vector::*` functions expect.
#[derive(Debug, Clone, Copy)]
pub struct ValuesVector {
    rec: DbRef,
}

impl Value {
    /// Wrap a `Node` record.
    #[must_use]
    #[inline]
    pub fn new(rec: DbRef) -> Self {
        Value { rec }
    }

    /// The hidden `DbRef`, for rust code that drives the existing functions
    /// directly.
    #[must_use]
    #[inline]
    pub fn db_ref(&self) -> DbRef {
        self.rec
    }

    /// The 1-based variant discriminant stored at the record's `enum` byte
    /// (offset 0).
    #[must_use]
    fn discriminant(&self, stores: &Stores) -> u8 {
        stores
            .store(&self.rec)
            .get_byte(self.rec.rec, self.rec.pos, 0) as u8
    }

    /// Which variant this value is.
    #[must_use]
    pub fn value_type(&self, stores: &Stores) -> ValueType {
        match self.discriminant(stores) {
            DISC_NULL => ValueType::Null,
            DISC_INT => ValueType::Int,
            DISC_CALL => ValueType::Call,
            DISC_BLOCK => ValueType::Block,
            other => ValueType::Other(other),
        }
    }

    /// `NdCall.def_nr` — the called definition number.
    #[must_use]
    pub fn call_to(&self, stores: &Stores) -> u32 {
        stores
            .store(&self.rec)
            .get_int(self.rec.rec, self.rec.pos + NDCALL_DEF_NR) as u32
    }

    /// `NdCall.args` — the call parameters.
    #[must_use]
    pub fn call_parameters(&self) -> ValuesVector {
        ValuesVector {
            rec: DbRef {
                store_nr: self.rec.store_nr,
                rec: self.rec.rec,
                pos: self.rec.pos + NDCALL_ARGS,
            },
        }
    }

    /// `NdBlock`'s inlined `Block.name`.
    #[must_use]
    pub fn block_name<'a>(&self, stores: &'a Stores) -> &'a str {
        let pos = self.rec.pos + NDBLOCK_BLOCK + BLOCK_NAME;
        let store = stores.store(&self.rec);
        store.get_str(store.get_u32_raw(self.rec.rec, pos))
    }

    /// Set `NdBlock`'s inlined `Block.name`.
    pub fn block_name_set(&self, stores: &mut Stores, name: &str) {
        let pos = self.rec.pos + NDBLOCK_BLOCK + BLOCK_NAME;
        let store = stores.store_mut(&self.rec);
        let idx = store.set_str(name);
        store.set_u32_raw(self.rec.rec, pos, idx);
    }

    /// `NdBlock`'s inlined `Block.operators`.
    #[must_use]
    pub fn block_operators(&self) -> ValuesVector {
        ValuesVector {
            rec: DbRef {
                store_nr: self.rec.store_nr,
                rec: self.rec.rec,
                pos: self.rec.pos + NDBLOCK_BLOCK + BLOCK_OPERATORS,
            },
        }
    }
}

impl ValuesVector {
    /// Number of elements.
    #[must_use]
    pub fn len(&self, stores: &Stores) -> u32 {
        vector::length_vector(&self.rec, &stores.allocations)
    }

    /// Whether the vector is empty.
    #[must_use]
    pub fn is_empty(&self, stores: &Stores) -> bool {
        self.len(stores) == 0
    }

    /// The `i`-th element.  Out-of-range yields a null [`Value`] (rec 0).
    #[must_use]
    pub fn get(&self, i: u32, stores: &Stores) -> Value {
        let rec = vector::get_vector(&self.rec, NODE_STRIDE, i64::from(i), &stores.allocations);
        Value { rec }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir_schema_gen::register_ir_schema;

    /// `NdInt.n` offset — test-only scaffolding until a production int accessor
    /// needs it; guarded in [`baked_layout_mirrors_loft_schema`] like the rest.
    const NDINT_N: u32 = 8;

    /// Claim a fresh `Node` record and write its discriminant.
    fn new_node(stores: &mut Stores, disc: u8) -> DbRef {
        let rec = stores.database(16);
        stores
            .store_mut(&rec)
            .set_byte(rec.rec, rec.pos, 0, i32::from(disc));
        rec
    }

    /// Append an element slot into the `vector<Node>` field at `field`.
    fn push(stores: &mut Stores, field: DbRef) -> DbRef {
        let len = i64::from(vector::length_vector(&field, &stores.allocations));
        vector::insert_vector(&field, NODE_STRIDE, len, &mut stores.allocations)
    }

    /// Write an `NdInt` (disc + `n`) into `slot`.
    fn write_int(stores: &mut Stores, slot: DbRef, n: i64) {
        let store = stores.store_mut(&slot);
        store.set_byte(slot.rec, slot.pos, 0, i32::from(DISC_INT));
        store.set_int(slot.rec, slot.pos + NDINT_N, n);
    }

    #[test]
    fn ndcall_reads_back_through_handles() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let call = new_node(&mut stores, DISC_CALL);
        stores
            .store_mut(&call)
            .set_int(call.rec, call.pos + NDCALL_DEF_NR, 144);

        let args = Value::new(call).call_parameters();
        let s0 = push(&mut stores, args.rec);
        write_int(&mut stores, s0, 7);
        let s1 = push(&mut stores, args.rec);
        write_int(&mut stores, s1, 9);

        let v = Value::new(call);
        assert_eq!(v.value_type(&stores), ValueType::Call);
        assert_eq!(v.call_to(&stores), 144);
        let params = v.call_parameters();
        assert_eq!(params.len(&stores), 2);
        assert_eq!(params.get(0, &stores).value_type(&stores), ValueType::Int);
    }

    #[test]
    fn ndblock_name_and_operators_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let block = Value::new(new_node(&mut stores, DISC_BLOCK));
        block.block_name_set(&mut stores, "loop_body");

        let slot = push(&mut stores, block.block_operators().rec);
        write_int(&mut stores, slot, 42);

        assert_eq!(block.value_type(&stores), ValueType::Block);
        assert_eq!(block.block_name(&stores), "loop_body");
        let ops = block.block_operators();
        assert_eq!(ops.len(&stores), 1);
        assert_eq!(ops.get(0, &stores).value_type(&stores), ValueType::Int);

        block.block_name_set(&mut stores, "x");
        assert_eq!(block.block_name(&stores), "x");
    }

    /// The hard-coded layout must mirror the registered loft schema exactly —
    /// a guard so a schema change can't silently desync the baked consts.
    #[test]
    fn baked_layout_mirrors_loft_schema() {
        use crate::database::Parts;
        let mut stores = Stores::new();
        let ids = register_ir_schema(&mut stores);

        let disc = |tp: u16| {
            if let Parts::EnumValue(d, _) = &stores.get_type(tp).parts {
                *d
            } else {
                0
            }
        };
        assert_eq!(disc(ids.nd_null), DISC_NULL);
        assert_eq!(disc(ids.nd_int), DISC_INT);
        assert_eq!(disc(ids.nd_call), DISC_CALL);
        assert_eq!(disc(ids.nd_block), DISC_BLOCK);

        let pos = |tp: u16, f: &str| u32::from(stores.position(tp, f));
        assert_eq!(pos(ids.nd_int, "n"), NDINT_N);
        assert_eq!(pos(ids.nd_call, "args"), NDCALL_ARGS);
        assert_eq!(pos(ids.nd_call, "def_nr"), NDCALL_DEF_NR);
        assert_eq!(pos(ids.nd_block, "block"), NDBLOCK_BLOCK);
        assert_eq!(pos(ids.block, "name"), BLOCK_NAME);
        assert_eq!(pos(ids.block, "operators"), BLOCK_OPERATORS);
        assert_eq!(u32::from(stores.size(ids.node)), NODE_STRIDE);
    }
}
