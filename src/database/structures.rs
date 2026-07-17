// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//! Structure allocation, initialization, field get/set, parsing operations.

use crate::database::{Field, Parts, Stores};

/// `LOFT_TRACE_VADD=1` — trace `vector_add` stride resolution (read once;
/// `vector_add` is runtime-hot, a per-call `env::var` lookup is not).
fn vadd_trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_TRACE_VADD").is_ok())
}
use crate::keys::DbRef;
use crate::store::Store;
use crate::vector;
use crate::{hash, keys, tree};
use std::collections::HashSet;

/// Walker-native diagnostic for `walk_parsed_into` failures.
///
/// `at` is a byte offset into the original input; `path` is the
/// dotted-key / `[index]` path to the failing node.  `format.rs`
/// converts these into the user-visible `"line N:M path:X"` shape
/// using `crate::json::line_col_of`.
pub(super) struct WalkErr {
    pub at: usize,
    pub path: Vec<String>,
}

impl Stores {
    /**
    # Panics
    When requesting a record on a non-structure
    */
    /// @PLN25 single-payload — for a FIELD (`field != MAX`) inside a `__nullable<S>` parent,
    /// redirect to the inline `payload`'s dense `S` (the `key_owner` struct) + add the payload
    /// base to the record ref, so the field resolves on dense `S` instead of the enum top level
    /// (whose field 0 is the discriminant).  Returns `(resolved_parent_tp, adjusted_ref)`.
    /// A `field == MAX` call (creating the element record itself, which IS the enum) and any
    /// non-nullable parent are returned unchanged.  Shared by `record_new` / `record_finish` so
    /// the create + finalize halves agree on the type/offset.
    fn nullable_field_parent(&self, data: &DbRef, parent_tp: u16, field: u16) -> (u16, DbRef) {
        if field != u16::MAX {
            let owner = self.key_owner(parent_tp);
            if owner != parent_tp {
                let mut adj = *data;
                adj.pos += u32::from(self.key_base(parent_tp));
                return (owner, adj);
            }
        }
        (parent_tp, *data)
    }

    /// Create a fresh record for a collection element / nullable field and return its `DbRef`.
    ///
    /// # Panics
    /// Panics on an unsupported `parent_tp`/`field` parts kind (an internal invariant
    /// violation — the parser only emits `OpNewRecord` for collection/struct field types).
    pub fn record_new(&mut self, data: &DbRef, parent_tp: u16, field: u16) -> DbRef {
        // @PLN101 Slice 0 — count every heap record allocation (the cost value structs remove).
        self.records_created += 1;
        // @PLN25 single-payload: when creating a sub-record for a FIELD inside a
        // `__nullable<S>` element (a nested collection/struct), the field lives in the inline
        // `payload` (dense S), not at the enum's top level (field 0 there is the discriminant,
        // which has no sub-structure → `field_type` = MAX → OOB).  Redirect a `__nullable<S>`
        // parent to the payload's struct + base offset so the field resolution + sub-record
        // allocation target the dense `S` — the `key_owner` redirect.  Only for `field != MAX`:
        // a `field == MAX` call creates the element record ITSELF (which IS the enum), so it
        // must keep `parent_tp`.  Non-nullable parents are unchanged (`key_owner` = identity).
        let (parent_tp, data_owned) = self.nullable_field_parent(data, parent_tp, field);
        let data = &data_owned;
        let tp = if field == u16::MAX {
            // This case is when the top level is a data-structure
            parent_tp
        } else {
            self.field_type(parent_tp, field)
        };
        let d = self.field_ref(data, parent_tp, field);
        match self.types[tp as usize].parts {
            Parts::Sorted(c, _) => {
                vector::sorted_new(&d, u32::from(self.size(c)), &mut self.allocations)
            }
            Parts::Vector(c) => {
                // #475: when the content `c` is itself a vector (a nested vector
                // like `vector<vector<T>>`), stride the OUTER slot by the inner
                // scalar width — the de-facto stride the index
                // (`elm_size_raw.max(4)` in parser/fields.rs), iteration, and
                // local construction all use.  `self.size(c)` is the 4-byte rec-id
                // handle; a struct-field nested vector would stride the outer slot
                // by 4 and mismatch the stride-8 index → #475 crash.
                let stride = if matches!(
                    self.types[c as usize].parts,
                    Parts::Vector(_) | Parts::Array(_)
                ) {
                    u32::from(self.size(self.content(c))).max(4)
                } else {
                    u32::from(self.size(c))
                };
                vector::vector_append(&d, stride, &mut self.allocations)
            }
            Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Index(c, _, _)
            | Parts::Radix(c, _) => {
                let rec = self.claim(&d, 1 + ((u32::from(self.size(c)) + 7) >> 3));
                self.store_mut(&rec).set_u32_raw(rec.rec, 4, data.rec);
                rec
            }
            _ => panic!(
                "Cannot add to none-structure '{}'",
                self.types[tp as usize].name
            ),
        }
    }

