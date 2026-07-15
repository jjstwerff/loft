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

impl Stores {
    /// @PLN105 Phase 1 loopback host — reconstruct the delivered value from its
    /// layout descriptor and print a deterministic line. Called from the `OpDeliver`
    /// `#rust` body on both backends, so its output is the parity oracle.
    ///
    /// `val` is the value's record `DbRef` (a by-value struct/collection argument
    /// arrives already deref'd to its record on both backends). Scalars delivered by
    /// value are not record-shaped and are out of the Phase-1 subset.
    pub fn deliver_reconstruct(&self, tag: i64, val: DbRef, db_tp: u16) {
        // @PLN105 Phase 2 — browser target: hand the value to the JS host instead of the loopback.
        // Serialize the layout descriptor to JSON, compute the value record's RAW byte address in
        // wasm linear memory (`store.ptr + offset`), and call the `loft_host_deliver` import; the
        // generic JS reader walks the value from `base` driven by the descriptor (§2), SYNCHRONOUSLY
        // within this call so the borrow is still live (§5). `desc` stays owned until after the call.
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
        {
            if val.rec != 0 {
                let desc = self.layout_descriptor(&[db_tp]).to_json();
                let store = &self.allocations[val.store_nr as usize];
                // `Store.ptr` IS the store buffer's base address in wasm linear memory; the reader
                // addresses everything as `store_base + rec*8 + pos`, so it can follow child records.
                let store_base = store.ptr as usize;
                crate::loft_host_deliver(
                    tag,
                    store_base,
                    val.rec,
                    val.pos,
                    u32::from(db_tp),
                    desc.as_ptr(),
                    desc.len(),
                );
            }
            return;
        }
        // native / interpreter / wasi — the Phase-1 LOOPBACK host (the both-backend parity oracle).
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))))]
        {
            self.deliver_loopback(tag, val, db_tp);
        }
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
