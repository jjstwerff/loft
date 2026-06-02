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
//
// Every const below is pinned to the registered schema by the
// `baked_layout_mirrors_loft_schema` guard test — a mistyped offset fails CI
// rather than silently reading the wrong bytes.  `pub(crate)` so the arc B
// materializer (`crate::ir_store`) writes through the same offsets the readers
// use, keeping a single layout authority.

/// `Node` variant discriminants (1-based), in declaration order.
pub(crate) const DISC_NULL: u8 = 1;
pub(crate) const DISC_LINE: u8 = 2;
pub(crate) const DISC_SPAN: u8 = 3;
pub(crate) const DISC_INT: u8 = 4;
pub(crate) const DISC_ENUM: u8 = 5;
pub(crate) const DISC_BOOLEAN: u8 = 6;
pub(crate) const DISC_FLOAT: u8 = 7;
pub(crate) const DISC_LONG: u8 = 8;
pub(crate) const DISC_SINGLE: u8 = 9;
pub(crate) const DISC_TEXT: u8 = 10;
pub(crate) const DISC_CALL: u8 = 11;
pub(crate) const DISC_CALL_REF: u8 = 12;
pub(crate) const DISC_BLOCK: u8 = 13;
pub(crate) const DISC_INSERT: u8 = 14;
pub(crate) const DISC_VAR: u8 = 15;
pub(crate) const DISC_SET: u8 = 16;
pub(crate) const DISC_RETURN: u8 = 17;
pub(crate) const DISC_BREAK: u8 = 18;
pub(crate) const DISC_BREAK_WITH: u8 = 19;
pub(crate) const DISC_CONTINUE: u8 = 20;
pub(crate) const DISC_IF: u8 = 21;
pub(crate) const DISC_LOOP: u8 = 22;
pub(crate) const DISC_DROP: u8 = 23;
pub(crate) const DISC_ITER: u8 = 24;
pub(crate) const DISC_KEYS: u8 = 25;
pub(crate) const DISC_TUPLE: u8 = 26;
pub(crate) const DISC_TUPLE_GET: u8 = 27;
pub(crate) const DISC_TUPLE_PUT: u8 = 28;
pub(crate) const DISC_YIELD: u8 = 29;
pub(crate) const DISC_FN_REF: u8 = 30;
pub(crate) const DISC_FN_REF_DNR: u8 = 31;
pub(crate) const DISC_PARALLEL: u8 = 32;
pub(crate) const DISC_PAR_FOR: u8 = 33;
pub(crate) const DISC_RAW_EXPR: u8 = 34;

// Field byte offsets within each variant's record, relative to the record's
// `pos`.  The `enum` discriminant byte is always at 0.  The layout packs the
// first `vector<Node>` child at 4 and integers / further vectors at 8/12/16/20.
pub(crate) const NDLINE_N: u32 = 8;
pub(crate) const NDINT_N: u32 = 8;
pub(crate) const NDENUM_ORD: u32 = 8;
pub(crate) const NDENUM_TP: u32 = 16;
pub(crate) const NDBOOLEAN_B: u32 = 7;
pub(crate) const NDFLOAT_F: u32 = 8;
pub(crate) const NDLONG_N: u32 = 8;
pub(crate) const NDSINGLE_F: u32 = 4;
pub(crate) const NDTEXT_S: u32 = 4;
pub(crate) const NDCALL_ARGS: u32 = 4;
pub(crate) const NDCALL_DEF_NR: u32 = 8;
pub(crate) const NDCALLREF_ARGS: u32 = 4;
pub(crate) const NDCALLREF_VAR: u32 = 8;
pub(crate) const NDBLOCK_BLOCK: u32 = 8;
pub(crate) const BLOCK_NAME: u32 = 16;
pub(crate) const BLOCK_OPERATORS: u32 = 20;
pub(crate) const NDINSERT_ITEMS: u32 = 4;
pub(crate) const NDVAR_N: u32 = 8;
pub(crate) const NDSET_VAR: u32 = 8;
pub(crate) const NDSET_INNER: u32 = 4;
pub(crate) const NDRETURN_INNER: u32 = 4;
pub(crate) const NDBREAK_N: u32 = 8;
pub(crate) const NDBREAKWITH_N: u32 = 8;
pub(crate) const NDBREAKWITH_INNER: u32 = 4;
pub(crate) const NDCONTINUE_N: u32 = 8;
pub(crate) const NDIF_COND: u32 = 4;
pub(crate) const NDIF_T: u32 = 8;
pub(crate) const NDIF_F: u32 = 12;
pub(crate) const NDDROP_INNER: u32 = 4;
pub(crate) const NDITER_VAR: u32 = 8;
pub(crate) const NDITER_CREATE: u32 = 4;
pub(crate) const NDITER_NEXT: u32 = 16;
pub(crate) const NDITER_INIT: u32 = 20;
pub(crate) const NDTUPLE_ITEMS: u32 = 4;
pub(crate) const NDTUPLEGET_VAR: u32 = 8;
pub(crate) const NDTUPLEGET_IDX: u32 = 16;
pub(crate) const NDTUPLEPUT_VAR: u32 = 8;
pub(crate) const NDTUPLEPUT_IDX: u32 = 16;
pub(crate) const NDTUPLEPUT_INNER: u32 = 4;
pub(crate) const NDYIELD_INNER: u32 = 4;
pub(crate) const NDFNREFDNR_N: u32 = 8;
pub(crate) const NDPARALLEL_ARMS: u32 = 4;
pub(crate) const NDRAWEXPR_S: u32 = 4;