    /**
    # Panics
    When the implementation is not yet written
    */
    pub fn record_finish(&mut self, data: &DbRef, rec: &DbRef, parent_tp: u16, field: u16) {
        // @PLN25 single-payload: mirror `record_new`'s nullable-field redirect so the
        // create + finalize halves agree on the type/offset (a FIELD inside a `__nullable<S>`
        // element resolves on the payload's dense `S`, not the enum top level).
        let (parent_tp, data_owned) = self.nullable_field_parent(data, parent_tp, field);
        let data = &data_owned;
        let tp = if field == u16::MAX {
            // This case is when the top level is a data-structure
            parent_tp
        } else {
            self.field_type(parent_tp, field)
        };
        let d = self.field_ref(data, parent_tp, field);
        self.insert_record(&d, rec, tp, false);
        if field != u16::MAX
            && let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                self.types[parent_tp as usize].parts.clone()
        {
            let f = &fields[field as usize];
            let o = &f.other_indexes;
            if !o.is_empty() && o[0] != u16::MAX {
                for fld_nr in o {
                    let sibling_content = fields[*fld_nr as usize].content;
                    // @PLN25 Scope B — a keyed index over a shared NULLABLE array indexes only
                    // the non-null records: when the sibling keyed element is `__nullable<S>`
                    // and this record is the `Null` variant (discriminant 1 at byte offset 0),
                    // skip the keyed insert.  The null stays in the vector (the primary insert
                    // at the top of this fn) but is unreachable by key and cannot collide with a
                    // real empty/zero-key element.  Inert for dense (non-nullable) keyed elements.
                    // Index ONLY a `Some` record (discriminant 2).  A null element is either
                    // the zeroed/absent slot (discriminant 0) or the explicit `Null` variant
                    // (discriminant 1); both are skipped.
                    if self
                        .nullable_some_variant(self.content(sibling_content))
                        .is_some()
                        && self.store(rec).get_byte(rec.rec, rec.pos, 0) != 2
                    {
                        continue;
                    }
                    let o = self.field_ref(data, parent_tp, *fld_nr);
                    // Secondary index for a sibling field — index-only, never
                    // delete the displaced record (the primary collection owns it).
                    self.insert_record(&o, rec, sibling_content, true);
                }
            }
        }
    }

    /// @P305 — true when the keyed field at byte offset `byte_off` in
    /// struct / enum-value type `struct_tp` is cross-linked with a sibling
    /// index (the multi-index case: two-or-more keyed fields sharing an
    /// element type are auto-linked in `types.rs`).  `OpSetKeyed` lacks the
    /// struct + field context to maintain the sibling indexes, so the parser
    /// falls back to the (non-corrupting) update-only path for these.
    #[must_use]
    pub fn keyed_field_is_linked(&self, struct_tp: u16, byte_off: u16) -> bool {
        if (struct_tp as usize) >= self.types.len() {
            return false;
        }
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[struct_tp as usize].parts
        {
            for f in fields {
                if f.position == byte_off {
                    return !f.other_indexes.is_empty();
                }
            }
        }
        false
    }

    pub(super) fn field_ref(&self, data: &DbRef, parent_tp: u16, field: u16) -> DbRef {
        if field == u16::MAX {
            *data
        } else if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[parent_tp as usize].parts
        {
            DbRef {
                store_nr: data.store_nr,
                rec: data.rec,
                pos: data.pos + u32::from(fields[field as usize].position),
            }
        } else {
            *data
        }
    }

    pub(super) fn insert_record(&mut self, data: &DbRef, rec: &DbRef, tp: u16, secondary: bool) {
        match self.types[tp as usize].parts.clone() {
            Parts::Vector(_) => {
                vector::vector_finish(data, &mut self.allocations);
            }
            Parts::Sorted(c, _) => {
                let size = u32::from(self.size(c));
                vector::sorted_finish(
                    data,
                    size,
                    &self.types[tp as usize].keys,
                    &mut self.allocations,
                );
            }
            Parts::Array(_) => {
                let reference = vector::vector_append(data, 4, &mut self.allocations);
                self.store_mut(data)
                    .set_u32_raw(reference.rec, reference.pos, rec.rec);
                vector::vector_finish(data, &mut self.allocations);
            }
            Parts::Hash(c, _) => {
                // @P306 — replace any existing record with this key (dedup).
                self.dedup_keyed(data, rec, tp, c, secondary);
                let keys = self.types[tp as usize].keys.clone();
                hash::add(data, rec, &mut self.allocations, &keys);
            }
            Parts::Index(c, _, _) => {
                // @P306 — replace any existing record with this key (dedup);
                // tree::add otherwise rejects the duplicate and keeps the old.
                self.dedup_keyed(data, rec, tp, c, secondary);
                let left = self.fields(tp);
                let keys = self.types[tp as usize].keys.clone();
                tree::add(data, rec, left, &mut self.allocations, &keys);
            }
            Parts::Ordered(_, _) => {
                vector::ordered_finish(
                    data,
                    rec,
                    &self.types[tp as usize].keys,
                    &mut self.allocations,
                );
            }
            Parts::Radix(_, _) => {
                // @PLN48 S2 — no dedup: two records may share a cell (they differ in
                // the id suffix and land adjacent), which is what a spatial index
                // needs.  A future `radix<T[k]>` map surface can layer dedup on top.
                let keys = self.types[tp as usize].keys.clone();
                crate::radix_db::add(data, rec, &mut self.allocations, &keys);
            }
            _ => (),
        }
    }

    /// @PLAN53 cluster 3: sound cross-store byte copy.  Copies `len` bytes from
    /// `src_idx`'s record (`src_rec` @ `from_pos`) into `dst_idx`'s record
    /// (`dst_rec` @ `to_pos`).  `get_disjoint_mut` produces the two store borrows
    /// from disjoint sub-ranges of `allocations`, so neither overlaps the whole
    /// slice — the old `from_ref`/`from_mut` reborrow pair formed a `&[Store]`
    /// and a `&mut [Store]` over the same slice simultaneously, which Miri's
    /// Stacked Borrows (correctly) rejects as UB.  Behaviour is identical (same
    /// bytes copied).  Caller guarantees `src_idx != dst_idx`.
    #[allow(clippy::too_many_arguments)] // low-level (idx, rec, pos) src+dst+len copy descriptor
    pub(crate) fn copy_block_cross_store(
        &mut self,
        src_idx: u16,
        src_rec: u32,
        from_pos: isize,
        dst_idx: u16,
        dst_rec: u32,
        to_pos: isize,
        len: isize,
    ) {
        let [src_store, dst_store] = self
            .allocations
            .get_disjoint_mut([src_idx as usize, dst_idx as usize])
            .expect("copy_block_cross_store: src and dst store indices must be distinct");
        let src_store: &Store = src_store;
        src_store.copy_block_between(src_rec, from_pos, dst_store, dst_rec, to_pos, len);
    }

