// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @F54 — Browser / WASM target: the `deliver` FFI boundary that hands the host
// (the browser JS bridge) a self-describing handle to a live loft value.
//
// @PLN105 Phase 1 — the `deliver` boundary + its LOOPBACK host.
//
// `deliver(tag, value)` hands the host a self-describing handle to a live loft
// value; the host reads the value's layout DESCRIPTOR (@PLN105 Phase 0) and walks
// the bytes with no serialization. Phase 1 wires the boundary with a LOOPBACK host
// (no browser): the host reconstructs the value from the descriptor and prints a
// deterministic line. The whole point is the parity gate — `deliver` lowers to ONE
// `#rust` body (`stores.deliver_reconstruct(...)`) that feeds BOTH the interpreter
// (via fill.rs) and native codegen, and a value passed BY VALUE arrives as the same
// record `DbRef` on both backends (matching native's `file_to_bytes(self)`
// convention, sidestepping the interp/native slot-vs-record deref asymmetry). So
// `--interpret` and `--native` must print byte-identical output for any value.
//
// The descriptor reconstruction reuses `read_via_descriptor`, already proven
// (Phase 0) to reproduce `read_data`'s bytes; Phase 1 proves the HANDLE reaches the
// host correctly and identically across the loft-call boundary on both backends.

use crate::database::Stores;
use crate::keys::DbRef;

/// @PLN105 Phase 3 — the base synthetic descriptor type-id for a pre-flattened keyed collection's
/// `FlatArray` node (a top-level keyed root, or one per keyed field, decrementing). `u16::MAX`
/// never collides with a real type-id (it is the type table's "no type" sentinel), so it is safe
/// to insert alongside the element type's closure.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
const FLAT_ARRAY_ROOT: u16 = u16::MAX;

