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
        if val.rec == 0 {
            return;
        }
        let mut desc = self.layout_descriptor(&[db_tp]);
        // Recursively replace every keyed collection reachable through inline record fields with its
        // array-shaped node; `root` is the (possibly rewritten) node to describe the value.
        let mut synth = FLAT_ARRAY_ROOT;
        let root = self.flatten_at(&mut desc, db_tp, val, &mut synth);
        self.emit_deliver(tag, val, root, &desc.to_json());
    }

    /// The descriptor node that replaces an `Iterated` keyed collection at `coll_ref` (type `tp`)
    /// so JS can read its elements as an array (no keyed-layout knowledge). Two shapes:
    ///   * `hash` / `radix`/`spatial` have no walkable byte layout → MATERIALISE to a scratch array
    ///     (loft's own `for x in coll` path, so JS gets the same key/Morton order) and return a
    ///     `FlatArray` node carrying the scratch's fixed data record.
    ///   * `sorted` is ALREADY an INLINE vector kept IN KEY ORDER (`sorted_finish` inserts each
    ///     element in sorted position on every `+=`), and its field holds that vector directly →
    ///     just RECLASSIFY to an in-place `Vector` node (no materialisation).
    /// `ordered` / `index` have no array form yet (a later slice) → `None`.
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn keyed_replacement(
        &mut self,
        coll_ref: DbRef,
        tp: u16,
        it: &crate::database::Iterated,
    ) -> Option<crate::database::LayoutNode> {
        use crate::database::{Iterated, LayoutNode};
        let elem = it.elem();
        let scratch = match it {
            Iterated::Hash { .. } => self.build_hash_sorted_vec(&coll_ref, tp),
            Iterated::Radix { .. } => self.build_radix_sorted_vec(&coll_ref, tp),
            Iterated::Index { .. } => self.build_index_sorted_vec(&coll_ref, tp),
            Iterated::Sorted { .. } => return Some(LayoutNode::Vector(elem)),
            Iterated::Ordered { .. } => return None,
        };
        let data =
            self.allocations[scratch.store_nr as usize].get_u32_raw(scratch.rec, scratch.pos);
        Some(LayoutNode::FlatArray { elem, data })
    }

    /// Recursively replace every keyed collection reachable through INLINE record fields — the root
    /// itself, a direct field, or a field of a nested sub-struct — with its array-shaped node,
    /// returning the (possibly new) node id that describes `at`. A changed record/keyed node is
    /// re-inserted under a fresh synthetic id (decrementing from `u16::MAX`) so the shared TYPE
    /// descriptor is not mutated and each location gets its own materialised data record.
    ///
    /// Nested INLINE records stay in the same store record (just offset by the field position), so
    /// a keyed field of a sub-struct is at `(at.rec, at.pos + field.pos …)` — single-instance and
    /// addressable. NOT descended: `Vector`/`Array` ELEMENTS — those are multi-instance (each
    /// element's keyed collection lives at a different record), which a single fixed-data
    /// `FlatArray` cannot express; a keyed collection behind a vector element is a later slice
    /// (needs a different, per-element protocol).
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn flatten_at(
        &mut self,
        desc: &mut crate::database::LayoutDesc,
        node_id: u16,
        at: DbRef,
        synth: &mut u16,
    ) -> u16 {
        use crate::database::LayoutNode;
        match desc.nodes.get(&node_id).cloned() {
            Some(LayoutNode::Iterated(it)) => match self.keyed_replacement(at, node_id, &it) {
                Some(node) => {
                    let id = *synth;
                    *synth -= 1;
                    desc.nodes.insert(id, node);
                    id
                }
                None => node_id, // ordered — unsupported, left as Iterated (reader errors)
            },
            Some(LayoutNode::Record(fields)) => {
                match self.flatten_fields(desc, &fields, at, synth) {
                    Some(nf) => {
                        let id = *synth;
                        *synth -= 1;
                        desc.nodes.insert(id, LayoutNode::Record(nf));
                        id
                    }
                    None => node_id,
                }
            }
            Some(LayoutNode::EnumValue(tag, fields)) => {
                match self.flatten_fields(desc, &fields, at, synth) {
                    Some(nf) => {
                        let id = *synth;
                        *synth -= 1;
                        desc.nodes.insert(id, LayoutNode::EnumValue(tag, nf));
                        id
                    }
                    None => node_id,
                }
            }
            // Scalar / text / vector / array / ref — no single-instance keyed collection to descend.
            _ => node_id,
        }
    }

    /// Recurse `flatten_at` into each DATA field of a record (skipping the enum discriminant, absent,
    /// and `#`-synthetic fields, exactly as the reader does); returns a new field list iff any field
    /// changed. A nested field is at `(at.rec, at.pos + field.position)` (inline records share the
    /// record; a keyed field slot holds the collection root there).
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn flatten_fields(
        &mut self,
        desc: &mut crate::database::LayoutDesc,
        fields: &[crate::database::LayoutField],
        at: DbRef,
        synth: &mut u16,
    ) -> Option<Vec<crate::database::LayoutField>> {
        let mut new_fields = fields.to_vec();
        let mut changed = false;
        for (i, f) in fields.iter().enumerate() {
            if f.name == "enum" || f.position == u16::MAX || f.name.starts_with('#') {
                continue;
            }
            let field_at = DbRef {
                store_nr: at.store_nr,
                rec: at.rec,
                pos: at.pos + u32::from(f.position),
            };
            let new_content = self.flatten_at(desc, f.content, field_at, synth);
            if new_content != f.content {
                new_fields[i].content = new_content;
                changed = true;
            }
        }
        if changed { Some(new_fields) } else { None }
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