    /// P213: claim a fresh record in `host_field`'s Store of size matching
    /// `content_kt`, deep-copy `src`'s payload + nested heap fields into
    /// it, then write the new rec-id u32 into `host_field`.  Used by
    /// capturing-closure-in-struct-field writes to co-locate the
    /// parent-Store closure record in host's Store.  The new record's
    /// lifetime is bound to the host: `Parts::ChildRec(content_kt)`'s
    /// cascade in `copy_claims` / `remove_claims` deep-copies and frees
    /// it whenever the host is copied or freed.
    pub fn claim_child_rec(&mut self, host_field: &DbRef, src: &DbRef, content_kt: u16) {
        if src.rec == 0 {
            return;
        }
        let size = u32::from(self.size(content_kt));
        let new_rec = self.allocations[host_field.store_nr as usize].claim(size);
        let new_db = DbRef {
            store_nr: host_field.store_nr,
            rec: new_rec,
            pos: 8,
        };
        // Cross-store byte copy of the child record's payload.
        if host_field.store_nr == src.store_nr {
            self.store_mut(host_field).copy_block(
                src.rec,
                src.pos as isize,
                new_rec,
                new_db.pos as isize,
                size as isize,
            );
        } else {
            self.copy_block_cross_store(
                src.store_nr,
                src.rec,
                src.pos as isize,
                host_field.store_nr,
                new_rec,
                new_db.pos as isize,
                size as isize,
            );
        }
        // Deep-copy nested heap fields (text, Reference, vector, ...).
        self.copy_claims(src, &new_db, content_kt);
        // Write the new rec-id into host's field as a u32.
        self.store_mut(host_field)
            .set_u32_raw(host_field.rec, host_field.pos, new_rec);
    }

    /// P213: read the rec-id u32 at `host_field` and construct a `DbRef`
    /// pointing at element [0]'s standard struct payload start (`pos=8`)
    /// in the same Store as the host.  Returns the null sentinel
    /// (`store_nr=u16::MAX, rec=0, pos=0`) when the rec-id is 0
    /// (non-capturing / default-init case).
    #[must_use]
    pub fn ref_from_child_rec(&self, host_field: &DbRef) -> DbRef {
        let store = keys::store(host_field, &self.allocations);
        let rec = store.get_u32_raw(host_field.rec, host_field.pos);
        if rec == 0 {
            DbRef::NULL
        } else {
            DbRef {
                store_nr: host_field.store_nr,
                rec,
                pos: 8,
            }
        }
    }