impl Stores {
    /// @PLN105 Phase 1 loopback host — reconstruct the delivered value from its
    /// layout descriptor and print a deterministic line. Called from the `OpDeliver`
    /// `#rust` body on both backends, so its output is the parity oracle.
    ///
    /// `val` is the value's record `DbRef` (a by-value struct/collection argument
    /// arrives already deref'd to its record on both backends). Scalars delivered by
    /// value are not record-shaped and are out of the Phase-1 subset.
    ///
    /// `&mut self` because the browser path may MATERIALISE a keyed collection into a scratch
    /// vector before delivery (Phase 3 pre-flatten); the loopback path only reads.
    pub fn deliver_reconstruct(&mut self, tag: i64, val: DbRef, db_tp: u16) {
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
        {
            self.deliver_browser(tag, val, db_tp);
        }
        // native / interpreter / wasi — the Phase-1 LOOPBACK host (the both-backend parity oracle).
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))))]
        {
            self.deliver_loopback(tag, val, db_tp);
        }
    }

    /// @PLN105 Phase 2/3 — the browser host path: hand the value to JS driven by its layout
    /// descriptor, with no serialization (the value bytes stay in wasm linear memory). A struct /
    /// scalar / vector is delivered in place: JS walks it from `(store_base, rec, pos)` via the
    /// descriptor (§2). A KEYED collection (a top-level hash, or a hash FIELD of a struct) has no
    /// byte layout a reader can walk, so (Phase 3) it is PRE-FLATTENED — materialised to a scratch
    /// array of its element records (the same `for x in hash` path) and exposed via a synthetic
    /// `FlatArray` descriptor node carrying that array's fixed data record; the generic reader
    /// reads it like any array, and JS never sees the hash/tree layout. SYNCHRONOUS — the borrow
    /// ends when this returns, so `desc` stays owned across the `loft_host_deliver` call.
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn deliver_browser(&mut self, tag: i64, val: DbRef, db_tp: u16) {
        use crate::database::{Iterated, LayoutNode};
        if val.rec == 0 {
            return;
        }
        let mut desc = self.layout_descriptor(&[db_tp]);
        match desc.nodes.get(&db_tp).cloned() {
            // A top-level keyed `hash<T>`: materialise to an array + deliver a FlatArray root.
            Some(LayoutNode::Iterated(Iterated::Hash { elem, .. })) => {
                let data = self.materialize_hash(val, db_tp);
                let mut adesc = self.layout_descriptor(&[elem]);
                adesc
                    .nodes
                    .insert(FLAT_ARRAY_ROOT, LayoutNode::FlatArray { elem, data });
                self.emit_deliver(tag, val, FLAT_ARRAY_ROOT, &adesc.to_json());
            }
            // A struct: flatten any DIRECT keyed fields in place (each becomes a FlatArray node).
            Some(LayoutNode::Record(_)) => {
                self.flatten_record_keyed_fields(&mut desc, val, db_tp);
                self.emit_deliver(tag, val, db_tp, &desc.to_json());
            }
            // Scalar / vector / nested record without keyed fields — the P2.c in-place path.
            _ => self.emit_deliver(tag, val, db_tp, &desc.to_json()),
        }
    }

    /// Materialise a keyed hash at `hash_ref` (of type `tp`) to a scratch element array and return
    /// its DATA record in the value's store — the fixed `data` of a `FlatArray` node.
    /// `build_rec_scratch` returns `(header, pos)` where `header[pos]` holds the data record.
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn materialize_hash(&mut self, hash_ref: DbRef, tp: u16) -> u32 {
        let scratch = self.build_hash_sorted_vec(&hash_ref, tp);
        self.allocations[scratch.store_nr as usize].get_u32_raw(scratch.rec, scratch.pos)
    }

    /// Replace each DIRECT keyed FIELD of record `db_tp` with a `FlatArray` node carrying the
    /// materialised element array — so a struct with a `hash` field delivers as
    /// `{…scalars…, field: [elements]}`. The struct's own bytes stay in place; the keyed field's
    /// in-place bytes (a store-internal hash) are ignored in favour of the FlatArray's fixed data
    /// record. Keyed collections nested DEEPER (inside a sub-struct, or as an element type) are a
    /// later slice.
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn flatten_record_keyed_fields(
        &mut self,
        desc: &mut crate::database::LayoutDesc,
        val: DbRef,
        db_tp: u16,
    ) {
        use crate::database::{Iterated, LayoutNode};
        let fields = match desc.nodes.get(&db_tp) {
            Some(LayoutNode::Record(f)) => f.clone(),
            _ => return,
        };
        let mut new_fields = fields.clone();
        let mut synth = FLAT_ARRAY_ROOT;
        let mut changed = false;
        for (i, f) in fields.iter().enumerate() {
            let elem = match desc.nodes.get(&f.content) {
                Some(LayoutNode::Iterated(Iterated::Hash { elem, .. })) => *elem,
                _ => continue,
            };
            // The hash field SLOT `(val.rec, val.pos + f.position)` holds the bucket claim
            // (`hash::records` reads it there), so it IS the hash's DbRef for materialisation.
            let hash_ref = DbRef {
                store_nr: val.store_nr,
                rec: val.rec,
                pos: val.pos + u32::from(f.position),
            };
            let data = self.materialize_hash(hash_ref, f.content);
            desc.nodes
                .insert(synth, LayoutNode::FlatArray { elem, data });
            new_fields[i].content = synth;
            synth -= 1;
            changed = true;
        }
        if changed {
            desc.nodes.insert(db_tp, LayoutNode::Record(new_fields));
        }
    }

    /// Call the `loft_host_deliver` import: `Store.ptr` is the store buffer's base in wasm linear
    /// memory, so the reader addresses everything as `store_base + rec*8 + pos`.
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn emit_deliver(&self, tag: i64, r: DbRef, type_id: u16, desc: &str) {
        let store_base = self.allocations[r.store_nr as usize].ptr as usize;
        crate::loft_host_deliver(
            tag,
            store_base,
            r.rec,
            r.pos,
            u32::from(type_id),
            desc.as_ptr(),
            desc.len(),
        );
    }

    /// The Phase-1 loopback host — reconstruct the delivered value from its descriptor and print a
    /// deterministic line. The parity oracle for `--interpret` == `--native`. Split out of
    /// `deliver_reconstruct` so the browser (Phase 2) and loopback paths are each cfg-clean. Only
    /// the non-browser targets keep it (the browser calls the `loft_host_deliver` import instead).
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))))]
    fn deliver_loopback(&self, tag: i64, val: DbRef, db_tp: u16) {
        let name = self
            .types
            .get(db_tp as usize)
            .map_or_else(|| format!("#{db_tp}"), |t| t.name.clone());
        let desc = self.layout_descriptor(&[db_tp]);
        let mut bytes = Vec::new();
        match self.read_via_descriptor(&desc, &val, db_tp, true, &mut bytes) {
            Ok(()) => {
                use std::fmt::Write as _;
                let mut hex = String::with_capacity(bytes.len() * 2);
                for b in &bytes {
                    let _ = write!(hex, "{b:02x}");
                }
                println!("deliver tag={tag} type={name} bytes={hex}");
            }
            Err(e) => println!("deliver tag={tag} type={name} error={e}"),
        }
    }
}