// Cross-struct variants — the inlined sub-struct's fields, as absolute offsets
// within the Node record (= the sub-struct's base offset + the field's offset
// inside the sub-struct).  The guard ties each back to base + relative.
pub(crate) const NDSPAN_INNER: u32 = 4;
pub(crate) const SPAN_POS_LINE: u32 = 8; // Position base (8) + Position.line (0)
pub(crate) const SPAN_POS_POS: u32 = 16; // + Position.pos (8)
pub(crate) const SPAN_POS_FILE: u32 = 24; // + Position.file (16)
pub(crate) const PARFOR_X_VAR: u32 = 8; // ParForBody base (8) + x_var (0)
pub(crate) const PARFOR_R_VAR: u32 = 16; // + r_var (8)
pub(crate) const PARFOR_STITCH_ID: u32 = 24; // + stitch_id (16)
pub(crate) const PARFOR_INPUT: u32 = 32; // + input (24)
pub(crate) const PARFOR_WORKER: u32 = 36; // + worker (28)
pub(crate) const PARFOR_THREADS: u32 = 40; // + threads (32)
pub(crate) const PARFOR_BODY: u32 = 44; // + body (36)

/// The bit mask loft uses for a stored `boolean` field (`generation` emits
/// `get_boolean(rec, off, 1)`).
pub(crate) const BOOL_MASK: u8 = 1;

/// Element stride of a `vector<Node>` (the `Node` enum's record size).
pub(crate) const NODE_STRIDE: u32 = 48;