    /// Make `db` (dest) hold `o_db` (src)'s content — the aliasing-safe vector
    /// "deliver into buffer" the return machinery needs.  When `db` and `o_db`
    /// name the SAME backing vector (the NRVO case where a returned local still
    /// ALIASES the function's buffer — `out` borrows `__vdb_1`, and the buffer is
    /// the return slot), the content is ALREADY in place: clearing first would
    /// DESTROY it, which is the `clear(buf); append(buf, out)` self-copy that
    /// silently returned an empty vector.  Same vector → no-op.  Distinct vectors
    /// → clear dest and append src (`vector_add` snapshots a same-store source).
    pub fn vector_replace(&mut self, db: &DbRef, o_db: &DbRef, known: u16) {
        let dest_rec = keys::store(db, &self.allocations).get_u32_raw(db.rec, db.pos);
        let src_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_db.rec, o_db.pos);
        if db.store_nr == o_db.store_nr && dest_rec != 0 && dest_rec == src_rec {
            return;
        }
        vector::clear_vector(db, &mut self.allocations);
        self.vector_add(db, o_db, known);
    }

    pub fn vector_add(&mut self, db: &DbRef, o_db: &DbRef, known: u16) {
        // `LOFT_TRACE_VADD=1` prints one line per vector concat/append-copy
        // with the resolved stride — the instrument that settled the nested
        // `a += b` stride bug (rows strode by the 4-byte field-slot size
        // instead of the read path's clamped content size).
        if vadd_trace_enabled() {
            eprintln!(
                "[vadd] db={db:?} o_db={o_db:?} known={} size={} linked={} parts={:?}",
                known,
                self.size(known),
                self.is_linked(known),
                self.types.get(known as usize).map(|t| t.parts.clone())
            );
        }
        let o_length = vector::length_vector(o_db, &self.allocations);
        if o_length == 0 {
            // The other vector has no data
            return;
        }
        // @PLN90 phase 1 — make the copy visible. A non-empty source means `vector_add`
        // deep-copies `o_length` elements into the destination store: a real structure
        // copy. `LOFT_COPY_DUMP` reports it with the element count (the runtime size — the
        // "hundreds of MB just to be sure" the user cannot see today). No source line here:
        // `Stores` has no `State`; the source location is the compile-time decision's job
        // (COPY_DIAGNOSTICS.md phase 2).
        if keys::copy_dump_enabled() {
            eprintln!("[copy] vector-append elements={o_length}  tp={known}");
        }
        // @P376 — when `known` is "linked" (multiple containers share this
        // content type), `vector<known>` was promoted to `Array(known)` in
        // `Stores::finish_type`: storage is a u32 rec-id per element pointing
        // to a separate record of type `known`, not an inline `size(known)`
        // payload.  The inline byte-copy below would read garbage past the
        // 4-byte source pointer slots; route Array→Array appends through a
        // dedicated path that mirrors `out += [elem]` (claim fresh element
        // record in dest store, byte-copy from source's element record,
        // deep-copy nested heap, append u32 rec-id to dest's array).
        if self.is_linked(known) {
            self.vector_add_array(db, o_db, known, o_length);
            return;
        }
        // Snapshot the source record number BEFORE any resize: if `db` and `o_db` share the
        // same backing store the resize inside `vector_append` / `vector_set_size` may
        // reallocate the vector and invalidate `o_rec`.  Reading it after the resize would
        // reference freed memory, silently producing corrupt data.
        let o_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_db.rec, o_db.pos);
        // Element stride.  For VECTOR-typed rows (`vector<vector<T>>`), the
        // row is a small record holding the inner vector's u32 handle, and
        // its stride is what the READ path computes (`fields.rs`
        // `elm_size`): the inner content's size clamped to >= 4 (the
        // @PLAN58 boolean-handle clamp) — `size(known)` itself is the
        // 4-byte struct-FIELD slot size, which under-strides 8/16-byte
        // rows and made nested `a += b` read garbage handles (SIGSEGV) or
        // shallow-copy rows.  Scalar/struct rows keep `size(known)`.
        let size = if matches!(self.types[known as usize].parts, Parts::Vector(_)) {
            u32::from(self.size(self.content(known))).max(4)
        } else {
            u32::from(self.size(known))
        };
        // If source and destination share the same backing vector record, copy source elements
        // to a local buffer first so the resize cannot invalidate the source pointer.
        let same_vec = db.store_nr == o_db.store_nr && o_rec != 0 && {
            let dest_rec = keys::store(db, &self.allocations).get_u32_raw(db.rec, db.pos);
            dest_rec == o_rec
        };
        let snapshot: Vec<u8> = if same_vec {
            let store = keys::store(o_db, &self.allocations);
            let byte_len = o_length as usize * size as usize;
            (0..byte_len)
                .map(|i| *store.addr::<u8>(o_rec, 8 + i as u32))
                .collect()
        } else {
            Vec::new()
        };
        let new_db = vector::vector_append(db, size, &mut self.allocations);
        let append_pos = new_db.pos;
        // Claim more than 1 record if needed for the actual copy.
        self.vector_set_size(db, o_length, size);
        // `vector_set_size` may have relocated the destination record.
        // `new_db.rec` captured from `vector_append` is stale after relocation;
        // re-read the current rec from the field slot (which `vector_set_size`
        // keeps up to date) before we use it for the byte copy.  Element
        // offset (`append_pos`) is layout-stable across relocation.
        let dest_rec = keys::store(db, &self.allocations).get_u32_raw(db.rec, db.pos);
        let new_db = DbRef {
            store_nr: db.store_nr,
            rec: dest_rec,
            pos: append_pos,
        };
        if same_vec {
            // Write from the pre-resize snapshot; `new_db.rec` is already the correct
            // (possibly reallocated) destination record after `vector_set_size`.
            let store = keys::mut_store(db, &mut self.allocations);
            for (i, &byte) in snapshot.iter().enumerate() {
                *store.addr_mut::<u8>(new_db.rec, new_db.pos + i as u32) = byte;
            }
        } else if db.store_nr == o_db.store_nr {
            // Re-read o_rec after resize in case it moved (non-self-append same-store case).
            let o_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_db.rec, o_db.pos);
            keys::mut_store(db, &mut self.allocations).copy_block(
                o_rec,
                8,
                new_db.rec,
                new_db.pos as isize,
                o_length as isize * size as isize,
            );
        } else {
            // @PLAN53 cluster 3: two different data structures in distinct stores —
            // copy via the sound disjoint-borrow helper instead of an aliasing reborrow.
            self.copy_block_cross_store(
                o_db.store_nr,
                o_rec,
                8,
                db.store_nr,
                new_db.rec,
                new_db.pos as isize,
                o_length as isize * size as isize,
            );
        }
        // After the raw byte copy, slot indices for text and sub-structure fields in each
        // appended element still point into the source store.  Deep-copy those claims so
        // that the destination owns independent copies and is not affected when the source
        // vector is freed.
        for i in 0..o_length {
            self.copy_claims(
                &DbRef {
                    store_nr: o_db.store_nr,
                    rec: o_rec,
                    pos: 8 + size * i,
                },
                &DbRef {
                    store_nr: db.store_nr,
                    rec: new_db.rec,
                    pos: new_db.pos + size * i,
                },
                known,
            );
            // LOFT_WATCH_STORE — flag the element if this append left a garbage text-ptr.
            self.watch_oob_text(
                &DbRef {
                    store_nr: db.store_nr,
                    rec: new_db.rec,
                    pos: new_db.pos + size * i,
                },
                known,
                Some(&DbRef {
                    store_nr: o_db.store_nr,
                    rec: o_rec,
                    pos: 8 + size * i,
                }),
                "vector_add",
            );
        }
    }

    /// @P376 — Array(content) → Array(content) append.  When the content
    /// type is "linked" the vector storage is a u32-per-element rec-id
    /// table pointing to separately-claimed element records (see
    /// `Stores::finish_type` for the Vector→Array promotion).  Mirror what
    /// the IR emits for `out += [elem]`: claim a fresh element record in
    /// the destination's store, byte-copy the source record's payload,
    /// deep-copy nested heap, then push the new rec-id into the dest's
    /// array via `vector_append` (size=4) + `vector_finish`.
    fn vector_add_array(&mut self, db: &DbRef, o_db: &DbRef, known: u16, o_length: u32) {
        let elem_size = u32::from(self.size(known));
        // Source array record (u32 rec-id per slot, header at offset 0/4).
        let o_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_db.rec, o_db.pos);
        if o_rec == 0 {
            return;
        }
        // Words to claim per element record: matches `record_new`'s Array
        // arm — 1 header word + ceil(content_size / 8) data words.
        let elem_words = 1 + elem_size.div_ceil(8);
        for i in 0..o_length {
            let src_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_rec, 8 + 4 * i);
            // Append a slot to the destination array (4-byte rec-id stride);
            // `vector_append` allocates / resizes the dest vec_rec as needed.
            let slot = vector::vector_append(db, 4, &mut self.allocations);
            if src_rec == 0 {
                // Preserve null elements as null slots in the destination.
                self.store_mut(db).set_u32_raw(slot.rec, slot.pos, 0);
            } else {
                // Claim a fresh element record in the destination's store and
                // copy the source record's data + nested heap into it.
                let new_rec = self.allocations[db.store_nr as usize].claim(elem_words);
                if db.store_nr == o_db.store_nr {
                    self.store_mut(db)
                        .copy_block(src_rec, 8, new_rec, 8, elem_size as isize);
                } else {
                    // @PLAN53 cluster 3: sound disjoint-borrow cross-store copy.
                    self.copy_block_cross_store(
                        o_db.store_nr,
                        src_rec,
                        8,
                        db.store_nr,
                        new_rec,
                        8,
                        elem_size as isize,
                    );
                }
                self.copy_claims(
                    &DbRef {
                        store_nr: o_db.store_nr,
                        rec: src_rec,
                        pos: 8,
                    },
                    &DbRef {
                        store_nr: db.store_nr,
                        rec: new_rec,
                        pos: 8,
                    },
                    known,
                );
                self.store_mut(db).set_u32_raw(slot.rec, slot.pos, new_rec);
                // LOFT_WATCH_STORE — flag this element if the append left a garbage text-ptr.
                self.watch_oob_text(
                    &DbRef {
                        store_nr: db.store_nr,
                        rec: new_rec,
                        pos: 8,
                    },
                    known,
                    Some(&DbRef {
                        store_nr: o_db.store_nr,
                        rec: src_rec,
                        pos: 8,
                    }),
                    "vector_add_array",
                );
            }
            vector::vector_finish(db, &mut self.allocations);
        }
    }

    pub fn vector_set_size(&mut self, db: &DbRef, adding: u32, size: u32) {
        let store = keys::mut_store(db, &mut self.allocations);
        let mut vec_rec = store.get_u32_raw(db.rec, db.pos);
        let length = store.get_u32_raw(vec_rec, 4);
        if adding > 1 {
            let new_vec = store.resize(vec_rec, ((length + adding) * size + 15) / 8);
            if new_vec != vec_rec {
                store.set_u32_raw(db.rec, db.pos, new_vec);
                // track the relocation so the length write below lands
                // in the current record instead of the freed one.
                vec_rec = new_vec;
            }
        }
        store.set_u32_raw(vec_rec, 4, length + adding);
    }

    /// Walk a [`crate::json::Parsed`] tree into the record at
    /// `to`, dispatching on the target type's [`Parts`] variant.
    /// Returns `Ok(())` on success, `Err(WalkErr)` with byte offset
    /// + dotted path on a shape/type mismatch.
    ///
    /// `at` is the byte offset in the source text where this value
    /// was parsed (for struct fields, the field's key offset; for
    /// array elements / top-level, 0 — `Parsed::Array` doesn't carry
    /// per-element offsets).  Used to populate the `at` field of
    /// `WalkErr` when a leaf hits a type mismatch, so users see the
    /// real "line N:M path:X" instead of "line 1:1 path:X".
    ///
    /// Schema-driven counterpart to the parser-side
    /// [`crate::json::parse_with(text, Dialect::Lenient)`] — the
    /// parser stays schema-free; all type dispatch lives here.
    /// Together they form the only `text → struct` path in the
    /// crate; the legacy hand-rolled scanner was removed when
    /// every Parts arm gained walker coverage.
    #[allow(clippy::ptr_arg, clippy::too_many_arguments)] // path push/pop; arg-count is intrinsic to the dispatch
    pub(super) fn walk_parsed_into(
        &mut self,
        parsed: &crate::json::Parsed,
        tp: u16,
        rec_tp: u16,
        field: u16,
        to: &DbRef,
        path: &mut Vec<String>,
        at: usize,
    ) -> Result<(), WalkErr> {
        // `null` at any target position resets to the type's
        // default sentinel — mirrors the legacy scanner's
        // first-line behaviour and keeps round-tripping correct.
        if matches!(parsed, crate::json::Parsed::Null) {
            self.set_default_value(tp, to);
            return Ok(());
        }
        let mismatch = || WalkErr {
            at,
            path: path.clone(),
        };
        match self.types[tp as usize].parts.clone() {
            Parts::Base => self.walk_primitive_into(parsed, tp, to, path, at),
            Parts::Sorted(c, _)
            | Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Radix(c, _)
            | Parts::Index(c, _, _) => {
                let crate::json::Parsed::Array(items) = parsed else {
                    return Err(mismatch());
                };
                // @P357: an EMPTY JSON array must still zero the collection
                // header.  The per-item loop below is the ONLY thing that
                // initialises the vector/array field (the first `record_new`
                // writes its header) — so with zero items the field keeps
                // whatever bytes the recycled store record held, reading back
                // as a phantom non-zero length (e.g. `json_parse("[]").item(0)`
                // returning a garbage object, and `len` reporting 8 after a
                // run of earlier parses populated then freed that block).  The
                // `Parts::Null` arm above already calls `set_default_value` for
                // exactly this reason; do the same when the array is empty.
                if items.is_empty() {
                    // @P373: write the default to the COLLECTION FIELD's slot,
                    // not to `to` — which for a struct field is the struct base
                    // (field 0), so `set_default_value(tp, to)` zeroed the FIRST
                    // field's bytes and corrupted it (e.g. `{"name":"b","items":[]}`
                    // read `name` back as "").  A collection field reaches here
                    // via walk_parsed_struct's `else` branch with `to` = base +
                    // `field` = its index, exactly as the non-empty path below
                    // feeds `record_new(to, rec_tp, field)`.  `field_ref` maps
                    // (to, rec_tp, field) → the field slot, and returns `*to`
                    // unchanged when `field == u16::MAX` (top-level `json_parse("[]")`,
                    // the @P357 case), so that path is preserved.
                    let slot = self.field_ref(to, rec_tp, field);
                    self.set_default_value(tp, &slot);
                }
                for (idx, item) in items.iter().enumerate() {
                    path.push(format!("[{idx}]"));
                    let res = self.record_new(to, rec_tp, field);
                    self.walk_parsed_into(item, c, c, u16::MAX, &res, path, at)?;
                    self.record_finish(to, &res, rec_tp, field);
                    path.pop();
                }
                Ok(())
            }
            Parts::Struct(object) | Parts::EnumValue(_, object) => {
                // A type-tagged constructor `Type{…}` unwraps to its body for a
                // struct target — the tag is informational (the schema fixes the
                // type).  A plain object reads as fields directly, so old
                // un-tagged dumps still load: this is the disambiguation a single
                // `Constructor` node buys us over the collapsed-object shape.
                let body = match parsed {
                    crate::json::Parsed::Constructor(_tag, _, inner) => inner.as_ref(),
                    other => other,
                };
                self.walk_parsed_struct(body, tp, to, &object, path, at)
            }
            Parts::Enum(fields) => {
                // Accepted shapes:
                //   - `Str("Tag")` / `Ident("Tag")` / `Ident("Enum.Tag")` — unit
                //   - `Constructor("Tag", _, body)` — variant with payload (the
                //     `Tag { … }` shape from the Lenient parser)
                //   - `Object([("Tag", _, body)])` — the legacy collapsed shape,
                //     still accepted so older `{"Tag":{…}}` dumps keep loading
                // @PLN25 E2 (A4): a synthetic `__nullable<S>` enum (a `vector<S>`
                // element rewritten so it can be null — see `Data::nullable_enum_for`)
                // deserialises a bare JSON object as the PRESENT case: the object
                // holds S's fields directly, not a variant tag.  Route it to the
                // `Some` variant with the whole object as the payload.  `null` is
                // already handled above (Parsed::Null → `set_default_value` → the
                // absent disc 0).  Without this a present struct object falls to the
                // `_` mismatch arm and the `?` in the array loop aborts the whole
                // parse, dropping every present element (`[{…},null]` → len 0).
                let synth_nullable = self.types[tp as usize].name.starts_with("__nullable<");
                let (name, payload) = match parsed {
                    crate::json::Parsed::Object(_) | crate::json::Parsed::Constructor(..)
                        if synth_nullable =>
                    {
                        ("Some", Some(parsed))
                    }
                    crate::json::Parsed::Str(s) | crate::json::Parsed::Ident(s) => {
                        (s.as_str(), None)
                    }
                    crate::json::Parsed::Constructor(tag, _, body) => {
                        (tag.as_str(), Some(body.as_ref()))
                    }
                    crate::json::Parsed::Object(entries) if entries.len() == 1 => {
                        (entries[0].0.as_str(), Some(&entries[0].2))
                    }
                    _ => return Err(mismatch()),
                };
                // Accept a *qualified* tag (`Enum.Variant`): match on the last
                // segment.  The schema already fixes the enum type, so the prefix
                // is informational — the lenient dialect does not validate it.
                let name = name.rsplit('.').next().unwrap_or(name);
                let mut enum_tp = u16::MAX;
                let val = if name == "null" {
                    0
                } else {
                    // No-match degrades to the null sentinel (0), NOT variant 1:
                    // an unknown tag (e.g. a variant removed by a schema edit)
                    // must read back as null, never silently as a wrong variant.
                    // Preserve-as-much-as-possible across data-structure changes.
                    let mut v = 0;
                    for (f_nr, f) in fields.iter().enumerate() {
                        if f.1 == name {
                            v = f_nr as i32 + 1;
                            enum_tp = f.0;
                            break;
                        }
                    }
                    v
                };
                self.store_mut(to).set_byte(to.rec, to.pos, 0, val);
                // @PLN25 single-payload: a synth `__nullable<S>` `Some` variant holds the
                // object in its inline `payload` dense-`S` field — recurse the body into
                // the payload sub-record (`to.pos + payload offset`), NOT the `Some` variant
                // (whose direct fields are {enum, payload}, not S's).
                if synth_nullable
                    && name == "Some"
                    && enum_tp != u16::MAX
                    && let Some(body) = payload
                {
                    let pinfo = if let Parts::Struct(sf) | Parts::EnumValue(_, sf) =
                        &self.types[enum_tp as usize].parts
                    {
                        sf.iter()
                            .find(|f| f.name == "payload")
                            .map(|f| (f.content, f.position))
                    } else {
                        None
                    };
                    if let Some((pcontent, ppos)) = pinfo {
                        let payload_to = DbRef {
                            store_nr: to.store_nr,
                            rec: to.rec,
                            pos: to.pos + u32::from(ppos),
                        };
                        return self.walk_parsed_into(
                            body,
                            pcontent,
                            rec_tp,
                            field,
                            &payload_to,
                            path,
                            at,
                        );
                    }
                    return Ok(());
                }
                // Variant-with-payload (hand-written struct-enum): recurse so the payload
                // fields land in the same slot as the discriminant byte.
                if let Some(body) = payload
                    && enum_tp != u16::MAX
                    && self.types[enum_tp as usize].size > 1
                {
                    return self.walk_parsed_into(body, enum_tp, rec_tp, field, to, path, at);
                }
                Ok(())
            }
            Parts::Byte(from, _null) => {
                let Some(n) = parsed.as_i64() else {
                    return Err(mismatch());
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to).set_byte(to.rec, to.pos, from, n as i32);
                Ok(())
            }
            Parts::Short(from, _null) => {
                let Some(n) = parsed.as_i64() else {
                    return Err(mismatch());
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to)
                    .set_short(to.rec, to.pos, from, n as i32);
                Ok(())
            }
            Parts::ShortRaw(from, _null) => {
                let Some(n) = parsed.as_i64() else {
                    return Err(mismatch());
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to)
                    .set_i16_raw(to.rec, to.pos, from, n as i32);
                Ok(())
            }
            Parts::Int(_from, _null) => {
                let Some(v) = parsed.as_i64() else {
                    return Err(mismatch());
                };
                #[allow(clippy::cast_possible_truncation)]
                let raw = if v == i64::MIN { i32::MIN } else { v as i32 };
                self.store_mut(to).set_i32_raw(to.rec, to.pos, raw);
                Ok(())
            }
            // Plan-06 phase 4d.C step 2: stored DbRef pointer has no
            // JSON-representable form — closures don't survive a JSON
            // round-trip.  Surface as a clean error.
            Parts::DbRef => Err(mismatch()),
            // P213: child-record pointer (closures) — same JSON
            // restriction as DbRef.
            Parts::ChildRec(_) => Err(mismatch()),
        }
    }

    /// Schema-driven struct fill from a [`crate::json::Parsed::Object`].
    /// Matches fields by name, recurses into the walker for each
    /// value (passing the field's key byte offset as the recursion's
    /// `at` hint so a leaf type-mismatch reports the field's position),
    /// default-fills any unmentioned field (mirroring the legacy
    /// scanner's "missing field → default" behaviour).
    #[allow(clippy::ptr_arg)] // path needs push/pop, slice not enough
    fn walk_parsed_struct(
        &mut self,
        parsed: &crate::json::Parsed,
        tp: u16,
        to: &DbRef,
        object: &[Field],
        path: &mut Vec<String>,
        at: usize,
    ) -> Result<(), WalkErr> {
        let crate::json::Parsed::Object(entries) = parsed else {
            return Err(WalkErr {
                at,
                path: path.clone(),
            });
        };
        let fld = if to.rec == 0 { 0 } else { to.pos };
        let rec = if to.rec == 0 {
            let size = self.types[tp as usize].size;
            self.store_mut(to).claim(u32::from(size).div_ceil(8))
        } else {
            to.rec
        };
        let mut found_fields: HashSet<&str> = HashSet::new();
        for (name, key_at, value) in entries {
            let mut matched = false;
            for (f_nr, f) in object.iter().enumerate() {
                if f.name == *name {
                    matched = true;
                    path.push(name.clone());
                    let res = if self.content(f.content) == u16::MAX {
                        let slot = DbRef {
                            store_nr: to.store_nr,
                            rec,
                            pos: fld + u32::from(f.position),
                        };
                        self.walk_parsed_into(
                            value,
                            f.content,
                            tp,
                            f_nr as u16,
                            &slot,
                            path,
                            *key_at,
                        )
                    } else {
                        self.walk_parsed_into(value, f.content, tp, f_nr as u16, to, path, *key_at)
                    };
                    res?;
                    path.pop();
                    break;
                }
            }
            if !matched {
                // @P366: an unknown JSON key has no matching struct field.
                // Skip it (lenient-ignore) rather than aborting the element /
                // the whole array.  This matches the dynamic `JsonValue` walker
                // (`populate_struct_from_jsonvalue`), which only visits declared
                // fields and silently tolerates extra keys.  Previously this
                // returned a `WalkErr`, and the array loop's `?` aborted the
                // entire parse → `text as vector<Struct>` returned a silently
                // empty vector (`len == 0`) whenever the JSON carried any field
                // the struct did not declare.
                continue;
            }
            found_fields.insert(name.as_str());
        }
        for f in object {
            if (f.other_indexes.is_empty() || f.other_indexes[0] != u16::MAX)
                && !found_fields.contains(f.name.as_str())
                && f.name != "enum"
            {
                let slot = DbRef {
                    store_nr: to.store_nr,
                    rec,
                    pos: fld + u32::from(f.position),
                };
                self.set_default_value(f.content, &slot);
            }
        }
        Ok(())
    }

    /// Schema-driven primitive write.  `tp` is one of the
    /// low-numbered base-type IDs (0 = int32/Reference, 1 = long,
    /// 2 = single, 3 = float, 4 = bool, 5 = text, 6 = Reference).
    /// `at` is the byte offset where this primitive was parsed —
    /// reported on the [`WalkErr`] of any type mismatch so users
    /// see the real position instead of byte 0.
    #[allow(clippy::ptr_arg)] // path needs push/pop, slice not enough
    fn walk_primitive_into(
        &mut self,
        parsed: &crate::json::Parsed,
        tp: u16,
        to: &DbRef,
        path: &mut Vec<String>,
        at: usize,
    ) -> Result<(), WalkErr> {
        let mismatch = || WalkErr {
            at,
            path: path.clone(),
        };
        match tp {
            0 | 6 => {
                let Some(n) = parsed.as_i64() else {
                    return Err(mismatch());
                };
                self.store_mut(to).set_int(to.rec, to.pos, n);
                Ok(())
            }
            1 => {
                let Some(n) = parsed.as_i64() else {
                    return Err(mismatch());
                };
                self.store_mut(to).set_long(to.rec, to.pos, n);
                Ok(())
            }
            2 => {
                let Some(n) = parsed.as_f64() else {
                    return Err(mismatch());
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to).set_single(to.rec, to.pos, n as f32);
                Ok(())
            }
            3 => {
                let Some(n) = parsed.as_f64() else {
                    return Err(mismatch());
                };
                self.store_mut(to).set_float(to.rec, to.pos, n);
                Ok(())
            }
            4 => {
                let crate::json::Parsed::Bool(b) = parsed else {
                    return Err(mismatch());
                };
                self.store_mut(to)
                    .set_byte(to.rec, to.pos, 0, i32::from(*b));
                Ok(())
            }
            5 => {
                // Text accepts only a quoted string — bare
                // identifiers (`Parsed::Ident`) are NOT promoted to
                // text, matching the legacy `match_text` behaviour.
                let crate::json::Parsed::Str(s) = parsed else {
                    return Err(mismatch());
                };
                let text_pos = self.store_mut(to).set_str(s);
                self.store_mut(to).set_u32_raw(to.rec, to.pos, text_pos);
                Ok(())
            }
            _ => Err(mismatch()),
        }
    }

    /**
        Write default(null) values on all fields. This should normally only be done while debugging
        as all fields should be set anyway under correctly generated code.
        # Panics
        On inconsistent database definitions.
    */
    pub fn set_default_value(&mut self, tp: u16, rec: &DbRef) {
        // @PLN25 — a forward-referenced field's content can still be u16::MAX here (its known_type
        // is not laid out yet — e.g. a `__nullable<S>` element of a forward-ref'd struct, gate-on
        // 371_p375_forward_ref_positions).  It has no per-type default to write, and zero-on-claim
        // already zeroed the record (0 = the correct default: a `null` discriminant for a nullable
        // field, or a zero scalar), so skip rather than OOB-index `self.types[tp]` below.
        if tp == u16::MAX {
            return;
        }
        if tp <= 6 {
            match tp {
                0 => {
                    self.store_mut(rec).set_int(rec.rec, rec.pos, i64::MIN);
                }
                6 => {
                    // Content type 6 is a 4-byte u32-raw field (read via `get_u32_raw`,
                    // e.g. a `character` codepoint), NOT an 8-byte integer.  The old
                    // `set_int(i64::MIN)` wrote 8 bytes — its high 4 bytes spilled into
                    // the NEXT slot, which silently corrupts a tightly-sized record when
                    // the field is the LAST one (a trailing `character` in a single-payload
                    // `Some` payload overran the record and clobbered the adjacent free
                    // block's size header → `fl_size` negate-overflow).  `i64::MIN`'s low 4
                    // bytes are 0, so write 0 in exactly 4 bytes — same field value, no spill.
                    self.store_mut(rec).set_i32_raw(rec.rec, rec.pos, 0);
                }
                1 => {
                    self.store_mut(rec).set_long(rec.rec, rec.pos, i64::MIN);
                }
                2 => {
                    self.store_mut(rec).set_single(rec.rec, rec.pos, f32::NAN);
                }
                3 => {
                    self.store_mut(rec).set_float(rec.rec, rec.pos, f64::NAN);
                }
                4 => {
                    self.store_mut(rec).set_byte(rec.rec, rec.pos, 0, 0);
                }
                5 => {
                    self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, 0);
                }
                _ => (),
            }
            return;
        }
        match self.types[tp as usize].parts.clone() {
            Parts::Enum(_) => {
                self.store_mut(rec).set_byte(rec.rec, rec.pos, 0, 0);
            }
            Parts::Byte(_, null) => {
                self.store_mut(rec)
                    .set_byte(rec.rec, rec.pos, 0, if null { 255 } else { 0 });
            }
            Parts::Short(_, null) => {
                self.store_mut(rec)
                    .set_short(rec.rec, rec.pos, 0, if null { 65535 } else { 0 });
            }
            Parts::ShortRaw(from, null) => {
                self.store_mut(rec).set_i16_raw(
                    rec.rec,
                    rec.pos,
                    from,
                    if null { i32::MIN } else { from },
                );
            }
            Parts::Int(_, null) => {
                self.store_mut(rec)
                    .set_i32_raw(rec.rec, rec.pos, if null { i32::MIN } else { 0 });
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in &fields {
                    if f.name == "type" && f.position == 0 {
                        self.store_mut(rec)
                            .set_short(rec.rec, rec.pos, 0, i32::from(tp));
                        continue;
                    }
                    self.set_default_value(
                        f.content,
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        },
                    );
                }
            }
            Parts::Sorted(_, _)
            | Parts::Ordered(_, _)
            | Parts::Radix(_, _)
            | Parts::Hash(_, _)
            | Parts::Index(_, _, _)
            | Parts::Array(_)
            | Parts::Vector(_) => {
                self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, 0);
            }
            // Plan-06 phase 4d.C step 2: default for a 12-byte
            // stored DbRef = the null-DbRef bytes (store_nr=u16::MAX,
            // rec=0, pos=0 — three u32 zeros works since u16::MAX as
            // u32 is 0xFFFF, BUT for "not initialised" we want the
            // sentinel pattern; write all zeros and let the read
            // path treat rec=0 as null).
            Parts::DbRef => {
                let s = self.store_mut(rec);
                s.set_u32_raw(rec.rec, rec.pos, 0);
                s.set_u32_raw(rec.rec, rec.pos + 4, 0);
                s.set_u32_raw(rec.rec, rec.pos + 8, 0);
            }
            // P213: default child-record pointer = 0 (null sentinel /
            // empty / non-capturing).
            Parts::ChildRec(_) => {
                self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, 0);
            }
            Parts::Base => {
                panic!(
                    "not implemented default {:?}",
                    self.types[tp as usize].parts
                );
            }
        }
    }

    #[must_use]
    pub fn get_ref(&self, db: &DbRef, fld: u32) -> DbRef {
        if db.rec == 0 {
            return DbRef {
                store_nr: db.store_nr,
                rec: 0,
                pos: 0,
            };
        }
        let store = self.store(db);
        let res = store.get_u32_raw(db.rec, db.pos + fld);
        DbRef {
            store_nr: db.store_nr,
            rec: res,
            pos: 8,
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn get_field(db: &DbRef, fld: u32) -> DbRef {
        DbRef {
            store_nr: db.store_nr,
            rec: db.rec,
            pos: db.pos + fld,
        }
    }

    pub fn copy_block(&mut self, from: &DbRef, to: &DbRef, len: u32) {
        unsafe {
            std::ptr::copy(
                self.store(from)
                    .ptr
                    .offset(from.rec as isize * 8 + from.pos as isize),
                self.store_mut(to)
                    .ptr
                    .offset(to.rec as isize * 8 + to.pos as isize),
                len as usize,
            );
        }
    }
}
