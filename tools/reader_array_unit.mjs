// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 corpus — a SYNTHETIC guard for readLoftValue's by-ref `array` arm (Parts::Array). No loft
// SOURCE construct currently emits a by-ref Array — every IR vector is inline Parts::Vector (see
// src/data_store.rs:128 "probed — none promoted to a linked Array") — so unlike the other corpus
// shapes this arm cannot be reached through a `deliver` program. This test hand-lays a wasm memory
// buffer + an `array` descriptor and asserts the reader indexes the element records correctly. (The
// per-element loop is identical to the `flatarray` arm the keyed-collection tests exercise; this
// pins the field→vRec→element-index path specific to `array`.)
//
// Exit 0 on match, 1 on mismatch.

import process from "node:process";
import { readLoftValue } from "../doc/loft-deliver.js";

const mem = new WebAssembly.Memory({ initial: 1 }); // 64 KiB; storeBase = 0
const dv = new DataView(mem.buffer);
const STORE = 0;
const at = (rec, pos) => STORE + rec * 8 + pos;

// Root `array` field at (rec=1, pos=0): a u32 record-index of the data vector, vRec=2.
dv.setUint32(at(1, 0), 2, true);
// vRec=2: length at vRec*8+4, then one element record-index per slot at vRec*8+8 + 4*i.
// Element records are 10/11/12 — chosen past rec 2's index block (addrs 16..35) so nothing overlaps.
dv.setUint32(at(2, 4), 3, true); // len = 3
dv.setUint32(at(2, 8 + 4 * 0), 10, true);
dv.setUint32(at(2, 8 + 4 * 1), 11, true);
dv.setUint32(at(2, 8 + 4 * 2), 12, true);
// Each element's data lives at elmRec*8+8 (the by-ref element convention the reader uses).
dv.setBigInt64(at(10, 8), 100n, true);
dv.setBigInt64(at(11, 8), 200n, true);
dv.setBigInt64(at(12, 8), 300n, true);

const desc = {
  nodes: {
    0: { kind: "array", elem: 1 },
    1: { kind: "base", base: "integer" },
  },
  names: {},
  sizes: {},
};

const got = readLoftValue(mem, STORE, desc, 0, 1, 0);
const want = [100n, 200n, 300n];
const ok =
  Array.isArray(got) && got.length === want.length && got.every((v, i) => v === want[i]);
if (!ok) {
  const show = (a) => JSON.stringify(a, (_k, v) => (typeof v === "bigint" ? String(v) : v));
  console.error(`ARRAY-UNIT FAIL want=${show(want)} got=${show(got)}`);
  process.exit(1);
}
console.log("ARRAY-UNIT OK " + got.map(String).join(","));