/// Which `Node` variant a [`Value`] is.  Mirrors the native `data::Value`
/// variants 1:1; `Other(discriminant)` only on an unrecognised byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Null,
    Line,
    Span,
    Int,
    Enum,
    Boolean,
    Float,
    Long,
    Single,
    Text,
    Call,
    CallRef,
    Block,
    Insert,
    Var,
    Set,
    Return,
    Break,
    BreakWith,
    Continue,
    If,
    Loop,
    Drop,
    Iter,
    Keys,
    Tuple,
    TupleGet,
    TuplePut,
    Yield,
    FnRef,
    FnRefDnr,
    Parallel,
    ParFor,
    RawExpr,
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
    pub fn discriminant(&self, stores: &Stores) -> u8 {
        stores
            .store(&self.rec)
            .get_byte(self.rec.rec, self.rec.pos, 0) as u8
    }

    /// Which variant this value is.
    #[must_use]
    pub fn value_type(&self, stores: &Stores) -> ValueType {
        match self.discriminant(stores) {
            DISC_NULL => ValueType::Null,
            DISC_LINE => ValueType::Line,
            DISC_SPAN => ValueType::Span,
            DISC_INT => ValueType::Int,
            DISC_ENUM => ValueType::Enum,
            DISC_BOOLEAN => ValueType::Boolean,
            DISC_FLOAT => ValueType::Float,
            DISC_LONG => ValueType::Long,
            DISC_SINGLE => ValueType::Single,
            DISC_TEXT => ValueType::Text,
            DISC_CALL => ValueType::Call,
            DISC_CALL_REF => ValueType::CallRef,
            DISC_BLOCK => ValueType::Block,
            DISC_INSERT => ValueType::Insert,
            DISC_VAR => ValueType::Var,
            DISC_SET => ValueType::Set,
            DISC_RETURN => ValueType::Return,
            DISC_BREAK => ValueType::Break,
            DISC_BREAK_WITH => ValueType::BreakWith,
            DISC_CONTINUE => ValueType::Continue,
            DISC_IF => ValueType::If,
            DISC_LOOP => ValueType::Loop,
            DISC_DROP => ValueType::Drop,
            DISC_ITER => ValueType::Iter,
            DISC_KEYS => ValueType::Keys,
            DISC_TUPLE => ValueType::Tuple,
            DISC_TUPLE_GET => ValueType::TupleGet,
            DISC_TUPLE_PUT => ValueType::TuplePut,
            DISC_YIELD => ValueType::Yield,
            DISC_FN_REF => ValueType::FnRef,
            DISC_FN_REF_DNR => ValueType::FnRefDnr,
            DISC_PARALLEL => ValueType::Parallel,
            DISC_PAR_FOR => ValueType::ParFor,
            DISC_RAW_EXPR => ValueType::RawExpr,
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

    /// `NdInt.n` — the integer literal.
    #[must_use]
    pub fn int_value(&self, stores: &Stores) -> i64 {
        stores
            .store(&self.rec)
            .get_int(self.rec.rec, self.rec.pos + NDINT_N)
    }

    // ─── Write side (arc B materializer) ─────────────────────────────────────
    //
    // Each setter writes the discriminant byte then any scalar fields, via the
    // same baked offsets the readers use.  Vector children (`NdCall.args`,
    // `NdBlock.operators`) are filled by the caller through the matching
    // `call_parameters()` / `block_operators()` handle + `ValuesVector::push`.

    /// Stamp the variant discriminant byte (offset 0).
    pub(crate) fn set_discriminant(&self, stores: &mut Stores, disc: u8) {
        stores
            .store_mut(&self.rec)
            .set_byte(self.rec.rec, self.rec.pos, 0, i32::from(disc));
    }

    /// Write this slot as `NdNull`.
    pub fn write_null(&self, stores: &mut Stores) {
        self.set_discriminant(stores, DISC_NULL);
    }

    /// Write this slot as `NdInt`.
    pub fn write_int(&self, stores: &mut Stores, n: i64) {
        self.set_discriminant(stores, DISC_INT);
        stores
            .store_mut(&self.rec)
            .set_int(self.rec.rec, self.rec.pos + NDINT_N, n);
    }

    /// Write this slot as `NdCall` (`def_nr` only — fill `args` via
    /// [`Value::call_parameters`] + [`ValuesVector::push`]).
    pub fn write_call(&self, stores: &mut Stores, def_nr: u32) {
        self.set_discriminant(stores, DISC_CALL);
        stores.store_mut(&self.rec).set_int(
            self.rec.rec,
            self.rec.pos + NDCALL_DEF_NR,
            i64::from(def_nr),
        );
    }

    /// Write this slot as `NdBlock` with `name` (fill `operators` via
    /// [`Value::block_operators`] + [`ValuesVector::push`]).
    pub fn write_block(&self, stores: &mut Stores, name: &str) {
        self.set_discriminant(stores, DISC_BLOCK);
        self.block_name_set(stores, name);
    }

    /// Write this slot as `NdLoop` with `name` — same inlined `Block` layout as
    /// `NdBlock`, so `block_name`/`block_operators` apply unchanged.
    pub fn write_loop(&self, stores: &mut Stores, name: &str) {
        self.set_discriminant(stores, DISC_LOOP);
        self.block_name_set(stores, name);
    }

    // ─── Generic typed field access at a baked offset ────────────────────────
    //
    // Lower-level than the named accessors above: the arc B materializer writes
    // arbitrary variants by naming an offset const, without one method per
    // field.  Each folds to one `Store` primitive at one constant offset.

    /// Read the `integer` (8-byte) field at `off`.
    #[must_use]
    pub fn field_int(&self, stores: &Stores, off: u32) -> i64 {
        stores
            .store(&self.rec)
            .get_int(self.rec.rec, self.rec.pos + off)
    }

    /// Write the `integer` (8-byte) field at `off`.
    pub fn set_field_int(&self, stores: &mut Stores, off: u32, v: i64) {
        stores
            .store_mut(&self.rec)
            .set_int(self.rec.rec, self.rec.pos + off, v);
    }

    /// Read the `float` (f64) field at `off`.
    #[must_use]
    pub fn field_float(&self, stores: &Stores, off: u32) -> f64 {
        stores
            .store(&self.rec)
            .get_float(self.rec.rec, self.rec.pos + off)
    }

    /// Write the `float` (f64) field at `off`.
    pub fn set_field_float(&self, stores: &mut Stores, off: u32, v: f64) {
        stores
            .store_mut(&self.rec)
            .set_float(self.rec.rec, self.rec.pos + off, v);
    }

    /// Read the `single` (f32) field at `off`.
    #[must_use]
    pub fn field_single(&self, stores: &Stores, off: u32) -> f32 {
        stores
            .store(&self.rec)
            .get_single(self.rec.rec, self.rec.pos + off)
    }

    /// Write the `single` (f32) field at `off`.
    pub fn set_field_single(&self, stores: &mut Stores, off: u32, v: f32) {
        stores
            .store_mut(&self.rec)
            .set_single(self.rec.rec, self.rec.pos + off, v);
    }

    /// Read the `boolean` field at `off`.
    #[must_use]
    pub fn field_bool(&self, stores: &Stores, off: u32) -> bool {
        stores
            .store(&self.rec)
            .get_boolean(self.rec.rec, self.rec.pos + off, BOOL_MASK)
    }

    /// Write the `boolean` field at `off`.
    pub fn set_field_bool(&self, stores: &mut Stores, off: u32, v: bool) {
        stores
            .store_mut(&self.rec)
            .set_boolean(self.rec.rec, self.rec.pos + off, BOOL_MASK, v);
    }

    /// Read the `text` field at `off`.
    #[must_use]
    pub fn field_str<'a>(&self, stores: &'a Stores, off: u32) -> &'a str {
        let store = stores.store(&self.rec);
        store.get_str(store.get_u32_raw(self.rec.rec, self.rec.pos + off))
    }

    /// Write the `text` field at `off`.
    pub fn set_field_str(&self, stores: &mut Stores, off: u32, s: &str) {
        let store = stores.store_mut(&self.rec);
        let idx = store.set_str(s);
        store.set_u32_raw(self.rec.rec, self.rec.pos + off, idx);
    }

    /// Handle to the `vector<Node>` field at `off`.
    #[must_use]
    pub fn field_vec(&self, off: u32) -> ValuesVector {
        ValuesVector {
            rec: DbRef {
                store_nr: self.rec.store_nr,
                rec: self.rec.rec,
                pos: self.rec.pos + off,
            },
        }
    }
}

