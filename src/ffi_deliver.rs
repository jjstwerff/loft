// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
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
        let name = self
            .types
            .get(db_tp as usize)
            .map_or_else(|| format!("#{db_tp}"), |t| t.name.clone());
        let desc = self.layout_descriptor(&[db_tp]);
        let mut bytes = Vec::new();
        match self.read_via_descriptor(&desc, &val, db_tp, true, &mut bytes) {
            Ok(()) => {
                let mut hex = String::with_capacity(bytes.len() * 2);
                for b in &bytes {
                    hex.push_str(&format!("{b:02x}"));
                }
                println!("deliver tag={tag} type={name} bytes={hex}");
            }
            Err(e) => println!("deliver tag={tag} type={name} error={e}"),
        }
    }
}