impl ValuesVector {
    /// Wrap the field slot where a `vector<Node>` header lives (e.g. a host
    /// record's field, for the arc B materializer's root).
    #[must_use]
    #[inline]
    pub fn new(rec: DbRef) -> Self {
        ValuesVector { rec }
    }

    /// The hidden `DbRef` of the vector field slot.
    #[must_use]
    #[inline]
    pub fn db_ref(&self) -> DbRef {
        self.rec
    }

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

    /// Append one element slot and return a handle to it (the slot's bytes are
    /// zeroed — write a variant into it via `Value::write_*`).
    pub fn push(&self, stores: &mut Stores) -> Value {
        let len = i64::from(self.len(stores));
        let rec = vector::insert_vector(&self.rec, NODE_STRIDE, len, &mut stores.allocations);
        Value { rec }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir_schema_gen::register_ir_schema;

    /// Claim a fresh `Node` record and write its discriminant.
    fn new_node(stores: &mut Stores, disc: u8) -> DbRef {
        let rec = stores.database(16);
        stores
            .store_mut(&rec)
            .set_byte(rec.rec, rec.pos, 0, i32::from(disc));
        rec
    }

    /// Append an element slot into the `vector<Node>` field at `field`
    /// (delegates to the production [`ValuesVector::push`]).
    fn push(stores: &mut Stores, field: DbRef) -> DbRef {
        ValuesVector::new(field).push(stores).db_ref()
    }

    /// Write an `NdInt` (disc + `n`) into `slot` (delegates to the production
    /// [`Value::write_int`]).
    fn write_int(stores: &mut Stores, slot: DbRef, n: i64) {
        Value::new(slot).write_int(stores, n);
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
        // Discriminants — every covered variant.
        assert_eq!(disc(ids.nd_null), DISC_NULL);
        assert_eq!(disc(ids.nd_line), DISC_LINE);
        assert_eq!(disc(ids.nd_span), DISC_SPAN);
        assert_eq!(disc(ids.nd_int), DISC_INT);
        assert_eq!(disc(ids.nd_enum), DISC_ENUM);
        assert_eq!(disc(ids.nd_boolean), DISC_BOOLEAN);
        assert_eq!(disc(ids.nd_float), DISC_FLOAT);
        assert_eq!(disc(ids.nd_long), DISC_LONG);
        assert_eq!(disc(ids.nd_single), DISC_SINGLE);
        assert_eq!(disc(ids.nd_text), DISC_TEXT);
        assert_eq!(disc(ids.nd_call), DISC_CALL);
        assert_eq!(disc(ids.nd_call_ref), DISC_CALL_REF);
        assert_eq!(disc(ids.nd_block), DISC_BLOCK);
        assert_eq!(disc(ids.nd_insert), DISC_INSERT);
        assert_eq!(disc(ids.nd_var), DISC_VAR);
        assert_eq!(disc(ids.nd_set), DISC_SET);
        assert_eq!(disc(ids.nd_return), DISC_RETURN);
        assert_eq!(disc(ids.nd_break), DISC_BREAK);
        assert_eq!(disc(ids.nd_break_with), DISC_BREAK_WITH);
        assert_eq!(disc(ids.nd_continue), DISC_CONTINUE);
        assert_eq!(disc(ids.nd_if), DISC_IF);
        assert_eq!(disc(ids.nd_loop), DISC_LOOP);
        assert_eq!(disc(ids.nd_drop), DISC_DROP);
        assert_eq!(disc(ids.nd_iter), DISC_ITER);
        assert_eq!(disc(ids.nd_keys), DISC_KEYS);
        assert_eq!(disc(ids.nd_tuple), DISC_TUPLE);
        assert_eq!(disc(ids.nd_tuple_get), DISC_TUPLE_GET);
        assert_eq!(disc(ids.nd_tuple_put), DISC_TUPLE_PUT);
        assert_eq!(disc(ids.nd_yield), DISC_YIELD);
        assert_eq!(disc(ids.nd_fn_ref), DISC_FN_REF);
        assert_eq!(disc(ids.nd_fn_ref_dnr), DISC_FN_REF_DNR);
        assert_eq!(disc(ids.nd_parallel), DISC_PARALLEL);
        assert_eq!(disc(ids.nd_par_for), DISC_PAR_FOR);
        assert_eq!(disc(ids.nd_raw_expr), DISC_RAW_EXPR);

        // Field offsets — every baked offset against its computed position.
        let pos = |tp: u16, f: &str| u32::from(stores.position(tp, f));
        assert_eq!(pos(ids.nd_line, "n"), NDLINE_N);
        assert_eq!(pos(ids.nd_int, "n"), NDINT_N);
        assert_eq!(pos(ids.nd_enum, "ord"), NDENUM_ORD);
        assert_eq!(pos(ids.nd_enum, "tp"), NDENUM_TP);
        assert_eq!(pos(ids.nd_boolean, "b"), NDBOOLEAN_B);
        assert_eq!(pos(ids.nd_float, "f"), NDFLOAT_F);
        assert_eq!(pos(ids.nd_long, "n"), NDLONG_N);
        assert_eq!(pos(ids.nd_single, "f"), NDSINGLE_F);
        assert_eq!(pos(ids.nd_text, "s"), NDTEXT_S);
        assert_eq!(pos(ids.nd_call, "args"), NDCALL_ARGS);
        assert_eq!(pos(ids.nd_call, "def_nr"), NDCALL_DEF_NR);
        assert_eq!(pos(ids.nd_call_ref, "args"), NDCALLREF_ARGS);
        assert_eq!(pos(ids.nd_call_ref, "var"), NDCALLREF_VAR);
        assert_eq!(pos(ids.nd_block, "block"), NDBLOCK_BLOCK);
        assert_eq!(pos(ids.block, "name"), BLOCK_NAME);
        assert_eq!(pos(ids.block, "operators"), BLOCK_OPERATORS);
        assert_eq!(pos(ids.nd_insert, "items"), NDINSERT_ITEMS);
        assert_eq!(pos(ids.nd_var, "n"), NDVAR_N);
        assert_eq!(pos(ids.nd_set, "var"), NDSET_VAR);
        assert_eq!(pos(ids.nd_set, "inner"), NDSET_INNER);
        assert_eq!(pos(ids.nd_return, "inner"), NDRETURN_INNER);
        assert_eq!(pos(ids.nd_break, "n"), NDBREAK_N);
        assert_eq!(pos(ids.nd_break_with, "n"), NDBREAKWITH_N);
        assert_eq!(pos(ids.nd_break_with, "inner"), NDBREAKWITH_INNER);
        assert_eq!(pos(ids.nd_continue, "n"), NDCONTINUE_N);
        assert_eq!(pos(ids.nd_if, "cond"), NDIF_COND);
        assert_eq!(pos(ids.nd_if, "t"), NDIF_T);
        assert_eq!(pos(ids.nd_if, "f"), NDIF_F);
        assert_eq!(pos(ids.nd_drop, "inner"), NDDROP_INNER);
        assert_eq!(pos(ids.nd_iter, "var"), NDITER_VAR);
        assert_eq!(pos(ids.nd_iter, "create"), NDITER_CREATE);
        assert_eq!(pos(ids.nd_iter, "next"), NDITER_NEXT);
        assert_eq!(pos(ids.nd_iter, "init"), NDITER_INIT);
        assert_eq!(pos(ids.nd_tuple, "items"), NDTUPLE_ITEMS);
        assert_eq!(pos(ids.nd_tuple_get, "var"), NDTUPLEGET_VAR);
        assert_eq!(pos(ids.nd_tuple_get, "idx"), NDTUPLEGET_IDX);
        assert_eq!(pos(ids.nd_tuple_put, "var"), NDTUPLEPUT_VAR);
        assert_eq!(pos(ids.nd_tuple_put, "idx"), NDTUPLEPUT_IDX);
        assert_eq!(pos(ids.nd_tuple_put, "inner"), NDTUPLEPUT_INNER);
        assert_eq!(pos(ids.nd_yield, "inner"), NDYIELD_INNER);
        assert_eq!(pos(ids.nd_fn_ref_dnr, "n"), NDFNREFDNR_N);
        assert_eq!(pos(ids.nd_parallel, "arms"), NDPARALLEL_ARMS);
        assert_eq!(pos(ids.nd_raw_expr, "s"), NDRAWEXPR_S);

        // Cross-struct inline variants — each absolute offset must equal the
        // sub-struct's base offset (the inlined field's own position) plus the
        // field's offset inside that sub-struct.
        assert_eq!(pos(ids.nd_span, "inner"), NDSPAN_INNER);
        let span_base = pos(ids.nd_span, "pos"); // inlined Position base
        assert_eq!(span_base + pos(ids.position, "line"), SPAN_POS_LINE);
        assert_eq!(span_base + pos(ids.position, "pos"), SPAN_POS_POS);
        assert_eq!(span_base + pos(ids.position, "file"), SPAN_POS_FILE);
        let pf_base = pos(ids.nd_par_for, "body"); // inlined ParForBody base
        assert_eq!(pf_base + pos(ids.par_for_body, "x_var"), PARFOR_X_VAR);
        assert_eq!(pf_base + pos(ids.par_for_body, "r_var"), PARFOR_R_VAR);
        assert_eq!(
            pf_base + pos(ids.par_for_body, "stitch_id"),
            PARFOR_STITCH_ID
        );
        assert_eq!(pf_base + pos(ids.par_for_body, "input"), PARFOR_INPUT);
        assert_eq!(pf_base + pos(ids.par_for_body, "worker"), PARFOR_WORKER);
        assert_eq!(pf_base + pos(ids.par_for_body, "threads"), PARFOR_THREADS);
        assert_eq!(pf_base + pos(ids.par_for_body, "body"), PARFOR_BODY);

        assert_eq!(u32::from(stores.size(ids.node)), NODE_STRIDE);
    }
}
